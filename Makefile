.PHONY: build install install-tui install-gui clean

# Install location — override with `make install INSTALL_DIR=~/.local/bin`
INSTALL_DIR ?= /usr/local/bin
# macOS GUI bundle location — override with `make install-gui APP_DIR=~/Applications`
APP_DIR ?= /Applications

VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

# Both binaries (clash + clash-gui) build by default: plain `cargo build --release`
# produces both (gui/src-tauri is a default workspace member).
build:
	cargo build --release
ifeq ($(shell uname),Darwin)
	codesign --force --sign - target/release/clash
	codesign --force --sign - target/release/clash-gui
endif

# First install: both the TUI (clash) and the GUI (clash-gui)
install: install-tui install-gui

install-tui: build
	rm -f $(INSTALL_DIR)/clash
	cp target/release/clash $(INSTALL_DIR)/clash
ifeq ($(shell uname),Darwin)
	codesign --force --sign - $(INSTALL_DIR)/clash
endif

# GUI installs as a regular desktop app (Spotlight/Launchpad on macOS,
# app launcher on Linux) plus a `clash-gui` CLI entry point.
install-gui: build
	INSTALL_DIR=$(INSTALL_DIR) APP_DIR=$(APP_DIR) \
		scripts/install-gui-app.sh target/release/clash-gui $(VERSION)

clean:
	cargo clean
