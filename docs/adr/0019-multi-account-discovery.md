# 0019 — Multi-account discovery: per-account sources from local OAuth stores

**Status:** Accepted · **Date:** 2026-06-02
**Extends:** [0012 — consent model](./0012-consent-model.md) · [0017 — provider account identity](./0017-provider-account-identity.md) · [0018 — multi-provider popover](./0018-multi-provider-popover.md) · **Relates to:** [0014 — v1 provider set](./0014-v1-provider-set.md)

## Context
The first cut modelled each reused-login provider as **one static source** (`claude-code`,
`codex`) that read a single vendor store — the CLI's own credentials file/keychain. Reality is
messier:

- A person has **several logins** for the same provider (a personal plan and a work/Team account),
  and drives them through tools that keep **per-profile** credentials. On this machine the live
  Codex *and* Claude logins live in **Oh My Pi's** per-profile store
  (`~/.omp[/profiles/*]/agent/agent.db`, table `auth_credentials`), **not** in `~/.codex/auth.json`
  or Claude Code's keychain — those held only a stale/leftover token.
- A single static source per provider cannot represent more than one account, and "read the
  vendor CLI's one store" silently misses every login managed elsewhere.

So discovery must find **all** of a provider's logins across the local stores and present each as
its own connectable source — and this must be a **general** mechanism, so a new provider gets the
same treatment without bespoke code.

## Decision
- **A provider is *single* or *multi-account*.** Single sources stay in the static
  [`source_catalog`] (API-key sources; the standalone Claude Code CLI keychain login). A
  multi-account provider instead **expands into one source per discovered login**.
- **Per-account source id = `<base>:<account_id>`** (e.g. `codex:<uuid>`, `claude-code:<uuid>`),
  where `account_id` is the provider's stable account id. This single id namespaces **everything
  per account**: consent, [identity](./0017-provider-account-identity.md), and the refreshed-token
  cache key `oauth.<base>.<account_id>`. Two logins of the same provider therefore never collide,
  and disconnect purges exactly one account's cached token.
- **One registry, two small tables.** Core's `ACCOUNT_PROVIDERS` (base → display name +
  disclosure) and the adapter's `PROVIDERS` (base → Oh My Pi provider id) are the *only* places a
  multi-account provider is declared. Adding one is a row in each plus a per-account strategy
  builder — the discovery, dedup, consent, identity, cache, routing, and UI are all shared.
- **Discovery reads local OAuth stores, deduped.** The shared adapter (`accounts.rs`) reads every
  Oh My Pi profile DB (`provider = <id> AND credential_type = 'oauth'`) plus any provider-specific
  vendor store, and **dedupes by account id keeping the freshest token**. The credential blob
  shape is identical across providers (`{ access, refresh, accountId, email, expires }`), so the
  reader is provider-agnostic. An entry **without an `account_id` is skipped** — it has no stable
  id and is almost always a rotated/legacy token that must not surface as a phantom source.
- **Present by discovery.** A discovered account is *present* (it was found), so it skips the file
  probe; its email seeds the panel subtitle immediately, before any usage fetch resolves identity.
  The [ADR 0012](./0012-consent-model.md) gate still applies: nothing is fetched until the user
  opts that account in.
- **Read-only, refresh into OUR keychain.** MLT only **reads** the vendor and Oh My Pi stores;
  refreshed copies are cached under MLT's own keychain service, **never** written back (the
  AGENTS.md invariant). This holds for every multi-account provider uniformly.
- **A vendor-CLI login with no local account id stays a single static source.** Claude Code's
  keychain login carries no local account id (its identity comes from a network profile call), so
  it cannot key a per-account source without a fetch; it remains the static `claude-code` source.
  Codex's `auth.json` *does* carry an account id, so it folds into the per-account dedup. The
  asymmetry is data-driven, not provider-special-casing.

## Alternatives considered
- **One static source per provider.** Rejected: cannot represent multiple logins, and reads only
  the often-stale vendor CLI store, missing the user's real accounts.
- **Fully dynamic with no static CLI fallback.** Rejected: a user who only uses the Claude Code
  CLI (no Oh My Pi) would see no Claude source.
- **Don't couple to Oh My Pi's schema — make the user configure paths.** Rejected as the default:
  it isn't automatic. The coupling is accepted but **isolated** to one parser that degrades
  gracefully (a shape it doesn't recognize is skipped, never fatal).

## Consequences
- **+** Any OAuth-subscription provider gets multi-account discovery for free: one `ACCOUNT_PROVIDERS`
  row, one `PROVIDERS` row, one strategy builder. Codex and Claude Code share the entire path.
- **+** Per-account consent / identity / cache namespacing means logins are fully siloed and
  disconnect is surgical (purges one account's token, never another's).
- **+** Always uses the freshest token the vendor refreshed (re-read per fetch), so MLT rarely
  refreshes itself; when it must, the copy lands in OUR keychain.
- **−** Couples discovery to Oh My Pi's internal `auth_credentials` schema (`credential_type`,
  the `data` JSON shape). If Oh My Pi changes it, the single `parse_omp_credential` needs
  updating. Accepted: it is one isolated parser and fails safe.
- **−** The reused-login usage scope guard (Claude's `user:profile`) now fires only when scopes
  are **known and insufficient**: an Oh My Pi credential's stored blob omits scopes, so such a
  token is trusted and the endpoint is the authority (a real 401 surfaces as the stale/error tile).
- Interim: the front-end recognizes per-account ids by their `<base>:` prefix (mirroring the
  backend `fetch_for` routing) — hand-synced until `tauri-specta` ([ADR 0010](./0010-ui-svelte-tailwind.md)).
