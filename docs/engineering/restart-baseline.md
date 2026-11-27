# Restart baseline (S-001)

Recorded restart baseline for the Rust workspace, taken before any M0 work so that
later failures can be attributed to a change rather than to a pre-existing condition.

- **Date**: 2026-11-28
- **Baseline commit**: `cb463d6` (`main`, latest tag `v0.1.9`)
- **Platform measured here**: Windows 11 (10.0.26200), x86_64-pc-windows-msvc
- **Result**: the baseline is **green**. There are no pre-existing failures to open
  issues for, so this document records the reference numbers instead of a defect list.

## 1. Toolchain

| Tool | Version | Note |
|---|---|---|
| rustc | 1.98.0 (88d9e12ae 2026-08-18) | selected by `rust-toolchain.toml` (`channel = "1.98.0"`) |
| cargo | 1.98.0 (797e8a9bc 2026-08-05) | |
| clippy | 1.98.0 | component installed |
| rustfmt | 1.98.0 | component installed |
| Node.js | 24.16.0 | only used by tooling outside the Rust workspace |
| Python | 3.13.0 | matches the `test-workspace` CI job's `python-version: '3.13'` |
| PowerShell | 7.6.10 and 5.1 | CI is Linux; see §5 for why this is recorded |

CI pins the same compiler through `RUST_TOOLCHAIN: '1.98.0'` in
`.github/workflows/rust-ci.yml`, so a local result is comparable to CI.

## 2. Commands and results

Run from `rust/` (the workspace root), which is also the workflow's
`defaults.run.working-directory`.

| Command | Exit | Result |
|---|---|---|
| `cargo fmt --all --check` | 0 | clean, no diff |
| `cargo clippy --workspace --all-targets` | 0 | **0 warnings, 0 errors** |
| `cargo test --workspace` | 0 | **1135 passed, 0 failed, 2 ignored** across 33 test binaries |

`clippy` is run here without `-D warnings` (the CI job adds it). The absence of
warnings means the additional flag is currently a no-op rather than a latent break.

Test counts are the reference point: a later PR that changes them should say why.

## 3. Dependency and licence state

`rust/deny.toml` and its CI job (`dependency-audit`) already exist at this commit.
`runtime/tests/dependency_policy.rs` re-asserts the same policy offline on every
`cargo test --workspace` (crates.io-only sources, no git dependencies, policy file
present, every advisory waiver carries a reason), which is why the green test run
above also covers it.

Not verified locally: `cargo deny check` itself was **not** executed, because
`cargo-deny` is not installed on this machine. The CI job is the first place the
advisory database is actually consulted. Two waivers are recorded in
`rust/deny.toml` (both "unmaintained, not vulnerable", both transitive via
`syntect 5.3.0`).

## 4. Linux and macOS

Not measured here. The `test-workspace` job (ubuntu-latest) and `test-platforms`
job (windows-latest, macos-latest) cover them in CI, and both provision Python
3.13 with `SEGO_REQUIRE_PYTHON=1` so Python-dependent tests fail rather than skip.

A local Linux run was **not** produced: the only available Linux host has no Rust
toolchain, 2 GB RAM and ~7 GB free disk, and is shared with two unrelated active
projects. Building there risks filling the root filesystem and is not a
representative environment. Treat Linux as CI-covered, not locally verified.

## 5. Environment notes worth keeping

- **`cargo` must be on `PATH`.** The toolchain is installed under a non-default
  prefix on this machine. When it is absent from `PATH`, `scripts/contract-codegen.ps1`
  falls back to `%USERPROFILE%\.cargo\bin\cargo.exe`, which does not exist, and
  fails with `Cargo is missing` — a confusing failure that has nothing to do with
  the repository. The workspace itself only requires the pinned 1.98.0.
- **`core.autocrlf` is `true` on Windows.** `cargo test --workspace` prints
  `LF will be replaced by CRLF` warnings from tests that create throwaway Git
  repositories. They are expected on Windows and are not test failures.
- **PowerShell 5.1 vs 7.x differs on `[Environment]::SetEnvironmentVariable($null, 'Process')`.**
  This is outside the Rust workspace but relevant to the repository's `scripts/*.ps1`:
  under 5.1 the variable is deleted, under 7.6 it is left present with an empty
  value. Any script or test that asserts a variable was restored to *absent*
  therefore passes under 5.1 and fails under 7.6. Measured on the same machine:

  | Host | `Test-Path Env:X` after set-to-null |
  |---|---|
  | Windows PowerShell 5.1 | `False` (deleted) |
  | PowerShell 7.6.10 | `True` (empty string) |

  `scripts/test-windows-package-integrity.ps1` is affected because it asserts
  absence. Today's npm scripts invoke `powershell` (5.1), so the gate passes;
  the constraint matters for any move to PowerShell 7.

## 6. What is deliberately not fixed here

Nothing was changed in this commit except this document. The baseline is recorded,
not repaired: S-001 is a measurement PR.
