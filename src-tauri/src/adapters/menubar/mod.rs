//! メニューバーアダプタ — 常在する可視面と、唯一の明示的な終了経路 (CAP-3)。
//!
//! `LSUIElement` を立てたアプリは自前のメニューバーを表示しないため、ここで言う
//! 「メニューバー項目」は `NSStatusItem` = Tauri の tray である。
//!
//! メニューの中身は 2 つだけである — 「終了」と、ホットキーの現在状態を示す非活性の
//! 1 行。バッジ・件数・進捗・タスク一覧を出してはならない (AD-15、SPEC.md 非目標)。
//!
//! **`PredefinedMenuItem::quit` を使ってはならない。** `NSApplication terminate:` を
//! 直接送り、`RunEvent::ExitRequested` を迂回するため、誤終了の阻止機構ごと素通りする。
//! 明示的な終了は独自の `MenuItem` から [`tauri::AppHandle::exit`] を呼んで表現する。

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Runtime};

use crate::adapters::hotkey::HotkeyStatus;

/// 「終了」項目の id。
pub const QUIT_ITEM_ID: &str = "quit";
/// ホットキーの状態を示す非活性の 1 行の id。
pub const HOTKEY_STATUS_ITEM_ID: &str = "hotkey-status";

/// メニューバー用の単色テンプレートアイコン。
///
/// 多色のアプリアイコンをテンプレートとして指定すると、macOS はアルファだけを見るため
/// 黒い塊に潰れる。専用の 1 枚を用意してある。
const TEMPLATE_ICON: &[u8] = include_bytes!("../../../icons/menubar-template.png");

/// ホットキーの状態を示す 1 行の文言を組み立てる純粋関数。
///
/// 起動時のオーバーレイは一時的な面であり、フォーカスを失えば消える。この行は
/// 「以後いつでも確認できる副の経路」であるため、成功と失敗が見分けられなければ
/// 役に立たない。
pub fn status_line(status: &HotkeyStatus) -> String {
    if status.registered {
        format!("ホットキー {} — 有効", status.accelerator)
    } else {
        format!("ホットキー {} — 登録できなかった", status.accelerator)
    }
}

/// メニューバー項目を設置する。
///
/// 戻り値の [`TrayIcon`] は Tauri のリソーステーブルに登録されるため、呼び出し側が
/// 保持しなくても消えない。
pub fn install<R: Runtime>(
    app: &AppHandle<R>,
    status: &HotkeyStatus,
) -> tauri::Result<TrayIcon<R>> {
    // 非活性の 1 行。アクセラレータは持たせない。
    let hotkey_line = MenuItem::with_id(
        app,
        HOTKEY_STATUS_ITEM_ID,
        status_line(status),
        false,
        None::<&str>,
    )?;
    // 終了にもアクセラレータを与えない。Cmd+Q は無効化した対象そのものである。
    let quit = MenuItem::with_id(app, QUIT_ITEM_ID, "終了", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&hotkey_line, &quit])?;

    TrayIconBuilder::new()
        .icon(Image::from_bytes(TEMPLATE_ICON)?)
        .icon_as_template(true)
        .tooltip("My Task Manager")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            if event.id.as_ref() == QUIT_ITEM_ID {
                log::info!("explicit quit requested from the menu bar item");
                // AppHandle::exit は code: Some(0) で ExitRequested を発火する。
                // 暗黙の終了 (code: None) だけを拒む第 3 層をここだけが通り抜ける。
                app.exit(0);
            }
        })
        .build(app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::hotkey::ACCELERATOR;

    /// I/O マトリクス「ホットキー衝突」— 状態行に失敗が残ること。
    ///
    /// 成功時の文言と失敗時の文言が一致してしまうと、この行は副の伝達経路として
    /// 機能しない。両者が異なること、およびどちらもアクセラレータを含むことを固定する。
    #[test]
    fn the_status_line_distinguishes_failure_from_success() {
        let ok = status_line(&HotkeyStatus::registered());
        let ng = status_line(&HotkeyStatus::failed("衝突".to_string()));

        assert_ne!(ok, ng, "登録の成否が文言に現れること");
        assert!(ok.contains(ACCELERATOR), "どのキーの話かを示すこと");
        assert!(ng.contains(ACCELERATOR), "失敗時もどのキーの話かを示すこと");
    }

    /// テンプレートアイコンが同梱されており、単色 (PNG) であること。
    ///
    /// `Image::from_bytes` は `image-png` feature が無いと存在しない。素材の欠落は
    /// ビルドエラーになるが、空ファイルに差し替わっても気づかないため長さも見る。
    #[test]
    fn a_template_icon_is_bundled() {
        assert!(
            TEMPLATE_ICON.starts_with(b"\x89PNG\r\n\x1a\n"),
            "PNG であること"
        );
        assert!(TEMPLATE_ICON.len() > 64, "空の素材に差し替わっていないこと");
    }

    /// メニューの id は「終了」と状態行の 2 つだけであり、取り違えない。
    #[test]
    fn the_menu_has_exactly_two_distinct_ids() {
        assert_ne!(QUIT_ITEM_ID, HOTKEY_STATUS_ITEM_ID);
    }
}
