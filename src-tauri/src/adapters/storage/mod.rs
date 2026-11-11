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

use crate::domain::idea::{Idea, IdeaId};
use crate::domain::position::CurrentPosition;
use crate::domain::setting::Setting;
use crate::domain::switch::SwitchRecord;
use crate::domain::task::{InterruptionNote, Step, StepId, Task, TaskId};
use crate::domain::work_log::{WorkSegment, WorkSegmentKind};
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
        let settings = read_settings(&connection)?;
        let ideas = read_ideas(&connection)?;
        Ok(RestoredState {
            tasks,
            current_position,
            settings,
            ideas,
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

        // **順序が意味を持つ。** 外部キーの向き (現在地 → ステップ → タスク、
        // 切り替え履歴 → ステップ) から、**タスク**が先に書かれていなければ残り二つは
        // 書けない。同じトランザクションの中でも制約は文ごとに評価される。
        if let Some(task) = &commit.task {
            write_task(&transaction, task)?;
        }
        if let Some(position) = &commit.current_position {
            write_current_position(&transaction, *position)?;
        }
        if let Some(record) = &commit.switch_record {
            write_switch_record(&transaction, record)?;
        }
        for setting in &commit.settings {
            write_setting(&transaction, setting)?;
        }
        // **思いつき**は外部キーを持たないため順序は任意である。**タスク**の作成と同じ
        // トランザクションに入ることだけが意味を持つ (昇格)。
        if let Some(idea) = &commit.idea {
            write_idea(&transaction, idea)?;
        }
        for id in &commit.deleted_ideas {
            delete_idea(&transaction, *id)?;
        }
        // **作業区間**は**タスク**と**ステップ**を指す。**タスク**の後に書く。
        for segment in &commit.work_segments {
            write_work_segment(&transaction, segment)?;
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

/// **切り替え履歴**を 1 行追記する (CAP-7 / SM-C3)。
///
/// **追記専用であり、読み戻す関数を対に置かない。** コアが起動時に読む理由が無く、
/// 読めばメモリ上に「表示されうる値」を置くことになる (AD-15)。測定が必要になった
/// 時点で SQL から直接数える。
///
/// **本文は運ばれてこない。** [`SwitchRecord`] が欄を持たないため、ここで書ける形が
/// そもそも存在しない。
fn write_switch_record(
    transaction: &Transaction<'_>,
    record: &SwitchRecord,
) -> Result<(), StorageError> {
    transaction
        .execute(
            "INSERT INTO switch_record (id, departed_step_id, occurred_at, note_written) \
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                record.id().to_string(),
                record.departed_step_id().to_string(),
                record.occurred_at().to_iso8601(),
                i64::from(record.note_written()),
            ],
        )
        .map_err(|error| StorageError::Write(format!("切り替え履歴を書けない: {error}")))?;
    Ok(())
}

/// **設定値**を 1 行書く (AD-11)。
///
/// 鍵ごとの upsert であり、**行を消さない**。設定を消すことは既定値へ戻すことであり、
/// v1 にその経路は無い — 表現できる値を置かないことでそれを構造として保証する。
fn write_setting(transaction: &Transaction<'_>, setting: &Setting) -> Result<(), StorageError> {
    transaction
        .execute(
            "INSERT INTO setting (key, value) VALUES (?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![setting.key(), setting.value()],
        )
        .map_err(|error| StorageError::Write(format!("設定値を書けない: {error}")))?;
    Ok(())
}

/// **作業区間**を 1 行追記する。
///
/// **追記専用であり、読み戻す関数を対に置かない** (AD-15: 記録するが表示しない)。
/// 読み手は `scripts/export-work-log.sh` だけである。**一言はエラーにも載せない**
/// (ログに本文を書かない)。
fn write_work_segment(
    transaction: &Transaction<'_>,
    segment: &WorkSegment,
) -> Result<(), StorageError> {
    let (kind, task_id, step_id, label) = match segment.kind() {
        WorkSegmentKind::Step { task_id, step_id } => {
            ("step", task_id, Some(step_id.to_string()), None)
        }
        WorkSegmentKind::Wait { task_id, label } => ("wait", task_id, None, label.as_deref()),
    };
    transaction
        .execute(
            "INSERT INTO work_segment (id, kind, task_id, step_id, label, started_at, ended_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                segment.id().to_string(),
                kind,
                task_id.to_string(),
                step_id,
                label,
                segment.started_at().to_iso8601(),
                segment.ended_at().to_iso8601(),
            ],
        )
        .map_err(|error| StorageError::Write(format!("作業区間を書けない: {error}")))?;
    Ok(())
}

/// **思いつき**を 1 行追加する。
///
/// **本文はエラーにも載せない** (ログに本文を書かない)。
fn write_idea(transaction: &Transaction<'_>, idea: &Idea) -> Result<(), StorageError> {
    transaction
        .execute(
            "INSERT INTO idea (id, content, created_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                idea.id().to_string(),
                idea.content(),
                idea.created_at().to_iso8601(),
            ],
        )
        .map_err(|error| StorageError::Write(format!("思いつきを書けない: {error}")))?;
    Ok(())
}

