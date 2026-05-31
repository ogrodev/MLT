# 0014 — v1 provider set

**Status:** Accepted · **Date:** 2026-05-30
**Backed by:** [research/PROVIDERS.md](../research/PROVIDERS.md)

## Context
We need a concrete, buildable v1 provider list rather than "40+ someday." Each provider is
its own auth + endpoint reality, and the research surfaced a hard constraint: API-cost data
for Anthropic/OpenAI requires org **admin keys** most individuals can't obtain.

## Decision
**v1 ships five providers**, in two categories:

**Subscription (plan-window usage):**
- **Codex** (ChatGPT/OpenAI subscription) — reads `~/.codex/auth.json`, polls `wham/usage`.
- **Claude Code** (Anthropic subscription) — reads `~/.claude/.credentials.json`, polls
  `api/oauth/usage` (needs a rate-limit gate + `claude-code/<ver>` User-Agent).

**API cost (billing):**
- **OpenRouter** — normal key, `/api/v1/key` + `/credits`. The easy win.
- **OpenAI API** — admin key preferred (`/v1/organization/*`); normal key → legacy balance only.
- **Anthropic API** — admin key only (`/v1/organizations/*`); no data with a normal key.

**Build order:** OpenRouter → Codex → Claude Code → OpenAI API → Anthropic API.

## Consequences
- **+** A focused, validated backlog; each provider has a documented mechanism + fixtures.
- **−** **Admin-key friction:** Anthropic/OpenAI API cost is unavailable to most solo users.
  The UI **must** detect normal-vs-admin keys and clearly state when cost data is unavailable.
- **−** Subscription endpoints are private/undocumented (see PROVIDERS §"Risk & ToS"); we
  accept this and mitigate with resilience patterns (ADR 0015).
- **Roadmap (post-v1):** **GitHub Copilot** is the easiest provider of all (standard device
  flow) — strong first addition. **Cursor** (cookie-only) and **Gemini** (private API + JS
  secret-scraping) are deferred until the cookie subsystem and their fragility are addressed.
