//! 提示アダプタ — オーバーレイの表示・非表示とフォーカス復帰 (AD-6)。
//!
//! オーバーレイは起動時に生成して隠したままにする。ホットキー押下時に生成しないのは
//! CAP-1 の 300ms 制約を満たすためである (tauri.conf.json の `visible: false`)。
//! **一度生成したら破棄しない** — 閉じる要求は破棄ではなく非表示に変換する。
//!
//! ここは OS 呼び出しを行う層であるため、「表示中かどうか」から「次に何をするか」を
//! 決める判断だけを純粋関数 ([`toggle_action`] / [`close_action`]) に切り出してある。
//! 判断の正しさは OS を起動せずに単体テストで確認できる。

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use tauri::{AppHandle, Manager, Runtime, WebviewWindow};

/// オーバーレイウィンドウのラベル。tauri.conf.json の `app.windows[].label` と一致する。
pub const OVERLAY_LABEL: &str = "main";

/// オーバーレイの可視状態。**この層が唯一の所有者である。**
///
/// `WebviewWindow::is_visible()` を判断の入力に使わないのは、NSApplication ごと隠した
/// 状態 (`AppHandle::hide`) での戻り値が AppKit 側の事情に左右され、押下が無反応に見える
/// 事故を招くためである。表示・非表示はすべてこのモジュールの [`apply`] を通るため、
/// ここに記録した値が正となる。
static OVERLAY_VISIBLE: AtomicBool = AtomicBool::new(false);

/// 可視状態の遷移を直列化する錠 (AD-5 の排他規則)。
///
/// ホットキーのコールバックはメインスレッド外から、`hide_overlay` コマンドは IPC の
/// スレッドから到達する。「読んで・決めて・書く」を素の [`AtomicBool`] で行うと二つの
/// 遷移が交錯し、記録と実体が食い違う。状態を変えうる経路はすべてこの錠を通す。
static TRANSITION: Mutex<()> = Mutex::new(());

/// 遷移の錠を取る。毒されていても常駐は止めない。
fn lock_transition() -> MutexGuard<'static, ()> {
    TRANSITION.lock().unwrap_or_else(|error| error.into_inner())
}

/// 提示に失敗した理由。
///
/// **「ウィンドウが無い」を「隠れている」と同じ値に潰さない。** 潰すと、閉じたと
/// 呼び出し側に伝えながら実際には何も起きていない状態を作る。
#[derive(Debug)]
pub enum PresentationError {
    /// オーバーレイウィンドウを取得できない。起動時の生成に失敗している。
    WindowMissing(&'static str),
    /// OS 呼び出しが失敗した。
    Window(tauri::Error),
}

impl fmt::Display for PresentationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WindowMissing(label) => {
                write!(f, "オーバーレイウィンドウ `{label}` が存在しない")
            }
            Self::Window(error) => write!(f, "オーバーレイの操作に失敗した: {error}"),
        }
    }
}

impl std::error::Error for PresentationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::WindowMissing(_) => None,
            Self::Window(error) => Some(error),
        }
    }
}

impl From<tauri::Error> for PresentationError {
    fn from(error: tauri::Error) -> Self {
        Self::Window(error)
    }
}

/// オーバーレイに対して次に行う操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayAction {
    /// 表示してフォーカスを与える。
    Show,
    /// 隠して、直前に最前面だったアプリケーションへフォーカスを返す。
    Hide,
}

/// 現在の可視状態から、トグル (ホットキー押下) で行うべき操作を決める。
///
/// OS 呼び出しを含まない純粋関数である。1 押下につき 1 回だけ呼ばれることは呼び出し側
/// (ホットキーアダプタ) の責務であり、ここでは判断しない。
pub const fn toggle_action(is_visible: bool) -> OverlayAction {
    if is_visible {
        OverlayAction::Hide
    } else {
        OverlayAction::Show
    }
}

