# 019 — Wake-from-sleep catch-up refresh

> After my Mac wakes, usage refreshes promptly instead of showing stale numbers.

**Capability:** [§7 Reliability & always-on](../PRD.md#7-reliability--always-on) · **Status:** ◻ not started · **Depends on:** —

## User story
As a user, I want the app to refresh usage soon after my machine wakes from sleep, so I'm not
looking at hours-old numbers — and a flaky just-woke network shouldn't show a hard failure.

## Scope
- **In:** Detecting wake/resume and triggering a catch-up usage refresh; tolerating the
  transient post-wake network state.
- **Out:** Missed-*alarm* catch-up (013); start-at-login (018). (Steady-state retry/backoff
  and the 60s refresh loop already ship under §2/§7.)

## Acceptance criteria
- [ ] After the machine **wakes from sleep**, usage refreshes **promptly** rather than waiting
      for the next scheduled cycle.
- [ ] Transient network errors right after wake are **retried**, not surfaced as a hard failure.
- [ ] A provider that's still unreachable after retries shows **stale/error** and recovers
      automatically; other providers are unaffected.
- [ ] The **last-updated** time reflects the post-wake refresh.

## Done
Meets the [shared Definition of Done](./README.md#shared-definition-of-done-applies-to-every-task).
Tests simulate wake + transient failure with fakes; QA verifies on a real sleep/wake cycle.

## References
- [ADR 0015 — resilience patterns](../adr/0015-resilience-patterns.md)
