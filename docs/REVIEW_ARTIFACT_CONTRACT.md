# Sego Review Artifact Contract

This document describes the public, agent-readable review proof artifacts written by Sego under `.sego/reviews/`.

Sego is not a code generator. Sego is an independent review/proof engine that can be called after another AI coding agent generates or changes code. The review artifact is the handoff object that another agent can read and explain to a user.

**Contract identity**: this contract is `sego.review.artifact/v1` (contract source: `schema/review-artifact.schema.json`); its Rust type is `SegoReviewArtifact` in `runtime/src/code_review/report.rs`. It is Sego's **verification-domain** artifact and is deliberately distinct from EgoPulse's `VerificationArtifact`, which is an **acceptance record** — the two are related by reference (`invocation_id`, review id, content hash, schema version) and are never merged into one object.

---

## 1. Artifact locations

A review run writes files under the current project's `.sego/reviews/` directory:

```text
.sego/reviews/
├── review-<epoch>-<hash>.json   machine-readable review artifact
├── review-<epoch>-<hash>.md     human-readable Markdown report
├── index.jsonl                  append-only review index, one JSON object per line
└── status.jsonl                 optional finding disposition updates
```

The `.sego/` directory is runtime output and is normally git-ignored. Do not commit private review artifacts unless you intentionally want to publish them.

---

## 2. Which interface should an agent use?

Preferred machine-readable path:

```bash
sego review show latest --json
```

This prints a stable latest-review summary with:

- `schema_version`
- `kind`
- `found`
- latest review identity and paths
- finding count / highest severity / parse status
- optional status counts

It intentionally does not include full findings or raw model output. Agents can open `review.json_path` when they need the complete artifact.

Fallback path for older Sego versions:

1. Read `.sego/reviews/index.jsonl`.
2. Select the last non-empty line.
3. Read `json_path` from that index entry.
4. Parse that JSON artifact.
5. Explain the summary and findings to the user.

Agents should use JSON for machine-readable data and link to the Markdown report for human details.

---

## 3. Stability policy

| Field / file | Stability | Notes |
|---|---|---|
| `.sego/reviews/index.jsonl` line format | stable | Agents may use it to find the latest artifact. |
| `schema_version` | stable | Current value: `1`. |
| `id`, `created_at_epoch_seconds`, `scope`, `diff_hash` | stable | Core artifact identity fields. |
| `finding_count`, `highest_severity`, `parse_status`, `findings` | stable | Core summary and finding fields. |
| `reviewer`, `engine_version`, `review_mode` | stable-additive | Added in C21; absent in older artifacts. They are local trust metadata, not signatures. |
| `parse_error`, `parse_repair` | stable-additive | Optional parser diagnostics. |
| `evidence_status` | stable-additive | Optional per-finding deterministic evidence status. |
| `raw_text` | stable but human-oriented | Useful for debugging; agents should not rely on its prose format. |
| `status.jsonl` | experimental | Finding lifecycle/disposition updates may evolve. |
| Markdown report formatting | human-readable, not machine contract | Do not parse the Markdown table as the primary API. |

Stable-additive means a field can be added or absent without breaking old artifacts. Consumers should tolerate missing optional fields.

---

## 4. Minimal review artifact example

```json
{
  "schema_version": 1,
  "id": "review-1782600000-ab12cd34ef56",
  "created_at_epoch_seconds": 1782600000,
  "reviewer": "sego",
  "engine_version": "0.1.9",
  "review_mode": "model_code_review",
  "scope": "staged",
  "diff_hash": "ab12cd34ef56...",
  "finding_count": 1,
  "highest_severity": "medium",
  "parse_status": "structured",
  "git_status": "## main...origin/main",
  "findings": [
    {
      "id": "finding-123456789abc",
      "severity": "medium",
      "file": "src/auth.rs",
      "line": 42,
      "title": "Missing authorization check before account update",
      "evidence": "The update handler writes account data before checking the caller role.",
      "risk": "A non-admin caller may update another user's account.",
      "suggestion": "Check the caller role and account ownership before performing the write.",
      "confidence": 0.82,
      "verification_hint": "Add an integration test for a non-admin caller updating another account.",
      "evidence_status": "verified"
    }
  ],
  "raw_text": "..."
}
```

