# 014 — Google Calendar — read-only upcoming events

> Connect my Google Calendar and see my next events alongside usage.

**Capability:** [§6 Calendar awareness](../PRD.md#6-calendar-awareness-read-only) · **Status:** ◻ not started · **Depends on:** —

## User story
As a user, I want my upcoming Google Calendar events in view, so my AI usage sits in the
context of my day — read-only, with the narrowest access.

## Scope
- **In:** Connecting Google Calendar read-only and showing today's/next upcoming events.
- **Out:** Outlook (015), device calendars (016), revoke/manage (017), any write access.

## Acceptance criteria
- [ ] I can **connect** a Google account using the **narrowest read-only** calendar permission.
- [ ] The popover shows my **upcoming events** (today / next), read-only.
- [ ] The app **never creates, edits, or deletes** events.
- [ ] If the connection fails or returns nothing, the rest of the app is **unaffected** and the
      calendar area shows a clear empty/error state.
- [ ] Calendar data is **siloed** — never blended into a provider's usage tile.

## Done
Meets the [shared Definition of Done](./README.md#shared-definition-of-done-applies-to-every-task).
Tests use fixture event payloads; a live OAuth check is a hand-run example.

## References
- [OPEN_QUESTIONS Q1, D3](../OPEN_QUESTIONS.md)
