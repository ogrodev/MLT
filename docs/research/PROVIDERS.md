# Provider Integration Research

**Date:** 2026-05-30 · **Method:** reverse-engineered from CodexBar source
(`github.com/steipete/CodexBar`) + cross-checked against official provider docs (May 2026).
**Purpose:** ground the v1 provider set in reality before building, and extract reusable
mechanisms for our Rust/Tauri port.

> CodexBar's real provider engines live in **`Sources/CodexBarCore/Providers/<P>/`**;
> the per-provider docs in `docs/<p>.md` are accurate and were the primary reference.

## TL;DR — the three findings that change our plan

1. **Two product categories, not one.** "Subscription usage" (Claude Code / Codex plan
   windows) and "API-key cost" (Anthropic/OpenAI/OpenRouter billing) are completely
   different data behind completely different auth. We track both, but they're separate
   strategies. Don't conflate them in the model or the UI.
2. **API cost for Anthropic & OpenAI is gated behind org Admin keys** that individual
   developers usually **cannot get** ("The Admin API is unavailable for individual
   accounts"). A normal `sk-ant-api…` / `sk-…` key returns *no usage* (Anthropic) or only
   a flaky legacy balance (OpenAI). **OpenRouter is the only easy API-cost win** (normal
   key, pre-aggregated daily/weekly/monthly). This must be surfaced honestly in the UI.
3. **Every subscription usage endpoint is private/undocumented and reached by spoofing an
   official client's User-Agent.** This is the inherent, ongoing risk of the whole product
   category (rate-limits, schema drift, ToS). We mitigate with the resilience patterns in
   §"Cross-cutting" — we do not eliminate it. See [risk note](#risk--tos).

## v1 provider set (per [ADR 0014](../adr/0014-v1-provider-set.md))

| Provider | Category | Credential source | Usage source | Feasibility |
|----------|----------|-------------------|--------------|-------------|
| **Codex** (ChatGPT/OpenAI sub) | Subscription | `~/.codex/auth.json` (plain JSON, OAuth tokens) | `GET chatgpt.com/backend-api/wham/usage` | **Easy–Moderate** ✅ build first |
| **OpenRouter** (API) | API cost | normal `sk-or-v1…` key | `/api/v1/key` + `/api/v1/credits` | **Easy** ✅ build first |
| **Claude Code** (Anthropic sub) | Subscription | `~/.claude/.credentials.json` OR Keychain `"Claude Code-credentials"` | `GET api.anthropic.com/api/oauth/usage` | **Moderate** (429 rate-limits) |
| **Anthropic API** | API cost | **Admin key** `sk-ant-admin…` (org-only) | `/v1/organizations/{cost_report,usage_report/messages}` | **Moderate** (key rarely available) |
| **OpenAI API** | API cost | **Admin key** `sk-admin…` (preferred); normal key → legacy balance | `/v1/organization/{costs,usage/completions}` | **Moderate** (key + bucket chunking) |

Recommended build order: **OpenRouter → Codex → Claude Code → OpenAI API → Anthropic API**
(easiest/most-valuable first; the two admin-key API providers last since fewest users can use them).

---

## Subscription providers (plan-window usage)

### Codex (ChatGPT/OpenAI subscription) — *cleanest of all*
- **Credential:** read `~/.codex/auth.json` (or `$CODEX_HOME/auth.json`). Shape:
  `{ "tokens": { "id_token","access_token","refresh_token","account_id" }, "last_refresh" }`.
  Plain JSON, **no Keychain**. Refresh stale access tokens via
  `POST auth.openai.com/oauth/token` (`client_id: app_EMoamEEZ73f0CkXaXp7hrann`,
  `grant_type: refresh_token`). MLT stores refreshed copies only under its own keychain
  service, never back into `auth.json`, so it stays read-only against the vendor store.
- **Usage:** `GET chatgpt.com/backend-api/wham/usage` (base from `~/.codex/config.toml`
  `chatgpt_base_url`). Headers: `Authorization: Bearer`, `ChatGPT-Account-Id`, a `User-Agent`.
  Map: `rate_limit.primary_window` → session (5h), `secondary_window` → weekly; each has
  `used_percent`, `reset_at` (unix), `limit_window_seconds`; `credits` → balance.
- **Risk:** internal endpoint "can change without notice" (openai/codex#10869), but the
  Codex CLI itself polls it every 60s, so it's de-facto stable. Use **lossy decoding**.

### Claude Code (Anthropic subscription)
- **Credential:** read the Claude Code CLI's OAuth token, in order: (1) file
  `~/.claude/.credentials.json`; (2) macOS Keychain item service `"Claude Code-credentials"`
  (needs `security-framework` or `/usr/bin/security`; triggers OS prompts → **prefer the
  file**). Token is `sk-ant-oat…`, must have scope **`user:profile`** (not just
  `user:inference`). Refresh via `POST platform.claude.com/v1/oauth/token`.
- **Usage:** `GET api.anthropic.com/api/oauth/usage`. Headers: `Authorization: Bearer`,
  `anthropic-beta: oauth-2025-04-20`, **`User-Agent: claude-code/<version>`** (load-bearing —
  without it you get persistent 429). Map: `five_hour`→session, `seven_day`→weekly,
  `seven_day_opus`/`_sonnet`→model-specific, `extra_usage`→monthly spend.
- **Risk:** endpoint **aggressively rate-limited (429)** (anthropics/claude-code#31637).
  Must implement a **rate-limit gate** (backoff/block-until) and send the magic User-Agent.

### Notes on Cursor / Copilot / Gemini (not in v1, researched for the roadmap)
- **Copilot** — *Easiest of everything.* Standard GitHub **device flow**
  (`client_id Iv1.b507a08c87ecfe98`, scope `read:user`), then
  `GET api.github.com/copilot_internal/user` with VS-Code-spoofing headers. No cookies, no
  Keychain. **Strong candidate to add right after v1.**
- **Cursor** — *Hard/Risky.* **Cookie-only** (no OAuth): needs `WorkosCursorSessionToken`
  etc. from the browser, then unofficial `cursor.com/api/usage-summary`. Requires the full
  cookie-extraction subsystem. Defer until cookies are solved.
- **Gemini** — *Hard.* Reads `~/.gemini/oauth_creds.json` but the OAuth **client secret is
  not in the file** — CodexBar regex-scrapes it out of the installed `gemini` CLI's bundled
  JS, then calls private `cloudcode-pa.googleapis.com/v1internal:*`. Most fragile mechanism
  in the reference app. Defer.

---

## API-cost providers (billing)

### OpenRouter — *the easy API win*
- **Auth:** normal `sk-or-v1…` key, `Authorization: Bearer`. No admin key.
  (Caveat: docs now mark `/credits` "Management key required"; historically a plain key
  works — verify at build, fall back to `/key`'s `limit`/`usage` for balance.)
- **Endpoints:** `GET /api/v1/key` → `limit, usage, usage_daily/weekly/monthly,
  limit_remaining, is_free_tier`; `GET /api/v1/credits` → `total_credits, total_usage`
  (balance = diff). `GET /api/v1/generation?id=` for per-call cost (optional).
- **Why easy:** pre-aggregated daily/weekly/monthly spend — **no time-bucketing, no
  pagination**. Build your own history by snapshotting `/key` over time for charts.

### Anthropic API — Admin-key-gated
- **Auth:** **Admin key `sk-ant-admin…`** only (org-scoped, individuals can't get one).
  Header: `x-api-key` + `anthropic-version: 2023-06-01`.
- **Endpoints:** `GET /v1/organizations/cost_report` (USD, `1d` bucket only) and
  `/v1/organizations/usage_report/messages` (tokens, `1m/1h/1d`). Caps: `1d` ≤ 31 buckets →
  fix `bucket_width=1d, limit=31` for a rolling ~30-day window. Data lag ~5 min; poll ≤1/min.
- **Challenge:** the key, not the code. UI must detect "normal key → no API cost data."

### OpenAI API — Admin-key-preferred, with legacy fallback
- **Auth:** preferred **Admin key `sk-admin…`** (`OPENAI_ADMIN_KEY`) → `/v1/organization/*`.
  Normal key → only undocumented legacy `GET /v1/dashboard/billing/credit_grants` (balance,
  best-effort, can break). `Authorization: Bearer`.
- **Endpoints:** `GET /v1/organization/costs` (`group_by=line_item`, `1d` only) and
  `/v1/organization/usage/completions` (`group_by=model`). Times are **Unix seconds**
  (differs from Anthropic's RFC3339). ≤31 buckets/request → **chunk ranges >31 days and
  stitch** (real complexity to replicate).
- **Challenge:** admin-key friction + time-bucket chunking + flaky legacy fallback.

### Azure OpenAI / AWS Bedrock (out of scope, noted)
Both require **cloud billing APIs** (Azure Cost Management / AWS Cost Explorer `GetCostAndUsage`
via SigV4 + IAM), ~24h lag, heavy auth. **Hard — defer** unless specifically required.

---

## Cross-cutting mechanisms to port (validated against our architecture)

CodexBar's shared machinery maps almost 1:1 onto our hexagonal design and *refines* our
provider contract. Key adoptions:

### Strategy pipeline with ordered fallback (refines [ADR 0005](../adr/0005-provider-trait-blocks.md))
A provider is **not** one `fetch_windows` — it's an **ordered list of typed auth strategies**
tried until one succeeds:
```rust
enum FetchKind { Cli, OAuth, Cookie, ApiToken, LocalProbe, WebDashboard }
trait FetchStrategy {              // one per credential path
    fn kind(&self) -> FetchKind;
    async fn is_available(&self, ctx: &FetchContext) -> bool;     // skip if creds absent
    async fn fetch(&self, ctx: &FetchContext) -> Result<UsageSnapshot, FetchError>;
    fn should_fallback(&self, err: &FetchError) -> bool;          // advance or bail
}
// Pipeline runs strategies in order, records Vec<Attempt> for diagnostics.
```
E.g. Claude Code = `[OAuthFromFile, OAuthFromKeychain, CookieWeb, CliScrape]`. This is the
"capability blocks" idea from ADR 0005 made concrete as an ordered chain.

### Normalized usage model (improves on the reference)
CodexBar uses *positional* `primary/secondary/tertiary: RateWindow` + label strings — a
corner we will **not** cut. We keep our **typed** `Vec<UsageWindow { kind: Session|Weekly|
Monthly|Custom, used_pct, window, resets_at, .. }>`. Adopt their `RateWindow` fields:
`used_percent`, `window_minutes`, `resets_at`, `reset_description`, `next_regen_percent`.

### Resilience patterns (these are requirements, not nice-to-haves)
Every one of these earned its place in CodexBar; bake them into `core`:
- **Lossy/optional decoding** — a malformed extra field never breaks the whole snapshot.
- **Reset-time backfill** — carry a cached `resets_at` forward when a fetch omits it.
- **Consecutive-failure gate** — suppress the *first* failure if prior data exists; surface
  errors only on the 2nd+ consecutive failure. Prevents flicker on one network blip.
- **Per-probe timeout** — race each fetch against a timeout; one slow provider never stalls
  the popover.
- **Startup connectivity retry** — backoff-retry on launch ("app opened before Wi-Fi up").
- **Rate-limit gate** — per-provider backoff/block-until (mandatory for Claude's 429s).
- **Status/staleness states** — `Ok | Stale | Error`, never a hard crash.

### Credential & cookie infrastructure
- **"Read the vendor CLI's credentials" is the dominant pattern** for subscription
  providers — prefer the plain file over the Keychain everywhere possible.
- **Browser cookies** (needed for Cursor and others, not v1): decryption = `rusqlite` +
  PBKDF2(`"saltysalt"`)→AES-128-CBC(IV=16×0x20) for Chromium, binarycookies parser for
  Safari, plain sqlite for Firefox; Safe-Storage password from the OS secret store per-OS.
  **Fastest correct route: a Swift sidecar wrapping CodexBar's `SweetCookieKit`** (per
  [ADR 0008](../adr/0008-native-sidecars.md)); pure-Rust port (crate `rookie`-style) is a
  later option. Keep the gating/cooldown/normalizer/cache **in Rust** regardless.
- **Keychain prompt avoidance** — port the *pattern* in Rust over `keyring`:
  **preflight (no-UI query) → explain-then-prompt → cooldown on denial → global disable
  switch → cache reads**. The native no-UI read itself (macOS) goes in the sidecar.

## Risk & ToS

All subscription usage endpoints (`wham/usage`, `api/oauth/usage`, `copilot_internal/user`,
`cursor.com/api/*`, `cloudcode-pa v1internal`) are **private and reached by spoofing an
official client's identity headers**. Consequences we accept and design around:
- Endpoints/schemas can change without notice → lossy decoding + fixtures + graceful errors.
- Aggressive rate-limiting (Claude) → rate-limit gates.
- Possible ToS friction with reusing sessions/spoofing clients → prefer official OAuth/API
  where it exists (Codex/Claude read the vendor's *own* OAuth token, which is the gentlest
  form); document the tradeoff; per-source consent ([ADR 0012](../adr/0012-consent-model.md)).
This risk is inherent to the product concept (CodexBar lives with it); it is a *known,
managed* risk, recorded here so it's never a surprise.
