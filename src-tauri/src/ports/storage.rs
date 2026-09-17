//! ストレージのポート — コアが永続化に要求する契約 (AD-4 / AD-5)。
//!
//! **語彙はコア側のものだけである。** `rusqlite` の型はここに現れてはならない。現れた
//! 時点で、コアが SQLite という選択に縛られ、AD-1 のアダプタ差し替え可能性が失われる。
//!
//! 契約は 2 本しかない。
//!
//! - [`Storage::restore`] — 最後にコミットされた状態を読み戻す (起動時に 1 回)
//! - [`Storage::apply`] — 一つの操作が確定させる変更を、**単一のトランザクション**で
//!   書き込む
//!
//! **1 メソッド = 1 トランザクションである。** 複数回の呼び出しに分けて一つの操作を
//! 確定させてはならない。分ければその隙間で異常終了したとき、**現在地**だけが動いて
//! **完了**が落ちた状態が残りうる — AD-5 が禁じているのはまさにそれである。

use crate::domain::position::CurrentPosition;
use crate::domain::task::Task;

use std::fmt;

/// 起動時に読み戻される、最後にコミットされた状態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoredState {
    /// 全**タスク**。
    pub tasks: Vec<Task>,
    /// 唯一の**現在地**。何も書かれていなければ [`CurrentPosition::NotStarted`]。
    pub current_position: CurrentPosition,
}

impl Default for RestoredState {
    /// DB が空のとき (初回起動) の状態 — I/O マトリクス「初回起動」。
    fn default() -> Self {
        Self {
            tasks: Vec::new(),
            current_position: CurrentPosition::NotStarted,
        }
    }
}

/// 一つの操作が確定させる変更のまとまり。**これが単一トランザクションの単位である。**
///
/// **タスク**は丸ごと渡す。差分 (「この 1 ステップだけ」) を運ばないのは、追記と分割が
/// 必ず後続の連番を書き換えるため、差分の語彙が「ステップの追加」「連番の更新」…と
/// 際限なく増えるからである。**タスク**が持つ**ステップ**は高々数個であり、丸ごと
/// 書き直す代償は無視できる。
///
/// **行を消す変更が存在しない。** v1 は**タスク**も**ステップ**も削除しない (消滅の
/// 経路は CAP-20 にのみ属する)。削除を表現する値を置かないことで、それを構造として
/// 保証する。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Commit {
    /// 書き直す**タスク** (**ステップ**を含む)。変更が無ければ `None`。
    pub task: Option<Task>,
    /// 置き換える**現在地**。変更が無ければ `None`。
    pub current_position: Option<CurrentPosition>,
}

impl Commit {
    /// **タスク**だけを書き直すコミット。
    #[must_use]
    pub fn of_task(task: Task) -> Self {
        Self {
            task: Some(task),
            current_position: None,
        }
    }

    /// **現在地**だけを置き換えるコミット。
    #[must_use]
    pub const fn of_current_position(current_position: CurrentPosition) -> Self {
        Self {
            task: None,
            current_position: Some(current_position),
        }
    }

    /// 書き込むものが何も無いか。
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.task.is_none() && self.current_position.is_none()
    }
}

/// 永続化が失敗した理由。
///
/// アダプタの型 (`rusqlite::Error`) を包まず、文字列に落としてから運ぶ。包めば
/// `rusqlite` がコアの公開型に現れ、このポートの意味が無くなる。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
    /// DB を開けない (パスが作れない・権限が無いなど)。
    Open(String),
    /// スキーマの適用に失敗した。
    Migration(String),
    /// 読み出しに失敗した。
    Read(String),
    /// 書き込みに失敗した。
    Write(String),
    /// 読めたが、コアの不変条件を満たさない値が入っている。
    ///
    /// 起動を止める理由にはしない — I/O マトリクス「異常終了後の起動」は「破損時は
    /// `Err` を返し起動を止めない」と定めている。
    Corrupted(String),
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open(detail) => write!(f, "状態の保存先を開けない: {detail}"),
            Self::Migration(detail) => write!(f, "スキーマの適用に失敗した: {detail}"),
            Self::Read(detail) => write!(f, "状態の読み出しに失敗した: {detail}"),
            Self::Write(detail) => write!(f, "状態の書き込みに失敗した: {detail}"),
            Self::Corrupted(detail) => write!(f, "保存された状態が壊れている: {detail}"),
        }
    }
}

impl std::error::Error for StorageError {}

/// コアが永続化に要求する契約。
///
/// `Send + Sync` を要求するのは、コアが常駐プロセスの複数のスレッド (ホットキーの
/// コールバック・IPC・メインスレッド) から到達されるためである (AD-5)。直列化そのもの
/// はコアとアダプタの内側で行う。
pub trait Storage: Send + Sync {
    /// 最後にコミットされた状態を読み戻す。
    ///
    /// # Errors
    ///
    /// 読み出しに失敗した、または読めた値がコアの不変条件を満たさないとき。
    fn restore(&self) -> Result<RestoredState, StorageError>;

    /// 一つの操作が確定させる変更を、**単一のトランザクション**で書き込む。
    ///
    /// # Errors
    ///
    /// 書き込みに失敗したとき。失敗したコミットは一部だけ適用されてはならない。
    fn apply(&self, commit: &Commit) -> Result<(), StorageError>;
}