/// **思いつき**を 1 行消す。**この層で行を消す唯一の関数である。**
///
/// 既に無い ID は何もしない — 寿命で消えた後に昇格が確定しても失敗にしない。
fn delete_idea(transaction: &Transaction<'_>, id: IdeaId) -> Result<(), StorageError> {
    transaction
        .execute("DELETE FROM idea WHERE id = ?1", [id.to_string()])
        .map_err(|error| StorageError::Write(format!("思いつきを消せない: {error}")))?;
    Ok(())
}

/// 全**思いつき**を読む。並びは作成の古い順 (表示の並びは読み手が決める)。
fn read_ideas(connection: &Connection) -> Result<Vec<Idea>, StorageError> {
    let mut statement = connection
        .prepare("SELECT id, content, created_at FROM idea ORDER BY created_at, id")
        .map_err(|error| StorageError::Read(format!("思いつきを読めない: {error}")))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| StorageError::Read(format!("思いつきを読めない: {error}")))?;

    let mut ideas = Vec::new();
    for row in rows {
        let (id, content, created_at) =
            row.map_err(|error| StorageError::Read(format!("思いつきを読めない: {error}")))?;
        let id = IdeaId::parse(&id)
            .map_err(|error| StorageError::Corrupted(format!("思いつきの ID `{id}`: {error}")))?;
        let created_at = Timestamp::parse_iso8601(&created_at)
            .map_err(|error| StorageError::Corrupted(error.to_string()))?;
        ideas.push(Idea::rehydrate(id, content, created_at));
    }
    Ok(ideas)
}

