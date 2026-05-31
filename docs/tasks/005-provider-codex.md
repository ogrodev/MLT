# 005 — Codex subscription usage

> Connect my Codex subscription and see its usage windows next to Claude's.

**Capability:** [§3 Provider coverage](../PRD.md#3-provider-coverage) · **Status:** ◻ not started · **Depends on:** 002

## User story
As a Codex subscriber, I want my Codex usage tracked the same way Claude's is, so the number
reflects my real coding usage across tools.

## Scope
- **In:** Connecting Codex by reusing its existing local login, and rendering its normalized
  usage windows in the popover.
- **Out:** API-key providers (006–008); alarms on this data (009).

## Acceptance criteria
- [ ] Once connected, Codex shows its **usage windows** with **percent used** and a
      **human countdown to reset**, per §2.
- [ ] Data **auto-refreshes** with the rest and on opening the popover.
- [ ] If Codex data can't be fetched, the tile shows a **stale/error** state and keeps the
      last known values — other providers are unaffected.
- [ ] Codex identity/plan is shown **only** under Codex (provider data stays siloed).
- [ ] Adding Codex **does not change** the appearance or behaviour of existing providers.

## Done
Meets the [shared Definition of Done](./README.md#shared-definition-of-done-applies-to-every-task).
Tests use recorded fixtures; a live check is an `[ignore]`d example run by hand.

## References
- [ADR 0014 — v1 provider set](../adr/0014-v1-provider-set.md) · [research/PROVIDERS.md](../research/PROVIDERS.md)
