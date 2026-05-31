# Architecture Decision Records

Each ADR captures one decision: its context, the choice, the alternatives weighed, and
the consequences accepted. ADRs are immutable once accepted — to change a decision, write
a new ADR that supersedes the old one (and mark the old one `Superseded by NNNN`).

Format: lightweight [MADR](https://adr.github.io/madr/)-style.

| # | Title | Status |
|---|-------|--------|
| [0001](./0001-runtime-tauri.md) | Runtime: Tauri (Rust + system webview) | Accepted |
| [0002](./0002-local-only-topology.md) | Local-only topology + OS login-item | Accepted |
| [0003](./0003-usage-source-polling.md) | Usage via polling + multi-strategy credential layer | Accepted |
| [0004](./0004-scope.md) | Scope: usage + alarms + read-only calendar | Accepted |
| [0005](./0005-provider-trait-blocks.md) | Provider trait + capability building-blocks | Accepted |
| [0006](./0006-hexagonal-core.md) | Hexagonal core (ports & adapters) | Accepted |
| [0007](./0007-quality-gates.md) | "Evals" = deterministic quality gates | Accepted |
| [0008](./0008-native-sidecars.md) | Native code via sidecar helper processes | Accepted |
| [0009](./0009-alarm-engine.md) | Persisted alarms + wake/launch catch-up | Accepted |
| [0010](./0010-ui-svelte-tailwind.md) | UI: Svelte + Tailwind + tauri-specta | Accepted |
| [0011](./0011-data-layer-sqlx.md) | Data layer: sqlx (compile-time-checked SQL) | Accepted |
| [0012](./0012-consent-model.md) | Consent: metadata-only discovery → per-source opt-in | Accepted |
| [0013](./0013-single-tray-icon.md) | Single tray icon, CodexBar-style v0.1 | Accepted |
| [0014](./0014-v1-provider-set.md) | v1 provider set (5 providers, 2 categories) | Accepted |
| [0015](./0015-resilience-patterns.md) | Provider resilience patterns are core requirements | Accepted |