/// 全**設定値**を読む (AD-11)。
///
/// **1 行も無いことは失敗ではない。** 初回起動の DB は空であり、読み手はコード内の
/// 定数へ落ちる。値の意味付けはここでは行わない — 鍵と文字列の組のまま運ぶ。
fn read_settings(connection: &Connection) -> Result<Vec<Setting>, StorageError> {
    let mut statement = connection
        .prepare("SELECT key, value FROM setting ORDER BY key")
        .map_err(|error| StorageError::Read(format!("設定値を読めない: {error}")))?;
    let rows = statement
        .query_map([], |row| {
            Ok(Setting::new(
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
            ))
        })
        .map_err(|error| StorageError::Read(format!("設定値を読めない: {error}")))?;

    let mut settings = Vec::new();
    for row in rows {
        settings
            .push(row.map_err(|error| StorageError::Read(format!("設定値を読めない: {error}")))?);
    }
    Ok(settings)
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
                switch_record: None,
                settings: Vec::new(),
                idea: None,
                deleted_ideas: Vec::new(),
                work_segments: Vec::new(),
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
            switch_record: None,
            settings: Vec::new(),
            idea: None,
            deleted_ideas: Vec::new(),
            work_segments: Vec::new(),
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

    /// **一つのコミットが三つの欄をすべて運べる** (AD-5)。
    ///
    /// **切り替え**が確定させるのは、離脱側の**中断メモ**と**完了**・**現在地**の移動・
    /// **切り替え履歴**の追記の三つである。三者が同じトランザクションに載ることを、
    /// 実際に SQLite を通して確かめる。
    #[test]
    fn one_commit_can_carry_the_task_the_position_and_the_switch_record() {
        let storage = SqliteStorage::in_memory().expect("開ける");
        let mut task = a_task();
        let departed = task.steps()[0].id();
        let destination = task.steps()[1].id();
        task.set_interruption_note(departed, Some(InterruptionNote::new("3 段落目まで")))
            .expect("メモを置ける");
        task.declare_completion(NOW, departed).expect("宣言できる");
        let position = CurrentPosition::NotStarted.move_to(task.id(), destination, NOW);

        storage
            .apply(&Commit {
                task: Some(task.clone()),
                current_position: Some(position),
                switch_record: Some(SwitchRecord::new(NOW, departed, true)),
                settings: Vec::new(),
                idea: None,
                deleted_ideas: Vec::new(),
                work_segments: Vec::new(),
            })
            .expect("三つが同じトランザクションで書ける");

        let restored = storage.restore().expect("読める");
        assert!(
            restored.tasks[0]
                .step(departed)
                .expect("ある")
                .is_completed(),
            "完了が付いている"
        );
        assert_eq!(
            restored.tasks[0]
                .step(departed)
                .expect("ある")
                .interruption_note()
                .map(InterruptionNote::text),
            Some("3 段落目まで"),
            "メモが残っている"
        );
        assert_eq!(
            restored.current_position.step_id(),
            Some(destination),
            "現在地が移動している"
        );
        assert_eq!(count_switch_records(&storage), 1, "履歴が 1 行増えている");
    }

    /// **失敗したコミットは履歴も残さない。** 三者が同時に成立するか、何も起きないか
    /// のどちらかである。
    #[test]
    fn a_failed_commit_leaves_no_switch_record_either() {
        let storage = SqliteStorage::in_memory().expect("開ける");
        let task = a_task();
        let stranger = StepId::new(NOW);

        let outcome = storage.apply(&Commit {
            task: Some(task.clone()),
            current_position: Some(CurrentPosition::rehydrate(task.id(), stranger, true, NOW)),
            switch_record: Some(SwitchRecord::new(NOW, task.steps()[0].id(), true)),
            settings: Vec::new(),
            idea: None,
            deleted_ideas: Vec::new(),
            work_segments: Vec::new(),
        });
        assert!(outcome.is_err());
        assert_eq!(count_switch_records(&storage), 0);
    }

    /// **履歴は追記され、書かれた値がそのまま残る。**
    ///
    /// 行数だけを見ていると、`note_written` を 1 に固定しても `occurred_at` を
    /// ISO 8601 以外の形で書いても検査が通る。SM-C3 はこの二つの列だけで測るため、
    /// どちらが壊れても測定基盤としては無価値になる。**列の値を見る。**
    #[test]
    fn switch_records_accumulate_with_the_values_that_were_written() {
        let storage = SqliteStorage::in_memory().expect("開ける");
        let task = a_task();
        let departed = task.steps()[0].id();
        let later = Timestamp::from_unix_millis(NOW.unix_millis() + 60_000);
        storage
            .apply(&Commit::of_task(task.clone()))
            .expect("書ける");

        for (at, note_written) in [(NOW, true), (later, false)] {
            storage
                .apply(&Commit {
                    task: None,
                    current_position: None,
                    switch_record: Some(SwitchRecord::new(at, departed, note_written)),
                    settings: Vec::new(),
                    idea: None,
                    deleted_ideas: Vec::new(),
                    work_segments: Vec::new(),
                })
                .expect("書ける");
        }

        assert_eq!(
            switch_records(&storage),
            vec![
                (
                    departed.to_string(),
                    "2026-09-10T00:26:40.000Z".to_string(),
                    1
                ),
                (
                    departed.to_string(),
                    "2026-09-10T00:27:40.000Z".to_string(),
                    0
                ),
            ],
            "離脱元・発生時刻 (UTC の ISO 8601)・記入の有無がそのまま残る"
        );
        // 書いた値そのものと照合する。上のリテラルが「読める形」であることの担保。
        assert_eq!(switch_records(&storage)[0].1, NOW.to_iso8601());
        assert_eq!(switch_records(&storage)[1].1, later.to_iso8601());
    }

    /// **履歴に本文の列が無い。** スキーマの側で AD-15 を担保していることを見る。
    #[test]
    fn the_switch_record_table_has_no_column_for_the_note_text() {
        let storage = SqliteStorage::in_memory().expect("開ける");
        let connection = storage.lock();
        let mut statement = connection
            .prepare("SELECT name FROM pragma_table_info('switch_record') ORDER BY name")
            .expect("問い合わせは組み立てられる");
        let columns: Vec<String> = statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("実行できる")
            .map(|row| row.expect("行は読める"))
            .collect();

        assert_eq!(
            columns,
            vec!["departed_step_id", "id", "note_written", "occurred_at"],
            "本文を持つ列を足さない (AD-15)"
        );
    }

    /// 受け入れ条件「メモを書いて確定した → 再起動してもメモが残り、現在地は移動後の
    /// ステップである」。および「完了を伴う切り替え → 離脱側に `completed_at` が付き、
    /// 現在地が移動し、履歴が 1 行増えている (三者が同時に成立)」。
    ///
    /// **コアから実ファイルまでを通す。** 同じファイルを閉じて開き直し、プロセスを
    /// またぐ経路を実際に歩く。
    #[test]
    fn a_switch_survives_a_restart_with_all_three_effects() {
        let directory = std::env::temp_dir().join(format!(
            "my-task-manager-switch-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let path = SqliteStorage::database_path(&directory);
        let _ = std::fs::remove_dir_all(&directory);

        let task_id;
        let departed;
        let destination;
        {
            let core = Core::restore(
                Box::new(FixedClock::at(NOW.unix_millis())),
                Box::new(SqliteStorage::open(&path).expect("初回起動で作られる")),
            )
            .expect("復元できる");
            task_id = core
                .create_task("原稿", contents(&["下書き", "推敲", "投稿"]))
                .expect("作れる");
            let snapshot = core.snapshot();
            let steps = snapshot.task(task_id).expect("ある").steps();
            departed = steps[0].id();
            destination = steps[1].id();
            core.move_current_position(departed).expect("移せる");

            core.switch_current_position(Some(InterruptionNote::new("3 段落目まで")), true)
                .expect("切り替えられる");
        }

        // ここで前の接続は落ちている。確定済みのコミットだけが残る。
        let storage = SqliteStorage::open(&path).expect("再起動で開ける");
        assert_eq!(
            switch_records(&storage),
            vec![(departed.to_string(), NOW.to_iso8601(), 1)],
            "履歴が 1 行増えており、離脱元・時刻・記入の有無が残っている"
        );
        let core = Core::restore(
            Box::new(FixedClock::at(NOW.unix_millis())),
            Box::new(storage),
        )
        .expect("復元できる");

        assert_eq!(
            core.current_position().step_id(),
            Some(destination),
            "現在地は移動後のステップである"
        );
        let snapshot = core.snapshot();
        let step = snapshot
            .task(task_id)
            .expect("ある")
            .step(departed)
            .expect("ある");
        assert_eq!(
            step.interruption_note().map(InterruptionNote::text),
            Some("3 段落目まで"),
            "メモが残っている"
        );
        assert!(step.is_completed(), "離脱側に completed_at が付いている");

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// 履歴の行数を数える。**製品コードにはこの経路を置かない** (書くだけ / AD-15)。
    fn count_switch_records(storage: &SqliteStorage) -> i64 {
        storage
            .lock()
            .query_row("SELECT count(*) FROM switch_record", [], |row| row.get(0))
            .expect("数えられる")
    }

    /// 履歴を (離脱元, 発生時刻, 記入の有無) の形で発生順に読む。
    ///
    /// **測定のときに SQL で数える形そのものである** (spec Design Notes)。製品コードには
    /// 置かない — コアが読み戻せば、表示されうる値をメモリに置くことになる (AD-15)。
    fn switch_records(storage: &SqliteStorage) -> Vec<(String, String, i64)> {
        let connection = storage.lock();
        let mut statement = connection
            .prepare(
                "SELECT departed_step_id, occurred_at, note_written \
                 FROM switch_record ORDER BY occurred_at, id",
            )
            .expect("問い合わせは組み立てられる");
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .expect("実行できる");
        rows.map(|row| row.expect("行は読める")).collect()
    }

    // --- 設定値 (AD-11) ----------------------------------------------------------

    /// 空の DB には**設定値**が 1 行も無い。**それが正常な状態である。**
    #[test]
    fn an_empty_database_has_no_settings() {
        let storage = SqliteStorage::in_memory().expect("開ける");
        assert!(storage.restore().expect("読める").settings.is_empty());
    }

    /// **設定値**は書いたとおりに読み戻る (AD-11)。
    #[test]
    fn a_setting_round_trips() {
        let storage = SqliteStorage::in_memory().expect("開ける");
        storage
            .apply(&Commit::of_settings(vec![
                Setting::new("rest_threshold_seconds", "3000"),
                Setting::new("grace_period_seconds", "900"),
            ]))
            .expect("書ける");

        let settings = storage.restore().expect("読める").settings;
        assert_eq!(
            settings,
            vec![
                Setting::new("grace_period_seconds", "900"),
                Setting::new("rest_threshold_seconds", "3000"),
            ],
            "鍵の昇順で読み戻る"
        );
    }

    /// 同じ鍵を二度書いても行は増えない (upsert であり、行を消さない)。
    #[test]
    fn writing_the_same_setting_twice_replaces_the_value() {
        let storage = SqliteStorage::in_memory().expect("開ける");
        for value in ["3000", "600"] {
            storage
                .apply(&Commit::of_settings(vec![Setting::new(
                    "rest_threshold_seconds",
                    value,
                )]))
                .expect("書ける");
        }

        let settings = storage.restore().expect("読める").settings;
        assert_eq!(settings.len(), 1);
        assert_eq!(settings[0].value(), "600");
    }

    /// **設定値だけのコミットは空ではない。** 空と判定すると書き込みが黙って落ちる。
    #[test]
    fn a_settings_only_commit_is_not_empty() {
        assert!(!Commit::of_settings(vec![Setting::new("k", "v")]).is_empty());
        assert!(Commit::of_settings(Vec::new()).is_empty());
    }

    /// **設定値を書いても、タスク・現在地・履歴には触れない。**
    #[test]
    fn writing_a_setting_touches_nothing_else() {
        let storage = SqliteStorage::in_memory().expect("開ける");
        let task = a_task();
        let step_id = task.steps()[0].id();
        let task_id = task.id();
        storage.apply(&Commit::of_task(task)).expect("書ける");
        storage
            .apply(&Commit::of_current_position(CurrentPosition::rehydrate(
                task_id, step_id, true, NOW,
            )))
            .expect("書ける");

        storage
            .apply(&Commit::of_settings(vec![Setting::new("k", "v")]))
            .expect("書ける");

        let restored = storage.restore().expect("読める");
        assert_eq!(restored.tasks.len(), 1);
        assert_eq!(restored.current_position.step_id(), Some(step_id));
        assert!(restored.current_position.is_active());
    }

    /// **思いつき**が往復し、消せる。
    #[test]
    fn an_idea_round_trips_and_can_be_deleted() {
        let storage = SqliteStorage::in_memory().expect("開ける");
        let idea = Idea::new(NOW, "請求書の様式を直す");
        storage
            .apply(&Commit::of_idea(idea.clone()))
            .expect("書ける");
        assert_eq!(storage.restore().expect("読める").ideas, vec![idea.clone()]);

        storage
            .apply(&Commit::of_deleted_ideas(vec![idea.id()]))
            .expect("消せる");
        assert!(storage.restore().expect("読める").ideas.is_empty());

        // 既に無い ID を消しても失敗しない (寿命で消えた後の昇格)。
        storage
            .apply(&Commit::of_deleted_ideas(vec![idea.id()]))
            .expect("無い ID は無視される");
    }

    /// **昇格はタスクの作成と同じトランザクションである。** タスクが書けなければ
    /// 思いつきも消えない。
    #[test]
    fn a_failed_promotion_keeps_the_idea() {
        let storage = SqliteStorage::in_memory().expect("開ける");
        let idea = Idea::new(NOW, "請求書の様式を直す");
        storage
            .apply(&Commit::of_idea(idea.clone()))
            .expect("書ける");

        // **タスクの書き込みそのものを落とす。** ステップの挿入を拒む制約を試験用に
        // 張り、昇格のコミットがタスクの部分で失敗するようにする。
        storage
            .lock()
            .execute_batch(
                "CREATE TRIGGER refuse_step BEFORE INSERT ON step \
                 BEGIN SELECT RAISE(ABORT, '試験用の失敗'); END;",
            )
            .expect("制約を張れる");
        let task = a_task();
        let outcome = storage.apply(&Commit {
            task: Some(task.clone()),
            deleted_ideas: vec![idea.id()],
            ..Commit::default()
        });
        assert!(outcome.is_err(), "タスクの書き込みで失敗する");
        let restored = storage.restore().expect("読める");
        assert!(restored.tasks.is_empty(), "タスクは生まれていない");
        assert_eq!(restored.ideas, vec![idea.clone()], "思いつきは残る");
        storage
            .lock()
            .execute_batch("DROP TRIGGER refuse_step;")
            .expect("制約を外せる");

        // 成功する昇格では、タスクが生まれ思いつきが消える。
        storage
            .apply(&Commit {
                task: Some(task.clone()),
                deleted_ideas: vec![idea.id()],
                ..Commit::default()
            })
            .expect("確定できる");
        let restored = storage.restore().expect("読める");
        assert_eq!(restored.tasks, vec![task]);
        assert!(restored.ideas.is_empty());
    }

    // --- 作業区間と書き出し --------------------------------------------------------

    /// 区間を (種類, UTC の開始, UTC の終了) から作る。
    fn segment(kind: WorkSegmentKind, started_at: &str, ended_at: &str) -> WorkSegment {
        crate::domain::work_log::OpenWorkSegment::new(
            kind,
            Timestamp::parse_iso8601(started_at).expect("時刻として読める"),
        )
        .close(Timestamp::parse_iso8601(ended_at).expect("時刻として読める"))
        .expect("長さがある")
    }

    /// **作業区間**は書いたとおりの列で残り、**タスク**の書き直しと同じコミットにも乗る。
    #[test]
    fn work_segments_are_appended_with_their_columns() {
        let storage = SqliteStorage::in_memory().expect("開ける");
        let task = a_task();
        let step = task.steps()[0].id();
        storage
            .apply(&Commit {
                task: Some(task.clone()),
                work_segments: vec![
                    segment(
                        WorkSegmentKind::Step {
                            task_id: task.id(),
                            step_id: step,
                        },
                        "2026-11-11T01:00:00.000Z",
                        "2026-11-11T01:25:00.000Z",
                    ),
                    segment(
                        WorkSegmentKind::Wait {
                            task_id: task.id(),
                            label: Some("AI の出力".to_string()),
                        },
                        "2026-11-11T01:25:00.000Z",
                        "2026-11-11T01:30:00.000Z",
                    ),
                ],
                ..Commit::default()
            })
            .expect("同じトランザクションで書ける");

        /// (kind, step_id, label, started_at, ended_at)
        type Row = (String, Option<String>, Option<String>, String, String);
        let rows: Vec<Row> = {
            let connection = storage.lock();
            let mut statement = connection
                .prepare(
                    "SELECT kind, step_id, label, started_at, ended_at \
                     FROM work_segment ORDER BY started_at",
                )
                .expect("組み立てられる");
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                })
                .expect("実行できる");
            rows.map(|row| row.expect("読める")).collect()
        };
        assert_eq!(
            rows,
            vec![
                (
                    "step".to_string(),
                    Some(step.to_string()),
                    None,
                    "2026-11-11T01:00:00.000Z".to_string(),
                    "2026-11-11T01:25:00.000Z".to_string()
                ),
                (
                    "wait".to_string(),
                    None,
                    Some("AI の出力".to_string()),
                    "2026-11-11T01:25:00.000Z".to_string(),
                    "2026-11-11T01:30:00.000Z".to_string()
                ),
            ]
        );
        // 区間だけのコミットは空ではない — 空と判定すると書き込みが黙って落ちる。
        assert!(Commit::of_work_segments(Vec::new()).is_empty());
        assert!(!Commit::of_work_segments(vec![segment(
            WorkSegmentKind::Step {
                task_id: task.id(),
                step_id: step,
            },
            "2026-11-11T02:00:00.000Z",
            "2026-11-11T02:01:00.000Z",
        )])
        .is_empty());
    }

    /// 書き出しの検査に使う一時ディレクトリ。
    fn export_directory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "my-task-manager-export-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    /// `scripts/export-work-log.sh` を走らせる。時刻帯は東京 (+09:00) に固定する。
    fn run_export(database: &Path, args: &[&str]) -> std::process::Output {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/export-work-log.sh");
        std::process::Command::new("bash")
            .arg(script)
            .args(args)
            .env("MY_TASK_MANAGER_DB", database)
            .env("TZ", "Asia/Tokyo")
            .output()
            .expect("bash を起動できる")
    }

    /// 書き出しの CSV を読む。**先頭に UTF-8 の BOM が無ければ失敗する** (Mac の Excel で
    /// 日本語が化けないため / 利用者の裁定)。残りは RFC 4180 として読み、**引用の中の
    /// カンマ・引用符・改行を正しく扱えなければ失敗する** — 書き出しが CSV として壊れて
    /// いないことの検査である。
    fn parse_csv(text: &str) -> Vec<Vec<String>> {
        let text = text
            .strip_prefix('\u{feff}')
            .unwrap_or_else(|| panic!("先頭に BOM が付いている: {text:?}"));
        let mut records = Vec::new();
        let mut record = Vec::new();
        let mut field = String::new();
        let mut quoted = false;
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if quoted {
                match c {
                    '"' if chars.peek() == Some(&'"') => {
                        chars.next();
                        field.push('"');
                    }
                    '"' => quoted = false,
                    _ => field.push(c),
                }
                continue;
            }
            match c {
                '"' => {
                    assert!(field.is_empty(), "引用は欄の先頭でだけ始まる: {text}");
                    quoted = true;
                }
                ',' => record.push(std::mem::take(&mut field)),
                '\r' => {}
                '\n' => {
                    record.push(std::mem::take(&mut field));
                    records.push(std::mem::take(&mut record));
                }
                _ => field.push(c),
            }
        }
        assert!(!quoted, "引用が閉じている: {text}");
        assert!(
            field.is_empty() && record.is_empty(),
            "最後の行も改行で終わる"
        );
        records
    }

    fn csv(rows: &[[&str; 5]]) -> Vec<Vec<String>> {
        rows.iter()
            .map(|row| row.iter().map(|cell| (*cell).to_string()).collect())
            .collect()
    }

    /// 書き出し — 日 × ステップ。日またぎ (0 時で分ける)・期間の端・待機の行・CSV の
    /// 引用・書き出した時点の番号と内容を、実際の DB とスクリプトで確かめる。
    #[test]
    fn the_export_script_writes_days_by_steps() {
        let directory = export_directory("days");
        let path = SqliteStorage::database_path(&directory);
        let storage = SqliteStorage::open(&path).expect("開ける");

        let mut draft = Task::create(
            NOW,
            "原稿, 第1版",
            contents(&["下書き \"案\"", "推敲\n二行目"]),
        )
        .expect("作れる");
        let first = draft.steps()[0].id();
        let second = draft.steps()[1].id();
        let errand = Task::create(NOW, "買い物", contents(&["米"])).expect("作れる");
        let rice = errand.steps()[0].id();
        let on = |step_id, task_id| WorkSegmentKind::Step { task_id, step_id };
        let waiting = |label: Option<&str>| WorkSegmentKind::Wait {
            task_id: draft.id(),
            label: label.map(str::to_string),
        };
        storage
            .apply(&Commit::of_task(draft.clone()))
            .expect("書ける");
        storage
            .apply(&Commit::of_task(errand.clone()))
            .expect("書ける");
        storage
            .apply(&Commit::of_work_segments(vec![
                // 期間の前日 23:50 から 0:10 — 期間に入る 10 分だけ。
                segment(
                    on(first, draft.id()),
                    "2026-10-31T14:50:00.000Z",
                    "2026-10-31T15:10:00.000Z",
                ),
                // 日またぎ 23:30〜00:45 — 前日 30 分・当日 45 分。
                segment(
                    on(first, draft.id()),
                    "2026-11-10T14:30:00.000Z",
                    "2026-11-10T15:45:00.000Z",
                ),
                // 同じ日の同じステップは合計する (45 + 25.5)。
                segment(
                    on(first, draft.id()),
                    "2026-11-11T01:00:00.000Z",
                    "2026-11-11T01:25:30.000Z",
                ),
                // 秒の精度。丸めない。
                segment(
                    on(second, draft.id()),
                    "2026-11-11T03:00:00.000Z",
                    "2026-11-11T03:00:01.234Z",
                ),
                // 待機は別の行。
                segment(
                    waiting(Some("AI の出力")),
                    "2026-11-11T02:00:00.000Z",
                    "2026-11-11T02:05:00.000Z",
                ),
                segment(
                    waiting(None),
                    "2026-11-11T02:10:00.000Z",
                    "2026-11-11T02:12:00.000Z",
                ),
                segment(
                    on(rice, errand.id()),
                    "2026-11-11T04:00:00.000Z",
                    "2026-11-11T04:03:00.000Z",
                ),
                // 期間の後 (12 月 1 日 0 時から) — 出ない。
                segment(
                    on(rice, errand.id()),
                    "2026-11-30T15:00:00.000Z",
                    "2026-11-30T16:00:00.000Z",
                ),
            ]))
            .expect("書ける");
        // 書き出した時点の番号と内容を使う — 先頭に挿入すれば番号が一つずれる。
        draft.insert_step(NOW, 1, "資料").expect("挿入できる");
        storage.apply(&Commit::of_task(draft)).expect("書ける");
        // アプリを終えた後に書き出す (WAL が畳まれ -shm も無い状態)。動いている間の
        // 書き出しは下で確かめる。
        drop(storage);

        let output = run_export(&path, &["2026-11-01", "2026-11-30"]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "成功する: {stderr}");
        let stdout = String::from_utf8(output.stdout).expect("UTF-8");
        assert_eq!(
            parse_csv(&stdout),
            csv(&[
                ["日付", "タスク", "ステップ番号", "ステップ", "分"],
                ["2026-11-01", "原稿, 第1版", "2", "下書き \"案\"", "10.00"],
                ["2026-11-10", "原稿, 第1版", "2", "下書き \"案\"", "30.00"],
                ["2026-11-11", "原稿, 第1版", "2", "下書き \"案\"", "70.50"],
                ["2026-11-11", "原稿, 第1版", "3", "推敲\n二行目", "0.02"],
                ["2026-11-11", "原稿, 第1版", "", "待機", "2.00"],
                ["2026-11-11", "原稿, 第1版", "", "待機: AI の出力", "5.00"],
                ["2026-11-11", "買い物", "1", "米", "3.00"],
            ]),
            "{stdout}"
        );

        // 一日だけ。アプリが動いている間 (接続が開いたまま) に書き出す。
        let storage = SqliteStorage::open(&path).expect("開ける");
        let output = run_export(&path, &["2026-11-10", "2026-11-10"]);
        assert!(output.status.success());
        assert_eq!(
            parse_csv(&String::from_utf8(output.stdout).expect("UTF-8")),
            csv(&[
                ["日付", "タスク", "ステップ番号", "ステップ", "分"],
                ["2026-11-10", "原稿, 第1版", "2", "下書き \"案\"", "30.00"],
            ])
        );

        drop(storage);
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// 書き出し — 引数なしは今月 1 日から今日まで。前月末の区間は出ない。開始日だけを
    /// 渡せば今日まで。区間が無い期間でも BOM と見出しは出る。
    ///
    /// 「今日」は実際の時計から東京 (+09:00) の日付として求める。1 日に走っても成り立つ
    /// (1 日 0 時半の区間と今日の区間が同じ日の別の行になる)。
    #[test]
    fn the_export_script_defaults_to_this_month() {
        const OFFSET: i64 = 9 * 60 * 60 * 1_000;
        let directory = export_directory("default");
        let path = SqliteStorage::database_path(&directory);
        let storage = SqliteStorage::open(&path).expect("開ける");

        let now_millis = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("epoch より後")
                .as_millis(),
        )
        .expect("収まる");
        let today = Timestamp::from_unix_millis(now_millis + OFFSET).to_iso8601()[..10].to_string();
        let first = format!("{}01", &today[..8]);
        // 東京の壁時計の時刻を UTC の ISO 8601 に直す。
        let local = |date: &str, time: &str, shift_millis: i64| {
            let wall = Timestamp::parse_iso8601(&format!("{date}T{time}.000Z")).expect("読める");
            Timestamp::from_unix_millis(wall.unix_millis() - OFFSET + shift_millis).to_iso8601()
        };

        let task = Task::create(NOW, "原稿", contents(&["先月", "月初", "今日"])).expect("作れる");
        let step = |index: usize| WorkSegmentKind::Step {
            task_id: task.id(),
            step_id: task.steps()[index].id(),
        };
        storage
            .apply(&Commit::of_task(task.clone()))
            .expect("書ける");
        storage
            .apply(&Commit::of_work_segments(vec![
                // 前月の末日 12:00〜12:20 — 出ない。
                segment(
                    step(0),
                    &local(&first, "00:00:00", -12 * 60 * 60 * 1_000),
                    &local(&first, "00:00:00", -(11 * 60 + 40) * 60 * 1_000),
                ),
                // 今月 1 日 00:30〜00:40。
                segment(
                    step(1),
                    &local(&first, "00:30:00", 0),
                    &local(&first, "00:40:00", 0),
                ),
                // 今日 00:00:00〜00:00:30。
                segment(
                    step(2),
                    &local(&today, "00:00:00", 0),
                    &local(&today, "00:00:30", 0),
                ),
            ]))
            .expect("書ける");
        drop(storage);

        let header = ["日付", "タスク", "ステップ番号", "ステップ", "分"];
        let this_month = csv(&[
            header,
            [&first, "原稿", "2", "月初", "10.00"],
            [&today, "原稿", "3", "今日", "0.50"],
        ]);
        for args in [&[][..], &[first.as_str()][..]] {
            let output = run_export(&path, args);
            assert!(
                output.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                parse_csv(&String::from_utf8(output.stdout).expect("UTF-8")),
                this_month,
                "{args:?} は今月 1 日から今日まで"
            );
        }

        // 開始日だけ — 今日から今日まで。
        let output = run_export(&path, &[today.as_str()]);
        assert!(output.status.success());
        assert_eq!(
            parse_csv(&String::from_utf8(output.stdout).expect("UTF-8")),
            csv(&[header, [&today, "原稿", "3", "今日", "0.50"]])
        );

        // 区間の無い期間 — BOM と見出しだけ。
        let output = run_export(&path, &["2000-01-01", "2000-01-31"]);
        assert!(output.status.success());
        assert_eq!(
            parse_csv(&String::from_utf8(output.stdout).expect("UTF-8")),
            csv(&[header])
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// 書き出し — DB が無い・日付が不正なら理由を標準エラーへ出して非 0 で終わる。
    #[test]
    fn the_export_script_refuses_bad_input() {
        let directory = export_directory("refuse");
        let path = SqliteStorage::database_path(&directory);

        let missing = run_export(&path, &["2026-11-01", "2026-11-30"]);
        assert!(!missing.status.success(), "DB が無い");
        assert!(!missing.stderr.is_empty());
        assert!(missing.stdout.is_empty(), "見出しも出さない");

        let storage = SqliteStorage::open(&path).expect("開ける");
        drop(storage);
        for args in [
            // 改行を含む引数 — 行ごとに見れば形が合うが、文字列全体では合わない。
            &["2026-11-01
2026-11-30"][..],
            &[
                "2026-11-01
'); DROP TABLE task; --",
                "2026-11-30",
            ][..],
            &["2026-02-30", "2026-03-01"][..],
            &["2026/11/01"][..],
            &["2026-11-30", "2026-11-01"][..],
            &["2026-11-01", "2026-11-30", "余り"][..],
        ] {
            let output = run_export(&path, args);
            assert!(!output.status.success(), "{args:?} は拒まれる");
            assert!(!output.stderr.is_empty(), "{args:?} の理由が出る");
            assert!(output.stdout.is_empty(), "{args:?} は何も書き出さない");
        }
        let storage = SqliteStorage::open(&path).expect("開ける");
        assert!(storage.restore().is_ok(), "引数が SQL に入っていない");
        drop(storage);

        // 作業区間の表が無い DB (版 4 より前) — 理由を出して非 0 で終わる。見出しも出さない。
        let old = directory.join("old.sqlite3");
        Connection::open(&old)
            .expect("開ける")
            .execute_batch("CREATE TABLE task (id TEXT PRIMARY KEY NOT NULL, title TEXT NOT NULL);")
            .expect("作れる");
        let output = run_export(&old, &["2026-11-01", "2026-11-30"]);
        assert!(!output.status.success(), "表が無い");
        assert!(String::from_utf8_lossy(&output.stderr).contains("作業区間の表が無い"));
        assert!(output.stdout.is_empty());

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// DB ファイルのパスはアプリデータディレクトリ配下に組み立てられる。
    #[test]
    fn the_database_lives_in_the_application_data_directory() {
        let path = SqliteStorage::database_path(Path::new("/tmp/app-data"));
        assert_eq!(path, Path::new("/tmp/app-data").join(DATABASE_FILE_NAME));
        assert!(DATABASE_FILE_NAME.ends_with(".sqlite3"));
    }
}
