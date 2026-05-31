# MLT — Product Requirements

**Working title:** MLT · **Owner:** pedro@bigshotpictures.com · **Status:** living document

> A menu-bar/tray companion that tells me, at a glance, how much of my AI tooling I've used,
> warns me before I run out, and keeps that in the context of my day — without opening a
> window, leaving my machine, or leaking my keys.

This is the product source of truth: *what* we're building and *how we'll know it's done*
(acceptance criteria). It deliberately contains no implementation detail — for the "how",
see [`ARCHITECTURE.md`](./ARCHITECTURE.md), the [ADRs](./adr/), and [`PROVIDERS.md`](./research/PROVIDERS.md).

Legend: ✅ done · 🟡 partial · ◻ not started.

---

## Delivery checklist — what's left vs the original plan

Each item links to its detailed requirements + acceptance criteria below.

| Status | Capability | Detail |
|--------|-----------|--------|
| 🟡 | [Menu-bar app & popover experience](#1-menu-bar-app--popover-experience) | macOS popover built; not visually QA'd; cross-OS unverified |
| ✅ | [At-a-glance usage tracking](#2-at-a-glance-usage-tracking) | windows, %, countdowns, auto-refresh (for connected providers) |
| 🟡 | [Provider coverage](#3-provider-coverage) | Claude Code ✅; Codex / OpenRouter / OpenAI API / Anthropic API ◻ |
| 🟡 | [Connecting accounts: credentials & consent](#4-connecting-accounts-credentials--consent) | Claude reuse + refresh ✅; in-app connect/disconnect, API-key entry, consent UI ◻ |
| ◻ | [Alarms & notifications](#5-alarms--notifications) | threshold + user-defined alarms → OS notifications |
| ◻ | [Calendar awareness (read-only)](#6-calendar-awareness-read-only) | Google / Outlook / Apple calendars |
| 🟡 | [Reliability & always-on](#7-reliability--always-on) | resilience + refresh loop ✅; login-item + wake catch-up ◻ |
| 🟡 | [Cross-platform support](#8-cross-platform-support) | compiles for Linux in CI; only run/verified on macOS |
| 🟡 | [Privacy & security](#9-privacy--security) | local-only + keychain ✅; consent surfacing + telemetry choice ◻ |
| ◻ | [Distribution & updates](#10-distribution--updates) | signing, notarization, installers, auto-update |
| 🟡 | [Quality, CI & release readiness](#11-quality-ci--release-readiness) | **full CI suite green ✅**; signed-release pipeline ◻ |
| ✅ | [Manual QA & installability](#12-manual-qa--installability) | one-command build + install on macOS (`make qa`) |

**Hard gate for any release:** item 11's **full CI check must be green** (see its acceptance criteria).

---

## 1. Menu-bar app & popover experience

The product lives in the menu bar/tray. Clicking the icon reveals usage in place — no app
window, no Dock/taskbar entry, dismissed by clicking away.

*As a user, I want my AI usage one click away in the menu bar so I can check it without
breaking flow.*

**Acceptance criteria**
- A tray/menu-bar icon is always present while the app runs; the app shows **no Dock/taskbar entry**.
- Clicking the icon opens a popover **anchored to the icon**; clicking outside or the icon again dismisses it.
- The popover opens in **under 1 second** and shows current data without a manual refresh.
- The app has a way to **quit** and an indication of its **connected state**.
- Behaves correctly with the menu bar in light and dark appearance.

## 2. At-a-glance usage tracking

For each connected provider, show the usage windows that matter (e.g. session / weekly /
monthly) with how much is used and when each resets.

*As a user, I want to see how close I am to my limits and when they reset so I can pace my work.*

**Acceptance criteria**
- Each provider shows its windows with a **percent used** and a **human countdown to reset** (e.g. "resets in 4h", "resets in 3d").
- Values **auto-refresh at least every 60 seconds** while the app runs, and on opening the popover.
- Usage at/over a high threshold is **visually distinct** (e.g. colour change) from low usage.
- A **last-updated** time is visible.
- When a provider's data can't be fetched, the UI shows a clear **stale/error** state for *that* provider and keeps showing the last known values — it never blanks out or crashes.

## 3. Provider coverage

v1 targets the AI coding tools the owner actually uses; more follow on a roadmap.

*As a user, I want my main AI tools covered so the number reflects my real usage.*

**v1 scope:** Claude Code (subscription), Codex (subscription), OpenRouter (API), OpenAI API, Anthropic API.
**Roadmap:** GitHub Copilot, Cursor, Gemini (see [PROVIDERS.md](./research/PROVIDERS.md)).

**Acceptance criteria**
- Each v1 provider, once connected, displays normalized usage windows per §2.
- Where a provider **cannot** expose usage with the credentials a typical solo user has
  (e.g. Anthropic/OpenAI API cost needs an org admin key), the UI states this **honestly**
  rather than showing zero or a misleading value.
- Adding a new provider does not change the behaviour or appearance of existing ones.

## 4. Connecting accounts: credentials & consent

Users connect providers by reusing existing logins (vendor CLIs/browsers) or entering an API key.

*As a user, I want to connect my accounts safely and understand exactly what's accessed.*

**Acceptance criteria**
- The app can **discover** locally available sources (installed CLIs, etc.) and present them; **discovery reads only metadata** (presence), never secrets, until I opt in.
- I can **enable/disable each source individually**, with a plain-language note of what's accessed and why, before any secret is read.
- I can **enter/replace/remove an API key** for providers that need one, and **disconnect** any provider.
- Secrets are stored only in the OS keychain — **never** shown again in full, never written to logs or the local database.
- Connecting or disconnecting a provider takes effect without restarting the app.

## 5. Alarms & notifications

Two kinds: automatic alerts tied to usage, and user-defined alarms.

*As a user, I want to be warned before I hit a limit, and to set my own reminders, delivered as native notifications.*

**Acceptance criteria**
- I can enable **threshold alerts** per provider/window (e.g. notify at 80% and 95%); each fires **once per crossing** and re-arms after the window resets.
- I receive a notification when a tracked window **resets** (optional, per my setting).
- I can create **one-off and recurring** user alarms with a label; they fire native OS notifications at the set time.
- Notifications are delivered via the OS notification centre and respect OS Do-Not-Disturb.
- If an alarm was due while the app was asleep/closed, it is **caught up** on next launch/wake (fired or coalesced per my setting), not silently dropped.

## 6. Calendar awareness (read-only)

Show the user's calendar context alongside usage; **read-only** for v1.

*As a user, I want my upcoming events in view so usage sits in the context of my day.*

**Acceptance criteria**
- I can connect **Google and Outlook** calendars (and, on macOS, calendars already configured on the device).
- The app shows my **upcoming events** (today/next) read-only; it **never creates, edits, or deletes** events in v1.
- Calendar access uses the **narrowest read-only permission**, and I can **revoke** it from within the app.
- If calendar access is denied or unavailable, the rest of the app works unaffected.

## 7. Reliability & always-on

The app should be quietly dependable.

*As a user, I want it to "just keep working" — surviving sleep, flaky network, and odd provider responses.*

**Acceptance criteria**
- The app can **start automatically at login** (toggleable) so tracking is current without me launching it.
- A single failed/slow/garbled provider response **never** stalls the popover or loses other providers' data; the affected provider shows stale/error and recovers automatically.
- Transient network errors (e.g. just after wake) are **retried**, not surfaced as a hard failure.
- Rate-limited providers are backed off so the app doesn't make the limit worse.

## 8. Cross-platform support

macOS is primary; the architecture is portable.

*As a user on macOS/Windows/Linux, I want the same core experience.*

**Acceptance criteria**
- **macOS:** fully supported (primary target).
- **Windows & Linux:** the app builds and the core experience (tray, popover, usage, notifications) works; platform-specific gaps are documented.
- Connecting accounts and reading secrets works using each OS's native credential store.

## 9. Privacy & security

Trust is a feature.

*As a user, I want confidence that my keys and data stay mine.*

**Acceptance criteria**
- The app is **local-first**: no account required, and usage/credential data does not leave the device except the direct calls to the providers I connected.
- Secrets live only in the OS keychain (per §4).
- Any diagnostic/telemetry reporting is **off by default and opt-in**, and the app states plainly what would be sent. *(Note: the dependency-scanning tool used in development, Socket Firewall, has its own telemetry — that's a build-time tool, not the shipped app.)*
- There is a clear, accessible statement of what the app accesses and stores.

## 10. Distribution & updates

How users get and keep the app current.

*As a user, I want a normal install and seamless updates.*

**Acceptance criteria**
- macOS builds are **signed and notarized** so they install without Gatekeeper warnings.
- Windows builds are **signed** so they install without SmartScreen warnings.
- The app can **check for and apply updates** (or clearly prompt to), without a manual reinstall.
- A documented install path exists per platform.

> Depends on human prerequisites (Apple Developer account, Windows cert, etc.) — see
> [`human_prerequisites.md`](../human_prerequisites.md).

## 11. Quality, CI & release readiness

Engineering quality is a product requirement (the owner asked for "no smelly code / AI slop").

**Acceptance criteria**
- **A full CI check passes on every pull request and on `main`**, and is **required** before merge. The full check is: Rust format, Clippy with warnings denied, the architecture-fitness (core-purity) check, the test suite, dependency audit (cargo-deny), malware firewall on dependency fetch (Socket Firewall), unused-dependency check, frontend lint/format (Biome), Svelte+TypeScript type-check, secret scan (gitleaks), and **line coverage ≥ 80% on the core logic**.
- CI runs in a way that proves the codebase still **builds cross-platform** (not only on the developer's machine).
- Local developers can reproduce the full check with a **single command** before pushing, and git hooks enforce the fast subset on commit.
- A **signed, release-grade build pipeline** exists for shipping (ties to §10). *(Status: the full CI check is ✅ green today; the signed-release pipeline is ◻.)*

## 12. Manual QA & installability

The owner must be able to test the app hands-on, as a user, on demand.

*As the owner, after any task I want to ask for a ready-to-use build installed on my Mac so I can QA it like a real user.*

**Acceptance criteria**
- A **single command** (or a request to the agent) builds the app and **installs + launches** it on macOS as a real menu-bar app — not a dev server.
- The installed build **runs without a Gatekeeper prompt** for local testing.
- The command reports **where it was installed** and **how to use it** (menu-bar icon, no Dock).
- A faster **debug** build is the default for iteration; a **release-like** build is available on request.
- Re-running it cleanly **replaces** the previous install and relaunches.

---

### How this maps to the engineering docs
Product (this doc) → architecture & decisions: [`ARCHITECTURE.md`](./ARCHITECTURE.md),
[`adr/`](./adr/) · provider reality: [`research/PROVIDERS.md`](./research/PROVIDERS.md) ·
quality gates: [`QUALITY_GATES.md`](./QUALITY_GATES.md) · open product questions:
[`OPEN_QUESTIONS.md`](./OPEN_QUESTIONS.md) · human-only setup: [`../human_prerequisites.md`](../human_prerequisites.md).
