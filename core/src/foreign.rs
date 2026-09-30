//! Sessions from harnesses other than Claude Code. They have no registry
//! and no inbox, so they are found from their processes and their own
//! session files, and replies are typed into their tty.

use crate::context;
use crate::model::{AwaitKind, Awaiting, Card, Harness, State};
use crate::state::{asking_line, truncate, SNIPPET_CHARS};
use std::path::PathBuf;
use std::process::{Command, Stdio};

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
        stale: false,
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

pub fn list_tui_processes() -> Vec<(i32, String, Harness)> {
    Command::new("ps")
        .args(["-axo", "pid=,tty=,command="])
        .stdin(Stdio::null())
        .output()
        .map(|o| tui_processes(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
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
            Harness::ClaudeCode | Harness::Grok => {}
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
        Harness::ClaudeCode => ForeignTail::default(),
    }
}

/// Conversation turns for a foreign session, read from its files by harness.
pub fn turns_for(s: &ForeignSession, max_turns: usize) -> Vec<crate::transcript::Turn> {
    let big = crate::transcript::TURNS_TAIL_BYTES;
    match s.harness {
        Harness::Codex => crate::codex::parse_turns(&tail_of(&s.transcript_path, big), max_turns),
        Harness::Antigravity => crate::antigravity::parse_turns(&tail_of(&s.transcript_path, big), max_turns),
        Harness::Grok => crate::grok::parse_turns(&tail_of(&s.transcript_path.with_file_name("chat_history.jsonl"), big), max_turns),
        Harness::ClaudeCode => vec![],
    }
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

pub fn proc_info(pid: i32) -> ProcInfo {
    Command::new("lsof")
        .args(["-p", &pid.to_string(), "-Fn", "-w"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|o| parse_lsof(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
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
    fn lsof_output_gives_cwd_and_open_paths() {
        let out = "p76468\nfcwd\nn/Users/x\nf56\nn/Users/x/.gemini/antigravity-cli/brain/448e2da1\nf58\nn/Users/x/.gemini/antigravity-cli/brain/448e2da1/.user_uploaded\n";
        let info = parse_lsof(out);
        assert_eq!(info.cwd.as_deref(), Some("/Users/x"));
        assert_eq!(info.paths.len(), 2);
    }
}
