# 020 — Windows runtime parity

> The core experience actually runs on Windows, not just compiles for it.

**Capability:** [§8 Cross-platform support](../PRD.md#8-cross-platform-support) · **Status:** ◻ not started · **Depends on:** —

## User story
As a Windows user, I want the same core experience — tray, popover, usage, notifications — so
the app is real for me, not macOS-only.

## Scope
- **In:** Running and verifying tray + popover + usage + notifications on Windows, using the
  native Windows credential store for secrets. Document any platform gaps.
- **Out:** Code signing / installer (025); Linux (021).

## Acceptance criteria
- [ ] On Windows, a **tray icon** is present, the **popover** opens/anchors/dismisses, and
      **usage** displays per §2.
- [ ] **Notifications** (threshold + user alarms) deliver through Windows' notification system.
- [ ] Secrets are stored using the **native Windows credential store** — never the DB or logs.
- [ ] Any platform-specific gaps vs macOS are **documented** (not silently missing).

## Done
Meets the [shared Definition of Done](./README.md#shared-definition-of-done-applies-to-every-task),
including the **windows-latest CI lane**, and a hands-on smoke test on a real Windows session.

## References
- [OPEN_QUESTIONS Q8](../OPEN_QUESTIONS.md) (cookie/credential paths) · [QUALITY_GATES §6](../QUALITY_GATES.md)
