//! アプリケーションメニューアダプタ — 編集操作の打鍵を取り戻す面。
//!
//! # なぜ編集メニューが要るのか
//!
//! macOS では修飾キー付きの打鍵は通常の文字入力経路ではなく **key equivalent** として
//! 処理される。`NSApplication` は `sendEvent:` でそれを見つけ、キーウィンドウに
//! `performKeyEquivalent:` を送り、誰も処理しなければ**メニューバーのメニューに**送る。
//! `WKWebView` は編集操作を責任連鎖のアクション (`paste:` など) として実装しており、
//! `performKeyEquivalent:` で Cmd+V を主張しない。**メニューが無ければ打鍵をアクションへ
//! 変える者がいない。** 編集メニューを置くことは、その変換器を戻すことである。
//!
//! この経路は `NSApp.mainMenu` を辿るだけであり、**メニューバーが画面に描かれている
//! 必要は無い。** 本アプリは `ActivationPolicy::Accessory` で走るためメニューバーを
//! 一度も描かないが、それでも key equivalent は解決される。
//!
//! 誤終了阻止の第 1 層 ([`tauri::Builder::enable_macos_default_menu`] を `false` に
//! すること) は既定メニューごと編集項目を消してしまうため、作成の面のような**入力が
//! 主役の面**では Cmd+V・Cmd+C・Cmd+A・Cmd+Z が死ぬ。摩擦は capture に掛かってはならない。
//!
//! # なぜこれが第 1 層を弱めないのか
//!
//! [`tauri::Builder::build`] は、メニューが未設定でかつ `enable_macos_default_menu` が
//! 真のときにだけ `Menu::default` を組み込む。独自のメニューを与えればその分岐に入らない。
//! そして macOS はプログラムから設定した `mainMenu` に項目を補わない — AppKit は
//! `setMainMenu` された内容をそのまま描くだけである。したがって**終了の項目を置かなければ
//! `terminate:` はどの打鍵にも結び付かず、Cmd+Q は「どこにも束縛されていない打鍵」の
//! ままである。** メニューが無い状態と、終了項目の無いメニューがある状態は、Cmd+Q に
//! 関して等価である。
//!
//! **終了への唯一の到達経路はメニューバー項目 ([`crate::adapters::menubar`]) のままと
//! する。** [`tauri::menu::PredefinedMenuItem::quit`] は `NSApplication terminate:` を
//! 直接送り、誤終了阻止の全層を迂回する。ここでは使わない — というより、
//! [`EditItem`] はそれを**表現できない**。規律ではなく構造で禁じている。

use std::fmt;

use tauri::menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Runtime};

/// 編集メニューの表題。
pub const EDIT_SUBMENU_TITLE: &str = "編集";

/// アプリケーションメニューに据える行の定義。
///
/// 手で `MenuItem::with_id(..)` を書くと、`false` を `true` に変えてアクセラレータを
/// 足すだけで Cmd+Q が束縛されうる — 定数は一つも変わらず、テストも全件緑のままで
/// ある。**まさに第 1 層が防いでいる状態**であるため、行の性質を定数のデータとして置き、
/// 検査の対象にする。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuRow {
    /// 項目の id。
    pub id: &'static str,
    /// 表示名。
    pub label: &'static str,
    /// 活性かどうか。
    pub enabled: bool,
    /// アクセラレータ (打鍵の束縛)。
    pub accelerator: Option<&'static str>,
}

/// アプリケーションメニュー (最初のサブメニュー) の中身。
///
/// # この行が何をしているのか
///
/// **利用者はこの行を見ない。** 本アプリは `ActivationPolicy::Accessory` で走るため
/// メニューバーを一度も描かず、アプリケーションメニューは画面に出ない。
///
/// この行の仕事は**最初の枠を埋めることだけ**である。macOS は最上位の最初のサブメニューを
/// アプリケーションメニューとして扱い、その表題をアプリ名に差し替える。編集メニューを
/// 先頭に置けば、編集メニューが名前ごとアプリ名に化けてしまう。空のサブメニューを置く
/// より、何のために在るかを述べた 1 行を置くほうが、後から読む者が消しにくい。
/// **消すとその瞬間に編集メニューが先頭へ繰り上がる。**
///
/// 文言は、将来 activation policy が変わってこの面が描かれた場合の備えである — 終了の
/// 項目をここに置くことはできない (置けば Cmd+Q が束縛され第 1 層の目的が失われる) ため、
/// 唯一の終了経路 (CAP-3) の行き先を示すに留める。
pub const APPLICATION_MENU: &[MenuRow] = &[MenuRow {
    id: "appmenu-placeholder",
    label: "終了はメニューバーの項目から",
    // 活性にしてはならない。この面から起きてよい出来事は無い。
    enabled: false,
    // 打鍵を束縛してはならない。第 1 層は「Cmd+Q がどこにも束縛されていないこと」に
    // 掛かっている。
    accelerator: None,
}];

