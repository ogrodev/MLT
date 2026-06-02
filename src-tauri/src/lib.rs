// MLT app crate: tray + chromeless popover, wired to the provider slice in mlt-adapters.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mlt_adapters::{
    FileConsentStore, FileIdentityStore, FileLabelStore, KeyringSecretStore, LocalSourceProbe,
    ReqwestHttp, KEYCHAIN_SERVICE,
};
use mlt_core::domain::{ProviderId, UsageSnapshot};
use mlt_core::ports::{
    ConsentStore, HttpPort, IdentityStore, SecretStore, SourceLabels, SourceProbe,
};
use mlt_core::providers::openrouter::validate_key;
use mlt_core::providers::{FetchContext, FetchStrategy};
use mlt_core::sources::{
    active_sources, api_key_secret_key, discover_sources, find_source, source_catalog,
    CredentialKind, SourceDescriptor, SourceState,
};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager};
use tauri_plugin_positioner::{Position, WindowExt};

const POPOVER: &str = "main";
const REFRESH_SECS: u64 = 60;
/// How long after a focus-loss auto-hide a tray click still counts as the *same* click that
/// dismissed the popover, so it isn't immediately re-opened. Covers the macOS event race
/// where clicking the tray icon blurs the popover (hiding it) before the click is delivered.
const REOPEN_DEBOUNCE: Duration = Duration::from_millis(250);

/// What a tray-icon left-click should do, decided by [`popover_action`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PopoverAction {
    Show,
    Hide,
    Ignore,
}

/// Popover interaction state shared between the tray-click and focus-loss handlers.
#[derive(Default)]
struct PopoverState {
    /// When the popover was last auto-hidden because it lost focus — drives the re-click
    /// debounce (see [`REOPEN_DEBOUNCE`]).
    last_focus_hide: Mutex<Option<Instant>>,
}

/// Discovery + consent wiring shared by the source commands and the refresh loop. Holds the
/// metadata-only presence probe and the persisted per-source opt-in (ADR 0012); nothing here
/// reads a secret until [`active_sources`] clears a source for fetching.
struct AppSources {
    probe: Arc<dyn SourceProbe>,
    consent: Arc<dyn ConsentStore>,
    secrets: Arc<dyn SecretStore>,
    labels: Arc<dyn SourceLabels>,
    http: Arc<dyn HttpPort>,
    identity: Arc<dyn IdentityStore>,
}

/// Pure decision for a tray left-click, isolated from Tauri so it is unit-testable.
///
/// Dismissing an open popover by clicking the tray icon first fires a focus-loss that hides
/// the window, so by the time the click lands the popover already reads as hidden; a naive
/// toggle would then re-open it. `since_focus_hide` is how long ago that focus-loss hide
/// happened (`None` if it never did).
fn popover_action(
    visible: bool,
    since_focus_hide: Option<Duration>,
    debounce: Duration,
) -> PopoverAction {
    if visible {
        PopoverAction::Hide
    } else if since_focus_hide.is_some_and(|elapsed| elapsed < debounce) {
        PopoverAction::Ignore
    } else {
        PopoverAction::Show
    }
}

/// Fetch the current Claude Code subscription usage on demand (called by the UI on open).
/// Gated on consent: the secret is read only when the source is opted in *and* present, so a
/// stray call can never read credentials the user hasn't connected (ADR 0012).
#[tauri::command]
async fn fetch_claude_usage(
    sources: tauri::State<'_, AppSources>,
) -> Result<UsageSnapshot, String> {
    let id = ProviderId::new("claude-code");
    let connected = sources.consent.is_enabled(&id).map_err(|e| e.to_string())?
        && sources.probe.is_present(&id).await;
    if !connected {
        return Err("Claude Code is not connected".into());
    }
    claude_usage(sources.identity.clone())
        .await
        .map_err(|e| e.to_string())
}

/// Discover every known source for the connect screen: presence (metadata only) + the user's
/// stored opt-in. Reads no secret.
#[tauri::command]
async fn list_sources(sources: tauri::State<'_, AppSources>) -> Result<Vec<SourceState>, String> {
    discover_sources(
        &source_catalog(),
        sources.probe.as_ref(),
        sources.consent.as_ref(),
        sources.labels.as_ref(),
        sources.identity.as_ref(),
    )
    .await
    .map_err(|e| e.to_string())
}

