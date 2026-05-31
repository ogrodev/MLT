# 013 — Missed-alarm catch-up on launch/wake

> Alarms due while the app was asleep or closed aren't silently lost.

**Capability:** [§5 Alarms & notifications](../PRD.md#5-alarms--notifications) · **Status:** ◻ not started · **Depends on:** 011, 012

## User story
As a user, I want alarms that came due while my Mac was asleep or the app was closed to be
caught up when it next runs, so I don't miss reminders — but without a flood of stale pings.

## Scope
- **In:** Detecting alarms that were due during downtime and handling them on next launch/wake,
  per a user preference.
- **Out:** The wake-driven *usage* refresh (019); creating alarms (011, 012).

## Acceptance criteria
- [ ] On next launch/wake, alarms that became due during downtime are **caught up** — **not
      silently dropped**.
- [ ] I can choose between **fire each** missed alarm or **coalesce** them into one summary.
- [ ] Recurring alarms resume on their correct **next** occurrence after catch-up.
- [ ] Catch-up never **double-fires** an alarm that already fired.

## Done
Meets the [shared Definition of Done](./README.md#shared-definition-of-done-applies-to-every-task).
Tests simulate a downtime gap with a fake clock and assert fire-each vs coalesce behaviour.

## References
- [ADR 0009 — alarm engine](../adr/0009-alarm-engine.md) · [OPEN_QUESTIONS Q6](../OPEN_QUESTIONS.md)
