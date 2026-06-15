#!/usr/bin/env bash
# sego-resume.sh - Resume the latest Sego session after a crash or interruption.
#
# Usage:
#   sego-resume                  Restore latest session, enter REPL
#   sego-resume /status          Show status of restored session
#   sego-resume /recovery-export Export recovery summary to .sego/recovery/recovery-summary.md
#   sego-resume latest /status   Equivalent (explicit "latest" alias)
#
# Any extra args are forwarded to the resumed session.
set -euo pipefail

# Locate the sego binary: prefer a local cargo build, then PATH.
SEGO_BIN=""
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ -x "${SCRIPT_DIR}/../rust/target/debug/sego" ]]; then
    SEGO_BIN="${SCRIPT_DIR}/../rust/target/debug/sego"
elif [[ -n "${CARGO_TARGET_DIR:-}" && -x "${CARGO_TARGET_DIR}/debug/sego" ]]; then
    SEGO_BIN="${CARGO_TARGET_DIR}/debug/sego"
elif command -v sego >/dev/null 2>&1; then
    SEGO_BIN="sego"
else
    echo "sego-resume: sego binary not found." >&2
    echo "Build it first with:  cargo build -p rusty-claude-cli" >&2
    echo "Or ensure 'sego' is on your PATH." >&2
    exit 1
fi

exec "${SEGO_BIN}" --resume latest "$@"
