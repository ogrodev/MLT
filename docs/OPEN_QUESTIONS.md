# Open Questions & Deferred Decisions

Tracks everything deliberately left unresolved or assumed-by-default, so nothing becomes
a silent loose end. Each item has an owner action. Resolve → promote to an ADR.

## Operational defaults (assumed, not yet grilled)

| # | Topic | Assumed default | Revisit if |
|---|-------|-----------------|------------|
| O1 | Repo shape | Single Cargo workspace + pnpm, one monorepo | UI/team grows to want a split |
| O2 | CI provider | GitHub Actions, 3-OS matrix | you prefer GitLab/other |
| O3 | Auto-update | `tauri-plugin-updater` (signed manifest) | you want a store-only model |
| O4 | Observability | `tracing` local logs + **opt-in** crash reporting (e.g. Sentry) | privacy stance changes |
| O5 | License | TBD — pick before first public release | — |
| O6 | App/product name | "MLT" working title | branding decided |

## Real-world dependencies (logistics, not code)

| # | Item | Why it matters | Action |
|---|------|----------------|--------|
| D1 | **Apple Developer account** ($99/yr) | Required to notarize the macOS build; without it users get Gatekeeper blocks | acquire before mac release |
| D2 | **Windows code-signing cert** | Without it, SmartScreen warns on every install | acquire before win release; OV vs EV cost tradeoff |
| D3 | **Per-provider OAuth client registration** | Each OAuth provider (Google, MS, AI providers) needs a registered client_id; some need verification/review | inventory per provider |
| D4 | Provider usage-API access tiers | Some providers expose usage only to admin/org keys; some have **no** usage API | audit coverage per provider (see Q3) |

## Product/architecture questions still open

| # | Question | Notes |
|---|----------|-------|
| Q1 | Calendar **write**? | v1 is read-only. Write = new scopes, idempotency/undo, blast radius. Future ADR. |
| ~~Q2~~ | ~~Per-provider tray icon vs merged icon?~~ | **RESOLVED:** single tray icon; mirror the CodexBar experience as v0.1, then evolve. See [ADR 0013](./adr/0013-single-tray-icon.md). |
| ~~Q3~~ | ~~Which providers ship in v1?~~ | **RESOLVED:** v1 set = Claude Code (sub), Codex (sub), Anthropic API, OpenAI API, OpenRouter API. See [ADR 0014](./adr/0014-v1-provider-set.md) + [research/PROVIDERS.md](../research/PROVIDERS.md). |
| Q4 | What does "usage" mean per provider? | $ vs tokens vs requests vs quota headroom — normalize, but display rules differ. |
| ~~Q5~~ | ~~Recurring alarm grammar~~ | **RESOLVED:** RRULE-lite (daily / weekly / every-N-days), UTC-day arithmetic, no tz lib in core. See [ADR 0020](./adr/0020-rrule-lite-recurrence.md). |
| ~~Q6~~ | ~~Missed-alarm policy~~ | **RESOLVED:** per-user `MissedPolicy` (fire-each / coalesce, default fire-each); a recurring alarm's missed occurrences collapse to one catch-up. See [ADR 0021](./adr/0021-missed-alarm-policy.md). |
| Q7 | Linux tray reality | Tray support varies by desktop environment; define the supported set. |
| Q8 | Cookie decryption scope | Which browsers in v1? Chromium DPAPI/Keychain/libsecret each differ. |
| Q9 | Sidecar distribution | Bundle Swift/.NET helpers in the app package; signing + path resolution per OS. |
| Q10 | Telemetry opt-in copy + data map | Exactly what (if anything) leaves the device when opted in. |

## Risks to watch

- **Provider API drift** — usage endpoints are often undocumented/unstable. Mitigation:
  fixtures + status badges + graceful `Stale`/`Error` states, never a hard crash.
- **Cookie-reuse fragility & ToS** — browsers change cookie storage; some provider ToS may
  frown on session reuse. Mitigation: per-source opt-in, prefer official OAuth/API where
  it exists, document the tradeoff.
- **macOS notarization & EventKit entitlements** — slow feedback loop; wire the dry-run
  into CI early (D1).
- **Portability leak via native code** — keep it strictly behind ports + sidecars; the
  architecture-fitness gate (QUALITY_GATES §2) defends this.