/// Connect or disconnect a local-login source via its consent toggle. Takes effect
/// immediately, without a restart. Enabling persists consent and kicks an off-thread refresh so
/// the popover fills in right away (rather than waiting for the next poll); disabling routes
/// through [`disconnect`] so the source's cached credential is *purged* from the keychain, not
/// merely de-consented. Returns the refreshed source list for the UI.
#[tauri::command]
async fn set_source_enabled(
    app: tauri::AppHandle,
    sources: tauri::State<'_, AppSources>,
    id: String,
    enabled: bool,
) -> Result<Vec<SourceState>, String> {
    let id = ProviderId::new(id);
    let catalog = source_catalog();
    let descriptor = find_source(&catalog, &id).ok_or("Unknown source")?;
    // API-key sources connect by storing a validated key, not by a bare consent toggle — route
    // them through set_api_key / disconnect_source so consent never diverges from the key.
    if descriptor.credential == CredentialKind::ApiKey {
        return Err("Use set_api_key / disconnect_source for API-key sources".into());
    }

    if enabled {
        sources
            .consent
            .set_enabled(&id, true)
            .map_err(|e| e.to_string())?;
        kick_refresh(&app, &sources);
    } else {
        disconnect(
            sources.secrets.as_ref(),
            sources.consent.as_ref(),
            sources.identity.as_ref(),
            descriptor,
        )?;
    }

    discover_sources(
        &catalog,
        sources.probe.as_ref(),
        sources.consent.as_ref(),
        sources.labels.as_ref(),
        sources.identity.as_ref(),
    )
    .await
    .map_err(|e| e.to_string())
}

/// Validate, then store an API key and mark the source connected — the testable core of
/// [`set_api_key`]. Validation happens **first**, so a rejected or unverifiable key is never
/// written and consent is never set: a bad key can't leave a source silently reading as
/// connected. The key is stored in the keychain only; its value is neither returned nor logged.
async fn apply_api_key(
    secrets: &dyn SecretStore,
    consent: &dyn ConsentStore,
    http: &dyn HttpPort,
    id: &ProviderId,
    key: &str,
) -> Result<(), String> {
    let catalog = source_catalog();
    let descriptor = find_source(&catalog, id).ok_or("Unknown source")?;
    if descriptor.credential != CredentialKind::ApiKey {
        return Err("This source does not use an API key".into());
    }
    validate_key(http, key).await.map_err(|e| e.to_string())?;
    secrets
        .set(&api_key_secret_key(id), key.trim())
        .map_err(|e| e.to_string())?;
    consent.set_enabled(id, true).map_err(|e| e.to_string())
}

/// Disconnect a source: purge every secret MLT cached for it under our *own* service, forget
/// any provider-fetched identity, then clear consent. The testable core shared by
/// [`disconnect_source`] and the disable path of [`set_source_enabled`]. We only ever delete
/// copies WE wrote (the user-entered API key and/or a refreshed-OAuth copy) — the vendor's own
/// credential store is never touched (ADR 0012/0016). Secrets are purged *before* consent is
/// cleared, so a keychain failure leaves the source still connected rather than half-removed.
/// Idempotent: absent entries are not an error, so disconnecting twice is safe.
fn disconnect(
    secrets: &dyn SecretStore,
    consent: &dyn ConsentStore,
    identity: &dyn IdentityStore,
    descriptor: &SourceDescriptor,
) -> Result<(), String> {
    for key in descriptor.cached_secret_keys() {
        secrets.delete(&key).map_err(|e| e.to_string())?;
    }
    identity
        .clear_identity(&descriptor.id)
        .map_err(|e| e.to_string())?;
    consent
        .set_enabled(&descriptor.id, false)
        .map_err(|e| e.to_string())
}

