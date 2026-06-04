# 007 — OpenAI API usage (honest limits)

> Connect my OpenAI API key and see usage — or an honest note when a solo key can't expose it.

**Capability:** [§3 Provider coverage](../PRD.md#3-provider-coverage) · **Status:** ✅ done · **Depends on:** 003

## User story
As an OpenAI API user, I want my usage shown when possible, and a truthful explanation when
my key can't access it, so I'm never misled by a fake zero.

## Scope
- **In:** Reading OpenAI usage with the key entered via 003; the honest-limitation state when
  usage requires an org/admin key.
- **Out:** Key-entry UI (003); other API providers (006, 008).

## Acceptance criteria
- [x] When usage **is** retrievable, OpenAI shows it and auto-refreshes with the rest — as a
      **percent-used** window where a spend quota exists, else an **honest spend figure** (these
      cost APIs expose no quota to render as a percentage).
- [x] When usage **cannot** be exposed with the user's key (e.g. needs an org admin key), the
      tile states this **honestly** — **not zero or a misleading value**.
- [x] Fetch failure shows a **stale/error** state and retains last known values; other
      providers are unaffected.
- [x] OpenAI data stays **siloed** to its own tile.

## Done
Meets the [shared Definition of Done](./README.md#shared-definition-of-done-applies-to-every-task).
Tests cover both the "usage available" and "honest limitation" paths via fixtures.

## References
- [ADR 0014 — v1 provider set](../adr/0014-v1-provider-set.md) · [research/PROVIDERS.md](../research/PROVIDERS.md) · [OPEN_QUESTIONS D4](../OPEN_QUESTIONS.md)
