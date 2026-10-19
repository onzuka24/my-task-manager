//! **休息介入** の計時 (CAP-10 / FR-15 / AD-8)。
//!
//! **計時はコアが行う。** フロントエンドは経過時間を数えない。ここにあるのは「いまの
//! **現在地**・いまの時刻・直前の刻みの時刻・設定値」から「**介入**を発するべきか」と
//! 「**連続作業時間**の起点をどう扱うか」を決める**純粋関数**だけである
//! ([`decide`])。OS も時計も触らないため、`FixedClock` で全域を検証できる。
//!
//! # 起点は [`super::position::CurrentPosition`] が唯一の正である
//!
//! **連続作業時間**は `activated_at` からの経過であり、**非活性**から**活性**への遷移
//! でのみリセットされる。**切り替えではリセットしない** — リセットすれば、切り替えを
//! 繰り返すほど休息が遠のき、最も休息が要る使い方でツールが黙る (AD-8)。
//!
//! # スリープは時計の飛びで検出する
//!
//! 単調時計を持たないため、スリープを OS に問い合わせる術がこの層には無い。刻みの
//! 間隔よりはるかに長い実時間が経過していれば、その間プロセスは走っていない —
//! それがスリープである ([`slept_millis`])。長さが**休息閾値**を超えていれば**休息**が
//! 取られたものとみなして計時をリセットし、超えていなければその分を起点から差し引く
//! (「スリープ中は加算しない」/ AD-8)。
//!
//! # 復帰の介入は別の判定である
//!
//! [`decide`] は**活性**の間だけを見る。**休息**に入ってから**休息の長さ**が経ったら
//! 戻るきっかけを差し出す判定は [`decide_return`] が持つ。こちらは実時間で数え、
//! **スリープ中の時間も休息に含める** — スリープは離席である。**休息**を時間で
//! 終わらせはしない。促すだけである (FR-15)。

use super::position::CurrentPosition;
use super::setting::{self, Setting};
use super::Timestamp;

/// **休息閾値**の既定値 — 50 分 (CAP-10)。DB に値が無ければこれを使う (AD-11)。
pub const DEFAULT_REST_THRESHOLD_SECONDS: i64 = 50 * 60;

/// **猶予**の既定値 — 15 分 (CAP-10)。DB に値が無ければこれを使う (AD-11)。
pub const DEFAULT_GRACE_PERIOD_SECONDS: i64 = 15 * 60;

/// **休息閾値**を保存する鍵。
pub const REST_THRESHOLD_KEY: &str = "rest_threshold_seconds";

/// **猶予**を保存する鍵。
pub const GRACE_PERIOD_KEY: &str = "grace_period_seconds";

/// **休息の長さ**の既定値 — 10 分。DB に値が無ければこれを使う (AD-11)。
///
/// **休息**に入ってからこれだけ経つと復帰の**介入**を出し、「もう少し休む」が選ばれれば
/// 同じ長さの後に出直す。
pub const DEFAULT_REST_LENGTH_SECONDS: i64 = 10 * 60;

/// **休息の長さ**を保存する鍵。
pub const REST_LENGTH_KEY: &str = "rest_length_seconds";

/// 計時の刻みの間隔 (ミリ秒)。
///
/// **細かくしても得るものが無い。** 閾値は分の単位であり、待機時の CPU 予算 (AD-14) に
/// 対して刻みは唯一の常時走る仕事である。粗すぎると[スリープの検出][`slept_millis`]も
/// 粗くなるため、10 秒を採る。
pub const TICK_INTERVAL_MILLIS: i64 = 10_000;

/// 刻みの間隔の何倍を超える飛びをスリープとみなすか。
///
/// 1 倍では、負荷で刻みが 1 回遅れただけでスリープと判定される。**「はるかに長い」の
/// 実体がこの係数である。**
pub const SLEEP_GAP_FACTOR: i64 = 3;

