# Release QA Capability Matrix

This document records what the repository currently provides for Development Cutover evidence. It is deliberately a capability matrix, not a release approval.

## Status vocabulary

| Status | Meaning |
|---|---|
| `evidenced` | The capability is present in tracked source and can be inspected or run locally. |
| `local-only` | The capability was or can be exercised in the local development worktree, but no remote CI run is claimed. |
| `advisory` | The check exists but is not configured as a hard failure. |
| `not-evidenced` | No sufficient source, environment, owner, or run evidence exists in this cutover. |

## Repository workflow evidence

| Area | Current evidence | Status | What this does **not** mean |
|---|---|---|---|
| Local formatting gate | `cargo fmt --all --check` from `rust/`. | `evidenced` | It does not prove runtime behavior. |
| Local build gate | `cargo build --workspace --offline` from `rust/`. | `local-only` | It is not a remote CI run and does not publish binaries. |
| Local test gate | `cargo test --workspace --offline --no-fail-fast` from `rust/`. | `local-only` | It is not a release, public QA approval, or exhaustive static analysis. |
| Git whitespace gate | `git diff --check`. | `local-only` | It does not validate product behavior. |
| Rust CI workflow | `.github/workflows/rust-ci.yml` contains `cargo fmt --all --check` and `cargo test --workspace`. | `evidenced` | The local cutover does not claim that GitHub Actions has run for the current local commits. |
| Clippy workflow | `.github/workflows/rust-ci.yml` contains `cargo clippy --workspace --all-targets` with `continue-on-error: true`. | `advisory` | It must not be described as a blocking CI gate while this setting remains. |
| Release workflow | `.github/workflows/release.yml` is tag-triggered and builds Windows, Linux, and macOS artifacts with checksums and a GitHub Release step. | `evidenced` | It is not release approval, and it has not been run by Development Cutover. |
| Release notes body | `.github/workflows/release.yml` includes historical release-body wording. | `evidenced with boundary` | It is not current vNext public wording and must be reviewed in a separate release-gate task before tagging. |

## Environment evidence

| Environment / gate | Evidence in this cutover | Status | Required before claiming it |
|---|---|---|---|
| Remote CI for current local commits | No push/PR/check-run evidence is created by this local cutover. | `not-evidenced` | Push/PR authorization, CI run URL, check conclusion, and owner review. |
| FAT | No FAT environment, owner, test plan, or run record. | `not-evidenced` | Named owner, scope, fixtures, execution log, rollback/defect policy. |
| UAT | No UAT environment, user cohort, consent, or run record. | `not-evidenced` | Named user cohort, consent, acceptance criteria, run evidence. |
| CANARY | No canary deployment, monitoring, or rollback evidence. | `not-evidenced` | Deployment target, monitor, rollback, incident owner, and run evidence. |
| PRO / production | No production deployment or operating evidence. | `not-evidenced` | Release approval, deployment record, monitoring, support, and rollback proof. |

## Development Cutover verification command set

Use this command set when validating the canonical local `develop` entry:

```powershell
cd "E:\Sego max\08_code\worktrees\sego-agent-develop\rust"
cargo fmt --all --check
cargo build --workspace --offline
cargo test --workspace --offline --no-fail-fast
cd ..
git diff --check
git status --short --branch
```

Passing these commands means the local development branch is reproducibly buildable and testable offline. It does **not** mean remote CI, release approval, security certification, public launch approval, or customer-value proof.
