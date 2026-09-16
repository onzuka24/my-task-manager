---
title: '常駐の骨格 — 常駐プロセス・自動起動・ホットキー呼び出し'
type: 'feature'
created: '2026-09-15'
status: 'in-review'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: 'd35ce686464c3145106e818933c26208ca8372a3'
context:
  - '{project-root}/_bmad-output/specs/spec-my-task-manager/SPEC.md'
  - '{project-root}/_bmad-output/planning-artifacts/architecture/architecture-my-task-manager-2026-09-15/ARCHITECTURE-SPINE.md'
  - '{project-root}/_bmad-output/implementation-artifacts/research-scaffold.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** リポジトリにはコードが一行も存在しない。CAP-1 (ホットキー呼び出し) と CAP-3 (常駐と自動復帰) は他のすべての capability の土台であり、これが立たない限り何も検証できない。加えて AD-14 の資源予算 (待機時 CPU 1% 未満・メモリ 100MB 未満) は Electron を排除した根拠でありながら一度も実測されていない。骨格はそれを初めて測れる地点でもある。

**Approach:** Tauri 2.11 + Svelte 5 + Vite で macOS 常駐アプリの骨格を作る。Dock に出ず、ログイン時に自動起動し、グローバルホットキーで事前生成済みの隠しウィンドウを表示する。ドメインロジックは持たない — オーバーレイは中身のない器である。`ports/` と `adapters/` のディレクトリ境界 (AD-1) をこの時点で確立し、以降の capability がその内側に積まれるようにする。

## Boundaries & Constraints

**Always:**
- ディレクトリ構成は ARCHITECTURE-SPINE.md の「ソースツリー」に従う。OS API は `adapters/` 配下にのみ置く (AD-1)。`domain/` と `ports/` は空で作成する。
- ホットキーは `event.state == ShortcutState::Pressed` のみ処理する (AD-7)。1 押下で 1 トグル。
- ウィンドウは起動時に生成して隠す。押下時に生成しない (300ms 制約)。
- 外部送信コードを含めない (AD-12)。
- Rust は `rust-toolchain.toml` で固定し、利用者のグローバル既定を変更しない。
- **グローバルホットキーは `Ctrl+Option+Space`。** Spotlight とも入力ソース切替とも衝突しないため。
- **自動起動は `MacosLauncher::LaunchAgent`。** ログイン項目に `.app` ではなく内部の実行ファイル名で並ぶことは、既知の代償として受け入れる。
- **`tauri dev` では自動起動を登録しない。** `auto-launch` は `current_exe()` を登録するため、開発ビルドのパスがログイン項目に残る。リリースビルドでのみ有効化する。
- **オーバーレイはダミーの「次の一手」を 1 件表示する。** 単一の定数に切り出し `// TODO(CAP-2): ドメインモデル導入時に削除` を付す。次スライスで確実に除去できる形にすること。

**Never:**
- ドメインモデル・SQLite・介入パネルを実装しない (CAP-4/5/6/10 は別スライス)。
- `macos-private-api` を有効化しない。必要とするのは `transparent` のみで本スライスは透過を使わない。CAP-10 に先送り。
- SvelteKit を使わない。Tauri 3.x を使わない (`@latest` / `@next` を指定しない)。

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| 呼び出し | 他アプリが最前面、オーバーレイ非表示 | オーバーレイが最前面に表示されフォーカスを得る | N/A |
| 二重発火の抑止 | ホットキーを 1 回押下 | トグルが 1 回だけ起きる (Released で 2 回目が起きない) | N/A |
| 閉じる | オーバーレイ表示中に Esc | オーバーレイが隠れ、直前に最前面だったアプリにフォーカスが戻る | N/A |
| トグル | オーバーレイ表示中にホットキー | オーバーレイが隠れる (Esc と同じ経路) | N/A |
| ホットキー衝突 | 他アプリが同じキーを保持し登録に失敗 | 起動は継続し、失敗をログに記録して利用者に判る形で示す | 登録失敗を理由に常駐を止めない |
| 再起動後 | OS 再起動、ログイン | 利用者の操作なしに常駐プロセスが動作している | N/A |

</frozen-after-approval>

