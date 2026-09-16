---
title: '常駐の骨格 — 常駐プロセス・自動起動・ホットキー呼び出し'
type: 'feature'
created: '2026-09-15'
status: 'done'
route: 'dispatch'
review_loop_iteration: 1
baseline_commit: 'e90d03f80ae46b91d3c0ea077d33be77e291db91'
context:
  - '{project-root}/_bmad-output/specs/spec-my-task-manager/SPEC.md'
  - '{project-root}/_bmad-output/planning-artifacts/architecture/architecture-my-task-manager-2026-09-15/ARCHITECTURE-SPINE.md'
  - '{project-root}/_bmad-output/implementation-artifacts/research-scaffold.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** リポジトリにはコードが一行も存在しない。CAP-1 (ホットキー呼び出し) と CAP-3 (常駐と自動復帰) は他のすべての capability の土台であり、これが立たない限り何も検証できない。加えて AD-14 の資源予算 (待機時 CPU 1% 未満・メモリ 100MB 未満) は Electron を排除した根拠でありながら一度も実測されていない。第 1 試行はこの骨格を立てたが、常駐プロセスの**終了経路を定義していなかった**ため、反射的な Cmd+Q 一打で常駐が死に、次のログインまでホットキーが失われる状態にあった。

**Approach:** Tauri 2.11 + Svelte 5 + Vite で macOS 常駐アプリの骨格を作る。Dock には出ないが、**メニューバー項目を常在する可視面として持つ** — これが利用者にとって唯一の明示的な終了経路である。ログイン時に自動起動し、グローバルホットキーで事前生成済みの隠しウィンドウを表示する。ドメインロジックは持たない — オーバーレイは中身のない器である。`ports/` と `adapters/` のディレクトリ境界 (AD-1) をこの時点で確立し、以降の capability がその内側に積まれるようにする。

## Boundaries & Constraints

**Always:**
- ディレクトリ構成は ARCHITECTURE-SPINE.md の「ソースツリー」に従う。OS API は `adapters/` 配下にのみ置く (AD-1)。`domain/` と `ports/` は空で作成する。
- ホットキーは `Ctrl+Option+Space`。Spotlight とも入力ソース切替とも衝突しない。`Pressed` のみ処理する (AD-7) — 1 押下で 1 トグル。
- ウィンドウは起動時に生成して隠す。押下時に生成しない (300ms 制約)。**一度生成したら破棄しない** — 閉じる要求は破棄ではなく非表示に変換する。
- **終了はメニューバー項目からのみ到達できる。** macOS 既定メニューを無効化し、Cmd+W / Cmd+Q を効かなくする。既定メニューを消したうえでなお届く終了要求は、明示的な終了要求と区別して拒否する。
- **メニューバー項目は単色のテンプレートアイコンとする。** メニューの中身は「終了」と、ホットキーの現在状態を示す非活性の 1 行の 2 つだけ。単色アイコン素材を新たに 1 枚起こす。
- **ホットキーの登録に失敗したときは、起動時にオーバーレイを表示して理由を示す。** ホットキーが唯一の呼び出し経路である以上、それが死んでいることは確実に伝わらなければならない。メニューバー項目の状態行は、以後いつでも確認できる副の経路として持つ。
- **オーバーレイはフォーカスを失ったら隠れる。** Spotlight と同じ作法で、呼び出して使ったら消える一時的な面として扱う。常時表示に近づけない (AD-15)。
- **二重起動を許さない。** 後発のプロセスは自ら終了し、先発のプロセスが応答する。二重起動は AD-14 の実測を無効化する。
- **オーバーレイは他アプリのフルスクリーン空間でも最前面に出る。** CAP-1「いかなるアプリケーションが最前面にあっても」を満たすため。
- **自動起動は利用者が解除でき、解除が次のログインで覆らない。** 登録済みかを確認してから登録する。アンインストール手段をビルド経路と同じ場所に用意する。
- 自動起動は `MacosLauncher::LaunchAgent`。ログイン項目に `.app` ではなく内部の実行ファイル名で並ぶことは既知の代償として受け入れる。
- **`tauri dev` では自動起動を登録しない。** 開発ビルドのパスがログイン項目に残るため。
- 外部送信コードを含めない (AD-12)。ログはローカルファイルのみ。
- Rust は `rust-toolchain.toml` で固定し、利用者のグローバル既定を変更しない。
- **オーバーレイはダミーの「次の一手」を 1 件表示する。** 単一の定数に切り出し `// TODO(CAP-2): ドメインモデル導入時に削除` を付す。

**Never:**
- ドメインモデル・SQLite・介入パネルを実装しない (CAP-4/5/6/10 は別スライス)。
- メニューバー項目にバッジ・件数・進捗・タスク一覧を出さない (AD-15、SPEC.md 非目標「介入以外の通知・バッジ・音を持たない」「常時の一覧表示を持たない」)。
- `macos-private-api` を有効化しない。本スライスは透過を使わない。CAP-10 に先送り。
- SvelteKit を使わない。Tauri 3.x を使わない (`@latest` / `@next` / `3.0.0-alpha` を引かない)。

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| 呼び出し | 他アプリが最前面、オーバーレイ非表示 | オーバーレイが最前面に表示されフォーカスを得る | N/A |
| 二重発火の抑止 | ホットキーを 1 回押下 | トグルが 1 回だけ起きる (Released で 2 回目が起きない) | N/A |
| 閉じる | オーバーレイ表示中に Esc | オーバーレイが隠れ、直前に最前面だったアプリにフォーカスが戻る | N/A |
| トグル | オーバーレイ表示中にホットキー | オーバーレイが隠れる (Esc と同じ経路) | N/A |
| フォーカス離脱 | オーバーレイ表示中に他アプリをクリックする | オーバーレイが隠れる (Esc と同じ経路) | N/A |
| フルスクリーン空間 | 他アプリがフルスクリーンの空間でホットキー押下 | 空間を切り替えずに、その空間の最前面にオーバーレイが出る | N/A |
| ホットキー衝突 | 他アプリが同じキーを保持し登録に失敗 | 起動は継続し、オーバーレイを表示して理由を示す。メニューバー項目の状態行にも残る | 登録失敗を理由に常駐を止めない |
| 誤終了の抑止 | オーバーレイ表示中に Cmd+W / Cmd+Q | 何も起きない。常駐は継続し、ウィンドウも破棄されない | N/A |
| 明示的な終了 | メニューバー項目から「終了」を選ぶ | プロセスが終了する。次のログインでは自動起動する | N/A |
| 二重起動 | 常駐中に `.app` をもう一度起動する | 後発は起動せず終了する。常駐プロセスは 1 つのまま | N/A |
| 自動起動の解除 | 利用者が自動起動を解除した状態で再ログイン・再起動 | 自動起動しない。アプリを起動しても再登録されない | N/A |
| 再起動後 | OS 再起動、ログイン | 利用者の操作なしに常駐プロセスが動作している | N/A |
| ウィンドウが無い | 何らかの理由でオーバーレイを取得できない | 失敗として記録する。呼び出し側に成功を返さない | 「隠れている」と同じ値に潰さない |

</frozen-after-approval>

## Code Map

