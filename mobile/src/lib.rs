//! Maya for Android: a phone that joins a main Maya and mirrors its board.
//! The client, pairing and routing are `maya_core`'s; this crate is the
//! Tauri commands the page calls, and the Android glue: the foreground
//! service and the notifications.

pub mod alerts;
pub mod android;
mod commands;
pub mod hub;
pub mod voice;

use hub::{Hub, Sink};
use maya_core::config::NetworkRole;
use maya_core::log;
use maya_core::model::Card;
use maya_core::net::protocol::Request;
use maya_core::net::NetworkStatus;
use maya_core::store::now_ms;
use serde_json::Value;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

/// The hub's way out: events to the page, and the foreground service.
struct TauriSink {
    app: AppHandle,
}

impl Sink for TauriSink {
    fn sessions(&self, cards: &[Card]) {
        let _ = self.app.emit("sessions", cards);
    }

    fn remembered(&self, rows: &[maya_core::remembered::RememberedRow]) {
        let _ = self.app.emit("remembered", rows);
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

    fn chat(&self, turn: &Value) {
        let _ = self.app.emit("chat", turn);
    }

    fn brain(&self, status: &Value) {
        let _ = self.app.emit("brain", status);
    }

    fn chat_reset(&self, turns: &[Value]) {
        let _ = self.app.emit("chat-reset", turns);
    }
}

/// The phone's voice conversation: it asks the joined main, shows itself
/// to the page as the `voice` event, and hears the voice plugin's events.
fn make_voice(handle: &AppHandle, hub: &Arc<Hub>) {
    let ask_hub = Arc::downgrade(hub);
    let ask: voice::Ask = Arc::new(move |r: Request| -> Result<Option<Value>, String> { ask_hub.upgrade().ok_or("Maya is closing.")?.request(None, r) });
    let shown = handle.clone();
    let show: voice::Show = Arc::new(move |v: &voice::VoiceView| {
        let _ = shown.emit("voice", v);
    });
    let v = voice::Voice::new(android::voice_io(handle), ask, show, now_ms);
    // Read first: the settings lock is not held while the voice runs.
    let wake = hub.settings.lock().unwrap().config.wake_word;
    v.set_wake(wake);
    let word = hub.link.lock().unwrap().wake_word.clone();
    v.set_wake_word(&maya_core::wake::wake_word(word.as_deref()));
    let _ = hub.voice.set(v.clone());
    let quiet = handle.clone();
    let _ = hub.silenced.set(Box::new(move || android::silenced(&quiet)));
    // A board event that waited behind a conversation is spoken only if
    // Speak is still on and the phone not silenced by then. The voice asks
    // with its lock released, on a thread of its own.
    let gate_hub = Arc::downgrade(hub);
    v.set_event_gate(Arc::new(move || gate_hub.upgrade().is_some_and(|h| h.events_may_speak())));
    android::watch_voice(handle, Arc::downgrade(&v));
    if !android::on_device(handle) {
        v.set_note(Some("Speech goes through Google's service and may leave the phone.".into()));
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(android::init())
        .plugin(android::voice_init())
        .invoke_handler(tauri::generate_handler![
            commands::list_sessions,
            commands::session_history,
            commands::send_reply,
            commands::live_attach_remote,
            commands::live_input_remote,
            commands::live_detach_remote,
            commands::answer_question,
            commands::answer_pick,
            commands::stop_session,
            commands::set_session_option,
            commands::cycle_session_mode,
            commands::send_slash_command,
            commands::rename_session,
            commands::compact_session,
            commands::close_session,
            commands::save_attachment,
            commands::open_url,
            commands::update_kind,
            commands::update_check,
            commands::open_pr,
            commands::list_machines,
            commands::list_project_dirs,
            commands::list_agents,
            commands::list_resumable_sessions,
            commands::resume_session,
            commands::remembered_sessions,
            commands::resume_remembered,
            commands::forget_remembered,
            commands::start_session,
            commands::network_status,
            commands::network_pair,
            commands::network_new_key,
            commands::listen_stop,
            commands::get_config,
            commands::set_config,
            commands::log_lines,
            commands::log_clear,
            commands::log_path,
            commands::local_addresses,
            commands::request_battery_exemption,
            commands::notifications_allowed,
            commands::pending_tap,
            commands::voice_view,
            commands::voice_tap,
            commands::voice_cancel,
            commands::voice_open_settings,
            commands::chat_history,
            commands::brain_status,
            commands::chat_send,
            commands::chat_seen,
            commands::window_cancel,
            commands::brain_stop
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
            make_voice(&handle, &hub);
            android::watch_buttons(&handle, Arc::downgrade(&hub));
            if let Err(e) = android::create_channels(&handle) {
                log::line("android", format!("notification channels: {e}"));
            }
            android::clear_leftovers(&handle);
            log::line("app", format!("started Maya {} for Android", env!("CARGO_PKG_VERSION")));
            let was_assistant = hub.settings.lock().unwrap().config.network.role == NetworkRole::Assistant;
            if was_assistant {
                hub.start_client();
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Maya");
}
