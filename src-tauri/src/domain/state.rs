//! コア状態 — 全**タスク**と唯一の**現在地** (AD-2 / AD-5)。
//!
//! # なぜ状態をメモリに持ち、操作ごとに単一トランザクションで書くのか
//!
//! AD-5 は「状態を変更しうる操作はコア内で単一の直列化された経路を通す」「アグリゲート
//! 単位の個別ロックを禁じる」と定める。DB を読みながら判断する形にすると、判断と書き込み
//! の間に隙ができ、**休息**への遷移と**現在地**の移動が交錯しうる。コア状態を単一の
//! 所有者 ([`Core`]) に置き、その内側で **判断 → 永続化 → メモリ反映** を一続きに行えば、
//! 隙は構造的に生じない。
//!
//! # 一意性は型が負う
//!
//! [`CoreState`] は**現在地**を 1 個のフィールドとして持つ。集合として持たないため、
//! 「二つの現在地」を表現する値が存在しない (FR-6)。

use std::sync::{Mutex, MutexGuard};

use super::position::CurrentPosition;
use super::task::{InterruptionNote, Step, StepId, Task, TaskId};
use super::{Clock, DomainError};
use crate::ports::storage::{Commit, RestoredState, Storage, StorageError};

/// コアが保持する状態そのもの。
///
/// フィールドは外へ出さない。状態を変えうる操作は [`Core`] のメソッドだけであり、
/// この型を握って好きに書き換える経路を作らない (AD-2)。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CoreState {
    tasks: Vec<Task>,
    /// **唯一の現在地。** 集合ではないことが FR-6 の一意性そのものである。
    current_position: CurrentPosition,
}

impl CoreState {
    /// 全**タスク**。
    #[must_use]
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    /// 唯一の**現在地**。
    #[must_use]
    pub const fn current_position(&self) -> CurrentPosition {
        self.current_position
    }

    /// ID から**タスク**を引く。
    #[must_use]
    pub fn task(&self, id: TaskId) -> Option<&Task> {
        self.tasks.iter().find(|task| task.id() == id)
    }

    /// **ステップ**を含む**タスク**を引く。
    ///
    /// **ステップ**の ID は UUID であり全体で一意であるため、どの**タスク**に属するかを
    /// 呼び出し側に言わせる必要が無い。
    #[must_use]
    pub fn task_of_step(&self, step_id: StepId) -> Option<&Task> {
        self.tasks.iter().find(|task| task.step(step_id).is_some())
    }

    /// **現在地**が指す**ステップ**。**未着手**なら `None`。
    ///
    /// 用語集の**次の一手** (`NextAction`) が指すものだが、独立した型は持たせない
    /// (AD-10)。表示上の呼称であって実体ではない。
    #[must_use]
    pub fn step_at_current_position(&self) -> Option<&Step> {
        let step_id = self.current_position.step_id()?;
        self.task_of_step(step_id)?.step(step_id)
    }
}

/// 状態を変更しうる操作の失敗。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    /// ドメインの不変条件に反する要求。状態は変わっていない。
    Domain(DomainError),
    /// 永続化に失敗した。**メモリ上の状態も変えていない。**
    ///
    /// 書き込めなかった変更をメモリにだけ反映すると、次の起動で黙って消える。
    /// 「消えた」と気づけないほうが、失敗を返すより悪い。
    Storage(StorageError),
}

impl From<DomainError> for CoreError {
    fn from(error: DomainError) -> Self {
        Self::Domain(error)
    }
}

