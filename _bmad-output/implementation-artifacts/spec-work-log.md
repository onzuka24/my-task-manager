---
title: '作業時間の記録 — ステップごとの区間を残し、スクリプトで日 × ステップの CSV に書き出す'
type: 'feature'
created: '2026-11-11'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: '30bacf4ddabbc66f4e682474c3247b8c3d8e81c2'
context:
  - '{project-root}/_bmad-output/specs/spec-my-task-manager/glossary.md'
  - '{project-root}/_bmad-output/planning-artifacts/architecture/architecture-my-task-manager-2026-09-15/ARCHITECTURE-SPINE.md'
  - '{project-root}/_bmad-output/implementation-artifacts/spec-wait.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** 勤怠・工数の報告のために、どのステップにどれだけ時間を使ったかを知りたい。いまは切り替えの時刻しか残っておらず、作業時間は分からない。

**Approach:** **現在地**が**活性**だった時間を、ステップごとの**作業区間**として DB に残す。画面には一切出さない (AD-15)。必要なときにスクリプトを実行し、日 × ステップの CSV に書き出す。参考値であり、精度より軽さを優先する。

## Boundaries & Constraints

**Always:**
- **数える:** **現在地**が**活性**の時間 — そのステップの区間。
- **数えない:** **休息**中・Mac のスリープ中 (刻みの飛びで検出し、スリープ直前の刻みの時刻で区間を閉じ、戻った時刻から開き直す)・**未着手**・常駐が止まっていた間。
- **待機:** **待機**中に**現在地**が戻り先と同じタスクにある時間は「待機」の区間として記録する (戻り先のタスクと、何を待っているかの一言を付ける)。主作業のステップには数えない。**待機**中に別タスクへ移った時間はそのステップの区間。
- **書くのは区間が閉じた瞬間だけ。** 状態が変わったとき (現在地の移動・休息の出入り・待機の出入り・スリープからの復帰・メニューバー項目の「終了」) に、閉じた区間を 1 行書く。**定期的な書き込みはしない。** 状態を変える操作が既に書き込むときは、同じトランザクションに相乗りさせる。
- 開いている区間 (種類・ステップ・開始時刻) は**永続化しない** (メモリだけ)。**異常終了した場合、そのとき開いていた区間は失われてよい** (利用者の裁定 — 参考値であり、異常終了は気にしない)。
- **決定 (利用者の裁定・レビュー後): Mac のログアウト・再起動・シャットダウンでも開いている区間を閉じて書く。** メニューバー項目の「終了」を押さずに電源を切る運用で、その日の最後の区間 (数時間になりうる) を失わないため。macOS がその直前に出す通知を受けて閉じる。定期的な書き込みは足さない。
- 長さ 0 の区間は書かない。
- スキーマは版 4 を**追記**する (V1〜V3 を書き換えない)。
- **書き出し:** `scripts/export-work-log.sh [開始日] [終了日]` (いずれも `YYYY-MM-DD`、両端を含む。省略すれば今月の 1 日から今日まで) が CSV を標準出力へ出す。`sqlite3` だけで動く。 レビュー後: 先頭に常に BOM。問い合わせを先に済ませ、成功したときだけ BOM・見出し・本文を出す。日付の形は bash の `[[ =~ ]]` で文字列全体を検める。`.timeout 2000`。
  - 1 行 = 1 日 × 1 ステップ。列: 日付・タスク・ステップ番号・ステップ・分。見出しの行を付ける。
  - 待機は別の行: ステップ番号は空、ステップは「待機: <一言>」(一言が無ければ「待機」)。
  - 分は小数 2 桁 (秒の精度。15 分単位などの丸めはしない — 報告先に合わせて表計算で掛ける)。
  - **決定 (利用者の裁定・レビュー後): CSV の先頭に常に UTF-8 の BOM を付ける。** Mac の Excel でそのまま開いて日本語が化けないため。
  - 日をまたぐ区間はローカル時刻の 0 時で分ける。
  - タスク名・ステップの内容にカンマ・引用符・改行があっても CSV として壊れない。
  - ステップ番号と内容は書き出した時点のものを使う (後から追記・修正していれば、その形で出る)。
