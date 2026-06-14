<h1>Sego <img src="assets/sego-ui-icon.png" width="34" height="34" alt="Sego 图标" align="right"></h1>

<p align="center">
  <strong>AI Coding 的工程信任运行时</strong><br>
  让 AI 生成的代码变得可审查、可验证、可复盘、可交付。
</p>

<p align="center">
  <a href="#快速开始"><img src="https://img.shields.io/badge/快速开始-5分钟-blue?style=flat-square" alt="快速开始"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/许可证-MIT-green?style=flat-square" alt="MIT 许可证"></a>
  <img src="https://img.shields.io/badge/Rust-原生-orange?style=flat-square" alt="Rust 原生">
  <img src="https://img.shields.io/badge/平台-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey?style=flat-square" alt="支持平台">
</p>

---

## Sego 是什么

Sego 是面向 AI Coding 时代的工程信任工具。它不是再做一个“替你写更多代码”的 Agent，也不是传统 IDE；它更像一层开发工作流的本地信任运行时，帮助开发者判断 AI 写出的代码是否值得进入提交、合并和交付。

你可以继续使用 Claude Code、Codex、Cursor、OpenHands 或其他 AI 编码工具生成代码。Sego 负责在代码进入 Git、PR 或交付之前，补上工程化的审查、验证、记录和交接证据。

一句话：

```text
AI 负责生成，Sego 负责证明它值得被合并。
```

---

## 为什么需要 Sego

AI 编码工具已经能快速生成代码，但真实开发中的风险往往出现在后半段：

- 修完一个 bug 后，相关数据链路、调用链、测试和文档没有同步对齐。
- AI 只关注当前 diff，容易忽略历史上下文、项目约束和已接受规则。
- 代码 review 只给自然语言建议，缺少风险等级、证据、状态和可追踪记录。
- 测试通过与否缺少结构化沉淀，下一次仍然从头判断。
- 单次对话越来越长，项目经验无法稳定复用，也很难交接给另一个 Agent 或开发者。

Sego 希望把这些问题收敛成一个本地优先、可审计、可持续演进的工程闭环：

```text
生成代码 -> 审查改动 -> 验证证据 -> 沉淀上下文 -> 辅助交付
```

---

## 当前 MVP 能做什么

当前开源版本仍处在 MVP 迭代阶段，重点是把 review、verify 和本地工程记忆跑通。已公开的能力主要围绕“提交前工程信任闭环”展开。

### 提交前 readiness gate

```bash
sego /review ready
```

`/review ready` 会给出 staged 提交前状态：

- 当前暂存区文件数量。
- staged safety lock 结果。
- `/verify fast` 的计划命令。
- 下一步建议：是否需要先补 staged、修复安全风险、执行 review 或 verify。

它是只读报告，不会调用模型、不会执行 build/test、不会安装依赖、不会修改文件。

### 交付摘要

```bash
sego /review summary
```

`/review summary` 面向 demo、PR 交接和提交前复盘。它会把当前 Git 状态、staged safety 结果、最近一次 review 历史、`/verify fast` 计划和建议下一步汇总到一个只读报告里。

它适合在你准备交给同事、维护者或另一个 AI 工具继续处理前运行，帮助对方快速理解“现在代码处于什么状态、还缺什么证据”。

### staged 安全锁

```bash
sego /review safety staged
```

只扫描 Git 暂存区文件，用于提交前发现明显的初级安全风险，例如疑似密钥文件、硬编码密钥、危险 shell 命令、本机绝对路径等。

它适合作为 vibe coding 新手的第一层安全锁，但这只是 Sego 的一个应用场景，不是 Sego 的完整产品边界。

### 代码审查

```bash
sego /review staged
sego /review
```

Sego 可以针对 staged diff 或当前工作区改动发起只读代码审查，输出结构化 findings，并尽量保留问题位置、严重程度、证据、风险说明和修复建议。

### 审查历史与状态

```bash
sego /review list
sego /review show <review-id>
sego /review status <review-id>
sego /review mark <review-id> <finding-id> <status> [note]
```

审查结果会沉淀为本地记录，便于回看、标记、复盘和交接。finding 状态支持后续追踪，不必把每次 review 都当成一次性聊天结果。

### 验证计划

```bash
sego /verify fast
sego /verify
```

Sego 会根据当前项目识别基础验证计划，例如 Rust 项目的 `cargo build` / `cargo test`，或 Node 项目的 test/build 脚本。`/review ready` 只展示验证计划；真正执行仍由 `/verify` 明确触发。

### 工具链建议

```bash
sego /review tools
```

本地只读探测项目语言和工具链，给出建议执行的检查命令。它不会安装依赖，也不会自动执行外部工具。

