//! Kiro CLI sessions: `~/.kiro/sessions/cli/<id>.lock` names the agent
//! process of a live session, `<id>.json` carries its folder, title and
//! context usage, `<id>.jsonl` is the event log, and Kiro's run folder keeps
//! one `turn-markers/<tui pid>-<ms>.json` for each turn in progress.

use crate::foreign::ForeignTail;
use crate::transcript::{Turn, TurnKind};
use crate::usage::{Cost, CostUnit, SessionUsage, TokenCounts};
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
    /// Credits spent across every turn, in millionths.
    pub credits_micros: u64,
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
        credits_micros: (credits(&v) * 1e6).round() as u64,
    })
}

/// The credits of every `metering_usage` entry of every turn.
fn credits(v: &Value) -> f64 {
    v["session_state"]["conversation_metadata"]["user_turn_metadatas"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|t| t["metering_usage"].as_array().into_iter().flatten())
        .filter(|m| m["unit"] == "credit")
        .filter_map(|m| m["value"].as_f64())
        .sum()
}

/// The credits the session has spent, as its cost; Kiro bills no tokens.
pub fn apply_credits(tail: &mut ForeignTail, meta_json: &str) {
    let micros = session_meta(meta_json).map_or(0, |m| m.credits_micros);
    if micros > 0 {
        tail.usage = Some(SessionUsage {
            tokens: TokenCounts::default(),
            cost: Some(Cost { micros, unit: CostUnit::Credits, estimated: false, models: vec![] }),
        });
    }
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
                    out.push(Turn::new(TurnKind::User, s.trim().to_string()));
                }
            }
            "AssistantMessage" => {
                for b in blocks(&v) {
                    match b["kind"].as_str() {
                        Some("text") => {
                            if let Some(s) = b["data"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
                                out.push(Turn::new(TurnKind::Assistant, s.to_string()));
                            }
                        }
                        Some("toolUse") => out.push(Turn::new(TurnKind::Tool, tool_line(&b["data"]))),
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

/// How Kiro is asked for its usage card: `_kiro.dev/commands/execute` with
/// the `usage` command and no arguments (Kiro's commands are an adjacently
/// tagged `{"command", "args"}` pair). Unofficial: found by probing
/// `kiro-cli acp` 2.34 (`examples/kiro_usage_probe.rs`).
pub fn usage_request(id: u64, session_id: &str) -> String {
    crate::brain::jsonrpc::Envelope::Versioned.request(id, "_kiro.dev/commands/execute", serde_json::json!({"sessionId": session_id, "command": {"command": "usage", "args": {}}}))
}

/// The `result` of the answer to `usage_request` (the card's `message`
/// text and its structured `data`), else why there is none.
pub fn result_of(answer: &Value) -> Result<Value, String> {
    if let Some(e) = answer.get("error") {
        return Err(e["message"].as_str().unwrap_or("error").to_string());
    }
    let result = &answer["result"];
    match (result["success"].as_bool(), result["message"].as_str().filter(|m| !m.is_empty())) {
        (Some(true), _) => Ok(result.clone()),
        (_, Some(message)) => Err(message.to_string()),
        _ => Err("no usage card".to_string()),
    }
}

/// How long `acp_usage` gives Kiro to end once its stdin is closed.
const PROBE_EXIT_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

/// Spawns `kiro-cli acp`, sends `initialize` (as `brain::acp::initialize`),
/// `session/new` in the home folder with no MCP servers, then
/// `usage_request`, and returns its answer's `result` (see `result_of`). Gives up
/// after `timeout`. Before this returns the process has ended (given
/// `PROBE_EXIT_GRACE` to save and exit, then killed) and the session it
/// opened is forgotten.
pub fn acp_usage(timeout: std::time::Duration) -> Result<Value, String> {
    use std::io::{BufRead, BufReader, Write};
    use std::process::Stdio;
    use std::sync::mpsc::RecvTimeoutError;
    use std::time::Instant;
    let kiro = crate::launch::find_binary(crate::model::Harness::Kiro)?;
    let home = dirs::home_dir().ok_or("no home folder")?;
    let mut child = crate::command(&kiro).arg("acp").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| format!("kiro-cli acp: {e}"))?;
    let (mut stdin, stdout) = (child.stdin.take(), child.stdout.take());
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    if let Some(stdout) = stdout {
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
    }
    let deadline = Instant::now() + timeout;
    let mut session: Option<String> = None;
    let ask = |stdin: &mut Option<std::process::ChildStdin>, line: String| -> Result<(), String> {
        let pipe = stdin.as_mut().ok_or("kiro-cli acp has no stdin")?;
        pipe.write_all(line.as_bytes()).and_then(|_| pipe.flush()).map_err(|e| format!("kiro-cli acp: {e}"))
    };
    let new_session = crate::brain::jsonrpc::Envelope::Versioned.request(2, "session/new", serde_json::json!({"cwd": home.to_string_lossy(), "mcpServers": []}));
    let answer = ask(&mut stdin, crate::brain::acp::initialize(1)).and_then(|_| ask(&mut stdin, new_session)).and_then(|_| loop {
        let line = match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(line) => line,
            Err(RecvTimeoutError::Timeout) => break Err("kiro-cli acp timed out".to_string()),
            Err(RecvTimeoutError::Disconnected) => break Err("kiro-cli acp exited".to_string()),
        };
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        if v.get("method").is_some() {
            continue;
        }
        match v["id"].as_u64() {
            Some(2) => {
                let Some(id) = v["result"]["sessionId"].as_str() else { break Err("session/new failed".to_string()) };
                session = Some(id.to_string());
                ask(&mut stdin, usage_request(3, id))?;
            }
            Some(3) => break result_of(&v),
            _ => {}
        }
    });
    // `kiro-cli` runs the agent as a child of its own, which saves the
    // session as it ends on stdin's close: wait for that (stdout closes once
    // both are gone) before killing what is left and forgetting the session.
    drop(stdin);
    let grace = Instant::now() + PROBE_EXIT_GRACE;
    while rx.recv_timeout(grace.saturating_duration_since(Instant::now())).is_ok() {}
    let _ = child.kill();
    let _ = child.wait();
    if let Some(id) = session {
        forget_probe_session(&home.join(".kiro/sessions/cli"), &id);
    }
    answer
}

/// Removes the files of a session `acp_usage` opened, so the probe's
/// sessions do not pile up in Kiro's list and Maya's resume list. Only a
/// session with no turns (an empty or missing event log) is removed.
pub fn forget_probe_session(sessions_dir: &Path, id: &str) {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return;
    }
    let events = sessions_dir.join(format!("{id}.jsonl"));
    if std::fs::metadata(&events).is_ok_and(|m| m.len() > 0) {
        return;
    }
    let _ = std::fs::remove_file(sessions_dir.join(format!("{id}.json")));
    let _ = std::fs::remove_file(events);
}

