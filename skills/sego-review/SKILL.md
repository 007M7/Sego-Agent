---
name: sego-review
version: "0.1.0"
description: >
  AI coding engineering trust review. After the user generates or modifies code
  with any AI tool, invoke Sego to perform a structured review. Sego returns
  findings with severity/file/line/evidence/risk/suggestion and persists a review
  artifact to .sego/reviews/. Use this skill whenever the user wants to check,
  review, or validate AI-generated code changes before committing or shipping.
---

# Sego Review Skill

## When to use this skill

Use this skill when the user:
- Asks to "review", "check", or "validate" code changes
- Wants to know if AI-generated code is safe to commit
- Asks "is this code okay?" or "should I commit this?"
- Wants a second opinion on AI-generated diffs

Do NOT use this skill for:
- General coding questions (use normal tools)
- Running tests (use cargo test / npm test directly)
- Git operations (use git directly)

## How to invoke

Call the Sego sidecar via the platform-appropriate script:

### Unix / macOS

```bash
echo '{"schema_version":1,"action":"review","cwd":"'"$PWD"'","scope":"staged"}' \
  | bash skills/sego-review/scripts/sego-review.sh
```

### Windows (PowerShell)

```powershell
$request = @{ schema_version = 1; action = "review"; cwd = (Get-Location).Path; scope = "staged" } | ConvertTo-Json
$request | powershell -File skills/sego-review/scripts/sego-review.ps1
```

### Request format

```json
{
  "schema_version": 1,
  "action": "review",
  "cwd": "/absolute/path/to/project",
  "scope": "staged",
  "options": { "model": null },
  "context": { "user_intent": "optional description of what the user wants" }
}
```

- `scope`: `"staged"` (git staged changes), `"workspace"` (all uncommitted changes), or a path like `"src/auth"`.
- `options.model`: override the review model (default: auto-selected).

### Response format

```json
{
  "schema_version": 1,
  "status": "ok",
  "review_id": "rev-20260616-001",
  "diff_hash": "sha256:...",
  "artifact_path": ".sego/reviews/rev-20260616-001.json",
  "findings": [
    {
      "id": "f-001",
      "severity": "high",
      "file": "src/auth/login.rs",
      "line": 42,
      "title": "Missing error handling",
      "evidence": "...",
      "risk": "...",
      "suggestion": "...",
      "confidence": 0.85
    }
  ],
  "parse_status": "structured"
}
```

Severity levels: `critical` > `high` > `medium` > `low` > `info`.

## Interpreting results

After receiving the response:
1. Summarize the findings for the user in plain language
2. Highlight any `critical` or `high` severity findings first
3. If `status` is `"error"`, report the error message to the user
4. Mention the `artifact_path` so the user can review the full report later

## Requirements

- Sego must be installed and on PATH, or built from source (`cargo build -p rusty-claude-cli`)
- The project must be a git repository (review uses git diff)
