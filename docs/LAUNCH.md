# Sego Launch

Sego is looking for the first 20 AI coding users.

If you use Cursor, Claude Code, Codex, Copilot, Lovable, or another AI coding tool, you can send us a small AI-generated project. We will run a Sego review and send back a structured report.

## What we review

- Security risks such as SQL injection, hardcoded secrets, unsafe commands, and credential leaks
- Code quality issues that can block shipping
- Unexpected changes made by AI coding tools
- Whether the current diff looks ready to commit or needs another pass

## Free Sego Audit

The free audit is for small public projects or small diffs.

You receive:

- A structured Sego review summary
- The highest-risk findings
- Suggested next steps before commit or release

Apply here:

https://github.com/007M7/Sego-Agent/issues/new?assignees=&labels=free-audit,pending-review&template=free-sego-audit.yml&title=%5BFree+Audit%5D+

Public issue warning:

Do not include secrets, credentials, private customer data, or private source code in a public GitHub issue.

## Private AI Code Audit

For private projects, paid audits start at:

- $19 / RMB 99 for a basic private audit
- $99 / RMB 699 for a launch check

Request scope confirmation here:

https://github.com/007M7/Sego-Agent/issues/new?assignees=&labels=private-audit,pending-scope&template=private-audit.yml&title=%5BPrivate+Audit%5D+

Do not upload private code before the workflow is confirmed.

## Why this exists

AI coding tools can write code quickly. The open question is whether the generated code is safe enough to ship.

Sego sits after AI coding tools:

```text
AI writes code.
Sego reviews whether it is safe enough to commit.
```

This launch page is the first step toward a productized Sego workflow:

- Local-first code review
- Review artifacts
- First-user audits
- Case studies
- Later GitHub PR bot and team dashboard
