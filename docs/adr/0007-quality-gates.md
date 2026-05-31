# 0007 — "Evals" = deterministic quality gates

**Status:** Accepted · **Date:** 2026-05-30

## Context
The brief asked for "proper evals" to prevent AI slop. "Evals" normally means scoring LLM
outputs against a dataset — but this product has no LLM feature; it *tracks* AI usage, it
doesn't *call* models. We had to define what "evals" means here.

## Decision
"Evals" = a **strict, deterministic, automated quality pipeline** that fails CI on smelly
or AI-sloppy code: `clippy -D warnings`, `fmt --check`, coverage floor (≥80% on `core`),
`cargo-deny` (advisories/licenses/bans), complexity & duplication limits, dead-code/TODO
gates, plus Biome + strict `tsc` + `svelte-check` on the UI, and **architecture-fitness
checks** that forbid IO crates in `core`. Full spec in [QUALITY_GATES.md](../QUALITY_GATES.md).

## Alternatives considered
- **LLM-as-judge on every diff** — catches subjective slop linters miss, but adds API cost
  and non-determinism to the merge critical path.
- **Product LLM-eval harness** — only relevant if we add an in-product AI feature; none planned.
- **Both** — strongest but most expensive.

## Consequences
- **+** Deterministic, fast, free, no flakiness in the merge path.
- **+** Architecture-fitness checks make the hexagonal boundary self-enforcing.
- **−** Won't catch subjective "this abstraction is wrong" smells a human/LLM might.
- Door left open: an **advisory** (non-blocking) LLM-reviewer pass is documented as a
  future add-on requiring its own ADR.
