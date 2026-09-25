//! 介入アダプタ — **入力先を奪わない非活性パネル** (AD-6 / FR-15)。
//!
//! # オーバーレイと兼ねない
//!
//! オーバーレイは利用者が意図して呼び出す通常ウィンドウであり、フォーカスを取ってよい。
//! **介入**はツールの側から出るものであり、いかなる場合もキーボードの入力先を奪わない。
//! この二つを同じウィンドウ実装で兼ねてはならず、生成後に種別を切り替えることも禁じ
//! られている (AD-6) — 生存中のウィンドウの style mask の置き換えはプロセスを異常終了
//! させる。したがってウィンドウは二つあり、**片方だけがパネルになる。**
//!
//! `adapters/presentation` の静的変数 (`OVERLAY_VISIBLE` / `TRANSITION`) を共有しない
//! のも同じ理由である。あれは単一ウィンドウを前提に書かれており、二つ目が同じ記録を
//! 書き換えれば、ホットキー押下が無反応になる。
//!
//! # 三つの必須設定
//!
//! いずれを欠いても FR-15 は**特定の状況でのみ静かに失敗する** (AD-6)。
//!
//! - `nonactivating_panel` — 入力先を奪わない。欠けると、フルスクリーンで入力中の
//!   打鍵がパネルへ吸われる (PRD §7.2 がまさに禁じた事故)
//! - `set_hides_on_deactivate(false)` — 既定では自アプリの非活性化時にパネルが自ら
//!   隠れる。常時非活性な常駐ツールでは、出た瞬間に消える
//! - `full_screen_auxiliary` — 欠けるとフルスクリーン作業中にパネルが出ない
//!
//! # 起動時に生成し、隠しておく
//!
//! ウィンドウは tauri.conf.json が `visible: false` で生成する。ここで行うのは
//! **NSPanel への差し替え**であり、`setup` の中、すなわちメインスレッドで行う
//! (`NSPanel` は `MainThreadOnly`)。**スタイルマスクは加えるのであって置き換えない** —
//! 置き換えれば Tauri が組んだ構造的なスタイルが落ちる。

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use tauri::{AppHandle, Manager, Runtime, WebviewWindow};

use crate::adapters::hotkey;
use crate::domain::rest::InterventionChoice;
use crate::domain::state::Core;

/// 介入パネルのウィンドウラベル。tauri.conf.json の `app.windows[].label` と一致する。
///
/// **オーバーレイのラベルと別であることが、閉じる要求を取り違えないための鍵である**
/// (`lib.rs` の第 2 層)。
pub const INTERVENTION_LABEL: &str = "intervention";

/// パネルの可視状態。**この層が唯一の所有者である。**
///
/// `adapters/presentation` の `OVERLAY_VISIBLE` とは別の変数であり、共有しない。
static PANEL_VISIBLE: AtomicBool = AtomicBool::new(false);

/// 可視状態の遷移を直列化する錠。
///
/// 刻みのスレッドと、応答のホットキーのスレッドの双方から到達する。
static TRANSITION: Mutex<()> = Mutex::new(());

fn lock_transition() -> MutexGuard<'static, ()> {
    TRANSITION.lock().unwrap_or_else(|error| error.into_inner())
}

/// 介入パネルの操作が失敗した理由。
#[derive(Debug)]
pub enum InterventionError {
    /// パネル用のウィンドウを取得できない。起動時の生成に失敗している。
    WindowMissing(&'static str),
    /// NSPanel への差し替えができていない。
    NotAPanel(&'static str),
    /// OS 呼び出しが失敗した。
    Window(String),
}

impl fmt::Display for InterventionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WindowMissing(label) => {
                write!(f, "介入パネルのウィンドウ `{label}` が存在しない")
            }
            Self::NotAPanel(label) => write!(
                f,
                "ウィンドウ `{label}` が非活性パネルになっていない — 介入は入力先を奪わない形で出せない"
            ),
            Self::Window(detail) => write!(f, "介入パネルの操作に失敗した: {detail}"),
        }
    }
}

