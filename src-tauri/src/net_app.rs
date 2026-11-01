//! The network's Tauri side: starting the server and the client from the
//! app's state, and the adapters that let the main's commands and the
//! client's status reach this Maya's own functions and page.
//!
//! Locks: the adapters take `network` or `store` one at a time, never one
//! while holding the other.

use crate::actions;
use crate::listener::{self, Interpreted, VoiceContext};
use crate::net::client::{pair_with, reconnect, reply_with_attachments, ClientHandle, ClientNotify, Executor, NOT_PAIRED, REMOVED};
use crate::net::promote;
use crate::net::merge;
use crate::net::protocol::{CommandKind, Down, Request, CAP_FOLLOW, CAP_LEAD, CAP_WORKTREE};
use maya_core::net::routing::{agent_of, agents_reply, check_remote_agent, check_remote_worktree, target_of, Target, ROUTE_TIMEOUT};
use crate::net::server::{send_command_with, start_with, ServerHandle};
use crate::net::AssistantLink;
use crate::config::{self, NetworkRole};
use maya_core::config::GitProvider;
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
        self.set(|l| *l = AssistantLink { connected: true, main_name: Some(name), error: None, retrying: false, mirror: None, voice: false, chat: false, wake_word: None });
        // The read-only Chat names the main that is awake.
        crate::brain_app::emit_status(&self.app);
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
        vec![CAP_FOLLOW.into(), CAP_LEAD.into(), CAP_WORKTREE.into()]
    }

    /// From her record: None when this Maya never ran a brain.
    fn brain_dormant_since(&self) -> Option<u64> {
        let claude_dir = self.app.state::<AppState>().store.lock().unwrap().claude_dir().to_path_buf();
        maya_core::brain::record::dormant_since(maya_core::brain::record::load(&claude_dir.join("maya").join("brain").join("record.json")).as_ref(), now_ms())
    }

    /// Starts this Maya's server with the list, then saves the config as a
    /// main without `apply_config_change`: that would stop this very client
    /// before its answer is out. The client ends by itself once it is.
    /// Nothing is saved when the server cannot start, and the server stops
    /// again when the save fails, so either way this Maya stays an assistant.
    fn promoted(&self, list: Vec<config::PairedAssistant>, summary: Option<String>) -> Result<u16, String> {
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
        // A main runs her brain: she wakes on her own session, and the old
        // main's summary is her first turn.
        let handle = self.app.clone();
        std::thread::spawn(move || crate::brain_app::wake_as_main(&handle, &by, summary));
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

    fn remembered(&self) -> Option<Vec<maya_core::remembered::Remembered>> {
        Some(self.app.state::<AppState>().store.lock().unwrap().was_running())
    }

    fn board(&self) -> (Vec<Card>, Vec<String>) {
        let (cards, root) = {
            let state = self.app.state::<AppState>();
            let mut store = state.store.lock().unwrap();
            let cards = crate::local_cards(&mut store);
            (maya_core::notify::mask_judging(&cards, &store.judging_sessions()), store.config.projects_dir_path())
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

/// What a phone's `Interpret` gets back: the line to say, and the announced
/// actions it runs when its window ends when there are any.
pub(crate) fn interpret_data(outcome: &Interpreted) -> Value {
    match outcome {
        Interpreted::Say(s) => serde_json::json!({ "say": s }),
        // The phone waits out the window and then sends one `Execute` per action;
        // `action` is the first, for a phone on an older Maya.
        Interpreted::Announce { say, actions } => serde_json::json!({ "say": say, "pending": { "say": say, "actions": actions, "action": actions.first() } }),
        // Run is executed before answering; see `serve_voice`.
        Interpreted::Run(_) => serde_json::json!({ "say": "" }),
    }
}

/// Her session's reply to a phone, in the shape `interpret_data` gives: what
/// she said, then the announcement of the actions that wait (built from the
/// actions, as the desktop's is), which the phone waits out and runs. The
/// phone says `pending.say` when there is one, so it carries both.
pub(crate) fn phone_reply(said: &str, waiting: Vec<Value>) -> Value {
    if waiting.is_empty() {
        return said_data(said);
    }
    let announced = crate::brain_route::announcement(&waiting);
    let say = [said.trim(), announced.as_str()].iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(" ");
    serde_json::json!({ "say": say, "pending": { "say": say, "actions": waiting, "action": waiting.first() } })
}

/// Her reply to a phone's request: actions that run at once (compact) run
/// here and say what they did, focus and open are refused, and the rest
/// wait on the phone.
fn brain_reply_data(app: &AppHandle, say: &str, actions: Vec<Value>) -> Value {
    let (waiting, now): (Vec<Value>, Vec<Value>) = actions.into_iter().partition(maya_core::interpreter::waits);
    let mut lines: Vec<String> = Vec::new();
    // The phone speaks it: the words, not her markdown.
    lines.extend(Some(maya_core::speech::from_markdown(say)).filter(|s| !s.is_empty()));
    let mut results = Vec::new();
    for action in now {
        if let Some(why) = refuse_remote(&action) {
            lines.push(why);
            continue;
        }
        let outcome = listener::execute_action(app, &action);
        lines.push(match &outcome {
            Ok(ok) => listener::spoken_for(&action).unwrap_or_else(|| ok.clone()),
            Err(e) => e.clone(),
        });
        results.push((action, outcome));
    }
    crate::brain_app::batch_done(app, &results);
    phone_reply(&lines.join(" "), waiting)
}

pub(crate) fn said_data(say: &str) -> Value {
    serde_json::json!({ "say": say })
}

/// Why an action that runs at once cannot run for a phone: focus brings a
/// window forward and open opens a browser, both on this machine, which the
/// user holding the phone cannot see.
pub(crate) fn refuse_remote(action: &Value) -> Option<String> {
    match action["kind"].as_str() {
        Some("focus") => Some("I can only bring a window forward when you're at the computer.".into()),
        Some("open") => Some("I can only open a pull request on the computer itself.".into()),
        _ => None,
    }
}

/// A confirmed action checked again against the board as it is now: the
/// ask may have been answered at the desk while the phone waited for yes.
/// The action keeps the session id `validate` gave it: an id matches one
/// card exactly, where the name could match another session by then.
/// A pull request is looked up by the `repo#number` it resolved to, and a
/// remote project by `"<dir> (on <machine>)"`, the way the lists spell them.
pub(crate) fn revalidate(action: &Value, ctx: &VoiceContext) -> Result<Value, String> {
    if action["kind"] == "resume_all" {
        return maya_core::interpreter::validate_resume_all(action, &ctx.remembered);
    }
    let mut asked = action.clone();
    match action["kind"].as_str() {
        Some("review") => {
            if let (Some(repo), Some(number)) = (action["repo"].as_str(), action["number"].as_u64()) {
                let forge: GitProvider = serde_json::from_value(action["forge"].clone()).unwrap_or_default();
                asked["pr"] = Value::String(format!("{repo}{}{number}", forge.sigil()));
            }
        }
        Some("start" | "resume") => {
            // `validate` adds the machine back when the dir list still has it there.
            if let Some(obj) = asked.as_object_mut() {
                obj.remove("machine");
            }
            if let (Some(dir), Some(m)) = (action["dir"].as_str(), action["machine"].as_str().filter(|m| !m.is_empty())) {
                asked["dir"] = Value::String(format!("{dir} (on {m})"));
            }
        }
        _ => {}
    }
    let fresh = maya_core::interpreter::validate(&asked, &ctx.cards, &ctx.dirs, &ctx.machines, &ctx.prs)?;
    // An answer, or words for an own-words row, must still be for the ask the user heard read back.
    if matches!(action["kind"].as_str(), Some("answer" | "reply")) && fresh["askId"] != action["askId"] {
        return Err("The question has changed; ask me again.".into());
    }
    // A pull request that has gone must not resolve to another with its number.
    let forge_of = |v: &Value| v["forge"].as_str().unwrap_or("github").to_string();
    if action["kind"] == "review" && (fresh["repo"] != action["repo"] || fresh["number"] != action["number"] || forge_of(&fresh) != forge_of(action)) {
        return Err("The pull request has changed; ask me again.".into());
    }
    Ok(fresh)
}

/// A phone's voice request: her session answers it when it runs (the
/// request and her reply join her Chat), else the one-shot interpreter does
/// (running a compact at once) and the desktop's conversation is untouched;
/// or run a confirmed action. Nothing is spoken here.
fn serve_voice(app: &AppHandle, request: Request) -> Result<Option<Value>, String> {
    match request {
        Request::Interpret { command, history } => {
            // Her session, when it runs, remembers the conversation itself.
            if crate::brain_app::ready(app) {
                return Ok(Some(match crate::brain_app::ask_and_wait(app, &command, maya_core::interpreter::TIMEOUT) {
                    Some((say, actions)) => brain_reply_data(app, &say, actions),
                    // She carries on: her answer will be in Chat.
                    None => said_data(crate::brain_route::PHONE_STILL_WORKING),
                }));
            }
            let pairs = listener::exchanges(history.iter().map(|t| (t.who.as_str(), t.text.as_str())));
            let outcome = listener::interpret_text(app, &command, &pairs);
            if let Interpreted::Run(action) = &outcome {
                if let Some(why) = refuse_remote(action) {
                    return Ok(Some(said_data(&why)));
                }
                let said = listener::execute_action(app, action).map(|s| listener::spoken_for(action).unwrap_or(s)).unwrap_or_else(|e| e);
                return Ok(Some(said_data(&said)));
            }
            Ok(Some(interpret_data(&outcome)))
        }
        Request::Execute { action } => {
            let fresh = revalidate(&action, &listener::voice_context(app))?;
            // An error, not a line: the phone speaks only what went wrong.
            if let Some(why) = refuse_remote(&fresh) {
                return Err(why);
            }
            let outcome = listener::execute_action(app, &fresh);
            // She learns how an action she gave the phone ended, and a failure
            // is left in Chat too.
            let results = [(fresh, outcome.clone())];
            crate::brain_app::batch_done(app, &results);
            listener::report_failures(app, &results, maya_core::window::Via::Phone);
            Ok(Some(said_data(&outcome?)))
        }
        _ => unreachable!("serve_voice is only called for voice requests"),
    }
}

/// Answers a mirroring Maya's request as this Maya's own page would:
/// a session's command goes to whichever machine lists it, a machine's
/// command to the machine named (`None` is this Mac), and the listings
/// come from the same places the page reads.
pub fn serve_request(app: &AppHandle, machine: Option<&str>, request: Request) -> Result<Option<Value>, String> {
    let state = app.state::<AppState>();
    match request {
        voice @ (Request::Interpret { .. } | Request::Execute { .. }) => serve_voice(app, voice),
        Request::ChatHistory => to_data(serde_json::json!({ "turns": listener::chat_history(app.state()), "status": crate::brain_app::status(app) })),
        Request::ChatSend { text } => listener::chat_send(app.clone(), text).map(|()| None),
        Request::ChatSeen => {
            listener::chat_seen(app.clone());
            Ok(None)
        }
        Request::WindowCancel => {
            listener::window_cancel(app.clone());
            Ok(None)
        }
        Request::BrainStop => {
            crate::brain_app::brain_stop(app.clone());
            Ok(None)
        }
        Request::ListDirs => match machine {
            Some(m) => to_data(merge::dirs_of(&crate::remote_boards(&state), m)),
            None => to_data(actions::list_project_dirs(&crate::local(&state))?),
        },
        Request::ListAgents => match machine {
            Some(m) => {
                let boards = crate::remote_boards(&state);
                to_data(agents_reply(false, merge::agents_of(&boards, m), merge::worktrees_of(&boards, m), Vec::new))
            }
            None => to_data(agents_reply(true, None, false, maya_core::agents::current)),
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
                    if let CommandKind::Start { options, .. } = &kind {
                        check_remote_worktree(options, merge::worktrees_of(&crate::remote_boards(&state), m), m)?;
                    }
                    if let CommandKind::Forget { sessions } = &kind {
                        // Dropped here first, so the next mirror leaves the
                        // rows out; an away assistant is not waited for.
                        let server = state.network.lock().unwrap().server.clone();
                        let connected = server.is_some_and(|s| s.drop_remembered(m, sessions));
                        crate::refresh_and_emit(app);
                        if !connected {
                            return Ok(None);
                        }
                    }
                    send_command(app, m, kind, ROUTE_TIMEOUT)
                }
                None => execute(app, kind),
            },
        },
    }
}

/// Sends `frame` to the phones that mirror and asked for Chat. The caller
/// holds no lock: the server handle is cloned out of `network` first.
pub(crate) fn to_mirrors(app: &AppHandle, frame: Down) {
    let server = app.state::<AppState>().network.lock().unwrap().server.clone();
    if let Some(s) = server {
        s.to_mirrors(frame);
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
        CommandKind::Stop { session } => done(actions::stop_session(&l, &session)),
        CommandKind::Pick { session, ask_id, question, pick } => done(actions::answer_pick(&l, &session, ask_id, question, &pick)),
        CommandKind::Compact { session } => done(actions::compact_session(&l, &session)),
        CommandKind::Close { session } => done(actions::close_session(&l, &session)),
        CommandKind::Rename { session, name } => done(actions::rename_session(&l, &session, &name)),
        CommandKind::SetOption { session, setting, value } => done(actions::set_session_option(&l, &session, &setting, &value)),
        CommandKind::CycleMode { session } => done(actions::cycle_session_mode(&l, &session)),
        CommandKind::Slash { session, text } => done(actions::send_slash_command(&l, &session, &text)),
        CommandKind::Start { dir, prompt, options } => to_data(actions::start_session(&l, dir, prompt, options)?),
        CommandKind::Resume { dir, session, agent } => done(actions::resume_relayed(&l, agent, &dir, &session)),
        CommandKind::ListResumable { dir, agent } => to_data(actions::list_resumable_sessions(&l, agent, &dir)?),
        CommandKind::History { session } => to_data(actions::remote_history(&l, &session)?),
        CommandKind::Forget { sessions } => {
            state.store.lock().unwrap().forget_remembered(&sessions);
            // The banner here and the board the main holds catch up at once.
            crate::refresh_and_emit(app);
            Ok(None)
        }
        // The client opens a handover itself (`client::handle`); it never reaches an executor.
        CommandKind::Promote { .. } => Err(maya_core::net::client::CANNOT_LEAD.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    use crate::model::{AwaitKind, Awaiting, Choice, Harness, Question, State};

    #[test]
    fn interpret_answers_say_and_a_pending_action() {
        let said = interpret_data(&Interpreted::Say("All quiet.".into()));
        assert_eq!(said, serde_json::json!({"say": "All quiet."}));
        let asked = interpret_data(&Interpreted::Announce { say: "Answering hexgrid with \"Yes\".".into(), actions: vec![serde_json::json!({"kind": "answer"})] });
        assert_eq!(asked["say"], "Answering hexgrid with \"Yes\".");
        assert_eq!(asked["pending"]["say"], asked["say"]);
        assert_eq!(asked["pending"]["action"]["kind"], "answer");
        assert_eq!(asked["pending"]["actions"][0]["kind"], "answer");
        let two = interpret_data(&Interpreted::Announce { say: "Two.".into(), actions: vec![serde_json::json!({"kind": "reply"}), serde_json::json!({"kind": "compact"})] });
        assert_eq!(two["pending"]["actions"].as_array().unwrap().len(), 2);
        assert_eq!(two["pending"]["action"]["kind"], "reply", "an older phone gets the first");
    }

    #[test]
    fn her_reply_to_a_phone_says_her_words_then_the_announcement_and_waits_with_the_actions() {
        assert_eq!(phone_reply("All quiet.", vec![]), serde_json::json!({"say": "All quiet."}));
        let a = serde_json::json!({"kind":"reply","name":"api-fix","text":"open the PR"});
        let r = phone_reply("Sure.", vec![a.clone()]);
        assert_eq!(r["say"], "Sure. Telling api-fix: open the PR.");
        assert_eq!(r["pending"]["say"], r["say"], "the phone says pending.say");
        assert_eq!(r["pending"]["actions"], serde_json::json!([a.clone()]));
        assert_eq!(r["pending"]["action"], a);
        assert_eq!(phone_reply("", vec![a])["say"], "Telling api-fix: open the PR.");
    }

    #[test]
    fn focus_and_open_cannot_run_for_a_phone() {
        assert_eq!(refuse_remote(&serde_json::json!({"kind": "focus", "session": "s1"})).as_deref(), Some("I can only bring a window forward when you're at the computer."));
        assert_eq!(refuse_remote(&serde_json::json!({"kind": "open", "repo": "dosaki/collector", "number": 14})).as_deref(), Some("I can only open a pull request on the computer itself."));
        assert!(refuse_remote(&serde_json::json!({"kind": "compact", "session": "s1"})).is_none());
    }

    #[test]
    fn execute_revalidates_against_the_board_now() {
        let gone = VoiceContext { cards: vec![], dirs: vec![], machines: vec![], prs: vec![], remembered: vec![] };
        let err = revalidate(&serde_json::json!({"kind": "reply", "session": "s1", "name": "hexgrid", "text": "ship it"}), &gone).unwrap_err();
        assert!(!err.is_empty(), "a session that has gone fails with a reason");
    }

    fn asking(id: &str, name: &str, state_since: u64) -> Card {
        Card {
            session_id: id.into(), pid: 1, name: name.into(), cwd: format!("/Users/x/dev/{name}"), state: State::Awaiting, state_since, snippet: "".into(),
            awaiting: Some(Awaiting {
                kind: AwaitKind::Question,
                detail: "Ship it?".into(),
                questions: vec![Question { question: "Ship it?".into(), header: "".into(), options: vec![Choice { label: "Yes".into(), description: "".into() }], multi_select: false, own: None, chat_row: None }],
            }),
            has_inbox: true, harness: Harness::ClaudeCode, pr: None, context: None, machine: None, machine_address: None, machine_platform: None, terminal: None, stale: false, model: None, branch: None, brain: false, native: false, shell: false, usage: None, limits: None, status_line: None, live: false
        }
    }

    fn board(cards: Vec<Card>) -> VoiceContext {
        VoiceContext { cards, dirs: vec![], machines: vec![], prs: vec![], remembered: vec![] }
    }

    #[test]
    fn own_words_for_an_ask_answered_at_the_desk_are_refused_and_a_stop_is_revalidated() {
        let own = |since| {
            let mut c = asking("s1", "hexgrid", since);
            c.awaiting.as_mut().unwrap().questions[0].own = Some(maya_core::model::OwnRow { label: "Type something.".into(), row: 2 });
            c
        };
        let action = serde_json::json!({"kind": "reply", "session": "s1", "name": "hexgrid", "text": "MariaDB", "own": true, "askId": 42});
        assert_eq!(revalidate(&action, &board(vec![own(42)])).unwrap()["askId"], 42);
        assert_eq!(revalidate(&action, &board(vec![own(99)])).unwrap_err(), "The question has changed; ask me again.");
        let mut working = asking("s1", "hexgrid", 5);
        working.state = State::Working;
        working.awaiting = None;
        let stop = serde_json::json!({"kind": "stop", "session": "s1", "name": "hexgrid"});
        assert_eq!(revalidate(&stop, &board(vec![working])).unwrap()["session"], "s1");
        assert!(revalidate(&stop, &board(vec![])).is_err());
    }

    #[test]
    fn an_ask_answered_at_the_desk_is_refused_with_the_reason() {
        let action = serde_json::json!({"kind": "answer", "session": "s1", "name": "hexgrid", "option": 1, "askId": 42});
        assert_eq!(revalidate(&action, &board(vec![asking("s1", "hexgrid", 42)])).unwrap()["askId"], 42);
        let err = revalidate(&action, &board(vec![asking("s1", "hexgrid", 99)])).unwrap_err();
        assert_eq!(err, "The question has changed; ask me again.");
    }

    #[test]
    fn a_reply_for_a_session_still_there_resolves_to_its_id() {
        let action = serde_json::json!({"kind": "reply", "session": "s1", "name": "hexgrid", "text": "ship it"});
        let fresh = revalidate(&action, &board(vec![asking("s1", "hexgrid", 42)])).unwrap();
        assert_eq!(fresh["session"], "s1");
    }

    fn pr(repo: &str, number: u64) -> maya_core::reviews::ReviewPr {
        maya_core::reviews::ReviewPr { number, forge: maya_core::config::GitProvider::Github, repo: repo.into(), title: format!("{repo} {number}"), author: "alex".into(), url: format!("https://github.com/{repo}/pull/{number}"), is_draft: false, updated_at: "2026-10-01T07:00:00Z".into(), reasons: vec![], my_review: None, pr_review: None, pr_reviewers: vec![], ssh_url: None, https_url: None }
    }

    #[test]
    fn a_review_whose_pull_request_has_gone_is_refused_not_moved_to_another() {
        let action = serde_json::json!({"kind": "review", "pr": "#14", "repo": "dosaki/collector", "number": 14, "title": "dosaki/collector 14"});
        let still = VoiceContext { cards: vec![], dirs: vec![], machines: vec![], prs: vec![pr("dosaki/collector", 14), pr("dosaki/maya", 14)], remembered: vec![] };
        let fresh = revalidate(&action, &still).unwrap();
        assert_eq!((fresh["repo"].as_str(), fresh["number"].as_u64()), (Some("dosaki/collector"), Some(14)), "the confirmed one, though another repo has a #14 now");
        let gone = VoiceContext { cards: vec![], dirs: vec![], machines: vec![], prs: vec![pr("dosaki/maya", 14)], remembered: vec![] };
        assert_eq!(revalidate(&action, &gone).unwrap_err(), "The pull request has changed; ask me again.");
    }

    #[test]
    fn a_review_whose_forge_changed_is_refused() {
        // Same repo path and number, but the action was for GitLab's MR.
        let action = serde_json::json!({"kind": "review", "pr": "dosaki/collector!14", "forge": "gitlab", "repo": "dosaki/collector", "number": 14, "title": "dosaki/collector 14"});
        let ctx = VoiceContext { cards: vec![], dirs: vec![], machines: vec![], prs: vec![pr("dosaki/collector", 14)], remembered: vec![] };
        assert_eq!(revalidate(&action, &ctx).unwrap_err(), "The pull request has changed; ask me again.", "the number found GitHub's PR, which is not the MR the user heard");
    }

    #[test]
    fn a_confirmed_remote_project_revalidates_on_its_machine() {
        let ctx = |dirs: &[&str]| VoiceContext { cards: vec![], dirs: dirs.iter().map(|d| d.to_string()).collect(), machines: vec!["laptop".into()], prs: vec![], remembered: vec![] };
        let remote = serde_json::json!({"kind": "start", "dir": "maya", "machine": "laptop", "prompt": "go"});
        for dirs in [&["maya (on laptop)"][..], &["maya", "maya (on laptop)"][..]] {
            let fresh = revalidate(&remote, &ctx(dirs)).unwrap();
            assert_eq!((fresh["dir"].as_str(), fresh["machine"].as_str()), (Some("maya"), Some("laptop")), "{dirs:?}");
        }
        let resumed = revalidate(&serde_json::json!({"kind": "resume", "dir": "maya", "machine": "laptop"}), &ctx(&["maya (on laptop)"])).unwrap();
        assert_eq!(resumed["machine"].as_str(), Some("laptop"));
        let local = revalidate(&serde_json::json!({"kind": "start", "dir": "maya", "prompt": "go"}), &ctx(&["maya", "maya (on laptop)"])).unwrap();
        assert_eq!((local["dir"].as_str(), local["machine"].as_str()), (Some("maya"), None), "a local project stays local");
        assert!(revalidate(&remote, &ctx(&["maya"])).is_err(), "the laptop no longer lists it");
    }

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

    #[test]
    fn a_confirmed_resume_all_is_counted_again_on_the_main() {
        let entry = maya_core::remembered::Remembered { agent: Harness::ClaudeCode, session_id: "a".into(), dir: "/x".into(), name: "a".into(), first_seen_ms: 1, last_seen_ms: 2, gone_at_ms: None, offered: true, offered_at_ms: None, error: None, native: false };
        let ctx = VoiceContext { cards: vec![], dirs: vec![], machines: vec![], prs: vec![], remembered: vec![maya_core::remembered::RememberedRow { machine: None, waiting: false, entry }] };
        let action = serde_json::json!({"kind": "resume_all", "scope": "all", "count": 5});
        assert_eq!(revalidate(&action, &ctx).unwrap()["count"], 1, "counted against the list as it is now");
        let empty = VoiceContext { remembered: vec![], ..ctx };
        assert_eq!(revalidate(&action, &empty).unwrap_err(), "There's nothing to resume.");
    }
}
