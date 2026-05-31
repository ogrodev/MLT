# 0013 — Single tray icon, CodexBar-style v0.1 experience

**Status:** Accepted · **Date:** 2026-05-30

## Context
CodexBar offers both a per-provider status item and a "merged icon" mode. We had to decide
the tray model and the v0.1 interaction baseline.

## Decision
**One tray icon.** The popover aggregates all enabled providers. We **mirror the CodexBar
experience as v0.1** (a known-good baseline) and evolve our own design from there rather
than designing the interaction from scratch.

## Alternatives considered
- **Per-provider tray icons** — more at-a-glance info, but clutters the menu bar and
  multiplies tray/positioning code across OSes.
- **Both, user-configurable** — most flexible, but more UI + state to build before we even
  know our own design opinion.

## Consequences
- **+** Simpler tray/positioning code; one anchored popover (ADR 0001); cleaner cross-OS story.
- **+** v0.1 has a proven reference UX; we ship, learn, then diverge deliberately.
- **−** Less at-a-glance density than multiple icons; revisit if users want a "pin provider
  to menu bar" option later (would reopen OPEN_QUESTIONS Q2).