## Code Map

greenfield。既存コードはなく、再利用も破壊回避の対象もない。

- `_bmad-output/implementation-artifacts/research-scaffold.md` -- **実装前に必読。** 検証済みの scaffolding 手順、`Info.plist` の扱い、各プラグインの正確な API シグネチャ、footprint 計測スクリプト。docs.rs が誤っている箇所 (`event.state` はフィールドでありメソッドではない) を含む。
- `ARCHITECTURE-SPINE.md` の「ソースツリー」 -- ディレクトリ構成の正

## Tasks & Acceptance

**Execution:**
- [x] `rust-toolchain.toml` -- 現行 stable を固定 -- 宣言 MSRV (1.77.2) は依存ツリーの実要求より低く、手元の 1.81.0 では依存が解決しない可能性がある。グローバル既定を変更せずに回避する
- [x] `package.json` ほかフロント足場 -- Vite の `svelte-ts` テンプレートを使う -- `create-tauri-app` の Svelte テンプレートは SvelteKit であり採用しない
- [x] `src-tauri/tauri.conf.json` -- `app.windows[]` に `visible: false`, `focus: false`, `alwaysOnTop: true`, `decorations: false` を設定 -- 起動時生成・隠し保持で 300ms を満たす
- [x] `src-tauri/Info.plist` -- `LSUIElement` を設定 -- Dock に出さない。`bundle.macOS.infoPlist` はパス文字列でありインラインの辞書ではない
- [x] `src-tauri/Cargo.toml` -- `tauri 2.11`, `tauri-plugin-global-shortcut 2.3.2`, `tauri-plugin-autostart 2.5.1` を固定 -- 3.x を引かないよう厳密に指定する
- [x] `src-tauri/src/lib.rs` -- `setup()` で Dock 非表示化とプラグイン登録、ウィンドウの事前生成 -- 常駐の起点
- [x] `src-tauri/src/adapters/hotkey/mod.rs` -- ホットキー登録と `Pressed` のみのトグル処理 -- AD-7
- [x] `src-tauri/src/adapters/autostart/mod.rs` -- ログイン時自動起動の有効化 -- CAP-3
- [x] `src-tauri/src/adapters/presentation/mod.rs` -- 表示・非表示とフォーカス復帰 -- `show()` の後に `set_focus()` が必要。復帰は macOS の `AppHandle::hide()` を用いる
- [x] `src-tauri/src/domain/mod.rs` / `ports/mod.rs` -- 空モジュールを作成 -- AD-1 の境界を先に確立し、次スライスが正しい場所に積まれるようにする
- [x] `src/App.svelte` -- 器としてのオーバーレイ。Esc で閉じる -- 表示内容は Open Questions の決定に従う
- [x] `Makefile` -- ビルドから `/Applications` への配置までを 1 タスクに -- AD-13
- [x] `scripts/measure-footprint.sh` -- WKWebView ヘルパープロセスを含む全プロセスの実測 -- AD-14。研究成果の検証済みスクリプトを用いる
- [x] `.gitignore` -- `node_modules/`, `src-tauri/target/`, `dist/` を追記
- [x] `src-tauri/src/adapters/presentation/mod.rs` ほか -- トグル判定を OS 呼び出しから切り離した純粋関数として抽出し `#[cfg(test)]` で単体テストする -- I/O マトリクスの「二重発火の抑止」「トグル」「呼び出し」「閉じる」を自動テストで覆うため。OS やログインを伴う行 (ホットキー衝突・再起動後) は手動確認とし、その旨を Implementation Notes に記録すること

**Acceptance Criteria:**
- Given OS を再起動しログインした直後、when 利用者が何も操作しない、then 常駐プロセスが動作しており Dock にアイコンが出ていない
- Given 任意のアプリケーションが最前面、when グローバルホットキーを押下する、then 300ms 以内にオーバーレイが入力を受け付ける状態で最前面に出る
- Given オーバーレイが表示されている、when Esc を押す、then オーバーレイが隠れ、直前に最前面だったアプリケーションがフォーカスを回復する
- Given 待機状態 (オーバーレイ非表示)、when `scripts/measure-footprint.sh` を実行する、then WKWebView ヘルパーを含む全プロセス合計のメモリが 100MB 未満、CPU が 1% 未満である
- Given `make` を実行した、when 完了した、then `/Applications` に `.app` が配置され、そこから起動できる

