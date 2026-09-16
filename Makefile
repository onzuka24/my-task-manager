# 配布はローカルビルド (AD-13)。ビルドから /Applications への配置までを 1 タスクに集約する。
# 署名は v1 では行わない。自動更新機構も持たない (AD-12)。

APP_NAME  := My Task Manager
BUNDLE    := src-tauri/target/release/bundle/macos/$(APP_NAME).app
DEST      := /Applications
INSTALLED := $(DEST)/$(APP_NAME).app

# mise / ~/.tool-versions が RUSTUP_TOOLCHAIN を export していると rustup の優先順位で
# rust-toolchain.toml より強くなり、このプロジェクトの固定が効かない。make の中では外し、
# rust-toolchain.toml を正にする。利用者のグローバル既定は変更しない。
unexport RUSTUP_TOOLCHAIN

.DEFAULT_GOAL := install
.PHONY: install build deps dev test lint measure open clean

## make        — ビルドして /Applications に配置する
install: build
	@# 旧版が常駐していると差し替えに失敗しうる。終了させてから置き換える。
	-@pkill -x "$(APP_NAME)" 2>/dev/null || true
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

test:
	cargo test --manifest-path src-tauri/Cargo.toml
	pnpm check

lint:
	cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings

## 待機時の資源予算を実測する (AD-14)。リリースビルドを起動した状態で実行すること。
measure:
	./scripts/measure-footprint.sh -a "$(APP_NAME)"

open:
	open -a "$(INSTALLED)"

clean:
	rm -rf dist src-tauri/target
