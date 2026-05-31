# 0003 — Usage via polling + multi-strategy credential layer

**Status:** Accepted · **Date:** 2026-05-30

## Context
"Track usage of AI accounts" is the murkiest part of the brief. Most AI providers are
API-key based with limited/admin-only usage endpoints; true consumer OAuth into "AI
accounts" barely exists. The reference product (CodexBar) solves this with **privacy-first
credential reuse**: it harvests existing sessions (OAuth/device-flow tokens, browser
cookies, API keys in config, CLI creds/JSONL logs) and polls each provider's usage
endpoint read-only.

## Decision
**Poll provider usage/billing endpoints**, fed by a **multi-strategy credential-acquisition
layer**: OAuth (auth-code+PKCE), device flow, API key, browser-cookie reuse, and
CLI/local-config/JSONL discovery. **No metering proxy.** Results normalize into a common
`UsageWindow` model (session/weekly/monthly + reset countdowns).

## Alternatives considered
- **Local metering proxy** — exact, universal accuracy, but heavy (TLS/cert handling, users
  must re-point tools), and only sees traffic routed through it. Rejected for v1.
- **Poll APIs only (no credential reuse)** — simpler but worse coverage and worse UX (user
  must manually paste every key). The credential-reuse layer is what makes it pleasant.

## Consequences
- **+** Low effort per provider, read-only, no traffic interception, great onboarding.
- **−** Coverage/granularity varies wildly per provider; some need admin/org keys; some have
  no usage API. Tracked as a per-provider audit (OPEN_QUESTIONS D4/Q3).
- **−** Credential reuse is per-OS and somewhat fragile (browsers change formats); quarantined
  behind ports + sidecars (ADR 0008) and gated by consent (ADR 0012).
