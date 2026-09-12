# AGENTS.md — Contributor Quickstart

This file is the short orientation for contributors. It is intentionally minimal so the public repo stays focused on what an external contributor needs to land a change.

For end-user usage, see [USAGE.md](USAGE.md) and the [Chinese user guide](docs/Sego使用指南.md).
For project direction, see [ROADMAP.md](ROADMAP.md).
For design philosophy, see [PHILOSOPHY.md](PHILOSOPHY.md).

---

## 1. What Sego is

Sego is a local-first code review and engineering trust layer that sits after AI coding tools. It does not generate code — it reviews code that has just been generated or changed, and writes structured review artifacts under `.sego/reviews/`.

If you are evaluating whether your contribution fits Sego's scope, the two questions to ask are:

- Does it improve how Sego reviews code, or how Sego presents review results to a developer?
- Does it keep the workflow local-first and the artifact format honest — no silent "0 findings", and no unverified result presented as acceptance?

Changes that broaden Sego into a code generator, IDE, or cloud platform are out of scope.

---

## 2. Build and test

Sego's primary implementation is Rust.

```bash
cd rust
cargo build --workspace
cargo test --workspace
cargo fmt --all --check
```

A successful PR typically:

- builds clean on `cargo build --workspace`
- passes `cargo test --workspace` — note that this is also what compiles **every test target**. `cargo build` does not, so a change that breaks only test code (a struct literal in a test, for example) can still pass a build; run the tests
- is formatted with `cargo fmt --all --check` — the `--all` matters, because `cargo fmt --check` alone can miss files
- updates the relevant section of `CHANGELOG.md` under `[Unreleased]`
- keeps temporary or scratch files out of the worktree

Clippy is **advisory today**: `cargo clippy --workspace --all-targets -- -D warnings` does not pass on `main` because of a pre-existing pedantic-warning baseline. A clippy failure is therefore not by itself evidence that your change is at fault — but please do not add new warnings. Clearing that baseline is tracked in [ROADMAP.md](ROADMAP.md).

For releases and packaging, see the workflows in `.github/workflows/`.

---

## 3. How to submit a change

1. Fork the repo and create a topic branch.
2. Keep the change small and focused. One concern per PR.
3. If your change touches review behavior, include before/after examples in the PR description so reviewers can judge the user-visible effect.
4. If your change touches the public schemas under `schema/` or the sidecar protocol, treat it as a **contract change**. Those schema files are the contract source and carry `x-contract-id`, `x-contract-revision` and `x-contract-ratified`; change the contract there rather than only in the implementation, and call the change out explicitly. Widening an enum because the implementation already emits an extra value is still a contract change — and for the sidecar envelope it also has to be coordinated with the consumer.
5. Run the build and tests locally before pushing.
6. Open a PR against `main` with a clear summary, verification list, and risk notes.

---

## 4. What to avoid in PRs

- Do not include API keys, tokens, internal URLs, or maintainer machine paths.
- Do not include private customer data in test fixtures or examples.
- Do not change the release workflow (`release.yml`), the schema files, or the security-sensitive paths without a separate, narrowly scoped PR and an explicit note in the PR title.
- Do not push large binary assets to the repo. Use release assets instead.
- Do not commit generated scratch output (temp logs, probe scripts, one-off fixtures); clean it up or keep it out of the tree.

---

## 5. Traps that already exist in this tree

These have all misled a reader or a change at least once. They are cheap to avoid if you know about them.

- **Two CLI files are not compiled.** `rust/crates/rusty-claude-cli/src/args.rs` and `rust/crates/rusty-claude-cli/src/app.rs` are not declared as modules in `main.rs`, and the crate does not depend on `clap`. The real CLI entry point is `main.rs`. Both files now open with a `NOT COMPILED` banner — do not treat them as the current CLI surface, and do not "fix" them expecting the change to take effect.
- **Artifact ids can collide by design.** An id is `review-<epoch-second>-<diff-prefix>`, so two reviews of the same diff that finish within the same second compute the same id. Artifact files are written with `create_new`, so the second one fails with `artifact_id_conflict` instead of overwriting. Keep both halves of that behaviour: a silent overwrite destroys earlier evidence, and retrying with a fresh id would silently change identity.
- **Tests must not share global state.** A test that mutates `std::env::set_current_dir` / `HOME` / `CLAW_CONFIG_HOME` needs a unique temp directory per test, and its cleanup should tolerate failure. A path built only from a coarse clock reading plus a shared root, cleaned with `remove_dir_all(...).expect(...)`, produced intermittent failures that looked like real regressions.
- **Review results stay model-assisted.** `positive_acceptance` is always `false` and a zero-findings result is never a pass. Do not add a path that promotes an unverified, partly parsed, or incompletely captured result to acceptance.

---

## 6. Where to ask

- Open a [GitHub Issue](https://github.com/007M7/Sego-Agent/issues) for bug reports and feature requests.
- For security issues, follow [SECURITY.md](SECURITY.md) rather than filing a public issue.
- For private audit scope discussions, use the private-audit Issue template described in [docs/LAUNCH.md](docs/LAUNCH.md). Do not include the actual code in the public issue — scope confirmation comes first.

Thanks for contributing to Sego.
