---
title: '中断メモを空にして確定したら消す'
type: 'bugfix'
created: '2026-11-10'
status: 'done'
route: 'oneshot'
review_loop_iteration: 0
context: []
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** 中断メモの欄を全部消して確定しても (切り替えの Enter・完了の ⌘Enter・待機から早めに戻る ⌘D)、次に開くと元のメモが残っている。コマンド境界が「空欄 = 省略 (既存のメモを変えない)」と定義しており、欄を触らなかったことと、欄を空にしたことを区別していないため。

**Approach:** 「触っていない / 書き換えた / 消した」の三つを区別する。フロントは既に、触っていなければ `null`、触っていれば欄の文字列 (空を含む) を送っている。Rust 側で、前後の空白を除いて空の文字列を「消した」として扱い、既存のメモを削除する。消した場合は切り替え履歴の `note_written` に数えない (書いたのではないため)。

</frozen-after-approval>

## Implementation Notes

- `domain/task.rs` に `NoteEdit { Keep, Write(InterruptionNote), Clear }` を足した。`written()` は `Write` のときだけ真 (消したことは `note_written` に数えない)。`apply_to(task, step)` が複製の上で反映する。
- `Core::switch_current_position` / `complete_current_step` / `return_from_wait` は `impl Into<NoteEdit>` を受ける。`From<Option<InterruptionNote>>` (`None` → `Keep`、`Some` → `Write`) を置いたため、既存の 39 の呼び出し (大半がテスト) は書き換えずに済んだ。待機の介入の「主作業に戻る」は `NoteEdit::Keep`。
- `commands/mod.rs`: `as_interruption_note` を `as_note_edit` に改め、`null` → `Keep`、前後の空白を除いて空 → `Clear`、それ以外 → `Write` (本文はそのまま)。三つの要求型の文書も同じ規則に改めた。
- フロントの送り方は変えていない。既に「触っていなければ `null`、触っていれば欄の文字列 (空を含む)」を送っており、`Overlay.test.ts` の「メモを空にしたまま確定しても切り替えは完了する」がそれを固定している。切り替え後の 1 行 (「切り替えた。現在地を次のステップへ移した。」) も消した場合に偽りを述べない。
- レビュー後: 最終ステップで提示されたメモを空にして確定したとき、移動先が無い旨の 1 行が「記録するものは無かった」ではなく「メモは消した」と述べるようにした (`Overlay.svelte` の `noDestination` に `noteCleared`)。`leave_wait` に変わっていないタスクを書き直さない守りを足し、`NoteEdit` の単体テスト (Rust 421 → 424 件) とフロントの二経路の検査 (vitest 222 → 225 件) を足した。
- テスト: 切り替え・完了・待機から早めに戻る、の三経路で「空にすると既存のメモが消え、`note_written` は偽」を足した (Rust 418 → 421 件)。コマンド境界の純粋関数の検査を三状態に改めた。
- README の中断メモの節に、空にして確定すると消えることを 1 文足した。

## Review Triage Log

| # | 指摘 | 判定 | 根拠 | 行き先 |
|---|---|---|---|---|
| 1 | 最終ステップでメモを空にして確定すると「記録するものは無かった」と出る (実際は消えている) | low | `noDestinationNotice` に「消した」の場合が無い | patch — 「メモは消した」を述べる。提示されたメモがあったときだけ |
| 2 | 最終ステップでメモを消すと、移動が無くても切り替え履歴が 1 行積まれる | false | 同じ関数の注釈が「移動先が無くてもメモや完了が変わったなら残す」と定めている。消したことはメモが変わったことであり、意図どおり | reject |
| 3 | `leave_wait` が変わっていないタスクも書き直す (メモの無い欄を空にして戻ったとき) | low | `switch_current_position` には同じ守りがある。直しは比較 1 行 | patch |
| 4 | `state.rs` の rustdoc が「`None` は省略」のまま | low | 三つの関数と一つのテストの文書 | patch |
| 5 | `Overlay.svelte` の注釈「空白だけの本文はコア側で省略として扱われる」が誤り | low | コアは今「消した」として扱う | patch |
| 6 | `From<Option<InterruptionNote>>` が `Clear` を表せず、本番の呼び出しが黙って `Keep` になりうる | low | 本番の三経路はすべて `as_note_edit` を通り `NoteEdit` を明示している。変換を外すと既存の 39 の呼び出しを書き換えることになり、直しが修正の範囲を超える | reject |
| 7 | `NoteEdit` の直接の単体テストが無い | low | `Core` 越しにしか検査されていない | patch — `task.rs` に三つ足した |
| 8 | メモの無いステップへの `Clear`、完了済みへの `Clear` の検査が無い | low | 前者は #3 の守りで書き直さない。後者は既存の「何も変わらないなら書かない」が `draft` の比較で吸収する | #3・#7 に吸収 |
| 9 | ⌘Enter・⌘D で空欄を送ることのフロントの検査が無い | low | 切り替えの経路だけが固定されていた | patch — 二経路の検査を足した |
| 10 | PRD の FR-7 が「触っていない」と「空にした」を区別していない | low | 文書の直接の修正 | patch |
| 11 | 仕様書の状態が `in-progress` のまま | false | 確定の手順で `done` にする | reject |
