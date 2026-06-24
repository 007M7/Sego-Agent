# Sego Agent ? Rust Implementation

A high-performance Rust implementation of the Sego AI coding agent. Built for speed, safety, and native tool execution.

Sego is an engineering trust layer / review layer for AI-generated code: let your AI coding tool (Claude Code, Codex, Cursor, Zcode, etc.) write the code, then run Sego to independently verify correctness, regressions, security risks, and data-flow drift before you commit.

For a task-oriented guide with copy/paste examples, see [`USAGE.md`](USAGE.md).

## Quick Start

```bash
# Inspect available commands
cd rust/
cargo run -p rusty-claude-cli -- --help

# Build the workspace
cargo build --workspace

# Run the interactive REPL
cargo run -p rusty-claude-cli -- --model claude-opus-4-6

# One-shot prompt
cargo run -p rusty-claude-cli -- prompt "explain this codebase"

# JSON output for automation
cargo run -p rusty-claude-cli -- --output-format json prompt "summarize src/main.rs"
```

## Code Review

Sego supports multiple review scopes:

```bash
# Review current workspace changes
cargo run -p rusty-claude-cli -- review

# Review staged changes only
cargo run -p rusty-claude-cli -- review staged

# Full repository audit (clean clones, non-Git directories)
cargo run -p rusty-claude-cli -- review --full /path/to/your/repo

# Inspect saved review artifacts
cargo run -p rusty-claude-cli -- review list
cargo run -p rusty-claude-cli -- review show <id>
```

Review artifacts are saved to `.sego/reviews/` in the workspace root. Each review produces a Markdown report, a JSON findings file, and an index entry in `index.jsonl`.

### Export / Save

```bash
# In the REPL, export the latest assistant response:
/export ./review.md

# Or via natural language (requires explicit "last/previous/刚才/上一条"):
# "把刚才的审查结果写成 ./review.md"
# "save the last review report to PR43-review.md"
```

## Natural-Language Local Actions

Sego supports deterministic natural-language triggers for local actions. Say `/dir` to see the action directory, or try:

| Natural language | What Sego does |
|---|---|
| `帮我 review 当前改动` | Code review |
| `切换到工作区 D:\Project` | Switch workspace |
| `把刚才的回复保存到 ./out.md` | Export latest response |
| `检查更新` | Check for updates |
| `退出` | Exit Sego |

The export/save action has a **safety boundary**: it requires explicit `last/previous/刚才/上一条/上回`. Bare phrases like "保存报告" or "导出 md" without a clear target will show `/dir` guidance instead of guessing.

## Configuration

Set your API credentials:

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
# Or use a proxy
export ANTHROPIC_BASE_URL="https://your-proxy.com"
```

Or authenticate via OAuth and let the CLI persist credentials locally:

```bash
cargo run -p rusty-claude-cli -- login
```

## Mock parity harness

The workspace includes a deterministic Anthropic-compatible mock service and a clean-environment CLI harness for end-to-end parity checks.

```bash
cd rust/

# Run the scripted clean-environment harness
./scripts/run_mock_parity_harness.sh

# Or start the mock service manually for ad hoc CLI runs
cargo run -p mock-anthropic-service -- --bind 127.0.0.1:0
```

Harness coverage:

- `streaming_text`
- `read_file_roundtrip`
- `grep_chunk_assembly`
- `write_file_allowed`
- `write_file_denied`
- `multi_tool_turn_roundtrip`
- `bash_stdout_roundtrip`
- `bash_permission_prompt_approved`
- `bash_permission_prompt_denied`
- `plugin_tool_roundtrip`

Primary artifacts:

- `crates/mock-anthropic-service/` ? reusable mock Anthropic-compatible service
- `crates/rusty-claude-cli/tests/mock_parity_harness.rs` ? clean-env CLI harness
- `scripts/run_mock_parity_harness.sh` ? reproducible wrapper
- `scripts/run_mock_parity_diff.py` ? scenario checklist + PARITY mapping runner
- `mock_parity_scenarios.json` ? scenario-to-PARITY manifest

## Features

| Feature | Status |
|---------|--------|
| Anthropic API + streaming | ? |
| OAuth login/logout | ? |
| Interactive REPL (rustyline) | ? |
| Tool system (bash, read, write, edit, grep, glob) | ? |
| Web tools (search, fetch) | ? |
| Sub-agent orchestration | ? |
| Todo tracking | ? |
| Notebook editing | ? |
| CLAUDE.md / project memory | ? |
| Config file hierarchy (.claude.json) | ? |
| Permission system | ? |
| MCP server lifecycle | ? |
| Session persistence + resume | ? |
| Extended thinking (thinking blocks) | ? |
| Cost tracking + usage display | ? |
| Git integration | ? |
| Full repo audit (non-Git) | ? |
| Markdown terminal rendering (ANSI) | ? |
| Model aliases (opus/sonnet/haiku) | ? |
| Slash commands (/status, /compact, /clear, etc.) | ? |
| Hooks (PreToolUse/PostToolUse) | ?? Config only |
| Plugin system | ?? Planned |
| Skills registry | ?? Planned |

## Model Aliases

Short names resolve to the latest model versions:

| Alias | Resolves To |
|-------|------------|
| `opus` | `claude-opus-4-6` |
| `sonnet` | `claude-sonnet-4-6` |
| `haiku` | `claude-haiku-4-5-20251213` |

## CLI Flags

```
sego [OPTIONS] [COMMAND]

