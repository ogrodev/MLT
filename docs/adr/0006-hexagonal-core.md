# 0006 — Hexagonal core (ports & adapters)

**Status:** Accepted · **Date:** 2026-05-30

## Context
The brief's load-bearing goals are testability and portability, plus resistance to AI
slop. The Rust core's organization decides what can be tested without IO and how contained
OS-specific code stays.

## Decision
**Hexagonal / ports-and-adapters.** A pure **`core`** crate holds domain types + business
logic with **zero IO** — no network, disk, OS, time, or randomness; all injected via
**ports** (traits): `Clock`, `HttpPort`, `SecretStore`, `UsageRepo`, `AlarmRepo`,
`SettingsRepo`, `Notifier`, `CalendarPort`, `CookieSource`, `CliCredSource`, `LoginItem`.
**Adapter** crates implement the ports; the **`app`** (Tauri shell) wires them at startup.

## Alternatives considered
- **Layered (services over repositories)** — familiar, less ceremony, but looser IO
  abstractions; impure code drifts into services over time.
- **Feature-modular (vertical slices)** — great per-feature cohesion, but cross-cutting IO
  (db, secrets) and OS portability get duplicated/inconsistent across slices.

## Consequences
- **+** Core is 100% unit-testable with fakes, in milliseconds, no network/OS.
- **+** OS-specific code is quarantined in adapters/sidecars — porting touches one place.
- **+** Enables a future `RemoteGateway` (ADR 0002) without rewriting logic.
- **−** More upfront discipline; the boundary must be *enforced*, not trusted — done via
  the architecture-fitness gate (QUALITY_GATES §2) that forbids IO crates in `core`.
