//! The Antigravity brain: one `agy -p` process kept alive, spoken to in
//! stream-json on stdin and read the same way on stdout.
//!
//! Each user line runs a turn and ends with a `result` event. Her
//! conversation id comes with the `init` event. agy has no in-band stop
//! (a control request ends the session with an error), so a stop
//! interrupts her process and starts it again on the same conversation
//! (`--conversation <id>`), behind the same driver: see `stdio::Protocol::relaunch`.
//! agy has no system-prompt flag either: on a fresh conversation the
//! system prompt leads her first message.

use std::sync::mpsc::Sender;

use serde_json::{json, Value};

use super::driver::{Event, Launch};
use super::stdio::{self, LineDriver, Protocol};

/// The arguments for the long-lived process, in auto mode. A resumed
/// conversation gets `--conversation`; each writable folder an `--add-dir`.
pub fn args(l: &Launch) -> Vec<String> {
    let s = |v: &str| v.to_string();
    let mut a = vec![s("-p"), s("--input-format"), s("stream-json"), s("--output-format"), s("stream-json"), s("--mode"), s("auto")];
    if let Some(id) = &l.resume {
        a.extend([s("--conversation"), id.clone()]);
    }
    for d in &l.writable {
        a.extend([s("--add-dir"), d.to_string_lossy().into_owned()]);
    }
    if let Some(m) = &l.model {
        a.extend([s("--model"), m.clone()]);
    }
    a
}

/// One user message, as a line for stdin.
pub fn user_line(text: &str) -> String {
    format!("{}\n", json!({"event": "user", "message": {"content": text}}))
}

/// The first of `keys` an event has, at its top level or under the
/// event's own name (`{"event":"result","result":{…}}`).
fn field<'a>(v: &'a Value, kind: &str, keys: &[&str]) -> Option<&'a str> {
    [&v[kind], v].into_iter().flat_map(|o| keys.iter().map(move |k| &o[*k])).find_map(Value::as_str).filter(|x| !x.is_empty())
}

/// Her conversation's id, as agy may spell it.
const CONVERSATION: &[&str] = &["conversation_id", "conversationId"];

/// Her agy conversation as a line protocol, with no process in it.
pub struct Parser {
    l: Launch,
    /// Her conversation: named by her agent, else the one resumed. A
    /// relaunch goes back to it.
    conversation: Option<String>,
    /// The conversation last named in `Ready`.
    named: Option<String>,
    /// A fresh conversation's system prompt, leading her first message.
    lead: Option<String>,
    /// Messages written whose `result` has not come.
    unanswered: usize,
    in_turn: bool,
    /// Her process is being started again: messages wait for it.
    relaunching: bool,
    waiting: Vec<String>,
    /// The arguments a stop asked to start her again with, until taken.
    relaunch: Option<Vec<String>>,
}

impl Parser {
    pub fn new(l: &Launch) -> Parser {
        let mut p = Parser { l: l.clone(), conversation: l.resume.clone(), named: None, lead: None, unanswered: 0, in_turn: false, relaunching: false, waiting: vec![], relaunch: None };
        p.lead = p.fresh_lead();
        p
    }

    /// The system prompt, when her conversation is a fresh one.
    fn fresh_lead(&self) -> Option<String> {
        (self.conversation.is_none() && !self.l.system_prompt.is_empty()).then(|| self.l.system_prompt.clone())
    }

    /// One line from her agent: `init` names her conversation, `result` ends a turn.
    pub fn line(&mut self, line: &str) -> Vec<Event> {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { return vec![] };
        let kind = v["event"].as_str().or_else(|| v["type"].as_str()).unwrap_or("");
        let mut out = vec![];
        match kind {
            "init" => self.name(field(&v, kind, CONVERSATION), &mut out),
            "result" => {
                self.name(field(&v, kind, CONVERSATION), &mut out);
                // One with no message of hers before it: a turn she started herself.
                self.begin(&mut out);
                let ok = field(&v, kind, &["status"]).is_some_and(|s| s.eq_ignore_ascii_case("success"));
                let text = field(&v, kind, &["response"]).unwrap_or("").to_string();
                self.end(ok.then_some(text), &mut out);
                self.unanswered = self.unanswered.saturating_sub(1);
                if self.unanswered > 0 {
                    self.begin(&mut out);
                }
            }
            _ => {}
        }
        out
    }

