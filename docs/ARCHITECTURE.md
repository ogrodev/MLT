# MLT — Architecture

> Working title: **MLT**. A cross-platform menu-bar/tray app that tracks AI-provider
> usage (CodexBar-style), fires user-defined alarms as OS notifications, and surfaces
> read-only calendar context — all local-first, with no backend.

**Status:** Draft v0.1 — foundational architecture, pre-implementation.
**Date:** 2026-05-30.
**Owner:** pedro@bigshotpictures.com.

This document is the single source of truth for *how* MLT is built. Every decision
below has a corresponding ADR in [`docs/adr/`](./adr/). If code disagrees with this
doc, one of them is a bug — fix the doc or the code, never let them drift.

---

## 1. Product in one paragraph

MLT lives in the macOS menu bar and the Windows/Linux system tray. Clicking the icon
opens a chromeless, anchored popover (no visible window chrome, no taskbar entry). It
shows per-provider AI usage — session / weekly / monthly windows with countdowns to the
next reset — across many providers (the CodexBar model). It also lets the user set
alarms that fire OS notifications, and reads (read-only) the user's calendar to provide
time context. It is **local-first**: no server, no account, data stays on the device.

## 2. Reality-checks (things that are commonly misunderstood)

- **There is no true "expand in place."** Tauri renders the popover as a borderless,
  always-on-top, chromeless window positioned flush against the tray icon. It behaves
  like a popover (click-outside dismiss, no title bar, no taskbar/dock entry) but is
  technically a window. This is how every framework does it.
- **Local-only ⇒ usage is only polled while running.** We register an OS login-item so
  the app auto-starts into the tray; combined with the alarm catch-up reconciler, the
  only true blind spot is "machine off / app force-quit," which self-heals on next launch.
- **Credential reuse is inherently per-OS.** Reading Safari/Chrome cookies and CLI creds
  is the one place "write once" leaks. It is quarantined behind ports + native sidecars.
- **Calendar native access (EventKit) forces a macOS CI runner** and a mac-only test lane.

## 3. Decision log

| # | Decision | Choice | ADR |
|---|----------|--------|-----|
| 1 | Runtime / framework | Tauri (Rust core + system webview) | [0001](./adr/0001-runtime-tauri.md) |
| 2 | Topology | Local-only + OS login-item | [0002](./adr/0002-local-only-topology.md) |
| 3 | Usage source | Poll provider endpoints via multi-strategy credential layer; no proxy | [0003](./adr/0003-usage-source-polling.md) |
| 4 | Scope | Usage + user alarms + read-only calendar | [0004](./adr/0004-scope.md) |
| 5 | Provider model | One `Provider` trait + reusable capability blocks | [0005](./adr/0005-provider-trait-blocks.md) |
| 6 | Architecture style | Hexagonal / ports-and-adapters; pure no-IO core | [0006](./adr/0006-hexagonal-core.md) |
| 7 | "Evals" / quality | Deterministic quality gates (see QUALITY_GATES.md) | [0007](./adr/0007-quality-gates.md) |
| 8 | Native bridging | Sidecar helper processes (JSON-RPC/stdio); `keyring` for secrets | [0008](./adr/0008-native-sidecars.md) |
| 9 | Alarm engine | Persisted schedule + wake/launch catch-up; injected `Clock` | [0009](./adr/0009-alarm-engine.md) |
| 10 | UI stack | Svelte + Tailwind; types via `tauri-specta` | [0010](./adr/0010-ui-svelte-tailwind.md) |
| 11 | Data layer | sqlx (compile-time-checked SQL) + committed `.sqlx` cache | [0011](./adr/0011-data-layer-sqlx.md) |
| 12 | Consent model | Auto-discover (metadata-only) → per-source opt-in; secrets in keychain only | [0012](./adr/0012-consent-model.md) |
| 13 | Tray model | Single tray icon; mirror CodexBar UX as v0.1 | [0013](./adr/0013-single-tray-icon.md) |
| 14 | v1 providers | Codex, Claude Code, OpenRouter, OpenAI API, Anthropic API | [0014](./adr/0014-v1-provider-set.md) |
| 15 | Resilience | Lossy decode, backfill, failure-gate, timeout, rate-limit gate — in core | [0015](./adr/0015-resilience-patterns.md) |

