# 016 — macOS device calendars — read-only

> Use the calendars already set up on my Mac, without connecting anything new.

**Capability:** [§6 Calendar awareness](../PRD.md#6-calendar-awareness-read-only) · **Status:** ◻ not started · **Depends on:** —

## User story
As a macOS user, I want the app to show events from the calendars already configured on my
device, so I don't have to re-authorize accounts I've already added to the system.

## Scope
- **In:** Reading the macOS device calendars read-only (via the system permission) and showing
  upcoming events.
- **Out:** Google (014), Outlook (015), revoke/manage (017), any write access.

## Acceptance criteria
- [ ] The app requests **read-only** access to macOS calendars through the **system permission
      prompt**, and shows **upcoming events** (today / next) once granted.
- [ ] The app **never creates, edits, or deletes** events.
- [ ] If the user **denies** the system prompt, the rest of the app works **unaffected** and the
      calendar area explains access is off.
- [ ] Device-calendar events render the **same way** as connected calendars and stay **siloed**.

## Done
Meets the [shared Definition of Done](./README.md#shared-definition-of-done-applies-to-every-task).
QA on macOS includes both the granted and denied permission paths.

## References
- [OPEN_QUESTIONS Q9](../OPEN_QUESTIONS.md) (sidecar/entitlements) · [ADR 0008 — native sidecars](../adr/0008-native-sidecars.md)
