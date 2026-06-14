# Sego Agent Usage

This guide covers the current Rust workspace under `rust/` and the `sego` CLI binary.

## Prerequisites

- Rust toolchain with `cargo`
- One of:
  - `ANTHROPIC_API_KEY` for direct API access
  - `sego login` for OAuth-based auth
- Optional: `ANTHROPIC_BASE_URL` when targeting a proxy or local service

## Build the workspace

```bash
cd rust
cargo build --workspace
```

The CLI binary is available at `rust/target/debug/sego` after a debug build.

## Quick start

### Interactive REPL

```bash
cd rust
./target/debug/sego
```

### One-shot prompt

```bash
cd rust
./target/debug/sego "summarize this repository"
```

### JSON output for scripting

```bash
cd rust
./target/debug/sego --output-format json status
```

## Model and permission controls

```bash
cd rust
./target/debug/sego --model sonnet "review this diff"
./target/debug/sego --permission-mode read-only "summarize Cargo.toml"
./target/debug/sego --permission-mode workspace-write "update README.md"
./target/debug/sego --allowedTools read,glob "inspect the runtime crate"
```

Supported permission modes:

- `read-only`
- `workspace-write`
- `danger-full-access`

Model aliases currently supported by the CLI:

- `opus` → `claude-opus-4-7`
- `sonnet` → `claude-sonnet-4-6`
- `haiku` → `claude-haiku-4-5`

## Authentication

### API key

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
```

### OAuth

```bash
cd rust
./target/debug/sego login
./target/debug/sego logout
```

## Common operational commands

```bash
cd rust
./target/debug/sego status
./target/debug/sego doctor
./target/debug/sego review
./target/debug/sego learn
```

## Review MVP workflow

Sego's current public MVP focuses on a local review and verification loop before code is committed or handed off.

Recommended staged workflow:

```bash
git add <files>
./target/debug/sego /review ready
./target/debug/sego /review summary
./target/debug/sego /review safety staged
./target/debug/sego /review staged
./target/debug/sego /verify fast
```

Review commands:

| Command | Purpose |
|---|---|
| `/review ready` | Read-only staged readiness report with safety and verify-plan hints |
| `/review summary` | Read-only delivery summary for demo, PR handoff, and agent handoff |
| `/review safety staged` | Local staged safety lock for obvious secrets, dangerous commands, and hardcoded local paths |
| `/review staged` | Model-assisted review for staged diff |
| `/review` | Model-assisted review for the current workspace diff |
| `/review tools` | Read-only project toolchain suggestions |
| `/review list` | List persisted review reports |
| `/review show <review-id>` | Print a persisted review markdown report |
| `/review status <review-id>` | Show finding statuses for a persisted review |
| `/review mark <review-id> <finding-id> <status> [note]` | Mark a finding as `open`, `acknowledged`, `fixed`, or `ignored` |
| `/verify fast` | Execute the fast verification plan explicitly |

`/review ready`, `/review summary`, `/review safety staged`, and `/review tools` are read-only. They do not call models, run builds/tests, install dependencies, or modify files unless the command explicitly says it performs review or verification.

## Workflow review & learning (built-in)

sego automatically records every session as structured Lane Events.
Use `review` and `learn` to analyze your AI coding workflows.

### Review recent sessions

```bash
# Review the last session
./target/debug/sego review

# Review last 5 sessions
./target/debug/sego review --last 5

# JSON output for scripting
./target/debug/sego review --output-format json
```

### Learning & optimization

```bash
# Get optimization suggestions based on your history
./target/debug/sego learn

# JSON output for dashboards
./target/debug/sego learn --output-format json
```

### What gets recorded automatically

Every `sego` session records:
- Lane Events: started → ready → running → green/red → finished
- Failure classification (11 types) and recovery attempts
- Green Contract level achieved
- Efficiency scoring

Data is stored in `.sego/workflow/sessions/` under your workspace.

## Session management

REPL turns are persisted under `.sego/sessions/` in the current workspace.

```bash
cd rust
./target/debug/sego --resume latest
./target/debug/sego --resume latest /status /diff
```

Useful interactive commands include `/help`, `/status`, `/cost`, `/config`, `/session`, `/model`, `/permissions`, and `/export`.

## Config file resolution order

Runtime config is loaded in this order, with later entries overriding earlier ones:

1. `~/.sego.json`
2. `~/.config/sego/settings.json`
3. `<repo>/.sego.json`
4. `<repo>/.sego/settings.json`
5. `<repo>/.sego/settings.local.json`

## Mock parity harness

The workspace includes a deterministic Anthropic-compatible mock service and parity harness.

```bash
cd rust
./scripts/run_mock_parity_harness.sh
```

Manual mock service startup:

```bash
cd rust
cargo run -p mock-anthropic-service -- --bind 127.0.0.1:0
```

## Verification

```bash
cd rust
cargo test --workspace
```

## Workspace overview

Current Rust crates:

- `api`
- `commands`
- `compat-harness`
- `mock-anthropic-service`
- `plugins`
- `runtime`
- `rusty-claude-cli`
- `telemetry`
- `tools`
