# Changelog

All notable changes to the Sego Agent project will be documented in this file.

## [Unreleased]

### Added
- **Mitigation awareness in review prompt**: the code review prompt now instructs the model to check for existing mitigations (parameterized queries, allowlists, HMAC/JWT signature verification, sandboxed eval, `shell=False` argument-list form) before reporting a vulnerability pattern. If a mitigation is present and correctly implemented, the finding is downgraded to info; if incomplete or bypassable, it is reported with the specific bypass explained. Identified by an internal 40-diff calibration evaluation (security recall 10/10; false-positive traps revealed over-reporting on mitigated code such as CSRF-exempt webhooks with HMAC verification and allowlist-validated redirects). (#76)

### Changed
- **README rewritten** to a narrative six-section structure (problem → reproducible usage → how to read results → capability boundaries → EgoPulse relationship → development), aligned with the current product positioning ("engineering trust layer for verifying AI claims"). Removes "independent review" wording (review is model-driven constrained review + deterministic evidence gate), corrects the 91.5% statistic to the faithful source phrasing, documents the read-only default permission, and adds an available/experimental/planned capability matrix. Now bilingual: English default (`README.md`) + 简体中文 (`README.zh-CN.md`) with a cross-linked language switcher and English-language architecture/pipeline figures (`assets/figures/`, TikZ sources included).

### Documented
- **Public boundary of the Python parity layer clarified**: `src/` and `tests/` are now recorded as an explicit `retained-boundary` item — a parity scaffolding (~2.1k lines) that is not the mainline, is not executed by CI, and is not part of the supported user path. The archived upstream TypeScript snapshot used for parity comparison (`archive/claude_code_ts_snapshot/`) is gitignored and has never been committed to this repository. `PARITY.md` previously stated that parity work "is not tracked publicly", which was inaccurate given the tracked `src/` tree; the corrected public/private layer table replaces it.
- The default permission mode fallback changed from `DangerFullAccess` to **ReadOnly** in code merged before v0.1.9 (PR #74), but this change was missing from the original v0.1.9 release notes. Recorded here for completeness: a plain `sego` launch now starts read-only; write/command access requires explicit `--permission-mode`, `RUSTY_CLAUDE_PERMISSION_MODE`, or project config.

## [0.1.9] - 2026-08-24

### Added
- **Sego Review Card MVP**: `sego review card`, `sego review card latest`, and `sego review card <review-id>` render an existing local review JSON artifact into an escaped, offline HTML card, update `latest-card.html`, print a compact Green/Yellow/Red summary, and request opening it in the default browser. The card is a review proof, not release approval or security certification.
- **Task acceptance record minimum practice**: adds a locale-neutral `AcceptanceRecord` contract that aggregates node, task-end, and full-review events, remediation trace, unresolved findings, and evidence links. A Chinese/English task-box and compact HTML display adapter localize Sego-owned copy while preserving original artifact evidence.
- **Development Cutover evidence docs**: documented local cutover verification, release-QA capability boundaries, public claim boundaries, and legacy source no-copy rules.
- **C21 latest review summary interface**: `sego review show latest --json` and `/review show latest --json` now print a stable machine-readable summary of the latest review proof. Human-readable `show latest` prints the latest Markdown report, and the no-review case returns a stable JSON shape or clear guidance.
- **C21 agent-callable review proof**: documented the public review artifact contract, agent handoff workflow, and integration templates so AI coding agents can call Sego after code generation and explain the resulting proof to users.
- **Reviewer identity metadata**: new review artifacts now include local trust metadata (`reviewer`, `engine_version`, `review_mode`). These fields are attribution/debug metadata, not cryptographic signatures or provenance attestations.

### Changed
- Updated review artifact JSON Schemas to include current parse/evidence status values (`parse_attempted_but_failed`, `evidence_status`) plus optional C21 metadata fields.
- Plugin lifecycle and hook script paths now use explicit synchronous platform runners instead of Windows file associations; unsupported scripts, missing interpreters, and non-zero exits fail closed.

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