- 区間の本文 (一言・タスク名) をログに書かない。
- 文書: スパイン AD-2 (開いている区間: コア・永続化しない / 作業区間: コア・永続化する)、AD-15 に「記録するが表示しない」、ERD、用語集 (**作業区間** と識別子)、README (書き出し方と精度の限界)、`docs/database.md`。

**Never:**
- 作業時間・合計・グラフをオーバーレイ・パネル・メニューバー項目に出す (AD-15)。
- 定期的な書き込み (心拍) を足す。
- コマンド・ボタン・打鍵を足す。書き出しはスクリプトだけである。
- 休息を宣言しない離席を推測で差し引く (観測は v2)。

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|---|---|---|---|
| 切り替え | 第 3 で 10:00〜10:25 作業 → Enter で第 4 へ | 第 3 の区間 10:00〜10:25 が 1 行。第 4 の区間が開く | 切り替えの書き込みが失敗すれば区間も書かれない (メモリの区間も閉じない) |
| 休息 | 作業中に ⌘R → 15 分後に休息を終える | 休息に入った時刻で区間が閉じ、終えた時刻から開く。休息の 15 分はどこにも数えない | N/A |
| スリープ | 作業中に Mac が 30 分スリープ | スリープ直前の刻みで閉じ、復帰した刻みで開き直す | 書けなければ次の刻みでやり直す |
| 待機 | 第 3 で ⌘D (「AI の出力」) → 5 分後に主作業へ戻る | 第 3 の区間が閉じ、待機の区間 (主作業のタスク・「AI の出力」) が 5 分。戻ったら第 3 が開く | N/A |
| 待機中に別タスク | 待機中に一覧で別タスクのステップへ 3 分 | その 3 分はそのステップの区間。待機の区間はその間閉じる | N/A |
| 完了 | ⌘Enter (現在地は動かない) | 区間は閉じない | N/A |
| 終了 | メニューバー項目の「終了」 | 開いている区間を閉じて書いてから終わる | 書けなければ失われる |
| シャットダウン | 作業中に「終了」を押さずに Mac をシャットダウン (ログアウト・再起動も) | 開いている区間を閉じて書いてから終わる | 書けなければ失われる |
| 異常終了 | 作業中にプロセスが落ちる | その区間は残らない。次の起動で現在地が活性なら起動時刻から開く | N/A |
| 書き出し | `export-work-log.sh 2026-11-01 2026-11-30` | 11 月の日 × ステップの CSV | DB が無い・日付が不正なら理由を標準エラーへ出して非 0 で終わる |
| 日またぎ | 23:30〜00:45 の区間 | 前日 30 分・当日 45 分の 2 行 | N/A |
| 既定の期間 | 引数なし | 今月 1 日から今日まで | N/A |

</frozen-after-approval>

## Code Map

- `src-tauri/src/adapters/storage/schema.rs:108` -- `MIGRATIONS` に V4 を追記: `work_segment (id TEXT PK, kind TEXT CHECK (kind IN ('step','wait')), task_id TEXT NOT NULL REFERENCES task(id), step_id TEXT REFERENCES step(id), label TEXT, started_at TEXT NOT NULL, ended_at TEXT NOT NULL) STRICT`。`kind='step'` なら `step_id` 必須、`'wait'` なら NULL (CHECK で縛る)。版のテストを改める
- `src-tauri/src/domain/` -- 新しい `work_log.rs`: `WorkSegment` (閉じた区間) と、状態から「いま開いているべき区間の種類」を決める純粋関数 (`NotStarted`/`Inactive` → 無し、活性かつ待機中かつ現在地が戻り先のタスク → 待機、活性 → そのステップ)。区間の比較・長さ 0 の判定もここ
- `src-tauri/src/domain/state.rs` -- `CoreState` に非永続の開いている区間。状態を変える各経路 (`transition_position`・`switch_current_position`・`select_step`・`move_current_position`・`create_task_promoting` の着手・`rest_from`/`leave_rest`・`begin_wait`/`leave_wait`/`end_wait`・`answer_intervention`) の後で、望ましい区間と食い違えば閉じた区間を書いて開き直す。**状態を変えるコミットがあればそれに相乗りする** (`Commit` に `work_segments: Vec<WorkSegment>` を足す)。`begin_wait` のように書き込みの無い経路は区間を書く 1 コミットになる。`tick` のスリープ検出 (`rest::slept_millis`) で、直前の刻みで閉じて今から開く。`restore` で現在地が活性なら起動時刻から開く。終了のための `Core::close_work_segment()` (閉じて書く)
- `src-tauri/src/ports/storage.rs` / `adapters/storage/mod.rs` -- `Commit` の新しい欄の書き込み (`is_empty` も改める)。読み戻しは不要 (書き出しはスクリプトが DB を直接読む)
- `src-tauri/src/lib.rs:326` (`run` の閉包) と `adapters/menubar/mod.rs:96` (`app.exit(0)`) -- 明示的な終了の前に `Core::close_work_segment()` を呼ぶ (失敗はログのみ)。どちらに置くかは確実に通る方を選ぶ
- `scripts/export-work-log.sh` -- 新規。DB の場所は `make uninstall` と同じ (`~/Library/Application Support/dev.onzuka.mytaskmanager/state.sqlite3`、環境変数で上書き可にすると検査しやすい)。日の分割は再帰 CTE、`datetime(..., 'localtime')`。`sqlite3 -csv -header`。手本: `scripts/measure-footprint.sh` の書き方
- テスト: `work_log.rs` (純粋関数)、`state.rs` (I/O マトリクスの各行。手本は待機の `a_core_on_the_main_work`)、`schema.rs` (版 4)、書き出しは一時 DB に区間を入れてスクリプトを走らせ CSV を比べる検査 (CI は macos-15 で `sqlite3` がある)
- 変更しない: フロントエンド (`src/`)、コマンド、介入

