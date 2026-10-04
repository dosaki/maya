//! OpenCode sessions live in its background server, one per user, which
//! owns every session, permission and tool run; the terminal UI is a client.
//! Maya reads the server's state file for its url and password and talks
//! to it over HTTP: the board polls it on each refresh, and every control
//! is a call rather than keys typed into a terminal.
//!
//! `fixtures/opencode/permission.json` and `form.json` follow the server's
//! OpenAPI schema rather than a captured reply: OpenCode's default rules
//! allowed every tool the probe tried, so no pending request could be
//! provoked.

use crate::model::{AwaitKind, Awaiting, Card, Choice, Harness, Question, State};
use crate::state::{truncate, SNIPPET_CHARS};
use crate::transcript::{Turn, TurnKind};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The background server, from its state file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    pub url: String,
    pub pid: i32,
    pub password: String,
}

/// `<state dir>/opencode/service.json`.
pub fn service_file(state_dir: &Path) -> PathBuf {
    state_dir.join("opencode/service.json")
}

pub fn read_service(path: &Path) -> Option<Service> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    Some(Service { url: v["url"].as_str()?.trim_end_matches('/').to_string(), pid: v["pid"].as_i64()? as i32, password: v["password"].as_str()?.to_string() })
}

/// HTTP to the server, with its password as basic auth on every request.
pub struct Client {
    agent: ureq::Agent,
    base: String,
    password: String,
}

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

impl Client {
    pub fn new(s: &Service) -> Self {
        let config = ureq::Agent::config_builder().timeout_global(Some(REQUEST_TIMEOUT)).http_status_as_error(true).build();
        Client { agent: ureq::Agent::new_with_config(config), base: s.url.clone(), password: s.password.clone() }
    }

    fn auth(&self) -> String {
        use base64::Engine;
        format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("opencode:{}", self.password)))
    }

    fn read(r: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Value, String> {
        match r {
            Ok(mut resp) => resp.body_mut().read_json::<Value>().map_err(|e| format!("OpenCode's server: unreadable reply: {e}")),
            Err(ureq::Error::StatusCode(code)) => Err(format!("OpenCode's server: HTTP {code}")),
            Err(e) => Err(format!("OpenCode's server: {e}")),
        }
    }

    pub fn get(&self, path: &str) -> Result<Value, String> {
        Self::read(self.agent.get(format!("{}{path}", self.base)).header("Authorization", &self.auth()).call())
    }

    pub fn post(&self, path: &str, body: Value) -> Result<Value, String> {
        Self::read(self.agent.post(format!("{}{path}", self.base)).header("Authorization", &self.auth()).send_json(body))
    }

    pub fn patch(&self, path: &str, body: Value) -> Result<Value, String> {
        Self::read(self.agent.patch(format!("{}{path}", self.base)).header("Authorization", &self.auth()).send_json(body))
    }
}

/// A session as the server lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    pub id: String,
    pub parent_id: Option<String>,
    pub title: Option<String>,
    pub directory: String,
    pub agent: String,
    /// `(provider, model, variant)`.
    pub model: Option<(String, String, Option<String>)>,
    pub created_ms: u64,
    pub updated_ms: u64,
    pub idle_ms: Option<u64>,
    pub viewed_ms: Option<u64>,
    /// Input, output and cache-read tokens of the last turn.
    pub tokens_used: u64,
}

fn str_of(v: &Value) -> Option<String> {
    v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

pub fn sessions(v: &Value) -> Vec<SessionInfo> {
    v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let t = &s["tokens"];
            Some(SessionInfo {
                id: s["id"].as_str()?.to_string(),
                parent_id: str_of(&s["parentID"]),
                title: str_of(&s["title"]),
                directory: s["location"]["directory"].as_str()?.to_string(),
                agent: s["agent"].as_str().unwrap_or("build").to_string(),
                model: s["model"]["id"].as_str().and_then(|id| Some((s["model"]["providerID"].as_str()?.to_string(), id.to_string(), str_of(&s["model"]["variant"])))),
                created_ms: s["time"]["created"].as_u64().unwrap_or(0),
                updated_ms: s["time"]["updated"].as_u64().unwrap_or(0),
                idle_ms: s["time"]["idle"].as_u64(),
                viewed_ms: s["time"]["viewed"].as_u64(),
                tokens_used: t["input"].as_u64().unwrap_or(0) + t["output"].as_u64().unwrap_or(0) + t["cache"]["read"].as_u64().unwrap_or(0),
            })
        })
        .collect()
}

