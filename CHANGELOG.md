# Changelog

All notable changes to the Sego Agent project will be documented in this file.

## [Unreleased]

### Added
- **Full repository audit mode**: `sego review --full <path>` for clean cloned repos and non-Git directories. Walks the full directory tree, reads key manifest files and entry points, and produces `.sego/reviews/` artifacts without requiring a working Git repository.
- **Review parser hardening**: raw JSON findings, fenced JSON, and pretty JSON embedded in prose text are now all parsed reliably. A new `parse_attempted_but_failed` status prevents misleading "Findings 0" display when the model clearly produced findings.
- **Latest-response export improvements**: `Kind markdown` and `Bytes` fields added to export output. Clearer recovery hint when no assistant response is available to export.

### Changed
- **Natural-language local action hardening**: conservative export boundary (requires explicit `last/previous/??/???`). Fuzzy save/export phrases without a target now route to `/dir` guidance. Safer `/dir` action directory with usage examples and safety notes.
- **Non-Git review recovery**: explains `sego review --full <path>` instead of a raw `git fatal` error message.
- **Recovery hints format**: structured `Action / Reason / Workspace / Next step` output for export and review failure modes.

### Known issues
- Review model output may still occasionally emit invalid JSON; tracked as `C20.5-REVIEW-003`.

## [0.1.7] - 2026-06-20
- Improved ordinary `sego review` terminal output with a human-readable structured report while keeping Markdown/JSON/index artifacts.
- Fixed fenced JSON parsing when review finding fields contain nested Markdown code fences.
- Narrowed natural-language latest-response export routing to avoid accidental export on phrases such as "????" or "write report".
- Improved Windows startup/update guidance and refreshed README, USAGE, and the Chinese user guide for the v0.1.7 behavior.

### Added
- Initial open-source release of Sego Agent
- Rust-native AI coding agent with 40+ built-in tools
- Self-Learning System: Lane Events, Failure Taxonomy, Recovery Recipes
- Policy Engine for autonomous coding decisions
- Green Contract with 4 quality enforcement levels
- Anthropic-compatible API client with SSE streaming
- OpenAI-compatible API client for broader model support
- MCP server lifecycle management
- LSP client integration
- Interactive REPL and one-shot prompt modes
- Windows, Linux, and macOS support
- 9-crate Rust workspace (~50K LOC)

### Changed
- Renamed from claw-code to Sego Agent
- Migrated from Python to pure Rust implementation

### Documentation
- README.md with quick start guide and architecture overview
- USAGE.md with detailed CLI reference
- PHILOSOPHY.md explaining the design principles
- ROADMAP.md with active development roadmap
- CONTRIBUTING.md with guidelines for contributors
- Parity tracking documentation

## [0.1.6] - 2026-06-19
- Added deterministic natural-language local action routing and /dir action directory.
- Changed bare sego review to run code review by default and persist .sego/reviews artifacts.
- Added explicit workflow/session review entrypoints: sego workflow-review / sego session-review.
