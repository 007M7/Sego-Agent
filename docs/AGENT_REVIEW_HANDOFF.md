# Agent Review Handoff Playbook

This document explains when and how an AI coding agent should call Sego and how it should present Sego review proof to a user.

Sego is an independent proof engine. It is meant to run **after** an agent generates or changes code, and **before** the user accepts risk, opens a PR, or ships.

---

## 1. When to call Sego

An AI coding agent should consider calling Sego:

- after generating or modifying code,
- before opening a pull request,
- before a release or deployment step,
- after a major refactor,
- after changing authentication, authorization, billing, database, filesystem, network, dependency, or security-sensitive code,
- after changing tests that are intended to prove safety,
- when a task file explicitly requires `/review staged`, `/review workspace`, or an allowlisted full review.

Sego should not be called as a substitute for tests or human judgment. It is an additional independent review signal.

---

## 2. Recommended command flow

### Review staged changes

```bash
sego /review staged
```

Use when the agent has staged a focused change.

### Review workspace changes

```bash
sego /review workspace
```

Use when changes are present but not staged.

### Review a repository snapshot

```bash
sego review --full /path/to/project
```

Use for clean cloned repositories or non-Git directories. This is a manifest / entry-point / context snapshot review, not exhaustive static analysis.

---

## 3. How to read the latest proof

Preferred machine-readable path:

```bash
sego review show latest --json
```

This prints a stable JSON summary and does not include Markdown, color codes, or explanatory prose.

Fallback path (for older Sego versions):

1. Open `.sego/reviews/index.jsonl`.
2. Read the last non-empty line.
3. Parse `json_path` and open the referenced JSON artifact.
4. Use the artifact fields documented in `docs/REVIEW_ARTIFACT_CONTRACT.md`.
5. Link the user to `markdown_path` for human-readable details.

If `index.jsonl` is missing, or `sego review show latest --json` returns `found: false`, no review artifact has been written in this workspace yet.

---

## 4. What to summarize to the user

A calling agent should present the result in a short, conservative form:

```text
Sego review: <id>
Scope: <scope>
Findings: <finding_count>
Highest severity: <highest_severity or none>
Parse status: <parse_status>
Report: <markdown_path>
```

Then list only the most important findings, preserving file/line/evidence details.

Example:

```text
Sego produced review review-1782600000-ab12cd34ef56.
It found 2 findings; highest severity is high.
The most important finding is in src/auth.rs:42: missing authorization check before account update.
Please review .sego/reviews/review-1782600000-ab12cd34ef56.md before accepting or shipping the change.
```

---

## 5. Finding disposition template

When a user responds to a finding, keep the disposition explicit:

```text
Finding: <finding id or title>
Disposition: confirmed | false_positive | accepted_risk | deferred
Reason: <short explanation>
Next action: <fix now | add test | document risk | defer to ticket>
Owner: <optional>
```

Suggested meanings:

| Disposition | Meaning |
|---|---|
| `confirmed` | The finding is valid and should be fixed or tested. |
| `false_positive` | The finding is not valid; record why. |
| `accepted_risk` | The finding is valid, but the user intentionally accepts the risk. |
| `deferred` | The finding is valid or unresolved, but action is moved to a later task. |

Do not silently ignore Sego findings. If the user decides not to fix one, record the reason.

---

## 6. Standard prompt snippets for AI coding agents

### After code generation

```text
I changed the code. Before I ask you to accept it, I will run Sego review on the changed scope and summarize the review proof. I will not treat Sego as an automatic approval; I will show findings and ask you to decide what to do.
```

### Before PR

```text
Before opening a PR, run Sego on the staged changes. Then call `sego review show latest --json` and summarize finding count, highest severity, parse status, and the top findings with file/line evidence. If the command is unavailable, fall back to reading the latest `.sego/reviews/index.jsonl` entry and opening the referenced JSON artifact.
```

### After risky changes

```text
This change touches a sensitive area (auth, billing, database, dependency, filesystem, network, or data flow). Run Sego review before presenting the change as ready.
```

---

## 7. What not to claim

A calling agent must not claim:

- "Sego approved this code."
- "Sego guarantees this is safe."
- "All issues are fixed" unless the fixes and tests are actually present.
- "No risk" when `parse_status` is `fallback_raw_text` or `parse_attempted_but_failed`.
- "Verified" unless evidence is present in the captured review target and the artifact says so.

Safer wording:

```text
Sego found no structured findings in this run, but this is not a guarantee. Please still review tests and behavior before shipping.
```

---

## 8. Integration templates

External users can copy the examples in `docs/AGENT_INTEGRATION_TEMPLATES.md` into an `AGENTS.md`, `CLAUDE.md`, Cursor rule file, or another AI coding tool instruction file.
