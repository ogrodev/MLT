# 006 — OpenRouter API usage

> Add my OpenRouter API key and see my credit/usage standing.

**Capability:** [§3 Provider coverage](../PRD.md#3-provider-coverage) · **Status:** ✅ done · **Depends on:** 003

## User story
As an OpenRouter user, I want my API usage/credit shown in the popover so I know how much
headroom I have left.

## Scope
- **In:** Reading OpenRouter usage with the key entered via 003 and rendering normalized windows.
- **Out:** Key-entry UI itself (003); other API providers (007, 008).

## Acceptance criteria
- [x] Once a key is connected, OpenRouter shows its **usage/credit** with **percent used**
      (or remaining) and a **reset countdown** where one applies, per §2.
- [x] Data **auto-refreshes** with the rest and on opening the popover.
- [x] Fetch failure shows a **stale/error** state and retains last known values; other
      providers are unaffected.
- [x] Where OpenRouter exposes **no reset window** (e.g. prepaid credit), the UI states that
      **honestly** rather than inventing a countdown.
- [x] OpenRouter data stays **siloed** to its own tile.

## Done
Meets the [shared Definition of Done](./README.md#shared-definition-of-done-applies-to-every-task).
Tests use recorded fixtures; a live check is an `[ignore]`d example run by hand.

## References
- [ADR 0014 — v1 provider set](../adr/0014-v1-provider-set.md) · [research/PROVIDERS.md](../research/PROVIDERS.md)