greenfield。HEAD に応用コードは一行も無い。再導出の参照元は下記。

- **`resident-shell-attempt-1` タグ (commit `a30a832`)** -- 第 1 試行の全ソース。`git show resident-shell-attempt-1:<path>` で読める。Spec Change Log の **KEEP** 項目はここから取る。**ただし Review Triage Log の欠陥をそのまま持ち込まないこと。**
- `_bmad-output/implementation-artifacts/research-scaffold.md` -- **実装前に必読。** 検証済みの scaffolding 手順、`Info.plist` の扱い、global-shortcut / autostart / activation policy の正確な API シグネチャ、footprint 計測の方式。docs.rs が誤っている箇所 (`event.state` はフィールドでありメソッドではない) を含む。**本スライスで新たに必要になる 4 領域 (メニュー・トレイ / 終了の阻止 / 単一インスタンス / フルスクリーン空間) は一切記載が無い** — それらは下記 Design Notes に検証済みの事実を置いた。
- `ARCHITECTURE-SPINE.md` の「ソースツリー」 -- ディレクトリ構成の正。
- `_bmad-output/specs/spec-my-task-manager/glossary.md:17` -- 「オーバーレイ」の定義。メニューバー項目の追加に伴い、可視面が 2 つになったことを追記する必要がある。なお第 1 回ループバックの申し送りは「唯一の可視面」が glossary にあると記していたが、**実際には glossary.md にも SPEC.md にも無く**、`prds/prd-my-task-manager-2026-09-14/prd.md:70` に残っているだけである。正典側の修正はこの追記 1 箇所で足りる。

## Tasks & Acceptance

