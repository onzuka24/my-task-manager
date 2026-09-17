//! 常駐の骨格 — 常駐プロセス・自動起動・ホットキー呼び出し (CAP-1, CAP-3)。
//!
//! 層の分割は ports and adapters (AD-1) に従う。
//! - [`domain`] — OS を知らないドメインコア
//! - [`ports`] — コアが外界に要求する契約
//! - [`adapters`] — OS API を呼んでよい唯一の場所
//! - [`commands`] — Tauri command 境界 (AD-3)
//!
//! # 誤終了の阻止は 3 層に分ける
//!
//! 常駐プロセスの終了経路はメニューバー項目からの「終了」だけである。反射的な
//! Cmd+Q / Cmd+W 一打で常駐が死ぬと、次のログインまでホットキーが失われる
//! (CAP-3「明示的な終了まで動作を継続する」)。
//!
//! 1. **既定メニューを組み込ませない** ([`tauri::Builder::enable_macos_default_menu`])。
//!    `Builder::menu()` を設定していない macOS ビルドでは `Menu::default()` が自動で
//!    組み込まれ、そこに quit (Cmd+Q) と Window サブメニュー (Cmd+W) が入る。
//!    activation policy とは無関係で `Accessory` でも入る。既定メニューの Cmd+Q は
//!    `NSApplication terminate:` を直接送り `RunEvent::ExitRequested` を迂回するため、
//!    **これが Cmd+Q に対する唯一有効な防御である。**
//! 2. **閉じる要求を非表示に変換する** ([`tauri::WindowEvent::CloseRequested`])。
//!    ウィンドウを生かし続けることは 300ms 制約の前提でもある。
//! 3. **暗黙の終了だけを拒む** ([`tauri::RunEvent::ExitRequested`])。`code` が `None`
//!    のときだけ `prevent_exit` する。メニューバー項目からの `AppHandle::exit(0)` は
//!    `code: Some(0)` で来るため通る。フラグは要らない。

pub mod adapters;
pub mod commands;
pub mod domain;
pub mod ports;

use tauri::{Emitter, Manager};

use adapters::hotkey::HotkeyStatus;
use adapters::storage::SqliteStorage;
use adapters::{autostart, clock, hotkey, menubar, presentation};
use domain::state::Core;

/// 終了要求を拒むべきか決める純粋関数。
///
/// `code` が `None` なのは、最後のウィンドウが破棄されてウィンドウ集合が空になった
/// ときなど、利用者が明示的に求めていない終了である。`Some(_)` は
/// [`tauri::AppHandle::exit`] / `restart` から来る明示的な終了であり、通す。
pub const fn should_prevent_exit(code: Option<i32>) -> bool {
    code.is_none()
}

/// **現在地**が変わったことを伝えるイベントの名前 (AD-3)。
///
/// event は `名詞_過去分詞` (スパイン「一貫性の規約」)。
pub const CURRENT_POSITION_CHANGED: &str = "current_position_changed";

