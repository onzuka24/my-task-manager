---
title: 'ドメインモデルと永続化 — タスク・ステップ・現在地'
type: 'feature'
created: '2026-09-17'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: '863233ebcccf95e8cc892a0a6c79131a9d940c34'
context:
  - '{project-root}/_bmad-output/specs/spec-my-task-manager/SPEC.md'
  - '{project-root}/_bmad-output/specs/spec-my-task-manager/glossary.md'
  - '{project-root}/_bmad-output/planning-artifacts/architecture/architecture-my-task-manager-2026-09-15/ARCHITECTURE-SPINE.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** 常駐の骨格は立ったが、`domain/` と `ports/` は doc コメントだけの空の器であり、リポジトリにドメインの語彙が一つも存在しない。切り替え (CAP-7) も再開時の位置提示 (CAP-8) も、指す対象である**タスク**・**ステップ**・**現在地**が無ければ載せられない。加えて AD-5 が要求する「プロセスの異常終了をまたいで**現在地**が残る」は、永続化が無い現在まったく満たされていない。

**Approach:** CAP-4・CAP-5・CAP-6 を、ユーザーに見える面を持たない一層として立てる。`domain/` に **Task**・**Step**・**CurrentPosition** と、その不変条件を破れない形の操作を置く。`ports/` に単一のストレージ契約を置き、`adapters/storage/` に rusqlite による実装を閉じる。**現在地**を変えうる経路をコア内の一本に絞り (AD-2/AD-5)、書き込みは操作ごとに単一トランザクションで確定させる。

## Boundaries & Constraints

**Always:**
- `domain/` は `tauri::`・OS API・Web API のいずれも参照しない (AD-1)。時刻の取得はコアが所有する時計の抽象を通し、`SystemTime::now()` を `domain/` に直接書かない (AD-8)。
- 型・フィールド・コマンド名は `glossary.md` 識別子対応表に 1:1 で従う (AD-10)。`NextAction`・`Drift`・`Fixation`・`Reentry` を型にしてはならない。
- SQL は `adapters/storage/` にのみ存在してよい。`tauri-plugin-sql` と `sqlx` を使わない (AD-4)。
- **現在地**はシステム全体で同時に一つ。新たな**ステップ**が**現在地**になった時点で直前は解除される。値を持たないのは `NotStarted` のときだけで、非活性 (休息) でも値は保持される (FR-6)。
- **完了**はユーザーの明示宣言のみ。**現在地**の移動で自動的に付与も取り消しもされない (FR-4, AD-2)。
- **現在地**は `ordinal` ではなく**ステップ**の安定した ID を指す。追記・分割で連番が再計算されても同一の作業単位を指し続ける (FR-5)。
- ID は UUID v7、時刻は UTC の ISO 8601 文字列、期間は秒の整数 (スパイン「一貫性の規約」)。
- 状態を変更しうる操作はコア内の単一の直列化された経路を通す。アグリゲート単位の個別ロックを禁じる (AD-5)。
- コアは `Result` を返し panic しない。
- 依存クレートは `=` で厳密に固定する (Cargo.toml の既存方針)。
- **ステップ**は**中断メモ**の欄を 0..1 で持ち、分割時は前半に帰属する (FR-5)。本スライスが持つのはデータとしての欄と帰属規則だけであり、記録の機会も**切り替え**儀式も CAP-7 に属する。
- データを削除しない。**腐敗** (v2) は表示から外すだけでレコードを消さない。**タスク**および**ステップ**の削除操作を v1 に作らない — 消滅の経路は CAP-20 にのみ属する。

