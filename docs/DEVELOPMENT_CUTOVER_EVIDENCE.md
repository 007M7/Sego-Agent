# Development Cutover Evidence

This document records the local Development Cutover evidence model for the repository.

## What cutover means

Development Cutover means the local development entry is a clean Git-controlled `develop` worktree that can be used for future implementation through focused branches and reviewed merges.

It does not mean remote push, PR approval, release approval, tag creation, production deployment, live-model review, or public launch approval.

## Local acceptance checklist

From repository root:

```powershell
cd rust
cargo fmt --all --check
cargo build --workspace --offline
cargo test --workspace --offline --no-fail-fast
cd ..
git diff --check
git status --short --branch
```

The worktree should end clean on `develop`.

## Boundary documents

- [`RELEASE_QA_CAPABILITY_MATRIX.md`](RELEASE_QA_CAPABILITY_MATRIX.md) records local, CI, release-workflow, and environment evidence boundaries.
- [`PUBLIC_CLAIM_BOUNDARY.md`](PUBLIC_CLAIM_BOUNDARY.md) states which public claims are allowed and which require separate evidence.
- [`LEGACY_SOURCE_BOUNDARY.md`](LEGACY_SOURCE_BOUNDARY.md) records the no-copy legacy source rule.

## Review boundary

A full live `sego review` may require a configured model/API. If live model/API use is not authorized for a cutover run, the absence of that run must be recorded instead of silently claimed.
