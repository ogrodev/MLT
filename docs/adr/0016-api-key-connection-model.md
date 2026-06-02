# 0016 — API-key sources: a typed credential kind, validate-before-store

**Status:** Accepted · **Date:** 2026-06-01
**Extends:** [0012 — consent model](./0012-consent-model.md) · **Backed by:** [research/PROVIDERS.md](../research/PROVIDERS.md)

## Context
ADR 0012 modelled "connecting" as **metadata-only discovery → per-source opt-in**: a source
is read only when it is both *present* (a login was discovered locally) and *enabled* (the
user opted in). That fits sources that **reuse an existing login** (Claude Code, Codex), but
not API-key providers (OpenRouter first, per [ADR 0014](./0014-v1-provider-set.md)): there is
**no local login to discover** — the user pastes a key. Task 003 needs that key's full
lifecycle (enter / replace / remove) to be safe and to take effect without a restart.

## Decision
Add a typed **`CredentialKind { LocalLogin, ApiKey }`** to the source catalog, so one catalog
still drives both the connect screen and the refresh loop, and each source declares how it
connects:

- **`LocalLogin`** — unchanged ADR 0012 semantics: `active = present && enabled`.
- **`ApiKey`** — there is nothing to discover, so **storing a validated key *is* the act of
  connecting and consenting**. `active = enabled`; presence is irrelevant.

Two rules make API keys safe:

1. **Validate before storing.** Entering a key first authenticates it against the provider
   (OpenRouter: an authenticated `GET /api/v1/key`, status only). The key is written to the
   keychain and the source marked connected **only on success** — a rejected or unverifiable
   key returns a clear error and the source stays disconnected (never a silent "connected").
   The decision (HTTP status → verdict) is pure `core` logic over the `HttpPort`; the IO is an
   adapter. Reading *usage* with the key is a separate, later task — validation is not usage.
2. **Keychain-only, write-once-direction.** The key lives in our OS keychain
   (`api_key.<id>`, namespaced apart from `oauth.*`) and is **never** returned across the
   Tauri boundary, logged, or written to the DB. The UI shows only a connected/disconnected
   state and an "Add / Replace / Remove" affordance — never the key. Removal deletes the
   keychain entry and clears consent. All of this takes effect immediately, no restart.

## Alternatives considered
- **Reuse the plain consent toggle, store the key separately, no validation** — simplest, but
  violates acceptance 4 (an invalid key would silently read as connected) and lets consent and
  key existence diverge.
- **Treat "a key is stored" as `present` via the discovery probe** — overloads the
  metadata-only probe (ADR 0012) to read our own secret store, blurring a deliberate boundary.
  Rejected: the consent flag already captures "connected" for `ApiKey`, kept in lockstep with
  the key by the command layer.

## Consequences
- **+** One catalog, two honest connect affordances; the consent gate (ADR 0012) is preserved
  and made explicit per kind, and unit-tested with fakes (no live account, no real keychain).
- **+** "Validate before store" gives a truthful connected state and a clear error path.
- **−** `active()` and `active_sources()` now branch on `CredentialKind`; new API providers
  must register their own validation endpoint. Accepted — it is one match arm per provider.
