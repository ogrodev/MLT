# 0002 — Local-only topology + OS login-item

**Status:** Accepted · **Date:** 2026-05-30

## Context
Usage history accumulates over time, and some "always-on tracking" instinct pushes toward
a server. We must decide whether there's any backend at all. A server would add ops cost,
attack surface, and a harder privacy story.

## Decision
**100% local.** SQLite store, secrets in the OS keychain, OAuth tokens local, scheduler
runs in-process. Register an **OS login-item** so the app auto-starts into the tray and
polls whenever the machine is on. No backend.

## Alternatives considered
- **Local + thin sync/relay backend** — enables cross-device sync, secret brokering, and
  24/7 polling, but adds ops, attack surface, and weakens privacy. Not justified for v1.
- **Design-for-both behind a trait** — we keep this option implicitly: all data access is
  already behind ports (ADR 0006), so a `RemoteGateway` could slot in later without a rewrite.

## Consequences
- **+** Max privacy, no ops, cheap, simple to reason about.
- **−** No cross-device sync, no team features, no polling while fully quit.
- The "polling while quit" gap is mitigated by the login-item + alarm catch-up reconciler
  (ADR 0009); the only true blind spot is "machine off," which self-heals on next launch.
