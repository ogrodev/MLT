# 0015 — Provider resilience patterns are core requirements

**Status:** Accepted · **Date:** 2026-05-30
**Backed by:** [research/PROVIDERS.md](../research/PROVIDERS.md)

## Context
Every subscription usage endpoint we depend on is private, undocumented, rate-limited, and
subject to schema drift (see PROVIDERS §"Risk & ToS"). CodexBar survives this with a set of
defensive patterns earned in production. Treating these as optional polish would make the
popover flicker, blank out, or crash on the first network blip or schema change.

## Decision
The following are **mandatory behaviors implemented in pure `core` logic** (testable via the
injected `Clock` + fakes), not per-provider afterthoughts:

1. **Lossy/optional decoding** — one malformed/added field never breaks a whole snapshot.
2. **Reset-time backfill** — carry a cached `resets_at` forward when a fetch omits it.
3. **Consecutive-failure gate** — suppress the *first* failure if prior data exists; surface
   an error only on the 2nd+ consecutive failure.
4. **Per-probe timeout** — race each fetch against a timeout; one slow provider can't stall
   the popover.
5. **Startup connectivity retry** — backoff-retry the first refresh (app launched before Wi-Fi).
6. **Per-provider rate-limit gate** — backoff / block-until (mandatory for Claude's 429s).
7. **Explicit staleness states** — `Ok | Stale | Error` on every `UsageWindow`; never panic.

## Alternatives considered
- **Add resilience reactively when bugs appear** — guarantees a flaky early product and
  re-discovering lessons the reference already paid for.
- **Push resilience into each provider** — duplication + drift; the whole point of the pure
  core is that these live once, above the providers.

## Consequences
- **+** A stable popover despite hostile upstreams; behaviors are unit-tested deterministically.
- **+** Centralized in `core` so every provider inherits them for free.
- **−** More upfront logic + tests before the first provider "works" end-to-end. Accepted.
