# Deferred Work

- source_spec: none
  summary: ドメインモデルと永続化 — タスク・ステップ・現在地の定義、完了の宣言、ステップの追記と分割、現在地の一意性 (CAP-4, CAP-5, CAP-6)。
  evidence: v1 の 10 capability を単一ゴール基準で分割した際に、常駐の骨格 (CAP-1, CAP-3) を先行させたため。単独ではユーザーに見える面を持たないが、切り替えと再開の前提となる。

- source_spec: none
  summary: 切り替えと再開 — 既定表示の最小化、中断メモの記録、再開時の位置提示 (CAP-2, CAP-7, CAP-8)。
  evidence: v1 の中核仮説そのものだが、常駐の骨格とドメインモデルの双方に依存するため後続とした。

- source_spec: none
  summary: 全体像の開示面 — 明示操作によってのみ到達し、オーバーレイを閉じた時点で破棄される一覧 (CAP-9)。
  evidence: 独立して出荷可能だが、ドメインモデルに依存するため後続とした。

- source_spec: none
  summary: 休息介入 — 連続作業時間の計時、非活性パネルによる提示、一時登録ホットキーでの二択応答 (CAP-10)。
  evidence: 独立して出荷可能。v1 唯一の能動機能であり、未解決の UI 表現 (SPEC.md の open question) を抱えるため、骨格の確立後に着手する。

- source_spec: `_bmad-output/implementation-artifacts/spec-resident-shell.md`
  summary: 誤終了阻止の第 1 層 (`enable_macos_default_menu(false)`) と第 2 層 (`CloseRequested` → `prevent_close`) に自動検証が無く、1 行消しても全テストが通る。
  evidence: レビュー #39 (verification-gap、検証済み)。どちらも AppKit を起動した実アプリでしか観測できず、本リポジトリはまだ UI 自動化の基盤を持たない。今回のループバックを引き起こしたのと同じ欠陥 (一打で常駐が死ぬ / 呼び出し面が永久に失われる) が再発しても検出されない。UI 自動化を導入するか、リリース前の手動確認を必須項目として運用で固定するかの判断を要する。

- source_spec: `_bmad-output/implementation-artifacts/spec-resident-shell.md`
  summary: CI が存在せず、`cargo test` / `clippy -D warnings` / `pnpm check` が人の記憶に依存している。
  evidence: レビュー #30 (第 1 回 #22 から carried)。安価で決定的な検査が揃っているのに push 時に走らない。次のスライスで静かに回帰する。

- source_spec: `_bmad-output/implementation-artifacts/spec-domain-model.md`
  summary: 自動起動マーカーを個別ファイルから SQLite の設定テーブルへ移し、`adapters/autostart/mod.rs:17-18` の TODO を解消する (AD-11)。
  evidence: 永続化スライスが設定テーブルを作るため前提は揃うが、移行には既存インストールのマーカーを読んで引き継ぐ処理が要る。これを誤ると「ユーザーの解除を後の起動が覆さない」という slice 1 の保証が壊れる。自動起動は自動テストを持たず前回のループバックの発生源でもあるため、スライスを単一ゴールに保つ判断として分離した。

- source_spec: `_bmad-output/implementation-artifacts/spec-domain-model.md`
  summary: `lib.rs` の永続化の配線 (app data dir 解決 → DB を開く → スキーマ適用 → 復元 → `manage`) に自動検証が無い。`restore_core` を丸ごと消しても全テストが通る。
  evidence: レビュー対象の 4 つの失敗経路 (ディレクトリ不明・DB 開けない・破損・成功) のうち、ユニットテストが覆うのはストレージアダプタとコアの内側だけであり、Tauri の `AppHandle` を要する配線そのものは覆えない。既存の「誤終了阻止の第 1・2 層に自動検証が無い」と同じ性質の穴であり、UI 自動化の基盤を入れる判断と併せて扱うのが妥当。

- source_spec: `_bmad-output/implementation-artifacts/spec-domain-model.md`
  summary: 破損した DB を隔離して作り直す経路が無い。(復元失敗が利用者に伝わらない件は spec-switch-and-resume.md の `CORE_MISSING` 表示で部分的に解消したが、隔離・再作成の経路は未着手のまま残る。)
  evidence: レビュー #10 (blind-hunter / edge-case-hunter、検証済み)。`restore_core` は `log::error!` を出すだけで、ホットキー登録失敗が持つオーバーレイ表示・メニューバーの状態行のような可視面を持たない。現在地の喪失はホットキーの死と同程度に重大でありながら、利用者はログファイルを開く以外に気づく手段が無い。本スライスの凍結された Intent が「ユーザーに見える面を持たない一層」であるため可視面を足せず、可視面が立つ CAP-7 と併せて扱う。

