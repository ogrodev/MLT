# 001 — Popover experience & appearance QA (macOS)

> The popover that already exists is verified to behave and look right as a real menu-bar app.

**Capability:** [§1 Menu-bar app & popover experience](../PRD.md#1-menu-bar-app--popover-experience) · **Status:** ✅ done · **Depends on:** —

## User story
As a user, I want my usage one click away in the menu bar, opening instantly and looking
correct in any appearance, so I can check it without breaking flow.

## Scope
- **In:** Verify and finish the macOS popover so all of §1 holds in a real installed build.
- **Out:** Windows/Linux behaviour (020, 021); new usage content (already shipped under §2).

## Acceptance criteria
- [x] A tray/menu-bar icon is always present while the app runs, with **no Dock entry**.
- [x] Clicking the icon opens a popover **anchored to the icon**; clicking outside or the
      icon again dismisses it.
- [x] The popover opens in **under 1 second** and shows current data with no manual refresh.
- [x] There is a clear way to **quit**, and a visible indication of **connected state**.
- [x] Looks correct in both **light and dark** menu-bar appearance (no clipped, low-contrast,
      or misaligned elements).

## Done
Meets the [shared Definition of Done](./README.md#shared-definition-of-done-applies-to-every-task).
QA note: verify by hand in both appearances and after a fresh `make qa` install.

## References
- [ADR 0013 — single tray icon](../adr/0013-single-tray-icon.md)