/// The usage answer's `result` as the account's limits: from its
/// structured `data` when that has a credit breakdown with a cap, else from
/// the card's `message` text (`parse_usage_card`).
pub fn parse_usage_result(result: &Value, now_ms: u64) -> Option<crate::usage::AccountLimits> {
    let data = &result["data"];
    let breakdowns = data["usageBreakdowns"].as_array().into_iter().flatten();
    let credits = breakdowns.filter(|b| b["hasLimit"].as_bool() != Some(false)).find(|b| b["resourceType"].as_str() == Some("CREDIT")).and_then(|b| Some((b["used"].as_f64()?, b["limit"].as_f64()?)));
    match credits {
        Some((used, cap)) => month(used, cap, data["billingCycleReset"].as_str(), now_ms),
        None => parse_usage_card(result["message"].as_str()?, now_ms),
    }
}

/// The usage card's text (`Estimated Usage | resets on 2026-11-01 | …` then
/// `Credits (0.62 of 50 covered in plan), 1.2%`) as the account's limits.
/// `None` without a used-of-cap figure.
pub fn parse_usage_card(text: &str, now_ms: u64) -> Option<crate::usage::AccountLimits> {
    let (head, _) = text.split_once(" covered in plan)")?;
    let (used, cap) = head.rsplit_once('(')?.1.split_once(" of ")?;
    let reset = text.split_once("resets on ").and_then(|(_, rest)| rest.get(..10));
    month(used.trim().parse().ok()?, cap.trim().parse().ok()?, reset, now_ms)
}

