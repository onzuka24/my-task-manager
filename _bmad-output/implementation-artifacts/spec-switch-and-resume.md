---
title: '切り替えと再開 — 既定表示の最小化・中断メモの記録・再開時の位置提示'
type: 'feature'
created: '2026-09-17'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: '17d9e31f7faa4dded5ce9c45011889e73733d891'
context:
  - '{project-root}/_bmad-output/specs/spec-my-task-manager/SPEC.md'
  - '{project-root}/_bmad-output/specs/spec-my-task-manager/glossary.md'
  - '{project-root}/_bmad-output/planning-artifacts/architecture/architecture-my-task-manager-2026-09-15/ARCHITECTURE-SPINE.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** ドメインは**タスク**・**ステップ**・**現在地**を持ち永続化もできるが、そこへ到達する経路が一つも無い。オーバーレイは `DUMMY_NEXT_ACTION` という定数を表示しており、コマンドはドメインに触れず、イベントは一つも発行されていない。v1 が検証すべき唯一の仮説 — 中断地点を記録し再開時に全体の中の位置を返すことで前のタスクを手放せるようになるか — は、**切り替え**が存在しない現在まったく試せない。

**Approach:** CAP-2・CAP-7・CAP-8 を一つの面として立てる。オーバーレイは既定で**次の一手**と位置情報だけを見せ、記録済みの**中断メモ**があればその場で読ませる。**切り替え**は「離脱側のメモ確定・完了宣言 (任意)・**現在地**の移動・**切り替え履歴**の追記」を**単一のトランザクション**で確定させる儀式として実装する (AD-5)。AD-3 のコマンド/イベント契約を、消費者が存在するこの時点で初めて定める。

## Boundaries & Constraints

**Always:**
- 既定表示は**次の一手**のみ。二つ以上の**タスク**名または二つ以上の**ステップ**内容が同時に現れてはならない (FR-2)。位置情報「第 N ステップ / 全 M ステップ」はこの制限の唯一の例外である。
- 再び**現在地**となった**ステップ**では、位置情報と記録済みの**中断メモ**が、他の画面を参照せず読める位置に自動的に現れる (FR-8)。
- **切り替え**の移動先は同一**タスク**内の次の**ステップ**とする。任意の**ステップ**への移動は CAP-9 の**開示面**に属する。
- **中断メモ**の入力欄は既存のメモで初期化し、カーソルを末尾に置く。これが FR-7 の「上書き前の内容の提示」と「追記の形を選べる」を同時に満たす。
- **メモの入力は省略可能であり、省略しても切り替えは完了する** (FR-7)。
- **完了**の宣言は**切り替え**と同じ一連の操作から 1 打鍵で到達でき、宣言しない**切り替え**も同じく可能であること。**完了**は**現在地**の移動から導出しない (FR-4 / AD-2)。
- オーバーレイの呼び出しから**中断メモ**の確定および離脱までが **5 打鍵以内** (FR-7)。呼び出しの 1 打鍵を含めて数える。メモ本文のタイピングは数えない。
- 離脱側のメモ確定・完了宣言・**現在地**の移動・**切り替え履歴**の追記を**単一のトランザクション**で書く (AD-5)。入力途中のメモを永続化しない。
- **切り替え履歴** (`SwitchRecord`) は発生時刻と**メモ記入の有無**のみを持ち、メモ本文を持たない。書くだけで読み戻さない。
- Svelte → コアは command のみ、コア → Svelte は event のみ (AD-3)。command は `動詞_名詞`、event は `名詞_過去分詞`。
- **オーバーレイは表示されるたびにコマンドで完全なスナップショットを取得してから描画する** (AD-3 鮮度規則)。イベントは再描画の契機にすぎず、状態の出所ではない。
- コマンドは `Core` の不在を `try_state` で扱い、既定値で埋めたコアを代用しない (`lib.rs:199-202`)。
- 時刻は UTC の ISO 8601、ID は UUID v7。依存は `=` で固定する。
- **中断メモの本文をログに書かない** (スパイン「一貫性の規約」)。

