# DEVELOPMENT.md — Sego Development Guide

This is the short development guide for the Sego repo. It is intentionally focused on what a contributor needs to build, test, and ship a change.

For project direction and design rationale, see [ROADMAP.md](ROADMAP.md) and [PHILOSOPHY.md](PHILOSOPHY.md).

---

## 1. Prerequisites

- Rust toolchain (stable). `rustup` is recommended.
- Git.
- A POSIX-style shell on macOS / Linux, or PowerShell / Git Bash on Windows.

Sego's primary implementation lives under [`rust/`](rust/). Other folders contain documentation, public schemas, packaging, and integration helpers.

---

## 2. Build

```bash
cd rust
cargo build --workspace
```

The release binary is produced by:

```bash
cd rust
cargo build --release --workspace
```

The release artifact is `rust/target/release/sego` (or `sego.exe` on Windows).

---

## 3. Test

```bash
cd rust
cargo test --workspace
cargo fmt --check
```

A change is considered ready when:

- `cargo build --workspace` succeeds,
- `cargo test --workspace` is green,
- `cargo fmt --check` is clean,
- the relevant entry has been added under `[Unreleased]` in `CHANGELOG.md`,
- public-facing docs (README, USAGE, the Chinese user guide) reflect any user-visible change.

If your change touches review behavior, attach a short before/after example to the PR description.

---

## 4. Development Cutover and release-QA boundaries

For local Development Cutover or source-governance work, use the evidence checklist in [`docs/DEVELOPMENT_CUTOVER_EVIDENCE.md`](docs/DEVELOPMENT_CUTOVER_EVIDENCE.md).

The current repository distinguishes local verification from release approval:

- [`docs/RELEASE_QA_CAPABILITY_MATRIX.md`](docs/RELEASE_QA_CAPABILITY_MATRIX.md) records local/CI/release-workflow capability evidence and explicitly marks FAT/UAT/CANARY/PRO as not evidenced unless separate records exist.
- [`docs/PUBLIC_CLAIM_BOUNDARY.md`](docs/PUBLIC_CLAIM_BOUNDARY.md) states which public claims are allowed and which require separate evidence.
- [`docs/LEGACY_SOURCE_BOUNDARY.md`](docs/LEGACY_SOURCE_BOUNDARY.md) records the no-copy legacy source rule.

Tests passed, local review evidence, workflow existence, release approval, and production readiness are separate states.

---

## 5. Code style

- Run `cargo fmt` before pushing.
- Follow the existing module structure under `rust/crates/`. Add new code to the most specific crate that fits; create a new crate only with a clear reason.
- Public items used across crates should have doc comments.
- Avoid `unsafe`. The workspace forbids it by default.

---

## 6. Configuration during development

Sego reads runtime configuration from a small number of well-known locations. For day-to-day development you usually do not need to set anything — the defaults work.

If you need a developer-mode configuration, copy the example file:

```bash
cp .sego/dev.toml.example .sego/dev.toml
```

`.sego/dev.toml` is git-ignored. Do not commit a populated dev config.

---

## 7. Public surface and contracts

The following parts of the repo are public contracts. Changes to them require an explicit note in the PR:

- `schema/` — JSON Schemas for review artifacts and the sidecar protocol.
- `skills/sego-review/` — sidecar skill package consumed by AI coding tools.
- `.github/workflows/release.yml` — release pipeline.
- Public docs: `README.md`, `USAGE.md`, `ROADMAP.md`, `PHILOSOPHY.md`, `docs/LAUNCH.md`, `docs/Sego使用指南.md`.

If your change modifies any of the above, call that out explicitly in the PR title and description so reviewers can apply the right level of scrutiny.

---

## 8. Releasing

Releases are produced by the GitHub Actions workflow under `.github/workflows/release.yml`. Maintainers tag a release; the workflow builds platform binaries, validates artifacts, generates checksums, and publishes the GitHub Release.

You generally do not need to run the release workflow locally. If a release fails to publish, follow the steps printed by the workflow run — do not attempt to re-upload assets by hand.

---

## 9. Sensitive content

Before pushing, double-check that your change does not include:

- API keys, tokens, or session credentials,
- internal service URLs,
- maintainer / personal machine paths (for example `E:\code\...`),
- private customer data in test fixtures or examples.

The repo has secret scanning enabled. If you are unsure whether something is sensitive, ask in the PR before pushing.

---

## 10. Where to ask

- GitHub Issues for bug reports and feature requests.
- [SECURITY.md](SECURITY.md) for security-sensitive reports — do not file those publicly.
- [docs/LAUNCH.md](docs/LAUNCH.md) for private audit scope inquiries.

Thanks for contributing.
