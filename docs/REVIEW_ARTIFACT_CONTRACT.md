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
| `data_egress_class`, `compute_boundary`, `budget` | **stable-additive, declared at revision 3** | Where the data went, where the work ran, and what budget it ran under. **Absent means `unknown`, not `none`** — see below. The engine is the only writer of the observed values. |
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

### `data_egress_class`, `compute_boundary`, `budget`

Declared at contract revision 3, from `SEG-ADR-004` (数据外发类别、算力边界与预算). Sego's public
positioning is local-first, while a review has to send the diff to a model provider to reach a
conclusion. Both can be true, but only once it is said clearly what leaves the machine. That is what
these three fields are for.

| field | values | meaning |
|---|---|---|
| `data_egress_class` | `none` / `provider` / `provider_and_fetch` / `unknown` | where the reviewed content or fetched results **actually went** |
| `compute_boundary` | `local` / `remote` / `unknown` | where the work **actually ran** |
| `budget` | object | `declared` = what the caller permitted; `actual` = what the engine observed |

**They are two orthogonal axes, not one enum.** Local inference that fetched a page is `local`
compute with `provider_and_fetch` egress, and no single enum expresses that combination.

**The values are observed, never read from configuration.** A config that names a local model while
the call went to a remote provider must not be recorded as `none`. Filling these from configuration
would be fabricating evidence.

**Absent is `unknown`, and `unknown` is not `none`.** This is the hard requirement of the decision,
because treating the two as the same is how "local-first" gets over-claimed: a consumer that reads an
`unknown` artifact as "reviewed locally" has made a claim Sego did not make. A consumer must
therefore treat a missing field as `unknown` and decide for itself whether to refuse, degrade, or ask
for more evidence.

**The consumer's decision, recorded.** The Verification Authority — the only consumer of this
contract today — chose to **refuse**: any value outside `{none, provider, provider_and_fetch}` ×
`{local, remote}`, *including an explicit `unknown`*, and a missing field, all fail as
`InconsistentArtifact` and the whole artifact is rejected. Its stated reason is that a governance
consumer cannot accept an unattributable egress claim, and that a missing field means the producing
path did not observe at all. Two consequences follow, both intended:

- **An artifact written by the plain CLI path is not consumable.** That path does not observe a run,
  so it omits the fields, so this consumer refuses it. Only the governed `sego sidecar review` path
  produces artifacts this consumer accepts. This is the fail-closed direction working, not a defect —
  but a second consumer that wants the findings without the egress claim must decide differently
  rather than inherit this behaviour by accident.
- **An artifact whose endpoint Sego cannot name is refused too.** Today all four providers have
  base-URL accessors, so `unknown` is unreachable in practice; if that changes, the review runs and
  persists but will not be accepted downstream, and that will be visible rather than silent.

**The engine is the only writer of the observed values.** A caller may declare an intent ceiling;
it cannot declare what actually happened. When the two disagree, the observed value is what goes
into the artifact **and the disagreement is recorded** — that is the event worth seeing, not one to
smooth over.

**What is emitted today, and what is not.** The governed sidecar path
(`sego sidecar review`) writes `data_egress_class` and `compute_boundary`, both derived from the turn
that ran: the request count is the conversation loop's own iteration count, the fetch count is the
tool calls that actually returned, and the endpoint is the one this invocation resolved. An endpoint
Sego cannot name is written as `unknown` rather than left out, so a consumer never has to infer an
observation gap from a missing field.

`budget` is **declared but not produced**: the object requires a declared ceiling and an observed
usage together, and no caller of the governed path declares a ceiling yet. Writing the observed half
alone is what the ADR rules out, so the artifact carries no budget object and reads as `unknown` —
the truthful answer to "was this review limited?", which is that nobody said.