impl std::error::Error for InterventionError {}

/// 介入パネルに対して次に行う操作。
///
/// **トグルが無い。** **介入**は利用者の応答でのみ閉じる (FR-15) — 表示と非表示を
/// 同じ入力で切り替える経路が型として存在しない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelAction {
    /// 出す。**フォーカスは取らない。**
    Show,
    /// 引っ込める。
    Hide,
}

/// [`PanelAction`] を適用した後の可視状態。テストと実装で同じ規則を使う純粋関数。
#[must_use]
pub const fn visibility_after(action: PanelAction) -> bool {
    matches!(action, PanelAction::Show)
}

/// 既に求める状態なら、行うべき操作は無い。
///
/// **二重に出さない / 二重に引っ込めない。** 出し直せば、フルスクリーン空間で
/// 空間の切り替えが起きうる。
#[must_use]
pub const fn action_for(target: PanelAction, is_visible: bool) -> Option<PanelAction> {
    if visibility_after(target) == is_visible {
        None
    } else {
        Some(target)
    }
}

/// パネル用のウィンドウを取得する。
pub fn window<R: Runtime>(app: &AppHandle<R>) -> Option<WebviewWindow<R>> {
    app.get_webview_window(INTERVENTION_LABEL)
}

/// この層が記録している可視状態。
#[must_use]
pub fn is_visible() -> bool {
    PANEL_VISIBLE.load(Ordering::SeqCst)
}

fn record(visible: bool) {
    PANEL_VISIBLE.store(visible, Ordering::SeqCst);
}

#[cfg(target_os = "macos")]
mod nspanel {
    //! `tauri_panel!` が展開する `use` を閉じ込めるための入れ物。
    //!
    //! マクロはモジュール直下に `use` を並べるため、そのまま親モジュールへ展開すると
    //! 名前空間が汚れ、`objc2` の型が意図せず見えるようになる。

    use tauri_nspanel::tauri_panel;

    tauri_panel! {
        // **`can_become_key_window` は偽である** (spec Boundaries)。真にすると、
        // パネルがキーウィンドウになりうる = 入力先を奪いうる。
        //
        // `is_floating_panel` は他のウィンドウの上に浮かせる。強さは**消えないこと**で
        // 表すのであって大きさでは表さないため、これ以上の水準は与えない。
        panel!(InterventionPanel {
            config: {
                can_become_key_window: false,
                can_become_main_window: false,
                is_floating_panel: true
            }
        })
    }
}

/// 起動時に呼び、ウィンドウを**非活性パネル**へ差し替える (AD-6)。
///
/// **メインスレッドから呼ぶこと。** `NSPanel` は `MainThreadOnly` である。`setup` の
/// 中から呼べば条件を満たす。
///
/// # Errors
///
/// ウィンドウが無いとき、差し替えに失敗したとき、または三つの必須設定のうち
/// スタイルマスクの追加を AppKit が拒んだとき。**いずれも常駐は止めない** — 呼び出し側
/// (`lib.rs`) が記録して進む。
#[cfg(target_os = "macos")]
pub fn install<R: Runtime>(app: &AppHandle<R>) -> Result<(), InterventionError> {
    use tauri_nspanel::{PanelLevel, WebviewWindowExt};

    let window = window(app).ok_or(InterventionError::WindowMissing(INTERVENTION_LABEL))?;
    let panel = window
        .to_panel::<nspanel::InterventionPanel<R>>()
        .map_err(|error| InterventionError::Window(error.to_string()))?;

    // 必須設定 1 — 入力先を奪わない。**加えるのであって置き換えない。**
    panel
        .add_style_mask(panel_style_mask_addition())
        .map_err(|error| InterventionError::Window(error.to_string()))?;
    // 必須設定 2 — 自アプリの非活性化で自ら隠れない。既定は真である。
    panel.set_hides_on_deactivate(false);
    // 必須設定 3 — フルスクリーン作業中にも出る。
    panel.set_collection_behavior(panel_collection_behavior());

    panel.set_level(PanelLevel::Floating.into());
    // **画面の隅に置く。** 強さは消えないことで表し、大きさでも位置の押し出しでも
    // 表さない (spec Design Notes)。置けなくても常駐は止めない — OS の既定の位置で
    // 出るだけである。
    if let Err(error) = place_in_a_corner(&window) {
        log::error!("failed to place the intervention panel in a corner: {error}");
    }
    // 起動直後は隠れている。tauri.conf.json の `visible: false` と対である。
    panel.hide();
    record(false);

    // **キーウィンドウになれてはならない。** なれるなら入力先を奪いうる。落ちても
    // 常駐は止めないが、静かに通してはならない。
    if panel.can_become_key_window() {
        log::error!("the intervention panel can become the key window; it would steal input focus");
    }
    if panel.hides_on_deactivate() {
        log::error!("the intervention panel still hides on deactivate; it would vanish on sight");
    }
    Ok(())
}