**Never:**
- UI・Svelte 側の変更を行わない。オーバーレイの表示内容は本スライスの対象外。
- **切り替え** (`Switch`) の儀式・**切り替え履歴** (`SwitchRecord`)・休息の計時そのものを実装しない (CAP-7 / CAP-10)。
- Tauri command と event を本スライスで定義しない。AD-3 の契約は、消費者であるオーバーレイが立つ CAP-7 で決める。ドメイン操作はユニットテストからのみ叩く。
- 自動起動マーカー (`adapters/autostart/mod.rs:17-18` の TODO) を設定テーブルへ移さない。設定テーブルは作るが移行は行わず、AD-11 との差分は deferred-work.md へ送る。自動テストの無い自動起動に本スライスで触れないため。
- 進捗率・消化数・達成グラフに類する集計を `domain/` に持たせない (AD-15)。
- 一括編集・並べ替え・棚卸しを促す操作を作らない (AD-15)。

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|---|---|---|---|
| タスク作成 | 題名 + 1 個以上のステップ | `Task` が ordinal 1..N で生成される | N/A |
| ステップ 0 個で作成 | 題名のみ | 拒否 | `Err(EmptyTask)` |
| 完了の宣言 | 未完了のステップ | `completed_at` が設定される。現在地は動かない | N/A |
| 完了の取り消し | 完了済みステップへの明示的な取り消し | `completed_at` が `None` に戻る | N/A |
| 途中へのステップ追記 | 現在地より前の位置へ挿入 | 後続の ordinal が再計算され、現在地は同一 ID を指し続ける | N/A |
| ステップの分割 | 現在地が指す未完了ステップ + 前半/後半の本文 | 前半は元の ID・**中断メモ**を保持、後半は新 ID。現在地は前半のまま | N/A |
| 完了済みステップの分割 | `completed_at` を持つステップ | 拒否 (完了は自動で付与も取消もされないため分割後の帰属が定義できない) | `Err(SplitCompleted)` |
| 現在地の移動 | 別ステップを指定 | 直前の現在地は解除され、同時に二つ存在しない | N/A |
| 非活性からの復帰 | 非活性の現在地を活性化 | `activated_at` が更新される (連続作業時間の起点 / AD-8) | N/A |
| 活性のまま再指定 | 活性の現在地を別ステップへ移す | `activated_at` は更新されない (切り替えではリセットしない / AD-8) | N/A |
| 異常終了後の起動 | 直前の操作がコミット済み | 最後にコミットされた現在地と完了が復元される | 破損時は `Err` を返し起動を止めない |
| 初回起動 | DB ファイルが無い | スキーマを作成し `NotStarted` で開始する | N/A |

</frozen-after-approval>

## Code Map

- `src-tauri/src/domain/mod.rs` -- doc コメントのみの空モジュール。ここに本スライスの型を積む。`tauri::`/OS API 参照の禁止が冒頭に明記されている
- `src-tauri/src/ports/mod.rs` -- 同じく空。「実装 (アダプタ) の型がここに現れてはならない」が明記されている
- `src-tauri/src/adapters/mod.rs:3-6` -- `pub mod autostart; hotkey; menubar; presentation;`。`storage` の追加先
- `src-tauri/src/lib.rs:27-30` -- `pub mod adapters/commands/domain/ports`。`lib.rs:76` の `.manage(...)` が `invoke_handler` より前に置かれている理由 (webview の早期呼び出し) を壊さないこと
- `src-tauri/src/adapters/autostart/mod.rs:141-151` -- `app.path().app_data_dir()` の唯一の既存利用。DB ファイルの置き場所はこれに揃える。identifier は `dev.onzuka.mytaskmanager`
- `src-tauri/Cargo.toml:17-19` -- 全依存を `=` 固定する方針とその理由 (Tauri 3.x alpha の誤解決回避)。`serde_json` は dev-dependencies のみ
- `_bmad-output/specs/spec-my-task-manager/glossary.md:32-69` -- 識別子対応表。型名・フィールド名の唯一の正 (AD-10)
- `ARCHITECTURE-SPINE.md` 「構造の種」ERD -- `STEP{ordinal, completed_at}` / `CURRENT_POSITION{is_active, activated_at}` / `SETTING{key,value}` のフィールドはここが出典
- `src-tauri/src/adapters/presentation/mod.rs:91-141` -- 純粋な判断関数を OS 呼び出しから切り出してテストする既存パターン。ストレージでも踏襲する
- `Makefile:53-60` -- 検証ゲート。`make test` と `make lint` がそのまま本スライスの検証手段

