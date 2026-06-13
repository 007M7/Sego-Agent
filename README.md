# Sego

<p align="center">
  <img src="assets/sego-ui-icon.png" width="96" height="96" alt="Sego 图标">
</p>

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

Sego 是面向 AI Coding 时代的工程信任工具。它不把重点放在“再做一个写代码的 Agent”，而是关注 AI 写完代码之后更关键的问题：

```text
这次改动改了什么？
有没有隐藏风险？
验证证据是否充分？
能不能放心合并？
这次经验能不能沉淀到下一次开发？
```

你可以继续用 Claude Code、Codex、Cursor、OpenHands 或其他 AI 编码工具完成代码生成；Sego 负责在代码进入合并和交付之前，提供 review、verify、记录和交付证据。

一句话：

```text
AI 负责生成，Sego 负责证明它值得被合并。
```

---

## 为什么需要 Sego

AI 编码工具已经能快速生成代码，但真实开发中的难点往往出现在后半段：

- 修完一个 bug 后，相关数据链路、调用链、测试和文档没有同步对齐。
- AI 只关注当前 diff，容易忽略历史上下文和项目约束。
- 代码 review 只给建议，没有证据、风险等级和可追踪记录。
- 测试通过与否缺少结构化沉淀，下一次仍然从头判断。
- 单次对话越来越长，项目经验无法稳定复用。

Sego 希望把这些问题收敛成一个本地优先、可审计、可持续演进的工程闭环：

```text
生成代码 -> 审查改动 -> 验证证据 -> 沉淀上下文 -> 辅助交付
```

---

## 当前开源版本能做什么

Sego 仍处在 MVP 迭代阶段。当前开源版本优先提供一套可以在本地运行的 CLI 基础能力，并逐步补齐 review 与 verify 工作流。

### 代码审查

- 支持通过 `/review` 进入代码审查流程。
- 面向 diff 做只读审查，强调风险、证据和建议。
- 审查目标是辅助开发者发现真实问题，而不是替代人类批准。

### 本地验证

- 支持通过 `/verify` 触发验证计划。
- 可用于快速确认构建、测试和基础质量检查结果。
- 验证输出会尽量保留失败摘要，方便定位问题。

### 会话与工程记忆

- 支持本地会话恢复。
- 支持记录开发过程中的关键状态。
- 长期目标是把已接受的问题、误报、验证结果和项目规则沉淀为可复用的工程记忆。

### 权限与安全边界

- 支持只读、工作区写入等权限模式。
- review 默认应以只读方式运行。
- 交付相关动作应保留人工确认边界。

### 模型兼容

- 支持接入兼容的主流大模型服务。
- 开源版本保留模型无关的使用方式，避免绑定单一供应商。

---

## Sego 的产品方向

Sego 的目标不是成为传统 IDE，也不是单纯的聊天式 Coding Assistant。它更接近 AI Coding 工作流中的信任层：

```text
AI 编码工具       ->  负责生成代码
Sego             ->  负责审查、验证、记录和交付证据
Git / CI / 平台   ->  负责合并、部署和生产交付
```

未来 Sego 会围绕四个方向演进：

| 方向 | 说明 |
|---|---|
| Review | 让代码审查更结构化，输出风险等级、证据、文件位置和建议 |
| Verify | 将审查结论与测试、构建、lint 等验证证据连接起来 |
| Memory | 沉淀项目上下文、历史误报、已接受规则和验证记录 |
| Ship | 生成面向 PR、commit 和发布的交付报告 |

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
sego /review
```

审查当前工作区改动。

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

## 适合谁使用

### 个人开发者

如果你已经在使用 AI 写代码，Sego 可以帮你在提交前多一层工程化确认：看清改动、发现风险、补足验证证据。

### 开源项目维护者

Sego 可以作为本地 review 和 verify 辅助工具，在 PR 前提前暴露问题，降低维护者 review 成本。

### AI Coding 工具重度用户

如果你经常在多个 AI 工具之间切换，Sego 可以作为统一的工程信任层，帮助你把不同工具生成的改动纳入同一套审查和验证流程。

---

## 当前状态

Sego 当前处于早期 MVP 阶段，重点是把 review、verify 和本地工程记忆跑通。部分能力仍在快速迭代中，接口和输出格式可能继续调整。

已完成的公开方向：

- CLI 基础能力
- 本地会话恢复
- `/review` 初版
- `/verify` 初版
- 基础权限边界
- GitHub Actions 基础验证

正在推进：

- 更稳定的审查输出结构
- 审查报告落盘
- 验证证据与审查结论关联
- 更清晰的 PR / commit 报告
- 面向个人开发者的完整 MVP 体验

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
