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
  summary: 誤終了阻止の各層の**実挙動**を確かめる手段が無い。行の削除は spec-hardening.md の固定で捉えられるようになったが、無効化・`cfg` による封じ・未到達の関数への移動は捉えられない。
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
  resolution: `spec-rest-intervention.md` で部分的に決着した。既定表示の Enter は**休息**中には**切り替え**ではなく**休息の終了の宣言**であり (`Overlay.svelte`)、黙って休息が終わる経路は既定表示から消えている。**開示面から**ステップ**を選ぶ経路 (CAP-9) は従来どおり**再入**として扱い、意図的に**活性**へ戻す** — 一覧から行を選ぶ行為は用語集の**再入**そのものであるため。ドメインの `switch_current_position` は依然として**非活性**から呼べば活性化するが、そこへ到達するコマンドは無い。

- source_spec: `_bmad-output/implementation-artifacts/spec-switch-and-resume.md`
  summary: `switch_record` に保持期間の方針が無く、`departed_step_id` の外部キーに `ON DELETE` の方針も無い。
  evidence: レビュー #20。今日は**ステップ**の削除経路が存在しないため無害だが、CAP-20 (腐敗による消滅の経路) が削除を導入した時点で衝突する。CAP-20 の設計と併せて決めるのが妥当。

- source_spec: `_bmad-output/implementation-artifacts/spec-task-creation.md`
  summary: 進行中の**タスク**への**ステップ**の追記と分割の到達経路 — CAP-5 の利用者に見える半分。ドメインには `insert_step` / `append_step` / `split_step` が既にあるが、そこへ到達する UI もコマンドも無い。
  evidence: 作成 (CAP-4) と追記/分割 (CAP-5) はそれぞれ独立して出荷できる。作成だけで sqlite3 による種まきが終わり v1 の 1 か月の試用に入れるため、単一ゴールを保って作成を先に出した。FR-5 が対象とするのは「現在の**タスク**」であり、対象の選択に一覧を要さないため CAP-9 とは独立に実装できる。

- source_spec: `_bmad-output/implementation-artifacts/spec-task-creation.md`
  summary: 「作成して着手」で**現在地**を移すとき、離脱側に**切り替え履歴**が残らない。CAP-9 (spec-disclosure-surface.md) は**自らの経路についてのみ**「移動は記録するが中断メモの機会は与えない」と決めた。作成の経路は intent の外として据え置かれており、同じ観測上の契約に対して二つの振る舞いが並んでいる。
  evidence: レビュー #11。用語集は**切り替え**を「**現在地**をあるステップから別のステップへ移す操作」と定義しており、この移動もそれに当たる。ただし「機会を与えていない移動を記入率の分母に入れてよいか」は設計判断であり、CAP-9 が任意の**ステップ**への移動を導入するとき同じ問いに直面する。CAP-9 と併せて決める。

- source_spec: `_bmad-output/implementation-artifacts/spec-task-creation.md`
  summary: テキスト入力面で貼り付け・全選択・取り消し (⌘V / ⌘C / ⌘X / ⌘A / ⌘Z) が効かない見込みである。アプリのメニューが存在しないため。
  evidence: レビュー #12。`enable_macos_default_menu(false)` かつ `Builder::menu()` 未設定であり、macOS ではこれらの打鍵はメニューの key equivalent 経由で WKWebView に届く。終了・閉じるを含まない編集メニューのみを足せば解決する見込みだが、既定メニューの無効化は slice 1 の誤終了阻止の第 1 層そのものであり、その層には自動検証が無い (既知の申し送り)。無検証のまま触れば、一打で常駐が死ぬ状態を再発させうる。誤終了阻止の自動検証と併せて扱う。

- source_spec: `_bmad-output/implementation-artifacts/spec-rest-intervention.md`
  summary: 常駐プロセスが走っていなかった時間が**連続作業時間**に加算される。**現在地**を**活性**のまま終了し、翌日起動すると、最初の刻み (10 秒後) で**介入**が出る。
  evidence: スリープの検出は「刻みと刻みの間の時計の飛び」でしか行えず (`domain/rest.rs::slept_millis`)、直前の刻みの時刻は永続化されない (AD-2 の状態表に無い)。spec の I/O マトリクスは起動をまたぐ間隔を扱っておらず、実装は spec のとおりである。ただし PRD は「反射的に無視される介入は介入全体の信頼性を損なう」と述べており、毎朝の起動直後に出る介入はまさにそれに当たりうる。直すには「直前の刻みの時刻」を永続化する必要があり、AD-2 の状態表への追加を伴う — アーキテクチャの改訂を要する判断であるため送る。

