# Sego 使用指南

> 本文件是 Word 版使用指南的在线阅读版。Word 下载版见 [Sego使用指南.docx](./Sego使用指南.docx)。

版本：v0.1.6
更新日期：2026-06-19

Sego 是一个面向 AI coding 的本地代码审查工具。你可以先让 Claude Code、Codex、Cursor、Zcode 等 AI 工具写代码，再用 Sego 做一次独立检查，确认改动是否安全、是否可靠、是否适合提交。

## 1. 这次 v0.1.6 更新了什么

v0.1.6 是一个小版本更新，重点解决“用户下载的 release 版本落后 main”的问题，并带来 Cycle 14 的体验优化。

- `sego review` 现在默认执行代码审查，并把结果保存到 `.sego/reviews/`。
- 审查结果会生成 `.md`、`.json`、`INDEX.md` 和 `index.jsonl`。
- 新增自然语言本地动作路由，可以直接说“帮我 review 当前改动”“检查更新”“切换到 D:\Project”。
- 新增 `/dir`，用于查看常用命令和自然语言说法。
- 旧的会话复盘入口改为 `sego workflow-review` 或 `sego session-review`，不再和代码审查混淆。

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

如果你愿意复制一行命令，可以用安装脚本。它会下载最新版 Sego，并创建桌面快捷方式。

```powershell
irm https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.ps1 | iex
```

### Mac / Linux

```bash
curl -fsSL https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.sh | bash
```

### 从源码构建

只有你想开发 Sego，或者需要本地编译最新版时，才需要这种方式。

```bash
git clone https://github.com/007M7/Sego-Agent.git
cd Sego-Agent/rust
cargo build --release
```

## 4. 从旧版本升级

如果你已经安装过 Sego，优先运行：

```powershell
sego update
```

检查是否已经升级成功：

```powershell
sego --version
```

看到 `Version 0.1.6` 或更高版本即可。

如果旧版 `sego update` 失败，请直接重新下载 `sego-windows.zip`，解压后用新的 `Sego.cmd`。

## 5. 配置模型 API Key

完整代码审查需要连接一个大模型。推荐先配置 DeepSeek，也可以使用 Anthropic。

### Windows 配置 DeepSeek

```powershell
setx DEEPSEEK_API_KEY "sk-你的key"
setx DEEPSEEK_MODEL "deepseek-v4-flash"
```

设置后请关闭旧窗口，重新打开 Sego 或 PowerShell。

### Mac / Linux 配置 DeepSeek

```bash
export DEEPSEEK_API_KEY="sk-你的key"
export DEEPSEEK_MODEL="deepseek-v4-flash"
```

### 可选：Anthropic

```powershell
setx ANTHROPIC_API_KEY "sk-你的key"
```

```bash
export ANTHROPIC_API_KEY="sk-你的key"
```

不要把 API Key 发到群里、截图里、公开仓库里，也不要让 AI 把 key 写进代码。

## 6. 第一次代码审查

进入你的项目目录：

```powershell
cd E:\YourProject
```

如果你已经改了代码，可以直接运行：

```powershell
sego review
```

这会审查当前工作区改动，并生成标准报告文件。

如果你熟悉 Git，建议先暂存改动再审查：

```powershell
git add -A
sego review staged
```

审查结果会保存在当前项目的：

```text
.sego/reviews/
```

常见文件包括：

```text
.sego/reviews/
  INDEX.md
  index.jsonl
  review-xxxx.md
  review-xxxx.json
```

查看历史：

```powershell
sego review list
sego review show <review-id>
```

标记某个问题已修复：

```powershell
sego review mark <review-id> <finding-id> fixed
```

## 7. 常用自然语言说法

v0.1.6 开始，很多本地动作可以直接用自然语言表达。Sego 会优先在本地确定性执行这些动作，避免把控制类请求交给模型乱猜。

| 你可以这样说 | Sego 会做什么 |
|---|---|
| 当前工作区在哪 | 显示当前工作区 |
| 切换到 `D:\Project` | 切换工作区 |
| 帮我 review 当前改动 | 执行代码审查 |
| 检查已暂存代码的安全风险 | 执行 staged 安全审查 |
| 把刚才的审查结果写成 `E:\code\review.md` | 导出最近结果 |
| 导出当前会话 | 保存会话内容 |
| 检查更新 | 只检查新版本，不安装 |
| 更新到最新版 | 执行更新流程 |
| 退出 | 退出 Sego |

