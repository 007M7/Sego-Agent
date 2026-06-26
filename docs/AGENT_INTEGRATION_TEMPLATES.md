# Agent Integration Templates

Copy one of these templates into the instruction file used by your AI coding tool. Adjust paths and commands for your project.

These templates are intentionally generic. They do not depend on a private Sego setup or maintainer machine path.

---

## 1. Minimal `AGENTS.md` snippet

````md
## Sego review handoff

After generating or modifying code, especially before a PR or release, run an independent Sego review when possible:

```bash
sego /review staged
```

If changes are not staged, use:

```bash
sego /review workspace
```

After the review, prefer:

```bash
sego review show latest --json
```

Then summarize:

- review id,
- scope,
- finding count,
- highest severity,
- parse status,
- top findings with file/line evidence,
- Markdown report path.

Do not claim Sego approved the code. Sego is an independent review signal, not a guarantee. Ask the user whether each finding is confirmed, false positive, accepted risk, or deferred.
````

---

## 2. Claude / Codex / Cursor-style instruction snippet

```text
When you complete a non-trivial code change, run Sego review before presenting the change as ready.

Preferred command:
  sego /review staged

If there are unstaged workspace changes:
  sego /review workspace

Then run `sego review show latest --json` and parse the returned summary. If unavailable, read the latest entry from `.sego/reviews/index.jsonl` and parse the referenced JSON artifact.
Explain the result to the user with:
  - review id
  - finding count
  - highest severity
  - parse status
  - top findings and evidence
  - Markdown report path

Never say "Sego approved this". Say "Sego produced a review proof" and ask the user how to handle findings.
```

---

## 3. Risk-trigger snippet

```text
If a change touches authentication, authorization, billing, database migrations, secrets, dependency versions, filesystem access, network calls, or data-flow logic, run Sego before marking the task complete.
```

---

## 4. Finding disposition snippet

```text
For each Sego finding, record one disposition:

- confirmed: valid finding, fix or add tests
- false_positive: not valid, explain why
- accepted_risk: valid risk accepted by user
- deferred: valid or unresolved, moved to later task

Do not silently drop findings.
```

---

## 5. Review artifact contract

For field meanings and stable/experimental status, see:

```text
docs/REVIEW_ARTIFACT_CONTRACT.md
```

For the recommended handoff workflow, see:

```text
docs/AGENT_REVIEW_HANDOFF.md
```
