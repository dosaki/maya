//! The Grok and Kiro brains: one ACP agent kept alive on stdio, spoken to
//! in JSON-RPC 2.0, one object per line (`grok agent stdio`, `kiro-cli acp`).
//!
//! The session opens with `initialize`, then `session/new` (or
//! `session/load` to resume); her session id is the one the agent answers
//! with. A turn is one `session/prompt` request: its text streams in as
//! `session/update` notifications and the response to the request ends it.
//! A turn she starts herself (a Grok subagent woke her) has no request: it
//! ends on a turn-end update or after `QUIET_MS` of silence.

use std::collections::{HashSet, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::driver::{Driver, Event, Launch};
use crate::model::Harness;

/// A turn she started herself, with no tool running, ends after this much quiet.
pub const QUIET_MS: u64 = 2_000;
/// A cancelled turn whose prompt gets no answer within this is failed.
pub const CANCEL_GRACE_MS: u64 = 5_000;
/// A turn she started herself ends after this much quiet even with a tool
/// call open: one whose end never came would keep her working for ever.
pub const STUCK_MS: u64 = 30_000;

/// The arguments after the binary. Grok's agent options go before the mode,
/// and `--no-leader` keeps her agent in her own process (Grok's config may
/// default to a shared leader).
pub fn command(l: &Launch) -> Vec<String> {
    let s = |v: &str| v.to_string();
    let mut a = vec![];
    match l.agent {
        Harness::Kiro => {
            a.extend([s("acp"), s("--trust-all-tools")]);
            if let Some(m) = &l.model {
                a.extend([s("--model"), m.clone()]);
            }
        }
        _ => {
            a.push(s("agent"));
            if let Some(m) = &l.model {
                a.extend([s("--model"), m.clone()]);
            }
            a.extend([s("--no-leader"), s("stdio")]);
        }
    }
    a
}

fn request(id: u64, method: &str, params: Value) -> String {
    format!("{}\n", json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
}

/// The client offers no files or terminal: her agent uses its own tools.
pub fn initialize(id: u64) -> String {
    request(id, "initialize", json!({"protocolVersion": 1, "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false}, "clientInfo": {"name": "maya", "version": env!("CARGO_PKG_VERSION")}}))
}

/// Grok's session options: auto permission mode and her system prompt as rules.
fn grok_meta(l: &Launch) -> Option<Value> {
    (l.agent == Harness::Grok).then(|| json!({"autoMode": true, "rules": l.system_prompt}))
}

/// `session/new`; Grok's carries `_meta`. Kiro has no system-prompt slot:
/// its first prompt leads with it (see `Parser`).
pub fn new_session(id: u64, l: &Launch) -> String {
    let mut p = json!({"cwd": l.cwd.to_string_lossy(), "mcpServers": []});
    if let Some(m) = grok_meta(l) {
        p["_meta"] = m;
    }
    request(id, "session/new", p)
}

/// `session/load`. Grok gets its `_meta` again: whether it keeps the
/// session's own is not known, and it is harmless when it does.
pub fn load_session(id: u64, session_id: &str, l: &Launch) -> String {
    let mut p = json!({"sessionId": session_id, "cwd": l.cwd.to_string_lossy(), "mcpServers": []});
    if let Some(m) = grok_meta(l) {
        p["_meta"] = m;
    }
    request(id, "session/load", p)
}

pub fn prompt(id: u64, session_id: &str, text: &str) -> String {
    request(id, "session/prompt", json!({"sessionId": session_id, "prompt": [{"type": "text", "text": text}]}))
}

/// The `session/cancel` notification.
pub fn cancel(session_id: &str) -> String {
    format!("{}\n", json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": session_id}}))
}

fn reply(id: &Value, result: Value) -> String {
    format!("{}\n", json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn reply_error(id: &Value, code: i64, message: &str) -> String {
    format!("{}\n", json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}))
}

/// The option a permission request is answered with: `reject_once`, else
/// the first `reject_*` one. None when nothing rejects. In Grok's auto mode
/// a request means its auto-mode check declined the call, and approving it
/// would turn auto mode into approve-everything; Kiro trusts all tools and
/// should never ask.
fn reject_option(options: &Value) -> Option<&str> {
    let opts = options.as_array()?;
    let kind = |o: &&Value| o["kind"].as_str().unwrap_or("").to_string();
    opts.iter().find(|o| kind(o) == "reject_once").or_else(|| opts.iter().find(|o| kind(o).starts_with("reject"))).and_then(|o| o["optionId"].as_str())
}

/// An update kind with case and underscores folded: `TurnEnd`, `turn_end`.
fn folded(kind: &str) -> String {
    kind.to_ascii_lowercase().replace('_', "")
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    /// `initialize` is out.
    Starting,
    /// `session/new` or `session/load` is out.
    Opening,
    Open,
    /// The session could not open: the process is done for.
    Failed,
}

/// The protocol, with no process in it: lines in, events and lines to
/// write out. Requests are numbered per parser, from 1.
pub struct Parser {
    l: Launch,
    next_id: u64,
    phase: Phase,
    init_id: Option<u64>,
    open_id: Option<u64>,
    session_id: Option<String>,
    /// Kiro on a fresh session: the system prompt, leading her first message.
    lead: Option<String>,
    /// Messages sent before her session opened, in order.
    waiting: Vec<String>,
    /// Prompt requests without an answer, oldest first.
    in_flight: VecDeque<u64>,
    /// Prompts whose turn already ended: their answer adds nothing.
    settled: HashSet<u64>,
    in_turn: bool,
    text: String,
    /// A tool ran since the last text: the next text starts her reply afresh.
    text_stale: bool,
    /// Tool calls of this turn still running.
    tools: HashSet<String>,
    cancel_at: Option<u64>,
    /// A turn ended without its turn-end update (by the prompt's answer or
    /// the cancel grace): the next turn-end update, before any other update,
    /// is that turn's, late.
    stray_end: bool,
}

impl Parser {
    pub fn new(l: &Launch) -> Parser {
        Parser {
            l: l.clone(),
            next_id: 0,
            phase: Phase::Starting,
            init_id: None,
            open_id: None,
            session_id: None,
            lead: (l.agent == Harness::Kiro && l.resume.is_none() && !l.system_prompt.is_empty()).then(|| l.system_prompt.clone()),
            waiting: vec![],
            in_flight: VecDeque::new(),
            settled: HashSet::new(),
            in_turn: false,
            text: String::new(),
            text_stale: false,
            tools: HashSet::new(),
            cancel_at: None,
            stray_end: false,
        }
    }

    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn label(&self) -> &'static str {
        crate::launch::label(self.l.agent)
    }

    /// The first line to write: `initialize`.
    pub fn start(&mut self) -> String {
        let id = self.id();
        self.init_id = Some(id);
        initialize(id)
    }

    /// A message for her: the prompt to write now, or nothing while her
    /// session has not opened yet (it goes out once it has). Her turn
    /// starts either way.
    pub fn send(&mut self, text: &str) -> (Vec<Event>, Option<String>) {
        match self.phase {
            Phase::Open => {
                let mut out = vec![];
                let line = self.prompt_line(text, &mut out);
                (out, Some(line))
            }
            Phase::Starting | Phase::Opening => {
                self.waiting.push(text.to_string());
                let mut out = vec![];
                self.begin(&mut out);
                (out, None)
            }
            Phase::Failed => (vec![], None),
        }
    }

    fn prompt_line(&mut self, text: &str, out: &mut Vec<Event>) -> String {
        let id = self.id();
        let text = match self.lead.take() {
            Some(lead) => format!("{lead}\n\n{text}"),
            None => text.to_string(),
        };
        self.in_flight.push_back(id);
        self.begin(out);
        prompt(id, self.session_id.as_deref().unwrap_or(""), &text)
    }

    /// Stops the running turn: the `session/cancel` to write, or None when
    /// no turn runs. Before her session opens, the messages waiting for it
    /// are dropped and her turn fails at once: no agent turn runs to cancel.
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
        self.cancel_at = Some(now_ms);
        (out, Some(cancel(self.session_id.as_deref().unwrap_or(""))))
    }

    /// What time alone ends: a cancelled turn with no answer within
    /// `CANCEL_GRACE_MS` fails; a turn of her own ends after `QUIET_MS`
    /// with no line from her and no tool running, or after `STUCK_MS`
    /// whatever tool calls are still open.
    pub fn tick(&mut self, now_ms: u64, last_output_ms: u64) -> Vec<Event> {
        let mut out = vec![];
        if !self.in_turn {
            return out;
        }
        if self.cancel_at.is_some_and(|t| now_ms.saturating_sub(t) >= CANCEL_GRACE_MS) {
            self.settled.extend(self.in_flight.drain(..));
            self.end(false, &mut out);
            self.stray_end = true;
        } else if self.in_flight.is_empty() {
            let quiet = now_ms.saturating_sub(last_output_ms);
            if (self.tools.is_empty() && quiet >= QUIET_MS) || quiet >= STUCK_MS {
                self.end(true, &mut out);
            }
        }
        out
    }

    /// One line from her agent: the events it makes and what to write back.
    pub fn line(&mut self, line: &str) -> (Vec<Event>, Option<String>) {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { return (vec![], None) };
        let id = v.get("id").filter(|i| !i.is_null());
        match (v["method"].as_str(), id) {
            (Some(m), Some(id)) => (vec![], Some(self.request(m, id, &v["params"]))),
            (Some(m), None) => (self.notification(m, &v["params"]), None),
            (None, Some(id)) => self.response(id.as_u64(), &v),
            _ => (vec![], None),
        }
    }

    /// A request from her agent. A permission request is refused (see
    /// `reject_option`); anything else gets method-not-found, never silence.
    fn request(&mut self, method: &str, id: &Value, params: &Value) -> String {
        if method == "session/request_permission" {
            let title = params["toolCall"]["title"].as_str().unwrap_or("a tool");
            crate::log::line("brain", format!("{}: refused {title} — the auto-mode check declined it", self.label()));
            return match reject_option(&params["options"]) {
                Some(option) => reply(id, json!({"outcome": {"outcome": "selected", "optionId": option}})),
                None => reply(id, json!({"outcome": {"outcome": "cancelled"}})),
            };
        }
        crate::log::line("brain", format!("{} asked for {method}, which Maya does not handle", self.label()));
        reply_error(id, -32601, &format!("Method not found: {method}"))
    }

    fn response(&mut self, id: Option<u64>, v: &Value) -> (Vec<Event>, Option<String>) {
        let Some(id) = id else { return (vec![], None) };
        let error = v.get("error").filter(|e| !e.is_null()).map(|e| e["message"].as_str().unwrap_or("an error").to_string());
        let mut out = vec![];
        if self.init_id == Some(id) {
            self.init_id = None;
            if let Some(e) = error {
                return (vec![self.fail(format!("{} did not start: {e}", self.label()))], None);
            }
            let id = self.id();
            self.open_id = Some(id);
            self.phase = Phase::Opening;
            let line = match self.l.resume.clone() {
                Some(sid) => load_session(id, &sid, &self.l),
                None => new_session(id, &self.l),
            };
            return (out, Some(line));
        }
        if self.open_id == Some(id) {
            self.open_id = None;
            if let Some(e) = error {
                return (vec![self.fail(format!("her {} session did not open: {e}", self.label()))], None);
            }
            // A load answers with no id of its own: it confirms the one asked for.
            let Some(sid) = v["result"]["sessionId"].as_str().map(str::to_string).or_else(|| self.l.resume.clone()) else {
                return (vec![self.fail(format!("her {} session opened with no id", self.label()))], None);
            };
            self.session_id = Some(sid.clone());
            self.phase = Phase::Open;
            out.push(Event::Ready { session_id: sid });
            let mut lines = String::new();
            for text in std::mem::take(&mut self.waiting) {
                lines.push_str(&self.prompt_line(&text, &mut out));
            }
            return (out, (!lines.is_empty()).then_some(lines));
        }
        if self.settled.remove(&id) {
            return (out, None);
        }
        if let Some(pos) = self.in_flight.iter().position(|p| *p == id) {
            self.in_flight.remove(pos);
            let ok = error.is_none() && matches!(v["result"]["stopReason"].as_str(), None | Some("end_turn"));
            self.begin(&mut out);
            self.end(ok, &mut out);
            // Its turn-end update, if one comes, comes late.
            self.stray_end = true;
            if !self.in_flight.is_empty() {
                self.begin(&mut out);
            }
        }
        (out, None)
    }

    fn notification(&mut self, method: &str, params: &Value) -> Vec<Event> {
        let mut out = vec![];
        // Before her session opens (a load replays her history), and from
        // any other session (a subagent's), updates are not her turn.
        if !method.ends_with("session/update") || self.phase != Phase::Open {
            return out;
        }
        if params["sessionId"].as_str().is_some_and(|s| Some(s) != self.session_id.as_deref()) {
            return out;
        }
        let u = &params["update"];
        let standard = method == "session/update";
        let kind = folded(u["sessionUpdate"].as_str().unwrap_or(""));
        let turn_end = matches!(kind.as_str(), "turnend" | "turnended" | "turncompleted" | "endturn");
        if turn_end && std::mem::take(&mut self.stray_end) {
            // The turn already ended: its own turn end, late.
            return out;
        }
        if standard && matches!(kind.as_str(), "agentmessagechunk" | "agentthoughtchunk" | "toolcall" | "toolcallupdate") {
            // Her agent has moved on: a turn end from now on is this turn's.
            self.stray_end = false;
        }
        match kind.as_str() {
            "agentmessagechunk" if standard => {
                self.begin(&mut out);
                if std::mem::take(&mut self.text_stale) {
                    self.text.clear();
                }
                if u["content"]["type"] == "text" {
                    self.text.push_str(u["content"]["text"].as_str().unwrap_or(""));
                }
            }
            "agentthoughtchunk" if standard => self.begin(&mut out),
            "toolcall" if standard => {
                self.begin(&mut out);
                self.text_stale = true;
                self.track_tool(u);
            }
            "toolcallupdate" if standard => {
                if self.in_turn {
                    self.text_stale = true;
                }
                self.track_tool(u);
            }
            // Kiro's turn end, and Grok's (`_x.ai/session/update`). Grok's
            // ends only a turn of her own: a prompt's answer is the sure end.
            _ if turn_end && self.in_turn && (self.l.agent == Harness::Kiro || self.in_flight.is_empty()) => {
                if let Some(id) = self.in_flight.pop_front() {
                    self.settled.insert(id);
                }
                let stop = u["stop_reason"].as_str().or_else(|| u["stopReason"].as_str());
                self.end(matches!(stop, None | Some("end_turn")), &mut out);
                if !self.in_flight.is_empty() {
                    self.begin(&mut out);
                }
            }
            _ => {}
        }
        out
    }

    fn track_tool(&mut self, u: &Value) {
        let Some(id) = u["toolCallId"].as_str() else { return };
        match u["status"].as_str() {
            Some("completed") | Some("failed") => {
                self.tools.remove(id);
            }
            _ if self.in_turn => {
                self.tools.insert(id.to_string());
            }
            _ => {}
        }
    }

    fn fail(&mut self, why: String) -> Event {
        self.phase = Phase::Failed;
        Event::Exited(why)
    }

    fn begin(&mut self, out: &mut Vec<Event>) {
        if !self.in_turn {
            self.in_turn = true;
            self.text.clear();
            self.text_stale = false;
            self.tools.clear();
            self.cancel_at = None;
            out.push(Event::TurnStarted);
        }
    }

    fn end(&mut self, ok: bool, out: &mut Vec<Event>) {
        if !self.in_turn {
            return;
        }
        self.in_turn = false;
        self.cancel_at = None;
        self.tools.clear();
        self.text_stale = false;
        let text = std::mem::take(&mut self.text);
        // Her agent compacts itself: no context size is reported.
        out.push(if ok { Event::TurnEnded { text, context_tokens: None } } else { Event::TurnFailed { context_tokens: None } });
    }
}

/// What the reader, the ticker and the driver share: the protocol, her
/// stdin and the events, under one lock so lines and events keep order.
struct Shared {
    parser: Parser,
    stdin: Option<ChildStdin>,
    events: Sender<Event>,
    /// `Exited` went out: nothing follows it.
    done: bool,
}

impl Shared {
    fn write(&mut self, line: &str) -> Result<(), String> {
        let stdin = self.stdin.as_mut().ok_or("the brain is shut down")?;
        stdin.write_all(line.as_bytes()).and_then(|_| stdin.flush()).map_err(|e| format!("cannot write to the brain: {e}"))
    }

    /// Sends `events` in order; an `Exited` among them ends her session:
    /// her stdin closes and nothing more is sent.
    fn emit(&mut self, events: Vec<Event>) {
        for e in events {
            if self.done {
                return;
            }
            if matches!(e, Event::Exited(_)) {
                self.done = true;
                self.stdin = None;
            }
            let _ = self.events.send(e);
        }
    }
}

pub struct AcpDriver {
    child: Child,
    agent: Harness,
    shared: Arc<Mutex<Shared>>,
    /// When she last wrote a line: see `Driver::output_clock`.
    last_output: Arc<AtomicU64>,
}

impl AcpDriver {
    pub fn spawn(l: &Launch, events: Sender<Event>) -> Result<Self, String> {
        let label = crate::launch::label(l.agent);
        std::fs::create_dir_all(&l.cwd).map_err(|e| format!("cannot create {}: {e}", l.cwd.display()))?;
        let mut cmd = crate::command(&l.binary);
        cmd.args(command(l))
            .current_dir(&l.cwd)
            .env_clear()
            .envs(crate::launch::clean_env(std::env::vars()))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = cmd.spawn().map_err(|e| format!("cannot start {}: {e}", l.binary.display()))?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let mut parser = Parser::new(l);
        let first = parser.start();
        let shared = Arc::new(Mutex::new(Shared { parser, stdin, events, done: false }));
        // Ready comes from her agent's answer to the session request.
        if let Err(e) = shared.lock().unwrap().write(&first) {
            crate::log::line("brain", format!("{label}: {e}"));
        }
        let last_output = Arc::new(AtomicU64::new(0));
        let (stamp, reader_shared) = (last_output.clone(), shared.clone());
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                match reader.read_until(b'\n', &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                // Any line shows she is alive, whether or not it is an event.
                stamp.store(crate::now_ms(), Ordering::SeqCst);
                let line = String::from_utf8_lossy(&buf).into_owned();
                let mut s = reader_shared.lock().unwrap();
                if s.done {
                    return;
                }
                let (events, back) = s.parser.line(&line);
                if let Some(back) = back {
                    if let Err(e) = s.write(&back) {
                        crate::log::line("brain", format!("{label}: {e}"));
                    }
                }
                s.emit(events);
            }
            reader_shared.lock().unwrap().emit(vec![Event::Exited(format!("the {label} process ended"))]);
        });
        let (clock, ticker_shared) = (last_output.clone(), shared.clone());
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(200));
            let mut s = ticker_shared.lock().unwrap();
            if s.done {
                return;
            }
            let events = s.parser.tick(crate::now_ms(), clock.load(Ordering::SeqCst));
            s.emit(events);
        });
        Ok(Self { child, agent: l.agent, shared, last_output })
    }
}

