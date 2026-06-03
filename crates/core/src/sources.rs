//! Local source discovery + consent (ADR 0012, PRD §4/§9).
//!
//! A *source* is a place we can reuse an existing login from — a vendor CLI's credentials,
//! a browser profile, … Discovery is **metadata-only**: we present what a machine *could*
//! connect to, with a plain-language note of what each accesses and why, and read **nothing**
//! until the user opts that source in. The two rules that make this safe live here, pure and
//! unit-tested: presence detection never touches a secret (enforced in the adapter), and a
//! source is read only when it is both **present** and **enabled** ([`SourceState::active`]).
use crate::domain::{AccountIdentity, ProviderId};
use crate::ports::{ConsentStore, IdentityStore, PortError, SourceLabels, SourceProbe};
use serde::{Deserialize, Serialize};

/// How a source supplies its credential — this decides its connect-screen affordance and its
/// consent semantics. A `LocalLogin` source reuses a login discovered on the machine (a
/// presence check plus an opt-in toggle); an `ApiKey` source has no local login to discover,
/// so the user pastes a key and storing a *validated* key is itself the act of connecting
/// (ADR 0016).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CredentialKind {
    /// Reuse a login already on this machine (e.g. a vendor CLI's OAuth token).
    LocalLogin,
    /// A user-entered API key, stored in our keychain (e.g. OpenRouter).
    ApiKey,
}

/// A source MLT knows how to discover, with the disclosure shown *before* opt-in. Static
/// data — the catalog ([`source_catalog`]) is the single place new sources are registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDescriptor {
    pub id: ProviderId,
    /// Human name for the connect screen, e.g. `"Claude Code"`.
    pub display_name: &'static str,
    /// Plain-language note: *what* is accessed and *why*, shown before the user opts in
    /// (PRD §4, ADR 0012). Must be honest about the credential read and where data goes.
    pub access_note: &'static str,
    /// How this source is connected (reuse a local login vs. enter an API key).
    pub credential: CredentialKind,
    /// Keychain entry (under MLT's *own* service) where we cache a refreshed copy of this
    /// source's reused OAuth login, or `None` for sources we never refresh. Purged on
    /// disconnect — it is OUR copy, never the vendor's own credential store, which MLT only
    /// ever reads (ADR 0012). API-key sources leave this `None`: their secret is the
    /// user-entered key at [`api_key_secret_key`].
    pub oauth_cache_key: Option<String>,
}

impl SourceDescriptor {
    /// Combine this descriptor with a machine's live presence and the user's consent into a
    /// row for the connect screen. Pure: presence/consent are gathered by adapters upstream.
    pub fn to_state(&self, present: bool, enabled: bool) -> SourceState {
        SourceState {
            id: self.id.clone(),
            display_name: self.display_name.to_string(),
            access_note: self.access_note.to_string(),
            present,
            enabled,
            credential: self.credential,
            label: None,
            account: None,
        }
    }

    /// Every keychain entry MLT itself wrote for this source, under our *own* service — the
    /// exact set to purge on disconnect. This is only what we *cache*: the user-entered API
    /// key for an [`CredentialKind::ApiKey`] source, plus any refreshed-OAuth copy
    /// ([`oauth_cache_key`](Self::oauth_cache_key)) for a reused login. It never includes the
    /// vendor's own credential store, which MLT only ever reads (ADR 0012/0016).
    pub fn cached_secret_keys(&self) -> Vec<String> {
        let mut keys = Vec::new();
        if self.credential == CredentialKind::ApiKey {
            keys.push(api_key_secret_key(&self.id));
        }
        if let Some(oauth) = &self.oauth_cache_key {
            keys.push(oauth.clone());
        }
        keys
    }
}

