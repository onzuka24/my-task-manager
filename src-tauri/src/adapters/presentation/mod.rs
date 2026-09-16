//! 提示アダプタ — オーバーレイの表示・非表示とフォーカス復帰 (AD-6)。
//!
//! オーバーレイは起動時に生成して隠したままにする。ホットキー押下時に生成しないのは
//! CAP-1 の 300ms 制約を満たすためである (tauri.conf.json の `visible: false`)。
//!
//! ここは OS 呼び出しを行う層であるため、「表示中かどうか」から「次に何をするか」を
//! 決める判断だけを純粋関数 [`toggle_action`] に切り出してある。判断の正しさは OS を
//! 起動せずに単体テストで確認できる。

use tauri::{AppHandle, Manager, Runtime, WebviewWindow};

/// オーバーレイウィンドウのラベル。tauri.conf.json の `app.windows[].label` と一致する。
pub const OVERLAY_LABEL: &str = "main";

/// オーバーレイに対して次に行う操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayAction {
    /// 表示してフォーカスを与える。
    Show,
    /// 隠して、直前に最前面だったアプリケーションへフォーカスを返す。
    Hide,
}

/// 現在の可視状態から、トグルで行うべき操作を決める。
///
/// OS 呼び出しを含まない純粋関数である。1 押下につき 1 回だけ呼ばれることは呼び出し側
/// (ホットキーアダプタ) の責務であり、ここでは判断しない。
pub const fn toggle_action(is_visible: bool) -> OverlayAction {
    if is_visible {
        OverlayAction::Hide
    } else {
        OverlayAction::Show
    }
}

/// Esc が行う操作。トグルではなく、現在の可視状態を問わず常に [`OverlayAction::Hide`]。
///
/// トグル ([`toggle_action`]) と対になる。両者を取り違えると、隠れている状態で Esc 経路に
/// 入ったときにオーバーレイが開いてしまう。
pub const fn escape_action() -> OverlayAction {
    OverlayAction::Hide
}

/// [`OverlayAction`] を適用した後の可視状態。テストと実装で同じ規則を使うための純粋関数。
pub const fn visibility_after(action: OverlayAction) -> bool {
    matches!(action, OverlayAction::Show)
}

/// オーバーレイウィンドウを取得する。
pub fn overlay<R: Runtime>(app: &AppHandle<R>) -> Option<WebviewWindow<R>> {
    app.get_webview_window(OVERLAY_LABEL)
}

/// 決定済みの操作を実際のウィンドウへ適用し、適用後の可視状態を返す。
pub fn apply<R: Runtime>(app: &AppHandle<R>, action: OverlayAction) -> tauri::Result<bool> {
    let Some(window) = overlay(app) else {
        log::error!("overlay window `{OVERLAY_LABEL}` not found");
        return Ok(false);
    };
    match action {
        OverlayAction::Show => {
            window.show()?;
            // Accessory (Dock 非表示) では show() だけでは前面に来ずフォーカスも得ない。
            window.set_focus()?;
        }
        OverlayAction::Hide => {
            window.hide()?;
            // ウィンドウを隠しても macOS はアプリの活性を手放さない (tauri#7540)。
            // NSApplication の hide が、直前に最前面だったアプリへ活性を返す唯一の経路。
            #[cfg(target_os = "macos")]
            app.hide()?;
        }
    }
    Ok(visibility_after(action))
}

/// 表示中なら隠し、隠れているなら表示する。適用後の可視状態を返す。
pub fn toggle<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<bool> {
    let Some(window) = overlay(app) else {
        log::error!("overlay window `{OVERLAY_LABEL}` not found");
        return Ok(false);
    };
    apply(app, toggle_action(window.is_visible()?))
}

/// 表示してフォーカスを与える。
pub fn show<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<bool> {
    apply(app, OverlayAction::Show)
}

/// 隠して、直前に最前面だったアプリケーションへフォーカスを返す。
pub fn hide<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<bool> {
    apply(app, escape_action())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// I/O マトリクス「呼び出し」— 非表示のときは表示側へ倒す。
    #[test]
    fn hidden_overlay_is_shown() {
        assert_eq!(toggle_action(false), OverlayAction::Show);
        assert!(visibility_after(OverlayAction::Show));
    }

    /// I/O マトリクス「トグル」— 表示中のホットキーは非表示へ倒す。
    #[test]
    fn visible_overlay_is_hidden() {
        assert_eq!(toggle_action(true), OverlayAction::Hide);
        assert!(!visibility_after(OverlayAction::Hide));
    }

    /// I/O マトリクス「閉じる」— Esc はトグルではなく常に Hide である。
    ///
    /// 可視状態に関わらず Hide であること、そして同じ状態で `toggle_action` とは
    /// 結果が分かれることの両方を固定する。後者がないと「Esc がトグルになっていない」
    /// ことを検証したことにならない。
    #[test]
    fn escape_always_hides_regardless_of_visibility() {
        for visible in [true, false] {
            assert_eq!(
                escape_action(),
                OverlayAction::Hide,
                "可視状態 {visible} でも Esc は Hide であること"
            );
            assert!(!visibility_after(escape_action()));
        }
        // 隠れている状態では、トグルは Show・Esc は Hide と結果が分かれる。
        assert_eq!(toggle_action(false), OverlayAction::Show);
        assert_ne!(escape_action(), toggle_action(false));
    }

    /// トグルは 2 回で元に戻る。
    #[test]
    fn toggle_is_an_involution() {
        let mut visible = false;
        for _ in 0..2 {
            visible = visibility_after(toggle_action(visible));
        }
        assert!(!visible);
    }
}