/// **休息閾値**・**猶予**・**休息の長さ** (AD-11)。
///
/// 値は設定テーブルから読み、無ければコード内の定数を使う。**別建ての設定ファイルを
/// 持たない。**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestSettings {
    rest_threshold_seconds: i64,
    grace_period_seconds: i64,
    rest_length_seconds: i64,
}

impl Default for RestSettings {
    fn default() -> Self {
        Self {
            rest_threshold_seconds: DEFAULT_REST_THRESHOLD_SECONDS,
            grace_period_seconds: DEFAULT_GRACE_PERIOD_SECONDS,
            rest_length_seconds: DEFAULT_REST_LENGTH_SECONDS,
        }
    }
}

impl RestSettings {
    /// 永続化された**設定値**から読む。**値が無ければコード内の定数である** (AD-11)。
    ///
    /// 読めない値・0 以下の値も「無い」と同じ扱いにする
    /// ([`setting::positive_seconds`])。壊れた 1 行のために常駐の唯一の能動機能が
    /// 黙るほうが害が大きい。
    #[must_use]
    pub fn from_settings(settings: &[Setting]) -> Self {
        let fallback = Self::default();
        Self {
            rest_threshold_seconds: setting::positive_seconds(settings, REST_THRESHOLD_KEY)
                .unwrap_or(fallback.rest_threshold_seconds),
            grace_period_seconds: setting::positive_seconds(settings, GRACE_PERIOD_KEY)
                .unwrap_or(fallback.grace_period_seconds),
            rest_length_seconds: setting::positive_seconds(settings, REST_LENGTH_KEY)
                .unwrap_or(fallback.rest_length_seconds),
        }
    }

    /// **休息閾値** (秒)。
    #[must_use]
    pub const fn rest_threshold_seconds(&self) -> i64 {
        self.rest_threshold_seconds
    }

    /// **猶予** (秒)。
    #[must_use]
    pub const fn grace_period_seconds(&self) -> i64 {
        self.grace_period_seconds
    }

    /// **休息の長さ** (秒)。
    #[must_use]
    pub const fn rest_length_seconds(&self) -> i64 {
        self.rest_length_seconds
    }

    /// **休息の長さ** (ミリ秒)。
    #[must_use]
    pub const fn rest_length_millis(&self) -> i64 {
        self.rest_length_seconds.saturating_mul(1_000)
    }

    /// **休息閾値** (ミリ秒)。
    #[must_use]
    pub const fn rest_threshold_millis(&self) -> i64 {
        self.rest_threshold_seconds.saturating_mul(1_000)
    }

    /// **猶予** (ミリ秒)。
    #[must_use]
    pub const fn grace_period_millis(&self) -> i64 {
        self.grace_period_seconds.saturating_mul(1_000)
    }
}

/// **介入**の種類。どの契機で発せられたか。
///
/// パネルもホットキーも一つであり (AD-6 / AD-7)、種類が変えるのは文言と、二択それぞれが
/// コアで何を起こすかだけである。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InterventionKind {
    /// **休息**に入るよう促す。**連続作業時間**が**休息閾値**に達したとき。
    Rest,
    /// **休息**から戻るよう促す。**休息**に入ってから**休息の長さ**が経ったとき。
    Return,
}

/// 表示中の**介入** (AD-2「介入の表示状態」)。**永続化しない。起動時は常に非表示。**
///
/// **同時に一つしか存在しない。** コアはこれを 1 個のフィールドとして持ち、集合として
/// 持たない — 「二つの介入」を表現する値が無いことが AD-7 の単一性そのものである。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Intervention {
    /// 種類。
    kind: InterventionKind,
    /// 発せられた時刻。
    raised_at: Timestamp,
}

impl Intervention {
    /// 指定の種類を、指定の時刻に発する。
    #[must_use]
    pub const fn raised(kind: InterventionKind, raised_at: Timestamp) -> Self {
        Self { kind, raised_at }
    }

    /// 種類。
    #[must_use]
    pub const fn kind(&self) -> InterventionKind {
        self.kind
    }

