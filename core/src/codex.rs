//! Codex CLI sessions: a `codex` TUI per tty, rollout files under
//! `~/.codex/sessions/YYYY/MM/DD/rollout-<stamp>-<id>.jsonl`, a writer lock
//! per live thread, and thread names in `session_index.jsonl`.

use crate::foreign::ForeignTail;
use crate::transcript::{Turn, TurnKind};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub fn ms_of(ts: &str) -> Option<u64> {
    // "2026-09-29T08:47:58.953Z" → epoch millis, without a date crate.
    let (date, time) = ts.split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let time = time.trim_end_matches('Z');
    let (hms, frac) = time.split_once('.').unwrap_or((time, "0"));
    let mut t = hms.split(':').map(|p| p.parse::<i64>().ok());
    let (h, mi, s) = (t.next()??, t.next()??, t.next()??);
    let millis: i64 = format!("{:0<3}", frac).chars().take(3).collect::<String>().parse().ok()?;
    // Days from civil (Howard Hinnant's algorithm).
    let (y2, m2) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * m2 + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 86_400 + h * 3600 + mi * 60 + s) * 1000 + millis) as u64)
}

fn approval_detail(p: &Value) -> String {
    if let Some(cmd) = p["command"].as_array() {
        let joined = cmd.iter().filter_map(|c| c.as_str()).collect::<Vec<_>>().join(" ");
        return format!("Approve: {}", crate::state::truncate(&joined, 120));
    }
    if let Some(q) = p["question"].as_str().or_else(|| p["message"].as_str()) {
        return crate::state::truncate(q, 120);
    }
    "Waiting for your approval".to_string()
}

/// The state of a rollout from its (tail) text.
pub fn parse_tail(text: &str) -> ForeignTail {
    let mut t = ForeignTail::default();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if let Some(ms) = v["timestamp"].as_str().and_then(ms_of) {
            t.last_event_ms = t.last_event_ms.max(ms);
        }
        let p = &v["payload"];
        match v["type"].as_str() {
            Some("session_meta") => {
                t.cwd = p["cwd"].as_str().map(str::to_string);
                t.session_id = p["id"].as_str().or_else(|| p["session_id"].as_str()).map(str::to_string);
            }
            Some("event_msg") => match p["type"].as_str().unwrap_or("") {
                "task_started" => {
                    t.working = true;
                    t.awaiting = None;
                    if let Some(w) = p["model_context_window"].as_u64() {
                        t.context_window = Some(w);
                    }
                }
                "task_complete" | "turn_aborted" | "error" => {
                    t.working = false;
                    t.awaiting = None;
                    if let Some(m) = p["last_agent_message"].as_str() {
                        if !m.trim().is_empty() {
                            t.last_agent_text = Some(m.trim().to_string());
                        }
                    }
                }
                kind if kind.contains("approval") || kind.contains("user_input") => {
                    t.awaiting = Some(approval_detail(p));
                }
                "item_completed" => {
                    if p["item"]["type"].as_str() == Some("AgentMessage") {
                        if let Some(txt) = p["item"]["content"].as_array().and_then(|c| c.iter().find_map(|b| b["text"].as_str())) {
                            t.last_agent_text = Some(txt.trim().to_string());
                        }
                    }
                }
                _ => {}
            },
            Some("token_usage_record") => {
                let u = &p["usage"];
                let used = u["input_tokens"].as_u64().unwrap_or(0) + u["output_tokens"].as_u64().unwrap_or(0);
                if used > 0 {
                    t.context_used = Some(used);
                }
            }
            _ => {}
        }
    }
    t
}

