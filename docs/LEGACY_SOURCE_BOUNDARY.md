# Legacy Source Boundary

This repository may have historical or legacy source material outside the canonical development worktree. Development Cutover uses Git-controlled integration only; it does not copy dirty legacy trees into the current source.

## Canonical local development entry

The canonical local development entry after cutover is expected to be a clean `develop` worktree under:

```text
E:\Sego max\08_code\worktrees\sego-agent-develop
```

All future code changes should use a focused feature/fix branch or active worktree, then merge back into `develop` after review and verification.

## Legacy source rule

Legacy source trees may be used only as read-only forensic or design reference unless a separate work item authorizes a small, reviewed reimplementation.

Do not:

- copy a legacy dirty tree into the canonical worktree;
- use `.gitignore` to hide unreviewed external repositories or generated source candidates;
- treat a legacy diff as accepted code without a focused branch, tests, review, and merge record;
- delete, clean, reset, or move external repositories as part of Sego source governance.

## Disposition vocabulary

| Disposition | Meaning |
|---|---|
| `reimplemented` | The legacy intent was rebuilt in a clean branch and verified. |
| `retained-boundary` | The legacy material remains tracked or referenced with an explicit support boundary. |
| `blocked` | The item is tied to release metadata, public claims, or owner approval and must not be moved by cutover. |
| `deferred` | The item may become a future work item but is not required for local Development Cutover. |
| `not-migrated` | The legacy file or directory is not copied into the canonical source. |

Development Cutover completion means the local development entry is clean and governed. It does not mean every legacy idea became product code.
