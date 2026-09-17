//! ストレージアダプタ — SQLite による [`Storage`] の実装 (AD-4)。
//!
//! **SQL が存在してよい唯一の場所である。** ここと [`schema`] の外に SQL 文字列を
//! 置いてはならない。フロントエンド側は言うまでもなく、`domain/` にも `ports/` にも
//! 置かない。
//!
//! # 接続は錠の内側に閉じる (AD-5)
//!
//! `rusqlite::Connection` は `Sync` ではなく、トランザクションの開始に `&mut` を要求
//! する。[`Mutex`] の内側に閉じることで、この層に到達したすべての書き込みが一列に並ぶ。
//! コア側の錠 ([`crate::domain::state::Core`]) と二重になるが、二つを同じ順序でしか
//! 取らない (コア → ストレージ) ため、取り違えによる停止は起きない。
//!
//! # なぜ「タスクを丸ごと書き直す」のか
//!
//! [`Commit`] が運ぶのは変更後の**タスク**そのものである。追記と分割は必ず後続の連番を
//! 書き換えるため、差分を運ぶ設計では「ステップの追加」「連番の更新」…と語彙が際限なく
//! 増え、そのどれか一つが抜けた瞬間に DB とメモリが食い違う。**タスク**が持つ
//! **ステップ**は高々数個であり、丸ごと upsert する代償は無視できる。

pub mod schema;

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, Transaction};

use crate::domain::position::CurrentPosition;
use crate::domain::task::{InterruptionNote, Step, StepId, Task, TaskId};
use crate::domain::Timestamp;
use crate::ports::storage::{Commit, RestoredState, Storage, StorageError};

/// 状態を保存するファイル名。アプリデータディレクトリ配下に置く。
///
/// 自動起動の印 (`adapters/autostart`) と同じディレクトリであり、`make uninstall` が
/// ディレクトリごと消す対象でもある。
pub const DATABASE_FILE_NAME: &str = "state.sqlite3";

/// SQLite による永続化。
pub struct SqliteStorage {
    /// **この錠がこの層の直列化経路である** (AD-5)。
    connection: Mutex<Connection>,
}