**Never:**
- **切り替え履歴**を利用者に見せない。集計・件数・記入率の表示も作らない (AD-15)。
- 進捗率・バー・パーセント・色による進捗表現を作らない。「第 N / 全 M」は位置情報であって進捗の可視化ではない (AD-15)。
- 一覧・**開示面** (CAP-9)、**タスク**や**ステップ**の作成・編集の面 (別スライス)、**休息**の計時と介入 (CAP-10) を実装しない。
- **切り替え**で**連続作業時間**をリセットしない。リセットは非活性→活性の遷移のときだけである (AD-8)。
- テキスト入力を伴う面を非活性パネルとして実装しない。IME が阻害される (AD-6)。オーバーレイは通常ウィンドウのままとする。
- 既存の 3 コマンドの wire contract を壊さない。`OverlaySnapshot` は拡張してよいが `hotkey` を失わせない。

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|---|---|---|---|
| 既定表示 | **現在地**が第 3/全 6 の**ステップ**を指す | そのステップ内容と「第 3 ステップ / 全 6 ステップ」のみ。他のステップ内容・他のタスク名は現れない | N/A |
| 再入時のメモ提示 | 指している**ステップ**に**中断メモ**がある | 既定表示の中でメモが読める。追加操作を要しない | N/A |
| 未着手 | **現在地**が `NotStarted` | 未着手である旨の 1 行。空欄にしない | N/A |
| メモを書いて切り替え | 第 3 を離脱、本文あり | 第 3 にメモが残り、**現在地**は第 4 へ。履歴は `note_written = true` | 書き込み失敗時は状態を変えず失敗を提示 |
| メモを省いて切り替え | 本文が空 | **切り替え**は完了する。メモは変更しない。履歴は `note_written = false` | 同上 |
| 二度目の切り替え | 第 3 に既存メモがある状態で再度離脱 | 入力欄は既存メモで初期化され、カーソルは末尾。確定内容がメモを置き換える | 同上 |
| 完了して切り替え | 完了宣言を伴う操作 | 第 3 に `completed_at` が付き、**現在地**は第 4 へ。両者が同一トランザクション | 同上 |
| 最終ステップからの切り替え | **現在地**が第 6/全 6 | 移動先が無い旨を示し、**現在地**は動かない。メモと完了の宣言は成立してよい | N/A |
| 入力途中の離脱 | メモ入力中にフォーカスを失う / Esc | 下書きは破棄され、永続化されない。**現在地**も動かない | N/A |
| コア不在 | DB を開けず `Core` が `manage` されていない | コマンドはエラーを返し、オーバーレイはその旨を表示する。常駐は止まらない | `try_state` の不在を明示的なエラーに変換 |
| 打鍵数 | 呼び出しからメモ確定・離脱まで | 5 打鍵以内で完了する | N/A |
| 切り替え後の再描画 | **切り替え**が確定した | `current_position_changed` が発行され、オーバーレイは**スナップショットを取り直して**描画する | イベント取りこぼし時も次回表示で正しくなる |

</frozen-after-approval>

## Code Map