## Implementation Notes

### 構成上の判断

- **`src/App.svelte` と `src/overlay/`。** タスク行は `src/App.svelte` を指し、
  ARCHITECTURE-SPINE.md のソースツリーは `src/overlay/` を指す。両方を満たすため、
  `App.svelte` はマウント点として `overlay/Overlay.svelte` を描画するだけの器とし、
  オーバーレイの中身は `src/overlay/` に置いた。
- **`src-tauri/src/commands/` を追加した。** タスク行には挙がっていないが、Esc で閉じる
  経路は AD-3 (フロント → コアは Tauri command のみ) に従う必要があり、ソースツリーにも
  `commands/` がある。`hide_overlay` と `get_overlay_snapshot` の 2 つだけを置いた。
  後者は AD-3 の鮮度規則 (表示のたびに完全なスナップショットを取得する) の型を先に
  作るためのもので、現時点ではホットキーの登録状態しか運んでいない。
- **ダミーの「次の一手」は `src/overlay/Overlay.svelte` の定数 `DUMMY_NEXT_ACTION`。**
  `// TODO(CAP-2): ドメインモデル導入時に削除` を付した。Rust 側に置かなかったのは、
  ドメインの内容物を `domain/` が空のまま別の場所に作らないため。
- **ホットキーの登録はプラグイン登録と分離した。** `Builder::with_shortcut()` で登録
  するとショートカットの登録失敗がプラグイン登録ごと失敗させ、後から再登録する手段まで
  失われる。プラグインはショートカットなしで登録し、`global_shortcut().register()` を
  別に呼ぶ。`hotkey::register()` は `Result` ではなく `HotkeyStatus` を返す
  — 「登録失敗を理由に常駐を止めない」を呼び出し側の規律ではなく型で保証するため。
- **登録失敗の可視化。** Dock アイコンもメニューバー項目も持たないため、失敗を利用者に
  示せる面はオーバーレイ自身しかない。登録に失敗した場合のみ起動時にオーバーレイを表示し、
  スナップショット経由で理由を出す。介入 (AD-15 が制限する能動的な働きかけ) ではなく、
  ホットキーが使えない場合の唯一の伝達経路である。
- **`backgroundThrottling: "throttle"`。** 既定の `suspend` は隠れた webview のタスクを
  完全に停止し、view のアンロードまで起こしうる。300ms 制約 (CAP-1) を守るため `throttle`
  を明示した。macOS 14 以降でのみ効くため `minimumSystemVersion` を `14.0` にした。
- **CSP を設定した。** ARCHITECTURE-SPINE.md では「未決 / 実装着手時に確認する」と
  されていた項目。AD-12 (外部送信を行わない) の多層防御として
  `default-src 'self'` 系を設定し、バンドル済みリリースビルドで IPC が通ることを確認した
  (下記「検証」)。
- **`macos-private-api` は有効化していない。** `transparent` を使わないため不要。CAP-10
  に先送り。

### toolchain の固定

宣言 MSRV (1.77.2) は依存ツリーの実要求より低く、手元のグローバル既定 1.81.0 では
`time-core 0.1.9` が `edition2024` を要求して解決に失敗する (実際に再現した)。

- `rust-toolchain.toml` で `channel = "1.98.1"` (計測時点の stable) を固定した。
- ただしこの環境では mise が `~/.tool-versions` の `rust 1.81.0` を **`RUSTUP_TOOLCHAIN`
  環境変数として export** しており、rustup の優先順位では環境変数が `rust-toolchain.toml`
  より強い。そのためプロジェクト直下に `mise.toml` を置いて同じ 1.98.1 を固定した。
  グローバル設定 (`~/.tool-versions` / `rustup default`) は変更していない。
- `Makefile` は `unexport RUSTUP_TOOLCHAIN` で環境変数を外し、mise を使わない環境でも
  `rust-toolchain.toml` が正になるようにしている。
