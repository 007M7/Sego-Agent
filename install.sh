#!/bin/bash
# Sego Agent — Linux/macOS one-liner installer
# Run: curl -fsSL https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.sh | bash

set -e
REPO="007M7/Sego-Agent"
BINARY="sego"
INSTALL_DIR="$HOME/.local/bin"
OS=$(uname -s | tr '[:upper:]' '[:lower:]')

echo "🦞 Sego Agent Installer"
echo ""

mkdir -p "$INSTALL_DIR"

# Download latest release
RELEASE_URL="https://github.com/$REPO/releases/latest/download/$BINARY-$OS"
echo "Downloading $BINARY..."
if curl -fsSL "$RELEASE_URL" -o "$INSTALL_DIR/$BINARY"; then
    chmod +x "$INSTALL_DIR/$BINARY"
    echo "Installed to $INSTALL_DIR/$BINARY"
else
    # Fallback: build from source
    echo "No prebuilt binary for $OS. Building from source..."
    echo "This requires Rust: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    SRC_DIR=$(mktemp -d)
    git clone "https://github.com/$REPO.git" "$SRC_DIR"
    cd "$SRC_DIR/rust"
    cargo build --release 2>/dev/null
    cp "target/release/$BINARY" "$INSTALL_DIR/$BINARY"
    chmod +x "$INSTALL_DIR/$BINARY"
    rm -rf "$SRC_DIR"
    echo "Built and installed to $INSTALL_DIR/$BINARY"
fi

# Check PATH
if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
    echo "export PATH=\"\$PATH:$INSTALL_DIR\"" >> "$HOME/.bashrc"
    echo "export PATH=\"\$PATH:$INSTALL_DIR\"" >> "$HOME/.zshrc" 2>/dev/null || true
    echo "Added to PATH. Restart terminal or run: source ~/.bashrc"
fi

echo ""
echo "Setup complete! Configure your API key:"
echo "  export ANTHROPIC_API_KEY=sk-your-key"
echo "  export ANTHROPIC_BASE_URL=https://api.deepseek.com/anthropic"
echo "  sego"
