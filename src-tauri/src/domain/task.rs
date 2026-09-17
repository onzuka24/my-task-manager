//! **タスク**と**ステップ** (CAP-4 / CAP-5)。
//!
//! 順序は `ordinal` が単独で負い、同一性は安定した ID が単独で負う。**現在地**は ID を
//! 指すため、途中への追記や分割で連番が再計算されても同じ作業単位を指し続ける (FR-5)。
//!
//! この層の操作はすべて不変条件を保ったまま状態を移す。外から `ordinal` を書き換える
//! 手段は無い — 連番は追記・分割の結果として導出されるものであり、入力ではない。

use std::fmt;

use uuid::Uuid;

use super::{DomainError, Timestamp};

/// **タスク**の ID。UUID v7 (スパイン「一貫性の規約」)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
pub struct TaskId(Uuid);

/// **ステップ**の ID。UUID v7。
///
/// **現在地**はこの ID を指す。`ordinal` を指さないのは FR-5 のためである。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
pub struct StepId(Uuid);

/// UUID v7 を、コアが所有する時計の読みから作る。
///
/// `Uuid::now_v7()` を使わないのは、それが内部で壁時計を読むためである — AD-8 の
/// 「計時はコアが所有する」を迂回し、テストから決定的に検証できなくなる。
/// カウンタを持たせず乱数部を最大に取るのは、順序を `ordinal` が単独で負うため
/// 生成順の単調性に依存しないからである (Design Notes)。
fn new_v7(now: Timestamp) -> Uuid {
    // UUID v7 の時刻欄は符号なしであり、epoch より前を表せない。**秒と小数部を同じ値から
    // 導く。** 秒だけを 0 で止めて小数部を `rem_euclid` で取ると、epoch の 1 ミリ秒前が
    // `+999ms` という未来の時刻欄になる。丸めるなら両方を epoch へ丸める。
    let millis = now.unix_millis().max(0);
    let seconds = millis / 1_000;
    let subsec_nanos = (millis % 1_000) * 1_000_000;
    #[allow(clippy::cast_sign_loss)]
    let timestamp = uuid::Timestamp::from_unix_time(seconds as u64, subsec_nanos as u32, 0, 0);
    Uuid::new_v7(timestamp)
}

macro_rules! id_newtype {
    ($name:ident) => {
        impl $name {
            /// 新しい ID を発行する。
            #[must_use]
            pub fn new(now: Timestamp) -> Self {
                Self(new_v7(now))
            }

            /// 永続化された文字列から復元する。
            ///
            /// # Errors
            ///
            /// UUID として読めない文字列のときエラーを返す。
            pub fn parse(text: &str) -> Result<Self, uuid::Error> {
                Uuid::parse_str(text).map(Self)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                // ハイフン付きの小文字。永続化される形でもある。
                write!(f, "{}", self.0.as_hyphenated())
            }
        }
    };
}

id_newtype!(TaskId);
id_newtype!(StepId);

/// **中断メモ** — **現在地**を離れる際に記録される、再開に必要な最小限の文脈。
///
/// **ステップ**に対して 0..1。本スライスが持つのはデータとしての欄と、分割時に前半へ
/// 帰属するという規則だけである。記録の機会と**切り替え**の儀式は CAP-7 に属する。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InterruptionNote(String);

impl InterruptionNote {
    /// 本文からメモを作る。
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// 本文。
    #[must_use]
    pub fn text(&self) -> &str {
        &self.0
    }
}

/// **ステップ** — **タスク**内の作業単位。
///
/// `ordinal` はタスク内の通し番号 (1..N) であり、追記・分割のたびに再計算される。
/// 外から書き換える手段を持たせないのは、連番と並び順が食い違う状態を作らないため。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    id: StepId,
    ordinal: u32,
    content: String,
    completed_at: Option<Timestamp>,
    interruption_note: Option<InterruptionNote>,
}

impl Step {
    /// ID。
    #[must_use]
    pub const fn id(&self) -> StepId {
        self.id
    }