/// One "month" window at the share of the plan's credits used. Kiro names
/// only the reset's date (`2026-11-01`), with no time or zone: it is taken
/// as midnight UTC. `None` with no cap.
fn month(used: f64, cap: f64, reset_date: Option<&str>, now_ms: u64) -> Option<crate::usage::AccountLimits> {
    use crate::usage::{AccountLimits, LimitWindow};
    if cap <= 0.0 || used < 0.0 {
        return None;
    }
    let resets_at = reset_date.and_then(|d| d.get(..10)).and_then(|d| crate::codex::ms_of(&format!("{d}T00:00:00Z"))).map(|ms| ms / 1000);
    let percent = (used / cap * 100.0).round().clamp(0.0, 100.0) as u8;
    Some(AccountLimits { account: None, windows: vec![LimitWindow { label: "month".into(), percent, resets_at }], read_at: now_ms })
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
    fn session_meta_sums_the_credits_of_every_turn() {
        // The fixture's credit values add up to 0.06524060066334993.
        assert_eq!(session_meta(&fixture("session.json")).unwrap().credits_micros, 65_241);
    }

    #[test]
    fn apply_credits_sets_a_credit_cost_without_tokens() {
        let mut tail = parse_events(&fixture("events.jsonl"));
        apply_credits(&mut tail, &fixture("session.json"));
        assert_eq!(tail.usage, Some(SessionUsage { tokens: TokenCounts::default(), cost: Some(Cost { micros: 65_241, unit: CostUnit::Credits, estimated: false, models: vec![] }) }));
        let mut none = parse_events(&fixture("events.jsonl"));
        apply_credits(&mut none, r#"{"session_id":"x","cwd":"/p","session_state":{}}"#);
        assert_eq!(none.usage, None, "no credits spent, no cost");
        apply_credits(&mut none, "nope");
        assert_eq!(none.usage, None);
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

    #[test]
    fn parse_usage_card_reads_used_cap_and_reset() {
        use crate::usage::LimitWindow;
        let limits = parse_usage_card(&fixture("usage-card.txt"), 9).unwrap();
        // 0.62 of 50 credits; 2026-11-01T00:00:00Z is 1793491200.
        assert_eq!(limits.windows, vec![LimitWindow { label: "month".into(), percent: 1, resets_at: Some(1_793_491_200) }]);
        assert_eq!((limits.account, limits.read_at), (None, 9));
        let half = "Estimated Usage | resets on 2026-12-01 | KIRO PRO\nCredits (500.4 of 1000 covered in plan), 50.0%";
        let w = &parse_usage_card(half, 9).unwrap().windows[0];
        assert_eq!((w.percent, w.resets_at), (50, Some(1_796_083_200)));
        let over = "Credits (1200 of 1000 covered in plan), 120%";
        let w = &parse_usage_card(over, 9).unwrap().windows[0];
        assert_eq!((w.percent, w.resets_at), (100, None), "capped, and no date means no reset");
    }

    #[test]
    fn an_unrelated_text_gives_none() {
        assert_eq!(parse_usage_card("Manage your plan at https://app.kiro.dev/account/usage", 9), None);
        assert_eq!(parse_usage_card("", 9), None);
        assert_eq!(parse_usage_card("Credits (5 of 0 covered in plan)", 9), None, "no cap");
    }

    #[test]
    fn the_usage_request_names_the_command_with_empty_args() {
        let v: Value = serde_json::from_str(&usage_request(3, "s1")).unwrap();
        assert_eq!(v["method"], "_kiro.dev/commands/execute");
        assert_eq!(v["params"], serde_json::json!({"sessionId": "s1", "command": {"command": "usage", "args": {}}}));
    }

    #[test]
    fn the_usage_answer_gives_its_result_or_why_not() {
        use serde_json::json;
        let result = json!({"success": true, "message": "the card", "data": {}});
        assert_eq!(result_of(&json!({"id": 3, "result": result})), Ok(result));
        assert_eq!(result_of(&json!({"id": 3, "error": {"code": -32700, "message": "Parse error"}})), Err("Parse error".to_string()));
        assert_eq!(result_of(&json!({"id": 3, "result": {"success": false, "message": "signed out"}})), Err("signed out".to_string()));
        assert_eq!(result_of(&json!({"id": 3, "result": {"success": false}})), Err("no usage card".to_string()));
    }

    fn live_result() -> Value {
        let answer: Value = serde_json::from_str(&fixture("usage-result.json")).unwrap();
        result_of(&answer).unwrap()
    }

    #[test]
    fn the_structured_data_gives_the_same_limits_as_the_card_text() {
        let result = live_result();
        let from_text = parse_usage_card(result["message"].as_str().unwrap(), 9).unwrap();
        assert_eq!(parse_usage_result(&result, 9), Some(from_text.clone()));
        assert_eq!(from_text.windows[0].percent, 1);
        assert_eq!(from_text.windows[0].resets_at, Some(1_793_491_200));
        let mut moved = result.clone();
        moved["data"]["usageBreakdowns"][0]["used"] = 25.0.into();
        moved["data"]["billingCycleReset"] = "2026-12-01".into();
        let w = &parse_usage_result(&moved, 9).unwrap().windows[0];
        assert_eq!((w.percent, w.resets_at), (50, Some(1_796_083_200)), "the data wins over the text");
    }

    #[test]
    fn without_data_the_card_text_still_answers() {
        let mut result = live_result();
        result.as_object_mut().unwrap().remove("data");
        let w = &parse_usage_result(&result, 9).unwrap().windows[0];
        assert_eq!((w.label.as_str(), w.percent, w.resets_at), ("month", 1, Some(1_793_491_200)));
        let mut no_credits = live_result();
        no_credits["data"]["usageBreakdowns"] = serde_json::json!([]);
        assert_eq!(parse_usage_result(&no_credits, 9), parse_usage_result(&result, 9), "data with no credit breakdown falls back too");
        assert_eq!(parse_usage_result(&serde_json::json!({"success": true}), 9), None);
    }

    #[test]
    fn a_probe_session_with_no_turns_is_forgotten() {
        let t = tempfile::tempdir().unwrap();
        for (id, events) in [("empty", ""), ("used", "{}\n")] {
            std::fs::write(t.path().join(format!("{id}.json")), "{}").unwrap();
            std::fs::write(t.path().join(format!("{id}.jsonl")), events).unwrap();
            forget_probe_session(t.path(), id);
        }
        assert!(!t.path().join("empty.json").exists() && !t.path().join("empty.jsonl").exists());
        assert!(t.path().join("used.json").exists() && t.path().join("used.jsonl").exists(), "a session with turns is kept");
        forget_probe_session(t.path(), "../escape");
    }
}
