#!/usr/bin/env bash
set -euo pipefail

REPO="IWhitebird/tpdf"
INSTALL_DIR="${TPDF_INSTALL_DIR:-$HOME/.local/bin}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
BOLD='\033[1m'
RESET='\033[0m'

info()  { printf "${CYAN}%s${RESET}\n" "$*"; }
ok()    { printf "${GREEN}%s${RESET}\n" "$*"; }
fail()  { printf "${RED}error: %s${RESET}\n" "$*" >&2; exit 1; }

# --- Detect platform ---
detect_platform() {
    local os arch

    case "$(uname -s)" in
        Linux*)  os="linux" ;;
        Darwin*) os="macos" ;;
        *)       fail "Unsupported OS: $(uname -s)" ;;
    esac

    case "$(uname -m)" in
        x86_64|amd64)  arch="x86_64" ;;
        aarch64|arm64) arch="aarch64" ;;
        *)             fail "Unsupported architecture: $(uname -m)" ;;
    esac

    echo "tpdf-${os}-${arch}"
}

# --- Find latest release tag ---
get_latest_version() {
    local url="https://api.github.com/repos/${REPO}/releases/latest"
    if command -v curl &>/dev/null; then
        curl -fsSL "$url" | grep '"tag_name"' | head -1 | cut -d'"' -f4
    elif command -v wget &>/dev/null; then
        wget -qO- "$url" | grep '"tag_name"' | head -1 | cut -d'"' -f4
    else
        fail "curl or wget is required"
    fi
}

# --- Download ---
download() {
    local url="$1" dest="$2"
    if command -v curl &>/dev/null; then
        curl -fsSL "$url" -o "$dest"
    else
        wget -qO "$dest" "$url"
    fi
}

# --- Main ---
main() {
    printf "\n${BOLD}  tpdf installer${RESET}\n\n"

    local platform version archive_url tmp_dir

    platform="$(detect_platform)"
    info "Detected platform: ${platform}"

    info "Fetching latest release..."
    version="$(get_latest_version)"
    [ -z "$version" ] && fail "Could not determine latest version"
    info "Latest version: ${version}"

    archive_url="https://github.com/${REPO}/releases/download/${version}/${platform}.tar.gz"

    tmp_dir="$(mktemp -d)"
    trap 'rm -rf "$tmp_dir"' EXIT

    info "Downloading ${archive_url}..."
    download "$archive_url" "$tmp_dir/tpdf.tar.gz"

    info "Extracting..."
    tar xzf "$tmp_dir/tpdf.tar.gz" -C "$tmp_dir"

    mkdir -p "$INSTALL_DIR"
    mv "$tmp_dir/tpdf" "$INSTALL_DIR/tpdf"
    chmod +x "$INSTALL_DIR/tpdf"

    ok "Installed tpdf to ${INSTALL_DIR}/tpdf"

    # Check if INSTALL_DIR is in PATH
    if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
        printf "\n${BOLD}Add tpdf to your PATH:${RESET}\n"

        local shell_name
        shell_name="$(basename "${SHELL:-/bin/bash}")"

        case "$shell_name" in
            zsh)  echo "  echo 'export PATH=\"${INSTALL_DIR}:\$PATH\"' >> ~/.zshrc && source ~/.zshrc" ;;
            fish) echo "  fish_add_path ${INSTALL_DIR}" ;;
            *)    echo "  echo 'export PATH=\"${INSTALL_DIR}:\$PATH\"' >> ~/.bashrc && source ~/.bashrc" ;;
        esac
        echo ""
    fi

    ok "Run 'tpdf <file.pdf>' to get started!"
    echo ""
}

main
