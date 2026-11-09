---
title: '待機 — 主作業を戻り先として覚えたまま短い別作業をし、時間が来たら戻るきっかけを出す'
type: 'feature'
created: '2026-11-09'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: '8ce6562e2195c52ddf11d77479d22e94bca039d6'
context:
  - '{project-root}/_bmad-output/specs/spec-my-task-manager/glossary.md'
  - '{project-root}/_bmad-output/planning-artifacts/architecture/architecture-my-task-manager-2026-09-15/ARCHITECTURE-SPINE.md'
  - '{project-root}/_bmad-output/implementation-artifacts/spec-return-intervention.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** AI の出力待ちなどで主作業を離れ、短い別作業 (情報収集・AI への相談・別タスクの小さなステップ) をすることが多い。今は別タスクへ切り替えると**現在地**が動いて主作業が「戻る先」でなくなり、**休息**にすると作業していないことになる。戻り忘れれば出力が読まれないまま放置される。

**Approach:** **待機 (Wait)** を足す。⌘D で入ると、その時点の**現在地**を**戻り先**として覚える。待機中は別作業をしてよく、**現在地**を一覧から別タスクへ移しても戻り先は変わらない。**待機の長さ** (既定 5 分) が経つと、休息・復帰と同じ非活性パネルで「主作業に戻る」「あと少し」を出す。戻ると**現在地**が戻り先へ戻り、オーバーレイが開いて主作業の**中断メモ**の欄が出る。

## Boundaries & Constraints

**Always:**
- 入る: 既定表示 (作業中・**現在地**あり・**休息**中でない) で ⌘D → 待機の面 (分数の欄に既定値、「何を待っているか」の一行の欄は任意) → Enter で確定、Esc で戻る。分数が正の整数でなければ確定しない (理由を 1 行)。**連続作業時間**は数え続ける。
- 待機中の既定表示: 次の一手に「待機中 — <何を待っているか>」(空なら「待機中」) を 1 行添える。**残り時間・経過を出さない。** 案内に「⌘D で主作業に戻る」。
- 早めに戻る: 待機中の既定表示で ⌘D。中断メモの欄に未確定の下書きがあり、**現在地**が戻り先と違えば、その下書きは離れる側の**ステップ**の**中断メモ**として確定する (切り替えと同じ扱い)。
- 「主作業に戻る」(パネルの `Ctrl+Opt+R` / ⌘D): **現在地**が戻り先と違えば戻り先へ移し (**切り替え履歴**を 1 行。パネル経由は `note_written` 偽)、同じなら書かない。待機を終え、オーバーレイを開いて既定表示 (主作業の次の一手と**中断メモ**の欄) を見せる。
- 「あと少し」(`Ctrl+Opt+G`): 待機の長さ (確定時の分数) の後に出直す。回数の上限は無い。
- 介入は同時に一つ (AD-7)。休息・復帰の介入が表示中なら待機の介入は待つ。
- 待機の長さの既定は `setting` の `wait_length_seconds` (既定 300)。無い・0 以下・数値でなければ 300。待機の面の分数の初期値はこれを分に直した値。
- **決定 (利用者の裁定): 待機中に休息へ入ると (⌘R・休息の介入とも)、先に現在地を戻り先へ戻してから休息に入り、待機を終える。** 休息明けに主作業へ戻れるようにするため。
- **決定 (利用者の裁定): 戻り先・待機の期限・何を待っているかは永続化しない** (AD-2「休息の開始時刻」と同じ扱い)。待機中に常駐を終えると待機は消え、**現在地**はその時点のまま残る。
- **決定 (利用者の裁定・レビュー後): 待機が終わったら、表示中の待機の介入を取り下げてパネルを隠す。** 早めに戻る (⌘D)・休息に入る (⌘R・休息の介入) のどちらで終わっても同じ。取り残したパネルが他の介入を塞いだり、休息中にオーバーレイを開いたり、新しい待機を終えたりしない。
- **決定 (利用者の裁定・レビュー後): 「主作業に戻る」(パネル・⌘D・休息に入るときの戻し) は、現在地が戻り先と同じタスクの中にあれば動かさない。** 待機を終えるだけで、切り替え履歴も書かない。別のタスクにいるときだけ、覚えたステップへ戻す。
- PRD (介入の契機に待機を足す)・スパイン AD-2 (戻り先と待機の状態の行)・AD-15 (介入の種類)・用語集 (両表)・SPEC.md・README・`docs/` を改訂する。用語集に「将来タスクごとの作業時間を記録するときは待機中を主作業の時間に数えない」を申し送りとして書く。

