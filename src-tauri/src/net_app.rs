//! The network's Tauri side: starting the server and the client from the
//! app's state, and the adapters that let the main's commands and the
//! client's status reach this Maya's own functions and page.
//!
//! Locks: the adapters take `network` or `store` one at a time, never one
//! while holding the other.

use crate::actions;
use crate::net::client::{pair_with, reconnect, reply_with_attachments, ClientHandle, ClientNotify, Executor, NOT_PAIRED, REMOVED};
use crate::net::promote;
use crate::net::merge;
use crate::net::protocol::{CommandKind, Request, CAP_FOLLOW, CAP_LEAD};
use maya_core::net::routing::{agent_of, agents_reply, check_remote_agent, target_of, Target, ROUTE_TIMEOUT};
use crate::net::server::{send_command_with, start_with, ServerHandle};
use crate::net::AssistantLink;
use crate::config::{self, NetworkRole};
use crate::log;
use crate::model::Card;
use crate::store::now_ms;
use crate::AppState;
use maya_core::net::identity::Keypair;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

const KEY_STORE: &str = "Could not reach the key store: ";

/// The key pair from `load`, or a new one saved with `store` when none is
/// stored. A value that is not a key, or a store that cannot be read, is an
/// error: a new key would silently drop every pairing.
pub fn identity_from(load: impl FnOnce() -> Result<Option<String>, String>, store: impl FnOnce(&str) -> Result<(), String>) -> Result<Keypair, String> {
    match load().map_err(|e| format!("{KEY_STORE}{e}"))? {
        Some(secret) => Keypair::from_secret(&secret).map_err(|e| format!("{KEY_STORE}{e}")),
        None => {
            let k = Keypair::generate();
            store(&k.to_secret()).map_err(|e| format!("{KEY_STORE}{e}"))?;
            Ok(k)
        }
    }
}

/// A new key pair for New key: `clear` drops the pairings, then the key is
/// saved with `store`. A failed `clear` changes nothing; a failed `store`
/// keeps the old key and `restore` puts the pairings back.
pub fn new_key_with(clear: impl FnOnce() -> Result<(), String>, store: impl FnOnce(&str) -> Result<(), String>, restore: impl FnOnce()) -> Result<Keypair, String> {
    clear()?;
    let k = Keypair::generate();
    if let Err(e) = store(&k.to_secret()) {
        restore();
        return Err(format!("{KEY_STORE}{e}"));
    }
    Ok(k)
}

/// This Maya's key pair, read once and kept for the app's life.
pub fn identity(app: &AppHandle) -> Result<Arc<Keypair>, String> {
    let state = app.state::<AppState>();
    let mut held = state.identity.lock().unwrap();
    if let Some(k) = held.as_ref() {
        return Ok(k.clone());
    }
    let k = Arc::new(identity_from(|| crate::keystore::load(&crate::keystore::NETWORK_KEY), |s| crate::keystore::store(&crate::keystore::NETWORK_KEY, s))?);
    *held = Some(k.clone());
    Ok(k)
}

