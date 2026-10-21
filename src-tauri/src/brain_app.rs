//! Maya's own resident session as a board card.

use crate::AppState;
use maya_core::model::Card;
use maya_core::window::Via;
use tauri::{AppHandle, Emitter, Manager};

/// The brain's card, appended to the board the window sees. None until the
/// brain session exists.
pub(crate) fn card(_state: &AppState) -> Option<Card> {
    None
}

/// What the `brain` event carries; for now only the batch waiting in the
/// action window.
#[derive(Clone, serde::Serialize)]
struct BrainStatus {
    pending: Option<PendingView>,
}

#[derive(Clone, serde::Serialize)]
struct PendingView {
    say: String,
    /// When it runs; None until her announcement has been said.
    until: Option<u64>,
}

/// Tells the page what she has waiting. Takes the window lock: the caller
/// must hold none.
pub(crate) fn emit_status(app: &AppHandle) {
    let pending = app.state::<AppState>().window.lock().unwrap().pending().map(|b| PendingView { say: b.say.clone(), until: (b.until != u64::MAX).then_some(b.until) });
    let _ = app.emit("brain", BrainStatus { pending });
}

/// "Maya, stop": stops what her session is doing. Until her session exists
/// there is nothing to stop, and a spoken stop is told so.
pub(crate) fn stop(app: &AppHandle, via: Via) {
    if via == Via::Voice {
        crate::listener::say(app, "Nothing to stop.");
    }
}