## Tasks & Acceptance

**Execution:**
- [x] `src-tauri/Cargo.toml` -- `rusqlite = { version = "=0.40.2", features = ["bundled"] }` と `uuid = { version = "=1.26.1", features = ["v7", "serde"] }` を追加 -- `bundled` はシステム SQLite のバージョン差を排除するため。版はスパインのスタック表に一致
- [x] `src-tauri/src/domain/mod.rs` -- サブモジュール宣言と、コアが所有する時計の抽象 (`trait Clock` 相当) を置く -- AD-1/AD-8。`SystemTime::now()` を型の内側に直接書かないための唯一の入口
- [x] `src-tauri/src/domain/task.rs` -- `Task` / `Step` (0..1 の `InterruptionNote` 欄を含む) と、作成・追記・分割・完了宣言/取消を不変条件付きで実装 -- CAP-4/CAP-5。ordinal 再計算は ID を変えない。分割では前半が元の ID とメモを保持する
- [x] `src-tauri/src/domain/position.rs` -- `CurrentPosition` (`is_active`, `activated_at`) と移動・活性/非活性遷移 -- CAP-6/AD-8。**活性のまま移動したときに `activated_at` を更新しないこと**
- [x] `src-tauri/src/domain/state.rs` -- 全**タスク**と唯一の**現在地**を保持するコア状態。状態を変えうる操作をこの一箇所へ集約する -- AD-2/AD-5。現在地の二重化を型で防ぐ
- [x] `src-tauri/src/ports/storage.rs` -- コア語彙で書いたストレージ契約。復元 1 本と、コミット単位の適用 1 本 -- AD-4/AD-5。1 メソッド = 1 トランザクション。アダプタの型 (`rusqlite::*`) を露出させない
- [x] `src-tauri/src/adapters/storage/mod.rs` -- rusqlite 実装。接続を `Mutex` の内側に閉じて直列化経路とする -- AD-5。SQL が存在してよい唯一の場所
- [x] `src-tauri/src/adapters/storage/schema.rs` -- `PRAGMA user_version` による順序付きマイグレーションと初期スキーマ (`task`/`step`/`setting`。メモは `step` の nullable 列) -- スパインが移行方式を決めていないため本スライスで確定する。`journal_mode=WAL`・`synchronous=NORMAL`・`foreign_keys=ON`
- [x] `src-tauri/src/adapters/mod.rs` -- `pub mod storage;` を追加 -- 既存 4 モジュールと同じ並び
- [x] `src-tauri/src/lib.rs` -- 起動時に app data dir 配下の DB を開き、スキーマを適用し、復元した状態を `manage` する -- AD-5 の「最後にコミットされた状態に復帰する」を実アプリで成立させる。**`setup` 内で `?` を使わない** (slice 1 の既存方針)

**Acceptance Criteria:**
- Given アプリを初回起動した、when app data dir を見る、then `state.sqlite3` が存在し `PRAGMA user_version` が 1 以上である
- Given **現在地**を設定して正常終了した、when 再起動する、then 同じ**ステップ**を指す**現在地**が復元されている
- Given **現在地**を設定した直後にプロセスを強制終了した、when 再起動する、then 最後にコミットされた**現在地**が復元されている
- Given 何も登録していない、when 起動する、then `NotStarted` として扱われ、DB が壊れていなければ起動は失敗しない
- Given 実装完了、when `make lint` と `make test` を実行する、then いずれも成功する
- Given 実装完了、when `domain/` 配下を検索する、then `tauri::`・`rusqlite::`・`std::time::SystemTime::now` のいずれも現れない
- Given 実装完了、when `glossary.md` 識別子対応表と型名を突き合わせる、then 対応表にある語は表のとおりの識別子で現れ、`NextAction`/`Drift`/`Fixation`/`Reentry` の型は存在しない

