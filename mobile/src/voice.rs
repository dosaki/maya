//! The phone's voice conversation: the shared wake-word `Flow`, the turns
//! sent with each command, and what to say and when to listen. The main
//! interprets (`Request::Interpret`) and runs (`Request::Execute`); the
//! recogniser and the voice sit behind `VoiceIo` so this is tested on the
//! host. Nothing here listens or speaks while the app is off screen.

use maya_core::log;
use maya_core::net::protocol::{Request, VoiceTurn};
use maya_core::wake::{Effect, Flow, Pending};
use serde::Serialize;
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// The recogniser and the voice: Android's, or a fake in tests.
pub trait VoiceIo: Send + Sync {
    /// One utterance; results come back through `Voice::heard`/`ended`/`failed`.
    /// `wake` asks for the on-device recogniser the wake word loop needs.
    fn listen(&self, wake: bool);
    fn stop_listening(&self);
    /// Says `line`; `Voice::spoken` follows when it is done.
    fn speak(&self, line: &str);
    fn stop_speaking(&self);
}

pub type Ask = Arc<dyn Fn(Request) -> Result<Option<Value>, String> + Send + Sync>;
pub type Show = Arc<dyn Fn(&VoiceView) + Send + Sync>;

/// What the page shows.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceView {
    pub state: String,
    pub heard: String,
    pub said: String,
    /// The read-back waiting for yes or no.
    pub pending: Option<String>,
    pub error: Option<String>,
    /// Why voice is not offered; `None` when it is.
    pub offer: Option<String>,
    /// Shown while idle: tap-to-talk goes through an online recogniser.
    pub note: Option<String>,
}

/// Turns sent with each command (the interpreter pairs at most 6 of them).
const HISTORY: usize = 12;
/// Wake word failures in a row before the loop stops.
const MAX_FAILURES: u32 = 5;
/// The offer until the hub says otherwise: not joined to a main.
pub const NO_MAIN: &str = "Voice needs a desktop main.";

/// What to do once the current line has been spoken.
#[derive(Clone, Debug, PartialEq)]
enum After {
    Idle,
    /// Listen for a command (after "Yes?" or a question back).
    Command,
    /// Restart the confirmation window and listen for yes or no.
    Confirm(Pending),
}

struct Inner {
    flow: Flow,
    history: Vec<VoiceTurn>,
    on_screen: bool,
    wake: bool,
    offer: Option<String>,
    speaking: bool,
    thinking: bool,
    after: After,
    events: VecDeque<String>,
    generation: u64,
    failures: u32,
    backoffs: Vec<u64>,
    view: VoiceView,
}

pub struct Voice {
    inner: Mutex<Inner>,
    io: Arc<dyn VoiceIo>,
    ask: Ask,
    show: Show,
    now: fn() -> u64,
    /// Run requests on the calling thread (tests) instead of a spawned one.
    sync: bool,
    me: Mutex<std::sync::Weak<Voice>>,
}

impl Voice {
    pub fn new(io: Arc<dyn VoiceIo>, ask: Ask, show: Show, now: fn() -> u64) -> Arc<Voice> {
        Self::build(io, ask, show, now, false)
    }

    #[cfg(test)]
    fn new_sync(io: Arc<dyn VoiceIo>, ask: Ask, show: Show, now: fn() -> u64) -> Arc<Voice> {
        Self::build(io, ask, show, now, true)
    }

    fn build(io: Arc<dyn VoiceIo>, ask: Ask, show: Show, now: fn() -> u64, sync: bool) -> Arc<Voice> {
        let inner = Inner {
            flow: Flow::new(), history: vec![], on_screen: false, wake: false, offer: Some(NO_MAIN.into()), speaking: false, thinking: false,
            after: After::Idle, events: VecDeque::new(), generation: 0, failures: 0, backoffs: vec![], view: VoiceView { state: "off".into(), ..Default::default() },
        };
        let v = Arc::new(Voice { inner: Mutex::new(inner), io, ask, show, now, sync, me: Mutex::new(std::sync::Weak::new()) });
        *v.me.lock().unwrap() = Arc::downgrade(&v);
        v
    }