    /// 発せられた時刻。
    #[must_use]
    pub const fn at(&self) -> Timestamp {
        self.raised_at
    }
}

/// **介入の選択肢** (CAP-10)。**二つしかない。**
///
/// 変種を増やせば「無視する」「あとで考える」が型として成立してしまう。FR-15 が求める
/// 「いずれかが選ばれるまで消えない」は、**選べるものが二つしかないこと**で表す。
///
/// # 意味は**介入**の種類で決まる
///
/// 応答のホットキーは種類によらず同じ二つを使う (AD-7: 介入ごとのホットキーを作らない)。
/// そのため変種はパネルの「一つ目」「二つ目」に当たり、コアで何を起こすかは表示中の
/// [`InterventionKind`] が決める。
///
/// | 変種 | [`InterventionKind::Rest`] | [`InterventionKind::Return`] |
/// | --- | --- | --- |
/// | `Rest` (`Control + Option + R`) | **休息**に入る | **休息**を終える |
/// | `Grace` (`Control + Option + G`) | **猶予**の後に出直す | もう少し休む |
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InterventionChoice {
    /// 休息の**介入**では**休息**に入る (**現在地**は値を保ったまま**非活性**になり、
    /// 計数が止まる)。復帰の**介入**では**休息**を終える (**活性**へ戻り、計数を
    /// 数え直す)。
    Rest,
    /// 休息の**介入**では**猶予**の後に再提示する (**現在地**は**活性**のまま)。
    /// 復帰の**介入**では**休息の長さ**の後に出直す (**休息**は続く)。
    Grace,
}

/// 一回の刻みが見るすべて。**OS を知らない。**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tick {
    /// いまの**現在地**。**未着手**および**非活性**では計数しない。
    pub position: CurrentPosition,
    /// いまの時刻。
    pub now: Timestamp,
    /// 直前の刻みの時刻。**時計の飛びはこことの差で測る。**
    pub previous: Timestamp,
    /// 刻みの間隔 (ミリ秒)。通常は [`TICK_INTERVAL_MILLIS`]。
    pub interval_millis: i64,
    /// **休息閾値**と**猶予**。
    pub settings: RestSettings,
    /// **猶予**の後に再提示する時刻。猶予が与えられていなければ `None`。
    pub grace_until: Option<Timestamp>,
    /// **介入**が既に表示されているか。**表示中は後発が待つ** (AD-7)。
    pub showing: bool,
}

/// 一回の刻みが決めること。
///
/// **入力と比べて変わった欄だけが書き込みを起こす。** 「何も変わらないなら何も書かない」
/// をコア側で判定できるよう、据え置きを `None` ではなく**入力と同じ値**で表す。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    /// 付け替え後の**連続作業時間**の起点。**未着手**なら `None`。
    ///
    /// 入力の `position.activated_at()` と同じなら、起点は据え置きである。
    pub activated_at: Option<Timestamp>,
    /// 付け替え後の**猶予**の再提示時刻。
    pub grace_until: Option<Timestamp>,
    /// **介入**を発するか。
    pub raise: bool,
}

/// 時計の飛びのうち、スリープに帰せられる長さ (ミリ秒)。
///
/// 刻みの間隔の [`SLEEP_GAP_FACTOR`] 倍を超える飛びだけをスリープとみなし、**期待され
/// ていた 1 回分の間隔を引いた残り**を返す。引かないと、通常の刻みの揺らぎまで
/// 「スリープ」として起点から差し引かれる。
///
/// 飛びが基準に満たなければ 0 である — **短い揺らぎを補正しない。**
#[must_use]
pub fn slept_millis(jump_millis: i64, interval_millis: i64) -> i64 {
    let threshold = interval_millis.saturating_mul(SLEEP_GAP_FACTOR);
    if jump_millis <= threshold {
        return 0;
    }
    jump_millis.saturating_sub(interval_millis).max(0)
}

/// 時刻にミリ秒を足す。[`Timestamp`] は表せる範囲の端に丸める。
fn shifted(at: Timestamp, millis: i64) -> Timestamp {
    Timestamp::from_unix_millis(at.unix_millis().saturating_add(millis))
}

