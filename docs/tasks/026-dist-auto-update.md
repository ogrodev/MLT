# 026 — Auto-update (check + apply)

> The app keeps itself current without a manual reinstall.

**Capability:** [§10 Distribution & updates](../PRD.md#10-distribution--updates) · **Status:** ◻ not started · **Depends on:** 024

## User story
As a user, I want the app to update itself (or clearly prompt me) so I'm always on the latest
version without hunting for a download.

## Scope
- **In:** Checking for updates and applying them (or clearly prompting), from a signed update
  source.
- **Out:** Producing the signed artifacts (024, 025); the release pipeline (027).

## Acceptance criteria
- [ ] The app **checks for updates** and, when one exists, **applies it** (or clearly prompts to).
- [ ] Updating does **not** require a manual reinstall.
- [ ] Updates are accepted **only from a signed/verified source** (a tampered update is rejected).
- [ ] The user can see the **current version** and that they are up to date.

## Done
Meets the [shared Definition of Done](./README.md#shared-definition-of-done-applies-to-every-task).
QA: install an older build and confirm it updates to a newer signed build.

## References
- [OPEN_QUESTIONS O3](../OPEN_QUESTIONS.md)