- `src/overlay/Overlay.svelte:17` -- `DUMMY_NEXT_ACTION` 定数と `TODO(CAP-2)`。本スライスが埋める枠そのもの
- `src/overlay/Overlay.svelte:6-14` -- `HotkeyStatus` / `OverlaySnapshot` の TS 型。Rust 側と 1:1。拡張時は両方を同時に変える
- `src/overlay/Overlay.svelte:37-53` -- スナップショット取得の再試行 (10 回 × 100ms)。webview が Rust の `setup` より先に動くため。新コマンドも同じ扱いが要る
- `src/overlay/Overlay.svelte:63-99` -- `close()` と `onFocusChanged`。**フォーカス喪失で閉じる** — メモ入力中の離脱がここを通る
- `src/overlay/Overlay.svelte:80-85,107` -- 現在のキー処理は Escape のみ。`<svelte:window on:keydown>` で window 全体に掛かる
- `src/app.css:25` -- `user-select: none` が全体に掛かる。テキスト入力欄では解除が要る
- `src/overlay/Overlay.test.ts:24-76` -- `mockIPC` / `mockWindows` / `stubEventInternals` / `settle` の作法。`shouldMockEvents: true` が既に有効
- `src-tauri/src/commands/mod.rs:52-56` -- `OverlaySnapshot` の serde 形。`camelCase`。`tests::the_snapshot_keeps_its_wire_contract` が形を固定している
- `src-tauri/src/commands/mod.rs:59-66` -- 未解決時に `Err` を返して再試行させる作法。新コマンドも既定値で埋めない
- `src-tauri/src/domain/state.rs:273,289` -- `set_interruption_note` と `move_current_position` は**別々のコミット**である。AD-5 を満たすには両者を 1 トランザクションに束ねる新しい経路が要る
- `src-tauri/src/domain/state.rs:388-392` -- 判断 → 永続化 → メモリ反映。新しい操作もこの順序に従う
- `src-tauri/src/ports/storage.rs:50-56` -- `Commit { task, current_position }`。**切り替え履歴**の欄をここに足す
- `src-tauri/src/adapters/storage/schema.rs:64` -- `MIGRATIONS` は追記専用。`V1` を書き換えず `V2` を足す
- `src-tauri/src/lib.rs:199-202` -- `Core` 不在時の契約 (`try_state`)。コマンドはこれに従う
- `src-tauri/src/domain/position.rs:97` -- `move_to` は活性のまま移動したとき `activated_at` を保つ。**切り替え**でリセットしないのはここが担保する

## Tasks & Acceptance

**Execution:**
- [x] `src-tauri/src/domain/switch.rs` -- `SwitchRecord` (id・離脱元 `StepId`・`occurred_at`・`note_written`) を定義 -- 用語集の識別子に 1:1 (AD-10)。**本文を持たせない** (AD-15)
- [x] `src-tauri/src/domain/state.rs` -- `Core` に**切り替え**の経路を 1 本足す。離脱側のメモ確定・完了宣言 (任意)・次の**ステップ**への移動・履歴の追記を**一つの `Commit`** にまとめる -- AD-5。既存の `set_interruption_note` / `move_current_position` を順に呼ぶ実装にしない
- [x] `src-tauri/src/ports/storage.rs` -- `Commit` に `switch_record` の欄を足す -- 1 メソッド = 1 トランザクションを保つ
- [x] `src-tauri/src/adapters/storage/schema.rs` -- `V2` として `switch_record` 表を追加し `MIGRATIONS` へ**追記** -- `V1` は絶対に書き換えない
- [x] `src-tauri/src/adapters/storage/mod.rs` -- `apply` の同一トランザクション内に履歴の書き込みを足す -- 読み戻しは実装しない (書くだけ)
- [x] `src-tauri/src/commands/mod.rs` -- `OverlaySnapshot` に**次の一手**・位置情報・**中断メモ**を足し、`switch_current_position` を追加 -- AD-3 の命名。`try_state` で `Core` 不在を明示的なエラーにする。既存の `hotkey` 欄と 3 コマンドを壊さない
- [x] `src-tauri/src/lib.rs` -- 新コマンドを `invoke_handler` へ登録し、**切り替え**確定後に `current_position_changed` を発行 -- AD-3「event を伴わない状態変更を作らない」
- [x] `src/overlay/Overlay.svelte` -- 既定表示 (次の一手・位置情報・メモ)、メモ入力欄 (既存メモで初期化・カーソル末尾)、切り替えと完了の打鍵、`current_position_changed` の購読 -- CAP-2/7/8。**再描画は必ずスナップショットの取り直しを経由する** (AD-3 鮮度規則)
- [x] `src/app.css` または当該コンポーネント -- 入力欄に対してのみ `user-select` を解除 -- 全体設定 (`app.css:25`) が入力を妨げるため
- [x] `src/overlay/Overlay.test.ts` -- 既定表示・メモ初期化・省略時の切り替え・完了つき切り替え・打鍵数・イベント受信時の再取得を覆う -- 既存の `mockIPC` の作法に従う