/// **刻みの判断** (AD-8)。**この関数が計時のすべてである。**
///
/// # 規則
///
/// 1. **未着手**および**非活性** (**休息**中) では何も起きない。起点にも**猶予**にも
///    触れず、**介入**も発しない (I/O マトリクス「休息中の計時」「未着手」)。
/// 2. 時計が[はるかに長く飛んで][`slept_millis`]いればスリープである。
///    - 長さが**休息閾値**を超えていれば、**休息**が取られたものとみなして計時を
///      リセットし、**猶予**も解く。**介入**は発しない。
///    - 超えていなければ、その分を起点と**猶予**の双方から差し引く (加算しない)。
/// 3. **介入**を発するのは、**活性**であり、表示中の**介入**が無く、かつ
///    - **猶予**が与えられていれば、その時刻に達したとき
///    - そうでなければ、**連続作業時間**が**休息閾値**に達したとき
///
///   **猶予**が与えられている間は経過時間を見ない。見れば、既に閾値を超えている以上
///   次の刻みで即座に出直してしまい、「15 分後」が意味を失う。
#[must_use]
pub fn decide(tick: &Tick) -> Decision {
    let mut activated_at = tick.position.activated_at();
    let mut grace_until = tick.grace_until;

    // 規則 1 — **休息**中および**未着手**では計数しない。
    if !tick.position.is_active() {
        return Decision {
            activated_at,
            grace_until,
            raise: false,
        };
    }

    // 規則 2 — 時計の飛び。
    let jump = tick
        .now
        .unix_millis()
        .saturating_sub(tick.previous.unix_millis());
    let slept = slept_millis(jump, tick.interval_millis);
    if slept >= tick.settings.rest_threshold_millis() {
        return Decision {
            activated_at: Some(tick.now),
            grace_until: None,
            raise: false,
        };
    }
    if slept > 0 {
        activated_at = activated_at.map(|at| shifted(at, slept));
        grace_until = grace_until.map(|at| shifted(at, slept));
    }

    // 規則 3 — **介入**を発するか。
    let raise = !tick.showing
        && match grace_until {
            Some(deadline) => tick.now >= deadline,
            None => activated_at.is_some_and(|at| {
                tick.now.unix_millis().saturating_sub(at.unix_millis())
                    >= tick.settings.rest_threshold_millis()
            }),
        };

    Decision {
        activated_at,
        grace_until: if raise { None } else { grace_until },
        raise,
    }
}

/// 復帰の刻みが見るすべて。**OS を知らない。**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReturnTick {
    /// **休息**中か。**休息**中でなければ復帰の**介入**は出ない。
    pub resting: bool,
    /// **休息**に入った時刻。**永続化しない** (AD-2)。起動時に**休息**中なら起動時刻。
    pub rest_since: Option<Timestamp>,
    /// いまの時刻。
    pub now: Timestamp,
    /// **休息の長さ**。
    pub settings: RestSettings,
    /// 「もう少し休む」が選ばれた後に出直す時刻。選ばれていなければ `None`。
    pub return_until: Option<Timestamp>,
    /// **介入**が既に表示されているか。**表示中は後発が待つ** (AD-7)。
    pub showing: bool,
}

