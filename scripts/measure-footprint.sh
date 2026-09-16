#!/usr/bin/env bash
#
# 待機時の資源使用量を実測する (AD-14)。
#
# 測定対象は「当該アプリの全プロセスの合計」である。WKWebView のヘルパープロセス
# (com.apple.WebKit.WebContent / .GPU / .Networking) を除外した値を根拠にしてはならない。
# ヘルパーは launchd が起こす XPC サービスであり ppid が 1 になるため、プロセスツリーでは
# 辿れない。Activity Monitor と同じ「responsible pid」で束ねる。
#
# メモリは phys_footprint (Activity Monitor の「メモリ」列と同じ値) を使う。RSS は共有
# ページを重複計上するため使わない。
#
# 進行を止めることがこのスクリプトの唯一の役目である。計測に失敗したときは 0 を返して
# 「OK」と表示するのではなく、失敗として終了する。
#
# 使い方:
#   scripts/measure-footprint.sh                   # 既定: リリースビルドを 60 秒観測
#   scripts/measure-footprint.sh -d 120            # 観測時間を変える
#   scripts/measure-footprint.sh -a my-task-manager
#
# -a に渡すのは **プロセス名** であり productName ではない。実プロセス名は Cargo の
# package.name である `my-task-manager` である。
#
# 前提: オーバーレイが非表示の待機状態であること。リリースビルド
# (/Applications/My Task Manager.app) に対して実行すること — `tauri dev` はデバッグ
# シンボルと Vite 開発サーバを抱えており、予算の根拠にならない。
#
##### ヘッダここまで (usage はここまでを表示する) #####
set -euo pipefail

PROC_NAME="my-task-manager"
DURATION=60
MEM_BUDGET_MB=100
CPU_BUDGET_PCT=1.0

usage() {
  # ヘッダの末尾行を探して、そこまでだけを表示する。行番号を直書きすると、ヘッダを
  # 書き足したときに set -euo pipefail や変数代入まで help として出てしまう。
  local last
  last=$(grep -n '^##### ヘッダここまで' "$0" | head -1 | cut -d: -f1)
  sed -n "2,$((last - 1))p" "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

while getopts ":a:d:h" opt; do
  case "$opt" in
    a) PROC_NAME="$OPTARG" ;;
    d) DURATION="$OPTARG" ;;
    h) usage 0 ;;
    *) usage 1 ;;
  esac
done

# 観測時間が正の数でないと elapsed が 0 に近づき、CPU 率が発散する。
if ! printf '%s' "$DURATION" | grep -Eq '^[0-9]+(\.[0-9]+)?$' \
  || [ "$(awk -v d="$DURATION" 'BEGIN { print (d > 0) ? 1 : 0 }')" != "1" ]; then
  echo "-d には正の数を渡すこと (指定: $DURATION)" >&2
  exit 2
fi

command -v footprint >/dev/null 2>&1 || {
  echo "footprint(1) が見つからない。macOS で実行すること。" >&2
  exit 2
}

PROC_NAME="$PROC_NAME" DURATION="$DURATION" \
MEM_BUDGET_MB="$MEM_BUDGET_MB" CPU_BUDGET_PCT="$CPU_BUDGET_PCT" \
exec /usr/bin/env python3 - <<'PY'
import ctypes
import ctypes.util
import os
import subprocess
import sys
import time

PROC_NAME = os.environ["PROC_NAME"]
DURATION = float(os.environ["DURATION"])
MEM_BUDGET_MB = float(os.environ["MEM_BUDGET_MB"])
CPU_BUDGET_PCT = float(os.environ["CPU_BUDGET_PCT"])

_libc = ctypes.CDLL(ctypes.util.find_library("System"))
try:
    # Activity Monitor と同じ束ね方をする唯一の手段。解決できないまま進むと
    # WebKit ヘルパーが丸ごと落ち、過少計上した値で「OK」と言ってしまう。
    _responsible = _libc.responsibility_get_pid_responsible_for_pid
except AttributeError:
    sys.exit(
        "responsibility_get_pid_responsible_for_pid が libSystem に無い。"
        "\nWebKit ヘルパーを束ねられないため計測を中止する (AD-14)。"
    )
_responsible.argtypes = [ctypes.c_int]
_responsible.restype = ctypes.c_int

# 束ねられていなければならない WKWebView のヘルパー。1 つでも欠ければ過少計上である。
REQUIRED_HELPERS = ("com.apple.WebKit.WebContent", "com.apple.WebKit.Networking")


class MeasurementFailed(Exception):
    """計測そのものが失敗した。過少計上して「OK」と言うより、止まるほうが正しい。"""


def processes():
    """(pid, cputime_seconds, command) の一覧。"""
    out = subprocess.run(
        ["ps", "-Ao", "pid=,time=,comm="], capture_output=True, text=True, check=True
    ).stdout
    rows = []
    for line in out.splitlines():
        parts = line.split(None, 2)
        if len(parts) < 3:
            continue
        pid, cputime, comm = parts
        rows.append((int(pid), parse_cputime(cputime), comm))
    return rows


def parse_cputime(value):
    """ps の TIME 表記 (`[[DD-]HH:]MM:SS.ss`) を秒に直す。"""
    days = 0
    if "-" in value:
        day_part, value = value.split("-", 1)
        days = int(day_part)
    fields = [float(f) for f in value.split(":")]
    seconds = 0.0
    for field in fields:
        seconds = seconds * 60 + field
    return seconds + days * 86400


