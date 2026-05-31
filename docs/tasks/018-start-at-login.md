# 018 — Start at login (toggle)

> The app can launch itself at login so my usage is current without me opening it.

**Capability:** [§7 Reliability & always-on](../PRD.md#7-reliability--always-on) · **Status:** ◻ not started · **Depends on:** —

## User story
As a user, I want the app to start automatically when I log in, so tracking is already current
when I sit down — and I want to turn that off if I prefer.

## Scope
- **In:** A "start at login" setting and the OS login-item registration behind it.
- **Out:** Wake/sleep catch-up (019).

## Acceptance criteria
- [ ] There is a **"start at login"** toggle, **off or on** per my choice.
- [ ] When **on**, the app launches into the menu bar at next login (no window, no Dock entry).
- [ ] When **off**, the app does **not** auto-launch and any login-item registration is removed.
- [ ] The setting **persists** across restarts and reflects the true OS login-item state.

## Done
Meets the [shared Definition of Done](./README.md#shared-definition-of-done-applies-to-every-task).
QA on macOS confirms the login item appears/disappears with the toggle.

## References
- [OPEN_QUESTIONS O3](../OPEN_QUESTIONS.md)
