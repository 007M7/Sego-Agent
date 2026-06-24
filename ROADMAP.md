# Sego Agent Roadmap

This is the public roadmap for Sego — a local-first code review and engineering trust layer that sits after AI coding tools.

The intent of this roadmap is to give users, contributors, and integrators a clear view of where Sego is going, without making promises about exact dates or feature ordering. Items move based on user feedback and real release readiness, not pre-set timelines.

For what is already shipped today, see the [CHANGELOG](CHANGELOG.md) and the [latest GitHub Release](https://github.com/007M7/Sego-Agent/releases/latest).

---

## Recently Shipped (v0.1.8)

The `v0.1.8` release focused on review reliability, diagnosability, and task-file safety:

- **Review parser hardening** — structured findings, fenced JSON / pretty JSON / raw JSON parsing, and an explicit `parse_attempted_but_failed` state so an unparsable review never silently looks like "0 findings".
- **Evidence gate and recovery guidance** — review findings carry clearer evidence status; non-Git directories, recovered sessions, and path-sensitive workspaces get structured next-step hints instead of opaque errors.
- **Task-file review command execution** — task files that explicitly require `/review staged`, `/review workspace`, or allowlisted `--full <path>` review can have the review executed directly; unsupported commands (`/commit`, `dotnet build`, free-form prose, combined `/cd ... && /review ...`) are blocked with structured guidance instead of silently entering model conversation.
- **Natural-language export safety boundary** — phrases like "save the last report" or "export the previous response" are bound to the latest / previous confirmable response, not treated as arbitrary file-write commands.
- **Release packaging** — release workflow generates, validates, uploads, and documents `checksums.txt` so users can verify downloaded binaries.

---

## Near-Term Focus

Quality and trust on top of the existing review workflow.

- **Review artifact contract / schema** — keep `.sego/reviews/` artifacts forward-compatible; tighten the JSON Schema so external tools can rely on the format.
- **Install verification and tag-pinned install guidance** — make it easier for new users to verify they installed a real Sego release and pin to a known version.
- **Release provenance and signing** — improve integrity of release artifacts beyond the current checksum file.
- **Windows / macOS CI matrix** — extend pre-merge CI coverage so platform-specific regressions are caught earlier.
- **CI quality gate hardening** — turn currently advisory checks into blocking ones as the warning baseline becomes clean.
- **Public docs cleanup** — keep README, USAGE, and the Chinese user guide aligned with the latest release behavior; avoid stale wording or internal vocabulary leaking into public docs.

---

## Mid-Term Direction

Make Sego review trustworthy enough to live inside team and enterprise workflows.

- **Reviewer identity in review artifacts** — record which agent, which model, and which Sego version produced a review so artifacts are clearly attributable.
- **Review artifact signing** — optional integrity signing (for example sigstore-style) so a downstream consumer can verify a review artifact was not modified after the fact.
- **GitHub Actions / CI integration** — first-class support for running Sego review inside common CI systems and consuming structured findings.
- **Enterprise local-first documentation** — Windows execution-policy, proxy / TLS, offline / air-gapped setup, and audit-log expectations.
- **Session persistence hardening** — better handling of sensitive content in persisted session state.

---

## Long-Term Direction

Position Sego as the "review and trust" layer that other AI coding tools can plug into, while staying local-first by default.

- **Sidecar integrations and ecosystem compatibility** — make the sidecar JSON contract and skill packaging stable enough for AI coding tools and editors that want to call Sego review through a documented interface.
- **Review output JSON contract precision** — reduce `parse_attempted_but_failed` frequency by tightening the contract the model is asked to honor.
- **Cross-tool review artifact format** — explore whether the `.sego/reviews/` artifact format can become a public reference that other reviewers and CI tools can consume, not just Sego itself.

These items are exploratory. They depend on user demand, ecosystem maturity, and engineering capacity — they will not be added to a release just to expand surface area.

---

## What Sego Will Not Do

- **Replace developers or human review for high-risk releases.** Sego is a trust layer that helps developers judge AI-generated code; it does not remove the need for human judgment.
- **Promise exhaustive static analysis or bug completeness.** `sego review` (including `--full`) reviews key manifests, entry points, and a directory context snapshot. It is model-driven and is not a substitute for proven static analyzers, security scanners, or formal verification.
- **Execute arbitrary unsupported commands from task files.** Only allowlisted review commands run; everything else is blocked with structured guidance.
- **Force a cloud-first workflow.** Sego stays local-first by default. Any future cloud or team-collaboration mode will be opt-in.

---

## Feedback And Direction

Roadmap direction is driven by:

- real user feedback and audit reports,
- release-blocker findings from Sego's own pre-release review,
- contributor input through GitHub Issues and PRs.

If a use case you care about is missing, please open an issue with concrete context (project type, AI coding tool you are using, what you wanted Sego to confirm). Concrete usage stories move items up the roadmap faster than feature requests in the abstract.
