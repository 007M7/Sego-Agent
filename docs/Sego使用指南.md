# Sego 使用指南

> 本文件是 Word 版使用指南的在线阅读版。Word 下载版见 [Sego使用指南.docx](./Sego使用指南.docx)。

# Sego Agent
AI Coding 工程信任层

用户使用指南

写给所有用 AI 写代码的人，包括完全不懂代码的人

v0.1.3    2026-06-18

> 一句话：AI 负责帮你写代码，Sego 负责帮你检查这段代码是否安全、是否可靠、是否适合提交。

## 先看这页：你应该怎么用 Sego？

| 你的情况 | 最简单做法 | 你会得到什么 |
| --- | --- | --- |
| 只想马上用 | 下载 v0.1.3 的 sego-windows.zip，解压后双击 Sego.cmd | 打开一个不会闪退的 Sego 窗口 |
| 想用命令安装 | 复制 README 里的 PowerShell / bash 一键安装命令 | 自动安装到系统路径，并创建桌面快捷方式 |
| 想接入 Claude/Codex/Cursor | 先安装 Sego，再下载源码包，运行 skills/sego-review/install 脚本 | 让你的 AI 编码工具自动调用 Sego review |

> 重要区别：release zip 是给普通用户运行 Sego 的；源码 ZIP 里有 skill 包，适合接入 IDE。两者用途不同。

## 一、Sego 是什么？

Sego 不是另一个写代码的 AI，也不是完整 IDE。它更像一个独立的代码检查员：你让 AI 写完代码以后，用 Sego 再检查一遍。

- 它会指出潜在 bug、安全风险、危险命令、硬编码密钥、代码质量问题。
- 它会把结果保存到项目里的 .sego/reviews/，方便以后回看。
- 它可以单独在命令行使用，也可以被 Claude Code、Codex、Zcode、Cursor 等工具调用。
- 它当前的 IDE 接入还是 early integration / PoC，重点支持 review。
### 适合谁？

| AI coding 新手 | 你看不懂 AI 改了什么，但想知道能不能提交。 |
| --- | --- |
| 独立开发者 | 你想给 Cursor、Claude Code、Codex 产出的代码加一道独立检查。 |
| 团队试用 | 你想把 AI 代码审查结果保存下来，方便复盘和交接。 |

## 二、下载和安装

### 方法 A：Windows 直接下载使用（最推荐给普通用户）

1. 打开浏览器，进入 Sego 最新发布页：https://github.com/007M7/Sego-Agent/releases/latest
1. 下载 sego-windows.zip。
1. 解压这个 zip。
1. 双击 Sego.cmd。
1. 如果 Windows 提示未知发布者，确认你是从官方 GitHub 下载后，再选择继续运行。
> 从 v0.1.3 开始：Windows 用户下载 sego-windows.zip 后，解压双击就能打开 Sego。zip 内包含 sego.exe、Sego.cmd 和 README-WINDOWS.txt。

### 方法 B：Windows 一键安装

如果你愿意复制一行命令，可以用安装脚本。它会下载最新 sego.exe，加入 PATH，并创建桌面快捷方式。

```bash
irm https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.ps1 | iex
```

### 方法 C：Mac / Linux 安装

```bash
curl -fsSL https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.sh | bash
```

### 方法 D：源码编译（给开发者）

只有你已经安装 Rust，或者你想改 Sego 源码时，才需要这个方式。

```bash
git clone https://github.com/007M7/Sego-Agent.git
cd Sego-Agent/rust
cargo build --release
```

### 确认是否安装成功

```bash
sego --version
```

如果看到 Version 0.1.3 或更高版本，说明安装成功。

## 三、配置 AI 模型

Sego 做完整代码审查时需要连接一个大模型。最简单的选择是 DeepSeek，也可以用 Anthropic。

### Windows 配置 DeepSeek

```bash
setx DEEPSEEK_API_KEY "sk-你的key"
setx DEEPSEEK_MODEL "deepseek-v4-flash"
```

设置后请关闭旧窗口，重新打开 Sego 或 PowerShell。setx 是持久配置，桌面快捷方式也能读取。

### Mac / Linux 配置 DeepSeek

```bash
export DEEPSEEK_API_KEY="sk-你的key"
export DEEPSEEK_MODEL="deepseek-v4-flash"
```

### 可选：使用 Anthropic

```bash
setx ANTHROPIC_API_KEY "sk-你的key"        # Windows
export ANTHROPIC_API_KEY="sk-你的key"     # Mac / Linux
```

> 不要泄露 API Key：不要把 API Key 发到群里、截图里、公开仓库里，也不要让 AI 把你的 key 写进代码。

## 四、第一次使用：让 Sego 检查代码

Sego 最适合检查 Git 项目里的改动。你不懂 Git 也没关系，可以让 AI 帮你执行 git add。

### 提交前 review

```bash
cd 你的项目目录
git add -A
sego /review staged
```

Sego 会告诉你：哪个文件、哪一行、问题是什么、严重程度如何、应该怎么修。

### 快速安全扫描

```bash
sego /review safety staged
```

适合快速检查硬编码密码、危险命令、明显安全问题。

### 生成摘要

```bash
sego /review summary
```