/// Enter or replace the API key for an API-key source (e.g. OpenRouter). The key is
/// **validated against the provider before anything is stored** — a rejected key returns a
/// clear error and the source stays disconnected, never a silent "connected" with a bad key.
/// On success the key is written to the OS keychain only (never the DB or logs), consent is
/// recorded so the source reads as connected, and an off-thread refresh is kicked so it takes
/// effect without a restart. The key itself is never returned to the UI.
#[tauri::command]
async fn set_api_key(
    app: tauri::AppHandle,
    sources: tauri::State<'_, AppSources>,
    id: String,
    key: String,
) -> Result<Vec<SourceState>, String> {
    let id = ProviderId::new(id);
    apply_api_key(
        sources.secrets.as_ref(),
        sources.consent.as_ref(),
        sources.http.as_ref(),
        &id,
        &key,
    )
    .await?;
    kick_refresh(&app, &sources);
    discover_sources(
        &source_catalog(),
        sources.probe.as_ref(),
        sources.consent.as_ref(),
        sources.labels.as_ref(),
        sources.identity.as_ref(),
    )
    .await
    .map_err(|e| e.to_string())
}

/// Disconnect any connected source: remove every secret MLT cached for it from the OS keychain,
/// forget its provider-fetched identity, and clear consent — so its tile disappears and its
/// refresh stops, immediately and without a restart, and it can be reconnected afterwards. Only
/// copies WE wrote are deleted; the vendor's own credential store is never touched
/// (ADR 0012/0016). Works for any source kind — the disconnect action for both reused logins
/// and API-key sources.
#[tauri::command]
async fn disconnect_source(
    sources: tauri::State<'_, AppSources>,
    id: String,
) -> Result<Vec<SourceState>, String> {
    let id = ProviderId::new(id);
    let catalog = source_catalog();
    let descriptor = find_source(&catalog, &id).ok_or("Unknown source")?;
    disconnect(
        sources.secrets.as_ref(),
        sources.consent.as_ref(),
        sources.identity.as_ref(),
        descriptor,
    )?;
    discover_sources(
        &catalog,
        sources.probe.as_ref(),
        sources.consent.as_ref(),
        sources.labels.as_ref(),
        sources.identity.as_ref(),
    )
    .await
    .map_err(|e| e.to_string())
}

/// Set (or clear, with a blank string) the user-assigned custom name for a source — shown as
/// the panel *title*, distinct from the provider's own name and the auto-fetched account email.
/// Persisted as a plain setting (never the keychain); returns the refreshed source list so the
/// UI reflects the new title without a restart.
#[tauri::command]
async fn set_source_label(
    sources: tauri::State<'_, AppSources>,
    id: String,
    name: String,
) -> Result<Vec<SourceState>, String> {
    let id = ProviderId::new(id);
    let trimmed = name.trim();
    let label = (!trimmed.is_empty()).then_some(trimmed);
    sources
        .labels
        .set_label(&id, label)
        .map_err(|e| e.to_string())?;
    discover_sources(
        &source_catalog(),
        sources.probe.as_ref(),
        sources.consent.as_ref(),
        sources.labels.as_ref(),
        sources.identity.as_ref(),
    )
    .await
    .map_err(|e| e.to_string())
}

/// Kick an immediate off-thread refresh of the active sources, so a just-connected source
/// fills the popover without waiting for the next poll — and without blocking the command.
fn kick_refresh(app: &tauri::AppHandle, sources: &AppSources) {
    let app = app.clone();
    let probe = sources.probe.clone();
    let consent = sources.consent.clone();
    let identity = sources.identity.clone();
    tauri::async_runtime::spawn(async move {
        refresh_active(&app, probe.as_ref(), consent.as_ref(), identity).await;
    });
}

/// Quit the whole app. Both the popover footer and the tray menu route here.
#[tauri::command]
fn quit(app: tauri::AppHandle) {
    app.exit(0);
}

async fn claude_usage(
    identity: Arc<dyn IdentityStore>,
) -> Result<UsageSnapshot, mlt_core::providers::FetchError> {
    let strategy = mlt_adapters::claude_strategy(identity);
    let ctx = FetchContext {
        provider: ProviderId::new("claude-code"),
    };
    strategy.fetch(&ctx).await
}