---

## 5. Important field meanings

### `parse_status`

| Value | Meaning |
|---|---|
| `structured` | Sego parsed structured findings from model output. |
| `fallback_raw_text` | Sego did not get structured findings and preserved raw output. |
| `parse_attempted_but_failed` | The output looked findings-like, but structured parsing failed. Do not treat this as a clean "0 findings" review. Read the Markdown/raw output. |

### `evidence_status`

| Value | Meaning |
|---|---|
| `verified` | The cited file/line evidence was found in the captured review target. |
| `unverified_file` | The cited file was not present in the captured target. |
| `unverified_line` | The file existed, but the cited line was outside captured range. |
| `unverified_dependency` | The finding depends on dependency/manifest evidence that was not captured. |
| `scope_not_captured` | The finding refers to content outside the review scope. |
| `content_not_captured` | The file was listed in scope, but its content was not captured. |
| `content_truncated` | The file content was captured only partially. |
| absent / null | Legacy artifact or no deterministic evidence status attached. |

These seven values are exactly what `EvidenceStatus` serializes
(`runtime/src/code_review/report.rs:17-32`). Earlier revisions of this contract listed only
the first five; artifacts carrying `content_not_captured` or `content_truncated` therefore
failed the published JSON Schema even though the implementation already emitted them.

### `highest_severity`

The highest severity among findings, or `null` if there are no findings. Supported values:

```text
critical, high, medium, low, info
```

### `reviewer`, `engine_version`, `review_mode`

These identify the local review engine and mode that produced the artifact. They are useful for attribution and debugging.

They are **not** a cryptographic signature, provenance attestation, or certification. Future releases may add signing/provenance separately.

### Review Card confidence (`Green` / `Yellow` / `Red`)

The Review Card is a **presentation** of an artifact — not a field on it, and not a
verdict. Its colour is derived mechanically (`rusty-claude-cli/src/review_card.rs`):

- `Red` when `parse_status` is anything other than `structured`: the review did
  not produce findings Sego could read, so there is nothing to grade;
- `Yellow` when unresolved risks remain;
- `Green` otherwise.

`Green` therefore means "this artifact parsed and no unresolved risk was recorded
in it". It does not mean "the code has no problems", and it is not acceptance,
approval, or a pass. A consumer that treats `Green` as a merge gate has replaced
its own review with a colour, which is the opposite of what the artifact is for.

The machine-readable equivalent of the card is the artifact's `parse_status`,
`highest_severity`, `evidence_coverage` and each finding's `evidence_status`.
Read those; do not read the colour as a status.

---

## 6. How an agent should explain a Sego proof

A calling agent should explain:

- the review ID,
- scope,
- finding count,
- highest severity,
- parse status,
- whether any finding has weak/unverified evidence,
- where to open the Markdown report,
- that Sego is an independent review signal, not an automatic approval.

Suggested wording:

```text
Sego reviewed the latest staged changes and produced review <id>.
It found <n> finding(s); highest severity is <severity>.
Parse status is <parse_status>. Evidence status is attached per finding when available.
Please review the Markdown report before accepting or shipping the changes.
```

---

## 7. Boundaries

A Sego review artifact is a review proof, not a guarantee.

- It does not replace human review.
- It does not replace tests, CI, static analysis, or compliance processes.
- It does not certify that code is safe to ship.
- It may contain model-driven findings that require human disposition.
- A Review Card colour is a presentation grade; `Green` is not acceptance, and not a pass.

Recommended dispositions for each finding (the accepted vocabulary):

```text
open
acknowledged
fixed
accepted_risk
false_positive
ignored
```