---

## 推荐工作流

如果你已经使用 AI 工具完成了一轮代码修改，可以按下面的顺序提交前自查：

```bash
git add <files>
sego /review ready
sego /review summary
sego /review safety staged
sego /review staged
sego /verify fast
```

推荐理解方式：

| 步骤 | 作用 |
|---|---|
| `git add <files>` | 明确本次准备提交的范围 |
| `/review ready` | 查看提交前状态和下一步建议 |
| `/review summary` | 生成可复制的交付摘要，方便 demo、PR 和 agent 交接 |
| `/review safety staged` | 先用本地安全锁排除明显风险 |
| `/review staged` | 对暂存区 diff 做模型辅助审查 |
| `/verify fast` | 明确执行快速验证，生成可交付证据 |

---

## 产品方向

Sego 的目标不是成为一个“只有 review 优势、没有编码优势”的孤立工具。它的产品定位是 AI Coding 工作流中的工程信任层：

```text
AI 编码工具        ->  负责生成代码
Sego              ->  负责审查、验证、记录、交接证据
Git / CI / 平台    ->  负责合并、部署和生产交付
```

未来 Sego 会围绕四个方向演进：

| 方向 | 说明 |
|---|---|
| Review | 让代码审查更结构化，输出风险等级、证据、文件位置和建议 |
| Verify | 将审查结论与构建、测试、lint 等验证证据连接起来 |
| Memory | 沉淀项目上下文、历史误报、已接受规则和验证记录 |
| Ship | 生成面向 commit、PR 和发布的交付报告 |

---

## 适合谁使用

### 个人开发者

如果你已经在使用 AI 写代码，Sego 可以帮你在提交前多一层工程化确认：看清改动、发现风险、补足验证证据。

### vibe coding 新手

如果你不熟悉代码规范、Git 提交边界、安全风险和验证流程，Sego 可以作为提交前安全锁，提醒你不要把明显危险的内容直接提交。

### 开源项目维护者

Sego 可以作为本地 review 和 verify 辅助工具，在 PR 前提前暴露问题，降低维护者 review 成本。

### AI Coding 工具重度用户

如果你经常在多个 AI 工具之间切换，Sego 可以作为统一的工程信任层，帮助你把不同工具生成的改动纳入同一套审查和验证流程。

---

## 快速开始

### Windows

```powershell
irm https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.ps1 | iex
set ANTHROPIC_API_KEY=sk-your-key
sego
```

### macOS / Linux

```bash
curl -fsSL https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.sh | bash
export ANTHROPIC_API_KEY="sk-your-key"
sego
```

### 从源码构建

```bash
git clone https://github.com/007M7/Sego-Agent.git
cd Sego-Agent/rust
cargo build --release
./target/release/sego
```

---

## 常用命令

```bash
sego
```

进入交互式 CLI。

```bash
sego "总结当前项目结构"
```

执行一次性任务。

```bash
sego /review ready
```

查看提交前 readiness gate。

```bash
sego /review summary
```

生成只读交付摘要。

```bash
sego /review staged
```

审查暂存区改动。

```bash
sego /review list
```

查看本地审查历史。

```bash
sego /verify fast
```

执行快速验证。

```bash
sego status
```

查看当前工作区状态。

```bash
sego --resume latest
```

恢复最近一次会话。

---

## 当前状态

Sego 当前处于早期 MVP 阶段，重点是把 review、verify 和本地工程记忆跑通。部分能力仍在快速迭代中，接口和输出格式可能继续调整。

已完成的公开方向：

- CLI 基础能力。
- 本地会话恢复。
- `/review` 初版。
- staged code review。
- 结构化 review findings。
- review 历史查看。
- finding 状态标记。
- staged safety lock。
- `/review ready` 提交前 readiness gate。
- `/review summary` 只读交付摘要。
- `/verify` 初版。
- 基础权限边界。
- GitHub Actions 基础验证。

正在推进：

- 更稳定的审查输出结构。
- 审查报告与验证证据关联。
- 更清晰的 PR / commit 报告。
- 面向个人开发者的完整 MVP 体验。
- 可复用的工程上下文记忆。

---

## 开源边界

当前 README 只展示 Sego 的公开产品定位和 MVP 使用方式。更完整的内部开发流程、工程记忆实现、协作协议、评审策略、验证链路和商业化设计暂不在开源文档中展开。

随着 MVP 稳定，项目会逐步开放更多可以公开的能力说明和使用示例。

---

## 许可证

Sego 使用 MIT License。

---

## 一句话总结

```text
Sego 不是替你写更多代码的工具。
Sego 是帮你判断 AI 写出的代码是否值得合并的工程信任运行时。
```
