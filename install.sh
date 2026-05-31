#!/usr/bin/env bash
#
# IntentLoop one-line installer
#
# Recommended usage (no sudo, no compilation in most cases):
#   curl -fsSL https://intentloop.dev/install.sh | sh
#
# For system-wide installation (requires sudo on macOS):
#   curl -fsSL https://intentloop.dev/install.sh | sh -s -- --system
#
# Pin a specific version (recommended if network is slow/unstable):
#   INTENTLOOP_VERSION=0.5.0 curl -fsSL https://intentloop.dev/install.sh | sh
#
# Deployed: 2026-05-31 - v0.7.2 (feat: aggressive noise folding for `il dump chat` — extracts clean User/Agent core turns only)

set -euo pipefail

REPO="EeroEternal/IntentLoop"

# Colors only when stdout is a TTY (prevents \033 codes when piped)
if [ -t 1 ]; then
    GREEN=$'\033[32m'
    YELLOW=$'\033[33m'
    RED=$'\033[31m'
    RESET=$'\033[0m'
else
    GREEN=""
    YELLOW=""
    RED=""
    RESET=""
fi

info()  { printf "%s==> %s%s\n" "$GREEN" "$*" "$RESET"; }
warn()  { printf "%s==> %s%s\n" "$YELLOW" "$*" "$RESET"; }
error() { printf "%serror: %s%s\n" "$RED" "$*" "$RESET" >&2; }

CURL_OPTS="--connect-timeout 10 --max-time 30 -fsSL"

OS="$(uname -s)"
ARCH="$(uname -m)"

USE_SYSTEM=false
for arg in "$@"; do
    [[ "$arg" == "--system" ]] && USE_SYSTEM=true
done

if $USE_SYSTEM; then
    INSTALL_DIR="/usr/local/bin"
    INSTALL_MODE="system"
else
    INSTALL_DIR="${HOME}/.local/bin"
    INSTALL_MODE="user"
fi

VERSION="${INTENTLOOP_VERSION:-}"

fetch_latest_version() {
    local json
    if ! json=$(curl $CURL_OPTS "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null); then
        error "Failed to reach GitHub API (network or proxy issue)."
        echo
        echo "Suggestion: specify a version manually:"
        echo "  INTENTLOOP_VERSION=0.5.0 curl -fsSL https://intentloop.dev/install.sh | sh"
        echo
        exit 1
    fi

    VERSION=$(echo "$json" | grep -o '"tag_name": *"[^"]*"' | head -1 | cut -d'"' -f4 | sed 's/^v//')

    if [[ -z "$VERSION" ]]; then
        error "Could not parse latest version from GitHub."
        echo "Please specify INTENTLOOP_VERSION manually."
        exit 1
    fi
}

download_and_install_prebuilt() {
    local ver="$1"
    local dest_dir="$2"

    local filename
    case "$OS" in
        Darwin)
            filename="il-${ver}-macos-universal.tar.gz"
            ;;
        Linux)
            case "$ARCH" in
                x86_64|amd64) filename="il-${ver}-linux-x86_64.tar.gz" ;;
                aarch64|arm64)  filename="il-${ver}-linux-aarch64.tar.gz" ;;
                *) error "Unsupported architecture on Linux: $ARCH"; return 1 ;;
            esac
            ;;
        *)
            error "Unsupported operating system: $OS"
            return 1
            ;;
    esac

    local url="https://github.com/${REPO}/releases/download/v${ver}/${filename}"
    local tmp
    tmp=$(mktemp -d)

    info "Downloading prebuilt binary (${filename})..."

    if ! curl $CURL_OPTS --progress-bar -o "${tmp}/${filename}" "$url" >/dev/null 2>&1; then
        rm -rf "$tmp"
        return 1
    fi

    tar -xzf "${tmp}/${filename}" -C "$tmp" 2>/dev/null || {
        rm -rf "$tmp"
        return 1
    }

    mkdir -p "$dest_dir"
    cp "${tmp}/il" "$dest_dir/il"
    chmod +x "$dest_dir/il"
    rm -rf "$tmp"

    info "${GREEN}IntentLoop v${ver} installed successfully to ${dest_dir}${RESET}"
    echo "  Binary : ${dest_dir}/il"
    echo
    return 0
}

build_from_source() {
    local dest_dir="$1"
    local mode="$2"

    if ! command -v cargo >/dev/null 2>&1; then
        error "Rust/Cargo is required for fallback build but was not found."
        echo "Please install Rust from https://rustup.rs and try again."
        exit 1
    fi

    warn "Could not download prebuilt binary for v${VERSION}."
    warn "Falling back to building from source."
    echo "This may take several minutes on the first run (especially while updating Rust crates)."
    echo "Future releases will include prebuilt binaries, so you won't need to compile."

    local tmp
    tmp=$(mktemp -d)

    git clone --depth 1 "https://github.com/${REPO}.git" "$tmp/IntentLoop"
    (
        cd "$tmp/IntentLoop"
        cargo build --release
        mkdir -p "$dest_dir"
        cp target/release/il "$dest_dir/il"
        chmod +x "$dest_dir/il"
    )
    rm -rf "$tmp"

    info "${GREEN}IntentLoop installed successfully (built from source).${RESET}"
    echo "  Binary : ${dest_dir}/il"
}

install_macos_pkg() {
    local pkg_name="IntentLoop-${VERSION}.pkg"
    local pkg_url="https://github.com/${REPO}/releases/download/v${VERSION}/${pkg_name}"
    local sha_url="${pkg_url}.sha256"

    local tmp
    tmp=$(mktemp -d)

    info "Downloading official macOS package..."
    if ! curl $CURL_OPTS --progress-bar -o "${tmp}/${pkg_name}" "$pkg_url" >/dev/null 2>&1; then
        error "Failed to download package."
        exit 1
    fi

    info "Verifying checksum..."
    curl $CURL_OPTS -o "${tmp}/${pkg_name}.sha256" "$sha_url" >/dev/null 2>&1
    (cd "$tmp" && shasum -a 256 -c "${pkg_name}.sha256" >/dev/null) || {
        error "Checksum verification failed!"
        exit 1
    }

    info "Installing to /usr/local/bin (you may be asked for your password)..."
    sudo installer -pkg "${tmp}/${pkg_name}" -target /

    rm -rf "$tmp"
    info "${GREEN}IntentLoop v${VERSION} installed successfully to /usr/local/bin.${RESET}"
}

main() {
    echo "IntentLoop installer"
    echo

    if [[ -z "$VERSION" ]]; then
        info "Fetching latest release information..."
        fetch_latest_version
    else
        info "Using requested version: v${VERSION}"
    fi

    if $USE_SYSTEM; then
        info "System-wide installation requested"
        case "$OS" in
            Darwin) install_macos_pkg ;;
            Linux)  build_from_source "$INSTALL_DIR" "system" ;;
            *)      error "System install not supported on $OS"; exit 1 ;;
        esac
    else
        info "Installing to ${INSTALL_DIR} (no sudo)"
        if ! download_and_install_prebuilt "$VERSION" "$INSTALL_DIR"; then
            build_from_source "$INSTALL_DIR" "user"
        fi
    fi

    echo
    echo "If 'il' is not found, add this line to your shell profile:"
    echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
    echo
    echo "Then verify with:"
    echo "  il --version"
}

main "$@"
