//! Maya for Android: a main Maya with no sessions of its own. The server,
//! pairing, merging and routing are `maya_core`'s; this crate is the Tauri
//! commands the page calls, each routed to an assistant, and the Android
//! glue: the foreground service and the notifications.

pub mod alerts;
pub mod android;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(android::init())
        .run(tauri::generate_context!())
        .expect("error while running Maya");
}
