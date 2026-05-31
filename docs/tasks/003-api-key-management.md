# 003 — Enter / replace / remove an API key

> For providers that need a key, I can paste one in, swap it, or remove it — safely.

**Capability:** [§4 Connecting accounts](../PRD.md#4-connecting-accounts-credentials--consent) · **Status:** ◻ not started · **Depends on:** 002

## User story
As a user, I want to enter an API key for a provider that needs one, replace it later, and
remove it — knowing the key is stored safely and never shown back to me in full.

## Scope
- **In:** The key-entry UI and its lifecycle (add / replace / remove) for one API provider as
  the proving ground (e.g. OpenRouter).
- **Out:** Reading usage with that key (006–008); disconnecting a whole provider (004).

## Acceptance criteria
- [ ] I can **enter** an API key for a provider that requires one, and it takes effect
      **without restarting** the app.
- [ ] I can **replace** an existing key and **remove** it.
- [ ] The key is stored **only in the OS keychain** — **never** shown again in full, never
      written to logs or the local database.
- [ ] An invalid/rejected key shows a **clear error** and does not silently appear connected.

## Done
Meets the [shared Definition of Done](./README.md#shared-definition-of-done-applies-to-every-task).
QA note: verify the key never appears in logs or the DB, and is masked in the UI after entry.

## References
- [ADR 0012 — consent model](../adr/0012-consent-model.md)
