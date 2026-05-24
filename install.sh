#!/bin/bash
set -e

echo "=== IntentLoop Linux Installation Script ==="

# 1. Install GCC/build-essential if missing
if ! command -v gcc &> /dev/null; then
    echo "Installing build-essential for C compilation headers..."
    sudo apt-get update
    sudo apt-get install -y build-essential
else
    echo "GCC/build-essential is already installed."
fi

# 2. Check and install Rust/Cargo
if ! command -v cargo &> /dev/null; then
    echo "Rust/Cargo not found. Installing via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    # Load Cargo environment for the current process
    source "$HOME/.cargo/env"
else
    echo "Rust/Cargo is already installed."
fi

# Ensure cargo binary path is in the script's PATH
export PATH="$HOME/.cargo/bin:$PATH"

# Verify rustc is available now
if ! command -v rustc &> /dev/null; then
    echo "Error: rustc is not in the PATH."
    exit 1
fi

# 3. Build the project
echo "Building IntentLoop in release mode..."
cargo build --release

# 4. Install binary to ~/.local/bin/
INSTALL_DIR="$HOME/.local/bin"
mkdir -p "$INSTALL_DIR"

echo "Copying binary to $INSTALL_DIR..."
cp target/release/il "$INSTALL_DIR/il"

echo "=== Installation Completed ==="
echo "The binary has been installed to $INSTALL_DIR/il"
echo ""
echo "Please restart your shell or run 'source ~/.profile' / 'source ~/.bashrc' if $INSTALL_DIR is not in your current PATH."