**Acceptance Criteria:**
- Given 第 3/全 6 を指す**現在地**、when オーバーレイを開く、then その**ステップ**内容と「第 3 ステップ / 全 6 ステップ」が現れ、他の**ステップ**内容は現れない
- Given 記録済みの**中断メモ**を持つ**ステップ**が**現在地**、when オーバーレイを開く、then 追加操作なしにメモが読める
- Given メモを空のまま確定した、when **切り替え**が完了する、then **現在地**は次の**ステップ**へ移り、履歴の `note_written` は偽である
- Given メモを書いて確定した、when 再起動する、then そのメモが残っており、**現在地**は移動後の**ステップ**である
- Given 完了を伴う**切り替え**、when DB を見る、then 離脱側に `completed_at` が付き、**現在地**が移動しており、履歴が 1 行増えている (三者が同時に成立)
- Given メモ入力中にフォーカスを失った、when 再度開く、then 下書きは残っておらず、**現在地**も動いていない
- Given 実装完了、when 呼び出しからメモ確定・離脱までの打鍵を数える、then 5 打鍵以内である
- Given 実装完了、when `make lint` と `make test` を実行する、then いずれも成功する
- Given 実装完了、when `domain/` を検索する、then `tauri::`・`rusqlite::`・`SystemTime::now` のいずれも現れない
- Given 実装完了、when UI の描画対象を確認する、then **切り替え履歴**に由来する値がどこにも現れない (AD-15)

## Implementation Notes

- **`Commit` に欄を足し、`switch_current_position` を 1 本の経路にした。** `Core::switch_current_position` はタスクの複製の上で「メモの確定 → 完了の宣言 → 移動先の決定 → 履歴の起票」を済ませ、`Commit { task, current_position, switch_record }` を**一度だけ** `apply` する (`domain/state.rs`)。既存の `set_interruption_note` / `move_current_position` は呼ばない。
- **移動先が無いときも履歴を 1 行残す。** 最終**ステップ**からの**切り替え**では `current_position` を `None` にした同じコミットを書く。履歴を落とすと、最終**ステップ**で書いたメモだけが SM-C3 の分母から消え、記入率が歪む。
- **`next_step_of` は完了済みを読み飛ばさない。** 読み飛ばせば**完了**が移動先を決めることになり、「**完了**を**現在地**から導出しない」(FR-4 / AD-2) の裏返しになる。
- **`SwitchRecord` は `serde::Serialize` を実装しない。** コマンド境界へ出せる形が存在しないため、履歴を画面へ運ぶ経路が型として成立しない (AD-15)。`OverlaySnapshot` の欄の集合もテストで固定した。
- **`NextAction` の型を作らなかった。** 用語集が「v1 では型を持たない」と定めるため (AD-10)、`OverlaySnapshot` に `stepContent` / `stepOrdinal` / `stepCount` / `interruptionNote` を平らに並べた。比率は運ばない。
- **打鍵は 2 打鍵に収めた。** 呼び出し (1) → Enter (1)。オーバーレイは開いた時点で入力欄に既存メモを入れてフォーカスし、カーソルを末尾に置く。`⌘Enter` が**完了**を伴う**切り替え**であり、同じ一連の操作から 1 打鍵で到達する。
- **IME の変換確定を切り替えと取り違えない。** `event.isComposing` (と `keyCode === 229` の保険) で弾く。`Shift+Enter` は改行として残した。
- **報せは `refresh()` で畳まない。** 自分が確定させた**切り替え**も `current_position_changed` を招き、その購読が `refresh()` を非同期に走らせる。畳む契機は「新しい呼び出し」と「次の確定要求」だけである (`dismissNotices`)。
- **`Core` 不在は明示的なエラーにした。** `try_state` の `None` を利用者向けの 1 文へ変換し、オーバーレイがそれを描く。deferred-work.md の「復元の失敗が利用者にまったく伝わらない」は、可視面の側だけこれで埋まった (破損 DB の隔離・作り直しは依然として無い)。
- **`Core` 不在の判定を `require_core` に切り出した。** 呼び出し側 (`get_overlay_snapshot` / `switch_current_position`) に埋め込んだままでは、生きた Tauri アプリを起動しない限り I/O マトリクス「コア不在」の行を検証できず、経路を丸ごと消しても検査が通ってしまう。`should_prevent_exit` / `toggle_action` と同じ流儀の純粋関数にし、不在・存在の両方を単体テストで固定した。
- **`restore_core` を `setup` の中で前へ動かした。** ホットキー登録失敗時の起動直後オーバーレイより先にコアを `manage` する。後ろのままだと、その報せに「状態を読み込めていない」という別の失敗が被さる競合が残る。
- **提示されたメモを送り返さない。** オーバーレイは直近のスナップショットが運んできたメモ (`notePrefill`) を覚えており、下書きがそこから動いていなければ `note: null` を送る。読み返しただけの**切り替え**が記入として数えられれば、SM-C3 の記入率が膨らみ、偽陽性を排除するための指標がその役目を失う。
- **何も変わらない切り替えは履歴を残さない。** 最終**ステップ**で Enter を繰り返すだけで 1 行ずつ積めば、同じ指標が逆向きに歪む。`task_changed || moved.is_some()` のときだけ書く。メモが書かれていれば移動先が無くても残す。
- **コア不在で `get_overlay_snapshot` を失敗させない。** 失敗させるとホットキーの登録結果まで道連れになり、DB が開けずホットキーも死んでいるとき、唯一の呼び出し経路が死んでいる事実が伝わらない (Never「`hotkey` を失わせない」)。`stateError` として運び、**未着手**とは別に描く。
- **投影を純粋関数 `snapshot_of` に切り出した。** **ステップ**とそれを含む**タスク**を一度の引き当てから取るため、「内容は出ているが全体数だけ空」が表現できない。第 3/全 6 と**未着手**を単体テストで固定した。
- **`switch_current_position` の引数を `SwitchRequest` 型にした。** 平らな仮引数では、改名しても両方の検査が緑のまま実行時に毎回の Enter が引数の復元で落ちる。型にすれば `{"note":…,"declareCompletion":…}` の復元をテストで固定できる。
- **`current_position_changed` は動いたときだけ発行する。** 最終**ステップ**の経路で発行すれば、起きていない変化を主張することになる。その経路の再描画は呼び出し側の取り直しが担う。
- **IME の変換中は Esc も含めて一切割り込まない。** 変換取り消しの Esc でオーバーレイが閉じ、下書きが消えるのを防ぐ。確定の途中 (`switching`) でも Esc を無視する — 失敗の理由を読む機会が画面ごと消えるため。
- **イベント由来の取り直しは下書きに触れない。** 下書きを捨ててよいのはフォーカス離脱と Esc だけである (I/O マトリクス「入力途中の離脱」)。
- **受け付ける修飾キーを案内に合わせた。** `⌘Enter` のみ。Option / Control を伴う Enter は無視する。
- **`switch_record` に索引を張らない。** 読み手が製品コードに一つも無く、**切り替え**のたびの書き込みを重くするだけである。
- **オーバーレイの高さを 220 → 280 に広げた** (`tauri.conf.json`)。入力欄と操作行が `overflow: hidden` の器に収まらないため。

