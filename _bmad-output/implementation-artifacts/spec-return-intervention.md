---
title: '復帰の介入 — 休息が一定時間を超えたら、入力先を奪わずに復帰を促す'
type: 'feature'
created: '2026-10-19'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: 'bdca3a9bbefebdd71013fc9905a2e9c4a31363ab'
context:
  - '{project-root}/_bmad-output/specs/spec-my-task-manager/glossary.md'
  - '{project-root}/_bmad-output/implementation-artifacts/spec-rest-intervention.md'
  - '{project-root}/_bmad-output/planning-artifacts/architecture/architecture-my-task-manager-2026-09-15/ARCHITECTURE-SPINE.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** **休息介入**は休息に入るところまでしか働かない。**休息**の終了は利用者の宣言だけで起き、休息が長引いても誰も知らせないため、戻るきっかけを自力で作る必要がある — 休息を自力で管理できていないという JTBD の半分が残っている。

**Approach:** **休息**に入ってから**休息の長さ** (既定 10 分) が経つと、既存の介入パネルで「休息を終える」「もう少し休む」の二択を出す。「休息を終える」は**休息**を終わらせ (**活性**へ戻し**連続作業時間**を数え直す)、オーバーレイを開いて**次の一手**を FR-8 の形で見せる。

## Boundaries & Constraints

**Always:**
- **休息の終了は利用者の選択によってのみ起きる。** 時間が来ても自動では終わらない。介入は促すだけである (FR-15)。
- 介入は既存の介入パネル (非活性・入力先を奪わない・選ぶまで消えない) をそのまま使う。新しいウィンドウや webview を足さない (AD-6 / AD-14、待機時メモリの余裕は約 2 MB)。
- 同時に出る介入は一つだけ (AD-7)。表示中に別の契機が成立しても待たせる。閉じる経路は応答だけであり、他の経路から取り下げない。
- 応答のホットキーは既存の二つを使い回す (AD-7: 介入ごとのホットキーを作らない)。`Control + Option + R` = 「休息を終える」、`Control + Option + G` = 「もう少し休む」。
- 「もう少し休む」は**休息の長さ**の後に出直す。回数の上限は設けない (休息介入の猶予と同じく本人の身体の話である)。
- **休息の長さ**は `setting` 表の `rest_length_seconds` で上書きでき、無い・不正なときは既定の 600 秒 (AD-11)。
- **休息の開始時刻はコアがメモリ上に持ち、永続化しない** (猶予の期限と同じ扱い)。起動時に**休息**中であれば、起動時刻を開始とみなす。AD-2 の状態表にこの行を足す。
- 経過は実時間で数える。スリープ中の時間も**休息**に含める (スリープは離席である)。
- パネルに残り時間・経過時間・休息の長さを描かない。オーバーレイの休息中の表示も変えない (AD-15 / spec-rest-intervention の Never)。
- **決定 (利用者の裁定、レビュー後): 自分で休息を終えても復帰の介入のパネルは残す (AD-7 のまま)。** その結果として (1) パネルが応答されるまで休息介入が出ない、(2) 新しい休息に入った後で古いパネルの「休息を終える」を選ぶと新しい休息が終わる、の二つを受け入れたリスクとし、README に明記する。
- 「休息を終える」でオーバーレイを開くのは、利用者が明示的に選んだ直後だからである。オーバーレイは通常どおり開き、既定表示 (次の一手・位置情報・中断メモ) を見せる。

**Never:**
- **休息**を時間で自動終了しない。
- 休息介入 (休息に入れる側) の振る舞い・文言・既定値を変えない。
- 通知・音・バッジ・メニューバーの残り時間を足さない。
- 休息の開始時刻のためにスキーマを移行しない。

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|---|---|---|---|
| 復帰を促す | ⌘R で休息に入り 10 分経過 | パネルに「休息を終える」「もう少し休む」が出る。入力先は動かない | N/A |
| 休息を終える | 上の状態で Ctrl+Opt+R またはクリック | **現在地**が**活性**へ戻り、連続作業時間が 0 から数え直され、オーバーレイが開いて次の一手を見せる。パネルは消える | 永続化に失敗したら状態を変えず、パネルも残す |
| もう少し休む | Ctrl+Opt+G | パネルが消え、**休息**は続き、10 分後に再び出る。何度でも | N/A |
| 介入経由で休息 | 休息介入で「休息に入る」を選び 10 分 | 同じく復帰の介入が出る | N/A |
| 先に自分で戻る | 復帰の介入が出る前に Enter で休息を終える | 復帰の介入は出ない | N/A |
| 表示中に自分で戻る | 復帰の介入の表示中にオーバーレイで休息を終える | パネルは残る (AD-7)。後から「休息を終える」を選んでも二重に何も起きずオーバーレイが開くだけ、「もう少し休む」を選んでも何も予約されない | N/A |
| 休息介入の表示中 | 休息介入が出ている間に ⌘R で休息し 10 分 | 休息介入が応答されるまで復帰の介入は出ない | N/A |
| スリープ | 休息中に 30 分スリープして復帰 | 起床後の最初の刻みで復帰の介入が出る | N/A |
| 再起動 | 休息中に常駐を終了し、翌日起動 | 起動から 10 分で復帰の介入が出る | N/A |
| 設定 | `rest_length_seconds` = 300 | 5 分で出る。0 以下・数値でなければ 600 秒 | N/A |

