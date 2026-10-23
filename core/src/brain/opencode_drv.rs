//! The OpenCode brain: her session is one session on the user's OpenCode
//! background server (see `crate::opencode`), driven over its HTTP API
//! rather than a process of her own.
//!
//! Her session is created (`POST /api/session`, with an allow-everything
//! ruleset: OpenCode's `--auto`) or, on a resume, confirmed
//! (`GET /api/session/{id}`, its ruleset set again); `Ready` names the id
//! the server gave, the one Maya's board keys OpenCode cards by, so the
//! board hides it. A message is a prompt queued in her session. The
//! server's event stream (`GET /api/event`, server-sent events for every
//! session) says when an execution of hers starts and ends; a succeeded one
//! ends her turn with her newest text, fetched from her messages. Her child
//! sessions (`parentID` hers) are her background tasks. A permission
//! request in her session or a child's is declined, and a form cancelled:
//! Maya never answers for her on the server. The stream is attached again
//! after a drop, a few times, before her run ends.
//!
//! The stream is read on a plain TCP connection of its own (the server is
//! `http://` on this machine), so a shutdown can end it at once. Her
//! session stays on the server when Maya shuts her down: it is her memory
//! for the next resume. The event names and payloads come from OpenCode
//! 2.1's own client code, not from a live capture.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use super::driver::{Driver, Event, Launch};
use crate::opencode::{Client, Service};

/// A stopped turn with no end from the server within this is failed.
pub const CANCEL_GRACE_MS: u64 = 5_000;
/// Her session's title on the server.
pub const TITLE: &str = "Maya";
/// How long `spawn` waits for a server it had to start.
const START_WAIT: Duration = Duration::from_secs(10);
/// The waits before each new try to attach her event stream again.
const RETRY_MS: [u64; 5] = [250, 500, 1_000, 2_000, 4_000];
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// For the stream's answer; its events then come when they come.
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);
/// How often time alone is looked at (a stop's grace).
const TICK: Duration = Duration::from_millis(200);
/// For the stops a shutdown sends: it does not wait long on the server.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
/// The messages her declined requests and cancelled forms carry.
const DECLINED: &str = "Maya runs this session unattended and never approves on the agent's behalf; continue without this action.";
const NO_FORMS: &str = "Maya cannot answer a form in this session; ask the user in your reply instead.";
/// The server's own end-of-stream failure event.
const STREAM_FAILURE: &str = "effect/http-api/stream/failure";

/// The ruleset her session is created with: everything allowed, as
/// OpenCode's own `--auto` does (a rule that denies still wins).
pub fn allow_all() -> Value {
    json!([{"action": "*", "resource": "*", "effect": "allow"}])
}

/// `POST /api/session`: her folder, her title, auto mode and her model
/// (`provider/model`, split at the first slash; one with no provider is
/// left to the server's default).
pub fn create_body(l: &Launch) -> Value {
    let mut b = json!({"location": {"directory": l.cwd.to_string_lossy()}, "title": TITLE, "permissions": allow_all()});
    if let Some((provider, id)) = l.model.as_deref().and_then(|m| m.split_once('/')).filter(|(p, m)| !p.is_empty() && !m.is_empty()) {
        b["model"] = json!({"providerID": provider, "id": id});
    }
    b
}

/// `POST /api/session/{id}/prompt`: queued behind whatever runs in her
/// session, rather than steering it.
pub fn prompt_body(text: &str) -> Value {
    json!({"text": text, "delivery": "queue"})
}

/// One server-sent event: its type (the data's own `type`, else the SSE
/// event name) and its data.
pub fn parse_event(name: Option<&str>, data: &str) -> Option<(String, Value)> {
    let v: Value = serde_json::from_str(data).ok()?;
    let kind = v["type"].as_str().or(name).filter(|k| !k.is_empty())?.to_string();
    Some((kind, v.get("data").cloned().unwrap_or_else(|| json!({}))))
}

/// Her reply, from her messages newest first: the text of her newest
/// message with any, since the newest user message (empty when the turn
/// wrote none). None when the turn's end says it failed.
pub fn reply_of(messages: &Value) -> Option<String> {
    for m in messages["data"].as_array().into_iter().flatten() {
        match m["type"].as_str() {
            Some("idle") => {
                if m["outcome"].as_str().is_some_and(|o| o != "succeeded") {
                    return None;
                }
            }
            Some("assistant") => {
                let texts: Vec<&str> = m["content"].as_array().into_iter().flatten().filter(|p| p["type"] == "text").filter_map(|p| p["text"].as_str()).map(str::trim).filter(|t| !t.is_empty()).collect();
                if !texts.is_empty() {
                    return Some(texts.join("\n\n"));
                }
            }
            Some("user") => return Some(String::new()),
            _ => {}
        }
    }
    Some(String::new())
}

/// What an event asks of the driver.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    Emit(Event),
    /// Her execution succeeded: her reply is fetched, then `TurnEnded`.
    Reply,
    /// A permission request in her session or a child's, refused.
    Decline { session: String, request: String, what: String },
    /// A form (a question for the user) in her session or a child's,
    /// cancelled: Maya answers nothing on the server.
    CancelForm { session: String, form: String },
}

/// A prompt to post; `led` when her system prompt leads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Prompt {
    pub text: String,
    led: bool,
}

/// Her turns and tasks, with no server in them: messages, stops and the
/// server's events in, events and things to do out.
pub struct Turns {
    system_prompt: String,
    /// Her system prompt, until a prompt that leads with it is accepted.
    lead: Option<String>,
    id: Option<String>,
    /// Her session exists and its event stream is attached.
    open: bool,
    /// Messages sent before it was, in order.
    waiting: Vec<String>,
    in_turn: bool,
    cancel_at: Option<u64>,
    /// The server says an execution of hers runs.
    running: bool,
    /// Her stopped execution's end, still to come, is not a turn's.
    orphan: bool,
    /// Her child sessions, and the ones running now (her tasks), in order.
    children: HashSet<String>,
    live: Vec<String>,
}

impl Turns {
    pub fn new(l: &Launch) -> Turns {
        let lead = (l.resume.is_none() && !l.system_prompt.is_empty()).then(|| l.system_prompt.clone());
        Turns { system_prompt: l.system_prompt.clone(), lead, id: None, open: false, waiting: vec![], in_turn: false, cancel_at: None, running: false, orphan: false, children: HashSet::new(), live: vec![] }
    }

    fn is_her(&self, session: &str) -> bool {
        self.id.as_deref() == Some(session)
    }

    fn is_mine(&self, session: &str) -> bool {
        self.is_her(session) || self.children.contains(session)
    }

    /// Her session's id, once the server has given it.
    pub fn session(&self) -> Option<String> {
        self.id.clone()
    }

    /// Her session exists on the server: `Ready`.
    pub fn named(&mut self, id: &str) -> Vec<Event> {
        self.id = Some(id.to_string());
        vec![Event::Ready { session_id: id.to_string() }]
    }

    /// Her event stream is attached: the messages that waited go out now.
    pub fn open(&mut self) -> (Vec<Event>, Vec<Prompt>) {
        self.open = true;
        let prompts = std::mem::take(&mut self.waiting).into_iter().map(|t| self.prompt(&t)).collect();
        (vec![], prompts)
    }

    fn prompt(&mut self, text: &str) -> Prompt {
        match self.lead.take() {
            Some(lead) => Prompt { text: format!("{lead}\n\n{text}"), led: true },
            None => Prompt { text: text.to_string(), led: false },
        }
    }

