//! The Codex brain: one `codex app-server` kept alive on stdio, spoken to in
//! JSON-RPC, one object per line (Codex's messages carry no `"jsonrpc"`).
//!
//! The thread opens with `initialize`, the `initialized` notification, then
//! `thread/start` (or `thread/resume`); her session id is the thread id
//! Codex answers with, which is also the id its rollout file and writer
//! lock carry (what Maya's board keys Codex sessions by). A turn is one
//! `turn/start`: agent messages arrive as completed items and the
//! `turn/completed` notification ends it, with a status.
//!
//! Auto mode: `on-request` approvals reviewed by Codex's own auto-review,
//! the `workspace-write` sandbox with her writable folders added. An
//! approval request that reaches Maya is declined: Maya never approves on
//! the agent's behalf. Codex's multi-agent tools are off
//! for her server: its subagents do not wake her when they finish.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::Sender;

use serde_json::{json, Value};

use super::driver::{Event, Launch};
use super::jsonrpc::{self, Envelope, Incoming};
use super::stdio::{self, LineDriver, Protocol};

/// Codex's messages carry no `"jsonrpc"` field.
const RPC: Envelope = Envelope::Bare;

/// A stopped turn with no `turn/completed` within this is failed.
pub const CANCEL_GRACE_MS: u64 = 5_000;
/// How many ended turn ids are kept to recognise their late news.
const REMEMBERED_TURNS: usize = 32;

/// The arguments after the binary: the app server on stdio, with Codex's
/// subagent tools disabled for her (`--disable` is `-c features.<name>=false`).
pub fn command() -> Vec<String> {
    ["app-server", "--disable", "multi_agent", "--disable", "multi_agent_v2"].map(String::from).to_vec()
}

/// Only stable API: no experimental opt-in.
pub fn initialize(id: u64) -> String {
    RPC.request(id, "initialize", json!({"clientInfo": {"name": "maya", "title": "Maya", "version": env!("CARGO_PKG_VERSION")}}))
}

/// What `thread/start` and `thread/resume` both set: auto mode, her
/// sandbox's writable folders, her system prompt and model.
fn overrides(l: &Launch) -> Value {
    let roots: Vec<String> = l.writable.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    let mut p = json!({
        "cwd": l.cwd.to_string_lossy(),
        "approvalPolicy": "on-request",
        "approvalsReviewer": "auto_review",
        "sandbox": "workspace-write",
        "developerInstructions": l.system_prompt,
        // A dotted key, so it merges with her own [sandbox_workspace_write]
        // settings instead of replacing the table.
        "config": {"sandbox_workspace_write.writable_roots": roots},
    });
    if let Some(m) = &l.model {
        p["model"] = json!(m);
    }
    p
}

pub fn thread_start(id: u64, l: &Launch) -> String {
    RPC.request(id, "thread/start", overrides(l))
}

/// `thread/resume` with the same overrides; her past turns are not sent back.
pub fn thread_resume(id: u64, thread_id: &str, l: &Launch) -> String {
    let mut p = overrides(l);
    p["threadId"] = json!(thread_id);
    p["excludeTurns"] = json!(true);
    RPC.request(id, "thread/resume", p)
}

pub fn turn_start(id: u64, thread_id: &str, text: &str) -> String {
    RPC.request(id, "turn/start", json!({"threadId": thread_id, "input": [{"type": "text", "text": text}]}))
}

pub fn turn_interrupt(id: u64, thread_id: &str, turn_id: &str) -> String {
    RPC.request(id, "turn/interrupt", json!({"threadId": thread_id, "turnId": turn_id}))
}

/// The answer to a request from Codex, by method: approvals are declined,
/// anything else is refused with method-not-found, never left unanswered.
fn answer(method: &str) -> Option<Value> {
    match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => Some(json!({"decision": "decline"})),
        "execCommandApproval" | "applyPatchApproval" => Some(json!({"decision": "denied"})),
        // Grants nothing more than the sandbox already allows.
        "item/permissions/requestApproval" => Some(json!({"permissions": {}, "scope": "turn"})),
        "mcpServer/elicitation/request" => Some(json!({"action": "decline"})),
        _ => None,
    }
}

