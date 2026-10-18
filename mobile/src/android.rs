//! The Android side: the `keepalive` plugin (the foreground service, the
//! battery exemption, the device's name and addresses), the `voice` plugin
//! (the recogniser, the voice, the screen and quiet states) and the Android
//! notifications. On any other OS, which only happens when the crate is
//! run on a desktop for development, every call is a logged no-op.

use crate::alerts::{Alerts, Post};
use maya_core::log;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::plugin::{Builder, TauriPlugin};
use tauri::{AppHandle, Runtime};

#[allow(dead_code)]
#[derive(Serialize)]
struct LineArgs<'a> {
    line: &'a str,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct BoolReply {
    value: bool,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct TextReply {
    value: String,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct ListReply {
    value: Vec<String>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct MaybeIdReply {
    value: Option<i32>,
}

/// The Kotlin `KeepAlivePlugin`, once registered.
#[cfg(target_os = "android")]
struct KeepAlive<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// The app's own mobile plugin: on Android it loads `KeepAlivePlugin` from
/// the app's package; elsewhere it registers nothing.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("keepalive")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                let handle = api.register_android_plugin("com.dosaki.maya.mobile", "KeepAlivePlugin")?;
                app.manage(KeepAlive(handle));
            }
            #[cfg(not(target_os = "android"))]
            let _ = (app, api);
            Ok(())
        })
        .build()
}

/// The Kotlin `VoicePlugin`, once registered.
#[cfg(target_os = "android")]
struct VoiceHandle<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// The voice plugin: Android's recogniser and voice, the on-screen state
/// and the quiet state. Its events go straight to the conversation through
/// the channel `watch_voice` hands it; the page never calls it.
pub fn voice_init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("voice")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                let handle = api.register_android_plugin("com.dosaki.maya.mobile", "VoicePlugin")?;
                app.manage(VoiceHandle(handle));
            }
            #[cfg(not(target_os = "android"))]
            let _ = (app, api);
            Ok(())
        })
        .build()
}

#[cfg(target_os = "android")]
mod imp {
    use super::*;
    use tauri::Manager;
    use tauri_plugin_notification::{Channel, Importance, NotificationExt};

    fn call<R: Runtime, T: serde::de::DeserializeOwned>(app: &AppHandle<R>, command: &str, args: impl Serialize) -> Result<T, String> {
        app.state::<KeepAlive<R>>().0.run_mobile_plugin(command, args).map_err(|e| e.to_string())
    }

    fn unit<R: Runtime>(app: &AppHandle<R>, command: &str, args: impl Serialize) {
        if let Err(e) = call::<R, serde_json::Value>(app, command, args) {
            log::line("android", format!("{command}: {e}"));
        }
    }

    pub fn service_start<R: Runtime>(app: &AppHandle<R>, line: &str) {
        unit(app, "start", LineArgs { line })
    }

    pub fn service_update<R: Runtime>(app: &AppHandle<R>, line: &str) {
        unit(app, "update", LineArgs { line })
    }

    pub fn service_stop<R: Runtime>(app: &AppHandle<R>) {
        unit(app, "stop", ())
    }

