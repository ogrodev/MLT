# 022 — In-app privacy & data statement

> A clear, in-app statement of exactly what the app accesses and stores.

**Capability:** [§9 Privacy & security](../PRD.md#9-privacy--security) · **Status:** ◻ not started · **Depends on:** —

## User story
As a user, I want a plain, accessible statement of what the app reads, stores, and sends, so I
can trust that my keys and data stay mine.

## Scope
- **In:** An in-app, accessible statement covering data access/storage and the local-first
  stance, reachable from the popover/settings.
- **Out:** The telemetry toggle itself (023); the per-source consent notes (002).

## Acceptance criteria
- [ ] The app has a **clear, accessible** statement of **what it accesses and stores**,
      reachable from the UI.
- [ ] It states the app is **local-first**: no account required, and data leaves the device
      **only** as direct calls to the providers/calendars I connected.
- [ ] It states that **secrets live only in the OS keychain** and never in logs or the DB.
- [ ] The statement is **accurate** to the app's actual behaviour at release.

## Done
Meets the [shared Definition of Done](./README.md#shared-definition-of-done-applies-to-every-task).
Reviewer confirms the statement matches what the code actually does.

## References
- [ADR 0002 — local-only topology](../adr/0002-local-only-topology.md)
