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
# 使い方:
#   scripts/measure-footprint.sh                   # 既定: リリースビルドを 60 秒観測
#   scripts/measure-footprint.sh -d 120            # 観測時間を変える
#   scripts/measure-footprint.sh -a "My Task Manager"
#
# 前提: オーバーレイが非表示の待機状態であること。リリースビルド
# (/Applications/My Task Manager.app) に対して実行すること — `tauri dev` はデバッグ
# シンボルと Vite 開発サーバを抱えており、予算の根拠にならない。
set -euo pipefail

APP_NAME="My Task Manager"
DURATION=60
MEM_BUDGET_MB=100
CPU_BUDGET_PCT=1.0

usage() {
  sed -n '2,25p' "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

while getopts ":a:d:h" opt; do
  case "$opt" in
    a) APP_NAME="$OPTARG" ;;
    d) DURATION="$OPTARG" ;;
    h) usage 0 ;;
    *) usage 1 ;;
  esac
done

command -v footprint >/dev/null 2>&1 || {
  echo "footprint(1) が見つからない。macOS で実行すること。" >&2
  exit 2
}

APP_NAME="$APP_NAME" DURATION="$DURATION" \
MEM_BUDGET_MB="$MEM_BUDGET_MB" CPU_BUDGET_PCT="$CPU_BUDGET_PCT" \
exec /usr/bin/env python3 - <<'PY'
import ctypes
import ctypes.util
import os
import subprocess
import sys
import time

APP_NAME = os.environ["APP_NAME"]
DURATION = float(os.environ["DURATION"])
MEM_BUDGET_MB = float(os.environ["MEM_BUDGET_MB"])
CPU_BUDGET_PCT = float(os.environ["CPU_BUDGET_PCT"])

_libc = ctypes.CDLL(ctypes.util.find_library("System"))
_responsible = _libc.responsibility_get_pid_responsible_for_pid
_responsible.argtypes = [ctypes.c_int]
_responsible.restype = ctypes.c_int


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
    """アプリ本体と、それを responsible pid とする全プロセス (WebKit ヘルパー含む)。"""
    roots = {pid for pid, _t, comm in rows if os.path.basename(comm) == APP_NAME}
    if not roots:
        roots = {pid for pid, _t, comm in rows if APP_NAME in comm}
    if not roots:
        return set()
    group = set(roots)
    for pid, _t, _comm in rows:
        if _responsible(pid) in roots:
            group.add(pid)
    return group


def phys_footprint(pid):
    try:
        out = subprocess.run(
            ["footprint", "-p", str(pid), "--noCategories", "-f", "bytes"],
            capture_output=True,
            text=True,
        ).stdout
    except OSError:
        return 0
    for line in out.splitlines():
        if "phys_footprint:" in line:
            digits = "".join(ch for ch in line.split(":")[-1] if ch.isdigit())
            if digits:
                return int(digits)
    return 0


first = processes()
group = app_group(first)
if not group:
    print(f'"{APP_NAME}" のプロセスが見つからない。', file=sys.stderr)
    print("リリースビルドを起動し、オーバーレイを閉じた待機状態で実行すること。", file=sys.stderr)
    sys.exit(3)

print(f"対象: {APP_NAME}  観測時間: {DURATION:.0f} 秒  (待機状態であること)")
before = {pid: cputime for pid, cputime, _c in first}
started = time.monotonic()
time.sleep(DURATION)
elapsed = time.monotonic() - started

second = processes()
group |= app_group(second)
after = {pid: cputime for pid, cputime, _c in second}
names = {pid: os.path.basename(comm) for pid, _t, comm in second}
names.update({pid: os.path.basename(comm) for pid, _t, comm in first if pid not in names})

print()
print(f"{'PID':>7}  {'メモリ':>12}  {'CPU':>7}  プロセス")
total_bytes = 0
total_cpu = 0.0
for pid in sorted(group):
    footprint_bytes = phys_footprint(pid)
    # 観測窓の両端に存在したプロセスだけ CPU を差分で測れる。片側しかないものは 0 とする。
    cpu_pct = 0.0
    if pid in before and pid in after:
        cpu_pct = max(0.0, (after[pid] - before[pid]) / elapsed * 100.0)
    total_bytes += footprint_bytes
    total_cpu += cpu_pct
    print(
        f"{pid:>7}  {footprint_bytes / 1048576:9.1f} MB  {cpu_pct:6.2f}%  {names.get(pid, '?')}"
    )

total_mb = total_bytes / 1048576
print(f"{'合計':>7}  {total_mb:9.1f} MB  {total_cpu:6.2f}%  ({len(group)} プロセス)")
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
