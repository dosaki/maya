//! The Claude Code brain: one `claude -p` process kept alive, spoken to in
//! stream-json on stdin and read the same way on stdout.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::driver::{Driver, Event, Launch};

/// The arguments for the long-lived process. A fresh brain gets
/// `--session-id`; a resumed one gets `--resume` with the same id.
pub fn args(l: &Launch, session_id: &str) -> Vec<String> {
    let s = |v: &str| v.to_string();
    let mut a = vec![s("-p"), s("--input-format"), s("stream-json"), s("--output-format"), s("stream-json"), s("--verbose"), s("--permission-mode"), s("auto")];
    match &l.resume {
        Some(id) => a.extend([s("--resume"), id.clone()]),
        None => a.extend([s("--session-id"), s(session_id)]),
    }
    if let Some(m) = &l.model {
        a.extend([s("--model"), m.clone()]);
    }
    a.extend([s("--append-system-prompt"), l.system_prompt.clone()]);
    a
}

/// One user message, as a line for stdin.
pub fn user_line(text: &str) -> String {
    let v = json!({"type": "user", "message": {"role": "user", "content": text}, "parent_tool_use_id": null});
    format!("{v}\n")
}

pub fn interrupt_line(request_id: &str) -> String {
    let v = json!({"type": "control_request", "request_id": request_id, "request": {"subtype": "interrupt"}});
    format!("{v}\n")
}

pub fn stop_task_line(request_id: &str, task_id: &str) -> String {
    let v = json!({"type": "control_request", "request_id": request_id, "request": {"subtype": "stop_task", "task_id": task_id}});
    format!("{v}\n")
}

/// Turns stdout lines into events. A turn ends on `result`, not on the
/// `idle` state, which waits for background agents too.
#[derive(Default)]
pub struct Parser {
    in_turn: bool,
    text: String,
    /// Context size from the last assistant message of the turn.
    last_context: Option<u64>,
    /// Live background tasks, shared with the driver so `interrupt` can stop them.
    pub tasks: Arc<Mutex<BTreeSet<String>>>,
}

/// input + cache read + cache creation tokens of a `usage` object.
fn usage_total(u: &Value) -> Option<u64> {
    if !u.is_object() {
        return None;
    }
    let n = |k: &str| u[k].as_u64().unwrap_or(0);
    Some(n("input_tokens") + n("cache_read_input_tokens") + n("cache_creation_input_tokens"))
}

impl Parser {
    pub fn line(&mut self, line: &str) -> Vec<Event> {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { return vec![] };
        let kind = v["type"].as_str().unwrap_or("");
        let mut out = vec![];
        match kind {
            "assistant" => {
                self.begin(&mut out);
                if let Some(c) = usage_total(&v["message"]["usage"]) {
                    self.last_context = Some(c);
                }
                if let Some(parts) = v["message"]["content"].as_array() {
                    for p in parts {
                        if p["type"] == "text" {
                            self.text.push_str(p["text"].as_str().unwrap_or(""));
                        }
                    }
                }
            }
            "system" => match v["subtype"].as_str().unwrap_or("") {
                "init" => self.begin(&mut out),
                "task_started" if v["is_backgrounded"].as_bool() == Some(true) => {
                    if let Some(id) = v["task_id"].as_str() {
                        self.tasks.lock().unwrap().insert(id.to_string());
                        out.push(Event::TaskStarted(id.to_string()));
                    }
                }
                "task_notification" => {
                    if let Some(id) = v["task_id"].as_str() {
                        if self.tasks.lock().unwrap().remove(id) {
                            out.push(Event::TaskEnded(id.to_string()));
                        }
                    }
                }
                _ => {}
            },
            "result" => {
                self.begin(&mut out);
                let text = v["result"].as_str().map(str::to_string).unwrap_or_else(|| std::mem::take(&mut self.text));
                self.text.clear();
                self.in_turn = false;
                let context_tokens = self.last_context.take().or_else(|| usage_total(&v["usage"]));
                out.push(Event::TurnEnded { text, context_tokens });
            }
            _ => {}
        }
        out
    }