- **mise を使わないシェルで `cargo` / `pnpm tauri dev` を直接叩く場合**、環境に
  `RUSTUP_TOOLCHAIN` が残っていると 1.81.0 で失敗する。`make` 経由か、
  `env -u RUSTUP_TOOLCHAIN` を付けること。

### テストの範囲 — 自動と手動の切り分け

トグル判定は OS 呼び出しから切り離した純粋関数として抽出してある。

- `adapters/presentation::toggle_action` — 可視状態から次の操作を決める
- `adapters/presentation::visibility_after` — 操作の適用後の可視状態
- `adapters/hotkey::should_toggle` — `Pressed` かつ対象のショートカットのときだけ真

`cargo test` の 9 件が I/O マトリクスの次の 4 行を覆う。

| I/O マトリクスの行 | 覆っているテスト |
| --- | --- |
| 呼び出し | `hotkey::tests::one_keypress_shows_a_hidden_overlay`, `presentation::tests::hidden_overlay_is_shown` |
| 二重発火の抑止 | `hotkey::tests::released_does_not_toggle` (1 押下 = Pressed + Released でトグルが 1 回だけ起きることを固定) |
| トグル | `hotkey::tests::one_keypress_hides_a_visible_overlay`, `presentation::tests::visible_overlay_is_hidden` |
| 閉じる | `presentation::tests::escape_always_hides` |

**自動テストで覆えず手動確認とした行・受け入れ基準** (OS・ログイン・実時間を伴うため):

- **ホットキー衝突** — 他アプリが同じキーを保持した状態での登録失敗。失敗経路自体は
  `HotkeyStatus` として実装されているが、衝突の再現には別アプリが要る。
- **再起動後の常駐** — OS 再起動とログインが要る。LaunchAgent plist の生成と内容
  (`/Applications/My Task Manager.app/Contents/MacOS/my-task-manager` を `RunAtLoad`)
  までは自動で確認済み。
- **300ms 以内の表示 / Esc でのフォーカス復帰** — この環境ではアクセシビリティ権限
  (キー送出) も画面収録権限も与えられておらず、`osascript` によるキー入力送出が
  `(1002) キー操作の送信は許可されません` で拒否される。ホットキー押下と Esc を伴う
  経路は実機での手動確認に残す。

### 検証で分かったこと (記録)

- バンドルされていない裸のリリースバイナリ (`src-tauri/target/release/my-task-manager`)
  を直接起動すると、ウィンドウは出るが webview がページを読み込まない
  (`on_page_load` が発火しない)。`.app` として起動すれば正常。**検証は必ず
  `.app` に対して行うこと。**
- `dist/` を更新しても `cargo build` は再コンパイルしない (フロントの変更が Rust の
  再ビルド契機にならない)。フロントを変えたら `pnpm tauri build` を使うこと。
- `scripts/measure-footprint.sh` が使う responsible pid による束ね方は、バンドルされた
  `.app` では WebKit ヘルパーを正しく拾うが、裸のバイナリでは拾えない。これも `.app`
  に対して測る理由になる。

### マトリクス監査 (step-03, 実装後)

実装サブエージェントの報告ではなく、baseline からの差分を読んで判定した。

自動テストで覆われた行 — 呼び出し / 二重発火の抑止 / トグル / 閉じる / ホットキー衝突 (計 11 テスト、いずれも実行され成功)。

監査で 2 件の不足を見つけ、その場で埋めた:

- **「閉じる」のテストが実体を持っていなかった。** `assert!(!visibility_after(OverlayAction::Hide))` は恒真であり、Esc がトグルになっていないことを何も検証していなかった。Esc 経路を `escape_action()` として純粋関数に切り出し (`hide()` の実装経路に組み込み、死にコードにしない)、可視状態に関わらず Hide であること、および同じ状態で `toggle_action` と結果が分かれることを固定した。
- **「ホットキー衝突」に対応するテストが存在しなかった。** `register` の戻り値が `Result` ではなく `HotkeyStatus` であることが「登録失敗を理由に常駐を止めない」を型で保証している。失敗が値として表現され、利用者に示す情報 (アクセラレータ表記・理由) を保持することをテストで固定した。