## Tasks & Acceptance

**Execution:**
- [x] `src-tauri/src/adapters/storage/schema.rs` -- 版 4 -- 既存 DB を壊さず表を足す
- [x] `src-tauri/src/domain/work_log.rs` / `domain/mod.rs` -- 区間の型と純粋関数 -- 判定を時計から切り離して検証する
- [x] `src-tauri/src/ports/storage.rs` / `adapters/storage/mod.rs` -- 区間の書き込み -- 状態の変化と同じトランザクションに乗せる
- [x] `src-tauri/src/domain/state.rs` / `lib.rs` (または `adapters/menubar`) -- 各経路での開閉・スリープ・起動・終了 -- I/O マトリクスの全行
- [x] `scripts/export-work-log.sh` と その検査 -- CSV の書き出し -- 日またぎ・待機・CSV の引用・既定の期間
- [x] スパイン / 用語集 / README / `docs/database.md` -- 改訂 -- 記録するが表示しないことを実装と揃える

**Acceptance Criteria:**
- Given 作業区間が記録されている, when オーバーレイ・パネル・メニューバー項目を見る, then 作業時間に関するものは何も現れない
- Given 常駐が作業中のまま 1 時間動いている, when 状態が何も変わらない, then DB への書き込みは区間のために一度も起きない

## Implementation Notes