impl From<StorageError> for CoreError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Domain(error) => write!(f, "{error}"),
            Self::Storage(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for CoreError {}

/// **状態を変えうる唯一の経路** (AD-5)。
///
/// ホットキーのコールバックはメインスレッド外から、Tauri command は IPC のスレッドから
/// 到達する。すべての変更をこの型のメソッドに通し、内側で一つの錠を取ることで、二つの
/// 状態変更が同時に進まない。**アグリゲートごとの錠を作らない** — **休息**への遷移と
/// **切り替え**のコミットが交錯し、「**現在地**が活性のまま休息中」という状態を作りうる
/// ためである。
pub struct Core {
    state: Mutex<CoreState>,
    clock: Box<dyn Clock>,
    storage: Box<dyn Storage>,
}

impl Core {
    /// 永続化された状態を読み戻してコアを組み立てる。
    ///
    /// # Errors
    ///
    /// 読み出しに失敗した、または読めた値がコアの不変条件を満たさないとき。
    /// 呼び出し側 (`lib.rs`) はこれを記録して常駐を続ける — 起動を止めない
    /// (I/O マトリクス「異常終了後の起動」)。
    pub fn restore(clock: Box<dyn Clock>, storage: Box<dyn Storage>) -> Result<Self, StorageError> {
        let RestoredState {
            tasks,
            current_position,
        } = storage.restore()?;

        // **現在地**が実在しない**ステップ**を指していないことを、ここで一度だけ確かめる。
        // 通ってしまうと「指し先が消えている現在地」が以降ずっと残り、CAP-8 の位置提示が
        // 黙って何も出さなくなる。
        if let Some(step_id) = current_position.step_id() {
            let found = tasks.iter().find(|task| task.step(step_id).is_some());
            match found {
                Some(task) if Some(task.id()) == current_position.task_id() => {}
                Some(_) => {
                    return Err(StorageError::Corrupted(
                        "現在地のタスクとステップの対応が食い違っている".to_string(),
                    ))
                }
                None => {
                    return Err(StorageError::Corrupted(
                        "現在地が存在しないステップを指している".to_string(),
                    ))
                }
            }
        }

        Ok(Self {
            state: Mutex::new(CoreState {
                tasks,
                current_position,
            }),
            clock,
            storage,
        })
    }

    /// 状態の複製を返す。
    ///
    /// 参照ではなく複製を返すのは、読み手が錠を握ったまま任意のコードを走らせることを
    /// 不可能にするためである。**タスク**は数十件の規模であり、複製の代償は無視できる。
    #[must_use]
    pub fn snapshot(&self) -> CoreState {
        self.lock().clone()
    }

    /// 唯一の**現在地**。
    #[must_use]
    pub fn current_position(&self) -> CurrentPosition {
        self.lock().current_position
    }

    /// **タスク**を作る (CAP-4)。
    ///
    /// # Errors
    ///
    /// **ステップ**が 1 個も無いとき、または永続化に失敗したとき。
    pub fn create_task(
        &self,
        title: impl Into<String>,
        step_contents: Vec<String>,
    ) -> Result<TaskId, CoreError> {
        let mut state = self.lock();
        let task = Task::create(self.clock.now(), title, step_contents)?;
        let id = task.id();
        self.storage.apply(&Commit::of_task(task.clone()))?;
        state.tasks.push(task);
        Ok(id)
    }

    /// **ステップ**を指定の位置へ追記する (CAP-5)。
    ///
    /// **現在地**は動かない。**現在地**より前へ挿入しても、指しているのが ID である
    /// 以上ずれようがない (FR-5)。
    ///
    /// # Errors
    ///
    /// **タスク**が無いとき、位置が範囲外のとき、または永続化に失敗したとき。
    pub fn insert_step(
        &self,
        task_id: TaskId,
        ordinal: u32,
        content: impl Into<String>,
    ) -> Result<StepId, CoreError> {
        self.mutate_task(task_id, |now, task| task.insert_step(now, ordinal, content))
    }

    /// **ステップ**を末尾へ追記する (CAP-5)。
    ///
    /// # Errors
    ///
    /// **タスク**が無いとき、または永続化に失敗したとき。
    pub fn append_step(
        &self,
        task_id: TaskId,
        content: impl Into<String>,
    ) -> Result<StepId, CoreError> {
        self.mutate_task(task_id, |now, task| task.append_step(now, content))
    }

    /// **ステップ**を二つに分割する (CAP-5)。前半が元の ID と**中断メモ**を保つ。
    ///
    /// **現在地**は動かさない。前半が元の ID を保つため、**現在地**は分割後も前半を
    /// 指したままである (FR-5)。
    ///
    /// # Errors
    ///
    /// **ステップ**が無いとき、**完了**済みのとき、または永続化に失敗したとき。
    pub fn split_step(
        &self,
        step_id: StepId,
        first_content: impl Into<String>,
        second_content: impl Into<String>,
    ) -> Result<StepId, CoreError> {
        self.mutate_task_of_step(step_id, |now, task| {
            task.split_step(now, step_id, first_content, second_content)
        })
    }

    /// **完了**を宣言する (CAP-4)。**現在地**は動かない (FR-4 / AD-2)。
    ///
    /// # Errors
    ///
    /// **ステップ**が無いとき、または永続化に失敗したとき。
    pub fn declare_completion(&self, step_id: StepId) -> Result<(), CoreError> {
        self.mutate_task_of_step(step_id, |now, task| task.declare_completion(now, step_id))
    }

    /// **完了**の宣言を取り消す (CAP-4)。**現在地**は動かない (FR-4 / AD-2)。
    ///
    /// # Errors
    ///
    /// **ステップ**が無いとき、または永続化に失敗したとき。
    pub fn revoke_completion(&self, step_id: StepId) -> Result<(), CoreError> {
        self.mutate_task_of_step(step_id, |_, task| task.revoke_completion(step_id))
    }

    /// **中断メモ**の欄を設定する。
    ///
    /// 記録の機会と**切り替え**の儀式は CAP-7 に属する。ここにあるのは欄への書き込み
    /// だけである。
    ///
    /// # Errors
    ///
    /// **ステップ**が無いとき、または永続化に失敗したとき。
    pub fn set_interruption_note(
        &self,
        step_id: StepId,
        note: Option<InterruptionNote>,
    ) -> Result<(), CoreError> {
        self.mutate_task_of_step(step_id, |_, task| task.set_interruption_note(step_id, note))
    }

    /// **現在地**を指定の**ステップ**へ移す (CAP-6)。
    ///
    /// **完了**には一切触れない。**完了**は**現在地**の移動で自動的に付与も取消もされない
    /// (FR-4 / AD-2)。
    ///
    /// # Errors
    ///
    /// **ステップ**が無いとき、または永続化に失敗したとき。
    pub fn move_current_position(&self, step_id: StepId) -> Result<(), CoreError> {
        let mut state = self.lock();
        let task_id = state
            .task_of_step(step_id)
            .ok_or(DomainError::UnknownStep)?
            .id();
        let moved = state
            .current_position
            .move_to(task_id, step_id, self.clock.now());
        if moved == state.current_position {
            // 既に指しているステップを選び直しただけ。書くものが無いのにトランザクション
            // を張ると、何も変えない要求が永続化の失敗で `Err` になりうる。
            return Ok(());
        }
        self.storage.apply(&Commit::of_current_position(moved))?;
        state.current_position = moved;
        Ok(())
    }

    /// **現在地**を**活性**にする (**休息**からの復帰など / CAP-6)。
    ///
    /// # Errors
    ///
    /// 永続化に失敗したとき。
    pub fn activate_current_position(&self) -> Result<(), CoreError> {
        self.transition_position(|position, now| position.activate(now))
    }

    /// **現在地**を**非活性**にする (**休息**へ入るときなど / CAP-6)。値は保持される。
    ///
    /// # Errors
    ///
    /// 永続化に失敗したとき。
    pub fn deactivate_current_position(&self) -> Result<(), CoreError> {
        self.transition_position(|position, _| position.deactivate())
    }

    /// **現在地**の遷移を一つの経路に集める。
    fn transition_position(
        &self,
        transition: impl FnOnce(CurrentPosition, super::Timestamp) -> CurrentPosition,
    ) -> Result<(), CoreError> {
        let mut state = self.lock();
        let next = transition(state.current_position, self.clock.now());
        if next == state.current_position {
            // 書くものが無い。無変更のコミットを投げると、失敗しうる I/O が増えるだけで
            // 得るものが無い。
            return Ok(());
        }
        self.storage.apply(&Commit::of_current_position(next))?;
        state.current_position = next;
        Ok(())
    }

    /// ID で指した**タスク**を書き換える。
    fn mutate_task<T>(
        &self,
        task_id: TaskId,
        change: impl FnOnce(super::Timestamp, &mut Task) -> Result<T, DomainError>,
    ) -> Result<T, CoreError> {
        let mut state = self.lock();
        let index = state
            .tasks
            .iter()
            .position(|task| task.id() == task_id)
            .ok_or(DomainError::UnknownTask)?;
        Self::commit_task(
            self.clock.as_ref(),
            self.storage.as_ref(),
            &mut state,
            index,
            change,
        )
    }

    /// **ステップ**を含む**タスク**を書き換える。
    ///
    /// 「属する**タスク**を引く」と「書き換える」を別の錠に分けない。分ければその隙間で
    /// 別の変更が割り込み、AD-5 が禁じる交錯そのものになる。
    fn mutate_task_of_step<T>(
        &self,
        step_id: StepId,
        change: impl FnOnce(super::Timestamp, &mut Task) -> Result<T, DomainError>,
    ) -> Result<T, CoreError> {
        let mut state = self.lock();
        let index = state
            .tasks
            .iter()
            .position(|task| task.step(step_id).is_some())
            .ok_or(DomainError::UnknownStep)?;
        Self::commit_task(
            self.clock.as_ref(),
            self.storage.as_ref(),
            &mut state,
            index,
            change,
        )
    }

    /// 判断 → 永続化 → メモリ反映 を一続きに行う (AD-5)。
    ///
    /// 判断は複製の上で行い、**永続化が成功したときにだけ**メモリへ反映する。順序を
    /// 逆にすると、書き込みに失敗した変更がメモリにだけ残り、次の起動で黙って消える。
    fn commit_task<T>(
        clock: &dyn Clock,
        storage: &dyn Storage,
        state: &mut CoreState,
        index: usize,
        change: impl FnOnce(super::Timestamp, &mut Task) -> Result<T, DomainError>,
    ) -> Result<T, CoreError> {
        let mut draft = state.tasks[index].clone();
        let outcome = change(clock.now(), &mut draft)?;
        storage.apply(&Commit::of_task(draft.clone()))?;
        state.tasks[index] = draft;
        Ok(outcome)
    }

    /// 錠を取る。毒されていても常駐は止めない。
    ///
    /// 毒されるのは、この錠を握ったスレッドが panic したときだけである。コアの操作は
    /// panic しない契約であり (スパイン「一貫性の規約」)、毒された錠のために常駐が
    /// 以後まったく動かなくなるほうが害が大きい。
    fn lock(&self) -> MutexGuard<'_, CoreState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{FixedClock, Timestamp};
    use std::sync::{Arc, Mutex as StdMutex};

    /// 書き込みを記録するだけのストレージ。永続化の失敗も作れる。
    #[derive(Default)]
    struct RecordingStorage {
        restored: RestoredState,
        commits: StdMutex<Vec<Commit>>,
        fail_on_write: StdMutex<bool>,
    }

    impl RecordingStorage {
        fn shared() -> Arc<Self> {
            Arc::new(Self::default())
        }

        fn commits(&self) -> Vec<Commit> {
            self.commits.lock().expect("錠は毒されていない").clone()
        }

        fn set_failing(&self, failing: bool) {
            *self.fail_on_write.lock().expect("錠は毒されていない") = failing;
        }
    }

    impl Storage for Arc<RecordingStorage> {
        fn restore(&self) -> Result<RestoredState, StorageError> {
            Ok(self.restored.clone())
        }

        fn apply(&self, commit: &Commit) -> Result<(), StorageError> {
            if *self.fail_on_write.lock().expect("錠は毒されていない") {
                return Err(StorageError::Write("試験用の失敗".to_string()));
            }
            self.commits
                .lock()
                .expect("錠は毒されていない")
                .push(commit.clone());
            Ok(())
        }
    }

    fn core_with(storage: Arc<RecordingStorage>, clock: FixedClock) -> Core {
        Core::restore(Box::new(clock), Box::new(storage)).expect("空の状態は復元できる")
    }

    fn contents(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    fn a_core() -> (Core, Arc<RecordingStorage>, FixedClock) {
        let storage = RecordingStorage::shared();
        let clock = FixedClock::at(1_789_000_000_000);
        let core = core_with(Arc::clone(&storage), clock.clone());
        (core, storage, clock)
    }

    /// I/O マトリクス「初回起動」— 何も無ければ `NotStarted` で始まる。
    #[test]
    fn an_empty_store_starts_as_not_started() {
        let (core, _, _) = a_core();
        assert_eq!(core.current_position(), CurrentPosition::NotStarted);
        assert!(core.snapshot().tasks().is_empty());
    }

    /// タスク作成は 1 コミットで確定する。
    #[test]
    fn creating_a_task_commits_once() {
        let (core, storage, _) = a_core();
        let id = core
            .create_task("原稿", contents(&["下書き", "推敲"]))
            .expect("作れる");

        let commits = storage.commits();
        assert_eq!(commits.len(), 1, "1 操作 = 1 トランザクション");
        assert_eq!(commits[0].task.as_ref().map(Task::id), Some(id));
        assert!(commits[0].current_position.is_none());
        assert_eq!(core.snapshot().tasks().len(), 1);
    }

    /// ステップ 0 個は拒まれ、永続化も行われない。
    #[test]
    fn a_task_without_steps_is_refused_before_any_write() {
        let (core, storage, _) = a_core();
        assert_eq!(
            core.create_task("題名だけ", Vec::new()),
            Err(CoreError::Domain(DomainError::EmptyTask))
        );
        assert!(storage.commits().is_empty(), "拒否は何も書かない");
    }

    /// 永続化が失敗したらメモリ上の状態も変えない。
    #[test]
    fn a_failed_write_leaves_memory_untouched() {
        let (core, storage, _) = a_core();
        storage.set_failing(true);

        let outcome = core.create_task("原稿", contents(&["下書き"]));
        assert!(matches!(outcome, Err(CoreError::Storage(_))));
        assert!(
            core.snapshot().tasks().is_empty(),
            "書けなかった変更をメモリにだけ残さない"
        );
    }

    /// **タスクの書き換えも**、永続化が失敗したらメモリに残さない。
    ///
    /// `commit_task` の「永続化 → メモリ反映」の順序を入れ替えるとここが落ちる。
    #[test]
    fn a_failed_write_leaves_a_completion_undeclared() {
        let (core, storage, _) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き"]))
            .expect("作れる");
        let step_id = core.snapshot().task(task_id).expect("ある").steps()[0].id();

        storage.set_failing(true);
        let outcome = core.declare_completion(step_id);

        assert!(matches!(outcome, Err(CoreError::Storage(_))));
        assert!(
            !core
                .snapshot()
                .task(task_id)
                .expect("ある")
                .step(step_id)
                .expect("ある")
                .is_completed(),
            "書けなかった完了をメモリにだけ残さない"
        );
    }

    /// **現在地の移動も**、永続化が失敗したらメモリに残さない。
    #[test]
    fn a_failed_write_leaves_the_current_position_unchanged() {
        let (core, storage, _) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き", "推敲"]))
            .expect("作れる");
        let snapshot = core.snapshot();
        let task = snapshot.task(task_id).expect("ある");
        let (first, second) = (task.steps()[0].id(), task.steps()[1].id());
        core.move_current_position(first).expect("移せる");
        let before = core.current_position();

        storage.set_failing(true);
        let outcome = core.move_current_position(second);

        assert!(matches!(outcome, Err(CoreError::Storage(_))));
        assert_eq!(
            core.current_position(),
            before,
            "書けなかった移動をメモリにだけ残さない"
        );
    }

    /// 既に指しているステップを選び直しても何も書かない。
    #[test]
    fn re_selecting_the_current_step_writes_nothing() {
        let (core, storage, _) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き"]))
            .expect("作れる");
        let step_id = core.snapshot().task(task_id).expect("ある").steps()[0].id();
        core.move_current_position(step_id).expect("移せる");
        let commits = storage.commits().len();

        core.move_current_position(step_id).expect("失敗しない");

        assert_eq!(storage.commits().len(), commits, "無変更は書かない");
    }

    /// I/O マトリクス「完了の宣言」— 現在地は動かない (FR-4 / AD-2)。
    #[test]
    fn declaring_a_completion_does_not_move_the_current_position() {
        let (core, _, _) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き", "推敲"]))
            .expect("作れる");
        let snapshot = core.snapshot();
        let task = snapshot.task(task_id).expect("ある");
        let (first, second) = (task.steps()[0].id(), task.steps()[1].id());

        core.move_current_position(first).expect("移せる");
        core.declare_completion(second).expect("宣言できる");

        assert_eq!(
            core.current_position().step_id(),
            Some(first),
            "完了の宣言で現在地は動かない"
        );
        let snapshot = core.snapshot();
        let task = snapshot.task(task_id).expect("ある");
        assert!(task.step(second).expect("ある").is_completed());
        assert!(!task.step(first).expect("ある").is_completed());
    }

    /// 現在地の移動は完了に触れない。前のステップへ戻っても後続の完了は消えない。
    #[test]
    fn moving_the_current_position_never_touches_completion() {
        let (core, _, _) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き", "推敲"]))
            .expect("作れる");
        let snapshot = core.snapshot();
        let task = snapshot.task(task_id).expect("ある");
        let (first, second) = (task.steps()[0].id(), task.steps()[1].id());

        core.declare_completion(second).expect("宣言できる");
        core.move_current_position(first).expect("戻せる");

        let snapshot = core.snapshot();
        assert!(
            snapshot
                .task(task_id)
                .expect("ある")
                .step(second)
                .expect("ある")
                .is_completed(),
            "現在地が前へ戻っても後続の完了は取り消されない (FR-4)"
        );
    }

    /// I/O マトリクス「途中へのステップ追記」— 現在地は同一 ID を指し続ける。
    #[test]
    fn inserting_before_the_current_position_keeps_it_on_the_same_step() {
        let (core, _, _) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き", "推敲", "投稿"]))
            .expect("作れる");
        let snapshot = core.snapshot();
        let third = snapshot.task(task_id).expect("ある").steps()[2].id();
        core.move_current_position(third).expect("移せる");

        core.insert_step(task_id, 1, "資料を集める")
            .expect("挿入できる");

        assert_eq!(core.current_position().step_id(), Some(third));
        let snapshot = core.snapshot();
        let step = snapshot.step_at_current_position().expect("指し先がある");
        assert_eq!(step.ordinal(), 4, "連番は再計算される");
        assert_eq!(step.content(), "投稿", "指している作業単位は変わらない");
    }

    /// I/O マトリクス「ステップの分割」— 現在地は前半のまま。
    #[test]
    fn a_split_leaves_the_current_position_on_the_first_half() {
        let (core, _, _) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き", "推敲"]))
            .expect("作れる");
        let snapshot = core.snapshot();
        let second = snapshot.task(task_id).expect("ある").steps()[1].id();
        core.move_current_position(second).expect("移せる");
        core.set_interruption_note(second, Some(InterruptionNote::new("3 段落目まで")))
            .expect("メモを置ける");

        let created = core
            .split_step(second, "前半を推敲", "後半を推敲")
            .expect("分割できる");

        assert_eq!(core.current_position().step_id(), Some(second));
        let snapshot = core.snapshot();
        let step = snapshot.step_at_current_position().expect("指し先がある");
        assert_eq!(step.content(), "前半を推敲");
        assert_eq!(
            step.interruption_note().map(InterruptionNote::text),
            Some("3 段落目まで"),
            "中断メモは前半に帰属する"
        );
        assert_ne!(created, second);
    }

    /// I/O マトリクス「完了済みステップの分割」— 拒否され、何も書かれない。
    #[test]
    fn splitting_a_completed_step_is_refused_before_any_write() {
        let (core, storage, _) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き"]))
            .expect("作れる");
        let first = core.snapshot().task(task_id).expect("ある").steps()[0].id();
        core.declare_completion(first).expect("宣言できる");
        let before = storage.commits().len();

        assert_eq!(
            core.split_step(first, "前", "後"),
            Err(CoreError::Domain(DomainError::SplitCompleted))
        );
        assert_eq!(storage.commits().len(), before, "拒否は何も書かない");
    }

    /// I/O マトリクス「現在地の移動」— 同時に二つ存在しない。
    #[test]
    fn the_previous_position_is_released_when_a_new_one_is_taken() {
        let (core, _, _) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き", "推敲"]))
            .expect("作れる");
        let snapshot = core.snapshot();
        let task = snapshot.task(task_id).expect("ある");
        let (first, second) = (task.steps()[0].id(), task.steps()[1].id());

        core.move_current_position(first).expect("移せる");
        core.move_current_position(second).expect("移せる");

        assert_eq!(core.current_position().step_id(), Some(second));
        assert_ne!(core.current_position().step_id(), Some(first));
    }

    /// I/O マトリクス「活性のまま再指定」/「非活性からの復帰」— 連続作業時間の起点。
    #[test]
    fn the_work_clock_restarts_only_on_an_inactive_to_active_transition() {
        let (core, _, clock) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き", "推敲"]))
            .expect("作れる");
        let snapshot = core.snapshot();
        let task = snapshot.task(task_id).expect("ある");
        let (first, second) = (task.steps()[0].id(), task.steps()[1].id());

        let started = clock.now();
        core.move_current_position(first).expect("移せる");
        assert_eq!(core.current_position().activated_at(), Some(started));

        clock.advance(60_000);
        core.move_current_position(second).expect("移せる");
        assert_eq!(
            core.current_position().activated_at(),
            Some(started),
            "切り替えでは連続作業時間をリセットしない (AD-8)"
        );

        core.deactivate_current_position().expect("休息へ入れる");
        assert!(!core.current_position().is_active());
        assert_eq!(
            core.current_position().step_id(),
            Some(second),
            "非活性でも値は保持される (FR-6)"
        );

        clock.advance(600_000);
        let resumed = clock.now();
        core.activate_current_position().expect("復帰できる");
        assert_eq!(
            core.current_position().activated_at(),
            Some(resumed),
            "非活性から活性への遷移でのみ起点が更新される (AD-8)"
        );
    }

    /// 未着手のまま非活性化しても何も書かない。無変更のコミットを投げない。
    #[test]
    fn a_no_op_transition_writes_nothing() {
        let (core, storage, _) = a_core();
        core.deactivate_current_position().expect("失敗しない");
        core.activate_current_position().expect("失敗しない");
        assert!(storage.commits().is_empty());
    }

    /// 存在しないステップ・タスクへの操作は panic せず `Err` を返す。
    #[test]
    fn unknown_identifiers_are_errors_not_panics() {
        let (core, _, clock) = a_core();
        let stranger_step = StepId::new(clock.now());
        let stranger_task = TaskId::new(clock.now());

        assert_eq!(
            core.move_current_position(stranger_step),
            Err(CoreError::Domain(DomainError::UnknownStep))
        );
        assert_eq!(
            core.declare_completion(stranger_step),
            Err(CoreError::Domain(DomainError::UnknownStep))
        );
        assert_eq!(
            core.insert_step(stranger_task, 1, "x"),
            Err(CoreError::Domain(DomainError::UnknownTask))
        );
    }

    /// 現在地が存在しないステップを指す DB は破損として扱う。
    #[test]
    fn a_dangling_current_position_is_reported_as_corruption() {
        let clock = FixedClock::at(1_789_000_000_000);
        let storage = Arc::new(RecordingStorage {
            restored: RestoredState {
                tasks: Vec::new(),
                current_position: CurrentPosition::rehydrate(
                    TaskId::new(clock.now()),
                    StepId::new(clock.now()),
                    true,
                    Timestamp::from_unix_millis(0),
                ),
            },
            ..RecordingStorage::default()
        });

        let outcome = Core::restore(Box::new(clock), Box::new(storage));
        assert!(matches!(outcome, Err(StorageError::Corrupted(_))));
    }

    /// 現在地が別のタスクの ID を抱えている DB も破損として扱う。
    #[test]
    fn a_mismatched_task_on_the_current_position_is_reported_as_corruption() {
        let clock = FixedClock::at(1_789_000_000_000);
        let task = Task::create(clock.now(), "原稿", contents(&["下書き"])).expect("作れる");
        let step_id = task.steps()[0].id();
        let storage = Arc::new(RecordingStorage {
            restored: RestoredState {
                tasks: vec![task],
                current_position: CurrentPosition::rehydrate(
                    TaskId::new(clock.now()),
                    step_id,
                    true,
                    Timestamp::from_unix_millis(0),
                ),
            },
            ..RecordingStorage::default()
        });

        let outcome = Core::restore(Box::new(clock), Box::new(storage));
        assert!(matches!(outcome, Err(StorageError::Corrupted(_))));
    }

    /// 状態を変えうる操作が交錯しても、現在地は常に一つである (AD-5)。
    ///
    /// 錠が無ければ「読んで・決めて・書く」が割り込まれ、最後の書き手とメモリ上の値が
    /// 食い違いうる。スレッドを跨いで叩き、最終状態がどれか一つのステップを指している
    /// ことと、コミット数が操作数と一致することを見る。
    #[test]
    fn concurrent_operations_leave_exactly_one_current_position() {
        let storage = RecordingStorage::shared();
        let clock = FixedClock::at(1_789_000_000_000);
        let core = Arc::new(core_with(Arc::clone(&storage), clock));

        let task_id = core
            .create_task("原稿", contents(&["一", "二", "三", "四"]))
            .expect("作れる");
        let step_ids: Vec<StepId> = core
            .snapshot()
            .task(task_id)
            .expect("ある")
            .steps()
            .iter()
            .map(Step::id)
            .collect();

        let handles: Vec<_> = step_ids
            .iter()
            .copied()
            .map(|step_id| {
                let core = Arc::clone(&core);
                std::thread::spawn(move || {
                    for _ in 0..50 {
                        core.move_current_position(step_id).expect("移せる");
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("スレッドは panic しない");
        }

        let position = core.current_position();
        assert!(
            step_ids.contains(&position.step_id().expect("指し先がある")),
            "最終状態はどれか一つのステップを指す"
        );
        assert!(position.is_active());

        // **最後に書かれた現在地とメモリ上の現在地が一致する。** 錠が無ければ「読んで・
        // 決めて・書く」が割り込まれ、最後の書き手とメモリの値が食い違う。コミット数は
        // 無変更の短絡により上限だけが決まる (各スレッドの初回は必ず変化を起こすため、
        // 下限はスレッド数 + タスク作成の 1 件)。
        let commits = storage.commits();
        // タスク作成 1 件 + 各スレッドの初回の移動 (自分の担当ステップを指すのは自分
        // だけであるため、初回は必ず変化を起こす)。上限は無変更が一度も起きなかった場合。
        assert!(commits.len() >= 5);
        assert!(commits.len() <= 1 + 4 * 50);
        assert_eq!(
            commits
                .last()
                .expect("コミットがある")
                .current_position
                .expect("現在地のコミットである"),
            position
        );
    }
}