**Never:**
- 待機の残り時間・経過・回数をどこにも出さない (AD-15)。
- AI の完了などの外部の出来事を契機にする。契機は時間だけである。
- 待機を時間で自動終了する。促すだけである。
- **連続作業時間**を止める・リセットする。
- 新しいホットキー・新しいパネル (webview) を足す。休息・復帰と同じ二つとパネルを使う。

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|---|---|---|---|
| 入る | 主作業 第 3 で ⌘D →「AI の出力」→ Enter | 待機中。既定表示に「待機中 — AI の出力」。現在地は第 3 のまま | N/A |
| 分数が不正 | 分数「0」「abc」で Enter | 確定しない。理由を 1 行 | N/A |
| 別タスクへ | 待機中に一覧で別タスクのステップを選ぶ | 現在地が移る。戻り先は主作業 第 3 のまま | N/A |
| 時間が来る | 入って 5 分後の刻み | パネル「主作業に戻る / あと少し」。入力先は奪わない | 提示失敗なら取り下げて待機の長さの後に出直す |
| 戻る (移っていた) | 別タスクにいる状態で Ctrl+Opt+R | 現在地が主作業 第 3 へ。切り替え履歴 1 行。オーバーレイが開きメモ欄が出る | 書き込み失敗なら待機は続き、理由をログ |
| 戻る (移っていない) | 主作業のままで Ctrl+Opt+R | 何も書かない。待機を終え、オーバーレイが開く | N/A |
| あと少し | Ctrl+Opt+G | 5 分後に出直す | N/A |
| 早めに戻る | 待機中に ⌘D (別タスクのメモを書きかけ) | 書きかけのメモを別タスクのステップに確定し、主作業へ戻る | 失敗なら移らず理由を 1 行 |
| 主作業を進めていた | 第 3 で待機 → 第 4 へ切り替え → Ctrl+Opt+R | 第 4 のまま。何も書かない。待機を終え、オーバーレイが開く | N/A |
| 早めに戻った後のパネル | 待機の介入が表示中に ⌘D で戻る | パネルが消える。以後、休息の介入は通常どおり出る | N/A |
| 休息に入った後のパネル | 待機の介入が表示中に ⌘R | パネルが消え、休息に入る | N/A |
| 休息と重なる | 待機中に休息の介入が表示中で時間が来る | 待機の介入は休息の介入への応答の後に出る | N/A |
| 待機中に休息 | 別タスクにいる待機中に ⌘R | 現在地を主作業へ戻して休息に入り、待機を終える | N/A |
| 入れない | 休息中・未着手で ⌘D | 何もしない | N/A |
| 設定 | `wait_length_seconds` = 60 | 面の分数の初期値 1、1 分で出る。不正なら 5 分 | N/A |

</frozen-after-approval>

## Code Map

