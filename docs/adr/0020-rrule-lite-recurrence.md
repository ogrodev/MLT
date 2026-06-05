# 0020 — RRULE-lite recurrence for user alarms

**Status:** Accepted · **Date:** 2026-06-04 · **Resolves:** [OPEN_QUESTIONS Q5](../OPEN_QUESTIONS.md)

## Context
User alarms (tasks 011/012) need a recurrence grammar. Full iCalendar RRULE is large and
fiddly (BY* rules, COUNT/UNTIL, timezone/DST edge cases), and the core is pure with **no
timezone library** — time only enters as an injected `Clock` producing epoch-millisecond
`Timestamp`s. We must decide how expressive recurrence is for v1 and how occurrences advance.

## Decision
**RRULE-lite: `Daily | Weekly | EveryNDays { days }`** (`crates/core/src/alarms.rs::Recurrence`),
with occurrences advanced by **whole-day UTC arithmetic on epoch-ms** (`Daily = 1 day`,
`Weekly = 7 days`, `EveryNDays = n days`; `days` clamped to `>= 1`). `next_occurrence(anchor,
after, recurrence)` returns the first `anchor + k·period` strictly greater than `after`,
computed in O(1) by division so a long downtime gap never loops. A one-off alarm is simply
`recurrence: None`.

## Alternatives considered
- **Full RRULE** — maximally expressive, but heavy to implement/test, pulls a parser, and
  would force timezone handling into the (deliberately pure, tz-free) core.
- **Cron expressions** — compact but unfriendly to non-technical users and still needs tz logic
  for "9am local every day".
- **Weekly-with-weekday-set** (e.g. Mon/Wed/Fri) — more power, but beyond what the tasks ask
  ("daily / weekly / every-N"); deferred.

## Consequences
- **+** Tiny, pure, exhaustively unit-tested surface; each occurrence is deterministic from the
  anchor + `Clock`. Reschedule and downtime catch-up share one `next_occurrence`.
- **+** No timezone dependency in core; the engine stays IO-free and millisecond-deterministic.
- **−** Day arithmetic is **UTC-relative**: across a DST transition a "daily 9am" alarm drifts by
  the offset until re-anchored. Acceptable for a v1 reminder feature; a future ADR can add
  local-time/DST-aware or weekday-set recurrence if needed (it would supersede this one).
- The grammar serialises as a tagged enum (`{ "kind": "daily" | "weekly" | "every_n_days", … }`)
  shared verbatim with the frontend.