如果 Sego 判断你像是在说本地动作，但缺少路径或对象，会提示你补全。输入：

```text
/dir
```

可以查看常用命令和自然语言示例。

## 8. 代码审查相关命令

| 命令 | 用途 |
|---|---|
| `sego review` | 审查当前工作区改动，并保存报告 |
| `sego review staged` | 审查已暂存改动 |
| `sego review list` | 查看已保存的审查报告 |
| `sego review show <id>` | 打开某个审查报告 |
| `sego review status <id>` | 查看 finding 状态 |
| `sego review mark <id> <finding-id> fixed` | 标记问题已修复 |
| `/review safety staged` | 快速安全检查 |
| `/review ready` | 提交前 readiness gate |
| `/review summary` | 生成交付摘要 |
| `/verify fast` | 执行快速验证计划 |

旧的会话复盘请使用：

```powershell
sego workflow-review
sego workflow-review --last 5
```

## 9. 推荐工作流

适合 AI coding 新手：

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

如果审查过程中只读命令频繁要求确认，可以在受信任项目里使用：

```powershell
sego --permission-profile review-trust
```

`review-trust` 会减少常见只读检查的确认次数，但不会把危险命令全部放行。

## 10. 接入其他 AI 工具

Sego 可以通过 `skills/sego-review/` 被其他 AI coding 工具调用。当前这是 early integration / PoC，主要支持 review 场景，不是完整 IDE 插件。

基本步骤：

1. 先安装 Sego，并确认 `sego --version` 正常。
2. 下载或 clone Sego 源码仓库。
3. 进入源码目录。
4. 运行对应安装脚本。

Windows：

```powershell
powershell -ExecutionPolicy Bypass -File .\skills\sego-review\install.ps1
```

Mac / Linux：

```bash
bash skills/sego-review/install.sh
```

接入后，可以对 AI 工具这样说：

- 请用 Sego review 当前改动。
- 先用 Sego 检查这次 AI 写的代码有没有安全风险。
- 这次改动能不能提交？请用 Sego 看一下。
- 先用 Sego 做 review，再给我修复建议。

## 11. 崩溃恢复和会话恢复

如果窗口异常关闭，下次可以恢复最近会话：

```powershell
sego --resume latest
```

Sego 会恢复上下文，但不会自动重新执行上次的工具调用。

## 12. 常见问题

### 为什么我运行 `sego review` 没有生成报告？

先检查版本：

```powershell
sego --version
```

如果低于 `0.1.6`，请先运行：

```powershell
sego update
```

或者重新下载 `sego-windows.zip`。从 v0.1.6 开始，裸 `sego review` 才默认生成标准审查报告。

### 报告保存在哪里？

保存在当前工作区的 `.sego/reviews/`。如果你先切换到了 `E:\YourProject`，报告就在：

```text
E:\YourProject\.sego\reviews\
```

如果你从桌面快捷方式直接打开，当前工作区可能是 `E:\Sego`。可以在 Sego 中输入：

```text
当前工作区在哪
```

或：

```text
/workspace
```

### Sego 会修改我的代码吗？

默认不会。Sego 的核心定位是审查和提示风险。它主要读取代码、分析问题、输出建议。

### 没有 API Key 能用吗？

可以使用少量本地检查，但完整 code review 需要模型 API Key。

### 下载 GitHub 源码 ZIP 能直接运行吗？

源码 ZIP 不包含编译好的 `sego.exe`。Windows 用户如果想直接运行，请下载 release 里的 `sego-windows.zip`。

### `/dir` 是列文件目录吗？

不是。`/dir` 是 Sego 的常用动作目录，用来查看常用命令和自然语言说法。查看文件目录应使用系统命令或让 Sego 执行对应读取操作。

## 13. 一页速查

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
| 查看审查历史 | `sego review list` |
| 查看常用说法 | `/dir` |
| 会话复盘 | `sego workflow-review` |
| 恢复会话 | `sego --resume latest` |

最后提醒：Sego 是 AI coding 的工程信任层，不是万能安全扫描器。重要项目仍建议人工 review、测试和备份。
