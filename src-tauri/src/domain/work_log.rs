//! **作業区間** (`WorkSegment`) — **現在地**が**活性**だった時間を、ステップごとに残す。
//!
//! 勤怠・工数の報告のための**参考値**である。精度より軽さを優先する。
//!
//! # 記録するが表示しない (AD-15)
//!
//! 区間はどの面 (オーバーレイ・介入パネル・メニューバー項目) にも出さない。読み手は
//! `scripts/export-work-log.sh` だけであり、DB を直接読む。この型は `serde::Serialize`
//! を**実装しない** — コマンド境界 (AD-3) へ出せないため、「うっかり画面に出す」経路が
//! 型として成立しない (**切り替え履歴**と同じ作法)。
//!
//! # 書くのは区間が閉じた瞬間だけである
//!
//! 開いている区間 ([`OpenWorkSegment`]) は**永続化しない** (メモリだけ / AD-2)。状態が
//! 変わったときに閉じた区間を 1 行書く。**定期的な書き込みはしない** — 異常終了した
//! ときに開いていた区間は失われてよい (利用者の裁定)。
//!
//! # 判定は時計から切り離す
//!
//! 「いま開いているべき区間の種類」([`desired`])・区間の閉じ方 ([`shift`])・閉じる
//! 時刻 ([`closing_time`]) はいずれも純粋関数であり、「いま」を引数で受け取る。
//! 状態を変える経路 ([`super::state::Core`]) が時計を読み、これらに判断させ、
//! 状態の変化と同じトランザクションで書く。

use std::fmt;

use uuid::Uuid;

use super::position::CurrentPosition;
use super::rest;
use super::task::{id_newtype, new_v7, StepId, TaskId};
use super::Timestamp;

/// **作業区間**の ID。UUID v7 (スパイン「一貫性の規約」)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkSegmentId(Uuid);

id_newtype!(WorkSegmentId);

/// **作業区間**の種類 — 何に時間を使っていたか。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkSegmentKind {
    /// **ステップ**の作業。
    Step {
        /// そのステップの**タスク**。
        task_id: TaskId,
        /// そのステップ。
        step_id: StepId,
    },
    /// **待機** — **現在地**が**戻り先**と同じ**タスク**にある間。主作業のステップには
    /// 数えない (用語集の申し送り)。
    Wait {
        /// 戻り先の**タスク**。
        task_id: TaskId,
        /// 何を待っているか。空なら `None`。
        label: Option<String>,
    },
}

impl WorkSegmentKind {
    /// 区間の**タスク**。
    #[must_use]
    pub const fn task_id(&self) -> TaskId {
        match self {
            Self::Step { task_id, .. } | Self::Wait { task_id, .. } => *task_id,
        }
    }
}

/// 開いている**作業区間**。**永続化しない** (AD-2)。種類と開始時刻だけを持つ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenWorkSegment {
    kind: WorkSegmentKind,
    started_at: Timestamp,
}

impl OpenWorkSegment {
    /// `started_at` から開く。
    #[must_use]
    pub const fn new(kind: WorkSegmentKind, started_at: Timestamp) -> Self {
        Self { kind, started_at }
    }

    /// 種類。
    #[must_use]
    pub const fn kind(&self) -> &WorkSegmentKind {
        &self.kind
    }

    /// 開始時刻。
    #[must_use]
    pub const fn started_at(&self) -> Timestamp {
        self.started_at
    }

    /// `ended_at` で閉じる。**長さが 0 以下なら `None`** — 長さ 0 の区間は書かない。
    #[must_use]
    pub fn close(&self, ended_at: Timestamp) -> Option<WorkSegment> {
        (ended_at > self.started_at).then(|| WorkSegment {
            id: WorkSegmentId::new(ended_at),
            kind: self.kind.clone(),
            started_at: self.started_at,
            ended_at,
        })
    }
}

/// 閉じた**作業区間** — DB に書く 1 行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkSegment {
    id: WorkSegmentId,
    kind: WorkSegmentKind,
    started_at: Timestamp,
    /// 常に `started_at` より後である ([`OpenWorkSegment::close`] が保証する)。
    ended_at: Timestamp,
}

impl WorkSegment {
    /// ID。
    #[must_use]
    pub const fn id(&self) -> WorkSegmentId {
        self.id
    }

    /// 種類。
    #[must_use]
    pub const fn kind(&self) -> &WorkSegmentKind {
        &self.kind
    }

    /// 開始時刻。
    #[must_use]
    pub const fn started_at(&self) -> Timestamp {
        self.started_at
    }

    /// 終了時刻。
    #[must_use]
    pub const fn ended_at(&self) -> Timestamp {
        self.ended_at
    }
}

