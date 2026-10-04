//! Sessions from harnesses other than Claude Code. They have no registry
//! and no inbox, so they are found from their processes and their own
//! session files, and replies are typed into their tty.

use crate::context;
use crate::model::{AwaitKind, Awaiting, Card, Harness, State};
use crate::state::{asking_line, truncate, SNIPPET_CHARS};
use std::path::PathBuf;
#[cfg(unix)]
use std::process::Stdio;

/// A live session of another harness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignSession {
    pub harness: Harness,
    pub pid: i32,
    pub tty: Option<String>,
    pub session_id: String,
    pub cwd: String,
    pub name: String,
    pub transcript_path: PathBuf,
}

/// What a harness's session file says about the current turn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForeignTail {
    pub working: bool,
    /// An approval or input request the user must answer, when one is open.
    pub awaiting: Option<String>,
    pub last_agent_text: Option<String>,
    pub last_event_ms: u64,
    pub context_used: Option<u64>,
    pub context_window: Option<u64>,
    pub cwd: Option<String>,
    pub session_id: Option<String>,
}

/// The card for a foreign session, with the same state rules as Claude Code
/// sessions: an open request wins, then a running turn, then a prose ask,
/// then Completed decaying to Idle.
pub fn derive(s: &ForeignSession, t: &ForeignTail, now_ms: u64, completed_timeout_ms: u64) -> Card {
    let prose = if t.working { None } else { t.last_agent_text.as_deref().and_then(asking_line) };
    let (state, state_since, awaiting) = if let Some(detail) = &t.awaiting {
        (State::Awaiting, t.last_event_ms, Some(Awaiting { kind: AwaitKind::Permission, detail: truncate(detail, SNIPPET_CHARS), questions: vec![] }))
    } else if t.working {
        (State::Working, t.last_event_ms, None)
    } else if let Some(line) = prose {
        (State::Awaiting, t.last_event_ms, Some(Awaiting { kind: AwaitKind::Text, detail: truncate(&line, SNIPPET_CHARS), questions: vec![] }))
    } else if now_ms.saturating_sub(t.last_event_ms) < completed_timeout_ms {
        (State::Completed, t.last_event_ms, None)
    } else {
        (State::Idle, t.last_event_ms + completed_timeout_ms, None)
    };
    let context = match (t.context_used, t.context_window) {
        (Some(used), Some(window)) if window > 0 => Some(context::ContextUsage { used, window, percent: ((used as f64 / window as f64) * 100.0).round().min(100.0) as u8 }),
        _ => None,
    };
    Card {
        session_id: s.session_id.clone(),
        pid: s.pid,
        name: s.name.clone(),
        cwd: s.cwd.clone(),
        state,
        state_since,
        snippet: truncate(t.last_agent_text.as_deref().unwrap_or(""), SNIPPET_CHARS),
        awaiting,
        has_inbox: false,
        harness: s.harness,
        pr: None,
        context,
        machine: None,
        machine_address: None, machine_platform: None,
        terminal: None,
        stale: false, model: None
    }
}

/// `(pid, tty, harness)` for every `codex` or `agy` TUI in `ps -axo pid=,tty=,command=` output.
pub fn tui_processes(ps: &str) -> Vec<(i32, String, Harness)> {
    let mut out = Vec::new();
    for line in ps.lines() {
        let mut parts = line.split_whitespace();
        let (Some(pid), Some(tty), Some(cmd)) = (parts.next(), parts.next(), parts.next()) else { continue };
        if tty == "??" || tty == "?" || tty == "-" {
            continue;
        }
        let Ok(pid) = pid.parse::<i32>() else { continue };
        let base = cmd.rsplit('/').next().unwrap_or(cmd);
        let harness = match base {
            "codex" => Harness::Codex,
            "agy" => Harness::Antigravity,
            _ => continue,
        };
        // Helpers such as `codex app-server` are not sessions.
        if harness == Harness::Codex && parts.clone().next() == Some("app-server") {
            continue;
        }
        out.push((pid, format!("/dev/{tty}"), harness));
    }
    out
}