/// 閉じる要求 (Esc・フォーカス離脱・ウィンドウの閉じる要求) で行うべき操作を決める。
///
/// トグルと違い、閉じる要求は決してオーバーレイを開かない。既に隠れているときは
/// 行うべき操作が無く `None` を返す — OS を二度叩かないためであり、同時に
/// 「閉じる要求がトグルになっていない」ことを可視状態の関数として表現するためでもある。
pub const fn close_action(is_visible: bool) -> Option<OverlayAction> {
    if is_visible {
        Some(OverlayAction::Hide)
    } else {
        None
    }
}

/// 閉じる要求の結末。
///
/// **「ウィンドウが無い」を「隠れている」と同じ値に潰さない。** 潰すと、閉じたと
/// 呼び出し側に伝えながら実際には何も起きていない状態を作る。型で分けることで、
/// その取り違えを OS を起動せずに検証できる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HideOutcome {
    /// 実際に隠す操作を行う。
    Apply(OverlayAction),
    /// 既に隠れている。行うべき操作は無いが、失敗ではない。
    AlreadyHidden,
    /// オーバーレイウィンドウが存在しない。成功として返してはならない。
    WindowMissing,
}

/// 閉じる要求の結末を、可視状態とウィンドウの有無から決める純粋関数。
pub const fn hide_outcome(is_visible: bool, window_present: bool) -> HideOutcome {
    if !window_present {
        return HideOutcome::WindowMissing;
    }
    match close_action(is_visible) {
        Some(action) => HideOutcome::Apply(action),
        None => HideOutcome::AlreadyHidden,
    }
}

/// [`OverlayAction`] を適用した後の可視状態。テストと実装で同じ規則を使うための純粋関数。
pub const fn visibility_after(action: OverlayAction) -> bool {
    matches!(action, OverlayAction::Show)
}

/// オーバーレイウィンドウを取得する。
pub fn overlay<R: Runtime>(app: &AppHandle<R>) -> Option<WebviewWindow<R>> {
    app.get_webview_window(OVERLAY_LABEL)
}

/// この層が記録している可視状態。
pub fn is_visible() -> bool {
    OVERLAY_VISIBLE.load(Ordering::SeqCst)
}

/// 記録を実体に合わせる。可視状態を変える呼び出しが**成功した直後**にのみ呼ぶ。
///
/// フォーカスや NSApplication の扱いは best-effort であり、そこで失敗しても
/// ウィンドウ自体は既に表示/非表示になっている。記録を後回しにすると、実体と
/// 食い違ったまま次の押下が死ぬ。
fn record(visible: bool) {
    OVERLAY_VISIBLE.store(visible, Ordering::SeqCst);
}

/// 記録を「隠れている」に戻す。フロントが代替経路で自ら隠したときに使う。
pub fn mark_hidden() {
    let _guard = lock_transition();
    record(false);
    log::info!("overlay visibility record reset to hidden");
}

fn apply<R: Runtime>(app: &AppHandle<R>, action: OverlayAction) -> Result<bool, PresentationError> {
    let window = overlay(app).ok_or(PresentationError::WindowMissing(OVERLAY_LABEL))?;

    match action {
        OverlayAction::Show => {
            // NSApplication 側と対で扱う。Hide で `app.hide()` を呼んでいる以上、
            // Show で `app.show()` を呼ばないと 2 回目以降の前面化が不確実になる。
            // 可視状態そのものを変えるのは window.show() であり、こちらは best-effort。
            #[cfg(target_os = "macos")]
            if let Err(error) = app.show() {
                log::error!("failed to unhide the application: {error}");
            }
            window.show()?;
            record(true);
            // Accessory (Dock 非表示) では show() だけでは前面に来ずフォーカスも得ない。
            // 失敗しても既に表示されている以上、記録は表示のままにする。
            if let Err(error) = window.set_focus() {
                log::error!("failed to focus the overlay: {error}");
            }
        }
        OverlayAction::Hide => {
            window.hide()?;
            record(false);
            // ウィンドウを隠しても macOS はアプリの活性を手放さない (tauri#7540)。
            // NSApplication の hide が、直前に最前面だったアプリへ活性を返す唯一の経路。
            #[cfg(target_os = "macos")]
            if let Err(error) = app.hide() {
                log::error!("failed to hand activation back to the previous app: {error}");
            }
        }
    }

    let visible = visibility_after(action);
    log::info!("overlay {action:?} applied (visible={visible})");
    Ok(visible)
}

