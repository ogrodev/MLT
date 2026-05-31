# 023 — Opt-in telemetry / crash reporting

> Any diagnostics are off by default, clearly explained, and only on if I choose.

**Capability:** [§9 Privacy & security](../PRD.md#9-privacy--security) · **Status:** ◻ not started · **Depends on:** —

## User story
As a user, I want diagnostic/crash reporting to be off unless I turn it on, and to know
exactly what would be sent, so nothing leaves my device without my say-so.

## Scope
- **In:** A telemetry/crash-reporting toggle that is **off by default**, with a plain
  description of what would be sent and where.
- **Out:** Product privacy statement (022). (Socket Firewall's build-time telemetry is a dev
  tool, not the shipped app — out of scope.)

## Acceptance criteria
- [ ] Diagnostic/telemetry reporting is **off by default**.
- [ ] The setting **plainly states what would be sent** (a data map) and to where, before I opt in.
- [ ] With it **off**, **nothing** diagnostic leaves the device.
- [ ] Turning it on/off takes effect **without restart** and the choice **persists**.

## Done
Meets the [shared Definition of Done](./README.md#shared-definition-of-done-applies-to-every-task).
QA: with telemetry off, confirm (e.g. via traffic inspection) no diagnostic calls are made.

## References
- [OPEN_QUESTIONS O4, Q10](../OPEN_QUESTIONS.md)
