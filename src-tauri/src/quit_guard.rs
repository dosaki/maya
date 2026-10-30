//! Asking before a quit stops a working Native session: those terminals
//! are Maya's, so they end when she does. A quit (Cmd+Q, closing the
//! window) is held while the page asks; "Quit anyway" calls
//! `quit_confirmed`, whose exit then goes through. An update's restart is
//! asked about by the page before `update_install`.
//!
//! On macOS, Cmd+Q, the Dock's Quit and an AppleScript `quit` send AppKit's
//! `terminate:`, which tao turns straight into `RunEvent::Exit` with no
//! `ExitRequested` to hold. tao's app delegate has no
//! `applicationShouldTerminate:`, so Maya adds one that runs the same check.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use maya_core::log;
use maya_core::model::{Card, State};
use tauri::{AppHandle, Emitter, Manager};

use crate::AppState;

/// Set once the user said "Quit anyway", so the exit it starts goes through.
static CONFIRMED: AtomicBool = AtomicBool::new(false);
/// When macOS last said it is logging out, restarting or shutting down: the
/// system is ending Maya, and a question would only hold that up.
static SYSTEM_ENDING: Mutex<Option<Instant>> = Mutex::new(None);
/// How long that counts: a log-out the user cancelled must not keep the
/// question away for the rest of the run.
const SYSTEM_ENDING_FOR: Duration = Duration::from_secs(30);

/// Whether the system is still ending Maya at `now`, when it said so at `set`.
pub fn still_ending(set: Option<Instant>, now: Instant) -> bool {
    set.is_some_and(|at| now.saturating_duration_since(at) < SYSTEM_ENDING_FOR)
}

/// Whether the system is ending Maya now.
fn system_ending() -> bool {
    still_ending(*SYSTEM_ENDING.lock().unwrap_or_else(|e| e.into_inner()), Instant::now())
}

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
    let ending = system_ending();
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

/// Whether a `terminate:` goes ahead now: when the system is ending Maya
/// (log out, restart, shut down), or when nothing needed asking. `ask`
/// asks, and is not called when the system is ending: a held log-out is one
/// Maya would block.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn terminate_now(system_ending: bool, ask: impl FnOnce() -> bool) -> bool {
    system_ending || !ask()
}

/// Whether a quit Apple event comes from the system: log out, restart and
/// shut down give it a reason (`keyAEQuitReason`); the Dock's Quit and an
/// AppleScript `quit` give none.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn system_quit(reason: Option<u32>) -> bool {
    reason.is_some()
}

/// The app, for AppKit's `applicationShouldTerminate:` to check with.
#[cfg(target_os = "macos")]
static APP: std::sync::OnceLock<AppHandle> = std::sync::OnceLock::new();

/// `keyAEQuitReason`, `'why?'`: the attribute a system quit carries.
#[cfg(target_os = "macos")]
const KEY_AE_QUIT_REASON: u32 = u32::from_be_bytes(*b"why?");

/// The current Apple event's quit reason, if it has one.
#[cfg(target_os = "macos")]
fn quit_reason() -> Option<u32> {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager};
    let event = NSAppleEventManager::sharedAppleEventManager().currentAppleEvent()?;
    // SAFETY: `attributeDescriptorForKeyword:` takes an AEKeyword (a
    // FourCharCode, u32) and returns a descriptor or nil.
    let reason: Option<Retained<NSAppleEventDescriptor>> = unsafe { msg_send![&event, attributeDescriptorForKeyword: KEY_AE_QUIT_REASON] };
    reason.map(|r| r.enumCodeValue())
}

/// AppKit asks this before a `terminate:`. Cancel (0) holds the quit while
/// the page asks; Now (1) lets it go on.
#[cfg(target_os = "macos")]
extern "C-unwind" fn application_should_terminate(_this: *mut objc2::runtime::AnyObject, _sel: objc2::runtime::Sel, _sender: *mut objc2::runtime::AnyObject) -> usize {
    use objc2_app_kit::NSApplicationTerminateReply;
    // A panic must not cross into AppKit; a quit then goes on.
    let now = std::panic::catch_unwind(|| {
        let ending = system_ending() || system_quit(quit_reason());
        terminate_now(ending, || APP.get().is_some_and(|app| ask_if_needed(app, false)))
    })
    .unwrap_or(true);
    if now { NSApplicationTerminateReply::TerminateNow.0 } else { NSApplicationTerminateReply::TerminateCancel.0 }
}