**「再起動後」は自動テストで覆えない。** 実際の OS 再起動とログインを要するため、プロセス内から検証する手段がない。手動確認とする — 監査上は未充足のまま残る行であり、テストで覆ったふりはしない。

### 環境の制約により未検証のもの

ホットキー押下・Esc・フォーカス復帰・300ms の実測は、この環境にアクセシビリティ権限と画面収録権限がないため検証できていない。実装は正しく見えるが、動作は確認されていない。利用者による手動確認が必要。

## Spec Change Log

## Review Triage Log

### 第 1 回レビュー (blind-hunter / edge-case-hunter / verification-gap)

| # | 判定 | 所見と根拠 | 経路 |
| --- | --- | --- | --- |
| 1 | high | `lib.rs` に RunEvent ハンドラも `prevent_exit` もなく、`enable_macos_default_menu(false)` も呼んでいない (コードで確認)。macOS 既定メニューの Cmd+W / Cmd+Q が生きており、反射的な一打で常駐プロセスが終了する。次のログインまでホットキーが死ぬ。CAP-3「明示的な終了まで動作を継続する」に反する | intent_gap |
| 2 | high | 単一インスタンスの保証がない (`Cargo.toml`・`src/` に該当なしを確認)。LaunchAgent は `.app` ではなく実行ファイルを直接起動するため LaunchServices の単一化を迂回する。ログイン起動と `open -a` が併存し、後発はホットキー登録に失敗し、待機時フットプリントが倍になる。実測した 52.4MB は単一インスタンス時のみ有効 | bad_spec |
| 3 | high | `lib.rs:62` の `presentation::show(&handle)?` はホットキー登録失敗時のみ到達する。`show()`/`set_focus()` が Err を返すと setup が Err → `run()` が Err → `.expect()` で panic。「登録失敗を理由に常駐を止めない」を型で保証しておきながら 2 行後に `?` で手放している。`handle.plugin(log)?` と `set_activation_policy(..)?` も同じ形 | patch |
| 4 | high | `tauri.conf.json` の overlay に `fullScreenAuxiliary` 相当の collectionBehavior がない。他アプリがフルスクリーンの空間でホットキーを押すとオーバーレイが出ないか空間が切り替わる。CAP-1「いかなるアプリケーションが最前面にあっても」に反する。修正は objc2 経由の NSWindow 操作を要し些末ではない | bad_spec |
| 5 | medium | `presentation::apply` の Hide は `app.hide()` (NSApplication 全体) を呼ぶが Show は `window.show()` + `set_focus()` のみで `app.show()` を呼ばない。research-scaffold.md §6 が対になる API として記録している。2 回目以降の呼び出しで前面化・フォーカスが不確実。`toggle()` が `window.is_visible()` で分岐する点も同根 — NSApp 非表示中に stale な true を返すと押下が無反応に見える | patch |
| 6 | medium | Esc の配線 (`Overlay.svelte:40-46` の keydown → `invoke('hide_overlay')`) を検証するテストがリポジトリに存在しない。フロントエンドのテストハーネス自体がない (vitest/playwright なし、`package.json` に test スクリプトなし)。`event.key === 'Esc'` への書き換えや `void close()` の削除で Esc が死んでも 11 テストと svelte-check は通る | patch |
| 7 | medium | **本セッションで加えた「閉じる」テストの修正自体が恒真だった。** `escape_action()` は引数を取らない `const fn` で literal を返すため、`for visible in [true,false]` は同じ定数を 2 回検証しているだけ (`visible` は失敗メッセージにしか現れない)。`assert_ne!(escape_action(), toggle_action(false))` もコンパイル時定数同士の比較。恒真を長い恒真に置き換えた | patch |
| 8 | medium | ホットキー登録失敗時にオーバーレイを出す分岐 (`lib.rs:59-63`) を検証するテストがない。`a_failed_registration_is_a_value_not_an_error` は構造体を直接作って形を見るだけで、`register` も setup も通らない。条件を反転しても 11 テストは通る。Dock もメニューバーもないため、この分岐が唯一の伝達経路 | patch |
| 9 | medium | `Makefile:20` の `pkill -x "My Task Manager"` は決して一致しない。`-x` は完全一致で、実プロセス名は Cargo の `name` である `my-task-manager` (footprint 出力で確認済み)。`-@ ... \|\| true` が失敗を飲むため、稼働中のアプリに対して `rm -rf`/`cp -R` が走る | patch |
| 10 | medium | `measure-footprint.sh` の `phys_footprint()` が `subprocess.run` の終了コードを見ず、`OSError` 以外の失敗で 0 を返す。権限失敗や出力形式変更で過少計上し、予算超過を「OK」と表示する。進行を止めることが唯一の役目のスクリプトが fail-open している (AD-14) | patch |
| 11 | medium | Rust→TS の `OverlaySnapshot` 契約を検証するものが手書きの TS 型しかない。`invoke<T>` は実行時検査を行わないため、Rust 側のフィールド名変更で `snapshot.hotkey` が undefined になり、ホットキー失敗の警告が無言で出なくなる。`rename_all = "camelCase"` は現状すべて単語 1 つで不活性であり、複数語フィールドを足した瞬間に乖離する | patch |
| 12 | medium | `apply()`/`toggle()` はウィンドウが見つからないとき `Ok(false)` を返し、「ウィンドウが無い」と「隠れている」を同じ値に潰している。`hide_overlay` はこれを `Ok(())` にするため、フロントは閉じたと伝えられる | patch |
| 13 | medium | `commands::manage` は setup 内で呼ばれるが、webview がそれ以前に `get_overlay_snapshot` を呼ぶと state 未管理で reject される。`Overlay.svelte` 側に再試行がないため、スナップショットが永久に届かない | patch |
| 14 | medium | `Overlay.svelte` の `invoke('hide_overlay')` が reject した場合の代替経路がない。装飾なし・alwaysOnTop・全空間表示のウィンドウが閉じられなくなる | patch |
| 15 | maybe-false | `tauri.conf.json` の `backgroundThrottling: "throttle"` が、隠れた webview の復帰後の初回 invoke を遅らせ 300ms 予算を破る可能性。ただし 300ms 自体が一度も実測されていないため真偽を判定できない。判定に必要なもの: ホットキー押下から入力受付までの実測 | defer |
| 16 | medium | 自動起動の解除手段がない。リリースビルドは毎回 `is_enabled()` を見ずに `autolaunch().enable()` を呼ぶため、利用者がログイン項目から消しても次回ログインで復活する。`make clean` はビルド成果物しか消さず、`~/Library/LaunchAgents/my-task-manager.plist` と `/Applications` の `.app` を残す | bad_spec |
| 17 | low | `measure-footprint.sh -h` が `sed -n '2,25p'` でヘッダを超えて `set -euo pipefail` と変数代入まで help として表示する (ヘッダは 20 行目で終わる) | patch |
| 18 | low | `app_group()` の一次照合 `os.path.basename(comm) == APP_NAME` は死にコード。`comm` は `my-task-manager`、既定と Makefile が渡す値は `My Task Manager` のため常に部分一致の fallback に落ちる。fallback は無関係なプロセスを拾いうる | patch |
| 19 | low | `serde_json` が `Cargo.toml` にあり `src/` で未使用。「バージョンは厳密に固定する」のコメント下で実際に固定されているのは Tauri 3 crate のみで `log`/`serde`/`serde_json`/`tauri-plugin-log` は浮いている。`package.json` にも `packageManager` 指定がなく pnpm 版だけ固定されていない | patch |
| 20 | low | `rust-toolchain.toml` の `targets` が `aarch64-apple-darwin` のみ。Intel Mac ではターゲット不足でビルドが落ちる | patch |
| 21 | low | 仕様書の記述が実体と乖離。Verification 表は `cargo test` 9 passed だが実体は 11、カバレッジ表の `escape_always_hides` は実名 `escape_always_hides_regardless_of_visibility`、Commands 欄は無意味と自ら記録した `tsc --noEmit` を型検査として掲げたまま、タスク行は本仕様に存在しない Open Questions を参照している | patch |
| 22 | low | CI が存在しない。`make lint` / `make test` は人が思い出したときだけ走る | defer |