> **Provider reality:** see [research/PROVIDERS.md](../research/PROVIDERS.md). Headline: API
> cost for Anthropic/OpenAI needs org **admin keys** most individuals can't get (OpenRouter
> is the easy win), and every subscription usage endpoint is private/undocumented — managed,
> not eliminated, via the resilience patterns (ADR 0015).

Operational defaults (recorded here, see [OPEN_QUESTIONS.md](./OPEN_QUESTIONS.md) to revisit):
single Cargo workspace + pnpm monorepo; GitHub Actions (mac+win+linux matrix);
`tauri-plugin-updater` for auto-update; `tracing` local logs + opt-in crash reporting.

## 4. The hexagonal model

The codebase is divided into a **pure core** (domain types + business logic, zero IO)
and **adapters** (everything that touches the network, disk, OS, clock, or randomness).
The core depends only on **ports** (traits). The app shell wires concrete adapters into
those ports at startup. This is what makes the logic testable in milliseconds with fakes
and keeps OS-specific code in one place.

```
                         ┌───────────────────────────────────────┐
                         │              app (Tauri shell)          │
                         │  wires adapters → ports, exposes        │
                         │  Tauri commands/events to the webview   │
                         └───────────────┬─────────────────────────┘
                                         │ depends on
            ┌────────────────────────────▼────────────────────────────┐
            │                          core (PURE)                      │
            │  domain types: Provider, UsageWindow, Alarm, CalEvent     │
            │  logic: reset math, threshold eval, alarm reconciliation, │
            │         usage normalization, countdown computation         │
            │  PORTS (traits):                                          │
            │    Clock, HttpPort, SecretStore, UsageRepo, AlarmRepo,    │
            │    SettingsRepo, Notifier, CalendarPort, CookieSource,    │
            │    CliCredSource, LoginItem                                │
            └───────▲───────────────────────────────────────▲──────────┘
                    │ implemented by                          │
   ┌────────────────┴───────────┐             ┌───────────────┴───────────────┐
   │     pure / cross-OS         │             │      OS-specific adapters       │
   │     adapters                │             │      (quarantined)              │
   │  HttpAdapter (reqwest)      │             │  KeychainAdapter (keyring)      │
   │  SqliteAdapter (sqlx)       │             │  CookieAdapter → native sidecar │
   │  SystemClock                │             │  EventKitAdapter → swift sidecar│
   │  GoogleCalendar / MsGraph   │             │  LoginItemAdapter (per-OS)      │
   │  TauriNotifier              │             │  NotificationCenter perms (mac) │
   └─────────────────────────────┘             └─────────────────────────────────┘
                                                            │ JSON-RPC / stdio
                                            ┌───────────────▼───────────────┐
                                            │  native sidecars (separate     │
                                            │  binaries, crash-isolated):    │
                                            │   macOS: Swift  (EventKit,     │
                                            │          Safari cookies)       │
                                            │   Windows: .NET (Appointments  │
                                            │          if needed, DPAPI)     │
                                            └────────────────────────────────┘
```

### Workspace layout

We use the **standard Tauri layout** (frontend at the repo root, Rust app in `src-tauri/`)
rather than a bespoke one, wrapped in a Cargo workspace. `✅` = exists today, `◻` = planned.

```
MLT/
├─ Cargo.toml                 # ✅ workspace: members = src-tauri, crates/core, crates/adapters
├─ package.json  vite.config.js  svelte.config.js  tsconfig.json   # ✅ SvelteKit + TS frontend
├─ src/                       # ✅ frontend (routes/, app.html) — Tailwind + bindings/ to come
│  └─ bindings/               # ◻ GENERATED by tauri-specta — do not hand-edit
├─ static/                    # ✅ static assets
├─ crates/
│  ├─ core/                   # ✅ PURE. no tokio/reqwest/fs/SystemTime. domain + ports + providers.
│  │  └─ src/lib.rs           #    modules: domain, ports, providers (+ logic/ as it grows)
│  └─ adapters/               # ✅ concrete port impls (IO lives here). One crate for now;
│     └─ src/lib.rs           #    split per subsystem (http, store, secrets, calendar, …) as it grows.
├─ src-tauri/                 # ✅ the "app" crate (crate `mlt`): wiring, commands, events, scheduler
│  ├─ Cargo.toml  tauri.conf.json  build.rs
│  ├─ src/{lib.rs, main.rs}   #    depends on mlt-core + mlt-adapters
│  └─ capabilities/  icons/
├─ sidecars/                  # ◻ macos-helper (Swift: EventKit, Safari cookies), windows-helper (.NET)
├─ docs/                      # ✅ this folder
└─ .github/workflows/         # ◻ CI (see QUALITY_GATES.md)
```

