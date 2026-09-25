//! ホットキーアダプタ — グローバルホットキーの登録とトグル処理 (CAP-1, AD-7)。
//!
//! `tauri-plugin-global-shortcut` のハンドラは 1 回の押下につき `Pressed` と `Released`
//! の 2 回発火する (上流で `not_planned` として恒久化済み)。`Pressed` のみを処理し
//! `Released` を破棄しないと、1 押下でトグルが 2 回起きて何も起きなかったように見える。
//! この判定は [`should_toggle`] に純粋関数として切り出し、OS を起動せずに検証する。
//!
//! # 応答のホットキーは一時登録である (AD-7)
//!
//! **介入**は非活性パネルであり、キーボードを受け取れない。二択の応答をマウスに強制
//! しないため、**表示のときに登録し、応答で解除する**グローバルホットキーを二つ持つ
//! ([`register_response`] / [`unregister_response`])。
//!
//! **ハンドラの中から登録・解除・照会を呼んではならない。** プラグインは `shortcuts` の
//! 錠を保持したままハンドラを呼ぶため、その中で同じ錠に再入するとメインスレッドが
//! 停止する。`run_on_main_thread` で包んでも、既にメインスレッド上なら即時実行される
//! ため救われない。**別のスレッドへ退避してから呼ぶ** — それを行うのは呼び出し側
//! (`lib.rs`) である。

use tauri::{AppHandle, Runtime};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::adapters::{intervention, presentation};
use crate::domain::rest::InterventionChoice;

/// 利用者に示すためのアクセラレータ表記。
///
/// Ctrl+Option+Space を選ぶ理由: Spotlight (Cmd+Space) とも入力ソース切替
/// (Ctrl+Space / Cmd+Space) とも衝突しない。
pub const ACCELERATOR: &str = "Control + Option + Space";

/// 「**休息**に入る」の表記 (CAP-10)。
pub const REST_ACCELERATOR: &str = "Control + Option + R";

/// 「**猶予**の後に再提示する」の表記 (CAP-10)。
pub const GRACE_ACCELERATOR: &str = "Control + Option + G";

/// オーバーレイを呼び出すグローバルホットキー。
fn overlay_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::Space)
}

/// 「**休息**に入る」を受けるホットキー。**介入の表示中だけ登録される。**
fn rest_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyR)
}

/// 「**猶予**の後に再提示する」を受けるホットキー。**介入の表示中だけ登録される。**
fn grace_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyG)
}

/// ホットキー登録の結果。登録に失敗しても常駐は止めないため、失敗は値として持ち回る。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyStatus {
    /// 利用者に示すアクセラレータ表記。
    pub accelerator: String,
    /// 登録に成功したか。
    pub registered: bool,
    /// 失敗した場合の理由 (利用者に示す)。
    pub error: Option<String>,
}

impl HotkeyStatus {
    pub fn registered() -> Self {
        Self {
            accelerator: ACCELERATOR.to_string(),
            registered: true,
            error: None,
        }
    }

    pub fn failed(error: String) -> Self {
        Self {
            accelerator: ACCELERATOR.to_string(),
            registered: false,
            error: Some(error),
        }
    }
}

/// 応答のホットキーの登録結果 (AD-7)。
///
/// **登録に失敗しても介入は出す。** 失敗を `Result` ではなく値として持ち回るのは
/// [`HotkeyStatus`] と同じ理由である — 呼び出し側はこれを受け取るしかなく、`?` で
/// 介入を抑える書き方が成立しない。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseHotkeyStatus {
    /// 「**休息**に入る」の表記。
    pub rest_accelerator: String,
    /// 「**猶予**の後に再提示する」の表記。
    pub grace_accelerator: String,
    /// 二つとも登録できたか。
    pub registered: bool,
    /// 失敗した場合の理由 (利用者に示す)。
    pub error: Option<String>,
}

impl ResponseHotkeyStatus {
    /// 二つとも登録できた。
    #[must_use]
    pub fn registered() -> Self {
        Self {
            rest_accelerator: REST_ACCELERATOR.to_string(),
            grace_accelerator: GRACE_ACCELERATOR.to_string(),
            registered: true,
            error: None,
        }
    }

    /// 登録できなかった。**それでも介入は出る** — クリックだけで応答できる。
    #[must_use]
    pub fn failed(error: String) -> Self {
        Self {
            rest_accelerator: REST_ACCELERATOR.to_string(),
            grace_accelerator: GRACE_ACCELERATOR.to_string(),
            registered: false,
            error: Some(error),
        }
    }
}

