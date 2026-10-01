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
        return l.terminal.type_line(&tty, &line);
    }
    let (socket, pid) = {
        let store = l.store.lock().unwrap();
        let s = store.session(session_id).ok_or("Session is no longer running.")?;
        let socket = s.messaging_socket_path.clone().ok_or("This session has no inbox. Use the terminal.")?;
        (socket, s.pid)
    };
    let result = inbox::send(Path::new(&socket), pid, text);
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
    type_into_session(l, session_id, answer::COMPACT)
}

/// Types `/rename <name>` into the session's terminal. The new name comes
/// back through the session registry on the next refresh.
pub fn rename_session(l: &Local, session_id: &str, name: &str) -> Result<(), String> {
    type_into_session(l, session_id, &answer::rename_command(name)?)
}

/// Types `/model x` or `/effort y` into the session's terminal.
pub fn set_session_option(l: &Local, session_id: &str, setting: &str, value: &str) -> Result<(), String> {
    type_into_session(l, session_id, &answer::slash_command(setting, value)?)
}

/// Sends Shift+Tab to the session's terminal, cycling its permission mode.
pub fn cycle_session_mode(l: &Local, session_id: &str) -> Result<(), String> {
    type_into_session(l, session_id, answer::SHIFT_TAB)
}

/// Types `text` into a free Claude Code session's terminal.
fn type_into_session(l: &Local, session_id: &str, text: &str) -> Result<(), String> {
    let tty = {
        let mut store = l.store.lock().unwrap();
        let card = store.card_for(session_id, now_ms()).ok_or("Session is no longer running.")?;
        if card.harness != model::Harness::ClaudeCode {
            return Err("That command is only available for Claude Code sessions.".into());
        }
        answer::check_free(&card)?;
        session_tty(&store, session_id, card.pid)?
    };
    l.terminal.type_line(&tty, text)
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
        None => Ok(transcript::read_turns(&path, 30)),
    }
}

/// The folders of the projects directory.
pub fn list_project_dirs(l: &Local) -> Result<Vec<String>, String> {
    let root = projects_root(&l.store.lock().unwrap())?;
    Ok(launch::list_project_dirs(&root))
}

/// Past sessions of a project folder, newest first, with running ones marked.
pub fn list_resumable_sessions(l: &Local, dir: &str) -> Result<Vec<resume::ResumableSession>, String> {
    let (path, claude_dir, running) = {
        let store = l.store.lock().unwrap();
        (project_path(&store, dir)?, store.claude_dir().to_path_buf(), store.live_session_ids())
    };
    Ok(resume::list_sessions(&claude_dir, &path.to_string_lossy(), &running))
}

/// Opens a terminal in a project folder running `claude` on `prompt`. With no
/// `dir`, Claude picks the folder from the prompt.
pub fn start_session(l: &Local, dir: Option<String>, prompt: String, options: launch::LaunchOptions) -> Result<StartResult, String> {
    if prompt.trim().is_empty() {
        return Err("Type a prompt first.".into());
    }
    options.validate()?;
    let (root, maya_dir) = {
        let store = l.store.lock().unwrap();
        (projects_root(&store)?, store.claude_dir().join("maya"))
    };
    let dirs = launch::list_project_dirs(&root);
    let picked = match dir {
        Some(_) => None,
        None => {
            let binary = launch::claude_binary().ok_or("Could not find the claude command.")?;
            launch::classify(&binary, &root, &prompt, &dirs, launch::CLASSIFIER_TIMEOUT)
        }
    };
    let (target, how) = launch::resolve_target(&root, &dirs, dir.as_deref(), picked.as_deref())?;
    let file = launch::write_prompt_file(&maya_dir, &prompt)?;
    let label = tmux_label();
    let terminal = l.terminal.open(&launch::session_command(&target, &file, &options), &target, &label)?;
    Ok(StartResult { dir: target.to_string_lossy().into_owned(), how: how.to_string(), terminal })
}

/// Opens a terminal in the folder running `claude --resume <id>`.
pub fn resume_session(l: &Local, dir: &str, session_id: &str) -> Result<(), String> {
    let (path, claude_dir, running) = {
        let store = l.store.lock().unwrap();
        (project_path(&store, dir)?, store.claude_dir().to_path_buf(), store.live_session_ids())
    };
    if running.iter().any(|id| id == session_id) {
        return Err("That session is already running.".into());
    }
    if !resume::transcript_exists(&claude_dir, &path.to_string_lossy(), session_id) {
        return Err("No such session in that folder.".into());
    }
    l.terminal.open(&resume::resume_command(&path, session_id), &path, &tmux_label()).map(|_| ())
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
    use crate::launch::LaunchOptions;
    use crate::terminal::{Call, FakeTerminal};

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
        assert_eq!(resume_session(&l, "proj", "s1"), Err("That session is already running.".into()));
        assert_eq!(resume_session(&l, "proj", "nope"), Err("No such session in that folder.".into()));
        assert!(t.calls.lock().unwrap().is_empty());
        drop(dir);
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
