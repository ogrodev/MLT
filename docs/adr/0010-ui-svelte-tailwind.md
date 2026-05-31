# 0010 — UI: Svelte + Tailwind + tauri-specta

**Status:** Accepted · **Date:** 2026-05-30

## Context
The popover is a small, dense, frequently-updating surface (usage bars, countdowns,
lists) inside Tauri's system webview. We need a stack that's lean, reactive, and supports
a consistent design system, while keeping the Rust↔UI boundary type-safe.

## Decision
**Svelte + Tailwind.** Svelte compiles to tiny vanilla JS with ~no runtime — ideal for an
always-on popover with real-time countdowns. Tailwind provides design tokens for a
consistent system. The Rust↔UI seam is **type-checked, not hand-synced**: TypeScript
bindings are generated from Rust types via **`tauri-specta`** (committed to
`ui/src/bindings/`, CI fails if stale).

## Alternatives considered
- **React + Tailwind + shadcn/ui** — biggest ecosystem, easiest hiring, polished components
  fast, but heavier runtime. Safe default if libraries/contributor familiarity outweigh footprint.
- **SolidJS + Tailwind** — near-vanilla perf, React-like DX, but smallest ecosystem.

## Consequences
- **+** Lean bundle, fast updates, easy bundle-budget gate.
- **+** Generated bindings eliminate a whole class of boundary-drift slop.
- **−** Smaller component ecosystem than React; rarely bites given the small surface.
- State of record lives in Rust; UI subscribes via Tauri events and calls Tauri commands.