/// What a request Maya declines asked for, for the log: its kind, and the
/// command, files or message it names.
fn asked_for(method: &str, params: &Value) -> String {
    let text = |k: &str| params[k].as_str().filter(|s| !s.is_empty());
    let why = |s: String| match text("reason") {
        Some(r) => format!("{s} ({r})"),
        None => s,
    };
    match method {
        "item/commandExecution/requestApproval" | "execCommandApproval" => {
            let c = &params["command"];
            let command = c.as_str().map(str::to_string).or_else(|| c.as_array().map(|parts| parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")));
            match command {
                Some(c) => format!("a command approval for {c}"),
                None => why("a command approval".into()),
            }
        }
        "item/fileChange/requestApproval" => why(match text("grantRoot") {
            Some(root) => format!("a file change approval under {root}"),
            None => "a file change approval".into(),
        }),
        "applyPatchApproval" => {
            let files: Vec<&str> = params["fileChanges"].as_object().map(|m| m.keys().map(String::as_str).collect()).unwrap_or_default();
            if files.is_empty() {
                "a patch approval".into()
            } else {
                format!("a patch approval for {}", files.join(", "))
            }
        }
        "item/permissions/requestApproval" => why("a permissions request".into()),
        "mcpServer/elicitation/request" => {
            format!("an MCP elicitation from {}: {}", text("serverName").unwrap_or("a server"), text("message").unwrap_or(""))
        }
        _ => method.to_string(),
    }
}

/// The log line for a declined request.
fn declined(method: &str, params: &Value) -> String {
    format!("Codex: declined {} (Maya never approves on the agent's behalf)", asked_for(method, params))
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    /// `initialize` is out.
    Starting,
    /// `thread/start` or `thread/resume` is out.
    Opening,
    Open,
    /// The thread could not open: the process is done for.
    Failed,
}

/// A `turn/start` without an answer.
#[derive(Default)]
struct Pending {
    /// Its turn already ended here (a stop's grace ran out): the turn it
    /// names, late, is stopped and not hers.
    settled: bool,
}

/// The protocol, with no process in it: lines in, events and lines to
/// write out. Requests are numbered per parser, from 1.
pub struct Parser {
    l: Launch,
    next_id: u64,
    phase: Phase,
    init_id: Option<u64>,
    open_id: Option<u64>,
    thread_id: Option<String>,
    /// Messages sent before her thread opened, in order.
    waiting: Vec<String>,
    pending: HashMap<u64, Pending>,
    in_turn: bool,
    /// The running turn's id, once Codex has named it.
    turn_id: Option<String>,
    /// The turn's last agent message, and its last one marked final.
    last_text: String,
    final_text: Option<String>,
    cancel_at: Option<u64>,
    /// A stop is out but the turn had no id to name yet.
    interrupt_pending: bool,
    /// Turns that ended here, newest last: their late news adds nothing.
    over: VecDeque<String>,
}

impl Parser {
    pub fn new(l: &Launch) -> Parser {
        Parser {
            l: l.clone(),
            next_id: 0,
            phase: Phase::Starting,
            init_id: None,
            open_id: None,
            thread_id: None,
            waiting: vec![],
            pending: HashMap::new(),
            in_turn: false,
            turn_id: None,
            last_text: String::new(),
            final_text: None,
            cancel_at: None,
            interrupt_pending: false,
            over: VecDeque::new(),
        }
    }

    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn thread(&self) -> &str {
        self.thread_id.as_deref().unwrap_or("")
    }

    fn is_over(&self, turn: &str) -> bool {
        self.over.iter().any(|t| t == turn)
    }

    fn remember_over(&mut self, turn: String) {
        if !self.is_over(&turn) {
            self.over.push_back(turn);
            if self.over.len() > REMEMBERED_TURNS {
                self.over.pop_front();
            }
        }
    }

    /// The first line to write: `initialize`.
    pub fn start(&mut self) -> String {
        let id = self.id();
        self.init_id = Some(id);
        initialize(id)
    }

    /// A message for her: the `turn/start` to write now, or nothing while
    /// her thread has not opened yet (it goes out once it has). Her turn
    /// starts either way.
    pub fn send(&mut self, text: &str) -> (Vec<Event>, Option<String>) {
        let mut out = vec![];
        match self.phase {
            Phase::Open => {
                let line = self.turn_line(text, &mut out);
                (out, Some(line))
            }
            Phase::Starting | Phase::Opening => {
                self.waiting.push(text.to_string());
                self.begin(&mut out);
                (out, None)
            }
            Phase::Failed => (out, None),
        }
    }

    fn turn_line(&mut self, text: &str, out: &mut Vec<Event>) -> String {
        // Starts already failed by their stop are forgotten: a turn that
        // starts from now on is not taken for theirs.
        self.pending.retain(|_, p| !p.settled);
        let id = self.id();
        self.pending.insert(id, Pending::default());
        self.begin(out);
        turn_start(id, self.thread(), text)
    }

    /// Stops the running turn: the `turn/interrupt` to write, or None when
    /// no turn runs or Codex has not named it yet (it goes out once it
    /// has). Before her thread opens, the messages waiting for it are
    /// dropped and her turn fails at once: no Codex turn runs to stop.
    pub fn cancel(&mut self, now_ms: u64) -> (Vec<Event>, Option<String>) {
        let mut out = vec![];
        self.waiting.clear();
        if self.phase != Phase::Open {
            self.end(false, &mut out);
            return (out, None);
        }
        if !self.in_turn {
            return (out, None);
        }
        self.cancel_at.get_or_insert(now_ms);
        match self.turn_id.clone() {
            Some(turn) => {
                let id = self.id();
                (out, Some(turn_interrupt(id, self.thread(), &turn)))
            }
            None => {
                self.interrupt_pending = true;
                (out, None)
            }
        }
    }

    /// What time alone ends: a stopped turn with no `turn/completed`
    /// within `CANCEL_GRACE_MS` fails.
    pub fn tick(&mut self, now_ms: u64) -> Vec<Event> {
        let mut out = vec![];
        if self.in_turn && self.cancel_at.is_some_and(|t| now_ms.saturating_sub(t) >= CANCEL_GRACE_MS) {
            for p in self.pending.values_mut() {
                p.settled = true;
            }
            self.end(false, &mut out);
        }
        out
    }

    /// One line from Codex: the events it makes and what to write back.
    pub fn line(&mut self, line: &str) -> (Vec<Event>, Option<String>) {
        match jsonrpc::parse(line) {
            Some(Incoming::Request { id, method, params }) => (vec![], Some(self.request(&method, &id, &params))),
            Some(Incoming::Notification { method, params }) => self.notification(&method, &params),
            Some(Incoming::Response { id, result, error }) => self.response(id, &result, error),
            None => (vec![], None),
        }
    }

    fn request(&mut self, method: &str, id: &Value, params: &Value) -> String {
        match answer(method) {
            Some(result) => {
                crate::log::line("brain", declined(method, params));
                RPC.reply(id, result)
            }
            None => {
                crate::log::line("brain", format!("Codex asked for {method}, which Maya does not handle"));
                RPC.reply_error(id, jsonrpc::METHOD_NOT_FOUND, &format!("Method not found: {method}"))
            }
        }
    }

    fn response(&mut self, id: Option<u64>, result: &Value, error: Option<String>) -> (Vec<Event>, Option<String>) {
        let Some(id) = id else { return (vec![], None) };
        let mut out = vec![];
        if self.init_id == Some(id) {
            self.init_id = None;
            if let Some(e) = error {
                return (vec![self.fail(format!("Codex did not start: {e}"))], None);
            }
            let id = self.id();
            self.open_id = Some(id);
            self.phase = Phase::Opening;
            let open = match self.l.resume.clone() {
                Some(thread) => thread_resume(id, &thread, &self.l),
                None => thread_start(id, &self.l),
            };
            return (out, Some(RPC.notification("initialized", None) + &open));
        }
        if self.open_id == Some(id) {
            self.open_id = None;
            if let Some(e) = error {
                return (vec![self.fail(format!("her Codex thread did not open: {e}"))], None);
            }
            let Some(thread) = result["thread"]["id"].as_str().map(str::to_string) else {
                return (vec![self.fail("her Codex thread opened with no id".into())], None);
            };
            self.thread_id = Some(thread.clone());
            self.phase = Phase::Open;
            out.push(Event::Ready { session_id: thread });
            let mut lines = String::new();
            for text in std::mem::take(&mut self.waiting) {
                lines.push_str(&self.turn_line(&text, &mut out));
            }
            return (out, (!lines.is_empty()).then_some(lines));
        }
        let Some(p) = self.pending.remove(&id) else { return (out, None) };
        if let Some(e) = error {
            if !p.settled {
                crate::log::line("brain", format!("Codex refused her turn: {e}"));
                if self.turn_id.is_none() {
                    self.end(false, &mut out);
                }
            }
            return (out, None);
        }
        let back = match result["turn"]["id"].as_str() {
            Some(turn) if p.settled => self.stop_stray(turn),
            Some(turn) => self.named(turn, &mut out),
            None => None,
        };
        (out, back)
    }

    /// A turn that started after its stop had already failed it: stopped
    /// in Codex too, once.
    fn stop_stray(&mut self, turn: &str) -> Option<String> {
        if self.is_over(turn) {
            return None;
        }
        self.remember_over(turn.to_string());
        let id = self.id();
        Some(turn_interrupt(id, self.thread(), turn))
    }

    /// Codex named a turn: hers from now on (one she started herself
    /// starts here). A stop waiting for its id goes out now.
    fn named(&mut self, turn: &str, out: &mut Vec<Event>) -> Option<String> {
        if self.is_over(turn) || self.turn_id.as_deref().is_some_and(|t| t != turn) {
            return None;
        }
        self.begin(out);
        if self.turn_id.is_some() {
            return None;
        }
        self.turn_id = Some(turn.to_string());
        if std::mem::take(&mut self.interrupt_pending) {
            let id = self.id();
            return Some(turn_interrupt(id, self.thread(), turn));
        }
        None
    }

    fn notification(&mut self, method: &str, params: &Value) -> (Vec<Event>, Option<String>) {
        let mut out = vec![];
        // Before her thread opens, and from any other thread, news is not her turn.
        if self.phase != Phase::Open || params["threadId"].as_str() != self.thread_id.as_deref() {
            return (out, None);
        }
        let back = match method {
            "turn/started" => {
                let turn = params["turn"]["id"].as_str().unwrap_or("").to_string();
                if turn.is_empty() {
                    None
                } else if !self.in_turn && self.pending.values().any(|p| p.settled) {
                    // The turn of a start whose stop already failed it:
                    // stopped, and that start forgotten.
                    self.pending.retain(|_, p| !p.settled);
                    self.stop_stray(&turn)
                } else {
                    self.named(&turn, &mut out)
                }
            }
            "item/completed" => {
                let turn = params["turnId"].as_str().unwrap_or("");
                let item = &params["item"];
                if self.in_turn && item["type"] == "agentMessage" && !self.is_over(turn) && self.turn_id.as_deref().map_or(true, |t| t == turn) {
                    self.agent_text(item);
                }
                None
            }
            "turn/completed" => {
                let t = &params["turn"];
                let turn = t["id"].as_str().unwrap_or("");
                if self.in_turn && !self.is_over(turn) && self.turn_id.as_deref().map_or(true, |id| id == turn) {
                    if self.last_text.is_empty() && self.final_text.is_none() {
                        // No item came: the turn's own items stand in.
                        for item in t["items"].as_array().into_iter().flatten().filter(|i| i["type"] == "agentMessage") {
                            self.agent_text(item);
                        }
                    }
                    if t["status"] != "completed" {
                        let why = t["error"]["message"].as_str().unwrap_or("");
                        crate::log::line("brain", format!("her Codex turn ended {} {why}", t["status"].as_str().unwrap_or("?")));
                    }
                    self.turn_id.get_or_insert_with(|| turn.to_string());
                    self.end(t["status"] == "completed", &mut out);
                }
                None
            }
            "error" => {
                let retry = if params["willRetry"] == true { " (Codex retries)" } else { "" };
                crate::log::line("brain", format!("Codex: {}{retry}", params["error"]["message"].as_str().unwrap_or("an error")));
                None
            }
            _ => None,
        };
        (out, back)
    }

    fn agent_text(&mut self, item: &Value) {
        let text = item["text"].as_str().unwrap_or("").to_string();
        if item["phase"] == "final_answer" {
            self.final_text = Some(text.clone());
        }
        self.last_text = text;
    }

    fn fail(&mut self, why: String) -> Event {
        self.phase = Phase::Failed;
        Event::Exited(why)
    }

    fn begin(&mut self, out: &mut Vec<Event>) {
        if !self.in_turn {
            self.in_turn = true;
            self.turn_id = None;
            self.last_text.clear();
            self.final_text = None;
            self.cancel_at = None;
            self.interrupt_pending = false;
            out.push(Event::TurnStarted);
        }
    }

    fn end(&mut self, ok: bool, out: &mut Vec<Event>) {
        if !self.in_turn {
            return;
        }
        self.in_turn = false;
        self.cancel_at = None;
        self.interrupt_pending = false;
        if let Some(turn) = self.turn_id.take() {
            self.remember_over(turn);
        }
        let last = std::mem::take(&mut self.last_text);
        let text = self.final_text.take().unwrap_or(last);
        // Codex compacts itself: no context size is reported (Maya's
        // context windows are Claude's).
        out.push(if ok { Event::TurnEnded { text, context_tokens: None } } else { Event::TurnFailed { context_tokens: None } });
    }
}

impl Protocol for Parser {
    fn label(&self) -> &'static str {
        "Codex"
    }

    fn start(&mut self) -> Option<String> {
        Some(Parser::start(self))
    }

    fn line(&mut self, line: &str) -> (Vec<Event>, Option<String>) {
        Parser::line(self, line)
    }

    fn send(&mut self, text: &str) -> (Vec<Event>, Option<String>) {
        Parser::send(self, text)
    }

    /// Interrupts her turn (`turn/interrupt`); she runs no subagents.
    fn cancel(&mut self, now_ms: u64) -> (Vec<Event>, Option<String>) {
        Parser::cancel(self, now_ms)
    }

    fn tick(&mut self, now_ms: u64, _last_output_ms: u64) -> Vec<Event> {
        Parser::tick(self, now_ms)
    }

    // compact: the default, unsupported. Codex compacts itself. Its
    // `thread/compact/start {threadId}` (stable schema, empty response)
    // waits on a live check that it runs as a turn with turn/started and
    // turn/completed before Maya uses it.

    fn background_subagents(&self) -> bool {
        false
    }
}

pub type CodexDriver = LineDriver<Parser>;

/// Starts her app server; Ready comes from Codex's answer to the thread request.
pub fn spawn(l: &Launch, events: Sender<Event>) -> Result<CodexDriver, String> {
    stdio::spawn(l, command(), &[], Parser::new(l), events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Harness;
    #[cfg(unix)]
    use std::sync::atomic::Ordering;
    #[cfg(unix)]
    use std::time::{Duration, Instant};

    fn launch() -> Launch {
        Launch {
            agent: Harness::Codex,
            binary: "codex".into(),
            model: None,
            cwd: "/tmp/maya-brain".into(),
            system_prompt: "You are Maya.".into(),
            resume: None,
            writable: vec!["/Users/me/dev".into(), "/tmp".into()],
        }
    }

    /// Every line written, parsed: one JSON object per line.
    fn lines(text: &str) -> Vec<Value> {
        assert!(text.ends_with('\n'), "newline-terminated: {text:?}");
        text.lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    fn json_of(text: &str) -> Value {
        let mut v = lines(text);
        assert_eq!(v.len(), 1, "one line: {text:?}");
        v.remove(0)
    }

    /// A parser whose thread `th-1` is open, after the handshake.
    fn open() -> Parser {
        let mut p = Parser::new(&launch());
        p.start();
        let (e, w) = p.line(r#"{"id":1,"result":{"userAgent":"codex","codexHome":"/x","platformFamily":"unix","platformOs":"macos"}}"#);
        assert!(e.is_empty());
        let w = lines(&w.unwrap());
        assert_eq!(w[0]["method"], "initialized");
        assert_eq!(w[1]["method"], "thread/start");
        let (e, _) = p.line(&opened(2, "th-1"));
        assert_eq!(e, vec![Event::Ready { session_id: "th-1".into() }]);
        p
    }

    fn opened(id: u64, thread: &str) -> String {
        json!({"id": id, "result": {"thread": {"id": thread, "sessionId": thread}, "model": "gpt-5", "modelProvider": "openai", "cwd": "/tmp/maya-brain", "approvalPolicy": "on-request", "approvalsReviewer": "auto_review", "sandbox": {"type": "workspaceWrite"}}}).to_string()
    }

    fn started(id: u64, turn: &str) -> String {
        json!({"id": id, "result": {"turn": {"id": turn, "items": [], "status": "inProgress"}}}).to_string()
    }

    fn turn_started(turn: &str) -> String {
        json!({"method": "turn/started", "params": {"threadId": "th-1", "turn": {"id": turn, "items": [], "status": "inProgress"}}}).to_string()
    }

    fn agent_message(turn: &str, text: &str, phase: Option<&str>) -> String {
        let mut item = json!({"type": "agentMessage", "id": format!("m-{text}"), "text": text});
        if let Some(p) = phase {
            item["phase"] = json!(p);
        }
        json!({"method": "item/completed", "params": {"threadId": "th-1", "turnId": turn, "completedAtMs": 1, "item": item}}).to_string()
    }

    fn completed(turn: &str, status: &str) -> String {
        json!({"method": "turn/completed", "params": {"threadId": "th-1", "turn": {"id": turn, "items": [], "status": status}}}).to_string()
    }

    /// Sends `text` on an open parser; the id of the turn/start written.
    fn sent(p: &mut Parser, text: &str) -> u64 {
        let (e, w) = p.send(text);
        assert_eq!(e, vec![Event::TurnStarted]);
        let v = json_of(&w.unwrap());
        assert_eq!(v["method"], "turn/start");
        v["id"].as_u64().unwrap()
    }

    fn ended(text: &str) -> Vec<Event> {
        vec![Event::TurnEnded { text: text.into(), context_tokens: None }]
    }

    fn failed() -> Vec<Event> {
        vec![Event::TurnFailed { context_tokens: None }]
    }

    #[test]
    fn codex_reviews_its_own_approvals_and_can_write_projects_and_tmp() {
        let v = json_of(&thread_start(2, &launch()));
        assert_eq!((v["id"].as_u64(), v["method"].as_str()), (Some(2), Some("thread/start")));
        assert!(v.get("jsonrpc").is_none(), "Codex's JSON-RPC has no version field");
        let p = &v["params"];
        assert_eq!(p["cwd"], "/tmp/maya-brain");
        assert_eq!(p["approvalPolicy"], "on-request");
        assert_eq!(p["approvalsReviewer"], "auto_review");
        assert_eq!(p["sandbox"], "workspace-write");
        assert_eq!(p["developerInstructions"], "You are Maya.");
        // A dotted key merges with her own [sandbox_workspace_write] settings.
        assert_eq!(p["config"], json!({"sandbox_workspace_write.writable_roots": ["/Users/me/dev", "/tmp"]}));
        assert!(p.get("model").is_none(), "no model: Codex's default");
        let mut l = launch();
        l.model = Some("gpt-5.5".into());
        assert_eq!(json_of(&thread_start(2, &l))["params"]["model"], "gpt-5.5");
    }

    #[test]
    fn a_resume_names_the_thread_with_the_same_overrides_and_skips_its_history() {
        let mut l = launch();
        l.model = Some("gpt-5.5".into());
        let v = json_of(&thread_resume(2, "th-9", &l));
        assert_eq!(v["method"], "thread/resume");
        let p = &v["params"];
        assert_eq!(p["threadId"], "th-9");
        assert_eq!(p["excludeTurns"], true, "her history stays on disk, not in one huge line");
        let fresh = json_of(&thread_start(2, &l))["params"].clone();
        for k in ["cwd", "approvalPolicy", "approvalsReviewer", "sandbox", "developerInstructions", "config", "model"] {
            assert_eq!(p[k], fresh[k], "{k}");
        }
    }

    #[test]
    fn the_server_runs_without_codex_subagents() {
        // Codex's subagents do not wake her when they finish: she does jobs in the turn.
        assert_eq!(command(), vec!["app-server", "--disable", "multi_agent", "--disable", "multi_agent_v2"]);
    }

    #[test]
    fn the_request_lines_match_the_schema() {
        let v = json_of(&initialize(1));
        assert_eq!((v["id"].as_u64(), v["method"].as_str()), (Some(1), Some("initialize")));
        assert_eq!(v["params"]["clientInfo"]["name"], "maya");
        assert!(v["params"]["clientInfo"]["version"].is_string());
        let v = json_of(&turn_start(4, "th-1", "hi"));
        assert_eq!(v["method"], "turn/start");
        assert_eq!(v["params"]["threadId"], "th-1");
        assert_eq!(v["params"]["input"], json!([{"type": "text", "text": "hi"}]));
    }

    #[test]
    fn an_interrupt_names_the_running_turn() {
        let v = json_of(&turn_interrupt(6, "th-1", "tu-3"));
        assert_eq!((v["id"].as_u64(), v["method"].as_str()), (Some(6), Some("turn/interrupt")));
        assert_eq!(v["params"], json!({"threadId": "th-1", "turnId": "tu-3"}));
        let mut p = open();
        let id = sent(&mut p, "hi");
        p.line(&started(id, "tu-3"));
        let (e, w) = p.cancel(1_000);
        assert!(e.is_empty(), "the turn's completion ends it");
        let v = json_of(&w.unwrap());
        assert_eq!(v["method"], "turn/interrupt");
        assert_eq!(v["params"], json!({"threadId": "th-1", "turnId": "tu-3"}));
        assert_eq!(p.line(&completed("tu-3", "interrupted")).0, failed());
        assert!(p.tick(60_000).is_empty(), "nothing left to fail");
    }

    #[test]
    fn ready_names_the_thread_codex_reports() {
        let mut p = Parser::new(&launch());
        assert_eq!(json_of(&p.start())["method"], "initialize");
        let (e, w) = p.line(r#"{"id":1,"result":{"userAgent":"codex"}}"#);
        assert!(e.is_empty(), "not ready at initialize");
        let w = lines(&w.unwrap());
        assert_eq!(w.len(), 2);
        assert!(w[0].get("id").is_none(), "initialized is a notification");
        assert_eq!((w[1]["id"].as_u64(), w[1]["method"].as_str()), (Some(2), Some("thread/start")));
        assert_eq!(p.line(&opened(2, "019a-thread")).0, vec![Event::Ready { session_id: "019a-thread".into() }]);
        let mut l = launch();
        l.resume = Some("th-9".into());
        let mut p = Parser::new(&l);
        p.start();
        let w = lines(&p.line(r#"{"id":1,"result":{}}"#).1.unwrap());
        assert_eq!((w[1]["method"].as_str(), w[1]["params"]["threadId"].as_str()), (Some("thread/resume"), Some("th-9")));
        assert_eq!(p.line(&opened(2, "th-9")).0, vec![Event::Ready { session_id: "th-9".into() }]);
    }

    #[test]
    fn a_thread_that_cannot_open_ends_the_process_with_the_reason() {
        let mut l = launch();
        l.resume = Some("th-9".into());
        let mut p = Parser::new(&l);
        p.start();
        p.line(r#"{"id":1,"result":{}}"#);
        let (e, _) = p.line(r#"{"id":2,"error":{"code":-32600,"message":"no rollout found for thread id th-9"}}"#);
        assert!(matches!(&e[..], [Event::Exited(why)] if why.contains("no rollout found")), "{e:?}");
        let mut p = Parser::new(&launch());
        p.start();
        let (e, _) = p.line(r#"{"id":1,"error":{"code":-32603,"message":"Not logged in"}}"#);
        assert!(matches!(&e[..], [Event::Exited(why)] if why.contains("Not logged in")), "{e:?}");
    }

    #[test]
    fn turn_completed_ends_the_turn_with_the_agent_text() {
        let mut p = open();
        let id = sent(&mut p, "hi");
        assert!(p.line(&started(id, "tu-1")).0.is_empty());
        assert!(p.line(&turn_started("tu-1")).0.is_empty(), "the turn already started");
        assert!(p.line(&agent_message("tu-1", "Let me look.", Some("commentary"))).0.is_empty());
        p.line(&agent_message("tu-1", "Done.", Some("final_answer")));
        assert_eq!(p.line(&completed("tu-1", "completed")).0, ended("Done."));
        let id = sent(&mut p, "again");
        p.line(&started(id, "tu-2"));
        p.line(&agent_message("tu-2", "Bye", None));
        assert_eq!(p.line(&completed("tu-2", "completed")).0, ended("Bye"), "each turn's text is its own");
    }

    #[test]
    fn her_reply_is_the_final_answer_not_later_commentary() {
        let mut p = open();
        let id = sent(&mut p, "hi");
        p.line(&started(id, "tu-1"));
        p.line(&agent_message("tu-1", "The answer.", Some("final_answer")));
        p.line(&agent_message("tu-1", "Tidying up.", Some("commentary")));
        assert_eq!(p.line(&completed("tu-1", "completed")).0, ended("The answer."));
        let id = sent(&mut p, "hi");
        p.line(&started(id, "tu-2"));
        p.line(&agent_message("tu-2", "First.", Some("commentary")));
        p.line(&agent_message("tu-2", "Second.", Some("commentary")));
        assert_eq!(p.line(&completed("tu-2", "completed")).0, ended("Second."), "no final answer: the last message");
    }

    #[test]
    fn the_completed_turns_items_stand_in_when_no_item_came() {
        let mut p = open();
        let id = sent(&mut p, "hi");
        p.line(&started(id, "tu-1"));
        let done = json!({"method": "turn/completed", "params": {"threadId": "th-1", "turn": {"id": "tu-1", "status": "completed", "items": [
            {"type": "userMessage", "id": "u", "content": []},
            {"type": "agentMessage", "id": "a", "text": "From the items."}
        ]}}}).to_string();
        assert_eq!(p.line(&done).0, ended("From the items."));
    }

    #[test]
    fn a_failed_or_interrupted_turn_fails() {
        let mut p = open();
        for status in ["failed", "interrupted"] {
            let id = sent(&mut p, "hi");
            p.line(&started(id, &format!("tu-{status}")));
            p.line(&agent_message(&format!("tu-{status}"), "half", None));
            assert_eq!(p.line(&completed(&format!("tu-{status}"), status)).0, failed(), "{status}");
        }
        let id = sent(&mut p, "hi");
        let err = json!({"id": id, "error": {"code": -32603, "message": "model overloaded"}}).to_string();
        assert_eq!(p.line(&err).0, failed(), "a turn/start that is refused");
    }

    #[test]
    fn another_threads_news_is_not_her_turn() {
        let mut p = open();
        let other = json!({"method": "turn/started", "params": {"threadId": "sub-1", "turn": {"id": "x", "items": [], "status": "inProgress"}}}).to_string();
        assert!(p.line(&other).0.is_empty());
        let id = sent(&mut p, "hi");
        p.line(&started(id, "tu-1"));
        let other = json!({"method": "turn/completed", "params": {"threadId": "sub-1", "turn": {"id": "x", "items": [], "status": "completed"}}}).to_string();
        assert!(p.line(&other).0.is_empty());
        let other = json!({"method": "item/completed", "params": {"threadId": "sub-1", "turnId": "x", "completedAtMs": 1, "item": {"type": "agentMessage", "id": "m", "text": "sub"}}}).to_string();
        p.line(&other);
        p.line(&agent_message("tu-1", "mine", None));
        assert_eq!(p.line(&completed("tu-1", "completed")).0, ended("mine"));
        for noise in [r#"{"method":"thread/tokenUsage/updated","params":{"threadId":"th-1"}}"#, r#"{"method":"item/agentMessage/delta","params":{"threadId":"th-1","turnId":"tu-1","itemId":"m","delta":"x"}}"#, "not json", "42", "[1]", "true", "\"x\""] {
            assert_eq!(p.line(noise), (vec![], None), "{noise}");
        }
    }

    #[test]
    fn a_turn_she_starts_herself_runs_from_turn_started_to_turn_completed() {
        let mut p = open();
        assert_eq!(p.line(&turn_started("tu-own")).0, vec![Event::TurnStarted]);
        p.line(&agent_message("tu-own", "Hello.", None));
        assert_eq!(p.line(&completed("tu-own", "completed")).0, ended("Hello."));
        assert!(p.line(&completed("tu-own", "completed")).0.is_empty(), "ends once");
    }

    #[test]
    fn a_stop_before_the_turn_has_an_id_names_it_once_it_does() {
        let mut p = open();
        let id = sent(&mut p, "hi");
        assert_eq!(p.cancel(1_000), (vec![], None), "no turn id to name yet");
        let v = json_of(&p.line(&started(id, "tu-1")).1.expect("the interrupt goes out with the id"));
        assert_eq!((v["method"].as_str(), v["params"]["turnId"].as_str()), (Some("turn/interrupt"), Some("tu-1")));
        assert_eq!(p.line(&completed("tu-1", "interrupted")).0, failed());
        // The id may come first in turn/started.
        let id = sent(&mut p, "hi");
        p.cancel(2_000);
        let v = json_of(&p.line(&turn_started("tu-2")).1.unwrap());
        assert_eq!(v["params"]["turnId"], "tu-2");
        assert!(p.line(&started(id, "tu-2")).1.is_none(), "named once");
    }

    #[test]
    fn a_stop_with_no_completion_fails_the_turn_after_five_seconds() {
        let mut p = open();
        let id = sent(&mut p, "hi");
        p.line(&started(id, "tu-1"));
        p.cancel(1_000);
        assert!(p.tick(5_999).is_empty());
        assert_eq!(p.tick(6_000), failed());
        assert!(p.line(&completed("tu-1", "interrupted")).0.is_empty(), "the late completion adds nothing");
        assert!(p.tick(20_000).is_empty());
        let id = sent(&mut p, "next");
        p.line(&started(id, "tu-2"));
        assert_eq!(p.line(&completed("tu-2", "completed")).0, ended(""), "the next turn is its own");
    }

    #[test]
    fn a_turn_stopped_before_codex_answered_is_still_interrupted_when_it_does() {
        let mut p = open();
        let id = sent(&mut p, "hi");
        p.cancel(1_000);
        assert_eq!(p.tick(6_000), failed(), "no answer at all: failed in time");
        let v = json_of(&p.line(&started(id, "tu-1")).1.expect("the late turn is stopped"));
        assert_eq!((v["method"].as_str(), v["params"]["turnId"].as_str()), (Some("turn/interrupt"), Some("tu-1")));
        assert!(p.line(&turn_started("tu-1")).0.is_empty(), "not a turn of hers");
        assert!(p.line(&completed("tu-1", "interrupted")).0.is_empty());
    }

    #[test]
    fn cancel_with_nothing_running_sends_nothing() {
        let mut p = open();
        assert_eq!(p.cancel(1_000), (vec![], None));
        let mut p = Parser::new(&launch());
        p.start();
        assert_eq!(p.cancel(1_000), (vec![], None), "no thread yet");
    }

    #[test]
    fn a_message_before_her_thread_opens_waits_for_it() {
        let mut p = Parser::new(&launch());
        p.start();
        assert_eq!(p.send("early"), (vec![Event::TurnStarted], None), "nothing to send to yet, but her turn has started");
        p.line(r#"{"id":1,"result":{}}"#);
        let (e, w) = p.line(&opened(2, "th-1"));
        assert_eq!(e, vec![Event::Ready { session_id: "th-1".into() }]);
        let v = json_of(&w.unwrap());
        assert_eq!((v["method"].as_str(), v["params"]["threadId"].as_str()), (Some("turn/start"), Some("th-1")));
        assert_eq!(v["params"]["input"][0]["text"], "early");
    }

    #[test]
    fn a_stop_before_her_thread_opens_ends_the_turn_at_once() {
        let mut p = Parser::new(&launch());
        p.start();
        assert_eq!(p.send("early"), (vec![Event::TurnStarted], None));
        assert_eq!(p.cancel(1_000), (failed(), None), "no Codex turn to wait for");
        p.line(r#"{"id":1,"result":{}}"#);
        assert_eq!(p.line(&opened(2, "th-1")), (vec![Event::Ready { session_id: "th-1".into() }], None), "the dropped message is not sent");
    }

    fn request(id: Value, method: &str, params: Value) -> String {
        json!({"id": id, "method": method, "params": params}).to_string()
    }

    #[test]
    fn approval_requests_are_declined_never_approved() {
        let mut p = open();
        let cases = [
            ("item/commandExecution/requestApproval", json!({"threadId": "th-1", "turnId": "t", "itemId": "i", "startedAtMs": 1, "command": "rm -rf build"}), json!({"decision": "decline"})),
            ("item/fileChange/requestApproval", json!({"threadId": "th-1", "turnId": "t", "itemId": "i", "startedAtMs": 1}), json!({"decision": "decline"})),
            ("execCommandApproval", json!({"callId": "c", "command": ["rm", "-rf", "x"], "conversationId": "th-1", "cwd": "/", "parsedCmd": []}), json!({"decision": "denied"})),
            ("applyPatchApproval", json!({"callId": "c", "conversationId": "th-1", "fileChanges": {}}), json!({"decision": "denied"})),
            ("item/permissions/requestApproval", json!({"threadId": "th-1", "turnId": "t", "itemId": "i", "startedAtMs": 1, "cwd": "/", "permissions": {}}), json!({"permissions": {}, "scope": "turn"})),
            ("mcpServer/elicitation/request", json!({"threadId": "th-1"}), json!({"action": "decline"})),
        ];
        for (n, (method, params, answer)) in cases.into_iter().enumerate() {
            let (e, w) = p.line(&request(json!(40 + n), method, params));
            assert!(e.is_empty());
            let v = json_of(&w.expect(method));
            assert_eq!(v["id"], 40 + n, "{method}");
            assert_eq!(v["result"], answer, "{method}: the auto-review's no stands");
        }
    }

    #[test]
    fn a_request_she_cannot_answer_gets_an_error() {
        let mut p = open();
        for method in ["item/tool/requestUserInput", "item/tool/call", "account/chatgptAuthTokens/refresh", "something/new"] {
            let (e, w) = p.line(&request(json!("r7"), method, json!({})));
            assert!(e.is_empty());
            let v = json_of(&w.expect(method));
            assert_eq!(v["id"], "r7");
            assert_eq!(v["error"]["code"], -32601, "{method}");
            assert!(v.get("result").is_none());
        }
    }

    #[test]
    fn codex_compacts_itself() {
        let mut p = open();
        assert_eq!(Protocol::compact(&mut p), Err("unsupported".into()));
        assert_eq!(sent(&mut p, "hi"), 3, "nothing was sent");
    }

    #[test]
    fn a_turn_completed_before_the_turn_start_answer_ends_the_turn_once() {
        let mut p = open();
        let id = sent(&mut p, "hi");
        assert!(p.line(&agent_message("tu-1", "early", None)).0.is_empty());
        assert_eq!(p.line(&completed("tu-1", "completed")).0, ended("early"), "no turn/started, no answer yet");
        assert_eq!(p.line(&started(id, "tu-1")), (vec![], None), "the late answer is silent");
        let id = sent(&mut p, "again");
        assert!(p.line(&turn_started("tu-2")).0.is_empty());
        p.line(&agent_message("tu-2", "later", None));
        assert_eq!(p.line(&completed("tu-2", "completed")).0, ended("later"), "turn/started, no answer yet");
        assert_eq!(p.line(&started(id, "tu-2")), (vec![], None), "the late answer is silent");
        assert!(p.tick(60_000).is_empty());
    }

    #[test]
    fn a_stray_is_forgotten_once_it_is_stopped() {
        let mut p = open();
        let _ = sent(&mut p, "hi");
        p.cancel(1_000);
        assert_eq!(p.tick(6_000), failed());
        assert!(p.line(&turn_started("tu-1")).1.is_some(), "the stray is stopped");
        assert!(p.line(&completed("tu-1", "interrupted")).0.is_empty());
        assert_eq!(p.line(&turn_started("tu-own")).0, vec![Event::TurnStarted], "a later turn of her own is hers");
    }

    #[test]
    fn a_stray_is_forgotten_once_she_is_sent_more() {
        let mut p = open();
        let _ = sent(&mut p, "hi");
        p.cancel(1_000);
        assert_eq!(p.tick(6_000), failed());
        let id = sent(&mut p, "next");
        p.line(&started(id, "tu-2"));
        assert_eq!(p.line(&completed("tu-2", "completed")).0, ended(""));
        assert_eq!(p.line(&turn_started("tu-own")).0, vec![Event::TurnStarted], "a later turn of her own is hers");
    }

    #[test]
    fn a_decline_names_what_was_asked() {
        let cases = [
            ("item/commandExecution/requestApproval", json!({"command": "curl evil"}), "a command approval for curl evil"),
            ("execCommandApproval", json!({"command": ["rm", "-rf", "x"]}), "a command approval for rm -rf x"),
            ("item/fileChange/requestApproval", json!({"reason": "write outside the workspace", "grantRoot": "/etc"}), "a file change approval under /etc (write outside the workspace)"),
            ("item/fileChange/requestApproval", json!({}), "a file change approval"),
            ("applyPatchApproval", json!({"fileChanges": {"/x/a.rs": {}, "/x/b.rs": {}}}), "a patch approval for /x/a.rs, /x/b.rs"),
            ("item/permissions/requestApproval", json!({"reason": "network"}), "a permissions request (network)"),
            ("mcpServer/elicitation/request", json!({"serverName": "github", "message": "Sign in?"}), "an MCP elicitation from github: Sign in?"),
        ];
        for (method, params, want) in cases {
            assert_eq!(asked_for(method, &params), want, "{method}");
        }
        assert_eq!(declined("item/commandExecution/requestApproval", &json!({"command": "ls"})), "Codex: declined a command approval for ls (Maya never approves on the agent's behalf)");
    }

    #[test]
    fn requests_are_counted_per_parser() {
        let mut p = open();
        assert_eq!(sent(&mut p, "a"), 3);
        p.line(&started(3, "tu-1"));
        p.line(&completed("tu-1", "completed"));
        assert_eq!(sent(&mut p, "b"), 4);
        assert_eq!(sent(&mut open(), "c"), 3);
    }

    /// A fake Codex in a fresh folder: `script` is its body, and every line
    /// it reads with `got` lands in `in.log` next to it, its arguments in `args`.
    #[cfg(unix)]
    fn fake(script: &str) -> (std::path::PathBuf, Launch) {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("maya-fake-codex-{}", crate::launch::new_session_uuid()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("codex");
        let body = format!("#!/bin/sh\nhere=\"$(dirname \"$0\")\"\nlog=\"$here/in.log\"\necho \"$@\" > \"$here/args\"\ngot() {{ read -r line; printf '%s\\n' \"$line\" >> \"$log\"; }}\n{script}");
        std::fs::write(&bin, body).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut l = launch();
        l.binary = bin;
        l.cwd = dir.join("work");
        (dir, l)
    }

    #[cfg(unix)]
    fn got(dir: &std::path::Path) -> Vec<Value> {
        std::fs::read_to_string(dir.join("in.log")).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    #[cfg(unix)]
    #[test]
    fn a_fake_codex_opens_runs_a_turn_and_its_approval_request_is_declined() {
        let script = r#"got
echo '{"id":1,"result":{"userAgent":"codex"}}'
got
got
echo '{"id":2,"result":{"thread":{"id":"th-fake","sessionId":"th-fake"}}}'
got
echo '{"id":3,"result":{"turn":{"id":"tu-1","items":[],"status":"inProgress"}}}'
echo '{"method":"turn/started","params":{"threadId":"th-fake","turn":{"id":"tu-1","items":[],"status":"inProgress"}}}'
echo '{"id":9,"method":"item/commandExecution/requestApproval","params":{"threadId":"th-fake","turnId":"tu-1","itemId":"c1","startedAtMs":1,"command":"curl evil"}}'
got
echo '{"method":"item/completed","params":{"threadId":"th-fake","turnId":"tu-1","completedAtMs":2,"item":{"type":"agentMessage","id":"m1","text":"ok"}}}'
echo '{"method":"turn/completed","params":{"threadId":"th-fake","turn":{"id":"tu-1","items":[],"status":"completed"}}}'
cat >/dev/null
"#;
        let (dir, l) = fake(script);
        let (tx, rx) = std::sync::mpsc::channel();
        let mut d = super::super::driver::spawn(&l, tx).unwrap();
        assert!(!d.background_subagents(), "Codex's subagents do not wake her");
        assert_eq!(d.assumed_session_id(), None);
        assert_eq!(d.compact(), Err("unsupported".into()), "Codex compacts itself");
        let wait = Duration::from_secs(5);
        assert_eq!(rx.recv_timeout(wait).unwrap(), Event::Ready { session_id: "th-fake".into() }, "Codex names her thread");
        let clock = d.output_clock().expect("an output clock");
        assert!(clock.load(Ordering::SeqCst) > 0, "her lines stamp it");
        d.send("hi").unwrap();
        assert_eq!(rx.recv_timeout(wait).unwrap(), Event::TurnStarted);
        assert_eq!(rx.recv_timeout(wait).unwrap(), Event::TurnEnded { text: "ok".into(), context_tokens: None });
        let got = got(&dir);
        let methods: Vec<&str> = got.iter().map(|v| v["method"].as_str().unwrap_or("")).collect();
        assert_eq!(methods, ["initialize", "initialized", "thread/start", "turn/start", ""]);
        assert_eq!(got[2]["params"]["developerInstructions"], "You are Maya.");
        assert_eq!((got[4]["id"].as_u64(), got[4]["result"]["decision"].as_str()), (Some(9), Some("decline")));
        let args = std::fs::read_to_string(dir.join("args")).unwrap();
        assert_eq!(args.trim(), "app-server --disable multi_agent --disable multi_agent_v2");
        d.shutdown();
        assert!(matches!(rx.recv_timeout(wait).unwrap(), Event::Exited(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_stop_before_her_thread_opens_fails_the_turn_at_once() {
        let script = r#"got
echo '{"id":1,"result":{}}'
got
got
sleep 10
echo '{"id":2,"result":{"thread":{"id":"th-fake"}}}'
cat >/dev/null
"#;
        let (dir, l) = fake(script);
        let (tx, rx) = std::sync::mpsc::channel();
        let mut d = super::super::driver::spawn(&l, tx).unwrap();
        d.send("hi").unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Event::TurnStarted);
        let asked = Instant::now();
        d.interrupt().unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Event::TurnFailed { context_tokens: None });
        assert!(asked.elapsed() < Duration::from_secs(5), "before her thread opens");
        assert_eq!(rx.recv_timeout(Duration::from_secs(20)).unwrap(), Event::Ready { session_id: "th-fake".into() });
        assert!(rx.recv_timeout(Duration::from_millis(500)).is_err(), "the dropped message is not sent once she opens");
        assert!(got(&dir).iter().all(|v| v["method"] != "turn/start"));
        d.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn an_interrupt_with_no_completion_fails_the_turn_within_the_grace() {
        let script = r#"got
echo '{"id":1,"result":{}}'
got
got
echo '{"id":2,"result":{"thread":{"id":"th-fake"}}}'
got
echo '{"id":3,"result":{"turn":{"id":"tu-1","items":[],"status":"inProgress"}}}'
got
cat >/dev/null
"#;
        let (dir, l) = fake(script);
        let (tx, rx) = std::sync::mpsc::channel();
        let mut d = super::super::driver::spawn(&l, tx).unwrap();
        let wait = Duration::from_secs(5);
        assert_eq!(rx.recv_timeout(wait).unwrap(), Event::Ready { session_id: "th-fake".into() });
        d.send("hi").unwrap();
        assert_eq!(rx.recv_timeout(wait).unwrap(), Event::TurnStarted);
        // Let Codex's answer name the turn before the stop.
        let named = Instant::now() + wait;
        while got(&dir).len() < 4 && Instant::now() < named {
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(200));
        let asked = Instant::now();
        d.interrupt().unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), Event::TurnFailed { context_tokens: None });
        assert!(asked.elapsed() < Duration::from_secs(10), "within the stop grace");
        let interrupt = got(&dir).into_iter().find(|v| v["method"] == "turn/interrupt").expect("turn/interrupt went out");
        assert_eq!(interrupt["params"], json!({"threadId": "th-fake", "turnId": "tu-1"}));
        d.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
