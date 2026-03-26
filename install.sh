#!/usr/bin/env bash
set -euo pipefail

# clash installer — downloads the latest release binary for your platform.
# Usage: curl -fsSL https://raw.githubusercontent.com/defgenx/clash/main/install.sh | bash

REPO="defgenx/clash"
BINARY="clash"
INSTALL_DIR="${CLASH_INSTALL_DIR:-/usr/local/bin}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

info()  { printf "${CYAN}[info]${NC}  %s\n" "$*"; }
ok()    { printf "${GREEN}[ok]${NC}    %s\n" "$*"; }
warn()  { printf "${YELLOW}[warn]${NC}  %s\n" "$*"; }
error() { printf "${RED}[error]${NC} %s\n" "$*" >&2; exit 1; }

# Detect OS
detect_os() {
    case "$(uname -s)" in
        Linux*)  echo "linux" ;;
        Darwin*) echo "macos" ;;
        MINGW*|MSYS*|CYGWIN*) echo "windows" ;;
        *) error "Unsupported operating system: $(uname -s)" ;;
    esac
}

# Detect architecture
detect_arch() {
    case "$(uname -m)" in
        x86_64|amd64)  echo "x86_64" ;;
        aarch64|arm64) echo "aarch64" ;;
        *) error "Unsupported architecture: $(uname -m)" ;;
    esac
}

# Map to Rust target triple
target_triple() {
    local os="$1"
    local arch="$2"

    case "${os}-${arch}" in
        linux-x86_64)   echo "x86_64-unknown-linux-gnu" ;;
        linux-aarch64)  echo "aarch64-unknown-linux-gnu" ;;
        macos-x86_64)   echo "x86_64-apple-darwin" ;;
        macos-aarch64)  echo "aarch64-apple-darwin" ;;
        windows-x86_64) echo "x86_64-pc-windows-msvc" ;;
        *) error "No prebuilt binary for ${os}/${arch}" ;;
    esac
}

# Get latest release tag from GitHub
latest_version() {
    local url="https://api.github.com/repos/${REPO}/releases/latest"
    if command -v curl &>/dev/null; then
        curl -fsSL "$url" | grep '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/'
    elif command -v wget &>/dev/null; then
        wget -qO- "$url" | grep '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/'
    else
        error "Neither curl nor wget found. Please install one of them."
    fi
}

# Download file
download() {
    local url="$1"
    local dest="$2"
    if command -v curl &>/dev/null; then
        curl -fsSL -o "$dest" "$url"
    elif command -v wget &>/dev/null; then
        wget -qO "$dest" "$url"
    fi
}

main() {
    info "Detecting platform..."
    local os arch target
    os="$(detect_os)"
    arch="$(detect_arch)"
    target="$(target_triple "$os" "$arch")"
    info "Platform: ${os}/${arch} (${target})"

    if [ "$os" = "windows" ]; then
        warn "Windows support requires building from source."
        warn "Install Rust from https://rustup.rs then run:"
        warn "  cargo install --git https://github.com/${REPO}.git"
        exit 0
    fi

    info "Fetching latest version..."
    local version
    version="$(latest_version)"
    if [ -z "$version" ]; then
        error "Could not determine latest version. Check https://github.com/${REPO}/releases"
    fi
    info "Latest version: ${version}"

    local artifact="clash-${target}.tar.gz"
    local url="https://github.com/${REPO}/releases/download/${version}/${artifact}"

    info "Downloading ${artifact}..."
    local tmpdir
    tmpdir="$(mktemp -d)"
    trap 'rm -rf "$tmpdir"' EXIT

    download "$url" "${tmpdir}/${artifact}"

    info "Extracting..."
    tar xzf "${tmpdir}/${artifact}" -C "$tmpdir"

    info "Installing to ${INSTALL_DIR}/${BINARY}..."
    # Remove existing binary first so the new file gets a fresh inode.
    # On macOS, overwriting in-place invalidates the code signature and
    # the kernel kills the process with SIGKILL.
    if [ -w "$INSTALL_DIR" ]; then
        rm -f "${INSTALL_DIR}/${BINARY}"
        mv "${tmpdir}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
    else
        warn "Need sudo to install to ${INSTALL_DIR}"
        sudo rm -f "${INSTALL_DIR}/${BINARY}"
        sudo mv "${tmpdir}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
    fi
    chmod +x "${INSTALL_DIR}/${BINARY}"
    # Ad-hoc codesign on macOS
    if [ "$(uname)" = "Darwin" ]; then
        codesign --force --sign - "${INSTALL_DIR}/${BINARY}" 2>/dev/null || true
    fi

    ok "clash ${version} installed to ${INSTALL_DIR}/${BINARY}"
    echo ""
    info "Run 'clash' to start, or 'clash --help' for options."
}

main "$@"
