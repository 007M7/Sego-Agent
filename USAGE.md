# Sego Agent Usage

This guide covers the current Rust workspace under `rust/` and the `sego` CLI binary.

## Prerequisites

- Normal release install: no Rust toolchain required.
- Source build only: Rust toolchain with `cargo`.
- One of:
  - `DEEPSEEK_API_KEY` for native DeepSeek access (recommended)
  - `ANTHROPIC_API_KEY` for Anthropic access
  - `sego login` for OAuth-based auth
- Optional: `DEEPSEEK_MODEL` to override the default model (defaults to `deepseek-v4-flash`)
- Optional: `ANTHROPIC_BASE_URL` when targeting a proxy or local service

## Install for normal users

### Windows one-line install

```powershell
irm https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.ps1 | iex
```

The installer downloads the latest `sego.exe`, installs it to `~/sego`, adds it to PATH, creates `~/sego/Sego.cmd`, and creates a **Sego** desktop shortcut.

### Windows direct download

- From [GitHub Releases](https://github.com/007M7/Sego-Agent/releases/latest): download `sego-windows.zip`, unzip it, and double-click `Sego.cmd`.
- From GitHub **Code ? Download ZIP**: unzip the source package and double-click `start-sego-windows.cmd`. It bootstraps the latest release binary, creates the desktop shortcut, and starts Sego.

GitHub source ZIP files do not contain compiled binaries. Use `sego-windows.zip` for offline double-click usage.

### macOS / Linux

```bash
curl -fsSL https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.sh | bash
```

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

- `opus` ? `claude-opus-4-7`
- `sonnet` ? `claude-sonnet-4-6`
- `haiku` ? `claude-haiku-4-5`

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
- Lane Events: started ? ready ? running ? green/red ? finished
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

## Crash Recovery

```bash
sego --resume latest           # Restore the most recent session
sego --resume latest /status   # Show restored session status
sego --resume latest /recovery-export  # Export recovery summary
```

- Abnormally terminated sessions are detected on next launch.
- Restored sessions do not replay old tool calls (security boundary).
- Gracefully exited sessions do not trigger recovery prompts.

## Review-Trust Permission Profile

```bash
sego --permission-profile review-trust
```

Reduces permission prompt noise during review/verify sessions:

- **Auto-allowed**: read-only commands (cat, rg, grep, ls, git status), verification commands (cargo test, npm test), `.sego/` writes.
- **Requires confirmation**: source writes, dependency installs, git commit/merge.
- **Denied**: `rm -rf`, `git reset --hard`, `git push`, `sudo`.

`--permission-mode` and `--permission-profile` are mutually exclusive.

## Sidecar JSON Interface (PoC)

```bash
echo '{"schema_version":1,"action":"review","cwd":"/project","scope":"staged"}' \
  | sego sidecar review
```

- stdin receives a JSON request; stdout returns a JSON response.
- Error responses are structured envelopes (`schema_version` / `status` / `error`).
- The skill package at `skills/sego-review/` can be invoked by SKILL.md-compatible tools (Claude Code, Codex, Cursor, etc.).
- **PoC status**: only the `review` action is supported. stdout is reserved for pure JSON; diagnostics go to stderr.
# sego-review skill

AI coding engineering trust review ? a Sego sidecar skill package.

## What it does

After you generate or modify code with any AI tool (Claude Code, Codex, Cursor, etc.), invoke this skill to get a structured review from Sego. Sego returns findings with severity, file, line, evidence, risk, and suggestion ? then persists a review artifact to `.sego/reviews/`.

## Install

### Prerequisites

- [Sego](https://github.com/007M7/Sego-Agent) built and on PATH, or built from source:
  ```bash
  cargo build -p rusty-claude-cli
  ```
- A git repository (review uses `git diff`)

### Use in Zcode / Claude Code / any SKILL.md-compatible tool

Copy the `sego-review/` directory into your tool's skills directory.

## Usage

### Command line

```bash
# Unix
echo '{"schema_version":1,"action":"review","cwd":"'"$PWD"'","scope":"staged"}' \
  | bash skills/sego-review/scripts/sego-review.sh

# Windows PowerShell
$request = @{ schema_version = 1; action = "review"; cwd = (Get-Location).Path; scope = "staged" } | ConvertTo-Json
$request | powershell -File skills/sego-review/scripts/sego-review.ps1
```

### Via `sego sidecar review` directly

```bash
echo '{"schema_version":1,"action":"review","cwd":"/project","scope":"staged"}' | sego sidecar review
```

## Request format

| Field | Type | Required | Description |
|---|---|---|---|
| `schema_version` | integer | yes | Must be `1` |
| `action` | string | yes | Must be `"review"` |
| `cwd` | string | yes | Absolute path to project root |
| `scope` | string | no | `"staged"`, `"workspace"`, or a path (default: `staged`) |
| `options.model` | string | no | Override review model |
| `context.user_intent` | string | no | What the user wants to achieve |

## Response format

See [SKILL.md](SKILL.md) for the full response schema. Key fields:

- `status`: `"ok"` or `"error"`
- `findings`: array of structured findings (severity/file/line/evidence/risk/suggestion)
- `artifact_path`: path to persisted review JSON
- `error`: present only when `status` is `"error"`

## Graceful degradation

If the Sego binary is not found, the script outputs a structured error JSON to stderr and exits with code 1 ? it does not crash silently.

## ?? AI ?????Sidecar skill PoC?

### ???? skill

```bash
# Mac / Linux
bash skills/sego-review/install.sh

# Windows
powershell -File skills\sego-review\install.ps1
```

???? Claude Code / Codex / Zcode / Cursor ??? skill ??

### ?? sidecar ??

```bash
echo '{"schema_version":1,"action":"review","cwd":"/project","scope":"staged"}' | sego sidecar review
```

- stdout ??? JSON?findings + artifact_path?
- stderr ??????
- exit code?0 ???1 ??

### ?????PoC?

- ??? `review` action?verify/export ?????
- Cursor ?? `.cursorrules` ?????????
- ???????early integration??????? IDE ????