**経路の集計** — intent_gap 1 / bad_spec 3 / patch 15 / defer 2 (medium 1 は未検証として記録)。

intent_gap と bad_spec が存在するため、カスケード順に従いループバックとなる。patch 群はコードが再導出されるため本時点では適用しない。



## Verification

**Commands:**
- `pnpm tauri dev` -- expected: ビルドが通り、Dock にアイコンが出ずにアプリが常駐する
- `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` -- expected: 警告なし
- `pnpm exec tsc --noEmit` -- expected: 型エラーなし
- `make` -- expected: `/Applications` に `.app` が配置される
- `scripts/measure-footprint.sh` -- expected: 待機時メモリ 100MB 未満・CPU 1% 未満 (AD-14)

**Manual checks:**
- 1 押下でトグルが 1 回だけ起きること (2 回なら `ShortcutState` の判定漏れ)
- Esc 直後、キー入力が直前のアプリケーションに届くこと
- システム設定 > ログイン項目に登録されていること

### 実行結果 (2026-09-16, Darwin 25.6 / aarch64)

`RUSTUP_TOOLCHAIN` を外した状態で実行している (上記「toolchain の固定」参照)。

| コマンド | 結果 |
| --- | --- |
| `pnpm tauri dev` | 通る。`ApplicationType = UIElement` (Dock にアイコンなし)、ホットキー登録成功、`autostart is not registered in a development build` をログに記録、フロントが `get_overlay_snapshot` を呼べている |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` | 警告なし |
| `cargo test --manifest-path src-tauri/Cargo.toml` | 9 passed / 0 failed |
| `pnpm exec tsc --noEmit` | エラーなし。ただし**これは実質的に何も検査していない** — テンプレートのルート `tsconfig.json` は `files: []` + `references` の solution 形式であり、`tsc --noEmit` 単体では 0 ファイルを対象にする。`.svelte` を解決できる `pnpm check` (svelte-check) が実質的な型検査であり、101 ファイル / 0 errors / 0 warnings |
| `make` | `/Applications/My Task Manager.app` に配置され、そこから起動できる。`LSUIElement = true` がバンドルの `Info.plist` にマージされている |
| `scripts/measure-footprint.sh` | **メモリ 52.4 MB / 上限 100 MB、CPU 0.03% / 上限 1%** — いずれも予算内 (AD-14) |

待機時 footprint の内訳 (リリースビルド、起動後 2.5 分放置、60 秒観測):

```
   4581      18.2 MB   0.03%  my-task-manager
   4584       9.0 MB   0.00%  com.apple.WebKit.GPU
   4585       6.5 MB   0.00%  com.apple.WebKit.Networking
   4586      15.8 MB   0.00%  com.apple.WebKit.WebContent
   4589       2.8 MB   0.00%  com.apple.audio.SandboxHelper
     合計    52.4 MB   0.03%  (5 プロセス)