/// The harness an executable name belongs to on Windows.
pub fn harness_for_exe(name: &str) -> Option<Harness> {
    match name.to_ascii_lowercase().as_str() {
        "codex.exe" => Some(Harness::Codex),
        "agy.exe" => Some(Harness::Antigravity),
        _ => None,
    }
}

/// `(pid, console key, harness)` for every `codex` or `agy` process.
#[cfg(windows)]
pub fn list_tui_processes() -> Vec<(i32, String, Harness)> {
    crate::win_process::list()
        .into_iter()
        .filter_map(|(pid, name)| Some((pid as i32, crate::win_console::console_key(pid as i32), harness_for_exe(&name)?)))
        .collect()
}

#[cfg(unix)]
pub fn list_tui_processes() -> Vec<(i32, String, Harness)> {
    crate::command("ps")
        .args(["-axo", "pid=,tty=,command="])
        .stdin(Stdio::null())
        .output()
        .map(|o| tui_processes(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

/// A process with its parent, terminal and program, from
/// `ps -axo pid=,ppid=,tty=,comm=`. `command` is the program's base name:
/// `comm` is the executable's path (macOS) or name (Linux) with no
/// arguments, so a path with spaces in it still ends in the program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proc {
    pub pid: i32,
    pub ppid: i32,
    pub tty: Option<String>,
    pub command: String,
}

/// The text after the first `n` whitespace-separated tokens, trimmed.
fn after_tokens(line: &str, n: usize) -> &str {
    let mut rest = line;
    for _ in 0..n {
        rest = rest.trim_start();
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        rest = &rest[end..];
    }
    rest.trim()
}

pub fn parse_ps_tree(ps: &str) -> Vec<Proc> {
    ps.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let (pid, ppid, tty) = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?);
            // The program: after the last `/` of its path, up to any argument.
            let program = after_tokens(line, 3);
            let base = program.rsplit('/').next().unwrap_or(program);
            let command = base.split_whitespace().next().unwrap_or("").to_string();
            Some(Proc { pid, ppid, tty: crate::tty::tty_from_ps(tty), command })
        })
        .collect()
}