    /// A message for her: the prompt to post now, or nothing while her
    /// session is not open (it goes out once it is). Her turn starts either way.
    pub fn send(&mut self, text: &str) -> (Vec<Event>, Option<Prompt>) {
        let mut out = vec![];
        self.begin(&mut out);
        if !self.open {
            self.waiting.push(text.to_string());
            return (out, None);
        }
        (out, Some(self.prompt(text)))
    }

    /// The server refused `p`: its turn fails, and her system prompt, if
    /// it led, leads the next one.
    pub fn prompt_failed(&mut self, p: &Prompt) -> Vec<Event> {
        if p.led {
            self.lead = Some(self.system_prompt.clone());
        }
        self.end()
    }

    /// Stops her running turn and her tasks: the sessions to interrupt.
    /// Before her session is open no execution runs: what waited is
    /// dropped and her turn fails at once.
    pub fn cancel(&mut self, now_ms: u64) -> (Vec<Event>, Vec<String>) {
        if !self.open {
            self.waiting.clear();
            return (self.end(), vec![]);
        }
        let mut ids = vec![];
        if self.in_turn {
            self.cancel_at.get_or_insert(now_ms);
            ids.extend(self.id.clone());
        }
        ids.extend(self.live.iter().cloned());
        (vec![], ids)
    }

    /// What time alone ends: a stopped turn the server has not ended
    /// within `CANCEL_GRACE_MS` fails.
    pub fn tick(&mut self, now_ms: u64) -> Vec<Event> {
        if self.in_turn && self.cancel_at.is_some_and(|t| now_ms.saturating_sub(t) >= CANCEL_GRACE_MS) {
            self.orphan = self.running;
            return self.end();
        }
        vec![]
    }

    /// One event from the server, whichever session it is about.
    pub fn event(&mut self, kind: &str, data: &Value) -> Vec<Act> {
        let session = data["sessionID"].as_str().unwrap_or("").to_string();
        let mut acts = vec![];
        match kind {
            "session.created" => {
                let parent = data["parentID"].as_str().or_else(|| data["info"]["parentID"].as_str());
                if !session.is_empty() && parent.is_some_and(|p| self.is_her(p)) {
                    self.children.insert(session.clone());
                    self.task_started(&session, &mut acts);
                }
            }
            "session.deleted" => {
                if self.children.remove(&session) {
                    self.task_ended(&session, &mut acts);
                }
            }
            "session.execution.started" => {
                if self.is_her(&session) {
                    self.running = true;
                    self.orphan = false;
                    let mut out = vec![];
                    self.begin(&mut out);
                    acts.extend(out.into_iter().map(Act::Emit));
                } else if self.children.contains(&session) {
                    self.task_started(&session, &mut acts);
                }
            }
            "session.execution.succeeded" | "session.execution.failed" | "session.execution.interrupted" => {
                if self.is_her(&session) {
                    acts.extend(self.ended(kind, data));
                } else if self.children.contains(&session) {
                    self.task_ended(&session, &mut acts);
                }
            }
            "permission.asked" => {
                if self.is_mine(&session) {
                    let action = data["action"].as_str().unwrap_or("a tool");
                    let resources: Vec<&str> = data["resources"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                    let what = if resources.is_empty() { action.to_string() } else { format!("{action} ({})", resources.join(", ")) };
                    if let Some(request) = data["id"].as_str() {
                        acts.push(Act::Decline { session, request: request.to_string(), what });
                    }
                }
            }
            "form.created" => {
                let form = &data["form"];
                let session = form["sessionID"].as_str().unwrap_or("");
                if let (true, Some(id)) = (self.is_mine(session), form["id"].as_str()) {
                    acts.push(Act::CancelForm { session: session.to_string(), form: id.to_string() });
                }
            }
            _ => {}
        }
        acts
    }

    /// Her execution ended (`kind`): a stopped one's late end is dropped.
    fn ended(&mut self, kind: &str, data: &Value) -> Vec<Act> {
        self.running = false;
        if std::mem::take(&mut self.orphan) || !self.in_turn {
            return vec![];
        }
        if kind == "session.execution.succeeded" {
            self.in_turn = false;
            self.cancel_at = None;
            return vec![Act::Reply];
        }
        let why = data["error"]["message"].as_str().or_else(|| data["error"]["type"].as_str()).or_else(|| data["reason"].as_str()).unwrap_or("");
        crate::log::line("brain", format!("her OpenCode turn ended: {} {why}", kind.trim_start_matches("session.execution.")));
        self.end().into_iter().map(Act::Emit).collect()
    }

    /// The event stream was attached again: what was missed meanwhile,
    /// from the sessions running now (`GET /api/session/active`).
    pub fn resync(&mut self, active: &HashSet<String>) -> Vec<Act> {
        let mut acts = vec![];
        if !self.open {
            return acts;
        }
        for child in self.live.clone() {
            if !active.contains(&child) {
                self.task_ended(&child, &mut acts);
            }
        }
        let running = self.id.as_ref().is_some_and(|id| active.contains(id));
        if self.in_turn && !running {
            self.running = false;
            self.orphan = false;
            self.in_turn = false;
            self.cancel_at = None;
            acts.push(Act::Reply);
        } else if !self.in_turn && running && !self.orphan {
            let mut out = vec![];
            self.begin(&mut out);
            acts.extend(out.into_iter().map(Act::Emit));
        }
        self.running = running;
        acts
    }

    /// The event is about her session or one of her children: it keeps
    /// her output clock.
    pub fn concerns(&self, kind: &str, data: &Value) -> bool {
        let session = data["sessionID"].as_str().or_else(|| data["form"]["sessionID"].as_str()).unwrap_or("");
        let parent = data["parentID"].as_str().or_else(|| data["info"]["parentID"].as_str());
        (!session.is_empty() && self.is_mine(session)) || (kind == "session.created" && parent.is_some_and(|p| self.is_her(p)))
    }

    /// What a shutdown stops: her running turn and her tasks.
    pub fn shutdown_targets(&self) -> Vec<String> {
        let mut ids: Vec<String> = if self.in_turn { self.id.iter().cloned().collect() } else { vec![] };
        ids.extend(self.live.iter().cloned());
        ids
    }

    fn task_started(&mut self, child: &str, acts: &mut Vec<Act>) {
        if !self.live.iter().any(|c| c == child) {
            self.live.push(child.to_string());
            acts.push(Act::Emit(Event::TaskStarted(child.to_string())));
        }
    }

    fn task_ended(&mut self, child: &str, acts: &mut Vec<Act>) {
        if let Some(i) = self.live.iter().position(|c| c == child) {
            self.live.remove(i);
            acts.push(Act::Emit(Event::TaskEnded(child.to_string())));
        }
    }

    fn begin(&mut self, out: &mut Vec<Event>) {
        if !self.in_turn {
            self.in_turn = true;
            self.cancel_at = None;
            out.push(Event::TurnStarted);
        }
    }

    /// Her turn fails, if one runs. OpenCode compacts itself: no context
    /// size is reported (Maya's context windows are Claude's).
    fn end(&mut self) -> Vec<Event> {
        if !self.in_turn {
            return vec![];
        }
        self.in_turn = false;
        self.cancel_at = None;
        vec![Event::TurnFailed { context_tokens: None }]
    }
}

/// The body of the event stream: chunked or read to its end.
enum Body {
    Plain(BufReader<TcpStream>),
    Chunked { inner: BufReader<TcpStream>, left: usize, done: bool },
}

fn bad(what: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, what.to_string())
}

