# 024 — macOS signed & notarized installer

> A normal macOS install with no Gatekeeper warning.

**Capability:** [§10 Distribution & updates](../PRD.md#10-distribution--updates) · **Status:** ◻ not started · **Depends on:** —

## User story
As a user, I want to install the macOS app the normal way and have it open without scary
security warnings.

## Scope
- **In:** A **signed and notarized** macOS build artifact and a documented install path.
- **Out:** Automating it in CI (027); auto-update (026). Requires the Apple Developer account
  (human prerequisite D1).

## Acceptance criteria
- [ ] The macOS build is **code-signed and notarized**.
- [ ] Installing it on a clean Mac opens the app with **no Gatekeeper warning** (no
      right-click-open workaround needed).
- [ ] The app appears as a real **menu-bar app** (no Dock entry) after install.
- [ ] A **documented macOS install path** exists for users.

## Done
Meets the [shared Definition of Done](./README.md#shared-definition-of-done-applies-to-every-task).
QA: fresh-Mac (or fresh user) install verified to launch without warnings.

## References
- [OPEN_QUESTIONS D1](../OPEN_QUESTIONS.md) · [human_prerequisites.md](../../human_prerequisites.md)
