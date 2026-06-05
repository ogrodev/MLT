# 0021 — Missed-alarm catch-up policy (fire-each vs coalesce)

**Status:** Accepted · **Date:** 2026-06-04 · **Resolves:** [OPEN_QUESTIONS Q6](../OPEN_QUESTIONS.md)

## Context
[ADR 0009](./0009-alarm-engine.md) makes alarms survive sleep/quit via a `reconcile(now, alarms)`
catch-up pass on launch/wake. When several alarms (or a recurring alarm's several occurrences)
came due during downtime, firing one OS notification per occurrence is a stale-ping flood; firing
nothing silently drops reminders (task 013 forbids both). We must define the catch-up behaviour
and make it a user choice.

## Decision
**A per-user `MissedPolicy { FireEach | Coalesce }` setting (default `FireEach`),** applied by the
pure `reconcile`:
- A due alarm (`next_fire_at <= now`) yields **one** `DueAlarm` regardless of how many occurrences
  elapsed during downtime — a recurring alarm that missed many occurrences is **collapsed to a
  single** catch-up entry (this is itself the per-alarm anti-flood guarantee).
- `FireEach` → one notification per due alarm; `Coalesce` → a single summary notification
  ("N reminders came due while MLT was away") **when more than one** alarm is due (a lone due
  alarm always fires individually under either policy).
- Recurring alarms then resume on their **correct next** occurrence (`next_occurrence` strictly
  past `now`), and an alarm's schedule is always advanced past `now`, so a subsequent reconcile at
  the same instant **never double-fires**.

## Alternatives considered
- **Always fire each occurrence** — faithful but floods after a long sleep (the exact failure
  ADR 0009 calls out).
- **Always coalesce** — never floods, but buries the identity of individual reminders.
- **Per-alarm policy** — finer control, more UI/state than v1 warrants; a global setting is enough.

## Consequences
- **+** Bounded notifications (≤ number of distinct due alarms), user-controllable, no silent
  drops. Pure and deterministic — unit-tested with a fake `now` simulating a downtime gap.
- **−** Collapsing a recurring alarm's missed occurrences to one ping means the user isn't told
  *how many* times it elapsed; acceptable (the next occurrence is scheduled correctly).
- The policy persists in `AlarmSettings.missed_policy` and is editable from the Alarms screen.