- `src-tauri/src/domain/rest.rs:150-212` -- `InterventionKind` に `Wait` を足し、`InterventionChoice` の表に列を足す。待機の判定は `decide_return` と同形の純粋関数 (`decide_wait`: 待機中か・期限・いま・表示中か)。鍵と既定値の定数 (`WAIT_LENGTH_KEY` / 300) もここ
- `src-tauri/src/domain/state.rs` -- `CoreState` に非永続の `wait: Option<Wait { return_to: (TaskId, StepId), until, length_millis, label }>`。`begin_wait(minutes, label)`・`return_from_wait(note: Option<String>)`・`tick` で `decide_wait` (復帰の判定の後、思いつきの寿命の前。介入を出したら即 return する既存の形)・`answer_intervention` / `withdraw_intervention` に `Wait` の腕。`begin_rest` と休息の介入の「休息に入る」で、待機中なら先に戻り先へ戻す。戻り先への移動は `transition_position` / `select_step` と同じく切り替え履歴を積む経路を再利用する
- `src-tauri/src/adapters/intervention/mod.rs:443-470` -- `after_answer` の表に `Wait`: `Rest` 選択でオーバーレイを開き `current_position_changed` を送る
- `src-tauri/src/commands/mod.rs:120-200` -- `OverlaySnapshot` に `waiting: bool` と `wait_label: Option<String>` (**時刻・残りの欄を持たない**)。`begin_wait` (`BeginWaitRequest { minutes, label }`) と `return_from_wait` (`ReturnFromWaitRequest { note }`) のコマンド。待機の面の初期値のため `wait_length_minutes` を運ぶ方法 (スナップショットの欄で可)。ワイヤ契約テスト
- `src-tauri/src/lib.rs:160-175` -- `generate_handler!` に二つを足す
- `src/intervention/Intervention.svelte:61` -- `WORDING` に `wait`: 問い「主作業に戻る時間です。」、一つ目「主作業に戻る」、二つ目「あと少し」
- `src/overlay/Overlay.svelte` -- ⌘D (`isSurfaceKey` に `'d'`)、待機の面 (分数・一行の欄。思いつきの面の作法)、待機中の 1 行と案内、待機中の ⌘D で早めに戻る (下書きを `note` として送る)
- テスト: `rest.rs` (`decide_wait`)、`state.rs` (I/O マトリクスの各行。手本は復帰の介入の `tick_for`)、`commands` (契約)、`Overlay.test.ts`、`Intervention.test.ts`
- 文書: PRD (介入の節と用語)、`ARCHITECTURE-SPINE.md` AD-2 / AD-15、`glossary.md`、`SPEC.md`、`README.md`、`docs/settings.md` / `docs/architecture.md`
- 変更しない: スキーマ (表を足さない)、思いつき、休息・復帰の判定そのもの

## Tasks & Acceptance

**Execution:**
- [x] `src-tauri/src/domain/rest.rs` -- 種類・判定・鍵と既定値 -- 時計から切り離して検証する
- [x] `src-tauri/src/domain/state.rs` -- 入る・戻る・刻み・応答・休息との重なり -- I/O マトリクスの全行
- [x] `src-tauri/src/adapters/intervention/mod.rs` / `commands/mod.rs` / `lib.rs` -- 応答後の振る舞い・コマンド・スナップショット -- ワイヤ契約を固定する
- [x] `src/intervention/Intervention.svelte` / `src/overlay/Overlay.svelte` と各テスト -- 文言・⌘D の面・待機中の表示・早めに戻る -- I/O マトリクスの各行
- [x] PRD / スパイン / 用語集 / SPEC.md / README / `docs/` -- 改訂 -- 介入の契機と状態表を実装と食い違わせない

**Acceptance Criteria:**
- Given 待機中, when 既定表示・パネル・メニューバー項目を見る, then 残り時間・経過・回数はどこにも現れない
- Given 待機中, when 刻みが進む, then 連続作業時間は待機に入る前と同じく増え続け、休息の介入は通常どおり出る

## Implementation Notes

