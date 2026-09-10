<div align="center">

[English](README.md) | **简体中文**

</div>

<h1 align="center">Sego <img src="assets/sego-ui-icon.png" width="36" height="36" alt="Sego 图标" align="center"></h1>

<p align="center">
  <strong>验证 AI 主张的工程信任层</strong><br>
  为 AI 生成的改动提供有证据的审查与验证——<br>
  保留问题、覆盖范围和未验证项。
</p>

<p align="center">
  <a href="#快速开始"><img src="https://img.shields.io/badge/快速开始-blue?style=flat-square" alt="快速开始"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/许可证-MIT-green?style=flat-square" alt="MIT 许可证"></a>
  <img src="https://img.shields.io/badge/Rust-原生-orange?style=flat-square" alt="Rust 原生">
  <img src="https://img.shields.io/badge/平台-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey?style=flat-square" alt="支持平台">
</p>

---

<p align="center">
  <img src="assets/sego-cli-demo.png" width="760" alt="sego /review staged 终端演示：结构化 findings、严重度分级、证据持久化到 .sego/reviews/">
</p>

**目录**：[解决什么问题](#what-problem-does-sego-solve) · [快速开始](#quick-start) · [审查流水线](#the-review-pipeline) · [能力边界](#capability-boundaries) · [系统架构](#architecture) · [集成](#integration-optional) · [开发与贡献](#development--contributing)

---

<a id="what-problem-does-sego-solve"></a>
## Sego 解决什么问题

Sego 不是另一个 AI 编码工具，也不是 IDE。它工作在 AI 编码工具（Claude Code、Codex、Cursor 等）生成代码**之后**：对明确范围的改动做模型驱动的受约束审查 + 确定性证据校验，把结果变成可复查的结构化产物。

三个真实的痛点：

- **利益冲突**：AI 编码工具既生成代码又审查自己的代码。对 200+ 个 vibe-coded 应用的评估发现 91.5% 存在可溯源到 AI 的漏洞（[Keyhole Software 2026](https://keyholesoftware.com/vibe-coding-trends-2026/)）；而 63% 的 vibe coding 用户自我认同为非专业开发者——他们没有能力自己做第二遍把关（同上）。
- **"说完成"不等于"完成"**：模型声称任务完成，但缺少可接受的证据，未验证项被静默吞掉。
- **结果不可复查**：审查发现没有证据绑定，"零发现"被当成"通过"。

Sego 的回答：输入是明确范围的改动与验收预期，输出是结构化 findings + 逐条证据状态 + 持久化到 `.sego/reviews/` 的可复查产物。**零发现不等于验收通过。**

<p align="center">
  <img src="assets/figures/fig1-motivation.svg?v=2" width="820" alt="Three risk chains: review co-generated with code, done is not done, results cannot be re-checked">
</p>
<p align="center"><sub><b>图 1</b>：AI 编码工作流中的三条风险链。Sego 针对的正是这三点。</sub></p>

<p align="center">
  <img src="assets/figures/fig2-pipeline.svg?v=3" width="900" alt="Review pipeline: constrained model review + deterministic evidence gate + structured artifacts + human decision">
</p>
<p align="center"><sub><b>图 2</b>：审查流水线总览。Evidence Gate 对每条候选 finding 做确定性校验：通过者成为 verified finding，越界 / 截断 / 未捕获者保留为未验证缺口——两条路径都写入产物，零发现不等于通过。</sub></p>

---

<a id="quick-start"></a>
## 快速开始

📘 第一次使用建议先看这份指南：

- [在线阅读版：Sego 使用指南](docs/Sego使用指南.md)
- [Word 下载版](docs/Sego使用指南.docx?raw=1)（GitHub 不能直接预览 `.docx`，如果页面空白请下载后用 Word/WPS 打开）

### Windows（推荐：直接下载）

打开 [GitHub Releases](https://github.com/007M7/Sego-Agent/releases/latest)，下载 `sego-windows.zip`，解压后双击 `Sego.cmd` 即可启动。

如果你从 GitHub 右上角 **Code → Download ZIP** 下载的是源码包，里面不会直接包含 `sego.exe`。这种情况下可以运行仓库根目录的 `start-sego-windows.cmd`，它会自动下载最新 release binary 并启动 Sego。

### Windows（一行安装）

```powershell
irm https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.ps1 | iex
```

### macOS / Linux

```bash
curl -fsSL https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.sh | bash
```

### 从源码构建

```bash
git clone https://github.com/007M7/Sego-Agent.git
cd Sego-Agent/rust
cargo build --release
./target/release/sego
```

### 配置模型

Sego 支持 DeepSeek 和 Anthropic 模型。设置对应的环境变量：

**Windows PowerShell / CMD（设置后重新打开终端生效）：**

```powershell
setx DEEPSEEK_API_KEY "your-key"
setx DEEPSEEK_MODEL "deepseek-v4-flash"

# 或 Anthropic
setx ANTHROPIC_API_KEY "your-key"
```

**macOS / Linux：**

```bash
# DeepSeek（推荐，性价比高）
export DEEPSEEK_API_KEY="your-key"
export DEEPSEEK_MODEL="deepseek-v4-flash"

# 或 Anthropic
export ANTHROPIC_API_KEY="your-key"
```

### 安全默认

Sego **默认以只读（ReadOnly）权限启动**：审查会话不能写入文件或执行命令。需要写入、命令执行或自主能力时，必须通过 `--permission-mode` 显式选择（如 `workspace-write`、`danger-full-access`），或用 `RUSTY_CLAUDE_PERMISSION_MODE` 环境变量 / 项目配置授权。

### 第一次 review

```bash
cd your-project
git add -A
sego /review staged
```

Sego 会审查你的暂存区改动，输出结构化的 findings（严重程度 / 文件 / 行号 / 证据 / 风险 / 修复建议），并将审查结果持久化到 `.sego/reviews/`。

---

<a id="the-review-pipeline"></a>
## 审查流水线：从 diff 到可复查证据

一次 `sego review` 在内部经过五个阶段，每个阶段都有确定的工程行为：

**① 审查范围与预检（`ReviewScope` + preflight）**
三种范围：`Staged`（暂存区）/ `Workspace`（工作区）/ `FullRepo`（整仓快照，适用于非 Git 目录）。预检会检测嵌套 Git 仓库等风险并执行策略规则（PEP-001..005），产出包含 diff 与文件清单的 `ReviewTarget`。

**② Prompt 构建（`build_review_prompt`）**
把 ReviewTarget 组装为结构化 prompt：源 diff + 完整文件树作为上下文 + 明确的审查指令与输出契约，并在 token 预算内裁剪。

**③ 模型调用与三级解析**
模型输出经过三级解析策略——**Direct JSON → Fenced JSON → Prose Extraction**——兼容不同模型的输出格式。三级全部失败时明确标注 `parse_attempted_but_failed`，绝不静默显示"0 findings"。

**④ Evidence Gate（证据门）**
逐条校验 finding 引用的位置是否真实存在于被审改动中；通过的 finding 获得稳定的 `stable_finding_id` 用于跨版本追踪。

**⑤ 产物持久化与展示**
结果写入 `.sego/reviews/` 并渲染为终端摘要 / HTML Review Card（Green / Yellow / Red 置信度），聚合为 `AcceptanceRecord` 辅助验收决策。

<p align="center">
  <img src="assets/figures/fig3-artifact-lifecycle.svg?v=2" width="880" alt="Artifact lifecycle: diff_hash binding, append-only index, four-state separation, finding disposition state machine">
</p>
<p align="center"><sub><b>图 3</b>：审查产物生命周期。<code>diff_hash</code> 把产物绑定到被审代码状态；四种状态（执行 / 验证结论 / 问题处理 / 用户决定）严格分开；单条 finding 的修复必须关联后续复验。</sub></p>

### 每条 finding 的结构

- **severity**：`critical / high / medium / low / info`
- **file / line / title**：定位到具体改动
- **evidence**：来自 diff 或文件内容的具体证据
- **risk / suggestion**：为什么重要、怎么修
- **confidence**：模型置信度
- **evidence_status**：确定性证据门的校验结果（见下）

### 证据门（evidence gate）：`verified` 不等于"缺陷已复现"

| evidence_status | 含义 |
|---|---|
| `verified` | 引用路径在捕获范围内且行号有效——**仅代表位置有效、内容已捕获，不代表缺陷已被复现证实** |
| `unverified_file` / `unverified_line` / `unverified_dependency` | 引用的文件 / 行号 / 依赖无法在捕获内容中确认 |
| `scope_not_captured` / `content_not_captured` / `content_truncated` | 范围未捕获 / 内容未捕获 / 内容被截断 |

模型输出无法解析时，审查结果会明确标注 `parse_attempted_but_failed`——绝不静默显示"0 findings"。

### 四种状态分开看

| 维度 | 问的问题 | 公开措辞 |
|---|---|---|
| 执行状态 | 检查是否跑完？ | 检查完成 / 失败 / 取消 |
| 验证结论 | 证据是否支持主张？ | 未发现支持充分的问题（仍可能有未验证项）|
| 问题处理状态 | 发现如何处理？ | open → acknowledged → fixed（需关联复验）/ ignored |
| 用户决定 | 接受还是返工？ | 等待用户决定——**不由验证结论自动生成** |

### 审查产物

每次审查写入 `.sego/reviews/`：

- `review-<id>.json` — 机器可读审查产物
- `review-<id>.md` — 人类可读报告
- `index.jsonl` — append-only 索引，供 agent 定位审查历史

```bash
sego review show latest --json   # 机器可读的最新审查摘要
```

字段契约见 [`docs/REVIEW_ARTIFACT_CONTRACT.md`](docs/REVIEW_ARTIFACT_CONTRACT.md)，agent 接入工作流见 [`docs/AGENT_REVIEW_HANDOFF.md`](docs/AGENT_REVIEW_HANDOFF.md)。

### 示例：一次正常审查

对一段含安全漏洞的 Python 代码（`app.py`）做 `/review staged` 的输出示例：

```python
def get_user(name):
    query = "SELECT * FROM users WHERE name = '" + name + "'"  # SQL 注入
    return db.execute(query)

def hash_password(pw):
    return pw ^ 0x12345678  # XOR 不是安全 hash
```

| severity | file | line | title |
|---|---|---|---|
| critical | app.py | 2 | SQL injection via string concatenation |
| critical | app.py | 5 | XOR used as password hashing (reversible) |

### 示例：零发现不等于通过（示例数据）

对一个纯重构 diff（仅重命名变量、调整格式），Sego 可能返回 **0 findings**。这表示"未发现支持充分的问题"，**不是**"该改动已通过验收"——改动是否可交付仍由你决定。审查产物中的覆盖范围与未验证项会一并保留，供你复查。

---

<a id="capability-boundaries"></a>
## 真实能力边界

> Sego review is model-driven, not exhaustive static analysis. Unverified items are preserved as explicit gaps — never silently converted into "passed".

- **模型驱动，不是穷尽式静态分析**：`sego review`（含 `--full`）是模型对 manifests、入口点和目录上下文快照的审查，不保证找出所有 bug，不替代成熟的静态分析器、安全扫描器或形式化验证。
- **verify-before-trust**：证据缺失、截断、越界都会保留为明确缺口，不会被模型补全成"已观察事实"；未验证项不会被标成通过。
- **已知局限**（诚实列出）：
  - 对"看似危险但有缓解措施"的代码（参数化查询、白名单、HMAC 校验等）曾存在误报倾向——内部校准评测已识别此问题，review prompt 已加入缓解措施识别（已合入 main，将随下一版本发布）；
  - 对时序 / 并发类缺陷的检出能力有限，不能替代针对性测试；
  - 模型输出偶发无效 JSON（会被 `parse_attempted_but_failed` 显式标注，不会伪装成零发现）。
- **不取代**：Sego 的 review artifact 是工程判断证据，不是安全认证、合规认证、部署批准或发布批准。高风险合并 / 发布仍需要测试、CI、人工审查、Release QA 与业务上下文共同决策。

| 能力 | 状态 | 版本 / 证据 |
|---|---|---|
| `/review` 结构化审查 + 证据门 | available | v0.1.8+；[`docs/REVIEW_ARTIFACT_CONTRACT.md`](docs/REVIEW_ARTIFACT_CONTRACT.md) |
| Review card / acceptance record | available | v0.1.9 |
| Reviewer identity 元数据 | available（归因用途，非签名 / 非来源证明）| v0.1.9 |
| Review artifact JSON Schema | available | v0.1.7+（v0.1.9 更新至当前值）；[`schema/`](schema/) |
| Sidecar JSON 接口 + skill 包 | experimental (PoC) | 仅 `review` action，不承诺向后兼容 |
| 缓解措施识别（降低误报）| 已合入 main，将随下一版本发布 | [#76](https://github.com/007M7/Sego-Agent/pull/76) |
| CI 集成 / artifact 签名 / 跨工具产物格式 | planned | 见 [ROADMAP](ROADMAP.md) |

公开宣称边界详见：[`docs/PUBLIC_CLAIM_BOUNDARY.md`](docs/PUBLIC_CLAIM_BOUNDARY.md) · [`docs/RELEASE_QA_CAPABILITY_MATRIX.md`](docs/RELEASE_QA_CAPABILITY_MATRIX.md) · [`docs/LEGACY_SOURCE_BOUNDARY.md`](docs/LEGACY_SOURCE_BOUNDARY.md)

---

<a id="architecture"></a>
## 系统架构

<p align="center">
  <img src="assets/figures/fig4-architecture.svg" width="900" alt="Sego core architecture: CLI & Intent Router, Review Engine pipeline (preflight, prompt, model call, parser, Evidence Gate), Safety Lock & Permissions, Provider Layer, Review Artifacts, Runtime Engine, Verification, Integration">
</p>
<p align="center"><sub><b>图 4</b>：Sego 核心架构（按仓库真实子系统）。CLI 与意图路由承接输入；<b>Review Engine</b> 是核心流水线——scope 预检 → prompt 构建 → 模型调用 → report parser → <b>Evidence Gate</b>；Safety Lock 和 Permissions 全程守护（ReadOnly 默认）；Runtime Engine 承载会话与工具循环；Verification 提供验证证据；产物经受控消费进入集成层。</sub></p>

### Rust Workspace：9 个 crate

Sego 是一个由 9 个 crate 组成的 Rust workspace，依赖流向严格分层——`rusty-claude-cli` 构建出 `sego` 二进制，作为编排入口消费其余功能 crate；`telemetry` 是零依赖叶子节点。

| crate | 职责 | 关键组件 |
|---|---|---|
| **`rusty-claude-cli`** | `sego` 二进制入口：REPL、终端渲染、命令与意图路由 | `parse_args` · `parse_nl_intent` |
| **`runtime`** | 核心引擎：会话状态、权限、审查流水线、恢复 | `ConversationRuntime` · `EvidenceStatus` · `PermissionPolicy` |
| **`api`** | LLM HTTP 客户端与 provider 抽象、SSE 流式、prompt 缓存 | `Client` · `MessageRequest` · prompt cache |
| **`tools`** | 内置工具实现（read/grep/bash 等）与工具注册 | `ToolExecutor` |
| **`commands`** | 全部 slash 命令实现与注册表 | `CommandRegistry` |
| **`plugins`** | 插件与 hooks：外部工具 / 生命周期钩子接入 | `PluginRegistry` · `HookRunner` |
| **`telemetry`** | 轻量日志与性能监控（零第三方依赖叶子） | sink / tracer |
| **`compat-harness`** | 与参考实现的 parity 校验层 | `extract_commands` |
| **`mock-anthropic-service`** | 离线测试 harness：模拟 Anthropic API，按 scenario 返回确定性响应 | `MockAnthropicService` |

### 核心子系统

| 子系统 | 职责 | 关键符号 |
|---|---|---|
| **Review Engine** | diff 收集 → prompt 构建 → 模型审查 → 解析 → 证据门 → 持久化 | `ReviewScope` · `build_review_prompt` · `ReviewReport::from_model_output` · `stable_finding_id` |
| **Runtime Engine** | 会话循环（输入 → 上下文组装 → 模型回合 → 工具执行 → 持久化） | `ConversationRuntime` · `SystemPromptBuilder` · `compact_session` |
| **Safety Lock & Permissions** | 静态扫描（密钥 / 危险命令 / 硬编码路径）与权限策略 | `PermissionPolicy` · bash classifier |
| **Verification** | 项目级验证计划（按项目类型生成 cargo / npm 验证命令） | `build_verification_plan` |
| **MCP 集成** | 经 Stdio / SSE / WebSocket 消费外部 MCP server 的工具 | `McpToolRegistry` |

会话状态经 `persist_recovery_state` 持久化，支持崩溃后恢复；超出阈值的上下文由 `compact_session` 自动压缩。

`schema/` 目录提供公开 JSON Schema 契约（进 GitHub）：

- `review-artifact.schema.json`
- `review-index-entry.schema.json`
- `sidecar-request-response.schema.json`

- **纯 Rust，本地优先**：`unsafe_code = "forbid"`，clippy pedantic。
- **diff_hash 绑定**：review/verify 指向同一代码差异，防止"审查 A 提交 B"。
- **Integration 层目前是 experimental / PoC**：sidecar 协议、JSON Schema、skill 包属于早期集成，不承诺稳定生态契约，后续可能演进。

---

<a id="integration-optional"></a>
## 集成（可选）

<p align="center">
  <img src="assets/figures/fig5-integration.svg?v=2" width="820" alt="Integration topology: AI coding tools call Sego via sidecar/skill; governance platforms consume results via the VerificationArtifact contract">
</p>
<p align="center"><sub><b>图 5</b>：集成拓扑。左：AI 编码工具经 sidecar / skill 包调用 Sego；右：治理平台经版本化合同消费验证结果——裁决权保留在集成方。</sub></p>

Sego 完全可独立使用——本 README 描述的全部工作流不依赖任何平台。在此基础上，它的验证结果可以按两种方式被外部系统消费：

| 集成对象 | 方式 | 状态 |
|---|---|---|
| **AI 编码工具**（Claude Code / Codex / Cursor 等） | sidecar JSON 接口或 skill 包调用 Sego 审查 | experimental (PoC) |
| **治理平台 / CI 工作流** | 通过版本化的 VerificationArtifact 合同读取结构化验证结果与未验证项，作为独立验证证据；任务与发布裁决始终保留在集成方 | 设计中 |

> 该验证合同的首个消费方是 EgoPulse（一个个人 Agent 治理体系）。集成细节与公开示例将随验证合同稳定后另行发布。

---

<a id="development--contributing"></a>
## 开发与贡献

```bash
# 构建
cd rust && cargo build

# 测试
cargo test --workspace

# 格式 + lint
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings

# 运行
cargo run -p rusty-claude-cli --bin sego
```

贡献流程见 [AGENTS.md](AGENTS.md)（fork → topic branch → 小而聚焦的 PR；触及 `schema/` 或 sidecar 协议视为合同变更，需在 PR 中显式标注）与 [DEVELOPMENT.md](DEVELOPMENT.md)。PR 请更新 `CHANGELOG.md` 的 `[Unreleased]` 段。

- 问题反馈：[GitHub Issues](https://github.com/007M7/Sego-Agent/issues)
- 安全问题：按 [SECURITY.md](SECURITY.md) 处理，不要公开提交
- 免费 / 私有审查服务：见 [docs/LAUNCH.md](docs/LAUNCH.md)

## License

[MIT](LICENSE)
