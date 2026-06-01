# MLT — agent & contributor guide

Cross-platform menu-bar/tray app: AI-provider usage tracking + alarms + read-only calendar.
Tauri (Rust core + Svelte/SvelteKit webview), local-first. Product spec + acceptance criteria
in `docs/PRD.md`; full design in `docs/ARCHITECTURE.md` and `docs/adr/`; provider details in
`docs/research/PROVIDERS.md`.

## Layout
- `crates/core` — **pure** domain + ports. **Zero IO** (see invariants).
- `crates/adapters` — concrete port impls; IO lives here (http, keychain, credentials…).
- `src-tauri` — the Tauri app crate (`mlt`): wiring + commands.
- `src/` — SvelteKit + TypeScript frontend.

## Commands
- `make check` — run all quality gates (matches CI). Individual: `make fmt lint test deny purity ui-lint ui-check coverage`.
- `make hooks` — install git hooks (lefthook). `cargo test --workspace` — tests.
- `make deps` — install/refresh deps **through Socket Firewall** (`sfw`), which blocks
  confirmed-malware packages at fetch (cargo + pnpm). Prefer it over bare `cargo fetch` / `pnpm install`.
- Live Claude check: `cargo run -p mlt-adapters --example claude_live`.
- `make qa` — build + install + launch the app on this Mac for manual QA (real menu-bar app;
  `make qa-release` for a production-like build). Also available as the `/qa-build` skill.

## Invariants (enforced by gates — see `docs/QUALITY_GATES.md`)
- **`crates/core` performs no IO.** No `reqwest`/`sqlx`/`keyring`/`std::fs`/`std::net`, and
  no `SystemTime::now()` — time comes from the `Clock` port. (`make purity` enforces this.)
- **Secrets live in the OS keychain only** — never in the DB, never in logs.
- **Never write back to a vendor's own credential store** (e.g. Claude Code's keychain item);
  cache our refreshed copies under our own service (`com.bigshotpictures.mlt`).
- **Tests use fakes/fixtures** — never hit real provider accounts or trigger Keychain prompts
  in `cargo test`. Live checks are `[ignore]`d examples, run by hand.
- **Provider data is siloed** — never render one provider's identity/plan under another.
- **Resilience is mandatory** (ADR 0015): lossy decoding, reset-time backfill, failure-gate,
  per-probe timeout, rate-limit gate, explicit `Ok|Stale|Error` — never panic on bad upstream data.
- Commit messages follow **Conventional Commits**. A `PostToolUse` hook auto-formats Rust on edit.

## Toolchain note
`cargo`/`rustc` are Homebrew here; `cargo fmt`/`cargo clippy` can hit stale `~/.cargo/bin`
rustup shims, so the Makefile/hooks call the rustc-adjacent `cargo-fmt`/`cargo-clippy`
directly. CI uses a clean rustup toolchain, so plain `cargo fmt`/`cargo clippy` work there.