    pub fn view(&self) -> VoiceView {
        self.inner.lock().unwrap().view.clone()
    }

    fn active(i: &Inner) -> bool {
        i.on_screen && i.offer.is_none()
    }

    /// Recomputes the view's state and shows it.
    fn publish(&self, i: &mut Inner) {
        let now = (self.now)();
        i.view.state = if !Self::active(i) {
            "off"
        } else if i.speaking {
            "speaking"
        } else if i.thinking {
            "thinking"
        } else {
            match i.flow.state(now) {
                "idle" if i.wake => "listening",
                s => s,
            }
        }
        .to_string();
        i.view.pending = if i.flow.has_pending() { i.view.pending.clone() } else { None };
        i.view.offer = i.offer.clone();
        (self.show)(&i.view);
    }

    pub fn set_offer(&self, offer: Option<String>) {
        let mut i = self.inner.lock().unwrap();
        if i.offer == offer {
            return;
        }
        i.offer = offer;
        if !Self::active(&i) {
            self.reset(&mut i);
        } else if i.wake {
            self.io.listen(true);
        }
        self.publish(&mut i);
    }

    pub fn set_note(&self, note: Option<String>) {
        let mut i = self.inner.lock().unwrap();
        i.view.note = note;
        self.publish(&mut i);
    }

    pub fn set_wake(&self, on: bool) {
        let mut i = self.inner.lock().unwrap();
        i.wake = on;
        i.failures = 0;
        i.view.error = None;
        if Self::active(&i) && !i.speaking && !i.thinking {
            if on {
                self.io.listen(true);
            } else {
                self.io.stop_listening();
            }
        }
        self.publish(&mut i);
    }

    /// Off screen, or voice withdrawn: stop at once and forget the turn.
    fn reset(&self, i: &mut Inner) {
        i.generation += 1;
        i.flow = Flow::new();
        i.speaking = false;
        i.thinking = false;
        i.after = After::Idle;
        i.events.clear();
        i.view.pending = None;
        self.io.stop_listening();
        self.io.stop_speaking();
    }

    pub fn screen(&self, on: bool) {
        let mut i = self.inner.lock().unwrap();
        if i.on_screen == on {
            return;
        }
        i.on_screen = on;
        if !on {
            self.reset(&mut i);
        } else if Self::active(&i) && i.wake {
            self.io.listen(true);
        }
        self.publish(&mut i);
    }

    pub fn tap(&self) {
        let mut i = self.inner.lock().unwrap();
        // While the main thinks, anything heard would be thrown away.
        if !Self::active(&i) || i.thinking {
            return;
        }
        if i.speaking {
            self.io.stop_speaking();
            i.speaking = false;
        }
        i.flow.await_reply((self.now)());
        i.view.heard.clear();
        self.io.listen(false);
        self.publish(&mut i);
    }

    pub fn partial(&self, text: &str) {
        let mut i = self.inner.lock().unwrap();
        if !Self::active(&i) {
            return;
        }
        i.view.heard = text.to_string();
        self.publish(&mut i);
    }

    pub fn confirm(&self, yes: bool) {
        {
            let i = self.inner.lock().unwrap();
            if !i.flow.has_pending() {
                return;
            }
        }
        self.io.stop_listening();
        self.heard(if yes { "yes" } else { "no" });
    }

    pub fn heard(&self, text: &str) {
        let mut i = self.inner.lock().unwrap();
        if !Self::active(&i) || i.thinking {
            return;
        }
        i.failures = 0;
        i.view.heard = text.to_string();
        let effects = i.flow.on_segment(text, (self.now)());
        if effects.is_empty() {
            // Chatter, or Maya's own line: the recogniser has stopped, so
            // listen again (an open window, the wake loop) unless she speaks.
            if !i.speaking {
                self.carry_on(&mut i);
            }
            self.publish(&mut i);
            return;
        }
        for e in effects {
            match e {
                Effect::Say(s) => self.say(&mut i, &s, After::Command),
                Effect::Interpret(cmd) => {
                    // The history sent is what came before this command.
                    let history = last(&i.history);
                    push(&mut i.history, "user", &cmd);
                    self.request(i, Request::Interpret { command: cmd, history });
                    return;
                }
                Effect::Execute(action) => {
                    push(&mut i.history, "user", "yes");
                    i.view.pending = None;
                    self.request(i, Request::Execute { action });
                    return;
                }
                Effect::Cancelled => {
                    push(&mut i.history, "user", "no");
                    i.view.pending = None;
                    self.say(&mut i, "OK.", After::Idle);
                }
            }
        }
        self.publish(&mut i);
    }

