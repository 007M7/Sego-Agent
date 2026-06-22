# Sego 使用指南

> 本文件是 Word 版使用指南的在线阅读版。Word 下载版见 [Sego使用指南.docx](./Sego使用指南.docx)。

版本：v0.1.8
更新日期：2026-06-22

Sego 是一个面向 AI coding 的本地代码审查工具。你可以先让 Claude Code、Codex、Cursor、Zcode 等 AI 工具写代码，再用 Sego 做一次独立检查，确认改动是否安全、是否可靠、是否适合提交。

## 1. 这次 v0.1.8 更新了什么

v0.1.8 重点补全了代码审查的覆盖范围和自然语言路由安全性。

- **完整仓库审计模式**：`sego review --full <路径>` 可以在干净的克隆仓库甚至非 Git 目录中审查整个项目，不再依赖 `git diff`。会读取完整文件树、关键 manifest 文件和入口文件，跳过 `.git`、`node_modules`、虚拟环境和构建产物。
- **审查解析器增强**：支持裸 JSON、fenced JSON 和嵌入 prose 的 pretty JSON 解析。新增 `parse_attempted_but_failed` 状态，避免模型已输出 findings 但解析失败时显示误导性的「Findings 0」。
- **导出最新回复增强**：输出增加 `Kind`（markdown）和 `Bytes` 字段；空响应时提供清晰的 recovery hint，不再直接报错退出。
- **自然语言本地动作安全性加固**：导出最新回复必须显式包含 `last/previous/刚才/上一条/上回`，防止「保存报告」「导出 md」等模糊表达触发意外导出。模糊表达会路由到 `/dir` 提示。
- **非 Git 目录审查恢复提示**：不再显示原始 `git fatal` 错误，改为结构化 recovery hint，引导用户使用 `sego review --full`。
- **`/dir` 动作目录升级**：增加英文对照、使用示例和安全注意事项。

v0.1.8 之前的 v0.1.7 更新内容：
- `sego review` 默认执行代码审查，保存到 `.sego/reviews/`。
- 新增自然语言本地动作路由。
- 旧的会话复盘入口改为 `sego workflow-review` 或 `sego session-review`。

## 2. 最快开始

| 你的情况 | 推荐做法 | 结果 |
|---|---|---|
| Windows 普通用户 | 下载 `sego-windows.zip`，解压后双击 `Sego.cmd` | 打开 Sego 窗口，适合直接使用 |
| 已安装旧版 Sego | 在终端运行 `sego update` | 自动升级到最新 release |
| Mac / Linux 用户 | 下载对应二进制，或运行安装脚本 | 在终端使用 `sego` |
| 想接入 Claude/Codex/Cursor/Zcode | 先安装 Sego，再安装 `skills/sego-review` | 让其他 AI 工具调用 Sego review |
| 开发者 | clone 仓库后 `cargo build --release` | 从源码构建最新版 |

## 3. 下载和安装

### Windows：直接下载使用

1. 打开最新发布页：<https://github.com/007M7/Sego-Agent/releases/latest>
2. 下载 `sego-windows.zip`。
3. 解压 zip。
4. 双击 `Sego.cmd`。
5. 如果 Windows 提示未知发布者，请确认文件来自官方 GitHub 后再继续运行。

`sego-windows.zip` 内应包含：

- `sego.exe`
- `Sego.cmd`
- `README-WINDOWS.txt`

### Windows：一键安装

```powershell
irm https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.ps1 | iex
```

### Mac / Linux

```bash
curl -fsSL https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.sh | bash
```

### 从源码构建

```bash
git clone https://github.com/007M7/Sego-Agent.git
cd Sego-Agent/rust
cargo build --release
```

## 4. 从旧版本升级

```powershell
sego update
```

检查版本：

```powershell
sego --version
```

看到 `Version 0.1.8` 或更高即可。如果旧版 `sego update` 失败，请直接重新下载 `sego-windows.zip`。

## 5. 配置模型 API Key

完整代码审查需要连接大模型。推荐 DeepSeek，也可使用 Anthropic。

### Windows

