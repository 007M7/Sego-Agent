# AGENTS.md — Sego Agent Developer Instructions

This file provides comprehensive guidance to AI coding agents working in this repository. **Read this first before making any changes.**

---

## Repository Identity

- **Project:** Sego Agent — The AI coding agent that learns from every run
- **Language:** Rust (~50K LOC, 9-crate workspace)
- **Python:** Reference parity port (validation only, not compiled)
- **License:** MIT
- **Repo:** https://github.com/007M7/Sego-Agent

---

## Detected Stack

- **Languages:** Rust (primary), Python (reference port)
- **Build:** Cargo workspace with 9 crates
- **CI:** GitHub Actions (rust-ci.yml + release.yml)
- **Config:** TOML-based `.sego/dev.toml` for developer settings
- **Test harness:** Built-in `#[cfg(test)]` + `tests/` directories + mock parity harness
- **Frameworks:** None — pure Rust with tokio async runtime

---

## Quick Verification

```bash
# Always run these before committing
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Fast iteration (CLI tests only, ~200 tests)
cargo test -p rusty-claude-cli

# Python port tests
python -m pytest tests/ -v
```

---

## Repository Shape (Crate Map)

```
rust/crates/
├── api/                    # Anthropic-compatible client + SSE streaming
│   └── src/
│       ├── client.rs       # HTTP client, auth, retry
│       ├── providers/      # Model provider adapters (anthropic, openai_compat)
│       ├── sse.rs          # Server-Sent Events parser
│       ├── types.rs        # Request/response types
│       └── prompt_cache.rs # Prompt caching support
│
├── runtime/                # 🧠 Core runtime loop — THE most important crate
│   └── src/
│       ├── lib.rs          # Public API surface — re-exports everything
│       ├── conversation.rs # Turn-based conversation loop
│       ├── session.rs      # Session state + persistence (JSONL)
│       ├── permissions.rs  # Permission mode enforcement
│       ├── config.rs       # 5-layer config hierarchy
│       ├── prompt.rs       # System prompt builder
│       ├── compact.rs      # Context compaction
│       ├── sandbox.rs      # Sandbox detection
│       │
│       ├── lane_events.rs       # 16 structured event types
│       ├── recovery_recipes.rs  # 7 automatic recovery patterns
│       ├── policy_engine.rs     # Executable autonomous rules
│       ├── green_contract.rs    # 4-level quality gates
│       ├── stale_branch.rs      # Branch freshness detection
│       ├── worker_boot.rs       # Agent lifecycle (Spawning→TrustRequired→ReadyForPrompt→...)
│       ├── summary_compression.rs # Collapses events into checkpoints
│       ├── task_packet.rs       # Structured task format
│       │
│       ├── mcp.rs               # MCP config loading
│       ├── mcp_client.rs        # MCP client protocol
│       ├── mcp_stdio.rs         # MCP stdio transport
│       ├── mcp_lifecycle_hardened.rs
│       ├── mcp_tool_bridge.rs
│       │
│       ├── hooks.rs             # Pre/post tool hooks
│       ├── plugin_lifecycle.rs  # Plugin lifecycle management
│       ├── bash.rs              # Bash tool executor
│       ├── bash_validation.rs   # Command validation
│       ├── oauth.rs             # OAuth flow
│       ├── lsp_client.rs        # LSP integration
│       ├── trust_resolver.rs    # Trust gate resolver
│       ├── session_control.rs   # Session control API
│       ├── community_learning.rs # Anonymous telemetry
│       ├── task_registry.rs     # Background task registry
│       ├── team_cron_registry.rs
│       │
│       └── workflow/            # Session analysis & trends
│           ├── mod.rs
│           ├── report.rs
│           └── store.rs
│
├── commands/               # 140+ slash commands (Help, Status, Compact, Model, ...)
│   └── src/lib.rs
│
├── tools/                  # 40+ built-in tools (bash, read, write, edit, glob, grep, ...)
│   └── src/
│       ├── lib.rs          # GlobalToolRegistry
│       └── lane_completion.rs
│
├── rusty-claude-cli/       # 🖥️ CLI entrypoint — 8,384-line main.rs
│   └── src/
│       ├── main.rs         # ALL CLI logic (args, rendering, execution, session mgmt)
│       ├── app.rs          # Legacy (deprecated, kept for test references)
│       ├── args.rs         # Legacy (deprecated)
│       ├── input.rs        # Line editor for REPL
│       ├── init.rs         # Repo init (CLAUDE.md template)
│       └── render.rs       # Terminal rendering (ANSI/Markdown)
│
├── plugins/                # Plugin registry + hook system
│   └── src/
│       ├── lib.rs
│       └── hooks.rs
│
├── telemetry/              # Session tracing + usage analytics
│   └── src/lib.rs
│
├── mock-anthropic-service/ # Deterministic test harness
│   └── src/
│
└── compat-harness/         # Protocol compatibility layer
    └── src/lib.rs

src/                         # Python parity port (reference only)
├── main.py                  # Session recording tool
├── runtime.py               # PortRuntime with session bootstrap
├── commands.py / tools.py   # Command/tool registry mirrors
├── models.py                # Shared data types
├── parity_audit.py          # Port parity checker
├── port_manifest.py         # Workspace manifest
└── reference_data/          # JSON snapshots of archive surfaces

tests/                       # Python port tests
└── test_porting_workspace.py
```