/// パネルと画面の縁との間隔 (論理ピクセル)。
#[cfg(target_os = "macos")]
const CORNER_MARGIN: f64 = 24.0;

/// 画面の右下から [`CORNER_MARGIN`] だけ内側に置いたときの位置を決める純粋関数。
///
/// OS を呼ばずに検証できるよう、算術だけを切り出してある
/// (`adapters/presentation` の `toggle_action` と同じ流儀)。**画面の原点を足すのを
/// 忘れると、複数画面で主画面の外に置かれて見えなくなる。**
#[cfg(target_os = "macos")]
#[must_use]
pub fn corner_position(
    screen_origin: (i32, i32),
    screen_size: (u32, u32),
    panel_size: (u32, u32),
    margin: i32,
) -> (i32, i32) {
    let x = screen_origin.0 + i32::try_from(screen_size.0).unwrap_or(i32::MAX)
        - i32::try_from(panel_size.0).unwrap_or(0)
        - margin;
    let y = screen_origin.1 + i32::try_from(screen_size.1).unwrap_or(i32::MAX)
        - i32::try_from(panel_size.1).unwrap_or(0)
        - margin;
    (x, y)
}

/// パネルを画面の右下へ寄せる。
///
/// **画面が読めなければ何もしない。** OS の既定の位置で出るだけであり、介入そのものは
/// 失われない。
#[cfg(target_os = "macos")]
fn place_in_a_corner<R: Runtime>(window: &WebviewWindow<R>) -> Result<(), InterventionError> {
    let monitor = window
        .primary_monitor()
        .map_err(|error| InterventionError::Window(error.to_string()))?;
    let Some(monitor) = monitor else {
        log::info!("no primary monitor was reported; the panel keeps the default position");
        return Ok(());
    };
    let size = window
        .outer_size()
        .map_err(|error| InterventionError::Window(error.to_string()))?;

    let margin = (CORNER_MARGIN * monitor.scale_factor()).round();
    let margin = if margin.is_finite() && margin >= 0.0 {
        margin as i32
    } else {
        0
    };
    let (x, y) = corner_position(
        (monitor.position().x, monitor.position().y),
        (monitor.size().width, monitor.size().height),
        (size.width, size.height),
        margin,
    );
    window
        .set_position(tauri::PhysicalPosition::new(x, y))
        .map_err(|error| InterventionError::Window(error.to_string()))
}

/// 介入パネルに与える collection behavior を決める純粋関数 (AD-6)。
///
/// `FullScreenAuxiliary` がこの集合から落ちると、**フルスクリーン作業中にだけ**パネルが
/// 出なくなる — PRD §7.2 が名指しした、まさにその状況で機能しなくなる。
/// `CanJoinAllSpaces` が落ちると、別の空間へ切り替わってしまう。
#[cfg(target_os = "macos")]
#[must_use]
pub fn panel_collection_behavior() -> objc2_app_kit::NSWindowCollectionBehavior {
    use objc2_app_kit::NSWindowCollectionBehavior as Behavior;

    Behavior::CanJoinAllSpaces | Behavior::FullScreenAuxiliary
}