```powershell
setx DEEPSEEK_API_KEY "sk-你的key"
setx DEEPSEEK_MODEL "deepseek-v4-flash"
```

设置后关闭旧窗口，重新打开 Sego 或 PowerShell。

### Mac / Linux

```bash
export DEEPSEEK_API_KEY="sk-你的key"
export DEEPSEEK_MODEL="deepseek-v4-flash"
```

不要把 API Key 发到群里、截图里、公开仓库里。

## 6. 第一次代码审查

### 审查当前改动

```powershell
cd E:\YourProject
sego review
```

### 审查已暂存改动

```powershell
git add -A
sego review staged
```

### 完整仓库审计（干净克隆、非 Git 目录）

```powershell
# 审计整个仓库
sego review --full E:\repo

# 审计当前目录（无需 Git）
sego review --full .
```

此模式会遍历完整目录树，读取关键 manifest 文件和入口文件，跳过 `.git`、`node_modules`、`target`、`.venv` 等不需要的目录。

### 审查结果位置

```text
.sego/reviews/
  index.jsonl
  review-xxxx.md
  review-xxxx.json
```

- 普通 review 保存到当前工作区的 `.sego/reviews/`
- `sego review --full E:\repo` 的审查结果保存到 `E:\repo\.sego\reviews\`

### 查看和管理历史

```powershell
sego review list                    # 查看已保存的审查
sego review show <review-id>        # 打开某个审查报告
sego review mark <id> <finding-id> fixed   # 标记问题已修复
```

## 7. 常用自然语言说法

以下说法会触发本地动作，不会交给模型乱猜：

| 你可以这样说 | Sego 会做什么 |
|---|---|
| 当前工作区在哪 | 显示当前工作区 |
| 切换到 `D:\Project` | 切换工作区 |
| 帮我 review 当前改动 | 执行代码审查 |
| 检查已暂存代码的安全风险 | 执行 staged 安全审查 |
| 把**刚才**的审查结果写成 `E:\code\review.md` | 导出最近结果 |
| 导出当前会话 | 保存会话内容 |
| 检查更新 | 只检查新版本，不安装 |
| 更新到最新版 | 执行更新流程 |
| 退出 | 退出 Sego |

**重要安全边界**：导出最新回复（如审查结果）必须包含 `刚才/上一条/上回/last/previous`。只说「保存报告」「导出为 md」而没有明确指向哪个回复时，Sego 会提示 `/dir`，不会自动执行导出。

输入 `/dir` 可以查看完整动作目录和示例。

## 8. 代码审查相关命令

| 命令 | 用途 |
|---|---|
| `sego review` | 审查当前工作区改动 |
| `sego review staged` | 审查已暂存改动 |
| `sego review --full <路径>` | 完整仓库审计（无需 Git diff） |
| `sego review list` | 查看已保存的审查报告 |
| `sego review show <id>` | 打开某个审查报告 |
| `sego review status <id>` | 查看 finding 状态 |
| `sego review mark <id> <finding-id> fixed` | 标记问题已修复 |
| `/review safety staged` | 快速安全检查 |
| `/review ready` | 提交前 readiness gate |

## 9. 导出会话和审查结果

| 操作 | 命令 |
|---|---|
| 导出当前完整会话 | `/export` 或 `/export E:\code\session.md` |
| 导出最新审查结果 | 在 REPL 中说「把**刚才**的审查结果写成 E:\review.md」 |
| 导出最新回复为英文 | `save the last review report to PR43-review.md` |

如果只说「保存报告」「导出 md」等模糊表达，Sego 会显示 `/dir` 帮助信息，不会自动导出错误内容。

导出输出包含：

- `Result` — 写入成功/失败
- `Kind` — markdown
- `Bytes` — 文件大小
- `File` — 完整路径

## 10. 非 Git 目录怎么办

如果你在非 Git 目录运行 `sego review`，Sego 会显示结构化 recovery hint：

```
Review
  Result           failed
  Reason           no Git repository found
  Workspace        E:\code
  Next step        Run `sego review --full <path>` for non-Git directories, or use /dir.
