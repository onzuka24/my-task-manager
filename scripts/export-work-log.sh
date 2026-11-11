#!/usr/bin/env bash
#
# 作業区間を「日 × ステップ」の CSV にして標準出力へ書き出す。
#
# 勤怠・工数の報告のための参考値である。画面には一切出さない (AD-15) — 読み手は
# このスクリプトだけであり、DB を直接読む。sqlite3 だけで動く。アプリが動いている
# 間に実行してよい。DB は `PRAGMA query_only` で開くため行を書き換えない。ただし
# アプリが動いていないときは、sqlite が WAL の付随ファイル (-wal / -shm) を作って
# 片付けることがある。
#
# 使い方:
#   scripts/export-work-log.sh                         # 今月 1 日から今日まで
#   scripts/export-work-log.sh 2026-11-01 2026-11-30   # 両端を含む
#   scripts/export-work-log.sh 2026-11-01              # 11 月 1 日から今日まで
#   scripts/export-work-log.sh 2026-11-01 2026-11-30 > 2026-11.csv
#
# 出力は UTF-8 で、先頭に常に BOM を付ける (Mac の Excel でそのまま開いて日本語が
# 化けないため)。
#
# 列: 日付,タスク,ステップ番号,ステップ,分
#   - 1 行 = 1 日 × 1 ステップ。日付はローカル時刻の日であり、日をまたぐ区間は
#     0 時で分ける。
#   - 待機は別の行になる。ステップ番号は空、ステップは「待機: <一言>」(一言が
#     無ければ「待機」)。
#   - 分は小数 2 桁 (秒の精度)。15 分単位などの丸めはしない — 報告先に合わせて
#     表計算で掛けること。
#   - タスク名とステップの内容は書き出した時点のものである (後から直していれば、
#     直した形で出る)。
#
# 精度の限界 (参考値である):
#   - 数えるのは現在地が活性だった時間だけ。休息中・Mac のスリープ中・未着手・
#     常駐が止まっていた間は数えない。休息を宣言しない離席はそのまま数える。
#   - 区間は状態が変わった瞬間 (現在地の移動・休息や待機の出入り・メニューバー項目の
#     「終了」) にだけ書かれる。異常終了したときに開いていた区間は残らない。
#   - いま開いている (まだ閉じていない) 区間は含まれない。
#
# 環境変数:
#   MY_TASK_MANAGER_DB  DB の場所 (既定: ~/Library/Application Support/
#                       dev.onzuka.mytaskmanager/state.sqlite3 — make uninstall が
#                       消すのと同じ場所)
#
# 失敗 (DB が無い・日付が不正・sqlite3 が無い) は理由を標準エラーへ出して非 0 で終わる。
#
##### ヘッダここまで (usage はここまでを表示する) #####
set -euo pipefail

DB="${MY_TASK_MANAGER_DB:-$HOME/Library/Application Support/dev.onzuka.mytaskmanager/state.sqlite3}"

usage() {
  local last
  last=$(grep -n '^##### ヘッダここまで' "$0" | head -1 | cut -d: -f1)
  sed -n "2,$((last - 1))p" "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

fail() {
  echo "export-work-log: $1" >&2
  exit "${2:-1}"
}

case "${1:-}" in
  -h | --help) usage 0 ;;
esac
[ "$#" -le 2 ] || fail "引数は高々二つ (開始日 終了日)。-h で使い方を表示する。" 2

command -v sqlite3 >/dev/null 2>&1 || fail "sqlite3 が見つからない。"

