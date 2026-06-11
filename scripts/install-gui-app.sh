#!/usr/bin/env bash
set -euo pipefail

# Install the clash GUI as a regular desktop application, discoverable like
# any other app (Spotlight/Launchpad on macOS, app launcher on Linux).
#
# Usage: scripts/install-gui-app.sh <path-to-clash-gui-binary> <version>
# Env:   APP_DIR     — macOS bundle destination   (default: /Applications)
#        INSTALL_DIR — CLI symlink / Linux binary (default: /usr/local/bin)

BIN="${1:?usage: install-gui-app.sh <clash-gui binary> <version>}"
VERSION="${2:?usage: install-gui-app.sh <clash-gui binary> <version>}"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ICONS_DIR="$ROOT/gui/src-tauri/icons"

install_macos() {
    local app_dir="${APP_DIR:-/Applications}"
    # Fall back to ~/Applications when /Applications isn't writable.
    if [ ! -w "$app_dir" ]; then
        app_dir="$HOME/Applications"
        mkdir -p "$app_dir"
    fi
    local app="$app_dir/Clash.app"

    rm -rf "$app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    cp "$BIN" "$app/Contents/MacOS/clash-gui"
    cp "$ICONS_DIR/icon.icns" "$app/Contents/Resources/icon.icns"

    cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>Clash</string>
    <key>CFBundleDisplayName</key>
    <string>Clash</string>
    <key>CFBundleExecutable</key>
    <string>clash-gui</string>
    <key>CFBundleIdentifier</key>
    <string>dev.clash.gui</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleVersion</key>
    <string>${VERSION}</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundleIconFile</key>
    <string>icon</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.developer-tools</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
PLIST

    codesign --force --deep --sign - "$app"

    # Keep `clash-gui` on PATH for terminal launching.
    if [ -w "$INSTALL_DIR" ] || [ -w "$(dirname "$INSTALL_DIR")" ]; then
        ln -sf "$app/Contents/MacOS/clash-gui" "$INSTALL_DIR/clash-gui"
    fi

    echo "Installed $app (CLI: $INSTALL_DIR/clash-gui)"
}

install_linux() {
    # XDG locations: system-wide when run as root, per-user otherwise.
    local bin_dir="$INSTALL_DIR" data_dir
    if [ "$(id -u)" = "0" ]; then
        data_dir="/usr/local/share"
    else
        data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
        if [ ! -w "$bin_dir" ]; then
            bin_dir="$HOME/.local/bin"
            mkdir -p "$bin_dir"
        fi
    fi

    install -Dm755 "$BIN" "$bin_dir/clash-gui"
    install -Dm644 "$ICONS_DIR/icon.png" \
        "$data_dir/icons/hicolor/1024x1024/apps/clash.png"

    mkdir -p "$data_dir/applications"
    cat > "$data_dir/applications/clash.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Clash
Comment=Manage Claude Code sessions & agent teams
Exec=$bin_dir/clash-gui
Icon=clash
Terminal=false
Categories=Development;Utility;
StartupWMClass=clash
DESKTOP

    command -v update-desktop-database >/dev/null 2>&1 \
        && update-desktop-database "$data_dir/applications" || true

    echo "Installed $bin_dir/clash-gui (+ launcher entry clash.desktop)"
}

case "$(uname -s)" in
    Darwin) install_macos ;;
    Linux)  install_linux ;;
    *) echo "Unsupported OS: $(uname -s) — copy $BIN onto your PATH manually" >&2; exit 1 ;;
esac
