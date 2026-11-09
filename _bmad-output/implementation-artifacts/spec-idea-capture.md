---
title: '思いつき — 作業中の思いつきを一行で手放し、昇格しなければ 14 日で消える'
type: 'feature'
created: '2026-11-09'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: '000581b7f4097de6046c90b4d7ca5166b9a15563'
context:
  - '{project-root}/_bmad-output/specs/spec-my-task-manager/glossary.md'
  - '{project-root}/_bmad-output/planning-artifacts/architecture/architecture-my-task-manager-2026-09-15/ARCHITECTURE-SPINE.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** 作業中に別の作業を思いつくと、書き留める先が無い。タスクにすれば一覧が散らかり、外のメモ帳に置けば「後で見返す」義務を負ったゴミ山になる。どちらも頭から手放せない。

**Approach:** **思いつき (Idea)** を足す。オーバーレイから一行で置け、**現在地**は動かない。既定表示には出さず、**開示面**の末尾に別欄で並ぶ。そこで Enter を押すと作成の面が開き、**タスク**へ昇格する。昇格しなかった**思いつき**は**思いつきの寿命** (既定 14 日、`setting` で可変) を過ぎると DB から削除する。消えたことも件数も知らせない。

## Boundaries & Constraints

**Always:**
- 置く操作: 既定表示 (作業中・休息中とも) で ⌘I → 一行の入力欄 → Enter で確定、Esc で戻る。前後の空白を除いて空なら確定しない (理由を 1 行)。**現在地**・**中断メモ**・休息の状態・計時に触れない。
- **決定 (利用者の裁定): 置いた後はオーバーレイを閉じない。** 既定表示へ戻り「思いつきを置いた。」を 1 行出す (作成・切り替えと同じ作法)。閉じるのは Esc。
- **決定 (利用者の裁定): 昇格時の下書きは題名とステップ 1 の両方に思いつきの文を入れる。** ⌘Enter 一打で昇格でき、必要なら直してから確定する。
- **開示面**の末尾に見出し「思いつき」と、新しい順の行を並べる。**思いつき**が 0 件なら欄ごと出さない。↑↓ は**タスク**の行と同じ並びの上を動く。
- 行で Enter → 作成の面が開く (下書きは上の決定のとおり)。作成が確定したら、その**思いつき**は**タスク**の作成と**同じトランザクション**で削除する。作成の面を Esc で出たら**思いつき**は残る。
- 寿命の削除はコアの刻み (`Core::tick`) で行う。作成時刻 + 寿命を過ぎたものを 1 コミットで削除する。起動直後の最初の刻みでも働く。
- 寿命は `setting` の `idea_lifetime_seconds` (既定 1209600 = 14 日)。無い・0 以下・数値でなければ既定値 (既存の `positive_seconds`)。
- スキーマは版 3 を**追記**する (V1・V2 を書き換えない)。
- PRD §5・AD-2 の状態表・AD-15・用語集 (両表) に**思いつき**と寿命を書き足し、README と `docs/` (database.md / settings.md / architecture.md のコマンド表) を更新する。

**Never:**
- 件数・残り日数・作成からの経過・「消えた」旨を、どの面にもメニューバー項目にも出さない (AD-15)。
- **思いつき**を既定表示に出す。介入・通知・音で見返しを促す。
- **思いつき**を手で削除する経路、並べ替え、編集。
- **タスク**・**ステップ**を削除する経路を作る。削除できるのは**思いつき**だけである。
- 本文をログに書く。

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|---|---|---|---|
| 置く | 作業中に ⌘I →「請求書の様式を直す」→ Enter | 1 件増える。既定表示へ戻り「思いつきを置いた。」。現在地・メモの下書きは変わらない | 失敗なら入力欄を残して理由を 1 行 |
| 休息中に置く | 休息中に ⌘I → 入力 → Enter | 置ける。休息は続いている | N/A |
| 空 | 空白だけで Enter | 確定しない。理由を 1 行、入力欄は残る | N/A |
| 一覧 | 思いつき 2 件で ⌘L | タスク一覧の後に「思いつき」と 2 行 (新しい順) | N/A |
| 0 件 | 思いつき 0 件で ⌘L | 「思いつき」の欄が無い | N/A |
| 昇格 | 行で Enter → 題名とステップ 1 に文が入った作成の面で ⌘Enter | タスクができ、その思いつきは消える (同一トランザクション) | 作成失敗なら思いつきも残る |
| 昇格の途中で消える | 作成の面を開いている間に寿命で削除された | 作成は通常どおり確定する (削除済みの思いつきは無視) | N/A |
| 昇格をやめる | 行で Enter → 作成の面で Esc | 思いつきは残る | N/A |
| 寿命 | 作成から 14 日 + 1 秒後の刻み | DB から削除。何も表示しない | 削除に失敗したらログのみ。次の刻みでやり直す |
| 寿命の設定 | `idea_lifetime_seconds` = 60 | 60 秒で消える。0 以下・数値でなければ 14 日 | N/A |
| IME | 変換中の Enter | 確定しない | 既存の IME ガード |