/// Gives tao's app delegate an `applicationShouldTerminate:`, so every
/// `terminate:` (Cmd+Q, the Dock, AppleScript) passes the check. Must run on
/// the main thread, once, after the event loop set the delegate.
#[cfg(target_os = "macos")]
pub fn guard_terminate(app: &AppHandle) {
    use objc2::runtime::{AnyObject, Imp};
    use objc2::{sel, MainThreadMarker};
    use objc2_app_kit::NSApplication;
    let _ = APP.set(app.clone());
    let Some(mtm) = MainThreadMarker::new() else { return };
    let Some(delegate) = NSApplication::sharedApplication(mtm).delegate() else {
        log::line("app", "no app delegate: the Dock's Quit will not ask about Native sessions");
        return;
    };
    let delegate: &AnyObject = delegate.as_ref();
    let class = delegate.class();
    type ShouldTerminate = extern "C-unwind" fn(*mut AnyObject, objc2::runtime::Sel, *mut AnyObject) -> usize;
    let imp = application_should_terminate as ShouldTerminate;
    // SAFETY: the IMP's arguments and return match the type string `Q@:@`
    // (NSUInteger, self, _cmd, the sender), which is the method's real
    // signature; the class is the live delegate's, which tao registered.
    let added = unsafe {
        objc2::ffi::class_addMethod(class as *const _ as *mut _, sel!(applicationShouldTerminate:), std::mem::transmute::<ShouldTerminate, Imp>(imp), c"Q@:@".as_ptr())
    };
    if !added.as_bool() {
        log::line("app", "the app delegate already answers applicationShouldTerminate:; the Dock's Quit may not ask about Native sessions");
    }
}

/// Watches for macOS logging out, restarting or shutting down, and notes
/// the time in `SYSTEM_ENDING` when it does. Must run on the main thread, once.
#[cfg(target_os = "macos")]
pub fn watch_power_off() {
    use block2::RcBlock;
    use objc2_app_kit::{NSWorkspace, NSWorkspaceWillPowerOffNotification};
    use objc2_foundation::NSNotification;
    use std::ptr::NonNull;
    let block = RcBlock::new(|_: NonNull<NSNotification>| {
        *SYSTEM_ENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
        log::line("app", "the system is ending Maya");
    });
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    // SAFETY: a block that only stores a time and logs; no object is
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
    fn a_system_ending_lasts_30_seconds() {
        let set = Instant::now();
        assert!(!still_ending(None, set), "never set");
        assert!(still_ending(Some(set), set));
        assert!(still_ending(Some(set), set + Duration::from_secs(29)));
        assert!(!still_ending(Some(set), set + SYSTEM_ENDING_FOR), "a cancelled log-out no longer counts");
    }

    #[test]
    fn a_system_quit_goes_ahead_without_asking() {
        assert!(terminate_now(true, || panic!("a log-out is never held")));
        assert!(terminate_now(false, || false), "nothing to ask about");
        assert!(!terminate_now(false, || true), "held while the page asks");
    }

    #[test]
    fn only_log_out_restart_and_shut_down_carry_a_quit_reason() {
        assert!(system_quit(Some(u32::from_be_bytes(*b"rlgo"))));
        assert!(system_quit(Some(u32::from_be_bytes(*b"shut"))));
        assert!(!system_quit(None), "the Dock's Quit and AppleScript give none");
    }

    #[test]
    fn an_update_restart_and_a_closed_last_window_were_already_asked_about() {
        assert!(!agreed(Some(0)), "Cmd+Q and Quit anyway's exit go through the check");
        assert!(agreed(Some(tauri::RESTART_EXIT_CODE)), "the page asked before installing the update");
        assert!(agreed(None), "the last window's close was asked about, and no window is left to ask in");
    }
}
