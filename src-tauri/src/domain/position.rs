//! **現在地** (CAP-6 / FR-6)。
//!
//! 「どの**タスク**の第何**ステップ**にいるか」を指す唯一のポインタである。
//!
//! 一意性は型で担保する。[`CurrentPosition`] は値を一つしか持てない enum であり、
//! コア状態はこれを **1 個のフィールドとして** 保持する ([`super::state::CoreState`])。
//! 集合として持たないため、「二つの現在地」を表現する値が存在しない — 新たな
//! **ステップ**が**現在地**になった時点で直前が解除されるのは、規律ではなく型の帰結
//! である。
//!
//! 値を持たない状態は [`CurrentPosition::NotStarted`] だけである。**休息**中は値を
//! 保持したまま非活性となる (FR-6)。

use super::task::{StepId, TaskId};
use super::Timestamp;

/// **現在地**。システム全体で同時に一つ。
///
/// **活性** / **非活性**は真偽値のフィールドではなく、`Active` / `Inactive` という変種
/// そのものとして表す。用語集の識別子対応表が 活性 / 非活性 に `Active` / `Inactive` を
/// 割り当てており、コード上の識別子は表に 1:1 で従う (AD-10)。永続化側の列はスパインの
/// ERD どおり `is_active` のままである。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum CurrentPosition {
    /// **未着手** — まだ一度も**現在地**が置かれていない。**値を持たない唯一の状態**。
    NotStarted,
    /// **活性** — **ステップ**を指し、**連続作業時間**が進んでいる。
    Active {
        /// 指している**ステップ**が属する**タスク**。
        task_id: TaskId,
        /// 指している**ステップ**。`ordinal` ではなく ID を指す (FR-5)。
        step_id: StepId,
        /// **連続作業時間**の起点 (AD-8)。**非活性**から**活性**へ遷移した時点でのみ
        /// 更新される。
        activated_at: Timestamp,
    },
    /// **非活性** — **休息**中。**値は保持され**、計時だけが止まる (FR-6)。
    Inactive {
        /// 指している**ステップ**が属する**タスク**。
        task_id: TaskId,
        /// 指している**ステップ**。
        step_id: StepId,
        /// 直近に**活性**となった時点 (AD-8)。復帰するまで据え置かれる。
        activated_at: Timestamp,
    },
}

impl CurrentPosition {
    /// 指している**タスク**。**未着手**なら `None`。
    #[must_use]
    pub const fn task_id(&self) -> Option<TaskId> {
        match self {
            Self::NotStarted => None,
            Self::Active { task_id, .. } | Self::Inactive { task_id, .. } => Some(*task_id),
        }
    }

    /// 指している**ステップ**。**未着手**なら `None`。
    #[must_use]
    pub const fn step_id(&self) -> Option<StepId> {
        match self {
            Self::NotStarted => None,
            Self::Active { step_id, .. } | Self::Inactive { step_id, .. } => Some(*step_id),
        }
    }

