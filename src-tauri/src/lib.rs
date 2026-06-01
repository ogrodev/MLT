// MLT app crate: tray + chromeless popover, wired to the provider slice in mlt-adapters.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mlt_adapters::{FileConsentStore, LocalSourceProbe};
use mlt_core::domain::{ProviderId, UsageSnapshot};
use mlt_core::ports::{ConsentStore, SourceProbe};
use mlt_core::providers::{FetchContext, FetchStrategy};
use mlt_core::sources::{active_sources, discover_sources, source_catalog, SourceState};
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
    claude_usage().await.map_err(|e| e.to_string())
}

/// Discover every known source for the connect screen: presence (metadata only) + the user's
/// stored opt-in. Reads no secret.
#[tauri::command]
async fn list_sources(sources: tauri::State<'_, AppSources>) -> Result<Vec<SourceState>, String> {
    discover_sources(
        &source_catalog(),
        sources.probe.as_ref(),
        sources.consent.as_ref(),
    )
    .await
    .map_err(|e| e.to_string())
}

/// Enable or disable a source. Takes effect immediately and without a restart: the choice is
/// persisted, and opting in kicks an off-thread refresh so the popover fills in right away
/// (rather than waiting for the next poll). Returns the refreshed source list for the UI.
#[tauri::command]
async fn set_source_enabled(
    app: tauri::AppHandle,
    sources: tauri::State<'_, AppSources>,
    id: String,
    enabled: bool,
) -> Result<Vec<SourceState>, String> {
    let id = ProviderId::new(id);
    sources
        .consent
        .set_enabled(&id, enabled)
        .map_err(|e| e.to_string())?;

    if enabled {
        let app = app.clone();
        let probe = sources.probe.clone();
        let consent = sources.consent.clone();
        tauri::async_runtime::spawn(async move {
            refresh_active(&app, probe.as_ref(), consent.as_ref()).await;
        });
    }

    discover_sources(
        &source_catalog(),
        sources.probe.as_ref(),
        sources.consent.as_ref(),
    )
    .await
    .map_err(|e| e.to_string())
}

/// Quit the whole app. Both the popover footer and the tray menu route here.
#[tauri::command]
fn quit(app: tauri::AppHandle) {
    app.exit(0);
}

async fn claude_usage() -> Result<UsageSnapshot, mlt_core::providers::FetchError> {
    let strategy = mlt_adapters::claude_strategy();
    let ctx = FetchContext {
        provider: ProviderId::new("claude-code"),
    };
    strategy.fetch(&ctx).await
}

/// Fetch one source's usage, or `None` for a source with no fetch wired yet. The map of
/// id → fetcher is the single place a connected source becomes a network call.
async fn fetch_for(
    id: &ProviderId,
) -> Option<Result<UsageSnapshot, mlt_core::providers::FetchError>> {
    match id.as_str() {
        "claude-code" => Some(claude_usage().await),
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
) {
    let Ok(ids) = active_sources(&source_catalog(), probe, consent).await else {
        return;
    };
    for id in &ids {
        match fetch_for(id).await {
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
) {
    tauri::async_runtime::spawn(async move {
        loop {
            refresh_active(&app, probe.as_ref(), consent.as_ref()).await;
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
            app.manage(AppSources {
                probe: probe.clone(),
                consent: consent.clone(),
            });

            spawn_refresh_loop(app.handle().clone(), probe, consent);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::{popover_action, PopoverAction, REOPEN_DEBOUNCE};
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
}
