---
title: '休息のすすめ — 休息中の面に、緊張型頭痛を防ぐ過ごし方を開閉式で置く'
type: 'feature'
created: '2026-11-05'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: '29add1709849caed45734bf2b022c4cca777a4b7'
context:
  - '{project-root}/_bmad-output/specs/spec-my-task-manager/glossary.md'
  - '{project-root}/_bmad-output/implementation-artifacts/spec-rest-intervention.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** **休息**中の既定表示は「休息中。現在地は保たれている。」と述べるだけで、休息をどう過ごすかは利用者任せである。利用者は緊張型頭痛を起こしやすく、休息中にスマホを見て首と目を使い続けるような、休息にならない過ごし方をしやすい。

**Approach:** 休息中の既定表示に「休息のすすめ」(遠くを見る・水を飲む・スマホを避ける・肩と首をほぐす、など緊張型頭痛の対策) を置く。打鍵とボタンで開閉できるようにし、閉じているときは見出し 1 行だけを出す。

## Boundaries & Constraints

**Always:**
- すすめが出るのは**休息**中の既定表示だけ。作業中の既定表示・作成・修正・一覧の面には出さない。
- 開閉は打鍵 (⌘T) と、`aria-expanded` を持つボタンの両方で行える。休息中の案内に ⌘T を書き足す (案内と効く打鍵を食い違わせない)。
- 開閉はコアを呼ばない。休息の状態・計時・DB を変えない。
- 休息中の Enter は従来どおり休息の終了の宣言だけを意味する。
- 固定寸法の窓 (640×280) に収まらなければ、既存の `main` のスクロールで辿れる。切り取らない。
- **決定 (利用者の裁定): 開閉の状態は常駐が動いている間だけ覚える。** 最後に開閉した状態を Svelte 側のメモリに持ち、オーバーレイを閉じても次の休息でも引き継ぐ。初期値 (起動直後) は閉じた状態。永続化はしない — 再起動で閉じた状態に戻る。
- **決定 (利用者の裁定): すすめの中身はコードに固定する。** 文言は次の 7 項目をこの順で置く。
  - 窓の外など遠く (6m 以上) を 20 秒ほど眺める
  - 水をコップ 1 杯飲む
  - スマホを見ない — うつむく姿勢が首に、近くの画面が目に負担をかける
  - 肩をすくめて 5 秒止め、ストンと落とす (3 回)
  - 首をゆっくり左右に倒し、伸ばす
  - 歯の食いしばりをほどき、ゆっくり息を吐く
  - 立ち上がって少し歩く

**Never:**
- 残り時間・経過時間・何項目こなしたか等、休息に締め切りや達成を与える表示 (AD-15 / spec-rest-intervention の Never)。チェックボックスも置かない。
- 介入パネル・通知・音で、すすめを能動的に押し出すこと (AD-15)。
- 医療的な診断や断定。体の使い方の短い提案に留める。

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|---|---|---|---|
| 休息に入る (起動後初回) | ⌘R で休息中の面へ | 「休息のすすめ」の見出しだけが出る (閉じた状態) | N/A |
| 状態の引き継ぎ | 開いたまま休息を終え、閉じて、後で再び休息に入る | 開いた状態で出る | N/A |
| 開く | 休息中に ⌘T またはボタン | すすめの一覧が出て、ボタンの `aria-expanded` が true | N/A |
| 閉じる | 開いた状態で ⌘T | 一覧が畳まれ、見出しだけに戻る | N/A |
| 作業中 | 休息中でない既定表示で ⌘T | 何も起きない。すすめは描かれない | N/A |
| 休息を終える | 開いた状態で Enter | `end_rest` が呼ばれ、既定表示に戻る。すすめは消える | 既存 |
| IME 変換中 | 変換中に ⌘T | 何もしない (既存の IME ガード) | N/A |

</frozen-after-approval>

## Code Map

- `src/overlay/Overlay.svelte:1734-1744` -- 休息中の枝 (`{:else if resting}`)。すすめの見出しボタンと一覧をここに足す
- `src/overlay/Overlay.svelte:165-185` -- 休息まわりの状態 (`resting` ほか)。開閉の状態をここに並べる。**`forgetPosition()` / `dismissNotices()` / `close()` で畳まないこと** — 引き継ぐ決定である。窓は隠すだけで webview は生き続けるため、コンポーネントの `$state` がそのまま常駐中の記憶になる
- `src/overlay/Overlay.svelte:1202-1210` -- `isSurfaceKey`。引数の型に `'t'` を足して ⌘T を判定する
- `src/overlay/Overlay.svelte:1284-1292` -- ⌘R の分岐。同じ形で ⌘T の分岐を足し、`resting` でなければ何もしない
- `src/overlay/Overlay.svelte:1908-1910` -- 既定表示の案内。休息中の案内に「⌘T で休息のすすめ」を足す
- `src/overlay/Overlay.svelte:1686-1688` -- 一覧の `aria-expanded` の手本
- `src/overlay/Overlay.svelte:1939-1955` -- `.next-action` / `.position` のスタイル。すすめは `.position` 並みの小さな文字で、窓に収まりやすくする (2 列の格子も可)
- `src/overlay/Overlay.test.ts:2078-2190` -- 休息中の既存テストと `enterTheRestingSurface()`。新しいテストはこの節に足す。「残り / 分後 / 経過 / %」を出さない検査は、すすめの文言でも通ること (「20 秒」は可)
- 変更しない: Rust 側 (`commands` / `domain`)、`Intervention.svelte`

