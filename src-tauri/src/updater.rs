//! Updating Maya from the release feed: Tauri's updater where it can
//! replace the app (macOS, Windows, the AppImage), the core's notice where
//! it cannot (a `.deb`). The page asks `update_kind` once, then
//! `update_check` at start and every 6 hours.

use std::ffi::OsStr;
use std::sync::Mutex;

use maya_core::log;
use maya_core::update::{self, UpdateInfo};
use serde::Serialize;
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

/// How this install updates: `off` in a debug build, `notice` on Linux
/// outside an AppImage (the AppImage runtime sets `$APPIMAGE`), `self`
/// otherwise.
pub fn kind_for(debug: bool, linux: bool, appimage: Option<&OsStr>) -> &'static str {
    if debug {
        "off"
    } else if linux && appimage.is_none_or(|p| p.is_empty()) {
        "notice"
    } else {
        "self"
    }
}

fn kind() -> &'static str {
    kind_for(cfg!(debug_assertions), cfg!(target_os = "linux"), std::env::var_os("APPIMAGE").as_deref())
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
/// `update-progress`, then relaunches Maya. Sessions live in their
/// terminals, so the relaunch ends none of them.
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
    use std::ffi::OsStr;

    #[test]
    fn a_debug_build_never_updates() {
        assert_eq!(kind_for(true, false, None), "off");
        assert_eq!(kind_for(true, true, Some(OsStr::new("/home/u/.local/bin/maya-app"))), "off");
    }

    #[test]
    fn linux_updates_itself_only_from_an_appimage() {
        assert_eq!(kind_for(false, true, Some(OsStr::new("/home/u/.local/bin/maya-app"))), "self");
        assert_eq!(kind_for(false, true, None), "notice");
        assert_eq!(kind_for(false, true, Some(OsStr::new(""))), "notice");
    }

    #[test]
    fn macos_and_windows_update_themselves() {
        assert_eq!(kind_for(false, false, None), "self");
    }
}
