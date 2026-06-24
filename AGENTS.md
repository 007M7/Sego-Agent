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
- Does it keep the workflow local-first and the artifact format honest (no silent "0 findings")?

Changes that broaden Sego into a code generator, IDE, or cloud platform are out of scope.

---

## 2. Build and test

Sego's primary implementation is Rust.

```bash
cd rust
cargo build --workspace
cargo test --workspace
cargo fmt --check
```

A successful PR typically:

- builds clean on `cargo build --workspace`
- passes `cargo test --workspace`
- is formatted with `cargo fmt`
- updates the relevant section of `CHANGELOG.md` under `[Unreleased]`

For releases and packaging, see the workflows in `.github/workflows/`.

---

## 3. How to submit a change

1. Fork the repo and create a topic branch.
2. Keep the change small and focused. One concern per PR.
3. If your change touches review behavior, include before/after examples in the PR description so reviewers can judge the user-visible effect.
4. If your change touches the public schemas under `schema/` or the sidecar protocol, treat the change as a contract change and call that out explicitly.
5. Run the build and tests locally before pushing.
6. Open a PR against `main` with a clear summary, verification list, and risk notes.

---

## 4. What to avoid in PRs

- Do not include API keys, tokens, internal URLs, or maintainer machine paths.
- Do not include private customer data in test fixtures or examples.
- Do not change the release workflow (`release.yml`), the schema files, or the security-sensitive paths without a separate, narrowly scoped PR and an explicit note in the PR title.
- Do not push large binary assets to the repo. Use release assets instead.

---

## 5. Where to ask

- Open a [GitHub Issue](https://github.com/007M7/Sego-Agent/issues) for bug reports and feature requests.
- For security issues, follow [SECURITY.md](SECURITY.md) rather than filing a public issue.
- For private audit scope discussions, use the private-audit Issue template described in [docs/LAUNCH.md](docs/LAUNCH.md). Do not include the actual code in the public issue — scope confirmation comes first.

Thanks for contributing to Sego.
