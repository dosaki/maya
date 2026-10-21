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
/// `idle` state, which waits for background agents too. `Ready` comes once,
/// from her `init` line, which names the session she really runs.
#[derive(Default)]
pub struct Parser {
    /// The id Maya started her with: the session's when `init` names none.
    assumed_id: Option<String>,
    ready: bool,
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
    pub fn new(assumed_id: &str) -> Parser {
        Parser { assumed_id: Some(assumed_id.to_string()), ..Parser::default() }
    }

    /// `Ready` with `id` (else the assumed one), the first time only.
    fn ready(&mut self, id: Option<&str>, out: &mut Vec<Event>) {
        if self.ready {
            return;
        }
        if let Some(session_id) = id.map(str::to_string).or_else(|| self.assumed_id.clone()) {
            self.ready = true;
            out.push(Event::Ready { session_id });
        }
    }

    pub fn line(&mut self, line: &str) -> Vec<Event> {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { return vec![] };
        let kind = v["type"].as_str().unwrap_or("");
        let mut out = vec![];
        // A subagent's own messages: not her turn, her words or her context.
        if kind != "system" && !v["parent_tool_use_id"].is_null() {
            return out;
        }
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
            // Only her own messages and a result start a turn: an `init`
            // can come with no turn after it (a resumed, idle session), and
            // would leave her working for ever.
            "system" => match v["subtype"].as_str().unwrap_or("") {
                "init" => self.ready(v["session_id"].as_str().filter(|id| !id.is_empty()), &mut out),
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
                // No init came: a turn that ended proves the session Maya started.
                self.ready(None, &mut out);
                self.begin(&mut out);
                let text = v["result"].as_str().map(str::to_string).unwrap_or_else(|| std::mem::take(&mut self.text));
                self.text.clear();
                self.in_turn = false;
                let context_tokens = self.last_context.take().or_else(|| usage_total(&v["usage"]));
                // An error (an interrupt, a turn limit, an API error) is never spoken as her reply.
                let failed = v["is_error"].as_bool() == Some(true) || v["subtype"].as_str().is_some_and(|s| s.starts_with("error"));
                out.push(if failed { Event::TurnFailed { context_tokens } } else { Event::TurnEnded { text, context_tokens } });
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
    /// The id she was started with (`--session-id` or `--resume`).
    session_id: String,
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
        // Ready comes from her init line, not from the spawn: see `Parser`.
        let mut parser = Parser::new(&session_id);
        let tasks = parser.tasks.clone();
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
        Ok(Self { child, session_id, stdin, tasks, next_request: 0 })
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

    fn assumed_session_id(&self) -> Option<String> {
        Some(self.session_id.clone())
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

/// Shutting down waits for the process (or kills and waits), so a brain
/// that exited on its own leaves no zombie behind once dropped.
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
        assert!(!p.line(r#"{"type":"system","subtype":"init"}"#).contains(&Event::TurnStarted), "init alone starts nothing");
        assert_eq!(p.line(r#"{"type":"assistant","message":{"content":[]}}"#), vec![Event::TurnStarted]);
        assert!(p.line(r#"{"type":"assistant","message":{"content":[]}}"#).is_empty());
        assert!(p.line(r#"{"type":"system","subtype":"session_state_changed","state":"idle"}"#).is_empty());
        assert_eq!(p.line(r#"{"type":"result","subtype":"success","result":"b"}"#), vec![Event::TurnEnded { text: "b".into(), context_tokens: None }]);
    }

    #[test]
    fn ready_comes_once_from_the_init_line_with_its_session_id() {
        let mut p = Parser::new("assumed");
        assert_eq!(p.line(r#"{"type":"system","subtype":"init","session_id":"real"}"#), vec![Event::Ready { session_id: "real".into() }]);
        assert!(p.line(r#"{"type":"system","subtype":"init","session_id":"real"}"#).is_empty());
        let mut p = Parser::new("assumed");
        assert_eq!(p.line(r#"{"type":"system","subtype":"init"}"#), vec![Event::Ready { session_id: "assumed".into() }], "an init without an id: the one Maya chose");
        let mut p = Parser::new("assumed");
        assert_eq!(p.line(r#"{"type":"assistant","message":{"content":[]}}"#), vec![Event::TurnStarted]);
        assert_eq!(p.line(r#"{"type":"result","subtype":"success","result":"x"}"#), vec![Event::Ready { session_id: "assumed".into() }, Event::TurnEnded { text: "x".into(), context_tokens: None }], "no init at all: ready by the first result");
    }

    #[test]
    fn a_resumed_idle_session_is_not_working_after_its_init() {
        let mut p = Parser::default();
        assert!(!p.line(r#"{"type":"system","subtype":"init","session_id":"u"}"#).contains(&Event::TurnStarted));
        assert!(!p.line(r#"{"type":"system","subtype":"init","session_id":"u"}"#).contains(&Event::TurnStarted));
    }

    #[test]
    fn subagent_lines_start_no_turn_and_add_no_text_or_usage() {
        let mut p = Parser::default();
        let sub = r#"{"type":"assistant","parent_tool_use_id":"toolu_1","message":{"content":[{"type":"text","text":"sub says"}],"usage":{"input_tokens":900000}}}"#;
        assert!(p.line(sub).is_empty(), "a subagent's message is not her turn");
        assert_eq!(p.line(r#"{"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"text","text":"Hi"}],"usage":{"input_tokens":10}}}"#), vec![Event::TurnStarted]);
        assert!(p.line(sub).is_empty());
        assert_eq!(p.line(r#"{"type":"result","subtype":"success"}"#), vec![Event::TurnEnded { text: "Hi".into(), context_tokens: Some(10) }]);
    }

    #[test]
    fn an_error_result_fails_the_turn_and_its_text_is_never_her_reply() {
        let mut p = Parser::default();
        p.line(r#"{"type":"assistant","message":{"content":[{"type":"text","text":"half"}]}}"#);
        assert_eq!(p.line(r#"{"type":"result","subtype":"error_during_execution","is_error":true}"#), vec![Event::TurnFailed { context_tokens: None }]);
        assert_eq!(p.line(r#"{"type":"result","subtype":"success","is_error":true,"result":"API Error: 500"}"#), vec![Event::TurnStarted, Event::TurnFailed { context_tokens: None }]);
        assert_eq!(p.line(r#"{"type":"result","subtype":"error_max_turns"}"#), vec![Event::TurnStarted, Event::TurnFailed { context_tokens: None }]);
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
    fn a_brain_that_exited_is_reaped_when_dropped() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("maya-fake-claude-{}", crate::launch::new_session_uuid()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("claude");
        std::fs::write(&bin, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut l = launch();
        l.binary = bin;
        l.cwd = dir.join("work");
        let (tx, rx) = std::sync::mpsc::channel();
        let d = ClaudeDriver::spawn(&l, tx).unwrap();
        let pid = d.child.id().to_string();
        assert!(matches!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Event::Exited(_)));
        let alive = || std::process::Command::new("kill").args(["-0", &pid]).stderr(Stdio::null()).status().unwrap().success();
        assert!(alive(), "an exited child stays a zombie until it is waited for");
        drop(d);
        assert!(!alive(), "dropping the driver reaps it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_fake_claude_process_runs_a_turn_and_exits() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("maya-fake-claude-{}", crate::launch::new_session_uuid()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("claude");
        std::fs::write(&bin, "#!/bin/sh\nread line\necho '{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"fake-1\"}'\necho '{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"ok\"}]}}'\necho '{\"type\":\"result\",\"subtype\":\"success\",\"result\":\"ok\"}'\ncat >/dev/null\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut l = launch();
        l.binary = bin;
        l.cwd = dir.join("work");
        let (tx, rx) = std::sync::mpsc::channel();
        let mut d = super::super::driver::spawn(&l, tx).unwrap();
        assert!(d.assumed_session_id().is_some());
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "not ready until her init line");
        d.send("hi").unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Event::Ready { session_id: "fake-1".into() });
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Event::TurnStarted);
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Event::TurnEnded { text: "ok".into(), context_tokens: None });
        d.shutdown();
        assert!(matches!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Event::Exited(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
