# 0017 — Provider account identity: auto-fetched, cached once, siloed per source

**Status:** Accepted · **Date:** 2026-06-01
**Extends:** [0015 — resilience patterns](./0015-resilience-patterns.md) · **Relates to:** [0016 — API-key connection model](./0016-api-key-connection-model.md) · **Backed by:** [research/PROVIDERS.md](../research/PROVIDERS.md)

## Context
With more than one provider connectable, the popover needs to answer "**which account am I
looking at?**" — not just "which provider". A user may have several logins of the same kind,
so the panel should show the account's **email** when the provider exposes one. Requirements:
auto-fetched (never typed by the user), shown whenever possible, and **siloed** — one
provider's identity must never render under another (an MLT invariant).

Two facts shaped the design:
- The only identity Anthropic exposes for a Claude Code OAuth login is the **`account.email_address`
  / `organization.name`** returned by the OAuth profile endpoint
  (`GET api.anthropic.com/api/oauth/profile`, verified to exist: 401 without a token, not 404).
  The `api/oauth/usage` body carries no identity. OpenRouter's key endpoint exposes no email.
- The Claude usage endpoint is **aggressively rate-limited** (ADR 0015 / PROVIDERS.md), and the
  fetch strategy is rebuilt per poll (stateless across polls) — so we must **not** add a profile
  request to every 60-second poll.

## Decision
Model identity as **`AccountIdentity { email, organization }`** (both optional) and resolve it
**best-effort, once, then cache** it:

- A new **`IdentityStore`** port (file-backed `FileIdentityStore`, mirroring consent/labels)
  caches identity per provider as plain JSON in the app config dir. Identity is
  account-identifying **display metadata** — not a secret — so, like consent and labels, it
  lives there, **never the keychain, never the DB, never logs**.
- The Claude strategy, after a successful usage fetch, calls `resolve_identity`: returns the
  cached value if present, else does **one** profile `GET` (reusing the same OAuth token) and
  caches it. Later polls hit the cache → **no recurring load** on the rate-limited host.
- The profile fetch is **fully non-fatal** (ADR 0015): any failure — transport, non-200,
  unparseable, or an empty profile — yields no identity and never affects the usage result.
  The parser is pure and lossy: unknown shapes degrade to an empty identity, never an error.
- Identity rides out on `UsageSnapshot.account` (immediate, same-session display) and is
  surfaced on `SourceState.account` via `discover_sources` (persisted display on later opens
  and in the connect screen). The UI shows the selected provider's email only when the live
  snapshot's `provider` matches the selection — **never mixing providers**.

The user-assigned **label** is now strictly a **custom title** (a nickname shown as the panel
heading), decoupled from identity: it never replaces the provider's own `display_name`, and the
email is fetched, never typed.

## Alternatives considered
- **Capture email from the OAuth token-refresh response only.** Evidence-backed and needs no
  extra endpoint, but we refresh rarely (we piggyback on Claude Code's fresh token), so identity
  would lag up to a token lifetime. Rejected: not "shown whenever possible".
- **Fetch the profile on every poll (no cache).** Simplest, but adds a request per minute to a
  rate-limited host — exactly what ADR 0015 forbids. Rejected.
- **Inject an unverified profile URL.** Rejected on principle: the endpoint was first verified
  (401 vs 404) before shipping a call to it.

## Consequences
- **+** Immediate identity on first connect (rides the first usage fetch), persisted across
  restarts, shown in both the usage view and the connect screen, siloed per source.
- **+** No added steady-state load on the rate-limited usage endpoint (fetched at most once).
- **−** A third per-source settings file (`identity.json`) alongside `consent.json` /
  `labels.json`, and `discover_sources` gains an `IdentityStore` parameter. Accepted — it
  mirrors the existing store pattern exactly.
- **−** Identity for a provider that never has a successful usage fetch (e.g. persistent 429)
  stays unresolved until one succeeds. Accepted: identity is a display nicety, never load-bearing.