/// One row of the connect/sources screen: a known source plus its live presence and the
/// user's consent. Serialized across the Tauri boundary to the popover.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceState {
    pub id: ProviderId,
    pub display_name: String,
    pub access_note: String,
    /// Discovered locally via metadata only (a credentials file / Keychain item exists).
    pub present: bool,
    /// The user has opted this source in.
    pub enabled: bool,
    /// How this source is connected — drives both the connect-screen UI and [`active`].
    pub credential: CredentialKind,
    /// A user-assigned custom name (a nickname/title), shown as the panel *title* — distinct
    /// from `display_name` (the provider's own name) and from the auto-fetched `account`. It
    /// never replaces the provider name. `None` means no custom title. Populated by
    /// [`discover_sources`]; plays no part in [`active`] (a name is cosmetic, not consent).
    pub label: Option<String>,
    /// The provider-fetched account identity (email/org) for display, or `None` when not yet
    /// resolved. Cached via the identity store and siloed per source; never user-entered.
    pub account: Option<AccountIdentity>,
}

impl SourceState {
    /// Whether the app may read this source's credential. For a `LocalLogin` source this is
    /// the consent gate (ADR 0012): it must be both **discovered locally** and **opted in**.
    /// An `ApiKey` source has nothing to discover — a stored, validated key is the connection,
    /// so being `enabled` is sufficient (ADR 0016).
    pub fn active(&self) -> bool {
        match self.credential {
            CredentialKind::LocalLogin => self.present && self.enabled,
            CredentialKind::ApiKey => self.enabled,
        }
    }
}

/// Every source MLT can discover today. New providers register here (one line); the same
/// catalog drives the connect screen and the refresh loop's consent gate.
pub fn source_catalog() -> Vec<SourceDescriptor> {
    vec![
        SourceDescriptor {
            id: ProviderId::new("claude-code"),
            display_name: "Claude Code",
            access_note: "Reuses your Claude Code login — an OAuth token already on this Mac \
                          (in ~/.claude or your Keychain) — to read your Claude subscription \
                          usage. The token is never shown, never stored in MLT's database or \
                          logs, and is sent only to Anthropic.",
            credential: CredentialKind::LocalLogin,
            // Our refreshed-OAuth copy lives here; disconnect purges it. Never Claude Code's
            // own keychain item, which we only read.
            oauth_cache_key: Some(crate::providers::claude::CACHE_KEY.to_string()),
        },
        SourceDescriptor {
            id: ProviderId::new("openrouter"),
            display_name: "OpenRouter",
            access_note: "Uses an OpenRouter API key you paste in to read your API usage and \
                          credit balance. The key is stored only in your OS keychain — never \
                          shown again in full, never written to MLT's database or logs — and \
                          is sent only to OpenRouter.",
            credential: CredentialKind::ApiKey,
            oauth_cache_key: None,
        },
    ]
}

/// One OAuth login discovered for a multi-account provider (Codex, Claude Code) — in Oh My Pi's
/// per-profile credential store or the vendor CLI's own store — already deduplicated by account.
/// Each becomes its own per-account [`SourceDescriptor`] so multiple logins are connected and
/// shown independently. Pure data: the adapter does the IO and constructs these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredAccount {
    /// The base provider this login belongs to: `"codex"` or `"claude-code"`. Drives the
    /// per-account source id, display name, disclosure, and refreshed-token namespace.
    pub base: ProviderId,
    /// Provider account id — stable across profiles and token refreshes; the dedup key and the
    /// source-id suffix.
    pub account_id: String,
    /// Account email for the panel subtitle, when known.
    pub email: Option<String>,
    /// Where this login was found, e.g. `"Oh My Pi · work"` or `"Codex CLI"`.
    pub origin: String,
}

/// Display metadata for a base provider that supports per-account discovery.
struct AccountProviderSpec {
    base: &'static str,
    display_name: &'static str,
    access_note: &'static str,
}