- **`rest.rs`:** `InterventionKind::Wait` (綴り `"wait"`)、`WAIT_LENGTH_KEY` / `DEFAULT_WAIT_LENGTH_SECONDS` (300)、`RestSettings` に待機の長さ (`wait_length_seconds()` / 面の初期値の `wait_length_minutes()` — 切り上げ・最小 1)。判定は純粋関数 `decide_wait(&WaitTick { waiting, until, now, showing })`。実時間で比べるだけで、スリープ中も含める。
- **`state.rs`:** `CoreState.wait: Option<Wait { return_to, until, length_millis, label }>` (非永続)。`begin_wait(NonZeroU32, Option<String>) -> bool` は何も書かない (活性でない・既に待機中なら `false`)。`return_from_wait(note) -> WaitReturn { NotWaiting, Stayed, Moved }`。戻り先と違えば離れる側のメモ (あれば)・移動・切り替え履歴を 1 コミット。同じならメモを確定せず何も書かない。パネルの「主作業に戻る」は同じ遷移を `note = None` で通る (`note_written` 偽)。`tick` は休息 → 復帰 → 待機 → 思いつきの寿命の順。「あと少し」と取り下げは `until = now + length_millis` (確定時の分数)。
- **休息との重なり:** `begin_rest` と休息の介入の「休息に入る」を共通の `rest_from` へまとめた。待機中なら戻り先への移動と非活性化と切り替え履歴 (`note_written` 偽) を 1 コミットで書き、待機を終える。失敗すれば待機は続く。
- **AD-7:** 自分で早めに戻っても待機の介入のパネルは残す (休息の復帰の介入と同じ扱い)。後からの「主作業に戻る」は何も書かずオーバーレイを開くだけ、「あと少し」は何も予約しない。新しい待機に入った後で古いパネルの「主作業に戻る」を選ぶと新しい待機から戻る — 復帰の介入と同じ受け入れたリスクとして README に明記した。
- **commands:** `OverlaySnapshot` に `waiting` / `waitLabel` / `waitLengthMinutes` (設定値)。期限・残り時間の欄は無い (契約テストで固定)。`begin_wait` (`BeginWaitRequest { minutes: String, label }`、分数の検めは純粋関数 `as_wait_minutes` に一箇所) と `return_from_wait` (`ReturnFromWaitRequest { note }` → `{ returned, moved }`)。`begin_wait` はイベントを発行しない (現在地が動かず、描く面はオーバーレイだけで呼び出し側が取り直す)。`return_from_wait` は移ったときだけ `current_position_changed`。
- **adapter:** `after_answer` で Wait × Rest はオーバーレイを開き `current_position_changed` を送る。Wait × Grace は何もしない。
- **Overlay:** ⌘D は作業中なら待機の面 (分数・一行。入力位置は一行の欄)、待機中なら早めに戻る (`noteIsDirty` のときだけ下書きを送る。移らなければ下書きを残す)。休息中・未着手では何もしない。既定表示に「待機中 — …」の 1 行、案内に「⌘D で待機」/「⌘D で主作業に戻る」、ボタン「待機」/「主作業に戻る」を足した。AD-2 の揮発ビュー状態に「待機の下書き」の行を足した (6 行)。
- **文書:** PRD §3 / §4.6、スパイン AD-2 / AD-10 / AD-15、用語集 (両表・申し送り)、SPEC.md (CAP-23)、README、`docs/settings.md` / `docs/architecture.md`。

- **レビュー後の修正:** 待機が応答以外 (⌘D・休息に入る) で終わると、コアが表示中の待機の介入を取り下げ、一度だけ読める印 (`take_withdrawn_wait_panel`) を立てる。`begin_rest` / `return_from_wait` コマンドは `intervention::hide_withdrawn` でホットキーを解除しパネルを隠す。戻り先への移動は別のタスクにいるときだけ (同じタスクの中なら `Stayed`)。`wait_length_minutes` は溢れず `MAX_WAIT_MINUTES` (1 日) で頭打ち、`begin_wait` も同じ上限。全角数字の分数を読む。面の初期値はスナップショットだけが出所。PRD の待機の節を FR-22 とし、SPEC CAP-23 の印も FR-22 に揃えた。

## Spec Change Log

## Review Triage Log

