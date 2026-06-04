# 009 — Threshold usage alerts → notifications

> Get warned before I hit a limit, via a native notification, once per crossing.

**Capability:** [§5 Alarms & notifications](../PRD.md#5-alarms--notifications) · **Status:** 🟡 partial · **Depends on:** 005–008

## User story
As a user, I want to be notified when a provider/window crosses a usage threshold I set, so
I can slow down before I run out — without being nagged repeatedly.

> This task also establishes **native OS notification delivery**, which alarms 010–013 reuse.

## Scope
- **In:** Per provider/window threshold alerts and their delivery as native notifications.
- **Out:** Reset notifications (010), user-defined alarms (011–012).

## Acceptance criteria
- [x] I can enable **threshold alerts per provider/window** at chosen levels (e.g. 80% and 95%).
- [x] Each threshold fires **once per crossing** and **re-arms** after that window resets.
- [ ] Alerts are delivered through the **OS notification centre** and respect **Do-Not-Disturb**.
- [ ] The notification clearly names the **provider, window, and level** crossed.
- [x] Disabling an alert stops it without affecting others.

## Done
Meets the [shared Definition of Done](./README.md#shared-definition-of-done-applies-to-every-task).
Tests use a fake clock + fixture usage to prove "once per crossing" and re-arm behaviour.

**Status note (2026-06-04):** Implemented end-to-end — `ThresholdConfig`/`evaluate_thresholds`
(core, unit-tested for once-per-crossing + re-arm + disabled), persisted armed state, the
`set_threshold_alert` command, the usage-refresh evaluation hook, and the Usage-alerts UI.
Native delivery is wired via `tauri-plugin-notification` (`TauriNotifier`). The two unchecked
criteria assert the **rendered** notification (OS-centre delivery / DND / on-screen naming),
verifiable only by live `make qa` on macOS — pending, along with the cross-platform CI lanes.

## References
- [ADR 0009 — alarm engine](../adr/0009-alarm-engine.md)
