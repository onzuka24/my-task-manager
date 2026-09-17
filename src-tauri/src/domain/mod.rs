//! ドメインコア — OS を一切知らない層 (AD-1)。
//!
//! タスク・ステップ・現在地・中断メモのモデルと状態遷移、および計時がここに積まれる。
//!
//! ここに Tauri の API・OS API・Web API のいずれかを参照するコードを書いてはならない
//! (AD-1)。時刻の取得も例外ではなく、[`Clock`] を唯一の入口とする — 壁時計を直接読む
//! 呼び出しをこの層に書かない (AD-8)。実際の読み取りは `adapters/clock` にある。
//!
//! この三つの禁止は、spec の Verification 節にある grep が `domain/` に対して何も
//! 返さないことで検証できる。**この doc コメント群がその検索語を含まないよう言い換えて
//! あるのは意図である** — 説明が検査を無効にしてはならない。
//!
//! - [`task`] — [`task::Task`] / [`task::Step`] と、その不変条件を破れない形の操作
//! - [`position`] — [`position::CurrentPosition`] と活性/非活性の遷移
//! - [`state`] — 全タスクと唯一の現在地を保持し、状態を変えうる操作を集約する

pub mod position;
pub mod state;
pub mod task;

use std::fmt;

/// UTC の時刻。
///
/// 内部表現は Unix epoch からのミリ秒、外部表現 (永続化・コマンド境界) は UTC の
/// ISO 8601 文字列である (スパイン「一貫性の規約」)。文字列を持ち回すと比較のたびに
/// 解析が要り、解析の失敗が比較の中に紛れ込む。境界でだけ文字列に変換する。
///
/// 時計の読み取りは [`Clock`] を通る。この型は「いつか」を運ぶだけで、「いま」を
/// 知らない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp {
    unix_millis: i64,
}

/// 1 日のミリ秒数。うるう秒を持たない Unix 時間の定義そのものである。
const MILLIS_PER_DAY: i64 = 86_400_000;

impl Timestamp {
    /// 表せる最も古い時刻 — `0000-01-01T00:00:00.000Z`。
    pub const MIN: Self = Self {
        unix_millis: days_from_civil(0, 1, 1) * MILLIS_PER_DAY,
    };

    /// 表せる最も新しい時刻 — `9999-12-31T23:59:59.999Z`。
    pub const MAX: Self = Self {
        unix_millis: days_from_civil(9999, 12, 31) * MILLIS_PER_DAY + MILLIS_PER_DAY - 1,
    };

    /// Unix epoch からのミリ秒から作る。
    ///
    /// **範囲外の値は [`Self::MIN`] / [`Self::MAX`] に丸める。** ISO 8601 の年は 4 桁で
    /// あり、5 桁の年や負の年を書き出すと [`Self::parse_iso8601`] が読み戻せない — 一度
    /// 書けてしまえば、以後の復元が永久に「破損」を返し続ける。丸めるのは、範囲外の値が
    /// 壁時計の異常 (時計の巻き戻し・オーバーフロー) からしか来ないためである。
    #[must_use]
    pub const fn from_unix_millis(unix_millis: i64) -> Self {
        if unix_millis < Self::MIN.unix_millis {
            return Self::MIN;
        }
        if unix_millis > Self::MAX.unix_millis {
            return Self::MAX;
        }
        Self { unix_millis }
    }

    /// Unix epoch からのミリ秒。
    #[must_use]
    pub const fn unix_millis(self) -> i64 {
        self.unix_millis
    }

    /// UTC の ISO 8601 文字列 (`2026-09-17T01:02:03.004Z`)。
    ///
    /// ミリ秒は常に 3 桁で出す。桁数を可変にすると文字列の辞書順が時刻順と一致せず、
    /// SQL 側で素朴に並べ替えたときに壊れる。
    #[must_use]
    pub fn to_iso8601(self) -> String {
        let days = self.unix_millis.div_euclid(MILLIS_PER_DAY);
        let millis_of_day = self.unix_millis.rem_euclid(MILLIS_PER_DAY);
        let (year, month, day) = civil_from_days(days);
        let hour = millis_of_day / 3_600_000;
        let minute = (millis_of_day / 60_000) % 60;
        let second = (millis_of_day / 1_000) % 60;
        let milli = millis_of_day % 1_000;
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{milli:03}Z")
    }

