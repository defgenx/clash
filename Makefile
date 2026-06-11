.PHONY: build install install-tui install-gui clean

# Install location — override with `make install INSTALL_DIR=~/.local/bin`
INSTALL_DIR ?= /usr/local/bin

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

install-gui: build
	rm -f $(INSTALL_DIR)/clash-gui
	cp target/release/clash-gui $(INSTALL_DIR)/clash-gui
ifeq ($(shell uname),Darwin)
	codesign --force --sign - $(INSTALL_DIR)/clash-gui
endif

clean:
	cargo clean
