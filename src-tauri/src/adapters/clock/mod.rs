//! 時計アダプタ — コアが要求する時計 ([`crate::domain::Clock`]) の壁時計実装。
//!
//! **`SystemTime::now()` がこのファイルにしか現れないのは意図である。** AD-1 は
//! `domain/` が OS API を参照することを禁じ、AD-8 は計時をコアが所有することを求める。
//! 二つを同時に満たす形は「時計の抽象はコアに、時計の読み取りはアダプタに」しかない。
//!
//! 単調時計 (`Instant`) ではなく壁時計を読むのは、**現在地**の起点や**完了**の時刻が
//! プロセスをまたいで永続化され、再起動後も同じ意味を持たなければならないためである。
//! 利用者が時計を巻き戻せば過去の時刻が記録されうるが、それは単独利用のツールとして
//! 受け入れる。

use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::{Clock, Timestamp};

/// OS の壁時計。
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        // epoch より前を「読めなかった」に潰さない。負の経過をそのまま負のミリ秒として
        // 運ぶ (Timestamp は epoch 前後のどちらも表せる)。
        //
        // 収まらない値は `i64` の端ではなく **[`Timestamp`] が表せる端**に丸める。
        // `i64::MAX` に倒すと 5 桁の年になり、書き出した時刻を読み戻せなくなる。
        let millis = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(elapsed) => {
                i64::try_from(elapsed.as_millis()).unwrap_or(Timestamp::MAX.unix_millis())
            }
            Err(error) => i64::try_from(error.duration().as_millis())
                .map_or(Timestamp::MIN.unix_millis(), |millis| -millis),
        };
        Timestamp::from_unix_millis(millis)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 壁時計が epoch より後を返す。UNIX_EPOCH との引き算の向きを取り違えていれば
    /// 負の値になる。
    #[test]
    fn the_wall_clock_is_after_the_epoch() {
        // 2020-01-01T00:00:00Z。これより前を返すなら計算が壊れている。
        assert!(SystemClock.now().unix_millis() > 1_577_836_800_000);
    }

    /// 壁時計の読みは必ず書き出せる範囲に収まる。収まらなければ、書いた時刻を二度と
    /// 読み戻せない DB ができる。
    #[test]
    fn the_wall_clock_stays_inside_the_representable_range() {
        let now = SystemClock.now();
        assert!(now >= Timestamp::MIN && now <= Timestamp::MAX);
        assert_eq!(Timestamp::parse_iso8601(&now.to_iso8601()), Ok(now));
    }

    /// 時計は進みこそすれ戻らない。
    #[test]
    fn the_wall_clock_does_not_run_backwards() {
        let first = SystemClock.now();
        let second = SystemClock.now();
        assert!(second >= first);
    }
}
