//! Updating Maya from the release feed: Tauri's updater where it can
//! replace the app (macOS, Windows, the AppImage), the core's notice where
//! it cannot (a `.deb`, an MSI install). The page asks `update_kind` once, then
//! `update_check` at start and every 6 hours.

use std::sync::Mutex;

use maya_core::log;
use maya_core::update::{self, UpdateInfo};
use serde::Serialize;
use tauri::utils::config::BundleType;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::{Update, UpdaterExt};

/// The update the last check found, for `update_install` to take.
#[derive(Default)]
pub struct Pending(pub Mutex<Option<Update>>);

#[derive(Clone, Serialize)]
struct Progress {
    received: u64,
    total: u64,
}

/// How this install updates: `off` in a debug build; `self` from the
/// macOS app, the NSIS install and the AppImage, which Tauri's updater
/// replaces; `notice` otherwise. An MSI install gets the notice because
/// the feed's Windows update is the NSIS installer, which would put a
/// second copy beside it. `bundle` is what Tauri stamped into the build
/// (`None` on macOS, which it does not stamp, and in an unbundled binary).
pub fn kind_for(debug: bool, os: &str, bundle: Option<BundleType>) -> &'static str {
    match (debug, os, bundle) {
        (true, _, _) => "off",
        (_, "macos", _) | (_, "windows", Some(BundleType::Nsis)) | (_, "linux", Some(BundleType::AppImage)) => "self",
        _ => "notice",
    }
}

fn kind() -> &'static str {
    kind_for(cfg!(debug_assertions), std::env::consts::OS, tauri::utils::platform::bundle_type())
}

#[tauri::command]
pub fn update_kind() -> String {
    kind().to_string()
}

/// The newer release, if any. For `self` installs it is also kept for
/// `update_install`.
#[tauri::command]
pub async fn update_check(app: AppHandle, pending: State<'_, Pending>) -> Result<Option<UpdateInfo>, String> {
    match kind() {
        "off" => Ok(None),
        "notice" => tauri::async_runtime::spawn_blocking(|| update::notice(env!("CARGO_PKG_VERSION"), update::release_page))
            .await
            .map_err(|e| e.to_string())?,
        _ => {
            let checked = match app.updater() {
                Ok(u) => u.check().await.map_err(|e| format!("could not check for updates: {e}")),
                Err(e) => Err(format!("the updater is not set up: {e}")),
            };
            let found = checked.inspect_err(|e| log::line("update", e.clone()))?;
            let info = found.as_ref().map(|u| UpdateInfo { version: u.version.clone(), download: update::release_page(&u.version) });
            *pending.0.lock().unwrap() = found;
            Ok(info)
        }
    }
}

/// Downloads and installs the update the last check found, reporting
/// `update-progress`, then relaunches Maya. Sessions in external terminals
/// live on; Native sessions end, which the window asks about before it calls
/// this.
#[tauri::command]
pub async fn update_install(app: AppHandle, pending: State<'_, Pending>) -> Result<(), String> {
    let Some(found) = pending.0.lock().unwrap().take() else {
        return Err("No update is waiting; check again.".into());
    };
    log::line("update", format!("installing Maya {}", found.version));
    let emitter = app.clone();
    let mut received = 0u64;
    found
        .download_and_install(
            move |chunk, total| {
                received += chunk as u64;
                let _ = emitter.emit("update-progress", Progress { received, total: total.unwrap_or(0) });
            },
            || {},
        )
        .await
        .map_err(|e| {
            let m = format!("the update failed: {e}");
            log::line("update", m.clone());
            m
        })?;
    log::line("update", "installed; relaunching");
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::kind_for;
    use tauri::utils::config::BundleType;

    #[test]
    fn a_debug_build_never_updates() {
        assert_eq!(kind_for(true, "macos", None), "off");
        assert_eq!(kind_for(true, "linux", Some(BundleType::AppImage)), "off");
    }

    #[test]
    fn macos_updates_itself() {
        assert_eq!(kind_for(false, "macos", None), "self");
    }

    #[test]
    fn linux_updates_itself_only_from_an_appimage() {
        assert_eq!(kind_for(false, "linux", Some(BundleType::AppImage)), "self");
        assert_eq!(kind_for(false, "linux", Some(BundleType::Deb)), "notice");
        assert_eq!(kind_for(false, "linux", Some(BundleType::Rpm)), "notice");
        assert_eq!(kind_for(false, "linux", None), "notice");
    }

    #[test]
    fn windows_updates_itself_only_from_the_nsis_install() {
        // The feed's Windows update is the NSIS installer, which beside an
        // MSI install would put a second copy.
        assert_eq!(kind_for(false, "windows", Some(BundleType::Nsis)), "self");
        assert_eq!(kind_for(false, "windows", Some(BundleType::Msi)), "notice");
        assert_eq!(kind_for(false, "windows", None), "notice");
    }
}