    /// タスク内の通し番号 (1..N)。
    #[must_use]
    pub const fn ordinal(&self) -> u32 {
        self.ordinal
    }

    /// 内容。
    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }

    /// **完了**が宣言された時刻。宣言されていなければ `None`。
    #[must_use]
    pub const fn completed_at(&self) -> Option<Timestamp> {
        self.completed_at
    }

    /// **完了**が宣言されているか。
    #[must_use]
    pub const fn is_completed(&self) -> bool {
        self.completed_at.is_some()
    }

    /// **中断メモ**。
    #[must_use]
    pub fn interruption_note(&self) -> Option<&InterruptionNote> {
        self.interruption_note.as_ref()
    }

    /// 永続化された行から復元する。
    ///
    /// `ordinal` を受け取らないのは、連番が [`Task::rehydrate`] の並びから導出される
    /// ためである — DB 側の連番に欠番があっても復元後は 1..N に整う。
    #[must_use]
    pub fn rehydrate(
        id: StepId,
        content: String,
        completed_at: Option<Timestamp>,
        interruption_note: Option<InterruptionNote>,
    ) -> Self {
        Self {
            id,
            // 連番は Task::rehydrate が付け直す。
            ordinal: 0,
            content,
            completed_at,
            interruption_note,
        }
    }
}

/// **タスク** — 一つの達成対象。1 個以上の**ステップ**を順序付きで内包する。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    id: TaskId,
    title: String,
    steps: Vec<Step>,
}

impl Task {
    /// 題名と 1 個以上の**ステップ**の内容から**タスク**を作る (CAP-4)。
    ///
    /// `ordinal` は与えられた順に 1..N が振られる。
    ///
    /// # Errors
    ///
    /// **ステップ**が 1 個も無いとき [`DomainError::EmptyTask`]。**ステップ**を持たない
    /// **タスク**は作成できない (FR-4) — 作れてしまうと、**現在地**の指す先を持たない
    /// タスクが生まれ、CAP-9 の開示面に「何もできない行」が並ぶ。
    pub fn create(
        now: Timestamp,
        title: impl Into<String>,
        step_contents: Vec<String>,
    ) -> Result<Self, DomainError> {
        if step_contents.is_empty() {
            return Err(DomainError::EmptyTask);
        }

        let steps = step_contents
            .into_iter()
            .map(|content| Step {
                id: StepId::new(now),
                ordinal: 0,
                content,
                completed_at: None,
                interruption_note: None,
            })
            .collect();

        let mut task = Self {
            id: TaskId::new(now),
            title: title.into(),
            steps,
        };
        task.renumber();
        Ok(task)
    }

    /// 永続化された行から復元する。
    ///
    /// **ステップ**は与えられた順に 1..N が振り直される。呼び出し側 (ストレージ
    /// アダプタ) が `ordinal` 昇順で渡す責務を負う。
    ///
    /// # Errors
    ///
    /// **ステップ**が 1 個も無いとき [`DomainError::EmptyTask`]。DB がその状態を
    /// 持っているなら破損であり、黙って読み込んではならない。
    pub fn rehydrate(id: TaskId, title: String, steps: Vec<Step>) -> Result<Self, DomainError> {
        if steps.is_empty() {
            return Err(DomainError::EmptyTask);
        }
        let mut task = Self { id, title, steps };
        task.renumber();
        Ok(task)
    }

    /// ID。
    #[must_use]
    pub const fn id(&self) -> TaskId {
        self.id
    }

    /// 題名。
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// **ステップ**を `ordinal` 昇順で。
    #[must_use]
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// ID から**ステップ**を引く。
    #[must_use]
    pub fn step(&self, id: StepId) -> Option<&Step> {
        self.steps.iter().find(|step| step.id == id)
    }