- source_spec: `_bmad-output/implementation-artifacts/spec-switch-and-resume.md`
  summary: タスクとステップの作成・編集の面 — CAP-4 の利用者に見える半分。ドメインには `create_task` / `insert_step` / `append_step` / `split_step` が既にあるが、そこへ到達する UI もコマンドも存在しない。
  evidence: スライス分割の当初計画 (常駐の骨格 → ドメインモデル → 切り替えと再開 → 開示面 → 休息介入) のどこにも作成の面が無い。CAP-2/7/8 を単一ゴールに保つため本スライスには含めず、直後の独立したスライスとする。それまで切り替えと再開は sqlite3 で投入した種データに対してしか手動検証できない。

- source_spec: `_bmad-output/implementation-artifacts/spec-switch-and-resume.md`
  summary: 記録済みの**中断メモ**を消す経路が無い。入力欄を空にしても「省略」と解釈され、古いメモが残り続ける。
  evidence: レビュー #18。実装は凍結された I/O マトリクス (「メモを省いて切り替え → メモは変更しない」) のとおりであり逸脱ではない。省略と削除を区別するかは編集の面を持つスライスが決めるべき論点であり、本スライスの Intent は編集の面を明示的に除外している。

- source_spec: `_bmad-output/implementation-artifacts/spec-switch-and-resume.md`
  summary: **非活性** (休息中) の**現在地**から切り替えると、休息が黙って終わり**連続作業時間**の起点も更新される。
  evidence: レビュー #19。`move_to` が `Inactive` を `Active` へ遷移させるため。ただし `deactivate_current_position` を公開するコマンドが無く、今日この状態へ到達する経路は手編集の DB 以外に無い。切り替えが休息を終わらせてよいかは休息の意味付けそのものであり、CAP-10 が決める。

- source_spec: `_bmad-output/implementation-artifacts/spec-switch-and-resume.md`
  summary: `switch_record` に保持期間の方針が無く、`departed_step_id` の外部キーに `ON DELETE` の方針も無い。
  evidence: レビュー #20。今日は**ステップ**の削除経路が存在しないため無害だが、CAP-20 (腐敗による消滅の経路) が削除を導入した時点で衝突する。CAP-20 の設計と併せて決めるのが妥当。

- source_spec: `_bmad-output/implementation-artifacts/spec-task-creation.md`
  summary: 進行中の**タスク**への**ステップ**の追記と分割の到達経路 — CAP-5 の利用者に見える半分。ドメインには `insert_step` / `append_step` / `split_step` が既にあるが、そこへ到達する UI もコマンドも無い。
  evidence: 作成 (CAP-4) と追記/分割 (CAP-5) はそれぞれ独立して出荷できる。作成だけで sqlite3 による種まきが終わり v1 の 1 か月の試用に入れるため、単一ゴールを保って作成を先に出した。FR-5 が対象とするのは「現在の**タスク**」であり、対象の選択に一覧を要さないため CAP-9 とは独立に実装できる。

- source_spec: `_bmad-output/implementation-artifacts/spec-task-creation.md`
  summary: 「作成して着手」で**現在地**を移すとき、離脱側に**中断メモ**の機会も**切り替え履歴**も無く、SM-C3 の記入率から漏れる。
  evidence: レビュー #11。用語集は**切り替え**を「**現在地**をあるステップから別のステップへ移す操作」と定義しており、この移動もそれに当たる。ただし「機会を与えていない移動を記入率の分母に入れてよいか」は設計判断であり、CAP-9 が任意の**ステップ**への移動を導入するとき同じ問いに直面する。CAP-9 と併せて決める。

- source_spec: `_bmad-output/implementation-artifacts/spec-task-creation.md`
  summary: テキスト入力面で貼り付け・全選択・取り消し (⌘V / ⌘C / ⌘X / ⌘A / ⌘Z) が効かない見込みである。アプリのメニューが存在しないため。
  evidence: レビュー #12。`enable_macos_default_menu(false)` かつ `Builder::menu()` 未設定であり、macOS ではこれらの打鍵はメニューの key equivalent 経由で WKWebView に届く。終了・閉じるを含まない編集メニューのみを足せば解決する見込みだが、既定メニューの無効化は slice 1 の誤終了阻止の第 1 層そのものであり、その層には自動検証が無い (既知の申し送り)。無検証のまま触れば、一打で常駐が死ぬ状態を再発させうる。誤終了阻止の自動検証と併せて扱う。