#[cfg(unix)]
pub fn list_process_tree() -> Vec<Proc> {
    crate::command("ps")
        .args(["-axo", "pid=,ppid=,tty=,comm="])
        .stdin(Stdio::null())
        .output()
        .map(|o| parse_ps_tree(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

/// On Windows a console is found per process, by attaching to it (see
/// `console_of`), so the tree carries no "tty" of its own.
#[cfg(windows)]
pub fn list_process_tree() -> Vec<Proc> {
    let names: std::collections::HashMap<u32, String> = crate::win_process::list().into_iter().collect();
    crate::win_process::tree()
        .into_iter()
        .map(|(pid, ppid)| Proc { pid: pid as i32, ppid: ppid as i32, tty: None, command: names.get(&pid).cloned().unwrap_or_default() })
        .collect()
}

/// The console key of `pid` when it has a console Maya can attach to;
/// a headless process (a scheduler's, a service's) has none.
#[cfg(windows)]
fn console_of(pid: i32) -> Option<String> {
    crate::win_console::other_console_pids(pid as u32).ok().map(|_| crate::win_console::console_key(pid))
}

#[cfg(unix)]
fn console_of(_pid: i32) -> Option<String> {
    None
}

/// The tty of `pid` or of the nearest ancestor that has one, up to six levels.
pub fn tty_above(procs: &[Proc], pid: i32) -> Option<String> {
    let mut cur = pid;
    for _ in 0..6 {
        let p = procs.iter().find(|p| p.pid == cur)?;
        if let Some(t) = &p.tty {
            return Some(t.clone());
        }
        cur = p.ppid;
    }
    None
}

/// Whether `pid` or one of its ancestors (up to six levels) is `ancestor`.
pub fn descends_from(procs: &[Proc], pid: i32, ancestor: i32) -> bool {
    let mut cur = pid;
    for _ in 0..6 {
        if cur == ancestor {
            return true;
        }
        let Some(p) = procs.iter().find(|p| p.pid == cur) else { return false };
        cur = p.ppid;
    }
    false
}

/// Live Kiro sessions: each lock names its agent process; the session's TUI
/// is that process's parent, and the terminal is the first one above it.
/// Not a session: a lock whose pid is gone or now belongs to another program
/// (Kiro leaves the lock behind after a crash, and pids are reused), one
/// with no terminal above it (a headless run), and one under `self_pid`
/// (Maya's own one-shot runs, which inherit Maya's terminal when it has one).
pub fn kiro_sessions(sessions_dir: &std::path::Path, procs: &[Proc], self_pid: i32) -> Vec<ForeignSession> {
    crate::kiro::locks(sessions_dir)
        .into_iter()
        .filter_map(|lock| {
            let agent = procs.iter().find(|p| p.pid == lock.pid)?;
            let is_kiro = agent.command.to_ascii_lowercase().starts_with("kiro-cli");
            if !is_kiro || descends_from(procs, lock.pid, self_pid) {
                return None;
            }
            let tui = agent.ppid;
            let tty = tty_above(procs, tui).or_else(|| console_of(tui))?;
            let meta = crate::kiro::session_meta(&std::fs::read_to_string(sessions_dir.join(format!("{}.json", lock.session_id))).ok()?)?;
            let name = meta.title.unwrap_or_else(|| format!("kiro-{tui}"));
            Some(ForeignSession { harness: Harness::Kiro, pid: tui, tty: Some(tty), session_id: lock.session_id.clone(), cwd: meta.cwd, name, transcript_path: sessions_dir.join(format!("{}.jsonl", lock.session_id)) })
        })
        .collect()
}

/// What `lsof -p <pid> -Fn` reveals: the working directory and open paths.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProcInfo {
    pub cwd: Option<String>,
    pub paths: Vec<String>,
}

pub fn parse_lsof(out: &str) -> ProcInfo {
    let mut info = ProcInfo::default();
    let mut is_cwd = false;
    for line in out.lines() {
        match line.chars().next() {
            Some('f') => is_cwd = &line[1..] == "cwd",
            Some('n') => {
                let path = line[1..].to_string();
                if is_cwd {
                    info.cwd = Some(path);
                } else {
                    info.paths.push(path);
                }
            }
            _ => {}
        }
    }
    info
}

/// Sessions for the given TUI processes: a Codex process maps to the live
/// rollout recorded for its working directory; an Antigravity process to
/// the conversation folder it holds open. Processes with no session are skipped.
pub fn discover(procs: &[(i32, String, Harness)], info: impl Fn(i32) -> ProcInfo, codex_dir: &std::path::Path, agy_dir: &std::path::Path) -> Vec<ForeignSession> {
    let mut out = Vec::new();
    for (pid, tty, harness) in procs {
        let pi = info(*pid);
        let Some(cwd) = pi.cwd.clone() else { continue };
        match harness {
            Harness::Codex => {
                let Some((id, path)) = crate::codex::live_rollout_for(codex_dir, &cwd) else { continue };
                let index = std::fs::read_to_string(codex_dir.join("session_index.jsonl")).unwrap_or_default();
                let name = crate::codex::thread_name(&index, &id).unwrap_or_else(|| format!("codex-{pid}"));
                out.push(ForeignSession { harness: *harness, pid: *pid, tty: Some(tty.clone()), session_id: id, cwd, name, transcript_path: path });
            }
            Harness::Antigravity => {
                let Some(id) = crate::antigravity::conversation_id(&pi.paths, agy_dir) else { continue };
                let history = std::fs::read_to_string(agy_dir.join("history.jsonl")).unwrap_or_default();
                let name = crate::antigravity::conversation_name(&history, &id).unwrap_or_else(|| format!("agy-{pid}"));
                let path = crate::antigravity::transcript_path(agy_dir, &id);
                out.push(ForeignSession { harness: *harness, pid: *pid, tty: Some(tty.clone()), session_id: id, cwd, name, transcript_path: path });
            }
            Harness::ClaudeCode | Harness::Grok | Harness::Kiro | Harness::OpenCode | Harness::Other => {}
        }
    }
    out
}

fn tail_of(path: &std::path::Path, bytes: u64) -> String {
    crate::transcript::tail_text(path, bytes).unwrap_or_default()
}

/// The card state for a foreign session, read from its files by harness.
pub fn tail_for(s: &ForeignSession) -> ForeignTail {
    match s.harness {
        Harness::Codex => crate::codex::parse_tail(&tail_of(&s.transcript_path, crate::transcript::TAIL_BYTES)),
        Harness::Antigravity => crate::antigravity::parse_tail(&tail_of(&s.transcript_path, crate::transcript::TAIL_BYTES)),
        Harness::Grok => crate::grok::tail_for_dir(s.transcript_path.parent().unwrap_or(&s.transcript_path)),
        Harness::Kiro => crate::kiro::parse_events(&tail_of(&s.transcript_path, crate::transcript::TAIL_BYTES)),
        Harness::ClaudeCode | Harness::OpenCode | Harness::Other => ForeignTail::default(),
    }
}

/// Conversation turns for a foreign session, read from its files by harness.
pub fn turns_for(s: &ForeignSession, max_turns: usize) -> Vec<crate::transcript::Turn> {
    let big = crate::transcript::TURNS_TAIL_BYTES;
    match s.harness {
        Harness::Codex => crate::codex::parse_turns(&tail_of(&s.transcript_path, big), max_turns),
        Harness::Antigravity => crate::antigravity::parse_turns(&tail_of(&s.transcript_path, big), max_turns),
        Harness::Grok => crate::grok::parse_turns(&tail_of(&s.transcript_path.with_file_name("chat_history.jsonl"), big), max_turns),
        Harness::Kiro => crate::kiro::parse_turns(&tail_of(&s.transcript_path, big), max_turns),
        Harness::ClaudeCode | Harness::OpenCode | Harness::Other => vec![],
    }
}

/// Sessions for the given processes on Windows, where no `lsof` names a
/// process's working directory or open files: an `agy` process is found
/// in its own log (pid, workspace, conversation), a `codex` process by the
/// thread writer lock it holds (`holders` names a lock's processes).
pub fn discover_from_files(procs: &[(i32, String, Harness)], holders: impl Fn(&std::path::Path) -> Vec<u32>, codex_dir: &std::path::Path, agy_dir: &std::path::Path) -> Vec<ForeignSession> {
    let mut out = Vec::new();
    let codex_locks = if procs.iter().any(|p| p.2 == Harness::Codex) { crate::codex::locked_threads(codex_dir) } else { vec![] };
    for (pid, tty, harness) in procs {
        match harness {
            Harness::Antigravity => {
                let Some(log) = crate::antigravity::process_log(agy_dir, *pid) else { continue };
                let (Some(id), Some(cwd)) = (log.conversation, log.workspace) else { continue };
                let history = std::fs::read_to_string(agy_dir.join("history.jsonl")).unwrap_or_default();
                let name = crate::antigravity::conversation_name(&history, &id).unwrap_or_else(|| format!("agy-{pid}"));
                let path = crate::antigravity::transcript_path(agy_dir, &id);
                out.push(ForeignSession { harness: *harness, pid: *pid, tty: Some(tty.clone()), session_id: id, cwd, name, transcript_path: path });
            }
            Harness::Codex => {
                let Some(id) = codex_locks.iter().find(|(_, lock)| holders(lock).contains(&(*pid as u32))).map(|(id, _)| id.clone()) else { continue };
                let Some((path, cwd)) = crate::codex::rollout_for_id(codex_dir, &id) else { continue };
                let index = std::fs::read_to_string(codex_dir.join("session_index.jsonl")).unwrap_or_default();
                let name = crate::codex::thread_name(&index, &id).unwrap_or_else(|| format!("codex-{pid}"));
                out.push(ForeignSession { harness: *harness, pid: *pid, tty: Some(tty.clone()), session_id: id, cwd, name, transcript_path: path });
            }
            Harness::ClaudeCode | Harness::Grok | Harness::Kiro | Harness::OpenCode | Harness::Other => {}
        }
    }
    out
}

/// Live Grok Build sessions from its registry, for pids that are running.
pub fn grok_sessions(grok_dir: &std::path::Path, alive: &dyn Fn(i32) -> bool, tty_of: &dyn Fn(i32) -> Option<String>) -> Vec<ForeignSession> {
    let text = std::fs::read_to_string(grok_dir.join("active_sessions.json")).unwrap_or_default();
    crate::grok::registry(&text)
        .into_iter()
        .filter(|(_, pid, _)| alive(*pid))
        .map(|(id, pid, cwd)| {
            let dir = crate::grok::session_dir(grok_dir, &cwd, &id);
            let summary = std::fs::read_to_string(dir.join("summary.json")).unwrap_or_default();
            let name = crate::grok::title(&summary).unwrap_or_else(|| format!("grok-{pid}"));
            ForeignSession { harness: Harness::Grok, pid, tty: tty_of(pid), session_id: id, cwd, name, transcript_path: dir.join("events.jsonl") }
        })
        .collect()
}

#[cfg(unix)]
pub fn proc_info(pid: i32) -> ProcInfo {
    crate::command("lsof")
        .args(["-p", &pid.to_string(), "-Fn", "-w"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .inspect_err(|_e| {
            // macOS always has lsof; a Linux box may not.
            #[cfg(target_os = "linux")]
            {
                static MISSING: std::sync::Once = std::sync::Once::new();
                crate::log::missing_once(&MISSING, _e, "foreign", "lsof is not installed, so Codex and Antigravity sessions are not listed: install lsof");
            }
        })
        .map(|o| parse_lsof(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn concurrent_agy_and_codex_processes_each_find_their_own_session() {
        use super::*;
        let dir = tempfile::tempdir().unwrap();
        let (agy, codex) = (dir.path().join("agy"), dir.path().join("codex"));
        std::fs::create_dir_all(agy.join("log")).unwrap();
        let log = |pid: i32, ws: &str, id: &str| format!("] Starting language server process with pid {pid}\n] Initializing CLI store manager for workspace {ws}\n] Created conversation {id}\n");
        std::fs::write(agy.join("log/cli-1.log"), log(11, "/one", "11111111-1111-1111-1111-111111111111")).unwrap();
        std::fs::write(agy.join("log/cli-2.log"), log(22, "/two", "22222222-2222-2222-2222-222222222222")).unwrap();
        // A third agy that has not started a conversation yet.
        std::fs::write(agy.join("log/cli-3.log"), "] Starting language server process with pid 33\n").unwrap();
        let day = codex.join("sessions/2026/10/01");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::create_dir_all(codex.join("thread-writer-locks")).unwrap();
        for (id, cwd) in [("t1", "/c1"), ("t2", "/c2")] {
            let meta = serde_json::json!({"type": "session_meta", "payload": {"id": id, "cwd": cwd}});
            std::fs::write(day.join(format!("rollout-x-{id}.jsonl")), format!("{meta}\n")).unwrap();
            std::fs::write(codex.join(format!("thread-writer-locks/{id}.lock")), "").unwrap();
        }
        let procs = vec![(11, "console:11".to_string(), Harness::Antigravity), (22, "console:22".into(), Harness::Antigravity), (33, "console:33".into(), Harness::Antigravity), (44, "console:44".into(), Harness::Codex), (55, "console:55".into(), Harness::Codex)];
        // pid 44 holds t2's lock; 55 is a helper holding none.
        let holders = |p: &std::path::Path| if p.ends_with("t2.lock") { vec![44] } else { vec![] };
        let mut found: Vec<(i32, String, String)> = discover_from_files(&procs, holders, &codex, &agy).into_iter().map(|s| (s.pid, s.session_id, s.cwd)).collect();
        found.sort();
        assert_eq!(found, vec![
            (11, "11111111-1111-1111-1111-111111111111".into(), "/one".into()),
            (22, "22222222-2222-2222-2222-222222222222".into(), "/two".into()),
            (44, "t2".into(), "/c2".into()),
        ]);
    }

    use super::*;
    use crate::model::{AwaitKind, Harness, State};

    fn tail(working: bool, awaiting: Option<&str>, text: Option<&str>, at: u64) -> ForeignTail {
        ForeignTail { working, awaiting: awaiting.map(str::to_string), last_agent_text: text.map(str::to_string), last_event_ms: at, context_used: Some(1_000), context_window: Some(10_000), cwd: None, session_id: None }
    }

    fn session() -> ForeignSession {
        ForeignSession { harness: Harness::Codex, pid: 9, tty: Some("/dev/ttys002".into()), session_id: "s".into(), cwd: "/Users/x/dev/one".into(), name: "Say Hello".into(), transcript_path: "/t.jsonl".into() }
    }

    #[test]
    fn derives_the_four_states_and_a_prose_ask() {
        let min = 60_000;
        let c = derive(&session(), &tail(true, None, None, 1000), 5000, 30 * min);
        assert_eq!(c.state, State::Working);
        assert_eq!(c.harness, Harness::Codex);
        assert_eq!(c.pid, 9);
        assert!(!c.has_inbox, "no inbox socket: replies are typed into the tty instead");
        let a = derive(&session(), &tail(true, Some("Approve: rm -rf x"), None, 1000), 5000, 30 * min);
        assert_eq!(a.state, State::Awaiting);
        assert_eq!(a.awaiting.as_ref().unwrap().kind, AwaitKind::Permission);
        assert_eq!(a.awaiting.as_ref().unwrap().detail, "Approve: rm -rf x");
        let q = derive(&session(), &tail(false, None, Some("Should I push?"), 1000), 5000, 30 * min);
        assert_eq!(q.state, State::Awaiting);
        assert_eq!(q.awaiting.as_ref().unwrap().kind, AwaitKind::Text);
        let done = derive(&session(), &tail(false, None, Some("Done."), 1000), 5000, 30 * min);
        assert_eq!(done.state, State::Completed);
        assert_eq!(done.snippet, "Done.");
        assert_eq!(done.context.as_ref().unwrap().percent, 10);
        let idle = derive(&session(), &tail(false, None, Some("Done."), 1000), 1000 + 31 * min, 30 * min);
        assert_eq!(idle.state, State::Idle);
    }

    #[test]
    fn discover_maps_processes_to_sessions_with_names_and_transcripts() {
        let t = tempfile::tempdir().unwrap();
        let codex = t.path().join("codex");
        let agy = t.path().join("agy");
        let day = codex.join("sessions/2026/09/29");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::create_dir_all(codex.join("thread-writer-locks")).unwrap();
        std::fs::write(day.join("rollout-2026-09-29T08-00-00-aaa.jsonl"), "{\"timestamp\":\"2026-09-29T08:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"aaa\",\"cwd\":\"/Users/x/dev/one\"}}\n").unwrap();
        std::fs::write(codex.join("thread-writer-locks/aaa.lock"), "").unwrap();
        std::fs::write(codex.join("session_index.jsonl"), "{\"id\":\"aaa\",\"thread_name\":\"Fix CI\"}\n").unwrap();
        std::fs::create_dir_all(agy.join("brain/c1/.system_generated/logs")).unwrap();
        std::fs::write(agy.join("history.jsonl"), "{\"display\":\"Say hi\",\"conversationId\":\"c1\"}\n").unwrap();
        let procs = vec![(1, "/dev/ttys001".to_string(), Harness::Codex), (2, "/dev/ttys002".to_string(), Harness::Antigravity), (3, "/dev/ttys003".to_string(), Harness::Codex)];
        let info = |pid: i32| match pid {
            1 => ProcInfo { cwd: Some("/Users/x/dev/one".into()), paths: vec![] },
            2 => ProcInfo { cwd: Some("/Users/x/dev/two".into()), paths: vec![format!("{}/brain/c1", agy.display())] },
            _ => ProcInfo { cwd: Some("/Users/x/dev/nowhere".into()), paths: vec![] },
        };
        let found = discover(&procs, info, &codex, &agy);
        assert_eq!(found.len(), 2, "a codex process with no live rollout is skipped");
        let c = &found[0];
        assert_eq!((c.harness, c.pid, c.session_id.as_str(), c.name.as_str(), c.cwd.as_str()), (Harness::Codex, 1, "aaa", "Fix CI", "/Users/x/dev/one"));
        assert!(c.transcript_path.ends_with("rollout-2026-09-29T08-00-00-aaa.jsonl"));
        let a = &found[1];
        assert_eq!((a.harness, a.session_id.as_str(), a.name.as_str(), a.cwd.as_str()), (Harness::Antigravity, "c1", "Say hi", "/Users/x/dev/two"));
        assert_eq!(a.transcript_path, agy.join("brain/c1/.system_generated/logs/transcript.jsonl"));
        assert_eq!(a.tty.as_deref(), Some("/dev/ttys002"));
    }

    #[test]
    fn process_listing_finds_codex_and_agy_tuis_only() {
        let ps = concat!(
            "39566 ttys002 codex\n",
            "41445 ??      /Users/x/.codex/packages/app-server-daemon/releases/0.1/bin/codex app-server --listen unix:\n",
            "76468 ttys023 agy\n",
            "76470 ttys024 agy --continue\n",
            "  123 ttys001 /usr/local/bin/codex resume abc\n",
            "  999 ttys005 grep codex\n",
            // Linux's `ps` shows a process with no terminal as `?`.
            "  777 ?        codex\n",
        );
        let found = tui_processes(ps);
        assert_eq!(found, vec![(39566, "/dev/ttys002".to_string(), Harness::Codex), (76468, "/dev/ttys023".to_string(), Harness::Antigravity), (76470, "/dev/ttys024".to_string(), Harness::Antigravity), (123, "/dev/ttys001".to_string(), Harness::Codex)]);
    }

    #[test]
    fn a_process_tree_gives_the_first_tty_above_a_pid() {
        let ps = "95245 93903 ttys010 kiro-cli chat\n95295 95245 ttys010 /Users/x/.local/bin/kiro-cli-chat chat\n95441 95295 ttys010 /Users/x/Library/Application Support/kiro-cli/bun --no-env-file tui.js chat\n95508 95441 ?? /Users/x/.local/bin/kiro-cli-chat acp\n  777     1 ? zsh\n";
        let procs = parse_ps_tree(ps);
        assert_eq!(procs.len(), 5);
        assert_eq!(procs[3], Proc { pid: 95508, ppid: 95441, tty: None, command: "kiro-cli-chat".into() });
        assert_eq!(procs[2].command, "bun", "the command is the program's base name");
        assert_eq!(parse_ps_tree("1 0 ??\n")[0].command, "", "no command column is fine");
        assert_eq!(tty_above(&procs, 95508).as_deref(), Some("/dev/ttys010"), "the agent has no tty; its TUI has");
        assert_eq!(tty_above(&procs, 95441).as_deref(), Some("/dev/ttys010"));
        assert_eq!(tty_above(&procs, 777), None);
        assert_eq!(tty_above(&procs, 1), None, "an unknown pid has no tty");
    }

    #[test]
    fn kiro_sessions_come_from_live_locks_with_a_terminal_above_them() {
        let t = tempfile::tempdir().unwrap();
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/kiro");
        let id = "efcba1da-5c0b-4b39-96f9-9a4740ead331";
        std::fs::copy(fixtures.join("lock.json"), t.path().join(format!("{id}.lock"))).unwrap();
        std::fs::copy(fixtures.join("session.json"), t.path().join(format!("{id}.json"))).unwrap();
        std::fs::copy(fixtures.join("events.jsonl"), t.path().join(format!("{id}.jsonl"))).unwrap();
        // A headless run: alive, but nothing above it has a tty.
        std::fs::write(t.path().join("headless.lock"), r#"{"pid":600}"#).unwrap();
        std::fs::write(t.path().join("headless.json"), r#"{"session_id":"headless","cwd":"/p","updated_at":"2026-10-03T12:00:00Z","title":null}"#).unwrap();
        // A lock whose pid is gone.
        std::fs::write(t.path().join("dead.lock"), r#"{"pid":700}"#).unwrap();
        let procs = parse_ps_tree("95245 93903 ttys010 kiro-cli chat\n95295 95245 ttys010 kiro-cli-chat chat\n95441 95295 ttys010 bun tui.js chat\n95508 95441 ?? kiro-cli-chat acp\n600 599 ?? kiro-cli-chat acp\n599 1 ?? kiro-cli chat\n");
        let found = kiro_sessions(t.path(), &procs, 0);
        assert_eq!(found.len(), 1, "{found:?}");
        let s = &found[0];
        assert_eq!((s.harness, s.pid, s.tty.as_deref(), s.session_id.as_str(), s.cwd.as_str(), s.name.as_str()), (Harness::Kiro, 95441, Some("/dev/ttys010"), id, "/Users/tiagocorreia", "run ls"));
        assert_eq!(s.transcript_path, t.path().join(format!("{id}.jsonl")));
        let tail = tail_for(s);
        assert_eq!(tail.last_agent_text.as_deref(), Some("shell: mkdir -p /tmp/kiro-probe && sleep 90"));
        assert_eq!(turns_for(s, 10).len(), 5);
    }

    #[test]
    fn an_untitled_kiro_session_is_named_after_its_tui_pid() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("n1.lock"), r#"{"pid":50}"#).unwrap();
        std::fs::write(t.path().join("n1.json"), r#"{"session_id":"n1","cwd":"/p","updated_at":"2026-10-03T12:00:00Z","title":null}"#).unwrap();
        let procs = parse_ps_tree("40 1 ttys001 bun tui.js chat\n50 40 ?? kiro-cli-chat acp\n");
        let found = kiro_sessions(t.path(), &procs, 0);
        assert_eq!(found[0].name, "kiro-40");
        assert_eq!(found[0].pid, 40);
    }

    #[test]
    fn a_stale_lock_whose_pid_now_belongs_to_another_program_is_not_a_session() {
        // Kiro leaves its lock behind after a crash; the pid is reused within a day.
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("s1.lock"), r#"{"pid":50}"#).unwrap();
        std::fs::write(t.path().join("s1.json"), r#"{"session_id":"s1","cwd":"/p","updated_at":"2026-10-03T12:00:00Z","title":"old"}"#).unwrap();
        let procs = parse_ps_tree("40 1 ttys001 zsh\n50 40 ?? sleep 90\n");
        assert!(kiro_sessions(t.path(), &procs, 0).is_empty(), "a sleep is not a Kiro agent");
        let procs = parse_ps_tree("40 1 ttys001 zsh\n50 40 ?? /Applications/Kiro CLI.app/Contents/MacOS/kiro-cli-chat acp\n");
        assert_eq!(kiro_sessions(t.path(), &procs, 0).len(), 1);
    }

    #[test]
    fn a_lock_under_mayas_own_process_is_not_a_session() {
        // Maya's one-shot runs (the brain) inherit a tty when Maya itself runs from a terminal.
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("s1.lock"), r#"{"pid":50}"#).unwrap();
        std::fs::write(t.path().join("s1.json"), r#"{"session_id":"s1","cwd":"/p","updated_at":"2026-10-03T12:00:00Z","title":null}"#).unwrap();
        let procs = parse_ps_tree("30 1 ttys001 maya\n40 30 ttys001 kiro-cli chat\n50 40 ttys001 kiro-cli-chat acp\n");
        assert!(kiro_sessions(t.path(), &procs, 30).is_empty(), "under Maya (pid 30)");
        assert_eq!(kiro_sessions(t.path(), &procs, 999).len(), 1, "under someone else");
    }

    #[test]
    fn lsof_output_gives_cwd_and_open_paths() {
        let out = "p76468\nfcwd\nn/Users/x\nf56\nn/Users/x/.gemini/antigravity-cli/brain/448e2da1\nf58\nn/Users/x/.gemini/antigravity-cli/brain/448e2da1/.user_uploaded\n";
        let info = parse_lsof(out);
        assert_eq!(info.cwd.as_deref(), Some("/Users/x"));
        assert_eq!(info.paths.len(), 2);
    }
}