## Implementation Notes

### 何がどこに立ったか

```text
src-tauri/
  Cargo.toml                    # rusqlite =0.40.2 (bundled) / uuid =1.26.1 (v7, serde)
  src/
    domain/
      mod.rs                    # Timestamp (UTC/ISO 8601) + trait Clock + DomainError
      task.rs                   # Task / Step / InterruptionNote / TaskId / StepId
      position.rs               # CurrentPosition (NotStarted | Active | Inactive)
      state.rs                  # CoreState と Core — 状態を変えうる唯一の直列化経路
    ports/
      mod.rs  storage.rs        # Storage (restore 1 本 / apply 1 本) + Commit + StorageError
    adapters/
      clock/mod.rs              # SystemClock — 壁時計を読む唯一の場所
      storage/mod.rs            # rusqlite 実装。接続を Mutex に閉じる
      storage/schema.rs         # PRAGMA user_version による順序付きマイグレーション
    lib.rs                      # setup で DB を開き、復元し、Core を manage する
```

Rust のテストは 30 件から 118 件になった。

### スパイン・spec に無い判断を要したところ

1. **`current_position` 表を足した (表は 4 つになった)。** Execution の項目は初期スキーマを
   「`task`/`step`/`setting`」と書いているが、それだけでは受け入れ条件「再起動で**現在地**が
   復元される」を満たせない。スパインの ERD は `CURRENT_POSITION` を独立した実体として
   持つため、`setting` に押し込めて AD-2 の所有者表 (現在地と設定値は別の行) を崩すより、
   ERD どおりの表を足すほうを選んだ。`id INTEGER PRIMARY KEY CHECK (id = 1)` により、
   **二つ目の現在地が SQL のレベルで書けない** — FR-6 をコアの規律だけに頼らない。
   Verification の `.schema` は「`task`/`step`/`setting` が存在する」を要求しており、
   この追加はそれと両立する。

2. **時計アダプタ (`adapters/clock/`) を足した。** Execution は「時計の抽象を `domain/`
   に置く」としか言っていないが、抽象の実装が壁時計を読む以上、それは `domain/` に置けない
   (AD-1 / AD-8、および受け入れ条件の grep)。抽象はコア、読み取りはアダプタに分けた。
   `adapters/mod.rs` の宣言は `storage` と併せて 2 行増えている。

3. **`Commit` は「タスクを丸ごと」運ぶ。** 差分 (「このステップだけ」) を運ぶ設計にすると、
   追記と分割が必ず後続の `ordinal` を書き換えるため、差分の語彙が際限なく増え、どれか一つが
   抜けた瞬間に DB とメモリが食い違う。**ステップ**は高々数個であり、upsert し直す代償は
   無視できる。`ordinal` に一意制約を張っていないのは、連番の付け直しが一時的に重複した
   順序で書かれうるためである (SQLite の一意制約は文ごとに評価され、トランザクション末尾
   まで遅延しない)。

4. **`Commit` に「消す」を表現する値を置かなかった。** v1 は**タスク**も**ステップ**も削除
   しない (消滅の経路は CAP-20 にのみ属する)。唯一の `DELETE` は**現在地**が
   `NotStarted` に戻る枝にあるが、これはポインタ 1 行であって作業内容ではなく、かつ v1 の
   コアにこの枝へ至る経路は無い (契約を全域にするためだけに存在する)。

5. **活性 / 非活性は真偽値ではなく変種にした。** 用語集の対応表は 活性 / 非活性 に
   `Active` / `Inactive` を割り当てており、識別子は表に 1:1 で従う (AD-10)。`is_active: bool`
   のままでは、その二語がコードのどこにも現れない。`CurrentPosition` を
   `NotStarted | Active | Inactive` とした。永続化側の列はスパインの ERD どおり
   `is_active` (0/1) のままである。

