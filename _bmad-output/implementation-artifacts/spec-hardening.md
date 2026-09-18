---
title: '堅牢化 — 誤終了阻止の固定・CI・編集メニューによる貼り付けの回復'
type: 'chore'
created: '2026-09-18'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: '2d3514bcf90316be731a3edce28267a04e07a92d'
context:
  - '{project-root}/_bmad-output/implementation-artifacts/spec-resident-shell.md'
  - '{project-root}/_bmad-output/planning-artifacts/architecture/architecture-my-task-manager-2026-09-15/ARCHITECTURE-SPINE.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** 三つの穴が一つの仕事に収束している。(1) 誤終了阻止の第 1 層 (`enable_macos_default_menu(false)`) と第 2 層 (`CloseRequested` → `prevent_close`) は、どちらも 1 行消しても全テストが通る — 前回ループバックを起こした領域が無防備なままである。(2) 既定メニューを消した代償として ⌘V・⌘C・⌘A・⌘Z が死んでおり、slice 1 の spec が「CAP-7 の時点で再考する」と記した宿題が未処理のまま、作成の面という**入力が主役の面**が出荷された。摩擦は capture に掛かってはならない (PRD §1.1 賭け #3)。(3) CI が無く、検査は人の記憶に依存している。加えて `cargo test` は `dist/` の存在を前提としており、`dist/` は git 管理外であるため、新しいクローンでは検査が動かない。

**Approach:** 編集メニューを持つアプリケーションメニューを据える。終了と閉じるの項目を**持たない**ため、⌘Q と ⌘W は束縛されないままであり、第 1 層の目的は保たれる。第 1 層と第 2 層には、行が消えたら落ちる検査を置く。GitHub Actions で `make lint` と `make test` を走らせ、`make test` が新しいクローンでも動くようにする。

## Boundaries & Constraints

**Always:**
- アプリケーションメニューに**終了 (`quit`) と閉じる (`close_window`) の項目を置かない**。macOS はメニューを自動で補わないため、項目が無ければ ⌘Q と ⌘W はどこにも束縛されない。これが第 1 層の目的を保つ唯一の条件である。
- `enable_macos_default_menu(false)` を**外さない**。独自メニューを与えれば既定は使われないが、二重の防御として残す。
- メニューの**最上位の要素はすべてサブメニューとする** (平の項目を混ぜるとメニューバーが空になる既知の不具合がある)。macOS では最初のサブメニューがアプリケーションメニューとして表示されるため、そこに置く内容を意図して選ぶ。
- 編集メニューは取り消し・やり直し・切り取り・コピー・貼り付け・すべてを選択を持つ。いずれも OS 標準の項目であり、独自の処理を書かない。
- **終了への唯一の到達経路はメニューバー項目 (`NSStatusItem`) のままとする** (CAP-3)。`PredefinedMenuItem::quit` を使わない — `terminate:` を直接送り、誤終了阻止の全層を迂回する。
- 第 1 層と第 2 層に、**その行が消えたら落ちる検査**を置く。実挙動は AppKit を起動した実アプリでしか観測できないため、これは「存在の固定」であって「挙動の証明」ではない。**その限界をテスト自身に明記する。**
- 検査の対象を型で狭める。編集メニューの項目は、終了や閉じるを**表現できない**形で定義する — 規律ではなく構造で禁じる。
- `make test` は新しいクローンでも成功すること。`cargo test` は `generate_context!` 経由で `dist/` を要求する。
- CI は pull request と `main` への push で `make lint` と `make test` を走らせる。cargo のレジストリとビルド成果物を `Cargo.lock` とツールチェインで鍵付けして再利用する。
- CI は Rust 1.98.1 で走ること。`RUSTUP_TOOLCHAIN` が環境に漏れると `rust-toolchain.toml` より優先され、**黙って別の版でビルドされる**。
- 既存の 170 件の Rust テストと 56 件の vitest を壊さない。