Crate names: `mlt-core` (lib `mlt_core`), `mlt-adapters` (lib `mlt_adapters`), `mlt` (the app).
Adapters begin as a single crate and split into per-subsystem crates (`http`, `store`, `secrets`,
`calendar-web`, `calendar-native`, `cookies`, `notify`, `loginitem`) once they have real weight.

**The golden rule:** `crates/core` has no dependency that performs IO. Enforced in CI by
forbidding `reqwest`, `tokio::fs`, `std::fs`, `std::time::SystemTime::now`, etc. in core
(see QUALITY_GATES.md §"Architecture fitness"). Time comes from `Clock`. Randomness, if
ever needed, comes from a port too.

## 5. Subsystem: usage tracking

The heart of the product, and the place where the anti-slop contract lives.

A provider is a **descriptor + an ordered chain of typed `FetchStrategy` units** (refined
from CodexBar's pipeline, see [research/PROVIDERS.md](../research/PROVIDERS.md)). The pipeline
tries strategies in order, skipping unavailable ones and falling back on error:

```rust
// crates/core/src/providers/mod.rs
pub enum FetchKind { Cli, OAuth, Cookie, ApiToken, LocalProbe, WebDashboard }

#[async_trait]
pub trait FetchStrategy {                       // one credential path
    fn kind(&self) -> FetchKind;
    async fn is_available(&self, ctx: &FetchContext) -> bool;   // creds present?
    async fn fetch(&self, ctx: &FetchContext) -> Result<UsageSnapshot, FetchError>;
    fn should_fallback(&self, err: &FetchError) -> bool;        // advance or bail
}

pub struct ProviderDescriptor {
    pub id: ProviderId,                          // stable slug, e.g. "codex"
    pub metadata: ProviderMetadata,              // display name, labels, icon, dashboard URL
    pub strategies: Vec<Arc<dyn FetchStrategy>>, // ordered fallback chain
}
// e.g. ClaudeCode = [OAuthFromFile, OAuthFromKeychain, CookieWeb, CliScrape]
```

Strategies are **composed from reusable capability blocks** so each new provider is small
(a chain + a fixture), not a snowflake:

- `OAuthFlow` — authorization-code + PKCE, loopback redirect, token refresh.
- `DeviceFlow` — device-code grant for headless/CLI-style auth.
- `ApiKeyAuth` — static key from keychain or discovered config.
- `CookieSource` — pull a session cookie for a domain (via native sidecar).
- `CliCredSource` / `JsonlLogSource` — read an installed CLI's config/logs.
- `HttpFetcher` — typed GET/POST with retry/backoff/rate-limit handling.
- `UsageParser` — JSON-path/typed extraction into the normalized `UsageWindow`.

Bespoke providers override `fetch_windows` directly; common REST providers assemble
blocks. **Every provider ships with a recorded-HTTP fixture and a golden-file test** —
this is the enforceable contract (QUALITY_GATES.md §"Provider contract tests").

Normalized model (provider-agnostic):

```
UsageWindow { provider_id, kind: Session|Weekly|Monthly|Custom,
              used, limit, unit: Tokens|Requests|USD|Percent,
              resets_at: DateTime, fetched_at: DateTime, status: Ok|Stale|Error }
```

Scheduler loop (in `app`) ticks at the user's cadence (manual / 1m / 2m / 5m / 15m),
fans out the provider pipelines across enabled providers with bounded concurrency, writes
results via `UsageRepo`, and emits a Tauri event the UI subscribes to. All timing via
`Clock` so it's testable.

### 5.1 Resilience (mandatory, in `core`) — see [ADR 0015](./adr/0015-resilience-patterns.md)

Because every subscription endpoint is private, rate-limited, and drifts, these are core
behaviors, not polish: **lossy/optional decoding** (one bad field never breaks a snapshot),
**reset-time backfill**, **consecutive-failure gate** (hide the first flake when prior data
exists), **per-probe timeout**, **startup connectivity retry**, **per-provider rate-limit
gate** (mandatory for Claude 429s), and explicit `Ok | Stale | Error` states — never a
crash. All are pure functions over the injected `Clock`, so they're deterministically tested.

## 6. Subsystem: alarms

Persisted, reliable across sleep / quit / reboot. See [ADR 0009](./adr/0009-alarm-engine.md).

- Alarms persisted in SQLite with `next_fire_at` (one-off + recurring via RRULE-lite).
- While running: an in-process scheduler sleeps until the soonest `next_fire_at`.
- On **app launch** and on **system-wake** events: a `reconcile()` pass scans for
  alarms whose `next_fire_at <= now`, fires them (coalescing a flood into one
  "you missed N" notification per the user's setting), and recomputes the next fire.
- Pure logic: `reconcile(now, alarms) -> (to_fire, updated_schedule)` takes `now` as an
  argument (injected `Clock`), so the entire engine is unit-tested without real waiting.
- Two alarm sources unify through the same pipeline: **user alarms** and
  **usage-derived alarms** (threshold crossed, window about to reset).

## 7. Subsystem: calendar (read-only)

Behind one `CalendarPort` trait, multiple adapters:

- `GoogleCalendarAdapter` — Calendar API, `calendar.readonly` scope, OAuth+PKCE. All OS.
- `MsGraphAdapter` — Microsoft Graph, read-only scope. All OS.
- `EventKitAdapter` — **macOS only**, via the Swift sidecar; reads every calendar already
  configured in macOS Calendar.app (iCloud/Exchange/local) with no extra OAuth, behind an
  EventKit entitlement + usage-description prompt.

Read-only for v1 (no write scopes, no blast radius). Write is a future ADR. Calendar
events are cached in SQLite and used for time context / "schedule-around" hints.

## 8. Security & privacy model

- **Consent:** auto-discovery is **metadata-only** (does a Chrome profile / Codex CLI
  config *exist*?). No cookie decryption or credential read happens until the user
  toggles that specific source on, with a plain-language disclosure. See [ADR 0012](./adr/0012-consent-model.md).
- **Secret storage:** OAuth tokens and API keys live in the **OS keychain** via the
  `keyring` crate behind `SecretStore`. **Never** in SQLite, never in logs. Harvested
  cookies are used transiently and not persisted beyond what's needed to call the API.
- **Logs:** `tracing` with a redaction layer that scrubs tokens/keys. Crash/error
  reporting is **opt-in only**.
- **Least privilege:** each provider/source requests the narrowest scope/permission.
- **Native sidecars are crash-isolated** — a panic in EventKit/cookie code can't take
  down the core or leak through the address space.

## 9. Testing strategy

| Layer | What | How |
|-------|------|-----|
| Unit (core) | reset math, threshold eval, reconciler, normalizers, provider blocks | pure fns + fakes for ports; no IO; runs in ms |
| Provider contract | each provider parses recorded responses into correct `UsageWindow` | recorded-HTTP fixtures (VCR-style) + golden files |
| Adapter integration | sqlx queries, keychain, http retry | real SQLite (temp file), mocked HTTP server (`wiremock`) |
| Sidecar contract | JSON-RPC schema between core and Swift/.NET helpers | schema-validated fixtures both sides |
| E2E (smoke) | popover opens, providers render, alarm fires | Tauri WebDriver on the CI matrix |

Fakes for every port live in `core` test support so any subsystem can be exercised in
isolation. **No real provider credentials in CI** — fixtures only.

## 10. Portability matrix

| Concern | macOS | Windows | Linux |
|---------|-------|---------|-------|
| Tray + popover | ✓ | ✓ | ✓ (tray support varies by DE) |
| Notifications | ✓ | ✓ | ✓ |
| Login-item | LaunchAgent | Run key / Startup | systemd user / autostart |
| Keychain | Keychain | Credential Manager | Secret Service (libsecret) |
| Calendar web (Google/Graph) | ✓ | ✓ | ✓ |
| Calendar native | EventKit (sidecar) | — (web only v1) | — (web only v1) |
| Browser cookies | Safari + Chromium | Chromium (DPAPI) | Chromium (kwallet/gnome) |

Anything in an OS column that isn't ✓ is a deliberate v1 gap, tracked in OPEN_QUESTIONS.

## 11. What v1 explicitly does NOT include

No backend, no account, no cross-device sync, no team features, no calendar *write*,
no usage polling while fully quit, no Linux/Windows native calendar, no WASM/third-party
provider plugins, no in-product LLM feature. Each is a future ADR if pursued.
