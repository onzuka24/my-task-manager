//! Tauri command 境界 (AD-3)。
//!
//! フロントからコアへはコマンドのみ。コアからフロントへはイベントのみ。
//!
//! AD-3 の鮮度規則により、オーバーレイは**表示されるたびに**
//! [`get_overlay_snapshot`] で完全なスナップショットを取得してから描画する。
//! 隠れている間に受け取ったイベントに依存してはならない。
//!
//! 状態は `Builder::manage` で**起動前に**預ける。webview は常駐プロセスの `setup` が
//! 終わる前にもコマンドを呼びうるため、state そのものが未管理という状態を作らない。
//! 中身がまだ確定していないことは `Err` として表現し、フロントが再試行できるようにする。

use std::sync::Mutex;

use tauri::{AppHandle, Manager, Runtime, State};

use crate::adapters::hotkey::HotkeyStatus;
use crate::adapters::presentation;

/// 常駐プロセスが保持する、ドメインに属さない起動時の状態。
#[derive(Default)]
pub struct ResidentStatus {
    hotkey: Mutex<Option<HotkeyStatus>>,
}

impl ResidentStatus {
    /// ホットキーの登録結果を確定させる。
    pub fn set_hotkey(&self, status: HotkeyStatus) {
        // 毒された Mutex でも常駐は止めない。ここに保持するのは起動時の事実であり、
        // 不変条件を壊しうる途中状態ではない。
        let mut slot = self
            .hotkey
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *slot = Some(status);
    }

    /// 確定済みのホットキーの登録結果。まだ確定していなければ `None`。
    pub fn hotkey(&self) -> Option<HotkeyStatus> {
        self.hotkey
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

/// オーバーレイが描画に必要とするすべて。
///
/// フィールド名は `src/overlay/Overlay.svelte` の `OverlaySnapshot` 型と 1:1 で
/// 対応する。`invoke<T>` は実行時検査を行わないため、ここを変えると警告が無言で
/// 出なくなる。契約は `tests::the_snapshot_keeps_its_wire_contract` で固定する。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlaySnapshot {
    pub hotkey: HotkeyStatus,
}

/// 表示のたびに呼ばれ、描画に必要な完全なスナップショットを返す。
#[tauri::command]
pub fn get_overlay_snapshot(status: State<'_, ResidentStatus>) -> Result<OverlaySnapshot, String> {
    match status.hotkey() {
        Some(hotkey) => Ok(OverlaySnapshot { hotkey }),
        // 起動処理の途中。フロントは短い間隔で再試行する。
        None => Err("常駐プロセスの起動処理がまだ完了していない".to_string()),
    }
}

/// オーバーレイを閉じる (Esc・フォーカス離脱)。隠した上で直前に最前面だったアプリへ
/// フォーカスを返す。
///
/// 失敗を成功に潰さない。ウィンドウが取得できない場合は `Err` を返し、フロントは
/// 代替経路へ倒れる。
#[tauri::command]
pub fn hide_overlay<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    log::info!("the overlay asked to be closed");
    presentation::hide(&app)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// フロントが代替経路 (`getCurrentWindow().hide()`) で自ら隠したことを伝える。
///
/// `hide_overlay` が失敗した後の保険である。これを伝えないと、コア側の可視状態の記録が
/// 「表示中」のまま残り、次のホットキー押下が「隠す」に倒れて無反応になる。
#[tauri::command]
pub fn mark_overlay_hidden() {
    presentation::mark_hidden();
}

/// 起動時に確定したホットキーの登録結果を公開する。
pub fn publish_hotkey_status<R: Runtime>(app: &AppHandle<R>, status: HotkeyStatus) {
    app.state::<ResidentStatus>().set_hotkey(status);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// スナップショットがまだ確定していない間は成功を返さない。
    ///
    /// 「未確定」を既定値で埋めて返すと、ホットキーの登録失敗が「成功」として
    /// 描画されうる。フロントが再試行できるよう、未確定は失敗として表現する。
    #[test]
    fn an_unresolved_status_is_not_reported_as_success() {
        let status = ResidentStatus::default();
        assert!(status.hotkey().is_none());
    }

    /// 確定後は同じ値が読み出せる。
    #[test]
    fn a_published_status_is_readable() {
        let status = ResidentStatus::default();
        status.set_hotkey(HotkeyStatus::failed("衝突".to_string()));

        let hotkey = status.hotkey().expect("確定後は値が読める");
        assert!(!hotkey.registered);
        assert!(hotkey.error.is_some());
    }

    /// Rust → TS の契約。フィールド名を変えるとフロントの警告が無言で消えるため、
    /// 実際に送られる JSON の形をここで固定する。
    #[test]
    fn the_snapshot_keeps_its_wire_contract() {
        let snapshot = OverlaySnapshot {
            hotkey: HotkeyStatus::failed("衝突".to_string()),
        };
        let json: serde_json::Value =
            serde_json::to_value(&snapshot).expect("スナップショットは直列化できる");

        let hotkey = json
            .get("hotkey")
            .expect("`hotkey` は Overlay.svelte の OverlaySnapshot が読む名前");
        assert!(hotkey.get("accelerator").is_some_and(|v| v.is_string()));
        assert!(hotkey.get("registered").is_some_and(|v| v.is_boolean()));
        assert!(hotkey.get("error").is_some_and(|v| v.is_string()));

        let ok = serde_json::to_value(OverlaySnapshot {
            hotkey: HotkeyStatus::registered(),
        })
        .expect("成功時も直列化できる");
        assert!(
            ok["hotkey"]["error"].is_null(),
            "成功時の error は null であり、フロントの `string | null` と一致する"
        );
    }
}
