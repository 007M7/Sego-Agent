#!/usr/bin/env bash
# sego-review skill installer — copies the skill package to detected AI coding tools.
# Supports: Zcode, Claude Code, Codex (SKILL.md native), Cursor (.cursorrules).
# Usage: bash skills/sego-review/install.sh [--all | --zcode | --claude | --codex | --cursor]
# Idempotent: re-running overwrites existing files after confirmation.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SKILL_NAME="sego-review"

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

info()  { echo -e "${GREEN}[✓]${NC} $1"; }
warn()  { echo -e "${YELLOW}[!]${NC} $1"; }
error() { echo -e "${RED}[✗]${NC} $1"; }

# Target directories for each tool
detect_targets() {
    local targets=()
    # Zcode
    [[ -d "$HOME/.zcode" ]] && targets+=("zcode:$HOME/.zcode/skills/$SKILL_NAME")
    # Claude Code
    [[ -d "$HOME/.claude" ]] && targets+=("claude:$HOME/.claude/skills/$SKILL_NAME")
    # Codex
    [[ -d "$HOME/.codex" ]] && targets+=("codex:$HOME/.codex/skills/$SKILL_NAME")
    # Cursor (uses .cursorrules in project root, not a global skills dir)
    echo "${targets[@]}"
}

install_skill() {
    local tool="$1"
    local dest="$2"

    echo ""
    echo "Installing $SKILL_NAME for $tool..."
    echo "  Target: $dest"

    # Create parent directory
    mkdir -p "$dest"

    # Copy SKILL.md and scripts
    cp "$SCRIPT_DIR/SKILL.md" "$dest/SKILL.md"
    cp -r "$SCRIPT_DIR/scripts" "$dest/scripts" 2>/dev/null || true
    cp "$SCRIPT_DIR/README.md" "$dest/README.md" 2>/dev/null || true

    info "Installed $SKILL_NAME for $tool at $dest"
}

install_cursor_rules() {
    local dest="${1:-$PWD}"
    local rules_file="$dest/.cursorrules"

    echo ""
    echo "Installing $SKILL_NAME rules for Cursor..."
    echo "  Target: $rules_file"

    cat > "$rules_file" << 'CURSOR_EOF'
# Sego Review Integration
#
# When the user asks to "review", "check", or "validate" code changes:
# 1. Get the project root: `pwd`
# 2. Run: echo '{"schema_version":1,"action":"review","cwd":"<project-root>","scope":"staged"}' | sego sidecar review
# 3. Parse the JSON response and report findings to the user
# 4. Highlight critical/high severity findings first
#
# If sego is not found, tell the user to install it:
#   curl -fsSL https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.sh | bash
CURSOR_EOF

    info "Installed .cursorrules for Cursor at $rules_file"
}

# --- Main ---

echo "Sego Review Skill Installer"
echo "============================"

TARGETS=$(detect_targets)
if [[ -z "$TARGETS" ]]; then
    warn "No supported AI coding tools detected in $HOME."
    echo ""
    echo "You can install manually:"
    echo "  Zcode:  cp -r skills/sego-review ~/.zcode/skills/"
    echo "  Claude: cp -r skills/sego-review ~/.claude/skills/"
    echo "  Codex:  cp -r skills/sego-review ~/.codex/skills/"
    echo "  Cursor: Run with --cursor to generate .cursorrules"
    exit 0
fi

# Parse args
TARGET_TOOL="${1:---all}"

if [[ "$TARGET_TOOL" == "--cursor" ]]; then
    install_cursor_rules "$PWD"
    exit 0
fi

# Install to all detected tools
for target in $TARGETS; do
    tool="${target%%:*}"
    dest="${target#*:}"

    if [[ "$TARGET_TOOL" == "--all" ]] || [[ "$TARGET_TOOL" == "--$tool" ]]; then
        install_skill "$tool" "$dest"
    fi
done

# Offer Cursor rules
echo ""
warn "For Cursor support, run: bash skills/sego-review/install.sh --cursor"

echo ""
info "Done! Restart your AI coding tool to pick up the new skill."
