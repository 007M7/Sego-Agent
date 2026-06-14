# Sego Review MVP 演示与验收清单

本文档用于演示当前公开 MVP 的提交前工程信任闭环。它面向开发者、维护者和协作 agent，帮助他们快速理解 Sego 在 AI Coding 工作流中的位置。

Sego 的当前 MVP 不强调“替你写更多代码”，而是强调在 AI 生成代码之后补齐审查、验证、记录和交付证据。

```text
AI 生成代码 -> Git 明确范围 -> Sego 审查与安全锁 -> Sego 验证证据 -> PR / commit 交付
```

## 演示目标

一次完整演示应证明：

- Sego 能识别本次准备提交的 staged 范围。
- Sego 能在不修改文件的情况下给出 readiness 和 delivery summary。
- Sego 能对 staged 文件做本地安全锁检查。
- Sego 能对 staged diff 发起结构化 review。
- Sego 能保存 review 历史，并允许后续查看和标记 finding 状态。
- Sego 能显式执行 fast verification，作为提交或 PR 的证据。

## 前置条件

```bash
git status --short
sego --version
sego doctor
```

建议演示前确认：

- 当前目录是一个 Git 仓库。
- 已配置可用的模型 API key，或已经完成 `sego login`。
- 本地已安装项目所需的基础构建工具，例如 Rust 项目的 `cargo` 或 Node 项目的 `npm`。
- 当前工作区没有无关改动，或已经明确本次演示只处理哪些文件。

## 推荐演示脚本

### 1. 制造或选择一组 AI 生成改动

可以使用任意 AI 编码工具完成一组小改动，例如：

- 修复一个小 bug。
- 新增一个小函数。
- 更新一段 README 或测试。
- 调整一个 CLI 输出文案。

Sego 不要求代码一定由 Sego 生成。它可以审查 Claude Code、Codex、Cursor、OpenHands 或开发者手写的改动。

### 2. 明确提交范围

```bash
git status --short
git add <files>
git diff --cached --stat
```

这一步的关键是明确 staged 范围。Sego 的 staged review、staged safety 和 readiness gate 都围绕 Git 暂存区建立边界。

### 3. 查看提交前 readiness gate

```bash
sego /review ready
```

预期结果：

- 输出 `Review Readiness`。
- 显示 staged 文件数量。
- 显示 staged safety lock 是否通过。
- 显示 `/verify fast` 的计划命令。
- 显示下一步建议。

只读边界：

- 不调用模型。
- 不执行 build/test。
- 不安装依赖。
- 不修改文件。

### 4. 生成交付摘要

```bash
sego /review summary
```

预期结果：

- 输出 `Review Summary`。
- 显示 Git branch/status。
- 显示 staged diff / unstaged diff 是否存在。
- 显示 staged safety 结果。
- 显示最近一次 review 历史。
- 显示 `/verify fast` 的计划。
- 显示建议下一步命令。

这个输出适合复制到 PR、工作日志、agent handoff 或人工复盘中。

### 5. 运行 staged 安全锁

```bash
sego /review safety staged
```

预期结果：

- 没有明显风险时输出 clean/passed 类结果。
- 发现风险时列出文件位置、证据、风险说明和建议。

当前安全锁重点覆盖：

- 疑似密钥文件。
- 硬编码密钥。
- 危险 shell 命令。
- 本机绝对路径。
- 其他新手容易直接提交的明显风险。

### 6. 发起 staged code review

```bash
sego /review staged
```

预期结果：

- 对 staged diff 发起只读模型审查。
- 输出 review report ID。
- 尽量结构化 findings，包括严重程度、位置、证据、风险和建议。
- 持久化 markdown/json/index 记录到本地 `.sego/reviews`。

注意：这一步会调用模型，但仍应保持 read-only，不应修改代码。

### 7. 查看 review 历史

```bash
sego /review list
sego /review show <review-id>
sego /review status <review-id>
```

预期结果：

- `/review list` 能看到最新 review。
- `/review show` 能打印持久化 markdown 报告。
- `/review status` 能展示 finding 当前状态。

### 8. 标记 finding 处理状态

```bash
sego /review mark <review-id> <finding-id> fixed "已按建议修复并验证"
sego /review status <review-id>
```

可用状态：

- `open`
- `acknowledged`
- `fixed`
- `ignored`

这一步用于把 review 从“一次聊天建议”变成可追踪的工程记录。

### 9. 执行快速验证

```bash
sego /verify fast
```

预期结果：

- Sego 根据项目类型执行 fast verification。
- Rust 项目通常会计划 `cargo build` 或相关快速命令。
- Node 项目通常会依据 package scripts 计划 test/build 类命令。

`/review ready` 和 `/review summary` 只展示验证计划；真正执行必须由 `/verify fast` 明确触发。

## 验收清单

合格的 Review MVP 演示应满足：

- [ ] `sego /review ready` 可以在 staged 或 empty staged 状态下稳定输出。
- [ ] `sego /review summary` 可以生成只读交付摘要。
- [ ] `sego /review safety staged` 不会扫描无关工作区文件。
- [ ] `sego /review staged` 能产生 review report 或明确说明没有可审查改动。
- [ ] `sego /review list` 能看到持久化记录。
- [ ] `sego /review show <id>` 能回放 markdown 报告。
- [ ] `sego /review status <id>` 能展示 finding 状态。
- [ ] `sego /review mark <id> <finding-id> fixed` 能更新状态。
- [ ] `sego /verify fast` 能明确执行验证计划。
- [ ] 演示结束后 `git status --short` 中没有非预期产物被提交。

## PR 交接建议

PR 描述可以包含：

```markdown
## Sego Review Evidence

- Readiness: `sego /review ready`
- Summary: `sego /review summary`
- Safety: `sego /review safety staged`
- Review: `sego /review staged`
- Verification: `sego /verify fast`
```

如果存在 review finding，建议补充：

```markdown
## Review Findings

- `<finding-id>`: fixed / acknowledged / ignored
- Evidence: `<brief note>`
```

## 当前边界

当前 MVP 仍是本地优先的工程信任运行时，不替代：

- 人工代码审查。
- CI/CD 的正式门禁。
- SAST/DAST 等专业安全工具。
- 语言生态中的 lint、formatter、type checker。

Sego 更适合作为 AI Coding 和 Git/CI 之间的一层本地信任补丁：先在个人开发阶段暴露风险、整理证据，再进入正式协作流程。
