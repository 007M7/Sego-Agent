# 🦞 sego

<p align="center">
  <strong>The AI coding agent that learns from every run.</strong><br>
  Open-source · Rust-native · Model-agnostic · Self-evolving
</p>

<p align="center">
  <a href="#quick-start"><img src="https://img.shields.io/badge/quick_start-5_min-blue?style=flat-square" alt="Quick start in 5 minutes"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="MIT License"></a>
  <img src="https://img.shields.io/badge/rust-1.80+-orange?style=flat-square" alt="Rust 1.80+">
  <img src="https://img.shields.io/badge/platform-linux_|_macos_|_windows-lightgrey?style=flat-square" alt="Platforms">
</p>

---

## Why sego exists

There are hundreds of AI coding tools. Every single one follows the same pattern:

```
Input task → Execute → Output result → End (start from scratch next time)
```

**sego is different.** It runs the same loop, but adds something none of them have:

```
Input task → Execute → Record lane events → Output result
                          ↓
              Diagnose failures (11 types)
                          ↓
              Match recovery recipe (7 built-in)
                          ↓
              Auto-heal → Policy engine learns → Better next time
```

**Three things sego gives you that no other tool does:**

1. **Record** — Every session is tracked as structured Lane Events. You know exactly what happened, what failed, and why.
2. **Recover** — 11 failure types auto-classified. 7 built-in recovery recipes. sego fixes itself before asking for help.
3. **Evolve** — Policy engine learns from history. Green Contract enforces quality. Your agent gets smarter every run.

> **You don't just use sego. sego learns how you work.**

---

## Quick Start

### Download & run (Windows)

```powershell
irm https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.ps1 | iex
set ANTHROPIC_API_KEY=sk-your-key
set ANTHROPIC_BASE_URL=https://api.deepseek.com/anthropic
sego
```

