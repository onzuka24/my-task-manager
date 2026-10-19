---
title: '切り替えで閉じない — 移った先の次の一手を確かめてから閉じる'
type: 'feature'
created: '2026-10-09'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: 'bdca3a9bbefebdd71013fc9905a2e9c4a31363ab'
context:
  - '{project-root}/_bmad-output/specs/spec-my-task-manager/glossary.md'
  - '{project-root}/_bmad-output/implementation-artifacts/spec-switch-and-resume.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** 既定表示の Enter (**切り替え**) は、**現在地**を同一**タスク**の次の**ステップ**へ移すと同時にオーバーレイを閉じる。利用者は移った先の**次の一手**を見ないまま作業へ戻ることになり、「次が何だったか」を確かめるためにもう一度呼び出す必要がある。

**Approach:** **切り替え**が**現在地**を動かしたとき、オーバーレイを閉じずにスナップショットを取り直し、移った先の**次の一手**・位置情報・**中断メモ**を既定表示として見せる。閉じるのは利用者の明示的な操作 (またはフォーカス離脱) である。開示面で**ステップ**を選んだときの既存の振る舞い (閉じずに既定表示へ戻る) に揃える。

## Boundaries & Constraints

**Always:**
- 切り替え後に見せるのは移った先の**次の一手**だけ。離脱した**ステップ**の内容や他の**タスク**名を同時に出さない (FR-2 / 設計上の賭け #1)。
- 移った先に**中断メモ**があれば、追加操作なしに読める (FR-8)。
- 切り替えたことを 1 行で述べる。黙って表示が入れ替わると、確定したのか打鍵が効かなかったのか区別できない。
- 呼び出しから**中断メモ**の確定・離脱までを 5 打鍵以内に保つ (FR-7)。
- 再描画はスナップショットの取り直しを経由する (AD-3 鮮度規則)。
- フォーカス離脱と Esc で閉じる既存の経路は変えない。
- **決定 (利用者の裁定): 切り替え直後の表示は通常の既定表示と同じである。** そこでの Enter はもう一度**切り替え**を撃ち、閉じるのは Esc (またはフォーカス離脱・「閉じる」ボタン)。開示面で**ステップ**を選んだ後の振る舞いと同じ扱いにする。「Enter 一打で終わる」癖による 2 つ先への前進は、利用者が受け入れたリスクである。

**Never:**
- コア側 (`switch_current_position`・`Commit`・切り替え履歴) を変えない。変わるのはオーバーレイの離脱の扱いだけである。
- 自動で閉じるタイマーを置かない。
- ⌘Enter (完了して次を選ぶ)、最終**ステップ**の経路、開示面の選択の振る舞いを変えない。

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|---|---|---|---|
| 切り替え | 第 3/全 6 で Enter | 第 4 の内容と「第 4 ステップ / 全 6 ステップ」、切り替えた旨の 1 行。`hide_overlay` は呼ばれない | N/A |
| 移った先にメモ | 第 4 に既存メモ | メモが入力欄に入り、カーソル末尾 | N/A |
| 確認後に閉じる | 切り替え直後に Esc | オーバーレイが閉じ、現在地は第 4 のまま | N/A |
| 続けて切り替え | 切り替え直後にもう一度 Enter | 第 5 へ移り、閉じずに第 5 を見せる | N/A |
| 最終ステップ | 第 6/全 6 で Enter | 従来どおり「次のステップが無い」旨。閉じない | N/A |
| 失敗 | 切り替えのコマンドが失敗 | 従来どおり理由を出し、閉じない | 既存 |
| 次回の呼び出し | 切り替え後に閉じて再度開く | 切り替えた旨の 1 行は残っていない | N/A |

</frozen-after-approval>

## Code Map

- `src/overlay/Overlay.svelte:1122-1160` -- `confirmSwitch()`。`outcome.moved` のとき `close()` している箇所が変更点。非移動の経路 (`refresh()` → `noDestination`) はそのまま
- `src/overlay/Overlay.svelte:754-767` -- 開示面の選択。`discardDisclosure()` → `refresh()` → `focusNoteAtEnd()` → `selected = {…}` の流れを切り替え後にも再利用する手本
- `src/overlay/Overlay.svelte:207,306,343-366` -- `noDestination` / `selected` とその 1 行の通知。切り替えた旨の通知も同じ作法 (状態 + `$derived` の文言 + `role="alert"` の `<p>`) で足す
- `src/overlay/Overlay.svelte:500-502` -- `dismissNotices()`。新しい通知の状態もここで畳む (新しい呼び出し・次の確定で消える)
- `src/overlay/Overlay.svelte:1263-1290` -- 既定表示の Enter 分岐。変更しない (切り替え直後も同じ分岐を通る)。押しっぱなしの Enter (`event.repeat`) を弾く既存の守りが連続前進を防ぐ
- `src/overlay/Overlay.svelte:1869` -- 打鍵の案内。変更不要 (既定表示の案内がそのまま正しい)
- `src/overlay/Overlay.test.ts:625-700` -- 切り替えの既存テスト群。`hide_overlay` を期待しているものは書き換える
- `README.md` -- 「Enter | 切り替え | …オーバーレイを閉じる」の行を改める

## Tasks & Acceptance

**Execution:**
- [x] `src/overlay/Overlay.svelte` -- 移動したとき閉じずに取り直し、メモ欄へフォーカスし、切り替えた旨を 1 行出す -- 移った先を確かめるため
- [x] `src/overlay/Overlay.test.ts` -- I/O マトリクスの各行を覆う。既存の「閉じる」期待を改める -- 振る舞いの固定
- [x] `README.md` -- 既定表示の打鍵表と説明を改める -- 利用者向けの記述を実装に合わせる

**Acceptance Criteria:**
- Given 実装完了, when `make lint` と `make test` を実行する, then いずれも成功する
- Given 切り替え直後の表示, when 画面を確認する, then 離脱した**ステップ**の内容はどこにも現れない

## Implementation Notes

- **`confirmSwitch()` の移動経路を `close()` から `refresh()` → `switched = { noteWritten }` に替えた。** `refresh()` が入力欄へのフォーカスを担う。非移動・失敗の経路は変えていない。
- **切り替えた旨は 1 文で、メモを記録したかだけを言い分ける。** 「切り替えた。メモを記録し、現在地を次のステップへ移した。」/「切り替えた。現在地を次のステップへ移した。」。欄は移った先のメモで埋め直されるため、述べなければ打った本文が黙って消えたように見える。判定は `noDestination` と同じ (`trim() !== ''`)。どちらのステップの内容も番号も述べない (FR-2)。
- **報せは既定表示の枝の外に描く。** 確定の後の取り直しが失敗して現在地が描かれなくなっても、確定した切り替えを述べるため。
- **`switched` は `dismissNotices()` で畳む。** 次の確定要求 (二度目の Enter を含む) と新しい呼び出しで消えるため、続けて切り替えても 1 行のまま。
- **5 打鍵のテストは「呼び出し → Enter → Esc」の 3 打鍵で離脱に届くことを見る形に改めた。**
- `make lint` / `make test` は `~/.cargo/bin` を `PATH` に足して実行した (この環境のシェルでは `cargo` が `PATH` に無かった)。

## Spec Change Log

## Review Triage Log

| # | Layer | Finding | Verdict | Evidence | Route |
|---|---|---|---|---|---|
| 1 | BH | 移動後の 1 行が、今回書いた**中断メモ**を記録したかを述べない。入力欄は移った先のメモで置き換わり、書いた文が黙って消えて見える | medium | 最終**ステップ**の経路 (`noDestinationNotice`) は記録の有無を述べるが、移動の経路は固定文である。記入率 (SM-C3) を支える記入の手応えが失われる | patch |
| 2 | BH / EC | 切り替えは確定したが取り直しが再試行を使い切ると、`forgetPosition()` が `stepContent` を消し、`{#if switched}` が既定表示の枝の中にあるため確定が何も伝わらない | low | `refresh()` の失敗経路が `forgetPosition()` を呼ぶことを確認した。従来は `close()` が隠していた。修正は通知の置き場所を移すだけで済む | patch |
| 3 | EC / BH | 確定の往復中にフォーカスを失うと、閉じた後に続きが `switched = true` を立てる | low | 次の呼び出しの `onFocusChanged` が `dismissNotices()` を先に走らせるため、残るのは数 ms の往復中に開き直した場合だけであり、そのときも述べる内容 (切り替えた) は事実である。修正は世代カウンタの追加を要する | reject |
| 4 | EC | 他所からの `current_position_changed` で位置が変わっても「切り替えた」が残る | low | 自分の切り替えも同じイベントを招くため、購読で畳むと自分の通知まで消える。修正は単純な訂正ではなく、日常で出会う経路でもない | reject |
| 5 | EC | 空メモの切り替えテストが「閉じない」を確かめなくなった | low | 差分で `hide_overlay` の期待が消え、通知の確認に置き換わっていた。1 行で塞がる | patch |
| 6 | EC / BH | `refresh()` の後の `focusNoteAtEnd()` は重複している | low | `keepDirtyDraft` なしの `refresh()` が既に呼ぶ (`Overlay.svelte:438-444`)。削除で済む | patch |
| 7 | BH | 開示面の通知の注釈「移った場合は述べない」と、移動を述べる切り替えの経路が食い違う | low | 二つの原則が理由なく並び、次に通知を足す人がどちらに従うか決められない。注釈の訂正で済む | patch |
| 8 | BH | 通知群の注釈「これらが唯一の手がかり」が不正確 | low | 位置情報の行も手がかりであり、他の通知も面の外にある。注釈の訂正で済む | patch |
| 9 | BH | `noticeText()` が最初の `role="alert"` だけを読むため、新しいテストが DOM 順に依存する | low | 事実だが、別の通知を既定表示の先頭に足したときにだけ壊れ、壊れ方も検査の失敗として表に出る。修正は補助関数の作り直しを要する | reject |
| 10 | BH | 切り替え後の「閉じる」ボタンが未検証 | low | ボタンは変更していない `close()` を呼ぶだけで、既存のボタンのテストが経路を覆う | reject |
| 11 | BH | README の「呼び出して使ったら消える一時的な面」が新しい振る舞いと食い違って読める | low | `README.md:54`。限定の一句で済む | patch |
| 12 | BH | spec の Code Map の行番号が古い・ログが空 | false | spec の編集を修正とする指摘であり、本ビルドの規則で却下する | reject |

## Verification

**Commands:**
- `make test` -- expected: 全テスト成功
- `make lint` -- expected: 警告なし

**Manual checks (if no CLI):**
- `make dev` で起動し、Enter で切り替えた後にオーバーレイが開いたまま移った先が表示されること
