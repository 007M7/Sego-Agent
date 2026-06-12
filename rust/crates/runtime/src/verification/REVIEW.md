# Sego Code Review — Cycle 2 `/verify` MVP

**Reviewer:** Sego Agent (DeepSeek v4 Pro)
**Date:** 2026-06-12
**Commit:** feat(review): slash-verify-trust-phase1 / verify MVP

**Reviewed files:**

1. `rust/crates/runtime/src/verification/mod.rs`
2. `rust/crates/runtime/src/verification/scope.rs`
3. `rust/crates/runtime/src/verification/plan.rs`
4. `rust/crates/runtime/src/lib.rs`
5. `rust/crates/commands/src/lib.rs`
6. `rust/crates/rusty-claude-cli/src/main.rs`

**Verification baseline:**
- `cargo test -p runtime verification` → **8 passed, 0 failed**
- `cargo test -p rusty-claude-cli parses_direct_agents_mcp_and_skills_slash_commands` → **passed**
- `cargo test -p commands renders_help_from_shared_specs` → **passed**
- `cargo build -p rusty-claude-cli` → **passed**
- `sego /verify fast` (worktree root) → **passed** (1/1 commands, exit 0)
- `sego /verify auto` (worktree root) → **build passed**, tests failed (known pre-existing api credential test issue — not related to /verify)

---

## Findings（按严重度排序）

### Finding 1 — `run_verification_command` 失败时返回 `Err`，触发误导性 CLI usage 提示

| Field | Value |
|---|---|
| **Severity** | 🟡 Medium |
| **File** | `rust/crates/rusty-claude-cli/src/main.rs` Lines 3959-3960 |
| **Evidence** | `if failed { return Err("verification failed".into()); }` followed by `main()` 中的 `eprintln!("error: {message}\n\nRun `sego --help` for usage.")` |
| **Risk** | 当 2+ 个命令执行后首个命令失败时，用户看到后续命令的输出仍在打印，然后突然出现 `error: verification failed` 跟着 `Run sego --help for usage.`——这是误导性的，因为错误**不是** CLI 用法错误。 |
| **Suggestion** | 失败时不要返回 `Err`，改为 `return Ok(());` 让用户根据打印输出自行判断。当前 `Err` 会触发 `Run sego --help for usage.` 的后缀，与实际情况不符。 |
| **Confidence** | High |
| **Verification hint** | 在 worktree root 运行 `sego /verify auto` → 观察 `failed` 结果后的最后一行是否显示 `Run sego --help for usage.` |

### Finding 2 — 输出截断可能丢失错误诊断信息

| Field | Value |
|---|---|
| **Severity** | 🟡 Medium |
| **File** | `rust/crates/rusty-claude-cli/src/main.rs` Lines 4024-4035 (`summarize_command_output`) |
| **Evidence** | 对去重后的非空行取 `take(6)` 并截断到 `take(500)` 字符。对于 `cargo test` 输出，当有多个 crate 的编译警告在测试失败之前时，前 6 行 stderr 显示的是编译状态，而非测试失败的诊断信息。 |
| **Risk** | `/verify auto` 失败时 stdout 显示类似 `running 51 tests | test client::tests::... ok | ...`——被截断的 stdout 无法让用户看到具体哪些测试失败。stderr 显示的是编译信息而非测试失败。用户无法判断验证失败的**原因**，必须手动运行 `cargo test`。 |
| **Suggestion** | 对于 `failed` 结果，增加行数限制（例如 20 行）或显示**最后** N 行而非前 N 行。或者将完整输出打印到临时文件并提示用户：`Full output at .sego/verify-output.txt`。 |
| **Confidence** | Medium |
| **Verification hint** | 制造一个测试失败然后观察截断的输出是否省略了实际失败行。 |

### Finding 3 — `ExePath`（`sego.exe` 路径）依赖 `env::current_exe()`

| Field | Value |
|---|---|
| **Severity** | 🟢 Low |
| **File** | `rust/crates/rusty-claude-cli/src/main.rs` Lines 3945-3952（通过 `println!` + `run_verification_command`） |
| **Evidence** | N/A — 仅为观察记录 |
| **Risk** | N/A — 无实际问题。`env::current_exe()` 用于诊断输出的路径解析——这是正确的做法。 |
| **Suggestion** | 无需操作。路径解析完全正确。 |
| **Confidence** | High |
| **Verification hint** | N/A |

### Finding 4 — `build_verification_plan` 的 cwd 依赖是隐式的

| Field | Value |
|---|---|
| **Severity** | 🟢 Low |
| **File** | `rust/crates/runtime/src/verification/plan.rs` Lines 79-91 |
| **Evidence** | `build_verification_plan` 隐式使用 `&Path`（cwd）。如果调用者传入错误的路径，生成的 plan 也会错误。无法通过环境变量覆盖。 |
| **Risk** | 当前唯一调用者是 `run_code_verify_cli`，它传入 `env::current_dir()`，在当前用例下是正确的。但如果将来有其他调用者（REPL、adapter 等）未传入正确的 cwd 就调用 `build_verification_plan`，可能会生成错误的 plan。 |
| **Suggestion** | 不阻塞 MVP。后续如有需要可添加 `--cwd` 标志或支持 `SEGO_PROJECT_DIR` 环境变量覆盖。 |
| **Confidence** | Medium |
| **Verification hint** | N/A — 取决于未来用例。 |

---

## Summary

| Severity | Count |
|---|---|
| 🔴 High | 0 |
| 🟡 Medium | 2 |
| 🟢 Low | 2 |

**总体评价：** 代码质量高。实现干净、范围合理，遵循现有代码库的模式（slash 命令注册、re-exports、模块结构）。测试覆盖了关键路径。两个中等严重度的 findings 是可用性问题（错误消息和输出截断），而非正确性 bug。两者都不是 MVP 合并的阻断项——可以作为后续补丁处理。

**推荐：** ✅ 可以合并。Findings 1 和 2 应在发布给用户之前解决，但不阻塞 PR。
