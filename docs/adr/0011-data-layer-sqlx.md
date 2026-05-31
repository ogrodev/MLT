# 0011 — Data layer: sqlx (compile-time-checked SQL)

**Status:** Accepted · **Date:** 2026-05-30

## Context
The local store holds usage history, alarms, settings, and cached calendar events. The
tradeoff is compile-time safety (anti-slop) vs simplicity, and async vs sync. AI-written
queries hallucinating columns is exactly the slop we want gated out.

## Decision
**sqlx** with the `query!`/`query_as!` macros: queries are **verified against the real
schema at compile time** — a wrong column or type fails the build, not production. Async,
integrates with tokio. The **`.sqlx` offline cache is committed** so CI builds without a
live DB (`cargo sqlx prepare --check` is a gate).

## Alternatives considered
- **rusqlite + migrations** — simplest, fewest deps, synchronous, but queries are strings
  checked only at runtime; typo'd columns surface as runtime errors.
- **SeaORM** — typed builder, refactor-friendly, but heavier/more magic and hides the SQL.

## Consequences
- **+** Strongest anti-slop guarantee for the data layer; SQL stays explicit and reviewable.
- **−** Async ceremony; must regenerate + commit the offline cache when queries change
  (enforced in CI). Migrations via `sqlx migrate`.
- All DB access stays behind repo ports (`UsageRepo`, `AlarmRepo`, `SettingsRepo`); sqlx is
  an adapter detail, swappable without touching `core`.