impl SqliteStorage {
    /// 指定のファイルを開き、スキーマを適用する。
    ///
    /// 親ディレクトリが無ければ作る。初回起動で DB ファイルが無い場合はここで作られ、
    /// スキーマが適用された状態になる (I/O マトリクス「初回起動」)。
    ///
    /// # Errors
    ///
    /// ディレクトリを作れない、DB を開けない、スキーマを適用できないとき。
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                StorageError::Open(format!("保存先のディレクトリを作れない: {error}"))
            })?;
        }
        let connection = Connection::open(path).map_err(|error| {
            StorageError::Open(format!("{} を開けない: {error}", path.display()))
        })?;
        Self::from_connection(connection)
    }

    /// アプリデータディレクトリから DB ファイルのパスを組み立てる。
    #[must_use]
    pub fn database_path(app_data_dir: &Path) -> PathBuf {
        app_data_dir.join(DATABASE_FILE_NAME)
    }

    /// 開いた接続にこの層の前提を適用する。
    fn from_connection(mut connection: Connection) -> Result<Self, StorageError> {
        schema::configure(&connection)?;
        schema::migrate(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// テスト用のメモリ上の DB。ファイルを触らずに同じ SQL を通す。
    #[cfg(test)]
    fn in_memory() -> Result<Self, StorageError> {
        let connection =
            Connection::open_in_memory().map_err(|error| StorageError::Open(error.to_string()))?;
        Self::from_connection(connection)
    }

    /// 錠を取る。毒されていても常駐は止めない。
    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.connection
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
}

impl Storage for SqliteStorage {
    fn restore(&self) -> Result<RestoredState, StorageError> {
        let connection = self.lock();
        let tasks = read_tasks(&connection)?;
        let current_position = read_current_position(&connection)?;
        Ok(RestoredState {
            tasks,
            current_position,
        })
    }

    fn apply(&self, commit: &Commit) -> Result<(), StorageError> {
        if commit.is_empty() {
            return Ok(());
        }

        let mut connection = self.lock();
        // **1 コミット = 1 トランザクション** (AD-5)。ここで分けると、異常終了が
        // 「現在地だけ動いて完了が落ちた」状態を残しうる。
        let transaction = connection
            .transaction()
            .map_err(|error| StorageError::Write(format!("トランザクションを開けない: {error}")))?;

        if let Some(task) = &commit.task {
            write_task(&transaction, task)?;
        }
        if let Some(position) = &commit.current_position {
            write_current_position(&transaction, *position)?;
        }

        transaction
            .commit()
            .map_err(|error| StorageError::Write(format!("確定できない: {error}")))
    }
}

/// **タスク**と、それが持つ**ステップ**をすべて書く。
///
/// `ON CONFLICT ... DO UPDATE` による upsert であり、**行を消さない**。`ordinal` に
/// 一意制約を張っていないのは、連番の付け直しが一時的に重複する順序で書かれうるため
/// である (SQLite の一意制約は文ごとに評価され、トランザクション末尾まで遅延しない)。
fn write_task(transaction: &Transaction<'_>, task: &Task) -> Result<(), StorageError> {
    transaction
        .execute(
            "INSERT INTO task (id, title) VALUES (?1, ?2) \
             ON CONFLICT(id) DO UPDATE SET title = excluded.title",
            rusqlite::params![task.id().to_string(), task.title()],
        )
        .map_err(|error| StorageError::Write(format!("タスクを書けない: {error}")))?;

    for step in task.steps() {
        transaction
            .execute(
                "INSERT INTO step (id, task_id, ordinal, content, completed_at, interruption_note) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT(id) DO UPDATE SET \
                   task_id = excluded.task_id, \
                   ordinal = excluded.ordinal, \
                   content = excluded.content, \
                   completed_at = excluded.completed_at, \
                   interruption_note = excluded.interruption_note",
                rusqlite::params![
                    step.id().to_string(),
                    task.id().to_string(),
                    step.ordinal(),
                    step.content(),
                    step.completed_at().map(Timestamp::to_iso8601),
                    step.interruption_note().map(InterruptionNote::text),
                ],
            )
            .map_err(|error| StorageError::Write(format!("ステップを書けない: {error}")))?;
    }

    Ok(())
}

/// 唯一の**現在地**を書く。
fn write_current_position(
    transaction: &Transaction<'_>,
    position: CurrentPosition,
) -> Result<(), StorageError> {
    match position {
        // **未着手**は「行が無い」で表す。ここで消えるのはポインタ 1 行であって
        // **タスク**でも**ステップ**でもない (「データを削除しない」はユーザーの
        // 作業内容を指す)。なお v1 のコアに**未着手**へ戻る経路は無く、この枝は
        // 契約を全域にするために存在する。
        CurrentPosition::NotStarted => {
            transaction
                .execute("DELETE FROM current_position WHERE id = 1", [])
                .map_err(|error| StorageError::Write(format!("現在地を解けない: {error}")))?;
        }
        CurrentPosition::Active { .. } | CurrentPosition::Inactive { .. } => {
            let (Some(task_id), Some(step_id), Some(activated_at)) = (
                position.task_id(),
                position.step_id(),
                position.activated_at(),
            ) else {
                // **未着手**以外は必ず値を持つ。上の枝で分けているためここへは来ない。
                return Err(StorageError::Write(
                    "値を持たない現在地を書こうとした".to_string(),
                ));
            };
            transaction
                .execute(
                    "INSERT INTO current_position (id, task_id, step_id, is_active, activated_at) \
                     VALUES (1, ?1, ?2, ?3, ?4) \
                     ON CONFLICT(id) DO UPDATE SET \
                       task_id = excluded.task_id, \
                       step_id = excluded.step_id, \
                       is_active = excluded.is_active, \
                       activated_at = excluded.activated_at",
                    rusqlite::params![
                        task_id.to_string(),
                        step_id.to_string(),
                        i64::from(position.is_active()),
                        activated_at.to_iso8601(),
                    ],
                )
                .map_err(|error| StorageError::Write(format!("現在地を書けない: {error}")))?;
        }
    }
    Ok(())
}

/// 全**タスク**を読む。**ステップ**は `ordinal` 昇順で組み立てる。
fn read_tasks(connection: &Connection) -> Result<Vec<Task>, StorageError> {
    let mut statement = connection
        .prepare(
            "SELECT t.id, t.title, s.id, s.ordinal, s.content, s.completed_at, s.interruption_note \
             FROM task AS t \
             JOIN step AS s ON s.task_id = t.id \
             ORDER BY t.id, s.ordinal, s.id",
        )
        .map_err(|error| StorageError::Read(format!("タスクを読めない: {error}")))?;

    let mut rows = statement
        .query([])
        .map_err(|error| StorageError::Read(format!("タスクを読めない: {error}")))?;

    // (タスク ID, 題名, 収集中のステップ) を 1 件ずつ畳む。ORDER BY によりタスクは
    // まとまって現れるため、途中で別のタスクが割り込むことはない。
    let mut tasks: Vec<Task> = Vec::new();
    let mut current: Option<(TaskId, String, Vec<Step>)> = None;

    loop {
        let row = rows
            .next()
            .map_err(|error| StorageError::Read(format!("タスクを読めない: {error}")))?;
        let Some(row) = row else { break };

        let task_id = read_task_id(row.get::<_, String>(0), "タスク")?;
        let title = row
            .get::<_, String>(1)
            .map_err(|error| StorageError::Read(format!("題名を読めない: {error}")))?;
        let step = read_step(row)?;

        let continues = matches!(&current, Some((id, _, _)) if *id == task_id);
        if continues {
            if let Some((_, _, steps)) = &mut current {
                steps.push(step);
            }
        } else {
            if let Some((id, title, steps)) = current.take() {
                tasks.push(build_task(id, title, steps)?);
            }
            current = Some((task_id, title, vec![step]));
        }
    }

    if let Some((id, title, steps)) = current.take() {
        tasks.push(build_task(id, title, steps)?);
    }

    // JOIN はステップを持たないタスクを黙って落とす。落としたまま進むと、開示面から
    // 消えたタスクの理由が誰にも分からなくなる。数が合わなければ破損として返す。
    let stored: i64 = connection
        .query_row("SELECT count(*) FROM task", [], |row| row.get(0))
        .map_err(|error| StorageError::Read(format!("タスクを数えられない: {error}")))?;
    if stored != i64::try_from(tasks.len()).unwrap_or(i64::MAX) {
        return Err(StorageError::Corrupted(
            "ステップを持たないタスクが保存されている".to_string(),
        ));
    }

    Ok(tasks)
}

/// 1 行から**ステップ**を組み立てる。
fn read_step(row: &rusqlite::Row<'_>) -> Result<Step, StorageError> {
    let id = read_step_id(row.get::<_, String>(2), "ステップ")?;
    let content = row
        .get::<_, String>(4)
        .map_err(|error| StorageError::Read(format!("ステップの内容を読めない: {error}")))?;
    let completed_at = row
        .get::<_, Option<String>>(5)
        .map_err(|error| StorageError::Read(format!("完了の時刻を読めない: {error}")))?
        .map(|text| Timestamp::parse_iso8601(&text))
        .transpose()
        .map_err(|error| StorageError::Corrupted(error.to_string()))?;
    let interruption_note = row
        .get::<_, Option<String>>(6)
        .map_err(|error| StorageError::Read(format!("中断メモを読めない: {error}")))?
        .map(InterruptionNote::new);

    Ok(Step::rehydrate(
        id,
        content,
        completed_at,
        interruption_note,
    ))
}

/// 唯一の**現在地**を読む。行が無ければ**未着手**である。
fn read_current_position(connection: &Connection) -> Result<CurrentPosition, StorageError> {
    let row = connection
        .query_row(
            "SELECT task_id, step_id, is_active, activated_at FROM current_position WHERE id = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0),
                    row.get::<_, String>(1),
                    row.get::<_, i64>(2),
                    row.get::<_, String>(3),
                ))
            },
        )
        .optional()
        .map_err(|error| StorageError::Read(format!("現在地を読めない: {error}")))?;

    let Some((task_id, step_id, is_active, activated_at)) = row else {
        // I/O マトリクス「初回起動」— 何も無ければ未着手として始める。
        return Ok(CurrentPosition::NotStarted);
    };

    let task_id = read_task_id(task_id, "現在地のタスク")?;
    let step_id = read_step_id(step_id, "現在地のステップ")?;
    let activated_at = activated_at
        .map_err(|error| StorageError::Read(format!("連続作業時間の起点を読めない: {error}")))
        .and_then(|text| {
            Timestamp::parse_iso8601(&text)
                .map_err(|error| StorageError::Corrupted(error.to_string()))
        })?;
    let is_active =
        is_active.map_err(|error| StorageError::Read(format!("活性状態を読めない: {error}")))? != 0;

    Ok(CurrentPosition::rehydrate(
        task_id,
        step_id,
        is_active,
        activated_at,
    ))
}

