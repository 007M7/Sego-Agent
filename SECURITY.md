# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in Sego Agent, please report it responsibly.

**Do not open a public GitHub issue.**

Instead, please:

1. Email the maintainer directly at the address listed on the GitHub profile
2. Provide a detailed description of the vulnerability
3. Include steps to reproduce if possible
4. Allow reasonable time for a fix before public disclosure

## Supported Versions

| Version | Supported |
|---------|-----------|
| Latest main branch | ✅ |

## Security Considerations for Users

- **API Keys**: Never commit API keys to version control. Use environment variables or the config system.
- **Permission Modes**: Use the most restrictive permission mode needed for your task (`read-only` > `workspace-write` > `danger-full-access`).
- **Sandbox**: Sego Agent includes sandboxing for shell commands. Review sandbox behavior for your platform.
- **MCP Servers**: Only connect to trusted MCP servers. MCP connections can execute arbitrary code.
- **Session Data**: Session data stored in `.sego/sessions/` may contain sensitive conversation context. Add to `.gitignore`.