    /// 指定の位置へ**ステップ**を追記する (CAP-5 / FR-5)。
    ///
    /// `ordinal` は挿入後にその**ステップ**が占める位置であり、`1..=N+1` を取る。
    /// 後続の連番は再計算されるが **ID は変わらない** — **現在地**が**現在地**より前へ
    /// の挿入でずれないのは、指しているのが連番ではなく ID だからである。
    ///
    /// # Errors
    ///
    /// `ordinal` が `1..=N+1` の外にあるとき [`DomainError::OrdinalOutOfRange`]。
    pub fn insert_step(
        &mut self,
        now: Timestamp,
        ordinal: u32,
        content: impl Into<String>,
    ) -> Result<StepId, DomainError> {
        let len = u32::try_from(self.steps.len()).unwrap_or(u32::MAX);
        if ordinal < 1 || ordinal > len.saturating_add(1) {
            return Err(DomainError::OrdinalOutOfRange);
        }

        let id = StepId::new(now);
        let index = (ordinal - 1) as usize;
        self.steps.insert(
            index,
            Step {
                id,
                ordinal: 0,
                content: content.into(),
                completed_at: None,
                interruption_note: None,
            },
        );
        self.renumber();
        Ok(id)
    }

    /// 末尾へ**ステップ**を追記する。
    ///
    /// # Errors
    ///
    /// 実際には失敗しないが、[`Self::insert_step`] と同じ契約を保つため `Result` を返す。
    pub fn append_step(
        &mut self,
        now: Timestamp,
        content: impl Into<String>,
    ) -> Result<StepId, DomainError> {
        let ordinal = u32::try_from(self.steps.len())
            .unwrap_or(u32::MAX)
            .saturating_add(1);
        self.insert_step(now, ordinal, content)
    }

    /// **ステップ**を二つに分割する (CAP-5 / FR-5)。
    ///
    /// **前半は元の ID と中断メモを保持し**、後半が新しい ID を得る。**現在地**が
    /// 分割後も前半を指し続けるのはこのためであり、呼び出し側が**現在地**を動かす
    /// 必要は無い。**中断メモ**が前半に帰属するのも同じ理由である — メモは「ここまで
    /// やった / 次はこれ」という、離脱した地点の文脈だからである。
    ///
    /// 戻り値は後半の ID。
    ///
    /// # Errors
    ///
    /// - 指定の**ステップ**が無いとき [`DomainError::UnknownStep`]
    /// - **完了**が宣言済みのとき [`DomainError::SplitCompleted`]。**完了**は
    ///   ユーザーの明示宣言のみで付与・取消されるため (FR-4 / AD-2)、分割後の前半・
    ///   後半のどちらに帰属するかをコアが決めることができない
    pub fn split_step(
        &mut self,
        now: Timestamp,
        step_id: StepId,
        first_content: impl Into<String>,
        second_content: impl Into<String>,
    ) -> Result<StepId, DomainError> {
        let index = self
            .steps
            .iter()
            .position(|step| step.id == step_id)
            .ok_or(DomainError::UnknownStep)?;

        if self.steps[index].is_completed() {
            return Err(DomainError::SplitCompleted);
        }

        // 前半は元の ID と中断メモをそのまま保つ。置き換えるのは本文だけである。
        self.steps[index].content = first_content.into();

        let second_id = StepId::new(now);
        self.steps.insert(
            index + 1,
            Step {
                id: second_id,
                ordinal: 0,
                content: second_content.into(),
                completed_at: None,
                interruption_note: None,
            },
        );
        self.renumber();
        Ok(second_id)
    }

    /// **完了**を宣言する (CAP-4 / FR-4)。
    ///
    /// 既に宣言済みなら時刻を書き換えない — 二度目の宣言で最初の宣言時刻が失われる
    /// ほうが、黙って無視されるより悪い。
    ///
    /// # Errors
    ///
    /// 指定の**ステップ**が無いとき [`DomainError::UnknownStep`]。
    pub fn declare_completion(
        &mut self,
        now: Timestamp,
        step_id: StepId,
    ) -> Result<(), DomainError> {
        let step = self.step_mut(step_id)?;
        if step.completed_at.is_none() {
            step.completed_at = Some(now);
        }
        Ok(())
    }

