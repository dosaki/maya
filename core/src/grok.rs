//! Grok Build sessions: `~/.grok/active_sessions.json` lists live sessions
//! with pid and working directory; each session keeps `events.jsonl`,
//! `chat_history.jsonl` and `summary.json` under
//! `~/.grok/sessions/<percent-encoded cwd>/<id>/`.

use crate::codex::ms_of;
use crate::foreign::ForeignTail;
use crate::transcript::{Turn, TurnKind};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// `(session id, pid, cwd)` for every entry with all three fields.
pub fn registry(text: &str) -> Vec<(String, i32, String)> {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|e| Some((e["session_id"].as_str()?.to_string(), e["pid"].as_i64()? as i32, e["cwd"].as_str()?.to_string())))
        .collect()
}

/// Grok names session folders after the percent-encoded working directory.
pub fn encode_cwd(cwd: &str) -> String {
    let mut out = String::new();
    for b in cwd.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn session_dir(grok_dir: &Path, cwd: &str, id: &str) -> PathBuf {
    grok_dir.join("sessions").join(encode_cwd(cwd)).join(id)
}

/// The session's title from `summary.json`: the generated or manual title,
/// else the one-line summary.
pub fn title(summary_json: &str) -> Option<String> {
    let v: Value = serde_json::from_str(summary_json).ok()?;
    ["generated_title", "session_summary"]
        .iter()
        .find_map(|k| v[k].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string))
}

/// The turn state from `events.jsonl`: working between `turn_started` and
/// `turn_ended`, awaiting when a permission or question event follows.
pub fn parse_events(text: &str) -> ForeignTail {
    let mut t = ForeignTail::default();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if let Some(ms) = v["ts"].as_str().and_then(ms_of) {
            t.last_event_ms = t.last_event_ms.max(ms);
        }
        let kind = v["type"].as_str().unwrap_or("");
        let phase = v["phase"].as_str().unwrap_or("");
        let asks = |s: &str| s.contains("permission") || s.contains("approval") || s.contains("user_question") || s.contains("ask_user");
        match kind {
            "turn_started" => {
                t.working = true;
                t.awaiting = None;
            }
            "turn_ended" | "turn_cancelled" | "turn_failed" => {
                t.working = false;
                t.awaiting = None;
            }
            _ if asks(kind) => {
                t.awaiting = Some(v["question"].as_str().or_else(|| v["message"].as_str()).map(|q| crate::state::truncate(q.trim(), 120)).unwrap_or_else(|| "Waiting for your approval".to_string()));
            }
            "phase_changed" if asks(phase) => t.awaiting = Some("Waiting for your approval".to_string()),
            "phase_changed" => t.awaiting = None,
            _ => {}
        }
    }
    t
}

/// The prompt inside Grok's `<user_query>` wrapper.
pub fn user_query(text: &str) -> String {
    let inner = match (text.find("<user_query>"), text.find("</user_query>")) {
        (Some(a), Some(b)) if b > a => &text[a + "<user_query>".len()..b],
        _ => text,
    };
    inner.trim().to_string()
}

fn user_text(entry: &Value) -> Option<String> {
    let blocks = entry["content"].as_array()?;
    let text = blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).find(|s| s.contains("<user_query>"))?;
    let q = user_query(text);
    (!q.is_empty()).then_some(q)
}

/// User prompts and assistant replies from `chat_history.jsonl`, newest last.
pub fn parse_turns(text: &str, max_turns: usize) -> Vec<Turn> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        match v["type"].as_str() {
            Some("user") => {
                if let Some(q) = user_text(&v) {
                    out.push(Turn { kind: TurnKind::User, text: q });
                }
            }
            Some("assistant") => {
                if let Some(s) = v["content"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
                    out.push(Turn { kind: TurnKind::Assistant, text: s.to_string() });
                }
            }
            _ => {}
        }
    }
    if out.len() > max_turns {
        out.drain(..out.len() - max_turns);
    }
    out
}