    /// UTC の ISO 8601 文字列を読む。[`Self::to_iso8601`] の逆である。
    ///
    /// 受け付けるのは `YYYY-MM-DDTHH:MM:SS[.fff]Z` のみ。オフセット付きの表記を
    /// 受け付けないのは、この層が UTC 以外を持たないためである (スパイン「一貫性の規約」)。
    ///
    /// # Errors
    ///
    /// 形式が違う、または日付として成立しない値のとき [`TimestampError`] を返す。
    pub fn parse_iso8601(text: &str) -> Result<Self, TimestampError> {
        fn fail(text: &str) -> TimestampError {
            TimestampError {
                text: text.to_string(),
            }
        }

        let body = text.strip_suffix('Z').ok_or_else(|| fail(text))?;
        let (date, rest) = body.split_once('T').ok_or_else(|| fail(text))?;
        let (time, fraction) = match rest.split_once('.') {
            Some((time, fraction)) => (time, fraction),
            None => (rest, "0"),
        };

        let mut date_parts = date.splitn(3, '-');
        let year: i64 = parse_int(date_parts.next(), 4).ok_or_else(|| fail(text))?;
        let month: i64 = parse_int(date_parts.next(), 2).ok_or_else(|| fail(text))?;
        let day: i64 = parse_int(date_parts.next(), 2).ok_or_else(|| fail(text))?;
        if date_parts.next().is_some() {
            return Err(fail(text));
        }

        let mut time_parts = time.splitn(3, ':');
        let hour: i64 = parse_int(time_parts.next(), 2).ok_or_else(|| fail(text))?;
        let minute: i64 = parse_int(time_parts.next(), 2).ok_or_else(|| fail(text))?;
        let second: i64 = parse_int(time_parts.next(), 2).ok_or_else(|| fail(text))?;
        if time_parts.next().is_some() {
            return Err(fail(text));
        }

        if !(1..=12).contains(&month)
            || !(1..=31).contains(&day)
            || !(0..=23).contains(&hour)
            || !(0..=59).contains(&minute)
            // うるう秒 (60) は Unix 時間に存在しないため受け付けない。
            || !(0..=59).contains(&second)
        {
            return Err(fail(text));
        }

        // 小数部は 3 桁に揃える。`.5` は 500ms であって 5ms ではない。
        let mut millis_fraction = String::from(fraction);
        if millis_fraction.is_empty() || !millis_fraction.bytes().all(|b| b.is_ascii_digit()) {
            return Err(fail(text));
        }
        millis_fraction.truncate(3);
        while millis_fraction.len() < 3 {
            millis_fraction.push('0');
        }
        let milli: i64 = millis_fraction.parse().map_err(|_| fail(text))?;

        let days = days_from_civil(year, month, day);
        // 月末を超える日付 (2 月 31 日など) は往復しない。往復しない値を受け入れると
        // 破損した DB が静かに別の時刻として復元される。
        let (round_year, round_month, round_day) = civil_from_days(days);
        if round_year != year || round_month != month || round_day != day {
            return Err(fail(text));
        }

        Ok(Self::from_unix_millis(
            days * MILLIS_PER_DAY + hour * 3_600_000 + minute * 60_000 + second * 1_000 + milli,
        ))
    }
}

/// 固定長の 10 進数を読む。桁数が違えば `None` — `2026-9-1` のような表記を
/// 「読めた」ことにしない。
fn parse_int(part: Option<&str>, width: usize) -> Option<i64> {
    let part = part?;
    if part.len() != width || !part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    part.parse().ok()
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_iso8601())
    }
}

impl serde::Serialize for Timestamp {
    /// 境界へ出る形は ISO 8601 文字列で一つに固定する (スパイン「一貫性の規約」)。
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_iso8601())
    }
}

/// ISO 8601 として読めなかった文字列。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimestampError {
    text: String,
}

impl fmt::Display for TimestampError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UTC の ISO 8601 として読めない時刻: `{}`", self.text)
    }
}

impl std::error::Error for TimestampError {}

