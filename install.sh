#!/usr/bin/env bash
#
# IntentLoop one-line installer
#
# Recommended usage (no sudo, no compilation in most cases):
#   curl -fsSL https://intentloop.dev/install.sh | sh
#
# System-wide installation (requires sudo on macOS):
#   curl -fsSL https://intentloop.dev/install.sh | sh -s -- --system
#
# Pin a specific version:
#   INTENTLOOP_VERSION=0.5.0 curl -fsSL https://intentloop.dev/install.sh | sh

set -euo pipefail

REPO="EeroEternal/IntentLoop"

# Only use colors when stdout is a terminal.
# This prevents ugly \033[32m codes when the script is piped (curl | sh).
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

OS="$(uname -s)"
ARCH="$(uname -m)"

# Parse arguments
USE_SYSTEM=false
for arg in "$@"; do
    case "$arg" in
        --system) USE_SYSTEM=true ;;
    esac
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
    json=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null || true)
    VERSION=$(echo "$json" | grep -o '"tag_name": *"[^"]*"' | head -1 | cut -d'"' -f4 | sed 's/^v//')
    [[ -z "$VERSION" ]] && { error "Could not determine latest version from GitHub."; exit 1; }
}

# Download pre-built binary from GitHub Releases (preferred, no compilation)
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
    if ! curl -fL --progress-bar -o "${tmp}/${filename}" "$url"; then
        warn "Prebuilt binary not available for this platform/version yet."
        rm -rf "$tmp"
        return 1
    fi

    tar -xzf "${tmp}/${filename}" -C "$tmp"
    mkdir -p "$dest_dir"
    cp "${tmp}/il" "$dest_dir/il"
    chmod +x "$dest_dir/il"
    rm -rf "$tmp"

    info "${GREEN}IntentLoop v${ver} installed successfully to ${dest_dir}${RESET}"
    echo "  Binary : ${dest_dir}/il"
    echo
    return 0
}

# Fallback: build from source (only when no prebuilt exists)
build_from_source() {
    local dest_dir="$1"
    local mode="$2"

    if ! command -v cargo >/dev/null 2>&1; then
        error "Rust/Cargo is required for fallback build but was not found."
        echo "Please install Rust from https://rustup.rs and try again."
        exit 1
    fi

    warn "No prebuilt binary found for your platform. Falling back to building from source."
    warn "This may take 1-2 minutes on the first run."

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

# macOS system-wide install via official .pkg (requires sudo)
install_macos_pkg() {
    local pkg_name="IntentLoop-${VERSION}.pkg"
    local pkg_url="https://github.com/${REPO}/releases/download/v${VERSION}/${pkg_name}"
    local sha_url="${pkg_url}.sha256"

    local tmp
    tmp=$(mktemp -d)

    info "Downloading official macOS package..."
    curl -fL --progress-bar -o "${tmp}/${pkg_name}" "$pkg_url" || {
        error "Failed to download package."
        exit 1
    }

    info "Verifying checksum..."
    curl -fsSL -o "${tmp}/${pkg_name}.sha256" "$sha_url"
    (cd "$tmp" && shasum -a 256 -c "${pkg_name}.sha256" >/dev/null) || {
        error "Checksum verification failed!"
        exit 1
    }

    info "Installing to /usr/local/bin (sudo password may be required)..."
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
    fi

    if $USE_SYSTEM; then
        info "System-wide installation requested"
        case "$OS" in
            Darwin) install_macos_pkg ;;
            Linux)  build_from_source "$INSTALL_DIR" "system" ;;  # Linux system install usually needs sudo during build or manual copy
            *)      error "System install not supported on $OS"; exit 1 ;;
        esac
    else
        # Default: user-level install, prefer prebuilt binary
        info "Installing to ${INSTALL_DIR} (no sudo)"
        if ! download_and_install_prebuilt "$VERSION" "$INSTALL_DIR"; then
            build_from_source "$INSTALL_DIR" "user"
        fi
    fi

    echo
    echo "Run the following if the binary is not in your PATH:"
    echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
    echo
    echo "Then verify installation with:"
    echo "  il --version"
}

main "$@"
