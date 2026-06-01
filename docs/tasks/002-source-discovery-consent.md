# 002 — Local source discovery + consent screen

> The app shows what it *could* connect to and lets me opt in, source by source, before any secret is read.

**Capability:** [§4 Connecting accounts](../PRD.md#4-connecting-accounts-credentials--consent), [§9 Privacy & security](../PRD.md#9-privacy--security) · **Status:** ✅ done · **Depends on:** —

## User story
As a user, I want to see which local sources (installed CLIs, browser logins) are available
and choose which to enable — with a plain note of what each accesses and why — so I stay in
control before anything reads a credential.

## Scope
- **In:** A connect/sources screen that lists discoverable sources, each with an enable/disable
  control and a plain-language access note. Discovery reads **only presence/metadata**.
- **Out:** API-key entry (003), disconnect (004), the per-provider usage itself (005–008).

## Acceptance criteria
- [x] The app **discovers** locally available sources and presents them in a list.
- [x] **Discovery reads only metadata** (presence) — **no secret is read** until I enable a source.
- [x] Each source has a **plain-language note** of what is accessed and why, shown **before** I opt in.
- [x] I can **enable or disable each source individually**.
- [x] Enabling a source takes effect **without restarting the app**; the already-shipped Claude
      Code source connects through this flow.

## Done
Meets the [shared Definition of Done](./README.md#shared-definition-of-done-applies-to-every-task).
QA note: confirm with a network/keychain check that nothing is read until opt-in.

## References
- [ADR 0012 — consent model](../adr/0012-consent-model.md)
