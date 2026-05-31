# Skills & Automation Research

**Date:** 2026-05-30 · **Purpose:** decide which Claude Code skills to *adopt*, which custom
project skills to *author*, and which *hooks* to wire so the QUALITY_GATES are enforced
automatically — all mapped to MLT's hexagonal architecture and anti-slop goal.

> Two reference points grounded this: CodexBar's own `AGENTS.md` (a similar project's agent
> playbook — build/test commands, lint gate, and a hard "never trigger a Keychain prompt in
> tests" rule) and Claude Code's skill/hook/plugin model.
>
> ⚠️ **Verify before wiring:** exact hook event names, frontmatter fields, and settings.json
> shape should be confirmed against current Claude Code docs (or set up via the
> **`/update-config`** skill, which writes `settings.json` correctly). Treat the JSON shapes
> below as intent, not gospel.

---

## A. Existing skills to adopt (available in this environment)

Ranked by value to *this* project. "When" = where it slots into the workflow.

| Skill | Value | When to use |
|-------|-------|-------------|
| **tdd** | ★★★★★ | The pure `core` logic + every provider contract test. Red-green-refactor fits our "fixtures + golden tests" model exactly. Default loop for logic work. |
| **security-review** | ★★★★★ | **Non-negotiable here** — we touch OAuth tokens, Keychain, browser cookies, secret storage. Run on every change that reads creds, writes secrets, or touches a sidecar. |
| **simplify** | ★★★★☆ | Directly serves "no AI slop": run on changed code to strip over-abstraction/duplication before commit. Pairs with the QUALITY_GATES complexity limits. |
| **code-review** | ★★★★☆ | Diff review for correctness bugs + reuse/efficiency. Run pre-PR alongside the deterministic gates. |
| **review** | ★★★★☆ | Standards-vs-Spec review. We *have* documented standards (QUALITY_GATES) + specs (ADRs), so this skill has real material to check against. |
| **grill-with-docs** | ★★★★☆ | Continue what we're doing now: stress-test future plans against the domain model and update ADRs/CONTEXT inline. The natural successor to `/grill-me` once docs exist. |
| **design-an-interface** | ★★★★☆ | "Design it twice" for the load-bearing port traits (`FetchStrategy`, `CalendarPort`, `SecretStore`) before committing to a shape. |
| **improve-codebase-architecture** | ★★★☆☆ | As the codebase grows: finds deepening/refactor opportunities informed by our ADRs. Periodic, not per-change. |
| **deep-research** | ★★★☆☆ | Periodic provider-endpoint drift checks + scoping new providers (Copilot/Cursor/Gemini). Pairs with `/schedule` (see §E). |
| **verify** + **run** | ★★★☆☆ | From v0.1: launch the Tauri app and confirm a change actually works (not just unit-green). |
| **impeccable** | ★★★☆☆ | Polish the Svelte popover UI (design-quality goal). Use when shaping the popover/usage-bar visuals. |
| **qa** | ★★☆☆☆ | Once shipping: conversational bug-reporting → GitHub issues. |
| **init** | ★★☆☆☆ | One-time, at scaffold: generate the first `CLAUDE.md`. |
| **update-config** | ★★★★☆ (enabling) | Use it to wire the hooks in §C and the permission allowlist correctly. |
| **fewer-permission-prompts** | ★★☆☆☆ | Dev ergonomics for this CC workflow after a few sessions. |
| **claude-api** | ☆ (deferred) | **Not for v1** — there's no in-product LLM ([ADR 0007](../adr/0007-quality-gates.md)). Only relevant if we ever add an AI feature. |

Skipped as not project-relevant: `keybindings-help`, `statusline-setup`.

## B. Custom project skills to author (the anti-slop spine)

These don't exist yet; authoring them is what makes "every provider is a uniform 50-line
unit" actually true. Live in `.claude/skills/<name>/SKILL.md`. Priority order:

| # | Skill | What it scaffolds | Why it matters |
|---|-------|-------------------|----------------|
| S1 | **`/add-provider <id>`** | A full provider: `ProviderDescriptor`, an ordered `FetchStrategy` chain composed from capability blocks, registry registration, a **recorded-HTTP fixture stub**, and a **golden-file contract test** — matching [ADR 0005](../adr/0005-provider-trait-blocks.md). | **Highest value.** Turns the provider contract into a generator, so 40+ providers stay uniform instead of becoming 40 snowflakes. This *is* the anti-slop guarantee, mechanized. |
| S2 | **`/add-fixture <provider>`** | Records/refreshes a provider's HTTP fixture from a sample response, **stripping secrets/PII**, and regenerates the golden file. | Keeps the contract tests honest and makes "I changed the parser" safe. Bakes in the secret-scrubbing rule. |
| S3 | **`/add-adapter <port>`** | A new port adapter: `impl` of the trait + an in-`core` **fake** + an integration test — preserving the hexagonal boundary ([ADR 0006](../adr/0006-hexagonal-core.md)). | Stops IO from leaking into `core`; every new port gets a fake for free, keeping logic testable. |
| S4 | **`/gate`** | Runs the full local QUALITY_GATES suite and summarizes only the failures (fmt, clippy -D, sqlx prepare --check, cargo-deny, biome, tsc, svelte-check, tests, coverage). | One command to reproduce CI locally. (Back it with a `justfile`/`Makefile` `check` target — see §D.) |
| S5 | **`/release`** | The CodexBar-style release flow: build → sign → notarize → update manifest (`tauri-plugin-updater`) → tag. | Author **after** the human signing prereqs (H1–H4) are met. Mirrors CodexBar's `.agents/skills/release-codexbar` skill. |
| S6 | **`/provider-drift`** | Re-runs targeted research/probes against v1 provider endpoints to detect schema/endpoint changes, opening issues for breakage. | The endpoints are private and drift ([PROVIDERS §Risk](./PROVIDERS.md#risk--tos)); this is early-warning. Schedule it (§E). |

## C. Hooks — deterministic enforcement of QUALITY_GATES ([ADR 0007](../adr/0007-quality-gates.md))

Hooks make the gates *agent-proof*: they run on lifecycle events regardless of what the
model remembers to do. Configure via `/update-config`. Intended set:

1. **Auto-format on edit** — `PostToolUse` on `Edit|Write`: run `cargo fmt` on changed `.rs`,
   `biome format --write` on changed `.ts`/`.svelte`. Quiet, non-blocking.
2. **Pre-commit gate** — `PreToolUse` matching `git commit`: run the *fast* gate (fmt check,
   `clippy -D warnings`, `cargo sqlx prepare --check`, `tsc`, `svelte-check`, `cargo test`).
   Block the commit on failure. (Belt-and-suspenders with a `lefthook` pre-commit hook for
   non-Claude commits.)
3. **Secret-safety gate** (project-specific, borrowed from CodexBar's AGENTS.md) —
   `PreToolUse` on `Bash`: **block any command that can pop an OS Keychain prompt or hit a
   real provider account during tests** (e.g. `security find-generic-password`, live
   `*-usage` probes) unless an env flag explicitly allows it. Tests must use fixtures/fakes.
4. **Dangerous-command guard** — `PreToolUse` on `Bash`: block destructive patterns
   (`rm -rf`, force-push to main) as a safety net.

Example intent (verify schema via `/update-config`):
```jsonc
{ "hooks": {
  "PostToolUse": [{ "matcher": "Edit|Write",
    "hooks": [{ "type": "command", "command": "${CLAUDE_PROJECT_DIR}/.claude/hooks/on-edit.sh" }] }],
  "PreToolUse": [{ "matcher": "Bash", "if": "Bash(git commit *)",
    "hooks": [{ "type": "command", "command": "${CLAUDE_PROJECT_DIR}/.claude/hooks/pre-commit.sh", "timeout": 120 }] }]
}}
```

## D. CLAUDE.md / AGENTS.md / rules structure

Per the brief, Claude Code reads **`CLAUDE.md`** (not `AGENTS.md`); if we keep an `AGENTS.md`
for other tools, import it via `@AGENTS.md` from `CLAUDE.md`. Plan:

- **Root `CLAUDE.md`** (<200 lines): build/test/run commands, workspace map, and the **hard
  invariants** that protect the architecture:
  - `core` has **no IO** (time via `Clock`, no `reqwest`/`fs`/`SystemTime::now`).
  - No `unwrap()/expect()/panic!` outside tests.
  - Secrets live in the keychain only — **never** in DB or logs.
  - **Never run tests/commands that trigger a Keychain prompt or hit real provider accounts**
    — use fixtures/fakes (CodexBar's rule, and ours).
  - All provider data is **siloed** — never render one provider's identity/plan under another
    (a real bug class flagged in CodexBar's AGENTS.md).
  - Pointers to `docs/ARCHITECTURE.md`, `docs/adr/`, `docs/QUALITY_GATES.md`.
- **Path-scoped `.claude/rules/`** (load only when touching matching files):
  - `crates/core/**` → purity rules, ports-only-IO.
  - `crates/.../providers/**` → the descriptor + strategy-chain + fixture + golden-test contract.
  - `adapters/**` → impl trait + provide a fake + integration test.
  - `ui/**` → Svelte/Tailwind conventions; **never hand-edit generated `bindings/`**.
- **Per-crate `CLAUDE.md`** (50–100 lines) for `core`, `app`, `ui` as they grow.

## E. Automation (optional, interesting)

- **`/schedule`** a recurring `/provider-drift` (S6) run (e.g. weekly) so private-endpoint
  breakage is caught before users report it. Endpoints drift; this turns a reactive fire into
  a proactive ticket.
- **Plugin packaging (later):** once `/add-provider`, `/add-adapter`, and `/release` are
  stable, package them as a **plugin** (`.claude-plugin/plugin.json`) so the same scaffolds
  are reusable if the project ever splits into multiple repos. Not needed for a single repo now.

## F. Bottom line

- **Adopt now:** `tdd`, `security-review`, `simplify`, `code-review` as the per-change loop;
  `grill-with-docs` + `design-an-interface` for upcoming design decisions; `update-config` to
  wire hooks.
- **Author first:** **`/add-provider`** (S1) — it's the single highest-leverage anti-slop
  investment, because it makes the uniform provider contract a generator rather than a hope.
- **Wire the hooks** in §C so fmt/clippy/biome and the secret-safety rule are enforced
  automatically, not remembered.
- These complement — they don't replace — the deterministic CI gates in `QUALITY_GATES.md`.