```

请改用完整仓库审计模式：

```powershell
sego review --full .
```

## 11. 推荐工作流

1. 先用 Claude Code、Codex、Cursor 或其他 AI 工具写代码。
2. 确认项目至少能启动或没有明显报错。
3. 运行 `git add -A` 暂存改动。
4. 运行 `sego review staged`。
5. 把 Sego 发现的问题交给 AI 修复。
6. 修完后再次运行 `sego review staged`。
7. 没有高风险问题后，再提交代码。

如果只是想快速安全检查：

```powershell
sego /review safety staged
```

## 12. 接入其他 AI 工具

Sego 可以通过 `skills/sego-review/` 被其他 AI coding 工具调用。

基本步骤：

1. 先安装 Sego，确认 `sego --version` 正常。
2. 下载或 clone Sego 源码仓库。
3. 进入源码目录。
4. 运行安装脚本。

Windows：

```powershell
powershell -ExecutionPolicy Bypass -File .\skills\sego-review\install.ps1
```

Mac / Linux：

```bash
bash skills/sego-review/install.sh
```

接入后，可以对 AI 工具说：

- 请用 Sego review 当前改动。
- 先用 Sego 检查这次 AI 写的代码有没有安全风险。
- 这次改动能不能提交？请用 Sego 看一下。

## 13. 崩溃恢复和会话恢复

```powershell
sego --resume latest
```

Sego 会恢复上下文，但不会自动重新执行上次的工具调用。

## 14. 常见问题

### 为什么审查结果显示「Findings unknown (parse failed)」？

这说明模型输出了类似 findings 的内容，但 JSON 解析器没能提取出来。**打开 Markdown 报告**（`.sego/reviews/review-xxxx.md`）查看原始输出即可。这个问题已被跟踪为 `C20.5-REVIEW-003`，后续版本会改进。

### 报告保存在哪里？

保存在当前工作区（或 `--full` 指定的目标目录）的 `.sego/reviews/` 下。

### Sego 会修改我的代码吗？

默认不会。Sego 的核心定位是审查和提示风险。

### 没有 API Key 能用吗？

可以使用少量本地检查，但完整 code review 需要模型 API Key。

### `/dir` 是列文件目录吗？

不是。`/dir` 是 Sego 的常用动作目录，用来查看命令和自然语言示例。

## 15. 一页速查

| 事项 | 命令或入口 |
|---|---|
| 最新下载 | <https://github.com/007M7/Sego-Agent/releases/latest> |
| Windows 直接用 | 下载 `sego-windows.zip`，解压，双击 `Sego.cmd` |
| Windows 一键安装 | `irm https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.ps1 \| iex` |
| Mac / Linux 安装 | `curl -fsSL https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.sh \| bash` |
| 检查版本 | `sego --version` |
| 升级 | `sego update` |
| 配置 DeepSeek | `setx DEEPSEEK_API_KEY "sk-你的key"` |
| 当前工作区 | `当前工作区在哪` 或 `/workspace` |
| 切换目录 | `切换到 D:\YourProject` 或 `/cd "D:\YourProject"` |
| 审查当前改动 | `sego review` |
| 审查暂存改动 | `git add -A` 后 `sego review staged` |
| 完整仓库审计 | `sego review --full <路径>` |
| 查看审查历史 | `sego review list` |
| 查看常用说法 | `/dir` |
| 导出最新审查结果 | 「把刚才的审查结果写成 E:\review.md」 |
| 恢复会话 | `sego --resume latest` |

最后提醒：Sego 是 AI coding 的工程信任层，不是万能安全扫描器。重要项目仍建议人工 review、测试和备份。

## Windows 运行提示（v0.1.8）

- **完整仓库审计**：`sego review --full .` 可在任意目录运行，不依赖 Git。
- **非 Git 目录**：运行 `sego review` 会提示使用 `--full` 模式，不再显示原始 git 错误。
- **交互命令防护**：copy con、裸 shell 等自动阻止。
- **Shell 命令提示**：Windows 下使用 type/dir/where。
- **导出安全边界**：导出最新回复必须说「刚才/上一条/last/previous」。
