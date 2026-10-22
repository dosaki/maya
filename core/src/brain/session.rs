//! The brain's session rules, with no process in them: the request queue,
//! busy replies, which request a turn answers, stop, compaction and what is
//! handed back when the brain exits. The app glue drives it.

use std::collections::{HashSet, VecDeque};

use super::chat::Via;
use super::driver::Event;
use super::protocol;

pub const COMPACT_AT_PERCENT: u8 = 80;
/// Board notes kept between turns; older ones are dropped.
pub const MAX_NOTES: usize = 50;
const NOTES_DROPPED: &str = "(older changes dropped)";

/// Her job while she answers one of Maya's own follow-ups (refused actions).
pub const CHECKING: &str = "checking my last actions";
/// Her job during a turn of her own: a wake, notes, the primer.
pub const LOOKING: &str = "looking at what changed";
/// A brain with background subagents is busy only for a moment: the user
/// still hears that their request waits.
pub const STILL_ON_IT: &str = "I'm still on the last thing; I'll do that next.";
/// What the user hears when the turn answering them ended in an error.
pub const TURN_FAILED: &str = "Something went wrong on my side; try again.";

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub text: String,
    /// How the reply goes back. An internal request carries the via of the
    /// user request it follows up, so her answer goes back the same way.
    pub via: Via,
    /// Maya's own follow-up, not the user's words: never posted to Chat as a
    /// user turn and never answered with a busy line.
    pub internal: bool,
}

impl Request {
    pub fn new(text: &str, via: Via) -> Request {
        Request { text: text.into(), via, internal: false }
    }

    pub fn internal(text: &str, via: Via) -> Request {
        Request { text: text.into(), via, internal: true }
    }

    /// A request the user made, as opposed to one of Maya's or her own.
    fn from_user(&self) -> bool {
        !self.internal && self.via != Via::Maya
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    /// The glue builds `prompt::turn` with the fresh board and the taken
    /// notes. `None` means notes only (a watch fired).
    Deliver(Option<Request>),
    Reply { via: Via, say: String, actions: Vec<serde_json::Value>, errors: Vec<String> },
    Busy { via: Via, say: String },
    Interrupt,
    Compact,
    /// A line Maya says on her own behalf.
    Said { via: Via, say: String },
}

pub struct Session {
    queue: VecDeque<Request>,
    current: Option<Via>,
    in_flight: Option<Request>,
    turn_running: bool,
    stopped: bool,
    job: Option<String>,
    tasks: HashSet<String>,
    notes: VecDeque<String>,
    /// Notes were dropped since they were last taken.
    notes_dropped: bool,
    background: bool,
    wake_pending: bool,
    /// Context passed the limit; compaction waits for a quiet moment.
    compact_due: bool,
    /// The running turn is the compaction itself.
    compacting: bool,
}

impl Session {
    pub fn new(background_subagents: bool) -> Session {
        Session {
            queue: VecDeque::new(),
            current: None,
            in_flight: None,
            turn_running: false,
            stopped: false,
            job: None,
            tasks: HashSet::new(),
            notes: VecDeque::new(),
            notes_dropped: false,
            background: background_subagents,
            wake_pending: false,
            compact_due: false,
            compacting: false,
        }
    }

    pub fn submit(&mut self, req: Request) -> Vec<Out> {
        if !self.turn_running {
            return vec![self.deliver(Some(req))];
        }
        // A user's request always hears that it waits, so voice never goes
        // dead; her own turns and Maya's follow-ups queue without a word.
        let busy = req.from_user().then(|| Out::Busy {
            via: req.via,
            say: match &self.job {
                _ if self.background => STILL_ON_IT.into(),
                Some(job) => format!("I'm in the middle of {job}, I'll do that next."),
                None => "I'm in the middle of something, I'll do that next.".into(),
            },
        });
        self.queue.push_back(req);
        busy.into_iter().collect()
    }

