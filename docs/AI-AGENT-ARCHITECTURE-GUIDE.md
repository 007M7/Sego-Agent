# AI Agent 架构与开发完全指南

> 从 Sego Agent 的实践经验出发，系统讲解 AI Agent 的核心概念、架构设计、开发方法论。
> 作者：007M7 + Sego Agent

---

# 第一部分：核心概念速通

## 1.1 LLM — 大语言模型

```
用户输入 "写一个排序算法"
    → LLM 推理
    → 输出代码
```

LLM 是大脑，但大脑不能自己动手。Agent 的核心价值在于给大脑配上手脚。

| 概念 | 通俗理解 |
|------|---------|
| Token | 文本的最小单元，约 0.75 个英文单词或 0.5 个中文字 |
| Context Window | 一次对话能塞进多少 token。DeepSeek V4 = 1M tokens |
| Temperature | 模型"创造力"，0 = 保守，1 = 天马行空 |
| System Prompt | 给模型的"人设"，告诉它它是谁、该怎么做 |
| Max Tokens | 模型每次最多输出多少个 token |
| Streaming (SSE) | 逐字输出，不等到全写完再给用户 |

**实战要点：**
- System Prompt 是最重要的 prompt，决定了 Agent 的行为边界
- Context window 越大 ≠ 越聪明，长对话要定期压缩（compaction）
- 国产模型通过 Anthropic-compatible API 接入，用一个适配层搞定

## 1.2 Prompt Engineering — 提示词工程

```
糟糕的 prompt:  "帮我写代码"
好的 prompt:    "在 src/auth/ 下重构登录逻辑，加上 JWT token 验证和单元测试"
```

| 技巧 | 示例 |
|------|------|
| 角色设定 | "你是一个资深 Rust 工程师" |
| 约束条件 | "只用标准库，不要引入第三方依赖" |
| 分步指令 | "1. 先读代码 2. 分析问题 3. 给出方案 4. 我确认后执行" |
| 输出格式 | "用 markdown 表格列出修改的文件和原因" |
| Few-shot | 给 2-3 个示例让模型照猫画虎 |

**Sego 的 System Prompt 结构：**
```
# Sego instructions          ← CLAUDE.md / .claw/instructions.md
# Environment context        ← 动态注入：日期、目录、模型名
├─ Model family
├─ Working directory
├─ Date
├─ Platform
└─ Workflow recording hint
```

## 1.3 RAG — 检索增强生成

```
传统 LLM: 问题 → LLM → 回答（靠训练数据，可能过时）
RAG:     问题 → 检索相关文档 → 文档+问题 → LLM → 回答（有依据）
```

**RAG 的典型实现：**

| 步骤 | 技术选型 |
|------|---------|
| 文档切分 | 按段落/函数/类切分，每个 chunk 300-500 tokens |
| 向量化（Embedding） | 用 embedding 模型把文本转成向量 |
| 存储 | 向量数据库（Chroma、Qdrant、Milvus）或 SQLite+向量扩展 |
| 检索 | 用户问题 → embedding → 在向量库中找最相似的 k 个 chunk |
| 注入 | 检索结果拼到 prompt 里："参考以下文档回答问题：{chunks}" |

**Sego 的类 RAG 机制：**
- CLAUDE.md / .claw/instructions.md → 自动发现并注入到 System Prompt
- `ProjectContext::discover()` → 读 Git 状态、目录结构
- `sego review` / `sego learn` → 从历史会话中检索模式

## 1.4 MCP — 模型上下文协议

```
┌──────────────────────────────────────────────────┐
│                    Sego Agent                     │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐          │
│  │ MCP     │  │ MCP     │  │ MCP     │          │
│  │ Server 1│  │ Server 2│  │ Server 3│          │
│  │ (文件)  │  │ (数据库)│  │ (API)   │          │
│  └─────────┘  └─────────┘  └─────────┘          │
└──────────────────────────────────────────────────┘
```

MCP 是 Anthropic 提出的标准协议，让 LLM 能调用外部工具。一个 MCP Server 暴露多个 Tool。

| 组件 | 作用 |
|------|------|
| MCP Server | 独立进程，通过 stdio/HTTP 暴露工具 |
| Tool Discovery | Agent 启动时连接 Server，发现可用的 Tool 列表 |
| Tool Invocation | Agent 调用 Tool，Server 执行并返回结果 |
| MCP Lifecycle | Sego 自动管理 Server 的启动/心跳/重启/降级 |

**Sego 的 MCP 实现：**
- `mcp_client.rs` — 连接管理
- `mcp_stdio.rs` — stdio 传输
- `mcp_tool_bridge.rs` — 工具桥接
- `mcp_lifecycle_hardened.rs` — 容错生命周期

## 1.5 Skill — 可组合的能力单元

```
Skill = 一个 Markdown 文件 (SKILL.md) 定义了触发条件 + 执行流程
```

| Skill 组件 | 说明 |
|-----------|------|
| `name` | 技能名称，如 `systematic-debugging` |
| `description` | 何时触发，如 "当遇到 bug 或错误时" |
| 流程定义 | 步骤化的执行指令 |
| 工具调用 | Skill 内部调用的工具 |

