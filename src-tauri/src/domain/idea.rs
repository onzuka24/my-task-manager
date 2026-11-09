//! **思いつき** (`Idea`) — 作業中に浮かんだ別の作業を、一行で手放すための置き場。
//!
//! **タスク**ではない。**ステップ**を持たず、**現在地**になれず、既定表示に出ない。
//! **開示面**の末尾に並び、そこから**タスク**へ昇格するか、**思いつきの寿命**を過ぎて
//! 消えるかのどちらかである。
//!
//! # 消えることが仕様である
//!
//! 昇格しなかった**思いつき**は、作成から**思いつきの寿命** (既定 14 日) を過ぎた時点で
//! DB から削除する。消えたことも件数も知らせない (AD-15)。**削除できるのは
//! 思いつきだけである** — **タスク**・**ステップ**を消す経路はどこにも無い。
//!
//! # 判定は時計から切り離す
//!
//! 寿命の判定は [`expired`] が持つ純粋関数であり、「いま」を引数で受け取る。刻み
//! ([`super::state::Core::tick`]) が時計を読み、この関数に判断させ、永続化する。

use std::fmt;

use uuid::Uuid;

use super::setting::{self, Setting};
use super::task::{id_newtype, new_v7};
use super::Timestamp;

/// **思いつきの寿命**の既定値 — 14 日。DB に値が無ければこれを使う (AD-11)。
pub const DEFAULT_IDEA_LIFETIME_SECONDS: i64 = 14 * 24 * 60 * 60;

/// **思いつきの寿命**を保存する鍵。
pub const IDEA_LIFETIME_KEY: &str = "idea_lifetime_seconds";

/// **思いつき**の ID。UUID v7 (スパイン「一貫性の規約」)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IdeaId(Uuid);

id_newtype!(IdeaId);

/// **思いつき** — 一行の本文と、作られた時刻だけを持つ。
///
/// **件数・残り日数・経過を表示に出す欄を持たせない** のは境界の側 (コマンド) の責務で
/// あり、この型が運ぶ `created_at` は寿命の判定と並び順のためだけにある。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Idea {
    id: IdeaId,
    content: String,
    created_at: Timestamp,
}

impl Idea {
    /// 新しく置く。
    #[must_use]
    pub fn new(now: Timestamp, content: impl Into<String>) -> Self {
        Self {
            id: IdeaId::new(now),
            content: content.into(),
            created_at: now,
        }
    }

    /// 永続化された値から組み立て直す。
    #[must_use]
    pub fn rehydrate(id: IdeaId, content: String, created_at: Timestamp) -> Self {
        Self {
            id,
            content,
            created_at,
        }
    }

    /// ID。
    #[must_use]
    pub const fn id(&self) -> IdeaId {
        self.id
    }

    /// 本文。
    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }

    /// 置かれた時刻。
    #[must_use]
    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }
}

/// **思いつきの寿命** (秒) を読む。**無い・0 以下・数値でなければ既定値である**
/// ([`setting::positive_seconds`])。
///
/// 0 を受け入れれば、置いた次の刻みで消える — 手放す先が無くなる。
#[must_use]
pub fn lifetime_seconds(settings: &[Setting]) -> i64 {
    setting::positive_seconds(settings, IDEA_LIFETIME_KEY).unwrap_or(DEFAULT_IDEA_LIFETIME_SECONDS)
}

/// **寿命を過ぎた思いつき**の ID を返す純粋関数。
///
/// 作成時刻 + 寿命を**過ぎた** (厳密に後である) ものだけを返す。ちょうどの時刻では
/// まだ消さない。
#[must_use]
pub fn expired(ideas: &[Idea], now: Timestamp, lifetime_seconds: i64) -> Vec<IdeaId> {
    let lifetime_millis = lifetime_seconds.saturating_mul(1_000);
    ideas
        .iter()
        .filter(|idea| {
            now.unix_millis()
                > idea
                    .created_at
                    .unix_millis()
                    .saturating_add(lifetime_millis)
        })
        .map(Idea::id)
        .collect()
}

impl fmt::Display for Idea {
    /// **本文を出さない。** ログに書かれうる形に利用者の文章を載せない (スパイン
    /// 「一貫性の規約」)。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "idea {}", self.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;
    const T0: Timestamp = Timestamp::from_unix_millis(1_789_000_000_000);

    fn at(seconds_after_t0: i64) -> Timestamp {
        Timestamp::from_unix_millis(T0.unix_millis() + seconds_after_t0 * 1_000)
    }

    /// 14 日ちょうどでは消えず、1 秒過ぎれば消える (I/O マトリクス「寿命」)。
    #[test]
    fn an_idea_expires_only_after_its_lifetime() {
        let idea = Idea::new(T0, "請求書の様式を直す");
        let ideas = vec![idea.clone()];
        let lifetime = DEFAULT_IDEA_LIFETIME_SECONDS;

        assert!(expired(&ideas, at(14 * DAY), lifetime).is_empty());
        assert_eq!(expired(&ideas, at(14 * DAY + 1), lifetime), vec![idea.id()]);
    }

    /// 寿命を過ぎたものだけを選ぶ。
    #[test]
    fn only_the_old_ones_are_chosen() {
        let old = Idea::new(T0, "古い");
        let young = Idea::new(at(10 * DAY), "新しい");
        let ideas = vec![old.clone(), young];
        assert_eq!(
            expired(&ideas, at(15 * DAY), DEFAULT_IDEA_LIFETIME_SECONDS),
            vec![old.id()]
        );
    }

    /// 寿命の設定は秒で読み、壊れた値は既定値へ落ちる (I/O マトリクス「寿命の設定」)。
    #[test]
    fn the_lifetime_falls_back_to_fourteen_days() {
        assert_eq!(lifetime_seconds(&[]), 1_209_600);
        assert_eq!(
            lifetime_seconds(&[Setting::new(IDEA_LIFETIME_KEY, "60")]),
            60
        );
        for broken in ["0", "-1", "まもなく", ""] {
            assert_eq!(
                lifetime_seconds(&[Setting::new(IDEA_LIFETIME_KEY, broken)]),
                DEFAULT_IDEA_LIFETIME_SECONDS,
                "`{broken}` は既定値へ落ちること"
            );
        }
    }

    /// 60 秒の寿命なら 61 秒後に消える。
    #[test]
    fn a_short_lifetime_is_honoured() {
        let idea = Idea::new(T0, "短命");
        assert!(expired(std::slice::from_ref(&idea), at(60), 60).is_empty());
        assert_eq!(
            expired(std::slice::from_ref(&idea), at(61), 60),
            vec![idea.id()]
        );
    }

    /// 表示形は本文を含まない (ログに本文を書かない)。
    #[test]
    fn the_display_form_carries_no_content() {
        let idea = Idea::new(T0, "秘密の思いつき");
        assert!(!idea.to_string().contains("秘密"));
    }
}
