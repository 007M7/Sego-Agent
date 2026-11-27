<!--
  Keep this template short. It exists to make three things unavoidable:
  what changed, how you know it works, and whether it touches a published
  contract. Delete nothing; write "none"/"n/a" rather than leaving a section
  blank, so a reviewer can tell "checked" from "skipped".
-->

## What changed

<!-- One paragraph. What behaviour differs after this merges? -->

## Why

<!-- The problem this solves. Link the issue if there is one. -->

## Verification

The three gates run in CI exactly as written below; paste what you ran locally.
Do not check a box you did not actually run.

```bash
cd rust
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- [ ] `cargo fmt --all --check` — clean
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — no warnings
- [ ] `cargo test --workspace` — all pass
- [ ] Coverage floor respected (`rust/coverage-baseline.json`)

Local result, if it differs from the above: <!-- e.g. "not run: Windows-only, covered by CI" -->

## Contract, schema and policy impact

A change here can break every consumer of a published contract without any test
turning red, so state it explicitly.

- [ ] No `schema/` change
- [ ] No `sidecar` / review-sidecar contract change
- [ ] No `rust/deny.toml` policy or advisory-waiver change

If any box above is unchecked, describe the impact and the migration path:

<!-- contract id / revision, what breaks, who has to change -->

## Security and privacy

- [ ] No new network egress
- [ ] No new credential, token or key handling
- [ ] No change to what is logged, persisted or sent to a provider
- [ ] No new `unwrap()`/`expect()` on a path that can fail at runtime

If any box above is unchecked, say what a reviewer should look at.

## Scope check

- [ ] One logical change — a reviewer can finish this in roughly ten minutes
- [ ] Mechanical moves (file splits, renames) contain no behaviour change
- [ ] No unrelated reformatting of files this PR does not otherwise touch

## Notes for the reviewer

<!-- Anything that is easy to miss: a deliberately kept quirk, a rejected
     alternative, a follow-up that should not block this PR. -->