/// **ステップ**を組み合わせて**タスク**にする。不変条件はコア側が判定する。
fn build_task(id: TaskId, title: String, steps: Vec<Step>) -> Result<Task, StorageError> {
    Task::rehydrate(id, title, steps).map_err(|error| StorageError::Corrupted(error.to_string()))
}

/// 永続化された**タスク**の ID を読む。UUID として読めない値は破損である。
fn read_task_id(value: rusqlite::Result<String>, what: &str) -> Result<TaskId, StorageError> {
    let text = read_text(value, what)?;
    TaskId::parse(&text)
        .map_err(|error| StorageError::Corrupted(format!("{what}の ID `{text}`: {error}")))
}

/// 永続化された**ステップ**の ID を読む。
fn read_step_id(value: rusqlite::Result<String>, what: &str) -> Result<StepId, StorageError> {
    let text = read_text(value, what)?;
    StepId::parse(&text)
        .map_err(|error| StorageError::Corrupted(format!("{what}の ID `{text}`: {error}")))
}

fn read_text(value: rusqlite::Result<String>, what: &str) -> Result<String, StorageError> {
    value.map_err(|error| StorageError::Read(format!("{what}の ID を読めない: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::state::Core;
    use crate::domain::FixedClock;

    const NOW: Timestamp = Timestamp::from_unix_millis(1_789_000_000_000);

    fn contents(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    fn a_task() -> Task {
        Task::create(NOW, "原稿を仕上げる", contents(&["下書き", "推敲", "投稿"])).expect("作れる")
    }

    /// I/O マトリクス「初回起動」— 空の DB は未着手として読める。
    #[test]
    fn an_empty_database_restores_as_not_started() {
        let storage = SqliteStorage::in_memory().expect("開ける");
        let restored = storage.restore().expect("読める");
        assert!(restored.tasks.is_empty());
        assert_eq!(restored.current_position, CurrentPosition::NotStarted);
    }

    /// 書いたタスクがそのまま読み戻る。完了・中断メモ・連番を含む。
    #[test]
    fn a_task_round_trips_through_sqlite() {
        let storage = SqliteStorage::in_memory().expect("開ける");
        let mut task = a_task();
        let second = task.steps()[1].id();
        task.declare_completion(NOW, task.steps()[0].id())
            .expect("宣言できる");
        task.set_interruption_note(second, Some(InterruptionNote::new("3 段落目まで")))
            .expect("メモを置ける");

        storage
            .apply(&Commit::of_task(task.clone()))
            .expect("書ける");

        let restored = storage.restore().expect("読める");
        assert_eq!(restored.tasks, vec![task]);
    }

    /// 同じタスクを二度書いても行が増えない (upsert)。
    #[test]
    fn rewriting_a_task_updates_in_place() {
        let storage = SqliteStorage::in_memory().expect("開ける");
        let mut task = a_task();
        storage
            .apply(&Commit::of_task(task.clone()))
            .expect("書ける");

        task.insert_step(NOW, 1, "資料を集める")
            .expect("挿入できる");
        storage
            .apply(&Commit::of_task(task.clone()))
            .expect("書ける");

        let restored = storage.restore().expect("読める");
        assert_eq!(restored.tasks.len(), 1);
        assert_eq!(restored.tasks[0].steps().len(), 4);
        assert_eq!(restored.tasks[0], task);
    }

    /// **連番の付け直しが一意制約で落ちない。** 前方への挿入は後続の ordinal を
    /// すべて 1 つずらすため、途中で重複した状態を通る。
    #[test]
    fn renumbering_survives_a_shifted_write() {
        let storage = SqliteStorage::in_memory().expect("開ける");
        let mut task = a_task();
        storage
            .apply(&Commit::of_task(task.clone()))
            .expect("書ける");

        for _ in 0..3 {
            task.insert_step(NOW, 1, "先頭へ").expect("挿入できる");
            storage
                .apply(&Commit::of_task(task.clone()))
                .expect("書ける");
        }

        let restored = storage.restore().expect("読める");
        assert_eq!(
            restored.tasks[0]
                .steps()
                .iter()
                .map(Step::ordinal)
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5, 6]
        );
        assert_eq!(restored.tasks[0], task);
    }

    /// 現在地が往復する。活性状態と起点を含む。
    #[test]
    fn a_current_position_round_trips_through_sqlite() {
        let storage = SqliteStorage::in_memory().expect("開ける");
        let task = a_task();
        let step_id = task.steps()[1].id();
        storage
            .apply(&Commit::of_task(task.clone()))
            .expect("書ける");

        let position = CurrentPosition::NotStarted
            .move_to(task.id(), step_id, NOW)
            .deactivate();
        storage
            .apply(&Commit::of_current_position(position))
            .expect("書ける");

        let restored = storage.restore().expect("読める");
        assert_eq!(restored.current_position, position);
        assert_eq!(restored.current_position.step_id(), Some(step_id));
        assert!(!restored.current_position.is_active());
    }

    /// 現在地は何度置き換えても 1 行のままである (FR-6)。
    #[test]
    fn moving_the_current_position_replaces_the_single_row() {
        let storage = SqliteStorage::in_memory().expect("開ける");
        let task = a_task();
        storage
            .apply(&Commit::of_task(task.clone()))
            .expect("書ける");

        for step in task.steps() {
            let position = CurrentPosition::NotStarted.move_to(task.id(), step.id(), NOW);
            storage
                .apply(&Commit::of_current_position(position))
                .expect("書ける");
        }

        let count: i64 = storage
            .lock()
            .query_row("SELECT count(*) FROM current_position", [], |row| {
                row.get(0)
            })
            .expect("数えられる");
        assert_eq!(count, 1, "現在地は同時に一つしか存在しない");
    }

    /// 複数タスクが混ざっても、ステップが取り違えられない。
    #[test]
    fn steps_stay_with_their_own_task() {
        let storage = SqliteStorage::in_memory().expect("開ける");
        let first = Task::create(NOW, "一つ目", contents(&["a", "b"])).expect("作れる");
        let second = Task::create(NOW, "二つ目", contents(&["c"])).expect("作れる");
        storage
            .apply(&Commit::of_task(first.clone()))
            .expect("書ける");
        storage
            .apply(&Commit::of_task(second.clone()))
            .expect("書ける");

        let restored = storage.restore().expect("読める");
        assert_eq!(restored.tasks.len(), 2);
        let mut expected = vec![first, second];
        expected.sort_by_key(|task| task.id().to_string());
        assert_eq!(restored.tasks, expected);
    }

    /// 受け入れ条件「現在地を設定して正常終了した → 再起動で復元される」。
    ///
    /// 同じファイルを閉じて開き直し、実際にプロセスをまたぐ経路を通す。
    #[test]
    fn a_committed_state_survives_reopening_the_file() {
        let directory = std::env::temp_dir().join(format!(
            "my-task-manager-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let path = SqliteStorage::database_path(&directory);
        let _ = std::fs::remove_dir_all(&directory);

        let step_id;
        let task_id;
        {
            let storage = SqliteStorage::open(&path).expect("初回起動で作られる");
            assert!(path.exists(), "DB ファイルが作られる");
            let core = Core::restore(
                Box::new(FixedClock::at(NOW.unix_millis())),
                Box::new(storage),
            )
            .expect("復元できる");
            task_id = core
                .create_task("原稿", contents(&["下書き", "推敲"]))
                .expect("作れる");
            step_id = core.snapshot().task(task_id).expect("ある").steps()[1].id();
            core.move_current_position(step_id).expect("移せる");
            core.declare_completion(core.snapshot().task(task_id).expect("ある").steps()[0].id())
                .expect("宣言できる");
        }

        // ここで前の接続は落ちている。異常終了と同じく、確定済みのコミットだけが残る。
        {
            let storage = SqliteStorage::open(&path).expect("再起動で開ける");
            let core = Core::restore(
                Box::new(FixedClock::at(NOW.unix_millis())),
                Box::new(storage),
            )
            .expect("復元できる");
            assert_eq!(core.current_position().step_id(), Some(step_id));
            assert_eq!(core.current_position().task_id(), Some(task_id));
            assert!(core.current_position().is_active());
            let snapshot = core.snapshot();
            let task = snapshot.task(task_id).expect("ある");
            assert!(task.steps()[0].is_completed(), "完了も復元される");
            assert_eq!(task.steps()[1].id(), step_id);
        }

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// **一つのコミットが両方の欄を運べる。** CAP-7 の**切り替え**は、離脱側の
    /// **中断メモ**の確定と**現在地**の移動を単一のトランザクションに載せる (AD-5)。
    ///
    /// 外部キーの向き (現在地 → ステップ → タスク) から、同じトランザクションの中でも
    /// **タスクが先に書かれていなければ現在地が書けない**。その順序もここで固定する。
    #[test]
    fn one_commit_can_carry_both_the_task_and_the_position() {
        let storage = SqliteStorage::in_memory().expect("開ける");
        let mut task = a_task();
        let step_id = task.steps()[1].id();
        task.set_interruption_note(step_id, Some(InterruptionNote::new("3 段落目まで")))
            .expect("メモを置ける");
        let position = CurrentPosition::NotStarted.move_to(task.id(), step_id, NOW);

        storage
            .apply(&Commit {
                task: Some(task.clone()),
                current_position: Some(position),
            })
            .expect("タスクが同じトランザクションで先に書かれるため成立する");

        let restored = storage.restore().expect("読める");
        assert_eq!(restored.tasks, vec![task.clone()], "タスクが書かれている");
        assert_eq!(restored.current_position, position, "現在地も書かれている");
        // 現在地の参照が解決する — 指し先のステップが実在し、メモも一緒に載っている。
        let pointed = restored.tasks[0]
            .step(restored.current_position.step_id().expect("指し先がある"))
            .expect("指し先のステップが実在する");
        assert_eq!(
            pointed.interruption_note().map(InterruptionNote::text),
            Some("3 段落目まで")
        );
        assert_eq!(restored.current_position.task_id(), Some(task.id()));
    }

    /// **未チェックポイントの WAL からも復元できる。**
    ///
    /// 接続を綺麗に閉じると WAL がチェックポイントされ、本体ファイルだけで読める状態に
    /// なる。異常終了 (`pkill -9`) はそれを行わない — 確定済みのコミットは WAL の中に
    /// あり、そこから読めなければ AD-5 の「最後に完了した切り替えの状態に復帰する」は
    /// 成り立たない。最初の接続を**生かしたまま**開き直して確かめる。
    #[test]
    fn a_commit_is_readable_from_an_uncheckpointed_wal() {
        let directory = std::env::temp_dir().join(format!(
            "my-task-manager-wal-recovery-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let path = SqliteStorage::database_path(&directory);
        let _ = std::fs::remove_dir_all(&directory);

        let writer = SqliteStorage::open(&path).expect("開ける");
        let task = a_task();
        let step_id = task.steps()[2].id();
        let position = CurrentPosition::NotStarted.move_to(task.id(), step_id, NOW);
        writer
            .apply(&Commit::of_task(task.clone()))
            .expect("書ける");
        writer
            .apply(&Commit::of_current_position(position))
            .expect("書ける");

        // 本体ファイルにはまだ何も反映されていない (チェックポイントは起きていない)。
        assert!(
            std::fs::metadata(path.with_extension("sqlite3-wal"))
                .map(|meta| meta.len() > 0)
                .unwrap_or(false),
            "WAL にフレームが残っている状態で読む"
        );

        // **writer を落とさずに**開き直す。異常終了後の起動と同じ状態である。
        let reader = SqliteStorage::open(&path).expect("開ける");
        let restored = reader.restore().expect("読める");
        assert_eq!(restored.tasks, vec![task]);
        assert_eq!(restored.current_position, position);
        assert_eq!(restored.current_position.step_id(), Some(step_id));

        drop(reader);
        drop(writer);
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// **ステップを持たないタスクは破損として返る。** JOIN が黙って落とすため、
    /// 数を照合していなければ、保存されているのに開示面から消えたタスクが生まれる。
    #[test]
    fn a_task_without_steps_is_reported_as_corruption() {
        let storage = SqliteStorage::in_memory().expect("開ける");
        {
            let connection = storage.lock();
            connection
                .execute(
                    "INSERT INTO task (id, title) VALUES (?1, '原稿')",
                    [TaskId::new(NOW).to_string()],
                )
                .expect("書ける");
        }

        assert!(
            matches!(storage.restore(), Err(StorageError::Corrupted(_))),
            "ステップの無いタスクを黙って読み飛ばさない"
        );
    }

    /// 壊れた ID は破損として返る。黙って読み飛ばさない。
    #[test]
    fn a_corrupted_identifier_is_reported() {
        let storage = SqliteStorage::in_memory().expect("開ける");
        {
            let connection = storage.lock();
            connection
                .execute(
                    "INSERT INTO task (id, title) VALUES ('壊れた ID', '原稿')",
                    [],
                )
                .expect("書ける");
            connection
                .execute(
                    "INSERT INTO step (id, task_id, ordinal, content) \
                     VALUES ('s', '壊れた ID', 1, '下書き')",
                    [],
                )
                .expect("書ける");
        }

        assert!(matches!(storage.restore(), Err(StorageError::Corrupted(_))));
    }

    /// 壊れた時刻も破損として返る。
    #[test]
    fn a_corrupted_instant_is_reported() {
        let storage = SqliteStorage::in_memory().expect("開ける");
        let task = a_task();
        storage
            .apply(&Commit::of_task(task.clone()))
            .expect("書ける");
        {
            let connection = storage.lock();
            connection
                .execute(
                    "UPDATE step SET completed_at = '昨日' WHERE id = ?1",
                    [task.steps()[0].id().to_string()],
                )
                .expect("書ける");
        }

        assert!(matches!(storage.restore(), Err(StorageError::Corrupted(_))));
    }

    /// 書き込みに失敗したコミットは一部だけ適用されない。
    ///
    /// **現在地**が存在しない**ステップ**を指すコミットは外部キーで落ちる。同じ
    /// トランザクションに載せたタスクも書かれていないこと。
    #[test]
    fn a_failed_commit_leaves_nothing_behind() {
        let storage = SqliteStorage::in_memory().expect("開ける");
        let task = a_task();
        let stranger = StepId::new(NOW);

        let outcome = storage.apply(&Commit {
            task: Some(task.clone()),
            current_position: Some(CurrentPosition::rehydrate(task.id(), stranger, true, NOW)),
        });
        assert!(outcome.is_err(), "存在しないステップを指す現在地は書けない");

        let restored = storage.restore().expect("読める");
        assert!(
            restored.tasks.is_empty(),
            "落ちたトランザクションはタスクも残さない"
        );
        assert_eq!(restored.current_position, CurrentPosition::NotStarted);
    }

    /// 書くものが無いコミットは何もしない。
    #[test]
    fn an_empty_commit_is_a_no_op() {
        let storage = SqliteStorage::in_memory().expect("開ける");
        storage.apply(&Commit::default()).expect("失敗しない");
        assert_eq!(
            storage.restore().expect("読める").current_position,
            CurrentPosition::NotStarted
        );
    }

    /// DB ファイルのパスはアプリデータディレクトリ配下に組み立てられる。
    #[test]
    fn the_database_lives_in_the_application_data_directory() {
        let path = SqliteStorage::database_path(Path::new("/tmp/app-data"));
        assert_eq!(path, Path::new("/tmp/app-data").join(DATABASE_FILE_NAME));
        assert!(DATABASE_FILE_NAME.ends_with(".sqlite3"));
    }
}
