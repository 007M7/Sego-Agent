#!/bin/bash
# Sego Agent — Linux/macOS one-liner installer
# Run: curl -fsSL https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.sh | bash
#
# The installer pins a release tag and verifies the published SHA-256 before
# anything is written into place. Overrides, both explicit:
#   SEGO_INSTALL_VERSION=latest            # float to the newest release
#   SEGO_INSTALL_ALLOW_UNVERIFIED=1        # skip the checksum check

set -e
REPO="007M7/Sego-Agent"
BINARY="sego"
INSTALL_DIR="$HOME/.local/bin"
OS=$(uname -s | tr '[:upper:]' '[:lower:]')

# Bumped when a release is cut. A tag, not "latest": a floating pointer means
# the bytes a user installs change without anything in this file changing.
SEGO_VERSION="${SEGO_INSTALL_VERSION:-v0.1.9}"

case "$OS" in
    darwin*) RELEASE_BINARY="sego-macos" ;;
    linux*) RELEASE_BINARY="sego" ;;
    *) RELEASE_BINARY="sego" ;;
esac

if [ "$SEGO_VERSION" = "latest" ]; then
    RELEASE_BASE="https://github.com/$REPO/releases/latest/download"
else
    RELEASE_BASE="https://github.com/$REPO/releases/download/$SEGO_VERSION"
fi

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print tolower($1)}'
    else
        shasum -a 256 "$1" | awk '{print tolower($1)}'
    fi
}

# `checksums.txt` lists "<hash>  <name>" per binary.
expected_sha256_for() {
    awk -v name="$2" '{ n=$2; sub(/^\*/, "", n); if (n == name) { print tolower($1); exit } }' "$1"
}

echo "🦞 Sego Agent Installer"
echo ""

mkdir -p "$INSTALL_DIR"

# Download and verify before replacing anything: a failed check must leave the
# existing installation untouched.
TEMP_BINARY="$(mktemp "${TMPDIR:-/tmp}/sego-download.XXXXXX")"
trap 'rm -f "$TEMP_BINARY"' EXIT

echo "Downloading $RELEASE_BINARY ($SEGO_VERSION)..."
if curl -fsSL "$RELEASE_BASE/$RELEASE_BINARY" -o "$TEMP_BINARY"; then
    if [ "${SEGO_INSTALL_ALLOW_UNVERIFIED:-0}" = "1" ]; then
        echo "WARNING: SEGO_INSTALL_ALLOW_UNVERIFIED=1 - skipping checksum verification."
    else
        CHECKSUMS="$(mktemp "${TMPDIR:-/tmp}/sego-checksums.XXXXXX")"
        if ! curl -fsSL "$RELEASE_BASE/checksums.txt" -o "$CHECKSUMS"; then
            rm -f "$CHECKSUMS"
            echo "ERROR: checksums.txt is not available for $SEGO_VERSION." >&2
            echo "Refusing to install an unverified binary." >&2
            echo "Set SEGO_INSTALL_ALLOW_UNVERIFIED=1 to override." >&2
            exit 1
        fi
        EXPECTED="$(expected_sha256_for "$CHECKSUMS" "$RELEASE_BINARY")"
        rm -f "$CHECKSUMS"
        if [ -z "$EXPECTED" ]; then
            echo "ERROR: checksums.txt does not list $RELEASE_BINARY." >&2
            echo "Refusing to install an unverified binary." >&2
            exit 1
        fi
        ACTUAL="$(sha256_of "$TEMP_BINARY")"
        if [ "$ACTUAL" != "$EXPECTED" ]; then
            echo "ERROR: checksum mismatch for $RELEASE_BINARY" >&2
            echo "  expected $EXPECTED" >&2
            echo "  actual   $ACTUAL" >&2
            echo "The download was not installed. Do not run it." >&2
            exit 1
        fi
        echo "Checksum verified ($EXPECTED)."
    fi
    # Only now does the existing installation get replaced.
    chmod +x "$TEMP_BINARY"
    mv -f "$TEMP_BINARY" "$INSTALL_DIR/$BINARY"
    trap - EXIT
    echo "Installed to $INSTALL_DIR/$BINARY"
else
    # Fallback: build from source, pinned to the same tag.
    echo "No prebuilt binary. Building from source at $SEGO_VERSION..."
    echo "This requires Rust: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    SRC_DIR=$(mktemp -d)
    if [ "$SEGO_VERSION" = "latest" ]; then
        git clone "https://github.com/$REPO.git" "$SRC_DIR"
    else
        git clone --branch "$SEGO_VERSION" --depth 1 "https://github.com/$REPO.git" "$SRC_DIR"
    fi
    cd "$SRC_DIR/rust"
    cargo build --release 2>/dev/null
    cp "target/release/$BINARY" "$INSTALL_DIR/$BINARY"
    chmod +x "$INSTALL_DIR/$BINARY"
    cd - >/dev/null
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
echo "Setup complete! Configure your model:"
echo ""
echo "  # DeepSeek (recommended, native support)"
echo "  export DEEPSEEK_API_KEY=sk-your-deepseek-key"
echo "  export DEEPSEEK_MODEL=deepseek-v4-flash    # optional, defaults to flash"
echo ""
echo "  # Or Anthropic (alternative)"
echo "  export ANTHROPIC_API_KEY=sk-your-anthropic-key"
echo ""
echo "  sego"