## Spec Change Log

## Review Triage Log

第 1 回レビュー (blind-hunter / edge-case-hunter / verification-gap)。intent_gap・bad_spec は無く、ループバックは発生していない。

| # | 出所 | 所見 | 判定 | 根拠 | 経路 |
|---|---|---|---|---|---|
| 1 | BH / EC / VG | 既存メモを差し戻すだけで `note_written` が真になり、SM-C3 の記入率が水増しされる | high | `Overlay.svelte:78` が `noteDraft` を保存済みメモで初期化し、`confirmSwitch` がそれをそのまま送る。`state.rs:363` は `note.is_some()` で判定するため、何も書かずに Enter を押しても「書いた」と記録される。記入率は SPEC が偽陽性の成功判定を防ぐために置いた三つの逆指標の一つであり、そこが壊れる | patch |
| 2 | BH / EC | `Core` 不在時にホットキーの失敗通知ごと消える | high | `commands/mod.rs` は `hotkey` を解決した**後**に `require_core(...)?` で全体を `Err` にする。DB が開けず、かつホットキー登録も失敗した複合故障で、slice 1 が保証した「ホットキーが死んでいることは確実に伝わる」が破れる。本 spec の Never「`hotkey` を失わせない」からの直接の逸脱でもある | patch |
| 3 | VG / BH | `CoreState` → `OverlaySnapshot` の射影を実行するテストが一つも無い | high | `the_snapshot_*` は手で組んだ構造体を serialize するだけで `get_overlay_snapshot` を呼ばない。frontend は IPC をモックするため Rust 本体に到達しない。`step_ordinal` と `step_count` を入れ替えても全件緑であることを検証者が実演済み。FR-2 が唯一許した例外である位置情報が誤り得る。`step_count` が独立した `Option` 検索である (BH) ため内容ありで件数だけ欠ける形も同根 | patch |
| 4 | VG | コマンドの**要求側**の wire contract が両側の自作ミラーでしか検証されていない | high | Rust 側は `commands::switch_current_position` を一度も呼ばず、vitest は自分が渡した引数を照合しているだけ。`note` を `interruption_note` に改名しても両者緑のまま、実行時は毎回の Enter が引数の復元に失敗する。本スライスが存在する理由そのものの 1 打鍵が、改名一つで静かに壊れる | patch |
| 5 | VG | `switch_record` に実際に書かれた値を読み戻す検証が無い | medium | 4 つのテストと補助関数はすべて件数だけを見る。`note_written` を定数 1 にしても、`occurred_at` を ISO 8601 以外で書いても全件緑。この表の唯一の存在理由が測定であるため、壊れても将来の SQL でしか露見しない | patch |
| 6 | BH | Escape が IME ガードより先に処理される | medium | `onKeydown` は `Escape` の分岐を `isComposing` の検査より前に置く。日本語変換中の Escape は変換の取り消しであり、ここではオーバーレイが閉じて下書きが消える。Enter だけ IME から守り、もう半分が開いている | patch |
| 7 | BH / EC | イベント受信時の再取得が入力中の下書きを上書きする | medium | `listen(CURRENT_POSITION_CHANGED)` が無条件に `refresh()` を呼び、`refresh()` は `noteDraft` を書き換えてカーソルを移す。I/O マトリクスが下書きの破棄を認めているのは blur と Esc のときだけである | patch |
| 8 | BH / EC | 現在地が動いていないときにも `current_position_changed` を発行する | medium | 最終ステップの経路では `commit.current_position` が `None` でありながらイベントだけが出る。起きていない変化を宣言している | patch |
| 9 | BH / EC | 再試行を使い切った後も古い**現在地**が画面に残り、その上で切り替えを撃てる | medium | `refresh()` は `snapshotError` を立てるが `stepContent` 等を消さない。切り替えの可否は `stepContent === null` しか見ていないため、確認できない位置に対して Enter が通る | patch |
| 10 | BH | 5 打鍵のテストが構造的に失敗しえない | medium | テストが自分で持つカウンタを加算して自分で検査している。確定を二段階にしても通る。slice 1 が「所見 #7 トートロジーなテストを作らない」として明示的に禁じた形である | patch |
| 11 | VG | 最終ステップの通知が、完了を宣言していない切り替えでも「完了は記録した」と述べる | medium | 文言が無条件であり、平の Enter でも ⌘Enter でも同じ。オーバーレイが閉じない唯一の経路であり、この文が唯一の手掛かりである | patch |
| 12 | EC | 最終ステップで Enter を繰り返すと、何も変わらないまま履歴だけが増え続ける | medium | 移動も無く、メモも完了も変化しない要求で `SwitchRecord` が 1 行ずつ増える。記入率の分母が膨らみ、#1 とは逆向きに SM-C3 を歪める | patch |
| 13 | EC | 切り替えの往復中に Esc を押すと、失敗の通知を見る前に閉じる | low | 直接的な追加 1 行で塞がる | patch |
| 14 | EC / BH | Alt+Enter でも切り替わり、ヒントは ⌘Enter と書きながら Ctrl も受け付ける | low | 修飾キーの取り扱いと表示が食い違う。Ctrl 経路のテストも無い。修正は直接的な訂正である | patch |
| 15 | BH | 二つの通知が `role="alert"` を持たず、フォーカスは textarea に残る | low | 直接的な属性追加で塞がる | patch |
| 16 | BH | `switch_record_by_time` は読み手のいない索引であり、書き込みのたびに費用だけが増える | low | 製品側に読み出し経路が存在しないことを schema と switch.rs の双方が明記している。削除または根拠の明記という直接的な対処が可能 | patch |
| 17 | EC | 新しいテストが `AppHandle::exit(0)` の doc コメントと既存テストの間に挿入され、双方の説明がずれた | low | 位置の移動という直接的な訂正で済む | patch |
| 18 | BH / EC | 記録済みの**中断メモ**を消す経路が無い (空欄は「省略」と解釈され、古いメモが残り続ける) | medium | 実装は仕様どおりである — 凍結された I/O マトリクスが「メモを省いて切り替え → メモは変更しない」と明示的に決めている。削除の意味付けは編集の面を持つスライスが決めるべきであり、本スライスの凍結された Intent はその面を除外している | defer |
| 19 | EC | **非活性** (休息中) の**現在地**から切り替えると、休息が黙って終わり連続作業時間の起点も更新される | medium | `move_to` は `Inactive` を `Active` へ遷移させるため `activated_at` が更新される。ただし `deactivate_current_position` を公開するコマンドが無く、今日この状態へ到達する経路は手編集の DB 以外に存在しない。休息の意味付けは CAP-10 に属し、本 spec の Never がそれを除外している | defer |
| 20 | BH | `switch_record` に保持期間の方針も `ON DELETE` の方針も無い | medium | 今日は削除経路が存在しないため無害だが、CAP-20 (消滅の経路) が**ステップ**の削除を導入した時点で外部キーと衝突する | defer |
| 21 | EC | メモの長さに上限が無く、貼り付けた長文がそのまま永続化・描画される | low | 単独利用のツールであり日常の利用で遭遇する見込みが低い。修正は新たなエラー種別と、マトリクスが示していない閾値の導入を伴う複雑化である | 棄却 |