/// User and assistant messages, newest last, skipping developer prompts and
/// the environment preamble Codex injects as a user message.
pub fn parse_turns(text: &str, max_turns: usize) -> Vec<Turn> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if v["type"].as_str() != Some("response_item") || v["payload"]["type"].as_str() != Some("message") {
            continue;
        }
        let p = &v["payload"];
        let kind = match p["role"].as_str() {
            Some("user") => TurnKind::User,
            Some("assistant") => TurnKind::Assistant,
            _ => continue,
        };
        let text: String = p["content"].as_array().map(|c| c.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default();
        let text = text.trim();
        if text.is_empty() || text.starts_with("<environment_context>") {
            continue;
        }
        out.push(Turn { kind, text: text.to_string() });
    }
    if out.len() > max_turns {
        out.drain(..out.len() - max_turns);
    }
    out
}

/// The latest thread name recorded for `id` in `session_index.jsonl` text.
pub fn thread_name(index: &str, id: &str) -> Option<String> {
    index
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["id"].as_str() == Some(id))
        .last()
        .and_then(|v| v["thread_name"].as_str().map(str::to_string))
        .filter(|n| !n.trim().is_empty())
}

fn rollout_files(codex_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() && depth < 3 {
                walk(&p, depth + 1, out);
            } else if p.extension().map_or(false, |x| x == "jsonl") && p.file_name().and_then(|n| n.to_str()).map_or(false, |n| n.starts_with("rollout-")) {
                out.push(p);
            }
        }
    }
    walk(&codex_dir.join("sessions"), 0, &mut out);
    out
}

/// The first line of `path`, however long: session_meta can run to tens of KB.
fn first_line(path: &Path) -> Option<String> {
    use std::io::BufRead;
    let f = std::fs::File::open(path).ok()?;
    let mut line = String::new();
    std::io::BufReader::new(f).read_line(&mut line).ok()?;
    Some(line)
}

