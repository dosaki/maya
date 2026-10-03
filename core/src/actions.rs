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

/// Sends `text` to the session: through its inbox, or typed into the
/// terminal of a harness that has none.
pub fn send_reply(l: &Local, session_id: &str, text: &str) -> Result<(), String> {
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
        return type_line_for(l, f.harness, &tty, &line);
    }
    let (socket, pid) = {
        let store = l.store.lock().unwrap();
        let s = store.session(session_id).ok_or("Session is no longer running.")?;
        let socket = s.messaging_socket_path.clone().ok_or("This session has no inbox. Use the terminal.")?;
        (socket, s.pid)
    };
    let result = inbox::send(Path::new(&socket), pid, text);
    if result.is_ok() {
        l.store.lock().unwrap().note_sent(session_id, text);
    }
    crate::log::line(
        "reply",
        match &result {
            Ok(()) => format!("sent {} characters to {session_id} (pid {pid}) at {socket}", text.chars().count()),
            Err(e) => format!("could not send to {session_id} (pid {pid}) at {socket}: {e}"),
        },
    );
    result
}

/// Picks `option` of `question` in the session's open ask `ask_id`.
pub fn answer_question(l: &Local, session_id: &str, ask_id: u64, question: usize, option: usize) -> Result<(), String> {
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

/// Types `/compact` into the session's terminal.
pub fn compact_session(l: &Local, session_id: &str) -> Result<(), String> {
    type_into_session(l, session_id, answer::COMPACT, "/compact", |c| c.compact)
}

/// How long Close waits for Claude to exit before it leaves the shell alone.
const EXIT_WAIT: Duration = Duration::from_secs(5);

/// Ends an idle or completed Claude Code session and closes its terminal:
/// types `/exit`, waits for Claude's process to end, then types `exit` into
/// the shell left behind.
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
    // Everything is looked up before `/exit`: once Claude exits, its registry
    // entry goes, and on Windows so does the console key found through it.
    let (pid, tty) = {
        let mut store = l.store.lock().unwrap();
        let card = store.card_for(session_id, now_ms()).ok_or("Session is no longer running.")?;
        if card.harness != model::Harness::ClaudeCode {
            return Err("That command is only available for Claude Code sessions.".into());
        }
        if !matches!(card.state, model::State::Idle | model::State::Completed) {
            return Err("Only an idle or completed session can be closed.".into());
        }
        (card.pid, session_tty(&store, session_id, card.pid)?)
    };
    let after = l.terminal.reach_after_exit(&tty);
    l.terminal.type_line(&tty, answer::EXIT)?;
    // Typed while Claude still runs, `exit` would land in its prompt as a message.
    if !exited(pid) {
        return Err("Claude did not exit; the terminal was left open.".into());
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
    let shell = line.starts_with('!');
    type_into_session(l, session_id, &line, if shell { "! shell lines" } else { "/ commands" }, move |c| if shell { c.shell_lines } else { c.slash_lines })
}

/// Sends the agent's mode-cycle keys (Shift+Tab) to the session's terminal.
pub fn cycle_session_mode(l: &Local, session_id: &str) -> Result<(), String> {
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
        None => {
            // What Maya sent arrives as a message from another session: it is the user's.
            let mut turns = transcript::read_turns(&path, 30);
            let store = l.store.lock().unwrap();
            for t in turns.iter_mut().filter(|t| t.kind == transcript::TurnKind::Peer && store.was_sent(session_id, &t.text)) {
                t.kind = transcript::TurnKind::User;
            }
            Ok(turns)
        }
    }
}

/// The folders of the projects directory.
pub fn list_project_dirs(l: &Local) -> Result<Vec<String>, String> {
    let root = projects_root(&l.store.lock().unwrap())?;
    Ok(launch::list_project_dirs(&root))
}

/// The agent's past sessions of a project folder, newest first, running ones marked.
pub fn list_resumable_sessions(l: &Local, agent: model::Harness, dir: &str) -> Result<Vec<resume::ResumableSession>, String> {
    let (path, dirs, running) = {
        let store = l.store.lock().unwrap();
        (project_path(&store, dir)?, store.agent_dirs(), store.live_session_ids())
    };
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
            launch::classify(brain, &binary, model.as_deref(), &root, &prompt, &dirs, launch::CLASSIFIER_TIMEOUT)
        }
    };
    let (target, how) = launch::resolve_target(&root, &dirs, dir.as_deref(), picked.as_deref())?;
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
    let known = resume::list_sessions(agent, &dirs, &path.to_string_lossy(), &running);
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
        let pid = match store.foreign(session_id) {
            Some(f) => f.pid,
            None => store.session(session_id).ok_or("Session is no longer running.")?.pid,
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
    fn write_session(claude: &Path, id: &str, pid: i32, cwd: &str) {
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
        Card { session_id: id.into(), pid: 1, name: format!("auto-{id}"), cwd: cwd.into(), state: State::Working, state_since: 0, snippet: String::new(), awaiting: None, has_inbox: false, harness, pr: None, context: None, machine: None, machine_address: None, machine_platform: None, terminal: None, stale: false }
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

    #[test]
    fn a_kiro_session_has_no_compact_but_takes_model_effort_and_shell_lines() {
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
        let info = crate::agents::AgentInfo { harness: Harness::Kiro, models: vec![crate::agents::ModelInfo { id: "auto".into(), label: "auto".into(), efforts: vec![] }], efforts: vec!["low".into(), "high".into()], modes: vec![] };
        let store = with_agents(store, vec![agents::claude(), info]);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        store.lock().unwrap().refresh(now_ms());
        assert_eq!(compact_session(&l, id).unwrap_err(), "Kiro CLI has no /compact.");
        set_session_option(&l, id, "model", "auto").unwrap();
        set_session_option(&l, id, "effort", "high").unwrap();
        send_slash_command(&l, id, "!ls").unwrap();
        let typed: Vec<String> = fake.calls.lock().unwrap().iter().filter_map(|c| match c { Call::Type { tty, text } if tty == "/dev/ttys010" => Some(text.clone()), _ => None }).collect();
        assert_eq!(typed, vec!["/model auto", "/effort high", "!ls"]);
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
        let info = crate::agents::AgentInfo { harness: Harness::Grok, models: vec![crate::agents::ModelInfo { id: "grok-4.7".into(), label: "grok-4.7".into(), efforts: vec![] }], efforts: vec![], modes: vec![] };
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
        assert_eq!(close_session_with(&l, "s1", |_| false), Err("Claude did not exit; the terminal was left open.".into()));
        assert_eq!(t.calls.lock().unwrap().len(), 1);
        drop(dir);
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
}
