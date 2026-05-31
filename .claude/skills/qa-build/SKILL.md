---
description: Build, install, and launch the MLT app on this Mac for manual QA — the real menu-bar app (icon + popover), not `tauri dev`. Use when the user wants to test the app as a real user, "install it", "prepare for QA", or "let me try it".
---

# QA build & install

Get a real, installed build of MLT running on the user's Mac so they can QA it as a user would.

## Steps
1. Run `make qa` (fast **debug** bundle) — unless the user asks for a production-like build, then `make qa-release`. ($ARGUMENTS may be `release`.)
2. If the build fails, show the failing output and stop — do not pretend it installed.
3. On success, tell the user:
   - The **MLT icon is in the macOS menu bar** (top-right) — click it to open the popover.
   - There is **no Dock icon** by design.
   - A Keychain **"Always Allow"** prompt may appear once (Claude credentials).
   - It was installed to the path printed by the script (`/Applications/mlt.app` or `~/Applications`).
4. Produce a short **"What to test"** list tailored to what changed since the last QA build
   (use recent commits/the current task) so the user knows what to exercise.

## Notes
- The build is **unsigned** (local only); the script clears the quarantine attribute so it launches without a Gatekeeper warning.
- This launches a GUI app on the user's machine — only run it when the user has asked to QA/test/install.
