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
use super::rest::{self, Decision, Intervention, InterventionChoice, RestSettings, Tick};
use super::setting::Setting;
use super::switch::SwitchRecord;
use super::task::{InterruptionNote, Step, StepId, Task, TaskId};
use super::{Clock, DomainError, Timestamp};
use crate::ports::storage::{Commit, RestoredState, Storage, StorageError};

/// コアが保持する状態そのもの。
///
/// フィールドは外へ出さない。状態を変えうる操作は [`Core`] のメソッドだけであり、
/// この型を握って好きに書き換える経路を作らない (AD-2)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreState {
    tasks: Vec<Task>,
    /// **唯一の現在地。** 集合ではないことが FR-6 の一意性そのものである。
    current_position: CurrentPosition,
    /// 永続化された**設定値** (AD-11)。**解釈ではなく、読んだままを持つ。**
    ///
    /// [`RestSettings`] を畳んだ形で持たないのは、同じ真実が二つの場所に生まれるためで
    /// ある — 書き換えのたびに畳み直す規律が要り、片方だけ古い状態を作れてしまう。
    settings: Vec<Setting>,
    /// 表示中の**介入**。**永続化しない。起動時は常に非表示** (AD-2)。
    ///
    /// **1 個のフィールドである。** 集合として持たないため「二つの介入」を表現する値が
    /// 存在しない — AD-7 の単一性は規律ではなく型の帰結である。
    intervention: Option<Intervention>,
    /// **猶予**の後に再提示する時刻。**永続化しない** (AD-2)。
    ///
    /// 再起動で失われてよい。失われれば次の刻みで**連続作業時間**を見て判断し直すだけ
    /// であり、**介入**が消えるのではなく出直しが早まるにすぎない。
    grace_until: Option<Timestamp>,
    /// 直前の刻みの時刻。**時計の飛び (スリープ) はこことの差で測る** (AD-8)。
    last_tick: Timestamp,
}

impl Default for CoreState {
    /// 何も読み込まれていない状態。**`last_tick` は [`Core::restore`] が必ず上書きする。**
    fn default() -> Self {
        Self {
            tasks: Vec::new(),
            current_position: CurrentPosition::NotStarted,
            settings: Vec::new(),
            intervention: None,
            grace_until: None,
            last_tick: Timestamp::MIN,
        }
    }
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

    /// **休息閾値**と**猶予** (AD-11)。値が無ければコード内の定数である。
    #[must_use]
    pub fn rest_settings(&self) -> RestSettings {
        RestSettings::from_settings(&self.settings)
    }

    /// 永続化された**設定値**そのもの (AD-11)。
    #[must_use]
    pub fn settings(&self) -> &[Setting] {
        &self.settings
    }

    /// 表示中の**介入** (AD-2)。
    #[must_use]
    pub const fn intervention(&self) -> Option<Intervention> {
        self.intervention
    }