/// 最上位に置くサブメニューの種別。
///
/// **最上位の要素はすべてサブメニューである。** 平の項目を混ぜるとメニューバーが空に
/// なる既知の不具合があるため、この規律は [`build`] が `Vec<Submenu<_>>` を組み立てる
/// ことで型として担保している。
///
/// macOS では**最初のサブメニューがアプリケーションメニューとして表示される** —
/// 表題はアプリ名に差し替えられ、中身だけがそのまま出る。したがって順序は意味を持つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopLevelSubmenu {
    /// アプリケーションメニュー。macOS が表題をアプリ名に差し替える。
    Application,
    /// 編集メニュー。Cmd+V などの key equivalent をアクションへ変える変換器。
    Edit,
}

/// 最上位の並び。**先頭がアプリケーションメニューになる。**
pub const TOP_LEVEL: &[TopLevelSubmenu] = &[TopLevelSubmenu::Application, TopLevelSubmenu::Edit];

/// 編集項目を**一つの表から**組み立てる。
///
/// 変種・表示名・OS 標準の生成関数を三箇所に分けて書くと、`Paste` に `copy` を割り当てる
/// ような取り違えが起きても、定数も文言も検査も一つとして動かない — **Cmd+V だけが
/// 無言で死ぬ。** 表を一つにすれば、対応が食い違う書き方そのものが存在しなくなる。
/// 対応の正しさ自体も `the_variants_and_their_constructors_are_paired` が見る。
macro_rules! edit_items {
    ($( $(#[$meta:meta])* $variant:ident => $ctor:ident , $label:literal ; )*) => {
        /// 編集メニューに置いてよい項目。
        ///
        /// # 終了と閉じるを表現できないこと
        ///
        /// この列挙には `Quit` も `CloseWindow` も無く、変種はすべて上の表から生える。
        /// したがって**編集メニューに終了や閉じるを紛れ込ませるには、まず表に行を足さ
        /// なければならない。** 「うっかり `PredefinedMenuItem::quit` を書いてしまう」
        /// 経路が構造的に存在しない。
        ///
        /// いずれも OS 標準の項目であり、独自の処理は書かない。キー割り当ても既定の
        /// 挙動も AppKit が持っている — ここが与えるのは日本語の表示名だけである。
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum EditItem {
            $( $(#[$meta])* $variant, )*
            /// 区切り線。
            Separator,
        }

        impl EditItem {
            /// 表示名。区切り線は文言を持たない。
            pub const fn label(self) -> Option<&'static str> {
                match self {
                    $( Self::$variant => Some($label), )*
                    Self::Separator => None,
                }
            }

            /// OS 標準の項目を作る。表の行と 1:1 で対応する。
            fn predefined<R: Runtime>(
                self,
                app: &AppHandle<R>,
            ) -> tauri::Result<PredefinedMenuItem<R>> {
                match self {
                    $( Self::$variant => PredefinedMenuItem::$ctor(app, Some($label)), )*
                    Self::Separator => PredefinedMenuItem::separator(app),
                }
            }
        }

        /// 表が書いた「変種の名前」と「生成関数の名前」の対。検査だけが読む。
        #[cfg(test)]
        const PAIRINGS: &[(&str, &str)] = &[ $( (stringify!($variant), stringify!($ctor)), )* ];
    };
}

edit_items! {
    /// 取り消し (Cmd+Z)。
    Undo => undo, "取り消し";
    /// やり直し (Shift+Cmd+Z)。
    Redo => redo, "やり直し";
    /// 切り取り (Cmd+X)。
    Cut => cut, "切り取り";
    /// コピー (Cmd+C)。
    Copy => copy, "コピー";
    /// 貼り付け (Cmd+V)。**これが本スライスの主目的である。**
    Paste => paste, "貼り付け";
    /// すべてを選択 (Cmd+A)。
    SelectAll => select_all, "すべてを選択";
}

/// 編集メニューの並び。顔ぶれは上の表が、並び順はここが決める。
pub const EDIT_MENU: &[EditItem] = &[
    EditItem::Undo,
    EditItem::Redo,
    EditItem::Separator,
    EditItem::Cut,
    EditItem::Copy,
    EditItem::Paste,
    EditItem::Separator,
    EditItem::SelectAll,
];

/// アプリケーションメニューを据えられなかった理由。
#[derive(Debug)]
pub enum AppMenuError {
    /// メニューの組み立て、または据える呼び出しが失敗した。
    Build(tauri::Error),
    /// 据えたはずのメニューがアプリ全体のメニューとして読み戻せない。
    NotInPlace,
}

impl fmt::Display for AppMenuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Build(error) => write!(f, "アプリケーションメニューを組めなかった: {error}"),
            Self::NotInPlace => {
                write!(
                    f,
                    "アプリケーションメニューが据わっていない (Cmd+V は効かない)"
                )
            }
        }
    }
}

