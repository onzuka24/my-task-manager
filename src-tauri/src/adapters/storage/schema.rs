//! スキーマと順序付きマイグレーション。
//!
//! # なぜ `PRAGMA user_version` なのか
//!
//! スパインは移行方式を決めていない。外部のマイグレーションクレートを引かず
//! `user_version` を使うのは、(a) 依存を増やさない (b) 版番号が DB ファイル自身に
//! 入っており別表を読む前に判断できる (c) 単独利用のツールで分岐の履歴が線形である、
//! の三つによる。
//!
//! [`MIGRATIONS`] は**追記専用**である。適用済みの要素を書き換えると、既に走っている
//! インストールでは新しい SQL が二度と適用されない — 手元の DB とコードだけが一致し、
//! 実際に使われている DB は古いままになる。

use rusqlite::Connection;

use crate::ports::storage::StorageError;

/// 版 1 — **タスク** / **ステップ** / **現在地** / **設定値**。
///
/// `step.interruption_note` が**中断メモ**の欄である。**ステップ**に対して 0..1 である
/// ため別表にせず nullable 列とした (スパイン ERD の `STEP ||--o| INTERRUPTION_NOTE`)。
///
/// `current_position` は **1 行しか存在できない表**である。`CHECK (id = 1)` により、
/// 二つ目の**現在地**が SQL のレベルで書けない — FR-6 の一意性を、コアの規律だけに
/// 頼らず永続化側でも担保する。値が無い状態 (**未着手**) は「行が無い」で表す。
///
/// `setting` は AD-11 のための器である。本スライスは値を入れない — 自動起動の印の移行は
/// 既存インストールの引き継ぎを要し、単一ゴールを崩すため deferred-work.md へ送った。
///
/// **`DELETE` する経路をどこにも作っていない。** v1 は**タスク**も**ステップ**も削除
/// しない (消滅の経路は CAP-20 にのみ属する)。
const V1: &str = "\
CREATE TABLE task (
    id    TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL
) STRICT;

CREATE TABLE step (
    id                TEXT PRIMARY KEY NOT NULL,
    task_id           TEXT NOT NULL REFERENCES task(id),
    ordinal           INTEGER NOT NULL,
    content           TEXT NOT NULL,
    completed_at      TEXT,
    interruption_note TEXT
) STRICT;

CREATE INDEX step_by_task ON step(task_id, ordinal);

CREATE TABLE current_position (
    id           INTEGER PRIMARY KEY NOT NULL CHECK (id = 1),
    task_id      TEXT NOT NULL REFERENCES task(id),
    step_id      TEXT NOT NULL REFERENCES step(id),
    is_active    INTEGER NOT NULL CHECK (is_active IN (0, 1)),
    activated_at TEXT NOT NULL
) STRICT;

CREATE TABLE setting (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT;
";

/// 版の順序。**追記のみ。** 添字 + 1 が `PRAGMA user_version` の値になる。
const MIGRATIONS: &[&str] = &[V1];

/// コードが期待するスキーマの版。
pub const LATEST_VERSION: i64 = MIGRATIONS.len() as i64;

/// いまの版から、これから適用すべき SQL の並びを決める純粋関数。
///
/// SQLite を開かずに検証できる形に切り出してある (`adapters/presentation` と同じ流儀)。
/// **コードより新しい DB には何も適用しない** — 新しい版のアプリが書いた DB を古い版で
/// 開いたとき、黙って古いスキーマを被せて壊すより、何もしないほうが安全である。
#[must_use]
pub fn pending(user_version: i64) -> &'static [&'static str] {
    if !(0..LATEST_VERSION).contains(&user_version) {
        return &[];
    }
    #[allow(clippy::cast_sign_loss)]
    let applied = user_version as usize;
    &MIGRATIONS[applied..]
}

