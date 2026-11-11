# DB 設計

SQLite 1 ファイルである。定義は `src-tauri/src/adapters/storage/schema.rs`、読み書きは
`src-tauri/src/adapters/storage/mod.rs` にある。

- 場所: `~/Library/Application Support/dev.onzuka.mytaskmanager/state.sqlite3`
- ドライバ: `rusqlite` (`bundled` — SQLite 本体を同梱し、OS 同梱版の差を排除する)
- 全表 `STRICT`

## ER 図

```mermaid
erDiagram
    task ||--|{ step : "持つ (1..N)"
    task ||--o| current_position : "指される"
    step ||--o| current_position : "指される"
    step ||--o{ switch_record : "離脱元"
    task ||--o{ work_segment : "数えた先"
    step ||--o{ work_segment : "数えた先 (待機は NULL)"

    task {
        TEXT id PK "UUID v7"
        TEXT title
    }
    step {
        TEXT id PK "UUID v7"
        TEXT task_id FK
        INTEGER ordinal "タスク内 1..N"
        TEXT content
        TEXT completed_at "NULL = 未完了"
        TEXT interruption_note "中断メモ / NULL 可"
    }
    current_position {
        INTEGER id PK "常に 1"
        TEXT task_id FK
        TEXT step_id FK
        INTEGER is_active "0 / 1"
        TEXT activated_at
    }
    switch_record {
        TEXT id PK "UUID v7"
        TEXT departed_step_id FK
        TEXT occurred_at
        INTEGER note_written "0 / 1"
    }
    setting {
        TEXT key PK
        TEXT value
    }
    idea {
        TEXT id PK "UUID v7"
        TEXT content
        TEXT created_at
    }
    work_segment {
        TEXT id PK "UUID v7"
        TEXT kind "step / wait"
        TEXT task_id FK
        TEXT step_id FK "wait なら NULL"
        TEXT label "待機の一言 / NULL 可"
        TEXT started_at
        TEXT ended_at
    }
```

`setting` と `idea` はどの表とも結ばれていない。

## 表

### `task` — タスク

| 列 | 型 | 制約 | 意味 |
| --- | --- | --- | --- |
| `id` | TEXT | PK, NOT NULL | UUID v7 |
| `title` | TEXT | NOT NULL | 題名 |

### `step` — ステップ

| 列 | 型 | 制約 | 意味 |
| --- | --- | --- | --- |
| `id` | TEXT | PK, NOT NULL | UUID v7。現在地はこの ID を指す (`ordinal` ではない) |
| `task_id` | TEXT | NOT NULL, FK → `task.id` | 所属タスク |
| `ordinal` | INTEGER | NOT NULL | タスク内の通し番号 1..N。追記のたびに振り直す |
| `content` | TEXT | NOT NULL | ステップの中身 |
| `completed_at` | TEXT | NULL 可 | 完了時刻。NULL なら未完了 |
| `interruption_note` | TEXT | NULL 可 | 中断メモ (ステップに対して 0..1 なので別表にしない) |

索引: `step_by_task (task_id, ordinal)`

タスクの全ステップに `completed_at` が入ると、そのタスクは一覧から消える (行は残る)。

### `current_position` — 現在地

| 列 | 型 | 制約 | 意味 |
| --- | --- | --- | --- |
| `id` | INTEGER | PK, `CHECK (id = 1)` | **1 行しか存在できない** |
| `task_id` | TEXT | NOT NULL, FK → `task.id` | 現在地のタスク |
| `step_id` | TEXT | NOT NULL, FK → `step.id` | 現在地のステップ |
| `is_active` | INTEGER | `CHECK IN (0, 1)` | 1 = 活性 (作業中) / 0 = 非活性 (休息中) |
| `activated_at` | TEXT | NOT NULL | 活性になった時刻。連続作業時間の起点 |

**行が無い = 未着手**である。

### `switch_record` — 切り替え履歴 (版 2 で追加)

| 列 | 型 | 制約 | 意味 |
| --- | --- | --- | --- |
| `id` | TEXT | PK, NOT NULL | UUID v7 |
| `departed_step_id` | TEXT | NOT NULL, FK → `step.id` | 離れたステップ |
| `occurred_at` | TEXT | NOT NULL | 切り替えた時刻 |
| `note_written` | INTEGER | `CHECK IN (0, 1)` | 中断メモを書いたか。**本文は持たない** |

追記専用で、製品コードには読み出す経路が無い (測定用。必要時に SQL で直接数える)。
索引は張っていない。

### `idea` — 思いつき (版 3 で追加)

| 列 | 型 | 制約 | 意味 |
| --- | --- | --- | --- |
| `id` | TEXT | PK, NOT NULL | UUID v7 |
| `content` | TEXT | NOT NULL | 一行の本文 |
| `created_at` | TEXT | NOT NULL | 置いた時刻。寿命の起点 |

**行を消す唯一の表である。** 消える経路は二つだけ:

- **昇格** — 開示面から作成の面を経てタスクを作ったとき。タスクの作成と**同じ
  トランザクション**で消す。
- **寿命** — `created_at` + `idea_lifetime_seconds` (既定 14 日) を過ぎたものを、計時の
  刻み (`Core::tick`) が 1 コミットで消す。何も表示しない。

外部キーも索引も持たない。

### `work_segment` — 作業区間 (版 4 で追加)

現在地が活性だった時間を、ステップごとの区間として残す。勤怠・工数の報告のための参考値で
ある。