**Sego 的 Skills 体系：**
- Claude Code Skills 格式（SKILL.md）
- 通过 `/skills` 命令管理
- 安装路径：`.claude/skills/`

---

# 第二部分：AI Agent 架构设计

## 2.1 通用 Agent 架构

```
┌──────────────────────────────────────────────────────────┐
│                     Orchestrator                         │
│  (任务分解、流程控制、状态管理)                              │
├──────────────────────────────────────────────────────────┤
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐ │
│  │ Planning │  │ Memory   │  │ Tool Use │  │ Safety   │ │
│  │ (规划)   │  │ (记忆)   │  │ (工具)   │  │ (安全)   │ │
│  └──────────┘  └──────────┘  └──────────┘  └──────────┘ │
├──────────────────────────────────────────────────────────┤
│                     LLM Provider                         │
│         (Claude / GPT / DeepSeek / Qwen / ...)           │
└──────────────────────────────────────────────────────────┘
```

### 2.2 核心模块详解

#### Planning（规划层）

| 方式 | 说明 | 适用场景 |
|------|------|---------|
| ReAct | 思考→行动→观察→循环 | 通用任务 |
| Plan-and-Execute | 先写完整计划再执行 | 复杂多步任务 |
| Tree-of-Thought | 多路径探索，选最优 | 需要推理的任务 |
| Multi-Agent | 多代理分工并行 | 大规模重构 |

**Sego 的实现：**
- `EnterPlanMode` / `ExitPlanMode` — Plan-and-Execute
- `Agent` tool — Multi-Agent 并行
- `Task` / `Cron` / `Team` — 任务队列管理

#### Memory（记忆层）

```
短期记忆：当前会话的完整对话历史（在 context window 里）
长期记忆：持久化的会话记录 + 结构化知识
    ├── Session Persistence (JSONL)
    ├── Workflow Store (Lane Events)
    ├── Community Learning (匿名统计)
    └── CLAUDE.md (项目指令)
```

**Sego 的记忆机制：**

| 层级 | 存储位置 | 作用 |
|------|---------|------|
| 短期 | Context Window | 当前会话上下文 |
| 会话 | `.claw/sessions/*.jsonl` | 会话恢复 |
| 工作流 | `.claw/workflow/sessions/*.json` | 效率分析 |
| 趋势 | `.claw/workflow/trends.json` | 跨会话学习 |
| 项目 | `CLAUDE.md` / `.claw/instructions.md` | 项目级指令 |

#### Tool Use（工具层）

工具是 Agent 的手脚。每个工具需要定义：
- **input_schema**：需要什么参数
- **description**：什么时候该用
- **executor**：具体怎么执行

**Sego 的工具分类（40+）：**

| 类别 | 工具 |
|------|------|
| 文件 | Read, Write, Edit, Glob, Grep |
| 命令 | Bash (with sandbox/timeout) |
| Web | WebSearch, WebFetch |
| 编排 | Agent, Task, Cron, Team |
| 编辑 | NotebookEdit |
| 工作流 | TodoWrite, PlanMode |
| 外部 | MCP servers, LSP client |

#### Safety（安全层）

```
Permission Mode:
  read-only       → 只能读文件和搜索
  workspace-write → 可以修改工作区内文件
  danger-full-access → 完全权限（默认）

Sandbox:
  容器化的文件系统和网络隔离
  Bash 命令在沙箱中执行
```

## 2.3 Agent 开发的核心设计原则

### 原则 1：协议无关（Provider Abstraction）

```rust
// 不绑定特定模型
trait ApiClient {
    fn stream_message(&self, request: &MessageRequest) -> Result<Stream>;
}

// Anthropic 实现
struct AnthropicClient { ... }
impl ApiClient for AnthropicClient { ... }

// OpenAI 兼容实现（DeepSeek / Qwen / GLM）
struct OpenAICompatClient { ... }
impl ApiClient for OpenAICompatClient { ... }
```

### 原则 2：工具可插拔（Tool Registry）

```
注册工具 → Agent 启动时发现 → 注入到 System Prompt → LLM 决定调用
```

**Sego 的实现：** `GlobalToolRegistry` + `ToolSpec` + `mvp_tool_specs()`

### 原则 3：会话可恢复（Session Persistence）

```
中断前：完整 JSONL 对话历史
重启：sego --resume latest → 完整恢复上下文
```

### 原则 4：质量可度量（Observability）

```
每次会话自动产生：
├── Lane Events (16 种事件类型)
├── Session Report (效率评分)
├── Failure Stats (11 类故障)
└── Trend Analysis (长期趋势)
```

---

# 第三部分：全栈 AI 产品架构

## 3.1 典型 AI 产品技术栈

