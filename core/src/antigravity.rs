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
