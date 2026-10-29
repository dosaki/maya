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

/// `info.total_token_usage` → counts: input = input_tokens − cached_input_tokens, cached_input = cached_input_tokens, cache_write = cache_write_input_tokens, output = output_tokens (reasoning is inside output).
pub fn token_counts(p: &Value) -> Option<crate::usage::TokenCounts> {
    let u = p["info"]["total_token_usage"].as_object()?;
    let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
    let cached = n("cached_input_tokens");
    Some(crate::usage::TokenCounts { input: n("input_tokens").saturating_sub(cached), cached_input: cached, cache_write: n("cache_write_input_tokens"), output: n("output_tokens") })
}

/// `rate_limits` → windows from `primary` and `secondary` (skipping a null one), labelled by `window_label(window_minutes)`, `used_percent` rounded, `resets_at` in seconds; `read_at` is the line's timestamp. None when `rate_limits` is null or has no windows.
pub fn rate_limits(p: &Value, at_ms: u64) -> Option<crate::usage::AccountLimits> {
    let r = &p["rate_limits"];
    let windows: Vec<crate::usage::LimitWindow> = ["primary", "secondary"]
        .iter()
        .filter_map(|k| {
            let w = &r[*k];
            let minutes = w["window_minutes"].as_u64().filter(|m| *m > 0)?;
            let used = w["used_percent"].as_f64()?;
            Some(crate::usage::LimitWindow { label: crate::usage::window_label(minutes), percent: used.round().clamp(0.0, 100.0) as u8, resets_at: w["resets_at"].as_u64() })
        })
        .collect();
    (!windows.is_empty()).then_some(crate::usage::AccountLimits { account: None, windows, read_at: at_ms })
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
            Some("turn_context") => {
                if let Some(m) = p["model"].as_str() {
                    t.model = Some(m.to_string());
                }
            }
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
                "token_count" => {
                    if let Some(tokens) = token_counts(p) {
                        t.usage = Some(crate::usage::SessionUsage { tokens, cost: None });
                    }
                    if let Some(l) = rate_limits(p, v["timestamp"].as_str().and_then(ms_of).unwrap_or(0)) {
                        t.limits = Some(l);
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
    // Priced once, with the newest model, after the whole tail is read.
    if let (Some(u), Some(model)) = (t.usage.as_mut(), t.model.as_ref()) {
        if let Some(micros) = crate::pricing::cost_micros(model, &u.tokens, false) {
            u.cost = Some(crate::usage::Cost { micros, unit: crate::usage::CostUnit::Usd, estimated: true, models: vec![model.clone()] });
        }
    }
    t
}

/// User and assistant messages and tool calls with their output, newest
/// last, skipping developer prompts and the environment preamble Codex
/// injects as a user message. Tool calls do not count against `max_turns`.
pub fn parse_turns(text: &str, max_turns: usize) -> Vec<Turn> {
    let mut out: Vec<Turn> = Vec::new();
    // Where each call's turn sits, by its call_id, for its output.
    let mut calls: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if v["type"].as_str() != Some("response_item") {
            continue;
        }
        let p = &v["payload"];
        match p["type"].as_str() {
            Some("message") => {
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
                out.push(Turn::new(kind, text.to_string()));
            }
            Some("function_call" | "custom_tool_call") => {
                if let Some(id) = p["call_id"].as_str() {
                    calls.insert(id.to_string(), out.len());
                }
                out.push(Turn::new(TurnKind::Tool, call_line(p)));
            }
            Some("function_call_output" | "custom_tool_call_output") => {
                let Some(&i) = p["call_id"].as_str().and_then(|id| calls.get(id)) else { continue };
                let (text, failed) = call_output(p);
                out[i].is_error = failed;
                out[i].set_output(&text);
            }
            _ => {}
        }
    }
    crate::transcript::keep_last_messages(out, max_turns)
}

/// `name(arg)` for a Codex tool call: a shell call's command line, a
/// patch's file, else the input's first line; just the name without one.
fn call_line(p: &Value) -> String {
    let name = p["name"].as_str().unwrap_or("tool");
    let args: Value = p["arguments"].as_str().and_then(|a| serde_json::from_str(a).ok()).unwrap_or(Value::Null);
    let input = p["input"].as_str().unwrap_or("");
    let arg = shell_line(&args["command"])
        .or_else(|| args["cmd"].as_str().map(str::to_string))
        .or_else(|| args["path"].as_str().map(str::to_string))
        .or_else(|| patch_file(input))
        .or_else(|| input.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string));
    match arg.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
        Some(a) => format!("{name}({})", crate::state::truncate(a, 120)),
        None => name.to_string(),
    }
}

