Sego Agent for Windows

Quick start:
1. Double-click Sego.cmd.
2. If you use model calls, configure an API key first:
   setx DEEPSEEK_API_KEY "your-key"
   setx ANTHROPIC_API_KEY "your-key"

Workspace tips:
- Sego uses the folder where it was launched as the active workspace.
- Inside Sego, you can say: 切换到 D:\YourProject
- You can also start directly in a project:
  Sego.cmd --cwd "D:\YourProject"

You can also run sego.exe directly from a terminal, but Sego.cmd keeps errors visible.