/// **発火を応答へ変える純粋関数** (AD-7)。
///
/// `Pressed` のみを処理する — 1 押下につきハンドラは 2 回発火するため、`Released` を
/// 処理すると**介入**が押下の瞬間に二重確定する。
///
/// **どちらでもない発火・両方に一致する発火はいずれも `None` である。** 後者は
/// 「二つの介入が同じキーを登録する」事故の形そのものであり、意味を一つ選ぶより
/// 何もしないほうが安全である。
#[must_use]
pub fn response_choice(
    state: ShortcutState,
    is_rest_shortcut: bool,
    is_grace_shortcut: bool,
) -> Option<InterventionChoice> {
    if state != ShortcutState::Pressed {
        return None;
    }
    match (is_rest_shortcut, is_grace_shortcut) {
        (true, false) => Some(InterventionChoice::Rest),
        (false, true) => Some(InterventionChoice::Grace),
        _ => None,
    }
}

/// ハンドラの 1 回の発火を処理すべきか判定する純粋関数。
///
/// - `state` — プラグインが渡す押下/離鍵の別
/// - `is_overlay_shortcut` — 発火したショートカットがオーバーレイ用のものか
///
/// `Pressed` かつ対象のショートカットのときだけ真を返す。1 押下 = 1 トグル (AD-7)。
pub fn should_toggle(state: ShortcutState, is_overlay_shortcut: bool) -> bool {
    is_overlay_shortcut && state == ShortcutState::Pressed
}

/// ホットキーを登録する。
///
/// 失敗しても `Err` を返さない。登録失敗を理由に常駐を止めないことが要求であり、
/// 「止めない」を呼び出し側の規律ではなく型で保証するためである。呼び出し側はこの値を
/// 受け取るしかなく、`?` で常駐を落とす書き方が成立しない。
pub fn register<R: Runtime>(app: &AppHandle<R>) -> HotkeyStatus {
    let shortcut = overlay_shortcut();
    let rest = rest_shortcut();
    let grace = grace_shortcut();

    // ショートカットはプラグイン登録と切り離して登録する。Builder::with_shortcut で
    // 登録するとショートカットの登録失敗がプラグイン登録ごと失敗させ、後から
    // 再登録する手段まで失われるため。
    //
    // **ハンドラは一つである。** 応答のホットキーは後から一時登録されるが、プラグインの
    // ハンドラを差し替える手段は無く、また差し替えられては困る — 介入ごとに別の
    // ハンドラを持てば、先に応答したほうが他方を解除する事故の余地が生まれる (AD-7)。
    let plugin = tauri_plugin_global_shortcut::Builder::new()
        .with_handler(move |app, triggered, event| {
            if should_toggle(event.state, triggered == &shortcut) {
                if let Err(error) = presentation::toggle(app) {
                    log::error!("failed to toggle the overlay: {error}");
                }
                return;
            }

            let Some(choice) =
                response_choice(event.state, triggered == &rest, triggered == &grace)
            else {
                return;
            };
            // **この中から登録・解除・照会を呼んではならない。** プラグインは同じ錠を
            // 保持したままここを呼ぶ。応答は別のスレッドへ退避してから行う (AD-7)。
            intervention::answer_off_thread(app.clone(), choice);
        })
        .build();

    if let Err(error) = app.plugin(plugin) {
        return HotkeyStatus::failed(format!(
            "グローバルホットキーのプラグインを初期化できなかった: {error}"
        ));
    }

    match app.global_shortcut().register(overlay_shortcut()) {
        Ok(()) => HotkeyStatus::registered(),
        Err(error) => HotkeyStatus::failed(format!(
            "{ACCELERATOR} を登録できなかった (他のアプリケーションが同じキーを保持している可能性がある): {error}"
        )),
    }
}

/// 応答のホットキーを**一時登録する** (AD-7)。**介入を表示するときに呼ぶ。**
///
/// 失敗しても `Err` を返さない。**登録失敗を理由に介入を出さないことは禁じられている** —
/// 「出す」を呼び出し側の規律ではなく型で保証するため、失敗は値として返す。
pub fn register_response<R: Runtime>(app: &AppHandle<R>) -> ResponseHotkeyStatus {
    let shortcuts = app.global_shortcut();
    let mut failures = Vec::new();
    for (shortcut, accelerator) in [
        (rest_shortcut(), REST_ACCELERATOR),
        (grace_shortcut(), GRACE_ACCELERATOR),
    ] {
        if let Err(error) = shortcuts.register(shortcut) {
            failures.push(format!("{accelerator}: {error}"));
        }
    }

    if failures.is_empty() {
        return ResponseHotkeyStatus::registered();
    }
    ResponseHotkeyStatus::failed(format!(
        "応答のホットキーを登録できなかった (他のアプリケーションが同じ打鍵を保持している可能性がある) — {}",
        failures.join(" / ")
    ))
}