    /// Adds a note for her next message, keeping the latest `MAX_NOTES`.
    pub fn note(&mut self, line: String) {
        self.notes.push_back(line);
        while self.notes.len() > MAX_NOTES {
            self.notes.pop_front();
            self.notes_dropped = true;
        }
    }

    pub fn wake(&mut self) -> Vec<Out> {
        if self.turn_running {
            self.wake_pending = true;
            return vec![];
        }
        vec![self.deliver(None)]
    }

    /// The notes since the last take, led by a marker when older ones were dropped.
    pub fn take_notes(&mut self) -> Vec<String> {
        let dropped = std::mem::take(&mut self.notes_dropped).then(|| NOTES_DROPPED.to_string());
        dropped.into_iter().chain(self.notes.drain(..)).collect()
    }

    /// The driver cannot compact on request (it compacts itself): the
    /// compaction turn ends with nothing to swallow, and the next request
    /// or a pending wake goes out. A no-op when no compaction runs.
    pub fn compact_skipped(&mut self) -> Vec<Out> {
        if !self.compacting {
            return vec![];
        }
        self.compacting = false;
        self.compact_due = false;
        self.turn_running = false;
        self.current = None;
        self.job = None;
        let mut out = vec![];
        self.advance(&mut out);
        out
    }

    pub fn on_event(&mut self, e: &Event, context_percent: Option<u8>) -> Vec<Out> {
        match e {
            Event::Ready { .. } => vec![],
            Event::TurnStarted => {
                self.turn_running = true;
                self.current.get_or_insert(Via::Maya);
                vec![]
            }
            Event::TaskStarted(id) => {
                self.tasks.insert(id.clone());
                vec![]
            }
            Event::TaskEnded(id) => {
                self.tasks.remove(id);
                let mut out = vec![];
                if self.tasks.is_empty() && !self.turn_running {
                    self.advance(&mut out);
                }
                out
            }
            Event::Exited(_) => {
                self.turn_running = false;
                self.current = None;
                self.job = None;
                self.stopped = false;
                self.wake_pending = false;
                self.compact_due = false;
                self.compacting = false;
                self.tasks.clear();
                vec![]
            }
            Event::TurnEnded { text, .. } => self.turn_ended(Some(text), context_percent),
            Event::TurnFailed { .. } => self.turn_ended(None, context_percent),
        }
    }

    /// A turn ended with her reply `text`, or in an error (None): the
    /// reply goes back the way the request came, unless she was stopped.
    /// A failed answer to the user says so; a failed turn of her own is quiet.
    fn turn_ended(&mut self, text: Option<&String>, context_percent: Option<u8>) -> Vec<Out> {
        let stopped = std::mem::take(&mut self.stopped);
        self.turn_running = false;
        self.in_flight = None;
        self.job = None;
        let mut out = vec![];
        if std::mem::take(&mut self.compacting) {
            // The compaction's own turn: nothing to say.
            self.current = None;
        } else {
            let via = self.current.take().unwrap_or(Via::Maya);
            if !stopped {
                out.push(match text {
                    Some(text) => {
                        let p = protocol::parse(text);
                        Out::Reply { via, say: p.say, actions: p.actions, errors: p.errors }
                    }
                    None => Out::Reply { via, say: if via == Via::Maya { String::new() } else { TURN_FAILED.into() }, actions: vec![], errors: vec![] },
                });
            }
            if let Some(p) = context_percent {
                self.compact_due = p >= COMPACT_AT_PERCENT;
            }
        }
        self.advance(&mut out);
        out
    }

    pub fn stop(&mut self, via: Via) -> Vec<Out> {
        if !self.working() {
            return vec![Out::Said { via, say: "Nothing to stop.".into() }];
        }
        self.queue.clear();
        self.notes.clear();
        self.notes_dropped = false;
        self.wake_pending = false;
        self.in_flight = None;
        if self.turn_running {
            self.stopped = true;
        }
        vec![Out::Interrupt, Out::Said { via, say: "Stopped.".into() }]
    }

