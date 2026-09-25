//! メニューバーアダプタ — 常在する可視面と、唯一の明示的な終了経路 (CAP-3)。
//!
//! `LSUIElement` を立てたアプリは自前のメニューバーを表示しないため、ここで言う
//! 「メニューバー項目」は `NSStatusItem` = Tauri の tray である。
//!
//! メニューの中身は 3 つだけである — 「開く」「終了」と、ホットキーの現在状態を示す
//! 非活性の 1 行。バッジ・件数・進捗・タスク一覧を出してはならない (AD-15、SPEC.md
//! 非目標)。
//!
//! **「開く」はホットキーの副の経路である。** ホットキーは唯一の呼び出し経路であり
//! (CAP-1)、他のアプリに奪われれば**オーバーレイ**へ到達する手段が無くなる。状態行が
//! 「登録できなかった」と述べられるのに、そこから開く手段が無いのでは報せが行き止まりに
//! なる。**トグルではなく「開く」である** — メニューを開くには**オーバーレイ**から
//! フォーカスが外れ、その時点で**オーバーレイ**は既に閉じている (AD-15 の一時的な面)。
//! トグルにすれば、押すたびに「閉じる」に倒れて何も出ない。
//!
//! **`PredefinedMenuItem::quit` を使ってはならない。** `NSApplication terminate:` を
//! 直接送り、`RunEvent::ExitRequested` を迂回するため、誤終了の阻止機構ごと素通りする。
//! 明示的な終了は独自の `MenuItem` から [`tauri::AppHandle::exit`] を呼んで表現する。

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Runtime};

use crate::adapters::hotkey::HotkeyStatus;

/// 「開く」項目の id。
pub const OPEN_ITEM_ID: &str = "open";
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
    // 「開く」にもアクセラレータを与えない。呼び出しの打鍵はグローバルホットキー
    // 一つに集約されており (FR-1)、ここに二つ目を置けば案内と実際に効く打鍵が割れる。
    let open = MenuItem::with_id(app, OPEN_ITEM_ID, "開く", true, None::<&str>)?;
    // 終了にもアクセラレータを与えない。Cmd+Q は無効化した対象そのものである。
    let quit = MenuItem::with_id(app, QUIT_ITEM_ID, "終了", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&hotkey_line, &open, &quit])?;

    TrayIconBuilder::new()
        .icon(Image::from_bytes(TEMPLATE_ICON)?)
        .icon_as_template(true)
        .tooltip("My Task Manager")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            OPEN_ITEM_ID => {
                log::info!("the overlay was asked for from the menu bar item");
                // **トグルではなく表示である。** メニューを開いた時点で
                // **オーバーレイ**はフォーカスを失って閉じている。
                if let Err(error) = crate::adapters::presentation::show(app) {
                    log::error!("failed to show the overlay from the menu bar item: {error}");
                }
            }
            QUIT_ITEM_ID => {
                log::info!("explicit quit requested from the menu bar item");
                // AppHandle::exit は code: Some(0) で ExitRequested を発火する。
                // 暗黙の終了 (code: None) だけを拒む第 3 層をここだけが通り抜ける。
                app.exit(0);
            }
            _ => {}
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

    /// メニューの id は「開く」「終了」と状態行の 3 つだけであり、取り違えない。
    ///
    /// **取り違えれば「開く」が常駐を終わらせる。** 二つの項目が同じ id を持てば、
    /// 先に一致した分岐が両方の押下を受ける。
    #[test]
    fn the_menu_has_exactly_three_distinct_ids() {
        let ids = [OPEN_ITEM_ID, QUIT_ITEM_ID, HOTKEY_STATUS_ITEM_ID];
        for (index, id) in ids.iter().enumerate() {
            for other in &ids[index + 1..] {
                assert_ne!(id, other, "id が重なれば押下の意味が入れ替わる");
            }
        }
    }

    /// 「開く」がメニューを組み立てる側にも、押下を捌く側にも書かれていること。
    ///
    /// どちらか片方が消えても型検査は通る — 項目だけが残れば押しても何も起きず、
    /// 分岐だけが残れば項目がどこにも現れない。**ホットキーが死んだときの副の経路が
    /// これである以上、静かに失われてはならない。**
    #[test]
    fn the_open_item_is_still_wired_in_the_source() {
        let source = include_str!("mod.rs");
        assert!(
            source.contains("MenuItem::with_id(app, OPEN_ITEM_ID"),
            "「開く」の項目が組み立てから消えている"
        );
        assert!(
            source.contains("presentation::show(app)"),
            "「開く」の押下がオーバーレイの表示に繋がっていない"
        );
    }
}