/// **いま開いているべき区間の種類。**
///
/// 1. **未着手**・**非活性** (**休息**中) — 無し。数えない。
/// 2. **活性**かつ**待機**中で、**現在地**が**戻り先**と同じ**タスク**にある — 待機。
/// 3. **活性** — **現在地**のステップ。**待機**中に別の**タスク**へ移った時間もここ。
///
/// `wait` は**待機**中なら (戻り先の**タスク**, 何を待っているか)。
#[must_use]
pub fn desired(
    position: CurrentPosition,
    wait: Option<(TaskId, Option<&str>)>,
) -> Option<WorkSegmentKind> {
    let CurrentPosition::Active {
        task_id, step_id, ..
    } = position
    else {
        return None;
    };
    match wait {
        Some((return_task, label)) if return_task == task_id => Some(WorkSegmentKind::Wait {
            task_id,
            label: label.map(str::to_string),
        }),
        _ => Some(WorkSegmentKind::Step { task_id, step_id }),
    }
}

/// 区間を閉じる時刻。
///
/// 直前の刻みから**はるかに長い**時間が経っていれば ([`rest::slept_millis`])、その間は
/// Mac がスリープしていた — **スリープ直前の刻みの時刻**で閉じる。そうでなければ `now`。
///
/// 刻みより先に操作が届いた場合 (スリープから戻ってすぐホットキーを押した) も同じ判定で
/// スリープを区間に含めない。
#[must_use]
pub fn closing_time(last_tick: Timestamp, now: Timestamp, interval_millis: i64) -> Timestamp {
    let jump = now.unix_millis().saturating_sub(last_tick.unix_millis());
    if rest::slept_millis(jump, interval_millis) > 0 {
        last_tick
    } else {
        now
    }
}

/// [`shift`] の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shift {
    /// 書く区間。閉じなかった・長さ 0 なら `None`。
    pub closed: Option<WorkSegment>,
    /// 以後開いている区間。
    pub open: Option<OpenWorkSegment>,
}