/// 1970-01-01 を 0 とする通算日から (年, 月, 日) を得る。
///
/// Howard Hinnant の `civil_from_days`。3 月始まりの暦年に置き換えることで、
/// うるう年の分岐を 1 か所に畳んでいる。
const fn civil_from_days(days: i64) -> (i64, i64, i64) {
    // 0000-03-01 を原点に移す。
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

/// (年, 月, 日) から 1970-01-01 を 0 とする通算日を得る。[`civil_from_days`] の逆。
const fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = (month + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// コアが所有する時計 (AD-8)。
///
/// **ドメインが「いま」を知る唯一の入口である。** 壁時計を `domain/` から直接読まない
/// のは、AD-1 (OS API を参照しない) のためだけではない — 連続作業時間の起点やステップ
/// の完了時刻がテストから決定的に検証できなくなるためでもある。
///
/// 実装は `adapters/clock` にある。
pub trait Clock: Send + Sync {
    /// いまの時刻。
    fn now(&self) -> Timestamp;
}

/// ドメインの不変条件に反する要求。
///
/// コアは `Result` を返し panic しない (スパイン「一貫性の規約」)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    /// ステップを 1 個も持たないタスクは作れない (CAP-4 / FR-4)。
    EmptyTask,
    /// 完了済みのステップは分割できない (CAP-5)。
    ///
    /// 完了はユーザーの明示宣言のみで付与・取消される (FR-4 / AD-2) ため、分割後の
    /// 前半・後半のどちらに完了が帰属するかをコアが決めることができない。
    SplitCompleted,
    /// 指定されたタスクが存在しない。
    UnknownTask,
    /// 指定されたステップが存在しない。
    UnknownStep,
    /// 挿入位置が 1..=N+1 の範囲外である。
    OrdinalOutOfRange,
}

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyTask => f.write_str("ステップを持たないタスクは作成できない"),
            Self::SplitCompleted => f.write_str("完了済みのステップは分割できない"),
            Self::UnknownTask => f.write_str("指定されたタスクが存在しない"),
            Self::UnknownStep => f.write_str("指定されたステップが存在しない"),
            Self::OrdinalOutOfRange => f.write_str("指定された位置はステップの範囲外である"),
        }
    }
}

impl std::error::Error for DomainError {}

/// テスト用の、進まない時計。
///
/// 実装を `adapters/` 側に置いたままテストから壁時計を読むと、連続作業時間の起点や
/// 完了時刻の検証が実行時刻に依存する。テストは必ずこの時計を使う。
#[cfg(test)]
#[derive(Debug, Clone)]
pub struct FixedClock {
    now: std::sync::Arc<std::sync::atomic::AtomicI64>,
}

#[cfg(test)]
impl FixedClock {
    /// Unix epoch からのミリ秒で固定した時計を作る。
    pub fn at(unix_millis: i64) -> Self {
        Self {
            now: std::sync::Arc::new(std::sync::atomic::AtomicI64::new(unix_millis)),
        }
    }