    /// **活性**か。**未着手**は活性ではない。
    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(self, Self::Active { .. })
    }

    /// **連続作業時間**の起点。**未着手**なら `None`。
    #[must_use]
    pub const fn activated_at(&self) -> Option<Timestamp> {
        match self {
            Self::NotStarted => None,
            Self::Active { activated_at, .. } | Self::Inactive { activated_at, .. } => {
                Some(*activated_at)
            }
        }
    }

    /// **現在地**を指定の**ステップ**へ移す。
    ///
    /// 移した先は必ず**活性**である。直前の**現在地**は、値が一つしか無いため
    /// 移した時点で解除される。
    ///
    /// **`activated_at` の扱いが本メソッドの核心である (AD-8)。**
    /// - 直前が**活性**だった → **更新しない**。**切り替え**では**連続作業時間**を
    ///   リセットしないため (FR-15 / AD-8)。リセットしてしまうと、切り替えを繰り返す
    ///   ほど休息介入が遠のく — 最も休息が要る使い方でツールが黙る。
    /// - 直前が**非活性**または**未着手**だった → `now` に更新する。**非活性**から
    ///   **活性**への遷移が**連続作業時間**の唯一のリセット契機である。
    #[must_use]
    pub const fn move_to(self, task_id: TaskId, step_id: StepId, now: Timestamp) -> Self {
        let activated_at = match self {
            Self::Active { activated_at, .. } => activated_at,
            Self::NotStarted | Self::Inactive { .. } => now,
        };
        Self::Active {
            task_id,
            step_id,
            activated_at,
        }
    }

    /// **非活性**にする (**休息**へ入るときなど)。**値は保持される** (FR-6)。
    ///
    /// **未着手**は保持する値を持たないため何も起きない。
    ///
    /// 休息の計時そのもの (閾値・介入) は CAP-10 に属する。ここにあるのは**現在地**の
    /// 状態遷移だけである。
    #[must_use]
    pub const fn deactivate(self) -> Self {
        match self {
            Self::NotStarted | Self::Inactive { .. } => self,
            Self::Active {
                task_id,
                step_id,
                activated_at,
            } => Self::Inactive {
                task_id,
                step_id,
                activated_at,
            },
        }
    }

    /// **活性**にする (**休息**からの復帰など)。
    ///
    /// **非活性**からの遷移でのみ `activated_at` を `now` に更新する。既に**活性**なら
    /// 何も変えない — 重ねて呼ばれるだけで**連続作業時間**が伸び続けるのを防ぐ。
    /// **未着手**は活性化する対象を持たないため何も起きない。
    #[must_use]
    pub const fn activate(self, now: Timestamp) -> Self {
        match self {
            Self::NotStarted | Self::Active { .. } => self,
            Self::Inactive {
                task_id, step_id, ..
            } => Self::Active {
                task_id,
                step_id,
                activated_at: now,
            },
        }
    }

    /// 永続化された行から復元する。
    #[must_use]
    pub const fn rehydrate(
        task_id: TaskId,
        step_id: StepId,
        is_active: bool,
        activated_at: Timestamp,
    ) -> Self {
        if is_active {
            Self::Active {
                task_id,
                step_id,
                activated_at,
            }
        } else {
            Self::Inactive {
                task_id,
                step_id,
                activated_at,
            }
        }
    }
}