/// 介入パネルへ与えるスタイルマスクの追加分 (AD-6)。
///
/// **置き換えではなく追加である。** 置き換えれば Tauri が組んだ構造的なスタイルが落ち、
/// 生存中のウィンドウに対する置き換えはプロセスを異常終了させうる。
#[cfg(target_os = "macos")]
#[must_use]
pub fn panel_style_mask_addition() -> objc2_app_kit::NSWindowStyleMask {
    objc2_app_kit::NSWindowStyleMask::NonactivatingPanel
}

/// パネルを出す。**フォーカスは取らない** (AD-6)。
///
/// # Errors
///
/// パネルが取れないとき。
#[cfg(target_os = "macos")]
pub fn show<R: Runtime>(app: &AppHandle<R>) -> Result<(), InterventionError> {
    apply(app, PanelAction::Show)
}

/// パネルを引っ込める。
///
/// # Errors
///
/// パネルが取れないとき。
#[cfg(target_os = "macos")]
pub fn hide<R: Runtime>(app: &AppHandle<R>) -> Result<(), InterventionError> {
    apply(app, PanelAction::Hide)
}

#[cfg(target_os = "macos")]
fn apply<R: Runtime>(app: &AppHandle<R>, action: PanelAction) -> Result<(), InterventionError> {
    use tauri_nspanel::ManagerExt;

    // 読んで・決めて・書くを一つの錠の中で行う (AD-5)。
    let _guard = lock_transition();
    let Some(action) = action_for(action, is_visible()) else {
        return Ok(());
    };

    let panel = app
        .get_webview_panel(INTERVENTION_LABEL)
        .map_err(|_| InterventionError::NotAPanel(INTERVENTION_LABEL))?;
    match action {
        // `show` は `orderFrontRegardless` であり、キーウィンドウにしない。
        // **`set_focus` も `app.show()` も呼ばない** — どちらも入力先を奪う。
        PanelAction::Show => panel.show(),
        PanelAction::Hide => panel.hide(),
    }
    record(visibility_after(action));
    log::info!("intervention panel {action:?} applied");
    Ok(())
}

/// macOS 以外では**介入**の提示面を持たない (v1 のビルド対象は macOS のみ)。
///
/// **黙って成功しない。** 成功として返せば、コア側は**介入**が出ていると信じたまま
/// 応答を待ち続ける。
#[cfg(not(target_os = "macos"))]
pub fn install<R: Runtime>(_app: &AppHandle<R>) -> Result<(), InterventionError> {
    Err(InterventionError::NotAPanel(INTERVENTION_LABEL))
}

#[cfg(not(target_os = "macos"))]
pub fn show<R: Runtime>(_app: &AppHandle<R>) -> Result<(), InterventionError> {
    Err(InterventionError::NotAPanel(INTERVENTION_LABEL))
}

#[cfg(not(target_os = "macos"))]
pub fn hide<R: Runtime>(_app: &AppHandle<R>) -> Result<(), InterventionError> {
    Err(InterventionError::NotAPanel(INTERVENTION_LABEL))
}

