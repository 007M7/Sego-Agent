# sego-review skill

AI coding engineering trust review — a Sego sidecar skill package.

## What it does

After you generate or modify code with any AI tool (Claude Code, Codex, Cursor, etc.), invoke this skill to get a structured review from Sego. Sego returns findings with severity, file, line, evidence, risk, and suggestion — then persists a review artifact to `.sego/reviews/`.

## Install

### Prerequisites

- [Sego](https://github.com/007M7/Sego-Agent) built and on PATH, or built from source:
  ```bash
  cargo build -p rusty-claude-cli
  ```
- A git repository (review uses `git diff`)

### Use in any SKILL.md-compatible AI coding tool

Copy the `sego-review/` directory into your tool's skills directory.

## Usage

### Command line

```bash
# Unix
echo '{"schema_version":1,"action":"review","cwd":"'"$PWD"'","scope":"staged"}' \
  | bash skills/sego-review/scripts/sego-review.sh

# Windows PowerShell
$request = @{ schema_version = 1; action = "review"; cwd = (Get-Location).Path; scope = "staged" } | ConvertTo-Json
$request | powershell -File skills/sego-review/scripts/sego-review.ps1
```

### Via `sego sidecar review` directly

```bash
echo '{"schema_version":1,"action":"review","cwd":"/project","scope":"staged"}' | sego sidecar review
```

## Request format

| Field | Type | Required | Description |
|---|---|---|---|
| `schema_version` | integer | yes | Must be `1` |
| `action` | string | yes | Must be `"review"` |
| `cwd` | string | yes | Absolute path to project root |
| `scope` | string | no | `"staged"`, `"workspace"`, or a path (default: `staged`) |
| `options.model` | string | no | Override review model |
| `context.user_intent` | string | no | What the user wants to achieve |

## Response format

See [SKILL.md](SKILL.md) for the full response schema. Key fields:

- `status`: `"ok"` or `"error"`
- `findings`: array of structured findings (severity/file/line/evidence/risk/suggestion)
- `artifact_path`: path to persisted review JSON
- `error`: present only when `status` is `"error"`

## Graceful degradation

If the Sego binary is not found, the script outputs a structured error JSON to stderr and exits with code 1 — it does not crash silently.
