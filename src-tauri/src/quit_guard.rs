//! Asking before a quit stops a working Native session: those terminals
//! are Maya's, so they end when she does. A quit (Cmd+Q, closing the
//! window) is held while the page asks; "Quit anyway" calls
//! `quit_confirmed`, whose exit then goes through. An update's restart is
//! asked about by the page before `update_install`.
//!
//! On macOS the app menu's predefined Quit sends AppKit's `terminate:`,
//! which tao turns straight into `RunEvent::Exit` with no `ExitRequested`
//! to hold, so the menu's Quit is Maya's own item, which asks for an exit.

use std::sync::atomic::{AtomicBool, Ordering};

use maya_core::log;
use maya_core::model::{Card, State};
use tauri::{AppHandle, Emitter, Manager};

use crate::AppState;

/// Set once the user said "Quit anyway", so the exit it starts goes through.
static CONFIRMED: AtomicBool = AtomicBool::new(false);
/// Set when macOS logs out, restarts or shuts down: the system is ending
/// Maya, and a question would only hold that up.
static SYSTEM_ENDING: AtomicBool = AtomicBool::new(false);

/// The id of the macOS app menu's Quit item.
#[cfg(target_os = "macos")]
pub const QUIT_ID: &str = "maya-quit";

/// The names of the sessions a quit would stop while they work: this
/// machine's Native sessions that are Working. Maya's own brain is not one.
pub fn working_native(cards: &[Card]) -> Vec<String> {
    cards.iter().filter(|c| c.native && c.state == State::Working && c.machine.is_none() && !c.brain).map(|c| c.name.clone()).collect()
}

/// Whether to ask before the quit goes on: something would stop, nobody
/// has already said yes, and the system is not the one ending Maya.
pub fn should_ask(confirmed: bool, system_ending: bool, working: &[String]) -> bool {
    !confirmed && !system_ending && !working.is_empty()
}

/// Whether an exit with this code was already agreed to. Tauri's restart
/// code comes from an update, which the page asked about before installing
/// (and which `ExitRequestApi::prevent_exit` would not stop). No code means
/// the last window has gone: its close was asked about, and no window is
/// left to ask in. Any other code is an exit Maya asked for (Cmd+Q, "Quit
/// anyway"), which goes through the check.
pub fn agreed(code: Option<i32>) -> bool {
    match code {
        None => true,
        Some(c) => c == tauri::RESTART_EXIT_CODE,
    }
}

/// The working Native sessions, from a fresh read of the store. Only the
/// read holds its lock: no terminal names, no announcements.
fn working_now(app: &AppHandle) -> Vec<String> {
    let cards = app.state::<AppState>().store.lock().unwrap().refresh(crate::now_ms());
    working_native(&cards)
}

/// Asks the page whether to quit when a quit would stop a working Native
/// session: brings the window forward and emits `quit-confirm`. True when
/// it asked, so the caller holds the quit.
pub fn ask_if_needed(app: &AppHandle, agreed: bool) -> bool {
    let confirmed = agreed || CONFIRMED.load(Ordering::SeqCst);
    let ending = SYSTEM_ENDING.load(Ordering::SeqCst);
    // The store is read only when the answer could be yes.
    let working = if confirmed || ending { Vec::new() } else { working_now(app) };
    if !should_ask(confirmed, ending, &working) {
        return false;
    }
    log::line("app", format!("quit held: {} Native sessions are working", working.len()));
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    let _ = app.emit("quit-confirm", serde_json::json!({ "names": working }));
    true
}

/// The working Native sessions' names, for the page to ask about before an
/// update's restart.
#[tauri::command(async)]
pub fn native_working(state: tauri::State<'_, AppState>) -> Vec<String> {
    working_native(&state.store.lock().unwrap().refresh(crate::now_ms()))
}

