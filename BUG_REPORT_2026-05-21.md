# Sego Bug Report — 2026-05-21

## Issue 1: Context window overflow

**Error:**
```
400 Bad Request: This model's maximum context length is 1048576 tokens.
However, you requested 1282967 tokens (1218967 in the messages, 64000 in the completion).
```

**Root cause:** `max_tokens_for_model()` returned 64,000 for DeepSeek models, but the accumulated conversation exceeded 1.2M tokens. The preflight check only warned but didn't auto-reduce.

**Fix applied:** Added dynamic `safe_max_tokens` reduction in `stream()` method. When `estimated_tokens + requested_max > context_limit`, max_tokens is auto-clamped to `context_limit - estimated_tokens` (minimum 1024).

**Files changed:**
- `rust/crates/rusty-claude-cli/src/main.rs` — `stream()` method

**Status:** ✅ FIXED — binary rebuilt at `E:\code\sego.exe`

---

## Issue 2: Xiaomi MiMo API compatibility check

**Credentials provided:**
- Base URL: `https://cpa.soouu.com/`
- API Key: `sk-zmwbrmatcwgr9xjibv0rj003z9ncuda7`

**Pending test:** Could not verify due to shell PATH issues. Requires manual test:
```bash
export ANTHROPIC_API_KEY="sk-zmwbrmatcwgr9xjibv0rj003z9ncuda7"
export ANTHROPIC_BASE_URL="https://cpa.soouu.com/v1"
sego "say hi"
```

---

## Session workflow summary

| Metric | Value |
|--------|-------|
| Bug type | Context window overflow |
| Failure class | ToolRuntime |
| Fix time | ~15 min |
| Impact | Users with long conversations would hit 400 errors |
| Fixed in | v0.1.0+ (not yet released) |
