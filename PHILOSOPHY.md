# Sego Agent Philosophy

This document describes the design philosophy of Sego — a local-first AI code review and engineering trust layer that sits after AI coding tools.

It is not a technical manual. It explains *why* Sego exists and what it is — and is not — trying to be.

---

## AI code generation needs independent review

AI coding tools (Cursor, Claude Code, Codex, Copilot, and others) generate code faster than ever. That speed has a side effect:

> The bottleneck has moved from "writing code" to "deciding whether the code is safe to ship."

Generated code can be syntactically correct, look reasonable, and still contain real risks — SQL injection, hardcoded credentials, broken access control, unsafe shell, brittle architectural assumptions. The tools that write the code are not the best judges of whether the code should be committed:

- They have an inherent conflict of interest (they wrote it).
- They run inside a single session context, not across the whole project.
- Their "self-review" mode is often a comment, not a structured audit.

Sego is an **independent** review layer. It does not write the code. Its job is to look at the code AI tools just produced and give a structured opinion on whether the change is ready to commit.

---

## Local-first trust matters for private codebases

Most code that actually matters is private — internal services, customer projects, in-house libraries, regulated codebases. For that code, "send it to a third-party cloud service to review" is not always acceptable.

Sego is local-first:

- The CLI runs on your machine.
- Reviews execute against your local working tree.
- Artifacts are written to your project's `.sego/` directory.
- No source code, diff, or finding is implicitly shipped off-device by the tool itself.

You still pick the model provider (DeepSeek, Anthropic, etc.) for the actual review reasoning, and that provider sees the diff you submit to it — Sego is honest about that boundary. But everything around the model — the orchestration, the artifact, the recovery state — stays on your machine by default.

---

## Artifacts beat chat-only output

A "looks fine" reply from a chatbot is not an audit. You cannot file it, link to it from a PR, compare it with a later review, or feed it into a CI gate.

Sego treats every review as a real artifact:

- Structured **findings** (severity, file, line, evidence, risk, suggestion, confidence).
- A **Markdown report** that a human can read.
- A **JSON document** that another tool can consume.
- An **index** so reviews accumulate over time and can be inspected later.

When a review fails to parse cleanly, Sego says so explicitly (`parse_attempted_but_failed`) and points to the raw output. It does not silently present a broken review as "0 findings".

The artifact is the unit of trust. Anything that does not produce an artifact does not really exist.

---

## Human developers keep final judgment

Sego does not approve releases. It does not merge PRs. It does not decide that a change is "production ready."

A Sego review is **an input** to a human decision. It surfaces what looks risky and explains why; the developer (and, on team projects, the reviewer / release owner) decides what to do with it.

This is intentional. AI review can:

- Miss issues that need real product or business context.
- Flag things that look risky in isolation but are fine in the larger system.
- Disagree with itself across runs.

Treating Sego output as advisory keeps responsibility where it belongs — with the people shipping the software.

---

## Sego is an engineering trust layer, not a replacement for developers

There is a common narrative that AI will "replace developers." Sego is built on the opposite view:

> As AI generates more code, the scarce resource becomes judgment, taste, system design, and accountability — not raw typing speed.

In that world, the durable value of a developer is no longer "can write the code." It is:

- knowing what is worth building,
- recognizing what is risky to commit,
- understanding the system the code lives in,
- and standing behind the result.

Sego is a tool to make that judgment cheaper to exercise — by giving developers an independent, structured, local review they can actually trust as a starting point.

---

## What Sego is not

To make the boundary clear, Sego is intentionally not:

- **Not an IDE or code generator.** It does not autocomplete, refactor, or write features. Use your favorite AI coding tool for that.
- **Not a static analyzer.** It is a model-driven review. It is complementary to lint, SAST, fuzzing, and formal verification — not a replacement.
- **Not a guarantee of bug-free code.** A clean Sego review is one input among several (tests, CI, human review, security tooling). It is never the only gate.
- **Not a cloud platform.** It runs locally. Any future team or cloud features will be opt-in, on top of the local-first baseline.

---

## Short Version

AI writes code. Sego helps you decide whether it is safe to commit.

The code is the AI tool's output.
The judgment is still yours.
Sego is the structured layer in between.
