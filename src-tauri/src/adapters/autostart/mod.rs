//! 自動起動アダプタ — ログイン時の自動起動 (CAP-3)。
//!
//! `MacosLauncher::LaunchAgent` を使う。`~/Library/LaunchAgents/` に plist を書くだけで
//! 済み、`AppleScript` 方式のように System Events の自動化許可を要求しない。代償として
//! システム設定 > ログイン項目には `.app` ではなく `.app` 内部の実行ファイル名で並ぶ。
//! これは既知の代償として受け入れる。
//!
//! **開発ビルドでは有効化しない。** `auto-launch` は `current_exe()` を登録するため、
//! `tauri dev` で有効化すると `src-tauri/target/debug/` のパスがログイン項目に残る。

use tauri::{AppHandle, Runtime};
use tauri_plugin_autostart::{Builder, MacosLauncher};

/// LaunchAgent の plist 名に使う識別子。空白を含む productName をそのまま使わせない。
const AUTOSTART_APP_NAME: &str = "my-task-manager";

/// ログイン時の自動起動を有効化する。
///
/// プラグイン自体はデバッグビルドでも登録する (状態の問い合わせを可能にするため)。
/// 実際の登録はリリースビルドでのみ行う。
pub fn enable<R: Runtime>(app: &AppHandle<R>) {
    let plugin = Builder::new()
        .macos_launcher(MacosLauncher::LaunchAgent)
        .app_name(AUTOSTART_APP_NAME)
        .build();

    if let Err(error) = app.plugin(plugin) {
        log::error!("failed to initialize the autostart plugin: {error}");
        return;
    }

    #[cfg(debug_assertions)]
    {
        let _ = app;
        log::info!("autostart is not registered in a development build");
    }

    #[cfg(not(debug_assertions))]
    {
        use tauri_plugin_autostart::ManagerExt;

        let manager = app.autolaunch();
        match manager.enable() {
            Ok(()) => log::info!("autostart enabled"),
            // 自動起動の登録に失敗しても常駐そのものは続ける。
            Err(error) => log::error!("failed to enable autostart: {error}"),
        }
    }
}