    /// `Ready` for `id`, unless it is the conversation already named.
    fn name(&mut self, id: Option<&str>, out: &mut Vec<Event>) {
        let Some(id) = id else { return };
        self.conversation = Some(id.to_string());
        if self.named.as_deref() != Some(id) {
            self.named = Some(id.to_string());
            out.push(Event::Ready { session_id: id.to_string() });
        }
    }

    /// A message for her: the line to write now, or nothing while her
    /// process starts again (it goes out once it has). Her turn starts either way.
    pub fn send(&mut self, text: &str) -> (Vec<Event>, Option<String>) {
        let mut out = vec![];
        self.begin(&mut out);
        if self.relaunching {
            self.waiting.push(text.to_string());
            return (out, None);
        }
        (out, Some(self.message(text)))
    }

    fn message(&mut self, text: &str) -> String {
        self.unanswered += 1;
        match self.lead.take() {
            Some(lead) => user_line(&format!("{lead}\n\n{text}")),
            None => user_line(text),
        }
    }

    /// Stops her running turn, which fails at once. agy cannot stop a turn
    /// in-band, so her process goes with it (see `relaunch`), and every
    /// message it had not answered.
    pub fn cancel(&mut self) -> Vec<Event> {
        let mut out = vec![];
        self.waiting.clear();
        if !self.in_turn && self.unanswered == 0 {
            return out;
        }
        self.end(None, &mut out);
        let written = std::mem::take(&mut self.unanswered) > 0;
        if written && !self.relaunching {
            self.relaunching = true;
            // No conversation named yet: the new process starts a fresh one.
            if self.conversation.is_none() {
                self.lead = self.fresh_lead();
            }
            let mut l = self.l.clone();
            l.resume = self.conversation.clone();
            self.relaunch = Some(args(&l));
        }
        out
    }

    pub fn relaunch(&mut self) -> Option<Vec<String>> {
        self.relaunch.take()
    }

    /// Her process runs (again): the messages that waited for it.
    pub fn start(&mut self) -> Option<String> {
        self.relaunching = false;
        let lines: String = std::mem::take(&mut self.waiting).iter().map(|t| self.message(t)).collect();
        (!lines.is_empty()).then_some(lines)
    }

    fn begin(&mut self, out: &mut Vec<Event>) {
        if !self.in_turn {
            self.in_turn = true;
            out.push(Event::TurnStarted);
        }
    }

    /// Ends her turn with her reply, or fails it with None.
    fn end(&mut self, reply: Option<String>, out: &mut Vec<Event>) {
        if !std::mem::replace(&mut self.in_turn, false) {
            return;
        }
        // Her agent compacts itself: no context size is reported.
        out.push(match reply {
            Some(text) => Event::TurnEnded { text, context_tokens: None },
            None => Event::TurnFailed { context_tokens: None },
        });
    }
}

impl Protocol for Parser {
    fn label(&self) -> &'static str {
        "Antigravity"
    }

    fn start(&mut self) -> Option<String> {
        Parser::start(self)
    }

    fn line(&mut self, line: &str) -> (Vec<Event>, Option<String>) {
        (Parser::line(self, line), None)
    }

    fn send(&mut self, text: &str) -> (Vec<Event>, Option<String>) {
        Parser::send(self, text)
    }

    fn cancel(&mut self, _now_ms: u64) -> (Vec<Event>, Option<String>) {
        (Parser::cancel(self), None)
    }

    fn relaunch(&mut self) -> Option<Vec<String>> {
        Parser::relaunch(self)
    }

    fn background_subagents(&self) -> bool {
        true
    }
}

pub type AgyDriver = LineDriver<Parser>;