/// Fetch one source's usage, or `None` for a source with no fetch wired yet. The map of
/// id → fetcher is the single place a connected source becomes a network call.
async fn fetch_for(
    id: &ProviderId,
    identity: Arc<dyn IdentityStore>,
) -> Option<Result<UsageSnapshot, mlt_core::providers::FetchError>> {
    match id.as_str() {
        "claude-code" => Some(claude_usage(identity).await),
        _ => None,
    }
}

/// Refresh every *active* source (opted-in and present) and emit the result for the popover.
/// Disabled or absent sources are skipped here, so no secret is read for them — this is the
/// consent gate the whole refresh path funnels through.
async fn refresh_active(
    app: &tauri::AppHandle,
    probe: &dyn SourceProbe,
    consent: &dyn ConsentStore,
    identity: Arc<dyn IdentityStore>,
) {
    let Ok(ids) = active_sources(&source_catalog(), probe, consent).await else {
        return;
    };
    for id in &ids {
        match fetch_for(id, identity.clone()).await {
            Some(Ok(snapshot)) => {
                let _ = app.emit("usage-updated", snapshot);
            }
            Some(Err(e)) => {
                let _ = app.emit("usage-error", e.to_string());
            }
            None => {}
        }
    }
}

/// Background poll loop: refresh the active sources on a cadence and emit events the popover
/// listens to. Runs once immediately so a previously-connected source shows up at launch.
fn spawn_refresh_loop(
    app: tauri::AppHandle,
    probe: Arc<dyn SourceProbe>,
    consent: Arc<dyn ConsentStore>,
    identity: Arc<dyn IdentityStore>,
) {
    tauri::async_runtime::spawn(async move {
        loop {
            refresh_active(&app, probe.as_ref(), consent.as_ref(), identity.clone()).await;
            tokio::time::sleep(Duration::from_secs(REFRESH_SECS)).await;
        }
    });
}

/// Show the popover anchored under the tray icon, hide it if already open, or swallow a click
/// that merely dismissed it (see [`popover_action`]).
fn toggle_popover(app: &tauri::AppHandle) {
    let Some(win) = app.get_webview_window(POPOVER) else {
        return;
    };
    let visible = win.is_visible().unwrap_or(false);
    let since_hide = app
        .state::<PopoverState>()
        .last_focus_hide
        .lock()
        .ok()
        .and_then(|guard| *guard)
        .map(|at| at.elapsed());

    match popover_action(visible, since_hide, REOPEN_DEBOUNCE) {
        PopoverAction::Hide => {
            let _ = win.hide();
        }
        PopoverAction::Show => {
            let _ = win.move_window(Position::TrayBottomCenter);
            let _ = win.show();
            let _ = win.set_focus();
        }
        PopoverAction::Ignore => {}
    }
}