- **`work_log.rs`:** `WorkSegment` (閉じた区間・`Serialize` を実装しない)・`WorkSegmentKind { Step { task_id, step_id }, Wait { task_id, label } }`・`OpenWorkSegment` (非永続)。純粋関数は `desired(position, wait)` (未着手・非活性 → 無し / 活性かつ待機中かつ戻り先と同じタスク → 待機 / 活性 → そのステップ)・`shift(open, desired, ended_at, now)` (種類が同じなら何もしない、違えば閉じて開き直す)・`closing_time(last_tick, now, interval)` (刻みの飛びがあれば直前の刻み)。長さ 0 以下は `OpenWorkSegment::close` が `None` を返し、書かない。
- **`state.rs`:** `CoreState.open_segment`。`CoreState::segment_shift(遷移後の現在地, 遷移後の待機, now)` を各経路 (`move_current_position`・`switch_current_position`・`select_step`・`transition_position`・`rest_from`・`leave_rest`・`begin_wait`・`leave_wait` の両方の結末) で呼び、閉じた区間を**その経路のコミットに相乗り**させ、成功したときだけ `open_segment` を更新する (失敗ならメモリの区間も閉じない)。`create_task` の着手は `move_current_position` を通る。`answer_intervention` は `rest_from` / `leave_rest` / `leave_wait` を通る。書き込みの無かった `begin_wait` と `leave_wait` の `Stayed` は区間だけの 1 コミット。`tick` は飛びを見たら直前の刻みで閉じていまから開き直し、`adjusted_position` と同じコミットに載せる (失敗なら `last_tick` を進めず次の刻みでやり直す)。何も変わらない刻みは書かない。`restore` は活性なら起動時刻からステップの区間を開く。`close_work_segment()` は閉じて書き、二度目は何もしない。
- **操作の閉じる時刻もスリープを見る:** スリープの飛びと閉じる時刻は、直前の刻みと開いている区間の開始の遅いほう (`CoreState::segment_reference`) から測る。スリープから戻ってすぐ (最初の刻みより先に) 操作しても、スリープは区間に入らず、操作で開き直した区間を続く刻みが閉じ直すこともない (レビュー後)。
- **`begin_wait` の型:** `Result<bool, CoreError>` に変えた (区間を書けなければ待機に入らない — 他の経路と同じ「書けなければ何も変わらない」)。コマンドは `map_err(to_string)?` で理由を返す (既存の拒否と同じ 1 行の表示に乗る)。コマンドの追加・フロントの変更は無い。
- **終了:** `lib.rs` の `close_work_segment(app)` をメニューバー項目の「終了」(`app.exit(0)` の直前 — 確実に通る) と `RunEvent::Exit` (ログアウトなど `ExitRequested` を経ない終了の予備) の両方から呼ぶ。二度目は閉じる区間が無く何も書かない。失敗はログのみ。 ログアウト・再起動・シャットダウンは `adapters/power::observe` (`NSWorkspaceWillPowerOffNotification` を購読) が同じ関数を呼ぶ (レビュー後)。三か所の呼び出しは字面の検査で固定した。
- **書き出し:** `-readonly` では開かない — WAL の DB はアプリを終えた後 `-shm` が無く、読み取り専用の接続は開けない (検査で実際に落ちた)。読み書きで開き `PRAGMA query_only = ON` で書き込みを禁じる。見出しは自前で出す (`sqlite3 -header` は 0 件のとき見出しも出さない)。期間の両端と日の境目はローカル 0 時を UTC に直した瞬間で、長さは UTC のまま測る (夏時間で 1 時間ずれない)。
- **検査:** `work_log.rs` (純粋関数)・`state.rs` の `work_segments` (I/O マトリクスの各行と受け入れ条件「1 時間作業のまま書かない」)・`schema.rs` (版 4 の移行と CHECK)・`adapters/storage` (列の往復と、実 DB にスクリプトを走らせて CSV を読み比べる — 日またぎ・期間の端・待機・引用・書き出し時点の番号・既定の期間・不正な入力)。既存の待機の検査二つ (「戻る (移っていない)」「主作業を進めていた」) は「何も書かない」から「区間の 1 コミットだけ書く」へ改めた。
- **文書:** スパイン AD-2 (2 行)・AD-10・AD-15 (七つ目の禁止と Rule)・ERD、用語集 (語彙・識別子・申し送りを実装済みに)、README (書き出し方と精度の限界)、`docs/database.md`、`docs/architecture.md` (`begin_wait` / `return_from_wait` の「何も書かない」を改めた)。

## Spec Change Log

- **2026-11-11 — `begin_wait` の型 (Code Map「変更しない: コマンド」からの逸脱):** `Core::begin_wait` は `Result<bool, CoreError>` を返すようにした。主作業のステップの区間を閉じて書けなければ待機に入らない (他の経路と同じ「書けなければ何も変わらない」)。これに合わせ `begin_wait` コマンドは、区間の書き込みに失敗したとき理由の文字列をエラーとして返す (既存の拒否と同じ 1 行の表示に乗る)。コマンドの追加・引数の変更・フロントの変更は無い。
- **2026-11-11 — レビュー後:** 利用者の裁定二つ (ログアウト・再起動・シャットダウンでも閉じて書く / CSV の先頭に BOM) を凍結ブロックへ。前者は `adapters/power` が `NSWorkspaceWillPowerOffNotification` を購読して `close_work_segment` を呼ぶ。

## Review Triage Log

