# 0009 — Persisted alarms + wake/launch catch-up

**Status:** Accepted · **Date:** 2026-05-30

## Context
Reliability is a stated goal, and naive in-process timers are where alarms quietly fail: a
`tokio` timer does not fire while the laptop is asleep or the app is quit. We must define
what happens to an alarm that was due during sleep/quit.

## Decision
**Persisted schedule + catch-up reconciler.** Alarms persist in SQLite with `next_fire_at`.
While running, an in-process scheduler fires on time. On **app launch** and on
**system-wake** events, a `reconcile(now, alarms)` pass scans for due/missed alarms and
fires (or coalesces) them, then recomputes next fire. `now` is passed in via the injected
**`Clock`**, so the whole engine is unit-tested without real waiting.

## Alternatives considered
- **Delegate to OS scheduler** (launchd / Task Scheduler / systemd-timer) — most reliable
  for exact-time alarms even when quit, but per-OS registration code, harder to test, needs
  a headless helper to fire while quit.
- **Hybrid (in-process + OS backstop for critical alarms)** — best coverage, but both code
  paths and their test burden.

## Consequences
- **+** Survives sleep/quit/reboot by catching up; pure, fast, testable logic.
- **+** Same pipeline serves user alarms and usage-derived (threshold/reset) alarms.
- **−** Won't fire at the exact instant if the app is fully quit; catches up on next
  launch. Mitigated by the login-item (ADR 0002).
- Missed-alarm coalescing policy is a user setting (OPEN_QUESTIONS Q6).
