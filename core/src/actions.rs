//! The local session actions: what Maya does to a session on this machine,
//! over the `Terminal` seam. The app and the CLI share these bodies; routing
//! to another machine stays with the caller.

use crate::store::{now_ms, Store};
use crate::terminal::Terminal;
use crate::{answer, foreign, inbox, launch, model, resume, transcript, tty};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// This machine's sessions and the terminal that hosts them.
pub struct Local<'a> {
    pub store: &'a Mutex<Store>,
    pub terminal: &'a dyn Terminal,
}

/// Where a new session was started, how the folder was picked ("chosen",
/// "classifier" or "fallback"), and the terminal's name for it when it has names.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StartResult {
    pub dir: String,
    pub how: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<String>,
}

/// The terminal device of a live session: the one the harness or the registry
/// recorded, else the one `ps` reports for `pid`.
fn session_tty(store: &Store, session_id: &str, pid: i32) -> Result<String, String> {
    let known = match store.foreign(session_id) {
        Some(f) => f.tty,
        None => store.session(session_id).and_then(|s| s.tty),
    };
    known.map_or_else(|| tty::tty_for_pid(pid), Ok)
}

/// How long Codex must see no keys before an Enter submits instead of adding a line.
const CODEX_SUBMIT_DELAY: Duration = Duration::from_millis(300);

/// Held while a line is typed into any session. A due rename and a reply
/// can be typed at once from different threads; inside the Codex pause the
/// other line would land in the same input and both submit as one line.
static TYPING: Mutex<()> = Mutex::new(());

/// Types `line` then Enter into the session's terminal. Codex reads keys that
/// arrive in one burst as a paste, where the Enter typed with the line only
/// adds a new line to its input; a lone Enter typed after a pause submits it.
/// One line at a time per process: see `TYPING`. The store's lock is never
/// held here, so the board keeps refreshing through the pause.
fn type_line_for(l: &Local, harness: model::Harness, tty: &str, line: &str) -> Result<(), String> {
    // A thread that panicked while typing left nothing half-done worth refusing over.
    let _typing = TYPING.lock().unwrap_or_else(|e| e.into_inner());
    l.terminal.type_line(tty, line)?;
    if harness == model::Harness::Codex {
        std::thread::sleep(CODEX_SUBMIT_DELAY);
        l.terminal.type_line(tty, "")?;
    }
    Ok(())
}

/// An OpenCode card with its server: every action on such a card is a call
/// to the server rather than keys typed into a terminal.
struct ServerCard {
    client: crate::opencode::Client,
    card: model::Card,
    session: crate::opencode::SessionInfo,
    live: crate::opencode::Live,
}

/// The server behind `session_id` when it is an OpenCode session, as of the
/// last refresh; None for every other card (and for an id no card has,
/// which the caller reports). With `fresh` the board is refreshed first,
/// for the actions whose answer depends on the session's state right now.
fn server_card(l: &Local, session_id: &str, fresh: bool) -> Result<Option<ServerCard>, String> {
    let mut store = l.store.lock().unwrap();
    // Only an OpenCode card is worth a refresh here: another agent's action
    // must not wait on OpenCode's server.
    if store.opencode_card(session_id).is_none() {
        return Ok(None);
    }
    if fresh {
        store.refresh(now_ms());
    }
    let Some(card) = store.opencode_card(session_id) else { return Ok(None) };
    let service = store.opencode_service().ok_or("OpenCode's server is not running.")?;
    let (session, live) = store.opencode_session(session_id).ok_or("Session is no longer running.")?;
    Ok(Some(ServerCard { client: crate::opencode::Client::new(&service), card, session, live }))
}

/// A control reaches an OpenCode session only when no turn is running.
fn opencode_free(sc: &ServerCard) -> Result<(), String> {
    if sc.card.state == model::State::Working {
        return Err("Wait until the session is free.".into());
    }
    Ok(())
}

/// How a reply reached its session: as the user's own input (typed into its
/// terminal, or OpenCode's server), or through the inbox as a message from
/// another session, which cannot approve anything.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Route {
    Typed,
    Inbox,
}

impl Route {
    /// The `data` a `Reply` command's result carries.
    pub fn data(self) -> serde_json::Value {
        serde_json::json!({ "route": self })
    }

    /// The route in a `Reply` result's `data`; None from a Maya that sends none.
    pub fn from_data(data: Option<&serde_json::Value>) -> Option<Route> {
        data.and_then(|d| serde_json::from_value(d["route"].clone()).ok())
    }
}

/// Why a typed reply failed: before any key reached the terminal, or partway.
enum TypeFail {
    NotReached(String),
    Partial(String),
}

/// Types `lines` into the terminal on `tty` in one go, holding `TYPING` so
/// no other typed action lands inside the reply.
fn type_reply(l: &Local, tty: &str, lines: &[String]) -> Result<(), TypeFail> {
    let _typing = TYPING.lock().unwrap_or_else(|e| e.into_inner());
    l.terminal.type_lines(tty, lines).map_err(|(typed, e)| if typed == 0 { TypeFail::NotReached(e) } else { TypeFail::Partial(e) })
}

/// Sends `text` to the session as the user: typed into its terminal, or a
/// call to OpenCode's server. A Claude Code session Maya cannot type into
/// gets it through its inbox instead, as a message from another session.
pub fn send_reply(l: &Local, session_id: &str, text: &str) -> Result<Route, String> {
    if let Some(sc) = server_card(l, session_id, true)? {
        if text.trim().is_empty() {
            return Err("Message is empty.".into());
        }
        // A question with no options is answered from the composer; a pending
        // choice is answered on the card, not with a prompt into a waiting turn.
        if let Some(f) = &sc.live.form {
            if let Some(field) = f.fields.iter().find(|fl| fl.options.is_empty()) {
                return crate::opencode::reply_form(&sc.client, &sc.session.id, &f.id, &field.key, text.trim()).map(|()| Route::Typed);
            }
        }
        if sc.live.permission.is_some() || sc.live.form.is_some() {
            return Err("Answer the question on the card first.".into());
        }
        opencode_free(&sc)?;
        return crate::opencode::prompt(&sc.client, &sc.session.id, text.trim()).map(|()| Route::Typed);
    }
    // Other harnesses have no inbox: the reply is typed into their tty as one line.
    let foreign = l.store.lock().unwrap().foreign(session_id);
    if let Some(f) = foreign {
        let card = l.store.lock().unwrap().card_for(session_id, now_ms()).ok_or("Session is no longer running.")?;
        answer::check_free(&card)?;
        let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            return Err("Message is empty.".into());
        }
        let tty = session_tty(&l.store.lock().unwrap(), session_id, f.pid)?;
        return type_line_for(l, f.harness, &tty, &line).map(|()| Route::Typed);
    }
    if text.trim().is_empty() {
        return Err("Message is empty.".into());
    }
    if text.chars().count() > answer::TYPED_MAX_CHARS {
        return Err(format!("Message is too long to type (over {} characters).", answer::TYPED_MAX_CHARS));
    }
    let lines = answer::reply_lines(text);
    if lines.len() > answer::TYPED_MAX_LINES {
        return Err(format!("Message has too many lines to type (over {}).", answer::TYPED_MAX_LINES));
    }
    let (tty, inbox) = {
        let mut store = l.store.lock().unwrap();
        let card = store.claude_card(session_id, now_ms()).ok_or("Session is no longer running.")?;
        answer::check_free(&card)?;
        let s = store.session(session_id).ok_or("Session is no longer running.")?;
        (session_tty(&store, session_id, s.pid).ok(), s.messaging_socket_path.clone().map(|p| (p, s.pid)))
    };
    let typed = match &tty {
        Some(tty) => type_reply(l, tty, &lines),
        None => Err(TypeFail::NotReached("no terminal found for the session".into())),
    };
    let result = match typed {
        Ok(()) => Ok(Route::Typed),
        Err(TypeFail::Partial(e)) => {
            crate::log::line("reply", format!("typing into {session_id} stopped partway: {e}"));
            Err("Part of the reply was typed into the terminal; check it there.".into())
        }
        Err(TypeFail::NotReached(e)) => match inbox {
            Some((socket, pid)) => {
                crate::log::line("reply", format!("could not type into {session_id} ({e}); using its inbox"));
                inbox::send(Path::new(&socket), pid, text).map(|()| Route::Inbox)
            }
            None => Err("Maya can't type into this session's terminal and it has no inbox. Use the terminal.".into()),
        },
    };
    crate::log::line(
        "reply",
        match &result {
            Ok(route) => format!("sent {} characters to {session_id} ({route:?})", text.chars().count()),
            Err(e) => format!("could not send to {session_id}: {e}"),
        },
    );
    result
}

/// Picks `option` of `question` in the session's open ask `ask_id`.
pub fn answer_question(l: &Local, session_id: &str, ask_id: u64, question: usize, option: usize) -> Result<(), String> {
    if let Some(sc) = server_card(l, session_id, true)? {
        answer::check(&sc.card, ask_id, question, option, now_ms())?;
        if let Some(p) = &sc.live.permission {
            return crate::opencode::reply_permission(&sc.client, &sc.session.id, &p.id, crate::opencode::decision(option));
        }
        if let Some(f) = &sc.live.form {
            let field = f.fields.get(question).ok_or("That question has no such option.")?;
            let (value, _) = field.options.get(option).ok_or("That question has no such option.")?;
            return crate::opencode::reply_form(&sc.client, &sc.session.id, &f.id, &field.key, value);
        }
        return Err("This session is not waiting for a question.".into());
    }
    let (card, tty) = {
        let mut store = l.store.lock().unwrap();
        let card = store.card_for(session_id, now_ms()).ok_or("Session is no longer running.")?;
        answer::check(&card, ask_id, question, option, now_ms())?;
        let tty = session_tty(&store, session_id, card.pid)?;
        (card, tty)
    };
    let count = card.awaiting.as_ref().map(|a| a.questions.len()).unwrap_or(0);
    l.terminal.type_line(&tty, &answer::keys_for_option(option))?;
    if answer::needs_submit(question, count) {
        std::thread::sleep(Duration::from_millis(answer::SUBMIT_DELAY_MS));
        l.terminal.type_line(&tty, "")?;
    }
    Ok(())
}

/// Types `/compact` into the session's terminal, or asks OpenCode's server.
pub fn compact_session(l: &Local, session_id: &str) -> Result<(), String> {
    if let Some(sc) = server_card(l, session_id, false)? {
        opencode_free(&sc)?;
        return crate::opencode::compact(&sc.client, &sc.session.id);
    }
    type_into_session(l, session_id, answer::COMPACT, "/compact", |c| c.compact)
}

/// How long Close waits for Claude to exit before it leaves the shell alone.
const EXIT_WAIT: Duration = Duration::from_secs(5);

