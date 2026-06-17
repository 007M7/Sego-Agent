<h1>Sego <img src="assets/sego-ui-icon.png" width="34" height="34" alt="Sego 图标" align="right"></h1>

<p align="center">
  <strong>AI Coding 的工程信任层</strong><br>
  让 AI 生成的代码变得可审查、可验证、可交付。
</p>

<p align="center">
  <a href="#快速开始"><img src="https://img.shields.io/badge/快速开始-5分钟-blue?style=flat-square" alt="快速开始"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/许可证-MIT-green?style=flat-square" alt="MIT 许可证"></a>
  <img src="https://img.shields.io/badge/Rust-原生-orange?style=flat-square" alt="Rust 原生">
  <img src="https://img.shields.io/badge/平台-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey?style=flat-square" alt="支持平台">
</p>

---

## Sego 是什么

Sego 不是另一个 AI 编码工具，也不是 IDE。它是一层**工程信任层**——在 AI 编码工具（Claude Code、Codex、Cursor 等）生成代码之后，Sego 对改动做独立的工程审查、安全检查和结构化记录。

> AI 负责生成代码，Sego 负责审查它是否值得被合并。

### 为什么需要 Sego

AI 编码已经普及，但生成的代码质量缺乏独立把关：

- **91.5%** 的 AI 编码应用含安全漏洞（[Keyhole Software 2026](https://keyholesoftware.com/vibe-coding-trends-2026/)）
- **63%** 的 vibe coding 用户不是专业开发者——他们看不懂 AI 生成的代码（同上）
- 现有 AI 编码工具的内置审查有**利益冲突**：它们既生成代码又审查自己的代码

Sego 做独立的第三方审查——不和任何生成工具绑定，输出结构化的风险发现，帮你判断"这次改动能不能提交"。

---

## 快速开始

📘 第一次使用建议先看这份 Word 指南：[Sego 使用指南](docs/Sego使用指南.docx)

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

### 第一次 review

```bash
cd your-project
git add -A
sego /review staged
```

Sego 会审查你的暂存区改动，输出结构化的 findings（严重程度 / 文件 / 行号 / 证据 / 风险 / 修复建议），并将审查结果持久化到 `.sego/reviews/`。

---

## 已实现能力

### 1. AI 代码审查（Code Review）

```bash
sego /review staged       # 审查暂存区改动
sego /review              # 审查工作区改动
sego /review ready        # 提交前 readiness gate
sego /review summary      # 交付摘要
sego /review safety staged # staged 安全锁
```

- **结构化 findings**：每个发现含 severity / file / line / title / evidence / risk / suggestion / confidence
- **安全锁**：检测疑似密钥、硬编码凭据、危险命令、本机绝对路径
- **审查历史**：findings 持久化到 `.sego/reviews/`，支持查看、标记状态、复盘
- **多模型支持**：DeepSeek（含推理模式）和 Anthropic

### 2. Crash Recovery（崩溃恢复）

```bash
sego --resume latest           # 恢复最近一次会话
sego --resume latest /status   # 查看恢复状态
```

- 进程异常中断后，下次启动自动检测并提示恢复
- 恢复时不重放旧工具调用（安全边界）
- 正常退出的会话不会触发恢复提示

### 3. Review-Trust 权限画像

```bash
sego --permission-profile review-trust
```

在 review/verify 会话中降低权限交互噪音：

- **自动放行**：只读命令（cat/rg/grep/ls/git status）、验证命令（cargo test/npm test）、`.sego/` 写入
- **需要确认**：源码写入、依赖安装、git commit/merge
- **直接拒绝**：rm -rf、git reset --hard、git push、sudo

`--permission-mode` 和 `--permission-profile` 互斥（不能同时使用）。

### 4. Sidecar JSON 接口（PoC）

```bash
echo '{"schema_version":1,"action":"review","cwd":"/project","scope":"staged"}' \
  | sego sidecar review
```

让外部工具通过 stdin/stdout JSON 调用 Sego 的审查能力：

- stdin 接收 JSON request → stdout 返回 JSON response
- skill 包（`skills/sego-review/`）可被 Claude Code / Codex / Cursor / 任何 SKILL.md 兼容工具调用
- 错误时返回结构化 error envelope（不崩溃）

> **PoC 状态**：sidecar 当前仅支持 `review` action。stdout 混入 banner 输出的已知限制将在后续修复。

### 5. 验证计划

```bash
sego /verify fast    # 快速验证计划
sego /verify         # 完整验证
```

根据项目类型识别验证命令（Rust: cargo build/test，Node: npm test/build）。

### 6. 接入 AI 编码工具（Sidecar skill PoC）

Sego 提供一个 sidecar skill 包，让你的 AI 编码工具（Claude Code、Codex、Cursor 等）能自动调用 Sego review。

**一键安装**：

```bash
# Mac / Linux
bash skills/sego-review/install.sh

# Windows
powershell -File skills\sego-review\install.ps1
```

脚本会自动检测已安装的 AI 工具（Claude Code / Codex / Zcode / Cursor），复制 skill 包到对应目录。安装后重启你的 AI 编码工具即可。

**手动调用**（不依赖 IDE）：

```bash
echo '{"schema_version":1,"action":"review","cwd":"/your/project","scope":"staged"}' \
  | sego sidecar review
```

> **PoC 状态**：sidecar skill 当前仅支持 `review` action。这是早期集成（early integration），不承诺完整 IDE 插件生态。

---

## Demo 示例

以下是对一段含安全漏洞的 Python 代码做 `/review staged` 的真实输出：

**输入代码**（`app.py`，故意包含漏洞）：

```python
def get_user(name):
    query = "SELECT * FROM users WHERE name = '" + name + "'"  # SQL 注入
    return db.execute(query)

def get_password(user_id):
    return db.execute("SELECT password FROM users WHERE id = " + str(user_id))  # SQL 注入

def hash_password(pw):
    return pw ^ 0x12345678  # XOR 不是安全 hash

def check_access(user, action):
    return True  # 永远返回 True
```

**Sego 输出**（findings）：

| severity | file | line | title | confidence |
|---|---|---|---|---|
| critical | app.py | 6 | SQL injection in get_user — unsanitized string concatenation | 1.0 |
| critical | app.py | 11 | SQL injection in get_password — unsanitised integer concatenation | 1.0 |
| high | app.py | 15 | XOR is not a secure hash function | 0.9 |

每个 finding 包含完整的 evidence、risk、suggestion 和 verification_hint。审查结果持久化到 `.sego/reviews/`：

```
.sego/reviews/
├── index.jsonl                    # 索引（append-only）
├── review-{timestamp}-{hash}.json # 结构化 artifact
└── review-{timestamp}-{hash}.md   # 人类可读 markdown
```

---

## 架构

```
Sego Agent (Rust workspace, 9 crates)
├── rusty-claude-cli    CLI 主入口 + sidecar JSON 接口
├── runtime             核心运行时（session / permissions / code_review / recovery）
├── api                 多模型 provider（DeepSeek / Anthropic）
├── commands            slash 命令注册
├── tools               工具注册
├── plugins             插件系统
└── telemetry / compat-harness / mock-anthropic-service

.sego/ artifact（运行时产物，git 忽略）
├── reviews/            审查 artifact（JSON + MD + index）
└── recovery/           崩溃恢复状态

schema/                 JSON Schema 公开契约（仓库根，进 GitHub）
├── review-artifact.schema.json
├── review-index-entry.schema.json
└── sidecar-request-response.schema.json

skills/sego-review/     Sidecar skill 包（SKILL.md + 脚本）
```

- **纯 Rust**，`unsafe_code = "forbid"`，clippy pedantic
- **本地优先**：所有 artifact 存在项目 `.sego/` 目录，数据不出域
- **diff_hash 绑定**：review/verify 指向同一代码差异，防止"审查 A 提交 B"

---

## 开发

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

详细开发指南见 [DEVELOPMENT.md](DEVELOPMENT.md) 和 [AGENTS.md](AGENTS.md)。

---

## License

[MIT](LICENSE)