def app_group(rows):
    """アプリ本体と、それを responsible pid とする全プロセス (WebKit ヘルパー含む)。

    照合はプロセス名の完全一致のみとする。部分一致の fallback は無関係なプロセスを
    拾って合計を膨らませうるため持たない — 見つからないなら見つからないと言う。
    """
    roots = {pid for pid, _t, comm in rows if os.path.basename(comm) == PROC_NAME}
    if not roots:
        return set()
    group = set(roots)
    for pid, _t, _comm in rows:
        if _responsible(pid) in roots:
            group.add(pid)
    return group


def phys_footprint(pid):
    """1 プロセスの phys_footprint (バイト)。読めなければ例外を投げる。"""
    try:
        completed = subprocess.run(
            ["footprint", "-p", str(pid), "--noCategories", "-f", "bytes"],
            capture_output=True,
            text=True,
        )
    except OSError as error:
        raise MeasurementFailed(f"pid {pid}: footprint を起動できない ({error})") from error

    if completed.returncode != 0:
        raise MeasurementFailed(
            f"pid {pid}: footprint が失敗した (exit {completed.returncode}): "
            f"{completed.stderr.strip()}"
        )

    for line in completed.stdout.splitlines():
        if "phys_footprint:" in line:
            digits = "".join(ch for ch in line.split(":")[-1] if ch.isdigit())
            if digits:
                return int(digits)
    raise MeasurementFailed(f"pid {pid}: footprint の出力に phys_footprint が無い")


first = processes()
group = app_group(first)
if not group:
    print(f'"{PROC_NAME}" のプロセスが見つからない。', file=sys.stderr)
    print("リリースビルドを起動し、オーバーレイを閉じた待機状態で実行すること。", file=sys.stderr)
    print("-a に渡すのは productName ではなくプロセス名である。", file=sys.stderr)
    sys.exit(3)

print(f"対象: {PROC_NAME}  観測時間: {DURATION:.0f} 秒  (待機状態であること)")
before = {pid: cputime for pid, cputime, _c in first}
started = time.monotonic()
time.sleep(DURATION)
elapsed = time.monotonic() - started

second = processes()
group |= app_group(second)
after = {pid: cputime for pid, cputime, _c in second}
alive = {pid for pid, _t, _c in second}
names = {pid: os.path.basename(comm) for pid, _t, comm in second}
names.update({pid: os.path.basename(comm) for pid, _t, comm in first if pid not in names})

print()
print(f"{'PID':>7}  {'メモリ':>12}  {'CPU':>7}  プロセス")
total_bytes = 0
total_cpu = 0.0
measured = 0
try:
    for pid in sorted(group):
        # 観測窓の両端に居たプロセスだけ CPU を差分で測れる。片側しか居ないものを
        # 0% として数えると、ヘルパーの再起動が「CPU を使っていない」ことになり、
        # 超過しているのに OK と表示しうる。計測の失敗として扱う。
        if pid not in before:
            raise MeasurementFailed(
                f"pid {pid} ({names.get(pid, '?')}) は観測窓の途中で現れた。"
                "待機状態が保たれていない。"
            )
        if pid not in after:
            raise MeasurementFailed(
                f"pid {pid} ({names.get(pid, '?')}) は観測窓の途中で終了した。"
                "待機状態が保たれていない。"
            )
        footprint_bytes = phys_footprint(pid)
        cpu_pct = max(0.0, (after[pid] - before[pid]) / elapsed * 100.0)
        total_bytes += footprint_bytes
        total_cpu += cpu_pct
        measured += 1
        print(
            f"{pid:>7}  {footprint_bytes / 1048576:9.1f} MB  {cpu_pct:6.2f}%  {names.get(pid, '?')}"
        )
except MeasurementFailed as error:
    print()
    print(f"計測に失敗した: {error}", file=sys.stderr)
    print("AD-14: 実測できない値を根拠にして進めてはならない。", file=sys.stderr)
    sys.exit(4)

if measured == 0:
    print("計測できたプロセスが 1 つも無い。", file=sys.stderr)
    sys.exit(4)

# 束ね方が壊れていないことを確かめてから合否を言う。ヘルパーが静かに落ちた合計は
# 予算の根拠にならない (AD-14)。
grouped_names = [names.get(pid, "") for pid in sorted(group)]
missing = [
    helper
    for helper in REQUIRED_HELPERS
    if not any(name.startswith(helper) for name in grouped_names)
]
if missing:
    print()
    print(f"WKWebView のヘルパーを束ねられていない: {', '.join(missing)}", file=sys.stderr)
    print("ヘルパーを除外した値を根拠にしてはならない (AD-14)。", file=sys.stderr)
    sys.exit(4)

total_mb = total_bytes / 1048576
print(f"{'合計':>7}  {total_mb:9.1f} MB  {total_cpu:6.2f}%  ({measured} プロセス)")
print()

memory_ok = total_mb < MEM_BUDGET_MB
cpu_ok = total_cpu < CPU_BUDGET_PCT
print(f"メモリ: {total_mb:.1f} MB / 上限 {MEM_BUDGET_MB:.0f} MB  ... {'OK' if memory_ok else '超過'}")
print(f"CPU   : {total_cpu:.2f}% / 上限 {CPU_BUDGET_PCT:.0f}%   ... {'OK' if cpu_ok else '超過'}")

if not (memory_ok and cpu_ok):
    print()
    print("AD-14: 予算を超過したまま進めることは禁じられている。", file=sys.stderr)
    print("対処は (a) 実装の是正 (b) 上限の改訂 のいずれかで、いずれも記録を残すこと。", file=sys.stderr)
    sys.exit(1)
PY