适合把当前代码状态整理给自己、同伴或下一个 AI 工具。

### 查看历史记录

```bash
sego /review list
sego /review show <review-id>
sego /review mark <id> <finding-id> fixed
```

review 结果保存在项目的 .sego/reviews/ 目录里。

## 五、接入 IDE / AI 编码工具：安装 Sego skill

skill 可以理解成一小包说明书。安装后，Claude Code、Codex、Zcode 或 Cursor 里的 AI 就知道：当你让它 review 代码时，可以调用 Sego。

> 先装 Sego，再装 skill：release zip 只负责运行 Sego；skill 包在源码仓库的 skills/sego-review/ 目录里。要接入 IDE，需要下载源码 ZIP 或 clone 仓库。

### Windows：安装 skill

1. 先确认 sego --version 能正常显示版本。
1. 进入 GitHub 仓库页面：https://github.com/007M7/Sego-Agent
1. 点击 Code → Download ZIP，解压源码包。
1. 在解压后的 Sego-Agent 文件夹里打开 PowerShell。
1. 运行下面的命令。
```bash
powershell -ExecutionPolicy Bypass -File .\skills\sego-review\install.ps1
```

### Mac / Linux：安装 skill

> cd Sego-Agent
> bash skills/sego-review/install.sh

### 支持哪些工具？

| 工具 | 接入方式 | 说明 |
| --- | --- | --- |
| Claude Code | 复制 skill 包 | 重启后可让 Claude 调用 Sego review |
| Codex | 复制 skill 包 | 适合在 Codex 工作流里加独立审查 |
| Zcode | 复制 skill 包 | 适合三方协作开发时做 review |
| Cursor | 生成 .cursorrules | 当前不是原生扩展，是轻量规则接入 |

### 接入后怎么说？

你不需要记复杂命令，直接对 AI 说自然语言：

- 请用 Sego review 当前改动。
- 帮我检查这次 AI 写的代码有没有安全风险。
- 这次改动能不能提交？请用 Sego 看一下。
- 先用 Sego 做 review，再给我修复建议。
> 当前边界：sidecar skill 目前是 early integration / PoC，主要支持 review。它不是完整 IDE 插件，也不会替你自动提交代码。

## 六、推荐工作流：适合 vibe coding 新手

1. 先用 Cursor、Claude Code、Codex 等 AI 工具写代码。
1. 让项目能跑起来，至少确认没有明显报错。
1. 把改动加入暂存区：git add -A。
1. 先跑安全扫描：sego /review safety staged。
1. 再跑完整审查：sego /review staged。
1. 把 Sego 发现的问题交给 AI 修。
1. 修完后再跑一次 Sego。
1. 确认没有高风险问题后，再提交代码。
### 减少确认次数：review-trust

如果 review 过程中反复问你是否允许只读命令，可以使用 review-trust。它会放行常见只读检查，仍会拦住危险命令。

```bash
sego --permission-profile review-trust
```

适合 review / verify 场景，不建议把全权限当成默认设置。

### 崩溃恢复

如果窗口异常关闭，下次可以恢复最近一次会话。

```bash
sego --resume latest
```

Sego 不会重新执行上次的工具调用，只恢复上下文。

## 七、常见问题

Q：Sego 会修改我的代码吗？

A：默认不会。Sego 的核心定位是 review，它主要读取代码、分析问题、输出建议。

Q：我只下载 release zip，可以安装 IDE skill 吗？

A：不行。release zip 只包含运行文件。skill 在源码仓库里，需要下载源码 ZIP 或 git clone。

Q：下载 GitHub 源码 ZIP 能直接运行吗？

A：源码 ZIP 本身不包含 sego.exe。Windows 用户可以双击 start-sego-windows.cmd，它会联网下载最新 release binary。

Q：为什么需要 API Key？

A：完整 review 需要调用大模型。没有 API Key 时，只能使用少量不调模型的本地检查。

Q：review 结果在哪里？

A：保存在当前项目的 .sego/reviews/ 目录。

Q：可以审查什么语言？

A：常见语言都可以，例如 Python、JavaScript、TypeScript、Rust、Go、Java、C++。

Q：Cursor 是原生插件吗？

A：目前不是。Cursor 通过 .cursorrules 轻量接入。

## 八、一页纸速查表

| 最新下载页 | https://github.com/007M7/Sego-Agent/releases/latest |
| --- | --- |
| Windows 直接用 | 下载 sego-windows.zip → 解压 → 双击 Sego.cmd |
| Windows 一键安装 | irm https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.ps1 \| iex |
| Mac / Linux 安装 | curl -fsSL https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.sh \| bash |
| 配置 DeepSeek | setx DEEPSEEK_API_KEY "sk-你的key"（Windows） |
| 第一次 review | git add -A → sego /review staged |
| 安全扫描 | sego /review safety staged |
| 历史记录 | sego /review list；sego /review show <id> |
| 恢复会话 | sego --resume latest |
| 安装 skill | 下载源码 ZIP → 运行 skills/sego-review/install 脚本 → 重启 IDE |

> 最后提醒：Sego 是代码审查工具，不是万能安全扫描器。重要项目仍建议人工 review、测试和备份。
