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
