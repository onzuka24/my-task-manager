# 配布はローカルビルド (AD-13)。ビルドから /Applications への配置までを 1 タスクに集約する。
# 署名は v1 では行わない。自動更新機構も持たない (AD-12)。

APP_NAME     := My Task Manager
# 実プロセス名は Cargo の package.name であり productName ではない。`pkill -x` /
# `pgrep -x` は完全一致であるため、ここを取り違えると一致せず、稼働中のアプリに対して
# rm -rf / cp -R が走る。
PROC_NAME    := my-task-manager
BUNDLE       := src-tauri/target/release/bundle/macos/$(APP_NAME).app
DEST         := /Applications
INSTALLED    := $(DEST)/$(APP_NAME).app
# tauri-plugin-autostart (auto-launch) が書く LaunchAgent の plist。
# app_name に合わせて my-task-manager.plist となる。
LAUNCH_AGENT := $(HOME)/Library/LaunchAgents/$(PROC_NAME).plist
BUNDLE_ID    := dev.onzuka.mytaskmanager
# 自動起動を一度登録したことを示す印 (adapters/autostart)。これを残したまま
# 再インストールすると、次の起動で自動起動が登録されない。
# 実体は APP_DATA_DIR の下にあり、uninstall はディレクトリごと消す。
AUTOSTART_MARKER := $(HOME)/Library/Application Support/$(BUNDLE_ID)/autostart-registered
# ローカルに残る痕跡。uninstall はこれらも消す (AD-12: ログはローカルファイルのみ)。
APP_DATA_DIR := $(HOME)/Library/Application Support/$(BUNDLE_ID)
LOG_DIR      := $(HOME)/Library/Logs/$(BUNDLE_ID)

# mise / ~/.tool-versions が RUSTUP_TOOLCHAIN を export していると rustup の優先順位で
# rust-toolchain.toml より強くなり、このプロジェクトの固定が効かない。make の中では外し、
# rust-toolchain.toml を正にする。利用者のグローバル既定は変更しない。
unexport RUSTUP_TOOLCHAIN

.DEFAULT_GOAL := install
.PHONY: install build deps dev test lint measure open stop uninstall clean

## make        — ビルドして /Applications に配置する
install: build
	@# 置き換える中身があることを先に確かめる。パスが食い違ったまま先に消すと、
	@# 常駐を消しただけで何も入らない状態になる。
	@test -d "$(BUNDLE)" || { echo "ビルド成果物が無い: $(BUNDLE)" >&2; exit 1; }
	@# 旧版が常駐していると差し替えに失敗しうる。確実に終了させてから置き換える。
	@$(MAKE) --no-print-directory stop
	rm -rf "$(INSTALLED)"
	cp -R "$(BUNDLE)" "$(DEST)/"
	@echo "installed: $(INSTALLED)"
	@echo 'open -a "$(APP_NAME)" で起動できる。'

build: deps
	pnpm tauri build

deps:
	pnpm install --frozen-lockfile

dev:
	pnpm tauri dev

test: deps
	cargo test --manifest-path src-tauri/Cargo.toml
	pnpm check
	pnpm test

lint: deps
	cargo fmt --manifest-path src-tauri/Cargo.toml --check
	cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings

## 待機時の資源予算を実測する (AD-14)。リリースビルドを起動した状態で実行すること。
measure:
	./scripts/measure-footprint.sh -a "$(PROC_NAME)"

open:
	open -a "$(INSTALLED)"

## 常駐プロセスを終了させる。終了を確認できなければ失敗する — 稼働中のまま
## rm -rf / cp -R を走らせないため (静かに失敗を飲まない)。
stop:
	@if pgrep -x "$(PROC_NAME)" >/dev/null 2>&1; then \
	  echo "stopping $(PROC_NAME) ..."; \
	  pkill -x "$(PROC_NAME)" || true; \
	  for i in 1 2 3 4 5 6 7 8 9 10; do \
	    pgrep -x "$(PROC_NAME)" >/dev/null 2>&1 || break; \
	    sleep 0.5; \
	  done; \
	  if pgrep -x "$(PROC_NAME)" >/dev/null 2>&1; then \
	    echo "$(PROC_NAME) が終了しない。手動で終了させること。" >&2; \
	    exit 1; \
	  fi; \
	  echo "stopped."; \
	else \
	  echo "$(PROC_NAME) は稼働していない。"; \
	fi

## 常駐を止め、痕跡を残さずに消す。再ログインしても復活しない。
## auto-launch は launchctl を呼ばないため、plist の削除だけでは現在走っている
## プロセスは止まらない。終了・plist 削除・アプリデータ削除・ログ削除・.app 削除
## をすべて揃える。
uninstall:
	@$(MAKE) --no-print-directory stop
	@# 既に読み込まれている LaunchAgent があれば外す (無ければ何もしない)。
	-@launchctl bootout "gui/$$(id -u)/$(PROC_NAME)" 2>/dev/null || true
	rm -f "$(LAUNCH_AGENT)"
	rm -rf "$(APP_DATA_DIR)"
	rm -rf "$(LOG_DIR)"
	rm -rf "$(INSTALLED)"
	@echo "uninstalled: $(INSTALLED)"
	@echo "removed: $(LAUNCH_AGENT)"
	@echo "removed: $(APP_DATA_DIR) (自動起動の印を含む)"
	@echo "removed: $(LOG_DIR)"

clean:
	rm -rf dist src-tauri/target
