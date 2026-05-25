#!/bin/bash
set -euo pipefail

#
# Reliable macOS .pkg builder for IntentLoop
#
# This script produces a simple flat package using only pkgbuild.
# Flat packages are significantly more reliable on modern macOS (Ventura/Sonoma/Sequoia+)
# than distribution packages built with productbuild + Distribution.xml.
#
# The resulting .pkg can be installed with:
#   sudo installer -pkg IntentLoop-*.pkg -target /
# or by double-clicking in Finder.
#

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

# Read version from Cargo.toml
VERSION=$(grep '^version' "$PROJECT_DIR/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')

IDENTIFIER="com.intentloop.pkg"
INSTALL_LOCATION="/usr/local/bin"
PKG_BUILD_DIR="$PROJECT_DIR/target/pkg-build"
PKG_OUTPUT_DIR="$PROJECT_DIR/target/pkg"
PKG_NAME="IntentLoop-${VERSION}.pkg"

echo "==> Building IntentLoop v${VERSION} installer package (flat package)"

# -------------------------------------------------------
# 1. Build release binaries for both architectures
# -------------------------------------------------------
echo "==> Building aarch64-apple-darwin binary..."
cargo build --release --target aarch64-apple-darwin --manifest-path "$PROJECT_DIR/Cargo.toml"

echo "==> Building x86_64-apple-darwin binary..."
cargo build --release --target x86_64-apple-darwin --manifest-path "$PROJECT_DIR/Cargo.toml"

# -------------------------------------------------------
# 2. Create universal binary (il) with lipo
# -------------------------------------------------------
echo "==> Creating universal binary..."
mkdir -p "$PKG_BUILD_DIR/root"

lipo -create \
    "$PROJECT_DIR/target/aarch64-apple-darwin/release/il" \
    "$PROJECT_DIR/target/x86_64-apple-darwin/release/il" \
    -output "$PKG_BUILD_DIR/root/il"

chmod +x "$PKG_BUILD_DIR/root/il"
echo "    il: $(lipo -archs "$PKG_BUILD_DIR/root/il")"

# -------------------------------------------------------
# 3. Build a simple flat package directly with pkgbuild
#    (This is the reliable modern way — no productbuild / Distribution.xml)
# -------------------------------------------------------
echo "==> Building flat package..."
mkdir -p "$PKG_OUTPUT_DIR"

pkgbuild \
    --root "$PKG_BUILD_DIR/root" \
    --install-location "$INSTALL_LOCATION" \
    --identifier "$IDENTIFIER" \
    --version "$VERSION" \
    --ownership recommended \
    "$PKG_OUTPUT_DIR/$PKG_NAME"

# -------------------------------------------------------
# 4. Generate SHA256 checksum
# -------------------------------------------------------
echo "==> Generating checksum..."
shasum -a 256 "$PKG_OUTPUT_DIR/$PKG_NAME" | awk '{print $1}' > "$PKG_OUTPUT_DIR/${PKG_NAME}.sha256"

echo ""
echo "==> Done!"
echo "    Package: $PKG_OUTPUT_DIR/$PKG_NAME"
echo "    SHA256:  $(cat "$PKG_OUTPUT_DIR/${PKG_NAME}.sha256")"
echo ""
echo "    Recommended install (macOS):"
echo "      curl -fsSL https://intentloop.dev/install.sh | sh"
echo ""
echo "    Manual install:"
echo "      sudo installer -pkg $PKG_OUTPUT_DIR/$PKG_NAME -target /"
echo "      # or double-click the .pkg in Finder"
echo ""
echo "    Note: This is a flat package (more reliable on modern macOS than distribution packages)."