- source_spec: `_bmad-output/implementation-artifacts/spec-rest-intervention.md`
  summary: 待機時メモリの余裕が 2MB しかない (実測 97.8〜97.9 MB / 上限 100 MB)。起動から数分は 101 MB を指す。
  evidence: AD-14 が予告したとおり、常駐 webview が二つになったことで予算が逼迫した。定常値は予算内だが、三つ目の常駐 webview を足す余地は無く、WebKit 側の版が上がるだけで超過しうる。対処は (a) 提示面の webview を一つに畳む設計変更 (AD-6 が禁じているため実質不可能) (b) PRD §8 の上限の改訂 のいずれかであり、どちらも計画側の判断を要する。

- source_spec: `_bmad-output/implementation-artifacts/spec-rest-intervention.md`
  summary: 非活性パネルの三つの必須設定 (`nonactivating_panel` / `set_hides_on_deactivate(false)` / `full_screen_auxiliary`) が**効いていること**を確かめる自動検査が無い。組み立てたビットの字面と、設定を適用する呼び出しの存在までしか固定できていない。
  evidence: いずれも AppKit を起動した実アプリでしか観測できず、本リポジトリは UI 自動化の基盤を持たない (既存の申し送り「誤終了阻止の実挙動」と同じ性質)。欠けたときの失敗は「フルスクリーン作業中にだけ出ない」「入力中にだけ打鍵を奪う」という、最も気づきにくい形で現れる。リリース前の手動確認を運用で固定するか、UI 自動化を導入するかの判断と併せて扱う。

- source_spec: `_bmad-output/implementation-artifacts/spec-rest-intervention.md`
  summary: **休息閾値**と**猶予**を変更する面が無い。値は SQLite の `setting` 表に置かれ、コアに読み書きの口 (`Core::store_setting`) はあるが、そこへ到達する UI もコマンドも無い。
  evidence: CAP-10 は「閾値の既定値は 50 分で変更可能」と述べるが、本スライスの Intent は設定の面を含まない。変更は `sqlite3` から直接行える状態にしてある。設定の面をどこに置くか (オーバーレイの中か、別の面か) は FR-2 の「既定表示の最小化」と衝突しうる論点であり、独立した判断を要する。

- source_spec: `_bmad-output/implementation-artifacts/spec-disclosure-surface.md`
  summary: **開示面**に「このタスクを直す」への入口 (⌘E とボタン) を置いた。spec の非目標は「並べ替え・改名・削除の手がかりを一つも置かない」と定めており、改名への手がかりがこれに当たる。
  evidence: 利用者の裁定により、打ち間違えた**タスク**を直す経路 (CAP-5 の可視面) を v1 に入れた。直す相手を選ぶには一覧が要る — 既定表示からの ⌘E は**現在地**の**タスク**しか直せず、着手していない**タスク**へ届かない。**一覧そのものには依然として入力欄が無く**、修正は排他の別の面で行う (`the_disclosure_surface_has_no_editing_handles` 相当の検査は通ったままである)。手がかりを一つ置いたことが一覧を編集の面へ滑り出させるかは、1 か月の試用で判断する。

- source_spec: `_bmad-output/implementation-artifacts/spec-switch-and-resume.md`
  summary: ⌘Enter (**完了**を伴う確定) で書かれた**中断メモ**は SM-C3 の分子に入らない。
  evidence: 利用者の裁定により、**完了**の宣言は**現在地**を動かさなくなった (次に着手する**ステップ**は利用者が**開示面**で選ぶ)。`Core::complete_current_step` は離脱が起きていないため**切り替え履歴**を積まず、続く `Core::select_step` が積む 1 行は `note_written` が常に偽である (CAP-9 が「機会を与えていない移動を分子に数えない」と決めたとおり)。二つを跨いで「今回メモを書いたか」を運ぶには履歴の形を変える必要があり、指標の読み手がまだ存在しない (AD-15 は読み戻す経路を持たないと定める) ため、変更を伴う判断は送る。

- source_spec: `_bmad-output/implementation-artifacts/spec-disclosure-surface.md`
  summary: 全**ステップ**の**完了**が宣言された**タスク**を**開示面**から落とす扱いは、**腐敗** (CAP-20 / FR-17) とは別の規則として `disclosure_of` に立っている。二つの除外が同じ場所で別々の条件として並ぶことになる。
  evidence: 利用者の裁定により「完了したタスクは消す」を v1 に入れた。FR-17 の**腐敗**は「長期間**現在地**にならなかった」を契機とし、こちらは「全部終わった」を契機とする — 契機も、復帰の要否 (FR-18) も異なる。CAP-20 を設計する時点で、二つを一つの述語へ畳むか別々に保つかを決めること。終わった**タスク**へ戻る経路が v1 に無いことも併せて扱う (**現在地**を離れた時点で一覧から到達できなくなる)。