</frozen-after-approval>

## Code Map

- `src-tauri/src/adapters/storage/schema.rs` -- `MIGRATIONS` に V3 を追記: `idea (id TEXT PK, content TEXT NOT NULL, created_at TEXT NOT NULL) STRICT`。テストの「表の一覧」「最新版」の期待値も改める
- `src-tauri/src/domain/` -- 新しい `idea.rs`: `Idea` / `IdeaId` (`task.rs` の `id_newtype!` と `new_v7` を再利用。UUID v7)。寿命の判定は純粋関数 (`expired(ideas, now, lifetime) -> Vec<IdeaId>`)。鍵と既定値の定数もここ (`rest.rs` の `REST_LENGTH_KEY` の作法)
- `src-tauri/src/ports/storage.rs` -- `RestoredState` に `ideas`、`Commit` に `idea: Option<Idea>` (追加) と `deleted_ideas: Vec<IdeaId>` を足し、`is_empty` も改める。**「行を消す変更が存在しない」の注釈を「消せるのは思いつきだけ」に改める**
- `src-tauri/src/adapters/storage/mod.rs` -- `restore` で読み、`apply` で追加・削除を書く。外部キーは無いので順序は任意だが、タスクの書き込みと同じトランザクション内であること
- `src-tauri/src/domain/state.rs` -- `CoreState` に `ideas`。`restore` で受け取る。`capture_idea(content)`、`create_task` に昇格元の `Option<IdeaId>` を足す (なければ従来どおり)、`tick` の冒頭付近で寿命の削除 (失敗時は `CoreError` を返し状態を変えない — 既存の `tick` の流儀)
- `src-tauri/src/commands/mod.rs` -- `capture_idea` コマンド (要求型 `CaptureIdeaRequest { content }`)。`CreateTaskRequest` に `ideaId: Option<String>`。`DisclosureSurface` に `ideas: Vec<DisclosureIdea { ideaId, content }>` (行と別の欄。**件数・時刻の欄を持たない**)。ワイヤ契約テストを改める
- `src-tauri/src/lib.rs:160-175` -- `generate_handler!` に `capture_idea` を足す
- `src/overlay/Overlay.svelte` -- 型を 1:1 で足す。⌘I (`isSurfaceKey` に `'i'`)、一行入力の面 (作成の面の作法: `creating` と同列の揮発ビュー状態)、開示面の末尾の欄と ↑↓ / Enter (`rowKey` に `idea:` を足す)、`openCreation` に下書きを渡す引数と `confirmCreation` での `ideaId` の送付、既定表示の案内に「⌘I で思いつき」
- `src/overlay/Overlay.test.ts` -- 既存の開示面・作成の面のテストの手本に倣う
- 文書: `_bmad-output/planning-artifacts/prds/.../prd.md` §5、`ARCHITECTURE-SPINE.md` AD-2 / AD-15 / ERD、`_bmad-output/specs/spec-my-task-manager/glossary.md`、`README.md`、`docs/database.md` / `docs/settings.md` / `docs/architecture.md`
- 変更しない: 介入 (`rest.rs` の判定・`Intervention.svelte`)、切り替え・完了・修正の経路

## Tasks & Acceptance

**Execution:**
- [x] `src-tauri/src/adapters/storage/schema.rs` -- 版 3 を追記 -- 既存 DB を壊さず表を足す
- [x] `src-tauri/src/domain/idea.rs` / `domain/mod.rs` -- `Idea`・寿命の純粋関数・鍵と既定値 -- 判定を時計から切り離して検証する
- [x] `src-tauri/src/ports/storage.rs` / `adapters/storage/mod.rs` -- 読み戻し・追加・削除 -- 昇格の削除をタスク作成と同じトランザクションにする
- [x] `src-tauri/src/domain/state.rs` -- 置く・昇格つきの作成・刻みでの寿命の削除 -- I/O マトリクスの全行
- [x] `src-tauri/src/commands/mod.rs` / `lib.rs` -- コマンドと開示面の欄 -- ワイヤ契約を固定する
- [x] `src/overlay/Overlay.svelte` / `Overlay.test.ts` -- ⌘I の面・開示面の欄・昇格 -- I/O マトリクスの各行をテストする
- [x] PRD / スパイン / 用語集 / README / `docs/` -- 改訂 -- 非目標と状態表を実装と食い違わせない

