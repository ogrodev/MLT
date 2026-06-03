# MLT — Task backlog

Product-owner breakdown of [`PRD.md`](../PRD.md) into deliverable tasks. **One doc per task.**
Each task is the **smallest slice that delivers real, user-testable value** — a thing the
owner can connect, see, or be notified by — not a component or a layer.

These docs say **what** and **how we'll know it's done**. They deliberately contain no
architecture or implementation detail — for the "how", see
[`ARCHITECTURE.md`](../ARCHITECTURE.md), the [ADRs](../adr/), and
[`research/PROVIDERS.md`](../research/PROVIDERS.md).

## Checking status at a glance

`make tasks` prints every task with its declared **Status** and how many acceptance
criteria are checked, then **fails if the two disagree** — a task marked `✅ done` with
unchecked criteria, or one whose criteria are all checked but isn't marked done, is flagged
so a stale Status line can't slip by. Narrow the view with `make tasks ARGS=--todo` or
`make tasks ARGS=--done` (or run `scripts/check-tasks.sh` directly).

Each task's `**Status:**` is one of `◻ not started`, `🟡 partial`, or `✅ done`, and "done"
also requires every acceptance-criteria checkbox (`- [x]`) ticked plus the shared
Definition of Done below.

## How to read a task

Each task has: a one-line value statement, the user story, in/out scope, and its own
**Acceptance criteria**. Every task also inherits the **shared Definition of Done** below —
this is the "every task goes through full CI and QA" rule, written once.

## Shared Definition of Done (applies to every task)

A task is done only when **all** of these are true, in addition to its own acceptance criteria:

- [ ] **Full CI check is green** (`make check`, mirroring CI): Rust format, Clippy with
      warnings denied, core-purity / architecture-fitness, the test suite, `cargo-deny`
      (advisories + licenses + bans), Socket Firewall on dependency fetch, `cargo-machete`
      (no unused deps), Biome (frontend lint/format), `svelte-check` + `tsc` type-check,
      gitleaks secret scan, and **core line coverage ≥ 80%**. See [`QUALITY_GATES.md`](../QUALITY_GATES.md).
- [ ] **Cross-platform CI lanes pass** for the OSes the task touches (macOS / Windows / Linux).
- [ ] **New behaviour is covered by tests using fakes/fixtures** — never real provider
      accounts, never live Keychain prompts in `cargo test`.
- [ ] **Reused-login providers are multi-account by default** ([ADR 0019](../adr/0019-multi-account-discovery.md)):
      a task that adds an OAuth-subscription provider (one whose login is reused from a local
      store, like Codex or Claude Code) plugs into the **shared per-account discovery** — it
      registers the provider in the `ACCOUNT_PROVIDERS` (core) and `PROVIDERS` (adapter) tables
      and adds a per-account strategy builder, so **every** login (across Oh My Pi profiles + the
      vendor store, deduped by account id) becomes its own siloed source. It does **not** add a
      bespoke single-source provider, and it reuses the shared per-account consent / identity /
      cache-key namespacing. (N/A for API-key or non-provider tasks.)
- [ ] **Manual QA passes**: a real menu-bar build installs and launches via `make qa`
      (not a dev server), and each acceptance criterion is verified by hand as a user.
- [ ] **Docs updated**: the [PRD delivery checklist](../PRD.md#delivery-checklist--whats-left-vs-the-original-plan)
      status is flipped, any new user-facing setting is documented, and any resolved
      [open question](../OPEN_QUESTIONS.md) is promoted to an ADR.
- [ ] **Conventional Commits**, and **no secrets** in code, logs, or the local DB.

## Already shipped (no task needed)

- **§2 At-a-glance usage tracking** — windows, %, countdowns, auto-refresh, stale/error states.
- **§12 Manual QA & installability (macOS)** — one-command `make qa` build + install.
- **Claude Code** provider (subscription usage + safe token refresh).
- **§11** the full CI quality-gate suite is green (the *signed release pipeline* is task 027).

## Backlog

| # | Task | PRD capability | Depends on |
|---|------|----------------|-----------|
| [001](./001-popover-experience-macos.md) | Popover experience & appearance QA (macOS) | §1 | — |
| [002](./002-source-discovery-consent.md) | Local source discovery + consent screen | §4, §9 | — |
| [003](./003-api-key-management.md) | Enter / replace / remove an API key | §4 | 002 |
| [004](./004-disconnect-provider.md) | Disconnect a provider without restart | §4 | 002 |
| [005](./005-provider-codex.md) | Codex subscription usage | §3 | 002 |
| [006](./006-provider-openrouter.md) | OpenRouter API usage | §3 | 003 |
| [007](./007-provider-openai-api.md) | OpenAI API usage (honest limits) | §3 | 003 |
| [008](./008-provider-anthropic-api.md) | Anthropic API usage (honest limits) | §3 | 003 |
| [009](./009-alarm-threshold-alerts.md) | Threshold usage alerts → notifications | §5 | 005–008 |
| [010](./010-alarm-window-reset.md) | Window-reset notification (opt-in) | §5 | 009 |
| [011](./011-alarm-one-off.md) | One-off user alarm | §5 | 009 |
| [012](./012-alarm-recurring.md) | Recurring user alarm (RRULE-lite) | §5 | 011 |
| [013](./013-alarm-missed-catchup.md) | Missed-alarm catch-up on launch/wake | §5 | 011, 012 |
| [014](./014-calendar-google.md) | Google Calendar — read-only upcoming events | §6 | — |
| [015](./015-calendar-outlook.md) | Outlook Calendar — read-only upcoming events | §6 | — |
| [016](./016-calendar-macos-device.md) | macOS device calendars — read-only | §6 | — |
| [017](./017-calendar-manage-revoke.md) | Manage & revoke calendar access | §6 | 014–016 |
| [018](./018-start-at-login.md) | Start at login (toggle) | §7 | — |
| [019](./019-wake-catchup-refresh.md) | Wake-from-sleep catch-up refresh | §7 | — |
| [020](./020-platform-windows.md) | Windows runtime parity | §8 | — |
| [021](./021-platform-linux.md) | Linux runtime parity + documented gaps | §8 | — |
| [022](./022-privacy-statement.md) | In-app privacy & data statement | §9 | — |
| [023](./023-telemetry-opt-in.md) | Opt-in telemetry / crash reporting | §9 | — |
| [024](./024-dist-macos-signing.md) | macOS signed & notarized installer | §10 | — |
| [025](./025-dist-windows-signing.md) | Windows signed installer | §10 | — |
| [026](./026-dist-auto-update.md) | Auto-update (check + apply) | §10 | 024 |
| [027](./027-release-pipeline.md) | Release pipeline (tag → signed artifacts) | §11 | 024, 025, 026 |