---

## Which File to Edit For What

| Task | Edit This |
|------|-----------|
| Add a new slash command | `rust/crates/commands/src/lib.rs` |
| Add a new built-in tool | `rust/crates/tools/src/lib.rs` |
| Change session persistence | `rust/crates/runtime/src/session.rs` |
| Change crash/session resume recovery state | `rust/crates/runtime/src/recovery.rs` |
| Add a recovery recipe | `rust/crates/runtime/src/recovery_recipes.rs` |
| Add a failure class | `rust/crates/runtime/src/lane_events.rs` |
| Change CLI arg parsing | `rust/crates/rusty-claude-cli/src/main.rs` → `parse_args()` |
| Change terminal rendering | `rust/crates/rusty-claude-cli/src/render.rs` |
| Add API provider | `rust/crates/api/src/providers/` |
| Change system prompt | `rust/crates/runtime/src/prompt.rs` |
| Add MCP transport | `rust/crates/runtime/src/mcp_stdio.rs` |
| Change policy rules | `rust/crates/runtime/src/policy_engine.rs` |
| Change config format | `rust/crates/runtime/src/config.rs` |
| Add new CLI subcommand | `main.rs` → `CliAction` enum + `parse_args()` |
| Change tool permissions | `tools/lib.rs` → permission specs |
| Fix test assertion | `main.rs` `#[cfg(test)] mod tests` block |
| Update AGENTS.md for AI agents | `AGENTS.md` ← you are here |
| Update developer documentation | `DEVELOPMENT.md` |
| Update user-facing docs | `README.md`, `USAGE.md` |
| Update roadmap | `ROADMAP.md` |

---

## Development Workflow (Code Change Checklist)

Before pushing any change:

1. **Format check**: `cargo fmt --all --check`
2. **Lint (deny warnings)**: `cargo clippy --workspace --all-targets -- -D warnings`
3. **Fast tests**: `cargo test -p rusty-claude-cli`
4. **Full workspace tests**: `cargo test --workspace`
5. **Update docs** if behavior changed: `AGENTS.md`, `DEVELOPMENT.md`, `README.md`, `USAGE.md`
6. **Update CHANGELOG.md** if user-facing
7. **Check git status**: no unintended files
8. **Commit** with conventional prefix: `feat(crate):`, `fix(crate):`, `refactor(crate):`, `docs:`, `test:`, `chore:`

---

## Architecture Principles (Must Follow)