/// The (thread id, rollout path) of the newest live thread whose recorded
/// working directory is `cwd`. Live means a writer lock exists for it.
pub fn live_rollout_for(codex_dir: &Path, cwd: &str) -> Option<(String, PathBuf)> {
    let locks = codex_dir.join("thread-writer-locks");
    let mut files = rollout_files(codex_dir);
    files.sort();
    files.reverse();
    for path in files {
        let Some(first) = first_line(&path) else { continue };
        let Ok(v) = serde_json::from_str::<Value>(first.trim()) else { continue };
        if v["type"].as_str() != Some("session_meta") || v["payload"]["cwd"].as_str() != Some(cwd) {
            continue;
        }
        let Some(id) = v["payload"]["id"].as_str().or_else(|| v["payload"]["session_id"].as_str()) else { continue };
        if locks.join(format!("{id}.lock")).exists() {
            return Some((id.to_string(), path));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn fixture() -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/codex/rollout.jsonl")).unwrap()
    }

    #[test]
    fn parses_a_finished_turn_with_context_and_last_agent_message() {
        let t = parse_tail(&fixture());
        assert_eq!(t.cwd.as_deref(), Some("/Users/tiagocorreia"));
        assert_eq!(t.session_id.as_deref(), Some("01a0ec59-5564-7f32-b9eb-4287fff41e4c"));
        assert!(!t.working);
        assert_eq!(t.awaiting, None);
        assert_eq!(t.last_agent_text.as_deref(), Some("Hello"));
        assert_eq!(t.context_window, Some(258_400));
        assert!(t.context_used.unwrap() > 0);
        assert!(t.last_event_ms > 1_790_000_000_000);
    }

    #[test]
    fn a_started_turn_without_completion_is_working_and_approvals_are_awaiting() {
        let started = concat!(
            r#"{"timestamp":"2026-09-29T08:47:58.953Z","type":"event_msg","payload":{"type":"task_started","turn_id":"t1","model_context_window":100}}"#, "\n",
        );
        let t = parse_tail(started);
        assert!(t.working);
        let approval = concat!(
            r#"{"timestamp":"2026-09-29T08:47:58.953Z","type":"event_msg","payload":{"type":"task_started","turn_id":"t1"}}"#, "\n",
            r#"{"timestamp":"2026-09-29T08:47:59.953Z","type":"event_msg","payload":{"type":"exec_approval_request","call_id":"c1","command":["rm","-rf","x"],"cwd":"/x"}}"#, "\n",
        );
        let t = parse_tail(approval);
        assert_eq!(t.awaiting.as_deref(), Some("Approve: rm -rf x"));
        // A later completion clears the request.
        let done = format!("{approval}{}\n", r#"{"timestamp":"2026-09-29T08:48:01.517Z","type":"event_msg","payload":{"type":"task_complete","turn_id":"t1","last_agent_message":"ok"}}"#);
        let t = parse_tail(&done);
        assert_eq!(t.awaiting, None);
        assert!(!t.working);
        assert_eq!(t.last_agent_text.as_deref(), Some("ok"));
    }

    #[test]
    fn turns_come_from_user_and_assistant_messages_without_environment_noise() {
        let turns = parse_turns(&fixture(), 30);
        let kinds: Vec<(String, String)> = turns.iter().map(|t| (format!("{:?}", t.kind).to_lowercase(), t.text.clone())).collect();
        assert_eq!(kinds, vec![("user".to_string(), "Say \"Hello\"".to_string()), ("assistant".to_string(), "Hello".to_string())]);
    }

    #[test]
    fn thread_name_is_the_latest_index_entry_for_the_id() {
        let index = concat!(
            r#"{"id":"a","thread_name":"First","updated_at":"2026-09-29T08:38:54Z"}"#, "\n",
            r#"{"id":"b","thread_name":"Other","updated_at":"2026-09-29T08:48:02Z"}"#, "\n",
            r#"{"id":"a","thread_name":"Renamed","updated_at":"2026-09-29T08:48:06Z"}"#, "\n",
        );
        assert_eq!(thread_name(index, "a").as_deref(), Some("Renamed"));
        assert_eq!(thread_name(index, "zzz"), None);
    }

    #[test]
    fn finds_the_live_rollout_for_a_working_directory() {
        let t = tempfile::tempdir().unwrap();
        let codex = t.path();
        let day = codex.join("sessions/2026/09/29");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::create_dir_all(codex.join("thread-writer-locks")).unwrap();
        let meta = |id: &str, cwd: &str| format!(r#"{{"timestamp":"2026-09-29T08:47:58.953Z","type":"session_meta","payload":{{"id":"{id}","cwd":"{cwd}"}}}}"#) + "\n";
        std::fs::write(day.join("rollout-2026-09-29T08-00-00-aaa.jsonl"), meta("aaa", "/Users/x/dev/one")).unwrap();
        std::fs::write(day.join("rollout-2026-09-29T09-00-00-bbb.jsonl"), meta("bbb", "/Users/x/dev/two")).unwrap();
        std::fs::write(day.join("rollout-2026-09-29T10-00-00-ccc.jsonl"), meta("ccc", "/Users/x/dev/one")).unwrap();
        // A real session_meta line runs to tens of KB: it must still be read whole.
        let long = format!(r#"{{"timestamp":"2026-09-29T11:00:00Z","type":"session_meta","payload":{{"id":"ddd","cwd":"/Users/x/dev/long","padding":"{}"}}}}"#, "x".repeat(30_000)) + "\n";
        std::fs::write(day.join("rollout-2026-09-29T11-00-00-ddd.jsonl"), long).unwrap();
        std::fs::write(codex.join("thread-writer-locks/ddd.lock"), "").unwrap();
        assert_eq!(live_rollout_for(codex, "/Users/x/dev/long").unwrap().0, "ddd");
        // Only threads with a writer lock are live.
        std::fs::write(codex.join("thread-writer-locks/aaa.lock"), "").unwrap();
        std::fs::write(codex.join("thread-writer-locks/bbb.lock"), "").unwrap();
        let found = live_rollout_for(codex, "/Users/x/dev/one").unwrap();
        assert_eq!(found.0, "aaa");
        assert!(found.1.ends_with("rollout-2026-09-29T08-00-00-aaa.jsonl"));
        assert!(live_rollout_for(codex, "/Users/x/dev/three").is_none());
    }
}
