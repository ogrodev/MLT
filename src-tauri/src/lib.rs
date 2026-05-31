// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
use mlt_adapters::SystemClock;
use mlt_core::ports::Clock;

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

/// Smoke command proving the hexagonal wiring: the app obtains time through core's
/// `Clock` port, satisfied by the `SystemClock` adapter — never `SystemTime::now()` directly.
#[tauri::command]
fn core_now() -> i64 {
    SystemClock.now().0
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![greet, core_now])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
