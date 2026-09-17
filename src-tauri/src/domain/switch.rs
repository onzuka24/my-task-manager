//! **切り替え履歴** (`SwitchRecord`) — SM-C3 の測定基盤 (CAP-7 / AD-15)。
//!
//! **切り替え**が起きた事実だけを残す。持つのは発生時刻と**中断メモ**記入の有無であり、
//! **メモの本文を持たない**。本文を載せれば、表示しないと決めた履歴の中に利用者の
//! 文章が入り込み、AD-15 の「履歴を利用者に見せない」がログや将来の画面から破れる。
//!
//! # 書くだけで読み戻さない
//!
//! 履歴は起動時の復元対象ではない。コアが読む理由が無く、読めばメモリ上に「表示されうる
//! 値」を置くことになる。測定が必要になった時点で SQL から直接数える (Design Notes)。
//!
//! # 表示できない形にしてある
//!
//! この型は `serde::Serialize` を**実装しない**。コマンド境界 (AD-3) へ出せないため、
//! 「うっかり画面に出す」経路が型として成立しない。

use std::fmt;

use uuid::Uuid;

use super::task::{id_newtype, new_v7, StepId};
use super::Timestamp;

/// **切り替え履歴**の ID。UUID v7 (スパイン「一貫性の規約」)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SwitchRecordId(Uuid);

id_newtype!(SwitchRecordId);

/// **切り替え履歴** — 一度の**切り替え**が残す 1 行。
///
/// 用語集の識別子に 1:1 で対応する (AD-10)。**本文を持たないこと**が、この型の
/// 最も重要な性質である。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchRecord {
    id: SwitchRecordId,
    /// 離脱元の**ステップ**。移動先ではない — 記録の主題は「どこを離れたか」である。
    departed_step_id: StepId,
    occurred_at: Timestamp,
    /// **中断メモ**が記入されたか。**本文ではない。**
    note_written: bool,
}

impl SwitchRecord {
    /// 1 行を起こす。
    #[must_use]
    pub fn new(now: Timestamp, departed_step_id: StepId, note_written: bool) -> Self {
        Self {
            id: SwitchRecordId::new(now),
            departed_step_id,
            occurred_at: now,
            note_written,
        }
    }

    /// ID。
    #[must_use]
    pub const fn id(&self) -> SwitchRecordId {
        self.id
    }

    /// 離脱元の**ステップ**。
    #[must_use]
    pub const fn departed_step_id(&self) -> StepId {
        self.departed_step_id
    }

    /// 発生時刻。
    #[must_use]
    pub const fn occurred_at(&self) -> Timestamp {
        self.occurred_at
    }

    /// **中断メモ**が記入されたか。
    #[must_use]
    pub const fn note_written(&self) -> bool {
        self.note_written
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Timestamp = Timestamp::from_unix_millis(1_789_000_000_000);

    /// 記録は「いつ」「どこを離れ」「メモを書いたか」だけを持つ。
    #[test]
    fn a_record_holds_the_instant_the_origin_and_whether_a_note_was_written() {
        let step_id = StepId::new(NOW);
        let record = SwitchRecord::new(NOW, step_id, true);

        assert_eq!(record.departed_step_id(), step_id);
        assert_eq!(record.occurred_at(), NOW);
        assert!(record.note_written());
    }

    /// メモを省いた**切り替え**も 1 行を残す。記入率の分母はここで決まる (SM-C3)。
    #[test]
    fn omitting_the_note_still_leaves_a_record() {
        let record = SwitchRecord::new(NOW, StepId::new(NOW), false);
        assert!(!record.note_written());
    }

    /// ID は UUID v7 であり、コアの時計の読みから来る (AD-8)。
    #[test]
    fn an_id_is_a_v7_uuid_stamped_by_the_core_clock() {
        let record = SwitchRecord::new(NOW, StepId::new(NOW), false);
        let uuid = Uuid::parse_str(&record.id().to_string()).expect("UUID として読める");
        assert_eq!(uuid.get_version_num(), 7);
    }

    /// ID は往復する。永続化の形が壊れれば履歴が二重に書かれうる。
    #[test]
    fn an_id_round_trips_through_its_text_form() {
        let id = SwitchRecordId::new(NOW);
        assert_eq!(SwitchRecordId::parse(&id.to_string()).expect("読める"), id);
        assert!(SwitchRecordId::parse("not-a-uuid").is_err());
    }

    /// **本文を持つ欄が無い。** 型に無い以上、表示も永続化もされようがない (AD-15)。
    ///
    /// 欄が足された瞬間にこの一覧が食い違うため、追加は必ずここを通る。
    #[test]
    fn a_record_has_no_field_for_the_note_text() {
        let record = SwitchRecord::new(NOW, StepId::new(NOW), true);
        let rendered = format!("{record:?}");
        assert!(rendered.contains("note_written"));
        assert!(
            !rendered.contains("note_text") && !rendered.contains("interruption_note"),
            "履歴に本文の欄を持たせない (AD-15): {rendered}"
        );
    }
}
