# Security Policy

Sego currently publishes a static launch site on Cloudflare Pages and releases binaries through GitHub Releases.

## Official channels

- Website: https://sego-8dw.pages.dev/
- Repository: https://github.com/007M7/Sego-Agent
- Releases: https://github.com/007M7/Sego-Agent/releases/latest

If another site asks you to download Sego, submit private source code, or pay for an audit, verify it against the official repository first.

## Reporting security issues

Please do not post secrets, private source code, customer data, wallet private keys, or production credentials in public GitHub issues.

For now, open a GitHub issue with a minimal description and mark that it is security-related. We will confirm a private handoff path before requesting sensitive details.

## Payment safety

Overseas private audits currently use BNB Chain only after scope confirmation.

Do not send payment before the audit scope, price, delivery window, and payment details are confirmed. Do not trust wallet addresses posted by third parties. Treat any unsolicited payment request as suspicious.

## Release safety

Before running a downloaded binary:

1. Download from the official GitHub Releases page.
2. Prefer the latest release unless a specific version is required.
3. Check the file name and version.
4. If checksums are provided for a release, compare the local file hash before running it.

## Website hardening

The Cloudflare Pages site uses security headers in `docs/_headers` to reduce common browser-side risks:

- deny iframe embedding
- disable MIME sniffing
- restrict referrer leakage
- disable browser permissions that the static site does not need
- apply a restrictive Content Security Policy

These headers do not replace account security. GitHub and Cloudflare accounts must still use strong passwords and two-factor authentication.