    pub fn ended(&self) {
        let mut i = self.inner.lock().unwrap();
        if !Self::active(&i) || i.speaking || i.thinking {
            return;
        }
        let now = (self.now)();
        if i.flow.drop_expired(now) {
            i.view.pending = None;
            self.say(&mut i, "OK.", After::Idle);
        } else {
            self.carry_on(&mut i);
        }
        self.publish(&mut i);
    }

    pub fn failed(&self, message: &str, fatal: bool) {
        let mut i = self.inner.lock().unwrap();
        if !Self::active(&i) {
            return;
        }
        if fatal || !i.wake {
            i.view.error = Some(message.to_string());
            i.flow = Flow::new();
            self.publish(&mut i);
            return;
        }
        i.failures += 1;
        if i.failures >= MAX_FAILURES {
            i.view.error = Some(format!("Wake word stopped: {message}"));
            self.publish(&mut i);
            return;
        }
        let delay = 1_000u64 << (i.failures - 1);
        i.backoffs.push(delay);
        let generation = i.generation;
        drop(i);
        if !self.sync {
            let me = self.me.lock().unwrap().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(delay));
                if let Some(v) = me.upgrade() {
                    let i = v.inner.lock().unwrap();
                    if i.generation == generation && Self::active(&i) && i.wake && !i.speaking && !i.thinking {
                        v.io.listen(true);
                    }
                }
            });
        }
    }

    #[cfg(test)]
    fn backoffs(&self) -> Vec<u64> {
        self.inner.lock().unwrap().backoffs.clone()
    }

    pub fn spoken(&self, ok: bool) {
        let mut i = self.inner.lock().unwrap();
        if !ok {
            i.view.error = Some("No text-to-speech voice is installed; replies show here instead.".into());
        }
        if !i.speaking {
            return;
        }
        i.speaking = false;
        if !Self::active(&i) {
            return;
        }
        let now = (self.now)();
        match std::mem::replace(&mut i.after, After::Idle) {
            After::Confirm(p) => {
                i.flow.set_pending(p, now);
                self.io.listen(false);
            }
            After::Command => {
                i.flow.await_reply(now);
                self.io.listen(false);
            }
            After::Idle => self.carry_on(&mut i),
        }
        self.publish(&mut i);
    }

    /// Nothing is being said and the recogniser has stopped: listen again
    /// inside an open window, else speak the next waiting event, else go
    /// back to the wake loop when it is on.
    fn carry_on(&self, i: &mut Inner) {
        if i.flow.state((self.now)()) != "idle" {
            self.io.listen(false);
        } else if let Some(line) = i.events.pop_front() {
            self.say(i, &line, After::Idle);
        } else if i.wake {
            self.io.listen(true);
        }
    }

    /// Speaks a board event now, or once the conversation is idle. False
    /// when it will not be spoken at all (off screen, voice not offered).
    pub fn announce(&self, line: &str) -> bool {
        let mut i = self.inner.lock().unwrap();
        if !Self::active(&i) {
            return false;
        }
        let busy = i.speaking || i.thinking || i.flow.state((self.now)()) != "idle";
        if busy {
            i.events.push_back(line.to_string());
        } else {
            // Speaking cancels a running recognition on the Android side.
            self.say(&mut i, line, After::Idle);
            self.publish(&mut i);
        }
        true
    }

    fn say(&self, i: &mut Inner, line: &str, after: After) {
        i.flow.ignore_line(line);
        i.view.said = line.to_string();
        i.speaking = true;
        i.after = after;
        self.io.speak(line);
    }

    /// The generation a request started now carries; a test seam.
    #[cfg(test)]
    fn begin_request(&self) -> u64 {
        self.inner.lock().unwrap().generation
    }

    /// Sends `request` (with the history) and acts on the answer, on a
    /// thread unless `sync`. The lock is released while the main thinks.
    fn request(&self, mut i: std::sync::MutexGuard<'_, Inner>, request: Request) {
        let execute = matches!(request, Request::Execute { .. });
        // The recogniser has stopped: it hears one utterance per `listen`.
        i.thinking = true;
        let generation = i.generation;
        self.publish(&mut i);
        drop(i);
        let ask = self.ask.clone();
        let me = self.me.lock().unwrap().clone();
        let run = move || {
            let answer = ask(request);
            if let Some(v) = me.upgrade() {
                if execute {
                    v.finish_execute(generation, answer);
                } else {
                    v.finish_interpret(generation, answer);
                }
            }
        };
        if self.sync {
            run();
        } else {
            std::thread::spawn(run);
        }
    }

    fn finish_interpret(&self, generation: u64, answer: Result<Option<Value>, String>) {
        let mut i = self.inner.lock().unwrap();
        if i.generation != generation {
            log::line("voice", "an answer arrived after the conversation ended; dropped");
            return;
        }
        i.thinking = false;
        let data = match answer {
            Ok(Some(d)) => d,
            Ok(None) => Value::Null,
            Err(e) => {
                i.flow = Flow::new();
                self.say(&mut i, &e, After::Idle);
                self.publish(&mut i);
                return;
            }
        };
        let say = data["say"].as_str().unwrap_or("").to_string();
        push(&mut i.history, "maya", &say);
        let pending = data.get("pending").filter(|p| !p.is_null()).map(|p| Pending { say: p["say"].as_str().unwrap_or(&say).to_string(), action: p["action"].clone() });
        match pending {
            Some(p) => {
                i.flow.set_pending(p.clone(), (self.now)());
                let line = p.say.clone();
                i.view.pending = Some(line.clone());
                self.say(&mut i, &line, After::Confirm(p));
            }
            None if say.trim_end().ends_with('?') => self.say(&mut i, &say, After::Command),
            None if say.trim().is_empty() => self.carry_on(&mut i),
            None => self.say(&mut i, &say, After::Idle),
        }
        self.publish(&mut i);
    }

    fn finish_execute(&self, generation: u64, answer: Result<Option<Value>, String>) {
        let mut i = self.inner.lock().unwrap();
        if i.generation != generation {
            return;
        }
        i.thinking = false;
        let say = match answer {
            Ok(d) => d.and_then(|d| d["say"].as_str().map(str::to_string)).unwrap_or_else(|| "Done.".into()),
            Err(e) => e,
        };
        push(&mut i.history, "maya", &say);
        self.say(&mut i, &say, After::Idle);
        self.publish(&mut i);
    }
}

