# 004 — Disconnect a provider without restart

> I can cleanly disconnect any provider, and its data and secret go away immediately.

**Capability:** [§4 Connecting accounts](../PRD.md#4-connecting-accounts-credentials--consent) · **Status:** ◻ not started · **Depends on:** 002

## User story
As a user, I want to disconnect a provider I no longer want tracked and trust that its
credential is removed and its tile disappears — without restarting the app.

## Scope
- **In:** A disconnect action for any connected provider and the cleanup it triggers.
- **Out:** Connecting / key entry (002, 003).

## Acceptance criteria
- [ ] Each connected provider has a **disconnect** action.
- [ ] Disconnecting **removes that provider's tile** from the popover and stops its refresh.
- [ ] Any secret the app cached for that provider is **removed from the OS keychain**
      (the app never writes back to a vendor's own credential store).
- [ ] Disconnect takes effect **without restarting** the app, and the provider can be
      **reconnected** afterwards.

## Done
Meets the [shared Definition of Done](./README.md#shared-definition-of-done-applies-to-every-task).
QA note: confirm via keychain that our cached item is gone and the vendor's item is untouched.

## References
- [ADR 0014 — v1 provider set](../adr/0014-v1-provider-set.md)