### Download & run (Linux/macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.sh | bash
export ANTHROPIC_API_KEY="sk-your-key"
export ANTHROPIC_BASE_URL="https://api.deepseek.com/anthropic"
sego
```

### Or build from source

```bash
git clone https://github.com/007M7/Sego-Agent.git
cd Sego-Agent/rust && cargo build --release
./target/release/sego
```

That's it. **One binary. No Node.js. No Python. No Docker.**

---

## The Self-Learning Workflow (built-in, always on)

You don't enable it. You don't configure it. **It just works.**

### What happens every time you run sego

```
🦞 sego "refactor the auth module and add unit tests"

  lane.started ────────────────────────────────────────── 14:30:00
  lane.ready ──────────────────────────────────────────── 14:30:02
  ├─ Reading src/auth/*.rs
  ├─ Writing refactored modules
  ├─ Running cargo test
  │   └─ 2 tests failed → lane.red
  │       ├─ FailureClass::Test auto-detected
  │       └─ RecoveryRecipe: "rerun with --verbose"
  ├─ Rerun tests → all pass → lane.green ✓
  ├─ Commit created → lane.commit.created
  └─ Workflow complete → lane.finished ───────────────── 14:52:30

  📊 Session Report:
  Duration: 22min | Events: 8 | Recoveries: 1 | Green Level: Package
```

### What you get from this

- **Session replay:** `sego status` — see exactly what happened
- **Failure forensics:** Every error is classified into 1 of 11 types. No more "something broke."
- **Auto-recovery:** 7 known failure patterns heal themselves. You don't even notice most problems.
- **Quality gates:** Green Contract (Targeted → Package → Workspace → MergeReady) prevents broken code from shipping.
- **Learning data:** Policy engine adapts based on what works and what doesn't.

---

## Features

### AI Coding Engine

| Category | Tools |
|----------|-------|
| **File operations** | Read, Write, Edit (structured patches), Glob search, Grep search |
| **Shell** | Bash execution with sandbox, timeout, and permission control |
| **Web** | WebSearch, WebFetch |
| **Orchestration** | Sub-agents (Agent), Background tasks (Task), Cron jobs, Team registry |
| **Editor** | NotebookEdit (Jupyter) |
| **Workflow** | TodoWrite, Plan mode (EnterPlanMode / ExitPlanMode) |
| **External** | MCP servers (full lifecycle), LSP client (diagnostics, hover, references) |

### Self-Learning System

| Module | What it does |
|--------|-------------|
| **Lane Events** | 16 structured event types track every step: `started → ready → running → green/red → commit → finished` |
| **Failure Taxonomy** | 11 failure classes auto-detected: PromptDelivery, TrustGate, BranchDivergence, Compile, Test, PluginStartup, McpStartup, McpHandshake, GatewayRouting, ToolRuntime, Infra |
| **Recovery Recipes** | 7 automatic recovery steps: trust resolution, prompt redirection, branch rebase, clean build, MCP handshake retry, plugin restart, worker restart |
| **Policy Engine** | Executable rules: "if green + scoped diff + review passed → merge", "if stale branch → rebase before tests" |
| **Green Contract** | 4 graduated quality levels enforced before merge |
| **Session Persistence** | Full conversation history saved as structured JSONL, resumable any time |

### CLI Surface

```bash
sego                          # Interactive REPL
sego "your task"              # One-shot prompt
sego --model sonnet "task"    # Pick your model
sego --resume latest          # Continue last session
sego status                   # Workspace snapshot
sego doctor                   # System diagnostics
sego --output-format json status  # Machine-readable output
```

### Model Support

| Alias | Resolves to |
|-------|------------|
| `opus` | `claude-opus-4-7` |
| `sonnet` | `claude-sonnet-4-6` |
| `haiku` | `claude-haiku-4-5` |

**Any Anthropic-compatible API works.** Use a proxy and sego works with DeepSeek, Qwen, GLM, and other domestic LLMs.

---

## Architecture

```
rust/
├── crates/
│   ├── api/                    # Anthropic-compatible client + SSE streaming
│   ├── commands/               # Slash command registry (/help, /status, /cost...)
│   ├── runtime/                # Core loop, sessions, permissions, MCP, prompts
│   │   ├── lane_events.rs      #   ─┐
│   │   ├── recovery_recipes.rs #    ├─ Self-learning system
│   │   ├── policy_engine.rs    #    │
│   │   ├── green_contract.rs   #   ─┘
│   │   ├── worker_boot.rs      # Agent lifecycle state machine
│   │   ├── stale_branch.rs     # Branch freshness detection
│   │   └── summary_compression.rs  # Smart context compaction
│   ├── tools/                  # 40+ built-in tool implementations
│   ├── rusty-claude-cli/       # CLI entrypoint (REPL, one-shot, JSON output)
│   ├── plugins/                # Plugin registry + hook system
│   ├── telemetry/              # Session tracing + usage analytics
│   ├── mock-anthropic-service/ # Deterministic test harness
│   └── compat-harness/         # Protocol compatibility layer
```

**~50,000 lines of Rust. 9 crates. One binary: `sego`.**

---

## Design Philosophy

```
Competitors = Input → Execute → Output → Forget
Sego        = Input → Execute → Record → Diagnose → Recover → Learn → Output
```

**Core principles:**

1. **Every session is learning data** — not disposable, but an asset that compounds
2. **Failure is not the end** — it's experience for the next automatic recovery
3. **Quality is enforced, not trusted** — Green Contract gates prevent broken code from shipping
4. **Models don't lock you in** — any Anthropic-compatible API works. DeepSeek, Qwen, GLM, your choice.

---

## Full Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                       sego CLI (rusty-claude-cli)               │
│  ┌─────────┐ ┌──────┐ ┌────────┐ ┌────────┐ ┌──────────────┐  │
│  │  REPL   │ │Prompt│ │ review │ │ learn  │ │ --resume     │  │
│  │ (对话)  │ │(单次)│ │(复盘)  │ │(学习)  │ │ (恢复会话)   │  │
│  └────┬────┘ └──┬───┘ └───┬────┘ └───┬────┘ └──────┬───────┘  │
│       └─────────┼─────────┼─────────┼──────────────┘          │
├─────────────────┼─────────┼─────────┼──────────────────────────┤
│            ConversationRuntime (会话引擎)                        │
│                                                                 │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │                   API Layer (api crate)                   │  │
│  │  ┌──────────────┐ ┌──────────────┐ ┌──────────────────┐  │  │
│  │  │  Anthropic   │ │  OpenAI      │ │  DeepSeek Adapt  │  │  │
│  │  │  Client      │ │  Compat      │ │  (convert_msg)   │  │  │
│  │  └──────────────┘ └──────────────┘ └──────────────────┘  │  │
│  └──────────────────────────────────────────────────────────┘  │
│                                                                 │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │                  Tools Layer (tools crate)                │  │
│  │  Bash │ Read │ Write │ Edit │ Glob │ Grep │ Web │ Agent │  │
│  │  Task │ Cron │ Team  │ MCP  │ LSP  │ Skill│Todo │Plan   │  │
│  └──────────────────────────────────────────────────────────┘  │
│                                                                 │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │              Self-Learning System (runtime/workflow)       │  │
│  │                                                          │  │
│  │  ┌──────────┐   ┌──────────────┐   ┌───────────────┐    │  │
│  │  │  Lane    │──→│  Failure     │──→│  Recovery     │    │  │
│  │  │  Events  │   │  Taxonomy    │   │  Recipes      │    │  │
│  │  │ (16 种)  │   │  (11 类)     │   │  (7 配方)     │    │  │
│  │  └──────────┘   └──────────────┘   └───────┬───────┘    │  │
│  │                                            │            │  │
│  │                    ┌───────────────────────┘            │  │
│  │                    ↓                                    │  │
│  │  ┌──────────┐   ┌──────────────┐   ┌───────────────┐   │  │
│  │  │  Workflow│──→│  Session     │──→│  Trend        │   │  │
│  │  │  Store   │   │  Report      │   │  Analyzer     │   │  │
│  │  │ (持久化) │   │  (分析)      │   │  (learn)      │   │  │
│  │  └──────────┘   └──────────────┘   └───────────────┘   │  │
│  └──────────────────────────────────────────────────────────┘  │
│                                                                 │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │               Quality Gates                               │  │
│  │  ┌──────────────┐ ┌───────────────┐ ┌────────────────┐   │  │
│  │  │ Green Contract│ │Stale Branch   │ │ Policy Engine  │   │  │
│  │  │ (4 级门控)   │ │ Detection     │ │ (自主规则)     │   │  │
│  │  └──────────────┘ └───────────────┘ └────────────────┘   │  │
│  └──────────────────────────────────────────────────────────┘  │
│                                                                 │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │               Infrastructure                             │  │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌────────────┐  │  │
│  │  │ Session  │ │ Config   │ │Permission│ │ Plugin     │  │  │
│  │  │ (JSONL)  │ │ (5-layer)│ │ Enforcer │ │ Registry   │  │  │
│  │  └──────────┘ └──────────┘ └──────────┘ └────────────┘  │  │
│  └──────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

---

## vs Hermes Agent

[Hermes](https://github.com/NousResearch/hermes-agent) (by Nous Research) is a leading open-source AI agent platform. Here's how Sego compares:

| | Sego | Hermes |
|---|------|--------|
| **Developer** | 007M7 (individual) | Nous Research (company) |
| **Language** | Rust (~50K LOC) | Python |
| **Startup** | <50ms, single binary | Requires WSL on Windows |
| **Positioning** | Coding agent engine | General-purpose agent platform |
| **Learning** | Lane Events → Recovery → Policy | Skills → Nudges → Honcho memory |
| **Learning style** | Automatic, passive, always-on | Active, user-corrected, skill-based |
| **Multi-platform** | CLI-focused | Telegram/Discord/Slack/WhatsApp/Signal |
| **Programming** | Deep toolchain (MCP/LSP/Agent/Cron) | General tools |
| **Quality gates** | Green Contract (4 levels) | Not built-in |
| **Failure recovery** | 11 types auto-classified, 7 recipes | Manual |
| **Model freedom** | Any Anthropic-compatible API | OpenRouter (200+), Nous Portal, etc. |
| **Windows native** | Yes | No (requires WSL2) |
| **Deployment** | Single binary | Docker/SSH/Modal/Daytona |
| **License** | MIT | MIT |

**Key difference:** Hermes is a general-purpose agent platform (chat, automate, research). Sego is a **coding-specific agent engine** where **every session automatically records, diagnoses failures, recovers, and improves** — a built-in learning loop that requires zero user configuration.

---

## Real-World Use Cases

### 1. Solo developer — daily coding

```bash
sego "add retry logic with exponential backoff to all API calls in src/client/"
```

sego reads every file, analyzes the existing pattern, applies retry logic, runs tests, fixes failures, commits. **What took 2 hours now takes 10 minutes.** And the workflow is recorded for future reference.

### 2. Domestic LLM deployment (DeepSeek / Qwen / GLM)

```bash
export ANTHROPIC_BASE_URL="https://your-deepseek-proxy.com/v1"
export ANTHROPIC_API_KEY="sk-deepseek-xxx"
sego "write a backtesting data loader in Python"
```

sego's recovery system compensates for model weaknesses — when the domestic model produces unstable output, sego detects failures and auto-heals. **No VPN. No overseas API. Full AI coding experience.**

### 3. Enterprise on-premise deployment

```
Corporate intranet → deploy sego + private model proxy
→ All code, API calls, session logs stay inside the firewall
→ Green Contract enforces Workspace-level test coverage on every PR
→ Lane Event logs satisfy compliance audit requirements
→ IT admin controls which models and tools are available
```

**AI-assisted coding that passes security review.** This is the reason banks, hospitals, and government agencies can't use Claude Code — sego solves this.

### 4. Team quality enforcement

```bash
# Tech lead sets Green Contract = MergeReady
# Every developer's sego agent runs full workspace tests before PR creation
# PRs that don't meet the contract can't be merged

sego "implement user registration API with full test coverage"
# → Green Contract: Workspace ✓
# → All 1,200+ tests pass
# → PR created, ready for review
```

No more "tests passed on my machine." **The contract enforces quality, not trust.**

### 5. CI/CD auto-remediation

```yaml
# .github/workflows/auto-fix.yml
- name: AI auto-fix CI failures
  run: |
    sego "analyze CI failure in job #${{ github.run_id }}, fix root cause, commit"
```

sego's failure taxonomy instantly classifies the error type, and recovery recipes attempt automatic fixes. **CI goes from hours of manual debugging to automated recovery.**

### 6. Education & assessment

```bash
# Instructor reviews a student's AI coding workflow
sego review --history --student alice

# Output:
# Alice: 47 sessions this semester
# Avg tests-before-commit rate: 12% (class avg: 68%)
# Most common failure: Compile errors (not checking before commit)
# Recommendation: Require Green Contract ≥ Package
```

Lane Event data provides objective evidence for grading in the AI era. **No more "I wrote it" — the data shows how.**

---

## vs Competitors

| | sego | Claude Code | Aider | Cursor CLI | koda |
|---|------|-------------|-------|------------|------|
| **Open source** | ✅ MIT | ❌ Closed | ✅ Apache 2.0 | ❌ Closed | ✅ |
| **Language** | Rust | TypeScript | Python | TypeScript | Rust |
| **Self-learning workflow** | ✅ Built-in | ❌ | ❌ | ❌ | ❌ |
| **Failure auto-recovery** | ✅ 7 recipes | ❌ | ❌ | ❌ | ❌ |
| **Quality gates** | ✅ Green Contract | ❌ | ❌ | ❌ | ❌ |
| **Model-agnostic** | ✅ Any API | ❌ Anthropic only | ✅ | ❌ | ✅ |
| **Multi-agent** | ✅ Built-in | Basic | ❌ | ❌ | ❌ |
| **MCP / LSP** | ✅ Full lifecycle | ✅ | ❌ | ✅ | ✅ |
| **On-premise deploy** | ✅ Single binary | ❌ Cloud | ✅ | ❌ | ✅ |
| **Startup time** | <50ms | ~2s (Node.js) | ~500ms | ~1.5s | <100ms |

sego is not trying to be "another Claude Code alternative." It's the first AI coding agent where **the workflow recording, failure recovery, and quality enforcement are built into the runtime itself** — not bolted on as an afterthought.

---

## Installation

### One-click (Windows PowerShell)

```powershell
irm https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.ps1 | iex
```

### One-click (Linux/macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.sh | bash
```

### Download binary directly

Download the latest `sego.exe` (Windows) or `sego-linux` from [GitHub Releases](https://github.com/007M7/Sego-Agent/releases).

### From source

```bash
git clone https://github.com/007M7/Sego-Agent.git
cd Sego-Agent/rust
cargo build --release
./target/release/sego --help
```

### Requirements

- **Windows/Linux/macOS** — single binary, no runtime dependencies
- **API key** — any Anthropic-compatible endpoint works
- Rust toolchain 1.80+ (source build only)

### Verification

```bash
sego doctor                      # System diagnostics
cargo test --workspace           # Full test suite (source build)
```

---

## Configuration

### API key

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
```

### Custom endpoint (proxy / domestic models)

```bash
# DeepSeek proxy example
export ANTHROPIC_BASE_URL="https://deepseek-proxy.example.com/v1"
export ANTHROPIC_API_KEY="sk-deepseek-xxx"

# Qwen proxy example
export ANTHROPIC_BASE_URL="https://dashscope.aliyuncs.com/compatible-mode/v1"
export ANTHROPIC_API_KEY="sk-qwen-xxx"
```

### Permission modes

```bash
sego --permission-mode read-only "summarize this repo"
sego --permission-mode workspace-write "refactor src/"
```

### OAuth

```bash
sego login    # Browser-based OAuth
sego logout   # Clear stored credentials
```

### Config file hierarchy

```
~/.sego.json                    # User-level
~/.config/sego/settings.json    # User config directory
<repo>/.sego.json               # Project-level
<repo>/.sego/settings.json      # Project settings
<repo>/.sego/settings.local.json  # Local overrides (gitignored)
```

### Community learning telemetry

Sego can anonymously share aggregate statistics to help improve the community model. **Off by default — you must opt in.**

```bash
sego telemetry              # Check status
sego telemetry on           # Enable anonymous sharing
sego telemetry off          # Disable
sego telemetry export       # View pending data before sending
```

**What is shared (only when enabled):**

| Shared (anonymous) | Never shared |
|--------------------|--------------|
| Random device UUID | Conversation content |
| Efficiency scores | Code or file paths |
| Failure type stats | API keys or tokens |
| Recovery success rates | Personal identifiers |
| Model family name | Anything identifying you |

All data helps improve recovery recipes, efficiency benchmarks, and API compatibility for the entire community.

---

## Documentation Map

| File | What's in it |
|------|-------------|
| `USAGE.md` | Build, auth, CLI, sessions, config, parity harness |
| `rust/README.md` | Crate map, CLI reference, features, workspace layout |
| `PHILOSOPHY.md` | Why sego exists, the system design philosophy |
| `ROADMAP.md` | Active roadmap and planned features |
| `PARITY.md` | Protocol parity status and migration notes |

---

## Roadmap

### Done
- [x] Anthropic API + SSE streaming
- [x] Interactive REPL + one-shot prompt
- [x] 40+ built-in tools (bash, file ops, search, web, agents, MCP, LSP)
- [x] Lane Events — 16 structured workflow event types
- [x] Failure Taxonomy — 11 failure classes auto-detected
- [x] Recovery Recipes — 7 automatic recovery patterns
- [x] Policy Engine — executable autonomous rules
- [x] Green Contract — 4-level quality enforcement
- [x] Session persistence + resume
- [x] Multi-provider ready (any Anthropic-compatible API)

### Next up
- [ ] `sego review` — structured workflow analysis command
- [ ] `sego learn` — historical trend analysis and optimization suggestions
- [ ] Pre-built binary releases (GitHub Releases)
- [ ] Plugin marketplace
- [ ] Team dashboard (web UI for Lane Events across team members)

---

## Contributing

sego is open source and welcomes contributions.

1. Fork the repo
2. Create a feature branch
3. Make your changes
4. Verify: `cargo test --workspace && cargo fmt --all --check && cargo clippy --workspace -- -D warnings`
5. Open a PR

See `ROADMAP.md` for current priorities. See `rust/README.md` for crate-level architecture.

---

## License

MIT © 2026 sego contributors

---

<p align="center">
  <strong>sego — The AI coding agent that learns from every run.</strong><br>
  <sub>Open source. Rust native. Model free. Always improving.</sub>
</p>