```

**AD-14 の資源予算はこれで初めて実測された。** WKWebView ヘルパーを含む全プロセス合計で
上限の約半分であり、Electron を排除した根拠は現時点では保たれている。介入パネル
(AD-6 / CAP-10) で常駐 webview が 2 つに増えた時点で再測定が要る。

その他に自動で確認したこと:

- `~/Library/LaunchAgents/my-task-manager.plist` が生成され、`RunAtLoad` で
  `/Applications/My Task Manager.app/Contents/MacOS/my-task-manager` を起動する
- ログは `~/Library/Logs/dev.onzuka.mytaskmanager/my-task-manager.log` にのみ出る
  (AD-12)
- バンドル済み `.app` において、CSP を設定した状態でフロントが起動し
  `get_overlay_snapshot` を呼べている (CSP が IPC を壊していない)

**残る手動確認** (この環境ではアクセシビリティ / 画面収録権限が無く自動化できない):

- ホットキー押下 → 300ms 以内にオーバーレイが最前面に出て入力を受け付ける
- 1 押下でトグルが 1 回だけ起きる (純粋関数としては自動テスト済み)
- Esc でオーバーレイが隠れ、直前に最前面だったアプリにフォーカスが戻る
- 他アプリと衝突させた場合に、常駐が継続し失敗がオーバーレイに提示される
- OS 再起動 → ログイン後に操作なしで常駐している
