#!/usr/bin/env bash
# ============================================================================
# Sego Agent — Dev Build Script
# ============================================================================
# One-command full build cycle: format → lint → test → build release
# Usage: ./scripts/dev-build.sh [--fast] [--release] [--check-only]
# ============================================================================

set -euo pipefail
cd "$(dirname "$0")/../rust"

FAST=false
RELEASE=false
CHECK_ONLY=false

for arg in "$@"; do
    case "$arg" in
        --fast) FAST=true ;;
        --release) RELEASE=true ;;
        --check-only) CHECK_ONLY=true ;;
        --help|-h)
            echo "Usage: ./scripts/dev-build.sh [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --fast        Skip full workspace tests (CLI tests only)"
            echo "  --release     Also build release binary"
            echo "  --check-only  Only format + clippy, no tests"
            exit 0
            ;;
        *)
            echo "Unknown option: $arg"
            exit 1
            ;;
    esac
done

echo "============================================"
echo "🦞 Sego Agent — Dev Build"
echo "============================================"
echo ""

# Step 1: Format
echo "━ Step 1/3: cargo fmt --all --check"
if cargo fmt --all --check; then
    echo "✅ Format OK"
else
    echo "❌ Format failed. Fix with: cargo fmt --all"
    exit 1
fi
echo ""

# Step 2: Clippy
echo "━ Step 2/3: cargo clippy --workspace --all-targets -- -D warnings"
if cargo clippy --workspace --all-targets -- -D warnings; then
    echo "✅ Clippy OK"
else
    echo "❌ Clippy failed. Fix warnings above."
    exit 1
fi
echo ""

# Step 3: Tests
if [ "$CHECK_ONLY" = true ]; then
    echo "━ Step 3/3: Skipped (--check-only)"
else
    if [ "$FAST" = true ]; then
        echo "━ Step 3/3: cargo test -p rusty-claude-cli (--fast)"
        if cargo test -p rusty-claude-cli; then
            echo "✅ CLI tests OK"
        else
            echo "❌ CLI tests failed"
            exit 1
        fi
    else
        echo "━ Step 3/3: cargo test --workspace"
        if cargo test --workspace; then
            echo "✅ Workspace tests OK"
        else
            echo "❌ Workspace tests failed"
            exit 1
        fi
    fi
fi
echo ""

# Step 4: Build release (optional)
if [ "$RELEASE" = true ]; then
    echo "━ Step 4/4: cargo build --release"
    if cargo build --release; then
        echo "✅ Release build OK"
        echo "   Binary: rust/target/release/sego"
    else
        echo "❌ Release build failed"
        exit 1
    fi
    echo ""
fi

echo "============================================"
echo "✅ All checks passed!"
echo "============================================"