/// Honest, per-base disclosures shown before opt-in. Identical across a provider's accounts; the
/// email (and origin) tell them apart in the UI.
const CODEX_ACCESS_NOTE: &str = "Reuses a Codex login already on this machine — the OAuth token \
     kept by the Codex CLI (~/.codex/auth.json) or by Oh My Pi (per profile) — to read that \
     ChatGPT subscription's Codex usage. The token is never shown, never stored in MLT's \
     database or logs, and is sent only to OpenAI.";
const CLAUDE_CODE_ACCESS_NOTE: &str = "Reuses a Claude Code login already on this machine — the \
     OAuth token kept by Claude Code or by Oh My Pi (per profile) — to read that Claude \
     subscription's usage. The token is never shown, never stored in MLT's database or logs, \
     and is sent only to Anthropic.";

/// The providers that expand into per-account sources from local OAuth stores. Registering a new
/// multi-account provider is one entry here (plus its strategy + store wiring in the adapter).
const ACCOUNT_PROVIDERS: &[AccountProviderSpec] = &[
    AccountProviderSpec {
        base: "codex",
        display_name: "Codex",
        access_note: CODEX_ACCESS_NOTE,
    },
    AccountProviderSpec {
        base: "claude-code",
        display_name: "Claude Code",
        access_note: CLAUDE_CODE_ACCESS_NOTE,
    },
];

/// The source id for a per-account login: `<base>:<account_id>`. Stable per account, so a login
/// keeps the same id — and thus the same consent and cached-token namespace — across refreshes
/// and across the profiles/stores it appears in.
pub fn account_source_id(base: &str, account_id: &str) -> String {
    format!("{base}:{account_id}")
}

/// The keychain key (under MLT's own service) for OUR refreshed copy of one account's token,
/// namespaced by base + account id so two logins never collide — never the vendor's own store,
/// which MLT only reads (AGENTS.md). The catalog and the fetch strategy both derive the key from
/// here, so disconnect purges exactly what the strategy wrote.
pub fn account_cache_key(base: &str, account_id: &str) -> String {
    format!("oauth.{base}.{account_id}")
}

/// Build the connect-screen descriptor for one discovered account, or `None` if its base isn't a
/// known multi-account provider. The account id makes the source id and cache key unique, so two
/// logins never collide in consent, identity, or keychain storage.
pub fn account_descriptor(base: &str, account_id: &str) -> Option<SourceDescriptor> {
    let spec = ACCOUNT_PROVIDERS.iter().find(|s| s.base == base)?;
    Some(SourceDescriptor {
        id: ProviderId::new(account_source_id(spec.base, account_id)),
        display_name: spec.display_name,
        access_note: spec.access_note,
        credential: CredentialKind::LocalLogin,
        oauth_cache_key: Some(account_cache_key(spec.base, account_id)),
    })
}

/// Build every connect-screen row: probe presence for each static source and pair it with the
/// stored consent, then expand the discovered per-account logins. Reading a secret still requires
/// [`SourceState::active`].
pub async fn discover_sources(
    catalog: &[SourceDescriptor],
    accounts: &[DiscoveredAccount],
    probe: &dyn SourceProbe,
    consent: &dyn ConsentStore,
    labels: &dyn SourceLabels,
    identity: &dyn IdentityStore,
) -> Result<Vec<SourceState>, PortError> {
    let mut states = Vec::with_capacity(catalog.len() + accounts.len());
    for descriptor in catalog {
        let enabled = consent.is_enabled(&descriptor.id)?;
        let present = probe.is_present(&descriptor.id).await;
        let mut state = descriptor.to_state(present, enabled);
        state.label = labels.label(&descriptor.id)?;
        state.account = identity.identity(&descriptor.id)?;
        states.push(state);
    }
    // Discovered accounts are present by construction (discovery already found the credential),
    // so they skip the file probe. The email surfaced by discovery seeds the account subtitle
    // immediately, before any usage fetch resolves identity.
    for account in accounts {
        let Some(descriptor) = account_descriptor(account.base.as_str(), &account.account_id)
        else {
            continue;
        };
        let enabled = consent.is_enabled(&descriptor.id)?;
        let mut state = descriptor.to_state(true, enabled);
        state.label = labels.label(&descriptor.id)?;
        state.account = identity.identity(&descriptor.id)?.or_else(|| {
            account.email.clone().map(|email| AccountIdentity {
                email: Some(email),
                organization: None,
            })
        });
        states.push(state);
    }
    Ok(states)
}

