# 010 — Window-reset notification (opt-in)

> Optionally get pinged when a usage window resets, so I know my budget is fresh.

**Capability:** [§5 Alarms & notifications](../PRD.md#5-alarms--notifications) · **Status:** ◻ not started · **Depends on:** 009

## User story
As a user, I want an optional notification when a tracked window resets, so I know when my
quota is back without checking manually.

## Scope
- **In:** A per-setting notification fired when a tracked window resets.
- **Out:** Threshold alerts (009); user-defined alarms (011–012).

## Acceptance criteria
- [ ] I can turn **window-reset notifications on or off** (off is a valid default per my setting).
- [ ] When enabled, a notification fires when a tracked window **resets**, naming the
      **provider and window**.
- [ ] When disabled, **no** reset notification fires.
- [ ] Delivered via the OS notification centre and respects **Do-Not-Disturb**.

## Done
Meets the [shared Definition of Done](./README.md#shared-definition-of-done-applies-to-every-task).
Tests use a fake clock to fire a reset deterministically.

## References
- [ADR 0009 — alarm engine](../adr/0009-alarm-engine.md)
