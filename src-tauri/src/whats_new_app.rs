//! What's new after an update, on the computer: the page asks once at
//! start what to show (`maya_core::whats_new` decides) and says when it
//! was closed.

use maya_core::whats_new::{at_start, Offer};
use tauri::{AppHandle, State};

use crate::AppState;

const RUNNING: &str = env!("CARGO_PKG_VERSION");

/// The window to show at start, if any. A brand-new install shows none
/// and saves the version it started on.
#[tauri::command]
pub fn whats_new(app: AppHandle, state: State<AppState>) -> Result<Option<Offer>, String> {
    let saved = state.store.lock().unwrap().config.last_shown_version.clone();
    let (offer, save) = at_start(saved.as_deref(), RUNNING, state.config_existed);
    if let Some(v) = save {
        crate::update_config(&app, |c| c.last_shown_version = Some(v))?;
    }
    Ok(offer)
}

/// The window was closed: this version's notes have been shown.
#[tauri::command]
pub fn whats_new_seen(app: AppHandle) -> Result<(), String> {
    crate::update_config(&app, |c| c.last_shown_version = Some(RUNNING.into())).map(|_| ())
}