/// 応答のホットキーを解除する (AD-7)。**介入を閉じるときに呼ぶ。**
///
/// **登録できていなかったものの解除も失敗ではない。** 登録は部分的に成功しうるため、
/// 解除は常に両方に対して試み、結果を記録するだけにする — 解除漏れは、介入が無いのに
/// 打鍵が効く状態を残す。
pub fn unregister_response<R: Runtime>(app: &AppHandle<R>) {
    let shortcuts = app.global_shortcut();
    for (shortcut, accelerator) in [
        (rest_shortcut(), REST_ACCELERATOR),
        (grace_shortcut(), GRACE_ACCELERATOR),
    ] {
        if let Err(error) = shortcuts.unregister(shortcut) {
            log::info!("`{accelerator}` was not registered when it was released: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::presentation::{toggle_action, visibility_after};

    /// ハンドラに届いた一連の発火を畳み込み、(最終的な可視状態, トグル回数) を返す。
    /// OS を介さずに「押下 1 回で何回トグルが起きるか」だけを見る。
    fn fold(events: &[(ShortcutState, bool)], initially_visible: bool) -> (bool, usize) {
        let mut visible = initially_visible;
        let mut toggles = 0;
        for &(state, is_overlay_shortcut) in events {
            if should_toggle(state, is_overlay_shortcut) {
                visible = visibility_after(toggle_action(visible));
                toggles += 1;
            }
        }
        (visible, toggles)
    }

    /// 1 押下がハンドラに与える発火列。プラグインは押下と離鍵の 2 回発火する。
    const ONE_KEYPRESS: [(ShortcutState, bool); 2] = [
        (ShortcutState::Pressed, true),
        (ShortcutState::Released, true),
    ];

    /// I/O マトリクス「呼び出し」— 非表示のときに 1 押下すると表示になる。
    #[test]
    fn one_keypress_shows_a_hidden_overlay() {
        assert_eq!(fold(&ONE_KEYPRESS, false), (true, 1));
    }

    /// I/O マトリクス「トグル」— 表示中に 1 押下すると非表示になる。
    #[test]
    fn one_keypress_hides_a_visible_overlay() {
        assert_eq!(fold(&ONE_KEYPRESS, true), (false, 1));
    }

    /// I/O マトリクス「二重発火の抑止」— Released を処理するとトグルが 2 回起き、
    /// 1 押下で元の状態に戻ってしまう。Pressed のみを処理していることを固定する。
    #[test]
    fn released_does_not_toggle() {
        assert!(!should_toggle(ShortcutState::Released, true));

        let (visible, toggles) = fold(&ONE_KEYPRESS, false);
        assert_eq!(toggles, 1, "1 押下につきトグルは 1 回だけ起きること");
        assert!(visible, "Released を処理していれば非表示に戻ってしまう");
    }

    /// 別のショートカットの発火ではトグルしない。
    #[test]
    fn other_shortcuts_are_ignored() {
        assert!(!should_toggle(ShortcutState::Pressed, false));
        assert_eq!(fold(&[(ShortcutState::Pressed, false)], false), (false, 0));
    }

    /// I/O マトリクス「ホットキー衝突」— 登録に失敗しても常駐を止めない。
    ///
    /// `register` の戻り値が `Result` ではなく [`HotkeyStatus`] であることが、
    /// 「登録失敗を理由に常駐を止めない」を型で保証している。失敗が値として表現され、
    /// かつ利用者に示すための情報 (アクセラレータ表記と理由) を保持することを固定する。
    #[test]
    fn a_failed_registration_is_a_value_not_an_error() {
        let status = HotkeyStatus::failed("他のアプリケーションが保持している".to_string());

        assert!(
            !status.registered,
            "失敗は registered=false として表現される"
        );
        assert_eq!(
            status.accelerator, ACCELERATOR,
            "失敗時もアクセラレータ表記は保持し、利用者に何が使えなかったかを示せること"
        );
        assert!(
            status.error.is_some(),
            "利用者に示す理由を保持すること (起動時のオーバーレイとメニューバー項目の状態行が伝達経路)"
        );
    }

    /// 成功時は理由を持たない。
    #[test]
    fn a_successful_registration_carries_no_error() {
        let status = HotkeyStatus::registered();
        assert!(status.registered);
        assert!(status.error.is_none());
        assert_eq!(status.accelerator, ACCELERATOR);
    }

    /// 表示文字列 [`ACCELERATOR`] と、実際に登録するキーが一致していること。
    ///
    /// 両者は別々に書かれており、片方だけ変えても コンパイル は通る。ずれると
    /// オーバーレイとメニューバーの状態行が、押しても効かないキーを利用者に示す。
    /// 実際の [`Shortcut`] から表示文字列を組み立て直して突き合わせる。
    #[test]
    fn the_displayed_accelerator_matches_the_registered_shortcut() {
        fn render(shortcut: &Shortcut) -> String {
            let mut parts = Vec::new();
            if shortcut.mods.contains(Modifiers::CONTROL) {
                parts.push("Control");
            }
            if shortcut.mods.contains(Modifiers::ALT) {
                parts.push("Option");
            }
            if shortcut.mods.contains(Modifiers::SHIFT) {
                parts.push("Shift");
            }
            if shortcut.mods.contains(Modifiers::SUPER) {
                parts.push("Command");
            }
            parts.push(match shortcut.key {
                Code::Space => "Space",
                other => panic!("表示名を持たないキーに変更されている: {other:?}"),
            });
            parts.join(" + ")
        }

        assert_eq!(render(&overlay_shortcut()), ACCELERATOR);
    }

    // --- 応答のホットキー (AD-7 / CAP-10) ---------------------------------------

    /// 二つの打鍵がそれぞれの選択肢へ繋がっている。**取り違えは検出されにくい。**
    #[test]
    fn each_response_shortcut_maps_to_its_own_choice() {
        assert_eq!(
            response_choice(ShortcutState::Pressed, true, false),
            Some(InterventionChoice::Rest)
        );
        assert_eq!(
            response_choice(ShortcutState::Pressed, false, true),
            Some(InterventionChoice::Grace)
        );
    }

    /// **`Released` は破棄する** (AD-7)。処理すれば 1 押下で介入が二重確定する。
    #[test]
    fn a_released_response_is_discarded() {
        assert_eq!(response_choice(ShortcutState::Released, true, false), None);
        assert_eq!(response_choice(ShortcutState::Released, false, true), None);
    }

    /// どちらでもない発火は何も起こさない。
    #[test]
    fn an_unrelated_shortcut_is_not_a_response() {
        assert_eq!(response_choice(ShortcutState::Pressed, false, false), None);
    }

    /// **両方に一致する発火も何も起こさない。**
    ///
    /// それは「二つの介入が同じキーを登録した」形そのものである (AD-7)。意味を一つ
    /// 選ぶより、何もしないほうが安全である。
    #[test]
    fn an_ambiguous_shortcut_chooses_nothing() {
        assert_eq!(response_choice(ShortcutState::Pressed, true, true), None);
    }

    /// 応答の打鍵はオーバーレイの打鍵と別である。**同じなら呼び出しが応答に化ける。**
    #[test]
    fn the_response_shortcuts_differ_from_the_overlay_shortcut() {
        let overlay = overlay_shortcut();
        assert_ne!(rest_shortcut(), overlay);
        assert_ne!(grace_shortcut(), overlay);
        assert_ne!(rest_shortcut(), grace_shortcut());
    }

    /// 表示文字列と実際に登録するキーが一致していること
    /// ([`the_displayed_accelerator_matches_the_registered_shortcut`] の双子)。
    ///
    /// ずれると、パネルが押しても効かない打鍵を案内する。
    #[test]
    fn the_displayed_response_accelerators_match_the_registered_shortcuts() {
        fn render(shortcut: &Shortcut) -> String {
            let mut parts = Vec::new();
            if shortcut.mods.contains(Modifiers::CONTROL) {
                parts.push("Control".to_string());
            }
            if shortcut.mods.contains(Modifiers::ALT) {
                parts.push("Option".to_string());
            }
            parts.push(match shortcut.key {
                Code::KeyR => "R".to_string(),
                Code::KeyG => "G".to_string(),
                other => panic!("表示名を持たないキーに変更されている: {other:?}"),
            });
            parts.join(" + ")
        }

        assert_eq!(render(&rest_shortcut()), REST_ACCELERATOR);
        assert_eq!(render(&grace_shortcut()), GRACE_ACCELERATOR);
    }

    /// I/O マトリクス「ホットキー登録の失敗」— **失敗は値であり、介入は出る。**
    ///
    /// 戻り値が `Result` ではないことが「登録失敗を理由に介入を抑えない」を型で
    /// 保証している。失敗時も表記を保つのは、パネルが何を案内できないかを述べるためで
    /// ある。
    #[test]
    fn a_failed_response_registration_is_a_value_not_an_error() {
        let status = ResponseHotkeyStatus::failed("衝突".to_string());

        assert!(!status.registered);
        assert_eq!(status.rest_accelerator, REST_ACCELERATOR);
        assert_eq!(status.grace_accelerator, GRACE_ACCELERATOR);
        assert!(status.error.is_some());
    }

    /// 成功時は理由を持たない。
    #[test]
    fn a_successful_response_registration_carries_no_error() {
        let status = ResponseHotkeyStatus::registered();
        assert!(status.registered);
        assert!(status.error.is_none());
    }

    /// 2 押下で元に戻る。
    #[test]
    fn two_keypresses_return_to_the_original_state() {
        let mut events = ONE_KEYPRESS.to_vec();
        events.extend_from_slice(&ONE_KEYPRESS);
        assert_eq!(fold(&events, false), (false, 2));
    }
}
