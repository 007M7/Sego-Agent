#!/usr/bin/env bash
# ============================================================================
# Sego Agent — Install Git Hooks
# ============================================================================
# Installs pre-commit hook from scripts/ to .git/hooks/
# Usage: ./scripts/install-hooks.sh
# ============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
HOOK_SOURCE="$SCRIPT_DIR/pre-commit"
HOOK_DEST="$REPO_ROOT/.git/hooks/pre-commit"

if [ ! -f "$HOOK_SOURCE" ]; then
    echo "❌ Hook source not found: $HOOK_SOURCE"
    exit 1
fi

if [ ! -d "$REPO_ROOT/.git/hooks" ]; then
    echo "❌ Not a git repository (no .git/hooks directory)"
    exit 1
fi

cp "$HOOK_SOURCE" "$HOOK_DEST"
chmod +x "$HOOK_DEST"

echo "✅ Pre-commit hook installed: $HOOK_DEST"
echo ""
echo "The hook will run:"
echo "  - cargo fmt --all --check"
echo "  - cargo clippy --workspace --all-targets -- -D warnings"
echo ""
echo "To skip the hook for a commit: git commit --no-verify"