Options:
  --model MODEL                    Override the active model
  --dangerously-skip-permissions   Skip all permission checks
  --permission-mode MODE           Set read-only, workspace-write, or danger-full-access
  --allowedTools TOOLS             Restrict enabled tools
  --output-format FORMAT           Non-interactive output format (text or json)
  --resume SESSION                 Re-open a saved session or inspect it with slash commands
  --version, -V                    Print version and build information locally

Commands:
  prompt <text>      One-shot prompt (non-interactive)
  login              Authenticate via OAuth
  logout             Clear stored credentials
  init               Initialize project config
  status             Show the current workspace status snapshot
  sandbox            Show the current sandbox isolation snapshot
  agents             Inspect agent definitions
  mcp                Inspect configured MCP servers
  skills             Inspect installed skills
  system-prompt      Render the assembled system prompt
```

For the current canonical help text, run `cargo run -p rusty-claude-cli -- --help`.

## Slash Commands (REPL)

Tab completion expands slash commands, model aliases, permission modes, and recent session IDs.

| Command | Description |
|---------|-------------|
| `/help` | Show help |
| `/status` | Show session status (model, tokens, cost) |
| `/cost` | Show cost breakdown |
| `/compact` | Compact conversation history |
| `/clear` | Clear conversation |
| `/model [name]` | Show or switch model |
| `/permissions` | Show or switch permission mode |
| `/config [section]` | Show config (env, hooks, model) |
| `/memory` | Show CLAUDE.md contents |
| `/diff` | Show git diff |
| `/export [path]` | Export conversation |
| `/resume [id]` | Resume a saved conversation |
| `/session [id]` | Resume a previous session |
| `/version` | Show version |
| `/dir` | Show action directory with usage examples |

See [`USAGE.md`](USAGE.md) for examples covering interactive use, JSON automation, sessions, permissions, and the mock parity harness.

## Workspace Layout

```
rust/
??? Cargo.toml              # Workspace root
??? Cargo.lock
??? crates/
    ??? api/                # Anthropic API client + SSE streaming
    ??? commands/           # Shared slash-command registry
    ??? compat-harness/     # TS manifest extraction harness
    ??? mock-anthropic-service/ # Deterministic local Anthropic-compatible mock
    ??? plugins/            # Plugin registry and hook wiring primitives
    ??? runtime/            # Session, config, permissions, MCP, prompts
    ??? rusty-claude-cli/   # Main CLI binary (`sego`)
    ??? telemetry/          # Session tracing and usage telemetry types
    ??? tools/              # Built-in tool implementations
```

### Crate Responsibilities

- **api** ? HTTP client, SSE stream parser, request/response types, auth (API key + OAuth bearer)
- **commands** ? Slash command definitions and help text generation
- **compat-harness** ? Extracts tool/prompt manifests from upstream TS source
- **mock-anthropic-service** ? Deterministic `/v1/messages` mock for CLI parity tests and local harness runs
- **plugins** ? Plugin metadata, registries, and hook integration surfaces
- **runtime** ? `ConversationRuntime` agentic loop, `ConfigLoader` hierarchy, `Session` persistence, permission policy, MCP client, system prompt assembly, usage tracking
- **rusty-claude-cli** ? REPL, one-shot prompt, streaming display, tool call rendering, CLI argument parsing
- **telemetry** ? Session trace events and supporting telemetry payloads
- **tools** ? Tool specs + execution: Bash, ReadFile, WriteFile, EditFile, GlobSearch, GrepSearch, WebSearch, WebFetch, Agent, TodoWrite, NotebookEdit, Skill, ToolSearch, REPL runtimes

## Stats

- **~20K lines** of Rust
- **9 crates** in workspace
- **Binary name:** `sego`
- **Default model:** `claude-opus-4-6`
- **Default permissions:** `danger-full-access`

## License

See repository root.