/// The ids of the sessions a turn is running in.
pub fn active_ids(v: &Value) -> HashSet<String> {
    v["data"].as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default()
}

/// A tool call waiting for the user's say-so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Permission {
    pub id: String,
    pub action: String,
    pub resource: String,
}

pub fn permissions(v: &Value) -> Vec<Permission> {
    v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let resource = p["resources"][0].as_str().or_else(|| p["message"].as_str()).unwrap_or("").to_string();
            Some(Permission { id: p["id"].as_str()?.to_string(), action: p["action"].as_str().unwrap_or("").to_string(), resource })
        })
        .collect()
}

/// A question the agent asked, as a form with fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Form {
    pub id: String,
    pub title: String,
    pub fields: Vec<FormField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormField {
    pub key: String,
    pub title: String,
    /// `(value, label)`.
    pub options: Vec<(String, String)>,
}

pub fn forms(v: &Value) -> Vec<Form> {
    v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| f["state"]["status"].as_str().map_or(true, |s| s == "pending"))
        .filter_map(|f| {
            let fields = f["fields"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|fl| {
                    let key = fl["key"].as_str()?.to_string();
                    let options = fl["options"].as_array().into_iter().flatten().filter_map(|o| Some((o["value"].as_str()?.to_string(), o["label"].as_str().unwrap_or(o["value"].as_str()?).to_string()))).collect();
                    Some(FormField { title: fl["title"].as_str().unwrap_or(&key).to_string(), key, options })
                })
                .collect();
            Some(Form { id: f["id"].as_str()?.to_string(), title: f["title"].as_str().unwrap_or("Question").to_string(), fields })
        })
        .collect()
}

fn tool_line(part: &Value) -> String {
    let name = part["tool"].as_str().unwrap_or("tool");
    let input = &part["state"]["input"];
    match ["command", "filePath", "path", "pattern"].iter().find_map(|k| input[k].as_str()).map(str::trim).filter(|a| !a.is_empty()) {
        Some(a) => format!("{name}: {}", truncate(a, 120)),
        None => name.to_string(),
    }
}

