# Changelog

All notable changes to the Sego Agent project will be documented in this file.

## [Unreleased]

## [0.1.8] - 2026-06-23

### Added
- **C20.6-C task-file review command execution**: when a task file explicitly tells Sego to run a review command (`Required command:`, `Required review command:`, `Must run:`, `必须执行：`, `请执行：`, `执行以下命令`), Sego now executes the allowlisted `/review staged|workspace|--full <path>` (or `sego review` equivalents) directly instead of replying conversationally. Required-marker commands that are NOT `/review`/`sego review` (e.g. `/commit`, `dotnet build`, `xcopy /E src dst`, free-form prose) are **blocked by rule** with structured `Detected / Reason / Guidance` output. Combined commands like `/cd <path> && /review staged` are blocked and the guidance preserves both the `/cd <path>` and `/review <scope>` parts so the user can run them on separate lines. Continuation lines whose lead-in negates or cautions against review (`do not run /review staged`, `skip /review staged`) are no longer interpreted as an instruction to run review.
- **C20.6-B evidence gate and recovery guidance**: review artifacts now persist an explicit evidence status alongside each finding, and the review pipeline emits structured recovery hints when intermediate artifacts are missing or stale.
- **C20.6-A review parser diagnostics and artifact export guidance**: parse-failure path now records concrete diagnostics in the JSON artifact, and the terminal output explains what the user can do next instead of silently degrading.
- **Full repository audit mode**: `sego review --full <path>` for clean cloned repos and non-Git directories. Reads key manifest files, entry points, and a context snapshot of the directory tree, then produces `.sego/reviews/` artifacts without requiring a working Git repository. This is a manifest/entrypoint/context snapshot review, not exhaustive static analysis.
- **Review parser hardening**: raw JSON findings, fenced JSON, and pretty JSON embedded in prose text are now all parsed reliably. A new `parse_attempted_but_failed` status prevents misleading "Findings 0" display when the model clearly produced findings.
- **Latest-response export improvements**: `Kind: markdown` and `Bytes` fields added to export output. Clearer recovery hint when no assistant response is available to export.

### Changed
- **Blocked task-file commands print cleanly**: direct CLI invocations like `sego "Required command: /commit"` now print a `Task command blocked` block with `Detected / Reason / Guidance` and exit zero, instead of being formatted as a parse error with an `error:` prefix and a `Run sego --help` footer.
- **Natural-language local action hardening**: conservative export boundary (requires explicit `last/previous/刚才/上一条`). Fuzzy save/export phrases without a target now route to `/dir` guidance. Safer `/dir` action directory with usage examples and safety notes.
- **Non-Git review recovery**: explains `sego review --full <path>` instead of a raw `git fatal` error message.
- **Recovery hints format**: structured `Action / Reason / Workspace / Next step` output for export and review failure modes.

### Known issues
- Review model output may still occasionally emit invalid JSON; tracked as `C20.5-REVIEW-003`.
- `sego review` (including `--full`) is a model-driven review of manifests, entry points, and a context snapshot of the directory. It is not exhaustive static analysis and is not guaranteed to find every bug.

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
- Rust workspace implementation

### Changed
- Project renamed and re-launched as Sego Agent
- Migrated to a pure Rust implementation

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
