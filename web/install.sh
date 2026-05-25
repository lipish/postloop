#!/usr/bin/env bash
#
# IntentLoop one-line installer
# Usage (recommended):
#   curl -fsSL https://intentloop.dev/install.sh | sh
#
# Advanced:
#   INTENTLOOP_VERSION=0.5.0 sh -s --   # install a specific version
#
# This script prefers the official macOS .pkg when possible.
# On other platforms it falls back to building from source (requires Rust).

set -euo pipefail

REPO="EeroEternal/IntentLoop"
GREEN="\033[32m"
YELLOW="\033[33m"
RED="\033[31m"
RESET="\033[0m"

info()  { printf "${GREEN}==> %s${RESET}\n" "$*"; }
warn()  { printf "${YELLOW}==> %s${RESET}\n" "$*"; }
error() { printf "${RED}error: %s${RESET}\n" "$*" >&2; }

# Detect platform
OS="$(uname -s)"
ARCH="$(uname -m)"

# Allow overriding version
VERSION="${INTENTLOOP_VERSION:-}"

fetch_latest_version() {
    local api_url="https://api.github.com/repos/${REPO}/releases/latest"
    local json
    json=$(curl -fsSL "$api_url" 2>/dev/null || true)

    if [[ -z "$json" ]]; then
        error "Failed to reach GitHub API. Check your network or try again later."
        exit 1
    fi

    # Extract tag_name, strip leading 'v'
    VERSION=$(echo "$json" | grep -o '"tag_name": *"[^"]*"' | head -1 | cut -d'"' -f4 | sed 's/^v//')

    if [[ -z "$VERSION" ]]; then
        error "Could not determine latest version from GitHub."
        exit 1
    fi
}

install_macos_pkg() {
    local pkg_name="IntentLoop-${VERSION}.pkg"
    local pkg_url="https://github.com/${REPO}/releases/download/v${VERSION}/${pkg_name}"
    local sha_url="${pkg_url}.sha256"

    local tmpdir
    tmpdir=$(mktemp -d)
    trap 'rm -rf "$tmpdir"' EXIT

    info "Downloading IntentLoop v${VERSION} for macOS..."
    if ! curl -fL --progress-bar -o "${tmpdir}/${pkg_name}" "$pkg_url"; then
        error "Download failed: ${pkg_url}"
        exit 1
    fi

    info "Downloading checksum..."
    if ! curl -fsSL -o "${tmpdir}/${pkg_name}.sha256" "$sha_url"; then
        error "Failed to download checksum file."
        exit 1
    fi

    info "Verifying checksum..."
    (
        cd "$tmpdir"
        if ! shasum -a 256 -c "${pkg_name}.sha256" >/dev/null 2>&1; then
            error "Checksum verification FAILED!"
            cat "${pkg_name}.sha256"
            shasum -a 256 "${pkg_name}"
            exit 1
        fi
    )
    info "Checksum verified."

    echo
    info "Installing to /usr/local/bin (sudo may ask for your password)..."
    sudo installer -pkg "${tmpdir}/${pkg_name}" -target /

    echo
    info "${GREEN}IntentLoop v${VERSION} installed successfully!${RESET}"
    echo "  Location : /usr/local/bin/il"
    echo "  Version  : $(/usr/local/bin/il --version 2>/dev/null || echo 'unknown')"
    echo
    echo "Try it now:"
    echo "  il --version"
    echo "  il run echo 'hello from IntentLoop'"
    echo
}

install_from_source() {
    warn "No prebuilt package available for your platform."
    info "Falling back to building from source (requires Rust + Cargo)."

    if ! command -v cargo >/dev/null 2>&1; then
        error "Rust/Cargo not found."
        echo "Please install Rust first: https://rustup.rs"
        echo "Then run:"
        echo "  git clone https://github.com/${REPO}.git"
        echo "  cd IntentLoop && cargo build --release"
        echo "  # then copy target/release/il into your \$PATH"
        exit 1
    fi

    local tmpdir
    tmpdir=$(mktemp -d)
    trap 'rm -rf "$tmpdir"' EXIT

    info "Cloning repository..."
    git clone --depth 1 "https://github.com/${REPO}.git" "$tmpdir/IntentLoop"

    cd "$tmpdir/IntentLoop"
    info "Building in release mode (this may take a minute)..."
    cargo build --release

    local install_dir="${HOME}/.local/bin"
    mkdir -p "$install_dir"

    info "Installing binary to ${install_dir}/il"
    cp target/release/il "$install_dir/il"
    chmod +x "$install_dir/il"

    echo
    info "Build complete!"
    echo "  Binary installed to: ${install_dir}/il"
    echo
    echo "Make sure ${install_dir} is in your PATH."
    echo "You may need to run:"
    echo "  export PATH=\"${install_dir}:\$PATH\""
    echo "  # or add it to your shell profile (.zshrc, .bashrc, etc.)"
    echo
    echo "Then verify with:"
    echo "  il --version"
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

    case "$OS" in
        Darwin)
            install_macos_pkg
            ;;
        Linux)
            install_from_source
            ;;
        *)
            error "Unsupported operating system: $OS"
            echo "Please install from source following the instructions at:"
            echo "  https://github.com/${REPO}#install"
            exit 1
            ;;
    esac
}

main "$@"