    /// **完了**の宣言を取り消す (CAP-4 / FR-4)。
    ///
    /// **現在地**の移動では決して呼ばれない。完了の付与も取消もユーザーの明示宣言
    /// のみである (AD-2)。
    ///
    /// # Errors
    ///
    /// 指定の**ステップ**が無いとき [`DomainError::UnknownStep`]。
    pub fn revoke_completion(&mut self, step_id: StepId) -> Result<(), DomainError> {
        self.step_mut(step_id)?.completed_at = None;
        Ok(())
    }

    /// **中断メモ**の欄を設定する。
    ///
    /// 本スライスはデータとしての欄しか持たない。いつ記録の機会を提示するか、上書き
    /// 前の内容をどう見せるかは**切り替え**の儀式であり CAP-7 に属する。
    ///
    /// # Errors
    ///
    /// 指定の**ステップ**が無いとき [`DomainError::UnknownStep`]。
    pub fn set_interruption_note(
        &mut self,
        step_id: StepId,
        note: Option<InterruptionNote>,
    ) -> Result<(), DomainError> {
        self.step_mut(step_id)?.interruption_note = note;
        Ok(())
    }

    fn step_mut(&mut self, id: StepId) -> Result<&mut Step, DomainError> {
        self.steps
            .iter_mut()
            .find(|step| step.id == id)
            .ok_or(DomainError::UnknownStep)
    }

