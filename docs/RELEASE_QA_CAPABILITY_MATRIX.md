# Release QA Capability Matrix

This document records what the repository currently provides for Development Cutover evidence. It is deliberately a capability matrix, not a release approval.

## Status vocabulary

| Status | Meaning |
|---|---|
| `evidenced` | The capability is present in tracked source and can be inspected or run locally. |
| `local-only` | The capability was or can be exercised in the local development worktree, but no remote CI run is claimed. |
| `advisory` | The check exists but is not configured as a hard failure. |
| `unenforced` | The check fails hard when it runs, but nothing requires it to pass before a merge, because the branch is not protected. |
| `not-evidenced` | No sufficient source, environment, owner, or run evidence exists in this cutover. |

## Repository workflow evidence

| Area | Current evidence | Status | What this does **not** mean |
|---|---|---|---|
| Local formatting gate | `cargo fmt --all --check` from `rust/`. | `evidenced` | It does not prove runtime behavior. |
| Local build gate | `cargo build --workspace --offline` from `rust/`. | `local-only` | It is not a remote CI run and does not publish binaries. |
| Local test gate | `cargo test --workspace --offline --no-fail-fast` from `rust/`. | `local-only` | It is not a release, public QA approval, or exhaustive static analysis. |
| Git whitespace gate | `git diff --check`. | `local-only` | It does not validate product behavior. |
| Rust CI workflow | `.github/workflows/rust-ci.yml` contains `cargo fmt --all --check` and `cargo test --workspace`. | `evidenced` | The local cutover does not claim that GitHub Actions has run for the current local commits. |
| CI trigger coverage | `.github/workflows/rust-ci.yml` `pull_request`/`push` paths cover `rust/**`, `schema/**`, the installers, `packaging/**`, `skills/**` and `.github/workflows/**`. | `evidenced` | Widening the paths means a schema-only or installer-only change does run the suite; it does not mean the suite covers those surfaces. |
| Cross-platform CI job | `.github/workflows/rust-ci.yml` `test-platforms` runs `cargo test --workspace` on a windows/macos matrix. | `unenforced` | Job definition reviewed in this cutover; no run evidence. See the merge-gating row below. |
| Contract metadata job | `.github/workflows/rust-ci.yml` `contracts` validates the `schema/*.json` `x-contract-*` metadata. | `unenforced` | It checks that the contract headers are present and consistent. It does not check that the implementation matches the schema. |
| Dependency policy job | `.github/workflows/rust-ci.yml` `dependency-audit` runs `cargo deny check` with `continue-on-error: true`. | `advisory` | It must not be described as a blocking supply-chain gate while this setting remains. |
| Dependency source policy | `rust/crates/runtime/tests/dependency_policy.rs` fails the normal suite if a resolved dependency leaves crates.io or a manifest declares a git source. | `evidenced` | Offline and declaration-based: it does not detect a compromised crates.io release, only a source that is not crates.io. |
| Coverage measurement | `cargo llvm-cov --workspace` measured 81.39% lines (40291/49504) and 80.64% regions on 2026-09-17, recorded with provenance in `rust/coverage-baseline.json`. | `local-only` | One platform. The run also included another window's uncommitted module, so it is not reproducible from the commit alone. Coverage counts execution, not correctness. |
| Coverage floor | `.github/workflows/rust-ci.yml` `coverage` job runs `cargo llvm-cov --workspace --fail-under-lines 75`; `runtime/tests/coverage_policy.rs` asserts the CI number equals the recorded one and that every crate has a floor. | `unenforced` | A ratchet against a drop, not a quality target. The floor is below the measured value on purpose because CI measures on a platform where more code compiles. |
| Clippy workflow | `.github/workflows/rust-ci.yml` contains `cargo clippy --workspace --all-targets` with `continue-on-error: true`. | `advisory` | It must not be described as a blocking CI gate while this setting remains. |
| Release test gate | `.github/workflows/release.yml` `verify` runs `cargo fmt --all --check` and `cargo test --workspace` on the tagged commit, and `build-windows` / `build-linux` / `build-macos` each declare `needs: [verify]`. | `evidenced` | A failing `verify` stops the build jobs and therefore the release, independently of branch protection. It does not cover the update path on an installed host. |
| Merge gating | `gh api repos/007M7/Sego-Agent/branches/main/protection` returns 404 `Branch not protected`. No check is a required status check. | `unenforced` | Every job above reports a result; none of them prevents a merge. Until protection is enabled, a red run is information, not a block. |
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