**Acceptance Criteria:**
- Given 版 2 の既存 DB, when 起動する, then 版 3 になり、既存のタスクと現在地はそのまま
- Given 思いつきが何件あっても, when 既定表示・メニューバー項目を見る, then 思いつきに関する文字も数も現れない

## Implementation Notes

## Spec Change Log

## Review Triage Log

| # | 層 | 指摘 | 判定 | 根拠 | 行き先 |
|---|---|---|---|---|---|
| 1 | BH / EC / VG | 寿命の削除が失敗すると `tick` が冒頭で返り、休息・復帰の介入が止まり `last_tick` も進まない (復旧後にスリープと誤認しうる) | medium | 削除の `?` が `follow_rest` / `rest::decide` より前にある。I/O マトリクスは「ログのみ。次の刻みでやり直す」で、介入を止めることは想定していない | patch (介入を出さない経路の末尾へ移す) |
| 2 | EC / BH | 置く要求の応答が、面を捨てた後に届いても通知と入力位置を書き換える。応答待ちの間に ⌘I で面を開き直せる | low | `confirmCapture` は await の後に面が開いているかを見ない。`openCapture` は `capturingIdea` を見ない | patch |
| 3 | EC / BH | タスク 0 件・思いつき 1 件以上のとき「まだタスクが無い。」が思いつきの欄の下に出る | low | 描画順がその形 | patch |
| 4 | BH | `surface.ideas ?? []` が契約の破れを「思いつき 0 件」に見せる | low | ファイル自身の 1:1 規約に反する。直しは削除だけ | patch |
| 5 | BH | 既定表示の受け入れテストが何も証明していない | low | 思いつきは開示面の模擬にしか流れず、入力した文は確定で消える | patch |
| 6 | BH | `idea_id` の `#[serde(default)]` (欄の欠落) が未検査 | low | `null` の場合しか試していない | patch |
| 7 | BH | 保存のテスト群で文書注釈が別のテストに付いた | low | 新しいテストが注釈と関数の間に挿入された | patch |
| 8 | BH | `a_failed_promotion_keeps_the_idea` がタスク書き込みの失敗を試していない | low | `task: None` で現在地の外部キー違反により失敗している | patch |
| 9 | BH | FR-19・用語集の開示面・スパイン ERD の IDEA の `id`・README の ⌘I 行の名前・SPEC.md の CAP が未更新 | low | いずれも文書の直接の修正 | patch |
| 10 | VG | コマンド本体 (`capture_idea` の正規化、`create_task` の昇格元の受け渡し) を通すテストが無い | medium (未検証の退行の余地) | 純粋関数・コア・フロントの模擬はそれぞれ検査されるが、コマンド本体を通す仕掛けがリポジトリに無い。既存のどのコマンドも同じ状態 | defer |
| 11 | EC | 本文に長さの上限が無い | low | 大量の貼り付けは日常的でない。直しは新しい検査と文言を足す | reject |
| 12 | EC | U+0085・垂直タブ・改ページが改行として畳まれない | low | 日常の入力で現れない。直しは条件の追加 | reject |
| 13 | EC | 既定表示に「思いつきを置いた。」と案内の「⌘I で思いつき」が出る | false | 受け入れ条件が禁じるのは思いつきの本文と数であり、通知と案内は Always が求めている | reject |
| 14 | BH | 他の面での ⌘I が無効であることの検査が無い | false | 作成・修正・一覧の分岐は ⌘I の分岐より前に返り、今日悪い結果は起きない | reject |
| 15 | BH | 昇格の作成の面を Esc で出ると一覧ではなく既定表示に戻る | false | 作成の面の Esc は既存の作法 (既定表示へ戻る) どおりであり、思いつきは残る (I/O マトリクス「昇格をやめる」) | reject |
| 16 | BH | 仕様書の状態・チェック欄が未完了 | false | 差分の採取後に更新済み。直しは仕様の編集でもある | reject |

## Verification

**Commands:**
- `make test` -- expected: cargo test / pnpm check / pnpm test がすべて通る
- `make lint` -- expected: 警告なし

**Manual checks (if no CLI):**
- `sqlite3` で `idea_lifetime_seconds` を 60 にして `make dev` → ⌘I で置き、⌘L に出ること、1 分後に消えること