**Never:**
- 誤終了阻止の三層のいずれも弱めない。⌘Q と ⌘W が何かに束縛される状態を作らない。
- トレイのメニュー (終了項目・ホットキー状態行) の構成と挙動を変えない (AD-15)。
- 貼り付けのために JavaScript 側で打鍵を横取りしたり、クリップボードのプラグインを導入したりしない。OS 標準の経路で解く。
- CI で署名・公証・リリース成果物の作成を行わない (v1 の対象外)。
- ドメイン・ストレージ・コマンド・オーバーレイの振る舞いを変えない。本スライスは足場の仕事である。

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|---|---|---|---|
| 貼り付け | 作成の面の入力欄で ⌘V | クリップボードの内容が入る。⌘C・⌘X・⌘A・⌘Z も同様に効く | N/A |
| 誤終了 (⌘Q) | どの面が出ていても ⌘Q | 何も起きない。常駐は生き続ける | N/A |
| 誤って閉じる (⌘W) | オーバーレイ表示中に ⌘W | 何も起きない。ウィンドウは破棄されない | N/A |
| 明示的な終了 | メニューバー項目の「終了」 | これまでどおり終了する | N/A |
| メニューの構成 | アプリケーションメニューを組み立てる | 終了と閉じるを表現する項目が存在しない。最上位はすべてサブメニュー | 組み立てに失敗したら起動を止めず記録する |
| 第 1 層の削除 | `enable_macos_default_menu(false)` を消す | テストが落ちる | N/A |
| 第 2 層の削除 | `CloseRequested` を非表示へ変換する箇所を消す | テストが落ちる | N/A |
| 新しいクローン | `dist/` が無い状態で `make test` | 成功する | N/A |
| CI (pull request) | PR を開く / 更新する | `make lint` と `make test` が走り、失敗すれば赤くなる | N/A |
| CI (ツールチェイン) | 環境に `RUSTUP_TOOLCHAIN` が漏れている | 1.98.1 で走る | N/A |
| CI (キャッシュ) | 依存が変わっていない 2 回目の実行 | ビルド成果物を再利用する | キャッシュが無くても成功する |

</frozen-after-approval>

## Code Map