/// The command a shell call runs: `["bash", "-lc", "ls"]` is the line `ls`;
/// other argument lists are joined with spaces.
fn shell_line(command: &Value) -> Option<String> {
    match command {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => {
            let parts: Vec<&str> = parts.iter().filter_map(Value::as_str).collect();
            match parts.as_slice() {
                [shell, flag, line] if shell.ends_with("sh") && flag.starts_with('-') && flag.ends_with('c') => Some(line.to_string()),
                _ => Some(parts.join(" ")).filter(|s| !s.is_empty()),
            }
        }
        _ => None,
    }
}

/// The file an `apply_patch` input changes, from its first `*** … File:` line.
fn patch_file(input: &str) -> Option<String> {
    input
        .lines()
        .find_map(|l| ["*** Update File: ", "*** Add File: ", "*** Delete File: "].iter().find_map(|p| l.strip_prefix(*p)))
        .map(|f| f.trim().to_string())
}

/// A tool call's output text and whether it failed. Older rollouts wrap it
/// as `{"output": …, "metadata": {"exit_code": n}}`; newer ones lead with
/// `Exit code: n`.
fn call_output(p: &Value) -> (String, bool) {
    let raw = p["output"].as_str().unwrap_or("");
    if let Ok(v) = serde_json::from_str::<Value>(raw) {
        if let Some(out) = v["output"].as_str() {
            return (out.to_string(), v["metadata"]["exit_code"].as_i64().is_some_and(|c| c != 0));
        }
    }
    let code = raw.lines().next().and_then(|l| l.strip_prefix("Exit code: ")).and_then(|c| c.trim().parse::<i64>().ok());
    (raw.to_string(), code.is_some_and(|c| c != 0))
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

/// Every rollout file under `sessions/`.
pub fn rollouts(codex_dir: &Path) -> Vec<PathBuf> {
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

/// The thread id and working directory a rollout's `session_meta` records.
pub fn rollout_meta(path: &Path) -> Option<(String, String)> {
    let first = first_line(path)?;
    let v: Value = serde_json::from_str(first.trim()).ok()?;
    if v["type"].as_str() != Some("session_meta") {
        return None;
    }
    let id = v["payload"]["id"].as_str().or_else(|| v["payload"]["session_id"].as_str())?.to_string();
    Some((id, v["payload"]["cwd"].as_str()?.to_string()))
}

/// The rollout of thread `id` and the working directory it records.
pub fn rollout_for_id(codex_dir: &Path, id: &str) -> Option<(PathBuf, String)> {
    let suffix = format!("-{id}.jsonl");
    rollouts(codex_dir)
        .into_iter()
        .find(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(&suffix)))
        .and_then(|p| rollout_meta(&p).map(|(_, cwd)| (p, cwd)))
}

/// The thread ids with a writer lock: the live threads.
pub fn locked_threads(codex_dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(codex_dir.join("thread-writer-locks")) else { return vec![] };
    entries
        .flatten()
        .map(|e| e.path())
        .filter_map(|p| Some((p.file_name()?.to_str()?.strip_suffix(".lock")?.to_string(), p.clone())))
        .collect()
}

/// The (thread id, rollout path) of the newest live thread whose recorded
/// working directory is `cwd`. Live means a writer lock exists for it.
pub fn live_rollout_for(codex_dir: &Path, cwd: &str) -> Option<(String, PathBuf)> {
    let locks = codex_dir.join("thread-writer-locks");
    let mut files = rollouts(codex_dir);
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

    fn token_fixture() -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/codex/token_count.jsonl")).unwrap()
    }

    fn payload(line: usize) -> Value {
        let text = token_fixture();
        let v: Value = serde_json::from_str(text.lines().nth(line).unwrap()).unwrap();
        v["payload"].clone()
    }

    #[test]
    fn token_counts_split_cached_from_input() {
        let c = token_counts(&payload(1)).unwrap();
        assert_eq!(c, crate::usage::TokenCounts { input: 2918, cached_input: 12160, cache_write: 0, output: 5 });
    }

    #[test]
    fn rate_limits_label_windows_by_minutes_and_skip_null() {
        let l = rate_limits(&payload(1), 7).unwrap();
        let got: Vec<_> = l.windows.iter().map(|w| (w.label.as_str(), w.percent, w.resets_at)).collect();
        assert_eq!(got, vec![("5h", 0, Some(1_790_986_755)), ("7d", 1, Some(1_791_276_482))]);
        assert_eq!(l.read_at, 7);
        assert!(rate_limits(&payload(2), 7).is_none());
    }

    #[test]
    fn parse_tail_keeps_the_newest_usage_model_and_limits() {
        let t = parse_tail(&token_fixture());
        assert_eq!(t.model.as_deref(), Some("gpt-6.1-sol"));
        let u = t.usage.unwrap();
        assert_eq!(u.tokens.output, 90);
        assert_eq!(t.limits.unwrap().read_at, ms_of("2026-10-29T09:12:33.123Z").unwrap());
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
    fn a_rollout_is_found_by_thread_id_with_its_working_directory() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("sessions/2026/10/01");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(day.join("rollout-2026-10-01T08-00-00-abc.jsonl"), "{\"type\":\"session_meta\",\"payload\":{\"id\":\"abc\",\"cwd\":\"E:\\\\dev\\\\x\"}}\n").unwrap();
        let (path, cwd) = rollout_for_id(dir.path(), "abc").unwrap();
        assert!(path.ends_with("rollout-2026-10-01T08-00-00-abc.jsonl"));
        assert_eq!(cwd, "E:\\dev\\x");
        assert_eq!(rollout_for_id(dir.path(), "zzz"), None);
        std::fs::create_dir_all(dir.path().join("thread-writer-locks")).unwrap();
        std::fs::write(dir.path().join("thread-writer-locks/abc.lock"), "").unwrap();
        std::fs::write(dir.path().join("thread-writer-locks/notes.txt"), "").unwrap();
        assert_eq!(locked_threads(dir.path()).into_iter().map(|(id, _)| id).collect::<Vec<_>>(), ["abc"]);
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

    #[test]
    fn tool_calls_carry_their_output_and_a_failed_exit_code() {
        let text = concat!(
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"list files"}]}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"bash\",\"-lc\",\"ls\"],\"workdir\":\"/x\"}","call_id":"call_1"}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"function_call_output","call_id":"call_1","output":"{\"output\":\"a.rs\\nb.rs\\n\",\"metadata\":{\"exit_code\":0,\"duration_seconds\":0.1}}"}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"cat nope\"}","call_id":"call_2"}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"function_call_output","call_id":"call_2","output":"Exit code: 1\nWall time: 0.1 seconds\nOutput:\ncat: nope: No such file or directory\n"}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"custom_tool_call","name":"apply_patch","input":"*** Begin Patch\n*** Update File: src/a.rs\n@@\n-x\n+y\n*** End Patch","call_id":"call_3"}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"call_3","output":"Success. Updated the following files:\nM src/a.rs\n"}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"function_call_output","call_id":"call_from_before_the_tail","output":"lost"}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Two files."}]}}"#, "\n",
        );
        let turns = parse_turns(text, 30);
        let got: Vec<(TurnKind, &str, Option<&str>, bool)> = turns.iter().map(|t| (t.kind, t.text.as_str(), t.output.as_deref(), t.is_error)).collect();
        assert_eq!(got, vec![
            (TurnKind::User, "list files", None, false),
            (TurnKind::Tool, "shell(ls)", Some("a.rs\nb.rs"), false),
            (TurnKind::Tool, "exec_command(cat nope)", Some("Exit code: 1\nWall time: 0.1 seconds\nOutput:\ncat: nope: No such file or directory"), true),
            (TurnKind::Tool, "apply_patch(src/a.rs)", Some("Success. Updated the following files:\nM src/a.rs"), false),
            (TurnKind::Assistant, "Two files.", None, false),
        ]);
    }

    #[test]
    fn tool_calls_do_not_push_codex_messages_out() {
        let mut text = String::from(r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"go"}]}}"#);
        for i in 0..40 {
            text.push_str(&format!("\n{{\"type\":\"response_item\",\"payload\":{{\"type\":\"function_call\",\"name\":\"shell\",\"arguments\":\"{{\\\"cmd\\\":\\\"echo {i}\\\"}}\",\"call_id\":\"c{i}\"}}}}"));
        }
        let turns = parse_turns(&text, 2);
        assert_eq!(turns.len(), 41, "the one message and every call after it");
        assert_eq!(turns[0].text, "go");
    }
}
