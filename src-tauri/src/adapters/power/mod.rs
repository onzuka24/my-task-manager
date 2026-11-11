//! 電源断の予告 — ログアウト・再起動・シャットダウンの直前に、開いている**作業区間**を
//! 閉じて書く (spec-work-log「シャットダウン」/ 利用者の裁定・レビュー後)。
//!
//! # なぜ終了の経路では足りないのか
//!
//! メニューバー項目の「終了」を押さずに電源を切ると、終了要求は暗黙の終了
//! (`ExitRequested { code: None }`) として届き、誤終了阻止の第 3 層が拒む。最後に届く
//! SIGTERM は `RunEvent::Exit` を通らない。どちらでも、その日の最後の区間 (数時間に
//! なりうる) が失われる。
//!
//! macOS はその直前に `NSWorkspaceWillPowerOffNotification` を出す。これを受けて
//! [`crate::close_work_segment`] を呼ぶ。**定期的な書き込みは足さない** — 書くのは
//! 通知を受けた一度だけである。失敗はログのみ (書けなければ失われる)。
//!
//! # OS を知るのはここだけである (AD-1)
//!
//! コアは「閉じて書く」を [`crate::domain::state::Core::close_work_segment`] として
//! 持つだけで、電源の事情を知らない。

/// 電源断の予告を購読する。起動時に一度だけ呼ぶ (メインスレッドの `setup` から)。
///
/// 購読は常駐が終わるまで続く。通知センターが返す観察者は手放さない (解除しない)。
#[cfg(target_os = "macos")]
pub fn observe<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use std::ptr::NonNull;

    use block2::RcBlock;
    use objc2_app_kit::{NSWorkspace, NSWorkspaceWillPowerOffNotification};
    use objc2_foundation::NSNotification;

    let handle = app.clone();
    let block = RcBlock::new(move |_notification: NonNull<NSNotification>| {
        log::info!("the system is about to log out, restart or shut down");
        crate::close_work_segment(&handle);
    });

    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    // SAFETY: 通知名は AppKit が定める静的な定数である。`object` は指定せず、`queue` を
    // 指定しないため、ブロックは通知を出したスレッド (NSWorkspace の通知はメイン
    // スレッド) で走る。ブロックが捕らえるのは `AppHandle` (Send + Sync) だけである。
    let observer = unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceWillPowerOffNotification),
            None,
            None,
            &block,
        )
    };
    // 常駐が終わるまで購読を続ける。解除する経路は持たない。
    std::mem::forget(observer);
    log::info!("watching for logout, restart and shutdown");
}

/// macOS 以外では何もしない (v1 のビルド対象は macOS のみ)。
#[cfg(not(target_os = "macos"))]
pub fn observe<R: tauri::Runtime>(_app: &tauri::AppHandle<R>) {}