## Tasks & Acceptance

**Execution:**
- [x] `src/overlay/Overlay.svelte` -- 開閉の状態・⌘T の分岐・休息中の枝のボタンと一覧・案内・スタイルを足す -- I/O マトリクスの全行
- [x] `src/overlay/Overlay.test.ts` -- I/O マトリクスの各行と、作業中に出ないこと・AD-15 の禁止語を検査する -- 回帰を防ぐ
- [x] `README.md` -- 休息の節に、すすめと ⌘T を 2〜3 行で書き足す -- 打鍵の表と食い違わせない

**Acceptance Criteria:**
- Given 休息中の面, when すすめを開く, then 一覧は窓の中で読める (はみ出す分はスクロールで辿れる)
- Given 休息中の面, when すすめを開閉する, then `invoke` は一度も呼ばれない

## Implementation Notes

## Spec Change Log

## Review Triage Log

| # | 層 | 指摘 | 判定 | 根拠 | 行き先 |
|---|---|---|---|---|---|
| 1 | EC | `stateError` があるのに `resting` が真のとき、案内に ⌘T が出て隠れた状態を反転する | false | `commands::snapshot_of` は `state_error` を立てるとき必ず `resting: false` を返す (コアが無いときだけ。`commands/mod.rs:1520` の検査が固定) | reject |
| 2 | EC | ⌘T を押し続けると自動反復で開閉が明滅し、最終状態が反復回数で決まる | low | ⌘T の分岐は `event.repeat` を見ていない。直しは 1 条件の追加 | patch |
| 3 | BH | README の打鍵の表に ⌘T が無い | false | その表は作業中の既定表示の打鍵であり、そこで ⌘T は何もしない。⌘T は休息の節に書いてあり、食い違いは無い | reject |
| 4 | BH | 閉じているとき `aria-controls` が存在しない要素を指す | low | `<ul id>` は `{#if restAdviceOpen}` の中にしか無い。直しは属性の条件化だけ | patch |
| 5 | BH / VG | 「切り取られない」受け入れテストが `overflow` の略記しか見ず、縦横の個別指定による切り取りを見逃す | low | jsdom は個別指定だけのとき略記を空で返す (VG が実測)。隣の一覧のテストは `overflowY/X` を見ている | patch |
| 6 | BH | すすめを開くと案内と「休息を終える」ボタンが窓 (640×280) の下へ押し出されうる | maybe-false | 見積りでは中身が約 270px で、内側の高さ 224px を超えうる。jsdom に配置計算は無い。`make dev` で実寸を見れば決まる | defer (medium・未検証) |
| 7 | BH | 2 列の格子は行方向に並ぶため「この順」が読み取りにくい | low | DOM の順は仕様どおり。横書きで行方向に読むのは自然で、1 列にすると #6 を悪化させる | reject |
| 8 | BH / VG | 作業中の ⌘T が飲まれることも、次の休息へ状態を漏らさないことも検査されていない | low | 作業中のテストは `press` で即時の結果しか見ていない。`if (!resting) return` を消しても通る (VG が実証) | patch |
| 9 | BH | 作成・修正・一覧の面での ⌘T に検査が無い | false | それらの分岐は ⌘T の分岐より前に return しており、今日、悪い結果は起きない | reject |
| 10 | BH | Tab でボタンに当てて Enter を押すと、休息の終了とボタンの click が二重に起きうる | false | 打鍵は `<svelte:window on:keydown>` で受け、休息中の Enter はそこで `preventDefault` される。既定動作 (click) は取り消される | reject |
| 11 | BH | 「3 回」「5 秒」が AD-15 の回数の禁止に触れる | false | 文言は利用者が凍結した決定であり、数えも計時もしない目安である | reject |
| 12 | BH | Code Map の行番号が古い | low | 直しは仕様の編集であり、対象外 | reject |

## Verification

**Commands:**
- `make test` -- expected: cargo test / pnpm check / pnpm test がすべて通る
- `make lint` -- expected: 警告なし

**Manual checks (if no CLI):**
- `make dev` で ⌘R → 休息中の面で ⌘T を押し、すすめが開閉すること、窓の中で読めること