- `src-tauri/src/lib.rs:15-20` -- 第 1 層の理由。「既定メニューの Cmd+Q は `NSApplication terminate:` を直接送り `RunEvent::ExitRequested` を迂回するため、**これが Cmd+Q に対する唯一有効な防御である**」
- `src-tauri/src/lib.rs:117` -- 第 1 層の呼び出し。**外さない**
- `src-tauri/src/lib.rs:128-136` -- 第 2 層。`CloseRequested` → `prevent_close` → `presentation::hide`
- `src-tauri/src/lib.rs:230-238` -- 第 3 層。`should_prevent_exit` を通す唯一の箇所。純粋関数は既に検査済み
- `src-tauri/src/adapters/menubar/mod.rs:9-11` -- `PredefinedMenuItem::quit` 禁止の理由。アプリケーションメニューにも同じ理由が掛かる
- `src-tauri/src/adapters/menubar/mod.rs:48-79` -- トレイのメニュー。**触らない**。新しいメニューはアプリケーションメニューであり別物である
- `src-tauri/src/adapters/autostart/mod.rs:153-160` -- `include_str!` で別ファイルの内容を検査する既存の作法。第 1・第 2 層の固定はこれに倣う
- `src-tauri/src/adapters/presentation/mod.rs:91-141` -- 判断を純粋関数に切り出す作法。ただし引数の無い関数はトートロジーになる (slice 1 所見 #7)
- `Makefile:53-56` -- `make test` の中身。`deps` にしか依存しておらず `dist/` を作らない
- `Makefile:24-27` -- `unexport RUSTUP_TOOLCHAIN` とその理由。CI でも同じ危険がある
- `Makefile:44-48` -- `build` は `pnpm tauri build`。フロントエンドだけを作る経路は現状無い
- `package.json` -- `build` スクリプトと `packageManager` (pnpm 11.24.0)。Node の版はどこにも固定されていない
- `rust-toolchain.toml:6-11` -- 1.98.1、`clippy`/`rustfmt`、darwin 二種
- `src-tauri/tauri.conf.json` -- `frontendDist` は `../dist`。`minimumSystemVersion` は 14.0

## Tasks & Acceptance

**Execution:**
- [x] `src-tauri/src/adapters/menubar/mod.rs` または新規モジュール -- 編集メニューの項目を、終了と閉じるを**表現できない**列挙として定義し、そこからメニューを組み立てる -- 構造で禁じる。組み立ては `Manager` を要するがこの定義は要さない
- [x] `src-tauri/src/lib.rs` -- アプリケーションメニューを据える。最上位はすべてサブメニュー。最初のサブメニューがアプリケーション名として表示されることを踏まえて内容を選ぶ -- 既知の不具合の回避。`enable_macos_default_menu(false)` は残す
- [x] `src-tauri/src/lib.rs` -- 第 1 層と第 2 層に、行が消えたら落ちる検査を足す -- `include_str!` の既存作法。**「存在の固定であって挙動の証明ではない」ことをテストの doc に明記する**
- [x] `Makefile` -- `test` がフロントエンドを先に作るようにする -- 新しいクローンで `cargo test` が `dist/` の不在で落ちるため
- [x] `.github/workflows/` -- macOS で `make lint` と `make test` を走らせる。pull request と `main` への push が契機。cargo のレジストリと `src-tauri/target` を `Cargo.lock` とツールチェインで鍵付けして再利用する -- `RUSTUP_TOOLCHAIN` を環境へ出さないこと。署名も成果物の公開も行わない

**Acceptance Criteria:**
- Given 実装完了、when メニューの定義を検索する、then 終了・閉じるを表現する項目がどこにも現れない
- Given 実装完了、when `enable_macos_default_menu(false)` の行を消す、then テストが落ちる
- Given 実装完了、when `CloseRequested` を非表示へ変換する箇所を消す、then テストが落ちる
- Given `dist/` を消した状態、when `make test` を実行する、then 成功する
- Given 実装完了、when `make lint` と `make test` を実行する、then いずれも成功し、既存の 170 件と 56 件が保たれる
- Given 実アプリを起動した、when 作成の面で ⌘V を押す、then クリップボードの内容が入る
- Given 実アプリを起動した、when ⌘Q と ⌘W を押す、then 常駐は生き続け、ウィンドウも破棄されない
- Given 実アプリを起動した、when メニューバー項目から「終了」を選ぶ、then これまでどおり終了する

## Implementation Notes

### 何が立ったか

```text
my-task-manager/
  .github/workflows/ci.yml          # 新規。macOS で make lint / make test
  Makefile                          # deps / frontend をファイルで追い test / lint がこれに依存する
                                    # toolchain ターゲットを追加 (CI が版の一致を確かめる)
  src-tauri/src/
    lib.rs                          # アプリケーションメニューの設置 + 第 1〜3 層の固定
    adapters/mod.rs                 # appmenu を公開
    adapters/appmenu/mod.rs         # 新規。編集メニューの定義と組み立て
```

### 編集メニューを別モジュールに置いた

`adapters/menubar/` はトレイ (`NSStatusItem`) のモジュールであり、**触らないと決まって
いる** (AD-15、本仕様の Never)。アプリケーションメニューは `NSApp.mainMenu` であって
別物であるため、`adapters/appmenu/` を新設した。OS API はアダプタ層にのみ置く (AD-1)。

### 終了と閉じるを「構造で」禁じた形

- **編集項目は一つの表から生える。** `edit_items!` マクロが、変種・表示名・OS 標準の
  生成関数を 1 行ずつ受け取り、列挙と `label()` と `predefined()` をまとめて作る。
  三箇所に分けて書くと `Paste` に `copy` を割り当てる取り違えが起きても、定数も文言も
  他のどの検査も動かない — **Cmd+V だけが無言で死ぬ。** その 1 行の左右が一致している
  こと自体も、マクロが吐く `PAIRINGS` を `the_variants_and_their_constructors_are_paired`
  が読んで確かめる (`SelectAll` → `select_all`)。
- 表に無いものは綴れない。`Quit` も `CloseWindow` も `EditItem` に存在せず、足すには
  まず表に行を足さなければならない。「うっかり `PredefinedMenuItem::quit` と書く」経路が
  構造的に無い。
- 最上位が平の項目を持たないことも型で担保した。`build` は `Vec<Submenu<R>>` を作って
  から `&dyn IsMenuItem` に落とす — 平の項目はそもそも中間の型に入らない。
- 並びは `TOP_LEVEL` / `EDIT_MENU` / `APPLICATION_MENU` という定数のデータであり、
  `Manager` を要さない。したがって「貼り付けが編集メニューにある」「アプリケーション
  メニューが先頭にある」「その中身が押せない」を OS を起動せずに検査できる。

### アプリケーションメニューの 1 行が何をしているのか

**利用者はこの行を見ない。** 本アプリは `ActivationPolicy::Accessory` で走るため
メニューバーを一度も描かない。それでも key equivalent は `NSApp.mainMenu` を辿って
解決されるため、編集メニューは効く — 描画と解決は別である。

この行の仕事は**最初の枠を埋めることだけ**である。macOS は最上位の最初のサブメニューを
アプリケーションメニューとして扱い、表題をアプリ名に差し替える。編集メニューを先頭に
置けば、編集メニューが名前ごとアプリ名に化ける。空のサブメニューでも枠は埋まるが、
何のために在るかを述べた 1 行のほうが、後から読む者が「無駄だから」と消しにくい。

行の性質は `APPLICATION_MENU` の定数データ (id・文言・活性・アクセラレータ) として
置いた。手で `MenuItem::with_id(..)` を書くと、**`false` を `true` に変えてアクセラレータを
足すだけで Cmd+Q が束縛されうる** — 定数は一つも変わらず全テストも緑のままで、まさに
第 1 層が防いでいる状態に入る。`the_application_submenu_is_a_single_inert_row` が
「1 行・非活性・アクセラレータ無し」を固定する。

### `Builder::menu` ではなく `setup` 内の `set_menu` を使った

`Builder::menu` の閉包が `Err` を返すと `Builder::build` ごと `Err` となり、`run()` の
`.expect()` で常駐そのものが立ち上がらない。I/O マトリクス「メニューの構成」は
「組み立てに失敗したら起動を止めず記録する」と定めているため、`setup` 内で
`appmenu::install` を呼び、失敗は `log::error!` で記録して進む (setup 内で `?` を
使わないという既存の規律と同じ)。

`AppHandle::set_menu` は macOS では `run_on_main_thread` 経由で `init_for_nsapp` を
呼ぶが、`send_user_message` はメインスレッドから呼ばれた場合に**同期実行する**
(`tauri-runtime-wry` のソースで確認)。`setup` はメインスレッドで走るため、遅延も
取りこぼしも起きない。

ただし `set_menu` はその取り付けの結果を `let _ = ..` で捨てる。したがって `Ok` は
「組み立てが通った」以上を意味せず、**メニューの無いまま起動しても何もログに残らない** —
I/O マトリクスが求める「失敗を記録する」を半分しか満たさない。据えた直後にアプリ全体の
メニューを読み戻し、それが自分の組んだもの (同じ `MenuId`) であることを確かめ、違えば
`AppMenuError::NotInPlace` を返すようにした。成功時も据わった旨を `INFO` で残す。

呼び出しは `#[cfg(target_os = "macos")]` で囲んだ。近傍の `set_activation_policy` /
`allow_fullscreen_spaces` と同じ扱いである — 他のデスクトップではアプリケーション
メニューが `decorations: false` のウィンドウにメニューバーを生やしてしまい、
取り消し・やり直しはそもそも対応されていない。

### 第 1〜3 層の固定 — 自己一致と、コメントによる偽装を塞いだ

`include_str!("lib.rs")` で自分自身を読むため、素直に書くと**テストのリテラルが本文に
現れて一致してしまい、守るべき行を消しても通る**。四重に塞いだ。

1. needle を `concat!("enable_macos_default_menu", "(false)")` のように連結で組み立てる。
   連結後の文字列は呼び出し箇所にしか存在しない。
2. `#[cfg(test)]` 以降を切り落とす。検査自身の字面 — needle を組み立てる行そのもの —
   が探索の対象に入らないようにする。
3. ブロックコメントを取り除く。`/* .enable_macos_default_menu(false) */` と書けば、
   コードの位置に任意の字面を残せてしまう。
4. 行コメントを行頭に限らず落とす。行末に書き足しても同じ偽装ができる。

**第 3 層も固定した** (仕様の Execution は第 1・第 2 層しか挙げていない)。
`should_prevent_exit` の単体検査は純粋関数の判断しか見ないため、`.run()` の閉包ごと
消しても緑のままである — 判断は正しいが誰もそれを問わない状態になる。

**検査の名前を「配線が効いている」から「字面が書かれている」へ改めた**
(`..._is_still_wired` → `..._is_still_written_in_the_source`)。言えるのはそこまでであり、
`if cfg!(feature = "never") { .. }` で包んでも、どこからも呼ばれない関数へ移しても、この
検査は通る。**到達性は覆っていない。** 名前でそれを主張しない。

**変異試験で荷重を確認した** (10 件、いずれも守るべき欠陥そのもので失敗する)。

| 変異 | 落ちたテスト |
| --- | --- |
| `.enable_macos_default_menu(false)` を消す | `the_first_layer_call_is_still_written_in_the_source` |
| 同上を消し、needle を**行末コメント**に残す | 同上 |
| 同上を消し、needle を**ブロックコメント**に残す | 同上 |
| `api.prevent_close();` のみ消す | `the_second_layer_calls_are_still_written_in_the_source` |
| `on_window_event` の `CloseRequested` ブロック全体を消す | 同上 |
| `.run()` の閉包を空にする (第 3 層ごと) | `the_third_layer_calls_are_still_written_in_the_source` |
| `appmenu::install(&handle)` の呼び出しを消す | `the_application_menu_call_is_still_written_in_the_source` |
| `EDIT_MENU` から `EditItem::Paste` を落とす | `every_editing_shortcut_has_a_menu_item_to_translate_it` |
| 表の `Paste => paste` を `Paste => copy` に取り違える | `the_variants_and_their_constructors_are_paired` |
| アプリケーションメニューの行を活性にしアクセラレータを足す | `the_application_submenu_is_a_single_inert_row` |

`appmenu::install` の固定は仕様の Execution に無いが足した。編集メニューの定義を
いくら検査しても、据える呼び出しが消えれば項目はどこにも存在しないまま全テストが通る
— Cmd+V の回復という本スライスの主目的が無言で失われる。

### 仕様の前提との食い違い — `dist/` は現状 `cargo test` に要求されていない

仕様は「`cargo test` は `generate_context!` 経由で `dist/` を要求する」と記しているが、
**現在の設定では要求しない。** `tauri-codegen` は `dev && config.build.dev_url.is_some()`
のとき埋め込み資産を空にする分岐を持ち、`dev` は `cfg!(not(feature = "custom-protocol"))`
である。`tauri.conf.json` に `devUrl` があるため、`cargo test` / `cargo clippy` は
`dist/` が無くても通る (`dist/` を消して実際に確かめた)。

それでも `dist/index.html` を `test` と `lint` の前提にした (`frontend` はその別名)。
理由は二つ。

- 成り立ちが `devUrl` という 1 条件に寄りかかっている。外すか `custom-protocol` を
  立てた瞬間に、新しいクローンでは検査そのものが落ちる。
- 受け入れ基準「`dist/` を消した状態で `make test` が成功する」を、分岐の副作用ではなく
  **意図した経路として**満たす。

`lint` にも同じ前提を付けた。`cargo clippy --all-targets` も `generate_context!` を展開
するため、`test` だけ直しても CI の最初の段が新しいクローンで落ちうる。

**前提は phony ではなく実体のあるファイルにした。** `deps` は
`node_modules/.modules.yaml`、`frontend` は `dist/index.html` を追う。CI は lint と test を
別々の段として走らせる (片方が落ちても他方の結果を見たいため) ので、phony のままだと
`pnpm install` と `vite build` が呼び出しのたびに繰り返される。ファイルで追えば 2 回目は
何もしない — `dist/` を消した場合や `src/` を変えた場合はこれまでどおり作り直す
(どちらも実際に確かめた)。

### CI

`.github/workflows/ci.yml` — `pull_request` と `main` への push で `macos-15` に
`make lint` → `make test`。署名・公証・成果物の公開は行わない (v1 の対象外)。

- **ツールチェイン。** `rustup toolchain install` を引数なしで呼び、`rust-toolchain.toml`
  を唯一の正とする (channel / components / targets をワークフローに書けば二重管理になる)。
  そのうえで `make toolchain` の出力と `rust-toolchain.toml` の `channel` の一致を確かめる。
  **その段では `RUSTUP_TOOLCHAIN` にわざと誤った版 (1.81.0) を設定する。** runner は普段
  これを設定しないため、素直に書くと Makefile の `unexport RUSTUP_TOOLCHAIN` を消しても
  CI は緑のままで、検査が何も守らない。漏れを再現してはじめて `unexport` が荷重を負う
  (手元で両方向を確認 — `unexport` 有りで 1.98.1、無しで 1.81.0 となり比較が落ちる)。
- **`mise.toml` との一致。** mise は `RUSTUP_TOOLCHAIN` を export するため、**make を
  経由しない経路** (素の `cargo`・エディタ・スクリプト) はすべて mise 側の版で走る。
  `rust-toolchain.toml` と食い違ったまま気づかない状態を作らないよう、同じ段で比較する
  (1.97.0 へずらして落ちることを確認済み)。
- **lint が落ちても test を走らせる。** `if: ${{ !cancelled() }}`。書式の誤り 1 件で
  その PR のテスト結果が丸ごと見えなくなると、直して押し直すまで何も分からない。
  job は lint の失敗で落ちたままである。
- **runner とタイムアウト。** `macos-15` に固定する — 他のすべての版を厳密に固定して
  いるリポジトリで runner だけ追跡にする理由が無い。`timeout-minutes: 45` を置き、
  詰まった実行が macOS runner を既定の 6 時間占有しないようにする。
- **キャッシュ。** `~/.cargo/registry/{index,cache}` / `~/.cargo/git/db` /
  `src-tauri/target` を、`rust-toolchain.toml` と `src-tauri/Cargo.lock` のハッシュで
  鍵付けする。`restore-keys` はツールチェインまでを前方一致とするため、依存だけが
  変わった回は前回の成果物を引き継ぐ。キャッシュが無くても検査はそのまま成功する。
- **pnpm の版。** `pnpm/action-setup@v4` に版を書かず `package.json` の
  `packageManager` (11.24.0) から取らせる。Node は 22 を明示した — リポジトリのどこにも
  固定が無いため、ここだけが唯一の宣言になる。

### 手動確認は済んでいない

**本セッションでは実アプリを起動していない。** Cmd+V の効き、Cmd+Q / Cmd+W で常駐が
生き続けること、メニューバー項目からの終了、CI が実際に緑になることは、いずれも
下の Manual checks のまま残っている。自動テストが主張しているのは**配線の存在**だけで
ある。

## Spec Change Log

## Review Triage Log

第 1 回レビュー (blind-hunter / edge-case-hunter / verification-gap)。intent_gap・bad_spec は無く、ループバックは発生していない。

| # | 出所 | 所見 | 判定 | 根拠 | 経路 |
|---|---|---|---|---|---|
| 1 | BH / VG | `EditItem` から `PredefinedMenuItem` への対応付けを検査するものが無い | high | `Self::Paste => PredefinedMenuItem::copy(...)` と書き違えても、定数表も `label()` も `TOP_LEVEL` も変わらないため全件緑のまま ⌘V だけが死ぬ。本スライスの主目的が一行の取り違えで失われる。網羅 `match` は「終了を足せない」ことしか保証しておらず、七つの変種の取り違えは防げない | patch |
| 2 | VG / EC | アプリケーションメニューの第 1 サブメニューが手組みで、何も検査していない | high | `false` を `true` に、`None` を `Some("CmdOrCtrl+Q")` に変えても定数は一つも変わらず全件緑。**⌘Q が項目に束縛される** — 第 1 層が存在を賭けて防いでいる状態そのものである。編集メニューと違い、ここはデータ駆動になっていない | patch |
| 3 | VG / BH / EC | 配線の固定は「行の削除」しか捉えない。無効化・`cfg` による封じ・未到達の関数への移動・ブロックコメントはすべて素通りする | high | 検証者が同じ述語を変異させた写しに対して実際に走らせ、`if cfg!(feature = "never") { api.prevent_close(); }` でも五つの needle が一致することを確認済み。加えて `LIB_SOURCE` は `mod tests` 以降も含むため、将来のテストが文字列にその語を書けば荷重が抜ける。テスト名 `..._is_still_wired` は「配線が生きている」と過大に主張している | patch |
| 4 | BH | 第 3 層 (`ExitRequested` → `should_prevent_exit` → `prevent_exit`) にだけ固定が無い | medium | 純粋関数は検査済みだが `.run()` の閉包ごと消しても通る。三層のうち一層だけ無防備という非対称は、本 spec の Intent が問題視した状態と同じである | patch |
| 5 | BH / EC | CI のツールチェイン検査が GitHub 上では荷重ゼロ | medium | runner に `RUSTUP_TOOLCHAIN` が存在しないため、`Makefile` から `unexport` を消しても CI は緑のまま。手元で行った確認 (`RUSTUP_TOOLCHAIN=1.81.0 make toolchain`) と同じ条件を CI が作っていない | patch |
| 6 | BH / EC | `mise.toml` が第二の版の正でありながら、誰も一致を見ていない | medium | ここがずれると `make` を経由しない経路 (rust-analyzer・素の `cargo`・保存時 clippy) だけが黙って別の版になる。本ワークフローが防ごうとしている失敗そのものである | patch |
| 7 | EC | `make lint` が落ちると `make test` の段が実行されない | medium | 書式の誤り一つで、その PR のテスト結果が丸ごと不明になる | patch |
| 8 | EC / BH | ジョブに時間制限が無く、runner の像も固定されていない | medium | 停止しない実行が macOS の runner を既定の 6 時間占有しうる。また AppKit に依存するアプリで runner 像の更新は前提を黙って変える。本リポジトリは他のすべての版を厳密に固定している | patch |
| 9 | BH / EC | `appmenu::install` に `#[cfg(target_os = "macos")]` が無い | medium | すぐ上の `set_activation_policy` も `allow_fullscreen_spaces` も macOS で囲ってある。他 OS では `decorations: false` のウィンドウにメニューが生え、取り消し・やり直しは未対応である | patch |
| 10 | VG / EC | `set_menu` は macOS 側の取り付け結果を捨てるため、取り付けの失敗が記録に残らない | medium | `init_app_menu` の結果が `let _` で捨てられており、`install` が返すのは組み立ての失敗だけである。I/O マトリクスの「組み立てに失敗したら起動を止めず記録する」が半分しか満たされていない | patch |
| 11 | BH / EC | 第 1 サブメニューの 1 行は利用者に見えない。doc コメントがそうでない前提を語っている | low | アプリは `Accessory` で走り、メニューバーを描かない。実際の役割は「編集メニューが先頭に来てアプリ名に化けるのを防ぐ場所取り」である。そう書かないと、後から不要と判断されて消され、編集メニューが改題される事故を招く | patch |
| 12 | BH | Implementation Notes の変異件数 (4) と直後の表 (5 行)・Verification (5 件) が食い違う。`make lint test` で `pnpm install` と `vite build` が二度走る | low | どちらも直接的な訂正で済む | patch |
| 13 | EC | ツールチェイン検査は `channel` が `stable`/`beta`/二成分のとき必ず落ちる | low | 本リポジトリの `channel` は `1.98.1` であり、厳密な固定は方針として明記されている。起きていない不都合であり、修正はこのプロジェクトが採らない設定のための分岐を足すことになる | 棄却 |
| 14 | EC | 受け入れ基準「終了・閉じるを表現する項目がどこにも現れない」は grep では合否を判定できない | low | 基準が指すのは**アプリケーションメニュー**であり、トレイの `終了` 項目 (CAP-3 の唯一の終了経路) は対象外である。文言の修正は凍結された節の編集に当たるため行わない。検証はアプリケーションメニューの定義に対して行った | 棄却 |
| 15 | EC | 主張「`cargo test` は `dist/` を前提とし、新しいクローンでは動かない」は偽である | — | **確認済み。`dist/` を完全に消して 178 件すべて成功した。** `devUrl` が設定されているとき `tauri-codegen` は埋め込みを省くためである。この誤りは spec の凍結された Intent に含まれており、私が検証せずに書いた。凍結節は変更できないため Implementation Notes に記録済みであり、利用者にも訂正を伝えたうえで Makefile の変更を残す判断を得ている | 訂正済 |


## Design Notes

**なぜメニューを足すことが第 1 層を弱めないのか。** Tauri の `Builder::build` は、メニューが未設定でかつ `enable_macos_default_menu` が真のときにだけ `Menu::default` を組み込む。独自のメニューを与えればその分岐に入らない。そして macOS はプログラムから設定した `mainMenu` に項目を補わない — AppKit は `setMainMenu` された内容をそのまま描くだけである。したがって終了の項目を置かなければ `terminate:` はどの打鍵にも結び付かず、⌘Q は「どこにも束縛されていない打鍵」のままである。**メニューが無い状態と、終了項目の無いメニューがある状態は、⌘Q に関して等価である。**

**なぜ ⌘V がメニュー無しでは効かないのか。** macOS では修飾キー付きの打鍵は通常の文字入力経路ではなく **key equivalent** として処理される。`NSApplication` は `sendEvent:` でそれを見つけ、キーウィンドウに `performKeyEquivalent:` を送り、誰も処理しなければ**メニューバーのメニューに**送る。WKWebView は編集操作を責任連鎖のアクション (`paste:` など) として実装しており、`performKeyEquivalent:` で ⌘V を主張しない。メニューが無ければ打鍵をアクションへ変える者がいない。編集メニューを置くことは、その変換器を戻すことである。

**なぜ「存在の固定」で満足するのか。** 第 1 層と第 2 層の実挙動は AppKit を起動した実アプリでしか観測できず、本リポジトリは UI 自動化の基盤を持たない。合成した打鍵は最前面のアプリに届くため、自動で ⌘Q を押す検査は利用者のアプリを終了させうる (slice 1 の判断)。ここで置くのは**行が黙って消えることを防ぐ仕掛け**であり、それ以上を主張しない。挙動の確認は手順として残す。

## Verification

**Commands (すべて本セッションで実行済み。結果を併記する):**

| コマンド | 期待 | 結果 |
| --- | --- | --- |
| `make lint` | `cargo fmt --check` と `clippy --all-targets -- -D warnings` が警告なしで成功する | **PASS.** 0 warnings |
| `make test` | Rust 170 件以上、vitest 56 件以上がすべて成功する | **PASS.** Rust 180 passed / 0 failed (170 → 180、10 件追加)、vitest 56 passed / 0 failed、`pnpm check` 0 errors 0 warnings |
| `rm -rf dist && make test` | 成功する (新しいクローンと同じ条件) | **PASS.** `frontend` が `dist/` を作り直してから検査へ進む |
| `rg -n 'quit\|close_window' src-tauri/src/lib.rs` | アプリケーションメニューに終了・閉じるを足した箇所が無い | **PASS.** 一致はモジュールドキュメントの説明・トレイが立たないときの `exiting instead` のログ・固定テストの関数名のみ。`PredefinedMenuItem::quit` / `close_window` の呼び出しは `src-tauri/src` のどこにも無い (禁止を述べる doc コメント 3 箇所のみ) |
| `RUSTUP_TOOLCHAIN=1.81.0 make toolchain` | `rust-toolchain.toml` の 1.98.1 が返る | **PASS.** `unexport` が効いており 1.98.1。CI の版検査はこれと同じ経路を見る |
| 同上で `unexport RUSTUP_TOOLCHAIN` を外す | 検査が落ちる | **PASS.** 1.81.0 が返り比較が食い違う。CI がこの段で意図的に誤った版を漏らしているのは、これを再現するためである |
| `mise.toml` の rust を 1.97.0 へずらす | 検査が落ちる | **PASS.** `rust-toolchain.toml` との比較で検出 |

**変異試験 (固定テストが荷重を負っていることの確認):** 10 件。表は Implementation Notes
にある。いずれも守るべき欠陥そのもので失敗する。

**Manual checks (if no CLI):**
- `make install && make open` の後、作成の面で ⌘V・⌘C・⌘A・⌘Z を試す。効かなければメニューの構成が誤っている
- 同じ状態で **⌘Q と ⌘W を押し、常駐が生き続けることを確かめる。** これが本スライスで最も重要な手動確認である — ここを壊すと一打で常駐が死に、次のログインまでホットキーが失われる
- メニューバー項目の「終了」でこれまでどおり終了できることを確かめる
- CI は最初の pull request で実際に緑になることをもって確認する

**いずれも本セッションでは未実施である。** 実アプリを起動しておらず、pull request も
開いていない。自動テストが主張しているのは配線の存在だけであり、挙動ではない。
