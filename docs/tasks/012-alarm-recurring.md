# 012 — Recurring user alarm (RRULE-lite)

> Set a repeating reminder (daily / weekly / every-N) and have it keep firing on schedule.

**Capability:** [§5 Alarms & notifications](../PRD.md#5-alarms--notifications) · **Status:** 🟡 partial · **Depends on:** 011

## User story
As a user, I want recurring alarms — daily, weekly, or every N days — so I don't recreate the
same reminder each time.

## Scope
- **In:** Adding recurrence to user alarms with a lean grammar (daily / weekly / interval) and
  firing each occurrence.
- **Out:** Full RRULE/calendar-grammar; missed-alarm catch-up (013).

## Acceptance criteria
- [x] I can create a **recurring** alarm with a label and a **daily / weekly / every-N-days**
      schedule.
- [ ] Each occurrence fires a **native notification** at the right time and the alarm
      **re-schedules** to its next occurrence.
- [x] I can **edit or delete** a recurring alarm, which stops future occurrences.
- [ ] Delivered via the OS notification centre and respects **Do-Not-Disturb**.

## Done
Meets the [shared Definition of Done](./README.md#shared-definition-of-done-applies-to-every-task).
Tests use a fake clock to fire several occurrences in sequence.

**Status note (2026-06-04):** Implemented — `Recurrence` (daily / weekly / every-N-days, [ADR
0020](../adr/0020-rrule-lite-recurrence.md)), `next_occurrence` + `reconcile` re-scheduling (core
unit-tested across several occurrences in sequence), the `create_alarm`/`update_alarm`/
`delete_alarm` commands, and the recurring create/edit/delete UI. The unchecked criteria assert the
**rendered** native notification (delivery / DND); the re-scheduling itself is unit-tested. Pending
live `make qa` on macOS and the cross-platform CI lanes.

## References
- [ADR 0009 — alarm engine](../adr/0009-alarm-engine.md) · [OPEN_QUESTIONS Q5](../OPEN_QUESTIONS.md)
