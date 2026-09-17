# Contributing to sego

Thanks for your interest in contributing. Sego is a local-first code review and engineering trust layer that runs after AI coding tools — it reviews code that has just been generated or changed, and does not generate code itself. Changes that broaden it into a code generator, IDE, or cloud platform are out of scope; see [AGENTS.md](AGENTS.md) for the two questions to ask when judging fit.

## Getting Started

1. Fork the repository
2. Clone your fork: `git clone https://github.com/YOUR_USERNAME/Sego-Agent.git`
3. Build: `cd rust && cargo build --workspace`
4. Run tests: `cargo test --workspace`
5. Make your changes on a feature branch

## Development Workflow

### Before submitting a PR

```bash
cd rust
cargo fmt --all --check       # Format check — gate
cargo test --workspace        # All tests must pass — gate
cargo clippy --workspace --all-targets -- -D warnings  # Lint — advisory
```

`cargo fmt --all --check` and `cargo test --workspace` are gates. **Clippy is advisory today**: the command above does not pass on `main` because of a pre-existing pedantic-warning baseline, so a clippy failure is not by itself evidence that your change is at fault. Please do not add new warnings. Clearing that baseline is tracked in `ROADMAP.md`.

### Commit style

- Use present tense: "add feature X" not "added feature X"
- Keep commits focused — one logical change per commit
- Reference issues with #issue-number

### Code guidelines

- Follow existing patterns in each crate
- No `unsafe` code: the workspace sets `unsafe_code = "forbid"`, so it does not compile rather than needing justification
- New modules need corresponding tests
- New tools need tool spec definitions in `tools/src/lib.rs`
- Lane Events should be emitted for new workflow steps

## Architecture

See `rust/README.md` for the crate map and architecture overview.

The runtime is organized into focused modules:

| Module | Responsibility |
|--------|---------------|
| `lane_events.rs` | Structured workflow event definitions |
| `recovery_recipes.rs` | Automatic failure recovery patterns |
| `policy_engine.rs` | Autonomous decision rules |
| `green_contract.rs` | Quality level enforcement |
| `worker_boot.rs` | Agent lifecycle state machine |

## Where to Start

- **Good first issues:** Look for `// TODO` comments in the codebase
- **Current priorities:** See `ROADMAP.md`
- **Documentation:** Improving docs is always welcome
- **Tool implementations:** See `tools/src/lib.rs` for the tool registry

## Questions?

Open an issue or start a discussion.