impl Read for Body {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let (inner, left, done) = match self {
            Body::Plain(r) => return r.read(buf),
            Body::Chunked { inner, left, done } => (inner, left, done),
        };
        if *done || buf.is_empty() {
            return Ok(0);
        }
        if *left == 0 {
            let mut size = String::new();
            if inner.read_line(&mut size)? == 0 {
                *done = true;
                return Ok(0);
            }
            *left = usize::from_str_radix(size.trim().split(';').next().unwrap_or("").trim(), 16).map_err(|_| bad("a bad chunk size"))?;
            if *left == 0 {
                *done = true;
                return Ok(0);
            }
        }
        let want = buf.len().min(*left);
        let n = inner.read(&mut buf[..want])?;
        if n == 0 {
            *done = true;
            return Ok(0);
        }
        *left -= n;
        if *left == 0 {
            let mut crlf = String::new();
            inner.read_line(&mut crlf)?;
        }
        Ok(n)
    }
}

/// The server's event stream, frame by frame.
struct Stream {
    lines: BufReader<Body>,
}

impl Stream {
    /// The next frame's event name and data; None once the stream ends.
    fn next(&mut self) -> Option<(Option<String>, String)> {
        let (mut name, mut data, mut has_data) = (None, String::new(), false);
        loop {
            let mut line = String::new();
            if self.lines.read_line(&mut line).ok()? == 0 {
                return None;
            }
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if has_data {
                    return Some((name, data));
                }
                name = None;
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            let (field, value) = line.split_once(':').map(|(f, v)| (f, v.strip_prefix(' ').unwrap_or(v))).unwrap_or((line, ""));
            match field {
                "event" => name = Some(value.to_string()),
                "data" => {
                    if has_data {
                        data.push('\n');
                    }
                    data.push_str(value);
                    has_data = true;
                }
                _ => {}
            }
        }
    }
}

/// Opens `GET /api/event` on its own connection; the socket comes back
/// too, for a shutdown to end the stream with.
fn connect(svc: &Service) -> Result<(Stream, TcpStream), String> {
    let rest = svc.url.strip_prefix("http://").ok_or_else(|| format!("OpenCode's server at {} is not plain http", svc.url))?;
    let (host, prefix) = match rest.split_once('/') {
        Some((h, p)) => (h, format!("/{}", p.trim_end_matches('/'))),
        None => (rest, String::new()),
    };
    let addr = host.to_socket_addrs().ok().and_then(|mut a| a.next()).ok_or_else(|| format!("OpenCode's server: cannot resolve {host}"))?;
    let tcp = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(|e| format!("OpenCode's server: {e}"))?;
    let io = |e: std::io::Error| format!("OpenCode's event stream: {e}");
    tcp.set_read_timeout(Some(HEAD_TIMEOUT)).map_err(io)?;
    (&tcp).write_all(format!("GET {prefix}/api/event HTTP/1.1\r\nHost: {host}\r\nAuthorization: {}\r\nAccept: text/event-stream\r\nCache-Control: no-cache\r\n\r\n", crate::opencode::basic_auth(&svc.password)).as_bytes()).map_err(io)?;
    let mut r = BufReader::new(tcp.try_clone().map_err(io)?);
    let mut status = String::new();
    r.read_line(&mut status).map_err(io)?;
    let code = status.split_whitespace().nth(1).unwrap_or("");
    if code != "200" {
        return Err(format!("OpenCode's event stream: HTTP {}", if code.is_empty() { "no answer" } else { code }));
    }
    let mut chunked = false;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).map_err(io)? == 0 {
            return Err("OpenCode's event stream ended before it began".into());
        }
        let h = h.trim();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            chunked |= k.trim().eq_ignore_ascii_case("transfer-encoding") && v.to_ascii_lowercase().contains("chunked");
        }
    }
    tcp.set_read_timeout(None).map_err(io)?;
    let body = if chunked { Body::Chunked { inner: r, left: 0, done: false } } else { Body::Plain(r) };
    Ok((Stream { lines: BufReader::new(body) }, tcp))
}

struct Shared {
    turns: Turns,
    events: Sender<Event>,
    /// `Exited` went out: nothing follows it.
    done: bool,
}

impl Shared {
    fn emit(&mut self, events: impl IntoIterator<Item = Event>) {
        for e in events {
            if self.done {
                return;
            }
            self.done = matches!(e, Event::Exited(_));
            let _ = self.events.send(e);
        }
    }
}

/// What her threads and the driver share.
#[derive(Clone)]
struct Ctx {
    shared: Arc<Mutex<Shared>>,
    client: Arc<Client>,
    service: Service,
    /// When the server last said something about her or her children.
    clock: Arc<AtomicU64>,
    closed: Arc<AtomicBool>,
    /// The stream's socket, for a shutdown to end it.
    socket: Arc<Mutex<Option<TcpStream>>>,
    retry_ms: &'static [u64],
}

impl Ctx {
    fn emit(&self, events: impl IntoIterator<Item = Event>) {
        self.shared.lock().unwrap().emit(events);
    }

    fn exit(&self, why: String) {
        crate::log::line("brain", format!("OpenCode: {why}"));
        self.emit([Event::Exited(why)]);
    }

    fn finished(&self) -> bool {
        self.closed.load(Ordering::SeqCst) || self.shared.lock().unwrap().done
    }

    /// Her session: created, or on a resume confirmed and set to auto mode again.
    fn open_session(&self, l: &Launch) -> Result<String, String> {
        match &l.resume {
            Some(id) => {
                let v = self.client.get(&format!("/api/session/{id}")).map_err(|e| format!("her OpenCode session {id} did not open: {e}"))?;
                let named = v["data"]["id"].as_str().ok_or_else(|| format!("her OpenCode session {id} did not open: the server named no session"))?.to_string();
                if let Err(e) = self.client.patch(&format!("/api/session/{named}"), json!({"permissions": allow_all()})) {
                    crate::log::line("brain", format!("OpenCode: could not set her session {named} to auto mode again: {e}"));
                }
                Ok(named)
            }
            None => {
                let v = self.client.post("/api/session", create_body(l)).map_err(|e| format!("her OpenCode session could not be created: {e}"))?;
                v["data"]["id"].as_str().map(str::to_string).ok_or_else(|| "her OpenCode session was created with no id".to_string())
            }
        }
    }

