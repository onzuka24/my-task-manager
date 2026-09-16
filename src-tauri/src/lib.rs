//! 常駐の骨格 — 常駐プロセス・自動起動・ホットキー呼び出し (CAP-1, CAP-3)。
//!
//! 層の分割は ports and adapters (AD-1) に従う。
//! - [`domain`] — OS を知らないドメインコア (本スライスでは空)
//! - [`ports`] — コアが外界に要求する契約 (本スライスでは空)
//! - [`adapters`] — OS API を呼んでよい唯一の場所
//! - [`commands`] — Tauri command 境界 (AD-3)

mod adapters;
mod commands;
mod domain;
mod ports;

use adapters::{autostart, hotkey, presentation};
use commands::ResidentStatus;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::get_overlay_snapshot,
            commands::hide_overlay
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            // ログはローカルファイルのみ (AD-12)。外部送信は行わない。
            handle.plugin(
                tauri_plugin_log::Builder::default()
                    .level(log::LevelFilter::Info)
                    .targets(log_targets())
                    .build(),
            )?;

            // Dock にアイコンを出さない。Info.plist の LSUIElement が起動時を、
            // この呼び出しが dev 実行と再昇格の場合を覆う。
            #[cfg(target_os = "macos")]
            handle.set_activation_policy(tauri::ActivationPolicy::Accessory)?;

            // ウィンドウは tauri.conf.json で起動時に生成され、隠されている。
            // 押下時に生成しないことが CAP-1 の 300ms 制約を満たす前提である。
            if presentation::overlay(&handle).is_none() {
                log::error!(
                    "overlay window `{}` was not created at startup",
                    presentation::OVERLAY_LABEL
                );
            }

            let hotkey_status = hotkey::register(&handle);
            if let Some(error) = hotkey_status.error.as_deref() {
                log::error!("{error}");
            } else {
                log::info!("global hotkey registered: {}", hotkey_status.accelerator);
            }

            // ホットキーが使えないことを利用者が知る経路は、オーバーレイ自身しかない
            // (Dock アイコンもメニューバー項目も持たないため)。スナップショットを
            // 預けてから表示する。
            let show_failure = !hotkey_status.registered;
            commands::manage(&handle, ResidentStatus { hotkey: hotkey_status });
            if show_failure {
                presentation::show(&handle)?;
            }

            autostart::enable(&handle);

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// ログの出力先。ローカルファイルのみ (AD-12)。開発時は標準出力にも出す。
fn log_targets() -> Vec<tauri_plugin_log::Target> {
    use tauri_plugin_log::{Target, TargetKind};

    let mut targets = vec![Target::new(TargetKind::LogDir {
        file_name: Some("my-task-manager".to_string()),
    })];
    if cfg!(debug_assertions) {
        targets.push(Target::new(TargetKind::Stdout));
    }
    targets
}
