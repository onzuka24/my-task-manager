# DB で管理しているパラメータ

利用者が変えられるパラメータは SQLite の `setting` 表に置く。別建ての設定ファイルは無い。

## 一覧

すべて秒単位の正の整数である。定義は `src-tauri/src/domain/rest.rs`。

| 鍵 (`key`) | 既定値 | 意味 |
| --- | --- | --- |
| `rest_threshold_seconds` | `3000` (50 分) | 休息閾値。連続作業時間がこれを超えると休息の介入パネルが出る |
| `grace_period_seconds` | `900` (15 分) | 猶予。介入に「猶予」(`Ctrl+Opt+G`) で応じたとき、次に出直すまでの時間 |
| `rest_length_seconds` | `600` (10 分) | 休息の長さ。休息に入ってからこれだけ経つと戻るきっかけのパネルが出る。「もう少し休む」でも同じ長さの後に出直す |

### 値の読み方

`domain/setting.rs` の `positive_seconds` が次のように読む。

- 前後の空白は無視する (`' 300 '` は 300)。
- **行が無い・数値でない・0 以下**なら、エラーにせず既定値を使う。0 を受け入れると刻みの
  たびに介入が出て離れられなくなるため。
- 未知の鍵は単に無視される (打ち間違えても何も起きない — 既定値のままになる)。

### DB では管理していないもの

次はコード内の定数であり、変えるには再ビルドが要る。

| 定数 | 値 | 場所 |
| --- | --- | --- |
| `TICK_INTERVAL_MILLIS` | 10000 | `domain/rest.rs` — 計時の刻み |
| `SLEEP_GAP_FACTOR` | 3 | `domain/rest.rs` — 刻みの何倍の飛びをスリープとみなすか |
| ホットキー | `Ctrl+Opt+Space` ほか | `adapters/hotkey/` |

## 修正方法

**設定を変える画面はまだ無い。** `sqlite3` で直接書き換える。

### 1. 常駐を止める

設定値は**起動時に 1 回だけ**読み込まれ、以後はメモリ上の値を使う。動いている間に DB を
書き換えても反映されないので、先に止める。

```sh
make stop
# または メニューバー項目 → 終了
```

### 2. 書き換える

```sh
DB=~/Library/Application\ Support/dev.onzuka.mytaskmanager/state.sqlite3

# いまの値を見る (空なら全部既定値)
sqlite3 "$DB" 'SELECT key, value FROM setting ORDER BY key;'

# 設定する (無ければ追加、有れば置き換え)
sqlite3 "$DB" "INSERT INTO setting (key, value) VALUES ('rest_threshold_seconds', '2700')
               ON CONFLICT(key) DO UPDATE SET value = excluded.value;"
```

複数をまとめて変える例:

```sh
sqlite3 "$DB" <<'SQL'
INSERT INTO setting (key, value) VALUES
  ('rest_threshold_seconds', '2700'),  -- 45 分
  ('grace_period_seconds',   '600'),   -- 10 分
  ('rest_length_seconds',    '300')    -- 5 分
ON CONFLICT(key) DO UPDATE SET value = excluded.value;
SQL
```

値は TEXT 列である。数値リテラル `2700` で入れても文字列 `'2700'` に変換されて入るが、
表記を揃えるため文字列で書いておく。

### 3. 既定値へ戻す

行を消せば既定値に戻る。

```sh
sqlite3 "$DB" "DELETE FROM setting WHERE key = 'rest_threshold_seconds';"
```

### 4. 起動する

```sh
make open
```

### 動作確認のための短い値

介入を手早く出して確かめたいときの例 (確認後は消して既定値へ戻す):

```sh
sqlite3 "$DB" <<'SQL'
INSERT INTO setting (key, value) VALUES
  ('rest_threshold_seconds', '60'),
  ('grace_period_seconds',   '30'),
  ('rest_length_seconds',    '60')
ON CONFLICT(key) DO UPDATE SET value = excluded.value;
SQL
```

刻みが 10 秒なので、介入が出るのは閾値から最大 10 秒ほど遅れる。

## パラメータを増やすとき

1. `domain/rest.rs` (あるいは該当するドメインのモジュール) に鍵の定数と既定値の定数を置く。
2. 読み手で `setting::positive_seconds` などを使い、無い・壊れている場合は既定値へ落とす。
3. **スキーマの変更は要らない。** `setting` 表は鍵と文字列の組なので、行を足すだけで済む。
4. この文書の一覧に追記する。