/// Starts the server with the Tauri adapter and the store's paired list.
pub fn start_server(app: AppHandle, port: u16, id: Arc<Keypair>) -> Result<ServerHandle, String> {
    let view = app.state::<AppState>().store.lock().unwrap().config.network.clone();
    start_with(Arc::new(crate::TauriNetNotify { app }), Arc::new(Mutex::new(view)), port, id)
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
        match identity(&app) {
            Ok(id) => reconnect(&move || app.state::<AppState>().store.lock().unwrap().config.network.clone(), id, exec, notify, &stop),
            Err(e) => notify.disconnected(&e),
        }
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
/// main's key and the link in the config (which starts the client), and
/// returns the main's name. One blocking connection, at most `SILENCE` long
/// once connected.
pub fn pair(app: &AppHandle, host: &str, port: u16, name: &str, code: &str) -> Result<String, String> {
    // The connection ends at `welcome`, so no command reaches this executor.
    let exec: Arc<dyn Executor> = Arc::new(TauriExecutor { app: app.clone(), board_due: Arc::new(AtomicBool::new(false)) });
    let (main, main_key) = pair_with(exec, identity(app)?, host, port, name, code, vec![])?;
    crate::update_config(app, |c| {
        c.network.role = NetworkRole::Assistant;
        c.network.main_host = host.trim().into();
        c.network.main_port = port;
        c.network.name = name.trim().into();
        c.network.main_key = main_key;
        c.network.pair_again = false;
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
    fn paired(&self, _: &str) {}
    fn connected(&self, main_name: &str) {
        let name = main_name.to_string();
        self.set(|l| *l = AssistantLink { connected: true, main_name: Some(name), error: None, retrying: false, mirror: None, voice: false });
    }
    fn disconnected(&self, error: &str) {
        log::line("network", format!("disconnected: {error}"));
        // `reconnect` tries again after every failure but these two, and
        // the key store failing before it starts is final too.
        let retrying = error != NOT_PAIRED && error != REMOVED && !error.starts_with(KEY_STORE);
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

    fn caps(&self) -> Vec<String> {
        vec![CAP_FOLLOW.into(), CAP_LEAD.into()]
    }

    /// Starts this Maya's server with the list, then saves the config as a
    /// main without `apply_config_change`: that would stop this very client
    /// before its answer is out. The client ends by itself once it is.
    /// Nothing is saved when the server cannot start, and the server stops
    /// again when the save fails, so either way this Maya stays an assistant.
    fn promoted(&self, list: Vec<config::PairedAssistant>) -> Result<u16, String> {
        let state = self.app.state::<AppState>();
        let (mut after, path) = {
            let store = state.store.lock().unwrap();
            (store.config.clone(), store.config_path())
        };
        let handed = list.len().saturating_sub(1);
        promote::as_main(&mut after, list);
        let handle = start_with(Arc::new(crate::TauriNetNotify { app: self.app.clone() }), Arc::new(Mutex::new(after.network.clone())), after.listen_port(), identity(&self.app)?)?;
        if let Err(e) = config::save(&path, &after) {
            handle.stop();
            return Err(format!("Could not save the settings: {e}"));
        }
        let port = handle.port();
        state.store.lock().unwrap().config.network = after.network.clone();
        let by = {
            let mut net = state.network.lock().unwrap();
            let by = net.status.assistant.main_name.take().unwrap_or_else(|| "the main Maya".into());
            if let Some(old) = net.server.replace(handle) {
                old.stop();
            }
            net.status.main_error = None;
            net.status.assistant = Default::default();
            // Dropped, not stopped: stopping would end the connection before the answer is written.
            net.client.take();
            by
        };
        log::line("network", format!("promoted to main by {by}; {handed} assistants handed over"));
        let _ = self.app.emit("network", crate::network_status_of(&state));
        crate::refresh_and_emit(&self.app);
        Ok(port)
    }

    /// Saves the new main's host and port. The config change restarts the
    /// client against it (`RestartAssistant`), which stops this one.
    fn moved(&self, host: &str, port: u16, key: &str) -> Result<(), String> {
        crate::update_config(&self.app, |c| {
            c.network.main_host = host.into();
            c.network.main_port = port;
            c.network.main_key = key.into();
        })
        .map(|_| ())
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

/// Answers a mirroring Maya's request as this Maya's own page would:
/// a session's command goes to whichever machine lists it, a machine's
/// command to the machine named (`None` is this Mac), and the listings
/// come from the same places the page reads.
pub fn serve_request(app: &AppHandle, machine: Option<&str>, request: Request) -> Result<Option<Value>, String> {
    let state = app.state::<AppState>();
    match request {
        Request::Interpret { .. } | Request::Execute { .. } => Err("This Maya cannot interpret voice commands yet.".into()),
        Request::ListDirs => match machine {
            Some(m) => to_data(merge::dirs_of(&crate::remote_boards(&state), m)),
            None => to_data(actions::list_project_dirs(&crate::local(&state))?),
        },
        Request::ListAgents => match machine {
            Some(m) => to_data(agents_reply(false, merge::agents_of(&crate::remote_boards(&state), m), Vec::new)),
            None => to_data(agents_reply(true, None, maya_core::agents::current)),
        },
        Request::Command(kind) => match target_of(&kind) {
            Target::Session(session) => match crate::remote_machine_of(&state, &session) {
                Some(m) => send_command(app, &m, kind, ROUTE_TIMEOUT),
                None => execute(app, kind),
            },
            Target::Machine => match machine {
                Some(m) => {
                    if let Some(agent) = agent_of(&kind) {
                        check_remote_agent(agent, &merge::agents_of(&crate::remote_boards(&state), m), m)?;
                    }
                    send_command(app, m, kind, ROUTE_TIMEOUT)
                }
                None => execute(app, kind),
            },
        },
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
            reply_with_attachments(&maya_dir, exists, &text, attachments, now_ms(), |text| actions::send_reply(&l, &session, &text)).map(|route| Some(route.data()))
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
        // The client opens a handover itself (`client::handle`); it never reaches an executor.
        CommandKind::Promote { .. } => Err(maya_core::net::client::CANNOT_LEAD.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn a_stored_key_is_used_and_none_stored_makes_and_stores_one() {
        let k = Keypair::generate();
        let got = identity_from(|| Ok(Some(k.to_secret())), |_| panic!("not stored again")).unwrap();
        assert_eq!(got.public, k.public);
        let stored = RefCell::new(None);
        let made = identity_from(
            || Ok(None),
            |s| {
                *stored.borrow_mut() = Some(s.to_string());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(Keypair::from_secret(stored.borrow().as_ref().unwrap()).unwrap().public, made.public);
    }

    #[test]
    fn a_garbled_stored_key_is_an_error_not_a_new_key() {
        let out = identity_from(|| Ok(Some("garbage".into())), |_| panic!("never overwritten"));
        assert!(out.unwrap_err().starts_with("Could not reach the key store: "));
        let out = identity_from(|| Err("the keyring is locked".into()), |_| panic!("never overwritten"));
        assert_eq!(out.unwrap_err(), "Could not reach the key store: the keyring is locked");
    }

    #[test]
    fn a_new_key_is_kept_only_once_stored_and_a_failed_store_puts_the_pairings_back() {
        let stored = RefCell::new(None);
        let made = new_key_with(
            || Ok(()),
            |s| {
                *stored.borrow_mut() = Some(s.to_string());
                Ok(())
            },
            || panic!("nothing to put back"),
        )
        .unwrap();
        assert_eq!(Keypair::from_secret(stored.borrow().as_ref().unwrap()).unwrap().public, made.public);

        let restored = RefCell::new(false);
        let out = new_key_with(|| Ok(()), |_| Err("the keyring is locked".into()), || *restored.borrow_mut() = true);
        assert_eq!(out.unwrap_err(), "Could not reach the key store: the keyring is locked");
        assert!(*restored.borrow(), "the pairings come back");

        let out = new_key_with(|| Err("Could not save the settings.".into()), |_| panic!("no key made"), || panic!("nothing changed"));
        assert_eq!(out.unwrap_err(), "Could not save the settings.");
    }
}
