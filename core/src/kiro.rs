//! Kiro CLI sessions: `~/.kiro/sessions/cli/<id>.lock` names the agent
//! process of a live session, `<id>.json` carries its folder, title and
//! context usage, `<id>.jsonl` is the event log, and Kiro's run folder keeps
//! one `turn-markers/<tui pid>-<ms>.json` for each turn in progress.

use crate::foreign::ForeignTail;
use crate::transcript::{Turn, TurnKind};
use serde_json::Value;
use std::path::Path;

/// A live session's lock: the pid of its `kiro-cli-chat acp` agent process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lock {
    pub session_id: String,
    pub pid: i32,
}

/// Every `<id>.lock` in the sessions folder with a pid in it.
pub fn locks(sessions_dir: &Path) -> Vec<Lock> {
    let Ok(entries) = std::fs::read_dir(sessions_dir) else { return vec![] };
    let mut out: Vec<Lock> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "lock"))
        .filter_map(|e| {
            let id = e.path().file_stem()?.to_str()?.to_string();
            let v: Value = serde_json::from_str(&std::fs::read_to_string(e.path()).ok()?).ok()?;
            Some(Lock { session_id: id, pid: v["pid"].as_i64()? as i32 })
        })
        .collect();
    out.sort_by(|a, b| a.session_id.cmp(&b.session_id));
    out
}

/// What `<id>.json` says about a session.
#[derive(Debug, Clone, PartialEq)]
pub struct Meta {
    pub session_id: String,
    pub cwd: String,
    pub title: Option<String>,
    pub updated_ms: u64,
    pub context_percent: Option<f64>,
    pub context_window: Option<u64>,
}

pub fn session_meta(json: &str) -> Option<Meta> {
    let v: Value = serde_json::from_str(json).ok()?;
    let state = &v["session_state"]["rts_model_state"];
    Some(Meta {
        session_id: v["session_id"].as_str()?.to_string(),
        cwd: v["cwd"].as_str()?.to_string(),
        title: v["title"].as_str().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string),
        updated_ms: v["updated_at"].as_str().and_then(crate::codex::ms_of).unwrap_or(0),
        context_percent: state["context_usage_percentage"].as_f64(),
        context_window: state["model_info"]["context_window_tokens"].as_u64(),
    })
}

/// A turn in progress: Kiro writes one per turn, named `<tui pid>-<start ms>.json`,
/// and refreshes `last_alive_at_ms` while the turn runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marker {
    pub pid: i32,
    pub started_ms: u64,
    pub alive_ms: u64,
}

pub fn markers(run_dir: &Path) -> Vec<Marker> {
    let Ok(entries) = std::fs::read_dir(run_dir.join("turn-markers")) else { return vec![] };
    entries
        .flatten()
        .filter_map(|e| {
            let v: Value = serde_json::from_str(&std::fs::read_to_string(e.path()).ok()?).ok()?;
            Some(Marker { pid: v["pid"].as_i64()? as i32, started_ms: v["turn_started_at_ms"].as_u64()?, alive_ms: v["last_alive_at_ms"].as_u64().unwrap_or(0) })
        })
        .collect()
}

/// A marker whose heartbeat is older than this belongs to a TUI that died
/// mid-turn, or to a turn Kiro forgot to clean up.
pub const MARKER_STALE_MS: u64 = 120_000;

pub fn working_marker(markers: &[Marker], tui_pid: i32, now_ms: u64) -> Option<Marker> {
    markers.iter().copied().filter(|m| m.pid == tui_pid && now_ms.saturating_sub(m.alive_ms) <= MARKER_STALE_MS).max_by_key(|m| m.started_ms)
}

/// `shell: <command>` for the shell tool, else `<tool>: <purpose>`, else the tool name.
fn tool_line(block: &Value) -> String {
    let name = block["name"].as_str().unwrap_or("tool");
    let input = &block["input"];
    let arg = if name == "shell" { input["command"].as_str() } else { None }.or_else(|| input["__tool_use_purpose"].as_str()).or_else(|| input["path"].as_str()).or_else(|| input["command"].as_str());
    match arg.map(str::trim).filter(|a| !a.is_empty()) {
        Some(a) => format!("{name}: {}", crate::state::truncate(a, 120)),
        None => name.to_string(),
    }
}

fn blocks(v: &Value) -> impl Iterator<Item = &Value> {
    v["data"]["content"].as_array().into_iter().flatten()
}