## Design Notes

**なぜ `set_interruption_note` と `move_current_position` を順に呼んではならないか。** 両者はそれぞれ独立した `Commit` を書く。順に呼べば書き込みは二つのトランザクションに分かれ、その間で異常終了すると「メモは残ったが**現在地**が動いていない」あるいはその逆が残る。AD-5 が単一トランザクションを要求しているのはまさにこの状態を禁じるためである。新しい経路は一つの `Commit` に**タスク** (メモと完了を含む)・**現在地**・**切り替え履歴**を載せる。

**なぜ履歴を読み戻さないのか。** **切り替え履歴**は SM-C3 (メモ記入率) の測定基盤であり、利用者に表示しない (AD-15)。コアが起動時に読む理由が無く、読めばメモリ上に表示されうる値を置くことになる。書くだけにして、測定は必要になった時点で SQL から直接行う。

**イベントのペイロードを最小にする理由。** スパインはペイロード形式を未決として「最初のストーリーで確定させる」としている。AD-3 の鮮度規則により、オーバーレイは表示のたびにスナップショットを取り直す。したがってイベントが状態を運ぶ必要はなく、運べば「イベント経由の状態」と「スナップショット経由の状態」という二つの真実が生まれる。`current_position_changed` は変化したという事実だけを伝え、受け手は必ずスナップショットを取り直す。

## Verification

**Commands:**
- `make test` -- expected: 既存 118 件の Rust テストと新規テスト、および vitest がすべて成功する
- `make lint` -- expected: `cargo fmt --check` と `clippy --all-targets -- -D warnings` が警告なしで成功する
- `rg -n 'tauri::|rusqlite::|SystemTime::now' src-tauri/src/domain/` -- expected: 一致なし
- `sqlite3 "$HOME/Library/Application Support/dev.onzuka.mytaskmanager/state.sqlite3" 'PRAGMA user_version;'` -- expected: `2`
- `sqlite3 "$HOME/Library/Application Support/dev.onzuka.mytaskmanager/state.sqlite3" 'SELECT count(*) FROM switch_record;'` -- expected: 切り替えの回数と一致する

**Manual checks (if no CLI):**
- 作成の面がまだ無いため、`sqlite3` で**タスク**・**ステップ**・**現在地**を投入してから `make install && make open` し、呼び出し → メモ入力 → 確定までの打鍵を実際に数える (5 打鍵以内)。確定後に `switch_record` が 1 行増え、`current_position` が次の**ステップ**を指すことを SQL で確認する
