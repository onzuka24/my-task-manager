//! **設定値** (`Setting`) — 永続化層に一元化されるユーザー設定 (AD-11)。
//!
//! 別建ての設定ファイルを持たない。値は鍵と文字列の組であり、意味付けは読み手が行う
//! ([`super::rest::RestSettings`])。
//!
//! # なぜ型付きの構造体を保存の単位にしないのか
//!
//! 設定は AD-11 が「休息閾値・猶予・腐敗判定期間を含む」と述べるとおり、v2 で増える。
//! 保存の単位を型付きの構造体にすると、欄が増えるたびに永続化の形が変わり、既存の DB を
//! 読めなくなる版が生まれる。**鍵と文字列の組を運び、解釈は読み手に閉じる。**
//!
//! # 値が無いことは失敗ではない
//!
//! 既定値はコード内の定数であり、DB に値が無い場合のフォールバックである
//! (スパイン「一貫性の規約」)。**初回起動の DB は空であり、それが正常な状態である。**

use std::fmt;

/// 一つの**設定値**。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Setting {
    key: String,
    value: String,
}

impl Setting {
    /// 鍵と値から作る。
    #[must_use]
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }

    /// 鍵。
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// 値。**文字列のまま運ぶ。** 解釈は読み手が行う。
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

impl fmt::Display for Setting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}={}", self.key, self.value)
    }
}

/// 鍵で引く。**同じ鍵が二つ現れることは永続化層 (主キー) が禁じている。**
#[must_use]
pub fn find<'a>(settings: &'a [Setting], key: &str) -> Option<&'a str> {
    settings
        .iter()
        .find(|setting| setting.key() == key)
        .map(Setting::value)
}

/// 正の秒数として読む。**読めない値・0 以下の値は「無い」と同じ扱いにする。**
///
/// 0 や負の期間を受け入れると、**休息閾値**が 0 のときに刻みのたびに**介入**が発せられ、
/// 応答するまで画面から離れられなくなる。**壊れた設定値で常駐を止めもしない** — 既定値へ
/// 落ちるほうが、唯一の能動機能が黙って死ぬより軽い (AD-11)。
#[must_use]
pub fn positive_seconds(settings: &[Setting], key: &str) -> Option<i64> {
    let text = find(settings, key)?;
    let seconds: i64 = text.trim().parse().ok()?;
    (seconds > 0).then_some(seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_setting_list() -> Vec<Setting> {
        vec![
            Setting::new("rest_threshold_seconds", "1500"),
            Setting::new("grace_period_seconds", " 300 "),
            Setting::new("broken", "まもなく"),
            Setting::new("zero", "0"),
            Setting::new("negative", "-60"),
        ]
    }

    /// 鍵で引ける。
    #[test]
    fn a_stored_value_is_found_by_its_key() {
        let settings = a_setting_list();
        assert_eq!(find(&settings, "rest_threshold_seconds"), Some("1500"));
        assert_eq!(find(&settings, "missing"), None);
    }

    /// 前後の空白を除いて読む。手で `sqlite3` から入れた値が空白で死なない。
    #[test]
    fn surrounding_spaces_do_not_break_a_value() {
        assert_eq!(
            positive_seconds(&a_setting_list(), "grace_period_seconds"),
            Some(300)
        );
    }

    /// **読めない値・0 以下は「無い」と同じ扱いである。**
    ///
    /// 0 を受け入れれば、刻みのたびに介入が発せられる。
    #[test]
    fn a_nonsense_duration_falls_back_to_absence() {
        let settings = a_setting_list();
        for key in ["broken", "zero", "negative", "missing"] {
            assert_eq!(
                positive_seconds(&settings, key),
                None,
                "`{key}` は既定値へ落ちること"
            );
        }
    }
}
