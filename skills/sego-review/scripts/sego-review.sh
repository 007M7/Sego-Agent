#!/usr/bin/env bash
# sego-review.sh — Invoke Sego sidecar review from a skill package.
#
# Reads a JSON request from stdin, passes it to `sego sidecar review`,
# and outputs the JSON response to stdout.
#
# Usage:
#   echo '{"schema_version":1,"action":"review","cwd":"/project","scope":"staged"}' | sego-review.sh
set -euo pipefail

# Locate the sego binary: local cargo build > CARGO_TARGET_DIR > PATH.
SEGO_BIN=""
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"

if [[ -x "${REPO_ROOT}/rust/target/debug/sego" ]]; then
    SEGO_BIN="${REPO_ROOT}/rust/target/debug/sego"
elif [[ -x "${REPO_ROOT}/rust/target/release/sego" ]]; then
    SEGO_BIN="${REPO_ROOT}/rust/target/release/sego"
elif [[ -n "${CARGO_TARGET_DIR:-}" && -x "${CARGO_TARGET_DIR}/debug/sego" ]]; then
    SEGO_BIN="${CARGO_TARGET_DIR}/debug/sego"
elif command -v sego >/dev/null 2>&1; then
    SEGO_BIN="sego"
else
    echo '{"schema_version":1,"status":"error","error":{"code":"sego_not_found","message":"Sego binary not found. Build with: cargo build -p rusty-claude-cli, or add sego to PATH."}}' >&2
    exit 1
fi

exec "${SEGO_BIN}" sidecar review