    fn begin(&mut self, out: &mut Vec<Event>) {
        if !self.in_turn {
            self.in_turn = true;
            self.text.clear();
            self.last_context = None;
            out.push(Event::TurnStarted);
        }
    }
}

pub struct ClaudeDriver {
    child: Child,
    stdin: Option<ChildStdin>,
    tasks: Arc<Mutex<BTreeSet<String>>>,
    next_request: u32,
}

impl ClaudeDriver {
    pub fn spawn(l: &Launch, events: Sender<Event>) -> Result<Self, String> {
        let session_id = l.resume.clone().unwrap_or_else(crate::launch::new_session_uuid);
        std::fs::create_dir_all(&l.cwd).map_err(|e| format!("cannot create {}: {e}", l.cwd.display()))?;
        let mut cmd = crate::command(&l.binary);
        cmd.args(args(l, &session_id))
            .current_dir(&l.cwd)
            .env_clear()
            .envs(crate::launch::clean_env(std::env::vars()))
            .env("CLAUDE_CODE_EMIT_SESSION_STATE_EVENTS", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = cmd.spawn().map_err(|e| format!("cannot start {}: {e}", l.binary.display()))?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let mut parser = Parser::default();
        let tasks = parser.tasks.clone();
        let _ = events.send(Event::Ready { session_id });
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                match reader.read_until(b'\n', &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                let line = String::from_utf8_lossy(&buf).into_owned();
                for e in parser.line(&line) {
                    if events.send(e).is_err() {
                        return;
                    }
                }
            }
            let _ = events.send(Event::Exited("the Claude Code process ended".into()));
        });
        Ok(Self { child, stdin, tasks, next_request: 0 })
    }

    fn write(&mut self, line: &str) -> Result<(), String> {
        let stdin = self.stdin.as_mut().ok_or("the brain is shut down")?;
        stdin.write_all(line.as_bytes()).and_then(|_| stdin.flush()).map_err(|e| format!("cannot write to the brain: {e}"))
    }

    fn request_id(&mut self) -> String {
        self.next_request += 1;
        format!("r{}", self.next_request)
    }
}

impl Driver for ClaudeDriver {
    fn send(&mut self, text: &str) -> Result<(), String> {
        self.write(&user_line(text))
    }

    fn interrupt(&mut self) -> Result<(), String> {
        let id = self.request_id();
        self.write(&interrupt_line(&id))?;
        let live: Vec<String> = self.tasks.lock().unwrap().iter().cloned().collect();
        for t in live {
            let id = self.request_id();
            self.write(&stop_task_line(&id, &t))?;
        }
        Ok(())
    }

    fn compact(&mut self) -> Result<(), String> {
        self.write(&user_line("/compact"))
    }

    fn background_subagents(&self) -> bool {
        true
    }

