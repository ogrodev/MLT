# 0018 — Multi-provider popover: provider switcher + per-source custom names

**Status:** Accepted · **Date:** 2026-06-01
**Relates to:** [0012 — consent model](./0012-consent-model.md) · [0013 — single tray icon](./0013-single-tray-icon.md) · [0016 — API-key connection model](./0016-api-key-connection-model.md) · [0017 — provider account identity](./0017-provider-account-identity.md)

## Context
The popover began single-provider (a Claude-only usage view). [ADR 0016](./0016-api-key-connection-model.md)
added API-key providers (OpenRouter first), so **more than one source can be connected at
once**. The popover now has to: show *which* providers are connected, let the user **switch**
which one's panel they're viewing, and let them **tell two of the same kind apart** — including
assigning a custom name when a provider's generic name (e.g. "Claude Code") isn't enough.

Two constraints shape it. **Only Claude Code reports usage today** (OpenRouter usage is a later
task) — the UI must not fabricate bars for a provider that reports nothing. And provider data is
**siloed** (an MLT invariant): one provider's usage or identity must never render under another.

## Decision
- **Switcher over active sources.** The usage view enumerates the **active** sources (the
  [ADR 0012](./0012-consent-model.md) gate: `present && enabled` for `LocalLogin`, `enabled`
  alone for `ApiKey` per [ADR 0016](./0016-api-key-connection-model.md)) and renders a tab
  switcher (shown when ≥2 are active) of **icon + the provider's canonical `display_name`**.
  Selecting a tab chooses whose panel is shown; the selection falls back safely (→ Claude, then
  the first active source) so disconnecting the selected provider never blanks the view.
- **Per-source custom name = a title, not a rename.** A new **`SourceLabels`** port (file-backed
  `FileLabelStore`) persists an optional user-assigned name per source. It is shown as the panel
  **title** and **never replaces** the provider's own `display_name` in the tab or header. It is
  a non-secret UI preference stored alongside consent — never the keychain — and plays **no part**
  in the `active` gate (cosmetic, not consent). Blank clears it.
- **Each panel shows only its own data.** Claude renders its usage windows; a provider with no
  usage endpoint yet (OpenRouter) shows an honest *"connected — usage coming"* placeholder rather
  than fake bars ([ADR 0015](./0015-resilience-patterns.md)). Account identity ([ADR 0017](./0017-provider-account-identity.md))
  appears as the title's subtitle, sourced only from the **matching** provider's snapshot/cache.
- **Provider marks.** Tabs use the official single-path brand SVGs (simple-icons, `currentColor`,
  theme-adaptive) — nominative use to identify a provider, consistent with the CodexBar-style UX
  ([ADR 0013](./0013-single-tray-icon.md)). A generic fallback covers providers without a mark.

## Alternatives considered
- **Overload `display_name` with the custom name (rename in place).** Rejected: it erases which
  provider you're connected to, and per-kind logic keys on the canonical id/name.
- **One flat list instead of a switcher.** Rejected for a small, dense popover — tabs keep each
  provider's usage panel readable and match the reference UX.
- **Blank or fabricated usage for providers without an endpoint.** Rejected on honesty/resilience
  grounds ([ADR 0015](./0015-resilience-patterns.md)): a placeholder is truthful; invented bars are not.

## Consequences
- **+** Multiple providers coexist in one popover, each clearly identified (icon + canonical
  name) with an optional custom title; no provider's data renders under another.
- **+** `SourceLabels` mirrors the consent/identity store pattern exactly and is unit-tested with
  fakes (no live account, no keychain).
- **−** A second per-source settings file (`labels.json`, beside `consent.json` / `identity.json`)
  and a `SourceLabels` parameter on `discover_sources`. Accepted — one consistent pattern.
- **−** A provider's panel shows a placeholder until its usage fetch lands. Accepted: honest and
  incremental.
- Interim: the TS bindings (`src/lib/usage.ts`) for the new `label`/`account` fields are
  hand-synced — `tauri-specta` is still "to come" ([ADR 0010](./0010-ui-svelte-tailwind.md)).