    /// Her own turn runs (a delivered message, one she started herself, or
    /// a compaction); her background tasks do not count.
    pub fn in_turn(&self) -> bool {
        self.turn_running
    }

    pub fn working(&self) -> bool {
        self.turn_running || !self.tasks.is_empty()
    }

    pub fn job(&self) -> Option<&str> {
        self.job.as_deref()
    }

    pub fn drain(&mut self) -> Vec<Request> {
        self.in_flight.take().into_iter().chain(self.queue.drain(..)).collect()
    }

    /// No turn is running: compact if due and quiet, else hand over the next
    /// request or a pending wake.
    fn advance(&mut self, out: &mut Vec<Out>) {
        if self.compact_due && self.tasks.is_empty() {
            self.compact_due = false;
            self.compacting = true;
            self.turn_running = true;
            self.stopped = false;
            self.current = Some(Via::Maya);
            self.job = Some("tidying up my notes".into());
            self.in_flight = None;
            out.push(Out::Compact);
        } else if let Some(next) = self.queue.pop_front() {
            out.push(self.deliver(Some(next)));
        } else if self.wake_pending {
            out.push(self.deliver(None));
        }
    }

    fn deliver(&mut self, req: Option<Request>) -> Out {
        self.turn_running = true;
        self.stopped = false;
        self.wake_pending = false;
        self.current = Some(req.as_ref().map_or(Via::Maya, |r| r.via));
        self.job = Some(match &req {
            Some(r) if r.internal => CHECKING.to_string(),
            Some(r) if r.via != Via::Maya => short_job(&r.text),
            _ => LOOKING.to_string(),
        });
        self.in_flight = req.clone();
        Out::Deliver(req)
    }
}

/// The text cut to 60 characters on a word boundary.
fn short_job(text: &str) -> String {
    const MAX: usize = 60;
    let text = text.trim();
    if text.chars().count() <= MAX {
        return text.to_string();
    }
    let cut = text.char_indices().nth(MAX).map_or(text.len(), |(i, _)| i);
    let head = &text[..cut];
    if text[cut..].starts_with(char::is_whitespace) {
        return head.trim_end().to_string();
    }
    match head.rfind(char::is_whitespace) {
        Some(i) => head[..i].trim_end().to_string(),
        None => head.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(text: &str, via: Via) -> Request {
        Request::new(text, via)
    }

    #[test]
    fn an_idle_brain_gets_the_request_at_once() {
        let mut s = Session::new(true);
        assert_eq!(s.submit(req("make it a PR", Via::Voice)), vec![Out::Deliver(Some(req("make it a PR", Via::Voice)))]);
        assert!(s.working());
    }
    #[test]
    fn a_busy_brain_without_background_subagents_says_so_and_queues() {
        let mut s = Session::new(false);
        s.submit(req("refactor the parser", Via::Chat));
        s.on_event(&Event::TurnStarted, None);
        let out = s.submit(req("what's waiting on me?", Via::Voice));
        assert_eq!(out, vec![Out::Busy { via: Via::Voice, say: "I'm in the middle of refactor the parser, I'll do that next.".into() }]);
        let out = s.on_event(&Event::TurnEnded { text: "Done.".into(), context_tokens: None }, None);
        assert_eq!(out[0], Out::Reply { via: Via::Chat, say: "Done.".into(), actions: vec![], errors: vec![] });
        assert_eq!(out[1], Out::Deliver(Some(req("what's waiting on me?", Via::Voice))));
    }
    #[test]
    fn a_busy_brain_with_background_subagents_says_it_is_still_on_the_last_thing() {
        let mut s = Session::new(true);
        s.submit(req("a", Via::Chat));
        let still = |via| vec![Out::Busy { via, say: STILL_ON_IT.into() }];
        assert_eq!(s.submit(req("b", Via::Voice)), still(Via::Voice), "voice never goes dead");
        assert_eq!(s.submit(req("c", Via::Phone)), still(Via::Phone));
        assert_eq!(s.submit(req("d", Via::Chat)), still(Via::Chat), "posted in Chat");
        assert!(s.submit(Request::internal("x", Via::Voice)).is_empty(), "Maya's own follow-up queues quietly");
        assert!(s.submit(req("y", Via::Maya)).is_empty());
    }
    #[test]
    fn a_turn_she_starts_herself_replies_as_maya() {
        let mut s = Session::new(true);
        s.on_event(&Event::TurnStarted, None);
        let out = s.on_event(&Event::TurnEnded { text: "api-fix opened #52.".into(), context_tokens: None }, None);
        assert_eq!(out, vec![Out::Reply { via: Via::Maya, say: "api-fix opened #52.".into(), actions: vec![], errors: vec![] }]);
    }
    #[test]
    fn a_full_context_is_compacted_before_the_next_request() {
        let mut s = Session::new(true);
        s.submit(req("a", Via::Chat));
        s.submit(req("b", Via::Chat));
        let out = s.on_event(&Event::TurnEnded { text: "ok".into(), context_tokens: None }, Some(85));
        assert_eq!(out, vec![Out::Reply { via: Via::Chat, say: "ok".into(), actions: vec![], errors: vec![] }, Out::Compact]);
        let out = s.on_event(&Event::TurnEnded { text: "Compacted.".into(), context_tokens: None }, Some(10));
        assert_eq!(out, vec![Out::Deliver(Some(req("b", Via::Chat)))]);
    }
    #[test]
    fn compaction_waits_for_live_tasks() {
        let mut s = Session::new(true);
        s.submit(req("a", Via::Chat));
        s.on_event(&Event::TaskStarted("t".into()), None);
        let out = s.on_event(&Event::TurnEnded { text: "ok".into(), context_tokens: None }, Some(85));
        assert!(!out.contains(&Out::Compact));
        assert_eq!(s.on_event(&Event::TaskEnded("t".into()), None), vec![Out::Compact]);
    }
    #[test]
    fn a_submit_during_compaction_is_queued() {
        let mut s = Session::new(false);
        s.submit(req("a", Via::Chat));
        s.on_event(&Event::TurnEnded { text: "ok".into(), context_tokens: None }, Some(85));
        let out = s.submit(req("b", Via::Voice));
        assert_eq!(out, vec![Out::Busy { via: Via::Voice, say: "I'm in the middle of tidying up my notes, I'll do that next.".into() }]);
        let out = s.on_event(&Event::TurnEnded { text: "x".into(), context_tokens: None }, None);
        assert_eq!(out, vec![Out::Deliver(Some(req("b", Via::Voice)))]);
    }
    #[test]
    fn a_delivery_clears_a_pending_wake() {
        let mut s = Session::new(true);
        s.submit(req("a", Via::Chat));
        s.submit(req("b", Via::Chat));
        assert!(s.wake().is_empty());
        let out = s.on_event(&Event::TurnEnded { text: "x".into(), context_tokens: None }, None);
        assert_eq!(out[1], Out::Deliver(Some(req("b", Via::Chat))));
        let out = s.on_event(&Event::TurnEnded { text: "y".into(), context_tokens: None }, None);
        assert_eq!(out.len(), 1);
    }
    #[test]
    fn stop_with_only_tasks_live_interrupts_and_later_submits_deliver() {
        let mut s = Session::new(true);
        s.on_event(&Event::TaskStarted("t".into()), None);
        assert_eq!(s.stop(Via::Chat), vec![Out::Interrupt, Out::Said { via: Via::Chat, say: "Stopped.".into() }]);
        assert_eq!(s.submit(req("a", Via::Chat)), vec![Out::Deliver(Some(req("a", Via::Chat)))]);
    }
    #[test]
    fn stop_interrupts_and_clears_the_queue() {
        let mut s = Session::new(true);
        s.submit(req("a", Via::Chat));
        s.submit(req("b", Via::Chat));
        s.on_event(&Event::TurnStarted, None);
        assert_eq!(s.stop(Via::Voice), vec![Out::Interrupt, Out::Said { via: Via::Voice, say: "Stopped.".into() }]);
        assert!(s.drain().is_empty());
    }
    #[test]
    fn stop_with_nothing_running_says_nothing_to_stop() {
        assert_eq!(Session::new(true).stop(Via::Chat), vec![Out::Said { via: Via::Chat, say: "Nothing to stop.".into() }]);
    }
    #[test]
    fn the_job_is_cut_on_a_word_boundary() {
        let mut s = Session::new(true);
        let long = "please refactor the whole parser module so that every rule has its own function";
        s.submit(req(long, Via::Chat));
        let j = s.job().unwrap();
        assert!(j.len() <= 60 && long.starts_with(j) && long[j.len()..].starts_with(' '));
    }
    #[test]
    fn a_watch_fires_now_when_idle_and_after_the_turn_when_busy() {
        let mut s = Session::new(true);
        assert_eq!(s.wake(), vec![Out::Deliver(None)]);
        let out = s.on_event(&Event::TurnEnded { text: "x".into(), context_tokens: None }, None);
        assert_eq!(out.len(), 1);
        s.submit(req("a", Via::Chat));
        assert!(s.wake().is_empty());
        let out = s.on_event(&Event::TurnEnded { text: "x".into(), context_tokens: None }, None);
        assert_eq!(out[1], Out::Deliver(None));
    }
    #[test]
    fn tasks_keep_her_working_after_the_turn() {
        let mut s = Session::new(true);
        s.on_event(&Event::TaskStarted("t".into()), None);
        assert!(s.working());
        s.on_event(&Event::TaskEnded("t".into()), None);
        assert!(!s.working());
    }
    #[test]
    fn a_stopped_turns_reply_is_dropped() {
        let mut s = Session::new(true);
        s.submit(req("a", Via::Chat));
        s.on_event(&Event::TurnStarted, None);
        s.stop(Via::Chat);
        assert!(s.on_event(&Event::TurnEnded { text: "interrupted".into(), context_tokens: None }, None).is_empty());
    }
    #[test]
    fn exit_hands_back_the_in_flight_request_then_the_queue() {
        let mut s = Session::new(true);
        s.submit(req("a", Via::Chat));
        s.submit(req("b", Via::Voice));
        assert!(s.on_event(&Event::Exited("died".into()), None).is_empty());
        assert!(!s.working());
        assert_eq!(s.drain(), vec![req("a", Via::Chat), req("b", Via::Voice)]);
        assert!(s.drain().is_empty());
    }
    #[test]
    fn notes_keep_the_latest_fifty_and_say_when_older_ones_were_dropped() {
        let mut s = Session::new(true);
        for i in 0..53 {
            s.note(format!("n{i}"));
        }
        let notes = s.take_notes();
        assert_eq!(notes.len(), MAX_NOTES + 1);
        assert_eq!(notes[0], "(older changes dropped)");
        assert_eq!(notes[1], "n3");
        assert_eq!(notes.last().unwrap(), "n52");
        s.note("next".into());
        assert_eq!(s.take_notes(), vec!["next".to_string()], "the marker goes once");
    }
    #[test]
    fn a_skipped_compaction_delivers_the_next_request_and_swallows_nothing() {
        let mut s = Session::new(false);
        s.submit(req("a", Via::Chat));
        s.submit(req("b", Via::Voice));
        let out = s.on_event(&Event::TurnEnded { text: "ok".into(), context_tokens: None }, Some(90));
        assert_eq!(out.last(), Some(&Out::Compact));
        assert_eq!(s.compact_skipped(), vec![Out::Deliver(Some(req("b", Via::Voice)))]);
        let out = s.on_event(&Event::TurnEnded { text: "b done".into(), context_tokens: None }, Some(10));
        assert_eq!(out, vec![Out::Reply { via: Via::Voice, say: "b done".into(), actions: vec![], errors: vec![] }]);
    }
    #[test]
    fn a_skipped_compaction_goes_quiet_or_wakes_and_is_a_no_op_otherwise() {
        let mut s = Session::new(true);
        s.submit(req("a", Via::Chat));
        s.on_event(&Event::TurnEnded { text: "ok".into(), context_tokens: None }, Some(90));
        assert!(s.compact_skipped().is_empty());
        assert!(!s.working());
        assert!(s.compact_skipped().is_empty(), "nothing to skip");
        s.submit(req("c", Via::Chat));
        s.wake();
        s.on_event(&Event::TurnEnded { text: "ok".into(), context_tokens: None }, Some(90));
        assert_eq!(s.compact_skipped(), vec![Out::Deliver(None)]);
    }
    #[test]
    fn an_internal_request_queues_silently_under_a_fixed_label() {
        let mut s = Session::new(false);
        s.submit(req("a", Via::Chat));
        s.on_event(&Event::TurnStarted, None);
        let inner = Request::internal("<Maya> These actions were refused: x.", Via::Chat);
        assert!(s.submit(inner.clone()).is_empty(), "no busy line for her own follow-up");
        let out = s.on_event(&Event::TurnEnded { text: "ok".into(), context_tokens: None }, None);
        assert_eq!(out[1], Out::Deliver(Some(inner)));
        assert_eq!(s.job(), Some(CHECKING));
        let out = s.on_event(&Event::TurnEnded { text: "fixed".into(), context_tokens: None }, None);
        assert_eq!(out, vec![Out::Reply { via: Via::Chat, say: "fixed".into(), actions: vec![], errors: vec![] }], "answered the original way");
    }
    #[test]
    fn her_own_turns_are_labelled_and_never_busy() {
        let mut s = Session::new(false);
        s.submit(req("a", Via::Chat));
        assert!(s.submit(req("primer", Via::Maya)).is_empty());
        s.on_event(&Event::TurnEnded { text: "x".into(), context_tokens: None }, None);
        assert_eq!(s.job(), Some(LOOKING));
        s.on_event(&Event::TurnEnded { text: "".into(), context_tokens: None }, None);
        assert_eq!(s.wake(), vec![Out::Deliver(None)]);
        assert_eq!(s.job(), Some(LOOKING));
    }
    #[test]
    fn a_failed_turn_says_so_to_the_user_and_nothing_for_her_own() {
        let mut s = Session::new(true);
        s.submit(req("a", Via::Chat));
        assert_eq!(s.on_event(&Event::TurnFailed { context_tokens: None }, None), vec![Out::Reply { via: Via::Chat, say: TURN_FAILED.into(), actions: vec![], errors: vec![] }]);
        assert!(!s.working());
        s.on_event(&Event::TurnStarted, None);
        assert_eq!(s.on_event(&Event::TurnFailed { context_tokens: None }, None), vec![Out::Reply { via: Via::Maya, say: String::new(), actions: vec![], errors: vec![] }]);
        s.submit(req("b", Via::Voice));
        s.stop(Via::Voice);
        assert!(s.on_event(&Event::TurnFailed { context_tokens: None }, None).is_empty(), "an interrupted turn says nothing");
    }

    #[test]
    fn in_turn_is_her_turn_alone_not_her_tasks() {
        let mut s = Session::new(true);
        assert!(!s.in_turn());
        s.submit(req("a", Via::Chat));
        assert!(s.in_turn());
        s.on_event(&Event::TaskStarted("t".into()), None);
        s.on_event(&Event::TurnEnded { text: "ok".into(), context_tokens: None }, None);
        assert!(!s.in_turn());
        assert!(s.working(), "a task still runs");
    }

    #[test]
    fn notes_are_taken_once() {
        let mut s = Session::new(true);
        s.note("one".into());
        assert_eq!(s.take_notes(), vec!["one".to_string()]);
        assert!(s.take_notes().is_empty());
    }
}
