# Sego Review Artifact Contract

This document describes the public, agent-readable review proof artifacts written by Sego under `.sego/reviews/`.

Sego is not a code generator. Sego is an independent review/proof engine that can be called after another AI coding agent generates or changes code. The review artifact is the handoff object that another agent can read and explain to a user.

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

## 2. Which file should an agent read?

For an agent integration, the recommended read path is:

1. Read `.sego/reviews/index.jsonl`.
2. Select the last non-empty line.
3. Read `json_path` from that index entry.
4. Parse that JSON artifact.
5. Explain the summary and findings to the user.

Agents should use the JSON artifact for machine-readable data and link to the Markdown report for human details.

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
| absent / null | Legacy artifact or no deterministic evidence status attached. |

### `highest_severity`

The highest severity among findings, or `null` if there are no findings. Supported values:

```text
critical, high, medium, low, info
```

### `reviewer`, `engine_version`, `review_mode`

These identify the local review engine and mode that produced the artifact. They are useful for attribution and debugging.

They are **not** a cryptographic signature, provenance attestation, or certification. Future releases may add signing/provenance separately.

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

Recommended dispositions for each finding:

```text
confirmed
false_positive
accepted_risk
deferred
```

See `docs/AGENT_REVIEW_HANDOFF.md` for the recommended agent workflow.