/// 接続に、この層が前提とする PRAGMA を設定する。
///
/// - `foreign_keys=ON` — SQLite の既定は **OFF** である。宣言しただけの外部キーは
///   何も守らない。**現在地**が存在しない**ステップ**を指す行を書けてしまう。
/// - `journal_mode=WAL` — 書き込み中に異常終了しても、確定済みのコミットが失われない
///   (AD-5「最後に完了した切り替えの状態に復帰する」)。
/// - `synchronous=NORMAL` — WAL と組で使う既定的な選択。プロセスの異常終了に対しては
///   耐えるが、OS ごと落ちた場合に直近のコミットを失いうる。単独利用のツールで
///   `FULL` の同期コストを毎コミット払う価値が無いと判断した。
///
/// # Errors
///
/// PRAGMA の設定に失敗したとき。
pub fn configure(connection: &Connection) -> Result<(), StorageError> {
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(|error| StorageError::Open(format!("foreign_keys を有効にできない: {error}")))?;
    // journal_mode は設定後の値を返すため pragma_update では扱えない。**返ってきた値を
    // 検査する。** 捨てると、WAL に入れなかったことが何の音も立てずに素通りし、AD-5 の
    // 「確定済みのコミットが失われない」という前提が黙って崩れる。
    let journal_mode: String = connection
        .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))
        .map_err(|error| StorageError::Open(format!("WAL に切り替えられない: {error}")))?;
    // メモリ上の DB は WAL を取れず `memory` を返す。ファイルに対して WAL に入れなかった
    // 場合だけ失敗させる。
    if !journal_mode.eq_ignore_ascii_case("wal") && !journal_mode.eq_ignore_ascii_case("memory") {
        return Err(StorageError::Open(format!(
            "WAL に入れなかった (journal_mode={journal_mode})"
        )));
    }
    connection
        .pragma_update(None, "synchronous", "NORMAL")
        .map_err(|error| StorageError::Open(format!("synchronous を設定できない: {error}")))?;
    Ok(())
}

