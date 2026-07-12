# Public Claim Boundary

Sego is a local-first review and engineering trust layer for code that has just been generated or changed. The public boundary below keeps product wording aligned with what the current repository can prove.

## Claims that are allowed

It is acceptable to say that Sego:

- reviews local Git changes and writes structured review artifacts under `.sego/reviews/`;
- helps developers inspect findings, evidence, risk, and suggested fixes before merge or handoff;
- provides local build/test/development workflows for the Rust workspace;
- can be integrated into agent or sidecar workflows where the caller supplies an explicit review scope;
- treats review artifacts as decision support for engineering acceptance.

## Claims that are not allowed without separate evidence

Do not claim that Sego:

- guarantees bug-free, vulnerability-free, compliant, or production-safe code;
- replaces human review, CI, security review, release QA, or deployment approval;
- has completed FAT, UAT, canary, production, SOC2, penetration test, or customer ROI validation;
- has remote CI green status for local-only commits that have not been pushed or checked by GitHub Actions;
- provides cryptographic provenance, supply-chain attestation, or legal certification;
- has release approval merely because tests passed or a release workflow exists.

## Required wording for high-risk use

For high-risk merges or releases, describe Sego as one input in a broader gate:

> Sego review artifacts provide local evidence and findings for engineering judgment. They are not exhaustive static analysis and do not by themselves approve merge, release, deployment, security, or compliance.

## Current release/documentation boundary

The repository contains public docs, schemas, and GitHub workflows. Some release workflow text may describe an older release body until a dedicated release-gate task updates it. Development Cutover does not reinterpret those historical notes as current release approval.

See also:

- [`RELEASE_QA_CAPABILITY_MATRIX.md`](RELEASE_QA_CAPABILITY_MATRIX.md)
- [`REVIEW_ARTIFACT_CONTRACT.md`](REVIEW_ARTIFACT_CONTRACT.md)