/// Ends an idle or completed session and closes its terminal: types `/exit`
/// (every agent takes it), waits for the agent's process to end, then types
/// `exit` into the shell left behind.
pub fn close_session(l: &Local, session_id: &str) -> Result<(), String> {
    close_session_with(l, session_id, |pid| {
        let deadline = std::time::Instant::now() + EXIT_WAIT;
        while crate::registry::pid_alive(pid) {
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        true
    })
}

/// `close_session` with `exited(pid)`, which says whether the process ended in time.
fn close_session_with(l: &Local, session_id: &str, exited: impl Fn(i32) -> bool) -> Result<(), String> {
    // Everything is looked up before `/exit`: once the agent exits, its
    // registry entry goes, and on Windows so does the console key found through it.
    let (harness, pid, tty) = {
        let mut store = l.store.lock().unwrap();
        let card = store.card_for(session_id, now_ms()).ok_or("Session is no longer running.")?;
        if !launch::capabilities(card.harness).close {
            return Err(format!("{} has no Close.", launch::label(card.harness)));
        }
        if !matches!(card.state, model::State::Idle | model::State::Completed) {
            return Err("Only an idle or completed session can be closed.".into());
        }
        // An OpenCode session lives in its server; Close needs a window to type /exit into.
        if card.harness == model::Harness::OpenCode && store.opencode_window(&card.cwd).is_none() {
            return Err("No OpenCode window is open for that folder.".into());
        }
        (card.harness, card.pid, session_tty(&store, session_id, card.pid)?)
    };
    let after = l.terminal.reach_after_exit(&tty);
    type_line_for(l, harness, &tty, answer::EXIT)?;
    // Typed while the agent still runs, `exit` would land in its prompt as a message.
    if !exited(pid) {
        return Err(format!("{} did not exit; the terminal was left open.", launch::label(harness)));
    }
    let mut last = "nothing else is attached to its terminal".to_string();
    for key in &after {
        match l.terminal.type_line(key, "exit") {
            Ok(()) => return Ok(()),
            Err(e) => last = e,
        }
    }
    Err(format!("The session ended, but the terminal was not closed: {last}"))
}

/// Types `/rename <name>` into the session's terminal. Claude Code takes it
/// whenever it is not asking a question; another agent only when free, since
/// keys typed into its TUI elsewhere act as shortcuts. The new name comes
/// back through the agent's own files on the next refresh.
pub fn rename_session(l: &Local, session_id: &str, name: &str) -> Result<(), String> {
    if let Some(sc) = server_card(l, session_id, false)? {
        answer::rename_command(name)?;
        return crate::opencode::rename(&sc.client, &sc.session.id, name.trim());
    }
    let line = answer::rename_command(name)?;
    let (harness, tty) = {
        let mut store = l.store.lock().unwrap();
        let card = store.card_for(session_id, now_ms()).ok_or("Session is no longer running.")?;
        if card.harness == model::Harness::ClaudeCode {
            answer::check_free(&card)?;
        } else if !crate::pending_names::is_free(&card) {
            return Err("Wait until the session is free to rename it.".into());
        }
        // The user's own name wins over one still waiting from the start.
        store.forget_pending_name(session_id);
        (card.harness, session_tty(&store, session_id, card.pid)?)
    };
    type_line_for(l, harness, &tty, &line)
}

/// Types `/rename` into the sessions whose pending name is due. Looked up
/// under the store's lock, typed after it is released. A name was due when
/// some refresh saw its session free, up to a tick ago; the session may have
/// started a turn or opened a prompt since, where the keys would act as
/// shortcuts, so each is checked against a fresh card and put back to wait
/// when busy. Every other rename is logged, typed or not: it is taken once
/// and never comes back.
pub fn run_due_renames(l: &Local) {
    let due: Vec<(String, model::Harness, String, String)> = {
        let mut store = l.store.lock().unwrap();
        let due = store.take_due_renames();
        if due.is_empty() {
            return;
        }
        // One refresh for all of them; any it makes due are left for the next tick.
        let cards = store.refresh(now_ms());
        due.into_iter()
            .filter_map(|(id, name)| {
                let card = cards.iter().find(|c| c.session_id == id);
                if let Some(c) = card.filter(|c| !crate::pending_names::is_free(c)) {
                    crate::log::line("rename", format!("not typing {name:?} into {id} yet: it is {:?} again; retrying once it is free", c.state));
                    store.requeue_rename(&id);
                    return None;
                }
                let found = (|| {
                    card.ok_or("the session is gone")?;
                    let f = store.foreign(&id).ok_or("the session is gone")?;
                    Ok::<_, String>((f.harness, session_tty(&store, &id, f.pid)?, answer::rename_command(&name)?))
                })();
                match found {
                    Ok((harness, tty, line)) => Some((id, harness, tty, line)),
                    Err(e) => {
                        crate::log::line("rename", format!("could not rename {id}: {e}"));
                        None
                    }
                }
            })
            .collect()
    };
    for (id, harness, tty, line) in due {
        crate::log::line(
            "rename",
            match type_line_for(l, harness, &tty, &line) {
                Ok(()) => format!("typed {line:?} into {id} on {tty}"),
                Err(e) => format!("could not rename {id} on {tty}: {e}"),
            },
        );
    }
}

/// Types `/model x` or `/effort y` into the session's terminal, after
/// checking the value against the agent's own listing.
pub fn set_session_option(l: &Local, session_id: &str, setting: &str, value: &str) -> Result<(), String> {
    if let Some(sc) = server_card(l, session_id, false)? {
        opencode_free(&sc)?;
        // The listing can take seconds: never under the store's lock.
        let agents = l.store.lock().unwrap().agents_source();
        let info = agents().into_iter().find(|a| a.harness == model::Harness::OpenCode).unwrap_or_else(|| crate::agents::info_for(model::Harness::OpenCode, None));
        // Build on what the server has now, not the last poll: Apply sends a model then an effort.
        let current = crate::opencode::session(&sc.client, &sc.session.id)?;
        let (provider, model, variant) = current.model.clone().ok_or("The session has no model yet.")?;
        return match setting {
            "model" => {
                let m = info.models.iter().find(|m| m.id == value).ok_or_else(|| format!("Unknown model: {value}"))?;
                let (p, id) = m.id.split_once('/').ok_or_else(|| format!("Unknown model: {value}"))?;
                crate::opencode::switch_model(&sc.client, &sc.session.id, p, id, variant.as_deref().filter(|v| m.efforts.iter().any(|e| e == v)))
            }
            "effort" => {
                let current = format!("{provider}/{model}");
                let allowed = info.models.iter().find(|m| m.id == current).map(|m| m.efforts.clone()).unwrap_or_else(|| info.efforts.clone());
                if !allowed.iter().any(|e| e == value) {
                    return Err(format!("Unknown effort: {value}"));
                }
                crate::opencode::switch_model(&sc.client, &sc.session.id, &provider, &model, Some(value))
            }
            _ => Err(format!("Unknown setting: {setting}")),
        };
    }
    let (harness, agents) = {
        let mut store = l.store.lock().unwrap();
        let card = store.card_for(session_id, now_ms()).ok_or("Session is no longer running.")?;
        (card.harness, store.agents_source())
    };
    let is_model = setting == "model";
    let allowed = move |c: &launch::Capabilities| if is_model { c.model_switch } else { c.effort_switch };
    // An agent without the command is refused before its models are listed.
    if matches!(setting, "model" | "effort") && !allowed(&launch::capabilities(harness)) {
        return Err(format!("{} has no /{setting}.", launch::label(harness)));
    }
    let info = if harness == model::Harness::ClaudeCode {
        crate::agents::claude()
    } else {
        agents().into_iter().find(|a| a.harness == harness).unwrap_or_else(|| crate::agents::info_for(harness, None))
    };
    let line = answer::slash_command(setting, value, &info.model_ids(), &info.efforts)?;
    type_into_session(l, session_id, &line, &format!("/{setting}"), allowed)
}

/// Types a `/command` or `!` shell line from the composer into the
/// session's terminal, after `answer::check_terminal_command`.
pub fn send_slash_command(l: &Local, session_id: &str, text: &str) -> Result<(), String> {
    let line = answer::check_terminal_command(text)?;
    if let Some(sc) = server_card(l, session_id, false)? {
        opencode_free(&sc)?;
        if let Some(cmd) = line.strip_prefix('!') {
            return crate::opencode::shell(&sc.client, &sc.session.id, cmd.trim());
        }
        let rest = line.trim_start_matches('/');
        let (name, text) = rest.split_once(' ').map(|(n, t)| (n, t.trim())).unwrap_or((rest, ""));
        // A typed /model or /effort is the picker's switch, checked the same way.
        if matches!(name, "model" | "effort") && !text.is_empty() {
            return set_session_option(l, session_id, name, text);
        }
        return crate::opencode::command(&sc.client, &sc.session.id, name, text);
    }
    let shell = line.starts_with('!');
    type_into_session(l, session_id, &line, if shell { "! shell lines" } else { "/ commands" }, move |c| if shell { c.shell_lines } else { c.slash_lines })
}

/// Sends the agent's mode-cycle keys (Shift+Tab) to the session's terminal.
pub fn cycle_session_mode(l: &Local, session_id: &str) -> Result<(), String> {
    if let Some(sc) = server_card(l, session_id, false)? {
        opencode_free(&sc)?;
        let current = crate::opencode::session(&sc.client, &sc.session.id)?;
        return crate::opencode::switch_agent(&sc.client, &sc.session.id, crate::opencode::next_agent(&current.agent));
    }
    let harness = l.store.lock().unwrap().card_for(session_id, now_ms()).ok_or("Session is no longer running.")?.harness;
    let keys = launch::capabilities(harness).mode_cycle.ok_or_else(|| format!("{} has no mode cycle.", launch::label(harness)))?;
    type_into_session(l, session_id, keys, "mode cycle", |c| c.mode_cycle.is_some())
}

/// Types `text` into a free session's terminal when the agent has `control`
/// (`allowed` reads that off its capabilities). Claude Code queues what is
/// typed while it works; the other agents must be idle or completed, since
/// keys typed into their TUIs elsewhere act as shortcuts.
fn type_into_session(l: &Local, session_id: &str, text: &str, control: &str, allowed: impl Fn(&launch::Capabilities) -> bool) -> Result<(), String> {
    let (harness, tty) = {
        let mut store = l.store.lock().unwrap();
        let card = store.card_for(session_id, now_ms()).ok_or("Session is no longer running.")?;
        if !allowed(&launch::capabilities(card.harness)) {
            return Err(format!("{} has no {control}.", launch::label(card.harness)));
        }
        if card.harness == model::Harness::ClaudeCode {
            answer::check_free(&card)?;
        } else if !crate::pending_names::is_free(&card) {
            return Err("Wait until the session is free.".into());
        }
        (card.harness, session_tty(&store, session_id, card.pid)?)
    };
    type_line_for(l, harness, &tty, text)
}

/// The session's last turns.
pub fn session_history(l: &Local, session_id: &str) -> Result<Vec<transcript::Turn>, String> {
    if let Some(sc) = server_card(l, session_id, false)? {
        return crate::opencode::history(&sc.client, &sc.session.id);
    }
    let (path, foreign) = {
        let store = l.store.lock().unwrap();
        if let Some(f) = store.foreign(session_id) {
            (f.transcript_path.clone(), Some(f))
        } else {
            let s = store.session(session_id).ok_or("Session is no longer running.")?;
            (store.transcript_path_for(&s), None)
        }
    };
    match foreign {
        Some(f) => Ok(foreign::turns_for(&f, 30)),
        None => Ok(transcript::read_turns(&path, 30)),
    }
}

/// The folders of the projects directory.
pub fn list_project_dirs(l: &Local) -> Result<Vec<String>, String> {
    let root = projects_root(&l.store.lock().unwrap())?;
    Ok(launch::list_project_dirs(&root))
}

/// The agent's past sessions of a project folder, newest first, running ones marked.
pub fn list_resumable_sessions(l: &Local, agent: model::Harness, dir: &str) -> Result<Vec<resume::ResumableSession>, String> {
    let (path, dirs, running, service) = {
        let store = l.store.lock().unwrap();
        (project_path(&store, dir)?, store.agent_dirs(), store.live_session_ids(), store.opencode_service())
    };
    if agent == model::Harness::OpenCode {
        // With the server down there is nothing to list, not an error.
        return Ok(service.map(|s| crate::opencode::resumable(&crate::opencode::Client::new(&s), &path.to_string_lossy(), &running)).unwrap_or_default());
    }
    Ok(resume::list_sessions(agent, &dirs, &path.to_string_lossy(), &running))
}

/// A Codex effort must be one the chosen model takes, or with "Default" one
/// every model takes: Codex's efforts differ by model, and the modal is not
/// the only caller (the CLI and the network pass options too). Other agents'
/// efforts do not vary by model, and `validate_shape` has checked them.
fn check_codex_effort(info: &crate::agents::AgentInfo, options: &launch::LaunchOptions) -> Result<(), String> {
    let chosen = |v: &Option<String>| v.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
    let Some(effort) = chosen(&options.effort).filter(|_| options.agent == model::Harness::Codex) else { return Ok(()) };
    let (takes, model) = match chosen(&options.model) {
        Some(m) => (info.models.iter().find(|x| x.id == m).map(|x| x.efforts.clone()).unwrap_or_default(), m),
        None => (info.efforts.clone(), "the default model".to_string()),
    };
    if takes.contains(&effort) {
        Ok(())
    } else {
        Err(format!("Unknown effort for {model}: {effort}"))
    }
}

/// How long New session waits for `opencode service start` to write the state file.
const OPENCODE_START_WAIT: Duration = Duration::from_secs(10);

/// Starts an OpenCode session: created on the server with its name, model
/// and variant, prompted over the wire, then shown in a terminal opened on
/// its id. Returns the terminal's name, as `Terminal::open` does.
fn start_opencode(l: &Local, target: &Path, prompt: &str, name: Option<&str>, options: &launch::LaunchOptions, info: &crate::agents::AgentInfo) -> Result<Option<String>, String> {
    let model = options.model.as_deref().map(str::trim).filter(|m| !m.is_empty());
    let effort = options.effort.as_deref().map(str::trim).filter(|e| !e.is_empty());
    // An effort is one of the chosen model's variants, or with "Default" one every model has.
    if let Some(e) = effort {
        let allowed: Vec<String> = match model.and_then(|m| info.models.iter().find(|x| x.id == m)) {
            Some(m) => m.efforts.clone(),
            None => info.efforts.clone(),
        };
        if !allowed.iter().any(|a| a == e) {
            return Err(format!("Unknown effort: {e}"));
        }
    }
    let state_file = l.store.lock().unwrap().opencode_state_file();
    // The binary is only needed to start a server that is not running yet.
    let service = crate::opencode::ensure_service(&state_file, &crate::registry::pid_alive, || crate::opencode::start_service(&launch::find_binary(model::Harness::OpenCode)?), OPENCODE_START_WAIT)?;
    let client = crate::opencode::Client::new(&service);
    let dir = target.to_string_lossy();
    let id = crate::opencode::create_session(&client, &dir, name, model.and_then(|m| m.split_once('/')), effort)?;
    crate::opencode::prompt(&client, &id, prompt)?;
    let auto = options.mode.as_deref().map(str::trim) == Some("auto");
    l.terminal.open(&launch::opencode_open_command(target, &id, auto), target, &tmux_label())
}

/// Opens a terminal in a project folder running the chosen agent on
/// `prompt`. With no `dir`, Maya's agent picks the folder from the prompt. A
/// name for an agent that takes none on its command line waits in the store
/// for the new session, to be typed as `/rename` once it is free.
pub fn start_session(l: &Local, dir: Option<String>, prompt: String, options: launch::LaunchOptions) -> Result<StartResult, String> {
    if prompt.trim().is_empty() {
        return Err("Type a prompt first.".into());
    }
    options.validate_shape()?;
    let (root, maya_dir, agents) = {
        let store = l.store.lock().unwrap();
        (projects_root(&store)?, store.claude_dir().join("maya"), store.agents_source())
    };
    // Claude Code's models are Maya's own fixed list. Listing the agents is
    // only for the others, and can take seconds, so a Claude start skips it.
    let info = if options.agent == model::Harness::ClaudeCode {
        crate::agents::claude()
    } else {
        agents().into_iter().find(|a| a.harness == options.agent).ok_or_else(|| format!("{} is not installed on this machine.", launch::label(options.agent)))?
    };
    options.validate(&info.model_ids())?;
    check_codex_effort(&info, &options)?;
    let dirs = launch::list_project_dirs(&root);
    let picked = match dir {
        Some(_) => None,
        None => {
            let (brain, model) = {
                let store = l.store.lock().unwrap();
                (store.config.brain(), store.config.brain_model().map(String::from))
            };
            let binary = launch::find_binary(brain)?;
            // The one-shot runs in Maya's data folder, where no project's settings load and no board reads.
            launch::classify(brain, &binary, model.as_deref(), &root, &prompt, &dirs, &maya_dir, launch::CLASSIFIER_TIMEOUT)
        }
    };
    let (target, how) = launch::resolve_target(&root, &dirs, dir.as_deref(), picked.as_deref())?;
    if options.agent == model::Harness::OpenCode {
        let terminal = start_opencode(l, &target, &prompt, options.chosen_name(), &options, &info)?;
        return Ok(StartResult { dir: target.to_string_lossy().into_owned(), how: how.to_string(), terminal });
    }
    let file = launch::write_prompt_file(&maya_dir, &prompt)?;
    let grok_id = (options.agent == model::Harness::Grok).then(launch::new_session_uuid);
    // Sessions already running cannot be the new one. They come from a fresh
    // discovery: one started by hand seconds ago, not yet looked up by a
    // refresh, would otherwise look new and take the name.
    let known = {
        let mut store = l.store.lock().unwrap();
        store.refresh(now_ms());
        store.live_session_ids()
    };
    let terminal = l.terminal.open(&launch::session_command(&target, &file, &options, grok_id.as_deref()), &target, &tmux_label())?;
    if let (Some(name), false) = (options.chosen_name(), options.agent == model::Harness::ClaudeCode) {
        let target = target.to_string_lossy();
        l.store.lock().unwrap().add_pending_name(crate::pending_names::PendingName::new(options.agent, &target, name, now_ms(), known, grok_id));
    }
    Ok(StartResult { dir: target.to_string_lossy().into_owned(), how: how.to_string(), terminal })
}

/// Opens a terminal in the folder resuming `session_id` with `agent`. The id
/// must be one the agent recorded for that folder, so nothing typed or
/// relayed reaches the shell line unchecked.
pub fn resume_session(l: &Local, agent: model::Harness, dir: &str, session_id: &str) -> Result<(), String> {
    if !resume::plain_session_id(session_id) {
        return Err("No such session in that folder.".into());
    }
    let (path, dirs, running) = {
        let store = l.store.lock().unwrap();
        (project_path(&store, dir)?, store.agent_dirs(), store.live_session_ids())
    };
    if running.iter().any(|id| id == session_id) {
        return Err("That session is already running.".into());
    }
    let known = if agent == model::Harness::OpenCode { list_resumable_sessions(l, agent, dir)? } else { resume::list_sessions(agent, &dirs, &path.to_string_lossy(), &running) };
    if !known.iter().any(|s| s.id == session_id) {
        return Err("No such session in that folder.".into());
    }
    l.terminal.open(&resume::resume_command(agent, &path, session_id), &path, &tmux_label()).map(|_| ())
}

/// Opens a terminal reviewing `pr` with Maya's agent: in the project
/// checkout when it is free, else in a clone under the clones directory.
/// Returns the folder. For an agent that takes no name on its command line
/// the review name waits in the store, as `start_session`'s does.
pub fn start_review(l: &Local, pr: &crate::reviews::ReviewPr) -> Result<String, String> {
    let (agent, projects, clones, live, maya_dir, template, agents) = {
        let store = l.store.lock().unwrap();
        (store.config.brain(), store.config.projects_dir_path(), store.config.clones_dir_path(), store.live_cwds(), store.claude_dir().join("maya"), store.config.review_prompt.clone(), store.agents_source())
    };
    let projects = projects.ok_or("Set a projects directory in Settings first.")?;
    if agent != model::Harness::ClaudeCode && !agents().iter().any(|a| a.harness == agent) {
        return Err(format!("{} is not installed on this machine.", launch::label(agent)));
    }
    let target = crate::reviews::resolve_target(&projects, &clones, &pr.repo, pr.number, &live);
    if agent == model::Harness::OpenCode {
        // The review shell line clones a missing checkout before the agent runs;
        // OpenCode's session is created on the server first, so it needs the folder.
        if target.clone {
            return Err("OpenCode reviews need the repository checked out under the projects directory.".into());
        }
        let (brain_model, info) = {
            let store = l.store.lock().unwrap();
            (store.config.brain_model().map(String::from), agents().into_iter().find(|a| a.harness == model::Harness::OpenCode).unwrap_or_else(|| crate::agents::info_for(model::Harness::OpenCode, None)))
        };
        let options = launch::LaunchOptions { agent, model: brain_model, ..Default::default() };
        let name = crate::reviews::session_name(&pr.repo, pr.number);
        start_opencode(l, &target.dir, &crate::reviews::render_prompt(&template, pr), Some(&name), &options, &info)?;
        return Ok(target.dir.to_string_lossy().into_owned());
    }
    let file = launch::write_prompt_file(&maya_dir, &crate::reviews::render_prompt(&template, pr))?;
    let grok_id = (agent == model::Harness::Grok).then(launch::new_session_uuid);
    let known = {
        let mut store = l.store.lock().unwrap();
        store.refresh(now_ms());
        store.live_session_ids()
    };
    // A clone's folder does not exist yet: the terminal opens in its parent,
    // the clones root, made first (a terminal cannot start in a missing folder).
    let cwd = if target.clone { target.dir.parent().map(Path::to_path_buf).unwrap_or_else(|| target.dir.clone()) } else { target.dir.clone() };
    if target.clone {
        std::fs::create_dir_all(&cwd).map_err(|e| format!("Could not create the clones folder {}: {e}.", cwd.display()))?;
    }
    l.terminal.open(&crate::reviews::shell_command(agent, &target, &pr.repo, pr.number, &file, grok_id.as_deref()), &cwd, &tmux_label())?;
    if agent != model::Harness::ClaudeCode {
        let name = crate::reviews::session_name(&pr.repo, pr.number);
        l.store.lock().unwrap().add_pending_name(crate::pending_names::PendingName::new(agent, &target.dir.to_string_lossy(), &name, now_ms(), known, grok_id));
    }
    Ok(target.dir.to_string_lossy().into_owned())
}

/// Brings the terminal hosting the session forward.
pub fn focus_session(l: &Local, session_id: &str) -> Result<(), String> {
    let tty = {
        let store = l.store.lock().unwrap();
        let pid = match (store.foreign(session_id), store.opencode_card(session_id)) {
            (Some(f), _) => f.pid,
            (None, Some(c)) => c.pid,
            (None, None) => store.session(session_id).ok_or("Session is no longer running.")?.pid,
        };
        session_tty(&store, session_id, pid)?
    };
    l.terminal.focus(&tty)
}

/// The configured projects directory, which must exist.
pub fn projects_root(store: &Store) -> Result<PathBuf, String> {
    let root = store.config.projects_dir_path().ok_or("Set a projects directory in Settings first.")?;
    if !root.is_dir() {
        return Err(format!("Projects directory does not exist: {}.", root.display()));
    }
    Ok(root)
}

/// The project folder `dir` as an absolute path, refusing anything not listed.
pub fn project_path(store: &Store, dir: &str) -> Result<PathBuf, String> {
    let root = projects_root(store)?;
    if !launch::list_project_dirs(&root).iter().any(|d| d == dir) {
        return Err("That folder is not in the projects directory.".into());
    }
    Ok(root.join(dir))
}

/// A name for a new session's terminal: `maya-` and eight hex characters.
pub fn tmux_label() -> String {
    let mut bytes = [0u8; 4];
    rand::rng().fill_bytes(&mut bytes);
    format!("maya-{}", bytes.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use crate::config;
    use crate::store::Store;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    /// A temp dir that *is* `~/.claude` (with `sessions/` and `maya/`) plus
    /// `projects/`: its own path is the claude dir, so callers can hand
    /// `.path()` straight to anything that takes a claude dir.
    fn temp_claude() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().to_path_buf();
        std::fs::create_dir_all(claude.join("sessions")).unwrap();
        std::fs::create_dir_all(claude.join("maya")).unwrap();
        (dir, claude)
    }

    /// Writes an idle registry entry for `id` on `/dev/pts/3`.
    pub fn write_session(claude: &Path, id: &str, pid: i32, cwd: &str) {
        let entry = serde_json::json!({"pid": pid, "sessionId": id, "cwd": cwd, "name": id, "status": "idle", "tty": "/dev/pts/3"});
        std::fs::write(claude.join(format!("sessions/{pid}.json")), entry.to_string()).unwrap();
    }

    /// `projects/` under the temp dir with `dirs` as subfolders; its path.
    fn make_projects(root: &Path, dirs: &[&str]) -> PathBuf {
        let projects = root.join("projects");
        for d in dirs {
            std::fs::create_dir_all(projects.join(d)).unwrap();
        }
        projects
    }

    /// Builds the store and also persists the config to disk, so a second
    /// `Store::new` over the same claude dir (as the CLI commands do) sees
    /// the same projects directory.
    fn store(claude: PathBuf, projects: Option<&Path>) -> Mutex<Store> {
        let mut store = Store::new(claude).with_alive(|_| true);
        store.config.projects_dir = projects.map(|p| p.to_string_lossy().into_owned());
        config::save(&store.config_path(), &store.config).unwrap();
        Mutex::new(store)
    }

    /// A temp `~/.claude` with one live registry session (`tty: Some("/dev/pts/3")`, idle) and a store over it.
    pub fn store_with_session(id: &str, pid: i32) -> (tempfile::TempDir, Mutex<Store>) {
        let (dir, claude) = temp_claude();
        write_session(&claude, id, pid, "/Users/x/dev/eye");
        (dir, store(claude, None))
    }

    /// A temp `~/.claude` whose config's projects_dir holds the given subfolders.
    pub fn store_with_projects(dirs: &[&str]) -> (tempfile::TempDir, Mutex<Store>) {
        let (dir, claude) = temp_claude();
        let projects = make_projects(dir.path(), dirs);
        (dir, store(claude, Some(&projects)))
    }

    /// Both: projects plus one running session `running_id` in the first folder.
    pub fn store_with_projects_and_running(dirs: &[&str], running_id: &str) -> (tempfile::TempDir, Mutex<Store>) {
        let (dir, claude) = temp_claude();
        let projects = make_projects(dir.path(), dirs);
        let first = projects.join(dirs.first().copied().unwrap_or_default());
        write_session(&claude, running_id, 4242, &first.to_string_lossy());
        (dir, store(claude, Some(&projects)))
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use crate::agents;
    use crate::foreign::ForeignSession;
    use crate::launch::LaunchOptions;
    use crate::model::{Card, Harness, State};
    use crate::pending_names::PendingName;
    use crate::terminal::{Call, FakeTerminal};

    /// `store` with `list` as the agents installed on this machine.
    fn with_agents(store: Mutex<Store>, list: Vec<agents::AgentInfo>) -> Mutex<Store> {
        Mutex::new(store.into_inner().unwrap().with_agents(list))
    }

    /// A working session's card, under the agent's own name.
    fn test_card(id: &str, harness: Harness, cwd: &str) -> Card {
        Card { session_id: id.into(), pid: 1, name: format!("auto-{id}"), cwd: cwd.into(), state: State::Working, state_since: 0, snippet: String::new(), awaiting: None, has_inbox: false, harness, pr: None, context: None, machine: None, machine_address: None, machine_platform: None, terminal: None, stale: false, model: None }
    }

    /// Codex rollout lines: a turn starting, and that turn finished with a reply.
    const TURN_STARTED: &str = r#"{"timestamp":"2026-09-29T08:47:58.953Z","type":"event_msg","payload":{"type":"task_started","turn_id":"t1"}}"#;
    const TURN_COMPLETE: &str = r#"{"timestamp":"2026-09-29T08:48:01.517Z","type":"event_msg","payload":{"type":"task_complete","turn_id":"t1","last_agent_message":"Hello"}}"#;

    /// A store holding one Codex session "c1" on "ttys009" whose rollout is
    /// `rollout`, with the temp dir and the rollout's path.
    fn store_with_codex(rollout: &str) -> (tempfile::TempDir, Mutex<Store>, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("rollout.jsonl");
        std::fs::write(&path, rollout).unwrap();
        let s = ForeignSession { harness: Harness::Codex, pid: 77, tty: Some("ttys009".into()), session_id: "c1".into(), cwd: "/x".into(), name: "Say hi".into(), transcript_path: path.clone() };
        let store = Store::new(t.path().join("claude")).with_alive(|_| true).with_foreign(t.path().join("codex"), t.path().join("agy"), vec![s]);
        (t, Mutex::new(store), path)
    }

    #[test]
    fn controls_follow_the_agents_capabilities() {
        let (_t, store, _path) = store_with_codex(&format!("{TURN_STARTED}\n{TURN_COMPLETE}\n"));
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        // Codex: /compact is typed (with Codex's second Enter), a /model value is refused, Shift+Tab has no meaning.
        compact_session(&l, "c1").unwrap();
        assert!(fake.calls.lock().unwrap().iter().any(|c| matches!(c, Call::Type { tty, text } if tty == "ttys009" && text == "/compact")));
        assert_eq!(set_session_option(&l, "c1", "model", "gpt-5.5").unwrap_err(), "Codex has no /model.");
        assert_eq!(cycle_session_mode(&l, "c1").unwrap_err(), "Codex has no mode cycle.");
        send_slash_command(&l, "c1", "/status").unwrap();
        assert!(fake.calls.lock().unwrap().iter().any(|c| matches!(c, Call::Type { text, .. } if text == "/status")));
        assert_eq!(send_slash_command(&l, "c1", "!ls").unwrap_err(), "Codex has no ! shell lines.");
    }

    fn opencode_store(routes: Vec<(&'static str, &'static str)>) -> (tempfile::TempDir, Mutex<Store>, std::sync::Arc<Mutex<Vec<String>>>) {
        let (base, hits) = crate::opencode::fake::fake_server(routes);
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir_all(state.join("opencode")).unwrap();
        let me = std::process::id();
        std::fs::write(state.join("opencode/service.json"), format!("{{\"url\":\"{base}\",\"pid\":{me},\"password\":\"pw\"}}")).unwrap();
        let store = Store::new(dir.path().join("claude")).with_alive(move |pid| pid == me as i32).with_opencode(state, vec![(31, "/Users/tiagocorreia".into())]);
        (dir, Mutex::new(store), hits)
    }

    const SES: &str = "ses_efe57d285ffeRMX70aswKZ9qCO";

    fn base_routes(active: &'static str, permission: &'static str) -> Vec<(&'static str, &'static str)> {
        vec![
            ("GET /api/session?limit=50", include_str!("../fixtures/opencode/sessions.json")),
            ("GET /api/session/active", active),
            ("GET /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/permission", permission),
            ("GET /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/form", "{\"data\":[]}"),
            ("GET /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/message?limit=5&order=desc", include_str!("../fixtures/opencode/messages.json")),
            ("GET /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/message?limit=60&order=desc", include_str!("../fixtures/opencode/messages.json")),
            ("POST /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/prompt", "{\"data\":{}}"),
            ("POST /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/permission/per_01/reply", "{\"data\":{}}"),
            ("PATCH /api/session/ses_efe57d285ffeRMX70aswKZ9qCO", "{\"data\":{}}"),
            ("POST /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/compact", "{\"data\":{}}"),
            ("POST /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/model", "{\"data\":{}}"),
            ("POST /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/agent", "{\"data\":{}}"),
            ("POST /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/command", "{\"data\":{}}"),
            ("POST /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/shell", "{\"data\":{}}"),
            ("GET /api/session/ses_efe57d285ffeRMX70aswKZ9qCO", "{\"data\":{\"id\":\"ses_efe57d285ffeRMX70aswKZ9qCO\",\"projectID\":\"p\",\"agent\":\"build\",\"model\":{\"providerID\":\"opencode\",\"id\":\"fledge-alpha-free\",\"variant\":\"max\"},\"cost\":0,\"tokens\":{\"input\":1,\"output\":1,\"reasoning\":0,\"cache\":{\"read\":0,\"write\":0}},\"time\":{\"created\":1,\"updated\":2},\"location\":{\"directory\":\"/Users/tiagocorreia\"}}}"),
        ]
    }

    /// The JSON body of a recorded request, after its blank line.
    fn body_json(r: &str) -> serde_json::Value {
        r.split("\n\n").last().and_then(|b| serde_json::from_str(b.trim()).ok()).unwrap_or(serde_json::Value::Null)
    }

    fn opencode_info() -> crate::agents::AgentInfo {
        crate::agents::AgentInfo { harness: Harness::OpenCode, models: vec![crate::agents::ModelInfo { id: "opencode/fledge-alpha-free".into(), label: "Fledge".into(), efforts: vec!["low".into(), "max".into()], context: None }], efforts: vec!["low".into(), "max".into()], modes: vec!["default".into(), "auto".into()] }
    }

    #[test]
    fn opencode_actions_are_server_calls_and_nothing_is_typed() {
        let (_d, store, hits) = opencode_store(base_routes("{\"data\":{}}", "{\"data\":[]}"));
        let store = with_agents(store, vec![agents::claude(), opencode_info()]);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        store.lock().unwrap().refresh(now_ms());
        send_reply(&l, SES, "go on").unwrap();
        rename_session(&l, SES, "Renamed").unwrap();
        compact_session(&l, SES).unwrap();
        set_session_option(&l, SES, "model", "opencode/fledge-alpha-free").unwrap();
        set_session_option(&l, SES, "effort", "max").unwrap();
        cycle_session_mode(&l, SES).unwrap();
        send_slash_command(&l, SES, "/share now").unwrap();
        send_slash_command(&l, SES, "!ls").unwrap();
        let turns = session_history(&l, SES).unwrap();
        assert_eq!(turns.len(), 2);
        assert!(fake.calls.lock().unwrap().is_empty(), "nothing typed into any terminal");
        let h = hits.lock().unwrap();
        let sent = |path: &str, want: serde_json::Value| h.iter().any(|r| r.lines().next().unwrap_or("").ends_with(path) && body_json(r) == want);
        assert!(sent("/prompt", serde_json::json!({"text": "go on"})), "{h:?}");
        assert!(sent(SES, serde_json::json!({"title": "Renamed"})));
        assert!(h.iter().any(|r| r.starts_with("POST") && r.contains("/compact")));
        assert!(sent("/model", serde_json::json!({"model": {"providerID": "opencode", "id": "fledge-alpha-free", "variant": "max"}})), "the model switch keeps the variant, the effort switch keeps the model");
        assert!(sent("/agent", serde_json::json!({"agent": "plan"})), "build cycles to plan");
        assert!(sent("/command", serde_json::json!({"name": "share", "text": "now"})));
        assert!(sent("/shell", serde_json::json!({"command": "ls"})));
    }

    #[test]
    fn an_opencode_control_does_not_refresh_the_whole_board_first() {
        let (_d, store, hits) = opencode_store(base_routes("{\"data\":{}}", "{\"data\":[]}"));
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        store.lock().unwrap().refresh(now_ms());
        let lists = |h: &Vec<String>| h.iter().filter(|r| r.starts_with("GET /api/session?limit=50")).count();
        let before = lists(&hits.lock().unwrap());
        compact_session(&l, SES).unwrap();
        assert_eq!(lists(&hits.lock().unwrap()), before, "a control uses the last refresh's card");
    }

    #[test]
    fn a_free_text_question_is_answered_from_the_composer() {
        let mut routes = base_routes("{\"data\":{\"ses_efe57d285ffeRMX70aswKZ9qCO\":{\"type\":\"running\"}}}", "{\"data\":[]}");
        routes[3] = ("GET /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/form", "{\"data\":[{\"id\":\"frm_02\",\"sessionID\":\"ses_efe57d285ffeRMX70aswKZ9qCO\",\"title\":\"Commit message?\",\"fields\":[{\"key\":\"message\",\"type\":\"string\",\"title\":\"Message\"}],\"state\":{\"status\":\"pending\"}}]}");
        routes.push(("POST /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/form/frm_02/reply", "{\"data\":{}}"));
        let (_d, store, hits) = opencode_store(routes);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        let card = store.lock().unwrap().card_for(SES, now_ms()).unwrap();
        assert_eq!(card.state, State::Awaiting);
        assert_eq!(card.awaiting.as_ref().unwrap().kind, crate::model::AwaitKind::Text, "no buttons: the composer answers");
        send_reply(&l, SES, "fix: the thing").unwrap();
        let h = hits.lock().unwrap();
        assert!(h.iter().any(|r| r.contains("/form/frm_02/reply") && body_json(r) == serde_json::json!({"answer": {"message": "fix: the thing"}})), "{h:?}");
        assert!(!h.iter().any(|r| r.contains("/prompt")), "not a prompt into a waiting turn");
    }

    #[test]
    fn a_reply_to_another_agents_card_never_polls_opencode() {
        let (d, store, hits) = opencode_store(base_routes("{\"data\":{}}", "{\"data\":[]}"));
        std::fs::create_dir_all(d.path().join("claude/sessions")).unwrap();
        write_session(&d.path().join("claude"), "cc1", 4242, "/Users/x/dev/eye");
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        let before = hits.lock().unwrap().len();
        let _ = send_reply(&l, "cc1", "hi");
        assert_eq!(hits.lock().unwrap().len(), before, "a Claude Code reply asks OpenCode's server nothing");
    }

    #[test]
    fn composer_text_while_a_choice_is_pending_is_refused_not_prompted() {
        let (_d, store, hits) = opencode_store(base_routes("{\"data\":{\"ses_efe57d285ffeRMX70aswKZ9qCO\":{\"type\":\"running\"}}}", include_str!("../fixtures/opencode/permission.json")));
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        store.lock().unwrap().refresh(now_ms());
        assert_eq!(send_reply(&l, SES, "yes please").unwrap_err(), "Answer the question on the card first.");
        assert!(!hits.lock().unwrap().iter().any(|r| r.contains("/prompt")));
    }

    #[test]
    fn opencode_controls_wait_for_a_free_session_but_rename_and_history_do_not() {
        let (_d, store, hits) = opencode_store(base_routes("{\"data\":{\"ses_efe57d285ffeRMX70aswKZ9qCO\":{\"type\":\"running\"}}}", "{\"data\":[]}"));
        let store = with_agents(store, vec![agents::claude(), opencode_info()]);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        store.lock().unwrap().refresh(now_ms());
        for r in [compact_session(&l, SES), set_session_option(&l, SES, "effort", "max"), send_slash_command(&l, SES, "/share"), send_slash_command(&l, SES, "!ls"), cycle_session_mode(&l, SES)] {
            assert_eq!(r.unwrap_err(), "Wait until the session is free.");
        }
        rename_session(&l, SES, "Busy but renamed").unwrap();
        assert_eq!(session_history(&l, SES).unwrap().len(), 2);
        let h = hits.lock().unwrap();
        assert!(h.iter().any(|r| r.starts_with("PATCH")));
        assert!(!h.iter().any(|r| r.contains("/compact") || r.contains("/model") || r.contains("/command") || r.contains("/shell") || r.contains("/agent")));
    }

    #[test]
    fn effort_and_mode_changes_read_the_sessions_current_model_and_agent_from_the_server() {
        let mut routes = base_routes("{\"data\":{}}", "{\"data\":[]}");
        routes.retain(|(k, _)| *k != "GET /api/session/ses_efe57d285ffeRMX70aswKZ9qCO");
        // The session has since moved to another model and to the plan agent.
        routes.push(("GET /api/session/ses_efe57d285ffeRMX70aswKZ9qCO", "{\"data\":{\"id\":\"ses_efe57d285ffeRMX70aswKZ9qCO\",\"projectID\":\"p\",\"agent\":\"plan\",\"model\":{\"providerID\":\"openai\",\"id\":\"gpt-6.1-sol\",\"variant\":\"low\"},\"cost\":0,\"tokens\":{\"input\":1,\"output\":1,\"reasoning\":0,\"cache\":{\"read\":0,\"write\":0}},\"time\":{\"created\":1,\"updated\":2},\"location\":{\"directory\":\"/Users/tiagocorreia\"}}}"));
        let (_d, store, hits) = opencode_store(routes);
        let mut info = opencode_info();
        info.models.push(crate::agents::ModelInfo { id: "openai/gpt-6.1-sol".into(), label: "GPT".into(), efforts: vec!["low".into(), "xhigh".into()], context: None });
        let store = with_agents(store, vec![agents::claude(), info]);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        store.lock().unwrap().refresh(now_ms());
        set_session_option(&l, SES, "effort", "xhigh").unwrap();
        cycle_session_mode(&l, SES).unwrap();
        let h = hits.lock().unwrap();
        assert!(h.iter().any(|r| r.contains("/model") && body_json(r) == serde_json::json!({"model": {"providerID": "openai", "id": "gpt-6.1-sol", "variant": "xhigh"}})), "the effort goes with the model the server has now");
        assert!(h.iter().any(|r| r.contains("/agent") && body_json(r) == serde_json::json!({"agent": "build"})), "plan cycles to build");
    }

    #[test]
    fn a_typed_model_or_effort_line_goes_through_the_model_switch() {
        let (_d, store, hits) = opencode_store(base_routes("{\"data\":{}}", "{\"data\":[]}"));
        let store = with_agents(store, vec![agents::claude(), opencode_info()]);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        store.lock().unwrap().refresh(now_ms());
        send_slash_command(&l, SES, "/model opencode/fledge-alpha-free").unwrap();
        assert!(send_slash_command(&l, SES, "/effort ultra").unwrap_err().contains("effort"));
        let h = hits.lock().unwrap();
        assert!(h.iter().any(|r| r.contains("/model") && body_json(r)["model"]["id"] == "fledge-alpha-free"));
        assert!(!h.iter().any(|r| r.contains("/command")), "not a generic command");
    }

    #[test]
    fn an_opencode_permission_is_answered_with_a_decision_and_a_stale_ask_is_refused() {
        let (_d, store, hits) = opencode_store(base_routes("{\"data\":{\"ses_efe57d285ffeRMX70aswKZ9qCO\":{\"type\":\"running\"}}}", include_str!("../fixtures/opencode/permission.json")));
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        let card = store.lock().unwrap().card_for(SES, now_ms()).unwrap();
        assert_eq!(card.state, State::Awaiting);
        let ask = card.state_since;
        answer_question(&l, SES, ask, 0, 1).unwrap();
        assert!(hits.lock().unwrap().iter().any(|r| r.contains("/permission/per_01/reply") && body_json(r) == serde_json::json!({"decision": "always"})));
        assert_eq!(answer_question(&l, SES, ask + 1, 0, 1).unwrap_err(), "The question has changed; look again.");
        assert!(fake.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn a_working_opencode_session_refuses_a_reply() {
        let (_d, store, hits) = opencode_store(base_routes("{\"data\":{\"ses_efe57d285ffeRMX70aswKZ9qCO\":{\"type\":\"running\"}}}", "{\"data\":[]}"));
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        store.lock().unwrap().refresh(now_ms());
        assert_eq!(send_reply(&l, SES, "hi").unwrap_err(), "Wait until the session is free.");
        assert!(!hits.lock().unwrap().iter().any(|r| r.contains("/prompt")));
    }

    #[test]
    fn closing_an_opencode_session_types_exit_into_its_window() {
        let (_d, store, _hits) = opencode_store(base_routes("{\"data\":{}}", "{\"data\":[]}"));
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        // The window's pid (31) has no tty in a test: the lookup says so rather than typing anywhere.
        let err = close_session_with(&l, SES, |_| true).unwrap_err();
        assert!(err.contains("tty") || err.contains("console") || err.contains("terminal"), "{err}");
        assert!(fake.calls.lock().unwrap().is_empty());
    }

    /// A sessions list whose folder is `dir`, for a fake server (leaked: routes
    /// are static). The folder is JSON-escaped: a Windows path has backslashes.
    fn sessions_in(dir: &str) -> &'static str {
        let escaped = serde_json::to_string(dir).unwrap();
        let text = include_str!("../fixtures/opencode/sessions.json").replace("\"/Users/tiagocorreia\"", &escaped);
        Box::leak(text.into_boxed_str())
    }

    #[test]
    fn starting_an_opencode_session_creates_it_prompts_it_and_opens_a_window_on_it() {
        let projects = tempfile::tempdir().unwrap();
        let proj = projects.path().join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        let mut routes = base_routes("{\"data\":{}}", "{\"data\":[]}");
        routes.push(("POST /api/session", "{\"data\":{\"id\":\"ses_new01\"}}"));
        routes.push(("POST /api/session/ses_new01/prompt", "{\"data\":{}}"));
        let (_d, store, hits) = opencode_store(routes);
        store.lock().unwrap().config.projects_dir = Some(projects.path().to_string_lossy().into_owned());
        let store = with_agents(store, vec![agents::claude(), opencode_info()]);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        let opts = LaunchOptions { agent: Harness::OpenCode, model: Some("opencode/fledge-alpha-free".into()), effort: Some("low".into()), mode: Some("auto".into()), name: Some("Fix CI".into()) };
        start_session(&l, Some("proj".into()), "-v please say hi".into(), opts).unwrap();
        let h = hits.lock().unwrap();
        let create = h.iter().find(|r| r.starts_with("POST /api/session\n")).map(|r| body_json(r)).expect("created on the server");
        assert_eq!(create["title"], "Fix CI");
        assert!(create["location"]["directory"].as_str().unwrap().ends_with("proj"));
        assert_eq!(create["model"], serde_json::json!({"providerID": "opencode", "id": "fledge-alpha-free", "variant": "low"}));
        assert!(h.iter().any(|r| r.contains("/ses_new01/prompt") && body_json(r) == serde_json::json!({"text": "-v please say hi"})), "the prompt goes over the wire, never a shell line");
        let opened: Vec<String> = fake.calls.lock().unwrap().iter().filter_map(|c| match c { Call::Open { command, .. } => Some(command.clone()), _ => None }).collect();
        assert_eq!(opened.len(), 1);
        assert!(opened[0].ends_with("&& opencode --session 'ses_new01' --auto"), "{}", opened[0]);
        let bad = LaunchOptions { agent: Harness::OpenCode, model: Some("opencode/fledge-alpha-free".into()), effort: Some("ultra".into()), ..Default::default() };
        assert!(start_session(&l, Some("proj".into()), "hi".into(), bad).unwrap_err().contains("effort"), "an effort the model lacks is refused before anything runs");
    }

    #[test]
    fn the_server_is_started_on_demand_and_a_start_that_leaves_no_file_fails() {
        let t = tempfile::tempdir().unwrap();
        let file = t.path().join("opencode/service.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let me = std::process::id();
        let started = std::cell::Cell::new(false);
        let s = crate::opencode::ensure_service(&file, &|pid| pid == me as i32, || { started.set(true); std::fs::write(&file, format!("{{\"url\":\"http://127.0.0.1:1\",\"pid\":{me},\"password\":\"x\"}}")).unwrap(); Ok(()) }, Duration::from_secs(2)).unwrap();
        assert!(started.get() && s.pid == me as i32);
        let s2 = crate::opencode::ensure_service(&file, &|pid| pid == me as i32, || panic!("already running"), Duration::from_secs(2)).unwrap();
        assert_eq!(s2.url, "http://127.0.0.1:1");
        std::fs::remove_file(&file).unwrap();
        let err = crate::opencode::ensure_service(&file, &|_| false, || Ok(()), Duration::from_millis(300)).unwrap_err();
        assert_eq!(err, "OpenCode's server did not start.");
    }

    #[test]
    fn opencode_sessions_of_a_folder_are_listed_from_the_server_and_resumed_by_id() {
        let projects = tempfile::tempdir().unwrap();
        let proj = projects.path().join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        let dir = proj.to_string_lossy().into_owned();
        let mut routes = base_routes("{\"data\":{}}", "{\"data\":[]}");
        routes[0] = ("GET /api/session?limit=50", sessions_in(&dir));
        routes.push(("GET /api/session?limit=200", sessions_in(&dir)));
        let (_d, store, _hits) = opencode_store(routes);
        store.lock().unwrap().config.projects_dir = Some(projects.path().to_string_lossy().into_owned());
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        let list = list_resumable_sessions(&l, Harness::OpenCode, "proj").unwrap();
        assert!(list.iter().any(|s| s.id == SES && s.title == "Saying \"Hi\" request"), "{list:?}");
        assert!(list.windows(2).all(|w| w[0].last_active_ms >= w[1].last_active_ms), "newest first");
        resume_session(&l, Harness::OpenCode, "proj", SES).unwrap();
        let opened: Vec<String> = fake.calls.lock().unwrap().iter().filter_map(|c| match c { Call::Open { command, .. } => Some(command.clone()), _ => None }).collect();
        assert!(opened[0].ends_with(&format!("&& opencode --session '{SES}'")), "{}", opened[0]);
        assert_eq!(resume_session(&l, Harness::OpenCode, "proj", "ses_nope").unwrap_err(), "No such session in that folder.");
    }

    #[test]
    fn a_kiro_session_takes_compact_model_effort_and_shell_lines() {
        let (t, store, _path) = store_with_codex(&format!("{TURN_STARTED}\n{TURN_COMPLETE}\n"));
        let sessions = t.path().join("kiro");
        std::fs::create_dir_all(&sessions).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/kiro");
        let id = "efcba1da-5c0b-4b39-96f9-9a4740ead331";
        for (from, to) in [("lock.json", "lock"), ("session.json", "json")] {
            std::fs::copy(fixtures.join(from), sessions.join(format!("{id}.{to}"))).unwrap();
        }
        // A finished turn, no marker: the card is Completed, so free.
        let finished: String = std::fs::read_to_string(fixtures.join("events.jsonl")).unwrap().lines().take(4).map(|l| format!("{l}\n")).collect();
        std::fs::write(sessions.join(format!("{id}.jsonl")), finished).unwrap();
        let procs = crate::foreign::parse_ps_tree("95245 93903 ttys010 kiro-cli chat\n95295 95245 ttys010 kiro-cli-chat chat\n95441 95295 ttys010 bun tui.js chat\n95508 95441 ?? kiro-cli-chat acp\n");
        let store = Mutex::new(store.into_inner().unwrap().with_kiro(sessions, t.path().join("no-run"), procs));
        let info = crate::agents::AgentInfo { harness: Harness::Kiro, models: vec![crate::agents::ModelInfo { id: "auto".into(), label: "auto".into(), efforts: vec![], context: None }], efforts: vec!["low".into(), "high".into()], modes: vec![] };
        let store = with_agents(store, vec![agents::claude(), info]);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        store.lock().unwrap().refresh(now_ms());
        compact_session(&l, id).unwrap();
        set_session_option(&l, id, "model", "auto").unwrap();
        set_session_option(&l, id, "effort", "high").unwrap();
        send_slash_command(&l, id, "!ls").unwrap();
        let typed: Vec<String> = fake.calls.lock().unwrap().iter().filter_map(|c| match c { Call::Type { tty, text } if tty == "/dev/ttys010" => Some(text.clone()), _ => None }).collect();
        assert_eq!(typed, vec!["/compact", "/model auto", "/effort high", "!ls"]);
        // Close: /exit into Kiro, then exit into the shell once the TUI is gone.
        let waited = Mutex::new(None);
        close_session_with(&l, id, |pid| {
            *waited.lock().unwrap() = Some(pid);
            true
        })
        .unwrap();
        assert_eq!(*waited.lock().unwrap(), Some(95441), "waits on the TUI's pid");
        let typed: Vec<String> = fake.calls.lock().unwrap().iter().filter_map(|c| match c { Call::Type { tty, text } if tty == "/dev/ttys010" => Some(text.clone()), _ => None }).collect();
        assert_eq!(&typed[4..], ["/exit", "exit"]);
    }

    #[test]
    fn a_model_switch_is_checked_against_the_agents_own_list() {
        let (t, store, _path) = store_with_codex(&format!("{TURN_STARTED}\n{TURN_COMPLETE}\n"));
        // A Grok session whose last turn has ended, so the card is free. Grok's
        // own registry (under `with_foreign`'s Grok folder) keeps it listed.
        let grok_dir = t.path().join("agy").join("no-grok");
        let dir = t.path().join("grok-session");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&grok_dir).unwrap();
        std::fs::write(grok_dir.join("active_sessions.json"), r#"[{"session_id":"g1","pid":78,"cwd":"/x"}]"#).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/grok");
        for f in ["events.jsonl", "chat_history.jsonl"] {
            std::fs::copy(fixtures.join(f), dir.join(f)).unwrap();
        }
        let grok = ForeignSession { harness: Harness::Grok, pid: 78, tty: Some("ttys010".into()), session_id: "g1".into(), cwd: "/x".into(), name: "Grok".into(), transcript_path: dir.join("events.jsonl") };
        let store = Mutex::new(store.into_inner().unwrap().with_processes(vec![grok]));
        let info = crate::agents::AgentInfo { harness: Harness::Grok, models: vec![crate::agents::ModelInfo { id: "grok-4.7".into(), label: "grok-4.7".into(), efforts: vec![], context: None }], efforts: vec![], modes: vec![] };
        let store = with_agents(store, vec![agents::claude(), info]);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        store.lock().unwrap().refresh(now_ms());
        assert!(set_session_option(&l, "g1", "model", "grok-9").unwrap_err().contains("Unknown model"));
        set_session_option(&l, "g1", "model", "grok-4.7").unwrap();
        assert!(fake.calls.lock().unwrap().iter().any(|c| matches!(c, Call::Type { tty, text } if tty == "ttys010" && text == "/model grok-4.7")));
    }

    /// Codex listing two models that take different efforts.
    fn codex_info() -> crate::agents::AgentInfo {
        crate::agents::info_for(
            Harness::Codex,
            Some(r#"{"models":[{"slug":"gpt-6.1-sol","display_name":"GPT-6.1-Sol","visibility":"list","supported_reasoning_levels":[{"effort":"low"},{"effort":"ultra"}]},{"slug":"gpt-5.5","display_name":"GPT-5.5","visibility":"list","supported_reasoning_levels":[{"effort":"low"},{"effort":"high"}]}]}"#),
        )
    }

    #[test]
    fn a_codex_effort_must_be_one_the_chosen_model_takes() {
        let (dir, store) = store_with_projects(&["proj"]);
        let store = with_agents(store, vec![agents::claude(), codex_info()]);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        let opts = |model: Option<&str>, effort: &str| LaunchOptions { agent: Harness::Codex, model: model.map(String::from), effort: Some(effort.into()), ..Default::default() };
        assert_eq!(start_session(&l, Some("proj".into()), "hello".into(), opts(Some("gpt-5.5"), "ultra")).unwrap_err(), "Unknown effort for gpt-5.5: ultra");
        // With "Default" the model may be either: only what both take.
        assert_eq!(start_session(&l, Some("proj".into()), "hello".into(), opts(None, "high")).unwrap_err(), "Unknown effort for the default model: high");
        assert!(fake.calls.lock().unwrap().is_empty(), "nothing opened");
        start_session(&l, Some("proj".into()), "hello".into(), opts(Some("gpt-5.5"), "high")).unwrap();
        start_session(&l, Some("proj".into()), "hello".into(), opts(None, "low")).unwrap();
        assert_eq!(fake.calls.lock().unwrap().len(), 2);
        drop(dir);
    }

    #[test]
    fn a_session_started_by_hand_but_not_yet_discovered_never_takes_a_new_name() {
        let (dir, store) = store_with_projects(&["proj"]);
        let proj_path = dir.path().join("projects").join("proj").to_string_lossy().into_owned();
        // Running, but no refresh has looked it up yet.
        let by_hand = ForeignSession { harness: Harness::Codex, pid: 88, tty: Some("ttys011".into()), session_id: "c-hand".into(), cwd: proj_path.clone(), name: "By hand".into(), transcript_path: dir.path().join("none.jsonl") };
        let store = store.into_inner().unwrap().with_foreign(dir.path().join("codex"), dir.path().join("agy"), vec![]).with_processes(vec![by_hand]);
        assert!(!store.live_session_ids().contains(&"c-hand".to_string()));
        let store = with_agents(Mutex::new(store), vec![agents::claude(), codex_info()]);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        let opts = LaunchOptions { agent: Harness::Codex, name: Some("Fix CI".into()), ..Default::default() };
        start_session(&l, Some("proj".into()), "hello".into(), opts).unwrap();
        let mut cards = vec![test_card("c-hand", Harness::Codex, &proj_path)];
        store.lock().unwrap().pending_apply_for_test(&mut cards);
        assert_eq!(cards[0].name, "auto-c-hand");
        drop(dir);
    }

    #[test]
    fn lines_typed_at_once_into_one_codex_session_never_interleave() {
        let fake = FakeTerminal::default();
        let store = Mutex::new(Store::new(PathBuf::from("/nonexistent")));
        let l = Local { store: &store, terminal: &fake };
        std::thread::scope(|s| {
            s.spawn(|| type_line_for(&l, Harness::Codex, "ttys009", "/rename x").unwrap());
            s.spawn(|| type_line_for(&l, Harness::Codex, "ttys009", "a reply").unwrap());
        });
        let typed: Vec<String> = fake.calls.lock().unwrap().iter().map(|c| match c {
            Call::Type { text, .. } => text.clone(),
            other => panic!("{other:?}"),
        }).collect();
        let (a, b) = ("/rename x".to_string(), "a reply".to_string());
        let e = String::new();
        assert!(typed == [a.clone(), e.clone(), b.clone(), e.clone()] || typed == [b, e.clone(), a, e], "{typed:?}");
    }

    #[test]
    fn start_session_runs_the_chosen_agent_and_keeps_its_name_for_later() {
        let (dir, store) = store_with_projects(&["proj"]);
        let store = with_agents(store, vec![agents::claude(), codex_info()]);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        let proj_path = dir.path().join("projects").join("proj").to_string_lossy().into_owned();
        let opts = LaunchOptions { agent: Harness::Codex, model: Some("gpt-6.1-sol".into()), name: Some("Fix CI".into()), ..Default::default() };
        start_session(&l, Some("proj".into()), "hello".into(), opts).unwrap();
        let Call::Open { command, .. } = fake.calls.lock().unwrap()[0].clone() else { panic!() };
        assert!(command.ends_with("&& codex -m gpt-6.1-sol -- \"$p\""), "{command}");
        // A Codex session in that folder, new since the launch, takes the name.
        let mut cards = vec![test_card("c-new", Harness::Codex, &proj_path)];
        store.lock().unwrap().pending_apply_for_test(&mut cards);
        assert_eq!(cards[0].name, "Fix CI");
        drop(dir);
    }

    #[test]
    fn start_session_refuses_an_agent_that_is_not_installed_or_a_model_it_does_not_list() {
        let (dir, store) = store_with_projects(&["proj"]);
        let store = with_agents(store, vec![agents::claude(), codex_info()]);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        let opts = LaunchOptions { agent: Harness::Antigravity, ..Default::default() };
        assert!(start_session(&l, Some("proj".into()), "hello".into(), opts).unwrap_err().contains("not installed"));
        let opts = LaunchOptions { agent: Harness::Codex, model: Some("gpt-9".into()), ..Default::default() };
        assert!(start_session(&l, Some("proj".into()), "hello".into(), opts).unwrap_err().contains("model"));
        assert!(fake.calls.lock().unwrap().is_empty());
        drop(dir);
    }

    #[test]
    fn start_review_runs_maya_agent_and_queues_the_review_name() {
        let (dir, store) = store_with_projects(&["elsewhere"]);
        {
            let mut s = store.lock().unwrap();
            s.config.agent = Some(Harness::Codex);
            s.config.clones_dir = Some(dir.path().join("clones").to_string_lossy().into_owned());
            s.config.review_prompt = "/should-i-approve".into();
        }
        let store = with_agents(store, vec![agents::claude(), crate::agents::info_for(Harness::Codex, None)]);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        let pr = crate::reviews::ReviewPr { number: 451, repo: "Org/bedrock".into(), title: "docs".into(), author: "jane".into(), url: "https://github.com/Org/bedrock/pull/451".into(), is_draft: false, updated_at: String::new(), reasons: vec![] };
        assert!(!dir.path().join("clones").exists());
        let folder = start_review(&l, &pr).unwrap();
        assert!(Path::new(&folder).ends_with(Path::new("clones").join("bedrock-451")), "{folder}");
        // The terminal opens in the clones root, so it must exist by then.
        assert!(dir.path().join("clones").is_dir());
        let calls = fake.calls.lock().unwrap();
        let Call::Open { command, .. } = &calls[0] else { panic!("{calls:?}") };
        assert!(command.contains("gh repo clone 'Org/bedrock'") && command.ends_with("&& codex -- \"$p\""), "{command}");
        let file = command.split("cat '").nth(1).unwrap().split('\'').next().unwrap();
        assert_eq!(std::fs::read_to_string(file).unwrap(), "/should-i-approve PR #451 (https://github.com/Org/bedrock/pull/451)");
        drop(calls);
        assert!(store.lock().unwrap().has_pending_name_for_test("review bedrock #451"), "the name waits for the Codex session");
        // The clone folder does not exist yet: its pending cwd must still be
        // resolved physically, through whatever symlink its existing
        // ancestor sits behind (macOS's /tmp, a linked clones directory).
        let canonical_clone = std::fs::canonicalize(dir.path()).unwrap().join("clones").join("bedrock-451");
        assert_eq!(store.lock().unwrap().pending_cwds_for_test(), vec![canonical_clone.to_string_lossy().into_owned()]);
    }

    #[test]
    fn start_review_refuses_an_agent_that_is_not_installed() {
        let (_dir, store) = store_with_projects(&["elsewhere"]);
        store.lock().unwrap().config.agent = Some(Harness::Grok);
        let store = with_agents(store, vec![agents::claude()]);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        let pr = crate::reviews::ReviewPr { number: 1, repo: "o/r".into(), title: String::new(), author: String::new(), url: "https://github.com/o/r/pull/1".into(), is_draft: false, updated_at: String::new(), reasons: vec![] };
        assert_eq!(start_review(&l, &pr).unwrap_err(), "Grok Build is not installed on this machine.");
        assert!(fake.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn a_claude_start_does_not_wait_for_the_agents_listing() {
        // The listing lacks Claude Code entirely: a Claude start never consults it.
        let (dir, store) = store_with_projects(&["proj"]);
        let store = with_agents(store, vec![codex_info()]);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        let opts = LaunchOptions { model: Some("opus".into()), ..Default::default() };
        start_session(&l, Some("proj".into()), "hello".into(), opts).unwrap();
        let Call::Open { command, .. } = fake.calls.lock().unwrap()[0].clone() else { panic!() };
        assert!(command.contains("&& claude --model opus -- \"$p\""), "{command}");
        drop(dir);
    }

    #[test]
    fn a_grok_start_passes_a_session_id() {
        let (dir, store) = store_with_projects(&["proj"]);
        let store = with_agents(store, vec![agents::claude(), agents::info_for(Harness::Grok, None)]);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        start_session(&l, Some("proj".into()), "hello".into(), LaunchOptions { agent: Harness::Grok, name: Some("x".into()), ..Default::default() }).unwrap();
        let Call::Open { command, .. } = fake.calls.lock().unwrap()[0].clone() else { panic!() };
        assert!(command.contains("grok --session-id '") && command.ends_with("' -- \"$p\""), "{command}");
        drop(dir);
    }

    #[test]
    fn rename_reaches_other_agents_only_when_they_are_free() {
        let (dir, store, rollout) = store_with_codex(&format!("{TURN_STARTED}\n"));
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        assert!(rename_session(&l, "c1", "Fix CI").unwrap_err().contains("free"));
        std::fs::write(&rollout, format!("{TURN_STARTED}\n{TURN_COMPLETE}\n")).unwrap();
        rename_session(&l, "c1", "Fix CI").unwrap();
        let typed = fake.calls.lock().unwrap().clone();
        assert_eq!(typed, vec![Call::Type { tty: "ttys009".into(), text: "/rename Fix CI".into() }, Call::Type { tty: "ttys009".into(), text: String::new() }]);
        drop(dir);
    }

    #[test]
    fn a_line_for_codex_is_submitted_by_a_lone_enter_typed_apart() {
        let fake = FakeTerminal::default();
        let l = Local { store: &Mutex::new(Store::new(PathBuf::from("/nonexistent"))), terminal: &fake };
        type_line_for(&l, Harness::Codex, "ttys009", "/rename x").unwrap();
        let enter = Call::Type { tty: "ttys009".into(), text: String::new() };
        assert_eq!(fake.calls.lock().unwrap().clone(), vec![Call::Type { tty: "ttys009".into(), text: "/rename x".into() }, enter]);
        for harness in [Harness::Antigravity, Harness::Grok, Harness::ClaudeCode] {
            let fake = FakeTerminal::default();
            let l = Local { store: l.store, terminal: &fake };
            type_line_for(&l, harness, "ttys010", "/rename x").unwrap();
            assert_eq!(fake.calls.lock().unwrap().clone(), vec![Call::Type { tty: "ttys010".into(), text: "/rename x".into() }], "{harness:?}");
        }
    }

    #[test]
    fn a_pending_name_is_typed_as_one_rename_line() {
        let (dir, store, _) = store_with_codex(&format!("{TURN_STARTED}\n{TURN_COMPLETE}\n"));
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        store.lock().unwrap().add_pending_name(PendingName::new(Harness::Codex, "/x", "it's \"$(rm -rf ~)\"", now_ms(), vec![], Some("c1".into())));
        store.lock().unwrap().refresh(now_ms());
        run_due_renames(&l);
        run_due_renames(&l);
        let typed: Vec<Call> = fake.calls.lock().unwrap().clone();
        assert_eq!(typed, vec![Call::Type { tty: "ttys009".into(), text: "/rename it's \"$(rm -rf ~)\"".into() }, Call::Type { tty: "ttys009".into(), text: String::new() }], "typed once, literally");
        drop(dir);
    }

    #[test]
    fn a_due_rename_is_not_typed_into_a_session_that_got_busy_meanwhile() {
        let (dir, store, rollout) = store_with_codex(&format!("{TURN_STARTED}\n{TURN_COMPLETE}\n"));
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        store.lock().unwrap().add_pending_name(PendingName::new(Harness::Codex, "/x", "Fix CI", now_ms(), vec![], Some("c1".into())));
        // The refresh that queues the rename sees it free...
        store.lock().unwrap().refresh(now_ms());
        // ...then the user starts a turn before the tick that would type it.
        const TURN2_STARTED: &str = r#"{"timestamp":"2026-09-29T08:49:00.000Z","type":"event_msg","payload":{"type":"task_started","turn_id":"t2"}}"#;
        std::fs::write(&rollout, format!("{TURN_STARTED}\n{TURN_COMPLETE}\n{TURN2_STARTED}\n")).unwrap();
        run_due_renames(&l);
        assert!(fake.calls.lock().unwrap().is_empty(), "nothing typed into a busy session");
        // Free again: the name is due once more and typed once.
        const TURN2_COMPLETE: &str = r#"{"timestamp":"2026-09-29T08:49:05.000Z","type":"event_msg","payload":{"type":"task_complete","turn_id":"t2","last_agent_message":"Done"}}"#;
        std::fs::write(&rollout, format!("{TURN_STARTED}\n{TURN_COMPLETE}\n{TURN2_STARTED}\n{TURN2_COMPLETE}\n")).unwrap();
        store.lock().unwrap().refresh(now_ms());
        run_due_renames(&l);
        run_due_renames(&l);
        let typed = fake.calls.lock().unwrap().clone();
        assert_eq!(typed, vec![Call::Type { tty: "ttys009".into(), text: "/rename Fix CI".into() }, Call::Type { tty: "ttys009".into(), text: String::new() }], "typed once");
        drop(dir);
    }

    #[test]
    fn a_due_rename_that_cannot_be_typed_is_logged() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("rollout.jsonl");
        std::fs::write(&path, format!("{TURN_STARTED}\n{TURN_COMPLETE}\n")).unwrap();
        // No tty recorded, and no process to ask `ps` about.
        let s = ForeignSession { harness: Harness::Codex, pid: 2_000_000_000, tty: None, session_id: "c-notty".into(), cwd: "/x".into(), name: "Say hi".into(), transcript_path: path };
        let store = Mutex::new(Store::new(t.path().join("claude")).with_alive(|_| true).with_foreign(t.path().join("codex"), t.path().join("agy"), vec![s]));
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        store.lock().unwrap().add_pending_name(PendingName::new(Harness::Codex, "/x", "Fix CI", now_ms(), vec![], Some("c-notty".into())));
        store.lock().unwrap().refresh(now_ms());
        run_due_renames(&l);
        assert!(fake.calls.lock().unwrap().is_empty());
        assert!(crate::log::lines().iter().any(|l| l.source == "rename" && l.text.starts_with("could not rename c-notty: ")), "{:?}", crate::log::lines());
    }

    #[test]
    fn slash_commands_are_typed_into_the_sessions_tty_and_refused_off_tmux() {
        let (dir, store) = store_with_session("s1", 4242);
        let t = FakeTerminal::default();
        let l = Local { store: &store, terminal: &t };
        compact_session(&l, "s1").unwrap();
        assert!(matches!(&t.calls.lock().unwrap()[0], Call::Type { text, .. } if text == "/compact"));
        let refusing = FakeTerminal { fail_type: Some("This session is not in tmux; only replies reach it.".into()), ..Default::default() };
        let l2 = Local { store: &store, terminal: &refusing };
        assert_eq!(cycle_session_mode(&l2, "s1"), Err("This session is not in tmux; only replies reach it.".into()));
        drop(dir);
    }

    #[test]
    fn close_types_exit_into_claude_then_exit_into_the_shell_once_claude_has_gone() {
        let (dir, store) = store_with_session("s1", 4242);
        let t = FakeTerminal::default();
        let l = Local { store: &store, terminal: &t };
        let waited = Mutex::new(None);
        close_session_with(&l, "s1", |pid| {
            *waited.lock().unwrap() = Some(pid);
            true
        })
        .unwrap();
        assert_eq!(*waited.lock().unwrap(), Some(4242));
        let texts: Vec<_> = t.calls.lock().unwrap().iter().map(|c| match c { Call::Type { text, .. } => text.clone(), other => panic!("{other:?}") }).collect();
        assert_eq!(texts, ["/exit", "exit"]);
        drop(dir);
    }

    #[test]
    fn close_types_exit_through_a_key_that_still_reaches_the_terminal_once_claude_has_gone() {
        let (dir, store) = store_with_session("s1", 4242);
        let t = FakeTerminal::default();
        t.peers.lock().unwrap().insert("/dev/pts/3".into(), vec!["console:2".into(), "console:3".into()]);
        t.dead.lock().unwrap().insert("console:2".into());
        close_session_with(&Local { store: &store, terminal: &t }, "s1", |_| true).unwrap();
        assert_eq!(
            *t.calls.lock().unwrap(),
            [Call::Type { tty: "/dev/pts/3".into(), text: "/exit".into() }, Call::Type { tty: "console:3".into(), text: "exit".into() }]
        );
        drop(dir);
    }

    #[test]
    fn close_leaves_the_shell_alone_when_claude_does_not_exit() {
        let (dir, store) = store_with_session("s1", 4242);
        let t = FakeTerminal::default();
        let l = Local { store: &store, terminal: &t };
        assert_eq!(close_session_with(&l, "s1", |_| false), Err("Claude Code did not exit; the terminal was left open.".into()));
        assert_eq!(t.calls.lock().unwrap().len(), 1);
        drop(dir);
    }

    #[test]
    fn close_gives_codex_the_second_enter_that_submits_exit() {
        let (_t, store, _path) = store_with_codex(&format!("{TURN_STARTED}\n{TURN_COMPLETE}\n"));
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        close_session_with(&l, "c1", |_| true).unwrap();
        let texts: Vec<_> = fake.calls.lock().unwrap().iter().filter_map(|c| match c { Call::Type { text, .. } => Some(text.clone()), _ => None }).collect();
        assert_eq!(texts, ["/exit", "", "exit"], "Codex reads /exit plus Enter as a paste; a lone Enter after a pause submits it");
    }

    #[test]
    fn close_refuses_a_working_session() {
        let (dir, store) = store_with_session("s1", 4242);
        let entry = serde_json::json!({"pid": 4242, "sessionId": "s1", "cwd": "/Users/x/dev/eye", "name": "s1", "status": "busy", "tty": "/dev/pts/3"});
        std::fs::write(dir.path().join("sessions/4242.json"), entry.to_string()).unwrap();
        let t = FakeTerminal::default();
        let l = Local { store: &store, terminal: &t };
        assert_eq!(close_session_with(&l, "s1", |_| true), Err("Only an idle or completed session can be closed.".into()));
        assert!(t.calls.lock().unwrap().is_empty());
        drop(dir);
    }

    #[test]
    fn start_session_opens_the_session_command_under_a_maya_label() {
        let (dir, store) = store_with_projects(&["proj"]);
        let t = FakeTerminal::default();
        let l = Local { store: &store, terminal: &t };
        let r = start_session(&l, Some("proj".into()), "hello".into(), LaunchOptions::default()).unwrap();
        assert_eq!(r.how, "chosen");
        let label = r.terminal.unwrap();
        assert!(label.starts_with("maya-") && label.len() == 13, "{label}");
        let calls = t.calls.lock().unwrap();
        assert!(matches!(&calls[0], Call::Open { command, cwd, label: l2 } if command.contains("claude") && cwd.ends_with("proj") && l2 == &label));
        drop(dir);
    }

    #[test]
    fn resume_refuses_a_running_session_and_an_unknown_one() {
        let (dir, store) = store_with_projects_and_running(&["proj"], "s1");
        let t = FakeTerminal::default();
        let l = Local { store: &store, terminal: &t };
        assert_eq!(resume_session(&l, Harness::ClaudeCode, "proj", "s1"), Err("That session is already running.".into()));
        assert_eq!(resume_session(&l, Harness::ClaudeCode, "proj", "nope"), Err("No such session in that folder.".into()));
        assert!(t.calls.lock().unwrap().is_empty());
        drop(dir);
    }

    #[test]
    fn resume_refuses_ids_the_agent_did_not_record_and_resumes_known_ones() {
        let (dir, store) = store_with_projects(&["proj"]);
        let grok_dir = dir.path().join("grok");
        let proj = dir.path().join("projects").join("proj");
        let folder = grok_dir.join("sessions").join(crate::grok::encode_cwd(&proj.to_string_lossy())).join("g-1");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("summary.json"), r#"{"session_summary":"Probe","updated_at":"2026-10-02T20:24:44.754218Z"}"#).unwrap();
        let store = Mutex::new(store.into_inner().unwrap().with_agent_dirs(dir.path().join("codex"), dir.path().join("agy"), grok_dir));
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        assert_eq!(resume_session(&l, Harness::Grok, "proj", "../g-1").unwrap_err(), "No such session in that folder.");
        assert_eq!(resume_session(&l, Harness::Grok, "proj", "g-9").unwrap_err(), "No such session in that folder.");
        assert_eq!(resume_session(&l, Harness::Codex, "proj", "g-1").unwrap_err(), "No such session in that folder.", "another agent's id");
        resume_session(&l, Harness::Grok, "proj", "g-1").unwrap();
        let calls = fake.calls.lock().unwrap();
        assert!(matches!(&calls[0], Call::Open { command, .. } if command.ends_with("&& grok -r 'g-1'")), "{calls:?}");
    }

    #[test]
    fn focus_asks_the_terminal_for_the_sessions_tty() {
        let (dir, store) = store_with_session("s1", 4242);
        let t = FakeTerminal::default();
        focus_session(&Local { store: &store, terminal: &t }, "s1").unwrap();
        assert!(matches!(&t.calls.lock().unwrap()[0], Call::Focus { tty } if !tty.is_empty()));
        drop(dir);
    }

    #[test]
    fn tmux_labels_are_maya_and_eight_hex_chars() {
        let l = tmux_label();
        assert_eq!(l.len(), 13);
        assert!(l.starts_with("maya-") && l[5..].chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// Points `id`'s registry entry (pid `pid`, from `store_with_session`) at
    /// an inbox socket this test listens on.
    #[cfg(unix)]
    fn with_inbox(store: &Mutex<Store>, id: &str, pid: i32) -> std::os::unix::net::UnixListener {
        let claude = store.lock().unwrap().claude_dir().to_path_buf();
        let socket = claude.join(format!("{id}.sock"));
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let path = claude.join(format!("sessions/{pid}.json"));
        let mut entry: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        entry["messagingSocketPath"] = socket.to_string_lossy().into_owned().into();
        std::fs::write(&path, entry.to_string()).unwrap();
        listener
    }

    fn typed(fake: &FakeTerminal) -> Vec<String> {
        fake.calls.lock().unwrap().iter().filter_map(|c| match c { Call::Type { text, .. } => Some(text.clone()), _ => None }).collect()
    }

    #[test]
    fn a_claude_reply_is_typed_line_by_line_as_the_user() {
        let (_d, store) = store_with_session("s1", 4242);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        assert_eq!(send_reply(&l, "s1", "push it\n\nthanks"), Ok(Route::Typed));
        assert_eq!(typed(&fake), vec!["push it\\", "\\", "thanks"]);
        assert!(fake.calls.lock().unwrap().iter().all(|c| matches!(c, Call::Type { tty, .. } if tty == "/dev/pts/3")));
    }

    #[cfg(unix)]
    #[test]
    fn falls_back_to_the_inbox_when_nothing_reached_the_terminal() {
        use std::io::Read;
        let pid = std::process::id() as i32;
        let (_d, store) = store_with_session("s1", pid);
        let inbox = with_inbox(&store, "s1", pid);
        let fake = FakeTerminal { fail_type: Some("No Terminal tab found for /dev/pts/3.".into()), ..Default::default() };
        let l = Local { store: &store, terminal: &fake };
        let reader = std::thread::spawn(move || {
            let (mut s, _) = inbox.accept().unwrap();
            let mut got = String::new();
            s.read_to_string(&mut got).unwrap();
            got
        });
        assert_eq!(send_reply(&l, "s1", "approved"), Ok(Route::Inbox));
        assert_eq!(reader.join().unwrap(), inbox::message_line("approved"));
    }

    #[test]
    fn a_reply_cut_off_partway_is_not_resent_through_the_inbox() {
        let (_d, store) = store_with_session("s1", 4242);
        let fake = FakeTerminal { fail_after: Some(1), ..Default::default() };
        let l = Local { store: &store, terminal: &fake };
        assert_eq!(send_reply(&l, "s1", "one\ntwo"), Err("Part of the reply was typed into the terminal; check it there.".into()));
        assert_eq!(typed(&fake), vec!["one\\"]);
    }

    #[test]
    fn says_so_when_neither_the_terminal_nor_an_inbox_is_there() {
        let (_d, store) = store_with_session("s1", 4242);
        let fake = FakeTerminal { fail_type: Some("No Terminal tab found for /dev/pts/3.".into()), ..Default::default() };
        let l = Local { store: &store, terminal: &fake };
        assert_eq!(send_reply(&l, "s1", "hi"), Err("Maya can't type into this session's terminal and it has no inbox. Use the terminal.".into()));
    }

    #[test]
    fn a_reply_while_claude_asks_permission_is_refused_and_nothing_is_sent() {
        let (_d, store) = store_with_session("s1", 4242);
        let maya = store.lock().unwrap().claude_dir().join("maya");
        crate::hook_install::append_record_named(&maya, "events.jsonl", r#"{"session_id":"s1","hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"git push"}}"#, now_ms()).unwrap();
        store.lock().unwrap().refresh(now_ms());
        assert_eq!(store.lock().unwrap().card_for("s1", now_ms()).unwrap().state, State::Awaiting);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        assert_eq!(send_reply(&l, "s1", "yes"), Err("This session is waiting for a decision; answer it first.".into()));
        assert!(fake.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn a_typed_reply_has_its_own_length_cap() {
        let (_d, store) = store_with_session("s1", 4242);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        assert_eq!(send_reply(&l, "s1", &"x".repeat(answer::TYPED_MAX_CHARS + 1)), Err("Message is too long to type (over 20000 characters).".into()));
        assert_eq!(send_reply(&l, "s1", "  \n "), Err("Message is empty.".into()));
        assert!(fake.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn a_peer_message_in_the_history_stays_a_peer_turn() {
        let (_d, store) = store_with_session("s1", 4242);
        let s = store.lock().unwrap().session("s1").unwrap();
        let path = store.lock().unwrap().transcript_path_for(&s);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let line = serde_json::json!({"type":"user","message":{"role":"user","content":"Another Claude session sent a message:\nhello\n\nThis came from another Claude session."}});
        std::fs::write(&path, format!("{line}\n")).unwrap();
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        assert_eq!(session_history(&l, "s1").unwrap(), vec![transcript::Turn { kind: transcript::TurnKind::Peer, text: "hello".into() }]);
    }

    #[test]
    fn route_from_data_reads_the_route_and_tolerates_none() {
        assert_eq!(Route::Typed.data(), serde_json::json!({"route": "typed"}));
        assert_eq!(Route::from_data(Some(&Route::Inbox.data())), Some(Route::Inbox));
        assert_eq!(Route::from_data(None), None, "an older Maya sends no data");
        assert_eq!(Route::from_data(Some(&serde_json::json!({"route": "carrier pigeon"}))), None);
    }

    #[test]
    fn a_typed_reply_has_a_line_cap_so_it_cannot_outrun_the_phone() {
        let (_d, store) = store_with_session("s1", 4242);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        let long = vec!["x"; answer::TYPED_MAX_LINES + 1].join("\n");
        assert_eq!(send_reply(&l, "s1", &long), Err("Message has too many lines to type (over 200).".into()));
        assert!(fake.calls.lock().unwrap().is_empty());
        assert_eq!(send_reply(&l, "s1", &vec!["x"; answer::TYPED_MAX_LINES].join("\n")), Ok(Route::Typed));
    }
}
