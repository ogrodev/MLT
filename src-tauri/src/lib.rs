// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
use mlt_adapters::SystemClock;
use mlt_core::domain::{ProviderId, UsageSnapshot};
use mlt_core::ports::Clock;
use mlt_core::providers::{FetchContext, FetchStrategy};

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

/// Smoke command proving the hexagonal wiring: time comes through core's `Clock` port,
/// satisfied by the `SystemClock` adapter — never `SystemTime::now()` directly.
#[tauri::command]
fn core_now() -> i64 {
    SystemClock.now().0
}

/// Fetch the current Claude Code subscription usage (session / weekly / model windows).
/// Reads the local Claude OAuth token and polls `api/oauth/usage` via the provider slice.
#[tauri::command]
async fn fetch_claude_usage() -> Result<UsageSnapshot, String> {
    let strategy = mlt_adapters::claude_strategy();
    let ctx = FetchContext { provider: ProviderId::new("claude-code") };
    strategy.fetch(&ctx).await.map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![greet, core_now, fetch_claude_usage])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
