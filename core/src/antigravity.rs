//! Antigravity CLI (`agy`) sessions: one `agy` TUI per tty, conversation
//! state under `~/.gemini/antigravity-cli/brain/<id>/…/transcript.jsonl`,
//! prompts and renames in `history.jsonl`.

use crate::codex::ms_of;
use crate::foreign::ForeignTail;
use crate::transcript::{Turn, TurnKind};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The prompt inside Antigravity's `<USER_REQUEST>` wrapper, without the
/// metadata blocks appended after it.
pub fn user_request(content: &str) -> String {
    let inner = match (content.find("<USER_REQUEST>"), content.find("</USER_REQUEST>")) {
        (Some(a), Some(b)) if b > a => &content[a + "<USER_REQUEST>".len()..b],
        _ => content,
    };
    inner.trim().to_string()
}

/// The state of a conversation from its transcript text.
pub fn parse_tail(text: &str) -> ForeignTail {
    let mut t = ForeignTail::default();
    let mut last_status_done = true;
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if let Some(ms) = v["created_at"].as_str().and_then(ms_of) {
            t.last_event_ms = t.last_event_ms.max(ms);
        }
        last_status_done = v["status"].as_str().map_or(true, |s| s == "DONE" || s == "CANCELLED" || s == "ERROR");
        if v["source"].as_str() == Some("MODEL") && v["type"].as_str() == Some("PLANNER_RESPONSE") {
            if let Some(c) = v["content"].as_str() {
                if !c.trim().is_empty() {
                    t.last_agent_text = Some(c.trim().to_string());
                }
            }
        }
    }
    t.working = !last_status_done;
    t
}

/// User prompts and model responses, newest last.
pub fn parse_turns(text: &str, max_turns: usize) -> Vec<Turn> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let content = v["content"].as_str().unwrap_or("").trim();
        if content.is_empty() {
            continue;
        }
        match (v["source"].as_str(), v["type"].as_str()) {
            (Some("USER_EXPLICIT"), _) => out.push(Turn { kind: TurnKind::User, text: user_request(content) }),
            (Some("MODEL"), Some("PLANNER_RESPONSE")) => out.push(Turn { kind: TurnKind::Assistant, text: content.to_string() }),
            _ => {}
        }
    }
    if out.len() > max_turns {
        out.drain(..out.len() - max_turns);
    }
    out
}

/// The latest `/rename` for the conversation in `history.jsonl` text, else
/// its first prompt.
pub fn conversation_name(history: &str, id: &str) -> Option<String> {
    let entries: Vec<Value> = history.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter(|v| v["conversationId"].as_str() == Some(id)).collect();
    let renamed = entries
        .iter()
        .rev()
        .filter_map(|v| v["display"].as_str())
        .find_map(|d| d.strip_prefix("/rename ").map(|n| n.trim().to_string()))
        .filter(|n| !n.is_empty());
    if renamed.is_some() {
        return renamed;
    }
    entries.iter().filter_map(|v| v["display"].as_str()).find(|d| !d.starts_with('/')).map(|d| d.trim().to_string())
}

/// The conversation whose `<state_dir>/brain/<id>` folder the process holds open.
pub fn conversation_id(open_paths: &[String], state_dir: &Path) -> Option<String> {
    let prefix = format!("{}/brain/", state_dir.to_string_lossy().trim_end_matches('/'));
    open_paths.iter().find_map(|p| {
        let rest = p.strip_prefix(&prefix)?;
        let id = rest.split('/').next()?;
        (!id.is_empty() && !id.starts_with('.')).then(|| id.to_string())
    })
}

/// What an `agy` process's own log (`<state_dir>/log/cli-<start>.log`)
/// says about it: its pid, its workspace, and the conversation it is in.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProcessLog {
    pub pid: Option<i32>,
    pub workspace: Option<String>,
    /// The last conversation it created or opened.
    pub conversation: Option<String>,
}

fn uuid_at(s: &str) -> Option<String> {
    let id: String = s.chars().take(36).collect();
    let dashes_ok = id.len() == 36 && [8, 13, 18, 23].iter().all(|&i| id.as_bytes()[i] == b'-');
    let hex_ok = id.bytes().enumerate().all(|(i, b)| [8, 13, 18, 23].contains(&i) || b.is_ascii_hexdigit());
    (dashes_ok && hex_ok).then_some(id)
}