/// **介入を発する** — パネルを出し、応答のホットキーを一時登録する (FR-15 / AD-7)。
///
/// **順序が意味を持つ。** パネルを先に出し、ホットキーは後で登録する。逆にすると、
/// 登録に失敗したときに「出さない」へ倒す書き方が自然になってしまう — **登録の失敗を
/// 理由に介入を抑えることは禁じられている** (AD-7)。登録できなかったことは値として
/// 運ばれ、パネルは「クリックだけで応答できる」と述べて出る。
///
/// **メインスレッドから呼ぶこと** (`NSPanel` は `MainThreadOnly`)。
///
/// # Errors
///
/// パネルを出せなかったとき。呼び出し側は [`Core::withdraw_intervention`] で取り下げる。
pub fn raise<R: Runtime>(app: &AppHandle<R>) -> Result<(), InterventionError> {
    show(app)?;
    let status = hotkey::register_response(app);
    if let Some(error) = status.error.as_deref() {
        log::error!("{error}");
        log::info!("the intervention is shown anyway; it can be answered by clicking");
    }
    crate::commands::publish_response_hotkey_status(app, status);
    // **パネルは表示のたびにスナップショットを取り直す** (AD-3 鮮度規則)。非活性で
    // あるためフォーカスの取得という契機を持たず、これが唯一の契機である。
    crate::announce_intervention_raised(app);
    Ok(())
}

/// **介入を閉じる唯一の経路** (AD-7)。
///
/// 応答をコアへ渡し、**コアが受け付けたときにだけ**ホットキーを解除してパネルを
/// 引っ込める。受け付けなければ (既に応答済み・表示されていない) 何もしない —
/// 二重発火や、ホットキーとクリックが同時に届いた場合に**現在地**を二度動かさない。
///
/// # どのスレッドから呼んでよいか
///
/// **ホットキーのハンドラの中からは呼んではならない** ([`answer_off_thread`] を使う)。
/// 解除がプラグインの錠へ再入し、メインスレッドが停止する。パネルの操作はここから
/// メインスレッドへ回す。
///
/// # Errors
///
/// **休息**への遷移を永続化できなかったとき。そのとき**状態は変わらず、介入も閉じない**
/// (I/O マトリクス「休息に入る」)。
pub fn answer<R: Runtime>(app: &AppHandle<R>, choice: InterventionChoice) -> Result<bool, String> {
    let Some(core) = app.try_state::<Core>() else {
        return Err("保存された状態を読み込めていない。介入に応答できない。".to_string());
    };

    let answered = core
        .answer_intervention(choice)
        .map_err(|error| error.to_string())?;
    if !answered {
        log::info!("an answer arrived with no intervention on screen; nothing was done");
        return Ok(false);
    }

    // **解除と非表示は応答が確定した後である。** 先に消すと、書き込みに失敗した応答の
    // ためにパネルだけが消える。
    hotkey::unregister_response(app);
    hide_on_main_thread(app);

    log::info!("the intervention was answered ({choice:?})");
    Ok(true)
}

/// [`answer`] を別のスレッドで行う。**ホットキーのハンドラ専用である** (AD-7)。
///
/// プラグインは `shortcuts` の錠を保持したままハンドラを呼ぶ。その中で解除を呼べば
/// 同じ錠に再入し、メインスレッドが停止する。`run_on_main_thread` で包んでも、既に
/// メインスレッド上なら即時実行されるため救われない — **別のスレッドへ退避する以外に
/// 手が無い。**
pub fn answer_off_thread<R: Runtime>(app: AppHandle<R>, choice: InterventionChoice) {
    std::thread::spawn(move || {
        if let Err(error) = answer(&app, choice) {
            log::error!("failed to answer the intervention: {error}");
        }
    });
}

