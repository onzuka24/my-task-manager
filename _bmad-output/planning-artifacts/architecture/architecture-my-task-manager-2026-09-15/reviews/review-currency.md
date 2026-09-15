---
name: review-currency
type: review
lens: technology-currency
target: ARCHITECTURE-SPINE.md
reviewed: '2026-09-15'
method: independent web verification (crates.io API, npm registry, GitHub API, upstream source)
verdict: 'spine is substantially sound; 1 stale pin, 3 unpinned/under-specified choices, 2 unverified-as-stated claims, 2 newly-surfaced limitations'
---

# 技術選定の現時点性レビュー — my-task-manager

すべての項目を一次情報 (crates.io API / npm registry / GitHub API / 上流ソースコード) で再確認した。スパインと `.memlog.md` の記述、および訓練データは信用していない。

## 判定サマリ

| # | 主張 | 判定 |
| --- | --- | --- |
| 1 | Tauri 2.11.5 が安定版 | CONFIRMED |
| 2 | Svelte 5.57 が安定版 | CONFIRMED |
| 3 | tauri-plugin-global-shortcut 2.3.1 | **CORRECTED** → 2.3.2 |
| 4 | tauri-plugin-autostart 2.5.1 | CONFIRMED |
| 5 | 両プラグインが Tauri 2.11.x と整合 | CONFIRMED |
| 6 | tauri-nspanel が存在し保守されている | CONFIRMED (要注記) |
| 7 | tauri-nspanel が NonactivatingPanel を支持 | CONFIRMED (上流ソースで確認) |
| 8 | 既知の未解決 issue (#19) | **CORRECTED** → #19 は 2024-02 に解決済。ただし**別の未解決 issue #120 が存在** |
| 9 | rusqlite / sqlx が 2026 年の妥当な選択 | CONFIRMED (ただし**未決定のまま**) |
| 10 | global-shortcut の実行時 登録/解除 (AD-7) | CONFIRMED (上流ソースで確認) |
| 11 | Electron 168-300MB vs Tauri 30-50MB | **PARTIALLY UNVERIFIABLE** — 方向性は正しいが計測根拠が弱い |
| 12 | full_screen_auxiliary / hides_on_deactivate | CONFIRMED (memlog の記述どおり API が実在) |

**confirmed 8 / corrected 3 / partially-unverifiable 1**

---

## 1. Tauri 2.11.5 — CONFIRMED

crates.io API (`/api/v1/crates/tauri`) の一次データ:

- `max_stable_version`: **2.11.5**、公開日 **2026-07-01**、DL 5,261,525
- 直近系列: 2.11.4 (2026-06-30) / 2.11.3 (2026-06-17) / 2.11.2 (2026-05-16) / 2.11.1 (2026-05-06) / 2.11.0 (2026-04-30)

memlog 行 17 の「2.11.5 (2026-07-01 リリース)」は日付まで正確。2026-07-01 以降 2 ヶ月半パッチが出ておらず、系列として落ち着いている。

**注記 (スパイン未記載):** `3.0.0-alpha.0` が **2026-09-13**、つまり本レビューの 2 日前に crates.io へ公開された。Tauri 3 系の開発が始まっている。v1 の実装期間中に 3.0 が安定化する可能性は低いが、`Cargo.toml` では `tauri = "2.11"` のように 2 系へ明示的に固定すべきで、`"*"` や無指定にしてはならない。スパインのスタック表は「2.11.5」とだけ書いており、これが下限なのか固定なのかを示していない。

## 2. Svelte 5.57 — CONFIRMED

npm registry (`registry.npmjs.org/svelte/latest`) の `version` フィールドは **5.57.0**。すなわち 5.57 は現時点の `latest` タグそのものである。公式ブログ (What's new in Svelte: September 2026) によれば 5.57.0 は **2026-08-28** 公開で、`SvelteMap.getOrInsert` / `createContext` の第三戻り値 `has` / `<select defaultValue>` / `svelte/server` の型追加を含む。

memlog 行 23 の「SvelteKit 3 は RC、2.x 保守中」および「ルーティング/SSR 不要のため SvelteKit を使わない」という判断は、本件がオーバーレイ単一画面であることから妥当。SvelteKit を避ける決定は RC 版の安定性に依存しないため、この点にリスクはない。

## 3-5. プラグイン — 1 件 CORRECTED

### tauri-plugin-global-shortcut — **スパインの 2.3.1 は 1 パッチ古い**

crates.io API の一次データ:

| version | 公開日 | DL |
| --- | --- | --- |
| 3.0.0-alpha.0 | 2026-09-13 | 30 |
| **2.3.2 (最新安定)** | **2026-05-28** | 1,065,710 |
| 2.3.1 | 2025-10-27 | 1,864,801 |
| 2.3.0 | 2025-06-25 | 489,629 |

スパイン §スタックの `2.3.1` は **2.3.2 (2026-05-28) に更新すべき**。memlog 行 23 の調査時点で既に 2.3.2 が 3 ヶ月半前に出ていたため、これは「調査したが古い値を採った」のではなく **調査漏れ** の可能性が高い。差分はパッチレベルであり設計上の含意はないが、スパインが版を名指ししている以上は正確であるべきである。

### tauri-plugin-autostart — CONFIRMED

`max_stable_version` は **2.5.1** (2026-09-13 に 3.0.0-alpha.0 が出たが安定版は据え置き)。2.5.1 の公開日は **2025-10-27**。約 11 ヶ月更新がないが、これは放置ではなく機能的に完成しているためで、同一リポジトリ (plugins-workspace) が活発である以上、保守停止とは読めない。スパインの記述は正確。

### Tauri 2.11.x との整合 — CONFIRMED

両プラグインとも `tauri-apps/plugins-workspace` の `v2` ブランチで Tauri 2 系に追随しており、2.x の caret 要求で解決される。2.11.5 との組み合わせに既知の非互換はない。

## 6-8. tauri-nspanel — 存在し保守されているが、注記 3 件

### 実在と保守状況 — CONFIRMED

GitHub API (`repos/ahkohd/tauri-nspanel`):

- `archived`: **false**
- 最終 push: **2026-08-19** / 最終更新: 2026-09-10
- open issues: **12** / stars: 418
- default branch: **`v2.1`**

採用実績も実在する (Cap, Screenpipe, EcoPaste, Hyprnote, Coco, Overlayed, JET Pilot ほか)。放棄されたプロジェクトではない。

### **FLAG — crates.io に存在しない。git 依存である**

`https://crates.io/api/v1/crates/tauri-nspanel` は **HTTP 404**。このクレートは crates.io に公開されていない。README が指示する唯一の導入方法は:

```toml
tauri-nspanel = { git = "https://github.com/ahkohd/tauri-nspanel", branch = "v2.1" }
```

**ブランチ指定の git 依存**であり、`rev` による固定ではない。これは AD-13 (ローカルビルド) と組み合わさると実害がある — 上流が `v2.1` ブランチに push するたびに、再ビルドの結果が黙って変わる。issue #116「Crates.io artifact」(2026-05-20 起票、未解決) が公開を求めているが、未対応。

**推奨:** スパイン §スタック表の `tauri-nspanel` 行にバージョンが一切書かれていない (唯一「非活性パネル用 (AD-6)」とのみ)。**`branch = "v2.1"` ではなく `rev = "<commit sha>"` で固定する**ことをスタック表に明記すべき。現行の `v2.1` ブランチの package version は `2.1.0`。

### 上流の tauri 依存 — 整合するが margin は小さい

`v2.1` ブランチの `Cargo.toml`:

```toml
tauri = { version = "2.8.5", features = ["macos-private-api"] }
objc2 = "0.6.1"
objc2-app-kit = "0.3.1"
rust-version = "1.75"
```

caret 解決により **2.11.5 と互換** (2.8.5 ≤ 2.11.5 < 3.0.0)。ただし上流が検証しているのは 2.8.5 であり、2.11.5 での動作確認は自分で行う必要がある。

**FLAG:** `macos-private-api` feature が必須である。スパインはこれに言及していない。AD-13 が署名なしローカル配布であり App Store 提出がないため実害はないが、**この feature を有効にしたアプリは App Store に提出できない**。「先送り §署名への移行」の項に、署名は取得しても App Store 経路は tauri-nspanel 採用により恒久的に閉じる、と記録しておくべきである。

### NonactivatingPanel の支持 — CONFIRMED (一次ソース確認)

上流 `src/builder.rs` を直接確認した:

```rust
// builder.rs:422
pub fn nonactivating_panel(mut self) -> Self {
    self.0 |= objc2_app_kit::NSWindowStyleMask::NonactivatingPanel;
```

```rust
// builder.rs:161
pub fn full_screen_auxiliary(mut self) -> Self {
    self.0 |= objc2_app_kit::NSWindowCollectionBehavior::FullScreenAuxiliary;
```

`docs/panel-builder.md` にもビルダー API として文書化されている (`.style_mask(StyleMask::empty().nonactivating_panel())`、`.collection_behavior(...)`、`.hides_on_deactivate(false)`)。

**これは AD-6 と memlog 行 24/30 を全面的に裏づける。** memlog が必須と名指しした 3 要素 — NonactivatingPanel / `set_hides_on_deactivate(false)` / `full_screen_auxiliary` — はいずれも一級の API として実在する。`can_become_key_window` は `tauri_panel!` マクロの `config:` ブロックで指定する (ビルダーメソッドではない) 点のみ、実装時の注意事項。

### **CORRECTED — issue #19 は解決済。ただし別の未解決 issue がある**

指示にあった「NSWindowStyleMaskNonactivatingPanel の既知の未解決 issue」は **issue #19** を指すと思われるが、これは **closed** である。経緯:

- 2024-02-06 起票 (zzzze)。当時 Cocoa バインディングに該当の enum 値がなく、自作すると panic した
- 2024-02-12 メンテナ回答: **パネルへ変換する前に window の decorations を false にする必要がある**
- 2024-02-13 メンテナが Menubar app の例を追加
- 2024-02-18 起票者が解決を確認して close

当時の根本原因 (バインディングに定数がない) は、上流が `objc2-app-kit` へ移行し `NSWindowStyleMask::NonactivatingPanel` が正式に存在する現在、**完全に消滅している**。2024 年当時の情報に基づいて「未解決の既知問題がある」と考えているなら、その前提は古い。

**ただし、より新しい未解決 issue が 1 件あり、そちらは AD-6 に直接関係する:**

**issue #120 (2026-07-22 起票、OPEN)** — 「Replacing the style mask via `style_mask()`/`set_style_mask()` aborts with a non-unwinding panic」。ビットを OR で追加する分には動くが、**生きているウィンドウのマスクビットを消そうとすると AppKit が例外を投げ、Rust 側で非巻き戻し panic (= プロセス即死) になる**。

独立した裏づけ (philz.blog, 2025-03-30) も同じ領域の別の症状を報告している: `NSPanel` 初期化**後**に NonactivatingPanel フラグを切り替えても WindowServer 側のタグが同期されず、ウィンドウが key に見えるのにキー入力を受け付けない状態になる。回避には private API `_setPreventsActivation:` の呼び出しが必要。

**AD-6 への含意 (ブロッカーではないが設計制約):** 介入パネルの style mask は **生成時に一度だけ確定させ、実行時に変更してはならない**。AD-6 は「この二つを同一のウィンドウ実装で兼ねてはならない」と既に二面分離を規定しているため、**現行のスパインはこの地雷を踏まない設計になっている**。ただし規定の根拠が「フォーカス挙動の違い」だけになっている。AD-6 の Rule に *「パネルの style mask は生成時に確定し、実行時に変更しない (上流 #120: 生存中のマスク変更はプロセス中断を招く)」* を一行足すことを推奨する。これは将来「オーバーレイをパネルに昇格させる」といった最適化を誰かが思いつくのを防ぐ。

### その他の未解決 issue (参考)

- **#118 / #117 (2026-07-03, OPEN)** — RUSTSEC-2026-0194 / RUSTSEC-2026-0195。依存 XML パーサの DoS 系勧告。ネットワーク入力を扱わない本アプリでは実質無害だが、`cargo audit` を回すと警告が出る
- **#104 (2025-10-20, OPEN)** — window level を 20 超に設定すると入力メソッド (中国語 IME 等) が阻害される。**日本語 IME を使う本件では要注意。** AD-6 の介入パネルは入力を受け付けない設計 (AD-7 でホットキー応答) なのでパネル自身には影響しないが、**パネルが最前面にいる間に背後のアプリで日本語入力ができなくなる可能性**がある。これは PRD §7.2「ユーザーの入力先を奪わない」を実質的に破りうる。`PanelLevel::Floating` を使い、不必要に高い level を指定しないこと
- **#119 (2026-07-11, OPEN)** — パネルを閉じる際の fatal runtime error。常駐して隠すだけの本設計 (スパイン §起動と常駐: 生成して隠す) では close を呼ばないため該当しない見込み

## 9. rusqlite / sqlx — 両者とも健在。ただし**スパインが選んでいない**

crates.io の一次データ:

| crate | 最新安定 | 公開日 |
| --- | --- | --- |
| **rusqlite** | **0.40.2** | 2026-08-08 |
| **sqlx** | **0.9.0** | 2026-05-21 |

どちらも 2026 年時点で活発に保守されている。rusqlite は 0.38.0 (2025-12) → 0.39.0 (2026-03) → 0.40.0 (2026-05) → 0.40.2 (2026-08) と 2-3 ヶ月ごとにリリース、累計 1 億 DL 超。sqlx は 0.8.6 (2025-05) から約 1 年を経て 0.9.0 (2026-05-21) へメジャー前進、Rust 1.94.0 以上を要求。

**両者とも「生きた、正気な選択」である。** この点でスパインに誤りはない。

**FLAG — ただしこれは検証済みの決定ではなく、未決定である。** スパイン §スタックは「rusqlite または sqlx 経由」、memlog 行 25 も「rusqlite/sqlx で直接扱い」と書いており、**どちらを使うか決めていない**。AD-4 が本当に固定しているのは「`tauri-plugin-sql` を使わない」という否定形だけで、肯定側は空欄のまま build-substrate として下流に渡されている。

本件の性質からは **rusqlite が明確に適する**:

- ドメインコアは同期的であり (AD-8 の計時もコア所有)、sqlx の async ランタイム (tokio) をコアに引き込む理由がない。AD-1 の「コアは OS を知らない」に対し、async ランタイム依存は余計な結合である
- 単一プロセス・単一ユーザー・ローカルファイル。接続プールもコンパイル時クエリ検証も要らない規模
- sqlx 0.9.0 の Rust 1.94.0 要求に対し rusqlite の MSRV は緩い

**推奨:** スタック表を `rusqlite 0.40` に確定し、AD-4 の Rule に理由 (コアを同期に保ち async ランタイムを持ち込まない) を一行添える。「または」を残したままだと、実装時に人によって選択が割れる — これは AD-10 が名前について防いでいるのと同種の分岐である。

## 10. AD-7 の実行時 登録/解除 — CONFIRMED (一次ソース確認)

**可能である。** docs.rs の `GlobalShortcut<R>` API に以下が揃っている:

```rust
pub fn register<S>(&self, shortcut: S) -> Result<(), Error>
pub fn on_shortcut<S, F>(&self, shortcut: S, handler: F) -> Result<(), Error>
    where F: Fn(&AppHandle<R>, &Shortcut, ShortcutEvent) + Send + Sync + 'static
pub fn register_multiple<S, T>(&self, shortcuts: S) -> Result<(), Error>
pub fn on_shortcuts<S, T, F>(&self, shortcuts: S, handler: F) -> Result<(), Error>
pub fn unregister<S>(&self, shortcut: S) -> Result<(), Error>
pub fn unregister_multiple<T, S>(&self, shortcuts: S) -> Result<(), Error>
pub fn unregister_all(&self) -> Result<(), Error>
pub fn is_registered<S>(&self, shortcut: S) -> bool
```

すべて `&self` を取り `AppHandle` から任意時点で呼べる。起動時限定の API ではない。**AD-7 の「介入の表示中のみ登録し、応答後ただちに解除する」は実装可能である。**

さらに下層を確認した。`tauri-apps/global-hotkey` の macOS 実装 (`src/platform_impl/macos/mod.rs`) は Carbon の `RegisterEventHotKey` / `UnregisterEventHotKey` を対で呼んでいる:

```
27:  InstallEventHandler, OSStatus, RegisterEventHotKey, RemoveEventHandler, UnregisterEventHotKey,
117:     let result = RegisterEventHotKey(
190:     if UnregisterEventHotKey(ptr) != noErr as _ {
```

ここから 2 点が導かれる:

1. **登録/解除は OS レベルで対称であり、繰り返し行える。** AD-7 の要求に構造的な無理はない
2. **`RegisterEventHotKey` は Accessibility 権限を要求しない。** グローバルキー監視 (`NSEvent.addGlobalMonitorForEvents`) と異なり「この組み合わせが押されたときだけ通知せよ」という限定的な登録であるため。FR-1 のホットキーも AD-7 の一時ホットキーも、**権限ダイアログなしで動く**

これは memlog 行 33 の [ASSUMPTION] に対する補正情報でもある。同 [ASSUMPTION] は「署名なしのため v2 の FR-11 観測で Accessibility 権限がリセットされうる」と述べており、これは v2 の観測 (NSWorkspace / Accessibility) について正しい。**ただし v1 のホットキーは Accessibility を必要としないため、この懸念は v1 には及ばない。** スパインの「先送り §署名への移行」は v2 の話として正しく限定されており、修正は不要だが、v1 実装者が「ホットキーにも権限が要るのでは」と誤解しないよう AD-7 に一行あってよい。

### **FLAG — AD-7 に実装上の罠が一つある (スパイン未記載)**

`on_shortcut` のハンドラは `ShortcutEvent` を受け取り、その `state` は `ShortcutState::{Pressed, Released}` の 2 値である。**同一のキー押下に対しハンドラは押下時と解放時の 2 回発火する。**

Tauri issue #10025 (「Global shortcut event fire twice on macOS」、2024-06-09 起票) は **closed as `not_planned`** であり、これはバグではなく仕様として確定している。修正を待つ対象ではない。

**AD-7 への含意:** 介入の二択をホットキーで受ける際、`state` を判定せずに応答処理を書くと **一回の押下で二重に応答が確定する**。AD-7 の Rule は「応答後ただちに解除する」としているが、解除が Pressed ハンドラ内で走れば Released は届かないため偶然救われる可能性はある — が、それは規律ではなく偶然に依存する。AD-7 の Rule に *「ハンドラは `ShortcutState::Pressed` のみを応答として扱う」* を明記すべき。

同様に macOS では、同じホットキーを解除せずに二重登録すると OS バージョンによって**エラーではなく無言の失敗**になるという報告がある。AD-7 の「登録が残存したまま介入が消える経路を作ってはならない」はこの危険を既に禁じており、**この点はスパインが正しく先回りしている**。`is_registered` を防御的に使える。

## 11. Electron vs Tauri のメモリ — **PARTIALLY UNVERIFIABLE**

memlog 行 17-18 および行 21-22 は、Electron 43 の待機時 168-300MB と Tauri の 30-50MB を根拠に **「Electron は PRD §8 の 100MB 上限を 1.7-3 倍超過するため制約により脱落する」** と結論し、これを「好みではなく制約による除外」と位置づけている。この位置づけは検証に耐えるか。

**確認できたこと:**

- Electron 43 が 2026-06-30 に stable 化し、Chromium 150 / Node 24.17.0 / V8 15.0 を含む点は複数ソースで一致。memlog 行 17 の記述は正確
- 2026 年の複数の比較記事が、Tauri 約 42MB 対 Electron 43 約 168MB、あるいは Tauri 30-50MB 対 Electron 150-300MB といった数値を挙げている。方向と桁は一貫している

**確認できなかったこと / 弱点:**

1. **一次計測が存在しない。** 見つかった数値はすべて二次的な比較ブログ記事であり、測定手順・計測対象アプリ・OS・ウィンドウ数を開示した統制された計測ではない。ある記事は「often 100-300MB」「often 20-100MB」という**範囲の目安**と明記しており、特定バージョンの実測値ではない。memlog が「実測報告」と呼んでいる根拠は、この水準の情報である
2. **計測手法そのものに既知の異論がある。** Tauri 本体の issue #5889「Memory benchmark might be incorrect: Tauri might consume more RAM than Electron」(closed、最終更新 2024-05) は、**RSS ベースの計測は Chromium 系がプロセス間で共有するメモリを重複計上するため Tauri 有利に歪む**と主張し、USS/PSS で測ると Tauri のほうが 90MB 以上多かったケースを報告している
3. **macOS 固有の落とし穴として、Tauri 側の数値は過少計上になりやすい。** Tauri の macOS バックエンドは WKWebView であり、WKWebView の描画・JS 実行は **別プロセス (`com.apple.WebKit.WebContent`) に出る**。アプリ本体の RSS だけを見ると、この分がまるごと計上されない。「Tauri 30-50MB」がアプリプロセス単体の値であれば、実効的な常駐フットプリントはこれより大きい

**評価:** 「Electron では 100MB 上限を満たせず、Tauri なら満たせる公算が大きい」という**結論の方向は妥当**であり、Tauri 採用の判断を覆すものではない。実際 Tauri を採る理由は memlog 行 21 が挙げる 4 点のうちメモリ以外の 3 点 (Web フロントでの開発速度、Rust から macOS ネイティブ API に降りられること、移植余地) だけでも十分に立つ。

**しかし「制約による機械的な除外であって判断ではない」という言い方は、根拠の強度を実際より高く見せている。** 実態は「複数の二次情報が一貫して示す桁の差に基づく、妥当だが計測で確定はしていない判断」である。

**推奨 (2 点、いずれも設計変更ではなく記録の正確化):**

- memlog 行 18 の「実測報告で」を、出典の性質に合うよう弱める。もしくは Tauri 採用の根拠を、検証に耐える他の 3 点に主として置き換える
- **PRD §8 の 100MB を、実装中に自分で測る検証項目としてスパインに残す。** 現在スパインの「先送り §テスト戦略」は「v1 の規模では方針を固定する必要がない」としており、**待機時リソースの実測が誰の責任でもない状態になっている。** 100MB は PRD が課した数値制約であり、二次情報ではなく自分の実機で確認されるべき唯一の項目である。加えて、macOS では `WebContent` / `Networking` の各ヘルパプロセスを合算して測る、と計測方法まで書いておくこと

## 12. パネルの必須設定 — CONFIRMED

memlog 行 24/30 が「必須」と名指しした `set_hides_on_deactivate(false)` と `full_screen_auxiliary` の collection behavior は、いずれも上流に実在する (`builder.rs:161`、`docs/panel-builder.md:49,170`)。フルスクリーン作業中にパネルを出すために `FullScreenAuxiliary` が要るという理解も AppKit の挙動として正しい。**この 2 つは正確に調査されている。**

---

## 対応すべき事項 (優先順)

| 優先 | 箇所 | 対応 |
| --- | --- | --- |
| 高 | §スタック | `tauri-plugin-global-shortcut` を **2.3.1 → 2.3.2** に更新 |
| 高 | §スタック | `tauri-nspanel` に **`rev = "<sha>"` での固定**を明記 (crates.io 非公開のブランチ git 依存であるため) |
| 高 | §スタック / AD-4 | SQLite ドライバを **rusqlite に確定** (「または sqlx」を残さない)。理由: コアを同期に保ち async ランタイムを持ち込まない |
| 高 | AD-7 | Rule に **「`ShortcutState::Pressed` のみを応答として扱う」** を追加 (押下と解放で 2 回発火する。Tauri #10025 は not_planned で確定仕様) |
| 中 | AD-6 | Rule に **「パネルの style mask は生成時に確定し実行時に変更しない」** を追加 (上流 #120、未解決、プロセス即死) |
| 中 | §先送り テスト戦略 | **待機時メモリ/CPU の実測を検証項目として明記。** macOS ではヘルパプロセスを合算して測る |
| 中 | §先送り 署名への移行 | `macos-private-api` feature 必須により **App Store 経路は恒久的に閉じる**ことを記録 |
| 低 | AD-6 / AD-7 | window level を上げすぎると **日本語 IME を阻害しうる** (上流 #104、未解決)。`PanelLevel::Floating` を超える level を使わない |
| 低 | §スタック | `tauri = "2.11"` と 2 系に明示固定 (3.0.0-alpha.0 が 2026-09-13 に公開済) |
| 低 | memlog 行 18 | Electron メモリの根拠を「実測報告」から出典相応の表現へ弱める |

## 出典

- [crates.io — tauri](https://crates.io/crates/tauri) / [tauri-plugin-global-shortcut versions](https://crates.io/crates/tauri-plugin-global-shortcut/versions) / [tauri-plugin-autostart](https://crates.io/crates/tauri-plugin-autostart) / [rusqlite](https://crates.io/crates/rusqlite) / [sqlx](https://crates.io/crates/sqlx)
- [Tauri release index](https://v2.tauri.app/release/) / [tauri@2.11.5](https://v2.tauri.app/release/tauri/v2.11.5/)
- [npm registry — svelte](https://registry.npmjs.org/svelte/latest) / [What's new in Svelte: September 2026](https://svelte.dev/blog/whats-new-in-svelte-september-2026)
- [docs.rs — GlobalShortcut](https://docs.rs/tauri-plugin-global-shortcut/latest/tauri_plugin_global_shortcut/struct.GlobalShortcut.html) / [ShortcutState](https://docs.rs/tauri-plugin-global-shortcut/latest/tauri_plugin_global_shortcut/enum.ShortcutState.html) / [Global Shortcut plugin guide](https://v2.tauri.app/plugin/global-shortcut/)
- [tauri-apps/global-hotkey — macOS backend](https://github.com/tauri-apps/global-hotkey/blob/dev/src/platform_impl/macos/mod.rs)
- [ahkohd/tauri-nspanel](https://github.com/ahkohd/tauri-nspanel) / [Cargo.toml (v2.1)](https://github.com/ahkohd/tauri-nspanel/blob/v2.1/Cargo.toml) / [src/builder.rs (v2.1)](https://github.com/ahkohd/tauri-nspanel/blob/v2.1/src/builder.rs) / [docs/panel-builder.md](https://github.com/ahkohd/tauri-nspanel/blob/v2.1/docs/panel-builder.md) / [open issues](https://github.com/ahkohd/tauri-nspanel/issues)
- [tauri-nspanel #19 (closed)](https://github.com/ahkohd/tauri-nspanel/issues/19) / [#120 style mask panic (open)](https://github.com/ahkohd/tauri-nspanel/issues/120) / [#104 IME blocked (open)](https://github.com/ahkohd/tauri-nspanel/issues/104) / [#116 crates.io artifact (open)](https://github.com/ahkohd/tauri-nspanel/issues/116)
- [The Curious Case of NSPanel's Nonactivating Style Mask Flag (2025-03-30)](https://philz.blog/nspanel-nonactivating-style-mask-flag/)
- [tauri #10025 — global shortcut fires twice on macOS (closed, not_planned)](https://github.com/tauri-apps/tauri/issues/10025) / [tauri #5889 — memory benchmark might be incorrect](https://github.com/tauri-apps/tauri/issues/5889)
- [Electron vs Tauri 2026 (PkgPulse)](https://www.pkgpulse.com/guides/electron-vs-tauri-2026)
