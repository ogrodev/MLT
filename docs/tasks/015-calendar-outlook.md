# 015 — Outlook Calendar — read-only upcoming events

> Connect my Outlook/Microsoft calendar and see my next events alongside usage.

**Capability:** [§6 Calendar awareness](../PRD.md#6-calendar-awareness-read-only) · **Status:** ◻ not started · **Depends on:** —

## User story
As a user, I want my upcoming Outlook calendar events in view, so my day's context is there
too — read-only, with the narrowest access.

## Scope
- **In:** Connecting Outlook/Microsoft calendar read-only and showing today's/next events.
- **Out:** Google (014), device calendars (016), revoke/manage (017), any write access.

## Acceptance criteria
- [ ] I can **connect** a Microsoft account using the **narrowest read-only** calendar permission.
- [ ] The popover shows my **upcoming events** (today / next), read-only.
- [ ] The app **never creates, edits, or deletes** events.
- [ ] If the connection fails or returns nothing, the rest of the app is **unaffected** and the
      calendar area shows a clear empty/error state.
- [ ] Outlook events render the **same way** as Google's and stay **siloed** from usage tiles.

## Done
Meets the [shared Definition of Done](./README.md#shared-definition-of-done-applies-to-every-task).
Tests use fixture event payloads; a live OAuth check is a hand-run example.

## References
- [OPEN_QUESTIONS Q1, D3](../OPEN_QUESTIONS.md)
