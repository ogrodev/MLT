# 0012 — Consent: metadata-only discovery → per-source opt-in

**Status:** Accepted · **Date:** 2026-05-30

## Context
The app reads the user's *existing* browser cookies and CLI credentials to reuse sessions
(ADR 0003). This is powerful but invasive; how it's consented to is both a UX and a
security-architecture decision with trust/legal weight.

## Decision
**Auto-discover, then user confirms — with a metadata-only scan.** On setup the app
detects which sources are *available* using **metadata only** (does a Chrome profile /
Codex CLI config *exist* by path?), and presents them: "Found: Chrome, Codex CLI, Claude
CLI…". **No cookie decryption or credential read happens until the user toggles that
specific source on**, with a plain-language disclosure of what's accessed and why.
Harvested secrets live in the **OS keychain only — never in the DB or logs**.

## Alternatives considered
- **Explicit per-source opt-in, nothing scanned first** — most defensible, but clunkier
  onboarding (user must know what they have).
- **Auto-use everything available silently** — best "just works" demo, weakest trust story;
  rejected for something touching credentials.

## Consequences
- **+** Smooth onboarding (app shows what it found) while keeping reads behind explicit consent.
- **+** Defensible privacy posture; secrets quarantined in keychain, logs redacted.
- **−** The discovery scan touches filesystem metadata pre-consent — bounded strictly to
  existence/path checks, never content. This boundary is a documented invariant and should
  be covered by a test.
