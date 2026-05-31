# Human Prerequisites

Everything this project needs that **I (the agent) genuinely cannot do** and that requires
**you, a human**. This is deliberately strict.

### What counts as "not doable by me"
An item lands here only if it requires at least one of:
- **(P) Payment** or a billing relationship.
- **(I) Legal identity / entity** (your name, org, D-U-N-S, tax info).
- **(A) Accepting a third-party legal agreement** as a legal person (ToS, DPA, program terms).
- **(G) A GUI / OS permission grant on a physical machine** — macOS TCC prompts, Gatekeeper,
  Windows SmartScreen, Keychain "Always Allow". These are SIP/TCC-protected and **cannot be
  scripted or clicked headlessly**; a human must approve them at the machine.
- **(S) Possession of a secret/credential** I must never hold (signing keys, API tokens).
- **(R) Third-party human review** (e.g. Google OAuth verification, CA validation).

For everything below I note: the blocker code(s), **what I can prepare for you**, and rough
**cost / lead time**. Items marked ⛔ are hard blockers for shipping; ⚠️ block a feature; ℹ️
are decisions only you can make.

---

## 1. Code signing & distribution (⛔ blocks any public release)

| # | Item | Blockers | What I can prepare | Cost / lead time |
|---|------|----------|--------------------|------------------|
| H1 | **Apple Developer Program** enrollment | P, I, A | the `tauri.conf` signing config, the notarization CI step (waiting on your secrets) | $99/yr; hours–days (identity check) |
| H2 | **Apple signing assets**: Developer ID Application cert, App Store Connect API key (or app-specific password) for notarization | S, I | the workflow that consumes them; instructions to generate | included in H1; minutes once enrolled |
| H3 | **Windows code-signing certificate** (OV or EV) from a CA (DigiCert/Sectigo/…) | P, I, A, S | the signing step in CI; guidance on OV vs EV | OV ~$200–400/yr; **EV** ships on a **physical HSM/USB token** (R, hardware) and clears SmartScreen faster |
| H4 | Decide signing-secret storage = GitHub Actions secrets, and **paste the secret values** | S | the workflow references (`${{ secrets.* }}`); I never see the values | minutes |

> Note: I can build, lint, and produce *unsigned* artifacts all day. Signing + notarization
> are the wall I cannot climb without H1–H4.

## 2. OAuth app registrations (⚠️ blocks calendar + cleanest provider auth)

Each requires a human with an account on that platform to accept terms and (for sensitive
scopes) pass review. I can write all the client code and the redirect/PKCE handling.

| # | Item | Blockers | What I can prepare | Notes |
|---|------|----------|--------------------|-------|
| H5 | **Google Cloud project** + enable Calendar API + OAuth consent screen + Desktop/loopback client ID | I, A | the loopback OAuth+PKCE flow, scope list (`calendar.readonly`) | client_id is not secret for desktop loopback; you create it |
| H6 | **Google OAuth verification** for the sensitive `calendar.readonly` scope (if distributing beyond test users) | A, R | privacy policy text draft, scope justification draft | **Google human review**; needs published privacy policy + domain; can take days–weeks. Until then: "unverified app" screen + ≤100 test users |
| H7 | **Microsoft Entra ID (Azure AD)** app registration for Graph calendar read | I, A | Graph client code, scopes | publisher verification optional but recommended |
| H8 | **Publish a Privacy Policy + Terms** at a public URL | I, A | full draft text (I can write it; you own/approve/host it) | required by H6 and most stores; a legal/business artifact |

## 3. Provider accounts, keys & subscriptions (⚠️ feature data depends on these)

I cannot create accounts, subscribe, or mint keys for you. See `docs/research/PROVIDERS.md`
for which key each provider needs.

| # | Item | Blockers | Reality |
|---|------|----------|---------|
| H9 | **Anthropic org Admin API key** (`sk-ant-admin…`) for API cost tracking | P, I, A, S | org-owner only; **unavailable to individual accounts** — if you're solo, this feature simply has no data |
| H10 | **OpenAI org Admin key** (`sk-admin…`) for API cost | P, I, A, S | org-owner only; normal key gives only a flaky legacy balance |
| H11 | **OpenRouter account + API key** (`sk-or-v1…`) | P, A, S | normal key is enough — the easy win |
| H12 | Active **Claude Code** and **Codex** subscriptions, with their CLIs installed and **logged in** on the test machine | P, A, G | credential-reuse reads the CLIs' own tokens; without a logged-in CLI there is nothing to read |

## 4. OS permission grants on YOUR machine (⛔ runtime features; (G) = un-scriptable)

These are TCC/Gatekeeper/SmartScreen prompts. They **must be clicked by a human physically
at the machine** — no API, script, or headless agent can grant them (by OS design).

| # | Item | OS | Blocker |
|---|------|----|---------|
| H13 | Approve the app past **Gatekeeper** on first launch ("Open anyway") | macOS | G |
| H14 | Grant **Notifications** permission | mac/win | G |
| H15 | Grant **Calendar / EventKit** access prompt | macOS | G |
| H16 | Grant **Full Disk Access** (needed to read Safari cookies, if/when that provider lands) | macOS | G |
| H17 | Approve **Keychain** read prompts ("Always Allow") for reused credentials | macOS | G |
| H18 | Dismiss **SmartScreen** / grant notifications on first run | Windows | G |

> Mitigation I *can* build: the in-app onboarding that explains each prompt before it
> appears (the CodexBar "preflight → explain → then prompt" pattern). I can't pre-grant them.

## 5. Repo / CI / infra administration (mixed — some I can do via `gh`)

| # | Item | Blockers | What I can / can't do |
|---|------|----------|------------------------|
| H19 | Create the **GitHub repo/org**, set billing, enable Actions minutes | P (org), I | I can scaffold `.github/` configs; you own repo creation + billing |
| H20 | Add **branch protection** + required checks | — | I can draft; applying needs repo-admin (you, or grant me a token with rights) |
| H21 | Provision **CI secrets** (signing keys, notarization key, any tokens) | S | I write the workflow; you paste the secret values |
| H22 | **macOS CI runner** access (required for EventKit + Safari + notarization) | P | GitHub-hosted `macos-latest` works; self-hosted needs your hardware |
| H23 | (Optional) **Custom domain** for privacy policy / update feed | P, I | GitHub Releases needs none; a domain needs purchase |

## 6. Decisions only you can make (ℹ️)

| # | Decision | Why it's yours |
|---|----------|----------------|
| H24 | **Open-source license** (and whether the repo is public) | legal/business choice; affects `cargo-deny` allowlist |
| H25 | **Accept the ToS risk** of reusing sessions / calling private provider endpoints | see `docs/research/PROVIDERS.md` §Risk — a risk-acceptance only you can own |
| H26 | **Product name / branding** (current working title: "MLT") | branding/trademark |
| H27 | **Telemetry stance** — confirm opt-in-only crash reporting and exactly what may leave the device | privacy/legal posture |

---

### Summary: the critical path to a shippable signed build
**H1 → H2** (Apple) and **H3 → H4** (Windows) are the gating items for distribution, and
**H1** has identity-verification lead time — **start it first**. **H11/H12** unblock the most
valuable provider data immediately (OpenRouter key + logged-in Codex/Claude CLIs). **H9/H10**
are optional and, for solo users, may never yield data. Everything else I can scaffold while
you work these.
