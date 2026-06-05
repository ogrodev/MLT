# 011 — One-off user alarm

> Set a labelled reminder for a specific time and get a native notification when it fires.

**Capability:** [§5 Alarms & notifications](../PRD.md#5-alarms--notifications) · **Status:** 🟡 partial · **Depends on:** 009

## User story
As a user, I want to create a one-off alarm with a label at a chosen time, so I get reminded
even though this is a usage tracker — keeping my day in one place.

## Scope
- **In:** Create / view / delete a single one-off alarm and fire it as a native notification.
- **Out:** Recurrence (012); missed-alarm catch-up (013).

## Acceptance criteria
- [x] I can **create** a one-off alarm with a **label** and a **date/time**.
- [ ] The alarm fires a **native OS notification** at the set time, showing its label.
- [x] I can **see** my pending alarms and **delete** one.
- [ ] Delivered via the OS notification centre and respects **Do-Not-Disturb**.

## Done
Meets the [shared Definition of Done](./README.md#shared-definition-of-done-applies-to-every-task).
Tests use a fake clock to fire deterministically.

**Status note (2026-06-04):** Implemented — `create_alarm`/`list_alarms`/`delete_alarm` commands
over the persisted `FileAlarmStore`, the `reconcile`-driven scheduler that fires a one-off at its
time (core unit-tested: fires once then completes), and the "Your alarms" UI (create + list +
delete). The unchecked criteria assert the **rendered** native notification (delivery / DND),
pending live `make qa` on macOS and the cross-platform CI lanes.

## References
- [ADR 0009 — alarm engine](../adr/0009-alarm-engine.md) · [OPEN_QUESTIONS Q5](../OPEN_QUESTIONS.md)
