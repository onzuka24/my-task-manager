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
  summary: 復元の失敗 (アプリデータディレクトリ不明・DB を開けない・破損) が利用者にまったく伝わらず、破損した DB を隔離して作り直す経路も無い。
  evidence: レビュー #10 (blind-hunter / edge-case-hunter、検証済み)。`restore_core` は `log::error!` を出すだけで、ホットキー登録失敗が持つオーバーレイ表示・メニューバーの状態行のような可視面を持たない。現在地の喪失はホットキーの死と同程度に重大でありながら、利用者はログファイルを開く以外に気づく手段が無い。本スライスの凍結された Intent が「ユーザーに見える面を持たない一層」であるため可視面を足せず、可視面が立つ CAP-7 と併せて扱う。
