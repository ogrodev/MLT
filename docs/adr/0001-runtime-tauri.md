# 0001 — Runtime: Tauri (Rust core + system webview)

**Status:** Accepted · **Date:** 2026-05-30

## Context
We need a cross-platform (macOS, Windows, Linux) menu-bar/tray app with a chromeless
popover, OS notifications, calendar access, OAuth, and secure secret storage. The root
decision is the runtime, because it dictates language, packaging, the popover mechanism,
notification APIs, and the entire testing story. The reference product (CodexBar) is
Swift/macOS-only; we explicitly want portability, so we cannot copy its stack.

## Decision
Build on **Tauri**: a **Rust** core with the UI rendered in the OS's **system webview**.

## Alternatives considered
- **Electron** — easiest hiring, huge ecosystem, but 100–200 MB bundles, heavy RAM, ships
  Chromium. Logic in JS is weaker for the testable, portable core we want.
- **Native per-OS** (SwiftUI + WinUI) — best feel, but two UIs to build/test; directly
  contradicts the portability goal.
- **Flutter desktop** — single Dart codebase, but smaller desktop ecosystem and less
  battle-tested tray/popover support.

## Consequences
- **+** Tiny bundles (~5 MB), low RAM, true cross-platform tray + notifications.
- **+** Rust core is highly testable and portable; UI is any web framework.
- **−** Rust learning curve; webview rendering differs slightly per OS (WebKit/WebView2/WebKitGTK).
- The popover is a borderless anchored window, not literal in-place expansion (see ARCHITECTURE §2).
