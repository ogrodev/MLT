# 025 — Windows signed installer

> A normal Windows install with no SmartScreen warning.

**Capability:** [§10 Distribution & updates](../PRD.md#10-distribution--updates) · **Status:** ◻ not started · **Depends on:** —

## User story
As a Windows user, I want to install the app the normal way without SmartScreen blocking or
warning me.

## Scope
- **In:** A **signed** Windows installer artifact and a documented install path.
- **Out:** Automating it in CI (027); auto-update (026). Requires the Windows code-signing
  certificate (human prerequisite D2).

## Acceptance criteria
- [ ] The Windows build is **code-signed**.
- [ ] Installing it on a clean Windows machine completes with **no SmartScreen warning**.
- [ ] The app appears in the **tray** and runs after install.
- [ ] A **documented Windows install path** exists for users.

## Done
Meets the [shared Definition of Done](./README.md#shared-definition-of-done-applies-to-every-task).
QA: clean-Windows install verified to complete without warnings.

## References
- [OPEN_QUESTIONS D2](../OPEN_QUESTIONS.md) · [human_prerequisites.md](../../human_prerequisites.md)