These are the values `ReviewFindingStatus::parse` accepts
(`runtime/src/code_review/report.rs:697-711`). Earlier revisions of this contract listed
`confirmed` and `deferred`, and neither is accepted: use `acknowledged` for a confirmed
finding, and `acknowledged` plus an explicit reason for a deferred one.

See `docs/AGENT_REVIEW_HANDOFF.md` for the recommended agent workflow.

---

## 8. What each contract declares, and what it does not

Consumers were left to infer which of these properties a Sego contract actually
provides, and the honest answer differs per contract. The table below is the
current state, not a target: an entry marked **absent** means the contract does
not carry it, and a consumer must not assume it does. This is the consumer-facing
form of the completeness list in `SEGO-BASELINE-ARCHITECTURE.md` §11.3.

| Property | `sego.review.artifact/v1` (rev 2) | `sego.sidecar.envelope` (rev 1) | `sego.review.index-entry` (rev 1) |
|---|---|---|---|
| Version | declared (`schema_version`, `x-contract-revision`) | declared | declared |
| Stable id | declared (`id`) | **absent** — a response has no request id to correlate on | declared (`id`) |
| Provenance | declared (`git_status`, `diff_hash`, engine metadata) | partial (`cwd`, `scope`, `context.diff_hash`) | declared (`json_path`, `markdown_path`) |
| Idempotency | **not an idempotency key.** A repeat is refused with `artifact_id_conflict`, not deduplicated: same-second repeats for the same diff collide by design | **absent** — retrying a request is a new review, not a replay | **absent** |
| Permission | enforced in implementation (sidecar forces ReadOnly), **not declared in the schema** | partially: the envelope is machine-only, but the schema states no permission | **absent** |
| Audit | partial (`invocation_id` when the caller supplies it) | partial (`context.invocation_id` echoed) | **absent** — the index deliberately stays minimal (decision D-5) and carries no invocation id |
| Expiry | **absent** — artifacts do not expire; they are deleted by the operator or not at all | n/a (request/response) | **absent** |
| Failure semantics | partial (`parse_status`, `parse_error`, `parse_repair`) | **partial, and weaker than it looks**: `status: "error"` and `error.{code,message}` exist, but `code` is an open string with **no enumeration** and only `message` is required — so the implemented codes (`unknown_model`, `provider_model_conflict`, `invalid_request`, `unsupported_schema_version`, `artifact_id_conflict`, `review_failed`) are **implementation-only and may be absent from the payload**. `no_diff` *is* declared, but as a `parse_status` value rather than an error code | **absent** |
| Recovery | **absent** — nothing in the contract describes resuming an interrupted review | **absent** | **absent** |
| Compatibility | partial (`x-contract-revision` exists; no migration mechanism and no converter) | partial (same) | partial (same) |
| Deprecation window | **absent** — no revision has been retired and no window is defined | **absent** | **absent** |

Two consequences a consumer should act on rather than discover:

1. **Do not use an artifact as an idempotency key.** `diff_hash` plus scope
   identifies *what* was reviewed, not *an execution of* reviewing it; the same
   input reviewed twice produces two artifacts (or one refused collision) and two
   index lines. Bind executions with the caller's own `context.invocation_id`.
2. **Branch on `status`, not on `error.code`.** The code is not enumerated in
   the contract, so a consumer that switches on it is depending on implementation
   strings; `message` is the only required field. `no_diff` is not an error: it is
   a declared `parse_status` value meaning "this scope had nothing to review", and
   it is explicitly not a pass.
3. **A missing field is not a false one.** `invocation_id`, `resolved_provider`,
   `resolved_model`, `resolved_endpoint` and `identity_evidence_gap` are omitted
   when absent; the response uses `null` for some of them. Treat omitted and
   `null` as the same "not reported" and do not substitute a request-side value.

Filling the **absent** entries is a contract change: it needs a revision bump
here and a coordinated update on the consuming side, not a field added quietly in
the implementation.