| # | 層 | 指摘 | 判定 | 根拠 | 行き先 |
|---|---|---|---|---|---|
| 1 | EC / BH | 待機が終わっても待機のパネルが残り、休息・復帰の介入を塞ぎ、休息中の応答でオーバーレイを開き、新しい待機を終えうる | medium | `leave_wait` と休息の経路は `intervention` に触れず、`after_answer(Wait, Rest)` は結末を見ない。凍結された意図はパネルの扱いを決めていなかった | intent_gap → 利用者の裁定 (終わったら取り下げる) を凍結ブロックへ。差分全体は作り直さず、該当箇所の修正として実装担当へ返した |
| 2 | BH | 待機中に主作業の中で進めた (第 3 → 第 4) のに、戻ると第 3 へ戻され、起きていない切り替え履歴が書かれる | medium | `return_move` は戻り先のステップと一致するかだけを見る。凍結された意図は主作業内の移動を扱っていなかった | intent_gap → 利用者の裁定 (同じタスク内なら動かさない) を凍結ブロックへ。同上 |
| 3 | BH | 完了済みの戻り先ステップへ戻される | low | #2 の裁定で同じタスク内の場合は消える。別タスクから完了済みの戻り先へ戻るのは「覚えたステップへ戻す」の字義どおり | #2 に吸収 |
| 4 | BH | `WaitReturn::Stayed` の文書が、現在地が無い・戻り先が見つからない場合を含むことと食い違う | low | `return_move` が三つの場合に `None` を返す | patch |
| 5 | EC / BH | `wait_length_minutes` が巨大な設定値で桁あふれする (debug では panic) | low | 素の `+ 59`。隣は `saturating_mul` を使う | patch |
| 6 | BH | 既定の 5 分が三箇所に直書き | low | 定数・`$state(5)`・`?? 5` | patch |
| 7 | BH | 日本語入力の全角数字 (「１０」) が拒まれる | low | IME を入れたまま打つと日常的に起きる | patch |
| 8 | BH | 分数の欄への行き方が案内に無く、分数を変える検査も無い | low | 入力位置は一言の欄から始まる | patch |
| 9 | BH | 長い「何を待っているか」が窓から案内とボタンを押し出す | low | `.waiting` に折り返し・省略の規則が無い | patch |
| 10 | VG | `begin_wait` の拒否・`return_from_wait` の空振りの表示が未検査 | low | 模擬の切り替えはあるが、どのテストも拒否の枝に入らない (VG が実証) | patch |
| 11 | BH | CAP-23 の FR 付け・§4.6 の UJ・AD-10 / 用語集の実在しない `WaitLength`・docs の event の記述 | low | いずれも文書の直接の修正 | patch |
| 12 | VG | `forgetPosition` の `waiting = false` が未検査 | low | 同じ形の `resting = false` も未検査。古いスナップショットの検査として両方まとめて扱うのがよい | defer |
| 13 | EC | `u32::MAX` を超える分数が「正の整数で」と拒まれる | low | 日常で起きない。直しは分岐の追加 | reject |
| 14 | EC | フロントの模擬の正規表現が `+5` を拒むが Rust は受ける | low | 日常で起きない。模擬は Rust の拒否の文言を返すための仮置き | reject |
| 15 | BH | ⌘D が他の確定中 (休息の宣言中など) を見ない | low | 打鍵の競走は日常で起きにくい。直しは条件の追加 | reject |
| 16 | BH | メニューバー項目に残り時間が出ないことの検査が無い | false | メニューバー項目は待機を一切知らない (差分に `menubar` の変更が無い) | reject |
| 17 | BH | 仕様書の状態・ログが未完了 | false | 差分の採取後に更新済み。直しは仕様の編集でもある | reject |

## Verification

**Commands:**
- `make test` -- expected: cargo test / pnpm check / pnpm test がすべて通る
- `make lint` -- expected: 警告なし

**Manual checks (if no CLI):**
- `sqlite3` で `wait_length_seconds` を 60 にして `make dev` → ⌘D で入り、一覧で別タスクへ移り、1 分後のパネルで Ctrl+Opt+R → 主作業へ戻りメモ欄が出ること