    /// **休息**中か — **現在地**が値を保ったまま**非活性**である状態 (CAP-6 / FR-6)。
    ///
    /// **未着手**は休息ではない。値を持たない状態であり、そこから「復帰」する先が無い。
    #[must_use]
    pub const fn is_resting(&self) -> bool {
        matches!(self.current_position, CurrentPosition::Inactive { .. })
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
            settings,
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

        // **刻みの起点はいまである。** 起動前に流れた時間を時計の飛びとして扱わない —
        // 扱えば、朝いちばんの起動が「長いスリープから復帰した」ことになり、**連続作業
        // 時間**が黙ってリセットされる。プロセスが走っていなかった間の扱いは v1 の
        // 対象外であり、[`rest::decide`] が見るのは走っている間の飛びだけである。
        let last_tick = clock.now();
        Ok(Self {
            state: Mutex::new(CoreState {
                tasks,
                current_position,
                settings,
                // **起動時は常に非表示である** (AD-2)。
                intervention: None,
                grace_until: None,
                last_tick,
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

    /// **切り替え** — 離脱側の**中断メモ**の確定・**完了**の宣言 (任意)・同一**タスク**
    /// 内の次の**ステップ**への**現在地**の移動・**切り替え履歴**の追記を、**単一の
    /// トランザクション**で確定させる (CAP-7 / AD-5)。
    ///
    /// # なぜ既存の操作を順に呼ばないのか
    ///
    /// [`Self::set_interruption_note`] と [`Self::move_current_position`] はそれぞれ
    /// 独立した [`Commit`] を書く。順に呼べば書き込みは二つのトランザクションに割れ、
    /// その間の異常終了が「メモは残ったが**現在地**が動いていない」あるいはその逆を
    /// 残す。AD-5 が単一トランザクションを要求しているのは、まさにこの状態を禁じる
    /// ためである。
    ///
    /// # 引数
    ///
    /// - `note` — 離脱側に残す**中断メモ**。`None` は**省略**であり、**既存のメモを
    ///   消さない** (FR-7「省略しても切り替えは完了する」)。`Some` は「**今回**書かれた」
    ///   を意味し、**切り替え履歴**の `note_written` に写る。提示した既存のメモをその
    ///   まま送り返すのは「今回書いた」ではない — 呼び出し側 (コマンド境界とオーバー
    ///   レイ) が区別する責務を負う。読み返しただけの**切り替え**を記入として数えれば、
    ///   SM-C3 の記入率が膨らみ、偽陽性を排除するための指標が機能しなくなる。
    /// - `declare_completion` — **完了**を宣言するか。宣言しない**切り替え**も同じく
    ///   成立する。**完了**は**現在地**の移動から導出しない (FR-4 / AD-2)。
    ///
    /// # 戻り値
    ///
    /// 移動先の**ステップ**。最終**ステップ**からの**切り替え**では `None` であり、
    /// **現在地**は動かない — ただしメモ・**完了**・履歴は同じ 1 トランザクションで
    /// 確定する (I/O マトリクス「最終ステップからの切り替え」)。
    ///
    /// **連続作業時間はリセットされない。** 活性のまま移るため
    /// [`CurrentPosition::move_to`] が `activated_at` を保つ (AD-8)。
    ///
    /// # Errors
    ///
    /// **現在地**が**未着手**のとき [`DomainError::NoCurrentPosition`]、指し先が
    /// 見つからないとき [`DomainError::UnknownStep`]、永続化に失敗したとき
    /// [`CoreError::Storage`]。いずれの場合もメモリ上の状態は変わらない。
    pub fn switch_current_position(
        &self,
        note: Option<InterruptionNote>,
        declare_completion: bool,
    ) -> Result<Option<StepId>, CoreError> {
        let mut state = self.lock();
        let now = self.clock.now();

        let departed = state
            .current_position
            .step_id()
            .ok_or(DomainError::NoCurrentPosition)?;
        let index = state
            .tasks
            .iter()
            .position(|task| task.step(departed).is_some())
            .ok_or(DomainError::UnknownStep)?;

        // 判断は複製の上で行う。永続化が成功したときにだけメモリへ反映する
        // (`commit_task` と同じ順序)。
        let mut draft = state.tasks[index].clone();
        let note_written = note.is_some();
        if note_written {
            draft.set_interruption_note(departed, note)?;
        }
        if declare_completion {
            draft.declare_completion(now, departed)?;
        }

        // 移動先は同一**タスク**内の次の**ステップ**に限る。任意の**ステップ**への
        // 移動は CAP-9 の**開示面**に属する。
        let destination = next_step_of(&draft, departed);
        let moved = destination.map(|next| state.current_position.move_to(draft.id(), next, now));

        // **何も変わらないなら履歴も残さない。** 最終**ステップ**で Enter を繰り返す
        // だけで 1 行ずつ積めば、SM-C3 の分母が空の打鍵で膨らみ、記入率が下がった
        // ように見える。逆に、移動先が無くてもメモや**完了**が変わったなら残す —
        // 最終**ステップ**で書いたメモだけが分子から落ちるほうも同じく歪みである。
        let task_changed = draft != state.tasks[index];
        if !task_changed && moved.is_none() {
            return Ok(destination);
        }

        self.storage.apply(&Commit {
            // 変わっていない**タスク**を書き直さない。書けば失敗しうる I/O が増える
            // だけで、確定するものが一つも増えない。
            task: task_changed.then(|| draft.clone()),
            current_position: moved,
            switch_record: Some(SwitchRecord::new(now, departed, note_written)),
            settings: Vec::new(),
        })?;

        if task_changed {
            state.tasks[index] = draft;
        }
        if let Some(position) = moved {
            state.current_position = position;
        }
        Ok(destination)
    }

    /// **現在地**の**ステップ**の**完了**を宣言し、そこに留まる (CAP-7 / FR-4)。
    ///
    /// 離脱側の**中断メモ**の確定と**完了**の宣言を、**単一のトランザクション**で
    /// 確定させる (AD-5)。[`Self::switch_current_position`] との違いは一つだけである —
    /// **現在地**を動かさない。
    ///
    /// # なぜ次の**ステップ**へ進めないのか
    ///
    /// **完了**の次にどこへ行くかを決めるのは利用者である。同一**タスク**内の次の
    /// **ステップ**へ自動で進めば、そのタスクを続ける以外の選択が「進んでしまった後の
    /// 訂正」になり、訂正のたびに実際には作業していない**ステップ**からの離脱が
    /// **切り替え履歴**に残る。移動は利用者が選んだ時点で
    /// [`Self::select_step`] が確定させる。
    ///
    /// # なぜ**切り替え履歴**を残さないのか
    ///
    /// 用語集の**切り替え**は「**現在地**をあるステップから別のステップへ移す操作」で
    /// あり、ここでは**現在地**が動かない。離脱はまだ起きていない。1 行積めば、続く
    /// [`Self::select_step`] の 1 行と合わせて一度の離脱が二度数えられ、SM-C3 の分母が
    /// 膨らむ。
    ///
    /// **その代償として、ここで書かれた**中断メモ**は SM-C3 の分子に入らない** —
    /// 続く [`Self::select_step`] が `note_written` を常に偽で積むためである。この経路が
    /// メモの機会を与える唯一の「履歴を残さない」操作であることは申し送りに残してある。
    ///
    /// # 引数
    ///
    /// - `note` — 離脱側に残す**中断メモ**。`None` は**省略**であり、**既存のメモを
    ///   消さない** ([`Self::switch_current_position`] と同じ規則)。
    ///
    /// # Errors
    ///
    /// **現在地**が**未着手**のとき [`DomainError::NoCurrentPosition`]、指し先が
    /// 見つからないとき [`DomainError::UnknownStep`]、永続化に失敗したとき
    /// [`CoreError::Storage`]。いずれの場合もメモリ上の状態は変わらない。
    pub fn complete_current_step(&self, note: Option<InterruptionNote>) -> Result<(), CoreError> {
        let mut state = self.lock();
        let now = self.clock.now();

        let here = state
            .current_position
            .step_id()
            .ok_or(DomainError::NoCurrentPosition)?;
        let index = state
            .tasks
            .iter()
            .position(|task| task.step(here).is_some())
            .ok_or(DomainError::UnknownStep)?;

        // 判断は複製の上で行い、永続化が成功したときにだけメモリへ反映する
        // (`commit_task` と同じ順序)。
        let mut draft = state.tasks[index].clone();
        if note.is_some() {
            draft.set_interruption_note(here, note)?;
        }
        draft.declare_completion(now, here)?;

        // **何も変わらないなら書かない。** 既に**完了**しており、メモも省略された
        // 二度目の宣言で、何も変えない要求が永続化の失敗で `Err` になりうる。
        if draft == state.tasks[index] {
            return Ok(());
        }

        self.storage.apply(&Commit::of_task(draft.clone()))?;
        state.tasks[index] = draft;
        Ok(())
    }

    /// **タスク**を直す — 題名と既存の**ステップ**の本文を書き換え、末尾へ**ステップ**を
    /// 追記する (CAP-5 / FR-4 / FR-5)。
    ///
    /// 三つの変更は**単一のトランザクション**で確定する (AD-5)。別々に呼べば、題名だけが
    /// 直って**ステップ**が元のまま残る状態が異常終了で残りうる。
    ///
    /// **消す経路は無い。** [`Commit`] は行を消す変更を持たず (`ports/storage.rs`)、v1 は
    /// **タスク**も**ステップ**も削除しない。直せるのは本文だけである。
    ///
    /// **現在地**・**完了**・**中断メモ**のいずれにも触れない。指しているのが ID である
    /// 以上、本文を直しても**現在地**は同じ作業単位を指し続ける (FR-5)。
    ///
    /// # Errors
    ///
    /// **タスク**が無いとき [`DomainError::UnknownTask`]、指定の**ステップ**がその
    /// **タスク**に無いとき [`DomainError::UnknownStep`]、永続化に失敗したとき
    /// [`CoreError::Storage`]。いずれの場合もメモリ上の状態は変わらない。
    pub fn edit_task(
        &self,
        task_id: TaskId,
        title: String,
        contents: Vec<(StepId, String)>,
        appended: Vec<String>,
    ) -> Result<(), CoreError> {
        self.mutate_task(task_id, |now, task| {
            task.rename(title);
            for (step_id, content) in contents {
                task.set_step_content(step_id, content)?;
            }
            for content in appended {
                task.append_step(now, content)?;
            }
            Ok(())
        })
    }

    /// **開示面からの切り替え** — 任意の**ステップ**へ**現在地**を移し、**切り替え履歴**を
    /// 追記する。二つは**単一のトランザクション**で確定する (CAP-9 / FR-19 / AD-5)。
    ///
    /// 用語集は**切り替え**を「**現在地**をあるステップから別のステップへ移す操作」と
    /// 定めており、**タスク**を跨ぐ移動もそれに当たる。したがって履歴を残す。
    ///
    /// # なぜ [`Self::move_current_position`] を呼ばないのか
    ///
    /// あれは履歴を残さない素の移動であり、CAP-6 の一意性を保つためだけの原始操作である。
    /// この経路から呼べば、移動が記録されないまま**現在地**だけが動く。その履歴は
    /// SM-C3 の分母であり、経路が丸ごと落ちれば記入率が実態より高く見える。
    ///
    /// # なぜ**中断メモ**を受け取らないのか
    ///
    /// 一覧から行を選ぶ操作にメモの入力欄を挟めば、CAP-7 の儀式を一覧の中に作り直す
    /// ことになり、**開示面**が編集の面へ滑り出す。**この経路は機会を与えない**ため、
    /// 履歴の `note_written` は常に偽である。**機会を与えていない移動を記入率の分子に
    /// 数えない**ことが、この判断が SM-C3 を歪めないための条件である
    /// (spec Design Notes)。
    ///
    /// **完了**にも一切触れない。**完了**は**現在地**の移動で自動的に付与も取消もされない
    /// (FR-4 / AD-2)。離脱側の**中断メモ**も変えない。
    ///
    /// # **未着手**からの移動に履歴が無い理由
    ///
    /// [`SwitchRecord`] は離脱元の**ステップ**を必ず持つ。**未着手**には離れる場所が無く、
    /// 用語集の定義上そこからの移動は**切り替え**ではない。欠けた値を捏造して 1 行
    /// 積むより、離脱が無かったことを記録の不在で表す。着手せずに作った**タスク**へ
    /// 初めて到達する経路がこれであり、**移動そのものは成立する**。
    ///
    /// # **非活性** (**休息**中) に選んだときの扱い
    ///
    /// **移した先は活性である。** [`CurrentPosition::move_to`] が定める通りであり、
    /// [`Self::move_current_position`] も同じ値を通る — 「**現在地**を移す」の意味を
    /// 経路ごとに変えない。**非活性**から**活性**への遷移は AD-8 が唯一のリセット契機と
    /// して名指しているものであり、一覧から**ステップ**を選ぶ行為は用語集の**再入**
    /// (中断された**ステップ**が再び**現在地**となり作業が再開されること) そのものである。
    /// したがって**連続作業時間**はここで数え直される。**活性**のまま移った場合は
    /// リセットされない (AD-8 / FR-15)。
    ///
    /// **同じ**ステップ**を選んだ場合も同様に扱う。** 判定は `step_id` ではなく
    /// **現在地**の値そのもので行う ([`Self::move_current_position`] と同じ) —
    /// `step_id` だけで短絡すると、**休息**中に自分の行を選んだときだけ何も起きず、
    /// 「選べば現在地がそこへ移る」が状態によって成り立たなくなる。ただし**離脱が
    /// 起きていない**以上、**切り替え履歴**は残さない。
    ///
    /// # 戻り値
    ///
    /// **現在地**が変わったか — すなわち何かが書かれたか。値が一つも変わらない選択では
    /// `false` であり、**何も書かない — 履歴も増えない**
    /// (I/O マトリクス「同じステップを選ぶ」)。
    ///
    /// # Errors
    ///
    /// **ステップ**が見つからないとき [`DomainError::UnknownStep`]、永続化に失敗した
    /// とき [`CoreError::Storage`]。いずれの場合もメモリ上の状態は変わらない。
    pub fn select_step(&self, step_id: StepId) -> Result<bool, CoreError> {
        let mut state = self.lock();
        let now = self.clock.now();

        let task_id = state
            .task_of_step(step_id)
            .ok_or(DomainError::UnknownStep)?
            .id();

        let moved = state.current_position.move_to(task_id, step_id, now);
        // **値が一つも変わらないなら何も書かない。** 書けば、一覧を開いて Enter を押す
        // だけで履歴が 1 行ずつ積まれ、SM-C3 の分母が空の打鍵で膨らむ
        // (`move_current_position` の短絡と同じ判定である)。
        if moved == state.current_position {
            return Ok(false);
        }

        // **離脱元が無い、あるいは離脱先と同じなら切り替えではない。** 前者は**未着手**
        // からの移動、後者は**休息**からの**再入**である。どちらも「あるステップから別の
        // ステップへ移す」に当たらず、履歴に 1 行積めば SM-C3 の分母だけが膨らむ。
        let departed = state
            .current_position
            .step_id()
            .filter(|departed| *departed != step_id);

        self.storage.apply(&Commit {
            // **タスク**は書き直さない。メモにも**完了**にも触れないため、変わるものが
            // 一つも無い。
            task: None,
            current_position: Some(moved),
            // **機会を与えていないため `note_written` は常に偽である。**
            switch_record: departed.map(|departed| SwitchRecord::new(now, departed, false)),
            settings: Vec::new(),
        })?;
        state.current_position = moved;
        Ok(true)
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

    // --- 休息介入 (CAP-10 / FR-15) ---------------------------------------------

    /// **計時の刻み** (AD-8)。**連続作業時間**を測り、必要なら**介入**を発する。
    ///
    /// 判断そのものは [`rest::decide`] が持つ純粋関数であり、ここが行うのは
    /// 「状態を読む → 判断させる → 永続化 → メモリ反映」の直列化だけである
    /// ([`Self::commit_task`] と同じ順序)。**この経路も他の状態変更と同じ一つの錠を
    /// 通る** — **休息**への遷移と**切り替え**のコミットが交錯しない (AD-5)。
    ///
    /// # 戻り値
    ///
    /// 提示層が行うべきこと。**この関数はパネルを出さない** — OS を知らないためである。
    ///
    /// # Errors
    ///
    /// スリープ分の差し引きや計時のリセットを永続化できなかったとき。そのとき
    /// **直前の刻みの時刻は進めない** — 進めると、差し引けなかったスリープが
    /// **連続作業時間**に混ざったまま二度と補正されない。次の刻みが同じ飛びを見て
    /// やり直す。
    pub fn tick(&self) -> Result<TickEffect, CoreError> {
        let mut state = self.lock();
        let now = self.clock.now();

        let decision = rest::decide(&Tick {
            position: state.current_position,
            now,
            previous: state.last_tick,
            interval_millis: rest::TICK_INTERVAL_MILLIS,
            settings: state.rest_settings(),
            grace_until: state.grace_until,
            // **表示中なら後発は待つ** (AD-7)。
            showing: state.intervention.is_some(),
        });

        if let Some(adjusted) = adjusted_position(state.current_position, &decision) {
            self.storage.apply(&Commit::of_current_position(adjusted))?;
            state.current_position = adjusted;
        }

        state.last_tick = now;
        state.grace_until = decision.grace_until;
        if decision.raise {
            state.intervention = Some(Intervention::raised_at(now));
        }
        Ok(TickEffect {
            raised: decision.raise,
        })
    }

    /// **介入への応答** (CAP-10 / FR-15)。**介入を閉じる唯一の経路である** (AD-7)。
    ///
    /// - [`InterventionChoice::Rest`] — **現在地**は値を保ったまま**非活性**になり、
    ///   計数が止まる (FR-6)。
    /// - [`InterventionChoice::Grace`] — **現在地**は**活性**のまま、**猶予**の後に
    ///   出直す。**上限は無い** (spec Design Notes)。
    ///
    /// # 戻り値
    ///
    /// 応答すべき**介入**が表示されていたか。表示されていなければ `false` であり、
    /// **何も起きない** — ホットキーの二重発火や、応答と同時に届いた別経路の応答が
    /// **現在地**を二度非活性にすることを防ぐ。
    ///
    /// # Errors
    ///
    /// **休息**への遷移を永続化できなかったとき。そのとき**状態は何も変わらず、介入も
    /// 閉じない** (I/O マトリクス「休息に入る」)。閉じてしまえば、選んだはずの休息が
    /// どこにも残らないまま画面から消える。
    pub fn answer_intervention(&self, choice: InterventionChoice) -> Result<bool, CoreError> {
        let mut state = self.lock();
        if state.intervention.is_none() {
            return Ok(false);
        }
        let now = self.clock.now();

        match choice {
            InterventionChoice::Rest => {
                let next = state.current_position.deactivate();
                if next != state.current_position {
                    self.storage.apply(&Commit::of_current_position(next))?;
                    state.current_position = next;
                }
                // **休息**に入った以上、出直しの約束は残さない。
                state.grace_until = None;
            }
            InterventionChoice::Grace => {
                state.grace_until = Some(after(now, state.rest_settings().grace_period_millis()));
            }
        }

        state.intervention = None;
        Ok(true)
    }

    /// **介入を取り下げる** (AD-7)。提示そのものに失敗したときの経路である。
    ///
    /// 表示できなかった**介入**を状態の上だけ「表示中」にしておくと、後続の契機が永久に
    /// 待ち続ける。取り下げたうえで**猶予**を与え、出直させる — 与えなければ、出せない
    /// **介入**を刻みのたびに出し直すことになる。
    ///
    /// **応答ではない。** 利用者は何も選んでおらず、**現在地**にも触れない。
    pub fn withdraw_intervention(&self) -> bool {
        let mut state = self.lock();
        if state.intervention.take().is_none() {
            return false;
        }
        let now = self.clock.now();
        state.grace_until = Some(after(now, state.rest_settings().grace_period_millis()));
        true
    }

    /// **休息の終了** (CAP-10 / FR-15)。**ユーザーの明示的な宣言による。**
    ///
    /// **現在地**は**活性**へ戻り、**連続作業時間**はそこから数え直される — **非活性**
    /// から**活性**への遷移が AD-8 の唯一のリセット契機である。
    ///
    /// # 戻り値
    ///
    /// **休息**中であったか。**未着手**や**活性**のときは `false` であり、何も書かない。
    ///
    /// # Errors
    ///
    /// 永続化に失敗したとき。状態は変わらない。
    pub fn end_rest(&self) -> Result<bool, CoreError> {
        let mut state = self.lock();
        if !state.is_resting() {
            return Ok(false);
        }
        let next = state.current_position.activate(self.clock.now());
        self.storage.apply(&Commit::of_current_position(next))?;
        state.current_position = next;
        state.grace_until = None;
        Ok(true)
    }

    /// **設定値**を書き換える (AD-11)。
    ///
    /// v1 に設定の面は無い。この経路は、**休息閾値**を短くして**介入**を実際に出す手動
    /// 確認 (spec Verification) と、永続化の往復を確かめる検査のために存在する。
    ///
    /// # Errors
    ///
    /// 永続化に失敗したとき。**メモリ上の設定も変えない。**
    pub fn store_setting(&self, setting: Setting) -> Result<(), CoreError> {
        let mut state = self.lock();
        self.storage
            .apply(&Commit::of_settings(vec![setting.clone()]))?;
        match state
            .settings
            .iter_mut()
            .find(|stored| stored.key() == setting.key())
        {
            Some(stored) => *stored = setting,
            None => state.settings.push(setting),
        }
        Ok(())
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

/// [`Core::tick`] が提示層へ返す、行うべきこと。
///
/// **ここに OS の語彙は無い。** 「パネルを出す」ではなく「**介入**を発した」であり、
/// それをどう見せるかはアダプタの仕事である (AD-1)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TickEffect {
    /// **介入**を発したか。真ならアダプタがパネルを出し、応答のホットキーを登録する。
    pub raised: bool,
}

/// **起点の付け替えが要るなら、付け替え後の**現在地**を返す純粋関数** (AD-8)。
///
/// 要らないなら `None` — **何も変わらないなら何も書かない**。書けば、待機している
/// だけの常駐が 10 秒ごとに DB へ書き込むことになる。
fn adjusted_position(position: CurrentPosition, decision: &Decision) -> Option<CurrentPosition> {
    let activated_at = decision.activated_at?;
    if position.activated_at() == Some(activated_at) {
        return None;
    }
    Some(CurrentPosition::rehydrate(
        position.task_id()?,
        position.step_id()?,
        position.is_active(),
        activated_at,
    ))
}

/// 時刻にミリ秒を足す。範囲外は [`Timestamp`] が端へ丸める。
fn after(at: Timestamp, millis: i64) -> Timestamp {
    Timestamp::from_unix_millis(at.unix_millis().saturating_add(millis))
}

/// 同一**タスク**内で、指定の**ステップ**の次に来る**ステップ**。
///
/// 並びは `ordinal` 昇順であり ([`Task::steps`])、次とは「一つ後ろの要素」である。
/// **完了**済みを読み飛ばさない — 読み飛ばせば**完了**が**現在地**の移動先を決める
/// ことになり、「**完了**を**現在地**から導出しない」の裏返しが起きる (FR-4 / AD-2)。
fn next_step_of(task: &Task, step_id: StepId) -> Option<StepId> {
    let index = task.steps().iter().position(|step| step.id() == step_id)?;
    task.steps().get(index + 1).map(Step::id)
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

    /// **完了**の宣言は**現在地**を動かさず、1 コミットで確定する。
    ///
    /// **切り替え履歴**を積まないことが要である — 続く [`Core::select_step`] が 1 行
    /// 積むため、ここでも積めば一度の離脱が二度数えられる。
    #[test]
    fn completing_the_current_step_stays_where_it_is() {
        let (core, storage, _) = a_core();
        let task = core
            .create_task("原稿", contents(&["下書き", "推敲"]))
            .expect("作れる");
        let first = core.snapshot().task(task).expect("在る").steps()[0].id();
        core.move_current_position(first).expect("着手できる");
        let before = core.current_position();
        let commits_before = storage.commits().len();

        core.complete_current_step(Some(InterruptionNote::new("ここまで")))
            .expect("宣言できる");

        let commits = storage.commits();
        assert_eq!(
            commits.len(),
            commits_before + 1,
            "1 操作 = 1 トランザクション"
        );
        let commit = commits.last().expect("在る");
        assert!(commit.current_position.is_none(), "現在地は動かない");
        assert!(
            commit.switch_record.is_none(),
            "離脱していない以上、履歴も積まない"
        );
        assert_eq!(core.current_position(), before);

        let step = core.snapshot().task(task).expect("在る").steps()[0].clone();
        assert!(step.is_completed(), "完了は宣言されている");
        assert_eq!(
            step.interruption_note().map(InterruptionNote::text),
            Some("ここまで"),
            "メモも同じトランザクションで確定する"
        );
    }

    /// **未着手**では宣言する相手が無く、何も書かれない。
    #[test]
    fn completing_without_a_current_position_is_refused_before_any_write() {
        let (core, storage, _) = a_core();
        assert_eq!(
            core.complete_current_step(None),
            Err(CoreError::Domain(DomainError::NoCurrentPosition))
        );
        assert!(storage.commits().is_empty(), "拒否は何も書かない");
    }

    /// 二度目の宣言は何も変えず、書き込みも起こさない。
    #[test]
    fn a_second_completion_writes_nothing() {
        let (core, storage, _) = a_core();
        let task = core
            .create_task("原稿", contents(&["下書き"]))
            .expect("作れる");
        let first = core.snapshot().task(task).expect("在る").steps()[0].id();
        core.move_current_position(first).expect("着手できる");
        core.complete_current_step(None).expect("宣言できる");
        let commits_before = storage.commits().len();

        core.complete_current_step(None)
            .expect("二度目も拒まれない");

        assert_eq!(
            storage.commits().len(),
            commits_before,
            "変わるものが無いなら書かない"
        );
    }

    /// 修正は題名・本文・追記を 1 コミットで確定し、ID も完了も**現在地**も保つ。
    #[test]
    fn editing_a_task_commits_once_and_keeps_every_mark() {
        let (core, storage, _) = a_core();
        let task = core
            .create_task("原稿", contents(&["下書き", "推敲"]))
            .expect("作れる");
        let steps: Vec<StepId> = core
            .snapshot()
            .task(task)
            .expect("在る")
            .steps()
            .iter()
            .map(Step::id)
            .collect();
        core.move_current_position(steps[1]).expect("着手できる");
        core.declare_completion(steps[0]).expect("完了できる");
        let before = core.current_position();
        let commits_before = storage.commits().len();

        core.edit_task(
            task,
            "原稿を仕上げる".to_string(),
            vec![(steps[1], "推敲する".to_string())],
            contents(&["投稿する"]),
        )
        .expect("直せる");

        assert_eq!(
            storage.commits().len(),
            commits_before + 1,
            "三つの変更で 1 トランザクション"
        );
        let state = core.snapshot();
        let edited = state.task(task).expect("在る");
        assert_eq!(edited.title(), "原稿を仕上げる");
        assert_eq!(
            edited.steps().iter().map(Step::content).collect::<Vec<_>>(),
            vec!["下書き", "推敲する", "投稿する"]
        );
        assert_eq!(
            edited.steps()[..2].iter().map(Step::id).collect::<Vec<_>>(),
            steps,
            "既存のステップの ID は変わらない"
        );
        assert!(edited.steps()[0].is_completed(), "完了は落ちない");
        assert_eq!(core.current_position(), before, "現在地も動かない");
    }

    /// 無い**ステップ**を指した修正は、題名も含めて何も書かない。
    #[test]
    fn an_edit_naming_an_unknown_step_writes_nothing() {
        let (core, storage, _) = a_core();
        let task = core
            .create_task("原稿", contents(&["下書き"]))
            .expect("作れる");
        let elsewhere = StepId::new(Timestamp::from_unix_millis(1_789_000_000_000));
        let commits_before = storage.commits().len();

        assert_eq!(
            core.edit_task(
                task,
                "別の題名".to_string(),
                vec![(elsewhere, "どこにも無い".to_string())],
                Vec::new(),
            ),
            Err(CoreError::Domain(DomainError::UnknownStep))
        );

        assert_eq!(
            storage.commits().len(),
            commits_before,
            "拒否は何も書かない"
        );
        assert_eq!(
            core.snapshot().task(task).expect("在る").title(),
            "原稿",
            "題名も元のままである"
        );
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
                settings: Vec::new(),
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
                settings: Vec::new(),
            },
            ..RecordingStorage::default()
        });

        let outcome = Core::restore(Box::new(clock), Box::new(storage));
        assert!(matches!(outcome, Err(StorageError::Corrupted(_))));
    }

    /// 受け入れ条件「メモを書いて確定 → 現在地は次へ・履歴の `note_written` は真」。
    ///
    /// **三者が一つのコミットに載ることが本スライスの核心である** (AD-5)。
    #[test]
    fn a_switch_commits_the_note_the_move_and_the_record_together() {
        let (core, storage, _) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き", "推敲", "投稿"]))
            .expect("作れる");
        let snapshot = core.snapshot();
        let task = snapshot.task(task_id).expect("ある");
        let (first, second) = (task.steps()[0].id(), task.steps()[1].id());
        core.move_current_position(first).expect("移せる");
        let before = storage.commits().len();

        let destination = core
            .switch_current_position(Some(InterruptionNote::new("3 段落目まで")), false)
            .expect("切り替えられる");

        assert_eq!(
            destination,
            Some(second),
            "移動先は同一タスク内の次のステップ"
        );
        let commits = storage.commits();
        assert_eq!(
            commits.len(),
            before + 1,
            "離脱側のメモ・現在地の移動・履歴の追記で 1 トランザクション (AD-5)"
        );
        let commit = commits.last().expect("ある");
        assert!(
            commit.task.is_some(),
            "メモを載せたタスクが同じコミットにある"
        );
        assert_eq!(
            commit.current_position.and_then(|p| p.step_id()),
            Some(second),
            "現在地も同じコミットにある"
        );
        let record = commit
            .switch_record
            .as_ref()
            .expect("履歴も同じコミットにある");
        assert_eq!(record.departed_step_id(), first, "離脱元を記録する");
        assert!(record.note_written(), "メモを書いた事実が残る");

        assert_eq!(core.current_position().step_id(), Some(second));
        let snapshot = core.snapshot();
        assert_eq!(
            snapshot
                .task(task_id)
                .expect("ある")
                .step(first)
                .expect("ある")
                .interruption_note()
                .map(InterruptionNote::text),
            Some("3 段落目まで")
        );
    }

    /// 受け入れ条件「メモを空のまま確定 → 現在地は次へ・`note_written` は偽」。
    ///
    /// **省略は既存のメモを消さない。** 消すなら「省略」ではなく「削除」であり、
    /// FR-7 の「省略しても切り替えは完了する」とは別の操作になる。
    #[test]
    fn omitting_the_note_still_completes_the_switch_and_keeps_the_previous_note() {
        let (core, storage, _) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き", "推敲"]))
            .expect("作れる");
        let snapshot = core.snapshot();
        let task = snapshot.task(task_id).expect("ある");
        let (first, second) = (task.steps()[0].id(), task.steps()[1].id());
        core.move_current_position(first).expect("移せる");
        core.set_interruption_note(first, Some(InterruptionNote::new("前に書いた")))
            .expect("メモを置ける");

        let destination = core
            .switch_current_position(None, false)
            .expect("メモを省いても切り替えられる");

        assert_eq!(destination, Some(second));
        assert_eq!(core.current_position().step_id(), Some(second));
        let record = storage
            .commits()
            .last()
            .expect("ある")
            .switch_record
            .clone()
            .expect("履歴がある");
        assert!(!record.note_written(), "省略は記入なしとして記録される");
        assert_eq!(
            core.snapshot()
                .task(task_id)
                .expect("ある")
                .step(first)
                .expect("ある")
                .interruption_note()
                .map(InterruptionNote::text),
            Some("前に書いた"),
            "省略は既存のメモを消さない"
        );
    }

    /// I/O マトリクス「二度目の切り替え」— 確定内容が既存のメモを置き換える。
    #[test]
    fn a_second_switch_replaces_the_previous_note() {
        let (core, _, _) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き", "推敲", "投稿"]))
            .expect("作れる");
        let snapshot = core.snapshot();
        let task = snapshot.task(task_id).expect("ある");
        let (first, second) = (task.steps()[0].id(), task.steps()[1].id());
        core.move_current_position(first).expect("移せる");
        core.switch_current_position(Some(InterruptionNote::new("一度目")), false)
            .expect("切り替えられる");
        core.move_current_position(first).expect("戻せる");

        core.switch_current_position(Some(InterruptionNote::new("一度目 / 二度目")), false)
            .expect("切り替えられる");

        assert_eq!(
            core.snapshot()
                .task(task_id)
                .expect("ある")
                .step(first)
                .expect("ある")
                .interruption_note()
                .map(InterruptionNote::text),
            Some("一度目 / 二度目")
        );
        assert_eq!(core.current_position().step_id(), Some(second));
    }

    /// 受け入れ条件「完了を伴う切り替え → 離脱側に `completed_at`・現在地が移動・
    /// 履歴が 1 行。三者が同時に成立」。
    #[test]
    fn a_switch_can_declare_the_completion_in_the_same_transaction() {
        let (core, storage, _) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き", "推敲"]))
            .expect("作れる");
        let snapshot = core.snapshot();
        let task = snapshot.task(task_id).expect("ある");
        let (first, second) = (task.steps()[0].id(), task.steps()[1].id());
        core.move_current_position(first).expect("移せる");
        let before = storage.commits().len();

        core.switch_current_position(Some(InterruptionNote::new("ここまで")), true)
            .expect("切り替えられる");

        assert_eq!(storage.commits().len(), before + 1, "1 トランザクション");
        let commit = storage.commits().last().expect("ある").clone();
        let committed = commit.task.expect("タスクが載っている");
        assert!(
            committed.step(first).expect("ある").is_completed(),
            "完了も同じコミットに載る"
        );
        assert_eq!(
            commit.current_position.and_then(|p| p.step_id()),
            Some(second)
        );
        assert!(commit.switch_record.is_some());
        assert_eq!(core.current_position().step_id(), Some(second));
    }

    /// **完了を宣言しない切り替えも同じく成立する** (FR-4 / AD-2)。
    ///
    /// 完了は現在地の移動から導出しない。
    #[test]
    fn a_switch_without_a_declaration_leaves_the_step_incomplete() {
        let (core, _, _) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き", "推敲"]))
            .expect("作れる");
        let first = core.snapshot().task(task_id).expect("ある").steps()[0].id();
        core.move_current_position(first).expect("移せる");

        core.switch_current_position(None, false)
            .expect("切り替えられる");

        assert!(
            !core
                .snapshot()
                .task(task_id)
                .expect("ある")
                .step(first)
                .expect("ある")
                .is_completed(),
            "宣言しなければ完了は付かない (FR-4)"
        );
    }

    /// I/O マトリクス「最終ステップからの切り替え」— 現在地は動かないが、メモと完了は
    /// 成立し、履歴も残る。
    #[test]
    fn a_switch_from_the_last_step_records_but_does_not_move() {
        let (core, storage, _) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き", "推敲"]))
            .expect("作れる");
        let second = core.snapshot().task(task_id).expect("ある").steps()[1].id();
        core.move_current_position(second).expect("移せる");
        let before = storage.commits().len();

        let destination = core
            .switch_current_position(Some(InterruptionNote::new("続きは明日")), true)
            .expect("メモと完了は成立する");

        assert_eq!(destination, None, "移動先が無い");
        assert_eq!(
            core.current_position().step_id(),
            Some(second),
            "現在地は動かない"
        );
        let commit = storage.commits().last().expect("ある").clone();
        assert_eq!(storage.commits().len(), before + 1);
        assert!(
            commit.current_position.is_none(),
            "動かない現在地を書き直さない"
        );
        assert!(commit.switch_record.is_some(), "履歴は残る (SM-C3 の分母)");
        let snapshot = core.snapshot();
        let step = snapshot
            .task(task_id)
            .expect("ある")
            .step(second)
            .expect("ある");
        assert!(step.is_completed());
        assert_eq!(
            step.interruption_note().map(InterruptionNote::text),
            Some("続きは明日")
        );
    }

    /// **何も変わらない切り替えは履歴を残さない。**
    ///
    /// 最終**ステップ**で Enter を繰り返すだけで 1 行ずつ積めば、SM-C3 の分母が空の
    /// 打鍵で膨らみ、記入率が下がったように見える。
    #[test]
    fn a_switch_that_changes_nothing_writes_no_record() {
        let (core, storage, _) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き", "推敲"]))
            .expect("作れる");
        let second = core.snapshot().task(task_id).expect("ある").steps()[1].id();
        core.move_current_position(second).expect("移せる");
        let before = storage.commits().len();

        for _ in 0..3 {
            assert_eq!(
                core.switch_current_position(None, false),
                Ok(None),
                "最終ステップでは移動先が無い"
            );
        }

        assert_eq!(storage.commits().len(), before, "何も書かない");
    }

    /// **最終ステップでも、メモが書かれたなら履歴を残す。**
    ///
    /// 分子から落とせば「最終ステップで書いたメモだけ数えない」という逆の歪みになる。
    #[test]
    fn a_switch_from_the_last_step_records_when_a_note_was_written() {
        let (core, storage, _) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き", "推敲"]))
            .expect("作れる");
        let second = core.snapshot().task(task_id).expect("ある").steps()[1].id();
        core.move_current_position(second).expect("移せる");
        let before = storage.commits().len();

        core.switch_current_position(Some(InterruptionNote::new("続きは明日")), false)
            .expect("成立する");

        assert_eq!(storage.commits().len(), before + 1);
        let record = storage
            .commits()
            .last()
            .expect("ある")
            .switch_record
            .clone()
            .expect("履歴がある");
        assert!(record.note_written());
    }

    /// **切り替えでは連続作業時間をリセットしない** (AD-8)。
    #[test]
    fn a_switch_does_not_reset_the_work_clock() {
        let (core, _, clock) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き", "推敲"]))
            .expect("作れる");
        let first = core.snapshot().task(task_id).expect("ある").steps()[0].id();
        let started = clock.now();
        core.move_current_position(first).expect("移せる");

        clock.advance(600_000);
        core.switch_current_position(None, false)
            .expect("切り替えられる");

        assert_eq!(
            core.current_position().activated_at(),
            Some(started),
            "切り替えは非活性→活性の遷移ではない (AD-8)"
        );
    }

    /// **未着手からは切り替えられない。** 離れるべき場所が無い。
    #[test]
    fn switching_from_not_started_is_an_error() {
        let (core, storage, _) = a_core();
        assert_eq!(
            core.switch_current_position(None, false),
            Err(CoreError::Domain(DomainError::NoCurrentPosition))
        );
        assert!(storage.commits().is_empty(), "拒否は何も書かない");
    }

    /// I/O マトリクス「書き込み失敗」— 状態を変えず失敗を返す。
    ///
    /// **三者のどれ一つもメモリに残さない。** 一部だけ残れば、次の起動で黙って消える。
    #[test]
    fn a_failed_switch_changes_nothing_in_memory() {
        let (core, storage, _) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き", "推敲"]))
            .expect("作れる");
        let first = core.snapshot().task(task_id).expect("ある").steps()[0].id();
        core.move_current_position(first).expect("移せる");

        storage.set_failing(true);
        let outcome =
            core.switch_current_position(Some(InterruptionNote::new("書けないはず")), true);

        assert!(matches!(outcome, Err(CoreError::Storage(_))));
        assert_eq!(
            core.current_position().step_id(),
            Some(first),
            "現在地は動かない"
        );
        let snapshot = core.snapshot();
        let step = snapshot
            .task(task_id)
            .expect("ある")
            .step(first)
            .expect("ある");
        assert_eq!(step.interruption_note(), None, "メモも残らない");
        assert!(!step.is_completed(), "完了も付かない");
    }

    /// **移動先は同一タスク内の次のステップに限る。** 別タスクへは飛ばない
    /// (任意のステップへの移動は CAP-9 の開示面に属する)。
    #[test]
    fn a_switch_never_crosses_into_another_task() {
        let (core, _, _) = a_core();
        let first_task = core
            .create_task("一つ目", contents(&["a"]))
            .expect("作れる");
        core.create_task("二つ目", contents(&["b", "c"]))
            .expect("作れる");
        let only_step = core.snapshot().task(first_task).expect("ある").steps()[0].id();
        core.move_current_position(only_step).expect("移せる");

        let destination = core.switch_current_position(None, false).expect("成立する");

        assert_eq!(destination, None, "別タスクの先頭へは飛ばない");
        assert_eq!(core.current_position().task_id(), Some(first_task));
    }

    /// 完了済みのステップを読み飛ばさない。読み飛ばせば完了が移動先を決めることに
    /// なり、「完了を現在地から導出しない」の裏返しが起きる (FR-4 / AD-2)。
    #[test]
    fn the_destination_is_the_next_step_even_when_it_is_already_completed() {
        let (core, _, _) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き", "推敲", "投稿"]))
            .expect("作れる");
        let snapshot = core.snapshot();
        let task = snapshot.task(task_id).expect("ある");
        let (first, second) = (task.steps()[0].id(), task.steps()[1].id());
        core.declare_completion(second).expect("宣言できる");
        core.move_current_position(first).expect("移せる");

        let destination = core.switch_current_position(None, false).expect("成立する");

        assert_eq!(destination, Some(second));
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

    // --- 開示面からの切り替え (CAP-9 / FR-19) ---------------------------------

    /// 受け入れ条件「現在と異なるステップを選ぶ → 現在地が移り、履歴が 1 行増え、
    /// 『メモを書いたか』が偽である」。
    ///
    /// **両者が一つのコミットに載ることが本スライスの核心である** (AD-5)。別タスクへ
    /// 跨げることも同時に見る — これが解消する穴そのものである。
    #[test]
    fn selecting_a_step_moves_and_records_in_one_commit() {
        let (core, storage, _) = a_core();
        let origin = core
            .create_task("原稿", contents(&["下書き", "推敲"]))
            .expect("作れる");
        let other = core
            .create_task("買い物", contents(&["米", "味噌"]))
            .expect("作れる");
        let snapshot = core.snapshot();
        let departed = snapshot.task(origin).expect("ある").steps()[0].id();
        let chosen = snapshot.task(other).expect("ある").steps()[1].id();
        core.move_current_position(departed).expect("移せる");
        let before = storage.commits().len();

        let moved = core.select_step(chosen).expect("選べる");

        assert!(moved, "現在地が動いた");
        let commits = storage.commits();
        assert_eq!(
            commits.len(),
            before + 1,
            "現在地の移動と履歴の追記で 1 トランザクション (AD-5)"
        );
        let commit = commits.last().expect("ある");
        assert!(commit.task.is_none(), "タスクは書き直さない");
        assert_eq!(
            commit.current_position.and_then(|p| p.step_id()),
            Some(chosen),
            "現在地が同じコミットにある"
        );
        let record = commit
            .switch_record
            .as_ref()
            .expect("履歴も同じコミットにある");
        assert_eq!(record.departed_step_id(), departed, "離脱元を記録する");
        assert!(
            !record.note_written(),
            "機会を与えていないため記入は常に偽である (SM-C3)"
        );

        assert_eq!(core.current_position().step_id(), Some(chosen));
        assert_eq!(
            core.current_position().task_id(),
            Some(other),
            "タスクを跨いで移れる — これが CAP-7 との違いである"
        );
    }

    /// 受け入れ条件「同じステップを選ぶ → 切り替え履歴は増えていない」。
    #[test]
    fn selecting_the_current_step_writes_nothing() {
        let (core, storage, _) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き", "推敲"]))
            .expect("作れる");
        let first = core.snapshot().task(task_id).expect("ある").steps()[0].id();
        core.move_current_position(first).expect("移せる");
        let before = storage.commits();

        let moved = core.select_step(first).expect("選べる");

        assert!(!moved, "動いていない");
        assert_eq!(storage.commits(), before, "履歴も現在地も書かない");
    }

    /// 受け入れ条件「任意の移動 → 離脱側の中断メモも `completed_at` も変わっていない」。
    #[test]
    fn selecting_a_step_touches_neither_the_note_nor_the_completion() {
        let (core, _, _) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き", "推敲", "投稿"]))
            .expect("作れる");
        let snapshot = core.snapshot();
        let task = snapshot.task(task_id).expect("ある");
        let (first, third) = (task.steps()[0].id(), task.steps()[2].id());
        core.move_current_position(first).expect("移せる");
        core.set_interruption_note(first, Some(InterruptionNote::new("3 段落目まで")))
            .expect("メモを置ける");

        core.select_step(third).expect("選べる");

        let snapshot = core.snapshot();
        let departed = snapshot
            .task(task_id)
            .expect("ある")
            .step(first)
            .expect("ある");
        assert_eq!(
            departed.interruption_note().map(InterruptionNote::text),
            Some("3 段落目まで"),
            "離脱側のメモは変わらない"
        );
        assert!(!departed.is_completed(), "完了も付かない (FR-4 / AD-2)");
        assert!(
            !snapshot
                .task(task_id)
                .expect("ある")
                .step(third)
                .expect("ある")
                .is_completed(),
            "移動先の完了も動かない"
        );
    }

    /// **着手せずに作ったタスクへ、初めて到達する経路である。**
    ///
    /// **未着手**には離れる場所が無いため履歴は残らない。移動そのものは成立する。
    #[test]
    fn selecting_from_not_started_moves_without_a_record() {
        let (core, storage, _) = a_core();
        let task_id = core
            .create_task("着手せずに書き留めた", contents(&["一", "二"]))
            .expect("作れる");
        let second = core.snapshot().task(task_id).expect("ある").steps()[1].id();
        assert!(core.current_position().step_id().is_none(), "未着手である");

        let moved = core.select_step(second).expect("選べる");

        assert!(moved, "到達できる");
        assert_eq!(core.current_position().step_id(), Some(second));
        let commit = storage.commits().last().cloned().expect("ある");
        assert!(commit.current_position.is_some(), "現在地は書かれる");
        assert!(
            commit.switch_record.is_none(),
            "離脱元が無い移動は切り替えではない — 履歴を捏造しない"
        );
    }

    /// **一覧からの移動は連続作業時間をリセットしない** (AD-8 / FR-15)。
    ///
    /// `move_to` を `Active { activated_at: now }` の直接構築に置き換えると、⌘L で
    /// 渡り歩くたびに計時が振り出しに戻り、CAP-10 の休息介入が永久に発火しなくなる。
    /// 壊れても他のどの検査も落ちないため、ここで固定する
    /// (`a_switch_does_not_reset_the_work_clock` の双子)。
    #[test]
    fn selecting_a_step_does_not_reset_the_work_clock() {
        let (core, _, clock) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き", "推敲"]))
            .expect("作れる");
        let snapshot = core.snapshot();
        let task = snapshot.task(task_id).expect("ある");
        let (first, second) = (task.steps()[0].id(), task.steps()[1].id());
        core.move_current_position(first).expect("移せる");
        let started = clock.now();

        clock.advance(600_000);
        core.select_step(second).expect("選べる");

        assert!(core.current_position().is_active());
        assert_eq!(
            core.current_position().activated_at(),
            Some(started),
            "活性のまま移る限り、起点は据え置かれる (AD-8)"
        );
    }

    /// **休息中に選んだ行は再入である** — 非活性→活性の遷移であり、連続作業時間は
    /// そこから数え直される (AD-8 が名指す唯一のリセット契機)。
    ///
    /// 「現在地を移す」の意味を経路ごとに変えない。`move_current_position` も同じ値を
    /// 通る。
    #[test]
    fn selecting_a_step_while_inactive_takes_up_the_work_again() {
        let (core, storage, clock) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き", "推敲"]))
            .expect("作れる");
        let snapshot = core.snapshot();
        let task = snapshot.task(task_id).expect("ある");
        let (first, second) = (task.steps()[0].id(), task.steps()[1].id());
        core.move_current_position(first).expect("移せる");
        core.deactivate_current_position().expect("休息に入れる");

        clock.advance(600_000);
        let resumed = clock.now();
        let moved = core.select_step(second).expect("選べる");

        assert!(moved);
        assert!(core.current_position().is_active(), "再入である");
        assert_eq!(
            core.current_position().activated_at(),
            Some(resumed),
            "非活性→活性の遷移でのみ計時が数え直される (AD-8)"
        );
        assert!(
            storage
                .commits()
                .last()
                .expect("ある")
                .switch_record
                .is_some(),
            "離脱元があるため切り替えである"
        );
    }

    /// **休息中に自分の行を選んだ場合も再入である。** ただし離脱していないため履歴は
    /// 残さない。
    ///
    /// 判定を `step_id` だけで短絡すると、この一つの状態でだけ「選んでも何も起きない」
    /// が生まれる。`move_current_position` は現在地の値そのもので比べており、そちらと
    /// 食い違わせない。
    #[test]
    fn selecting_the_current_step_while_inactive_takes_up_the_work_without_a_record() {
        let (core, storage, clock) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き", "推敲"]))
            .expect("作れる");
        let first = core.snapshot().task(task_id).expect("ある").steps()[0].id();
        core.move_current_position(first).expect("移せる");
        core.deactivate_current_position().expect("休息に入れる");

        clock.advance(600_000);
        let resumed = clock.now();
        let moved = core.select_step(first).expect("選べる");

        assert!(moved, "値が変わったので書かれている");
        assert!(core.current_position().is_active());
        assert_eq!(core.current_position().step_id(), Some(first));
        assert_eq!(core.current_position().activated_at(), Some(resumed));
        let commit = storage.commits().last().cloned().expect("ある");
        assert!(commit.current_position.is_some());
        assert!(
            commit.switch_record.is_none(),
            "離れていないのだから切り替えではない — SM-C3 の分母を膨らませない"
        );
    }

    /// 知らない**ステップ**は panic ではなくエラーである。
    #[test]
    fn selecting_an_unknown_step_is_an_error() {
        let (core, storage, _) = a_core();
        let before = storage.commits();

        let outcome = core.select_step(StepId::new(Timestamp::from_unix_millis(0)));

        assert_eq!(outcome, Err(CoreError::Domain(DomainError::UnknownStep)));
        assert_eq!(storage.commits(), before, "何も書かれていない");
    }

    // --- 休息介入 (CAP-10 / FR-15) ---------------------------------------------

    use crate::domain::rest::{
        Intervention, InterventionChoice, RestSettings, GRACE_PERIOD_KEY, REST_THRESHOLD_KEY,
        TICK_INTERVAL_MILLIS,
    };
    use crate::domain::setting::Setting;

    const MINUTE: i64 = 60_000;

    /// **現在地**を第 1 **ステップ**へ置いたコアを作る。以降の刻みはここが起点である。
    fn a_core_at_work() -> (Core, Arc<RecordingStorage>, FixedClock) {
        let (core, storage, clock) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き", "推敲"]))
            .expect("作れる");
        let first = core.snapshot().task(task_id).expect("ある").steps()[0].id();
        core.move_current_position(first).expect("移せる");
        (core, storage, clock)
    }

    /// 刻みを 1 回進める。**時計も刻みの間隔だけ進める** — 進めなければ、次の刻みが
    /// 時計の飛びとしてスリープに見える。
    fn tick_once(core: &Core, clock: &FixedClock) -> bool {
        clock.advance(TICK_INTERVAL_MILLIS);
        core.tick().expect("刻める").raised
    }

    /// 指定の長さだけ、**実際の刻みの間隔で刻み続ける。** 途中で**介入**が発せられたら真。
    ///
    /// **時計だけを一気に進めてはならない。** 進めれば、それは刻みではなく時計の飛びで
    /// あり、[`rest::decide`] はスリープとして扱う — 計時の検査のつもりがスリープの
    /// 検査になる。
    fn tick_for(core: &Core, clock: &FixedClock, millis: i64) -> bool {
        let mut raised = false;
        let mut remaining = millis;
        while remaining > 0 {
            raised |= tick_once(core, clock);
            remaining -= TICK_INTERVAL_MILLIS;
        }
        raised
    }

    /// 受け入れ条件「現在地が活性で起点から 50 分、刻みが来る → 介入が発せられる」。
    #[test]
    fn the_threshold_raises_an_intervention_on_a_tick() {
        let (core, _, clock) = a_core_at_work();

        assert!(!tick_for(&core, &clock, 49 * MINUTE), "49 分では出ない");
        assert!(core.snapshot().intervention().is_none());

        assert!(tick_for(&core, &clock, MINUTE), "50 分で出る");
        assert!(core.snapshot().intervention().is_some());
    }

    /// I/O マトリクス「表示中の別の契機」— **後発は待つ。パネルは一つだけである** (AD-7)。
    #[test]
    fn only_one_intervention_is_raised_at_a_time() {
        let (core, _, clock) = a_core_at_work();
        assert!(tick_for(&core, &clock, 50 * MINUTE));
        let raised = core.snapshot().intervention().expect("出ている");

        for _ in 0..5 {
            assert!(!tick_once(&core, &clock), "表示中は後発が待つ");
        }
        assert_eq!(
            core.snapshot().intervention(),
            Some(raised),
            "表示中の介入は差し替わらない"
        );
    }

    /// 受け入れ条件「パネルで休息を選ぶ → 現在地は値を保ったまま `is_active` が偽」。
    #[test]
    fn choosing_rest_deactivates_the_current_position_without_losing_it() {
        let (core, storage, clock) = a_core_at_work();
        let before = core.current_position();
        assert!(tick_for(&core, &clock, 50 * MINUTE));
        let commits = storage.commits().len();

        assert!(core
            .answer_intervention(InterventionChoice::Rest)
            .expect("応答できる"));

        let position = core.current_position();
        assert!(!position.is_active(), "非活性になる");
        assert_eq!(position.step_id(), before.step_id(), "値は保たれる");
        assert_eq!(position.task_id(), before.task_id());
        assert_eq!(
            position.activated_at(),
            before.activated_at(),
            "起点は据え置かれる — 休息は計時を止めるのであって数え直すのではない"
        );
        assert!(core.snapshot().is_resting());
        assert!(core.snapshot().intervention().is_none(), "介入は閉じる");
        assert_eq!(storage.commits().len(), commits + 1, "1 トランザクション");
    }

    /// 受け入れ条件「休息中に 60 分経過 → 介入は発せられない」。
    #[test]
    fn no_intervention_is_raised_while_resting() {
        let (core, _, clock) = a_core_at_work();
        assert!(tick_for(&core, &clock, 50 * MINUTE));
        core.answer_intervention(InterventionChoice::Rest)
            .expect("応答できる");

        for _ in 0..6 {
            assert!(!tick_for(&core, &clock, 10 * MINUTE), "休息中は計数しない");
        }
        assert!(core.snapshot().intervention().is_none());
    }

    /// 受け入れ条件「休息中に終了を宣言 → 現在地が活性へ戻り、起点が更新される」。
    #[test]
    fn declaring_the_end_of_a_rest_takes_up_the_work_again() {
        let (core, _, clock) = a_core_at_work();
        let step_id = core.current_position().step_id();
        assert!(tick_for(&core, &clock, 50 * MINUTE));
        core.answer_intervention(InterventionChoice::Rest)
            .expect("応答できる");

        clock.advance(20 * MINUTE);
        let resumed_at = clock.now();
        assert!(core.end_rest().expect("終えられる"));

        let position = core.current_position();
        assert!(position.is_active());
        assert_eq!(position.step_id(), step_id, "戻る先は同じステップである");
        assert_eq!(
            position.activated_at(),
            Some(resumed_at),
            "非活性→活性の遷移でのみ起点が更新される (AD-8)"
        );
        assert!(!core.snapshot().is_resting());
    }

    /// **休息**中でなければ終了の宣言は何も書かない。
    #[test]
    fn declaring_the_end_of_a_rest_while_active_writes_nothing() {
        let (core, storage, _) = a_core_at_work();
        let commits = storage.commits().len();

        assert!(!core.end_rest().expect("失敗しない"));
        assert_eq!(storage.commits().len(), commits);
    }

    /// 受け入れ条件「猶予を選ぶ → 15 分後に再び現れる。現在地は活性のまま」。
    #[test]
    fn a_grace_period_postpones_the_intervention_without_ending_the_work() {
        let (core, _, clock) = a_core_at_work();
        assert!(tick_for(&core, &clock, 50 * MINUTE));

        assert!(core
            .answer_intervention(InterventionChoice::Grace)
            .expect("応答できる"));
        assert!(core.current_position().is_active(), "現在地は活性のまま");
        assert!(core.snapshot().intervention().is_none());

        assert!(!tick_for(&core, &clock, 14 * MINUTE), "まだ出ない");
        assert!(tick_for(&core, &clock, MINUTE), "15 分後に出直す");
    }

    /// I/O マトリクス「繰り返し猶予」— **上限で止まらない** (spec Design Notes)。
    #[test]
    fn a_grace_period_has_no_limit() {
        let (core, _, clock) = a_core_at_work();
        assert!(tick_for(&core, &clock, 50 * MINUTE));

        for round in 0..5 {
            core.answer_intervention(InterventionChoice::Grace)
                .expect("応答できる");
            assert!(
                tick_for(&core, &clock, 15 * MINUTE),
                "{round} 回目の猶予の後も出直す"
            );
        }
        assert!(core.current_position().is_active(), "猶予は休息に化けない");
    }

    /// 受け入れ条件「起点から 40 分で切り替え、さらに 10 分 → 介入が発せられる」。
    #[test]
    fn a_switch_does_not_postpone_the_intervention() {
        let (core, _, clock) = a_core_at_work();

        assert!(!tick_for(&core, &clock, 40 * MINUTE));
        core.switch_current_position(None, false)
            .expect("切り替えられる");

        assert!(
            tick_for(&core, &clock, 10 * MINUTE),
            "切り替えは起点を動かさない (AD-8)"
        );
    }

    /// 受け入れ条件「閾値を超えるスリープから復帰 → 計時はリセットされ、介入は出ない」。
    #[test]
    fn a_long_sleep_resets_the_work_clock_without_an_intervention() {
        let (core, _, clock) = a_core_at_work();
        assert!(!tick_for(&core, &clock, 40 * MINUTE));

        // 眠っている間、刻みは走らない。時計だけが飛ぶ。
        clock.advance(120 * MINUTE);
        assert!(
            !core.tick().expect("刻める").raised,
            "休息が取られたとみなす"
        );
        assert_eq!(
            core.current_position().activated_at(),
            Some(clock.now()),
            "計時はリセットされる"
        );

        assert!(!tick_for(&core, &clock, 49 * MINUTE), "数え直している");
        assert!(tick_for(&core, &clock, MINUTE));
    }

    /// I/O マトリクス「短いスリープ」— スリープ分は加算せず、計時は継続する。
    #[test]
    fn a_short_sleep_is_discounted_from_the_work_clock() {
        let (core, _, clock) = a_core_at_work();
        let started = core.current_position().activated_at().expect("起点がある");
        assert!(!tick_for(&core, &clock, 40 * MINUTE));

        clock.advance(10 * MINUTE);
        assert!(!core.tick().expect("刻める").raised);
        assert_eq!(
            core.current_position().activated_at(),
            Some(Timestamp::from_unix_millis(
                started.unix_millis() + 10 * MINUTE - TICK_INTERVAL_MILLIS
            )),
            "眠った分だけ起点をずらす — リセットではない"
        );

        assert!(tick_for(&core, &clock, 10 * MINUTE), "計時は継続する");
    }

    /// **待機しているだけの刻みは何も書かない。**
    ///
    /// 書けば、10 秒ごとに DB へ書き込む常駐になる。
    #[test]
    fn an_idle_tick_writes_nothing() {
        let (core, storage, clock) = a_core_at_work();
        let commits = storage.commits().len();

        for _ in 0..10 {
            assert!(!tick_once(&core, &clock));
        }
        assert_eq!(storage.commits().len(), commits);
    }

    /// I/O マトリクス「未着手」— 計時も介入も起きない。
    #[test]
    fn nothing_is_counted_before_the_first_step() {
        let (core, storage, clock) = a_core();
        for _ in 0..10 {
            clock.advance(10 * MINUTE);
            assert!(!core.tick().expect("刻める").raised);
        }
        assert!(storage.commits().is_empty());
    }

    /// I/O マトリクス「休息に入る」— **書き込みが失敗したら状態を変えず、介入も閉じない。**
    #[test]
    fn a_failed_rest_changes_nothing_and_keeps_the_intervention() {
        let (core, storage, clock) = a_core_at_work();
        assert!(tick_for(&core, &clock, 50 * MINUTE));
        let before = core.current_position();

        storage.set_failing(true);
        let outcome = core.answer_intervention(InterventionChoice::Rest);

        assert!(matches!(outcome, Err(CoreError::Storage(_))));
        assert_eq!(core.current_position(), before, "現在地は動かない");
        assert!(
            core.snapshot().intervention().is_some(),
            "選んだはずの休息が残らないまま画面から消えない"
        );
    }

    /// **表示されていない介入には応答できない。** 二重発火や競り合った応答で
    /// **現在地**が二度動かない。
    #[test]
    fn answering_without_an_intervention_does_nothing() {
        let (core, storage, _) = a_core_at_work();
        let commits = storage.commits().len();

        assert!(!core
            .answer_intervention(InterventionChoice::Rest)
            .expect("失敗しない"));
        assert!(!core
            .answer_intervention(InterventionChoice::Grace)
            .expect("失敗しない"));
        assert!(core.current_position().is_active());
        assert_eq!(storage.commits().len(), commits);
    }

    /// **二度目の応答は何も起こさない。** ホットキーとクリックが同時に届いた場合である。
    #[test]
    fn a_second_answer_is_ignored() {
        let (core, _, clock) = a_core_at_work();
        assert!(tick_for(&core, &clock, 50 * MINUTE));

        assert!(core
            .answer_intervention(InterventionChoice::Rest)
            .expect("応答できる"));
        assert!(
            !core
                .answer_intervention(InterventionChoice::Grace)
                .expect("失敗しない"),
            "二つ目の応答は何も起こさない"
        );
        assert!(core.snapshot().is_resting(), "休息のままである");
    }

    /// **取り下げた介入は猶予を伴って出直す。** 出せなかった介入を刻みのたびに
    /// 出し直さない (AD-7)。
    #[test]
    fn a_withdrawn_intervention_comes_back_after_a_grace_period() {
        let (core, _, clock) = a_core_at_work();
        assert!(tick_for(&core, &clock, 50 * MINUTE));

        assert!(core.withdraw_intervention());
        assert!(core.snapshot().intervention().is_none());
        assert!(core.current_position().is_active(), "現在地には触れない");

        assert!(!tick_for(&core, &clock, 14 * MINUTE), "すぐには出直さない");
        assert!(tick_for(&core, &clock, MINUTE));
    }

    /// 表示されていない**介入**は取り下げられない。
    #[test]
    fn withdrawing_without_an_intervention_does_nothing() {
        let (core, _, _) = a_core_at_work();
        assert!(!core.withdraw_intervention());
    }

    /// **起動時は常に非表示である** (AD-2)。表示状態も猶予も永続化されない。
    #[test]
    fn a_restored_core_shows_no_intervention() {
        let (core, _, _) = a_core_at_work();
        assert!(core.snapshot().intervention().is_none());
    }

    /// **設定値は既定値を上書きし、書き込みは往復する** (AD-11)。
    #[test]
    fn a_stored_setting_overrides_the_constant() {
        let (core, storage, _) = a_core();
        assert_eq!(core.snapshot().rest_settings(), RestSettings::default());

        core.store_setting(Setting::new(REST_THRESHOLD_KEY, "60"))
            .expect("書ける");
        core.store_setting(Setting::new(GRACE_PERIOD_KEY, "30"))
            .expect("書ける");

        assert_eq!(core.snapshot().rest_settings().rest_threshold_seconds(), 60);
        assert_eq!(core.snapshot().rest_settings().grace_period_seconds(), 30);
        let written: Vec<Setting> = storage
            .commits()
            .iter()
            .flat_map(|commit| commit.settings.clone())
            .collect();
        assert_eq!(written.len(), 2, "設定値は 1 操作 = 1 コミットで書かれる");
    }

    /// 同じ鍵を二度書いても行が二つにならない。
    #[test]
    fn storing_the_same_key_twice_replaces_it() {
        let (core, _, _) = a_core();
        core.store_setting(Setting::new(REST_THRESHOLD_KEY, "60"))
            .expect("書ける");
        core.store_setting(Setting::new(REST_THRESHOLD_KEY, "120"))
            .expect("書ける");

        assert_eq!(core.snapshot().settings().len(), 1);
        assert_eq!(
            core.snapshot().rest_settings().rest_threshold_seconds(),
            120
        );
    }

    /// 書き込みが失敗したらメモリ上の設定も変えない。
    #[test]
    fn a_failed_setting_write_leaves_memory_untouched() {
        let (core, storage, _) = a_core();
        storage.set_failing(true);

        let outcome = core.store_setting(Setting::new(REST_THRESHOLD_KEY, "60"));

        assert!(matches!(outcome, Err(CoreError::Storage(_))));
        assert!(core.snapshot().settings().is_empty());
        assert_eq!(core.snapshot().rest_settings(), RestSettings::default());
    }

    /// **短くした閾値が実際に効く。** 手動確認 (spec Verification) の前提である。
    #[test]
    fn a_shortened_threshold_actually_fires_sooner() {
        let (core, _, clock) = a_core_at_work();
        core.store_setting(Setting::new(REST_THRESHOLD_KEY, "60"))
            .expect("書ける");

        assert!(tick_for(&core, &clock, MINUTE), "1 分で出る");
    }

    /// **休息への遷移と切り替えのコミットが交錯しない** (AD-5)。
    ///
    /// 錠がアグリゲート単位に割れていれば、「現在地が活性のまま休息中」という状態が
    /// 作れてしまう。刻み・応答・切り替えをスレッドを跨いで叩き、最終状態が必ず
    /// 一貫していることを見る。
    #[test]
    fn resting_and_switching_never_interleave() {
        let storage = RecordingStorage::shared();
        let clock = FixedClock::at(1_789_000_000_000);
        let core = Arc::new(core_with(Arc::clone(&storage), clock.clone()));
        let task_id = core
            .create_task("原稿", contents(&["一", "二", "三"]))
            .expect("作れる");
        let first = core.snapshot().task(task_id).expect("ある").steps()[0].id();
        core.move_current_position(first).expect("移せる");

        let handles: Vec<_> = (0..4)
            .map(|worker| {
                let core = Arc::clone(&core);
                let clock = clock.clone();
                std::thread::spawn(move || {
                    for _ in 0..50 {
                        match worker % 4 {
                            0 => {
                                clock.advance(TICK_INTERVAL_MILLIS);
                                let _ = core.tick();
                            }
                            1 => {
                                let _ = core.answer_intervention(InterventionChoice::Rest);
                            }
                            2 => {
                                let _ = core.end_rest();
                            }
                            _ => {
                                let _ = core.switch_current_position(None, false);
                            }
                        }
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("スレッドは panic しない");
        }

        let snapshot = core.snapshot();
        let position = snapshot.current_position();
        assert!(position.step_id().is_some(), "現在地は値を持ったままである");
        assert_eq!(
            snapshot.is_resting(),
            !position.is_active(),
            "「活性のまま休息中」という状態が作られていない"
        );
        assert!(
            !(snapshot.intervention().is_some() && snapshot.is_resting()),
            "休息に入った以上、介入は残らない"
        );
    }

    /// **表示中の介入は一つの値である。** 型が集合を持たないことの確認 (AD-7)。
    #[test]
    fn the_intervention_is_a_single_value() {
        let (core, _, clock) = a_core_at_work();
        assert!(tick_for(&core, &clock, 50 * MINUTE));
        let raised: Option<Intervention> = core.snapshot().intervention();
        assert!(raised.is_some());
        assert_eq!(raised.map(|i| i.at()), Some(clock.now()));
    }

    /// 書き込みが失敗した刻みは、次の刻みでやり直せる。
    ///
    /// **`last_tick` を進めてしまうと、差し引けなかったスリープが二度と補正されない。**
    #[test]
    fn a_failed_tick_is_retried_on_the_next_one() {
        let (core, storage, clock) = a_core_at_work();
        let started = core.current_position().activated_at().expect("起点がある");

        clock.advance(10 * MINUTE);
        storage.set_failing(true);
        assert!(matches!(core.tick(), Err(CoreError::Storage(_))));
        assert_eq!(
            core.current_position().activated_at(),
            Some(started),
            "書けなかった補正をメモリにだけ残さない"
        );

        storage.set_failing(false);
        core.tick().expect("次の刻みは通る");
        assert_ne!(
            core.current_position().activated_at(),
            Some(started),
            "同じ飛びを見てやり直せている"
        );
    }

    /// 書き込みが失敗したなら**現在地**は動かない。
    ///
    /// 動かせば、書けなかった移動がメモリにだけ残り、次の起動で黙って戻る。
    #[test]
    fn a_failed_selection_leaves_the_current_position_unchanged() {
        let (core, storage, _) = a_core();
        let task_id = core
            .create_task("原稿", contents(&["下書き", "推敲"]))
            .expect("作れる");
        let snapshot = core.snapshot();
        let task = snapshot.task(task_id).expect("ある");
        let (first, second) = (task.steps()[0].id(), task.steps()[1].id());
        core.move_current_position(first).expect("移せる");
        storage.set_failing(true);

        let outcome = core.select_step(second);

        assert!(matches!(outcome, Err(CoreError::Storage(_))));
        assert_eq!(
            core.current_position().step_id(),
            Some(first),
            "メモリ上の現在地も動いていない"
        );
    }
}