/// **復帰の介入を発するか** (FR-15)。
///
/// # 規則
///
/// 1. **休息**中でなければ、また**介入**が表示中なら、発しない。
/// 2. 「もう少し休む」の期限があればその時刻に、なければ**休息**に入ってから
///    **休息の長さ**が経った時刻に達したとき発する。
///
/// **実時間で数える。** 直前の刻みとの差を見ない — スリープ中の時間も**休息**に含める
/// (スリープは離席である)。そのため長いスリープから起きた最初の刻みで発する。
///
/// **休息を終わらせない。** この関数が決めるのは促すかどうかだけである。
#[must_use]
pub fn decide_return(tick: &ReturnTick) -> bool {
    if !tick.resting || tick.showing {
        return false;
    }
    let deadline = match (tick.return_until, tick.rest_since) {
        (Some(deadline), _) => deadline,
        (None, Some(since)) => shifted(since, tick.settings.rest_length_millis()),
        (None, None) => return false,
    };
    tick.now >= deadline
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::task::{StepId, TaskId};

    const T0: Timestamp = Timestamp::from_unix_millis(1_789_000_000_000);
    const MINUTE: i64 = 60_000;

    fn active_since(at: Timestamp) -> CurrentPosition {
        CurrentPosition::NotStarted.move_to(TaskId::new(T0), StepId::new(T0), at)
    }

    fn a_tick(position: CurrentPosition, now: Timestamp) -> Tick {
        Tick {
            position,
            now,
            previous: Timestamp::from_unix_millis(
                now.unix_millis().saturating_sub(TICK_INTERVAL_MILLIS),
            ),
            interval_millis: TICK_INTERVAL_MILLIS,
            settings: RestSettings::default(),
            grace_until: None,
            showing: false,
        }
    }

    /// I/O マトリクス「閾値未満」— 49 分では何も起きない。
    #[test]
    fn nothing_happens_before_the_threshold() {
        let decision = decide(&a_tick(
            active_since(T0),
            Timestamp::from_unix_millis(T0.unix_millis() + 49 * MINUTE),
        ));
        assert!(!decision.raise);
        assert_eq!(decision.activated_at, Some(T0), "起点は据え置かれる");
    }

    /// I/O マトリクス「閾値超過」— 50 分で介入が発せられる。
    #[test]
    fn the_threshold_raises_an_intervention() {
        let decision = decide(&a_tick(
            active_since(T0),
            Timestamp::from_unix_millis(T0.unix_millis() + 50 * MINUTE),
        ));
        assert!(decision.raise);
        assert_eq!(
            decision.activated_at,
            Some(T0),
            "発するだけで起点は動かさない"
        );
    }

    /// 既定値が 50 分と 15 分であること (CAP-10)。
    #[test]
    fn the_defaults_are_fifty_and_fifteen_minutes() {
        let settings = RestSettings::default();
        assert_eq!(settings.rest_threshold_seconds(), 50 * 60);
        assert_eq!(settings.grace_period_seconds(), 15 * 60);
    }

    /// 設定テーブルの値が既定値を上書きする (AD-11)。
    #[test]
    fn stored_settings_override_the_constants() {
        let settings = RestSettings::from_settings(&[
            Setting::new(REST_THRESHOLD_KEY, "60"),
            Setting::new(GRACE_PERIOD_KEY, "30"),
        ]);
        assert_eq!(settings.rest_threshold_seconds(), 60);
        assert_eq!(settings.grace_period_seconds(), 30);
    }

    /// 値が無ければコード内の定数である (AD-11)。壊れた値も同じ扱いになる。
    #[test]
    fn missing_or_broken_settings_fall_back_to_the_constants() {
        assert_eq!(RestSettings::from_settings(&[]), RestSettings::default());
        assert_eq!(
            RestSettings::from_settings(&[
                Setting::new(REST_THRESHOLD_KEY, "0"),
                Setting::new(GRACE_PERIOD_KEY, "まもなく"),
            ]),
            RestSettings::default()
        );
    }

    /// I/O マトリクス「休息中の計時」— **非活性**のまま 60 分でも介入は発せられない。
    #[test]
    fn an_inactive_position_is_never_counted() {
        let resting = active_since(T0).deactivate();
        let decision = decide(&a_tick(
            resting,
            Timestamp::from_unix_millis(T0.unix_millis() + 60 * MINUTE),
        ));
        assert!(!decision.raise);
        assert_eq!(decision.activated_at, Some(T0), "起点にも触れない");
    }

    /// I/O マトリクス「未着手」— 計時も介入も起きない。
    #[test]
    fn not_started_is_never_counted() {
        let decision = decide(&a_tick(
            CurrentPosition::NotStarted,
            Timestamp::from_unix_millis(T0.unix_millis() + 60 * MINUTE),
        ));
        assert!(!decision.raise);
        assert_eq!(decision.activated_at, None);
    }

    /// I/O マトリクス「表示中の別の契機」— 表示中は後発が待つ (AD-7)。
    #[test]
    fn a_second_trigger_waits_while_one_is_shown() {
        let mut tick = a_tick(
            active_since(T0),
            Timestamp::from_unix_millis(T0.unix_millis() + 80 * MINUTE),
        );
        tick.showing = true;
        assert!(!decide(&tick).raise, "パネルは一つだけである");

        tick.showing = false;
        assert!(decide(&tick).raise, "閉じれば次が出る");
    }

    /// I/O マトリクス「猶予の後に再提示」— 15 分後に出直す。**それまでは出ない。**
    #[test]
    fn a_grace_period_silences_the_intervention_until_its_deadline() {
        let raised = Timestamp::from_unix_millis(T0.unix_millis() + 50 * MINUTE);
        let deadline = Timestamp::from_unix_millis(raised.unix_millis() + 15 * MINUTE);

        let mut tick = a_tick(
            active_since(T0),
            Timestamp::from_unix_millis(deadline.unix_millis() - MINUTE),
        );
        tick.grace_until = Some(deadline);
        let decision = decide(&tick);
        assert!(!decision.raise, "閾値を超えていても猶予の間は出ない");
        assert_eq!(decision.grace_until, Some(deadline), "猶予は残る");

        // 刻みは 10 秒ごとに来る。**直前の刻みも一緒に進めなければ、時計の飛びとして
        // スリープに見える** — 刻みを模す検査はここを忘れやすい。
        tick.previous = Timestamp::from_unix_millis(deadline.unix_millis() - TICK_INTERVAL_MILLIS);
        tick.now = deadline;
        let decision = decide(&tick);
        assert!(decision.raise, "猶予の時刻に達したら出直す");
        assert_eq!(decision.grace_until, None, "出した猶予は解ける");
    }

    /// I/O マトリクス「繰り返し猶予」— **上限で止まらない。**
    ///
    /// FR-14 (交差介入) の 2 回制限を持ち込まない。休息は本人の身体の話である
    /// (spec Design Notes)。
    #[test]
    fn a_grace_period_can_be_taken_without_limit() {
        let settings = RestSettings::default();
        let mut now = Timestamp::from_unix_millis(T0.unix_millis() + 50 * MINUTE);
        for round in 0..5 {
            let deadline = shifted(now, settings.grace_period_millis());
            let mut tick = a_tick(active_since(T0), deadline);
            tick.grace_until = Some(deadline);
            assert!(decide(&tick).raise, "{round} 回目の猶予の後も出直す");
            now = deadline;
        }
    }

    /// I/O マトリクス「切り替えでの継続」— 40 分で**切り替え**、さらに 10 分で発する。
    ///
    /// **切り替えは起点を動かさない** ([`CurrentPosition::move_to`] / AD-8)。ここが
    /// 逆になると、切り替えを繰り返すほど休息が遠のく。
    #[test]
    fn a_switch_does_not_postpone_the_intervention() {
        let switched = active_since(T0).move_to(
            TaskId::new(T0),
            StepId::new(T0),
            Timestamp::from_unix_millis(T0.unix_millis() + 40 * MINUTE),
        );
        assert_eq!(switched.activated_at(), Some(T0));

        let decision = decide(&a_tick(
            switched,
            Timestamp::from_unix_millis(T0.unix_millis() + 50 * MINUTE),
        ));
        assert!(decision.raise);
    }

    /// I/O マトリクス「スリープ跨ぎ」— 閾値を超えるスリープは休息とみなす。
    #[test]
    fn a_long_sleep_counts_as_rest_and_resets_the_clock() {
        let woke = Timestamp::from_unix_millis(T0.unix_millis() + 120 * MINUTE);
        let mut tick = a_tick(active_since(T0), woke);
        // 直前の刻みは眠る直前である。
        tick.previous = Timestamp::from_unix_millis(T0.unix_millis() + 10 * MINUTE);

        let decision = decide(&tick);
        assert!(!decision.raise, "休息が取られた以上、介入は出ない");
        assert_eq!(decision.activated_at, Some(woke), "計時はリセットされる");
        assert_eq!(decision.grace_until, None);
    }

    /// I/O マトリクス「短いスリープ」— スリープ分は加算せず、計時は継続する。
    #[test]
    fn a_short_sleep_is_discounted_but_does_not_reset_the_clock() {
        // 49 分働き、10 分眠り、起きた直後の刻み。
        let asleep_at = Timestamp::from_unix_millis(T0.unix_millis() + 49 * MINUTE);
        let woke = Timestamp::from_unix_millis(asleep_at.unix_millis() + 10 * MINUTE);
        let mut tick = a_tick(active_since(T0), woke);
        tick.previous = asleep_at;

        let decision = decide(&tick);
        assert!(!decision.raise, "眠った 10 分は連続作業時間ではない");
        let expected = shifted(T0, 10 * MINUTE - TICK_INTERVAL_MILLIS);
        assert_eq!(
            decision.activated_at,
            Some(expected),
            "起点を眠った分ずらす"
        );

        // 起きてから 1 分で 50 分に達する。**計時はリセットされていない。**
        let next = a_tick(
            CurrentPosition::rehydrate(
                TaskId::new(T0),
                StepId::new(T0),
                true,
                decision.activated_at.expect("起点がある"),
            ),
            shifted(woke, MINUTE),
        );
        assert!(decide(&next).raise, "計時は継続する");
    }

    /// 通常の刻みの揺らぎをスリープとして扱わない。
    #[test]
    fn an_ordinary_tick_is_not_mistaken_for_sleep() {
        assert_eq!(slept_millis(TICK_INTERVAL_MILLIS, TICK_INTERVAL_MILLIS), 0);
        assert_eq!(
            slept_millis(
                TICK_INTERVAL_MILLIS * SLEEP_GAP_FACTOR,
                TICK_INTERVAL_MILLIS
            ),
            0,
            "基準ちょうどはスリープではない"
        );
        assert!(
            slept_millis(
                TICK_INTERVAL_MILLIS * SLEEP_GAP_FACTOR + 1,
                TICK_INTERVAL_MILLIS
            ) > 0
        );
    }

    /// 時計が巻き戻っても panic せず、負の補正もしない。
    #[test]
    fn a_backwards_clock_does_not_panic() {
        let mut tick = a_tick(active_since(T0), T0);
        tick.previous = Timestamp::from_unix_millis(T0.unix_millis() + 10 * MINUTE);
        let decision = decide(&tick);
        assert_eq!(decision.activated_at, Some(T0), "起点を前へ動かさない");
        assert!(!decision.raise);
    }

    /// **介入**は同時に一つである — 型が 1 個の値しか持てないこと自体がそれを表す。
    #[test]
    fn an_intervention_remembers_when_it_was_raised() {
        let intervention = Intervention::raised(InterventionKind::Rest, T0);
        assert_eq!(intervention.at(), T0);
        assert_eq!(intervention.kind(), InterventionKind::Rest);
    }

    /// 種類の綴りを固定する。介入パネルが文言を出し分ける鍵である。
    #[test]
    fn the_kinds_keep_their_spelling() {
        assert_eq!(
            serde_json::to_value(InterventionKind::Rest).expect("直列化できる"),
            serde_json::json!("rest")
        );
        assert_eq!(
            serde_json::to_value(InterventionKind::Return).expect("直列化できる"),
            serde_json::json!("return")
        );
    }

    fn a_return_tick(rest_since: Timestamp, now: Timestamp) -> ReturnTick {
        ReturnTick {
            resting: true,
            rest_since: Some(rest_since),
            now,
            settings: RestSettings::default(),
            return_until: None,
            showing: false,
        }
    }

    /// **休息の長さ**の既定は 10 分である。
    #[test]
    fn the_default_rest_length_is_ten_minutes() {
        assert_eq!(RestSettings::default().rest_length_seconds(), 10 * 60);
    }

    /// I/O マトリクス「設定」— `rest_length_seconds` が既定値を上書きし、0 以下・数値で
    /// なければ 600 秒に落ちる (AD-11)。
    #[test]
    fn the_rest_length_is_read_from_the_settings() {
        assert_eq!(
            RestSettings::from_settings(&[Setting::new(REST_LENGTH_KEY, "300")])
                .rest_length_seconds(),
            300
        );
        for broken in ["0", "-60", "しばらく", ""] {
            assert_eq!(
                RestSettings::from_settings(&[Setting::new(REST_LENGTH_KEY, broken)])
                    .rest_length_seconds(),
                600,
                "{broken:?} は既定へ落ちる"
            );
        }
    }

    /// I/O マトリクス「復帰を促す」— 休息に入って 10 分で出る。それより前は出ない。
    #[test]
    fn the_return_intervention_comes_after_the_rest_length() {
        assert!(!decide_return(&a_return_tick(
            T0,
            Timestamp::from_unix_millis(T0.unix_millis() + 10 * MINUTE - 1)
        )));
        assert!(decide_return(&a_return_tick(
            T0,
            Timestamp::from_unix_millis(T0.unix_millis() + 10 * MINUTE)
        )));
    }

    /// **休息**中でなければ、また表示中なら出ない (AD-7)。
    #[test]
    fn the_return_intervention_needs_a_rest_and_an_empty_panel() {
        let now = Timestamp::from_unix_millis(T0.unix_millis() + 30 * MINUTE);
        let mut tick = a_return_tick(T0, now);
        tick.resting = false;
        assert!(!decide_return(&tick), "休息中でなければ出ない");

        let mut tick = a_return_tick(T0, now);
        tick.showing = true;
        assert!(!decide_return(&tick), "表示中は後発が待つ");
    }

    /// 「もう少し休む」の期限があれば、それまで出ない。**開始からの経過を見ない。**
    #[test]
    fn more_rest_waits_for_its_own_deadline() {
        let now = Timestamp::from_unix_millis(T0.unix_millis() + 15 * MINUTE);
        let mut tick = a_return_tick(T0, now);
        tick.return_until = Some(Timestamp::from_unix_millis(T0.unix_millis() + 20 * MINUTE));
        assert!(!decide_return(&tick), "期限の前は出ない");

        tick.now = Timestamp::from_unix_millis(T0.unix_millis() + 20 * MINUTE);
        assert!(decide_return(&tick), "期限で出直す");
    }

    /// I/O マトリクス「スリープ」— 実時間で数える。**時計の飛びを差し引かない。**
    #[test]
    fn sleep_counts_as_rest() {
        // 休息に入って 1 分で眠り、30 分後に起きた最初の刻み。
        assert!(decide_return(&a_return_tick(
            T0,
            Timestamp::from_unix_millis(T0.unix_millis() + 31 * MINUTE)
        )));
    }

    /// 休息の開始時刻が分からなければ出さない (起きないはずの状態で暴れない)。
    #[test]
    fn an_unknown_start_raises_nothing() {
        let mut tick = a_return_tick(
            T0,
            Timestamp::from_unix_millis(T0.unix_millis() + 60 * MINUTE),
        );
        tick.rest_since = None;
        assert!(!decide_return(&tick));
    }

    /// **選択肢は二つだけである。** 境界へ出る綴りも固定する。
    #[test]
    fn the_choices_are_exactly_two() {
        assert_eq!(
            serde_json::to_value(InterventionChoice::Rest).expect("直列化できる"),
            serde_json::json!("rest")
        );
        assert_eq!(
            serde_json::to_value(InterventionChoice::Grace).expect("直列化できる"),
            serde_json::json!("grace")
        );
    }
}