/// **切り替え**が確定したことを提示層へ伝える (AD-3)。
///
/// # ペイロードを持たない理由
///
/// AD-3 の鮮度規則により、オーバーレイは表示のたびにコマンドでスナップショットを
/// 取り直す。イベントが状態を運べば「イベント経由の状態」と「スナップショット経由の
/// 状態」という二つの真実が生まれる。**変化したという事実だけを伝え、受け手は必ず
/// スナップショットを取り直す。** 取りこぼしても次の表示で正しくなる。
///
/// **発行は失敗しても常駐を止めない。** 配送は保証されておらず、取りこぼしは鮮度規則が
/// 既に吸収している。
pub fn announce_current_position_changed<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Err(error) = app.emit(CURRENT_POSITION_CHANGED, ()) {
        log::error!("failed to announce that the current position changed: {error}");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default();

    // 二重起動を許さない。後発は自ら終了し、先発が応答する。
    //
    // **このプラグインは最初に登録しなければならない。** プラグインは登録順に setup が
    // 走るため、先に別のプラグインを登録すると、終了する運命の後発プロセスでもその
    // setup が実行されてしまう。
    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            log::info!("a second launch was detected; the resident process stays single");
            // コールバックは先発プロセスの tokio タスク上で走り、メインスレッドでは
            // ない。ウィンドウ操作はメインスレッドへ回す。
            let handle = app.clone();
            if let Err(error) = app.run_on_main_thread(move || {
                if let Err(error) = presentation::show(&handle) {
                    log::error!("failed to answer the second launch: {error}");
                }
            }) {
                log::error!("failed to hand the second launch to the main thread: {error}");
            }
        }));
    }

    builder
        // 第 1 層 — Cmd+Q / Cmd+W の出所そのものを消す。
        .enable_macos_default_menu(false)
        // 状態は起動前に預ける。webview が setup より先にコマンドを呼びうるため。
        .manage(commands::ResidentStatus::default())
        .invoke_handler(tauri::generate_handler![
            commands::get_overlay_snapshot,
            commands::switch_current_position,
            commands::hide_overlay,
            commands::mark_overlay_hidden
        ])
        // 第 2 層 — 閉じる要求は破棄ではなく非表示に変換する。
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                log::info!("a close request was converted into hiding the overlay");
                if let Err(error) = presentation::hide(window.app_handle()) {
                    log::error!("failed to hide the overlay on a close request: {error}");
                }
            }
        })
        .setup(|app| {
            // **この関数の中で `?` を使わない。** setup が Err を返すと run() が Err と
            // なり、常駐そのものが立ち上がらない。ここでの失敗はいずれも「常駐を止める
            // ほどではない」ものであり、記録して進む。
            let handle = app.handle().clone();

            // ログはローカルファイルのみ (AD-12)。外部送信は行わない。
            if let Err(error) = handle.plugin(
                tauri_plugin_log::Builder::default()
                    .level(log::LevelFilter::Info)
                    .targets(log_targets())
                    .build(),
            ) {
                eprintln!("failed to initialize logging: {error}");
            }

            // Dock にアイコンを出さない。Info.plist の LSUIElement が起動時を、
            // この呼び出しが dev 実行と再昇格の場合を覆う。
            #[cfg(target_os = "macos")]
            if let Err(error) = handle.set_activation_policy(tauri::ActivationPolicy::Accessory) {
                log::error!("failed to keep the app out of the Dock: {error}");
            }

            // ウィンドウは tauri.conf.json で起動時に生成され、隠されている。
            // 押下時に生成しないことが CAP-1 の 300ms 制約を満たす前提である。
            let overlay_missing = presentation::overlay(&handle).is_none();
            if overlay_missing {
                log::error!(
                    "overlay window `{}` was not created at startup",
                    presentation::OVERLAY_LABEL
                );
            }

            // 他アプリのフルスクリーン空間でも最前面に出す (CAP-1)。
            // setup はメインスレッドで走るため、ここから NSWindow を触ってよい。
            #[cfg(target_os = "macos")]
            if let Err(error) = presentation::allow_fullscreen_spaces(&handle) {
                log::error!("failed to join full-screen spaces: {error}");
            }

            let hotkey_status = if overlay_missing {
                // ウィンドウが無ければ、ホットキーが登録できても押下は何も起こさない。
                // 「有効」と言い続けるほうが、押しても無反応な状態より悪い。
                HotkeyStatus::failed(format!(
                    "オーバーレイウィンドウ `{}` が起動時に生成されなかった。ホットキーを押しても何も出ない。",
                    presentation::OVERLAY_LABEL
                ))
            } else {
                hotkey::register(&handle)
            };
            if let Some(error) = hotkey_status.error.as_deref() {
                log::error!("{error}");
            } else {
                log::info!("global hotkey registered: {}", hotkey_status.accelerator);
            }
            commands::publish_hotkey_status(&handle, hotkey_status.clone());

            // メニューバー項目 — 常在する可視面であり、唯一の明示的な終了経路 (CAP-3)。
            // ホットキーの状態行も持つ。
            // メニューバー項目は唯一の明示的な終了経路である。既定メニューを消し、
            // 暗黙の終了も拒んだうえでこれが立たないと、どの UI からも終了できない
            // 常駐が毎ログイン復活する — CAP-3 の反転である。立たなければ終了する。
            if let Err(error) = menubar::install(&handle, &hotkey_status) {
                log::error!("failed to install the menu bar item: {error}");
                log::error!("there would be no way to quit; exiting instead");
                handle.exit(1);
            }

            // 最後にコミットされた状態へ復帰する (AD-5)。DB は自動起動の印と同じ
            // アプリデータディレクトリに置く。
            //
            // **オーバーレイを出しうる経路より先に済ませる。** オーバーレイは表示の
            // たびにコマンドで完全なスナップショットを取得する (AD-3 鮮度規則) が、
            // その中身はここで `manage` されるコアから来る。後回しにすると、下の
            // ホットキー失敗の報せが「状態を読み込めていない」という別の失敗を
            // 被せた形で出かねない。
            restore_core(&handle);

            // ホットキーが唯一の呼び出し経路である以上、それが死んでいることは確実に
            // 伝わらなければならない。起動時にオーバーレイを出して理由を示す。
            // 以後はメニューバー項目の状態行が副の経路となる。
            if !hotkey_status.registered {
                if let Err(error) = presentation::show(&handle) {
                    log::error!("failed to report the hotkey failure on screen: {error}");
                }
            }

            autostart::enable(&handle);

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building the resident process")
        .run(|_app, event| {
            // 第 3 層 — 暗黙の終了だけを拒む。
            if let tauri::RunEvent::ExitRequested { code, api, .. } = event {
                if should_prevent_exit(code) {
                    log::info!("an implicit exit request was refused; the process stays resident");
                    api.prevent_exit();
                }
            }
        });
}

/// 永続化された状態を読み戻し、コアを常駐プロセスへ預ける (AD-5)。
///
/// **失敗しても常駐を止めない。** I/O マトリクス「異常終了後の起動」は「破損時は
/// `Err` を返し起動を止めない」と定めている。ホットキーによる呼び出しとメニューバー
/// 項目からの終了は、状態が読めなくても働かなければならない — 読めないまま何も
/// 立たないほうが、利用者にとって回復しようがない。
///
/// 失敗した場合、コアは `manage` されない。消費者 (CAP-7 以降のコマンド) は
/// [`tauri::Manager::try_state`] で不在を扱うこと。既定値で埋めたコアを預けると、
/// **現在地**が失われた事実が「未着手」として静かに上書きされ、次の書き込みで確定して
/// しまう。
fn restore_core<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let Ok(app_data_dir) = app.path().app_data_dir() else {
        log::error!("failed to resolve the application data directory; state is not restored");
        return;
    };

    let path = SqliteStorage::database_path(&app_data_dir);
    let storage = match SqliteStorage::open(&path) {
        Ok(storage) => storage,
        Err(error) => {
            log::error!("failed to open the state database: {error}");
            return;
        }
    };

    match Core::restore(Box::new(clock::SystemClock), Box::new(storage)) {
        Ok(core) => {
            log::info!("the core state was restored from disk");
            app.manage(core);
        }
        Err(error) => log::error!("failed to restore the core state: {error}"),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// I/O マトリクス「誤終了の抑止」— 利用者が明示的に求めていない終了は拒む。
    ///
    /// `code: None` は、最後のウィンドウが破棄されるなどの暗黙の経路である。
    #[test]
    fn an_implicit_exit_is_refused() {
        assert!(should_prevent_exit(None));
    }

    /// I/O マトリクス「明示的な終了」— メニューバー項目からの終了は通す。
    ///
    /// `AppHandle::exit(0)` は `code: Some(0)` で来る。0 を「値が無い」と同一視して
    /// いれば、唯一の終了経路が塞がる。
    #[test]
    fn an_explicit_exit_passes_through() {
        assert!(!should_prevent_exit(Some(0)));
        assert!(!should_prevent_exit(Some(1)));
        assert!(!should_prevent_exit(Some(tauri::RESTART_EXIT_CODE)));
    }

    /// event は `名詞_過去分詞` である (スパイン「一貫性の規約」)。名前を変えると
    /// オーバーレイの購読が無言で外れ、切り替え後の再描画が起きなくなる。
    #[test]
    fn the_event_name_follows_the_naming_rule() {
        assert_eq!(CURRENT_POSITION_CHANGED, "current_position_changed");
    }
}