| # | 層 | 指摘 | 判定 | 根拠 | 行き先 |
|---|---|---|---|---|---|
| 1 | BH | 「終了」を押さずにログアウト・再起動・シャットダウンすると開いている区間が失われる (数時間になりうる) | medium | ログアウトは `ExitRequested { code: None }` で来て第 3 層が拒み、SIGTERM は `RunEvent::Exit` を経ない。凍結された意図は異常終了しか扱っていなかった | intent_gap → 利用者の裁定 (拾う) を凍結ブロックへ。`NSWorkspaceWillPowerOffNotification` で閉じる。差分全体は作り直さず該当箇所の修正として返した |
| 2 | BH | CSV に BOM が無く、Mac の Excel で日本語が化ける | medium | 勤怠・工数の報告で Excel を使う見込みが高い | intent_gap → 利用者の裁定 (常に付ける) を凍結ブロックへ |
| 3 | BH | 日付の検めが行ごとの grep で、改行を含む引数がすり抜けて SQL に埋め込まれる | low | `printf | grep -E` は行単位で一致する。直しは bash の正規表現 1 行 | patch |
| 4 | EC | スリープ明けの操作の後の刻みが古い `last_tick` で閉じ、`close` が None を返して操作から刻みまでの時間が落ちる (二つ目の操作も同じ) | low | 閉じる時刻の基準が `last_tick` だけ | patch (`max(last_tick, started_at)` を基準に) |
| 5 | EC | 本体の問い合わせより先に見出しを出すため、失敗すると見出しだけの CSV が「空の月」に見える | low | 出力の順 | patch |
| 6 | EC | 書き出しに busy timeout が無い | low | 直しは 1 行 | patch |
| 7 | VG | 「作って着手」(`move_current_position`) で離れる区間の書き込みが未検査 | low | 区間を閉じる検査はすべて他の経路 (VG が実証) | patch |
| 8 | VG / BH | 既定の期間の検査が空の DB で、期間を取り違えても通る。1 引数の形・表が無い場合も未検査 | low | 見出しの行だけを見ていた | patch |
| 9 | VG / BH | 「終了」・`RunEvent::Exit` で区間を閉じる配線が、消えても検査が落ちない | low | 生きた Tauri を動かす仕掛けは無い。消えたら落ちる字面の守りは足せる | patch (字面の守り) + 実行の検査は defer |
| 10 | BH | AD-2 の待機の行が「何を待っているかは永続化しない」のまま (`work_segment.label` に残る) | low | 文書の直接の修正 | patch |
| 11 | BH | 最終ステップの完了後も、次の変化まで区間がそのステップに数えられる | low | 完了は現在地を動かさない (既存の設計) | patch (README の精度の限界に明記) |
| 12 | BH | スリープの誤検出 (刻みの間引き・時計の前進) で実作業が切り取られることを README が過小に書いている | low | 文書の直接の修正 | patch |
| 13 | BH | スクリプトの「書き込みはしない」が厳密でない (`-wal`/`-shm` の後始末) | low | 文書の直接の修正 | patch |
| 14 | BH | コードマップの「コマンドは変えない」と、`begin_wait` が書き込み失敗で `Err` を返すようになったことが食い違う | low | 書けなければ何も変えない、という全経路の規則に揃えた結果 | Spec Change Log に記録 |
| 15 | BH | 夏時間の主張が未検査 | low | 利用者の時刻帯 (Asia/Tokyo) に夏時間は無い | reject |
| 16 | EC | 0 時が存在しない・二重になる時刻帯で日の分割が止まらない | low | 同上 | reject |
| 17 | EC / BH | 時計が戻ると開いている区間が跡を残さず消える | low | 手動で時計を戻すことは日常で起きない。NTP の補正はミリ秒単位 | reject |
| 18 | BH | 「`Serialize` を実装しない」を強制する仕掛けが無い | low | 直しは依存か compile-fail の検査を足すこと。修正の範囲を超える | reject |
| 19 | BH | `work_segment` が `step_id` と `task_id` の整合を DB で縛らない | low | 書く経路はコアだけで、コアが常に組で書く。直しは複合外部キーへのスキーマ変更 | reject |
| 20 | EC | CI に `sqlite3` が無いと書き出しの検査が落ちる | false | CI は macos-15 で `/usr/bin/sqlite3` がある | reject |

## Verification

**Commands:**
- `make test` -- expected: cargo test / pnpm check / pnpm test がすべて通る
- `make lint` -- expected: 警告なし

**Manual checks (if no CLI):**
- `make dev` で 2〜3 回切り替え・休息・待機をしてから「終了」し、`scripts/export-work-log.sh` の CSV に区間が出ること
