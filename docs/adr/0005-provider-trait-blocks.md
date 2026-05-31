# 0005 — Provider trait + capability building-blocks

**Status:** Accepted · **Date:** 2026-05-30

## Context
With 40+ usage sources (plus calendar sources later), the way a provider is *added* is the
main defense against divergent, sloppy code. We need adding a provider to be "fill in a
small, typed, uniform unit," not "invent something new each time."

## Decision
One **`Provider` trait** (`id`, `metadata`, `credential_strategy`, `fetch_windows`)
implemented by each provider, composed from **reusable capability blocks**: `OAuthFlow`,
`DeviceFlow`, `ApiKeyAuth`, `CookieSource`, `CliCredSource`/`JsonlLogSource`,
`HttpFetcher`, `UsageParser`. Common REST providers assemble blocks (~50 lines); bespoke
providers override `fetch_windows`. **Every provider ships a recorded-HTTP fixture + a
golden-file test.**

## Alternatives considered
- **Declarative manifests (TOML/JSON)** — zero code per provider, beautiful for uniform
  REST, but can't express bespoke auth (cookie decryption, device flow, JSONL parsing)
  without escape hatches.
- **Hybrid manifest + trait escape hatch** — max coverage but two mental models to maintain.
- **WASM/dynamic plugins** — runtime-loaded community plugins; heavy infra (ABI, sandbox,
  signing). Overkill until a third-party ecosystem is a goal.

## Refinement (post-research, 2026-05-30)
The CodexBar study ([research/PROVIDERS.md](../research/PROVIDERS.md)) refined the trait into
an **ordered fallback chain of typed `FetchStrategy` units** rather than a single
`fetch_windows`. Each strategy is one credential path (`Cli | OAuth | Cookie | ApiToken |
LocalProbe | WebDashboard`) with `is_available` / `fetch` / `should_fallback`; a generic
pipeline runs them in order and records attempts for diagnostics. The capability blocks of
this ADR are the reusable parts those strategies compose from. Example:
`ClaudeCode = [OAuthFromFile, OAuthFromKeychain, CookieWeb, CliScrape]`.

## Consequences
- **+** Uniform, testable, small provider units; the fixture + golden test is an
  enforceable contract (QUALITY_GATES §"Provider contract tests").
- **+** Same trait/pipeline reused for calendar adapters and credential sources.
- **+** Ordered fallback chain mirrors how real providers expose multiple credential paths.
- **−** Slightly more upfront framework design than a pure manifest engine.
- Door left open: a manifest layer or WASM plugins can be added later atop the trait.
