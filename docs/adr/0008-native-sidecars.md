# 0008 — Native code via sidecar helper processes

**Status:** Accepted · **Date:** 2026-05-30

## Context
Several needs are inherently per-OS and native: macOS EventKit (calendar), browser-cookie
decryption (Safari binary format; Chromium AES key in the OS keychain; DPAPI on Windows),
and keychain access. We must settle *once* how native code integrates, or it scatters as
ad-hoc FFI and erodes portability + testability.

## Decision
**Sidecar helper processes.** Small native binaries — **Swift** on macOS, **.NET** on
Windows — that the Rust core spawns and talks to over **JSON-RPC on stdio**. Native code
is isolated, crash-contained, and independently testable; the Rust side stays pure behind
ports (`CalendarPort`, `CookieSource`). **Exception:** secrets use the cross-platform
`keyring` crate in-process behind `SecretStore` (it's already portable; a sidecar would be
overkill).

## Alternatives considered
- **In-process Rust FFI** (`objc2`/`windows-rs`/crypto crates) — single binary, lowest
  latency, but native crashes are fatal, Apple/Win SDKs enter the Rust build, and tests
  need the real OS.
- **Mix: FFI for simple, sidecar for heavy** — pragmatic but two patterns to document; we
  collapse to "sidecar by default, `keyring` as the one in-process exception."

## Consequences
- **+** Native crashes can't take down the core; clean JSON-RPC contract is fixture-testable.
- **+** Rust core stays portable and pure.
- **−** Must bundle + sign + path-resolve extra binaries per OS (OPEN_QUESTIONS Q9); small
  IPC overhead and spawn cost.
