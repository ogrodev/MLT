# Quality Gates ("Evals")

> In MLT, "evals" means a **deterministic, automated quality pipeline** that fails CI on
> smelly or AI-sloppy code. There is no LLM in the product, so there is no model-output
> scoring harness. See [ADR 0007](./adr/0007-quality-gates.md). An optional LLM-reviewer
> pass is a documented future add-on, not the floor.

Everything here runs in CI on every PR and is reproducible locally via `make check`.
A PR cannot merge unless every gate is green. Gates are *fast and deterministic* so they
never become flaky or ignored.

> **Status (2026-05-31): wired and green.** Implemented via `Makefile` (`make check`),
> `.github/workflows/ci.yml` (3 jobs: Rust, Frontend, Secret-scan), `lefthook.yml`
> (pre-commit/commit-msg/pre-push — install with `make hooks`), `deny.toml`, `biome.json`,
> and `scripts/check-core-purity.sh`. Config files: `cargo-deny` advisories use
> `unmaintained = "workspace"` (transitive GTK3 deps from tauri can't be fixed by us);
> `mlt-core` line coverage floor is **80%** (currently ~86%). A Claude `PostToolUse` hook
> (`.claude/settings.json`) auto-formats Rust on edit.

## 1. Rust gates

| Gate | Tool | Rule |
|------|------|------|
| Format | `cargo fmt --check` | zero diffs |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | **deny all warnings** |
| Tests | `cargo test --workspace` | all pass |
| Coverage | `cargo llvm-cov` | **≥ 80% on `crates/core`** (logic); adapters lower floor |
| Deps: advisories | `cargo deny check advisories` | no known CVEs |
| Deps: licenses | `cargo deny check licenses` | allowlist only (no GPL surprises) |
| Deps: bans | `cargo deny check bans` | no duplicate/banned crates |
| Unused deps | `cargo machete` | no unused dependencies |
| SQL correctness | `cargo sqlx prepare --check` | `.sqlx` offline cache matches schema |
| Docs | `cargo doc --no-deps` | builds without warnings |

## 2. Architecture fitness (the anti-slop core rule)

The hexagonal boundary is enforced by automated checks, not honor system:

- **`crates/core` may not depend on IO crates.** A CI step asserts `core`'s dependency
  tree excludes `reqwest`, `tokio` (fs/net features), `sqlx`, `keyring`, `std::fs`,
  and direct `SystemTime::now`/`Instant::now`/`Date` usage. Implemented via
  `cargo-deny` bans scoped to the core crate + a `grep`-based forbidden-API check.
- **Time is injected.** A lint forbids `SystemTime::now()` / `Instant::now()` outside
  the `SystemClock` adapter. Tests use a fake `Clock`.
- **Ports own all IO.** New IO must enter through a trait in `core::ports`, not a
  concrete call buried in logic.
- **No `unwrap()`/`expect()`/`panic!` in core or adapters** (except tests) —
  clippy `disallowed-methods` + `unwrap_used`/`expect_used` lints set to deny.

## 3. Complexity & duplication limits

| Gate | Tool | Rule |
|------|------|------|
| Cyclomatic complexity | clippy `cognitive_complexity` (+ `rust-code-analysis`) | flag functions over threshold |
| Duplication | `jscpd` (UI) + manual review for Rust | fail on copy-paste blocks above threshold |
| File/function size | custom lint | warn past soft caps; forces decomposition |
| Dead code | `cargo +nightly udeps` / clippy `dead_code` deny | no orphaned code |
| TODO/FIXME/HACK | `grep` gate | none on `main` without a linked issue |

## 4. Front-end (Svelte/TS) gates

| Gate | Tool | Rule |
|------|------|------|
| Format + lint | Biome | zero issues |
| Types | `tsc --noEmit` (strict) | no `any`, strict null checks |
| Svelte check | `svelte-check` | no warnings |
| Generated bindings | `tauri-specta` | `ui/src/bindings/` regenerated & committed; CI fails if stale |
| Unit tests | Vitest | pass |
| A11y | `eslint-plugin-svelte-a11y` / axe smoke | no violations on popover |
| Bundle budget | size check | popover JS under budget (Svelte makes this easy) |

## 5. Cross-cutting

- **Conventional Commits** + PR title lint → enables automated changelog/versioning.
- **Pre-commit hooks** (lefthook) run fmt + clippy + biome on staged files so most gates
  pass before push.
- **CODEOWNERS** + required review on `crates/core` and `docs/adr/` (architecture is
  protected; changing a port or an ADR needs deliberate sign-off).
- **PR template** requires: which ADR(s) this touches, what fixtures were added, and a
  checkbox confirming no secrets in code/logs.
- **Secret scanning** (gitleaks) on every push — no API keys/tokens committed.

## 6. CI matrix

GitHub Actions, three OS lanes:

- **ubuntu-latest** — fast lane: all Rust + UI gates, core coverage, deny, fixtures.
- **macos-latest** — EventKit sidecar build/test, Safari cookie path, notarization dry-run, E2E smoke.
- **windows-latest** — DPAPI cookie path, login-item, code-sign dry-run, E2E smoke.

Core logic + provider contract tests run on all three to catch platform-specific
surprises, but the expensive native lanes only run what's OS-specific.

## 7. Optional future add-on (NOT a merge gate today)

An **LLM-as-judge** step could review each diff against a rubric (over-abstraction,
hallucinated APIs, architecture-rule violations, missing fixtures) and post advisory
comments. Recorded here so the door is open; deliberately *not* a blocking gate to avoid
non-determinism and API cost in the critical path. Would require its own ADR.