```
┌─────────────── 前端层 ───────────────┐
│  Web UI (React/Vue/Next.js)          │
│  CLI (Rust, 终端交互)                │
│  IDE 插件 (VS Code / JetBrains)       │
├─────────────── 网关层 ───────────────┤
│  API Gateway (认证/限流/路由)          │
│  SSE/WebSocket (流式输出)            │
├─────────────── 服务层 ───────────────┤
│  Orchestrator (任务编排)              │
│  Agent Runtime (对话循环)             │
│  Tool Executor (工具执行)             │
├─────────────── 数据层 ───────────────┤
│  Session Store (PostgreSQL / SQLite)  │
│  Vector DB (Chroma / Qdrant)        │
│  Cache (Redis)                      │
├─────────────── 模型层 ───────────────┤
│  LLM API (Claude / GPT / DeepSeek)   │
│  Embedding Model (向量化)            │
│  Local Model (Ollama / vLLM)        │
└──────────────────────────────────────┘
```

## 3.2 DevOps / MLOps 考虑

| 环节 | 工具/实践 |
|------|---------|
| CI/CD | GitHub Actions：fmt + clippy + test + release |
| 二进制分发 | GitHub Releases，单一二进制，零依赖 |
| 监控 | Lane Events → 效率趋势 → 告警 |
| 遥测 | 匿名统计（opt-in），绝不收集隐私数据 |
| 回滚 | Git tag + 旧 Release 二进制 |

## 3.3 Sego 的部署模型

```
方式 1：本地单机
  用户 → sego.exe → DeepSeek API

方式 2：团队共享
  开发者A → sego → DeepSeek API
  开发者B → sego → DeepSeek API
  技术主管 → sego review --team

方式 3：企业内网
  内网 → sego + 私有模型 → 不开外网
```

---

# 第四部分：Vibe Coding 方法论

## 4.1 什么是 Vibe Coding

> **用自然语言描述意图，让 AI 写代码。你审核结果，而不是亲手敲键盘。**

```
传统开发：想 → 写代码 → 调试 → 改bug → 测试 → 重复
Vibe Coding：描述意图 → AI 生成 → 你审核 → AI 改 → 你合并
```

## 4.2 Vibe Coding 的四个层次

| 层次 | 说明 | 你做什么 | AI 做什么 |
|------|------|---------|----------|
| L1: 辅助 | AI 补全代码片段 | 写大部分代码 | 补全、纠错 |
| L2: 协作 | AI 写完整函数/模块 | 描述需求，审核 | 编写、测试 |
| L3: 主导 | AI 主导开发流程 | 定方向，审 PR | 设计、编码、测试、提 PR |
| L4: 自主 | AI 自主发现和修复 | 监控，决策 | 诊断、修复、优化 |

**Sego 目前处于 L2-L3，目标 L4。**

## 4.3 Vibe Coding 最佳实践

### 写好 Prompt 的五要素

```
1. 角色：你是什么角色（Rust 专家、全栈工程师）
2. 上下文：项目结构、现有代码、约束条件
3. 任务：具体要做什么，粒度越细越好
4. 格式：输出的格式要求（代码、文档、报告）
5. 验证：怎么判断做对了（测试通过、编译成功）
```

### 多代理分工策略

```
复杂任务 → 拆成独立子任务 → 每个子代理处理一块 → 合并结果
   │
   ├── Agent 1: 读代码、分析现状
   ├── Agent 2: 写新代码、改旧代码
   ├── Agent 3: 跑测试、验证
   └── Agent 4: 写文档、更新 README
```

### 反馈循环

```
使用 → 发现问题 → 记录到 workflow → 分析 → 修复 → 测试 → 合并 → 发布
  ↑                                                                      ↓
  └──────────────────────── 下一轮迭代 ←─────────────────────────────────┘
```

---

# 第五部分：Sego 实战速查

## 5.1 常用开发场景

```
"帮我在 tools/src/lib.rs 里加一个新工具 X"
"重构 runtime/src/conversation.rs 里的 Y 函数"
"在 api/src/providers/ 里适配新模型 Z"
"检查最近 5 次会话的 workflow，找出最频繁的故障类型"
"基于 review 结果，优化 recovery_recipes.rs"
```

## 5.2 架构决策速查表

| 场景 | 推荐方案 |
|------|---------|
| 需要调用外部 API | 加 MCP Server |
| 需要记住用户偏好 | 写到 CLAUDE.md 或 memory |
| 需要并行处理 | 用 Agent tool 生成子代理 |
| 需要定时任务 | 用 Cron tool |
| 新模型接入 | 在 providers/ 加适配 |
| API 格式兼容 | 在 convert_messages() 加转换逻辑 |

---

## 附录：推荐学习路径

1. **理解 LLM 基础** → 读 Anthropic/OpenAI API 文档
2. **手写一个简单 Agent** → 从 Python 开始，循环调用 LLM + 执行工具
3. **读 Sego 源码** → 从 `main.rs` 开始，追踪一次完整的对话流程
4. **加一个工具** → 在 `tools/src/lib.rs` 加一个新工具，理解 Tool Spec
5. **配一个 MCP Server** → 理解 MCP 协议和生命周期
6. **优化 System Prompt** → 调整 `prompt.rs`，观察行为变化

---

> **最后更新：** 2026-05-22
> **作者：** 007M7 + Sego Agent
> **相关文档：** Sego-Agent 开发维护指南、README.md、PHILOSOPHY.md