| 列 | 型 | 制約 | 意味 |
| --- | --- | --- | --- |
| `id` | TEXT | PK, NOT NULL | UUID v7 |
| `kind` | TEXT | NOT NULL, `CHECK IN ('step', 'wait')` | `step` = ステップの作業 / `wait` = 待機 |
| `task_id` | TEXT | NOT NULL, FK → `task.id` | ステップのタスク。待機なら戻り先のタスク |
| `step_id` | TEXT | FK → `step.id` | `step` なら必須、`wait` なら NULL |
| `label` | TEXT | NULL 可 | 待機の「何を待っているか」。`step` なら NULL |
| `started_at` | TEXT | NOT NULL | 区間の始まり |
| `ended_at` | TEXT | NOT NULL, `started_at < ended_at` | 区間の終わり。長さ 0 の区間は書かない |

- **閉じた区間だけが行になる。** 開いている区間 (種類・ステップ・開始時刻) はメモリにだけ
  あり、永続化しない。状態が変わったとき (現在地の移動・休息の出入り・待機の出入り・
  スリープからの復帰・メニューバー項目の「終了」・Mac のログアウト / 再起動 /
  シャットダウンの直前 — `NSWorkspaceWillPowerOffNotification`) に、その変化のコミットに
  相乗りして 1 行書く。**定期的には書かない。** 異常終了したときに開いていた区間は
  失われる。
- 数えないもの: 休息中・スリープ中 (刻みの飛びで検出し、直前の刻みで閉じて戻った時刻から
  開き直す)・未着手・常駐が止まっていた間。スリープでなくても刻みが 30 秒以上空けば
  (タイマーの間引き・時計を進めたなど) スリープとみなして数えない。実際のスリープの前後
  10 秒ほどはずれうる。
- 最終ステップを完了しても現在地は動かないため、次に状態が変わるまでその時間は完了した
  ステップの区間に数える。
- `label` は待機の「何を待っているか」をそのまま保存する (待機そのものは永続化しない)。
- 待機中に現在地が戻り先と同じタスクにある時間は `wait` の区間になり、主作業のステップには
  数えない。待機中に別タスクへ移った時間はそのステップの `step` の区間である。
- **追記専用で、製品コードには読み出す経路が無い** (記録するが表示しない / AD-15)。
  読み手は `scripts/export-work-log.sh` だけであり、日 × ステップの CSV に集計する
  (使い方は README)。索引は張っていない。

### `setting` — 設定値

| 列 | 型 | 制約 | 意味 |
| --- | --- | --- | --- |
| `key` | TEXT | PK, NOT NULL | 鍵 |
| `value` | TEXT | NOT NULL | 値 (文字列のまま保存し、解釈は読み手が行う) |

**空であることが正常な状態である。** 行が無ければコード内の既定値を使う。使っている鍵と
修正方法は [settings.md](settings.md) を参照。

## 値の表現

| 種類 | 表現 |
| --- | --- |
| ID | UUID v7 の文字列 (`uuid` クレート) |
| 時刻 | UTC の ISO 8601、ミリ秒付き。`YYYY-MM-DDTHH:MM:SS.fffZ` (オフセット表記は読めない) |
| 真偽 | INTEGER 0 / 1 |

## 接続時の PRAGMA

`schema::configure` が開くたびに設定する。

| PRAGMA | 値 | 理由 |
| --- | --- | --- |
| `foreign_keys` | ON | SQLite の既定は OFF で、宣言した外部キーが効かない |
| `journal_mode` | WAL | 書き込み中に落ちても確定済みのコミットを失わない。WAL に入れなければ失敗にする |
| `synchronous` | NORMAL | WAL と組の選択。OS ごと落ちた場合は直近のコミットを失いうる |

## 書き込みの単位

`ports/storage.rs` の `Storage::apply(&Commit)` が **1 回 = 1 トランザクション**である。
`Commit` は「書き直すタスク (ステップ込み)」「置き換える現在地」「追記する切り替え履歴」
「置き換える設定値」「追加する思いつき」「削除する思いつき」「追記する作業区間」を同時に
運ぶ。切り替え 1 回のメモ確定・現在地移動・履歴追記が割れずに確定するのも、昇格でタスクの
作成と思いつきの削除が割れずに確定するのも、閉じた作業区間が状態の変化と同じ
トランザクションで書かれるのもこのためである。

**消せるのは思いつきだけである。** タスクもステップも削除しない (`DELETE` は `idea` と、
未着手へ戻すための `current_position` の 1 行にしか向かない)。

## マイグレーション

外部クレートを使わず `PRAGMA user_version` で版を管理する。

| 版 | 内容 |
| --- | --- |
| 1 | `task` / `step` / `current_position` / `setting` |
| 2 | `switch_record` |
| 3 | `idea` |
| 4 | `work_segment` |

- `MIGRATIONS` は**追記専用**。適用済みの版の SQL を書き換えても、既存の DB には二度と
  適用されない。列や表を足すときは新しい版を末尾に足す。
- 1 版の適用と `user_version` の更新は同じトランザクションで行う。
- コードより新しい版・負の版の DB は、何も適用せずエラーにする (常駐は止めない)。

現在の版は次で確かめられる。

```sh
sqlite3 ~/Library/Application\ Support/dev.onzuka.mytaskmanager/state.sqlite3 'PRAGMA user_version;'
```