1. **State machine first** — workers have explicit lifecycle states
2. **Events over scraped prose** — output from typed events, not raw text
3. **Recovery before escalation** — auto-heal once before asking for help
4. **Branch freshness before blame** — detect stale branches before red tests
5. **Partial success is first-class** — MCP servers can fail individually
6. **Terminal is transport, not truth** — TUI is rendering, state is above it
7. **Policy is executable** — merge/retry/rebase rules are machine-enforced
8. **Recovery paths are absolute** — crash/session resume records live under `.sego/recovery/` and must store absolute session paths, even while session JSONL remains under `.claw/sessions/`

### Recovery State Boundary

- `rust/crates/runtime/src/recovery.rs` owns crash/session resume state, including `.sego/recovery/latest-session.json`, `.sego/recovery/exit-state.json`, and `.sego/recovery/recovery-summary.md`.
- `rust/crates/runtime/src/recovery_recipes.rs` owns automatic remediation recipes after tool/task failures. Do not use it for session crash recovery.
- Recovery startup detection must only read recovery JSON and check the referenced session file. It must not scan the repo, call a model, auto-resume, or replay old tool calls.
- `session_path` in recovery JSON must be absolute. This prevents `.claw` / `.sego` dual-directory references from breaking on Windows paths or future storage migration.
- **A2 CLI integration**: recovery startup notice is triggered only for actions that create/restore a persistent session and may execute model/tools (`Repl` / `Prompt` / `CodeReview` / `ResumeSession`). Pure-query actions (`Version`, `Help`, `Status`, etc.) must not trigger it.
- **A2 exit-state write timing**: `active` is written only after the session handle is known (`LiveCli::new` for Repl/Prompt/CodeReview; session load for ResumeSession). `graceful` is written only on the normal return path. Error paths (crash, Ctrl+C, `?` propagation) leave `active` in place so the next launch prompts recovery. Do not use a `Drop` guard for `graceful` (fallible IO in `Drop` is unclear and risks writing `graceful` on error paths).
- **A2 no signal handler (MVP)**: A2 deliberately does not add a signal handler. Interrupted/crashed sessions keep the `active` state and are surfaced as recoverable on next launch. Precise `interrupted` vs `crashed` distinction is deferred to a future `ctrlc` crate evaluation.
- **A2 `/recovery-export`**: a separate slash command from `/export`. `/export` writes the session transcript (forced `.txt`); `/recovery-export` writes the recovery summary (markdown, default `.sego/recovery/recovery-summary.md`). Both share the `write_recovery_export` helper pattern but must not share path resolution logic.

---

## Rust Style Rules

- **Indent:** 4 spaces (see `rustfmt.toml`)
- **Line width:** 100 chars
- **Imports:** Grouped (std → external → crate), granular by crate
- **Unsafe:** Forbidden (`unsafe_code = "forbid"`)
- **Clippy:** All + pedantic warnings enabled
- **New code:** Must not add `#[allow(...)]` without explicit justification
- **Tests:** `#[cfg(test)]` in the same file for unit tests; `tests/` for integration tests

---

## Key Patterns & Conventions

### Test Helpers
```rust
fn temp_dir() -> PathBuf { /* creates unique temp directory */ }
fn env_lock() -> MutexGuard<Mutex<()>> { /* serializes env var tests */ }
fn cwd_lock() -> MutexGuard<Mutex<()>> { /* serializes cwd-sensitive tests */ }
fn with_current_dir(path, closure) { /* runs closure in temp cwd */ }
fn git(args, cwd) { /* runs git command, asserts success */ }
```

### Session Persistence
- Sessions stored as JSONL (`.claw/sessions/<id>.jsonl`)
- Append-only writes for crash resilience
- `latest` alias resolves to most recent session
- `Session::new()` → `save_to_path()` → `load_from_path()`

### Tool Execution Flow
1. CLI parses input → identifies tool call
2. `CliToolExecutor::execute()` → dispatches to tool registry
3. Registry maps to implementation → returns JSON string
4. Result formatted with ANSI colors via `format_tool_result()`
5. Output saved in session JSONL

### Config Hierarchy (5 layers)
1. `~/.sego.json` — User-level
2. `~/.config/sego/settings.json` — User config directory
3. `<repo>/.sego.json` — Project-level
4. `<repo>/.sego/settings.json` — Project settings
5. `<repo>/.sego/settings.local.json` — Local overrides (gitignored)