**Execution:**
- [x] `rust-toolchain.toml` / `mise.toml` / `Makefile` -- 第 1 試行の 3 点セットをそのまま再導出する -- mise が `RUSTUP_TOOLCHAIN` を export すると rustup の優先順位で `rust-toolchain.toml` を上書きするため 3 箇所すべてが要る。グローバル既定は変更しない。`targets` に `x86_64-apple-darwin` を加える (所見 #20)
- [x] `package.json` ほかフロント足場 -- Vite の `svelte-ts` テンプレート。`packageManager` を明記する (所見 #19) -- `create-tauri-app` の Svelte テンプレートは SvelteKit であり採用しない
- [x] `src-tauri/Cargo.toml` -- 依存を厳密に固定する。`tauri` に `tray-icon` と `image-png` の feature を足す (既定に含まれない)。`tauri-plugin-single-instance` と `objc2` / `objc2-app-kit` を追加する。未使用の依存を置かない (所見 #19)
- [x] `src-tauri/Info.plist` -- `LSUIElement` を設定 -- Dock に出さない。`bundle.macOS.infoPlist` はパス文字列でありインラインの辞書ではない
- [x] `src-tauri/tauri.conf.json` -- ウィンドウを `visible: false`, `focus: false`, `alwaysOnTop: true`, `decorations: false`, `backgroundThrottling: "throttle"` で定義し、CSP を設定する
- [x] `src-tauri/src/lib.rs` -- 常駐の起点。単一インスタンスを最初に登録し、Dock 非表示化・既定メニュー無効化・ウィンドウ事前生成・各アダプタ登録を行う。**`setup` 内で `?` を使わない** (所見 #3 — 常駐を殺す)。`RunEvent` と `WindowEvent` のハンドラで誤終了・誤破棄を拒否する
- [x] `src-tauri/src/adapters/menubar/mod.rs` -- 単色テンプレートアイコンの tray、「終了」、ホットキー状態の非活性 1 行 -- 唯一の明示的終了経路 (CAP-3)。`PredefinedMenuItem::quit` は使わない (Design Notes 参照)
- [x] `src-tauri/src/adapters/hotkey/mod.rs` -- ホットキー登録と `Pressed` のみのトグル処理 -- AD-7。登録は `HotkeyStatus` を返し、失敗を値として表現する
- [x] `src-tauri/src/adapters/autostart/mod.rs` -- 登録済みかを確認してから登録する (所見 #16) -- 利用者の解除を次のログインで覆さない
- [x] `src-tauri/src/adapters/presentation/mod.rs` -- 表示・非表示・フォーカス復帰、およびフルスクリーン空間での可視化。`show()` と `hide()` は NSApplication 側と対で呼ぶ (所見 #5)。ウィンドウ不在を「隠れている」と潰さない (所見 #12)
- [x] `src-tauri/src/commands/mod.rs` / `src-tauri/capabilities/default.json` -- `hide_overlay` と `get_overlay_snapshot`、およびフロントが必要とする最小の権限 -- AD-3 (フロント → コアは Tauri command のみ) と鮮度規則。独自コマンドの invoke に権限は要らないが、ウィンドウのフォーカス変化を購読するには `core:event` 系が要る
- [x] `src-tauri/src/domain/mod.rs` / `ports/mod.rs` -- 空モジュールを作成 -- AD-1 の境界を先に確立する
- [x] `src/App.svelte` / `src/overlay/Overlay.svelte` -- 器としてのオーバーレイ。Esc で閉じ、フォーカスを失っても隠れる。スナップショット取得に再試行を持たせる (所見 #13)。閉じる経路が失敗したときの代替を持つ (所見 #14)
- [x] `Makefile` -- ビルドから `/Applications` への配置までを 1 タスクに (AD-13)。`uninstall` を追加する (所見 #16)。プロセス名は `my-task-manager` であり `My Task Manager` では `pkill -x` が一致しない (所見 #9、実測で確認済み)
- [x] `scripts/measure-footprint.sh` -- WKWebView ヘルパーを含む全プロセスの実測 (AD-14)。**計測失敗時に 0 を返して fail-open しないこと** (所見 #10)。help の範囲と `app_group` の死にコードを直す (所見 #17 #18)
- [x] `src-tauri/icons/menubar-template.png` ほか -- メニューバー用の単色テンプレートアイコンを 1 枚用意する -- 多色のアプリアイコンをテンプレート指定すると黒い塊に潰れる
- [x] `.gitignore` -- `node_modules/`, `src-tauri/target/`, `dist/` を追記 -- revert でこの追記も消えている
- [x] `src-tauri/src/adapters/hotkey/mod.rs` / `adapters/presentation/mod.rs` -- 判断を OS 呼び出しから切り離した純粋関数として抽出し `#[cfg(test)]` で単体テストする -- I/O マトリクスのうち OS を起動せず検証できる行を自動テストで覆うため。**恒真なテストを書かないこと** (所見 #7)。OS・ログイン・実時間を伴う行は手動確認とし、その旨を Implementation Notes に記録する
- [x] `_bmad-output/specs/spec-my-task-manager/glossary.md` -- 「オーバーレイ」の項に、メニューバー項目という second の可視面が存在することを追記する -- 上流を直さないと次に読む者が同じ前提で設計する

**Acceptance Criteria:**
- Given OS を再起動しログインした直後、when 利用者が何も操作しない、then 常駐プロセスが動作しており Dock にアイコンが出ていない
- Given 任意のアプリケーションが最前面、when グローバルホットキーを押下する、then 300ms 以内にオーバーレイが入力を受け付ける状態で最前面に出る
- Given オーバーレイが表示されている、when Esc を押す、then オーバーレイが隠れ、直前に最前面だったアプリケーションがフォーカスを回復する
- Given 待機状態 (オーバーレイ非表示)、when `scripts/measure-footprint.sh` を実行する、then WKWebView ヘルパーを含む全プロセス合計のメモリが 100MB 未満、CPU が 1% 未満である
- Given `make` を実行した、when 完了した、then `/Applications` に `.app` が配置され、そこから起動できる
- Given 常駐して自動起動が登録されている、when `make uninstall` を実行する、then プロセスが終了し、`.app` と LaunchAgent の plist が両方消え、再ログインしても復活しない

## Implementation Notes

### 何がどこに立ったか

```text
my-task-manager/
  index.html  vite.config.ts  svelte.config.js  tsconfig*.json  package.json
  Makefile                      # install / stop / uninstall / measure (AD-13)
  mise.toml  rust-toolchain.toml
  scripts/measure-footprint.sh  # AD-14 の実測
  src/
    main.ts  app.css  App.svelte
    overlay/Overlay.svelte      # 器としてのオーバーレイ
  src-tauri/
    Info.plist                  # LSUIElement
    tauri.conf.json             # visible:false / focus:false / alwaysOnTop / decorations:false
    capabilities/default.json   # core:event(listen/unlisten) + core:window:allow-hide のみ
    icons/menubar-template.png  # 単色テンプレート 36x36 (18pt @2x)
    src/
      lib.rs                    # 常駐の起点 + 誤終了阻止の 3 層
      domain/mod.rs  ports/mod.rs   # 空。AD-1 の境界を先に確立
      adapters/{hotkey,autostart,presentation,menubar}/mod.rs
      commands/mod.rs           # AD-3 の境界
```

### 再導出にあたって決めたこと (第 1 試行からの差分)

1. **誤終了の阻止を 3 層で実装した。** `enable_macos_default_menu(false)` /
   `WindowEvent::CloseRequested` での `prevent_close` + 非表示化 /
   `RunEvent::ExitRequested` で `code.is_none()` のときだけ `prevent_exit`。
   第 3 層の判断は `should_prevent_exit(Option<i32>) -> bool` として lib.rs に切り出し、
   「暗黙は拒む / `exit(0)` は通す」を単体テストで固定した。終了項目は独自 `MenuItem`
   であり `PredefinedMenuItem::quit` ではない。
2. **可視状態の所有者を提示アダプタに移した (所見 #5 の後半)。** トグルの分岐に
   `WebviewWindow::is_visible()` を使わず、`presentation` 内の `AtomicBool` を正とする。
   表示・非表示はすべて `presentation::apply` を通るため、この値が実体と乖離しない。
   NSApplication ごと隠した状態での `is_visible()` の戻り値に依存しなくなった。
   `Show` では `app.show()` → `window.show()` → `set_focus()` と NSApplication 側を対で
   呼ぶ。
3. **`escape_action()` を `close_action(is_visible) -> Option<OverlayAction>` に改めた
   (所見 #7)。** 引数を取らない定数関数では何を書いても恒真になる。可視状態を入力に取り、
   隠れているときは「行うべき操作が無い」を `None` として返す形にしたため、
   「閉じる要求はトグルではない」「2 回続けても反転しない」が実際に検証できる。
4. **提示の失敗を型で分けた (所見 #12)。** `PresentationError::WindowMissing` と
   `::Window`。`hide()` は既に隠れているときでもウィンドウの存在だけは確かめ、
   不在なら `Err` を返す。`hide_overlay` コマンドはこれをそのまま `Err` としてフロントへ
   返し、フロントは代替経路 (`getCurrentWindow().hide()`) に倒れる (所見 #14)。
5. **起動時の状態を `Builder::manage` で預けた (所見 #13)。** state 自体は起動前から
   存在し、中身が未確定であることを `Err` として表現する。フロントは 100ms 間隔で
   最大 10 回再試行する。「未確定」を既定値で埋めないため、ホットキーの登録失敗が
   成功として描画されることがない。
6. **Rust → TS の配線を JSON の形として固定した (所見 #11)。**
   `commands::tests::the_snapshot_keeps_its_wire_contract` が実際の直列化結果の
   フィールド名と型を見る。`serde_json` はそのための **dev-dependency** であり、
   本体の依存には置いていない (所見 #19)。
7. **`objc2` を直接依存に置かなかった。** 必要なのは `objc2-app-kit` の `NSWindow` /
   `NSWindowCollectionBehavior` だけである。未使用の依存を置かない (所見 #19) を優先した。
   feature は `NSWindow` と `NSResponder` の両方。
8. **フルスクリーン空間は二重化した。** `tauri.conf.json` の
   `visibleOnAllWorkspaces: true` を基礎として残し、その上で
   `presentation::allow_fullscreen_spaces` が `CanJoinAllSpaces | FullScreenAuxiliary`
   を `NSWindow` に直接立てる。objc2 の呼び出しが失敗しても全空間表示だけは残る。
9. **フォーカス離脱による非表示をフロント側に置いた。** I/O マトリクスが
   「Esc と同じ経路」を要求しているため、`onFocusChanged(false)` は Esc と同一の
   `hide_overlay` コマンドを呼ぶ。Rust 側に二重の経路を作らない。
10. **`make stop` を分離し、終了を確認できなければ失敗させた (所見 #9)。**
    `pkill -x` の対象はプロセス名 `my-task-manager`。`install` も `uninstall` も
    このターゲットを経由するため、稼働中のまま `rm -rf` / `cp -R` が走らない。
11. **`measure-footprint.sh` の fail-open を塞いだ (所見 #10)。** `footprint(1)` の
    終了コードと出力を検査し、読めなければ 0 ではなく例外として exit 4 で止まる。
    `-h` はヘッダ末尾の標識行を探して範囲を決めるため、ヘッダを書き足しても
    `set -euo pipefail` まで表示しない (所見 #17)。`app_group` の照合は
    プロセス名の完全一致のみとし、無関係なプロセスを拾う部分一致 fallback を消した
    (所見 #18)。`-a` の既定値も `my-task-manager` に改めたため、一次照合が死なない。

### 仕様の要求を満たすために追加した状態 — 要確認

**自動起動の「一度きり」の印。** I/O マトリクスの「自動起動の解除」行は
「アプリを起動しても再登録されない」を要求している。`auto-launch` の `is_enabled()` は
plist ファイルの存在を見るだけで、「まだ一度も登録していない」と「利用者が解除した」を
区別できない。区別する材料が他に無いため、
`~/Library/Application Support/dev.onzuka.mytaskmanager/autostart-registered` という
空ファイルを印として置いた。判断そのものは
`autostart::should_register(already_attempted, currently_enabled)` という純粋関数であり、
「印があって登録されていない = 利用者が解除した」を単体テストで固定してある。
`make uninstall` はこの印も消す。

[ASSUMPTION: これは AD-11「別建ての設定ファイルを持たない」と緊張する。永続化層が
入る次のスライスで設定テーブルへ移すべきであり、コードにも `TODO(AD-11)` を残した。
本スライスに SQLite を持ち込まないという境界のほうを優先した。]

### 自動テストで覆った行と、手動確認に残した行

`cargo test` **30 件** と `pnpm test` (vitest) **5 件**。I/O マトリクスのうち **OS を起動せずに検証できる判断**を覆っている。

| I/O マトリクスの行 | 覆っているテスト |
| --- | --- |
| 呼び出し | `presentation::hidden_overlay_is_shown` / `hotkey::one_keypress_shows_a_hidden_overlay` |
| 二重発火の抑止 | `hotkey::released_does_not_toggle` / `two_keypresses_return_to_the_original_state` |
| 閉じる | `presentation::a_close_request_hides_only_a_visible_overlay` / `a_close_request_never_opens_the_overlay`、および `Overlay.test.ts` の Esc テスト (フロントの分岐を実行する) |
| トグル | `presentation::visible_overlay_is_hidden` / `hotkey::one_keypress_hides_a_visible_overlay` |
| フォーカス離脱 | 上と同じ経路 (`close_action`)。加えて `Overlay.test.ts` が blur で `hide_overlay` が呼ばれ、focus では呼ばれないことを実行して検証する |
| フルスクリーン空間 | `presentation::the_overlay_joins_other_apps_full_screen_spaces` |
| ホットキー衝突 | `hotkey::a_failed_registration_is_a_value_not_an_error` / `menubar::the_status_line_distinguishes_failure_from_success` |
| 誤終了の抑止 | `should_prevent_exit` の `an_implicit_exit_is_refused` (第 3 層のみ) |
| 明示的な終了 | `should_prevent_exit` の `an_explicit_exit_passes_through` |
| 自動起動の解除 | `autostart::a_user_removal_is_not_undone_by_a_later_launch` |
| ウィンドウが無い | `presentation::a_missing_window_is_never_reported_as_hidden` / `a_present_window_follows_the_visibility` |

**自動テストで覆えない行** — 二重起動 (プラグイン内部の Unix ドメインソケットが担うため
プロセス内から検証できない) と、再起動後 (実際の OS 再起動とログインを要する)。どちらも
実機での確認に残す。**テストで覆ったふりはしない。**

### マトリクス監査 (step-03、実装後)

実装サブエージェントの報告ではなく、baseline `e90d03f` からの差分を読んで判定した。
`cargo test` / `cargo clippy --all-targets -D warnings` / `cargo fmt --check` /
`pnpm check` は本セッションで再実行し、結果を自分で確認している。

監査で 2 件の不足を見つけ、その場で埋めた。

- **「フルスクリーン空間」に対応するテストが無かった。** collection behavior の組み立てを
  `overlay_collection_behavior()` として純粋関数に切り出し (`allow_fullscreen_spaces` の
  実装経路に組み込み、死にコードにしない)、`FullScreenAuxiliary` と `CanJoinAllSpaces` の
  両方が立つことを固定した。
- **「ウィンドウが無い」が型の分離だけで、テストを持っていなかった。** 判断を
  `hide_outcome(is_visible, window_present) -> HideOutcome` に切り出し、**隠れている状態で
  ウィンドウが不在になった場合**を `AlreadyHidden` と同一視しないことを固定した。両者とも
  「見えない」ため最も紛れやすい組み合わせである。

**2 件とも変異試験で検証した** — 第 1 回レビューの所見 #7 が恒真テストであったため、
新しいテストが実際に荷重を負っていることを確かめる必要があった。`FullScreenAuxiliary` を
取り除くと `the_overlay_joins_other_apps_full_screen_spaces` が落ち、`WindowMissing` を
`AlreadyHidden` に潰すと `a_missing_window_is_never_reported_as_hidden` が落ちる。いずれも
守るべき欠陥そのもので失敗する。

### 本セッションで実機に確認したこと

リリースビルド (`/Applications/My Task Manager.app`) に対して行った。

- `LSUIElement` が `true` でバンドルされ、Dock にアイコンが出ない
- メニューバーに単色テンプレートアイコンが出る (スクリーンショットで目視)
- オーバーレイが最前面に出て「次の一手」のダミーと Esc / ホットキー / 終了経路の
  案内を描画する
- グローバルホットキー押下で **1 回だけ** トグルが起きる (ログに `overlay Show applied`
  が 1 件。`Released` による 2 件目が無い)
- フォーカスを失うと `hide_overlay` 経由で隠れる
  (ログ: `the overlay asked to be closed` → `overlay Hide applied`)
- 内部の実行ファイルを直接起動しても 2 つ目のプロセスが立たず、先発が
  `a second launch was detected` を記録してオーバーレイで応答する
- 自動起動が `~/Library/LaunchAgents/my-task-manager.plist` に登録され、印が書かれる。
  2 回目以降の起動は `autostart is left as is (already_attempted=true, enabled=true)` と
  記録して登録し直さない
- `make uninstall` でプロセス・plist・`.app`・印がすべて消える (第 1 試行の残骸の
  掃除に実際に使った)

### 第 2 回レビュー後の patch 適用 (step-04)

20 件の修正を適用した。ループバックは発生していない — コードは再導出ではなく修正されている。
主なものは次のとおり。

- **終了経路が失われる状態に入らない (#47)。** `menubar::install` が失敗したら
  `handle.exit(1)` する。既定メニューを消し暗黙の終了も拒んだ以上、メニューバー項目が
  立たなければどの UI からも終了できない常駐が毎ログイン復活する。起動を止めるほうが良い。
- **嘘の状態を表示しない (#48)。** 起動時にオーバーレイ窓が無ければホットキーを登録せず
  `HotkeyStatus::failed` にする。「有効」と表示しながら押下が無反応になる状態を作らない。
- **可視状態の遷移を直列化した (#50)。** `static TRANSITION: Mutex<()>` を置き、状態を
  変えうる経路をすべて通す。AD-5「状態変更はコア内の単一直列化経路に通す」への適合。
- **記録を実体に合わせる時点を早めた (#24/#49)。** `window.show()` / `hide()` が成功した
  直後に記録し、フォーカスと NSApplication の扱いは best-effort に降格した。
- **フロントの代替経路が Rust の記録を戻す (#23/#40/#51)。** `mark_overlay_hidden`
  コマンドを追加し、`getCurrentWindow().hide()` で隠した場合も可視状態を同期する。
- **自動起動の印が 3 状態を取り違えない (#45/#46)。** 「plist あり・印なし」でも印を書き、
  登録に**成功したときだけ**印を残す。失敗したまま印を残して二度と再試行しない状態を無くした。
- **判断がデバッグビルドでもコンパイルされる (#37)。** `#[cfg(not(debug_assertions))]` を
  OS を叩く `register_with_os` / `write_marker` の内側へ押し込んだ。判断の筋道は
  `cargo test` と `clippy` の対象になった。

### フロントのテスト基盤を入れた (#33/#36)

vitest + `@tauri-apps/api/mocks` の `mockIPC` を導入した (`src/overlay/Overlay.test.ts`、
5 件)。I/O マトリクスの「閉じる」「フォーカス離脱」は、これまで Rust の純粋関数しか
見ておらず、**フロントの分岐を反転しても全テストが通る**状態だった。

**変異試験で荷重を確認した** — `Overlay.svelte` の `if (focused)` を反転させると
「フォーカスを失うと閉じる経路に入る」と「フォーカスを得ても閉じない」の 2 件が落ちる。
レビューが指摘した「反転しても検出されない」は解消している。

### 残した診断ログ

`presentation::apply` と `hide_overlay` に `INFO` のログを残した。表示・非表示の経路が
どこから来たかを事後に追えないと、フォーカス挙動の不具合が再現待ちになるためである。
ユーザー内容は書かない (一貫性の規約) し、ローカルファイルのみ (AD-12)。


## Spec Change Log

### 第 1 回ループバック (intent_gap 1 / bad_spec 3)

**引き金** — レビュー所見 #1 (intent_gap): 凍結ブロックが常駐プロセスの終了経路を一切定義していなかった。macOS 既定メニューが生きているため Cmd+W / Cmd+Q で常駐が事故的に終了し、次のログインまでホットキーが死ぬ。CAP-3「明示的な終了まで動作を継続する」に反する。加えて bad_spec 3 件 (#2 単一インスタンス / #4 フルスクリーン空間での可視性 / #16 自動起動の解除手段) がいずれも lib.rs・tauri.conf.json・adapters の中核に触れるため、継ぎはぎではなく再導出とする。

**人間の決定** — 終了経路はメニューバー項目とする。Cmd+W / Cmd+Q は無効化する。

**回避される既知の不良状態** — 反射的な一打で常駐が死ぬ状態。二重起動でフットプリントが倍になり AD-14 の実測が無効化される状態。フルスクリーン作業中にホットキーが無反応になる状態。ログイン項目から消しても次回ログインで復活する状態。

**上流への波及** — `glossary.md` の「オーバーレイ — 常駐プロセスの唯一の可視面」はメニューバー項目の追加により誤りとなる。SPEC.md と glossary.md の更新を要する。

### KEEP — 再導出で必ず残すもの

第 1 試行は `resident-shell-attempt-1` タグ (commit a30a832) に保全されている。以下は検証済みであり、再導出時に同等以上を維持すること。

1. **`scripts/measure-footprint.sh` の測定方式。** `responsibility_get_pid_responsible_for_pid` で WKWebView ヘルパーを束ね、RSS ではなく `phys_footprint` を使う。AD-14 が要求する「ヘルパーを除外した値を根拠にしない」を実際に満たす唯一の実装。ただし所見 #10 (footprint 失敗時に 0 を返し fail-open する) と #17 #18 は修正すること。
2. **判断を純粋関数に切り出す形。** `should_toggle` / `toggle_action` / `visibility_after` により、OS を起動せずにトグル規則を検証できる。ただし所見 #7 のとおり `escape_action()` は引数を取らないため恒真である — Esc の判断は可視状態を入力に取る形へ改めること。
3. **`hotkey::register` が `Result` ではなく `HotkeyStatus` を返す設計。** 「登録失敗を理由に常駐を止めない」を型で保証する。ただし所見 #3 のとおり `lib.rs` 側が `?` で手放しているため、setup 内では `?` を使わないこと。
4. **`rust-toolchain.toml` + `mise.toml` + Makefile の `unexport RUSTUP_TOOLCHAIN`。** mise が `RUSTUP_TOOLCHAIN` を export すると rustup の優先順位で `rust-toolchain.toml` を上書きするため、3 箇所すべてが必要。グローバル既定は変更しない。
5. **`bundle.macOS.infoPlist` ではなく `src-tauri/Info.plist` を置いて自動マージさせる形。** インラインの辞書は Tauri 2 では無効。
6. **`macos-private-api` を有効化していないこと。** 本スライスは `transparent` を必要としない。CAP-10 まで先送りしたまま維持する。
7. **実測値 52.4MB / 0.05% (単一インスタンス時)。** 再導出後に再測定し、メニューバー項目の追加による増分を確認すること。


### 第 2 回計画 (ループバック後の再計画、2026-09-16)

**申し送りの消化** — 第 1 回ループバックが残した申し送りブロックは本計画で消化し、削除した。人間の決定 (終了経路はメニューバー項目、Cmd+W / Cmd+Q は無効化) を凍結ブロックに書き直し、bad_spec 3 件 (#2 単一インスタンス / #4 フルスクリーン空間 / #16 自動起動の解除) を制約と I/O マトリクスの行として明示した。

**申し送りの誤りを訂正** — 申し送りは `glossary.md` の「オーバーレイ — 常駐プロセスの唯一の可視面」が誤りになると記していたが、その文言は `glossary.md` にも `SPEC.md` にも存在しない。`prds/prd-my-task-manager-2026-09-14/prd.md:70` に残っているだけである。正典側に必要なのは glossary への追記 1 箇所。

**調査で新たに確定した事実** — Design Notes に記録した。research-scaffold.md はメニュー・トレイ・終了阻止・単一インスタンス・フルスクリーン空間のいずれも扱っていないため、crate のソースを直接読んで確定させた。特に **所見 #4 は実地に裏付けられた** — `tao-0.35.3` の `set_visible_on_all_workspaces` は `CanJoinAllSpaces` しか立てず、`FullScreenAuxiliary` は tao のどこにも現れない。`tauri.conf.json` にも該当設定は存在しない。

**所見 #1 の理解を修正** — 申し送りは `prevent_exit` で誤終了を防ぐとしていたが、`PredefinedMenuItem::quit` と既定メニューの Cmd+Q はいずれも `NSApplication terminate:` を送り、`RunEvent::ExitRequested` を**迂回する**。`prevent_exit` だけでは Cmd+Q を止められない。防御は 3 層に分ける (Design Notes 参照)。

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

### 第 2 回レビュー (blind-hunter / edge-case-hunter / verification-gap)

3 層で 32 件。判定は差分と実コードを読んで下した (レビュアーの付けた severity は採用しない)。

| # | 判定 | 所見と根拠 | 経路 |
| --- | --- | --- | --- |
| 23 | medium | `Overlay.svelte` の代替経路 `getCurrentWindow().hide()` は `presentation::apply` を通らないため `OVERLAY_VISIBLE` が `true` のまま残る。次のホットキー押下が `toggle_action(true) = Hide` となり無反応に見える。コマンド失敗が前提であり、かつ次の Hide で自己修復するため high ではない | patch |
| 24 | medium | `presentation::apply` は全 OS 呼び出しの成功後にしか `store` しない。`window.show()` 成功・`set_focus()` 失敗で記録だけが取り残される (`hide()`+`app.hide()` も同様) | patch |
| 25 | low | ホットキー登録失敗後に再登録する経路が無く、衝突アプリを終了させてもプロセス再起動を要する。凍結ブロックは「継続し失敗を示す」までしか求めておらず、再起動という通常の回復手段が残る。修正は新規メニュー項目と再登録経路を要し直接的な訂正を超える | reject |
| 26 | low | メニューバーの状態行は `install` 時に一度書かれるだけで更新経路が無い。ただし #25 を採らない限り更新すべき事象が発生しない | reject |
| 27 | low | `objc2-app-kit` だけがキャレット範囲で、同ファイルの「すべて `=` で厳密に固定する」方針と Design Notes の「ずらすと crate が二重になりリンクが壊れる」に反する。修正は直接的な訂正 | patch |
| 28 | low | `make test` / `make lint` が `deps` に依存せず、`pnpm check` が `node_modules` 無しの新規 clone で失敗する | patch |
| 29 | low | `cargo fmt --check` を仕様の検証表が掲げ `rustfmt` を導入しているのに `make lint` が実行しない。セッションの記録にしか存在しない検査は検査ではない | patch |
| 30 | low | CI が存在しない。**第 1 回 #22 と同一の主張であり、コードも当時のまま。carried** — 判定と経路を引き継ぐ | defer (carried) |
| 31 | medium | `measure-footprint.sh` が私有 SPI `responsibility_get_pid_responsible_for_pid` を存在確認なしに解決し、束ね方が壊れても WebKit ヘルパーを黙って落として過少計上する。進行を止めることが唯一の役目のスクリプトで fail-open が残っている (AD-14) | patch |
| 32 | low | `ACCELERATOR` の表示文字列と `overlay_shortcut()` の実バインドを結ぶものが無く、既存テストは `ACCELERATOR` を自分自身と比べているだけ。変更すると利用者に嘘を表示する | patch |
| 33 | medium | フロントのテストが 0 件。Esc ハンドラ・フォーカス離脱・再試行・代替経路のいずれも実行されない。#36 と同一の欠落 | patch |
| 34 | low | `refresh()` に多重起動の防止も打ち切りも無い。ただし代入は冪等で実害が無く、修正は世代カウンタ等の複雑さを足す | reject |
| 35 | low | `make uninstall` がログディレクトリとアプリデータディレクトリを残す。残骸の掃除のために存在する経路としては不足。**同所見が付記した frontmatter の指摘は false** — `review_loop_iteration: 1` はループバック 1 回後として正しく、`status` は差分作成後に `in-review` へ進めている | patch |
| 36 | medium | (verification-gap、検証済みで受理) I/O マトリクス「閉じる」「フォーカス離脱」の実体は `Overlay.svelte` の分岐にあるが、対応表が挙げるテストは Rust の `close_action` を呼ぶだけでフロントを実行しない。`if (focused)` を反転しても 29 件は全件成功する | patch |
| 37 | medium | (verification-gap、検証済みで受理) 自動起動の実登録ブロックは `#[cfg(not(debug_assertions))]` であり `cargo test` / `cargo clippy` でコンパイルすらされない。`should_register` の適用を削除しても全テストが通り、lint もかからない | patch |
| 38 | medium | (verification-gap、検証済みで受理) `the_launch_agent_name_matches_the_process_name` は doc で「Makefile と揃っていなければ」と述べながら本体はリテラル比較のみで Makefile を読まない。改名すると `make uninstall` が消し損ねる | patch |
| 39 | medium | (verification-gap、検証済みで受理) 誤終了阻止の第 1 層 (`enable_macos_default_menu(false)`) と第 2 層 (`prevent_close`) に自動検証が無い。1 行消しても全テストが通る。AppKit を起動した実アプリでしか観測できず、本リポジトリは UI 自動化基盤を持たない | defer |
| 40 | medium | (verification-gap の Other) #23 と同一の主張。独立した 2 層が同じ欠陥に到達した | patch |
| 41 | medium | (verification-gap の Other) `measure-footprint.sh` は観測窓の両端に居たプロセスしか CPU を差分計算せず、途中で生まれたプロセスを 0% として合計する。#10 で塞いだ fail-open が CPU 側に残っている | patch |
| 42 | low | `Makefile` の `install` が成果物の存在を確かめる前に既存 `.app` を削除する。`productName` 変更等で `BUNDLE` が変わると、消しただけで置けず常駐が消える | patch |
| 43 | low | `measure-footprint.sh` の `-d` が非数値・負値・0 を検証しない。`elapsed≈0` で CPU% が発散しうる | patch |
| 44 | medium | #41 と同一。edge-case 層が独立に到達 | patch |
| 45 | medium | `currently_enabled=true` の早期 return が印を書かずに抜ける。印の書き込みが失敗した後など「plist あり・印なし」の状態から利用者が解除すると、次回起動で `should_register(false,false)=true` となり**解除が覆る** — 所見 #16 が塞いだはずの欠陥が別経路で復活する | patch |
| 46 | medium | `enable()` が失敗しても印を書くため、自動起動は二度と試行されず、失敗を利用者に示す面も無い。CAP-3「ログイン時に自動起動し」が静かに破れたまま固定される | patch |
| 47 | high | `menubar::install` の失敗を log のみで飲む。既定メニュー無効化と暗黙終了の拒否と重なり、**UI から終了できない常駐**が残り、しかも自動起動で毎ログイン復活する。CAP-3「明示的な終了まで」の反転。凍結ブロックが既定メニューを禁じている以上、可能な読みは「その状態に入らない」の一つに定まるため intent_gap ではなく patch とする | patch |
| 48 | medium | 起動時にオーバーレイ窓が無くても log のみで続行する。ホットキーは登録に成功し、メニューバーは「有効」と表示するが、押しても `WindowMissing` がログに出るだけで何も起きない。利用者に嘘を表示し続ける | patch |
| 49 | medium | #24 と同一。edge-case 層が独立に到達 | patch |
| 50 | medium | `toggle` の `is_visible()` 読み → `store` が非アトミックで、ホットキーコールバック (メインスレッド外) と IPC スレッドが競合しうる。**AD-5「状態変更はコア内の単一直列化経路に通す」への直接の逸脱**である | patch |
| 51 | medium | #23 と同一。edge-case 層が独立に到達 | patch |
| 52 | medium | 再試行 10 回を使い切ると `hotkey` が `null` のままで警告ブロックが描画されない。**登録失敗を伝えるために出した起動時オーバーレイが空白で出る** — 利用者が選んだ伝達手段そのものが機能しない | patch |
| 53 | low | #25 と同一 | reject |
| 54 | low | #27 と同一 | patch |

**経路の集計** — high 1 / medium 17 / low 12 (うち reject 4) / false 0。intent_gap 0・bad_spec 0 のため**ループバックは発生しない**。patch 26 件 (重複を畳むと 20 件の修正)・defer 2 件・reject 4 件。

**intent_gap に振らなかった理由 (#47)** — 「メニューバー項目が作れなかったとき何をするか」は凍結ブロックが定めていない。しかし凍結ブロックは既定メニューの無効化を命じており、「既定メニューを戻して Cmd+Q を復活させる」という対抗案はそれ自体が凍結ブロック違反となる。残る読みは「終了できない常駐にならないよう起動を止める」の一つに定まる。読みが一つに定まる以上、intent を推定してよい (step-04 の規定)。




## Design Notes

### 検証済みの API 事実 — 本スライスで新たに必要になった 4 領域

research-scaffold.md はこの 4 領域を扱っていない。以下は crate のソースを直接読んで確定させたもので、docs や記憶ではない。

**1. 誤終了の阻止は 3 層に分ける。**
`Builder::enable_macos_default_menu(bool)` は既定 `true`。`Builder::menu()` を設定していない macOS ビルドでは `Menu::default()` が自動で組み込まれ、そこに quit (Cmd+Q) と Window サブメニュー (Cmd+W) が入る。activation policy とは無関係で、`Accessory` でも入る。
`RunEvent::ExitRequested { code: Option<i32>, api: ExitRequestApi }` が発火するのは 2 箇所だけ — 最後のウィンドウが破棄されてウィンドウ集合が空になったとき (`code: None`) と、`AppHandle::exit` / `restart` が呼ばれたとき (`code: Some(..)`)。
一方 `PredefinedMenuItem::quit` と既定メニューの Cmd+Q は `NSApplication terminate:` を直接送る。tao には `applicationShouldTerminate` ハンドラが存在しないため、これらは `ExitRequested` を**通らない**。したがって:

- 第 1 層 — `enable_macos_default_menu(false)`。Cmd+Q / Cmd+W の出所そのものを消す。**これが Cmd+Q に対する唯一有効な防御である。**
- 第 2 層 — `WindowEvent::CloseRequested { api }` で `api.prevent_close()` し、破棄の代わりに非表示にする。ウィンドウを生かし続けることは 300ms 制約の前提でもある。
- 第 3 層 — `RunEvent::ExitRequested { code, api }` で `code.is_none()` のときだけ `api.prevent_exit()`。暗黙の終了だけを拒み、メニューバー項目からの `AppHandle::exit(0)` (`code: Some(0)`) は通す。フラグは要らない。

**終了項目に `PredefinedMenuItem::quit` を使ってはならない。** `terminate:` を送って上記をすべて迂回する。独自 `MenuItem` + `on_menu_event` から `app.exit(0)` を呼ぶこと。

**2. メニューバー項目は tray である。**
`LSUIElement` を立てたアプリは自前のメニューバーを表示しないため、「メニューバー項目」は `NSStatusItem` = Tauri の tray を指す。
`tauri::tray::TrayIconBuilder`: `.menu(&menu)` (`with_menu` ではない)、`.icon(Image)`、`.icon_as_template(bool)`、`.tooltip(s)`、`.show_menu_on_left_click(bool)` (既定 `true`)、`.on_menu_event(|&AppHandle, MenuEvent|)`、`.build(&manager)`。
**feature が要る** — `tauri = { features = ["tray-icon", "image-png"] }`。既定 feature は `["wry","compression","common-controls-v6","dynamic-acl","x11","dbus"]` のみで、どちらも含まれない。`Image::from_path` / `from_bytes` は `image-png` か `image-ico` が無いと存在しない。
メニュー項目は `MenuItem::with_id(app, "quit", "終了", true, None::<&str>)` のようにアクセラレータ無しで作る。`MenuEvent` は `pub struct MenuEvent { pub id: MenuId }`。

**3. 単一インスタンスは Unix ドメインソケット。**
`tauri-plugin-single-instance` は 2.x の最新が **2.4.4** (3.0.0-alpha が存在するため厳密固定が要る)。
`init<R, F: FnMut(&AppHandle<R>, Vec<String>, String) + Send + Sync + 'static>(f: F) -> TauriPlugin<R>` — 引数は `(app, argv, cwd)`。
macOS では `/tmp/{identifier の . と - を _ に置換}_si.sock` を使う。本アプリでは `/tmp/dev_onzuka_mytaskmanager_si.sock`。後発プロセスはプラグインの `setup` 内で `std::process::exit(0)` する。
**このプラグインは最初に登録しなければならない** (README が明記)。プラグインは登録順に setup が走るため、先に登録したものは終了する運命の後発プロセスでも setup を実行してしまう。
コールバックは先発プロセスの **tokio タスク上**で走り、メインスレッドではない。ウィンドウ操作は `AppHandle::run_on_main_thread` を通すこと。
`RunEvent::Exit` でソケットは自動で片付けられる (`destroy` が内部で呼ばれる)。

**4. フルスクリーン空間での可視化は自分で書くしかない。**
`tao-0.35.3/src/platform_impl/macos/window.rs:1539` の `set_visible_on_all_workspaces` は `NSWindowCollectionBehavior::CanJoinAllSpaces` だけを立てる。`FullScreenAuxiliary` は tao のどこにも現れず、`WindowConfig` にも `collectionBehavior` / `fullScreenAuxiliary` に相当する項目は存在しない (`deny_unknown_fields` のため書いても弾かれる)。
したがって `tauri.conf.json` の `visibleOnAllWorkspaces: true` では CAP-1 を満たせない。`WebviewWindow::ns_window() -> Result<*mut c_void>` (macOS 限定、cargo feature 不要) から `objc2-app-kit` で直接立てる:

```rust
let ptr = window.ns_window()? as *mut NSWindow;
let ns_window: &NSWindow = unsafe { &*ptr };
ns_window.setCollectionBehavior(
    NSWindowCollectionBehavior::CanJoinAllSpaces
        | NSWindowCollectionBehavior::FullScreenAuxiliary,
);
```

`collectionBehavior()` / `setCollectionBehavior()` は objc2-app-kit 0.3.2 では **safe** メソッド。feature は `"NSWindow"` と `"NSResponder"` の両方が要る (`NSWindow` は `NSResponder` を引き込まない)。`NSWindow` は `MainThreadOnly` クラスなのでメインスレッドで呼ぶこと。バージョンは Tauri 2.11.5 自身の依存に合わせて `objc2 = "0.6"` / `objc2-app-kit = "0.3"` とする — ずらすと crate が二重になりリンクが壊れる。

**5. 自動起動の解除。**
`auto-launch` は `~/Library/LaunchAgents/{app_name}.plist` を書く。第 1 試行は `app_name` に `my-task-manager` を渡していたため `~/Library/LaunchAgents/my-task-manager.plist` になる (実機で確認済み。`Label` = `my-task-manager`、`ProgramArguments` は `.app` 内部の実行ファイル、`RunAtLoad`)。
`is_enabled()` は **plist ファイルの存在を見るだけ**で launchctl を参照しない。`disable()` は **ファイルを削除する** (launchctl の unload はしない)。`launchctl` を一切呼ばないため、plist を消しても現在走っているプロセスは止まらない — `make uninstall` は終了・plist 削除・`.app` 削除の 3 つを揃える必要がある。

### 再導出で踏まないための既知の罠 (第 1 試行で実地に確認したもの)

- **検証は必ず `.app` に対して行う。** バンドルされていない裸のリリースバイナリを直接起動すると、ウィンドウは出るが webview がページを読み込まない (`on_page_load` が発火しない)。`measure-footprint.sh` の responsible pid による束ね方も、裸のバイナリでは WebKit ヘルパーを拾えない。
- **`dist/` を更新しても `cargo build` は再コンパイルしない。** フロントを変えたら `pnpm tauri build` を使うこと。
- **`pkill -x "My Task Manager"` は決して一致しない。** 実プロセス名は Cargo の `name` である `my-task-manager` (本セッションで `pgrep -x` により再確認)。
- **mise を使わないシェルで直接 `cargo` を叩くと 1.81.0 で失敗する。** 環境に `RUSTUP_TOOLCHAIN` が残るため。`make` 経由か `env -u RUSTUP_TOOLCHAIN` を付けること。
- **`tsc --noEmit` は何も検査していない。** テンプレートのルート `tsconfig.json` は `files: []` + `references` の solution 形式であり 0 ファイルを対象にする。実質的な型検査は `pnpm check` (svelte-check)。

### 本スライスの後に効いてくる帰結

`enable_macos_default_menu(false)` は Cmd+Q / Cmd+W と同時に **Cmd+C / Cmd+V / Cmd+A などの標準編集ショートカットも失わせる**。本スライスのオーバーレイはテキスト入力を持たないため影響しないが、CAP-7 (中断メモの記録) で入力欄が入った時点で、終了項目を含まない最小の編集メニューを復活させる必要がある。

### 着手前に必要なマシン上の後始末

第 1 試行の `.app` が `/Applications` に**インストールされたまま稼働しており** (本セッション開始時点で pid 4581)、LaunchAgent に登録され `Ctrl+Option+Space` を握っている。リポジトリを巻き戻してもアンインストールはされない。**この状態のまま新しいビルドを起動するとホットキー登録に必ず失敗する。** `make uninstall` を実装したうえで、検証の前に実行すること。

## Verification

**Commands (すべて本セッションで実行済み。結果を併記する):**

`cargo test` / `cargo clippy --all-targets -D warnings` / `cargo fmt --check` / `pnpm check` は、
実装後に監査者が**自分で再実行して**結果を確認している (実装エージェントの報告に依らない)。

| コマンド | 期待 | 結果 |
| --- | --- | --- |
| `make uninstall` | 旧版のプロセスが止まり、`.app` と `~/Library/LaunchAgents/my-task-manager.plist` が消える | **PASS.** 第 1 試行の残骸 (pid 4581) の掃除に実際に使用。プロセス・plist・`.app`・自動起動の印がすべて消えたことを確認 |
| `pnpm tauri dev` | ビルドが通り、Dock にアイコンが出ずにアプリが常駐し、メニューバーに項目が出る | **PASS.** 起動しホットキーを登録。`autostart is not registered in a development build` を記録し、開発ビルドのパスをログイン項目に残さない |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` | 警告なし | **PASS.** 0 warnings |
| `cargo test --manifest-path src-tauri/Cargo.toml` | 全件成功 | **PASS.** 30 passed / 0 failed (step-03 の監査で 3 件、step-04 の patch で 2 件追加) |
| `pnpm test` (vitest) | フロントの閉じる経路を実行して検証 | **PASS.** 5 passed / 0 failed。`if (focused)` を反転させると 2 件が落ちることを変異試験で確認 |
| `make lint` / `make test` | 新規 clone でも通る | **PASS.** 両者に `deps` を前提として付け、`lint` は `cargo fmt --check` も走らせる |
| `pnpm check` | 0 errors / 0 warnings | **PASS.** `COMPLETED 101 FILES 0 ERRORS 0 WARNINGS` |
| `make` | `/Applications` に `.app` が配置される | **PASS.** `/Applications/My Task Manager.app` から起動できることを確認 |
| `scripts/measure-footprint.sh` | 待機時メモリ 100MB 未満・CPU 1% 未満 (AD-14) | **PASS.** patch 適用後の最終実測 (150 秒放置後 60 秒観測) で **51.3 MB / 0.00%**。WebKit ヘルパーを含む 5 プロセスが束ねられていることを、強化した束ね検査 (#31) が確認している。第 1 試行の 52.4MB / 0.03% と同等であり、メニューバー項目の追加による予算の圧迫は無い |

**AD-14 — 実装エージェントが報告した CPU 上昇は再現しなかった。**

実装エージェントは「オーバーレイを 1 度でも表示すると待機時 CPU が 0.72% まで上がり、
そのまま下がらない」と報告し、`sample(1)` が示した WebKit の display link
(`RemoteLayerTreeDisplayLinkClient::displayLinkFired`) を原因として挙げていた。

**監査者が実測したところ、この現象は再現しなかった。** 二重起動経由でオーバーレイを表示し
(`overlay Show applied`)、Finder を活性化して非表示にし (`overlay Hide applied`)、その後
120 秒放置して 90 秒観測した結果は **0.03%** であり、未表示時の 0.03% と変わらない。
メモリだけが 55.3MB → 58.2MB と約 3MB 増えた (webview が実際に描画したため)。

報告された 0.72% は、表示直後の落ち着く前を測ったものである可能性が高い。
**ただし「再現しなかった」は「存在しない」ではない。** 測定条件 (放置時間・他アプリの
状態) に依存する可能性は残る。AD-6 が要求する介入パネル (CAP-10) で常駐 webview が
2 つに増える時点で、いずれにせよ再測定が要る — そのとき display link を疑う手掛かりとして
この記録を残す。

**Manual checks (OS・ログイン・実時間を伴うため自動化できない):**

本セッションで機械的に確認できたものには結果を併記した。残りは人の手による確認を要する。

- **[確認済] 1 押下でトグルが 1 回だけ起きること** — 合成キーストロークで押下し、ログに
  `overlay Show applied` が 1 件だけ出ることを確認 (`Released` による 2 件目が無い)
- **[確認済・監査者が再現] 常駐中に `.app` をもう一度起動しても 2 つ目が立ち上がらないこと**
  — 内部の実行ファイルを直接起動し、`pgrep -x my-task-manager` が 1 件のままで、先発が
  `a second launch was detected` → `overlay Show applied` を記録することを確認
- **[確認済] メニューバー項目から終了できること** — アクセシビリティ API で項目を開き、
  中身が「ホットキー Control + Option + Space — 有効」(非活性) と「終了」(活性) の
  **2 つだけ**であることを確認。「終了」を選んでプロセスが終了することを確認
- **[確認済] 既定メニューが消えていること** — アクセシビリティ API で当該プロセスの
  メニューバー項目を列挙すると `subrole: AXMenuExtra` の status menu が 1 つだけで、
  アプリケーションメニュー (File / Edit / Window) が存在しない。Cmd+Q / Cmd+W の
  出所そのものが無い
- **[確認済・監査者が再現] フォーカスを失うとオーバーレイが隠れること** — 表示中に Finder を
  活性化させ、ログに `the overlay asked to be closed` → `overlay Hide applied` が出ることを
  確認 (Esc と同一経路を通っている)
- **[確認済] システム設定 > ログイン項目に登録されること** —
  `~/Library/LaunchAgents/my-task-manager.plist` が作られ、2 回目以降の起動は
  `autostart is left as is (already_attempted=true, enabled=true)` で登録し直さない
- **[未確認] Esc 直後、キー入力が直前のアプリケーションに届くこと** — 合成キーストロークが
  フォーカスを動かしてしまうため機械的に切り分けられない
- **[未確認] 他アプリをフルスクリーンにした空間でホットキーを押し、空間が切り替わらずに
  オーバーレイが出ること**
- **[未確認] オーバーレイ表示中に Cmd+W / Cmd+Q を押しても常駐が継続すること** —
  合成キーストロークは最前面アプリに届くため、誤って利用者のアプリを終了させうる。
  機械的な検証は行わなかった
- **[未確認] メニューバー項目から終了した後、ホットキーが無反応になること**
- **[未確認] 自動起動を解除して再ログインしても復活しないこと** — 再ログインを要する
- **[未確認] OS 再起動 → ログイン後に操作なしで常駐していること** — 再起動を要する