    /// 時計を進める。呼び出し側が明示したときにだけ時間が進む。
    pub fn advance(&self, millis: i64) {
        self.now
            .fetch_add(millis, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(test)]
impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        Timestamp::from_unix_millis(self.now.load(std::sync::atomic::Ordering::SeqCst))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// epoch そのものが往復する。境界の 0 を特別扱いしていないことの確認。
    #[test]
    fn the_epoch_round_trips() {
        let epoch = Timestamp::from_unix_millis(0);
        assert_eq!(epoch.to_iso8601(), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            Timestamp::parse_iso8601("1970-01-01T00:00:00.000Z"),
            Ok(epoch)
        );
    }

    /// うるう年の 2 月 29 日を含む日付が往復する。
    #[test]
    fn a_leap_day_round_trips() {
        let text = "2024-02-29T23:59:59.999Z";
        let parsed = Timestamp::parse_iso8601(text).expect("うるう日は読める");
        assert_eq!(parsed.to_iso8601(), text);
    }

    /// 任意の日を往復させ、暦の変換が自己整合であることを確かめる。
    #[test]
    fn many_days_round_trip_through_the_calendar() {
        // 1970-01-01 の前後 ±40 年分を 1 日ずつ。うるう年・世紀・400 年規則を跨ぐ。
        for day in -14_610..14_610 {
            let stamp = Timestamp::from_unix_millis(day * MILLIS_PER_DAY + 1);
            let text = stamp.to_iso8601();
            assert_eq!(
                Timestamp::parse_iso8601(&text),
                Ok(stamp),
                "往復しない日がある: {text}"
            );
        }
    }

    /// **書けた時刻は必ず読み戻せる。** 範囲の両端が往復すること。
    ///
    /// 範囲を持たなければ 5 桁の年や負の年が書き出され、読み戻せない値が DB に残る。
    /// 一度残れば以後の復元が永久に「破損」を返し続ける。
    #[test]
    fn both_ends_of_the_representable_range_round_trip() {
        assert_eq!(Timestamp::MIN.to_iso8601(), "0000-01-01T00:00:00.000Z");
        assert_eq!(Timestamp::MAX.to_iso8601(), "9999-12-31T23:59:59.999Z");
        for edge in [Timestamp::MIN, Timestamp::MAX] {
            assert_eq!(
                Timestamp::parse_iso8601(&edge.to_iso8601()),
                Ok(edge),
                "端の時刻が往復しない: {}",
                edge.to_iso8601()
            );
        }
    }

    /// 範囲外のミリ秒は端に丸められる。書き出せない値を作らせない。
    #[test]
    fn an_out_of_range_instant_is_clamped_to_the_ends() {
        assert_eq!(Timestamp::from_unix_millis(i64::MAX), Timestamp::MAX);
        assert_eq!(Timestamp::from_unix_millis(i64::MIN), Timestamp::MIN);
        assert_eq!(
            Timestamp::parse_iso8601(&Timestamp::from_unix_millis(i64::MAX).to_iso8601()),
            Ok(Timestamp::MAX)
        );
        assert_eq!(
            Timestamp::parse_iso8601(&Timestamp::from_unix_millis(i64::MIN).to_iso8601()),
            Ok(Timestamp::MIN)
        );
    }

    /// epoch より前の時刻でも壊れない。切り捨て除算では日付が 1 日ずれる。
    #[test]
    fn a_time_before_the_epoch_is_not_off_by_a_day() {
        let stamp = Timestamp::from_unix_millis(-1);
        assert_eq!(stamp.to_iso8601(), "1969-12-31T23:59:59.999Z");
    }

    /// 文字列の辞書順が時刻順と一致する。SQL 側で素朴に並べ替えても壊れないこと。
    #[test]
    fn the_text_form_sorts_like_the_instant() {
        let earlier = Timestamp::from_unix_millis(1_700_000_000_000);
        let later = Timestamp::from_unix_millis(1_700_000_000_007);
        assert!(earlier < later);
        assert!(earlier.to_iso8601() < later.to_iso8601());
    }

    /// 成立しない日付・桁数の違う表記・オフセット付きの表記はいずれも拒む。
    #[test]
    fn a_malformed_instant_is_rejected() {
        for text in [
            "2026-02-31T00:00:00.000Z", // 存在しない日
            "2026-9-17T00:00:00.000Z",  // 桁数が足りない
            "2026-09-17T00:00:00.000",  // Z が無い
            "2026-09-17T00:00:00+09:00",
            "2026-09-17T24:00:00.000Z", // 時が範囲外
            "2026-09-17T00:60:00.000Z",
            "2026-09-17T00:00:60.000Z", // うるう秒は Unix 時間に存在しない
            "2026-09-17T00:00:00.xxxZ",
            "",
        ] {
            assert!(
                Timestamp::parse_iso8601(text).is_err(),
                "受け付けてはならない: `{text}`"
            );
        }
    }

    /// 小数部は「3 桁に揃える」であって「そのまま読む」ではない。
    #[test]
    fn a_short_fraction_means_tenths_not_thousandths() {
        let stamp = Timestamp::parse_iso8601("1970-01-01T00:00:00.5Z").expect("読める");
        assert_eq!(stamp.unix_millis(), 500);
    }

    /// 境界へ出る形は ISO 8601 文字列である。
    #[test]
    fn an_instant_serializes_as_iso8601() {
        let json = serde_json::to_value(Timestamp::from_unix_millis(0)).expect("直列化できる");
        assert_eq!(json, serde_json::json!("1970-01-01T00:00:00.000Z"));
    }

    /// テスト用の時計は呼び出し側が進めたときにだけ進む。
    #[test]
    fn the_fixed_clock_only_moves_when_told() {
        let clock = FixedClock::at(1_000);
        assert_eq!(clock.now(), clock.now());
        clock.advance(500);
        assert_eq!(clock.now(), Timestamp::from_unix_millis(1_500));
    }
}
