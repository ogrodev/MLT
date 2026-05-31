// MLT app crate: tray + chromeless popover, wired to the provider slice in mlt-adapters.
use mlt_core::domain::{ProviderId, UsageSnapshot};
use mlt_core::providers::{FetchContext, FetchStrategy};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager};
use tauri_plugin_positioner::{Position, WindowExt};

const POPOVER: &str = "main";
const REFRESH_SECS: u64 = 60;

/// Fetch the current Claude Code subscription usage on demand (called by the UI on open).
#[tauri::command]
async fn fetch_claude_usage() -> Result<UsageSnapshot, String> {
    claude_usage().await.map_err(|e| e.to_string())
}

async fn claude_usage() -> Result<UsageSnapshot, mlt_core::providers::FetchError> {
    let strategy = mlt_adapters::claude_strategy();
    let ctx = FetchContext {
        provider: ProviderId::new("claude-code"),
    };
    strategy.fetch(&ctx).await
}

/// Background poll loop: refresh usage on a cadence and emit events the popover listens to.
fn spawn_refresh_loop(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            match claude_usage().await {
                Ok(snapshot) => {
                    let _ = app.emit("usage-updated", snapshot);
                }
                Err(e) => {
                    let _ = app.emit("usage-error", e.to_string());
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(REFRESH_SECS)).await;
        }
    });
}

/// Show the popover anchored under the tray icon, or hide it if already open.
fn toggle_popover(app: &tauri::AppHandle) {
    let Some(win) = app.get_webview_window(POPOVER) else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        let _ = win.hide();
    } else {
        let _ = win.move_window(Position::TrayBottomCenter);
        let _ = win.show();
        let _ = win.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_positioner::init())
        .invoke_handler(tauri::generate_handler![fetch_claude_usage])
        .setup(|app| {
            // Menu-bar app: no Dock icon / app-switcher entry on macOS.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            // Tray icon — click toggles the popover. Template image adapts to the menu bar.
            TrayIconBuilder::with_id("mlt-tray")
                .icon(tauri::include_image!("icons/32x32.png"))
                .icon_as_template(true)
                .tooltip("MLT — AI usage")
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

            // Click-outside-to-dismiss: hide the popover when it loses focus.
            if let Some(win) = app.get_webview_window(POPOVER) {
                let w = win.clone();
                win.on_window_event(move |event| {
                    if let tauri::WindowEvent::Focused(false) = event {
                        let _ = w.hide();
                    }
                });
            }

            spawn_refresh_loop(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
