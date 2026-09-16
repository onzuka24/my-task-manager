//! 自動起動アダプタ — ログイン時の自動起動 (CAP-3)。
//!
//! `MacosLauncher::LaunchAgent` を使う。`~/Library/LaunchAgents/` に plist を書くだけで
//! 済み、`AppleScript` 方式のように System Events の自動化許可を要求しない。代償として
//! システム設定 > ログイン項目には `.app` ではなく `.app` 内部の実行ファイル名で並ぶ。
//! これは既知の代償として受け入れる。
//!
//! **開発ビルドでは有効化しない。** `auto-launch` は `current_exe()` を登録するため、
//! `tauri dev` で有効化すると `src-tauri/target/debug/` のパスがログイン項目に残る。
//!
//! **利用者の解除を次のログインで覆さない。** `auto-launch` の `is_enabled()` は plist
//! ファイルの存在を見るだけであり、「まだ一度も登録していない」と「利用者が解除した」を
//! 区別できない。区別しないまま毎回 `enable()` を呼ぶと、ログイン項目から消しても
//! アプリを起動した時点で復活する。そこで一度きりの登録を行ったことを印として残し、
//! 印がある以降は登録し直さない。
//!
//! [TODO(AD-11): 印は現在アプリデータディレクトリの空ファイルである。永続化層
//! (CAP-4 以降) が入った時点で設定テーブルへ移し、別建てのファイルを無くすこと。]

use tauri::{AppHandle, Runtime};
use tauri_plugin_autostart::{Builder, MacosLauncher};

/// LaunchAgent の plist 名に使う識別子。空白を含む productName をそのまま使わせない。
/// `~/Library/LaunchAgents/my-task-manager.plist` になる。
const AUTOSTART_APP_NAME: &str = "my-task-manager";

/// 一度きりの登録を済ませたことを示す印のファイル名。
/// `make uninstall` が消す対象でもある (Makefile の `AUTOSTART_MARKER`)。
pub const REGISTRATION_MARKER: &str = "autostart-registered";

/// 自動起動を登録すべきかを決める純粋関数。
///
/// - `already_attempted` — 過去に一度でも登録を行ったか (印の有無)
/// - `currently_enabled` — いま LaunchAgent が登録されているか
///
/// 初回のみ登録する。印があるのに登録されていない状態は「利用者が解除した」であり、
/// 覆してはならない。
pub const fn should_register(already_attempted: bool, currently_enabled: bool) -> bool {
    !already_attempted && !currently_enabled
}

/// ログイン時の自動起動を登録する。
///
/// プラグイン自体はどちらのビルドでも登録する (状態の問い合わせを可能にするため)。
/// OS へ実際に書き込むのは [`register_with_os`] だけであり、判断の筋道は
/// デバッグビルドでも同じように compile / clippy の対象となる。
///
/// 自動起動の登録に失敗しても常駐そのものは続ける。
pub fn enable<R: Runtime>(app: &AppHandle<R>) {
    let plugin = Builder::new()
        .macos_launcher(MacosLauncher::LaunchAgent)
        .app_name(AUTOSTART_APP_NAME)
        .build();

    if let Err(error) = app.plugin(plugin) {
        log::error!("failed to initialize the autostart plugin: {error}");
        return;
    }

    let marker = marker_path(app);
    let already_attempted = marker.as_ref().is_some_and(|path| path.exists());
    let currently_enabled = read_registration(app);

    if !should_register(already_attempted, currently_enabled) {
        log::info!(
            "autostart is left as is (already_attempted={already_attempted}, enabled={currently_enabled})"
        );
        if currently_enabled && !already_attempted {
            // plist はあるのに印が無い状態を残してはならない。次に利用者が解除すると
            // 「まだ一度も登録していない」と誤認し、起動のたびに復活させてしまう。
            write_marker(marker.as_deref());
        }
        return;
    }

    if register_with_os(app) {
        // **登録できたときだけ**印を残す。失敗したまま印を残すと二度と再試行しない。
        write_marker(marker.as_deref());
    }
}

/// いま LaunchAgent が登録されているか。読めないときは「登録されている」に倒す —
/// 状態が読めないまま登録すると、利用者の解除を覆す側に倒れるためである。
fn read_registration<R: Runtime>(app: &AppHandle<R>) -> bool {
    use tauri_plugin_autostart::ManagerExt;

    match app.autolaunch().is_enabled() {
        Ok(enabled) => enabled,
        Err(error) => {
            log::error!("failed to read the autostart registration: {error}");
            true
        }
    }
}