6. **題名と本文の識別子。** `glossary.md` の対応表は「タスク」「ステップ」を縛るが、その
   属性名 (題名 / 内容) は縛っていない。新たなドメイン語を導入しない範囲で `Task::title` /
   `Step::content` とした。用語集への追加は行っていない。

7. **`domain/` の doc コメントから検索語を外した。** 受け入れ条件の grep
   (`tauri::|rusqlite::|SystemTime::now`) は、禁止を説明する doc コメント自身に引っかかると
   無効になる。言い換えて一致しないようにし、その意図を `domain/mod.rs` に明記した。

### 自動テストで覆った行と、実プロセスで確かめた行

- I/O マトリクスの 12 行すべてに対応するユニットテストがある (`domain::task` /
  `domain::position` / `domain::state` / `adapters::storage`)。「異常終了後の起動」は、
  同じファイルを閉じて開き直す
  `adapters::storage::tests::a_committed_state_survives_reopening_the_file` が覆う。
- AD-5 の直列化は `domain::state::tests::concurrent_operations_leave_exactly_one_current_position`
  が 4 スレッド × 50 回の移動で確かめる (最終状態が一つであること + コミット数の一致)。
- 実プロセスでは `make install && make open` の後に
  `~/Library/Application Support/dev.onzuka.mytaskmanager/state.sqlite3` が作られ、
  `PRAGMA user_version = 1` / `journal_mode = wal` / 4 表が揃うことを確認した。さらに
  `pkill -9` の後に、確定済みの**現在地**の行を持つ DB から起動し、破損を報告せずに
  復元されることを確認した (ログの `the core state was restored from disk`)。

### 本スライスで意図的にやっていないこと

- **Tauri command / event を定義していない** (spec の Never)。したがって実アプリから
  **タスク**を作る経路はまだ無く、受け入れ条件の「**現在地**を設定して再起動」は
  ユニットテストと、上記の手動での行挿入でのみ観測できる。AD-3 の契約は CAP-7 で決める。
- **DB を開けなかった場合、`Core` は `manage` されない。** 既定値で埋めたコアを預けると、
  **現在地**が失われた事実が「未着手」として静かに上書きされ、次の書き込みで確定して
  しまう。消費者 (CAP-7) は `try_state` で不在を扱うこと。
- `setting` 表は作ったが値を入れていない。自動起動の印の移行は deferred-work.md にある。

## Spec Change Log

## Review Triage Log

第 1 回レビュー (blind-hunter / edge-case-hunter / verification-gap)。intent_gap・bad_spec は無く、ループバックは発生していない。