# 日付は YYYY-MM-DD であり、暦に実在すること。sqlite の date() は 2 月 30 日を 3 月 2 日へ
# 繰り越すため、元の文字列と一致するかで実在を確かめる。形を先に検めるため、以降の SQL に
# 文字列として埋め込んでよい。**形は文字列全体で検める** — grep は行ごとに当てるため、
# 改行を含む引数が通り、残りの行が SQL に入る。
valid_date() {
  [[ $1 =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || return 1
  [ "$(sqlite3 :memory: "SELECT date('$1') IS '$1';")" = "1" ]
}

TODAY=$(sqlite3 :memory: "SELECT date('now', 'localtime');")
START="${1:-$(sqlite3 :memory: "SELECT date('now', 'localtime', 'start of month');")}"
END="${2:-$TODAY}"

valid_date "$START" || fail "開始日 '$START' は YYYY-MM-DD の実在する日付ではない。" 2
valid_date "$END" || fail "終了日 '$END' は YYYY-MM-DD の実在する日付ではない。" 2
[ "$(sqlite3 :memory: "SELECT '$START' <= '$END';")" = "1" ] \
  || fail "開始日 $START が終了日 $END より後である。" 2

[ -f "$DB" ] || fail "DB が無い: $DB (MY_TASK_MANAGER_DB で場所を指定できる)"

# **-readonly で開かない。** DB は WAL であり、アプリが閉じた後は -shm が無い。読み取り
# 専用の接続はそれを作れず「unable to open database file」で落ちる。読み書きで開き、
# query_only で書き込みを禁じる (書くのはアプリだけである / AD-4)。
# アプリが書き込んでいる最中なら 2 秒まで待つ。
query() {
  sqlite3 -bail -batch -cmd '.timeout 2000' -cmd 'PRAGMA query_only = ON;' "$@"
}

HAS_TABLE=$(query "$DB" "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'work_segment';") \
  || fail "DB を読めない: $DB"
[ "$HAS_TABLE" = "1" ] \
  || fail "DB に作業区間の表が無い (アプリを新しい版で一度起動すること): $DB"

# 時刻は UTC の ISO 8601 で保存されている。期間の両端と日の境目はローカル時刻の 0 時を
# UTC に直した瞬間であり (julianday(<日付>, 'utc'))、長さは UTC のまま測る — 夏時間の
# 切り替わりで 1 時間ずれない。
# **問い合わせを先に済ませる。** 見出しを先に出すと、問い合わせが失敗したときに見出し
# だけの CSV (区間の無い月と見分けがつかない) が残る。
BODY=$(query -csv "$DB" <<SQL
WITH RECURSIVE
bounds(lo, hi) AS (
  SELECT julianday('$START', 'utc'), julianday(date('$END', '+1 day'), 'utc')
),
segment(kind, task_id, step_id, label, s, e) AS (
  SELECT w.kind, w.task_id, w.step_id, w.label,
         max(julianday(w.started_at), b.lo), min(julianday(w.ended_at), b.hi)
  FROM work_segment AS w, bounds AS b
  WHERE w.ended_at > strftime('%Y-%m-%dT%H:%M:%fZ', b.lo)
    AND w.started_at < strftime('%Y-%m-%dT%H:%M:%fZ', b.hi)
),
piece(kind, task_id, step_id, label, day, s, e, until) AS (
  SELECT kind, task_id, step_id, label, date(s, 'localtime'),
         s, min(e, julianday(date(s, 'localtime', '+1 day'), 'utc')), e
  FROM segment
  UNION ALL
  SELECT kind, task_id, step_id, label, date(e, 'localtime'),
         e, min(until, julianday(date(e, 'localtime', '+1 day'), 'utc')), until
  FROM piece
  WHERE e < until
)
SELECT p.day,
       t.title,
       CASE p.kind WHEN 'step' THEN st.ordinal END,
       CASE p.kind
         WHEN 'step' THEN st.content
         ELSE '待機' || coalesce(': ' || nullif(p.label, ''), '')
       END,
       printf('%.2f', sum(p.e - p.s) * 1440.0)
FROM piece AS p
JOIN task AS t ON t.id = p.task_id
LEFT JOIN step AS st ON st.id = p.step_id
WHERE p.e > p.s
GROUP BY p.day, p.task_id, p.kind, p.step_id, p.label
ORDER BY p.day, t.title, p.task_id, p.kind = 'wait', st.ordinal, p.label;
SQL
) || fail "書き出しの問い合わせに失敗した: $DB"

# BOM と見出しは常に出す (sqlite3 -header は行が 0 件のとき見出しも出さない)。
printf '\357\273\277'
echo "日付,タスク,ステップ番号,ステップ,分"
if [ -n "$BODY" ]; then
  printf '%s\n' "$BODY"
fi