/// The turn state the event log alone can give: the last assistant text,
/// or the tool call still without a result, and the last prompt's time.
/// Working comes from the marker (`apply_live`), never from here.
pub fn parse_events(text: &str) -> ForeignTail {
    let mut t = ForeignTail::default();
    let mut last_text: Option<String> = None;
    let mut pending_tool: Option<String> = None;
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        match v["kind"].as_str().unwrap_or("") {
            "Prompt" => {
                pending_tool = None;
                if let Some(s) = v["data"]["meta"]["timestamp"].as_u64() {
                    t.last_event_ms = t.last_event_ms.max(s * 1000);
                }
            }
            "AssistantMessage" => {
                for b in blocks(&v) {
                    match b["kind"].as_str() {
                        Some("text") => {
                            if let Some(s) = b["data"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
                                last_text = Some(s.to_string());
                            }
                        }
                        Some("toolUse") => pending_tool = Some(tool_line(&b["data"])),
                        _ => {}
                    }
                }
            }
            "ToolResults" => pending_tool = None,
            _ => {}
        }
    }
    t.last_agent_text = pending_tool.or(last_text);
    t
}

/// Prompts, assistant text and tool calls, newest last.
pub fn parse_turns(text: &str, max_turns: usize) -> Vec<Turn> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        match v["kind"].as_str().unwrap_or("") {
            "Prompt" => {
                let s: String = blocks(&v).filter(|b| b["kind"] == "text").filter_map(|b| b["data"].as_str()).collect::<Vec<_>>().join("\n");
                if !s.trim().is_empty() {
                    out.push(Turn { kind: TurnKind::User, text: s.trim().to_string() });
                }
            }
            "AssistantMessage" => {
                for b in blocks(&v) {
                    match b["kind"].as_str() {
                        Some("text") => {
                            if let Some(s) = b["data"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
                                out.push(Turn { kind: TurnKind::Assistant, text: s.to_string() });
                            }
                        }
                        Some("toolUse") => out.push(Turn { kind: TurnKind::Tool, text: tool_line(&b["data"]) }),
                        _ => {}
                    }
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

/// The first line of the first prompt, as a title.
pub fn first_prompt(text: &str) -> Option<String> {
    parse_turns(text, usize::MAX).into_iter().find(|t| t.kind == TurnKind::User).and_then(|t| t.text.lines().map(str::trim).find(|l| !l.is_empty()).map(|l| crate::state::truncate(l, 120)))
}

fn mtime_ms(path: &Path) -> Option<u64> {
    Some(std::fs::metadata(path).ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as u64)
}

/// What the event log cannot say: Working from the TUI's turn marker,
/// context usage from the session file, and the time of the last write.
pub fn apply_live(tail: &mut ForeignTail, run_dir: &Path, tui_pid: i32, events_path: &Path, now_ms: u64) {
    if let Some(m) = working_marker(&markers(run_dir), tui_pid, now_ms) {
        tail.working = true;
        tail.awaiting = None;
        tail.last_event_ms = tail.last_event_ms.max(m.started_ms);
    }
    if let Some(meta) = std::fs::read_to_string(events_path.with_extension("json")).ok().and_then(|s| session_meta(&s)) {
        tail.last_event_ms = tail.last_event_ms.max(meta.updated_ms);
        if let (Some(p), Some(w)) = (meta.context_percent, meta.context_window) {
            tail.context_window = Some(w);
            tail.context_used = Some((p / 100.0 * w as f64).round() as u64);
        }
    }
    if let Some(ms) = mtime_ms(events_path) {
        tail.last_event_ms = tail.last_event_ms.max(ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/kiro").join(name)).unwrap()
    }

    #[test]
    fn locks_name_the_agent_pid_of_each_live_session() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("efcba1da-5c0b-4b39-96f9-9a4740ead331.lock"), fixture("lock.json")).unwrap();
        std::fs::write(t.path().join("bad.lock"), "not json").unwrap();
        std::fs::write(t.path().join("other.json"), "{}").unwrap();
        let l = locks(t.path());
        assert_eq!(l.len(), 1);
        assert_eq!((l[0].session_id.as_str(), l[0].pid), ("efcba1da-5c0b-4b39-96f9-9a4740ead331", 95508));
        assert!(locks(Path::new("/nonexistent")).is_empty());
    }

    #[test]
    fn session_meta_reads_folder_title_time_and_context() {
        let m = session_meta(&fixture("session.json")).unwrap();
        assert_eq!(m.session_id, "efcba1da-5c0b-4b39-96f9-9a4740ead331");
        assert_eq!(m.cwd, "/Users/tiagocorreia");
        assert_eq!(m.title.as_deref(), Some("run ls"));
        assert_eq!(m.updated_ms, 1_791_030_054_892);
        assert_eq!(m.context_window, Some(200_000));
        assert!((m.context_percent.unwrap() - 1.2537).abs() < 0.001);
        let fresh = session_meta(r#"{"session_id":"x","cwd":"/p","created_at":"2026-10-03T12:13:30.596448Z","updated_at":"2026-10-03T12:13:30.596448Z","title":null,"session_state":{"rts_model_state":{"model_info":null,"context_usage_percentage":null}}}"#).unwrap();
        assert_eq!(fresh.title, None);
        assert_eq!(fresh.context_percent, None);
        assert!(session_meta("nope").is_none());
    }

    #[test]
    fn markers_mark_a_turn_in_progress_for_their_tui_pid() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(t.path().join("turn-markers")).unwrap();
        std::fs::write(t.path().join("turn-markers/95441-1791030218653.json"), fixture("marker.json")).unwrap();
        let m = markers(t.path());
        assert_eq!(m.len(), 1);
        assert_eq!((m[0].pid, m[0].started_ms), (95441, 1_791_030_218_653));
        assert!(m[0].alive_ms > m[0].started_ms, "the heartbeat moved on after the turn started");
        let now = m[0].alive_ms + 1_000;
        assert_eq!(working_marker(&m, 95441, now).map(|x| x.started_ms), Some(1_791_030_218_653));
        assert!(working_marker(&m, 95442, now).is_none(), "another TUI's turn");
        assert!(working_marker(&m, 95441, m[0].alive_ms + MARKER_STALE_MS + 1).is_none(), "a heartbeat that stopped is not a turn");
        assert!(markers(Path::new("/nonexistent")).is_empty());
    }

    #[test]
    fn events_give_the_last_text_the_pending_tool_call_and_the_prompt_time() {
        let text = fixture("events.jsonl");
        let t = parse_events(&text);
        assert!(!t.working, "the event log alone never says working: the marker does");
        assert_eq!(t.awaiting, None);
        assert_eq!(t.last_agent_text.as_deref(), Some("shell: mkdir -p /tmp/kiro-probe && sleep 90"), "a tool call with no result yet is the snippet");
        assert_eq!(t.last_event_ms, 1_791_030_218_000, "the last prompt's timestamp");
        // The first four events are a finished turn: the final text is the snippet.
        let finished: String = text.lines().take(4).map(|l| format!("{l}\n")).collect();
        let f = parse_events(&finished);
        assert!(f.last_agent_text.unwrap().starts_with("```\nApplications"));
        assert_eq!(f.last_event_ms, 1_791_030_044_000);
        let empty = parse_events("");
        assert_eq!(empty.last_agent_text, None);
        assert_eq!(empty.last_event_ms, 0);
    }

    #[test]
    fn turns_are_prompts_text_and_tool_calls() {
        let turns = parse_turns(&fixture("events.jsonl"), 30);
        let got: Vec<(String, String)> = turns.iter().map(|t| (format!("{:?}", t.kind).to_lowercase(), t.text.chars().take(40).collect())).collect();
        assert_eq!(got[0], ("user".to_string(), "run ls".to_string()));
        assert_eq!(got[1], ("tool".to_string(), "shell: ls".to_string()));
        assert!(got[2].0 == "assistant" && got[2].1.starts_with("```\nApplications"));
        assert_eq!(got[3], ("user".to_string(), "run this: mkdir -p /tmp/kiro-probe && sl".to_string()));
        assert_eq!(got[4].0, "tool");
        assert_eq!(turns.len(), 5, "empty assistant text is not a turn");
        assert_eq!(parse_turns(&fixture("events.jsonl"), 2).len(), 2, "the newest turns are kept");
        assert_eq!(first_prompt(&fixture("events.jsonl")).as_deref(), Some("run ls"));
        assert_eq!(first_prompt(""), None);
    }

    #[test]
    fn apply_live_adds_the_marker_the_context_and_the_file_time() {
        let t = tempfile::tempdir().unwrap();
        let run = t.path().join("run");
        std::fs::create_dir_all(run.join("turn-markers")).unwrap();
        std::fs::write(run.join("turn-markers/95441-1791030218653.json"), fixture("marker.json")).unwrap();
        let events = t.path().join("s.jsonl");
        std::fs::write(&events, fixture("events.jsonl")).unwrap();
        std::fs::write(t.path().join("s.json"), fixture("session.json")).unwrap();
        let alive = markers(&run)[0].alive_ms;
        let mut tail = parse_events(&fixture("events.jsonl"));
        apply_live(&mut tail, &run, 95441, &events, alive + 500);
        assert!(tail.working);
        assert_eq!(tail.context_window, Some(200_000));
        assert_eq!(tail.context_used, Some(2_507), "1.2537% of 200000, rounded");
        assert!(tail.last_event_ms >= 1_791_030_218_653, "at least the turn's start: {}", tail.last_event_ms);
        let mut idle = parse_events(&fixture("events.jsonl"));
        apply_live(&mut idle, &run, 95441, &events, alive + MARKER_STALE_MS + 1);
        assert!(!idle.working, "a stale marker is ignored");
        let mut none = parse_events("");
        apply_live(&mut none, Path::new("/nonexistent"), 1, Path::new("/nonexistent/x.jsonl"), 5);
        assert!(!none.working && none.context_window.is_none());
    }
}