impl std::error::Error for AppMenuError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Build(error) => Some(error),
            Self::NotInPlace => None,
        }
    }
}

impl From<tauri::Error> for AppMenuError {
    fn from(error: tauri::Error) -> Self {
        Self::Build(error)
    }
}

/// アプリケーションメニューを組み立てて据える。
///
/// **失敗しても起動を止めない。** 呼び出し側 (`setup`) は `Err` を記録して進むこと。
/// 編集メニューが無い状態は Cmd+V が効かない状態であって、常駐そのものが立たない状態
/// より軽い。
///
/// # 取り付いたことをどう確かめるか
///
/// [`tauri::AppHandle::set_menu`] は macOS 側の取り付け (`init_for_nsapp`) を
/// `let _ = ..` で捨てる。したがって `Ok` は「組み立てが通った」以上を意味しない —
/// **メニューの無いまま起動しても何もログに残らない。** 据えた直後にアプリ全体の
/// メニューを読み戻し、それが自分の組んだものであることを確かめて初めて、取り付けの
/// 失敗が観測できる状態になる。
pub fn install<R: Runtime>(app: &AppHandle<R>) -> Result<(), AppMenuError> {
    let menu = build(app)?;
    let id = menu.id().clone();
    app.set_menu(menu)?;

    match app.menu() {
        Some(installed) if *installed.id() == id => {
            log::info!(
                "the application menu is in place ({} submenus, {} editing items)",
                TOP_LEVEL.len(),
                EDIT_MENU.len()
            );
            Ok(())
        }
        _ => Err(AppMenuError::NotInPlace),
    }
}

/// アプリケーションメニューを組み立てる。
///
/// 最上位は [`TOP_LEVEL`] の順に並ぶ [`Submenu`] だけである — 中間の `Vec<Submenu<R>>`
/// が、平の項目が混ざらないことを型として保証する。
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let submenus = TOP_LEVEL
        .iter()
        .map(|kind| submenu(*kind, app))
        .collect::<tauri::Result<Vec<Submenu<R>>>>()?;
    let items = submenus
        .iter()
        .map(|submenu| submenu as &dyn IsMenuItem<R>)
        .collect::<Vec<_>>();

    Menu::with_items(app, &items)
}

/// 種別からサブメニューを作る。
fn submenu<R: Runtime>(kind: TopLevelSubmenu, app: &AppHandle<R>) -> tauri::Result<Submenu<R>> {
    match kind {
        TopLevelSubmenu::Application => application_submenu(app),
        TopLevelSubmenu::Edit => edit_submenu(app),
    }
}

/// アプリケーションメニュー。中身は [`APPLICATION_MENU`] の行がすべてである。
fn application_submenu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Submenu<R>> {
    let rows = APPLICATION_MENU
        .iter()
        .map(|row| MenuItem::with_id(app, row.id, row.label, row.enabled, row.accelerator))
        .collect::<tauri::Result<Vec<MenuItem<R>>>>()?;
    let items = rows
        .iter()
        .map(|row| row as &dyn IsMenuItem<R>)
        .collect::<Vec<_>>();
    // 表題は macOS がアプリ名に差し替える。
    let title = app.package_info().name.clone();

    Submenu::with_items(app, title, true, &items)
}