    fn shutdown(&mut self) {
        self.stdin = None;
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for ClaudeDriver {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Harness;

    fn launch() -> Launch {
        Launch { agent: Harness::ClaudeCode, binary: "claude".into(), model: Some("sonnet".into()), cwd: std::env::temp_dir(), system_prompt: "sys".into(), resume: None, writable: vec![] }
    }

    #[test]
    fn claude_runs_long_lived_in_auto_mode_and_resumes_by_id() {
        let mut l = launch();
        let a = args(&l, "u-1");
        for w in ["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--permission-mode", "auto", "--session-id", "u-1", "--model", "sonnet", "--append-system-prompt"] {
            assert!(a.iter().any(|x| x == w), "{w} missing from {a:?}");
        }
        l.resume = Some("u-1".into());
        let r = args(&l, "u-1");
        assert!(r.windows(2).any(|p| p[0] == "--resume" && p[1] == "u-1"));
        assert!(!r.iter().any(|x| x == "--session-id"));
    }

    #[test]
    fn a_user_line_is_one_stream_json_message() {
        let v: serde_json::Value = serde_json::from_str(user_line("hi").trim_end()).unwrap();
        assert_eq!(v["type"], "user");
        assert_eq!(v["message"]["content"], "hi");
        assert!(user_line("hi").ends_with('\n'));
    }

    #[test]
    fn a_result_ends_the_turn_with_its_text_and_context() {
        let mut p = Parser::default();
        assert_eq!(p.line(r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hi"}],"usage":{"input_tokens":10,"cache_read_input_tokens":90,"cache_creation_input_tokens":0}}}"#), vec![Event::TurnStarted]);
        let e = p.line(r#"{"type":"result","subtype":"success","result":"Hi","usage":{"input_tokens":50,"cache_read_input_tokens":450,"cache_creation_input_tokens":0}}"#);
        assert_eq!(e, vec![Event::TurnEnded { text: "Hi".into(), context_tokens: Some(100) }]);
    }

    #[test]
    fn without_assistant_usage_the_result_usage_is_the_fallback() {
        let mut p = Parser::default();
        p.line(r#"{"type":"assistant","message":{"content":[]}}"#);
        let e = p.line(r#"{"type":"result","subtype":"success","result":"x","usage":{"input_tokens":10,"cache_read_input_tokens":90,"cache_creation_input_tokens":0}}"#);
        assert_eq!(e, vec![Event::TurnEnded { text: "x".into(), context_tokens: Some(100) }]);
    }

    #[test]
    fn a_turn_she_starts_herself_is_a_new_turn_and_starts_once() {
        let mut p = Parser::default();
        p.line(r#"{"type":"assistant","message":{"content":[]}}"#);
        p.line(r#"{"type":"result","subtype":"success","result":"a"}"#);
        assert_eq!(p.line(r#"{"type":"system","subtype":"init"}"#), vec![Event::TurnStarted]);
        assert!(p.line(r#"{"type":"assistant","message":{"content":[]}}"#).is_empty());
        assert!(p.line(r#"{"type":"system","subtype":"session_state_changed","state":"idle"}"#).is_empty());
        assert_eq!(p.line(r#"{"type":"result","subtype":"success","result":"b"}"#), vec![Event::TurnEnded { text: "b".into(), context_tokens: None }]);
    }

    #[test]
    fn background_tasks_are_tracked() {
        let mut p = Parser::default();
        assert_eq!(p.line(r#"{"type":"system","subtype":"task_started","task_id":"t1","is_backgrounded":true}"#), vec![Event::TaskStarted("t1".into())]);
        assert!(p.tasks.lock().unwrap().contains("t1"));
        assert!(p.line(r#"{"type":"system","subtype":"task_started","task_id":"t2","is_backgrounded":false}"#).is_empty());
        assert_eq!(p.line(r#"{"type":"system","subtype":"task_notification","task_id":"t1","status":"completed"}"#), vec![Event::TaskEnded("t1".into())]);
        assert!(p.tasks.lock().unwrap().is_empty());
    }

    #[test]
    fn an_interrupt_is_a_control_request() {
        let v: serde_json::Value = serde_json::from_str(interrupt_line("r1").trim_end()).unwrap();
        assert_eq!(v["request"]["subtype"], "interrupt");
        let s: serde_json::Value = serde_json::from_str(stop_task_line("r2", "t1").trim_end()).unwrap();
        assert_eq!(s["request"]["subtype"], "stop_task");
        assert_eq!(s["request"]["task_id"], "t1");
    }

    #[cfg(unix)]
    #[test]
    fn a_fake_claude_process_runs_a_turn_and_exits() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("maya-fake-claude-{}", crate::launch::new_session_uuid()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("claude");
        std::fs::write(&bin, "#!/bin/sh\nread line\necho '{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"ok\"}]}}'\necho '{\"type\":\"result\",\"subtype\":\"success\",\"result\":\"ok\"}'\ncat >/dev/null\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut l = launch();
        l.binary = bin;
        l.cwd = dir.join("work");
        let (tx, rx) = std::sync::mpsc::channel();
        let mut d = super::super::driver::spawn(&l, tx).unwrap();
        assert!(matches!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Event::Ready { .. }));
        d.send("hi").unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Event::TurnStarted);
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Event::TurnEnded { text: "ok".into(), context_tokens: None });
        d.shutdown();
        assert!(matches!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Event::Exited(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