/// The sources the refresh loop may actually fetch: opted-in **and** present. Consent is checked
/// first so a disabled source is never even probed — the presence check stays off the hot path
/// until the user has consented.
pub async fn active_sources(
    catalog: &[SourceDescriptor],
    accounts: &[DiscoveredAccount],
    probe: &dyn SourceProbe,
    consent: &dyn ConsentStore,
) -> Result<Vec<ProviderId>, PortError> {
    let mut active = Vec::new();
    for descriptor in catalog {
        let active_now = match descriptor.credential {
            // Consent first, so a disabled local source is never even probed.
            CredentialKind::LocalLogin => {
                consent.is_enabled(&descriptor.id)? && probe.is_present(&descriptor.id).await
            }
            // A stored, validated key is the connection — there is nothing local to probe.
            CredentialKind::ApiKey => consent.is_enabled(&descriptor.id)?,
        };
        if active_now {
            active.push(descriptor.id.clone());
        }
    }
    // A discovered account is present by construction, so consent alone gates it.
    for account in accounts {
        let id = ProviderId::new(account_source_id(
            account.base.as_str(),
            &account.account_id,
        ));
        if consent.is_enabled(&id)? {
            active.push(id);
        }
    }
    Ok(active)
}

/// Find a source descriptor by id within a catalog. The app layer uses this to look up a
/// source's [`CredentialKind`] before acting on it (e.g. routing an API-key edit).
pub fn find_source<'a>(
    catalog: &'a [SourceDescriptor],
    id: &ProviderId,
) -> Option<&'a SourceDescriptor> {
    catalog.iter().find(|descriptor| &descriptor.id == id)
}