/// The user said "Quit anyway": the exit goes through without asking again.
#[tauri::command]
pub fn quit_confirmed(app: AppHandle) {
    CONFIRMED.store(true, Ordering::SeqCst);
    app.exit(0);
}

/// The default menu with its Quit swapped for Maya's own, which asks for an
/// exit (and so passes the check) instead of terminating AppKit outright.
#[cfg(target_os = "macos")]
pub fn menu(app: &AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{Menu, MenuItem, MenuItemKind};
    let menu = Menu::default(app)?;
    // The app's own submenu comes first, with the predefined Quit in it.
    if let Some(MenuItemKind::Submenu(sub)) = menu.items()?.into_iter().next() {
        for (at, item) in sub.items()?.into_iter().enumerate() {
            if let MenuItemKind::Predefined(p) = &item {
                if p.text()?.starts_with("Quit") {
                    sub.remove(p)?;
                    let name = &app.package_info().name;
                    sub.insert(&MenuItem::with_id(app, QUIT_ID, format!("Quit {name}"), true, Some("CmdOrCtrl+Q"))?, at)?;
                    break;
                }
            }
        }
    }
    Ok(menu)
}

/// Watches for macOS logging out, restarting or shutting down, and sets
/// `SYSTEM_ENDING` when it does. Must run on the main thread, once.
#[cfg(target_os = "macos")]
pub fn watch_power_off() {
    use block2::RcBlock;
    use objc2_app_kit::{NSWorkspace, NSWorkspaceWillPowerOffNotification};
    use objc2_foundation::NSNotification;
    use std::ptr::NonNull;
    let block = RcBlock::new(|_: NonNull<NSNotification>| {
        SYSTEM_ENDING.store(true, Ordering::SeqCst);
        log::line("app", "the system is ending Maya");
    });
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    // SAFETY: a block that only stores an atomic and logs; no object is
    // filtered on, and with no queue it runs on the posting (main) thread.
    let observer = unsafe { center.addObserverForName_object_queue_usingBlock(Some(NSWorkspaceWillPowerOffNotification), None, None, &block) };
    // The observer lives as long as Maya does.
    std::mem::forget(observer);
}

#[cfg(test)]
mod tests {
    use super::*;
    use maya_core::model::Harness;

    fn card(name: &str, state: State, native: bool) -> Card {
        Card {
            session_id: name.into(), pid: 0, name: name.into(), cwd: String::new(), state, state_since: 0, snippet: String::new(),
            awaiting: None, has_inbox: false, harness: Harness::ClaudeCode, pr: None, context: None, machine: None,
            machine_address: None, machine_platform: None, terminal: None, stale: false, model: None, branch: None, brain: false, native, shell: false, usage: None, limits: None, status_line: None
        }
    }

    #[test]
    fn only_local_working_native_cards_count() {
        let cards = [
            card("fix-ci", State::Working, true),
            card("idle", State::Idle, true),
            Card { machine: Some("studio".into()), ..card("remote", State::Working, true) },
            card("external", State::Working, false),
            Card { brain: true, ..card("brain", State::Working, true) },
            card("docs", State::Working, true),
        ];
        assert_eq!(working_native(&cards), ["fix-ci", "docs"]);
        assert!(working_native(&[]).is_empty());
    }

    #[test]
    fn it_asks_only_when_something_would_stop_and_nobody_already_said_yes() {
        assert!(should_ask(false, false, &["a".into()]));
        assert!(!should_ask(true, false, &["a".into()]));
        assert!(!should_ask(false, true, &["a".into()]));
        assert!(!should_ask(false, false, &[]));
    }

    #[test]
    fn an_update_restart_and_a_closed_last_window_were_already_asked_about() {
        assert!(!agreed(Some(0)), "Cmd+Q and Quit anyway's exit go through the check");
        assert!(agreed(Some(tauri::RESTART_EXIT_CODE)), "the page asked before installing the update");
        assert!(agreed(None), "the last window's close was asked about, and no window is left to ask in");
    }
}