/// **開いている区間を、望ましい種類に合わせる。**
///
/// 種類が同じなら何もしない (閉じない・書かない)。違えば、開いている区間を `ended_at`
/// で閉じ、望ましい区間を `now` から開く。
#[must_use]
pub fn shift(
    open: Option<&OpenWorkSegment>,
    desired: Option<WorkSegmentKind>,
    ended_at: Timestamp,
    now: Timestamp,
) -> Shift {
    if open.map(OpenWorkSegment::kind) == desired.as_ref() {
        return Shift {
            closed: None,
            open: open.cloned(),
        };
    }
    Shift {
        closed: open.and_then(|open| open.close(ended_at)),
        open: desired.map(|kind| OpenWorkSegment::new(kind, now)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: Timestamp = Timestamp::from_unix_millis(1_789_000_000_000);
    const MINUTE: i64 = 60_000;

    fn at(minutes: i64) -> Timestamp {
        Timestamp::from_unix_millis(T0.unix_millis() + minutes * MINUTE)
    }

    fn ids() -> (TaskId, StepId, TaskId) {
        (TaskId::new(T0), StepId::new(T0), TaskId::new(T0))
    }

    /// **未着手**・**休息**中は何も開かない。
    #[test]
    fn nothing_is_counted_while_not_started_or_resting() {
        let (task, step, _) = ids();
        assert_eq!(desired(CurrentPosition::NotStarted, None), None);
        let resting = CurrentPosition::NotStarted
            .move_to(task, step, T0)
            .deactivate();
        assert_eq!(desired(resting, None), None);
        assert_eq!(desired(resting, Some((task, Some("AI の出力")))), None);
    }

    /// **活性**ならそのステップの区間。
    #[test]
    fn an_active_position_counts_for_its_step() {
        let (task, step, _) = ids();
        let position = CurrentPosition::NotStarted.move_to(task, step, T0);
        assert_eq!(
            desired(position, None),
            Some(WorkSegmentKind::Step {
                task_id: task,
                step_id: step
            })
        );
    }

    /// **待機**中に戻り先と同じタスクにいれば待機の区間 (一言つき)。
    #[test]
    fn waiting_on_the_main_task_counts_as_waiting() {
        let (task, step, _) = ids();
        let position = CurrentPosition::NotStarted.move_to(task, step, T0);
        assert_eq!(
            desired(position, Some((task, Some("AI の出力")))),
            Some(WorkSegmentKind::Wait {
                task_id: task,
                label: Some("AI の出力".to_string())
            })
        );
        assert_eq!(
            desired(position, Some((task, None))),
            Some(WorkSegmentKind::Wait {
                task_id: task,
                label: None
            })
        );
    }

    /// **待機**中に別タスクへ移った時間はそのステップの区間。
    #[test]
    fn waiting_on_another_task_counts_for_that_step() {
        let (task, step, main) = ids();
        let position = CurrentPosition::NotStarted.move_to(task, step, T0);
        assert_eq!(
            desired(position, Some((main, Some("AI の出力")))),
            Some(WorkSegmentKind::Step {
                task_id: task,
                step_id: step
            })
        );
    }

    /// 長さ 0 (以下) の区間は閉じても書かない。
    #[test]
    fn a_zero_length_segment_is_not_written() {
        let (task, step, _) = ids();
        let open = OpenWorkSegment::new(
            WorkSegmentKind::Step {
                task_id: task,
                step_id: step,
            },
            at(5),
        );
        assert_eq!(open.close(at(5)), None);
        assert_eq!(open.close(at(4)), None);
        let closed = open.close(at(30)).expect("長さがある");
        assert_eq!(closed.started_at(), at(5));
        assert_eq!(closed.ended_at(), at(30));
        assert_eq!(closed.kind(), open.kind());
    }

    /// 種類が同じなら閉じない — 書き込みも起きない。
    #[test]
    fn the_same_kind_keeps_the_segment_open() {
        let (task, step, _) = ids();
        let kind = WorkSegmentKind::Step {
            task_id: task,
            step_id: step,
        };
        let open = OpenWorkSegment::new(kind.clone(), at(0));
        let shifted = shift(Some(&open), Some(kind), at(60), at(60));
        assert_eq!(shifted.closed, None);
        assert_eq!(shifted.open, Some(open));
        assert_eq!(shift(None, None, at(60), at(60)).open, None);
    }

    /// 種類が変われば閉じて開き直す。
    #[test]
    fn a_different_kind_closes_and_reopens() {
        let (task, step, _) = ids();
        let next = StepId::new(T0);
        let open = OpenWorkSegment::new(
            WorkSegmentKind::Step {
                task_id: task,
                step_id: step,
            },
            at(0),
        );
        let shifted = shift(
            Some(&open),
            Some(WorkSegmentKind::Step {
                task_id: task,
                step_id: next,
            }),
            at(25),
            at(25),
        );
        let closed = shifted.closed.expect("閉じる");
        assert_eq!((closed.started_at(), closed.ended_at()), (at(0), at(25)));
        let reopened = shifted.open.expect("開く");
        assert_eq!(reopened.started_at(), at(25));

        // 休息へ — 閉じるだけで開かない。
        let shifted = shift(Some(&open), None, at(25), at(25));
        assert!(shifted.closed.is_some());
        assert_eq!(shifted.open, None);

        // 休息明け — 開くだけ。
        let shifted = shift(None, Some(open.kind().clone()), at(40), at(40));
        assert_eq!(shifted.closed, None);
        assert_eq!(shifted.open.map(|open| open.started_at()), Some(at(40)));
    }

    /// 待機の一言が違えば別の区間である。
    #[test]
    fn a_different_label_is_a_different_segment() {
        let (task, _, _) = ids();
        let open = OpenWorkSegment::new(
            WorkSegmentKind::Wait {
                task_id: task,
                label: Some("a".to_string()),
            },
            at(0),
        );
        let shifted = shift(
            Some(&open),
            Some(WorkSegmentKind::Wait {
                task_id: task,
                label: None,
            }),
            at(5),
            at(5),
        );
        assert!(shifted.closed.is_some());
    }

    /// スリープ (刻みの飛び) の後なら、直前の刻みの時刻で閉じる。
    #[test]
    fn a_sleep_closes_at_the_last_tick() {
        let interval = rest::TICK_INTERVAL_MILLIS;
        // 通常の刻み — いまで閉じる。
        assert_eq!(
            closing_time(
                at(0),
                Timestamp::from_unix_millis(at(0).unix_millis() + interval),
                interval
            ),
            Timestamp::from_unix_millis(at(0).unix_millis() + interval)
        );
        // 30 分の飛び — 直前の刻みで閉じる。
        assert_eq!(closing_time(at(0), at(30), interval), at(0));
    }

    /// ID は UUID v7 であり、文字列の形で往復する。
    #[test]
    fn an_id_is_a_v7_uuid() {
        let id = WorkSegmentId::new(T0);
        let uuid = Uuid::parse_str(&id.to_string()).expect("UUID として読める");
        assert_eq!(uuid.get_version_num(), 7);
        assert_eq!(WorkSegmentId::parse(&id.to_string()).expect("読める"), id);
    }
}