/// The keychain entry name under which the user-entered API key for `id` is stored (under our
/// own service). Namespaced apart from OAuth caches (`oauth.*`) so the two never collide.
pub fn api_key_secret_key(id: &ProviderId) -> String {
    format!("api_key.{}", id.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::collections::HashSet;

    fn descriptor(id: &str) -> SourceDescriptor {
        SourceDescriptor {
            id: ProviderId::new(id),
            display_name: "Test Source",
            access_note: "note",
            credential: CredentialKind::LocalLogin,
            oauth_cache_key: None,
        }
    }

    fn api_descriptor(id: &str) -> SourceDescriptor {
        SourceDescriptor {
            id: ProviderId::new(id),
            display_name: "Test API Source",
            access_note: "note",
            credential: CredentialKind::ApiKey,
            oauth_cache_key: None,
        }
    }

    /// Presence is decided by a set of ids; the probe never sees a secret. Counts probe
    /// calls so we can assert the consent gate skips disabled sources before any probe.
    #[derive(Default)]
    struct FakeProbe {
        present: HashSet<String>,
        probes: std::sync::atomic::AtomicUsize,
    }
    #[async_trait]
    impl SourceProbe for FakeProbe {
        async fn is_present(&self, id: &ProviderId) -> bool {
            self.probes
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.present.contains(id.as_str())
        }
    }

    struct FakeConsent {
        enabled: HashSet<String>,
    }
    impl ConsentStore for FakeConsent {
        fn is_enabled(&self, id: &ProviderId) -> Result<bool, PortError> {
            Ok(self.enabled.contains(id.as_str()))
        }
        fn set_enabled(&self, _id: &ProviderId, _enabled: bool) -> Result<(), PortError> {
            Ok(())
        }
    }

    struct FakeLabels {
        labels: std::collections::HashMap<String, String>,
    }
    impl SourceLabels for FakeLabels {
        fn label(&self, id: &ProviderId) -> Result<Option<String>, PortError> {
            Ok(self.labels.get(id.as_str()).cloned())
        }
        fn set_label(&self, _id: &ProviderId, _label: Option<&str>) -> Result<(), PortError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeIdentity {
        identities: std::collections::HashMap<String, AccountIdentity>,
    }
    impl IdentityStore for FakeIdentity {
        fn identity(&self, id: &ProviderId) -> Result<Option<AccountIdentity>, PortError> {
            Ok(self.identities.get(id.as_str()).cloned())
        }
        fn set_identity(
            &self,
            _id: &ProviderId,
            _identity: &AccountIdentity,
        ) -> Result<(), PortError> {
            Ok(())
        }
        fn clear_identity(&self, _id: &ProviderId) -> Result<(), PortError> {
            Ok(())
        }
    }

    #[test]
    fn active_requires_both_presence_and_consent() {
        let d = descriptor("x");
        assert!(!d.to_state(false, false).active());
        assert!(
            !d.to_state(true, false).active(),
            "present but not opted in"
        );
        assert!(
            !d.to_state(false, true).active(),
            "opted in but not on this machine"
        );
        assert!(d.to_state(true, true).active());
    }

    #[test]
    fn catalog_ships_claude_with_an_honest_disclosure() {
        let catalog = source_catalog();
        let claude = catalog
            .iter()
            .find(|d| d.id.as_str() == "claude-code")
            .expect("claude-code in catalog");
        // The note must disclose the credential read before opt-in, not be a placeholder.
        assert!(claude.access_note.contains("OAuth"));
        assert!(claude.access_note.to_lowercase().contains("anthropic"));
    }

    #[test]
    fn account_descriptor_is_a_reused_login_namespaced_per_provider_and_account() {
        let codex = account_descriptor("codex", "acct-123").expect("codex base is registered");
        assert_eq!(codex.id.as_str(), "codex:acct-123");
        assert_eq!(codex.credential, CredentialKind::LocalLogin);
        assert_eq!(codex.display_name, "Codex");
        assert!(
            codex.access_note.to_lowercase().contains("openai"),
            "discloses where data goes"
        );
        assert_eq!(
            codex.cached_secret_keys(),
            vec!["oauth.codex.acct-123".to_string()]
        );

        // The same machinery serves Claude — a distinct base / display / cache namespace.
        let claude =
            account_descriptor("claude-code", "acct-123").expect("claude-code is registered");
        assert_eq!(claude.id.as_str(), "claude-code:acct-123");
        assert_eq!(claude.display_name, "Claude Code");
        assert!(
            claude.access_note.to_lowercase().contains("anthropic"),
            "discloses where data goes"
        );
        assert_eq!(
            claude.cached_secret_keys(),
            vec!["oauth.claude-code.acct-123".to_string()]
        );

        // A different account → a different id; an unregistered base → no descriptor.
        assert_ne!(
            account_descriptor("codex", "acct-999").unwrap().id,
            codex.id
        );
        assert!(account_descriptor("mystery", "x").is_none());
    }

    #[tokio::test]
    async fn discovers_accounts_across_providers_as_present_per_account_sources() {
        let accounts = [
            DiscoveredAccount {
                base: ProviderId::new("codex"),
                account_id: "acct-a".into(),
                email: Some("a@example.com".into()),
                origin: "Oh My Pi · work".into(),
            },
            DiscoveredAccount {
                base: ProviderId::new("claude-code"),
                account_id: "acct-b".into(),
                email: None,
                origin: "Oh My Pi · default".into(),
            },
        ];
        let probe = FakeProbe::default(); // nothing present on disk
        let consent = FakeConsent {
            enabled: ["codex:acct-a"].into_iter().map(String::from).collect(),
        };
        let labels = FakeLabels {
            labels: std::collections::HashMap::new(),
        };
        let identity = FakeIdentity::default();

        let states = discover_sources(&[], &accounts, &probe, &consent, &labels, &identity)
            .await
            .unwrap();
        assert_eq!(states.len(), 2);
        // A Codex and a Claude account, each present-by-discovery (never file-probed), enabled
        // only where consented, email seeded from discovery — one shared code path, both providers.
        let a = states
            .iter()
            .find(|s| s.id.as_str() == "codex:acct-a")
            .unwrap();
        assert!(a.present && a.enabled && a.active());
        assert_eq!(
            a.account.as_ref().and_then(|x| x.email.as_deref()),
            Some("a@example.com")
        );
        let b = states
            .iter()
            .find(|s| s.id.as_str() == "claude-code:acct-b")
            .unwrap();
        assert!(
            b.present && !b.enabled && !b.active(),
            "unconsented account stays inactive"
        );

        let active = active_sources(&[], &accounts, &probe, &consent)
            .await
            .unwrap();
        assert_eq!(active, vec![ProviderId::new("codex:acct-a")]);
        assert_eq!(
            probe.probes.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "discovered accounts are present by discovery and never file-probed"
        );
    }

    #[tokio::test]
    async fn discover_pairs_presence_with_consent_for_every_source() {
        let catalog = [descriptor("a"), descriptor("b")];
        let probe = FakeProbe {
            present: ["a"].into_iter().map(String::from).collect(),
            ..Default::default()
        };
        let consent = FakeConsent {
            enabled: ["b"].into_iter().map(String::from).collect(),
        };
        let labels = FakeLabels {
            labels: [("a", "My A")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        };
        let identity = FakeIdentity {
            identities: [(
                "a".to_string(),
                AccountIdentity {
                    email: Some("a@example.com".into()),
                    organization: None,
                },
            )]
            .into_iter()
            .collect(),
        };
        let states = discover_sources(&catalog, &[], &probe, &consent, &labels, &identity)
            .await
            .unwrap();

        assert_eq!(states.len(), 2);
        // "a": found on the machine but not opted in → present, not enabled, not active.
        assert_eq!(states[0].id.as_str(), "a");
        assert!(states[0].present && !states[0].enabled && !states[0].active());
        // …and its stored label flows through, while "b" has none.
        assert_eq!(states[0].label.as_deref(), Some("My A"));
        assert_eq!(states[1].label, None);
        // The fetched account identity flows through too, siloed per source ("b" has none).
        assert_eq!(
            states[0].account.as_ref().and_then(|a| a.email.as_deref()),
            Some("a@example.com")
        );
        assert_eq!(states[1].account, None);
        // "b": opted in but not present → enabled, not present, not active.
        assert!(!states[1].present && states[1].enabled && !states[1].active());
    }

    #[tokio::test]
    async fn active_sources_skips_probing_disabled_sources() {
        let catalog = [descriptor("on"), descriptor("off")];
        let probe = FakeProbe {
            present: ["on", "off"].into_iter().map(String::from).collect(),
            ..Default::default()
        };
        let consent = FakeConsent {
            enabled: ["on"].into_iter().map(String::from).collect(),
        };

        let active = active_sources(&catalog, &[], &probe, &consent)
            .await
            .unwrap();
        assert_eq!(active, vec![ProviderId::new("on")]);
        // The disabled source is gated on consent *before* any presence probe runs: exactly
        // one probe fired (for the enabled source), so "off" was never inspected at all.
        assert_eq!(probe.probes.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn api_key_source_is_active_on_consent_alone_ignoring_presence() {
        let d = api_descriptor("k");
        // No local login to discover — a stored (enabled) key is the connection, so presence
        // is irrelevant; only `enabled` decides `active`.
        assert!(!d.to_state(false, false).active());
        assert!(
            !d.to_state(true, false).active(),
            "no key stored ⇒ not active"
        );
        assert!(
            d.to_state(false, true).active(),
            "key stored ⇒ active even if not probed"
        );
        assert!(d.to_state(true, true).active());
    }

    #[test]
    fn catalog_ships_openrouter_as_an_api_key_source() {
        let catalog = source_catalog();
        let openrouter =
            find_source(&catalog, &ProviderId::new("openrouter")).expect("openrouter in catalog");
        assert_eq!(openrouter.credential, CredentialKind::ApiKey);
        assert_eq!(openrouter.display_name, "OpenRouter");
        // The disclosure must be honest about where the key lives and where it does not.
        let note = openrouter.access_note.to_lowercase();
        assert!(note.contains("keychain"), "discloses keychain storage");
        assert!(note.contains("api key"), "names the credential");
    }

    #[tokio::test]
    async fn active_sources_admits_an_api_key_source_without_a_probe() {
        // An API-key source is active on consent alone; a local-login source still needs both.
        let catalog = [api_descriptor("router"), descriptor("cli")];
        let probe = FakeProbe::default(); // nothing present locally
        let consent = FakeConsent {
            enabled: ["router", "cli"].into_iter().map(String::from).collect(),
        };

        let active = active_sources(&catalog, &[], &probe, &consent)
            .await
            .unwrap();
        // "router" (api-key, consented) is active despite being absent from the probe; "cli"
        // (local-login, consented but not present) is not.
        assert_eq!(active, vec![ProviderId::new("router")]);
    }

    #[test]
    fn find_source_locates_by_id() {
        let catalog = [descriptor("a"), api_descriptor("b")];
        assert_eq!(
            find_source(&catalog, &ProviderId::new("b")).map(|d| d.credential),
            Some(CredentialKind::ApiKey)
        );
        assert!(find_source(&catalog, &ProviderId::new("missing")).is_none());
    }

    #[test]
    fn api_key_secret_key_is_namespaced_per_provider() {
        assert_eq!(
            api_key_secret_key(&ProviderId::new("openrouter")),
            "api_key.openrouter"
        );
        // Distinct from the OAuth cache namespace, so the two storage paths never collide.
        assert!(api_key_secret_key(&ProviderId::new("x")).starts_with("api_key."));
    }

    #[test]
    fn cached_secret_keys_lists_only_what_mlt_caches_itself() {
        // API-key source: exactly the namespaced key we store, nothing else.
        assert_eq!(
            api_descriptor("openrouter").cached_secret_keys(),
            vec!["api_key.openrouter".to_string()]
        );
        // Local-login source with no refreshed-OAuth cache: nothing of ours to purge.
        assert!(descriptor("cli").cached_secret_keys().is_empty());
        // Local-login source that caches a refreshed OAuth copy: exactly that entry, and never
        // an api_key.* key (it has no user-entered key).
        let oauth_source = SourceDescriptor {
            oauth_cache_key: Some("oauth.example".to_string()),
            ..descriptor("example")
        };
        assert_eq!(
            oauth_source.cached_secret_keys(),
            vec!["oauth.example".to_string()]
        );
    }

    #[test]
    fn catalog_declares_each_source_self_cached_secret() {
        let catalog = source_catalog();
        // Claude (reused login) caches OUR refreshed OAuth copy under our own service — that is
        // what disconnect must purge, distinct from Claude Code's own keychain item.
        let claude = find_source(&catalog, &ProviderId::new("claude-code")).unwrap();
        assert_eq!(
            claude.cached_secret_keys(),
            vec![crate::providers::claude::CACHE_KEY.to_string()]
        );
        assert!(crate::providers::claude::CACHE_KEY.starts_with("oauth."));
        // OpenRouter (api-key) caches the user-entered key, namespaced per provider.
        let openrouter = find_source(&catalog, &ProviderId::new("openrouter")).unwrap();
        assert_eq!(
            openrouter.cached_secret_keys(),
            vec!["api_key.openrouter".to_string()]
        );
    }
}