</frozen-after-approval>

## Code Map

- `src-tauri/src/domain/rest.rs:56-110,118-150,223` -- `RestSettings` に休息の長さを足す。`Intervention` は `raised_at` だけで種類を持たない → 種類 (休息 / 復帰) を足す。`InterventionChoice { Rest, Grace }` はパネルの二択。`decide` は非活性で即 return する (:228) ため、復帰の判定は別の分岐か関数が要る
- `src-tauri/src/domain/state.rs:31-51,712,760,793,830,858` -- `CoreState` (介入・猶予期限・直前の刻みは非永続)、`tick`、`answer_intervention`、`withdraw_intervention`、`begin_rest`、`end_rest`。休息の開始時刻と復帰の出直し期限をここに持つ。`restore` (:189) で休息中なら開始を起動時刻にする
- `src-tauri/src/lib.rs:349-390` -- 刻みのスレッド。`effect.raised` で `intervention::raise` を主スレッドで呼ぶ。変更は最小に
- `src-tauri/src/adapters/intervention/mod.rs:369-430` -- `raise` / `answer` / `answer_off_thread`。応答の成功時にだけホットキーを外してパネルを隠す。「休息を終える」の成功後に `presentation::show` (主スレッド) を呼ぶのはここ
- `src-tauri/src/adapters/presentation/mod.rs:214` -- `show`。`lib.rs:139,296` が Rust から呼んでいる手本
- `src-tauri/src/adapters/hotkey/mod.rs:33-36,134,214-240` -- R / G の二つと `response_choice`。種類によらず同じ二つを使う
- `src-tauri/src/commands/mod.rs:1222,1334,1521` -- `InterventionSnapshot` (種類の欄が無い)、`answer_intervention` (Rest 時のみ `current_position_changed`)、ワイヤ契約テスト。欄を足したら TS 側と契約テストを同時に改める
- `src/intervention/Intervention.svelte:155-191` -- 文言と二つのラベルが休息用に固定されている。種類で出し分ける。「数値を描かない」の注釈とテスト (`Intervention.test.ts:113,134`) を守る
- `_bmad-output/planning-artifacts/architecture/architecture-my-task-manager-2026-09-15/ARCHITECTURE-SPINE.md:63` -- AD-2 の状態表。休息の開始時刻 (と復帰の出直し期限) の行を足す
- 手本のテスト: `rest.rs` の `a_tick` / `active_since`、`state.rs:2314` 以降の `a_core_at_work` / `tick_for` (刻みは実際の間隔で進める。時計だけ飛ばすとスリープに見える)

## Tasks & Acceptance

**Execution:**
- [x] `src-tauri/src/domain/rest.rs` -- 休息の長さの既定値・鍵・`RestSettings` の欄、介入の種類、復帰の判定 (純粋関数) を足す -- 判定を時計から切り離して検証するため
- [x] `src-tauri/src/domain/state.rs` -- 休息の開始時刻と復帰の出直し期限を持ち、`begin_rest` / 介入経由の休息 / `restore` で開始を記録し、`tick` で復帰の介入を立て、応答を種類で振り分ける -- I/O マトリクスの全行
- [x] `src-tauri/src/adapters/intervention/mod.rs` / `src-tauri/src/commands/mod.rs` / `src-tauri/src/lib.rs` -- スナップショットに種類を運び、「休息を終える」の成功後にオーバーレイを開き、`current_position_changed` を発行する -- 復帰時に次の一手を見せる
- [x] `src/intervention/Intervention.svelte` -- 種類で文言とラベルを出し分ける -- パネルを使い回す
- [x] 各テスト (`rest.rs` / `state.rs` / `commands` / `Intervention.test.ts`) -- I/O マトリクスの各行と、ワイヤ契約を固定する
- [x] `ARCHITECTURE-SPINE.md` / `README.md` -- AD-2 の状態表に行を足し、README の休息の節に復帰の介入と `rest_length_seconds` を書く

**Acceptance Criteria:**
- Given 実装完了, when `make lint` と `make test` を実行する, then いずれも成功する
- Given 実装完了, when `domain/` を検索する, then `tauri::`・`rusqlite::`・`SystemTime::now` のいずれも現れない
- Given 復帰の介入のパネル, when 描画を確認する, then 数値 (分・秒・残り時間) が一つも現れない

## Implementation Notes