---

## Common Pitfalls & Fixes

| Symptom | Likely Cause | Fix |
|---------|-------------|-----|
| `cargo test` hangs | MCP test waiting for subprocess | `cargo test -- --test-threads=1` |
| Auth errors in tests | Missing dummy API key | `ANTHROPIC_API_KEY=test-key cargo test` |
| Clippy "too many lines" | Function > 100 lines | Extract helper functions |
| `Auth unavailable` on CLI | No API key set | Set `ANTHROPIC_API_KEY` env var |
| `unsupported tool in --allowedTools` | Tool name mismatch | Use exact canonical names |
| Test failures after rebase | Stale branch | `git merge main` first |
| Python test failures | Archive snapshot mismatch | Run `parity_audit.py` to diagnose |

---

## Known Issues (Don't Fix These Yet)

- `main.rs` is 8,384 lines — known, accepted, planned extraction later
- `manager_discovery_report_keeps_healthy_servers_when_one_server_fails` — flaky MCP timing, temporarily ignored
- Internal identifiers still reference `claw` — cosmetic renaming in progress
- `session_control.rs` and `trust_resolver.rs` are exported but not wired — see ROADMAP.md item #17

---

## Agent-Specific Notes

- **When asked to add a feature:** Check `DEVELOPMENT.md` → "Making Changes" for which file to edit
- **When running tests:** Use `cargo test -p rusty-claude-cli` for fast feedback; `cargo test --workspace` before commit
- **When editing `main.rs`:** Be careful — it's 8,384 lines with deeply nested logic. Prefer extracting new functions.
- **When changing session format:** Must maintain backward compatibility with existing JSONL files
- **When adding tools:** Must also update `ToolSearch` and permission specs
- **When changing errors:** Use typed errors (not strings) for machine-readable failure classification
- **When committing:** Use conventional commit prefixes
- **Developer mode:** Enable with `SEGO_DEV_MODE=1` to get extra diagnostics and skip permission prompts
- **Config:** See `.sego/dev.toml` for developer-specific settings like focus modules, test commands, and known failures

---

## Documentation Files

| File | Audience |
|------|----------|
| `AGENTS.md` | AI coding agents ← you are reading this |
| `DEVELOPMENT.md` | Human contributors |
| `README.md` | End users |
| `USAGE.md` | End users (detailed CLI) |
| `PHILOSOPHY.md` | Architecture enthusiasts |
| `ROADMAP.md` | Contributors picking tasks |
| `PARITY.md` | Parity port maintainers |
| `CONTRIBUTING.md` | First-time contributors |
| `CHANGELOG.md` | Release notes |

## Working Agreement

- Prefer small, reviewable changes
- Update `AGENTS.md` and `DEVELOPMENT.md` when workflows change
- Keep shared defaults in `.sego.json`; reserve `.sego/settings.local.json` for machine-local overrides
- Do not overwrite existing `AGENTS.md` content automatically; update it intentionally when repo workflows change
- Always run `cargo test --workspace` before pushing
- When in doubt, read `DEVELOPMENT.md` first

## .sego Schema (c9/b)

JSON Schema files in `schema/` are the **public contract** between the Rust
engine and future TS sidecar / Python consumers. They are version-controlled
and safe for GitHub.

- `schema/review-artifact.schema.json` — aligned with `ReviewArtifact` (the
  real on-disk structure in `.sego/reviews/{id}.json`, NOT `PersistedReviewArtifact`
  which is just a path handle). Contains `schema_version`, `findings`, etc.
- `schema/review-index-entry.schema.json` — aligned with `ReviewIndexEntry`
  and `ReviewFindingStatusEntry`. Append-only index, no `schema_version`.
- `schema/sidecar-request-response.schema.json` — minimal envelope for
  sidecar <-> engine stdin/stdout JSON. Refined in Cycle 9 C.

When changing any struct that has a schema file, **update the schema file too**.
The golden fixture tests in `code_review/report.rs` verify serde round-trip
consistency but do not validate against the JSON Schema file automatically.