/// The newest assistant text and the turns oldest first, from a message
/// list the server returned newest first.
pub fn messages(v: &Value) -> (Option<String>, Vec<Turn>) {
    let mut turns = Vec::new();
    let mut last: Option<String> = None;
    for m in v["data"].as_array().into_iter().flatten().rev() {
        match m["type"].as_str() {
            Some("user") => {
                if let Some(t) = str_of(&m["text"]) {
                    turns.push(Turn { kind: TurnKind::User, text: t });
                }
            }
            Some("assistant") => {
                for part in m["content"].as_array().into_iter().flatten() {
                    match part["type"].as_str() {
                        Some("text") => {
                            if let Some(t) = str_of(&part["text"]) {
                                last = Some(t.clone());
                                turns.push(Turn { kind: TurnKind::Assistant, text: t });
                            }
                        }
                        Some("tool") => turns.push(Turn { kind: TurnKind::Tool, text: tool_line(part) }),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    (last, turns)
}

/// One refresh's view of the server: every root session worth a card, with
/// what the server says about it and the card itself.
pub struct Fetched {
    pub session: SessionInfo,
    pub live: Live,
    pub card: Card,
}

/// Asks the server for its sessions and the running ones, then for each
/// candidate its pending permission, question and newest messages, and
/// builds the cards. `windows` are the open OpenCode terminals as
/// `(pid, folder)`; `context_of(provider, model)` is the model's window.
pub fn fetch(client: &Client, server_pid: i32, windows: &[(i32, String)], data_dir: &str, now_ms: u64, timeout_ms: u64, context_of: impl Fn(&str, &str) -> Option<u64>) -> Vec<Fetched> {
    let Ok(list) = client.get("/api/session?limit=50") else { return vec![] };
    let active = client.get("/api/session/active").map(|v| active_ids(&v)).unwrap_or_default();
    let mut out = Vec::new();
    for s in sessions(&list) {
        if s.parent_id.is_some() || s.directory == data_dir {
            continue;
        }
        let window = windows.iter().filter(|(_, dir)| *dir == s.directory).map(|(pid, _)| *pid).max();
        let last = s.updated_ms.max(s.idle_ms.unwrap_or(0)).max(s.viewed_ms.unwrap_or(0));
        let running = active.contains(&s.id);
        // A quiet session past the timeout with no window is not a card: no calls for it.
        if !running && now_ms.saturating_sub(last) >= timeout_ms && window.is_none() {
            continue;
        }
        let permission = client.get(&format!("/api/session/{}/permission", s.id)).map(|v| permissions(&v)).unwrap_or_default().into_iter().next();
        let form = if permission.is_none() { client.get(&format!("/api/session/{}/form", s.id)).map(|v| forms(&v)).unwrap_or_default().into_iter().next() } else { None };
        let (snippet, _) = client.get(&format!("/api/session/{}/message?limit=5&order=desc", s.id)).map(|v| messages(&v)).unwrap_or((None, vec![]));
        let live = Live { running, permission, form };
        let window_ctx = s.model.as_ref().and_then(|(p, m, _)| context_of(p, m));
        if let Some(card) = card_for(&s, &live, snippet, window_ctx, window, server_pid, data_dir, now_ms, timeout_ms) {
            out.push(Fetched { session: s, live, card });
        }
    }
    out
}

fn ok(r: Result<Value, String>) -> Result<(), String> {
    r.map(|_| ())
}

/// Posts a prompt: the server runs the turn, whichever window shows the session.
pub fn prompt(c: &Client, id: &str, text: &str) -> Result<(), String> {
    ok(c.post(&format!("/api/session/{id}/prompt"), serde_json::json!({ "text": text })))
}

pub fn reply_permission(c: &Client, id: &str, request_id: &str, decision: &str) -> Result<(), String> {
    ok(c.post(&format!("/api/session/{id}/permission/{request_id}/reply"), serde_json::json!({ "decision": decision })))
}

pub fn reply_form(c: &Client, id: &str, form_id: &str, key: &str, value: &str) -> Result<(), String> {
    ok(c.post(&format!("/api/session/{id}/form/{form_id}/reply"), serde_json::json!({ "answer": { key: value } })))
}

pub fn rename(c: &Client, id: &str, title: &str) -> Result<(), String> {
    ok(c.patch(&format!("/api/session/{id}"), serde_json::json!({ "title": title })))
}

pub fn compact(c: &Client, id: &str) -> Result<(), String> {
    ok(c.post(&format!("/api/session/{id}/compact"), serde_json::json!({})))
}

pub fn switch_model(c: &Client, id: &str, provider: &str, model: &str, variant: Option<&str>) -> Result<(), String> {
    let mut m = serde_json::json!({ "providerID": provider, "id": model });
    if let Some(v) = variant {
        m["variant"] = Value::String(v.to_string());
    }
    ok(c.post(&format!("/api/session/{id}/model"), serde_json::json!({ "model": m })))
}

pub fn switch_agent(c: &Client, id: &str, agent: &str) -> Result<(), String> {
    ok(c.post(&format!("/api/session/{id}/agent"), serde_json::json!({ "agent": agent })))
}

/// The mode cycle: the primary agent after `current`.
pub fn next_agent(current: &str) -> &'static str {
    if current == "build" { "plan" } else { "build" }
}

pub fn command(c: &Client, id: &str, name: &str, text: &str) -> Result<(), String> {
    ok(c.post(&format!("/api/session/{id}/command"), serde_json::json!({ "name": name, "text": text })))
}

pub fn shell(c: &Client, id: &str, line: &str) -> Result<(), String> {
    ok(c.post(&format!("/api/session/{id}/shell"), serde_json::json!({ "command": line })))
}

/// The session's last 60 messages as turns, oldest first.
pub fn history(c: &Client, id: &str) -> Result<Vec<Turn>, String> {
    c.get(&format!("/api/session/{id}/message?limit=60&order=desc")).map(|v| messages(&v).1)
}

/// What a permission card offers, in order.
pub const PERMISSION_CHOICES: [&str; 3] = ["Once", "Always", "Reject"];

/// The server's decision for a chosen option.
pub fn decision(option: usize) -> &'static str {
    ["once", "always", "reject"][option.min(2)]
}

/// What the server says about a session right now.
#[derive(Debug, Clone, Default)]
pub struct Live {
    pub running: bool,
    pub permission: Option<Permission>,
    pub form: Option<Form>,
}

/// The card for a session, or None when it is not on the board: a subagent
/// child, one of Maya's own one-shots (in its data folder), or a quiet
/// session past the completed timeout with no window open on its folder.
#[allow(clippy::too_many_arguments)]
pub fn card_for(s: &SessionInfo, live: &Live, snippet: Option<String>, context_window: Option<u64>, window_pid: Option<i32>, server_pid: i32, data_dir: &str, now_ms: u64, timeout_ms: u64) -> Option<Card> {
    if s.parent_id.is_some() || s.directory == data_dir {
        return None;
    }
    let last = s.updated_ms.max(s.idle_ms.unwrap_or(0)).max(s.viewed_ms.unwrap_or(0));
    let (state, since, awaiting) = if let Some(p) = &live.permission {
        let detail = format!("{}: {}", p.action, p.resource);
        let q = Question { question: detail.clone(), header: "Permission".into(), options: PERMISSION_CHOICES.iter().map(|l| Choice { label: l.to_string(), description: String::new() }).collect(), multi_select: false };
        (State::Awaiting, last, Some(Awaiting { kind: AwaitKind::Question, detail: truncate(&detail, SNIPPET_CHARS), questions: vec![q] }))
    } else if let Some(f) = &live.form {
        let qs = f.fields.iter().map(|fl| Question { question: fl.title.clone(), header: fl.key.clone(), options: fl.options.iter().map(|(v, l)| Choice { label: l.clone(), description: v.clone() }).collect(), multi_select: false }).collect();
        (State::Awaiting, last, Some(Awaiting { kind: AwaitKind::Question, detail: truncate(&f.title, SNIPPET_CHARS), questions: qs }))
    } else if live.running {
        (State::Working, s.updated_ms, None)
    } else if now_ms.saturating_sub(last) < timeout_ms {
        (State::Completed, last, None)
    } else if window_pid.is_some() {
        (State::Idle, last + timeout_ms, None)
    } else {
        return None;
    };
    let context = context_window.filter(|w| *w > 0).map(|w| crate::context::ContextUsage { used: s.tokens_used, window: w, percent: ((s.tokens_used as f64 / w as f64) * 100.0).round().min(100.0) as u8 });
    let short = &s.id[s.id.len().saturating_sub(6)..];
    Some(Card {
        session_id: s.id.clone(),
        pid: window_pid.unwrap_or(server_pid),
        name: s.title.clone().unwrap_or_else(|| format!("opencode {short}")),
        cwd: s.directory.clone(),
        state,
        state_since: since,
        snippet: truncate(snippet.as_deref().unwrap_or(""), SNIPPET_CHARS),
        awaiting,
        has_inbox: false,
        harness: Harness::OpenCode,
        pr: None,
        context,
        machine: None,
        machine_address: None,
        machine_platform: None,
        terminal: None,
        stale: false,
    })
}

#[cfg(test)]
pub(crate) mod fake {
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};

    /// A one-thread HTTP server answering `"<METHOD> <path>"` keys with a
    /// JSON body, 404 otherwise, recording every request it saw.
    pub fn fake_server(routes: Vec<(&'static str, &'static str)>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let hits = Arc::new(Mutex::new(Vec::new()));
        let seen = hits.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let head_end = loop {
                    let Ok(n) = s.read(&mut chunk) else { break None };
                    if n == 0 {
                        break None;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break Some(i + 4);
                    }
                };
                let Some(head_end) = head_end else { continue };
                let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                let len: usize = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0))).unwrap_or(0);
                while buf.len() < head_end + len {
                    let Ok(n) = s.read(&mut chunk) else { break };
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                let body = String::from_utf8_lossy(&buf[head_end..]).to_string();
                let line = head.lines().next().unwrap_or("").to_string();
                let key = line.rsplitn(2, ' ').last().unwrap_or("").to_string();
                seen.lock().unwrap().push(format!("{key}\n{head}\n{body}"));
                let (status, reply) = match routes.iter().find(|(k, _)| *k == key) {
                    Some((_, r)) => ("200 OK", r.to_string()),
                    None => ("404 Not Found", "{}".to_string()),
                };
                let _ = write!(s, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len());
                let _ = s.flush();
            }
        });
        (base, hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AwaitKind, State};
    use std::path::Path;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/opencode").join(name)).unwrap()
    }
    fn json(name: &str) -> Value {
        serde_json::from_str(&fixture(name)).unwrap()
    }

    #[test]
    fn the_state_file_names_the_server() {
        let t = tempfile::tempdir().unwrap();
        let path = service_file(t.path());
        assert_eq!(path, t.path().join("opencode/service.json"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, fixture("service.json")).unwrap();
        let s = read_service(&path).unwrap();
        assert_eq!((s.url.as_str(), s.pid, s.password.as_str()), ("http://127.0.0.1:49374", 4242, "not-the-real-one"));
        assert!(read_service(Path::new("/nonexistent")).is_none());
        std::fs::write(&path, "{}").unwrap();
        assert!(read_service(&path).is_none(), "no url, no server");
    }

    #[test]
    fn sessions_are_read_with_their_times_folder_and_tokens() {
        let list = sessions(&json("sessions.json"));
        assert!(list.len() >= 4);
        let hi = list.iter().find(|s| s.title.as_deref() == Some("Saying \"Hi\" request")).unwrap();
        assert_eq!(hi.directory, "/Users/tiagocorreia");
        assert_eq!(hi.model.as_ref().map(|m| (m.0.as_str(), m.1.as_str())), Some(("opencode", "fledge-alpha-free")));
        assert_eq!(hi.idle_ms, Some(1_791_029_162_638));
        assert_eq!(hi.tokens_used, 10_083 + 207, "input plus output plus cache reads");
        assert!(hi.parent_id.is_none());
        assert!(sessions(&Value::Null).is_empty());
    }

    #[test]
    fn active_permissions_and_forms_are_read() {
        assert!(active_ids(&json("active.json")).is_empty());
        let a = active_ids(&serde_json::json!({"data": {"ses_1": {"type": "running"}}}));
        assert!(a.contains("ses_1"));
        let p = permissions(&json("permission.json"));
        assert_eq!((p[0].id.as_str(), p[0].action.as_str(), p[0].resource.as_str()), ("per_01", "edit", "/tmp/probe-hello.txt"));
        assert!(permissions(&json("permission-empty.json")).is_empty());
        let f = forms(&json("form.json"));
        assert_eq!((f[0].id.as_str(), f[0].title.as_str(), f[0].fields[0].key.as_str()), ("frm_01", "Which branch?", "branch"));
        assert_eq!(f[0].fields[0].options, vec![("main".to_string(), "main".to_string()), ("dev".to_string(), "dev".to_string())]);
        assert!(forms(&json("form-empty.json")).is_empty());
        assert_eq!(decision(0), "once");
        assert_eq!(decision(2), "reject");
    }

    #[test]
    fn messages_give_the_last_text_and_the_turns_oldest_first() {
        let (last, turns) = messages(&json("messages.json"));
        assert_eq!(last.as_deref(), Some("Hi"));
        let got: Vec<(String, String)> = turns.iter().map(|t| (format!("{:?}", t.kind).to_lowercase(), t.text.clone())).collect();
        assert_eq!(got, vec![("user".to_string(), "say \"Hi\"".to_string()), ("assistant".to_string(), "Hi".to_string())], "idle messages and reasoning parts are not turns");
    }

    fn session(id: &str, dir: &str, updated: u64) -> SessionInfo {
        SessionInfo { id: id.into(), parent_id: None, title: Some("t".into()), directory: dir.into(), agent: "build".into(), model: None, created_ms: updated, updated_ms: updated, idle_ms: None, viewed_ms: None, tokens_used: 500 }
    }

    #[test]
    fn card_rules_running_waiting_completed_idle_and_dropped() {
        let min = 60_000;
        let s = session("ses_1", "/p", 1_000);
        let quiet = Live { running: false, permission: None, form: None };
        let c = card_for(&s, &Live { running: true, ..quiet.clone() }, Some("hi".into()), Some(1_000), None, 7, "/maya", 5_000, 30 * min).unwrap();
        assert_eq!((c.state, c.pid, c.snippet.as_str(), c.context.map(|x| x.percent)), (State::Working, 7, "hi", Some(50)));
        let p = card_for(&s, &Live { permission: Some(Permission { id: "per_1".into(), action: "edit".into(), resource: "/tmp/x".into() }), ..quiet.clone() }, None, None, Some(99), 7, "/maya", 5_000, 30 * min).unwrap();
        assert_eq!((p.state, p.pid), (State::Awaiting, 99));
        let aw = p.awaiting.unwrap();
        assert_eq!((aw.kind, aw.detail.as_str()), (AwaitKind::Question, "edit: /tmp/x"));
        assert_eq!(aw.questions[0].options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>(), PERMISSION_CHOICES);
        let f = card_for(&s, &Live { form: Some(forms(&json("form.json")).remove(0)), ..quiet.clone() }, None, None, None, 7, "/maya", 5_000, 30 * min).unwrap();
        let aw = f.awaiting.unwrap();
        assert_eq!((aw.kind, aw.detail.as_str(), aw.questions[0].options.len()), (AwaitKind::Question, "Which branch?", 2));
        let done = card_for(&s, &quiet, None, None, None, 7, "/maya", 1_000 + 10 * min, 30 * min).unwrap();
        assert_eq!(done.state, State::Completed);
        let idle = card_for(&s, &quiet, None, None, Some(55), 7, "/maya", 1_000 + 40 * min, 30 * min).unwrap();
        assert_eq!((idle.state, idle.pid), (State::Idle, 55), "a window for the folder keeps it as Idle");
        assert!(card_for(&s, &quiet, None, None, None, 7, "/maya", 1_000 + 40 * min, 30 * min).is_none(), "no window, past the timeout: gone");
        assert!(card_for(&session("ses_2", "/maya", 1_000), &Live { running: true, ..quiet.clone() }, None, None, None, 7, "/maya", 5_000, 30 * min).is_none(), "Maya's own one-shot");
        let child = SessionInfo { parent_id: Some("ses_1".into()), ..session("ses_3", "/p", 1_000) };
        assert!(card_for(&child, &Live { running: true, ..quiet }, None, None, None, 7, "/maya", 5_000, 30 * min).is_none(), "a subagent child");
    }

    #[test]
    fn the_client_sends_basic_auth_and_reads_json() {
        let (base, hits) = fake::fake_server(vec![("GET /api/session/active", "{\"data\":{}}")]);
        let c = Client::new(&Service { url: base, pid: 1, password: "pw".into() });
        assert_eq!(c.get("/api/session/active").unwrap()["data"], serde_json::json!({}));
        let h = hits.lock().unwrap();
        assert!(h[0].contains("authorization: Basic b3BlbmNvZGU6cHc=") || h[0].contains("Authorization: Basic b3BlbmNvZGU6cHc="), "opencode:pw in base64: {}", h[0]);
        drop(h);
        assert!(c.get("/api/nothing").unwrap_err().contains("404"));
    }
}