/// OS に自動起動を登録する。成功したときだけ `true`。
///
/// **開発ビルドでは OS を触らない。** `auto-launch` は `current_exe()` を登録するため、
/// `tauri dev` で有効化すると `src-tauri/target/debug/` のパスがログイン項目に残る。
fn register_with_os<R: Runtime>(app: &AppHandle<R>) -> bool {
    use tauri_plugin_autostart::ManagerExt;

    if cfg!(debug_assertions) {
        let _ = app;
        log::info!("autostart is not registered in a development build");
        return false;
    }

    match app.autolaunch().enable() {
        Ok(()) => {
            log::info!("autostart enabled");
            true
        }
        Err(error) => {
            log::error!("failed to enable autostart: {error}");
            false
        }
    }
}

/// 印を書く。開発ビルドは登録しないため印も残さない。
fn write_marker(path: Option<&std::path::Path>) {
    if cfg!(debug_assertions) {
        return;
    }
    let Some(path) = path else {
        return;
    };
    if let Some(parent) = path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            log::error!("failed to create the application data directory: {error}");
            return;
        }
    }
    if let Err(error) = std::fs::write(path, b"") {
        log::error!("failed to record the autostart registration marker: {error}");
    }
}

/// 印のファイルパス。アプリデータディレクトリが解決できないときは `None`。
fn marker_path<R: Runtime>(app: &AppHandle<R>) -> Option<std::path::PathBuf> {
    use tauri::Manager;

    match app.path().app_data_dir() {
        Ok(dir) => Some(dir.join(REGISTRATION_MARKER)),
        Err(error) => {
            log::error!("failed to resolve the application data directory: {error}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `make uninstall` の対象を決めている実ファイル。リテラルの二重管理を避ける。
    const MAKEFILE: &str = include_str!("../../../../Makefile");
    const TAURI_CONF: &str = include_str!("../../../tauri.conf.json");

    /// Makefile の `NAME := value` を読む。
    fn makefile_var(name: &str) -> String {
        MAKEFILE
            .lines()
            .find_map(|line| {
                let rest = line.strip_prefix(name)?;
                let rest = rest.trim_start().strip_prefix(":=")?;
                Some(rest.trim().to_string())
            })
            .unwrap_or_else(|| panic!("Makefile に {name} の定義が無い"))
    }

    /// 初回起動では登録する。
    #[test]
    fn the_first_launch_registers_autostart() {
        assert!(should_register(false, false));
    }

    /// I/O マトリクス「自動起動の解除」— 利用者が解除した後にアプリを起動しても
    /// 再登録しない。印があるのに登録されていない状態が、まさにその状態である。
    #[test]
    fn a_user_removal_is_not_undone_by_a_later_launch() {
        assert!(!should_register(true, false));
    }

    /// 既に登録されているなら何もしない (毎回上書きしない)。
    #[test]
    fn an_existing_registration_is_left_alone() {
        assert!(!should_register(true, true));
        assert!(!should_register(false, true));
    }

    /// 印のファイル名は空でない — 空だとディレクトリ自体を指してしまう。
    #[test]
    fn the_marker_has_a_file_name() {
        assert!(!REGISTRATION_MARKER.is_empty());
        assert!(!REGISTRATION_MARKER.contains('/'));
    }

    /// `make uninstall` が消す先と、この層が書く先が一致していること。
    ///
    /// 一致しなければアンインストールが plist と印を消し損ね、再ログインで復活する。
    /// リテラルの比較では気づけないため、実際の Makefile と tauri.conf.json を読む。
    #[test]
    fn the_makefile_targets_match_what_this_adapter_writes() {
        let proc_name = makefile_var("PROC_NAME");
        let bundle_id = makefile_var("BUNDLE_ID");
        let marker = makefile_var("AUTOSTART_MARKER");
        let launch_agent = makefile_var("LAUNCH_AGENT");

        assert_eq!(
            proc_name, AUTOSTART_APP_NAME,
            "Makefile の PROC_NAME と auto-launch の app_name が一致すること"
        );
        assert!(
            launch_agent.ends_with("/$(PROC_NAME).plist"),
            "LAUNCH_AGENT は PROC_NAME から組み立てられていること: {launch_agent}"
        );
        assert!(
            marker.ends_with(&format!("/{REGISTRATION_MARKER}")),
            "AUTOSTART_MARKER の末尾が印のファイル名であること: {marker}"
        );
        assert!(
            marker.contains("$(BUNDLE_ID)"),
            "AUTOSTART_MARKER はアプリデータディレクトリ (= bundle identifier) の下にあること: {marker}"
        );

        let conf: serde_json::Value =
            serde_json::from_str(TAURI_CONF).expect("tauri.conf.json は JSON である");
        assert_eq!(
            conf["identifier"].as_str().expect("identifier がある"),
            bundle_id,
            "Makefile の BUNDLE_ID が tauri.conf.json の identifier と一致すること \
             (app_data_dir はこの identifier で決まる)"
        );
    }
}
