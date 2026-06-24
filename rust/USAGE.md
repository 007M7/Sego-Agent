# Rust Usage Guide

This is the task-oriented usage guide for Sego Agent (Rust implementation). For the full Chinese user guide, see [../docs/Sego使用指南.md](../docs/Sego使用指南.md).

## Install / Build

```bash
# From source
git clone https://github.com/007M7/Sego-Agent.git
cd Sego-Agent/rust
cargo build --release

# Or download latest release:
# https://github.com/007M7/Sego-Agent/releases/latest
```

Set up API credentials:

```bash
# DeepSeek (recommended)
export DEEPSEEK_API_KEY="sk-..."
export DEEPSEEK_MODEL="deepseek-v4-flash"

# Anthropic
export ANTHROPIC_API_KEY="sk-ant-..."
```

## Review Current Changes

```bash
# Review all workspace changes (staged + unstaged)
sego review

# Review staged changes only
git add -A
sego review staged
```

## Full Repository Audit (Clean Clones)

For clean cloned repos or non-Git directories:

```bash
# Audit the full repository directory
sego review --full /path/to/your/repo

# Audit a specific subdirectory
sego review --full ./packages/mylib
```

This walks the entire directory tree, reads key manifest files (Cargo.toml, pyproject.toml, package.json, etc.), entry-point files, and bounded source samples from key directories. It skips `.git`, `node_modules`, virtual envs, build artifacts, and cache directories.

## Where Review Artifacts Are Saved

All reviews produce files under `.sego/reviews/` in the workspace root (for full repo audits, in the target repo):

```
.sego/reviews/
  index.jsonl           # machine-readable index
  review-xxxx.md        # Markdown report
  review-xxxx.json      # JSON findings
```

## Inspect Review Artifacts

```bash
# List all saved reviews
sego review list

# Show a specific review
sego review show <id>

# Check finding status
sego review status <id>

# Mark a finding as fixed
sego review mark <id> <finding-id> fixed
```

## Export Current Conversation vs Latest Response

**Export full conversation:**
```bash
# In REPL
/export
/export ./session.md

# Natural language
导出当前会话
export conversation
```

**Export latest assistant response (review result, code output, etc.):**
```bash
# In REPL — must explicitly say "last/previous/刚才/上一条/上回":
把刚才的审查结果写成 ./review.md
save the last review report to PR43-review.md
export the previous response to report.md
把上一条回复保存到 ./out.md
```

**Safety boundary**: bare phrases like "保存报告" or "导出 md" without a clear target (last/previous/刚才) will show `/dir` guidance instead of exporting unknown content. This prevents accidentally saving the wrong conversation turn.

## Non-Git Directories

If you run `sego review` outside a Git repository:

```bash
# Old behavior: shows "needs a Git project" error
# New behavior (v0.1.8): recovery hint
Review
  Result           failed
  Reason           no Git repository found
  Workspace        ./your-workspace
  Next step        Run `sego review --full <path>` for non-Git directories, or use /dir.

# Use the full repo audit mode instead:
sego review --full .
```

## Update to Latest Version

```bash
# Check for updates
sego update --check

# Install latest release
sego update

# Or download from:
# https://github.com/007M7/Sego-Agent/releases/latest
```

## Natural-Language Action Directory

Say `/dir` to see all available local actions with natural-language examples. Key actions:

| Say this | Result |
|---|---|
| `帮我 review 当前改动` | Code review |
| `切换工作区到 D:\Project` | Switch workspace |
| `把刚才的回复保存到 ./out.md` | Export latest response |
| `检查更新` | Check for updates |
| `退出` | Exit |

If Sego can't determine the action target (e.g., "保存报告" without a path or "导出" without specifying which response), it will show `/dir` guidance.

## Known Limitation

If a review output shows `Parse status: parse_attempted_but_failed` with `Findings unknown (parse failed)`, the model produced findings-like content but the JSON parser could not extract it. Open the Markdown report to inspect the raw output. This is tracked as `C20.5-REVIEW-003` and will be improved in a future release.