/// パネルを引っ込める。**`NSPanel` はメインスレッドからしか触れない。**
fn hide_on_main_thread<R: Runtime>(app: &AppHandle<R>) {
    let handle = app.clone();
    if let Err(error) = app.run_on_main_thread(move || {
        if let Err(error) = hide(&handle) {
            log::error!("failed to hide the intervention panel: {error}");
        }
    }) {
        log::error!("failed to hand the intervention panel to the main thread: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ラベルはオーバーレイと別である。**同じにすると閉じる要求が取り違えられる。**
    #[test]
    fn the_panel_has_its_own_label() {
        assert_ne!(
            INTERVENTION_LABEL,
            crate::adapters::presentation::OVERLAY_LABEL
        );
    }

    /// 既に出ているなら出し直さない。既に隠れているなら引っ込め直さない。
    #[test]
    fn an_already_satisfied_request_does_nothing() {
        assert_eq!(action_for(PanelAction::Show, true), None);
        assert_eq!(action_for(PanelAction::Hide, false), None);
        assert_eq!(
            action_for(PanelAction::Show, false),
            Some(PanelAction::Show)
        );
        assert_eq!(action_for(PanelAction::Hide, true), Some(PanelAction::Hide));
    }

    /// 適用後の可視状態。
    #[test]
    fn the_visibility_follows_the_action() {
        assert!(visibility_after(PanelAction::Show));
        assert!(!visibility_after(PanelAction::Hide));
    }

    /// 画面の右下に、縁から余白を空けて置かれる。
    #[cfg(target_os = "macos")]
    #[test]
    fn the_panel_sits_in_the_bottom_right_corner() {
        assert_eq!(
            corner_position((0, 0), (1920, 1080), (340, 150), 24),
            (1920 - 340 - 24, 1080 - 150 - 24)
        );
    }

    /// **画面の原点を足す。** 忘れると、複数画面で主画面の外に置かれて見えなくなる。
    #[cfg(target_os = "macos")]
    #[test]
    fn the_panel_follows_the_screen_origin() {
        assert_eq!(
            corner_position((-1920, 200), (1920, 1080), (340, 150), 24),
            (-1920 + 1920 - 340 - 24, 200 + 1080 - 150 - 24)
        );
    }

    /// パネルが画面より大きくても算術が破綻しない。
    #[cfg(target_os = "macos")]
    #[test]
    fn an_oversized_panel_does_not_overflow() {
        let (x, y) = corner_position((0, 0), (800, 600), (2000, 2000), 24);
        assert!(x < 0 && y < 0);
    }

    /// **三つの必須設定のうち二つは、ここで組み立てたビットがすべてである** (AD-6)。
    ///
    /// `FullScreenAuxiliary` が落ちれば、フルスクリーン作業中にだけ静かに失敗する。
    /// 落ちても他のどの検査も赤くならないため、ここで固定する。
    #[cfg(target_os = "macos")]
    #[test]
    fn the_panel_joins_full_screen_spaces() {
        use objc2_app_kit::NSWindowCollectionBehavior as Behavior;

        let behavior = panel_collection_behavior();
        assert!(
            behavior.contains(Behavior::FullScreenAuxiliary),
            "FullScreenAuxiliary が無いとフルスクリーン作業中にパネルが出ない"
        );
        assert!(
            behavior.contains(Behavior::CanJoinAllSpaces),
            "CanJoinAllSpaces が無いと別の空間へ切り替わる"
        );
    }

    /// 入力先を奪わないスタイルが落ちていないこと (AD-6)。
    #[cfg(target_os = "macos")]
    #[test]
    fn the_panel_does_not_activate_the_app() {
        use objc2_app_kit::NSWindowStyleMask as Mask;

        assert!(
            panel_style_mask_addition().contains(Mask::NonactivatingPanel),
            "NonactivatingPanel が無いとパネルが入力先を奪う"
        );
    }

    /// **追加するスタイルは構造的なスタイルを含まない。**
    ///
    /// `Titled` や `Closable` をここに混ぜると、加算した結果として Tauri が組んだ
    /// 装飾なしのウィンドウに枠が生える。
    #[cfg(target_os = "macos")]
    #[test]
    fn the_style_mask_addition_adds_nothing_structural() {
        use objc2_app_kit::NSWindowStyleMask as Mask;

        let addition = panel_style_mask_addition();
        for structural in [Mask::Titled, Mask::Closable, Mask::Resizable] {
            assert!(
                !addition.contains(structural),
                "構造的なスタイルを加えない: {structural:?}"
            );
        }
    }
}