pub fn parse_process_log(text: &str) -> ProcessLog {
    const PID: &str = "Starting language server process with pid ";
    const WORKSPACE: &str = "Initializing CLI store manager for workspace ";
    const DIRS: &str = "workspaceDirs=[";
    const OPENED: [&str; 2] = ["Created conversation ", "Starting conversation update stream for "];
    let mut log = ProcessLog::default();
    for line in text.lines() {
        if log.pid.is_none() {
            if let Some(rest) = line.split_once(PID).map(|(_, r)| r) {
                log.pid = rest.split(|c: char| !c.is_ascii_digit()).next().and_then(|d| d.parse().ok());
            }
        }
        if let Some((_, rest)) = line.split_once(WORKSPACE) {
            log.workspace = Some(rest.trim().to_string());
        } else if log.workspace.is_none() {
            // One directory between the brackets; several are ambiguous with spaces.
            if let Some((_, rest)) = line.split_once(DIRS) {
                log.workspace = rest.split_once(']').map(|(d, _)| d.trim().to_string()).filter(|d| !d.is_empty());
            }
        }
        for marker in OPENED {
            if let Some(id) = line.split_once(marker).and_then(|(_, r)| uuid_at(r)) {
                log.conversation = Some(id);
            }
        }
    }
    log
}

/// The log of the `agy` process `pid`: newest logs first, since a process
/// writes only its own.
pub fn process_log(state_dir: &Path, pid: i32) -> Option<ProcessLog> {
    let mut logs: Vec<PathBuf> = std::fs::read_dir(state_dir.join("log"))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("cli-") && n.ends_with(".log")))
        .collect();
    logs.sort();
    logs.reverse();
    logs.into_iter().find_map(|p| {
        let text = crate::transcript::tail_text(&p, 8 << 20).or_else(|| std::fs::read_to_string(&p).ok())?;
        let log = parse_process_log(&text);
        (log.pid == Some(pid)).then_some(log)
    })
}

pub fn transcript_path(state_dir: &Path, id: &str) -> PathBuf {
    state_dir.join("brain").join(id).join(".system_generated/logs/transcript.jsonl")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn fixture() -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/antigravity/transcript.jsonl")).unwrap()
    }

    const LOG: &str = "I1001 08:28:47.532823      66 server.go:1586] Starting language server process with pid 506660