/// 表示中なら隠し、隠れているなら表示する。適用後の可視状態を返す。
pub fn toggle<R: Runtime>(app: &AppHandle<R>) -> Result<bool, PresentationError> {
    // 読んで・決めて・書くを一つの錠の中で行う (AD-5)。
    let _guard = lock_transition();
    apply(app, toggle_action(is_visible()))
}

/// 表示してフォーカスを与える。
pub fn show<R: Runtime>(app: &AppHandle<R>) -> Result<bool, PresentationError> {
    let _guard = lock_transition();
    apply(app, OverlayAction::Show)
}

/// 隠して、直前に最前面だったアプリケーションへフォーカスを返す。
///
/// 既に隠れているときは何もしないが、ウィンドウの存在だけは確かめる。存在しないことを
/// 成功 (= 隠れている) として返してしまうと、呼び出し側は閉じたと信じてしまう。
pub fn hide<R: Runtime>(app: &AppHandle<R>) -> Result<bool, PresentationError> {
    // 読んで・決めて・書くを一つの錠の中で行う (AD-5)。
    let _guard = lock_transition();
    match hide_outcome(is_visible(), overlay(app).is_some()) {
        HideOutcome::Apply(action) => apply(app, action),
        HideOutcome::AlreadyHidden => Ok(false),
        HideOutcome::WindowMissing => Err(PresentationError::WindowMissing(OVERLAY_LABEL)),
    }
}

/// オーバーレイを、他アプリのフルスクリーン空間でも最前面に出せるようにする (CAP-1)。
///
/// `tao` の `set_visible_on_all_workspaces` (tauri.conf.json の
/// `visibleOnAllWorkspaces`) は `CanJoinAllSpaces` しか立てず、`FullScreenAuxiliary` は
/// `tao` にも `WindowConfig` にも存在しない。これを立てないと、他アプリがフルスクリーン
/// の空間でホットキーを押したときにオーバーレイが出ないか、空間が切り替わる。
///
/// `NSWindow` は `MainThreadOnly` クラスであるため、メインスレッドから呼ぶこと。
#[cfg(target_os = "macos")]
pub fn allow_fullscreen_spaces<R: Runtime>(app: &AppHandle<R>) -> Result<(), PresentationError> {
    use objc2_app_kit::NSWindow;

    let window = overlay(app).ok_or(PresentationError::WindowMissing(OVERLAY_LABEL))?;
    let ptr = window.ns_window()? as *mut NSWindow;
    if ptr.is_null() {
        return Err(PresentationError::WindowMissing(OVERLAY_LABEL));
    }

    // SAFETY: ns_window() は生存中の NSWindow を返す。ウィンドウは起動時に生成され、
    // 破棄しない (閉じる要求は非表示に変換する) ため、この参照の寿命の間は有効である。
    let ns_window: &NSWindow = unsafe { &*ptr };
    ns_window.setCollectionBehavior(overlay_collection_behavior());
    Ok(())
}

/// オーバーレイに与える collection behavior を決める純粋関数。
///
/// `FullScreenAuxiliary` がこの集合から落ちると、他アプリのフルスクリーン空間で
/// オーバーレイが出なくなる — `CanJoinAllSpaces` だけでは足りない。OS を起動せずに
/// 検証できるよう、ビットの組み立てだけを切り出してある。
#[cfg(target_os = "macos")]
pub fn overlay_collection_behavior() -> objc2_app_kit::NSWindowCollectionBehavior {
    use objc2_app_kit::NSWindowCollectionBehavior as Behavior;

    Behavior::CanJoinAllSpaces | Behavior::FullScreenAuxiliary
}

#[cfg(test)]
mod tests {
    use super::*;

    /// I/O マトリクス「呼び出し」— 非表示のときは表示側へ倒す。
    #[test]
    fn hidden_overlay_is_shown() {
        assert_eq!(toggle_action(false), OverlayAction::Show);
        assert!(visibility_after(OverlayAction::Show));
    }

