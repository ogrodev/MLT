# 021 — Linux runtime parity + documented gaps

> The core experience runs on Linux, with honest notes where the desktop can't deliver it.

**Capability:** [§8 Cross-platform support](../PRD.md#8-cross-platform-support) · **Status:** 🟡 partial · **Depends on:** —

## User story
As a Linux user, I want the same core experience — tray, popover, usage, notifications — and a
clear statement of any desktop-environment limitations.

## Scope
- **In:** Running and verifying tray + popover + usage + notifications on a supported Linux
  desktop, using the native secret store (libsecret). Define and document the supported set.
- **Out:** Packaging/distribution; Windows (020).

## Acceptance criteria
- [ ] On a **supported Linux desktop**, a **tray icon** is present, the **popover** works, and
      **usage** displays per §2.
- [ ] **Notifications** deliver through the Linux notification system.
- [ ] Secrets are stored using the **native secret store** — never the DB or logs.
- [ ] The **supported desktop-environment set** is documented, and tray behaviour on
      unsupported environments **degrades gracefully** with a documented note.

## Done
Meets the [shared Definition of Done](./README.md#shared-definition-of-done-applies-to-every-task),
including the **ubuntu-latest CI lane**, plus a hands-on smoke test on a supported desktop.

## References
- [OPEN_QUESTIONS Q7](../OPEN_QUESTIONS.md) (Linux tray reality) · [QUALITY_GATES §6](../QUALITY_GATES.md)