    /// 連番を並び順から付け直す。**ID には触れない。**
    fn renumber(&mut self) {
        for (index, step) in self.steps.iter_mut().enumerate() {
            step.ordinal = u32::try_from(index + 1).unwrap_or(u32::MAX);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Timestamp = Timestamp::from_unix_millis(1_789_000_000_000);

    fn contents(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    fn a_task() -> Task {
        Task::create(NOW, "原稿を仕上げる", contents(&["下書き", "推敲", "投稿"]))
            .expect("ステップがあるので作成できる")
    }

    /// I/O マトリクス「タスク作成」— ordinal 1..N で生成される。
    #[test]
    fn a_task_is_created_with_ordinals_from_one() {
        let task = a_task();
        assert_eq!(
            task.steps().iter().map(Step::ordinal).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(task.steps()[0].content(), "下書き");
        assert!(task.steps().iter().all(|step| !step.is_completed()));
    }

    /// I/O マトリクス「ステップ 0 個で作成」— 拒否される。
    #[test]
    fn a_task_without_steps_is_refused() {
        assert_eq!(
            Task::create(NOW, "題名だけ", Vec::new()),
            Err(DomainError::EmptyTask)
        );
    }

    /// 同時刻に発行しても ID は衝突しない。衝突すれば現在地が別のステップを指す。
    #[test]
    fn ids_issued_at_the_same_instant_are_distinct() {
        let task = a_task();
        let ids: std::collections::HashSet<_> = task.steps().iter().map(Step::id).collect();
        assert_eq!(ids.len(), task.steps().len());
    }

    /// ID は UUID v7 であり、時刻部はコアの時計の読みから来る (AD-8)。
    #[test]
    fn an_id_is_a_v7_uuid_stamped_by_the_core_clock() {
        let task = Task::create(NOW, "題名", contents(&["一手"])).expect("作れる");
        let text = task.id().to_string();
        let uuid = Uuid::parse_str(&text).expect("UUID として読める");
        assert_eq!(uuid.get_version_num(), 7);

        let (seconds, nanos) = uuid.get_timestamp().expect("v7 は時刻を持つ").to_unix();
        let millis = i64::try_from(seconds).expect("秒は i64 に収まる") * 1_000
            + i64::from(nanos) / 1_000_000;
        assert_eq!(
            millis,
            NOW.unix_millis(),
            "壁時計ではなく渡された時刻を使う"
        );
    }

    /// epoch より前の時刻から発行しても、時刻欄が未来へ跳ねない。
    ///
    /// 秒を 0 で止めながら小数部を剰余で取ると、epoch の 1 ミリ秒前が `+999ms` になる。
    #[test]
    fn an_id_from_before_the_epoch_does_not_jump_forward() {
        let id = StepId::new(Timestamp::from_unix_millis(-1));
        let uuid = Uuid::parse_str(&id.to_string()).expect("UUID として読める");
        let (seconds, nanos) = uuid.get_timestamp().expect("v7 は時刻を持つ").to_unix();
        assert_eq!((seconds, nanos), (0, 0), "epoch より前は epoch へ丸める");
    }

    /// I/O マトリクス「完了の宣言」— `completed_at` が設定される。
    #[test]
    fn a_completion_is_recorded_at_the_declared_instant() {
        let mut task = a_task();
        let first = task.steps()[0].id();
        task.declare_completion(NOW, first).expect("宣言できる");
        assert_eq!(task.step(first).expect("ある").completed_at(), Some(NOW));
    }

    /// 二度目の宣言で最初の宣言時刻を失わない。
    #[test]
    fn a_second_declaration_keeps_the_first_instant() {
        let mut task = a_task();
        let first = task.steps()[0].id();
        task.declare_completion(NOW, first).expect("宣言できる");
        let later = Timestamp::from_unix_millis(NOW.unix_millis() + 60_000);
        task.declare_completion(later, first)
            .expect("再度宣言できる");
        assert_eq!(task.step(first).expect("ある").completed_at(), Some(NOW));
    }

    /// I/O マトリクス「完了の取り消し」— `None` に戻る。
    #[test]
    fn a_completion_can_be_revoked_explicitly() {
        let mut task = a_task();
        let first = task.steps()[0].id();
        task.declare_completion(NOW, first).expect("宣言できる");
        task.revoke_completion(first).expect("取り消せる");
        assert_eq!(task.step(first).expect("ある").completed_at(), None);
    }

    /// I/O マトリクス「途中へのステップ追記」— 後続の ordinal が再計算されても
    /// **ID は変わらない**。現在地が同一の作業単位を指し続ける根拠そのもの (FR-5)。
    #[test]
    fn inserting_a_step_renumbers_without_changing_ids() {
        let mut task = a_task();
        let before: Vec<StepId> = task.steps().iter().map(Step::id).collect();
        let third_id = before[2];

        let inserted = task
            .insert_step(NOW, 1, "資料を集める")
            .expect("挿入できる");

        assert_eq!(
            task.steps().iter().map(Step::ordinal).collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
        assert_eq!(task.steps()[0].id(), inserted);
        assert_eq!(
            task.steps()[1..].iter().map(Step::id).collect::<Vec<_>>(),
            before,
            "既存ステップの ID は挿入で変わらない"
        );
        // 連番は 3 → 4 に変わったが、同じ ID で引ける。
        assert_eq!(task.step(third_id).expect("ある").ordinal(), 4);
    }

    /// 末尾への追記。
    #[test]
    fn a_step_can_be_appended_to_the_end() {
        let mut task = a_task();
        let appended = task.append_step(NOW, "見直す").expect("追記できる");
        assert_eq!(task.steps().last().expect("ある").id(), appended);
        assert_eq!(task.steps().last().expect("ある").ordinal(), 4);
    }

    /// 範囲外の位置は拒む。0 を許すと連番の起点が 1 でなくなる。
    #[test]
    fn an_out_of_range_position_is_refused() {
        let mut task = a_task();
        assert_eq!(
            task.insert_step(NOW, 0, "x"),
            Err(DomainError::OrdinalOutOfRange)
        );
        assert_eq!(
            task.insert_step(NOW, 5, "x"),
            Err(DomainError::OrdinalOutOfRange)
        );
        // N+1 (末尾への追記) は範囲内である。
        assert!(task.insert_step(NOW, 4, "x").is_ok());
    }

    /// I/O マトリクス「ステップの分割」— 前半は元の ID と中断メモを保持し、
    /// 後半が新しい ID を得る。
    #[test]
    fn a_split_keeps_the_original_id_and_note_on_the_first_half() {
        let mut task = a_task();
        let second = task.steps()[1].id();
        task.set_interruption_note(second, Some(InterruptionNote::new("3 段落目まで見た")))
            .expect("メモを置ける");

        let created = task
            .split_step(NOW, second, "前半を推敲", "後半を推敲")
            .expect("未完了なので分割できる");

        assert_eq!(
            task.steps().iter().map(Step::ordinal).collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
        let first_half = &task.steps()[1];
        assert_eq!(first_half.id(), second, "前半は元の ID を保つ");
        assert_eq!(first_half.content(), "前半を推敲");
        assert_eq!(
            first_half.interruption_note().map(InterruptionNote::text),
            Some("3 段落目まで見た"),
            "中断メモは前半に帰属する (FR-5)"
        );

        let second_half = &task.steps()[2];
        assert_eq!(second_half.id(), created);
        assert_ne!(second_half.id(), second, "後半は新しい ID を得る");
        assert_eq!(second_half.content(), "後半を推敲");
        assert_eq!(second_half.interruption_note(), None);
        assert!(!second_half.is_completed());
    }

    /// I/O マトリクス「完了済みステップの分割」— 拒否される。
    #[test]
    fn splitting_a_completed_step_is_refused() {
        let mut task = a_task();
        let first = task.steps()[0].id();
        task.declare_completion(NOW, first).expect("宣言できる");
        assert_eq!(
            task.split_step(NOW, first, "前", "後"),
            Err(DomainError::SplitCompleted)
        );
        assert_eq!(task.steps().len(), 3, "拒否された分割は何も変えない");
    }

    /// 存在しないステップへの操作は panic せず `Err` を返す。
    #[test]
    fn an_unknown_step_is_an_error_not_a_panic() {
        let mut task = a_task();
        let stranger = StepId::new(NOW);
        assert_eq!(
            task.declare_completion(NOW, stranger),
            Err(DomainError::UnknownStep)
        );
        assert_eq!(
            task.revoke_completion(stranger),
            Err(DomainError::UnknownStep)
        );
        assert_eq!(
            task.set_interruption_note(stranger, None),
            Err(DomainError::UnknownStep)
        );
        assert_eq!(
            task.split_step(NOW, stranger, "前", "後"),
            Err(DomainError::UnknownStep)
        );
    }

    /// 復元は与えられた並びから連番を付け直す。DB 側に欠番があっても 1..N に整う。
    #[test]
    fn rehydration_renumbers_from_the_given_order() {
        let id = TaskId::new(NOW);
        let steps = vec![
            Step::rehydrate(StepId::new(NOW), "一".to_string(), None, None),
            Step::rehydrate(StepId::new(NOW), "二".to_string(), None, None),
        ];
        let task = Task::rehydrate(id, "題名".to_string(), steps).expect("復元できる");
        assert_eq!(
            task.steps().iter().map(Step::ordinal).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    /// ステップを持たないタスクを DB から読んだら破損である。
    #[test]
    fn rehydrating_a_task_without_steps_is_refused() {
        assert_eq!(
            Task::rehydrate(TaskId::new(NOW), "題名".to_string(), Vec::new()),
            Err(DomainError::EmptyTask)
        );
    }

    /// ID は往復する。永続化の形が壊れれば現在地の復元が壊れる。
    #[test]
    fn an_id_round_trips_through_its_text_form() {
        let id = StepId::new(NOW);
        assert_eq!(StepId::parse(&id.to_string()).expect("読める"), id);
        let task_id = TaskId::new(NOW);
        assert_eq!(
            TaskId::parse(&task_id.to_string()).expect("読める"),
            task_id
        );
        assert!(StepId::parse("not-a-uuid").is_err());
    }
}
