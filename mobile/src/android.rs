//! The Android side: the `keepalive` plugin (the foreground service, the
//! battery exemption, the device's name and addresses) and the Android
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

    /// The two channels the switches gate; creating an existing channel is a no-op on Android.
    pub fn create_channels<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
        let n = app.notification();
        n.create_channel(Channel::builder(crate::alerts::Kind::Decision.channel_id(), "Decisions").description("A session needs a decision").importance(Importance::High).vibration(true).build()).map_err(|e| e.to_string())?;
        n.create_channel(Channel::builder(crate::alerts::Kind::Finished.channel_id(), "Finished").description("A session finished its turn").importance(Importance::Default).build()).map_err(|e| e.to_string())
    }

    struct AndroidAlerts<R: Runtime> {
        app: AppHandle<R>,
    }

    impl<R: Runtime> Alerts for AndroidAlerts<R> {
        fn post(&self, p: &Post) {
            let shown = self.app.notification().builder().id(p.id).channel_id(p.kind.channel_id()).title(&p.title).body(&p.body).extra("sessionId", &p.session_id).auto_cancel().show();
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
}

pub use imp::*;
