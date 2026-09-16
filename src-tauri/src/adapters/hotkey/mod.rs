//! ホットキーアダプタ — グローバルホットキーの登録とトグル処理 (CAP-1, AD-7)。
//!
//! `tauri-plugin-global-shortcut` のハンドラは 1 回の押下につき `Pressed` と `Released`
//! の 2 回発火する (上流で `not_planned` として恒久化済み)。`Pressed` のみを処理し
//! `Released` を破棄しないと、1 押下でトグルが 2 回起きて何も起きなかったように見える。
//! この判定は [`should_toggle`] に純粋関数として切り出し、OS を起動せずに検証する。

use tauri::{AppHandle, Runtime};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::adapters::presentation;

/// 利用者に示すためのアクセラレータ表記。
///
/// Ctrl+Option+Space を選ぶ理由: Spotlight (Cmd+Space) とも入力ソース切替
/// (Ctrl+Space / Cmd+Space) とも衝突しない。
pub const ACCELERATOR: &str = "Control + Option + Space";

/// オーバーレイを呼び出すグローバルホットキー。
fn overlay_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::Space)
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

    // ショートカットはプラグイン登録と切り離して登録する。Builder::with_shortcut で
    // 登録するとショートカットの登録失敗がプラグイン登録ごと失敗させ、後から
    // 再登録する手段まで失われるため。
    let plugin = tauri_plugin_global_shortcut::Builder::new()
        .with_handler(move |app, triggered, event| {
            if !should_toggle(event.state, triggered == &shortcut) {
                return;
            }
            if let Err(error) = presentation::toggle(app) {
                log::error!("failed to toggle the overlay: {error}");
            }
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

    /// 2 押下で元に戻る。
    #[test]
    fn two_keypresses_return_to_the_original_state() {
        let mut events = ONE_KEYPRESS.to_vec();
        events.extend_from_slice(&ONE_KEYPRESS);
        assert_eq!(fold(&events, false), (false, 2));
    }
}