/// The last assistant reply in `chat_history.jsonl` text.
pub fn last_assistant(text: &str) -> Option<String> {
    parse_turns(text, 200).into_iter().rev().find(|t| t.kind == TurnKind::Assistant).map(|t| t.text)
}

/// The card state for a session folder: events for the turn, chat for the text.
pub fn tail_for_dir(dir: &Path) -> ForeignTail {
    let events = crate::transcript::tail_text(&dir.join("events.jsonl"), crate::transcript::TAIL_BYTES).unwrap_or_default();
    let mut t = parse_events(&events);
    let chat = crate::transcript::tail_text(&dir.join("chat_history.jsonl"), crate::transcript::TAIL_BYTES).unwrap_or_default();
    t.last_agent_text = last_assistant(&chat);
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/grok").join(name)).unwrap()
    }

    #[test]
    fn registry_lists_sessions_with_pid_and_cwd() {
        let text = r#"[{"session_id":"01a0ed78-fb28","pid":4150,"cwd":"/Users/tiagocorreia","opened_at":"2026-09-29T14:02:04Z"},{"session_id":"x","cwd":"/y"}]"#;
        let r = registry(text);
        assert_eq!(r, vec![("01a0ed78-fb28".to_string(), 4150, "/Users/tiagocorreia".to_string())]);
        assert!(registry("not json").is_empty());
    }

    #[test]
    fn session_dir_encodes_the_working_directory_like_grok_does() {
        assert_eq!(encode_cwd("/Users/tiagocorreia"), "%2FUsers%2Ftiagocorreia");
        assert_eq!(encode_cwd("/Users/x/my dir"), "%2FUsers%2Fx%2Fmy%20dir");
        assert_eq!(session_dir(Path::new("/h/.grok"), "/Users/x", "abc"), Path::new("/h/.grok/sessions/%2FUsers%2Fx/abc"));
    }

    #[test]
    fn title_prefers_the_generated_title_and_falls_back_to_the_summary() {
        assert_eq!(title(&fixture("summary.json")).as_deref(), Some("Testing Grok Build"));
        assert_eq!(title(r#"{"session_summary":"Simple Greeting"}"#).as_deref(), Some("Simple Greeting"));
        assert_eq!(title(r#"{"generated_title":""}"#), None);
        assert_eq!(title("nope"), None);
    }

    #[test]
    fn events_give_working_awaiting_and_finished_turns() {
        let t = parse_events(&fixture("events.jsonl"));
        assert!(!t.working);
        assert_eq!(t.awaiting, None);
        assert!(t.last_event_ms > 1_790_000_000_000);
        let running = "{\"ts\":\"2026-09-29T14:02:15.012Z\",\"type\":\"turn_started\",\"turn_number\":1}\n{\"ts\":\"2026-09-29T14:02:16.931Z\",\"type\":\"phase_changed\",\"phase\":\"streaming_text\"}\n";
        assert!(parse_events(running).working);
        let asking = format!("{running}{}\n", "{\"ts\":\"2026-09-29T14:02:17.000Z\",\"type\":\"phase_changed\",\"phase\":\"waiting_for_permission\"}");
        let t = parse_events(&asking);
        assert_eq!(t.awaiting.as_deref(), Some("Waiting for your approval"));
        let asked = format!("{running}{}\n", "{\"ts\":\"2026-09-29T14:02:17.000Z\",\"type\":\"user_question\",\"question\":\"Which one?\"}");
        assert_eq!(parse_events(&asked).awaiting.as_deref(), Some("Which one?"));
    }

    #[test]
    fn chat_history_yields_clean_user_and_assistant_turns() {
        let text = fixture("chat_history.jsonl");
        let turns = parse_turns(&text, 30);
        let got: Vec<(String, String)> = turns.iter().map(|t| (format!("{:?}", t.kind).to_lowercase(), t.text.clone())).collect();
        assert_eq!(got, vec![("user".to_string(), "Say \"Hi".to_string()), ("assistant".to_string(), "Hi".to_string())]);
        assert_eq!(last_assistant(&text).as_deref(), Some("Hi"));
        assert_eq!(user_query("<user_query>\n  go \n</user_query>"), "go");
    }
}