impl Default for CurrentPosition {
    /// 何も登録されていない起動直後は**未着手**である (I/O マトリクス「初回起動」)。
    fn default() -> Self {
        Self::NotStarted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: Timestamp = Timestamp::from_unix_millis(1_789_000_000_000);
    const T1: Timestamp = Timestamp::from_unix_millis(1_789_000_060_000);
    const T2: Timestamp = Timestamp::from_unix_millis(1_789_000_120_000);

    fn ids() -> (TaskId, StepId, StepId) {
        (TaskId::new(T0), StepId::new(T0), StepId::new(T0))
    }

    /// I/O マトリクス「初回起動」— 何も無ければ `NotStarted`。
    #[test]
    fn nothing_registered_means_not_started() {
        let position = CurrentPosition::default();
        assert_eq!(position, CurrentPosition::NotStarted);
        assert_eq!(position.step_id(), None);
        assert!(!position.is_active());
        assert_eq!(position.activated_at(), None);
    }

    /// **未着手**から移ると活性となり、`activated_at` が起点となる (AD-8)。
    #[test]
    fn moving_from_not_started_starts_the_clock() {
        let (task, step, _) = ids();
        let position = CurrentPosition::NotStarted.move_to(task, step, T0);
        assert_eq!(position.task_id(), Some(task));
        assert_eq!(position.step_id(), Some(step));
        assert!(position.is_active());
        assert_eq!(position.activated_at(), Some(T0));
    }

    /// I/O マトリクス「現在地の移動」— 直前の現在地は解除され、同時に二つ存在しない。
    ///
    /// 値が一つしか無いことを、移動後に前の**ステップ**を指していないことで確かめる。
    #[test]
    fn only_one_position_exists_at_a_time() {
        let (task, first, second) = ids();
        let position = CurrentPosition::NotStarted
            .move_to(task, first, T0)
            .move_to(task, second, T1);
        assert_eq!(position.step_id(), Some(second));
        assert_ne!(position.step_id(), Some(first));
    }

    /// I/O マトリクス「活性のまま再指定」— `activated_at` は更新されない。
    ///
    /// **切り替えでは連続作業時間をリセットしない** (AD-8)。ここが逆になると、
    /// 切り替えを繰り返すほど休息介入が遠のく。
    #[test]
    fn a_switch_while_active_does_not_reset_the_work_clock() {
        let (task, first, second) = ids();
        let position = CurrentPosition::NotStarted
            .move_to(task, first, T0)
            .move_to(task, second, T1);
        assert_eq!(position.activated_at(), Some(T0));
        assert!(position.is_active());
    }

    /// 非活性でも値は保持される (FR-6)。休息中に現在地を見失わない。
    #[test]
    fn deactivating_keeps_the_value() {
        let (task, step, _) = ids();
        let position = CurrentPosition::NotStarted
            .move_to(task, step, T0)
            .deactivate();
        assert!(!position.is_active());
        assert_eq!(position.step_id(), Some(step));
        assert_eq!(position.task_id(), Some(task));
        assert_eq!(position.activated_at(), Some(T0));
    }

    /// I/O マトリクス「非活性からの復帰」— `activated_at` が更新される。
    #[test]
    fn activating_from_inactive_restarts_the_work_clock() {
        let (task, step, _) = ids();
        let position = CurrentPosition::NotStarted
            .move_to(task, step, T0)
            .deactivate()
            .activate(T2);
        assert!(position.is_active());
        assert_eq!(position.activated_at(), Some(T2));
    }

    /// 非活性からの**移動**も、非活性→活性の遷移である。
    #[test]
    fn moving_while_inactive_restarts_the_work_clock() {
        let (task, first, second) = ids();
        let position = CurrentPosition::NotStarted
            .move_to(task, first, T0)
            .deactivate()
            .move_to(task, second, T2);
        assert!(position.is_active());
        assert_eq!(position.activated_at(), Some(T2));
    }

    /// 既に活性なら活性化は何も変えない。重ねて呼んで時間が伸びない。
    #[test]
    fn activating_an_already_active_position_changes_nothing() {
        let (task, step, _) = ids();
        let active = CurrentPosition::NotStarted.move_to(task, step, T0);
        assert_eq!(active.activate(T2), active);
    }

    /// 未着手は活性化も非活性化もできない。値を持たないためである。
    #[test]
    fn not_started_has_nothing_to_activate() {
        assert_eq!(
            CurrentPosition::NotStarted.activate(T0),
            CurrentPosition::NotStarted
        );
        assert_eq!(
            CurrentPosition::NotStarted.deactivate(),
            CurrentPosition::NotStarted
        );
    }

    /// 活性 / 非活性が `Active` / `Inactive` として表されている (AD-10)。
    ///
    /// 真偽値のフィールドだけで表すと、用語集の識別子がコードのどこにも現れない。
    #[test]
    fn activity_is_modelled_as_active_and_inactive() {
        let (task, step, _) = ids();
        let active = CurrentPosition::NotStarted.move_to(task, step, T0);
        assert!(matches!(active, CurrentPosition::Active { .. }));
        assert!(matches!(
            active.deactivate(),
            CurrentPosition::Inactive { .. }
        ));
        assert!(matches!(
            active.deactivate().activate(T1),
            CurrentPosition::Active { .. }
        ));
    }

    /// 復元した値がそのまま読み出せる。異常終了をまたぐ保持の土台である。
    #[test]
    fn a_rehydrated_position_reads_back_identically() {
        let (task, step, _) = ids();
        let position = CurrentPosition::rehydrate(task, step, false, T1);
        assert_eq!(position.task_id(), Some(task));
        assert_eq!(position.step_id(), Some(step));
        assert!(!position.is_active());
        assert_eq!(position.activated_at(), Some(T1));
    }
}
