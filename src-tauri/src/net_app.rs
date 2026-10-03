//! The network's Tauri side: starting the server and the client from the
//! app's state, and the adapters that let the main's commands and the
//! client's status reach this Maya's own functions and page.
//!
//! Locks: the adapters take `network` or `store` one at a time, never one
//! while holding the other.

use crate::actions;
use crate::net::client::{pair_with, reconnect, reply_with_attachments, ClientHandle, ClientNotify, Executor, NOT_PAIRED, REMOVED};
use crate::net::protocol::CommandKind;
use crate::net::server::{send_command_with, start_with, ServerHandle};
use crate::net::AssistantLink;
use crate::config::NetworkRole;
use crate::log;
use crate::model::Card;
use crate::store::now_ms;
use crate::AppState;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// Starts the server with the Tauri adapter and the store's paired list.
pub fn start_server(app: AppHandle, port: u16) -> Result<ServerHandle, String> {
    let view = app.state::<AppState>().store.lock().unwrap().config.network.clone();
    start_with(Arc::new(crate::TauriNetNotify { app }), Arc::new(Mutex::new(view)), port)
}

/// `send_command_with` against the running server; an error when this Maya is not the main.
pub fn send_command(app: &AppHandle, machine: &str, kind: CommandKind, timeout: Duration) -> Result<Option<Value>, String> {
    let shared = app.state::<AppState>().network.lock().unwrap().server.as_ref().map(|s| s.shared.clone());
    let shared = shared.ok_or_else(|| format!("{machine} is not connected"))?;
    send_command_with(&shared, machine, kind, timeout)
}

/// Starts the client with the stored config, reconnecting until stopped.
pub fn start(app: AppHandle) -> ClientHandle {
    let board_due = Arc::new(AtomicBool::new(false));
    let exec: Arc<dyn Executor> = Arc::new(TauriExecutor { app: app.clone(), board_due: board_due.clone() });
    let notify_app = app.clone();
    let mut handle = ClientHandle::spawn(move |stop| {
        let notify: Arc<dyn ClientNotify> = Arc::new(TauriClientNotify { app: notify_app, stop: stop.clone() });
        reconnect(&move || app.state::<AppState>().store.lock().unwrap().config.network.clone(), exec, notify, &stop)
    });
    handle.board_due = board_due;
    handle
}

/// Asks the running client, if any, to send the board (throttled there to one a second).
pub fn push_board(app: &AppHandle) {
    if let Some(c) = app.state::<AppState>().network.lock().unwrap().client.as_ref() {
        c.board_due.store(true, Ordering::SeqCst);
    }
}

/// Pairs with the main at `host:port` using its six-digit code, stores the
/// id and token and the link in the config (which starts the client), and
/// returns the main's name. One blocking connection, at most `SILENCE` long
/// once connected.
pub fn pair(app: &AppHandle, host: &str, port: u16, name: &str, code: &str) -> Result<String, String> {
    // The connection ends at `welcome`, so no command reaches this executor.
    let exec: Arc<dyn Executor> = Arc::new(TauriExecutor { app: app.clone(), board_due: Arc::new(AtomicBool::new(false)) });
    let (main, id, token) = pair_with(exec, host, port, name, code)?;
    crate::update_config(app, |c| {
        c.network.role = NetworkRole::Assistant;
        c.network.main_host = host.trim().into();
        c.network.main_port = port;
        c.network.name = name.trim().into();
        c.network.assistant_id = id;
        c.network.token = token;
        c.listen = false;
    })?;
    log::line("network", format!("paired with {main}"));
    Ok(main)
}

/// Keeps `NetworkState.status.assistant` and tells the page (`network`).
struct TauriClientNotify {
    app: AppHandle,
    /// Its client's stop flag: a stopped client no longer reports.
    stop: Arc<AtomicBool>,
}

impl TauriClientNotify {
    fn set(&self, f: impl FnOnce(&mut AssistantLink)) {
        if self.stop.load(Ordering::SeqCst) {
            return;
        }
        {
            let state = self.app.state::<AppState>();
            let mut n = state.network.lock().unwrap();
            f(&mut n.status.assistant);
        }
        let _ = self.app.emit("network", crate::network_status_of(&self.app.state::<AppState>()));
    }
}