/// Starts her `agy -p`; Ready comes from her init line, not from the spawn.
pub fn spawn(l: &Launch, events: Sender<Event>) -> Result<AgyDriver, String> {
    stdio::spawn(l, args(l), &[], Parser::new(l), events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Harness;
    #[cfg(unix)]
    use std::time::{Duration, Instant};

    fn launch() -> Launch {
        Launch { agent: Harness::Antigravity, binary: "agy".into(), model: None, cwd: "/tmp/maya-brain".into(), system_prompt: "You are Maya.".into(), resume: None, writable: vec![] }
    }

    fn json_of(line: &str) -> Value {
        assert!(line.ends_with('\n'), "one line, newline-terminated: {line:?}");
        serde_json::from_str(line.trim_end()).unwrap()
    }

    fn result(status: &str, response: &str) -> String {
        json!({"event": "result", "result": {"conversation_id": "c-1", "status": status, "response": response}}).to_string()
    }

    const INIT: &str = r#"{"event":"init","conversation_id":"c-1"}"#;

    /// The text of the user line `send` wrote.
    fn sent(p: &mut Parser, text: &str) -> (Vec<Event>, String) {
        let (e, w) = p.send(text);
        let v = json_of(&w.expect("a line to write"));
        (e, v["message"]["content"].as_str().unwrap().to_string())
    }

    #[test]
    fn agy_runs_in_auto_mode_and_resumes_its_conversation() {
        let mut l = launch();
        assert_eq!(args(&l), vec!["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--mode", "auto"]);
        l.resume = Some("c-1".into());
        l.writable = vec!["/w/projects".into(), "/tmp".into()];
        l.model = Some("gemini-3-pro".into());
        assert_eq!(args(&l), vec!["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--mode", "auto", "--conversation", "c-1", "--add-dir", "/w/projects", "--add-dir", "/tmp", "--model", "gemini-3-pro"]);
    }

    #[test]
    fn a_user_line_is_one_stream_json_message() {
        let v = json_of(&user_line("hi \"there\"\nnext"));
        assert_eq!(v, json!({"event": "user", "message": {"content": "hi \"there\"\nnext"}}));
    }

    #[test]
    fn an_init_event_names_the_conversation() {
        let mut p = Parser::new(&launch());
        assert_eq!(p.line(INIT), vec![Event::Ready { session_id: "c-1".into() }]);
        assert!(p.line(INIT).is_empty(), "the same conversation is named once");
        assert!(p.line(r#"{"event":"init"}"#).is_empty(), "an init with no id names nothing");
        let mut p = Parser::new(&launch());
        assert_eq!(p.line(r#"{"event":"init","init":{"conversationId":"c-2"}}"#), vec![Event::Ready { session_id: "c-2".into() }], "nested or camel-cased");
        let mut l = launch();
        l.resume = Some("c-1".into());
        assert_eq!(Parser::new(&l).line(INIT), vec![Event::Ready { session_id: "c-1".into() }], "a resumed conversation is ready by her init, not by the spawn");
    }

    #[test]
    fn a_result_ends_the_turn() {
        let mut p = Parser::new(&launch());
        p.line(INIT);
        let (e, _) = sent(&mut p, "hi");
        assert_eq!(e, vec![Event::TurnStarted], "the turn starts as the message goes");
        assert!(p.line(r#"{"event":"message","text":"thinking"}"#).is_empty());
        assert!(p.line("not json").is_empty());
        assert_eq!(p.line(&result("success", "Hello")), vec![Event::TurnEnded { text: "Hello".into(), context_tokens: None }]);
    }

    #[test]
    fn a_result_that_is_not_a_success_fails_the_turn() {
        let mut p = Parser::new(&launch());
        p.line(INIT);
        for status in ["INTERRUPTED", "ERROR", "failed"] {
            sent(&mut p, "hi");
            assert_eq!(p.line(&result(status, "half a reply")), vec![Event::TurnFailed { context_tokens: None }], "{status}");
        }
        sent(&mut p, "hi");
        assert_eq!(p.line(&result("SUCCESS", "ok")), vec![Event::TurnEnded { text: "ok".into(), context_tokens: None }], "the case of the status does not matter");
    }

    #[test]
    fn a_result_names_the_conversation_when_no_init_did() {
        let mut p = Parser::new(&launch());
        sent(&mut p, "hi");
        assert_eq!(p.line(&result("success", "ok")), vec![Event::Ready { session_id: "c-1".into() }, Event::TurnEnded { text: "ok".into(), context_tokens: None }]);
    }

    #[test]
    fn messages_sent_together_run_one_turn_each() {
        let mut p = Parser::new(&launch());
        p.line(INIT);
        assert_eq!(sent(&mut p, "one").0, vec![Event::TurnStarted]);
        assert!(sent(&mut p, "two").0.is_empty(), "her turn is already running");
        assert_eq!(p.line(&result("success", "1")), vec![Event::TurnEnded { text: "1".into(), context_tokens: None }, Event::TurnStarted]);
        assert_eq!(p.line(&result("success", "2")), vec![Event::TurnEnded { text: "2".into(), context_tokens: None }]);
    }

    #[test]
    fn a_turn_she_starts_herself_starts_and_ends_on_its_result() {
        let mut p = Parser::new(&launch());
        p.line(INIT);
        assert_eq!(p.line(&result("success", "a subagent finished")), vec![Event::TurnStarted, Event::TurnEnded { text: "a subagent finished".into(), context_tokens: None }]);
    }

    #[test]
    fn the_system_prompt_leads_the_first_message_of_a_fresh_conversation_only() {
        let mut p = Parser::new(&launch());
        assert_eq!(sent(&mut p, "hi").1, "You are Maya.\n\nhi");
        assert_eq!(sent(&mut p, "again").1, "again");
        let mut l = launch();
        l.resume = Some("c-1".into());
        assert_eq!(sent(&mut Parser::new(&l), "hi").1, "hi", "a resumed conversation has it already");
        l.resume = None;
        l.system_prompt = String::new();
        assert_eq!(sent(&mut Parser::new(&l), "hi").1, "hi");
    }

    #[test]
    fn a_stop_fails_the_turn_at_once_and_relaunches_her_on_her_conversation() {
        let mut p = Parser::new(&launch());
        p.line(INIT);
        sent(&mut p, "one");
        sent(&mut p, "two");
        assert_eq!(p.cancel(), vec![Event::TurnFailed { context_tokens: None }], "both messages go with her process; one turn ends");
        let a = p.relaunch().expect("a relaunch");
        assert!(a.windows(2).any(|w| w[0] == "--conversation" && w[1] == "c-1"), "{a:?}");
        assert!(a.windows(2).any(|w| w[0] == "--mode" && w[1] == "auto"), "{a:?}");
        assert_eq!(p.relaunch(), None, "asked once");
        // Sent while her process starts again: it waits for it.
        let (e, w) = p.send("three");
        assert_eq!((e, w), (vec![Event::TurnStarted], None));
        let started = p.start().expect("the waiting message");
        assert_eq!(json_of(&started)["message"]["content"], "three", "no system prompt on her own conversation");
        assert!(p.line(INIT).is_empty(), "the same conversation: no second Ready");
        assert_eq!(p.line(&result("success", "3")), vec![Event::TurnEnded { text: "3".into(), context_tokens: None }]);
    }

    #[test]
    fn a_stop_with_no_turn_running_does_nothing() {
        let mut p = Parser::new(&launch());
        p.line(INIT);
        assert!(p.cancel().is_empty());
        assert_eq!(p.relaunch(), None);
        sent(&mut p, "hi");
        p.line(&result("success", "ok"));
        assert!(p.cancel().is_empty());
        assert_eq!(p.relaunch(), None);
    }

    #[test]
    fn a_stop_before_her_conversation_is_known_ends_the_turn_at_once_and_starts_her_fresh() {
        let mut p = Parser::new(&launch());
        assert_eq!(sent(&mut p, "hi"), (vec![Event::TurnStarted], "You are Maya.\n\nhi".to_string()));
        assert_eq!(p.cancel(), vec![Event::TurnFailed { context_tokens: None }]);
        let a = p.relaunch().expect("a relaunch");
        assert!(!a.iter().any(|x| x == "--conversation"), "no conversation to resume: {a:?}");
        assert_eq!(p.start(), None, "nothing waits");
        assert_eq!(sent(&mut p, "again").1, "You are Maya.\n\nagain", "her new conversation gets the system prompt");
    }

    #[test]
    fn a_stop_while_she_starts_again_drops_what_waits() {
        let mut p = Parser::new(&launch());
        p.line(INIT);
        sent(&mut p, "one");
        p.cancel();
        p.relaunch();
        assert_eq!(p.send("two").0, vec![Event::TurnStarted]);
        assert_eq!(p.cancel(), vec![Event::TurnFailed { context_tokens: None }]);
        assert_eq!(p.relaunch(), None, "the relaunch under way stands");
        assert_eq!(p.start(), None, "the stopped message is not sent");
    }

    #[test]
    fn antigravity_compacts_itself_and_runs_background_subagents() {
        let mut p = Parser::new(&launch());
        assert!(Protocol::background_subagents(&p));
        assert_eq!(Protocol::compact(&mut p).err().as_deref(), Some("unsupported"));
    }

    #[cfg(unix)]
    fn fake(script: &str) -> (std::path::PathBuf, Launch) {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("maya-fake-agy-{}", crate::launch::new_session_uuid()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("agy");
        std::fs::write(&bin, format!("#!/bin/sh\n{}", script.replace("$DIR", &dir.to_string_lossy()))).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut l = launch();
        l.binary = bin;
        l.cwd = dir.join("work");
        (dir, l)
    }

    /// Records each launch's arguments, names conversation c-1, answers each
    /// message, and on SIGINT ends (a message with "slow" in it runs until then).
    #[cfg(unix)]
    const FAKE_AGY: &str = r#"echo "$*" >> "$DIR/argv"
trap 'exit 130' INT
echo '{"event":"init","conversation_id":"c-1"}'
while read -r line; do
  case "$line" in
    *slow*) sleep 30 </dev/null >/dev/null 2>&1 & wait $! ;;
    *) echo '{"event":"result","result":{"conversation_id":"c-1","status":"success","response":"ok"}}' ;;
  esac
done
"#;

    #[cfg(unix)]
    #[test]
    fn a_stop_interrupts_her_process_and_starts_it_again_on_her_conversation() {
        let (dir, l) = fake(FAKE_AGY);
        let (tx, rx) = std::sync::mpsc::channel();
        let mut d = super::super::driver::spawn(&l, tx).unwrap();
        let wait = Duration::from_secs(5);
        assert_eq!(rx.recv_timeout(wait).unwrap(), Event::Ready { session_id: "c-1".into() });
        d.send("hi").unwrap();
        assert_eq!(rx.recv_timeout(wait).unwrap(), Event::TurnStarted);
        assert_eq!(rx.recv_timeout(wait).unwrap(), Event::TurnEnded { text: "ok".into(), context_tokens: None });
        d.send("slow").unwrap();
        assert_eq!(rx.recv_timeout(wait).unwrap(), Event::TurnStarted);
        let stopped = Instant::now();
        d.interrupt().unwrap();
        assert_eq!(rx.recv_timeout(wait).unwrap(), Event::TurnFailed { context_tokens: None });
        assert!(stopped.elapsed() < Duration::from_secs(1), "the turn fails at once");
        // Sent while she starts again: it waits for her new process.
        d.send("again").unwrap();
        assert_eq!(rx.recv_timeout(wait).unwrap(), Event::TurnStarted);
        assert_eq!(rx.recv_timeout(wait).unwrap(), Event::TurnEnded { text: "ok".into(), context_tokens: None }, "no Exited: her process came back");
        let argv = std::fs::read_to_string(dir.join("argv")).unwrap();
        let launches: Vec<&str> = argv.lines().collect();
        assert_eq!(launches.len(), 2, "{argv}");
        assert!(launches[0].contains("--mode auto") && !launches[0].contains("--conversation"), "{argv}");
        assert!(launches[1].contains("--mode auto") && launches[1].contains("--conversation c-1"), "{argv}");
        d.shutdown();
        assert!(matches!(rx.recv_timeout(wait).unwrap(), Event::Exited(_)));
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "Exited is the last event");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_process_that_dies_unasked_ends_her_session() {
        let (dir, l) = fake("echo '{\"event\":\"init\",\"conversation_id\":\"c-1\"}'\nread -r line\necho 'Error: quota' >&2\nexit 1\n");
        let (tx, rx) = std::sync::mpsc::channel();
        let mut d = super::super::driver::spawn(&l, tx).unwrap();
        let wait = Duration::from_secs(5);
        assert_eq!(rx.recv_timeout(wait).unwrap(), Event::Ready { session_id: "c-1".into() });
        d.send("hi").unwrap();
        assert_eq!(rx.recv_timeout(wait).unwrap(), Event::TurnStarted);
        assert_eq!(rx.recv_timeout(wait).unwrap(), Event::Exited("the Antigravity process ended (stderr: Error: quota)".into()));
        drop(d);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_relaunch_that_cannot_start_ends_her_session() {
        // Her process removes its own binary: the relaunch cannot start it.
        let (dir, l) = fake(r#"rm -f "$DIR/agy"
trap 'exit 130' INT
echo '{"event":"init","conversation_id":"c-1"}'
while read -r line; do sleep 30 </dev/null >/dev/null 2>&1 & wait $!; done
"#);
        let (tx, rx) = std::sync::mpsc::channel();
        let mut d = super::super::driver::spawn(&l, tx).unwrap();
        let wait = Duration::from_secs(5);
        assert_eq!(rx.recv_timeout(wait).unwrap(), Event::Ready { session_id: "c-1".into() });
        d.send("hi").unwrap();
        assert_eq!(rx.recv_timeout(wait).unwrap(), Event::TurnStarted);
        d.interrupt().unwrap();
        assert_eq!(rx.recv_timeout(wait).unwrap(), Event::TurnFailed { context_tokens: None });
        match rx.recv_timeout(wait).unwrap() {
            Event::Exited(why) => assert!(why.starts_with("cannot start Antigravity again"), "{why}"),
            e => panic!("{e:?}"),
        }
        drop(d);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