    pub fn request_battery_exemption<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
        call::<R, serde_json::Value>(app, "requestBatteryExemption", ()).map(|_| ())
    }

    pub fn notifications_allowed<R: Runtime>(app: &AppHandle<R>) -> bool {
        call::<R, BoolReply>(app, "notificationsAllowed", ()).map(|r| r.value).unwrap_or(true)
    }

    /// The id of the last tapped notification, once.
    pub fn pending_tap<R: Runtime>(app: &AppHandle<R>) -> Option<i32> {
        match call::<R, MaybeIdReply>(app, "pendingTap", ()) {
            Ok(r) => r.value,
            Err(e) => {
                log::line("android", format!("pendingTap: {e}"));
                None
            }
        }
    }

    pub fn device_model<R: Runtime>(app: &AppHandle<R>) -> String {
        call::<R, TextReply>(app, "deviceModel", ()).map(|r| r.value).unwrap_or_default()
    }

    pub fn local_addresses<R: Runtime>(app: &AppHandle<R>) -> Vec<String> {
        call::<R, ListReply>(app, "localAddresses", ()).map(|r| r.value).unwrap_or_default()
    }

    /// Clears the notifications an earlier run left behind (`alerts::leftovers`).
    pub fn clear_leftovers<R: Runtime>(app: &AppHandle<R>) {
        let n = app.notification();
        let active = match n.active() {
            Ok(active) => active,
            Err(e) => {
                log::line("android", format!("active notifications: {e}"));
                return;
            }
        };
        let ids = crate::alerts::leftovers(&active.iter().map(|a| a.id()).collect::<Vec<_>>());
        if ids.is_empty() {
            return;
        }
        if let Err(e) = n.remove_active(ids) {
            log::line("android", format!("clear leftover notifications: {e}"));
        }
    }

    /// The channels the switches and speech post on; creating an existing channel is a no-op on Android.
    pub fn create_channels<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
        let n = app.notification();
        n.create_channel(Channel::builder(crate::alerts::Kind::Decision.channel_id(), "Decisions").description("A session needs a decision").importance(Importance::High).vibration(true).build()).map_err(|e| e.to_string())?;
        n.create_channel(Channel::builder(crate::alerts::Kind::Finished.channel_id(), "Finished").description("A session finished its turn").importance(Importance::Default).build()).map_err(|e| e.to_string())?;
        n.create_channel(Channel::builder(crate::alerts::SPOKEN_CHANNEL, "Spoken").description("A session Maya just announced aloud").importance(Importance::Low).vibration(false).build()).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// `res/drawable/ic_stat_maya.xml`: a white glyph, since Android draws
    /// status-bar icons as alpha masks and the launcher icon shows as a blob.
    const STATUS_ICON: &str = "ic_stat_maya";

    struct AndroidAlerts<R: Runtime> {
        app: AppHandle<R>,
    }

    impl<R: Runtime> Alerts for AndroidAlerts<R> {
        fn post(&self, p: &Post) {
            let shown = self.app.notification().builder().id(p.id).channel_id(p.channel_id()).icon(STATUS_ICON).title(&p.title).body(&p.body).extra("sessionId", &p.session_id).auto_cancel().show();
            if let Err(e) = shown {
                log::line("android", format!("notification: {e}"));
            }
        }

        fn clear(&self, id: i32) {
            if let Err(e) = self.app.notification().remove_active(vec![id]) {
                log::line("android", format!("clear notification {id}: {e}"));
            }
        }

        fn service_line(&self, line: &str) {
            service_update(&self.app, line)
        }
    }

    pub fn alerts<R: Runtime>(app: &AppHandle<R>) -> Arc<dyn Alerts> {
        Arc::new(AndroidAlerts { app: app.clone() })
    }

    #[derive(Serialize)]
    struct ListenArgs {
        wake: bool,
    }

    fn voice_call<R: Runtime>(app: &AppHandle<R>, command: &str, args: impl Serialize) {
        if let Err(e) = app.state::<VoiceHandle<R>>().0.run_mobile_plugin::<serde_json::Value>(command, args) {
            log::line("voice", format!("{command}: {e}"));
        }
    }

    /// One call to the plugin, run in order on the voice worker thread.
    enum Call {
        Listen(bool),
        StopListening,
        Speak(String),
        StopSpeaking,
    }

    /// `Voice` calls these with its lock held, and a plugin call blocks
    /// until Kotlin answers, while Kotlin may send an event (which takes
    /// that lock) before answering. So each call is queued to one worker
    /// thread, in order, and returns at once (ledger ruling R8).
    struct AndroidVoice {
        calls: std::sync::Mutex<std::sync::mpsc::Sender<Call>>,
    }

    impl AndroidVoice {
        fn send(&self, call: Call) {
            if self.calls.lock().unwrap().send(call).is_err() {
                log::line("voice", "the voice worker has stopped");
            }
        }
    }

    impl crate::voice::VoiceIo for AndroidVoice {
        fn listen(&self, wake: bool) {
            self.send(Call::Listen(wake))
        }
        fn stop_listening(&self) {
            self.send(Call::StopListening)
        }
        fn speak(&self, line: &str) {
            self.send(Call::Speak(line.to_string()))
        }
        fn stop_speaking(&self) {
            self.send(Call::StopSpeaking)
        }
    }

    /// The recogniser and the voice, through the plugin's worker thread.
    pub fn voice_io<R: Runtime>(app: &AppHandle<R>) -> Arc<dyn crate::voice::VoiceIo> {
        let (tx, rx) = std::sync::mpsc::channel::<Call>();
        let app = app.clone();
        std::thread::Builder::new()
            .name("voice-plugin".into())
            .spawn(move || {
                for call in rx {
                    match call {
                        Call::Listen(wake) => voice_call(&app, "listen", ListenArgs { wake }),
                        Call::StopListening => voice_call(&app, "stopListening", ()),
                        Call::Speak(line) => voice_call(&app, "speak", LineArgs { line: &line }),
                        Call::StopSpeaking => voice_call(&app, "stopSpeaking", ()),
                    }
                }
            })
            .expect("a thread for the voice plugin");
        Arc::new(AndroidVoice { calls: std::sync::Mutex::new(tx) })
    }

    /// On silent or vibrate, or in Do Not Disturb.
    pub fn silenced<R: Runtime>(app: &AppHandle<R>) -> bool {
        app.state::<VoiceHandle<R>>().0.run_mobile_plugin::<BoolReply>("quiet", ()).map(|r| r.value).unwrap_or(false)
    }

    #[derive(Serialize)]
    struct WatchArgs {
        channel: tauri::ipc::Channel<serde_json::Value>,
    }

    /// Hands the plugin a channel for its events, which go straight to the
    /// conversation. `Channel::new` registers itself with mobile plugins.
    pub fn watch_voice<R: Runtime>(app: &AppHandle<R>, voice: std::sync::Weak<crate::voice::Voice>) {
        let channel = tauri::ipc::Channel::<serde_json::Value>::new(move |body| {
            if let tauri::ipc::InvokeResponseBody::Json(json) = body {
                match (serde_json::from_str::<serde_json::Value>(&json), voice.upgrade()) {
                    (Ok(e), Some(v)) => crate::voice::dispatch(&v, &e),
                    (Err(e), _) => log::line("voice", format!("unreadable plugin event: {e}")),
                    _ => {}
                }
            }
            Ok(())
        });
        voice_call(app, "watch", WatchArgs { channel });
    }

    #[derive(Deserialize)]
    struct MicPermission {
        microphone: String,
    }

    #[derive(Serialize)]
    struct PermissionArgs {
        permissions: Vec<&'static str>,
    }

    /// Whether Maya may use the microphone, asking the user the first time.
    /// Blocks until they answer; call it from an async command.
    pub fn microphone_allowed<R: Runtime>(app: &AppHandle<R>) -> bool {
        // `inner` borrows for the app's lifetime, not the `State` temporary's.
        let h = &app.state::<VoiceHandle<R>>().inner().0;
        if h.run_mobile_plugin::<MicPermission>("checkPermissions", ()).is_ok_and(|p| p.microphone == "granted") {
            return true;
        }
        h.run_mobile_plugin::<MicPermission>("requestPermissions", PermissionArgs { permissions: vec!["microphone"] }).is_ok_and(|p| p.microphone == "granted")
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RecognizerState {
        on_device: bool,
    }

    /// The phone has an on-device recogniser (the wake word needs one).
    pub fn on_device<R: Runtime>(app: &AppHandle<R>) -> bool {
        app.state::<VoiceHandle<R>>().0.run_mobile_plugin::<RecognizerState>("state", ()).map(|s| s.on_device).unwrap_or(true)
    }

    /// Opens the app's page in Android's settings, where a refused
    /// microphone is allowed again. Never called with the voice's lock held.
    pub fn open_app_settings<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
        app.state::<VoiceHandle<R>>().0.run_mobile_plugin::<serde_json::Value>("openSettings", ()).map(|_| ()).map_err(|e| e.to_string())
    }
}

