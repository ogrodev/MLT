# 0004 — Scope: usage + alarms + read-only calendar

**Status:** Accepted · **Date:** 2026-05-30

## Context
The brief added "alarms" and "access to user calendar" on top of the CodexBar concept.
CodexBar's only "calendar" is reset-window countdowns and its only "alarms" are quota
notifications. We had to decide the real v1 surface.

## Decision
**Full scope, three subsystems:**
1. **Usage tracking** — multi-provider, reset windows + countdowns (the CodexBar core).
2. **Alarms** — user-defined one-off/recurring alarms firing OS notifications, plus
   usage-derived alarms (threshold/reset).
3. **Calendar** — **read-only** sync of the user's Google/Outlook (web) and macOS
   (EventKit) calendars for time context.

## Alternatives considered
- **CodexBar clone + reset alarms only** — tightest scope, fastest, but drops the user's
  explicit alarm + calendar asks.
- **Usage + general alarms, no calendar** — one fewer subsystem, but the brief explicitly
  wanted calendar access.

## Consequences
- **+** Matches the full brief; each subsystem is independently bounded behind ports.
- **−** Largest surface of the options; calendar adds OAuth-per-provider + a macOS-only
  native path (ADR 0008) and the most consent screens.
- Calendar **write** is explicitly deferred (OPEN_QUESTIONS Q1).