impl Driver for AcpDriver {
    fn send(&mut self, text: &str) -> Result<(), String> {
        let mut s = self.shared.lock().unwrap();
        if s.done {
            return Err("her session has ended".into());
        }
        let (events, line) = s.parser.send(text);
        if let Some(line) = line {
            s.write(&line)?;
        }
        s.emit(events);
        Ok(())
    }

    /// Cancels her turn; a Grok subagent she started is left to the agent
    /// (ACP has no call to stop one).
    fn interrupt(&mut self) -> Result<(), String> {
        let mut s = self.shared.lock().unwrap();
        let (events, line) = s.parser.cancel(crate::now_ms());
        let wrote = match line {
            Some(line) => s.write(&line),
            None => Ok(()),
        };
        s.emit(events);
        wrote
    }

    fn compact(&mut self) -> Result<(), String> {
        Err("unsupported".into())
    }

    fn background_subagents(&self) -> bool {
        self.agent == Harness::Grok
    }

    fn output_clock(&self) -> Option<Arc<AtomicU64>> {
        Some(self.last_output.clone())
    }

    fn shutdown(&mut self) {
        self.shared.lock().unwrap().stdin = None;
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

/// Shutting down waits for the process (or kills and waits), so an agent
/// that exited on its own leaves no zombie behind once dropped.
impl Drop for AcpDriver {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launch(agent: Harness) -> Launch {
        Launch { agent, binary: "x".into(), model: None, cwd: "/tmp/maya-brain".into(), system_prompt: "You are Maya.".into(), resume: None, writable: vec![] }
    }

    fn json_of(line: &str) -> Value {
        assert!(line.ends_with('\n'), "one line, newline-terminated: {line:?}");
        serde_json::from_str(line.trim_end()).unwrap()
    }

    /// A parser whose session `s-1` is open, after the handshake.
    fn open(agent: Harness) -> Parser {
        let mut p = Parser::new(&launch(agent));
        p.start();
        let (e, w) = p.line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        assert!(e.is_empty());
        assert_eq!(json_of(&w.unwrap())["method"], "session/new");
        let (e, _) = p.line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s-1"}}"#);
        assert_eq!(e, vec![Event::Ready { session_id: "s-1".into() }]);
        p
    }

    fn chunk(text: &str) -> String {
        json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s-1", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}}}}).to_string()
    }

    fn update(kind: &str) -> String {
        json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s-1", "update": {"sessionUpdate": kind, "toolCallId": "t1", "status": "pending"}}}).to_string()
    }

    fn response(id: u64, stop: &str) -> String {
        json!({"jsonrpc": "2.0", "id": id, "result": {"stopReason": stop}}).to_string()
    }

    /// The id of the prompt request `send` wrote.
    fn sent(p: &mut Parser, text: &str) -> u64 {
        let (e, w) = p.send(text);
        assert_eq!(e, vec![Event::TurnStarted]);
        json_of(&w.unwrap())["id"].as_u64().unwrap()
    }

    #[test]
    fn grok_starts_its_session_in_auto_mode() {
        let v = json_of(&new_session(2, &launch(Harness::Grok)));
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["id"], 2);
        assert_eq!(v["method"], "session/new");
        assert_eq!(v["params"]["cwd"], "/tmp/maya-brain");
        assert_eq!(v["params"]["mcpServers"], json!([]));
        assert_eq!(v["params"]["_meta"]["autoMode"], true);
        assert_eq!(v["params"]["_meta"]["rules"], "You are Maya.");
    }

    #[test]
    fn kiro_starts_its_session_with_no_meta() {
        let v = json_of(&new_session(2, &launch(Harness::Kiro)));
        assert_eq!(v["method"], "session/new");
        assert!(v["params"].get("_meta").is_none(), "Kiro has no system-prompt slot: {v}");
    }

    #[test]
    fn kiro_trusts_all_tools() {
        let mut l = launch(Harness::Kiro);
        assert_eq!(command(&l), vec!["acp", "--trust-all-tools"]);
        l.model = Some("claude-sonnet-4.5".into());
        assert_eq!(command(&l), vec!["acp", "--trust-all-tools", "--model", "claude-sonnet-4.5"]);
    }

    #[test]
    fn grok_runs_its_own_agent_over_stdio_with_the_model() {
        let mut l = launch(Harness::Grok);
        assert_eq!(command(&l), vec!["agent", "--no-leader", "stdio"]);
        l.model = Some("grok-4.7".into());
        assert_eq!(command(&l), vec!["agent", "--model", "grok-4.7", "--no-leader", "stdio"], "agent options go before the mode");
    }

    #[test]
    fn the_handshake_lines_are_json_rpc_requests() {
        let v = json_of(&initialize(1));
        assert_eq!((v["id"].as_u64(), v["method"].as_str()), (Some(1), Some("initialize")));
        assert_eq!(v["params"]["protocolVersion"], 1);
        assert_eq!(v["params"]["clientCapabilities"]["terminal"], false, "her agent uses its own tools");
        let mut l = launch(Harness::Grok);
        let v = json_of(&load_session(3, "s-9", &l));
        assert_eq!(v["method"], "session/load");
        assert_eq!(v["params"]["sessionId"], "s-9");
        assert_eq!(v["params"]["cwd"], "/tmp/maya-brain");
        assert_eq!(v["params"]["_meta"]["autoMode"], true, "auto mode again on a resume");
        assert_eq!(v["params"]["_meta"]["rules"], "You are Maya.");
        l.agent = Harness::Kiro;
        assert!(json_of(&load_session(3, "s-9", &l))["params"].get("_meta").is_none());
        let v = json_of(&prompt(4, "s-9", "hi"));
        assert_eq!(v["method"], "session/prompt");
        assert_eq!(v["params"]["sessionId"], "s-9");
        assert_eq!(v["params"]["prompt"], json!([{"type": "text", "text": "hi"}]));
        let v = json_of(&cancel("s-9"));
        assert_eq!(v["method"], "session/cancel");
        assert!(v.get("id").is_none(), "a notification");
        assert_eq!(v["params"]["sessionId"], "s-9");
    }

    #[test]
    fn a_fresh_session_opens_after_initialize_and_ready_names_the_agents_id() {
        let mut p = Parser::new(&launch(Harness::Grok));
        assert_eq!(json_of(&p.start())["method"], "initialize");
        let (e, w) = p.line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true}}}"#);
        assert!(e.is_empty(), "not ready at initialize");
        let v = json_of(&w.unwrap());
        assert_eq!((v["id"].as_u64(), v["method"].as_str()), (Some(2), Some("session/new")));
        assert_eq!(p.line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"g-7"}}"#).0, vec![Event::Ready { session_id: "g-7".into() }]);
    }

    #[test]
    fn a_resumed_session_loads_and_ignores_the_replayed_history() {
        let mut l = launch(Harness::Grok);
        l.resume = Some("g-7".into());
        let mut p = Parser::new(&l);
        p.start();
        let v = json_of(&p.line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#).1.unwrap());
        assert_eq!((v["method"].as_str(), v["params"]["sessionId"].as_str()), (Some("session/load"), Some("g-7")));
        let replay = json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "g-7", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "old"}}}}).to_string();
        assert_eq!(p.line(&replay), (vec![], None), "the load replays her history: no turn");
        assert_eq!(p.line(r#"{"jsonrpc":"2.0","id":2,"result":{}}"#).0, vec![Event::Ready { session_id: "g-7".into() }], "the load's answer confirms the id");
    }

    #[test]
    fn a_session_that_cannot_open_ends_the_process_with_the_reason() {
        let mut l = launch(Harness::Kiro);
        l.resume = Some("k-1".into());
        let mut p = Parser::new(&l);
        p.start();
        p.line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        let (e, _) = p.line(r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32002,"message":"Session not found"}}"#);
        assert!(matches!(&e[..], [Event::Exited(why)] if why.contains("Session not found")), "{e:?}");
        let mut p = Parser::new(&launch(Harness::Grok));
        p.start();
        let (e, _) = p.line(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"Authentication required"}}"#);
        assert!(matches!(&e[..], [Event::Exited(why)] if why.contains("Authentication required")), "{e:?}");
    }

    #[test]
    fn a_message_before_her_session_opens_waits_for_it() {
        let mut p = Parser::new(&launch(Harness::Grok));
        p.start();
        assert_eq!(p.send("early"), (vec![Event::TurnStarted], None), "nothing to send to yet, but her turn has started");
        p.line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        let (e, w) = p.line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s-1"}}"#);
        assert_eq!(e, vec![Event::Ready { session_id: "s-1".into() }]);
        let v = json_of(&w.unwrap());
        assert_eq!((v["method"].as_str(), v["params"]["sessionId"].as_str()), (Some("session/prompt"), Some("s-1")));
        assert_eq!(v["params"]["prompt"][0]["text"], "early");
    }

    #[test]
    fn the_prompt_response_ends_the_turn_with_the_streamed_text() {
        let mut p = open(Harness::Grok);
        let id = sent(&mut p, "hi");
        assert_eq!(p.line(&chunk("Hel")), (vec![], None));
        assert_eq!(p.line(&chunk("lo")), (vec![], None));
        assert_eq!(p.line(&response(id, "end_turn")).0, vec![Event::TurnEnded { text: "Hello".into(), context_tokens: None }]);
        let id = sent(&mut p, "again");
        p.line(&chunk("Bye"));
        assert_eq!(p.line(&response(id, "end_turn")).0, vec![Event::TurnEnded { text: "Bye".into(), context_tokens: None }], "each turn's text is its own");
    }

    #[test]
    fn her_reply_is_the_text_after_her_last_tool_call() {
        let mut p = open(Harness::Grok);
        let id = sent(&mut p, "hi");
        p.line(&chunk("Let me look."));
        p.line(&update("tool_call"));
        p.line(&update("tool_call_update"));
        p.line(&chunk("Done."));
        assert_eq!(p.line(&response(id, "end_turn")).0, vec![Event::TurnEnded { text: "Done.".into(), context_tokens: None }]);
    }

    #[test]
    fn kiro_hears_the_system_prompt_first_on_a_fresh_session_only() {
        let mut p = open(Harness::Kiro);
        let (_, w) = p.send("hi");
        let text = json_of(&w.unwrap())["params"]["prompt"][0]["text"].as_str().unwrap().to_string();
        assert!(text.starts_with("You are Maya.") && text.ends_with("hi"), "{text}");
        p.line(&response(3, "end_turn"));
        let (_, w) = p.send("again");
        assert_eq!(json_of(&w.unwrap())["params"]["prompt"][0]["text"], "again", "only the first message");
        let mut l = launch(Harness::Kiro);
        l.resume = Some("k-1".into());
        let mut p = Parser::new(&l);
        p.start();
        p.line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        p.line(r#"{"jsonrpc":"2.0","id":2,"result":{}}"#);
        let (_, w) = p.send("hi");
        assert_eq!(json_of(&w.unwrap())["params"]["prompt"][0]["text"], "hi", "a resumed session already has it");
        let mut p = open(Harness::Grok);
        let (_, w) = p.send("hi");
        assert_eq!(json_of(&w.unwrap())["params"]["prompt"][0]["text"], "hi", "Grok has it as rules");
    }

    #[test]
    fn an_error_turn_fails() {
        let mut p = open(Harness::Grok);
        for stop in ["refusal", "cancelled", "max_tokens", "max_turn_requests"] {
            let id = sent(&mut p, "hi");
            p.line(&chunk("half"));
            assert_eq!(p.line(&response(id, stop)).0, vec![Event::TurnFailed { context_tokens: None }], "{stop}");
        }
        let id = sent(&mut p, "hi");
        let err = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": "Internal error"}}).to_string();
        assert_eq!(p.line(&err).0, vec![Event::TurnFailed { context_tokens: None }]);
    }

    #[test]
    fn an_unprompted_update_starts_a_turn() {
        let mut p = open(Harness::Kiro);
        assert_eq!(p.line(&chunk("A subagent finished.")).0, vec![Event::TurnStarted]);
        assert!(p.line(&chunk(" All good.")).0.is_empty());
        let end = json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s-1", "update": {"sessionUpdate": "TurnEnd"}}}).to_string();
        assert_eq!(p.line(&end).0, vec![Event::TurnEnded { text: "A subagent finished. All good.".into(), context_tokens: None }]);
        assert!(p.line(&end).0.is_empty(), "ends once");
        assert_eq!(p.line(&update("tool_call")).0, vec![Event::TurnStarted], "a tool call starts one too");
    }

    #[test]
    fn grok_ends_a_turn_of_her_own_on_its_turn_completed_update() {
        let mut p = open(Harness::Grok);
        p.line(&chunk("Woke up."));
        let done = json!({"jsonrpc": "2.0", "method": "_x.ai/session/update", "params": {"sessionId": "s-1", "update": {"sessionUpdate": "turn_completed", "stop_reason": "end_turn"}}}).to_string();
        assert_eq!(p.line(&done).0, vec![Event::TurnEnded { text: "Woke up.".into(), context_tokens: None }]);
    }

    #[test]
    fn a_late_kiro_turn_end_does_not_end_the_next_prompt() {
        let mut p = open(Harness::Kiro);
        let end = json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s-1", "update": {"sessionUpdate": "TurnEnd"}}}).to_string();
        let id = sent(&mut p, "one");
        p.line(&chunk("first"));
        assert_eq!(p.line(&response(id, "end_turn")).0, vec![Event::TurnEnded { text: "first".into(), context_tokens: None }], "no turn end seen: the answer ends it");
        let id = sent(&mut p, "two");
        assert!(p.line(&end).0.is_empty(), "the first prompt's late turn end, before anything of the second");
        p.line(&chunk("second"));
        assert_eq!(p.line(&end).0, vec![Event::TurnEnded { text: "second".into(), context_tokens: None }], "the second's own turn end");
        assert!(p.line(&response(id, "end_turn")).0.is_empty());
        // Once an update of the new prompt arrived, a turn end is its own.
        let id = sent(&mut p, "three");
        p.line(&chunk("x"));
        let _ = p.line(&response(id, "end_turn"));
        let _ = sent(&mut p, "four");
        p.line(&chunk("y"));
        assert_eq!(p.line(&end).0, vec![Event::TurnEnded { text: "y".into(), context_tokens: None }], "after an update of the new prompt, the turn end counts");
    }

    #[test]
    fn a_turn_of_her_own_with_a_stuck_tool_ends_after_a_long_quiet() {
        let mut p = open(Harness::Grok);
        p.line(&update("tool_call"));
        assert!(p.tick(38_999, 9_000).is_empty(), "29.999 s: the tool may still run");
        assert_eq!(p.tick(39_000, 9_000), vec![Event::TurnEnded { text: String::new(), context_tokens: None }]);
        let _ = sent(&mut p, "hi");
        assert!(p.tick(100_000, 9_000).is_empty(), "a prompted turn still waits for its answer");
    }

    #[test]
    fn grok_waits_for_the_prompts_answer_to_end_a_prompted_turn() {
        let mut p = open(Harness::Grok);
        let id = sent(&mut p, "hi");
        p.line(&chunk("ok"));
        let done = json!({"jsonrpc": "2.0", "method": "_x.ai/session/update", "params": {"sessionId": "s-1", "update": {"sessionUpdate": "turn_completed", "stop_reason": "end_turn"}}}).to_string();
        assert!(p.line(&done).0.is_empty());
        assert_eq!(p.line(&response(id, "end_turn")).0, vec![Event::TurnEnded { text: "ok".into(), context_tokens: None }]);
    }

    #[test]
    fn a_turn_end_update_and_the_prompt_response_end_the_turn_once() {
        let mut p = open(Harness::Kiro);
        let id = sent(&mut p, "hi");
        p.line(&chunk("ok"));
        let end = json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s-1", "update": {"sessionUpdate": "turn_end"}}}).to_string();
        assert_eq!(p.line(&end).0, vec![Event::TurnEnded { text: "ok".into(), context_tokens: None }]);
        assert!(p.line(&response(id, "end_turn")).0.is_empty(), "the response to a settled prompt adds nothing");
        let id = sent(&mut p, "again");
        assert_eq!(p.line(&response(id, "end_turn")).0, vec![Event::TurnEnded { text: String::new(), context_tokens: None }]);
    }

    #[test]
    fn noise_starts_no_turn() {
        let mut p = open(Harness::Grok);
        for kind in ["available_commands_update", "current_mode_update", "config_option_update", "session_info_update", "usage_update", "user_message_chunk"] {
            assert!(p.line(&update(kind)).0.is_empty(), "{kind}");
        }
        let hook = json!({"jsonrpc": "2.0", "method": "_x.ai/session/update", "params": {"sessionId": "s-1", "update": {"sessionUpdate": "hook_execution"}}}).to_string();
        assert!(p.line(&hook).0.is_empty());
        let other = json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "sub-1", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "sub"}}}}).to_string();
        assert!(p.line(&other).0.is_empty(), "another session's update (a subagent) is not her turn");
        assert!(p.line("not json").0.is_empty());
    }

    #[test]
    fn a_turn_of_her_own_ends_after_two_seconds_of_quiet() {
        let mut p = open(Harness::Grok);
        p.line(&chunk("Hi"));
        assert!(p.tick(10_000, 9_000).is_empty(), "one second of quiet");
        assert_eq!(p.tick(11_000, 9_000), vec![Event::TurnEnded { text: "Hi".into(), context_tokens: None }]);
        assert!(p.tick(20_000, 9_000).is_empty(), "once");
        // A prompted turn waits for its response, however quiet.
        let _ = sent(&mut p, "hi");
        assert!(p.tick(60_000, 9_000).is_empty());
    }

    #[test]
    fn quiet_does_not_end_her_turn_while_a_tool_runs() {
        let mut p = open(Harness::Grok);
        p.line(&update("tool_call"));
        assert!(p.tick(11_000, 9_000).is_empty(), "the tool is still running");
        let done = json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s-1", "update": {"sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "completed"}}}).to_string();
        p.line(&done);
        assert_eq!(p.tick(11_000, 9_000), vec![Event::TurnEnded { text: String::new(), context_tokens: None }]);
    }

    #[test]
    fn cancel_sends_session_cancel_and_the_cancelled_response_fails_the_turn() {
        let mut p = open(Harness::Grok);
        let id = sent(&mut p, "hi");
        let (e, w) = p.cancel(1_000);
        assert!(e.is_empty(), "the answer ends it");
        let v = json_of(&w.unwrap());
        assert_eq!((v["method"].as_str(), v["params"]["sessionId"].as_str()), (Some("session/cancel"), Some("s-1")));
        assert_eq!(p.line(&response(id, "cancelled")).0, vec![Event::TurnFailed { context_tokens: None }]);
        assert!(p.tick(60_000, 0).is_empty(), "nothing left to fail");
    }

    #[test]
    fn a_cancel_with_no_answer_fails_the_turn_after_five_seconds() {
        let mut p = open(Harness::Grok);
        let id = sent(&mut p, "hi");
        p.cancel(1_000);
        assert!(p.tick(5_000, 4_900).is_empty());
        assert_eq!(p.tick(6_000, 4_900), vec![Event::TurnFailed { context_tokens: None }]);
        assert!(p.line(&response(id, "cancelled")).0.is_empty(), "the late answer adds nothing");
        assert!(p.tick(20_000, 4_900).is_empty());
    }

    #[test]
    fn cancel_with_nothing_running_sends_nothing() {
        let mut p = open(Harness::Grok);
        assert_eq!(p.cancel(1_000), (vec![], None));
        let mut p = Parser::new(&launch(Harness::Grok));
        p.start();
        assert_eq!(p.cancel(1_000), (vec![], None), "no session yet");
    }

    #[test]
    fn a_cancel_drops_messages_still_waiting_for_her_session() {
        let mut p = Parser::new(&launch(Harness::Grok));
        p.start();
        assert_eq!(p.send("early"), (vec![Event::TurnStarted], None), "her turn starts when it is sent");
        assert_eq!(p.cancel(1_000), (vec![Event::TurnFailed { context_tokens: None }], None), "a stop before her session opens ends it at once");
        p.line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        assert_eq!(p.line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s-1"}}"#), (vec![Event::Ready { session_id: "s-1".into() }], None));
    }

    fn permission(options: Value) -> String {
        json!({"jsonrpc": "2.0", "id": 41, "method": "session/request_permission", "params": {"sessionId": "s-1", "toolCall": {"toolCallId": "t1", "title": "rm -rf build"}, "options": options}}).to_string()
    }

    #[test]
    fn a_permission_request_is_answered_reject_once() {
        let mut p = open(Harness::Grok);
        let opts = json!([
            {"optionId": "always", "kind": "allow_always", "name": "Always"},
            {"optionId": "once", "kind": "allow_once", "name": "Allow"},
            {"optionId": "never", "kind": "reject_always", "name": "Never"},
            {"optionId": "no", "kind": "reject_once", "name": "Reject"}
        ]);
        let (e, w) = p.line(&permission(opts));
        assert!(e.is_empty());
        let v = json_of(&w.unwrap());
        assert_eq!(v["id"], 41);
        assert_eq!(v["result"]["outcome"], json!({"outcome": "selected", "optionId": "no"}), "auto mode's check declined it: never approved behind its back");
        let (_, w) = p.line(&permission(json!([{"optionId": "yes", "kind": "allow_once"}, {"optionId": "never", "kind": "reject_always"}])));
        assert_eq!(json_of(&w.unwrap())["result"]["outcome"]["optionId"], "never", "no reject_once: the first reject option");
        let (_, w) = p.line(&permission(json!([{"optionId": "yes", "kind": "allow_once"}])));
        assert_eq!(json_of(&w.unwrap())["result"]["outcome"], json!({"outcome": "cancelled"}), "nothing to reject with");
        let mut p = open(Harness::Kiro);
        let (_, w) = p.line(&permission(json!([{"optionId": "yes", "kind": "allow_once"}, {"optionId": "no", "kind": "reject_once"}])));
        assert_eq!(json_of(&w.unwrap())["result"]["outcome"]["optionId"], "no", "Kiro trusts all tools; if it asks anyway, the same rule");
    }

    #[test]
    fn a_request_she_cannot_answer_gets_method_not_found() {
        let mut p = open(Harness::Grok);
        let (e, w) = p.line(r#"{"jsonrpc":"2.0","id":"r7","method":"fs/read_text_file","params":{"path":"/x"}}"#);
        assert!(e.is_empty());
        let v = json_of(&w.unwrap());
        assert_eq!(v["id"], "r7");
        assert_eq!(v["error"]["code"], -32601);
        assert!(p.line(r#"{"jsonrpc":"2.0","method":"_kiro.dev/metadata","params":{}}"#).1.is_none(), "a notification gets no answer");
    }

    #[test]
    fn requests_are_counted_per_parser() {
        let mut p = open(Harness::Grok);
        assert_eq!(sent(&mut p, "a"), 3);
        p.line(&response(3, "end_turn"));
        assert_eq!(sent(&mut p, "b"), 4);
        assert_eq!(sent(&mut Parser::new(&launch(Harness::Grok)).tap_open(), "c"), 3);
    }

    /// A fake agent in a fresh folder: `script` is its body, and every line
    /// it reads with `got` lands in `in.log` next to it.
    #[cfg(unix)]
    fn fake(agent: Harness, script: &str) -> (std::path::PathBuf, Launch) {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("maya-fake-acp-{}", crate::launch::new_session_uuid()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("agent");
        let body = format!("#!/bin/sh\nlog=\"$(dirname \"$0\")/in.log\"\ngot() {{ read -r line; printf '%s\\n' \"$line\" >> \"$log\"; }}\n{script}");
        std::fs::write(&bin, body).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut l = launch(agent);
        l.binary = bin;
        l.cwd = dir.join("work");
        (dir, l)
    }

    #[cfg(unix)]
    #[test]
    fn a_fake_acp_agent_opens_runs_a_turn_and_its_permission_request_is_refused() {
        let script = r#"got
echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}'
got
echo '{"jsonrpc":"2.0","id":2,"result":{"sessionId":"acp-1"}}'
got
echo '{"jsonrpc":"2.0","id":9,"method":"session/request_permission","params":{"sessionId":"acp-1","toolCall":{"title":"ls"},"options":[{"optionId":"no","kind":"reject_once"},{"optionId":"ok-once","kind":"allow_once"}]}}'
got
echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"acp-1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"ok"}}}}'
echo '{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}'
cat >/dev/null
"#;
        let (dir, l) = fake(Harness::Kiro, script);
        let (tx, rx) = std::sync::mpsc::channel();
        let mut d = super::super::driver::spawn(&l, tx).unwrap();
        assert!(!d.background_subagents(), "Kiro's subagents block");
        assert_eq!(d.assumed_session_id(), None);
        assert_eq!(d.compact(), Err("unsupported".into()));
        let wait = Duration::from_secs(5);
        assert_eq!(rx.recv_timeout(wait).unwrap(), Event::Ready { session_id: "acp-1".into() }, "her agent names her session");
        let clock = d.output_clock().expect("an output clock");
        assert!(clock.load(Ordering::SeqCst) > 0, "her lines stamp it");
        d.send("hi").unwrap();
        assert_eq!(rx.recv_timeout(wait).unwrap(), Event::TurnStarted);
        assert_eq!(rx.recv_timeout(wait).unwrap(), Event::TurnEnded { text: "ok".into(), context_tokens: None });
        let got: Vec<Value> = std::fs::read_to_string(dir.join("in.log")).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        let methods: Vec<&str> = got.iter().map(|v| v["method"].as_str().unwrap_or("")).collect();
        assert_eq!(methods, ["initialize", "session/new", "session/prompt", ""]);
        let first = got[2]["params"]["prompt"][0]["text"].as_str().unwrap();
        assert!(first.starts_with("You are Maya.") && first.ends_with("hi"), "{first}");
        assert_eq!((got[3]["id"].as_u64(), got[3]["result"]["outcome"]["optionId"].as_str()), (Some(9), Some("no")));
        d.shutdown();
        assert!(matches!(rx.recv_timeout(wait).unwrap(), Event::Exited(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn an_agent_that_cannot_start_exits_with_the_reason_and_is_reaped() {
        let script = r#"got
echo '{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"Authentication required"}}'
cat >/dev/null
"#;
        let (dir, l) = fake(Harness::Grok, script);
        let (tx, rx) = std::sync::mpsc::channel();
        let d = AcpDriver::spawn(&l, tx).unwrap();
        assert!(d.background_subagents(), "Grok's subagents run in the background");
        let pid = d.child.id().to_string();
        match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
            Event::Exited(why) => assert!(why.contains("Authentication required"), "{why}"),
            e => panic!("{e:?}"),
        }
        assert!(rx.recv_timeout(Duration::from_millis(500)).is_err(), "Exited is the last event");
        drop(d);
        let alive = std::process::Command::new("kill").args(["-0", &pid]).stderr(Stdio::null()).status().unwrap().success();
        assert!(!alive, "dropping the driver reaps it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_stop_before_her_session_opens_fails_the_turn_at_once() {
        let script = r#"got
echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}'
got
sleep 3
echo '{"jsonrpc":"2.0","id":2,"result":{"sessionId":"acp-1"}}'
cat >/dev/null
"#;
        let (dir, l) = fake(Harness::Grok, script);
        let (tx, rx) = std::sync::mpsc::channel();
        let mut d = super::super::driver::spawn(&l, tx).unwrap();
        d.send("hi").unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(1)).unwrap(), Event::TurnStarted);
        let asked = Instant::now();
        d.interrupt().unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(1)).unwrap(), Event::TurnFailed { context_tokens: None });
        assert!(asked.elapsed() < Duration::from_secs(1), "before her session opens");
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Event::Ready { session_id: "acp-1".into() });
        assert!(rx.recv_timeout(Duration::from_millis(500)).is_err(), "the dropped message is not sent once she opens");
        let got = std::fs::read_to_string(dir.join("in.log")).unwrap();
        assert!(!got.contains("session/prompt"), "{got}");
        d.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn an_interrupt_with_no_answer_fails_the_turn() {
        let script = r#"got
echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}'
got
echo '{"jsonrpc":"2.0","id":2,"result":{"sessionId":"acp-1"}}'
cat >/dev/null
"#;
        let (dir, l) = fake(Harness::Grok, script);
        let (tx, rx) = std::sync::mpsc::channel();
        let mut d = super::super::driver::spawn(&l, tx).unwrap();
        let wait = Duration::from_secs(5);
        assert_eq!(rx.recv_timeout(wait).unwrap(), Event::Ready { session_id: "acp-1".into() });
        d.send("hi").unwrap();
        assert_eq!(rx.recv_timeout(wait).unwrap(), Event::TurnStarted);
        let asked = Instant::now();
        d.interrupt().unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), Event::TurnFailed { context_tokens: None });
        assert!(asked.elapsed() < Duration::from_secs(10), "within the stop grace");
        d.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    impl Parser {
        fn tap_open(mut self) -> Parser {
            self.start();
            self.line(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#);
            self.line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s-1"}}"#);
            self
        }
    }
}