fn push(history: &mut Vec<VoiceTurn>, who: &str, text: &str) {
    history.push(VoiceTurn { who: who.into(), text: text.into() });
    let extra = history.len().saturating_sub(HISTORY * 2);
    history.drain(..extra);
}

fn last(history: &[VoiceTurn]) -> Vec<VoiceTurn> {
    history[history.len().saturating_sub(HISTORY)..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::Mutex;

    // Per test thread: cargo runs tests in parallel, and `new_sync` calls
    // `now` on the test's own thread.
    thread_local! {
        static NOW: Cell<u64> = const { Cell::new(1_000) };
    }
    fn now() -> u64 {
        NOW.with(|n| n.get())
    }
    fn advance(ms: u64) {
        NOW.with(|n| n.set(n.get() + ms));
    }

    #[derive(Default)]
    struct FakeIo {
        calls: Mutex<Vec<String>>,
    }
    impl FakeIo {
        fn take(&self) -> Vec<String> {
            std::mem::take(&mut self.calls.lock().unwrap())
        }
    }
    impl VoiceIo for FakeIo {
        fn listen(&self, wake: bool) {
            self.calls.lock().unwrap().push(if wake { "listen wake".into() } else { "listen".into() });
        }
        fn stop_listening(&self) {
            self.calls.lock().unwrap().push("stop listening".into());
        }
        fn speak(&self, line: &str) {
            self.calls.lock().unwrap().push(format!("say {line}"));
        }
        fn stop_speaking(&self) {
            self.calls.lock().unwrap().push("stop speaking".into());
        }
    }

    /// A voice whose requests answer with `answers` in order, synchronously.
    fn voice(answers: Vec<Result<Value, String>>) -> (Arc<Voice>, Arc<FakeIo>, Arc<Mutex<Vec<Request>>>) {
        let io = Arc::new(FakeIo::default());
        let asked = Arc::new(Mutex::new(vec![]));
        let queue = Arc::new(Mutex::new(answers));
        let (a, q) = (asked.clone(), queue.clone());
        let ask: Ask = Arc::new(move |r| {
            a.lock().unwrap().push(r);
            q.lock().unwrap().remove(0).map(Some)
        });
        let v = Voice::new_sync(io.clone(), ask, Arc::new(|_| {}), now);
        v.screen(true);
        v.set_offer(None);
        io.take();
        (v, io, asked)
    }

    #[test]
    fn a_tap_asks_the_main_and_speaks_the_answer() {
        let (v, io, asked) = voice(vec![Ok(serde_json::json!({"say": "Two sessions need you."}))]);
        v.tap();
        assert_eq!(io.take(), vec!["listen"]);
        v.heard("what needs me");
        assert!(matches!(&asked.lock().unwrap()[0], Request::Interpret { command, .. } if command == "what needs me"));
        assert_eq!(io.take(), vec!["say Two sessions need you."]);
        v.spoken(true);
        assert_eq!(v.view().state, "idle");
        assert_eq!(io.take(), Vec::<String>::new(), "no follow-up after a statement");
    }

    #[test]
    fn a_question_back_reopens_the_mic() {
        let (v, io, _) = voice(vec![Ok(serde_json::json!({"say": "Which one?"}))]);
        v.tap();
        v.heard("answer it");
        io.take();
        v.spoken(true);
        assert_eq!(io.take(), vec!["listen"]);
        assert_eq!(v.view().state, "awaiting-command");
    }

    #[test]
    fn a_read_back_then_yes_runs_the_action() {
        let action = serde_json::json!({"kind": "answer", "session": "s1", "askId": 42});
        let (v, io, asked) = voice(vec![
            Ok(serde_json::json!({"say": "Answering hexgrid with \"Yes\". Yes?", "pending": {"say": "Answering hexgrid with \"Yes\". Yes?", "action": action}})),
            Ok(serde_json::json!({"say": "Answered."})),
        ]);
        v.tap();
        io.take();
        v.heard("answer hexgrid yes");
        assert_eq!(io.take(), vec!["say Answering hexgrid with \"Yes\". Yes?"]);
        assert_eq!(v.view().pending.as_deref(), Some("Answering hexgrid with \"Yes\". Yes?"));
        v.spoken(true);
        assert_eq!(io.take(), vec!["listen"]);
        v.heard("yes");
        assert!(matches!(&asked.lock().unwrap()[1], Request::Execute { action: a } if a["askId"] == 42));
        assert_eq!(io.take(), vec!["say Answered."]);
        assert_eq!(v.view().pending, None);
    }

    #[test]
    fn the_buttons_answer_a_read_back_like_words() {
        let action = serde_json::json!({"kind": "reply", "session": "s1"});
        let (v, io, asked) = voice(vec![Ok(serde_json::json!({"say": "Telling x: y. Yes?", "pending": {"say": "Telling x: y. Yes?", "action": action}}))]);
        v.tap();
        v.heard("tell x y");
        v.spoken(true);
        io.take();
        v.confirm(false);
        assert_eq!(io.take(), vec!["stop listening", "say OK."]);
        assert_eq!(asked.lock().unwrap().len(), 1, "no means nothing runs");
    }

    #[test]
    fn no_answer_to_a_read_back_cancels() {
        let action = serde_json::json!({"kind": "reply", "session": "s1"});
        let (v, io, asked) = voice(vec![Ok(serde_json::json!({"say": "Telling x: y. Yes?", "pending": {"say": "Telling x: y. Yes?", "action": action}}))]);
        v.tap();
        v.heard("tell x y");
        v.spoken(true);
        io.take();
        advance(maya_core::wake::CONFIRM_WAIT_MS + 1);
        v.ended();
        assert_eq!(io.take(), vec!["say OK."]);
        assert_eq!(v.view().pending, None);
        assert_eq!(asked.lock().unwrap().len(), 1);
    }

    #[test]
    fn silence_after_a_tap_goes_idle() {
        let (v, io, asked) = voice(vec![]);
        v.tap();
        io.take();
        advance(maya_core::wake::COMMAND_WAIT_MS + 1);
        v.ended();
        assert_eq!(io.take(), Vec::<String>::new());
        assert_eq!(v.view().state, "idle");
        assert!(asked.lock().unwrap().is_empty());
    }

    #[test]
    fn the_wake_word_loops_and_answers_bare_maya_with_yes() {
        let (v, io, _) = voice(vec![]);
        v.set_wake(true);
        assert_eq!(io.take(), vec!["listen wake"]);
        v.heard("nothing for me here");
        assert_eq!(io.take(), vec!["listen wake"], "chatter without the word restarts the loop");
        v.heard("Maya");
        assert_eq!(io.take(), vec!["say Yes?"]);
        v.spoken(true);
        assert_eq!(io.take(), vec!["listen"]);
    }

    #[test]
    fn leaving_the_screen_stops_everything_and_drops_a_pending_action() {
        let action = serde_json::json!({"kind": "reply", "session": "s1"});
        let (v, io, _) = voice(vec![Ok(serde_json::json!({"say": "Telling x: y. Yes?", "pending": {"say": "Telling x: y. Yes?", "action": action}}))]);
        v.tap();
        v.heard("tell x y");
        io.take();
        v.screen(false);
        assert_eq!(io.take(), vec!["stop listening", "stop speaking"]);
        assert_eq!(v.view().state, "off");
        assert_eq!(v.view().pending, None);
        v.screen(true);
        v.heard("yes");
        assert_eq!(io.take(), Vec::<String>::new(), "a yes after coming back confirms nothing");
    }

    #[test]
    fn a_late_answer_after_leaving_the_screen_is_dropped() {
        let (v, io, _) = voice(vec![]);
        v.tap();
        let gen = v.begin_request();
        v.screen(false);
        io.take();
        v.finish_interpret(gen, Ok(Some(serde_json::json!({"say": "Too late."}))));
        assert_eq!(io.take(), Vec::<String>::new());
    }

    #[test]
    fn an_event_waits_for_the_conversation() {
        let action = serde_json::json!({"kind": "reply", "session": "s1"});
        let (v, io, _) = voice(vec![Ok(serde_json::json!({"say": "Telling x: y. Yes?", "pending": {"say": "Telling x: y. Yes?", "action": action}}))]);
        v.tap();
        v.heard("tell x y");
        io.take();
        assert!(v.announce("laptop needs a decision"));
        assert_eq!(io.take(), Vec::<String>::new(), "not during a read-back");
        v.confirm(false);
        assert_eq!(io.take(), vec!["stop listening", "say OK."]);
        v.spoken(true);
        assert_eq!(io.take(), vec!["say laptop needs a decision"]);
    }

    #[test]
    fn an_event_off_screen_is_not_spoken() {
        let (v, io, _) = voice(vec![]);
        v.screen(false);
        io.take();
        assert!(!v.announce("laptop needs a decision"));
        assert_eq!(io.take(), Vec::<String>::new());
    }

    #[test]
    fn no_tap_without_the_voice_cap() {
        let (v, io, asked) = voice(vec![]);
        v.set_offer(Some("Update Yhi to talk to Maya.".into()));
        io.take();
        v.tap();
        assert_eq!(io.take(), Vec::<String>::new());
        assert_eq!(v.view().offer.as_deref(), Some("Update Yhi to talk to Maya."));
        assert!(asked.lock().unwrap().is_empty());
    }

    #[test]
    fn the_main_failing_resets_the_conversation_with_its_reason() {
        let (v, io, _) = voice(vec![Err("Yhi did not answer in time.".into())]);
        v.tap();
        io.take();
        v.heard("what needs me");
        assert_eq!(io.take(), vec!["say Yhi did not answer in time."]);
    }

    #[test]
    fn the_wake_loop_backs_off_then_gives_up() {
        let (v, io, _) = voice(vec![]);
        v.set_wake(true);
        io.take();
        for _ in 0..4 {
            v.failed("busy", false);
        }
        assert_eq!(v.view().error, None);
        v.failed("busy", false);
        assert_eq!(v.view().error.as_deref(), Some("Wake word stopped: busy"));
        assert_eq!(v.backoffs(), vec![1_000, 2_000, 4_000, 8_000]);
    }

    #[test]
    fn the_note_shows_in_the_view() {
        let (v, _, _) = voice(vec![]);
        v.set_note(Some("Speech goes through Google's service and may leave the phone.".into()));
        assert_eq!(v.view().note.as_deref(), Some("Speech goes through Google's service and may leave the phone."));
    }

    #[test]
    fn history_keeps_the_last_twelve_turns() {
        let answers = (0..10).map(|i| Ok(serde_json::json!({"say": format!("a{i}")}))).collect();
        let (v, _, asked) = voice(answers);
        for i in 0..10 {
            v.tap();
            v.heard(&format!("q{i}"));
            v.spoken(true);
        }
        let Request::Interpret { history, .. } = &asked.lock().unwrap()[9] else { panic!() };
        assert_eq!(history.len(), 12);
        assert_eq!(history.last().unwrap().text, "a8");
    }

    #[test]
    fn chatter_during_a_read_back_keeps_the_mic_open() {
        let action = serde_json::json!({"kind": "reply", "session": "s1"});
        let (v, io, asked) = voice(vec![
            Ok(serde_json::json!({"say": "Telling x: y. Yes?", "pending": {"say": "Telling x: y. Yes?", "action": action}})),
            Ok(serde_json::json!({"say": "Sent."})),
        ]);
        v.tap();
        v.heard("tell x y");
        v.spoken(true);
        io.take();
        v.heard("hmm let me think");
        assert_eq!(io.take(), vec!["listen"], "the read-back still waits, so the mic reopens");
        assert_eq!(v.view().state, "awaiting-confirm");
        v.heard("yes");
        assert!(matches!(&asked.lock().unwrap()[1], Request::Execute { .. }));
    }

    #[test]
    fn an_event_queued_behind_a_question_is_spoken_when_the_window_closes() {
        let (v, io, _) = voice(vec![Ok(serde_json::json!({"say": "Which one?"}))]);
        v.tap();
        v.heard("answer it");
        v.spoken(true);
        io.take();
        assert!(v.announce("laptop needs a decision"));
        assert_eq!(io.take(), Vec::<String>::new(), "not while waiting for an answer");
        advance(maya_core::wake::COMMAND_WAIT_MS + 1);
        v.ended();
        assert_eq!(io.take(), vec!["say laptop needs a decision"]);
    }

    #[test]
    fn a_tap_while_the_main_thinks_does_nothing() {
        let io = Arc::new(FakeIo::default());
        let (release, wait) = std::sync::mpsc::channel::<()>();
        let wait = Mutex::new(wait);
        let ask: Ask = Arc::new(move |_| {
            let _ = wait.lock().unwrap().recv();
            Ok(Some(serde_json::json!({"say": "Done."})))
        });
        let v = Voice::new(io.clone(), ask, Arc::new(|_| {}), now);
        v.screen(true);
        v.set_offer(None);
        v.tap();
        v.heard("what needs me");
        io.take();
        assert_eq!(v.view().state, "thinking");
        v.tap();
        assert_eq!(io.take(), Vec::<String>::new(), "a mic opened now would be thrown away");
        assert_eq!(v.view().state, "thinking");
        let _ = release.send(());
    }
}
