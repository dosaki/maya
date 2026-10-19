//! Resume after a restart on the desktop: the "was running" list across this
//! machine and its assistants, Resume and Forget for the banner and the
//! voice, and the heartbeat. Locks: `store` and `network` one at a time.

use crate::AppState;
use maya_core::log;
use maya_core::net::merge;
use maya_core::net::protocol::CommandKind;
use maya_core::net::routing::ROUTE_TIMEOUT;
use maya_core::remembered::{self, RememberedRow, ResumeOutcome};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State as TauriState};

/// The merged list as of the store's last refresh.
pub(crate) fn rows(state: &AppState) -> Vec<RememberedRow> {
    let local = state.store.lock().unwrap().was_running();
    merge::remembered(local, &crate::remote_boards(state))
}

/// Tells the page (`remembered`) and the mirroring Mayas; on every refresh.
/// Runs after `mirror(cards)`, so a phone never gets a mirror without cards.
pub(crate) fn publish(app: &AppHandle) {
    let state = app.state::<AppState>();
    let rows = rows(&state);
    let server = state.network.lock().unwrap().server.clone();
    if let Some(s) = server {
        s.mirror_remembered(rows.clone());
    }
    let _ = app.emit("remembered", &rows);
}

#[tauri::command(async)]
pub(crate) fn remembered_sessions(state: TauriState<AppState>) -> Vec<RememberedRow> {
    // A session that outlived a restart of Maya alone leaves the list on this refresh.
    crate::local_cards(&mut state.store.lock().unwrap());
    rows(&state)
}

/// Resumes `rows`, each on its own machine: this Mac's through the core,
/// each assistant's one at a time as `Resume` commands, `RESUME_GAP_MS`
/// apart; a waiting machine's rows are skipped and say so.
pub(crate) fn resume_rows(app: &AppHandle, rows: &[RememberedRow]) -> Vec<ResumeOutcome> {
    let state = app.state::<AppState>();
    // The page's word is not enough: a row's machine state is re-derived here.
    let boards = crate::remote_boards(&state);
    let rows = remembered::settle(rows, &|m| boards.iter().any(|b| b.machine == m && b.connected));
    let plan = remembered::plan(&rows);
    // Only what this Maya remembers, in the folder it remembers, is resumed here.
    let stored = state.store.lock().unwrap().was_running();
    let here = remembered::trusted(&stored, &plan.here);
    if here.len() < plan.here.len() {
        log::line("remembered", format!("{} requested sessions are not remembered here; skipped", plan.here.len() - here.len()));
    }
    let server = state.network.lock().unwrap().server.clone();
    let mut out = Vec::new();
    for (key, result) in maya_core::actions::resume_remembered(&crate::local(&state), &here) {
        if let Some(e) = here.iter().find(|e| e.key() == key) {
            out.push(ResumeOutcome::of(None, e, &result));
        }
    }
    for (machine, entries) in &plan.remote {
        for (i, e) in entries.iter().enumerate() {
            if i > 0 {
                std::thread::sleep(Duration::from_millis(remembered::RESUME_GAP_MS));
            }
            let kind = CommandKind::Resume { dir: e.dir.clone(), session: e.session_id.clone(), agent: e.agent };
            let result = crate::net_app::send_command(app, machine, kind, ROUTE_TIMEOUT).map(|_| ());
            if result.is_ok() {
                // The assistant's next Board would say so; until then do not list it again.
                if let Some(s) = &server {
                    s.drop_remembered(machine, &[e.key()]);
                }
            }
            out.push(ResumeOutcome::of(Some(machine), e, &result));
        }
    }
    for w in &plan.waiting {
        let machine = w.machine.clone().unwrap_or_default();
        out.push(ResumeOutcome::of(Some(&machine), &w.entry, &Err(remembered::waiting_for(&machine))));
    }
    crate::refresh_and_emit(app);
    out
}

#[tauri::command(async)]
pub(crate) fn resume_remembered(app: AppHandle, rows: Vec<RememberedRow>) -> Vec<ResumeOutcome> {
    resume_rows(&app, &rows)
}

/// Forgets `rows`: this Mac's at once; each assistant's from the main's
/// memory at once and, when it is connected, by `Forget` on a thread of its
/// own (an older assistant never answers; that is logged and ignored).
#[tauri::command(async)]
pub(crate) fn forget_remembered(app: AppHandle, state: TauriState<AppState>, rows: Vec<RememberedRow>) {
    let server = state.network.lock().unwrap().server.clone();
    for (machine, waiting, keys) in remembered::keys_by_machine(&rows) {
        let Some(machine) = machine else {
            state.store.lock().unwrap().forget_remembered(&keys);
            continue;
        };
        if let Some(s) = &server {
            s.drop_remembered(&machine, &keys);
        }
        if waiting {
            continue;
        }
        let handle = app.clone();
        std::thread::spawn(move || {
            if let Err(e) = crate::net_app::send_command(&handle, &machine, CommandKind::Forget { sessions: keys }, ROUTE_TIMEOUT) {
                log::line("remembered", format!("{machine} did not forget its sessions: {e}"));
            }
        });
    }
    // `drop_remembered` refreshes nothing itself: re-merge, tell the mirrors
    // (after their cards), and the page.
    crate::refresh_and_emit(&app);
}

/// Writes this run's last sign of life.
pub(crate) fn heartbeat(app: &AppHandle) {
    app.state::<AppState>().store.lock().unwrap().heartbeat(maya_core::store::now_ms());
}
