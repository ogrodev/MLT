//! Local source discovery + consent (ADR 0012, PRD §4/§9).
//!
//! A *source* is a place we can reuse an existing login from — a vendor CLI's credentials,
//! a browser profile, … Discovery is **metadata-only**: we present what a machine *could*
//! connect to, with a plain-language note of what each accesses and why, and read **nothing**
//! until the user opts that source in. The two rules that make this safe live here, pure and
//! unit-tested: presence detection never touches a secret (enforced in the adapter), and a
//! source is read only when it is both **present** and **enabled** ([`SourceState::active`]).
use crate::domain::ProviderId;
use crate::ports::{ConsentStore, PortError, SourceProbe};
use serde::{Deserialize, Serialize};

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
        }
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
}

impl SourceState {
    /// The app reads a source — and only then touches a secret — when it is **both**
    /// discovered on this machine **and** opted in. This is the consent gate (ADR 0012).
    pub fn active(&self) -> bool {
        self.present && self.enabled
    }
}

/// Every source MLT can discover today. New providers register here (one line); the same
/// catalog drives the connect screen and the refresh loop's consent gate.
pub fn source_catalog() -> Vec<SourceDescriptor> {
    vec![SourceDescriptor {
        id: ProviderId::new("claude-code"),
        display_name: "Claude Code",
        access_note: "Reuses your Claude Code login — an OAuth token already on this Mac \
                      (in ~/.claude or your Keychain) — to read your Claude subscription \
                      usage. The token is never shown, never stored in MLT's database or \
                      logs, and is sent only to Anthropic.",
    }]
}

/// Build every connect-screen row: probe presence for each known source and pair it with the
/// stored consent. Presence is checked for *all* sources (the user needs to see what's
/// available); reading a secret still requires [`SourceState::active`].
pub async fn discover_sources(
    catalog: &[SourceDescriptor],
    probe: &dyn SourceProbe,
    consent: &dyn ConsentStore,
) -> Result<Vec<SourceState>, PortError> {
    let mut states = Vec::with_capacity(catalog.len());
    for descriptor in catalog {
        let enabled = consent.is_enabled(&descriptor.id)?;
        let present = probe.is_present(&descriptor.id).await;
        states.push(descriptor.to_state(present, enabled));
    }
    Ok(states)
}

/// The sources the refresh loop may actually fetch: opted-in **and** present. Consent is
/// checked first so a disabled source is never even probed — the presence check stays off
/// the hot path until the user has consented.
pub async fn active_sources(
    catalog: &[SourceDescriptor],
    probe: &dyn SourceProbe,
    consent: &dyn ConsentStore,
) -> Result<Vec<ProviderId>, PortError> {
    let mut active = Vec::new();
    for descriptor in catalog {
        if consent.is_enabled(&descriptor.id)? && probe.is_present(&descriptor.id).await {
            active.push(descriptor.id.clone());
        }
    }
    Ok(active)
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
        let states = discover_sources(&catalog, &probe, &consent).await.unwrap();

        assert_eq!(states.len(), 2);
        // "a": found on the machine but not opted in → present, not enabled, not active.
        assert_eq!(states[0].id.as_str(), "a");
        assert!(states[0].present && !states[0].enabled && !states[0].active());
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

        let active = active_sources(&catalog, &probe, &consent).await.unwrap();
        assert_eq!(active, vec![ProviderId::new("on")]);
        // The disabled source is gated on consent *before* any presence probe runs: exactly
        // one probe fired (for the enabled source), so "off" was never inspected at all.
        assert_eq!(probe.probes.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