    /// Attaches her event stream, trying again after each of `retry_ms`;
    /// the last failure when it cannot be (or "shut down" once she is).
    fn attach(&self) -> Result<Stream, String> {
        let mut last = String::new();
        for (i, wait) in std::iter::once(0).chain(self.retry_ms.iter().copied()).enumerate() {
            let until = std::time::Instant::now() + Duration::from_millis(wait);
            while std::time::Instant::now() < until {
                if self.closed.load(Ordering::SeqCst) {
                    return Err("shut down".into());
                }
                std::thread::sleep(Duration::from_millis(20).min(until.saturating_duration_since(std::time::Instant::now())));
            }
            if self.closed.load(Ordering::SeqCst) {
                return Err("shut down".into());
            }
            match connect(&self.service) {
                Ok((stream, tcp)) => {
                    let keep = tcp.try_clone().ok();
                    *self.socket.lock().unwrap() = keep;
                    // A shutdown that came while it connected ends it now.
                    if self.closed.load(Ordering::SeqCst) {
                        let _ = tcp.shutdown(Shutdown::Both);
                        return Err("shut down".into());
                    }
                    if i > 0 {
                        crate::log::line("brain", "OpenCode: her event stream is attached again");
                    }
                    return Ok(stream);
                }
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// Posts a prompt to her session; one the server refuses fails its turn.
    fn post(&self, p: Prompt) -> Result<(), String> {
        let id = self.shared.lock().unwrap().turns.session().unwrap_or_default();
        match self.client.post(&format!("/api/session/{id}/prompt"), prompt_body(&p.text)) {
            Ok(_) => Ok(()),
            Err(e) => {
                crate::log::line("brain", format!("OpenCode refused her message: {e}"));
                let mut s = self.shared.lock().unwrap();
                let events = s.turns.prompt_failed(&p);
                s.emit(events);
                Err(e)
            }
        }
    }

    /// Does what the server's news asks, in order.
    fn act(&self, acts: Vec<Act>) {
        for a in acts {
            match a {
                Act::Emit(e) => self.emit([e]),
                Act::Reply => self.reply(),
                Act::Decline { session, request, what } => {
                    crate::log::line("brain", format!("OpenCode: declined a permission request for {what} (Maya never approves on the agent's behalf)"));
                    if let Err(e) = self.client.post(&format!("/api/session/{session}/permission/{request}/reply"), json!({"decision": "reject", "message": DECLINED})) {
                        crate::log::line("brain", format!("OpenCode: could not decline it: {e}"));
                    }
                }
                Act::CancelForm { session, form } => {
                    crate::log::line("brain", format!("OpenCode: cancelled a form in session {session}: Maya answers none"));
                    if let Err(e) = self.client.delete(&format!("/api/session/{session}/form/{form}"), &[("message", NO_FORMS)]) {
                        crate::log::line("brain", format!("OpenCode: could not cancel it: {e}"));
                    }
                }
            }
        }
    }

    /// Her turn succeeded: her reply, from her newest messages.
    fn reply(&self) {
        let id = self.shared.lock().unwrap().turns.session().unwrap_or_default();
        let event = match self.client.get(&format!("/api/session/{id}/message?limit=20&order=desc")) {
            Ok(v) => match reply_of(&v) {
                Some(text) => Event::TurnEnded { text, context_tokens: None },
                None => {
                    crate::log::line("brain", "her OpenCode turn ended in a failure");
                    Event::TurnFailed { context_tokens: None }
                }
            },
            Err(e) => {
                crate::log::line("brain", format!("OpenCode: could not read her reply: {e}"));
                Event::TurnFailed { context_tokens: None }
            }
        };
        self.emit([event]);
    }

    /// One frame from the stream. False when the server says it failed.
    fn frame(&self, name: Option<String>, data: &str) -> bool {
        if name.as_deref() == Some(STREAM_FAILURE) {
            return false;
        }
        let Some((kind, data)) = parse_event(name.as_deref(), data) else { return true };
        let rest = {
            let mut s = self.shared.lock().unwrap();
            if s.turns.concerns(&kind, &data) {
                self.clock.store(crate::now_ms(), Ordering::SeqCst);
            }
            // Events go out under the lock that changed her state, so a
            // stop or a message cannot come between; what calls the server
            // waits for the lock to go.
            let mut rest = vec![];
            for a in s.turns.event(&kind, &data) {
                match a {
                    Act::Emit(e) if rest.is_empty() => s.emit([e]),
                    a => rest.push(a),
                }
            }
            rest
        };
        self.act(rest);
        true
    }

    /// Her stream is back: what it missed, from the sessions running now.
    fn resync(&self) {
        match self.client.get("/api/session/active") {
            Ok(v) => {
                let active = crate::opencode::active_ids(&v);
                let acts = self.shared.lock().unwrap().turns.resync(&active);
                self.act(acts);
            }
            Err(e) => crate::log::line("brain", format!("OpenCode: could not see what runs after her stream came back: {e}")),
        }
    }

    /// Her run: her session, her stream, then the server's news until it
    /// ends or she is shut down.
    fn run(&self, l: Launch) {
        let id = match self.open_session(&l) {
            Ok(id) => id,
            Err(e) => return self.exit(e),
        };
        {
            let mut s = self.shared.lock().unwrap();
            let events = s.turns.named(&id);
            s.emit(events);
        }
        let mut stream = match self.attach() {
            Ok(s) => s,
            Err(_) if self.closed.load(Ordering::SeqCst) => return,
            Err(e) => return self.exit(format!("her OpenCode event stream did not open: {e}")),
        };
        let prompts = {
            let mut s = self.shared.lock().unwrap();
            let (events, prompts) = s.turns.open();
            s.emit(events);
            prompts
        };
        for p in prompts {
            let _ = self.post(p);
        }
        loop {
            let alive = match stream.next() {
                Some((name, data)) => self.frame(name, &data),
                None => false,
            };
            if alive {
                continue;
            }
            if self.finished() {
                return;
            }
            crate::log::line("brain", "OpenCode: her event stream ended; attaching it again");
            match self.attach() {
                Ok(s) => {
                    stream = s;
                    self.resync();
                }
                Err(_) if self.closed.load(Ordering::SeqCst) => return,
                Err(e) => return self.exit(format!("OpenCode's server stopped answering: {e}")),
            }
        }
    }
}

/// Her session on the OpenCode server.
pub struct OpenCodeDriver {
    ctx: Ctx,
}

impl Driver for OpenCodeDriver {
    fn send(&mut self, text: &str) -> Result<(), String> {
        let prompt = {
            let mut s = self.ctx.shared.lock().unwrap();
            if s.done {
                return Err("her OpenCode session has ended".into());
            }
            let (events, prompt) = s.turns.send(text);
            s.emit(events);
            prompt
        };
        match prompt {
            Some(p) => self.ctx.post(p),
            None => Ok(()),
        }
    }

    /// `POST /api/session/{id}/interrupt` for her running turn and for each
    /// live child. Her turn ends with the server's news of it, or fails
    /// `CANCEL_GRACE_MS` later.
    fn interrupt(&mut self) -> Result<(), String> {
        let ids = {
            let mut s = self.ctx.shared.lock().unwrap();
            let (events, ids) = s.turns.cancel(crate::now_ms());
            s.emit(events);
            ids
        };
        let mut failed = None;
        for id in ids {
            if let Err(e) = crate::opencode::interrupt(&self.ctx.client, &id) {
                crate::log::line("brain", format!("OpenCode: could not stop session {id}: {e}"));
                failed.get_or_insert(e);
            }
        }
        failed.map_or(Ok(()), Err)
    }

    /// OpenCode compacts itself.
    fn compact(&mut self) -> Result<(), String> {
        Err("unsupported".into())
    }

    fn background_subagents(&self) -> bool {
        true
    }

    fn output_clock(&self) -> Option<Arc<AtomicU64>> {
        Some(self.ctx.clock.clone())
    }

    /// Her threads stop and `Exited` goes out, last. Her running turn and
    /// tasks are stopped; her session stays on the server.
    fn shutdown(&mut self) {
        if self.ctx.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        let ids = {
            let mut s = self.ctx.shared.lock().unwrap();
            let ids = if s.done { vec![] } else { s.turns.shutdown_targets() };
            s.emit([Event::Exited("her session was shut down".into())]);
            ids
        };
        if let Some(tcp) = self.ctx.socket.lock().unwrap().take() {
            let _ = tcp.shutdown(Shutdown::Both);
        }
        if !ids.is_empty() {
            let quick = Client::with_timeout(&self.ctx.service, SHUTDOWN_TIMEOUT);
            for id in ids {
                let _ = crate::opencode::interrupt(&quick, &id);
            }
        }
    }
}

impl Drop for OpenCodeDriver {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Starts her session on the user's OpenCode server, starting the server
/// first if it is not running. An Err when it cannot be: the quick
/// interpreter answers instead.
pub fn spawn(l: &Launch, events: Sender<Event>) -> Result<OpenCodeDriver, String> {
    let home = dirs::home_dir().unwrap_or_else(|| "/".into());
    let state_file = crate::store::opencode_state_file(&home);
    let service = crate::opencode::ensure_service(&state_file, &crate::registry::pid_alive, || crate::opencode::start_service(&l.binary), START_WAIT)?;
    start(l, service, events, &RETRY_MS)
}

/// `spawn` on `service`, attaching her stream again after each of `retry_ms`.
pub(crate) fn start(l: &Launch, service: Service, events: Sender<Event>, retry_ms: &'static [u64]) -> Result<OpenCodeDriver, String> {
    std::fs::create_dir_all(&l.cwd).map_err(|e| format!("cannot create {}: {e}", l.cwd.display()))?;
    let ctx = Ctx {
        shared: Arc::new(Mutex::new(Shared { turns: Turns::new(l), events, done: false })),
        client: Arc::new(Client::new(&service)),
        service,
        clock: Arc::new(AtomicU64::new(0)),
        closed: Arc::new(AtomicBool::new(false)),
        socket: Arc::new(Mutex::new(None)),
        retry_ms,
    };
    let (runner, launch) = (ctx.clone(), l.clone());
    std::thread::spawn(move || runner.run(launch));
    let ticker = ctx.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(TICK);
        if ticker.finished() {
            return;
        }
        let mut s = ticker.shared.lock().unwrap();
        let events = s.turns.tick(crate::now_ms());
        s.emit(events);
    });
    Ok(OpenCodeDriver { ctx })
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Harness;

    const HER: &str = "ses_her";

    fn launch() -> Launch {
        Launch { agent: Harness::OpenCode, binary: "opencode".into(), model: Some("anthropic/claude-sonnet-5".into()), cwd: "/tmp/maya-brain".into(), system_prompt: "You are Maya.".into(), resume: None, writable: vec![] }
    }

    /// Turns on her open session `ses_her`, fresh.
    fn open() -> Turns {
        let mut t = Turns::new(&launch());
        assert_eq!(t.named(HER), vec![Event::Ready { session_id: HER.into() }]);
        assert_eq!(t.open(), (vec![], vec![]));
        t
    }

    fn exec(kind: &str, session: &str) -> (String, Value) {
        (format!("session.execution.{kind}"), json!({"sessionID": session}))
    }

    fn on(t: &mut Turns, (kind, data): (String, Value)) -> Vec<Act> {
        t.event(&kind, &data)
    }

    fn texts(p: &[Prompt]) -> Vec<&str> {
        p.iter().map(|p| p.text.as_str()).collect()
    }

    #[test]
    fn the_session_allows_everything_like_auto() {
        let b = create_body(&launch());
        assert_eq!(b["permissions"], json!([{"action": "*", "resource": "*", "effect": "allow"}]));
        assert_eq!(b["permissions"], allow_all());
        assert_eq!(b["location"], json!({"directory": "/tmp/maya-brain"}));
        assert_eq!(b["title"], "Maya");
        assert_eq!(b["model"], json!({"providerID": "anthropic", "id": "claude-sonnet-5"}));
    }

    #[test]
    fn a_model_is_split_at_its_first_slash_and_one_without_a_provider_is_left_to_the_server() {
        let mut l = launch();
        l.model = Some("openrouter/anthropic/claude".into());
        assert_eq!(create_body(&l)["model"], json!({"providerID": "openrouter", "id": "anthropic/claude"}));
        l.model = Some("claude".into());
        assert!(create_body(&l).get("model").is_none());
        l.model = None;
        assert!(create_body(&l).get("model").is_none());
    }

    #[test]
    fn a_prompt_queues_behind_whatever_runs() {
        assert_eq!(prompt_body("hi"), json!({"text": "hi", "delivery": "queue"}));
    }

    #[test]
    fn the_system_prompt_leads_the_first_message_of_a_fresh_session_only() {
        let mut t = open();
        let (e, p) = t.send("hi");
        assert_eq!(e, vec![Event::TurnStarted]);
        assert_eq!(p.unwrap().text, "You are Maya.\n\nhi");
        on(&mut t, exec("succeeded", HER));
        assert_eq!(t.send("again").1.unwrap().text, "again");

        let mut l = launch();
        l.resume = Some(HER.into());
        let mut t = Turns::new(&l);
        t.named(HER);
        t.open();
        assert_eq!(t.send("hi").1.unwrap().text, "hi", "a resumed session has had it");

        l.resume = None;
        l.system_prompt = String::new();
        let mut t = Turns::new(&l);
        t.named(HER);
        t.open();
        assert_eq!(t.send("hi").1.unwrap().text, "hi");
    }

    #[test]
    fn a_first_prompt_the_server_refused_fails_its_turn_and_the_next_one_leads_again() {
        let mut t = open();
        let (_, p) = t.send("hi");
        assert_eq!(t.prompt_failed(&p.unwrap()), vec![Event::TurnFailed { context_tokens: None }]);
        assert_eq!(t.send("again").1.unwrap().text, "You are Maya.\n\nagain");
    }

    #[test]
    fn messages_sent_before_her_session_opens_wait_for_it() {
        let mut t = Turns::new(&launch());
        assert_eq!(t.send("one"), (vec![Event::TurnStarted], None));
        assert_eq!(t.send("two"), (vec![], None));
        assert_eq!(t.named(HER), vec![Event::Ready { session_id: HER.into() }]);
        let (e, p) = t.send("three");
        assert_eq!((e, p), (vec![], None), "named is not open: her event stream is not attached yet");
        let (e, p) = t.open();
        assert!(e.is_empty());
        assert_eq!(texts(&p), ["You are Maya.\n\none", "two", "three"]);
    }

    #[test]
    fn a_stop_before_her_session_exists_fails_the_turn_at_once() {
        let mut t = Turns::new(&launch());
        t.send("one");
        let (e, ids) = t.cancel(1_000);
        assert_eq!(e, vec![Event::TurnFailed { context_tokens: None }]);
        assert!(ids.is_empty());
        t.named(HER);
        assert_eq!(t.open(), (vec![], vec![]), "what waited is dropped");
        assert_eq!(t.send("two").1.unwrap().text, "You are Maya.\n\ntwo", "the system prompt still leads");
    }

    #[test]
    fn a_succeeded_execution_ends_the_turn() {
        let mut t = open();
        t.send("hi");
        assert!(on(&mut t, exec("started", HER)).is_empty(), "the turn already began when sent");
        assert_eq!(on(&mut t, exec("succeeded", HER)), vec![Act::Reply]);
        assert!(on(&mut t, exec("succeeded", HER)).is_empty(), "once");
    }

    #[test]
    fn a_failed_or_interrupted_execution_fails_the_turn() {
        for kind in ["failed", "interrupted"] {
            let mut t = open();
            t.send("hi");
            on(&mut t, exec("started", HER));
            assert_eq!(on(&mut t, exec(kind, HER)), vec![Act::Emit(Event::TurnFailed { context_tokens: None })], "{kind}");
        }
    }

    #[test]
    fn another_sessions_execution_is_not_hers() {
        let mut t = open();
        t.send("hi");
        assert!(on(&mut t, exec("started", "ses_other")).is_empty());
        assert!(on(&mut t, exec("succeeded", "ses_other")).is_empty());
        assert_eq!(on(&mut t, exec("succeeded", HER)), vec![Act::Reply]);
    }

    #[test]
    fn an_execution_she_starts_herself_is_a_turn() {
        let mut t = open();
        assert_eq!(on(&mut t, exec("started", HER)), vec![Act::Emit(Event::TurnStarted)]);
        assert_eq!(on(&mut t, exec("succeeded", HER)), vec![Act::Reply]);
    }

    #[test]
    fn a_child_session_is_a_background_task() {
        let mut t = open();
        let created = json!({"sessionID": "ses_kid", "parentID": HER, "projectID": "p", "slug": "s", "version": "2"});
        assert_eq!(t.event("session.created", &created), vec![Act::Emit(Event::TaskStarted("ses_kid".into()))]);
        assert!(on(&mut t, exec("started", "ses_kid")).is_empty(), "already started");
        assert!(t.concerns("session.step.started", &json!({"sessionID": "ses_kid"})));
        assert_eq!(on(&mut t, exec("succeeded", "ses_kid")), vec![Act::Emit(Event::TaskEnded("ses_kid".into()))]);
        assert!(on(&mut t, exec("failed", "ses_kid")).is_empty(), "once");
        // Woken again by a message of hers: a task again.
        assert_eq!(on(&mut t, exec("started", "ses_kid")), vec![Act::Emit(Event::TaskStarted("ses_kid".into()))]);
        assert_eq!(on(&mut t, exec("interrupted", "ses_kid")), vec![Act::Emit(Event::TaskEnded("ses_kid".into()))]);
        // Someone else's child is not hers.
        assert!(t.event("session.created", &json!({"sessionID": "ses_x", "parentID": "ses_other"})).is_empty());
        assert!(on(&mut t, exec("started", "ses_x")).is_empty());
        assert!(!t.concerns("session.step.started", &json!({"sessionID": "ses_x"})));
        // The older shape names the parent in `info`.
        assert_eq!(t.event("session.created", &json!({"sessionID": "ses_k2", "info": {"id": "ses_k2", "parentID": HER}})), vec![Act::Emit(Event::TaskStarted("ses_k2".into()))]);
        assert_eq!(t.event("session.deleted", &json!({"sessionID": "ses_k2"})), vec![Act::Emit(Event::TaskEnded("ses_k2".into()))]);
    }

    #[test]
    fn a_stop_interrupts_her_and_each_live_child_and_fails_the_turn_if_the_server_says_nothing() {
        let mut t = open();
        t.event("session.created", &json!({"sessionID": "ses_kid", "parentID": HER}));
        t.send("hi");
        on(&mut t, exec("started", HER));
        let (e, ids) = t.cancel(1_000);
        assert!(e.is_empty());
        assert_eq!(ids, [HER, "ses_kid"]);
        assert!(t.tick(1_000 + CANCEL_GRACE_MS - 1).is_empty());
        assert_eq!(t.tick(1_000 + CANCEL_GRACE_MS), vec![Event::TurnFailed { context_tokens: None }]);
        assert!(t.tick(1_000 + CANCEL_GRACE_MS * 2).is_empty(), "once");
        // The next turn is not ended by the stopped one's late news.
        t.send("next");
        assert!(on(&mut t, exec("interrupted", HER)).is_empty(), "the stopped execution's end");
        on(&mut t, exec("started", HER));
        assert_eq!(on(&mut t, exec("succeeded", HER)), vec![Act::Reply]);
    }

    #[test]
    fn a_stop_the_server_answers_ends_the_turn_once() {
        let mut t = open();
        t.send("hi");
        on(&mut t, exec("started", HER));
        t.cancel(1_000);
        assert_eq!(on(&mut t, exec("interrupted", HER)), vec![Act::Emit(Event::TurnFailed { context_tokens: None })]);
        assert!(t.tick(1_000 + CANCEL_GRACE_MS).is_empty());
    }

    #[test]
    fn a_stop_with_no_turn_running_stops_only_the_background_tasks() {
        let mut t = open();
        assert_eq!(t.cancel(1), (vec![], vec![]));
        t.event("session.created", &json!({"sessionID": "ses_kid", "parentID": HER}));
        assert_eq!(t.cancel(1), (vec![], vec!["ses_kid".to_string()]));
        assert!(t.tick(1 + CANCEL_GRACE_MS).is_empty());
    }

    #[test]
    fn a_permission_request_from_her_or_her_child_is_declined() {
        let mut t = open();
        t.event("session.created", &json!({"sessionID": "ses_kid", "parentID": HER}));
        let asked = |s: &str| json!({"id": "per_1", "sessionID": s, "action": "bash", "resources": ["rm -rf build"]});
        assert_eq!(t.event("permission.asked", &asked(HER)), vec![Act::Decline { session: HER.into(), request: "per_1".into(), what: "bash (rm -rf build)".into() }]);
        assert_eq!(t.event("permission.asked", &asked("ses_kid")).len(), 1);
        assert!(t.event("permission.asked", &asked("ses_other")).is_empty(), "the user's own sessions are the board's");
    }

    #[test]
    fn a_form_she_raises_is_cancelled() {
        let mut t = open();
        let form = json!({"form": {"id": "frm_1", "sessionID": HER, "title": "Which?"}});
        assert_eq!(t.event("form.created", &form), vec![Act::CancelForm { session: HER.into(), form: "frm_1".into() }]);
        assert!(t.event("form.created", &json!({"form": {"id": "frm_2", "sessionID": "ses_other"}})).is_empty());
    }

    #[test]
    fn an_event_is_its_type_and_data() {
        let frame = r#"{"id":"evt_1","created":1,"type":"session.execution.succeeded","data":{"sessionID":"ses_her"}}"#;
        assert_eq!(parse_event(Some("session.execution.succeeded"), frame), Some(("session.execution.succeeded".into(), json!({"sessionID": "ses_her"}))));
        // The SSE event name stands in for a type the data lacks.
        assert_eq!(parse_event(Some("server.connected"), r#"{"data":{}}"#), Some(("server.connected".into(), json!({}))));
        assert_eq!(parse_event(None, "not json"), None);
        assert_eq!(parse_event(None, r#"{"data":{}}"#), None);
    }

    fn messages(list: Value) -> Value {
        json!({ "data": list })
    }

    #[test]
    fn her_reply_is_the_newest_text_of_the_turn() {
        let v = messages(json!([
            {"type": "idle", "outcome": "succeeded"},
            {"type": "assistant", "content": [{"type": "tool", "tool": "bash", "state": {"status": "completed"}}]},
            {"type": "assistant", "content": [{"type": "reasoning", "text": "hm"}, {"type": "text", "text": "Done."}, {"type": "text", "text": "```maya\n{}\n```"}]},
            {"type": "user", "text": "hi"},
            {"type": "assistant", "content": [{"type": "text", "text": "older"}]}
        ]));
        assert_eq!(reply_of(&v).as_deref(), Some("Done.\n\n```maya\n{}\n```"));
        let none = messages(json!([{"type": "assistant", "content": []}, {"type": "user", "text": "hi"}, {"type": "assistant", "content": [{"type": "text", "text": "older"}]}]));
        assert_eq!(reply_of(&none).as_deref(), Some(""), "no text this turn: not the last turn's");
        let failed = messages(json!([{"type": "idle", "outcome": "failed"}, {"type": "assistant", "content": [{"type": "text", "text": "partial"}]}]));
        assert_eq!(reply_of(&failed), None);
        let older_failure = messages(json!([{"type": "assistant", "content": [{"type": "text", "text": "ok"}]}, {"type": "user", "text": "x"}, {"type": "idle", "outcome": "failed"}]));
        assert_eq!(reply_of(&older_failure).as_deref(), Some("ok"));
    }

    #[test]
    fn a_reattached_stream_settles_what_it_missed() {
        let mut t = open();
        t.event("session.created", &json!({"sessionID": "ses_kid", "parentID": HER}));
        t.send("hi");
        let active: HashSet<String> = HashSet::new();
        let mut acts = t.resync(&active);
        acts.sort_by_key(|a| format!("{a:?}"));
        assert_eq!(acts, vec![Act::Emit(Event::TaskEnded("ses_kid".into())), Act::Reply]);
        let running: HashSet<String> = [HER.to_string()].into();
        assert_eq!(t.resync(&running), vec![Act::Emit(Event::TurnStarted)], "she began a turn of her own meanwhile");
        assert!(t.resync(&running).is_empty());
    }

    #[test]
    fn only_her_and_her_childrens_news_keeps_her_clock() {
        let t = open();
        assert!(t.concerns("session.text.ended", &json!({"sessionID": HER})));
        assert!(t.concerns("session.created", &json!({"sessionID": "ses_new", "parentID": HER})));
        assert!(!t.concerns("session.text.ended", &json!({"sessionID": "ses_other"})));
        assert!(!t.concerns("server.connected", &json!({})));
    }

    #[test]
    fn shutting_down_stops_her_running_turn_and_her_tasks() {
        let mut t = open();
        assert!(t.shutdown_targets().is_empty());
        t.event("session.created", &json!({"sessionID": "ses_kid", "parentID": HER}));
        t.send("hi");
        assert_eq!(t.shutdown_targets(), [HER, "ses_kid"]);
    }

    /// A fake OpenCode server: canned JSON per `"METHOD path"`, 404
    /// otherwise, and an event stream (`GET /api/event`) the test writes to.
    mod fake {
        use std::collections::HashMap;
        use std::io::{Read, Write};
        use std::net::{Shutdown, TcpListener, TcpStream};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Mutex};
        use std::time::{Duration, Instant};

        use serde_json::{json, Value};

        use crate::opencode::Service;

        #[derive(Clone, Default)]
        pub struct Fake {
            pub base: String,
            hits: Arc<Mutex<Vec<(String, String)>>>,
            routes: Arc<Mutex<HashMap<String, (u16, String, u64)>>>,
            streams: Arc<Mutex<Vec<TcpStream>>>,
            pub refuse_events: Arc<AtomicBool>,
        }

        fn read_request(s: &mut TcpStream) -> Option<(String, String)> {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            let head_end = loop {
                let n = s.read(&mut chunk).ok().filter(|n| *n > 0)?;
                buf.extend_from_slice(&chunk[..n]);
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break i + 4;
                }
            };
            let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
            let len: usize = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0))).unwrap_or(0);
            while buf.len() < head_end + len {
                let n = s.read(&mut chunk).ok().filter(|n| *n > 0)?;
                buf.extend_from_slice(&chunk[..n]);
            }
            let line = head.lines().next().unwrap_or("").to_string();
            let key = line.rsplitn(2, ' ').last().unwrap_or("").to_string();
            Some((key, String::from_utf8_lossy(&buf[head_end..]).to_string()))
        }

        impl Fake {
            pub fn start() -> Fake {
                let listener = TcpListener::bind("127.0.0.1:0").unwrap();
                let f = Fake { base: format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port()), ..Fake::default() };
                let server = f.clone();
                std::thread::spawn(move || {
                    for stream in listener.incoming() {
                        let Ok(s) = stream else { continue };
                        let f = server.clone();
                        std::thread::spawn(move || f.serve(s));
                    }
                });
                f
            }

            fn serve(&self, mut s: TcpStream) {
                let Some((key, body)) = read_request(&mut s) else { return };
                self.hits.lock().unwrap().push((key.clone(), body));
                if key == "GET /api/event" {
                    if self.refuse_events.load(Ordering::SeqCst) {
                        let _ = write!(s, "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        return;
                    }
                    let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n");
                    let _ = s.write_all(&chunk(&frame("server.connected", json!({}))));
                    self.streams.lock().unwrap().push(s);
                    return;
                }
                let (status, reply, delay) = self.routes.lock().unwrap().get(&key).cloned().unwrap_or((404, "{}".into(), 0));
                std::thread::sleep(Duration::from_millis(delay));
                let _ = write!(s, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len());
            }

            pub fn service(&self) -> Service {
                Service { url: self.base.clone(), pid: 0, password: "pw".into() }
            }

            pub fn route(&self, key: &str, reply: Value) {
                self.route_slow(key, reply, 0);
            }

            pub fn route_slow(&self, key: &str, reply: Value, delay_ms: u64) {
                self.routes.lock().unwrap().insert(key.into(), (200, reply.to_string(), delay_ms));
            }

            /// Writes one event to every open event stream.
            pub fn push(&self, kind: &str, data: Value) {
                for s in self.streams.lock().unwrap().iter_mut() {
                    let _ = s.write_all(&chunk(&frame(kind, data.clone())));
                }
            }

            /// Ends every open event stream.
            pub fn drop_streams(&self) {
                for s in self.streams.lock().unwrap().drain(..) {
                    let _ = s.shutdown(Shutdown::Both);
                }
            }

            pub fn streams(&self) -> usize {
                self.streams.lock().unwrap().len()
            }

            /// The bodies of the requests to `key` so far.
            pub fn hits(&self, key: &str) -> Vec<String> {
                self.hits.lock().unwrap().iter().filter(|(k, _)| k == key).map(|(_, b)| b.clone()).collect()
            }

            pub fn keys(&self) -> Vec<String> {
                self.hits.lock().unwrap().iter().map(|(k, _)| k.clone()).collect()
            }

            /// Waits up to 5 s for `ok`.
            pub fn wait(&self, what: &str, ok: impl Fn(&Fake) -> bool) {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !ok(self) {
                    assert!(Instant::now() < deadline, "timed out waiting for {what}; requests: {:?}", self.keys());
                    std::thread::sleep(Duration::from_millis(10));
                }
            }

            /// The first body sent to `key`, waiting for it.
            pub fn wait_hit(&self, key: &str) -> Value {
                self.wait(key, |f| !f.hits(key).is_empty());
                serde_json::from_str(&self.hits(key)[0]).unwrap_or(Value::Null)
            }
        }

        fn frame(kind: &str, data: Value) -> String {
            format!("event: {kind}\ndata: {}\n\n", json!({"id": "evt_1", "created": 1, "type": kind, "data": data}))
        }

        fn chunk(text: &str) -> Vec<u8> {
            format!("{:x}\r\n{text}\r\n", text.len()).into_bytes()
        }
    }

    mod over_http {
        use super::fake::Fake;
        use super::*;
        use crate::brain::driver::Driver;
        use std::sync::atomic::Ordering;
        use std::sync::mpsc::{channel, Receiver};
        use std::time::Duration;

        const PROMPT: &str = "POST /api/session/ses_her/prompt";
        const MESSAGES: &str = "GET /api/session/ses_her/message?limit=20&order=desc";

        fn recv(rx: &Receiver<Event>) -> Event {
            rx.recv_timeout(Duration::from_secs(5)).expect("an event")
        }

        fn launch_in(dir: &std::path::Path) -> Launch {
            let mut l = launch();
            l.cwd = dir.join("brain");
            l
        }

        fn reply(text: &str) -> Value {
            json!({"data": [{"type": "assistant", "content": [{"type": "text", "text": text}]}, {"type": "user", "text": "hi"}]})
        }

        /// A server that creates `ses_her` and takes prompts.
        fn server() -> Fake {
            let f = Fake::start();
            f.route("POST /api/session", json!({"data": {"id": "ses_her", "location": {"directory": "/x"}}}));
            f.route(PROMPT, json!({"data": {"id": "msg_1", "type": "user"}}));
            f.route("POST /api/session/ses_her/interrupt", json!({"interrupted": true}));
            f.route("POST /api/session/ses_kid/interrupt", json!({"interrupted": true}));
            f
        }

        #[test]
        fn a_fresh_session_runs_her_turns_on_the_server() {
            let dir = tempfile::tempdir().unwrap();
            let f = server();
            f.route(MESSAGES, reply("Hello."));
            let (tx, rx) = channel();
            let mut d = start(&launch_in(dir.path()), f.service(), tx, &[10]).unwrap();
            assert!(d.background_subagents());
            assert_eq!(d.compact(), Err("unsupported".to_string()));
            d.send("hi").unwrap();
            assert_eq!(recv(&rx), Event::TurnStarted);
            assert_eq!(recv(&rx), Event::Ready { session_id: "ses_her".into() });
            let created = f.wait_hit("POST /api/session");
            assert_eq!(created["permissions"], allow_all());
            assert_eq!(created["location"]["directory"], dir.path().join("brain").to_string_lossy().as_ref());
            assert!(dir.path().join("brain").is_dir(), "her folder exists for the server");
            assert_eq!(f.wait_hit(PROMPT), json!({"text": "You are Maya.\n\nhi", "delivery": "queue"}));
            let clock = d.output_clock().unwrap();
            assert_eq!(clock.load(Ordering::SeqCst), 0);
            f.push("session.execution.started", json!({"sessionID": "ses_other"}));
            f.push("session.execution.started", json!({"sessionID": "ses_her"}));
            f.push("session.execution.succeeded", json!({"sessionID": "ses_her"}));
            assert_eq!(recv(&rx), Event::TurnEnded { text: "Hello.".into(), context_tokens: None });
            assert!(clock.load(Ordering::SeqCst) > 0, "her news stamps her clock");
            d.shutdown();
            assert!(matches!(recv(&rx), Event::Exited(_)));
            d.shutdown();
            assert!(rx.recv_timeout(Duration::from_millis(200)).is_err(), "Exited once, last");
            assert!(!f.keys().iter().any(|k| k.starts_with("DELETE")), "her session is her memory: never deleted");
            assert!(f.hits("POST /api/session/ses_her/interrupt").is_empty(), "no turn ran to stop");
        }

        #[test]
        fn a_resumed_session_is_confirmed_by_the_server_and_one_it_lost_ends_her_run() {
            let dir = tempfile::tempdir().unwrap();
            let f = server();
            f.route("GET /api/session/ses_old", json!({"data": {"id": "ses_old", "location": {"directory": "/x"}}}));
            f.route("PATCH /api/session/ses_old", json!({"data": {"id": "ses_old"}}));
            let mut l = launch_in(dir.path());
            l.resume = Some("ses_old".into());
            let (tx, rx) = channel();
            let mut d = start(&l, f.service(), tx, &[10]).unwrap();
            assert_eq!(recv(&rx), Event::Ready { session_id: "ses_old".into() });
            assert_eq!(f.wait_hit("PATCH /api/session/ses_old"), json!({"permissions": allow_all()}), "auto mode again");
            assert!(f.hits("POST /api/session").is_empty());
            d.shutdown();

            l.resume = Some("ses_gone".into());
            let (tx, rx) = channel();
            let _d = start(&l, f.service(), tx, &[10]).unwrap();
            match recv(&rx) {
                Event::Exited(why) => assert!(why.contains("ses_gone"), "{why}"),
                e => panic!("{e:?}"),
            }
        }

        #[test]
        fn a_stop_interrupts_her_and_her_tasks_on_the_server() {
            let dir = tempfile::tempdir().unwrap();
            let f = server();
            let (tx, rx) = channel();
            let mut d = start(&launch_in(dir.path()), f.service(), tx, &[10]).unwrap();
            assert_eq!(recv(&rx), Event::Ready { session_id: "ses_her".into() });
            f.wait("her stream", |f| f.streams() == 1);
            f.push("session.created", json!({"sessionID": "ses_kid", "parentID": "ses_her"}));
            assert_eq!(recv(&rx), Event::TaskStarted("ses_kid".into()));
            d.send("hi").unwrap();
            assert_eq!(recv(&rx), Event::TurnStarted);
            f.wait_hit(PROMPT);
            d.interrupt().unwrap();
            f.wait_hit("POST /api/session/ses_her/interrupt");
            f.wait_hit("POST /api/session/ses_kid/interrupt");
            f.push("session.execution.interrupted", json!({"sessionID": "ses_kid", "reason": "user"}));
            assert_eq!(recv(&rx), Event::TaskEnded("ses_kid".into()));
            f.push("session.execution.interrupted", json!({"sessionID": "ses_her", "reason": "user"}));
            assert_eq!(recv(&rx), Event::TurnFailed { context_tokens: None });
        }

        #[test]
        fn a_stop_before_her_session_exists_ends_the_turn_at_once() {
            let dir = tempfile::tempdir().unwrap();
            let f = server();
            f.route_slow("POST /api/session", json!({"data": {"id": "ses_her"}}), 3000);
            let (tx, rx) = channel();
            let mut d = start(&launch_in(dir.path()), f.service(), tx, &[10]).unwrap();
            d.send("hi").unwrap();
            d.interrupt().unwrap();
            assert_eq!(recv(&rx), Event::TurnStarted);
            assert_eq!(rx.recv_timeout(Duration::from_millis(1500)), Ok(Event::TurnFailed { context_tokens: None }), "at once, before her session exists");
            assert_eq!(recv(&rx), Event::Ready { session_id: "ses_her".into() });
            f.wait("her stream", |f| f.streams() == 1);
            std::thread::sleep(Duration::from_millis(100));
            assert!(f.hits(PROMPT).is_empty(), "the stopped message never went");
        }

        #[test]
        fn her_permission_requests_are_declined_and_her_forms_cancelled() {
            let dir = tempfile::tempdir().unwrap();
            let f = server();
            f.route("POST /api/session/ses_her/permission/per_1/reply", json!({"data": true}));
            let (tx, rx) = channel();
            let _d = start(&launch_in(dir.path()), f.service(), tx, &[10]).unwrap();
            assert_eq!(recv(&rx), Event::Ready { session_id: "ses_her".into() });
            f.wait("her stream", |f| f.streams() == 1);
            f.push("permission.asked", json!({"id": "per_1", "sessionID": "ses_her", "action": "bash", "resources": ["rm -rf /"]}));
            let reply = f.wait_hit("POST /api/session/ses_her/permission/per_1/reply");
            assert_eq!(reply["decision"], "reject");
            assert!(reply["message"].as_str().unwrap().contains("never approves"));
            f.push("form.created", json!({"form": {"id": "frm_1", "sessionID": "ses_her", "title": "Which?"}}));
            f.wait("the form's cancel", |f| f.keys().iter().any(|k| k.starts_with("DELETE /api/session/ses_her/form/frm_1?message=")));
        }

        #[test]
        fn her_stream_is_attached_again_and_a_server_gone_for_good_ends_her_run() {
            let dir = tempfile::tempdir().unwrap();
            let f = server();
            f.route("GET /api/session/active", json!({"data": {}}));
            f.route(MESSAGES, reply("Back."));
            let (tx, rx) = channel();
            let _d = start(&launch_in(dir.path()), f.service(), tx, &[10, 10]).unwrap();
            assert_eq!(recv(&rx), Event::Ready { session_id: "ses_her".into() });
            f.wait("her stream", |f| f.streams() == 1);
            let mut d = _d;
            d.send("hi").unwrap();
            assert_eq!(recv(&rx), Event::TurnStarted);
            f.wait_hit(PROMPT);
            // Her turn ended while the stream was away.
            f.drop_streams();
            assert_eq!(recv(&rx), Event::TurnEnded { text: "Back.".into(), context_tokens: None });
            f.wait("her stream again", |f| f.streams() == 1);
            f.refuse_events.store(true, Ordering::SeqCst);
            f.drop_streams();
            match recv(&rx) {
                Event::Exited(why) => assert!(why.contains("OpenCode"), "{why}"),
                e => panic!("{e:?}"),
            }
        }
    }
}