impl ClientNotify for TauriClientNotify {
    fn paired(&self, _: &str, _: &str) {}
    fn connected(&self, main_name: &str) {
        let name = main_name.to_string();
        self.set(|l| *l = AssistantLink { connected: true, main_name: Some(name), error: None, retrying: false });
    }
    fn disconnected(&self, error: &str) {
        log::line("network", format!("disconnected: {error}"));
        // `reconnect` tries again after every failure but these two.
        let retrying = error != NOT_PAIRED && error != REMOVED;
        self.set(|l| {
            l.connected = false;
            l.error = Some(error.to_string());
            l.retrying = retrying;
        });
    }
    fn removed(&self) {
        log::line("network", "removed by the main Maya");
        self.set(|l| {
            l.connected = false;
            l.error = Some(REMOVED.into());
            l.retrying = false;
        });
    }
}

/// Runs the main's commands through the same functions this Maya's own buttons use.
struct TauriExecutor {
    app: AppHandle,
    board_due: Arc<AtomicBool>,
}

impl Executor for TauriExecutor {
    fn execute(&self, kind: CommandKind) -> Result<Option<Value>, String> {
        execute(&self.app, kind)
    }

    fn board(&self) -> (Vec<Card>, Vec<String>) {
        let (cards, root) = {
            let state = self.app.state::<AppState>();
            let mut store = state.store.lock().unwrap();
            (crate::local_cards(&mut store), store.config.projects_dir_path())
        };
        let dirs = root.filter(|r| r.is_dir()).map(|r| crate::launch::list_project_dirs(&r)).unwrap_or_default();
        (cards, dirs)
    }

    fn board_requested(&self) -> bool {
        self.board_due.swap(false, Ordering::SeqCst)
    }

    fn agents(&self) -> Option<Vec<maya_core::agents::AgentInfo>> {
        Some(maya_core::agents::snapshot())
    }
}

fn to_data<T: serde::Serialize>(v: T) -> Result<Option<Value>, String> {
    serde_json::to_value(v).map(Some).map_err(|e| e.to_string())
}

/// Runs one command from the main with this Maya's local session actions:
/// no routing check, since the main already chose this machine.
pub fn execute(app: &AppHandle, kind: CommandKind) -> Result<Option<Value>, String> {
    let state = app.state::<AppState>();
    let l = crate::local(&state);
    let done = |r: Result<(), String>| r.map(|_| None);
    match kind {
        CommandKind::Reply { session, text, attachments } => {
            let (maya_dir, exists) = {
                let mut store = state.store.lock().unwrap();
                (store.claude_dir().join("maya"), store.card_for(&session, now_ms()).is_some())
            };
            // The files are local once saved; nothing further to attach.
            done(reply_with_attachments(&maya_dir, exists, &text, attachments, now_ms(), |text| actions::send_reply(&l, &session, &text)))
        }
        CommandKind::Answer { session, ask_id, question, option } => done(actions::answer_question(&l, &session, ask_id, question, option)),
        CommandKind::Compact { session } => done(actions::compact_session(&l, &session)),
        CommandKind::Close { session } => done(actions::close_session(&l, &session)),
        CommandKind::Rename { session, name } => done(actions::rename_session(&l, &session, &name)),
        CommandKind::SetOption { session, setting, value } => done(actions::set_session_option(&l, &session, &setting, &value)),
        CommandKind::CycleMode { session } => done(actions::cycle_session_mode(&l, &session)),
        CommandKind::Slash { session, text } => done(actions::send_slash_command(&l, &session, &text)),
        CommandKind::Start { dir, prompt, options } => to_data(actions::start_session(&l, dir, prompt, options)?),
        CommandKind::Resume { dir, session, agent } => done(actions::resume_session(&l, agent, &dir, &session)),
        CommandKind::ListResumable { dir, agent } => to_data(actions::list_resumable_sessions(&l, agent, &dir)?),
        CommandKind::History { session } => to_data(actions::session_history(&l, &session)?),
    }
}