    /// I/O マトリクス「トグル」— 表示中のホットキーは非表示へ倒す。
    #[test]
    fn visible_overlay_is_hidden() {
        assert_eq!(toggle_action(true), OverlayAction::Hide);
        assert!(!visibility_after(OverlayAction::Hide));
    }

    /// I/O マトリクス「閉じる」/「フォーカス離脱」— 閉じる要求は可視状態の関数であり、
    /// 表示中にのみ Hide を生む。
    #[test]
    fn a_close_request_hides_only_a_visible_overlay() {
        assert_eq!(close_action(true), Some(OverlayAction::Hide));
        assert_eq!(close_action(false), None);
    }

    /// 閉じる要求はトグルではない — 隠れている状態で閉じる要求が来ても開かない。
    ///
    /// トグルは同じ入力で Show を返す。両者が分岐することを固定する。
    #[test]
    fn a_close_request_never_opens_the_overlay() {
        assert_eq!(toggle_action(false), OverlayAction::Show);
        assert_ne!(close_action(false), Some(toggle_action(false)));

        for visible in [true, false] {
            assert_ne!(
                close_action(visible),
                Some(OverlayAction::Show),
                "可視状態 {visible} でも閉じる要求は決して Show を生まない"
            );
        }
    }

    /// トグルは 2 回で元に戻る。
    #[test]
    fn toggle_is_an_involution() {
        let mut visible = false;
        for _ in 0..2 {
            visible = visibility_after(toggle_action(visible));
        }
        assert!(!visible);
    }

    /// I/O マトリクス「ウィンドウが無い」— 不在を「隠れている」と潰さない。
    ///
    /// 潰すと、閉じたと呼び出し側に伝えながら実際には何も起きていない状態になる。
    /// **隠れている状態で不在になったときが最も紛れやすい** — どちらも「見えない」
    /// ため、成功として返してしまいやすい。
    #[test]
    fn a_missing_window_is_never_reported_as_hidden() {
        assert_eq!(hide_outcome(false, false), HideOutcome::WindowMissing);
        assert_eq!(hide_outcome(true, false), HideOutcome::WindowMissing);

        assert_ne!(
            hide_outcome(false, false),
            HideOutcome::AlreadyHidden,
            "ウィンドウ不在を「既に隠れている」と同一視しないこと"
        );
    }

    /// ウィンドウがあるときは、可視状態どおりの結末になる。
    #[test]
    fn a_present_window_follows_the_visibility() {
        assert_eq!(
            hide_outcome(true, true),
            HideOutcome::Apply(OverlayAction::Hide)
        );
        assert_eq!(hide_outcome(false, true), HideOutcome::AlreadyHidden);
    }

    /// I/O マトリクス「フルスクリーン空間」— `FullScreenAuxiliary` が落ちていないこと。
    ///
    /// `tao` の `set_visible_on_all_workspaces` (tauri.conf.json の
    /// `visibleOnAllWorkspaces`) は `CanJoinAllSpaces` しか立てない。これだけでは
    /// 他アプリのフルスクリーン空間にオーバーレイが出ない。
    #[cfg(target_os = "macos")]
    #[test]
    fn the_overlay_joins_other_apps_full_screen_spaces() {
        use objc2_app_kit::NSWindowCollectionBehavior as Behavior;

        let behavior = overlay_collection_behavior();
        assert!(
            behavior.contains(Behavior::FullScreenAuxiliary),
            "FullScreenAuxiliary が無いとフルスクリーン空間でオーバーレイが出ない"
        );
        assert!(
            behavior.contains(Behavior::CanJoinAllSpaces),
            "CanJoinAllSpaces が無いと別の空間へ切り替わってしまう"
        );
    }

    /// 閉じる要求は冪等である — 2 回続けても状態が反転しない。
    #[test]
    fn a_close_request_is_idempotent() {
        let mut visible = true;
        for _ in 0..2 {
            visible = match close_action(visible) {
                Some(action) => visibility_after(action),
                None => visible,
            };
        }
        assert!(!visible);
    }
}