| # | 出所 | 所見 | 判定 | 根拠 | 経路 |
|---|---|---|---|---|---|
| 1 | BH / EC | `migrate` は `user_version > LATEST` しか弾かず、負の版番号では `pending` が空を返して表が一つも作られない | medium | `schema.rs:125` に下限の検査が無いことを確認。`pending` は `0..LATEST` 外で `&[]` を返すため `migrate` は `Ok` を返し、失敗は後の `read_tasks` で `no such table` として現れる | patch |
| 2 | BH / EC | `Timestamp` は書けるが読み戻せない値を作れる (年が 0000..9999 の外) | medium | `to_iso8601` の `{year:04}` は 5 桁や符号付きを出し、`parse_iso8601` の `parse_int(_, 4)` がそれを拒む。epoch 前の時計で到達しうる。一度書かれると以後の `restore` が恒久的に `Corrupted` を返す | patch |
| 3 | VG | 「永続化に成功したときだけメモリへ反映する」が 8 経路中 1 経路しか検証されていない | medium | `set_failing` の呼び出しは `state.rs:509` の 1 箇所のみで、`create_task` しか通らない。`commit_task` と `move_current_position` の順序を反転させても 105 件全て通ることを変異試験で確認済み | patch |
| 4 | VG | ステップを持たないタスクを破損として報告する検査に、それを起こすテストが無い | medium | `read_tasks` の件数照合ブロックを丸ごと削除しても 105 件全て通ることを確認済み。タスク行のみを挿入するテストは存在しない | patch |
| 5 | VG / BH | WAL と `synchronous` を観測するテストが無く、`pragma_update_and_check` は戻り値を捨てているため実際に WAL に入ったかを確かめていない | medium | 全テストが `in_memory()` を通り、そこでは WAL は無言の no-op。両 PRAGMA 文を削除しても 105 件全て通る。唯一のファイル実体テストは接続を正常に閉じるためチェックポイント済みの経路しか通らず、AD-5 が依拠する異常終了後の復帰を再現していない | patch |
| 6 | BH | `Commit` が `task` と `current_position` を同時に運ぶ正常系が一度も実行されていない | medium | `of_task` / `of_current_position` は片方しか埋めず、両方を埋めるのは失敗系の 1 テストのみ。AD-5 の「切り替えを単一トランザクションで書く」— CAP-7 が依拠する保証 — の正常系と、トランザクション内の外部キー順序が未検証 | patch |
| 7 | EC | 用語集が `活性 / 非活性` に与えた識別子 `Active` / `Inactive` がコードに存在しない | medium | 用語集 42 行目の対応表に対し、`rg '\bActive\b|\bInactive\b' src-tauri/src/` は 0 件。実装は `is_active: bool`。本 spec の Always は「識別子対応表に 1:1 で従う (AD-10)」を要求しており、これは仕様からの直接の逸脱である | patch |
| 8 | BH / EC | `move_current_position` に `transition_position` が持つ無変更時の短絡が無い | low | 同一ステップを指し直すだけで毎回トランザクションが走る。データは壊れないが、無変更の要求が I/O の失敗で `Err` になりうる。修正は 1 行の直接的な追加 | patch |
| 9 | VG | `new_v7` は秒を `.max(0)` で潰しながらナノ秒を `rem_euclid` で求めるため、負の時刻で両者が食い違う | low | v7 の時刻ビットを読む経路は現時点で存在せず (順序は `ordinal` が負う) 実害は無いが、修正は直接的な訂正である | patch |
| 10 | BH / EC | 復元の失敗が利用者にまったく伝わらず、破損した DB を隔離・再作成する経路も無い | medium | `restore_core` は `log::error!` のみで、ホットキー失敗時のようなオーバーレイもメニューバー表示も持たない。ただし本スライスの凍結された Intent は「ユーザーに見える面を持たない一層」であり、可視面の追加は intent 自身が排除している。可視面が立つ CAP-7 で扱う | defer |
| 11 | VG | `lib.rs` の永続化の配線 (`restore_core`) に自動検証が無い | medium | `restore_core` の呼び出しを no-op に置き換えても 105 件全て通ることを確認済み。ただし検証には live な `AppHandle` が要り、spec の Verification は当該行を手動確認として明示的に定めている。実装時点で deferred-work.md に記録済みのため重複記載はしない | defer |
| 12 | BH | `restore_core` が `setup` の末尾にあり、CAP-1 の 300ms 予算を脅かす | false | Core を消費するコマンドは本スライスに存在せず (Never が禁じている)、既存の 3 コマンドは Core に触れない。300ms はホットキー押下から入力受付までの制約であってプロセス起動の制約ではなく、ウィンドウは起動時に生成済みである | 棄却 |
| 13 | EC | `restore` がタスクと現在地を別々のスナップショットで読むため、並行する書き手とずれうる | false | `restore` は `self.lock()` で接続の錠を取得したまま両方を読み、全ての書き込みも同じ錠を通る。割り込む書き手が存在しえない | 棄却 |
| 14 | BH | `Deserialize` がどの型にも実装されていないため CAP-7 のコマンドが ID を受け取れない | false | 不都合は当該箇所では起きない。CAP-7 がコマンドを足す際に derive を 1 行加えるだけであり、本スライスはコマンドを持たない (Never) | 棄却 |
| 15 | BH | コアの錠を disk I/O をまたいで保持しており、読み手が書き込みの間ブロックする | low | 事実だが、判断→永続化→メモリ反映を不可分にしているのは AD-5 の要求そのものである。錠を手放す修正は AD-5 が禁じる交錯を招きうる複雑化であり、日常の利用で観測される見込みも低い | 棄却 |
| 16 | BH / EC | 空文字・空白のみの題名・内容・中断メモを受け付ける | low | 本スライスにコマンドが無く、利用者から到達する経路が存在しない。入力の検証は入力境界の責務であり、メモの有無 (`Some("")` と `None` の区別) の意味付けは spec が明示的に CAP-7 に委ねている。修正は新たなエラー種別という公開面の追加を伴う | 棄却 |
| 17 | BH | `write_task` はステップ行を消せないため、将来ステップが減ったときに古い行が残る | low | 本スライスに減る操作は存在せず (`Commit` に削除を表す値が無い)、到達経路が無い。投機的な照合削除は、削除経路を持たないという設計判断そのものに反する | 棄却 |
| 18 | BH | 壊れた `ordinal` が `renumber()` により無言で書き換えられ、ID や時刻と扱いが不揃いである | low | 不揃いであることは事実だが、`Task::rehydrate` の doc が「DB 側の連番に欠番があっても復元後は 1..N に整う」と意図として明記した振る舞いである。自己修復は起動拒否より安全であり、意図どおり動いている | 棄却 |
| 19 | BH | spec の Execution・Verification が実際に出荷された 4 表目と時計アダプタを反映していない | — | 修正が本ビルドの spec 自身の編集に当たるため、規定により棄却する。逸脱は Implementation Notes に記録済みである | 棄却 |