/// 編集メニュー。[`EDIT_MENU`] の順に OS 標準の項目を並べる。
fn edit_submenu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Submenu<R>> {
    let entries = EDIT_MENU
        .iter()
        .map(|item| item.predefined(app))
        .collect::<tauri::Result<Vec<PredefinedMenuItem<R>>>>()?;
    let items = entries
        .iter()
        .map(|item| item as &dyn IsMenuItem<R>)
        .collect::<Vec<_>>();

    Submenu::with_items(app, EDIT_SUBMENU_TITLE, true, &items)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `SelectAll` → `select_all`。表の変種名から生成関数名を導く。
    fn snake_case(pascal: &str) -> String {
        let mut out = String::new();
        for (index, ch) in pascal.char_indices() {
            if ch.is_uppercase() && index > 0 {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        }
        out
    }

    /// I/O マトリクス「貼り付け」— 変種と OS 標準の生成関数が取り違えられていないこと。
    ///
    /// `Paste => copy` と書いても、変種は揃い、文言は正しく、他のどの検査も通る。
    /// **Cmd+V だけが無言で死ぬ** — 押しても何も起きず、エラーも出ない。表の 1 行に
    /// 閉じ込めたうえで、その 1 行の左右が一致していることをここで見る。
    #[test]
    fn the_variants_and_their_constructors_are_paired() {
        for (variant, ctor) in PAIRINGS {
            assert_eq!(
                &snake_case(variant),
                ctor,
                "{variant} に {ctor} が割り当てられている — 対応する打鍵が別の操作になる"
            );
        }
    }

    /// I/O マトリクス「貼り付け」— 作成の面で Cmd+V・Cmd+C・Cmd+X・Cmd+A・Cmd+Z が効くこと。
    ///
    /// 打鍵をアクションへ変えるのはメニュー項目の存在そのものである。一つ落とせば、
    /// その打鍵だけが無言で死ぬ。並びは変えてよいが顔ぶれは変えてはならない。
    #[test]
    fn every_editing_shortcut_has_a_menu_item_to_translate_it() {
        for required in [
            EditItem::Undo,
            EditItem::Redo,
            EditItem::Cut,
            EditItem::Copy,
            EditItem::Paste,
            EditItem::SelectAll,
        ] {
            assert!(
                EDIT_MENU.contains(&required),
                "{required:?} が編集メニューから落ちている — 対応する打鍵が無言で死ぬ"
            );
        }
    }

    /// 区切り線以外はすべて文言を持つ。文言の無い活性項目は空行として描かれる。
    #[test]
    fn only_separators_are_without_a_label() {
        for item in EDIT_MENU {
            let label = item.label();
            if *item == EditItem::Separator {
                assert!(label.is_none(), "区切り線は文言を持たない");
            } else {
                assert!(
                    label.is_some_and(|label| !label.is_empty()),
                    "{item:?} に文言が無い"
                );
            }
        }
    }

    /// 編集メニューの文言が終了や閉じるを名乗らないこと。
    ///
    /// 型は `Quit` を表現できないが、文言まで型が縛るわけではない。既存の項目に
    /// 「終了」と書けば、押しても何も起きない偽の終了経路ができる。
    #[test]
    fn no_editing_item_pretends_to_quit_or_close() {
        for item in EDIT_MENU {
            let Some(label) = item.label() else { continue };
            assert!(!label.contains("終了"), "{item:?} が終了を名乗っている");
            assert!(
                !label.contains("閉じる"),
                "{item:?} が閉じるを名乗っている: {label}"
            );
        }
    }

    /// I/O マトリクス「メニューの構成」— 最初のサブメニューがアプリケーションメニューと
    /// して表示される。
    ///
    /// 順序を入れ替えると、macOS は**編集メニューをアプリケーションメニューとして**
    /// 扱う。表題がアプリ名に差し替わるため、編集メニューは名前ごと消えて見える。
    #[test]
    fn the_application_submenu_comes_first() {
        assert_eq!(TOP_LEVEL.first(), Some(&TopLevelSubmenu::Application));
        assert!(
            TOP_LEVEL.contains(&TopLevelSubmenu::Edit),
            "編集メニューが最上位に無ければ、どの打鍵も変換されない"
        );
    }

    /// I/O マトリクス「メニューの構成」— アプリケーションメニューは打鍵を一つも束縛しない。
    ///
    /// ここが活性になりアクセラレータを持った瞬間、誤終了阻止の第 1 層が守っている
    /// 「Cmd+Q はどこにも束縛されていない」が崩れうる。枠を埋めるための 1 行であって、
    /// 押せる項目ではない。
    #[test]
    fn the_application_submenu_is_a_single_inert_row() {
        assert_eq!(
            APPLICATION_MENU.len(),
            1,
            "枠を埋める 1 行のほかに置くものは無い"
        );
        for row in APPLICATION_MENU {
            assert!(!row.enabled, "{} が活性になっている", row.id);
            assert_eq!(
                row.accelerator, None,
                "{} が打鍵を束縛している — 第 1 層の前提が崩れる",
                row.id
            );
            assert!(!row.label.is_empty(), "{} に文言が無い", row.id);
        }
    }
}