- **判定は `domain/rest.rs::decide_return` (純粋関数)。** 入力は「休息中か・休息の開始時刻・いま・設定・もう少し休むの期限・表示中か」。実時間で比べるだけであり、直前の刻みとの差 (スリープ補正) を見ない — スリープ中も休息に含める。`decide` は非活性で即 return するため、同じ刻みで両方が発することは無い。
- **`Intervention` に `InterventionKind { Rest, Return }` を足した。** `InterventionChoice { Rest, Grace }` は二つのまま、パネルの「一つ目 / 二つ目」として扱い、意味は表示中の種類でコアが決める (Return × Rest = 休息を終える、Return × Grace = もう少し休む)。ホットキーは既存の R / G のまま。
- **`CoreState` に `rest_since` / `return_until` (非永続) を足した。** `begin_rest`・休息介入の「休息に入る」で開始を記録し、`restore` は休息中なら起動時刻を開始とする。それ以外の経路で非活性/活性が変わった場合も `follow_rest` (刻みの冒頭と `transition_position`) が合わせる。
- **`Core::answer_intervention` は `Option<InterventionKind>` を返すようにした** (旧 `bool`)。アダプタ `intervention::answer` が種類を見て、Return × Rest の成功後にオーバーレイを主スレッドで開く。表示中に自分で戻っていた場合は何も書かず、オーバーレイだけ開く。Return × Grace は休息中でなければ何も予約しない。
- **`current_position_changed` の発行をコマンドからアダプタ `intervention::answer` へ移した。** 以前はクリック経由の応答だけが発行し、ホットキー経由の応答は発行していなかった。
- `withdraw_intervention` (提示失敗) は種類で分け、復帰の介入は休息の長さの後に出直す。
- `InterventionSnapshot` に `kind` (`"rest"` / `"return"` / null) を足し、契約テストの欄一覧を改めた。
- 既存の state.rs テスト二件 (「休息中に 60 分 → 介入は出ない」) は、復帰の介入が出るようになったため「休息の介入は出ない」に読み替えて改めた。

## Spec Change Log

## Review Triage Log

| # | Layer | Finding | Verdict | Evidence | Route |
|---|---|---|---|---|---|
| 1 | BH / EC | 自分で休息を終えた後も復帰の介入が残り、応答されるまで休息介入が出ない。新しい休息の後に古いパネルで「休息を終える」を選ぶと新しい休息が終わる | medium | 凍結された I/O マトリクス「表示中に自分で戻る」と AD-7 の帰結であり、`the_rest_intervention_waits_for_the_return_intervention` が固定している。利用者に提示し、(B) 現状維持とリスクの明記を選んだ | intent_gap → 利用者の裁定で解消、README への明記を patch |
| 2 | BH / EC / VG | 取り直しの失敗・往復中に `kind` が前の介入のまま残り、種類不明は休息の文言に倒れるため、ボタンの文言と実際の意味が逆になりうる | medium | `Intervention.svelte` の catch 枝は `kind` を戻さず、`WORDING[kind ?? 'rest']`。コアは表示中の種類で選択を解釈する | patch |
| 3 | VG | `answer` の後処理 (オーバーレイを開く・`current_position_changed`) を消しても落ちる検査が無い | high (pre-verified) | 純粋関数 `opens_overlay` だけが検査され、`answer` を実行する検査は存在しない | patch |
| 4 | BH | Svelte のテストが名前の後半 (オーバーレイを自ら開かない) を確かめていない | low | `hide_overlay` の不在しか見ていない。1 行で塞がる | patch |
| 5 | BH | 「数値を描かない」テストが '分' 単体を禁じ、自分・十分で誤って落ちる | low | 正規表現への置き換えで済む | patch |
| 6 | BH | `answer_intervention` コマンドの `# Errors` が古い | low | 休息の終了の失敗が書かれていない | patch |
| 7 | BH | パネルからの休息の終了が `end_rest` を複製している。`enter_rest` の後の `grace_until = None` は重複 | low | 二つの経路が別々に保守され、片方だけ変わりうる | patch |
| 8 | BH | 用語集に**休息の長さ**・復帰の介入が無い | low | 用語集は「同義語の導入を禁ずる」正典であり、新語を足さずに使っている | patch |
| 9 | EC | move / switch で休息に入り直した後 `deactivate_current_position` を呼ぶと開始時刻が古いまま残る | false | `deactivate_current_position` はコマンドとして公開されておらず (`commands/` からはテストでのみ呼ばれる)、本番で到達する経路が無い | reject |
| 10 | EC | バックエンドが未知の `kind` を送ると `wording` が undefined になる | low | `InterventionKind` は serde の閉じた enum であり未知の値は生成されない。#2 の修正 (種類不明の中立文言) と併せて実害は無い | reject |
| 11 | BH | `follow_rest` の安全網に検査が無い | low | 安全網が拾う「他の経路での休息入り」は #9 と同じく本番で到達しない | reject |
| 12 | BH | 刻みのログがどちらの介入が出たかを述べない | low | 現場のログの読みやすさのみ。`TickEffect` に欄を足す修正は直接の訂正を超える | reject |

## Verification

**Commands:**
- `make test` -- expected: 全テスト成功
- `make lint` -- expected: 警告なし

**Manual checks (if no CLI):**
- `sqlite3` で `rest_length_seconds` を 60 にし、`make dev` で ⌘R → 1 分後にパネルが出ること、Ctrl+Opt+R でオーバーレイが開き次の一手が見えること