## Design Notes

**なぜコア状態をメモリに持ち、操作ごとに単一トランザクションで書くのか。** AD-5 は「状態を変更しうる操作はコア内で単一の直列化された経路を通す」「アグリゲート単位の個別ロックを禁じる」と定める。DB を読みながら判断する形にすると、判断と書き込みの間に隙ができ、休息への遷移と現在地の移動が交錯しうる。コア状態を単一の所有者に置き、その内側で判断→永続化→メモリ反映を一続きに行えば、隙は構造的に生じない。CAP-7 の**切り替え**はこの経路にメモと**切り替え履歴**を足すだけで済む。

**なぜ `ordinal` と ID を分けるのか。** FR-5 は「連番が再計算されても**現在地**は同一の作業単位を指し続ける」ことを求める。**現在地**が ordinal を指していれば、前方への挿入がそのまま現在地のずれになる。UUID v7 は生成順に単調増加するが**ステップ**順序とは独立であり (スパイン「一貫性の規約」)、順序は `ordinal` が単独で負う。

## Verification

**Commands:**
- `make test` -- expected: 既存 30 件の Rust テストと新規テストがすべて成功し、`pnpm check` / `pnpm test` も成功する
- `make lint` -- expected: `cargo fmt --check` と `cargo clippy --all-targets -- -D warnings` がいずれも警告なしで成功する
- `rg -n 'tauri::|rusqlite::|SystemTime::now' src-tauri/src/domain/` -- expected: 一致なし (AD-1 / AD-8)
- `sqlite3 "$HOME/Library/Application Support/dev.onzuka.mytaskmanager/state.sqlite3" '.schema'` -- expected: `task` / `step` / `setting` が存在する

**Manual checks (if no CLI):**
- `make install && make open` の後、**現在地**を設定した状態で `pkill -x my-task-manager` し、再度起動して**現在地**が復元されることを確認する (異常終了をまたぐ保持は実プロセスでしか観測できない)
