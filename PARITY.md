# Parity Status

Sego's primary implementation is the Rust workspace under [`rust/`](rust/). Parity work — making the Rust implementation behavior-compatible with earlier prototypes — is internal engineering history.

## What is public and what is not

| Layer | Location | Public? | Status |
|---|---|---|---|
| Rust workspace (primary implementation) | [`rust/`](rust/) | yes | active |
| Python porting workspace (parity scaffolding) | [`src/`](src/), [`tests/`](tests/) | yes | **retained boundary** — not the mainline, not executed in CI |
| Archived upstream TypeScript snapshot used for parity comparison | `archive/claude_code_ts_snapshot/` | **no** | gitignored; never committed to this repository |

The Python layer under `src/` is a small, self-contained scaffolding that mirrors the command and tool surface for parity bookkeeping. It is not the Rust implementation, and it is not a copy of the archived snapshot. It is not built, not covered by CI, and not part of the supported user path.

The archived snapshot referenced by `src/parity_audit.py` is **not** part of this repository and never has been. Legacy source may only be used as read-only forensic or design reference; see [docs/LEGACY_SOURCE_BOUNDARY.md](docs/LEGACY_SOURCE_BOUNDARY.md).

For what Sego does today, see:

- [README.md](README.md) — overview and quick start
- [USAGE.md](USAGE.md) — task-oriented usage
- [ROADMAP.md](ROADMAP.md) — current direction
- [CHANGELOG.md](CHANGELOG.md) — released changes

For how to build and test, see [DEVELOPMENT.md](DEVELOPMENT.md).