/// 未適用の版を順に適用する。既に最新なら何もしない。
///
/// 一つの版の適用と `user_version` の更新を**同じトランザクション**に入れる。分ければ、
/// スキーマだけ変わって版番号が古いまま残った DB が生まれ、次の起動で同じ SQL が二度
/// 走る。
///
/// # Errors
///
/// 版番号が読めない、または SQL の適用に失敗したとき。
pub fn migrate(connection: &mut Connection) -> Result<(), StorageError> {
    let user_version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| StorageError::Migration(format!("版番号を読めない: {error}")))?;

    // **下限も見る。** 負の版は `pending` が空を返すため、ここで弾かないとスキーマを
    // 一つも作らないまま `Ok` を返し、失敗が後の「no such table」として現れる。
    if user_version < 0 {
        return Err(StorageError::Migration(format!(
            "保存された状態の版 {user_version} は版番号として成立しない"
        )));
    }
    if user_version > LATEST_VERSION {
        return Err(StorageError::Migration(format!(
            "保存された状態の版 {user_version} はこのアプリが知る版 {LATEST_VERSION} より新しい"
        )));
    }

    for (offset, sql) in pending(user_version).iter().enumerate() {
        let version = user_version + offset as i64 + 1;
        let transaction = connection
            .transaction()
            .map_err(|error| StorageError::Migration(format!("版 {version}: {error}")))?;
        transaction
            .execute_batch(sql)
            .map_err(|error| StorageError::Migration(format!("版 {version}: {error}")))?;
        // PRAGMA は値を束縛できないため組み立てる。`version` は i64 であり外から来ない。
        transaction
            .pragma_update(None, "user_version", version)
            .map_err(|error| StorageError::Migration(format!("版 {version}: {error}")))?;
        transaction
            .commit()
            .map_err(|error| StorageError::Migration(format!("版 {version}: {error}")))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version_of(connection: &Connection) -> i64 {
        connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("版番号は読める")
    }

    fn tables(connection: &Connection) -> Vec<String> {
        let mut statement = connection
            .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")
            .expect("問い合わせは組み立てられる");
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("実行できる");
        rows.map(|row| row.expect("行は読める")).collect()
    }

    /// 新しい DB には全版が適用される。
    #[test]
    fn a_fresh_database_gets_every_version() {
        assert_eq!(pending(0).len(), MIGRATIONS.len());
    }

    /// 最新の DB には何も適用しない。
    #[test]
    fn an_up_to_date_database_gets_nothing() {
        assert!(pending(LATEST_VERSION).is_empty());
    }

    /// **コードより新しい DB には何も適用しない。** 古いスキーマを被せて壊さない。
    #[test]
    fn a_newer_database_is_left_alone() {
        assert!(pending(LATEST_VERSION + 1).is_empty());
        assert!(pending(i64::MAX).is_empty());
    }

    /// 壊れた版番号 (負) でも panic せず、何も適用しない。
    #[test]
    fn a_nonsense_version_applies_nothing() {
        assert!(pending(-1).is_empty());
        assert!(pending(i64::MIN).is_empty());
    }

    /// 受け入れ条件「初回起動で `PRAGMA user_version` が 1 以上」。
    #[test]
    fn migrating_a_fresh_database_creates_the_schema() {
        let mut connection = Connection::open_in_memory().expect("メモリ DB は開ける");
        configure(&connection).expect("PRAGMA を設定できる");
        migrate(&mut connection).expect("適用できる");

        assert!(version_of(&connection) >= 1);
        assert_eq!(
            tables(&connection),
            vec!["current_position", "setting", "step", "task"]
        );
    }

    /// 二度目の適用は何もしない (冪等)。
    #[test]
    fn migrating_twice_is_a_no_op() {
        let mut connection = Connection::open_in_memory().expect("メモリ DB は開ける");
        configure(&connection).expect("PRAGMA を設定できる");
        migrate(&mut connection).expect("1 回目");
        migrate(&mut connection).expect("2 回目も失敗しない");
        assert_eq!(version_of(&connection), LATEST_VERSION);
    }

    /// **成立しない版番号は失敗として返す。** 空の並びを返して `Ok` にすると、表を一つも
    /// 作らないまま起動し、失敗が後の「no such table」として現れる。
    #[test]
    fn a_nonsense_version_is_an_error_not_a_silent_no_op() {
        // `PRAGMA user_version` は 32 ビット符号付きであり、これより小さい値は書けない。
        for version in [-1_i64, i64::from(i32::MIN)] {
            let mut connection = Connection::open_in_memory().expect("メモリ DB は開ける");
            configure(&connection).expect("PRAGMA を設定できる");
            connection
                .pragma_update(None, "user_version", version)
                .expect("版番号は書ける");

            assert!(
                matches!(migrate(&mut connection), Err(StorageError::Migration(_))),
                "版 {version} は拒まれる"
            );
            assert!(
                tables(&connection).is_empty(),
                "拒んだ以上、表を作らないまま Ok を返してもいない"
            );
        }
    }

    /// ファイルに対しては WAL に入り、`synchronous` が `NORMAL` (=1) になっている。
    ///
    /// メモリ DB では WAL が無効であり、この二つは何も検証されない。
    #[test]
    fn a_file_backed_database_runs_in_wal_with_normal_synchronous() {
        let directory = std::env::temp_dir().join(format!(
            "my-task-manager-wal-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&directory).expect("作れる");
        let path = directory.join("state.sqlite3");
        let _ = std::fs::remove_file(&path);

        let mut connection = Connection::open(&path).expect("開ける");
        configure(&connection).expect("PRAGMA を設定できる");
        migrate(&mut connection).expect("適用できる");

        let journal_mode: String = connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .expect("読める");
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");

        let synchronous: i64 = connection
            .pragma_query_value(None, "synchronous", |row| row.get(0))
            .expect("読める");
        assert_eq!(synchronous, 1, "NORMAL");

        drop(connection);
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// コードより新しい版の DB は、黙って被せず失敗として返す。
    #[test]
    fn opening_a_newer_database_is_an_error() {
        let mut connection = Connection::open_in_memory().expect("メモリ DB は開ける");
        connection
            .pragma_update(None, "user_version", LATEST_VERSION + 1)
            .expect("版番号は書ける");
        let outcome = migrate(&mut connection);
        assert!(matches!(outcome, Err(StorageError::Migration(_))));
    }

    /// 外部キーは既定で OFF である。[`configure`] を通した接続では効いていること。
    #[test]
    fn foreign_keys_are_enforced_after_configuration() {
        let mut connection = Connection::open_in_memory().expect("メモリ DB は開ける");
        configure(&connection).expect("PRAGMA を設定できる");
        migrate(&mut connection).expect("適用できる");

        let outcome = connection.execute(
            "INSERT INTO step (id, task_id, ordinal, content) VALUES ('s', 'missing', 1, 'x')",
            [],
        );
        assert!(outcome.is_err(), "存在しないタスクを指すステップは書けない");
    }

    /// **現在地**は SQL のレベルでも 1 行しか存在できない (FR-6)。
    #[test]
    fn the_current_position_table_holds_at_most_one_row() {
        let mut connection = Connection::open_in_memory().expect("メモリ DB は開ける");
        configure(&connection).expect("PRAGMA を設定できる");
        migrate(&mut connection).expect("適用できる");
        connection
            .execute("INSERT INTO task (id, title) VALUES ('t', '原稿')", [])
            .expect("タスクは書ける");
        connection
            .execute(
                "INSERT INTO step (id, task_id, ordinal, content) VALUES ('s', 't', 1, '下書き')",
                [],
            )
            .expect("ステップは書ける");

        let insert =
            "INSERT INTO current_position (id, task_id, step_id, is_active, activated_at) \
                      VALUES (?1, 't', 's', 1, '1970-01-01T00:00:00.000Z')";
        connection.execute(insert, [1]).expect("1 行目は書ける");
        assert!(
            connection.execute(insert, [2]).is_err(),
            "二つ目の現在地は CHECK 制約で書けない"
        );
    }
}
