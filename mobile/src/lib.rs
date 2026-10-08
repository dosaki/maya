//! Maya for Android: a main Maya with no sessions of its own. The server,
//! pairing, merging and routing are `maya_core`'s; this crate is the Tauri
//! commands the page calls, each routed to an assistant, and the Android
//! glue: the foreground service and the notifications.

pub mod alerts;
pub mod android;
mod commands;
pub mod hub;

use hub::{Hub, Sink};
use maya_core::config::NetworkRole;
use maya_core::log;
use maya_core::model::Card;
use maya_core::net::NetworkStatus;
use std::sync::{Arc, Weak};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// The hub's way out: events to the page, and the foreground service.
struct TauriSink {
    app: AppHandle,
}

impl Sink for TauriSink {
    fn sessions(&self, cards: &[Card]) {
        let _ = self.app.emit("sessions", cards);
    }

    fn network(&self, status: &NetworkStatus) {
        let _ = self.app.emit("network", status);
    }

    fn service_start(&self, line: &str) {
        android::service_start(&self.app, line);
    }

    fn service_stop(&self) {
        android::service_stop(&self.app);
    }
}

/// How often the boards are re-merged with no new frame, so an assistant
/// that went away greys and, after the core's five minutes, leaves the
/// board and takes its notifications with it.
const REFRESH_EVERY: Duration = Duration::from_secs(10);

/// Re-merges the boards every `REFRESH_EVERY` for as long as the hub lives.
fn spawn_refresh(hub: Weak<Hub>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(REFRESH_EVERY);
        match hub.upgrade() {
            Some(hub) => hub.refresh(),
            None => break,
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(android::init())
        .invoke_handler(tauri::generate_handler![
            commands::list_sessions,
            commands::session_history,
            commands::send_reply,
            commands::answer_question,
            commands::set_session_option,
            commands::cycle_session_mode,
            commands::send_slash_command,
            commands::rename_session,
            commands::compact_session,
            commands::close_session,
            commands::save_attachment,
            commands::open_url,
            commands::open_pr,
            commands::list_machines,
            commands::list_project_dirs,
            commands::list_agents,
            commands::list_resumable_sessions,
            commands::resume_session,
            commands::start_session,
            commands::network_status,
            commands::network_pairing_code,
            commands::network_remove_assistant,
            commands::get_config,
            commands::set_config,
            commands::log_lines,
            commands::log_clear,
            commands::log_path,
            commands::server_start,
            commands::server_stop,
            commands::local_addresses,
            commands::request_battery_exemption,
            commands::notifications_allowed,
            commands::pending_tap
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let maya_dir = app.path().app_data_dir().map_err(|e| e.to_string())?.join("maya");
            if let Err(e) = log::init(&maya_dir.join("maya.log")) {
                eprintln!("{e}");
            }
            let emitter = handle.clone();
            log::install_emitter(move |line| {
                let _ = emitter.emit("log", line);
            });
            let settings = hub::load_settings(&maya_dir, &android::device_model(&handle));
            let hub = Hub::new(maya_dir, settings, android::alerts(&handle), Arc::new(TauriSink { app: handle.clone() }));
            app.manage(hub.clone());
            spawn_refresh(Arc::downgrade(&hub));
            if let Err(e) = android::create_channels(&handle) {
                log::line("android", format!("notification channels: {e}"));
            }
            android::clear_leftovers(&handle);
            log::line("app", format!("started Maya {} for Android", env!("CARGO_PKG_VERSION")));
            // The server comes back on its own when it was running last time.
            let was_main = hub.settings.lock().unwrap().config.network.role == NetworkRole::Main;
            if was_main {
                let hub = hub.clone();
                std::thread::spawn(move || {
                    let _ = hub.start_server();
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Maya");
}
