//! Tauri command 境界 (AD-3)。
//!
//! フロントからコアへはコマンドのみ。コアからフロントへはイベントのみ。
//!
//! AD-3 の鮮度規則により、オーバーレイは**表示されるたびに**
//! [`get_overlay_snapshot`] で完全なスナップショットを取得してから描画する。
//! 隠れている間に受け取ったイベントに依存してはならない。

use tauri::{AppHandle, Manager, Runtime, State};

use crate::adapters::hotkey::HotkeyStatus;
use crate::adapters::presentation;

/// 常駐プロセスが保持する、ドメインに属さない起動時の状態。
pub struct ResidentStatus {
    pub hotkey: HotkeyStatus,
}

/// オーバーレイが描画に必要とするすべて。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlaySnapshot {
    pub hotkey: HotkeyStatus,
}

/// 表示のたびに呼ばれ、描画に必要な完全なスナップショットを返す。
#[tauri::command]
pub fn get_overlay_snapshot(status: State<'_, ResidentStatus>) -> OverlaySnapshot {
    log::info!("overlay snapshot requested");
    OverlaySnapshot {
        hotkey: status.hotkey.clone(),
    }
}

/// オーバーレイを閉じる (Esc)。隠した上で直前に最前面だったアプリへフォーカスを返す。
#[tauri::command]
pub fn hide_overlay<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    presentation::hide(&app)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// 起動時の状態をアプリケーションに預ける。
pub fn manage<R: Runtime>(app: &AppHandle<R>, status: ResidentStatus) {
    app.manage(status);
}
