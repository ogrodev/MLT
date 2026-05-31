# 017 — Manage & revoke calendar access

> Turn any connected calendar off from inside the app, and trust that access stops.

**Capability:** [§6 Calendar awareness](../PRD.md#6-calendar-awareness-read-only) · **Status:** ◻ not started · **Depends on:** 014–016

## User story
As a user, I want to see which calendars are connected and revoke any of them from within the
app, so I stay in control of what's accessed.

## Scope
- **In:** A management view listing connected calendar sources with a **revoke** action and the
  cleanup it triggers.
- **Out:** The initial connect flows (014–016).

## Acceptance criteria
- [ ] The app lists **connected calendar sources** (Google / Outlook / device).
- [ ] I can **revoke** any calendar source from within the app; its events disappear and it
      stops being read.
- [ ] After revoke, any stored calendar token/grant is **removed** and the source can be
      **reconnected** later.
- [ ] Revoking one calendar leaves the others — and the rest of the app — **unaffected**.

## Done
Meets the [shared Definition of Done](./README.md#shared-definition-of-done-applies-to-every-task).
QA: confirm no further calendar fetches occur after revoke.

## References
- [OPEN_QUESTIONS Q1](../OPEN_QUESTIONS.md)