The plain CLI review path (`sego review`) does not observe a run and omits all three. That is the
distinction the field's absence carries: absent means the writing path does not observe, `unknown`
means it observed and could not determine. A consumer reads both as unknown, and neither as `none`.

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
| Version | declared (`schema_version`) | declared (`schema_version`) | **contract-level only**: the schema root carries `x-contract-revision`, but an index entry itself has no version field - a consumer reading a line cannot tell which revision wrote it |
| Revision | declared (`x-contract-revision`), and it is the field a consumer must branch on when two revisions are in circulation - `schema_version` does **not** change for every contract revision | declared (`x-contract-revision`) | declared (`x-contract-revision`) |
| Stable id | declared (`id`) | partial — a response carries `review_id` always and echoes `invocation_id` when the caller supplied one; the **request** has no server-minted id, so correlation exists only if the caller binds one | declared (`id`) |
| Provenance | declared (`git_status`, `diff_hash`, engine metadata) | partial (`cwd`, `scope`, `context.diff_hash`) | declared (`json_path`, `markdown_path`) |
| Idempotency | **not an idempotency key.** A repeat is refused with `artifact_id_conflict`, not deduplicated: same-second repeats for the same diff collide by design | **absent** — retrying a request is a new review, not a replay | **absent** |
| Permission | enforced in implementation (sidecar forces ReadOnly), **not declared in the schema** | partially: the envelope is machine-only, but the schema states no permission | **absent** |
| Audit | partial (`invocation_id` when the caller supplies it) | partial (`context.invocation_id` echoed) | **absent** — the index deliberately stays minimal (decision D-5) and carries no invocation id |
| Expiry | **absent** — artifacts do not expire; they are deleted by the operator or not at all | n/a (request/response) | **absent** |
| Failure semantics | partial (`parse_status`, `parse_error`, `parse_repair`) | **partial, and weaker than it looks**: `status: "error"` and `error.{code,message}` exist, but `code` is an open string with **no enumeration** and only `message` is required — so the implemented codes (`unknown_model`, `provider_model_conflict`, `invalid_request`, `unsupported_schema_version`, `artifact_id_conflict`, `review_failed`) are **implementation-only and may be absent from the payload**. `no_diff` *is* declared, but as a `parse_status` value rather than an error code | **absent** |
| Recovery | **not a contract concern, by decision** — resuming an interrupted review is carried by the runtime (`.sego/recovery`, and the per-run ledger at `.sego/runtime/active_task.json`), not by the artifact. The artifact is written after the work, so there is nothing in it to resume from | same | **absent** — the index is append-only and holds no resumable state |
| Compatibility | partial — `x-contract-revision` exists; the mechanism is **a revision bump plus a coordinated update on the consuming side, with a migration window written down before the bump** (see the note below). No converter is provided, and that is the policy rather than an omission: a converter that silently reshapes an old artifact would be a second implementation of this contract | partial (same) | partial (same) |
| Deprecation window | **none is open, and the trigger is stated** — no revision has been retired, so no window is running. Retiring one requires the three things in the note below *before* the bump, not after | same | same |

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
the implementation. **Revision 3 did exactly that for a capability rather than a
missing entry**: `data_egress_class`, `compute_boundary` and `budget` were declared, and
the twelve properties above are unchanged by it.

### Revision bumps, migration windows and retirement

This is the mechanism the Compatibility and Deprecation-window rows refer to.
It is written here because it is a contract term, and leaving it implicit means
each revision bump is improvised.

1. **Classify the change before making it.** An additive optional field, a
   tightened validation, the removal of an unimplemented field, an enum change
   and a semantic change are five different things. A semantic change is
   incompatible even when `schema_version` stays `1`, because a consumer cannot
   see it from the version number.
2. **Write the migration window down before the bump, not after.** It states
   either which existing references need migrating and by when, or that there
   are none and how that was checked. "No window was needed" is a conclusion
   with evidence, not an omission - the empty window recorded for
   `sego.review.artifact/v1` rev 2 was accepted on exactly that basis.
3. **A retirement needs three things before it starts:** the list of consumers
   affected, the result of migrating them, and an end date for the window. Until
   all three exist, the revision is not retired and no window is open.
4. **Do not provide a converter.** An old artifact is evidence of what was
   reviewed under the contract that was in force; reshaping it into the current
   shape makes it a second, unreviewed claim about the same work. Consumers that
   must read both revisions should branch on `x-contract-revision` explicitly.