I1001 08:28:47.848211       1 auto_updater.go:334] Spawned background update process with PID 504360
I1001 08:28:47.848723       1 server.go:323] Creating CLI server backend: product=antigravity workspaceDirs=[C:\\Users\\lordg] appDataDir=C:\\Users\\lordg\\.gemini\\antigravity-cli
I1001 08:28:47.858978       1 manager.go:448] Initializing CLI store manager for workspace C:\\Users\\lordg\\My Projects\\app
I1001 08:06:19.406162    2713 server.go:1248] Created conversation c7cddf6f-4e9e-4bae-9d31-ce941cbfb391
I1001 08:07:19.406162    2713 server.go:1257] Starting conversation update stream for d1e2f3a4-0000-4bae-9d31-ce941cbfb391
I1001 08:08:19.406162    2713 server.go:3175] GetConversationDetail: found conversation c7cddf6f-4e9e-4bae-9d31-ce941cbfb391 (active=true)
";

    #[test]
    fn a_process_log_names_its_pid_workspace_and_latest_conversation() {
        let log = parse_process_log(LOG);
        assert_eq!(log.pid, Some(506660), "the language server's pid, not the updater's");
        assert_eq!(log.workspace.as_deref(), Some("C:\\Users\\lordg\\My Projects\\app"), "the full path, spaces and all");
        assert_eq!(log.conversation.as_deref(), Some("d1e2f3a4-0000-4bae-9d31-ce941cbfb391"), "the last one created or opened, not merely looked up");
    }

    #[test]
    fn a_fresh_process_has_no_conversation_and_falls_back_to_workspace_dirs() {
        let log = parse_process_log("x] Starting language server process with pid 7\ny] Creating CLI server backend: workspaceDirs=[/Users/x/dev/eye] appDataDir=/a\n");
        assert_eq!(log, ProcessLog { pid: Some(7), workspace: Some("/Users/x/dev/eye".into()), conversation: None });
        assert_eq!(parse_process_log("Created conversation not-a-uuid"), ProcessLog::default());
    }

    #[test]
    fn the_process_log_is_found_by_pid() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("log")).unwrap();
        std::fs::write(dir.path().join("log/cli-20261001_074928.log"), "] Starting language server process with pid 504176\n] Created conversation c7cddf6f-4e9e-4bae-9d31-ce941cbfb391\n").unwrap();
        std::fs::write(dir.path().join("log/cli-20261001_082847.log"), LOG).unwrap();
        assert_eq!(process_log(dir.path(), 504176).unwrap().conversation.as_deref(), Some("c7cddf6f-4e9e-4bae-9d31-ce941cbfb391"));
        assert_eq!(process_log(dir.path(), 506660).unwrap().pid, Some(506660));
        assert_eq!(process_log(dir.path(), 1), None);
    }

    #[test]
    fn parses_a_finished_conversation() {
        let t = parse_tail(&fixture());
        assert!(!t.working);
        assert_eq!(t.awaiting, None);
        assert_eq!(t.last_agent_text.as_deref(), Some("hello"));
        assert!(t.last_event_ms > 1_790_000_000_000);
        assert_eq!(t.context_used, None);
    }

    #[test]
    fn a_step_that_is_not_done_means_working() {
        let text = concat!(
            r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-09-29T08:43:54Z","content":"<USER_REQUEST>\nDo it\n</USER_REQUEST>"}"#, "\n",
            r#"{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"RUNNING","created_at":"2026-09-29T08:43:55Z","content":"On it"}"#, "\n",
        );
        let t = parse_tail(text);
        assert!(t.working);
    }

    #[test]
    fn turns_strip_the_request_wrapper_and_metadata() {
        let turns = parse_turns(&fixture(), 30);
        let got: Vec<(String, String)> = turns.iter().map(|t| (format!("{:?}", t.kind).to_lowercase(), t.text.clone())).collect();
        assert_eq!(got, vec![("user".to_string(), "Say \"hello\"".to_string()), ("assistant".to_string(), "hello".to_string())]);
        assert_eq!(user_request("<USER_REQUEST>\n  hi there \n</USER_REQUEST>\n<ADDITIONAL_METADATA>x</ADDITIONAL_METADATA>"), "hi there");
        assert_eq!(user_request("plain"), "plain");
    }

    #[test]
    fn name_is_the_latest_rename_else_the_first_prompt() {
        let history = concat!(
            r#"{"display":"/rename Testing AGY","timestamp":1,"workspace":"/Users/x","type":"slash_command"}"#, "\n",
            r#"{"display":"Say \"hello\"","timestamp":2,"workspace":"/Users/x","conversationId":"c1"}"#, "\n",
            r#"{"display":"/rename Testing AGY","timestamp":3,"workspace":"/Users/x","conversationId":"c1","type":"slash_command"}"#, "\n",
            r#"{"display":"Other prompt","timestamp":4,"workspace":"/Users/x","conversationId":"c2"}"#, "\n",
        );
        assert_eq!(conversation_name(history, "c1").as_deref(), Some("Testing AGY"));
        assert_eq!(conversation_name(history, "c2").as_deref(), Some("Other prompt"));
        assert_eq!(conversation_name(history, "c3"), None);
    }

    #[test]
    fn conversation_id_comes_from_the_open_brain_folder() {
        let paths = vec!["/Users/x/.gemini/antigravity-cli/brain".to_string(), "/Users/x/.gemini/antigravity-cli/brain/448e2da1-4835-4ef7-af63-8918dc91bff3".to_string(), "/Users/x/.gemini/antigravity-cli/brain/448e2da1-4835-4ef7-af63-8918dc91bff3/scratch".to_string()];
        let dir = Path::new("/Users/x/.gemini/antigravity-cli");
        assert_eq!(conversation_id(&paths, dir).as_deref(), Some("448e2da1-4835-4ef7-af63-8918dc91bff3"));
        assert_eq!(conversation_id(&["/Users/x/other".to_string()], dir), None);
        assert_eq!(transcript_path(Path::new("/Users/x/.gemini/antigravity-cli"), "abc"), Path::new("/Users/x/.gemini/antigravity-cli/brain/abc/.system_generated/logs/transcript.jsonl"));
    }
}