#[cfg(not(target_os = "android"))]
mod imp {
    use super::*;

    pub fn service_start<R: Runtime>(_: &AppHandle<R>, line: &str) {
        log::line("android", format!("service start: {line}"))
    }

    pub fn service_update<R: Runtime>(_: &AppHandle<R>, line: &str) {
        log::line("android", format!("service: {line}"))
    }

    pub fn service_stop<R: Runtime>(_: &AppHandle<R>) {
        log::line("android", "service stop")
    }

    pub fn request_battery_exemption<R: Runtime>(_: &AppHandle<R>) -> Result<(), String> {
        Ok(())
    }

    pub fn notifications_allowed<R: Runtime>(_: &AppHandle<R>) -> bool {
        true
    }

    pub fn pending_tap<R: Runtime>(_: &AppHandle<R>) -> Option<i32> {
        None
    }

    pub fn device_model<R: Runtime>(_: &AppHandle<R>) -> String {
        String::new()
    }

    pub fn local_addresses<R: Runtime>(_: &AppHandle<R>) -> Vec<String> {
        vec![]
    }

    pub fn clear_leftovers<R: Runtime>(_: &AppHandle<R>) {}

    pub fn create_channels<R: Runtime>(_: &AppHandle<R>) -> Result<(), String> {
        Ok(())
    }

    struct HostAlerts;

    impl Alerts for HostAlerts {
        fn post(&self, p: &Post) {
            log::line("android", format!("would notify: {} — {}", p.title, p.body))
        }

        fn clear(&self, id: i32) {
            log::line("android", format!("would clear notification {id}"))
        }

        fn service_line(&self, line: &str) {
            log::line("android", format!("service: {line}"))
        }
    }

    pub fn alerts<R: Runtime>(_: &AppHandle<R>) -> Arc<dyn Alerts> {
        Arc::new(HostAlerts)
    }

    struct HostVoice;

    impl crate::voice::VoiceIo for HostVoice {
        fn listen(&self, wake: bool) {
            log::line("voice", format!("would listen (wake word: {wake})"))
        }
        fn stop_listening(&self) {
            log::line("voice", "would stop listening")
        }
        fn speak(&self, line: &str) {
            log::line("voice", format!("would say: {line}"))
        }
        fn stop_speaking(&self) {
            log::line("voice", "would stop speaking")
        }
    }

    pub fn voice_io<R: Runtime>(_: &AppHandle<R>) -> Arc<dyn crate::voice::VoiceIo> {
        Arc::new(HostVoice)
    }

    pub fn silenced<R: Runtime>(_: &AppHandle<R>) -> bool {
        false
    }

    pub fn watch_voice<R: Runtime>(_: &AppHandle<R>, _: std::sync::Weak<crate::voice::Voice>) {}

    pub fn microphone_allowed<R: Runtime>(_: &AppHandle<R>) -> bool {
        true
    }

    pub fn on_device<R: Runtime>(_: &AppHandle<R>) -> bool {
        true
    }

    pub fn open_app_settings<R: Runtime>(_: &AppHandle<R>) -> Result<(), String> {
        log::line("voice", "would open the app's settings");
        Ok(())
    }
}

pub use imp::*;
