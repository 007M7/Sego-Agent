# Changelog

All notable changes to the Sego Agent project will be documented in this file.

## [Unreleased]

## [0.1.7] - 2026-06-20
- Improved ordinary `sego review` terminal output with a human-readable structured report while keeping Markdown/JSON/index artifacts.
- Fixed fenced JSON parsing when review finding fields contain nested Markdown code fences.
- Narrowed natural-language latest-response export routing to avoid accidental export on phrases such as "输出结论" or "write report".
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
