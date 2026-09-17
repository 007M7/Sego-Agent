# Security Policy

Sego currently publishes a static launch site on Cloudflare Pages and releases binaries through GitHub Releases.

## Official channels

- Website: https://sego-8dw.pages.dev/
- Repository: https://github.com/007M7/Sego-Agent
- Releases: https://github.com/007M7/Sego-Agent/releases/latest

If another site asks you to download Sego, submit private source code, or pay for an audit, verify it against the official repository first.

## Reporting security issues

Report vulnerabilities through GitHub Private Advisories, which is enabled for this repository:

https://github.com/007M7/Sego-Agent/security/advisories/new

That channel stays private between you and the maintainer, so a fix can be prepared before anything is disclosed. Do not post secrets, private source code, customer data, wallet private keys, or production credentials in a public GitHub issue.

A normal public issue is fine for reports that are not sensitive on their own — a hardening suggestion, a missing header, or a bug with no exploitable path. If you are unsure which applies, choose the private channel.

A useful report includes the Sego version or commit, the platform, what you ran, what happened, and what you expected. A minimal reproduction is worth more than a long description. There is no bug bounty and no paid disclosure programme.

## Payment safety

Private audits run only after the audit scope, delivery window, and handling process are confirmed in writing.

Do not send payment, credentials, or private code before that confirmation. Do not trust wallet addresses or payment instructions posted by third parties. Treat any unsolicited payment request as suspicious.

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