/// Record that the popover was just auto-hidden by losing focus, for the re-click debounce.
fn note_focus_hide(app: &tauri::AppHandle) {
    if let Ok(mut guard) = app.state::<PopoverState>().last_focus_hide.lock() {
        *guard = Some(Instant::now());
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_positioner::init())
        .manage(PopoverState::default())
        .invoke_handler(tauri::generate_handler![
            fetch_claude_usage,
            list_sources,
            set_source_enabled,
            set_api_key,
            disconnect_source,
            set_source_label,
            quit
        ])
        .setup(|app| {
            // Menu-bar app: no Dock icon / app-switcher entry on macOS.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            // Right-click menu: an always-available way to quit, even with the popover closed.
            let quit_item = MenuItem::with_id(app, "quit", "Quit MLT", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&quit_item])?;

            // Tray icon — left-click toggles the popover, right-click opens the menu. The
            // template image adapts to light/dark menu bars.
            TrayIconBuilder::with_id("mlt-tray")
                .icon(tauri::include_image!("icons/32x32.png"))
                .icon_as_template(true)
                .tooltip("MLT — AI usage")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    if event.id().as_ref() == "quit" {
                        app.exit(0);
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_popover(tray.app_handle());
                    }
                })
                .build(app)?;

            // Click-outside-to-dismiss: hide the popover when it loses focus, recording the
            // moment so the tray click that caused the blur doesn't immediately reopen it.
            if let Some(win) = app.get_webview_window(POPOVER) {
                let w = win.clone();
                win.on_window_event(move |event| {
                    if let tauri::WindowEvent::Focused(false) = event {
                        note_focus_hide(w.app_handle());
                        let _ = w.hide();
                    }
                });
            }

            // Discovery + consent: a metadata-only presence probe and the persisted opt-in.
            // Consent lives in a plain JSON settings file (never the keychain), so it survives
            // restarts; presence is re-checked each poll so a source appears as soon as the
            // user logs into it. Both are shared with the source commands via managed state.
            let consent_path = app.path().app_config_dir()?.join("consent.json");
            let probe: Arc<dyn SourceProbe> = Arc::new(LocalSourceProbe);
            let consent: Arc<dyn ConsentStore> = Arc::new(FileConsentStore::load(consent_path));
            let labels_path = app.path().app_config_dir()?.join("labels.json");
            let labels: Arc<dyn SourceLabels> = Arc::new(FileLabelStore::load(labels_path));
            let identity_path = app.path().app_config_dir()?.join("identity.json");
            let identity: Arc<dyn IdentityStore> = Arc::new(FileIdentityStore::load(identity_path));
            let secrets: Arc<dyn SecretStore> = Arc::new(KeyringSecretStore::new(KEYCHAIN_SERVICE));
            let http: Arc<dyn HttpPort> = Arc::new(ReqwestHttp::new());
            app.manage(AppSources {
                probe: probe.clone(),
                consent: consent.clone(),
                secrets,
                http,
                labels,
                identity: identity.clone(),
            });

            spawn_refresh_loop(app.handle().clone(), probe, consent, identity);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::{apply_api_key, disconnect};
    use super::{popover_action, PopoverAction, REOPEN_DEBOUNCE};
    use async_trait::async_trait;
    use mlt_core::domain::{AccountIdentity, ProviderId};
    use mlt_core::ports::{
        ConsentStore, HttpPort, HttpRequest, HttpResponse, IdentityStore, PortError, SecretStore,
    };
    use mlt_core::sources::{find_source, source_catalog};
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::Duration;

    #[test]
    fn opens_when_hidden_with_no_recent_dismiss() {
        assert_eq!(
            popover_action(false, None, REOPEN_DEBOUNCE),
            PopoverAction::Show
        );
    }

    #[test]
    fn hides_when_currently_visible() {
        // Once the window is genuinely visible, a stale focus-hide timestamp is irrelevant.
        assert_eq!(
            popover_action(true, Some(Duration::from_secs(10)), REOPEN_DEBOUNCE),
            PopoverAction::Hide
        );
    }

    #[test]
    fn swallows_click_that_just_dismissed_via_focus_loss() {
        // The blur-then-click race: the window reads hidden, but it was hidden just now.
        assert_eq!(
            popover_action(false, Some(Duration::ZERO), REOPEN_DEBOUNCE),
            PopoverAction::Ignore
        );
        assert_eq!(
            popover_action(
                false,
                Some(REOPEN_DEBOUNCE - Duration::from_millis(1)),
                REOPEN_DEBOUNCE
            ),
            PopoverAction::Ignore
        );
    }

    #[test]
    fn reopens_once_the_debounce_window_elapses() {
        assert_eq!(
            popover_action(false, Some(REOPEN_DEBOUNCE), REOPEN_DEBOUNCE),
            PopoverAction::Show
        );
        assert_eq!(
            popover_action(
                false,
                Some(REOPEN_DEBOUNCE + Duration::from_millis(1)),
                REOPEN_DEBOUNCE
            ),
            PopoverAction::Show
        );
    }

    #[derive(Default)]
    struct MemSecrets(Mutex<HashMap<String, String>>);
    impl SecretStore for MemSecrets {
        fn get(&self, k: &str) -> Result<Option<String>, PortError> {
            Ok(self.0.lock().unwrap().get(k).cloned())
        }
        fn set(&self, k: &str, v: &str) -> Result<(), PortError> {
            self.0.lock().unwrap().insert(k.into(), v.into());
            Ok(())
        }
        fn delete(&self, k: &str) -> Result<(), PortError> {
            self.0.lock().unwrap().remove(k);
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeConsent(Mutex<HashMap<String, bool>>);
    impl ConsentStore for FakeConsent {
        fn is_enabled(&self, id: &ProviderId) -> Result<bool, PortError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(id.as_str())
                .copied()
                .unwrap_or(false))
        }
        fn set_enabled(&self, id: &ProviderId, enabled: bool) -> Result<(), PortError> {
            self.0.lock().unwrap().insert(id.as_str().into(), enabled);
            Ok(())
        }
    }

    #[derive(Default)]
    struct MemIdentity(Mutex<HashMap<String, AccountIdentity>>);
    impl IdentityStore for MemIdentity {
        fn identity(&self, id: &ProviderId) -> Result<Option<AccountIdentity>, PortError> {
            Ok(self.0.lock().unwrap().get(id.as_str()).cloned())
        }
        fn set_identity(&self, id: &ProviderId, v: &AccountIdentity) -> Result<(), PortError> {
            self.0.lock().unwrap().insert(id.as_str().into(), v.clone());
            Ok(())
        }
        fn clear_identity(&self, id: &ProviderId) -> Result<(), PortError> {
            self.0.lock().unwrap().remove(id.as_str());
            Ok(())
        }
    }

    /// Returns a fixed HTTP status, or a transport error, for any request.
    struct FakeHttp(Result<u16, ()>);
    #[async_trait]
    impl HttpPort for FakeHttp {
        async fn send(&self, _req: HttpRequest) -> Result<HttpResponse, PortError> {
            match self.0 {
                Ok(status) => Ok(HttpResponse {
                    status,
                    body: Vec::new(),
                }),
                Err(()) => Err(PortError::Io("offline".into())),
            }
        }
    }

    fn openrouter() -> ProviderId {
        ProviderId::new("openrouter")
    }

    #[tokio::test]
    async fn apply_api_key_stores_and_connects_a_valid_key() {
        let secrets = MemSecrets::default();
        let consent = FakeConsent::default();
        apply_api_key(
            &secrets,
            &consent,
            &FakeHttp(Ok(200)),
            &openrouter(),
            "  sk-or-v1-good ",
        )
        .await
        .expect("a valid key is accepted");
        // Stored under the namespaced keychain key, whitespace-trimmed, and marked connected.
        assert_eq!(
            secrets.get("api_key.openrouter").unwrap().as_deref(),
            Some("sk-or-v1-good")
        );
        assert!(consent.is_enabled(&openrouter()).unwrap());
    }

    #[tokio::test]
    async fn apply_api_key_rejects_a_bad_key_without_storing_or_connecting() {
        let secrets = MemSecrets::default();
        let consent = FakeConsent::default();
        let err = apply_api_key(
            &secrets,
            &consent,
            &FakeHttp(Ok(401)),
            &openrouter(),
            "nope",
        )
        .await
        .expect_err("a rejected key must not connect");
        assert!(
            err.to_lowercase().contains("rejected"),
            "clear error: {err}"
        );
        // The safety property (acceptance 4): nothing stored, source not connected.
        assert_eq!(secrets.get("api_key.openrouter").unwrap(), None);
        assert!(!consent.is_enabled(&openrouter()).unwrap());
    }

    #[tokio::test]
    async fn apply_api_key_fails_closed_when_verification_is_unreachable() {
        let secrets = MemSecrets::default();
        let consent = FakeConsent::default();
        apply_api_key(&secrets, &consent, &FakeHttp(Err(())), &openrouter(), "k")
            .await
            .expect_err("an unverifiable key must not connect");
        assert_eq!(secrets.get("api_key.openrouter").unwrap(), None);
        assert!(!consent.is_enabled(&openrouter()).unwrap());
    }

    #[tokio::test]
    async fn apply_api_key_refuses_a_non_api_key_source() {
        let secrets = MemSecrets::default();
        let consent = FakeConsent::default();
        // claude-code is a LocalLogin source; a 200 fake proves the guard runs *before* (and
        // instead of) any validation or storage.
        apply_api_key(
            &secrets,
            &consent,
            &FakeHttp(Ok(200)),
            &ProviderId::new("claude-code"),
            "x",
        )
        .await
        .expect_err("local-login sources reject set_api_key");
        assert!(secrets.0.lock().unwrap().is_empty());
        assert!(!consent.is_enabled(&ProviderId::new("claude-code")).unwrap());
    }

    #[tokio::test]
    async fn apply_api_key_refuses_an_unknown_source() {
        let secrets = MemSecrets::default();
        let consent = FakeConsent::default();
        let err = apply_api_key(
            &secrets,
            &consent,
            &FakeHttp(Ok(200)),
            &ProviderId::new("ghost"),
            "x",
        )
        .await
        .expect_err("unknown sources are refused");
        assert!(err.to_lowercase().contains("unknown"), "clear error: {err}");
        assert!(secrets.0.lock().unwrap().is_empty());
    }

    #[test]
    fn disconnect_purges_an_api_key_sources_secret_and_consent() {
        let secrets = MemSecrets::default();
        secrets.set("api_key.openrouter", "sk-or-v1-good").unwrap();
        let consent = FakeConsent::default();
        consent.set_enabled(&openrouter(), true).unwrap();
        let identity = MemIdentity::default();

        let catalog = source_catalog();
        let descriptor = find_source(&catalog, &openrouter()).unwrap();
        disconnect(&secrets, &consent, &identity, descriptor).expect("disconnect succeeds");

        assert_eq!(secrets.get("api_key.openrouter").unwrap(), None);
        assert!(!consent.is_enabled(&openrouter()).unwrap());
    }

    #[test]
    fn disconnect_purges_a_reused_logins_cached_oauth_copy_and_identity() {
        // The task-004 fix: disconnecting a reused-login source (Claude Code) must remove the
        // refreshed-OAuth copy MLT cached under its OWN service — not merely drop consent.
        let claude = ProviderId::new("claude-code");
        let secrets = MemSecrets::default();
        secrets
            .set("oauth.claude", "{\"access_token\":\"x\"}")
            .unwrap();
        let consent = FakeConsent::default();
        consent.set_enabled(&claude, true).unwrap();
        let identity = MemIdentity::default();
        identity
            .set_identity(
                &claude,
                &AccountIdentity {
                    email: Some("dev@example.com".into()),
                    organization: None,
                },
            )
            .unwrap();

        let catalog = source_catalog();
        let descriptor = find_source(&catalog, &claude).unwrap();
        disconnect(&secrets, &consent, &identity, descriptor).expect("disconnect succeeds");

        // Acceptance 3: the cached secret is gone from the keychain…
        assert_eq!(
            secrets.get("oauth.claude").unwrap(),
            None,
            "the cached OAuth copy must be purged"
        );
        // …consent is cleared so the tile disappears and refresh stops…
        assert!(!consent.is_enabled(&claude).unwrap());
        // …and the provider-fetched identity is forgotten, so a reconnect re-resolves it fresh.
        assert_eq!(identity.identity(&claude).unwrap(), None);
    }

    #[test]
    fn disconnect_is_idempotent_and_scoped_to_this_sources_own_keys() {
        let secrets = MemSecrets::default();
        // Another source's secret must survive disconnecting Claude — disconnect only ever
        // removes the keys WE cached for the source being disconnected.
        secrets.set("api_key.openrouter", "sk-or-v1-keep").unwrap();
        let consent = FakeConsent::default();
        let identity = MemIdentity::default();

        let catalog = source_catalog();
        let claude = find_source(&catalog, &ProviderId::new("claude-code")).unwrap();
        // Nothing of Claude's is stored — disconnect must be a no-op success, not an error.
        disconnect(&secrets, &consent, &identity, claude).expect("idempotent disconnect");

        assert!(!consent.is_enabled(&ProviderId::new("claude-code")).unwrap());
        assert_eq!(
            secrets.get("api_key.openrouter").unwrap().as_deref(),
            Some("sk-or-v1-keep"),
            "another source's secret is left untouched"
        );
    }
}
