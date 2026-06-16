# 🛠️ Sego Agent — Developer Guide

> **Target audience:** Contributors modifying the Rust runtime, CLI, API layer, tools, plugins, MCP integration, or Python parity port.

---

## Table of Contents

1. [Quick Start](#quick-start)
2. [Repository Architecture](#repository-architecture)
3. [Developer Mode](#developer-mode)
4. [Build & Test](#build--test)
5. [Code Quality](#code-quality)
6. [Making Changes](#making-changes)
7. [Testing Strategy](#testing-strategy)
8. [CI/CD Pipeline](#cicd-pipeline)
9. [Debugging](#debugging)
10. [Contribution Workflow](#contribution-workflow)
11. [Architecture Decision Records](#architecture-decision-records)

---

## Quick Start

```bash
# Clone and build
git clone https://github.com/007M7/Sego-Agent.git
cd Sego-Agent/rust
cargo build

# Run tests
cargo test --workspace

# Run linting
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings

# Build release binary
cargo build --release
```

### Prerequisites

| Tool | Version | Required for |
|------|---------|-------------|
| Rust | 1.80+ | Core development |
| Git | 2.40+ | Version control |
| Python | 3.10+ | Python parity port tests |
| cargo-nextest | latest (optional) | Faster test runner |

---

## Repository Architecture

```
Sego-Agent/
├── rust/                          # 🦀 Main Rust workspace (~50K LOC)
│   ├── Cargo.toml                 # Workspace root
│   ├── rustfmt.toml               # Format rules
│   └── crates/
│       ├── api/                   # Anthropic-compatible client + SSE
│       │   └── src/
│       │       ├── client.rs      # HTTP client, auth, retry
│       │       ├── providers/     # Model provider adapters
│       │       │   ├── anthropic.rs
│       │       │   └── openai_compat.rs
│       │       ├── sse.rs         # Server-Sent Events parser
│       │       ├── types.rs       # Request/response types
│       │       └── prompt_cache.rs
│       │
│       ├── runtime/               # 🧠 Core runtime loop
│       │   └── src/
│       │       ├── lib.rs         # Public API surface
│       │       ├── conversation.rs # Turn-based conversation loop
│       │       ├── session.rs     # Session state + persistence (JSONL)
│       │       ├── recovery.rs    # Crash/session resume recovery state
│       │       ├── permissions.rs # Permission mode enforcement
│       │       ├── config.rs      # 5-layer config hierarchy
│       │       ├── prompt.rs      # System prompt builder
│       │       ├── compact.rs     # Context compaction
│       │       ├── sandbox.rs     # Sandbox detection
│       │       │
│       │       ├── lane_events.rs      # 16 structured event types
│       │       ├── recovery_recipes.rs # 7 automatic recovery patterns
│       │       ├── policy_engine.rs    # Executable autonomous rules
│       │       ├── green_contract.rs   # 4-level quality gates
│       │       ├── stale_branch.rs     # Branch freshness detection
│       │       ├── worker_boot.rs      # Agent lifecycle state machine
│       │       ├── summary_compression.rs # Smart context compaction
│       │       ├── task_packet.rs      # Structured task format
│       │       │
│       │       ├── mcp.rs             # MCP config loading
│       │       ├── mcp_client.rs      # MCP client protocol
│       │       ├── mcp_stdio.rs       # MCP stdio transport
│       │       ├── mcp_lifecycle_hardened.rs # MCP lifecycle hardening
│       │       ├── mcp_tool_bridge.rs # MCP tool bridge
│       │       │
│       │       ├── hooks.rs           # Pre/post tool hooks
│       │       ├── plugin_lifecycle.rs # Plugin lifecycle management
│       │       ├── bash.rs            # Bash tool executor
│       │       ├── bash_validation.rs # Bash command validation
│       │       ├── oauth.rs           # OAuth flow
│       │       ├── lsp_client.rs      # LSP integration
│       │       ├── trust_resolver.rs  # Trust gate resolver
│       │       ├── session_control.rs # Session control API
│       │       ├── community_learning.rs # Anonymous telemetry
│       │       ├── task_registry.rs   # Background task registry
│       │       ├── team_cron_registry.rs # Team/Cron registry
│       │       │
│       │       └── workflow/          # Session analysis
│       │           ├── mod.rs
│       │           ├── report.rs
│       │           └── store.rs
│       │
│       ├── commands/              # Slash command registry (140+ commands)
│       │   └── src/lib.rs
│       │
│       ├── tools/                 # 40+ built-in tool implementations
│       │   └── src/
│       │       ├── lib.rs         # GlobalToolRegistry
│       │       └── lane_completion.rs
│       │
│       ├── rusty-claude-cli/      # CLI entrypoint — main.rs is the entry
│       │   └── src/
│       │       ├── main.rs        # 8000+ LOC, all CLI logic
│       │       ├── app.rs         # Legacy app module (deprecated)
│       │       ├── args.rs        # Arg parsing (deprecated)
│       │       ├── input.rs       # Line editor
│       │       ├── init.rs        # Repo init template
│       │       └── render.rs      # Terminal rendering (ANSI/Markdown)
│       │
│       ├── plugins/               # Plugin registry + hooks
│       │   └── src/
│       │       ├── lib.rs
│       │       └── hooks.rs
│       │
│       ├── telemetry/             # Session tracing + usage analytics
│       │   └── src/lib.rs
│       │
│       ├── mock-anthropic-service/ # Deterministic test harness
│       │   └── src/
│       │
│       └── compat-harness/        # Protocol compatibility layer
│           └── src/lib.rs
│
├── src/                           # 🐍 Python parity port (reference)
│   ├── main.py                    # Session recording entrypoint
│   ├── runtime.py                 # PortRuntime with session bootstrap
│   ├── commands.py                # Command registry mirror
│   ├── tools.py                   # Tool registry mirror
│   ├── models.py                  # Shared data types
│   ├── parity_audit.py            # Port parity checker
│   ├── port_manifest.py           # Workspace manifest
│   ├── execution_registry.py      # Command/tool execution registry
│   ├── query_engine.py            # Query engine port
│   ├── history.py                 # History log
│   ├── context.py                 # Port context builder
│   ├── permissions.py             # Permission context
│   ├── system_init.py             # System init message builder
│   ├── setup.py                   # Workspace setup
│   ├── session_store.py           # Session persistence
│   ├── prefetch.py                # Module preloading
│   └── reference_data/            # JSON snapshots of archive surfaces
│       ├── commands_snapshot.json
│       ├── tools_snapshot.json
│       └── archive_surface_snapshot.json
│
├── tests/                         # Python port tests
│   └── test_porting_workspace.py
│
├── .github/workflows/             # CI/CD
│   ├── rust-ci.yml                # Rust CI (fmt, clippy, test)
│   └── release.yml                # Binary release workflow
│
├── .sego/                         # Developer configuration
│   └── dev.toml                   # Developer mode settings
│
├── install.ps1 / install.sh       # One-click installers
├── README.md                      # User-facing docs
├── DEVELOPMENT.md                 # ← You are here
├── AGENTS.md                      # AI agent instructions
├── PHILOSOPHY.md                  # Design philosophy
├── ROADMAP.md                     # Active roadmap + backlogs
├── PARITY.md                      # Protocol parity status
├── CONTRIBUTING.md                # Contribution guidelines
├── CHANGELOG.md                   # Release changelog
├── LICENSE                        # MIT
├── .gitignore
├── .editorconfig
└── rustfmt.toml
```

### Crate Dependency Graph

```
rusty-claude-cli (CLI entrypoint)
├── api (API client)
├── runtime (core loop)
├── commands (slash commands)
├── tools (tool registry)
├── plugins (plugin manager)
├── compat-harness
└── telemetry

runtime
├── api
├── tools
└── plugins

tools
├── runtime (for types)
└── plugins (for plugin tool aggregation)
```

### Crash Recovery State

Crash/session resume recovery is owned by `rust/crates/runtime/src/recovery.rs`.

- Recovery records are written under `.sego/recovery/`.
- Conversation JSONL sessions currently remain under `.claw/sessions/`; this is intentional until a dedicated migration is designed.
- Every recovery record that references a session must store `session_path` as an absolute path. Do not rely on `.claw` or `.sego` relative prefixes.
- Startup recovery detection must stay lightweight: read recovery JSON, check whether the referenced session file exists, and render a prompt. It must not scan the repo, call a model, auto-resume, or replay previous tool calls.
- `recovery_recipes.rs` is a separate automatic remediation subsystem and must not be used as crash/session resume state storage.

### Crash Recovery CLI Integration (A2)

The CLI layer (`rusty-claude-cli`) integrates the runtime recovery module as follows:

- **Startup notice**: `maybe_print_recovery_notice` runs after `parse_args` and only reads recovery JSON (no writes, no scans, no model calls). It triggers only for `Repl` / `Prompt` / `CodeReview` / `ResumeSession` — actions that create or restore a persistent session. Pure-query actions do not trigger it.
- **Two-layer write timing**: startup notice (read-only) is separate from session-state writes (`active`/`graceful`). `active` is written only after the session handle is known; `graceful` only on the normal return path. Error paths leave `active` in place.
- **No signal handler (MVP)**: A2 deliberately avoids signal handling. An interrupted/crashed session keeps `active` and is surfaced as recoverable on the next launch. Precise `interrupted` vs `crashed` distinction is deferred to a future `ctrlc` crate evaluation.
- **`/recovery-export` command**: separate from `/export`. `/export` writes the session transcript (forced `.txt` suffix); `/recovery-export` writes the recovery summary (markdown, supports `.md`, defaults to `.sego/recovery/recovery-summary.md`). Both resume and REPL modes support it via the shared `write_recovery_export` helper.
- **`sego-resume` script**: `scripts/sego-resume.bat` (Windows) and `scripts/sego-resume.sh` (Unix) wrap `sego --resume latest "$@"`. They auto-detect the sego binary (local cargo build > `CARGO_TARGET_DIR` > `PATH`).

---

## Developer Mode

### Enabling

```bash
# Option 1: Environment variable
export SEGO_DEV_MODE=1
sego

# Option 2: Config file (created by default in .sego/dev.toml)
# Already present — just build and run
```

### What Developer Mode Provides

| Feature | Description |
|---------|-------------|
| **Verbose logging** | Tool I/O tracing, lane event streaming to stderr |
| **Skip permissions** | All tool calls auto-approved (no interactive prompts) |
| **Policy trace** | See every policy engine decision |
| **Diagnostics** | Pre-turn diagnostics, config merge validation, orphan check |
| **Agent hints** | Focus modules, test commands, known failures surfaced |

### Configuration

See `.sego/dev.toml` for all settings. Key sections:

- `[dev.logging]` — Trace levels
- `[dev.diagnostics]` — Startup checks
- `[dev.tools]` — Permission/Prompt overrides
- `[dev.rust]` — Build profiles, extra lints
- `[dev.python]` — Parity audit settings
- `[dev.agents]` — AI agent context hints

---

## Build & Test

### Fast Iteration Loop

```bash
# Check formatting (fast, no build)
cargo fmt --all --check

# Check clippy (medium, builds but no tests)
cargo clippy --workspace --all-targets -- -D warnings

# Run just the CLI tests (fast, ~200 tests)
cargo test -p rusty-claude-cli

# Run all workspace tests
cargo test --workspace
```

### Build Matrix

```bash
# Development build (fast, debug symbols)
cargo build

# Release build (optimized)
cargo build --release

# Build specific crate
cargo build -p runtime
cargo build -p rusty-claude-cli
```

### Test Categories

```bash
# Unit tests (default)
cargo test --workspace

# Doc tests
cargo test --workspace --doc

# Integration tests only
cargo test --workspace --test '*'

# Specific test
cargo test -p rusty-claude-cli -- test_name

# Run tests single-threaded (avoids flaky failures)
cargo test --workspace -- --test-threads=1

# With nextest (faster, better output)
cargo nextest run --workspace
```

### Python Port Tests

```bash
# From project root
cd Sego-Agent
python -m pytest tests/ -v

# Run parity audit
python -c "from src.parity_audit import run_parity_audit; print(run_parity_audit().to_markdown())"
```

---

## Code Quality

### Pre-Commit Checklist

Before pushing, run:

```bash
# 1. Format
cargo fmt --all --check

# 2. Lint (deny warnings)
cargo clippy --workspace --all-targets -- -D warnings

# 3. Test workspace
cargo test --workspace

# 4. (Optional) Extra clippy lints
cargo clippy --workspace --all-targets -- -D warnings -W clippy::cargo -W clippy::nursery
```

### Rust Style

- **Indent:** 4 spaces (see `rustfmt.toml`)
- **Line width:** 100 chars
- **Imports:** Grouped (std → external → crate), granular by crate
- **Unsafe:** Forbidden (`unsafe_code = "forbid"`)
- **Clippy:** All + pedantic, with `module_name_repetitions`, `missing_panics_doc`, `missing_errors_doc` allowed

### Common Clippy Annotations

The CLI `main.rs` uses these at the crate level for backwards compatibility:
```rust
#![allow(dead_code, unused_imports, unused_variables)]
```

New code should NOT add to these — fix the warnings instead.

### Large File Warning

`rust/crates/rusty-claude-cli/src/main.rs` is 8,384 lines. This is known and accepted for the CLI entrypoint, but new functionality should extract to modules within the appropriate crate.

---

## Making Changes

### Which file to edit?

| You want to... | Edit this |
|---------------|-----------|
| Add a new slash command | `rust/crates/commands/src/lib.rs` |
| Add a new built-in tool | `rust/crates/tools/src/lib.rs` |
| Change session persistence format | `rust/crates/runtime/src/session.rs` |
| Add a recovery recipe | `rust/crates/runtime/src/recovery_recipes.rs` |
| Add a failure class | `rust/crates/runtime/src/lane_events.rs` |
| Change how the CLI parses args | `rust/crates/rusty-claude-cli/src/main.rs` → `parse_args()` |
| Change terminal rendering | `rust/crates/rusty-claude-cli/src/render.rs` |
| Add API provider | `rust/crates/api/src/providers/` |
| Change system prompt | `rust/crates/runtime/src/prompt.rs` |
| Add MCP transport | `rust/crates/runtime/src/mcp_stdio.rs` |
| Change policy engine rules | `rust/crates/runtime/src/policy_engine.rs` |
| Change config format | `rust/crates/runtime/src/config.rs` |

### Architecture Principles

1. **State machine first** — Every worker has explicit lifecycle states
2. **Events over scraped prose** — Channel output from typed events
3. **Recovery before escalation** — Auto-heal once before asking for help
4. **Branch freshness before blame** — Detect stale branches before red tests
5. **Partial success is first-class** — MCP servers can fail individually
6. **Terminal is transport, not truth** — TUI is rendering, state is above it
7. **Policy is executable** — Merge/retry/rebase rules are machine-enforced

### Adding a New Tool

1. Define tool in `GlobalToolRegistry::builtin()` in `rust/crates/tools/src/lib.rs`
2. Add tool execution logic
3. Add `ToolSearch` support
4. Add permission spec in registry
5. Add display formatting in `main.rs`
6. Add tests

### Adding a New Slash Command

1. Add variant to `SlashCommand` enum in `rust/crates/commands/src/lib.rs`
2. Add parsing logic
3. Add handler in `main.rs` → `handle_repl_command()`
4. Add resume support if applicable
5. Add help text
6. Add completion candidates

---

## Testing Strategy

### Test Layers

| Layer | Location | What it tests |
|-------|----------|--------------|
| **Unit** | Each crate `src/` | Individual functions/types |
| **Integration** | `rust/crates/runtime/tests/` | Cross-module workflows |
| **CLI** | `main.rs` `#[cfg(test)]` block | Arg parsing, formatting, commands |
| **Mock parity** | `mock_parity_harness.rs` | CLI parity scenarios |
| **Python port** | `tests/test_porting_workspace.py` | Python port surface |

### Test Patterns

- Use `#[test]` in the same file for unit tests
- Use `tests/` directory for integration tests
- Use `temp_dir()` helper for filesystem tests
- Use `env_lock()` for environment variable tests
- Use `cwd_lock()` for current-directory-sensitive tests
- Use `with_current_dir()` to temporarily change directory

### Known Flaky Tests

- `manager_discovery_report_keeps_healthy_servers_when_one_server_fails` — intermittent MCP timing
- Some MCP integration tests need `python3` on PATH

### Running Tests That Need Dummy API Keys

```bash
# Set a dummy key for tests that check auth initialization
ANTHROPIC_API_KEY="test-dummy-key-for-tests" cargo test --workspace
```

---

## CI/CD Pipeline

### GitHub Actions

| Workflow | Trigger | What it does |
|----------|---------|-------------|
| `rust-ci.yml` | Push/PR to main, `gaebal/**`, `omx-issue-*` | fmt → clippy → test |
| `release.yml` | Tag push | Build binaries → GitHub Release |

### CI Commands (equivalent to local)

```bash
# fmt job
cargo fmt --all --check

# clippy job
cargo clippy --workspace --all-targets -- -D warnings

# test job
cargo test --workspace
```

---

## Debugging

### Enable Extra Output

```bash
# Show full tool output
SEGO_DEV_MODE=1 sego

# Show SSE stream events
RUST_LOG=api=debug sego

# Debug specific crate
RUST_LOG=runtime=debug,api=trace sego
```

### Common Issues

| Symptom | Likely Cause | Fix |
|---------|-------------|-----|
| `cargo test` hangs | MCP test waiting for subprocess | `cargo test -- --test-threads=1` |
| Auth errors in tests | Missing dummy API key | `export ANTHROPIC_API_KEY=test-key` |
| Clippy "too many lines" | Function too large | Extract helper functions |
| Compile warnings on first build | Legacy allow-lints | Expected; fix incrementally |
| `Auth unavailable` on `sego doctor` | No API key set | Expected; `doctor` is local-only |

### Profiling

```bash
# Build timings
cargo build --timings

# Test with timing
cargo test --workspace -- -Z unstable-options --report-time

# Benchmark (if available)
cargo bench
```

---

## Contribution Workflow

### 1. Pick an Issue

Check `ROADMAP.md` for current priorities:
- **P0** — CI reliability, must-fix
- **P1** — Integration wiring
- **P2** — Clawability hardening
- **P3** — Swarm efficiency

### 2. Create a Branch

```bash
git checkout -b dev/my-feature
```

### 3. Make Changes

Follow the [Making Changes](#making-changes) section above.

### 4. Verify

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

### 5. Commit

```bash
git commit -m "feat(runtime): add description of change"
```

Use conventional commit prefixes:
- `feat(crate):` — new feature
- `fix(crate):` — bug fix
- `refactor(crate):` — code restructuring
- `docs:` — documentation
- `test:` — test additions/changes
- `chore:` — CI, build, config

### 6. Open a PR

PR description should include:
1. What changed and why
2. Which crates are affected
3. Test results
4. Any breaking changes

### PR Review Checklist

- [ ] All tests pass (`cargo test --workspace`)
- [ ] No clippy warnings (`cargo clippy --workspace --all-targets -- -D warnings`)
- [ ] Formatting correct (`cargo fmt --all --check`)
- [ ] New code has tests
- [ ] Documentation updated (USAGE.md, AGENTS.md, etc.)
- [ ] CHANGELOG.md updated for user-facing changes
- [ ] No new `#![allow(...)]` annotations without justification

---

## Architecture Decision Records

### Why `main.rs` is 8,384 lines

The CLI entrypoint (`rusty-claude-cli/src/main.rs`) consolidates all CLI logic — arg parsing, tool execution, formatting, session management, plugin/MCP lifecycle — into one file. This was inherited from the initial port. **The long-term plan** is to extract into modules as the code stabilizes.

### Why both Rust and Python

- **Rust:** The active, production implementation. Single binary, <50ms startup.
- **Python:** The parity reference port. Validates the Rust implementation's completeness. Not compiled into the binary.

### Why JSONL for sessions

Sessions are stored as JSONL (one JSON object per line) for:
- Append-only writes (no rewriting the whole file)
- Streaming reads
- Crash resilience
- Human readability

### Why the `Claw` → `Sego` renaming is incomplete

The codebase was originally forked from `claw-code`. The renaming to `sego` is in progress. Some internal identifiers still reference `claw` — this is purely cosmetic and will be completed over time.

---

## Quick Reference

```bash
# Development cycle
cargo build                                 # Build
cargo test -p rusty-claude-cli              # Fast tests
cargo clippy --workspace --all-targets      # Lint
cargo fmt --all --check                     # Format check
cargo test --workspace                      # Full test suite

# Run sego from source
cargo run -- "summarize rust/Cargo.toml"

# Run with developer mode
SEGO_DEV_MODE=1 cargo run

# Publish workflow
cargo build --release                       # Build release
cargo test --workspace                      # Full test suite
cargo fmt --all --check                     # Format
cargo clippy --workspace --all-targets -- -D warnings  # Lint
git push                                    # Push to GitHub

# Python port
cd Sego-Agent
python -m pytest tests/ -v
```

### .sego Schema Formalization (c9/b)

The `schema/` directory contains hand-written JSON Schema files that serve as
the public contract for `.sego` artifacts. Aligned with Codex decision D-B-1~4:

- Schemas are hand-written (no `schemars`/`jsonschema` runtime dependency).
- Rust production path uses `serde` deserialization + golden fixture round-trip
  tests; full JSON Schema validation is a future dev-dependency if needed.
- `review-artifact.schema.json` aligns with `ReviewArtifact` (the real on-disk
  struct), which already has `schema_version: u32` (currently hardcoded to 1).
- `review-index-entry.schema.json` has no `schema_version` (append-only, stable).
- `sidecar-request-response.schema.json` defines a minimal stable envelope;
  Cycle 9 C refines action-specific fields during PoC.
- When modifying a struct with a schema, update the schema file and the golden
  fixture test together.

