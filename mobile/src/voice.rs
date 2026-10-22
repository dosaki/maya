//! The phone's voice conversation: the shared wake-word `Flow`, the turns
//! sent with each command, and what to say and when to listen. The main
//! interprets (`Request::Interpret`), and runs each announced action
//! (`Request::Execute`) once its 5-second window ends; the
//! recogniser and the voice sit behind `VoiceIo` so this is tested on the
//! host. Nothing here listens or speaks while the app is off screen.

use maya_core::log;
use maya_core::net::protocol::{Request, VoiceTurn};
use maya_core::wake::{Effect, Flow};
use maya_core::window::{self, Batch, Via, Window};
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
/// Whether a board event may be spoken now (Speak on, the phone not silenced).
pub type Gate = Arc<dyn Fn() -> bool + Send + Sync>;

/// What the page shows.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceView {
    pub state: String,
    pub heard: String,
    pub said: String,
    /// The announcement of the actions that run when the window ends.
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
/// A refused microphone; the page offers the app's settings for exactly
/// this text (`MIC_OFF` in `src/mobile/voice.ts`).
pub const MIC_OFF: &str = "Microphone access is off.";
/// Android has no text-to-speech voice to say a line with.
const NO_TTS: &str = "No text-to-speech voice is installed; replies show here instead.";
/// The wake loop gave up; its error stays until the setting is changed.
const WAKE_STOPPED: &str = "Wake word stopped: ";

/// What to do once the current line has been spoken.
#[derive(Clone, Debug, PartialEq)]
enum After {
    Idle,
    /// Listen for a command (after "Yes?" or a question back).
    Command,
    /// Start the clock of this batch and listen for a stop, a yes or a correction.
    Window(u64),
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
    /// `carry_on` found the conversation idle with an event waiting;
    /// `play_queued` asks the gate and speaks it. Busy meanwhile.
    playing: bool,
    generation: u64,
    /// The announced actions waiting for their window to end.
    window: Window,
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
    /// See `set_event_gate`; never called with `inner` held.
    gate: Mutex<Option<Gate>>,
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
            after: After::Idle, events: VecDeque::new(), playing: false, generation: 0, window: Window::default(), failures: 0, backoffs: vec![], view: VoiceView { state: "off".into(), ..Default::default() },
        };
        let v = Arc::new(Voice { inner: Mutex::new(inner), io, ask, show, now, sync, me: Mutex::new(std::sync::Weak::new()), gate: Mutex::new(None) });
        *v.me.lock().unwrap() = Arc::downgrade(&v);
        v
    }

    pub fn view(&self) -> VoiceView {
        self.inner.lock().unwrap().view.clone()
    }

    fn active(i: &Inner) -> bool {
        i.on_screen && i.offer.is_none()
    }

    /// Something worked: the last error goes, except a wake loop that gave
    /// up, which only `set_wake` clears.
    fn clear_error(i: &mut Inner) {
        if !i.view.error.as_deref().is_some_and(|e| e.starts_with(WAKE_STOPPED)) {
            i.view.error = None;
        }
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
        i.view.pending = i.window.pending().map(|b| b.say.clone());
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

    /// Asked before each waiting board event is spoken: one queued while
    /// Speak was on may not play once it is off or the phone is silenced.
    pub fn set_event_gate(&self, gate: Gate) {
        *self.gate.lock().unwrap() = Some(gate);
    }

    pub fn set_note(&self, note: Option<String>) {
        let mut i = self.inner.lock().unwrap();
        i.view.note = note;
        self.publish(&mut i);
    }

    /// The word to answer to, the main's; the conversation so far stays.
    pub fn set_wake_word(&self, word: &str) {
        self.inner.lock().unwrap().flow.set_word(word);
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
        i.flow.reset();
        i.window.cancel();
        i.speaking = false;
        i.thinking = false;
        i.after = After::Idle;
        i.events.clear();
        i.playing = false;
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

    /// The mic button. While Maya speaks it cuts her off and listens, for
    /// a stop or a yes when she was announcing actions. While a command
    /// window listens it stops listening; a waiting window stops
    /// listening too but keeps its actions and its Cancel button, and they
    /// still run when it ends. Otherwise it listens for one command.
    pub fn tap(&self) {
        self.tap_now();
        self.play_queued();
    }

    fn tap_now(&self) {
        let mut i = self.inner.lock().unwrap();
        // While the main thinks, anything heard would be thrown away.
        if !Self::active(&i) || i.thinking {
            return;
        }
        Self::clear_error(&mut i);
        let now = (self.now)();
        if i.speaking {
            self.io.stop_speaking();
            i.speaking = false;
            if let After::Window(id) = std::mem::replace(&mut i.after, After::Idle) {
                self.start_window(&mut i, id);
                self.io.listen(false);
                self.publish(&mut i);
                return;
            }
        } else {
            if let Some(batch) = Self::take_due(&mut i, (self.now)()) {
                self.io.stop_listening();
                self.execute_batch(i, batch.actions);
                return;
            }
            if i.window.pending().is_some() {
                self.io.stop_listening();
                self.publish(&mut i);
                return;
            }
            if i.flow.state(now) == "awaiting-command" {
                i.flow.reset();
                self.io.stop_listening();
                self.carry_on(&mut i);
                self.publish(&mut i);
                return;
            }
        }
        i.flow.await_reply(now);
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
        // Still talking: a command window lasts until the words stop. An
        // announced batch keeps its own deadline.
        let now = (self.now)();
        if i.flow.state(now) == "awaiting-command" {
            i.flow.await_reply(now);
        }
        self.publish(&mut i);
    }

    /// The Cancel button: the announced actions are dropped, as by "stop".
    pub fn cancel(&self) {
        {
            let mut i = self.inner.lock().unwrap();
            if i.window.pending().is_none() || !Self::active(&i) {
                return;
            }
            i.window.cancel();
            i.flow.close_window();
            push(&mut i.history, "user", "no");
            // Her announcement may still be playing: the cancel cuts it off.
            if i.speaking {
                self.io.stop_speaking();
                i.speaking = false;
            }
            self.io.stop_listening();
            self.say(&mut i, "Cancelled.", After::Idle);
            self.publish(&mut i);
        }
        self.play_queued();
    }

    pub fn heard(&self, text: &str) {
        self.heard_now(text);
        self.play_queued();
    }

    fn heard_now(&self, text: &str) {
        let mut i = self.inner.lock().unwrap();
        if !Self::active(&i) || i.thinking {
            return;
        }
        i.failures = 0;
        Self::clear_error(&mut i);
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
        let say_follows = effects.iter().any(|e| matches!(e, Effect::Say(_)));
        for e in effects {
            match e {
                Effect::Say(s) => self.say(&mut i, &s, After::Command),
                Effect::Interpret(cmd) => {
                    // A new request never joins an old batch.
                    i.window.cancel();
                    // The history sent is what came before this command.
                    let history = last(&i.history);
                    push(&mut i.history, "user", &cmd);
                    self.request(i, Request::Interpret { command: cmd, history });
                    return;
                }
                Effect::RunNow => {
                    push(&mut i.history, "user", "yes");
                    match i.window.take_now() {
                        Some(batch) => self.execute_batch(i, batch.actions),
                        None => {
                            self.carry_on(&mut i);
                            self.publish(&mut i);
                        }
                    }
                    return;
                }
                Effect::Correct(cmd) => {
                    // Cancelled silently: what she heard is a new command.
                    i.window.cancel();
                    let history = last(&i.history);
                    push(&mut i.history, "user", &cmd);
                    self.request(i, Request::Interpret { command: cmd, history });
                    return;
                }
                Effect::Cancelled => {
                    i.window.cancel();
                    push(&mut i.history, "user", "no");
                    // A bare wake word cancels and asks "Yes?" in the same breath.
                    if !say_follows {
                        self.say(&mut i, "Cancelled.", After::Idle);
                    }
                }
                Effect::Stop => {
                    // A batch left open without its window dies with the stop.
                    if i.window.cancel().is_some() {
                        push(&mut i.history, "user", "no");
                        self.say(&mut i, "Cancelled.", After::Idle);
                    } else {
                        self.carry_on(&mut i);
                    }
                }
            }
        }
        self.publish(&mut i);
    }

    pub fn ended(&self) {
        self.ended_now();
        self.play_queued();
    }

    fn ended_now(&self) {
        let mut i = self.inner.lock().unwrap();
        if !Self::active(&i) || i.speaking || i.thinking {
            return;
        }
        // The recogniser ran to its end: the failures are no longer in a row.
        i.failures = 0;
        if let Some(batch) = Self::take_due(&mut i, (self.now)()) {
            self.execute_batch(i, batch.actions);
            return;
        }
        self.carry_on(&mut i);
        self.publish(&mut i);
    }

    pub fn failed(&self, message: &str, fatal: bool) {
        self.failed_now(message, fatal);
        self.play_queued();
    }

    fn failed_now(&self, message: &str, fatal: bool) {
        let mut i = self.inner.lock().unwrap();
        if !Self::active(&i) {
            return;
        }
        // Inside a tap, follow-up or announcement window the failed
        // recogniser was that window's, not the wake loop's: end the window
        // as without the wake word, and carry on (the wake loop resumes).
        let window = i.flow.state((self.now)()) != "idle";
        if fatal || !i.wake || window {
            i.view.error = Some(message.to_string());
            i.flow.reset();
            // With the window's ear gone nothing could stop the batch: drop it.
            let dropped = i.window.cancel().is_some();
            if dropped {
                push(&mut i.history, "user", "no");
                if i.speaking {
                    self.io.stop_speaking();
                    i.speaking = false;
                }
            }
            // A fatal error would fail again at once: no wake loop this session.
            if fatal {
                i.wake = false;
            }
            if dropped && !i.thinking {
                self.say(&mut i, "Cancelled.", After::Idle);
            } else if !i.speaking && !i.thinking {
                self.carry_on(&mut i);
            }
            self.publish(&mut i);
            return;
        }
        i.failures += 1;
        if i.failures >= MAX_FAILURES {
            // Off for this session; `set_wake` turns it back on.
            i.wake = false;
            i.view.error = Some(format!("{WAKE_STOPPED}{message}"));
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
                    v.retry_wake(generation);
                }
            });
        }
    }

    /// A backoff ran out: restart the wake loop if nothing else has taken
    /// the recogniser or the conversation since. A tap, a question's reply
    /// or an announced batch listens in a window of its own; the wake loop
    /// comes back by itself once that window closes, since every way one
    /// closes carries on: heard, ended, a failure (`failed` ends the
    /// window), a second tap, or a batch's own expiry timer.
    fn retry_wake(&self, generation: u64) {
        let i = self.inner.lock().unwrap();
        let idle = i.flow.state((self.now)()) == "idle";
        if i.generation == generation && Self::active(&i) && i.wake && !i.speaking && !i.thinking && idle {
            self.io.listen(true);
        }
    }

    /// The announcement of batch `id` has been said (or cut off): its
    /// 5 seconds start now, the flow listens for a stop, a yes or a
    /// correction, and a timer runs the batch even when nothing is listening
    /// (a tap stopped the mic). The timer thread holds only a `Weak`; the
    /// generation and the batch id tell it whether this is still the batch
    /// waiting.
    fn start_window(&self, i: &mut Inner, id: u64) {
        let Some(until) = i.window.start_clock(id, (self.now)()) else { return };
        i.flow.open_window(until);
        if self.sync {
            return;
        }
        let generation = i.generation;
        let me = self.me.lock().unwrap().clone();
        std::thread::spawn(move || {
            let mut wait = window::WAIT_MS + 1;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(wait));
                match me.upgrade() {
                    Some(v) if v.expire_window(generation, id) => wait = 250,
                    _ => break,
                }
            }
        });
    }

    /// The open batch, when its deadline has come.
    fn take_due(i: &mut Inner, now: u64) -> Option<Batch> {
        let id = i.window.pending()?.id;
        i.window.take_due(id, now)
    }

    /// A batch's timer ran out: run its actions. True when the batch is
    /// still waiting (a clock that lagged the sleep): try again.
    fn expire_window(&self, generation: u64, id: u64) -> bool {
        let mut i = self.inner.lock().unwrap();
        if i.generation != generation || !Self::active(&i) {
            return false;
        }
        match i.window.take_due(id, (self.now)()) {
            Some(batch) => {
                self.execute_batch(i, batch.actions);
                false
            }
            None => i.window.pending().is_some_and(|b| b.id == id),
        }
    }

    /// The generation and batch id a window's timer now carries; a test seam.
    #[cfg(test)]
    fn window_timer(&self) -> (u64, u64) {
        let i = self.inner.lock().unwrap();
        (i.generation, i.window.pending().map_or(0, |b| b.id))
    }

    #[cfg(test)]
    fn backoffs(&self) -> Vec<u64> {
        self.inner.lock().unwrap().backoffs.clone()
    }

    pub fn spoken(&self, ok: bool) {
        self.spoken_now(ok);
        self.play_queued();
    }

    fn spoken_now(&self, ok: bool) {
        let mut i = self.inner.lock().unwrap();
        // Off screen Android refuses to speak at all: not a missing voice.
        let active = Self::active(&i);
        if active {
            let error = i.view.error.as_deref();
            if ok {
                // Speaking proves the voice, not the recogniser or the microphone.
                if error == Some(NO_TTS) {
                    i.view.error = None;
                }
            } else if error != Some(MIC_OFF) && !error.is_some_and(|e| e.starts_with(WAKE_STOPPED)) {
                // A refused microphone keeps its Open settings button, and a
                // stopped wake loop its error until the setting changes.
                i.view.error = Some(NO_TTS.into());
            }
        }
        if !i.speaking {
            if active {
                self.publish(&mut i);
            }
            return;
        }
        i.speaking = false;
        if !active {
            return;
        }
        let now = (self.now)();
        match std::mem::replace(&mut i.after, After::Idle) {
            After::Window(id) => {
                self.start_window(&mut i, id);
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
    /// inside an open window, else speak the next waiting event (through
    /// `play_queued`, once the lock is released), else go back to the wake
    /// loop when it is on.
    fn carry_on(&self, i: &mut Inner) {
        if i.flow.state((self.now)()) != "idle" {
            self.io.listen(false);
        } else if !i.events.is_empty() {
            i.playing = true;
        } else if i.wake {
            self.io.listen(true);
        }
    }

    /// Speaks the next waiting event once `carry_on` has found the
    /// conversation idle, if the gate still allows events. Called by every
    /// entry point that can carry on, after it has released the lock. The
    /// gate calls into Android: never under the lock, and on a thread of its
    /// own (unless `sync`), not on the plugin's event thread that called in.
    fn play_queued(&self) {
        let generation = {
            let i = self.inner.lock().unwrap();
            if !i.playing {
                return;
            }
            i.generation
        };
        let gate = self.gate.lock().unwrap().clone();
        let inline = self.sync || gate.is_none();
        let me = self.me.lock().unwrap().clone();
        let run = move || {
            let allowed = gate.map_or(true, |g| g());
            if let Some(v) = me.upgrade() {
                v.play_next(generation, allowed);
            }
        };
        if inline {
            run();
        } else {
            std::thread::spawn(run);
        }
    }

    fn play_next(&self, generation: u64, allowed: bool) {
        let mut i = self.inner.lock().unwrap();
        if !std::mem::take(&mut i.playing) || i.generation != generation {
            return;
        }
        // Something took over meanwhile; it carries on when it is done.
        if !Self::active(&i) || i.speaking || i.thinking || i.flow.state((self.now)()) != "idle" {
            return;
        }
        if !allowed && !i.events.is_empty() {
            log::line("voice", format!("{} waiting board events dropped: Speak is off or the phone is silenced", i.events.len()));
            i.events.clear();
        }
        match i.events.pop_front() {
            Some(line) => self.say(&mut i, &line, After::Idle),
            None if i.wake => self.io.listen(true),
            None => {}
        }
        self.publish(&mut i);
    }

    /// Speaks a board event now, or once the conversation is idle. False
    /// when it will not be spoken at all (off screen, voice not offered).
    pub fn announce(&self, line: &str) -> bool {
        let mut i = self.inner.lock().unwrap();
        if !Self::active(&i) {
            return false;
        }
        let busy = i.speaking || i.thinking || i.playing || i.flow.state((self.now)()) != "idle";
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
        // The user has moved on: an announcement still being said must not
        // start a window when it finishes.
        i.after = After::Idle;
        if i.speaking {
            self.io.stop_speaking();
            i.speaking = false;
        }
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
                v.finish_interpret(generation, answer);
            }
        };
        if self.sync {
            run();
        } else {
            std::thread::spawn(run);
        }
    }

    /// Runs the actions of a batch whose window ended (or that got a yes),
    /// one `Execute` each, in order, on a thread unless `sync`. Only what
    /// went wrong is spoken afterwards.
    fn execute_batch(&self, mut i: std::sync::MutexGuard<'_, Inner>, actions: Vec<Value>) {
        i.flow.close_window();
        i.after = After::Idle;
        if i.speaking {
            self.io.stop_speaking();
            i.speaking = false;
        }
        i.thinking = true;
        let generation = i.generation;
        self.io.stop_listening();
        self.publish(&mut i);
        drop(i);
        let ask = self.ask.clone();
        let me = self.me.lock().unwrap().clone();
        let run = move || {
            let failures: Vec<String> = actions.into_iter().filter_map(|action| ask(Request::Execute { action }).err()).collect();
            if let Some(v) = me.upgrade() {
                v.finish_execute(generation, failures);
            }
        };
        if self.sync {
            run();
        } else {
            std::thread::spawn(run);
        }
    }

    fn finish_interpret(&self, generation: u64, answer: Result<Option<Value>, String>) {
        self.finish_interpret_now(generation, answer);
        self.play_queued();
    }

    fn finish_interpret_now(&self, generation: u64, answer: Result<Option<Value>, String>) {
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
                i.flow.reset();
                self.say(&mut i, &e, After::Idle);
                self.publish(&mut i);
                return;
            }
        };
        let say = data["say"].as_str().unwrap_or("").to_string();
        // The announced actions; a main from before the window sends only `action`.
        let announced = data.get("pending").filter(|p| !p.is_null()).and_then(|p| {
            let actions: Vec<Value> = match p["actions"].as_array() {
                Some(list) => list.clone(),
                None if !p["action"].is_null() => vec![p["action"].clone()],
                None => vec![],
            };
            (!actions.is_empty()).then(|| (p["say"].as_str().unwrap_or(&say).to_string(), actions))
        });
        // The turn is what Maya says: the announcement when there is one.
        push(&mut i.history, "maya", announced.as_ref().map_or(&say, |(line, _)| line));
        match announced {
            Some((line, actions)) => {
                // Held while it is announced; `spoken` starts its clock.
                // Every batch here comes from the phone: nothing is cancelled.
                let (id, _) = i.window.open(line.clone(), actions, Via::Phone);
                i.view.pending = Some(line.clone());
                self.say(&mut i, &line, After::Window(id));
            }
            None if say.trim_end().ends_with('?') => self.say(&mut i, &say, After::Command),
            None if say.trim().is_empty() => self.carry_on(&mut i),
            None => self.say(&mut i, &say, After::Idle),
        }
        self.publish(&mut i);
    }

    fn finish_execute(&self, generation: u64, failures: Vec<String>) {
        self.finish_execute_now(generation, failures);
        self.play_queued();
    }

    fn finish_execute_now(&self, generation: u64, failures: Vec<String>) {
        let mut i = self.inner.lock().unwrap();
        if i.generation != generation {
            return;
        }
        i.thinking = false;
        if failures.is_empty() {
            self.carry_on(&mut i);
        } else {
            let say = failures.join(" ");
            push(&mut i.history, "maya", &say);
            self.say(&mut i, &say, After::Idle);
        }
        self.publish(&mut i);
    }
}

/// One event from the voice plugin (`{event, …}`) onto the conversation.
pub fn dispatch(v: &Voice, e: &Value) {
    let text = || e["text"].as_str().unwrap_or("");
    match e["event"].as_str().unwrap_or("") {
        "partial" => v.partial(text()),
        "final" => v.heard(text()),
        "ended" => v.ended(),
        "failed" => v.failed(e["message"].as_str().unwrap_or("speech recognition failed"), e["fatal"].as_bool().unwrap_or(false)),
        "spoken" => v.spoken(e["ok"].as_bool().unwrap_or(true)),
        "screen" => v.screen(e["on"].as_bool().unwrap_or(false)),
        other => log::line("voice", format!("ignored plugin event {other:?}")),
    }
}

fn push(history: &mut Vec<VoiceTurn>, who: &str, text: &str) {
    if text.trim().is_empty() {
        return;
    }
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

    /// What the main answers to a command it announces actions for.
    fn announce_answer(say: &str, actions: Vec<Value>) -> Result<Value, String> {
        Ok(serde_json::json!({"say": say, "pending": {"say": say, "actions": actions, "action": actions.first()}}))
    }

    fn reply() -> Value {
        serde_json::json!({"kind": "reply", "session": "s1"})
    }

    /// How many `Execute` requests the main has been sent.
    fn executed(asked: &Mutex<Vec<Request>>) -> usize {
        asked.lock().unwrap().iter().filter(|r| matches!(r, Request::Execute { .. })).count()
    }

    /// A voice whose requests answer with `answers` in order, synchronously.
    fn voice(answers: Vec<Result<Value, String>>) -> (Arc<Voice>, Arc<FakeIo>, Arc<Mutex<Vec<Request>>>) {
        let io = Arc::new(FakeIo::default());
        let asked = Arc::new(Mutex::new(vec![]));
        let queue = Arc::new(Mutex::new(answers));
        let (a, q) = (asked.clone(), queue.clone());
        let ask: Ask = Arc::new(move |r| {
            let execute = matches!(r, Request::Execute { .. });
            a.lock().unwrap().push(r);
            // An action that runs succeeds; the answers are for the commands.
            if execute {
                return Ok(Some(serde_json::json!({"say": "Sent."})));
            }
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
    fn an_announced_action_runs_after_the_window_unless_cancelled() {
        let (v, io, asked) = voice(vec![announce_answer("Telling x: y.", vec![reply()])]);
        v.tap();
        v.heard("tell x y");
        assert_eq!(io.take(), vec!["listen", "say Telling x: y."]);
        assert_eq!(v.view().pending.as_deref(), Some("Telling x: y."));
        v.spoken(true);
        assert_eq!(io.take(), vec!["listen"]);
        assert_eq!(v.view().state, "window");
        advance(window::WAIT_MS - 1);
        v.ended();
        assert_eq!(executed(&asked), 0, "not before the 5 seconds");
        io.take();
        advance(2);
        v.ended();
        assert_eq!(executed(&asked), 1);
        assert_eq!(v.view().pending, None);
        assert_eq!(io.take(), vec!["stop listening"], "nothing is spoken for a success");
    }

    #[test]
    fn the_clock_starts_when_the_announcement_ends() {
        let (v, io, asked) = voice(vec![announce_answer("Telling x: y.", vec![reply()])]);
        v.tap();
        v.heard("tell x y");
        advance(window::WAIT_MS + 1);
        v.spoken(true);
        io.take();
        v.ended();
        assert_eq!(executed(&asked), 0, "a long announcement does not eat the window");
        advance(window::WAIT_MS - 1);
        v.heard("yes");
        assert_eq!(executed(&asked), 1);
    }

    #[test]
    fn no_during_the_window_cancels_the_announced_action() {
        let (v, io, asked) = voice(vec![announce_answer("Telling x: y.", vec![reply()])]);
        v.tap();
        v.heard("tell x y");
        v.spoken(true);
        io.take();
        v.heard("no");
        assert_eq!(io.take(), vec!["say Cancelled."]);
        assert_eq!(v.view().pending, None);
        advance(window::WAIT_MS + 1);
        v.ended();
        assert_eq!(executed(&asked), 0);
    }

    #[test]
    fn yes_during_the_window_runs_the_batch_at_once() {
        let (v, _, asked) = voice(vec![announce_answer("Telling x: y.", vec![reply()])]);
        v.tap();
        v.heard("tell x y");
        v.spoken(true);
        v.heard("yes");
        assert_eq!(executed(&asked), 1);
        assert_eq!(v.view().pending, None);
    }

    #[test]
    fn something_else_in_the_window_cancels_silently_and_is_a_new_command() {
        let (v, io, asked) = voice(vec![
            announce_answer("Telling x: y.", vec![reply()]),
            Ok(serde_json::json!({"say": "Two sessions need you."})),
        ]);
        v.tap();
        v.heard("tell x y");
        v.spoken(true);
        io.take();
        v.heard("actually what needs me");
        assert!(matches!(&asked.lock().unwrap()[1], Request::Interpret { command, .. } if command == "actually what needs me"));
        assert_eq!(io.take(), vec!["say Two sessions need you."], "no Cancelled. in between");
        v.spoken(true);
        advance(window::WAIT_MS + 1);
        v.ended();
        assert_eq!(executed(&asked), 0);
    }

    #[test]
    fn the_cancel_button_drops_the_batch_even_while_it_is_announced() {
        let (v, io, asked) = voice(vec![announce_answer("Telling x: y.", vec![reply()])]);
        v.tap();
        v.heard("tell x y");
        io.take();
        v.cancel();
        assert_eq!(io.take(), vec!["stop speaking", "stop listening", "say Cancelled."]);
        assert_eq!(v.view().pending, None);
        v.spoken(true);
        v.ended();
        advance(window::WAIT_MS + 1);
        v.ended();
        assert_eq!(executed(&asked), 0);
        v.cancel();
        assert_eq!(io.take(), Vec::<String>::new(), "nothing to cancel");
    }

    #[test]
    fn a_recogniser_failure_in_the_window_drops_the_batch() {
        let (v, io, asked) = voice(vec![announce_answer("Telling x: y.", vec![reply()])]);
        v.tap();
        v.heard("tell x y");
        v.spoken(true);
        io.take();
        v.failed("the recogniser is busy", false);
        assert_eq!(io.take(), vec!["say Cancelled."]);
        assert_eq!(v.view().pending, None);
        advance(window::WAIT_MS + 1);
        v.ended();
        assert_eq!(executed(&asked), 0);
    }

    #[test]
    fn a_stop_cancels_a_batch_left_open_without_its_window() {
        let (v, io, asked) = voice(vec![announce_answer("Telling x: y.", vec![reply()])]);
        v.tap();
        v.heard("tell x y");
        v.spoken(true);
        v.inner.lock().unwrap().flow.reset();
        io.take();
        v.heard("Maya stop");
        assert_eq!(io.take(), vec!["say Cancelled."]);
        advance(window::WAIT_MS + 1);
        v.ended();
        assert_eq!(executed(&asked), 0);
    }

    #[test]
    fn a_new_command_never_joins_an_old_batch() {
        let one = serde_json::json!({"kind": "reply", "n": 1});
        let two = serde_json::json!({"kind": "reply", "n": 2});
        let (v, _, asked) = voice(vec![announce_answer("One.", vec![one]), announce_answer("Two.", vec![two])]);
        v.tap();
        v.heard("tell x y");
        v.spoken(true);
        v.inner.lock().unwrap().flow.reset();
        v.heard("Maya tell z w");
        v.spoken(true);
        advance(window::WAIT_MS + 1);
        v.ended();
        let asked = asked.lock().unwrap();
        let ran: Vec<u64> = asked.iter().filter_map(|r| match r { Request::Execute { action } => action["n"].as_u64(), _ => None }).collect();
        assert_eq!(ran, vec![2]);
    }

    #[test]
    fn every_announced_action_runs_in_order() {
        let (v, _, asked) = voice(vec![announce_answer("Two things.", vec![serde_json::json!({"kind": "reply", "n": 1}), serde_json::json!({"kind": "compact", "n": 2})])]);
        v.tap();
        v.heard("do two things");
        v.spoken(true);
        advance(window::WAIT_MS + 1);
        v.ended();
        let asked = asked.lock().unwrap();
        let order: Vec<u64> = asked.iter().filter_map(|r| match r { Request::Execute { action } => action["n"].as_u64(), _ => None }).collect();
        assert_eq!(order, vec![1, 2]);
    }

    #[test]
    fn a_main_from_before_the_window_sends_only_the_first_action() {
        let old = Ok(serde_json::json!({"say": "Telling x: y.", "pending": {"say": "Telling x: y.", "action": reply()}}));
        let (v, _, asked) = voice(vec![old]);
        v.tap();
        v.heard("tell x y");
        v.spoken(true);
        advance(window::WAIT_MS + 1);
        v.ended();
        assert_eq!(executed(&asked), 1);
    }

    #[test]
    fn only_failures_are_spoken_after_the_actions_run() {
        let answers = vec![announce_answer("Two things.", vec![reply(), reply(), reply()])];
        let io = Arc::new(FakeIo::default());
        let asked = Arc::new(Mutex::new(vec![]));
        let queue = Arc::new(Mutex::new(answers));
        let (a, q) = (asked.clone(), queue.clone());
        let ask: Ask = Arc::new(move |r| {
            let execute = matches!(r, Request::Execute { .. });
            let n = a.lock().unwrap().len();
            a.lock().unwrap().push(r);
            if !execute {
                return q.lock().unwrap().remove(0).map(Some);
            }
            // The second action fails; the others succeed with a line nobody hears.
            if n == 2 { Err("Hexgrid has gone.".into()) } else { Ok(Some(serde_json::json!({"say": "Sent."}))) }
        });
        let v = Voice::new_sync(io.clone(), ask, Arc::new(|_| {}), now);
        v.screen(true);
        v.set_offer(None);
        v.tap();
        v.heard("do three things");
        v.spoken(true);
        io.take();
        advance(window::WAIT_MS + 1);
        v.ended();
        assert_eq!(io.take(), vec!["stop listening", "say Hexgrid has gone."]);
        assert_eq!(executed(&asked), 3, "a failure does not stop the others");
    }

    #[test]
    fn a_partial_resume_all_failure_from_the_main_is_spoken() {
        let (v, io, _) = voice(vec![announce_answer("Resuming all.", vec![serde_json::json!({"kind": "resume_all"})])]);
        v.tap();
        v.heard("resume everything");
        v.spoken(true);
        io.take();
        // The main's `Execute` answers a resume_all that left sessions behind with an error.
        v.finish_execute(v.begin_request(), vec!["Resumed 2; 1 could not be resumed.".into()]);
        assert_eq!(io.take(), vec!["say Resumed 2; 1 could not be resumed."]);
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
    fn the_wake_word_follows_the_main_and_survives_leaving_the_screen() {
        let (v, io, _) = voice(vec![]);
        v.set_wake_word("Jarvis");
        v.set_wake(true);
        io.take();
        v.heard("Maya");
        assert_eq!(io.take(), vec!["listen wake"], "Maya is chatter once the main's word is Jarvis");
        v.heard("Jarvis");
        assert_eq!(io.take(), vec!["say Yes?"]);
        v.spoken(true);
        io.take();
        v.screen(false);
        v.screen(true);
        io.take();
        v.heard("Jarvis");
        assert_eq!(io.take(), vec!["say Yes?"], "the word outlives a reset");
    }

    #[test]
    fn leaving_the_screen_stops_everything_and_drops_an_announced_action() {
        let (v, io, asked) = voice(vec![announce_answer("Telling x: y.", vec![reply()])]);
        v.tap();
        v.heard("tell x y");
        io.take();
        v.screen(false);
        assert_eq!(io.take(), vec!["stop listening", "stop speaking"]);
        assert_eq!(v.view().state, "off");
        assert_eq!(v.view().pending, None);
        v.screen(true);
        v.heard("yes");
        assert_eq!(io.take(), Vec::<String>::new(), "a yes after coming back runs nothing");
        advance(window::WAIT_MS + 1);
        v.ended();
        assert_eq!(executed(&asked), 0);
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
        let (v, io, _) = voice(vec![announce_answer("Telling x: y.", vec![reply()])]);
        v.tap();
        v.heard("tell x y");
        io.take();
        assert!(v.announce("laptop needs a decision"));
        assert_eq!(io.take(), Vec::<String>::new(), "not during an announcement");
        v.cancel();
        assert_eq!(io.take(), vec!["stop speaking", "stop listening", "say Cancelled."]);
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

    #[test]
    fn only_failures_in_a_row_stop_the_wake_loop_and_it_stays_stopped() {
        let (v, io, _) = voice(vec![]);
        v.set_wake(true);
        for _ in 0..10 {
            v.failed("busy", false);
            v.ended();
        }
        assert_eq!(v.view().error, None, "an ended() between failures breaks the run");
        for _ in 0..5 {
            v.failed("busy", false);
        }
        assert_eq!(v.view().error.as_deref(), Some("Wake word stopped: busy"));
        assert_eq!(v.view().state, "idle");
        io.take();
        v.ended();
        assert!(v.announce("laptop needs a decision"));
        v.spoken(true);
        assert_eq!(io.take(), vec!["say laptop needs a decision"], "the loop does not restart by itself");
        v.set_wake(true);
        assert_eq!(io.take(), vec!["listen wake"]);
    }

    #[test]
    fn a_recogniser_failure_lets_a_waiting_event_through() {
        let (v, io, _) = voice(vec![]);
        v.tap();
        assert!(v.announce("laptop needs a decision"));
        io.take();
        v.failed("x", false);
        assert_eq!(io.take(), vec!["say laptop needs a decision"]);
        assert_eq!(v.view().error.as_deref(), Some("x"));
    }

    #[test]
    fn a_fatal_failure_does_not_restart_the_wake_loop() {
        let (v, io, _) = voice(vec![]);
        v.set_wake(true);
        io.take();
        v.failed("No recogniser.", true);
        assert_eq!(io.take(), Vec::<String>::new());
        assert_eq!(v.view().state, "idle");
    }

    #[test]
    fn plugin_events_reach_the_conversation() {
        let (v, io, asked) = voice(vec![Ok(serde_json::json!({"say": "Hi."}))]);
        v.tap();
        io.take();
        dispatch(&v, &serde_json::json!({"event": "partial", "text": "what ne"}));
        assert_eq!(v.view().heard, "what ne");
        dispatch(&v, &serde_json::json!({"event": "final", "text": "what needs me"}));
        assert_eq!(asked.lock().unwrap().len(), 1);
        dispatch(&v, &serde_json::json!({"event": "spoken", "ok": true}));
        assert_eq!(v.view().state, "idle");
        dispatch(&v, &serde_json::json!({"event": "screen", "on": false}));
        assert_eq!(v.view().state, "off");
        dispatch(&v, &serde_json::json!({"event": "nonsense"}));
    }

    #[test]
    fn history_records_the_announcement_that_was_spoken() {
        let (v, _, asked) = voice(vec![
            Ok(serde_json::json!({"say": "I'll tell x.", "pending": {"say": "Telling x: y.", "actions": [reply()]}})),
            Ok(serde_json::json!({"say": ""})),
            Ok(serde_json::json!({"say": "Done."})),
            Ok(serde_json::json!({"say": "Done."})),
        ]);
        v.tap();
        v.heard("tell x y");
        v.spoken(true);
        v.heard("Maya tell z w");
        {
            let Request::Interpret { history, .. } = &asked.lock().unwrap()[1] else { panic!() };
            assert_eq!(history.last().unwrap().text, "Telling x: y.");
        }
        v.tap();
        v.heard("q3");
        let Request::Interpret { history, .. } = &asked.lock().unwrap()[2] else { panic!() };
        assert!(history.iter().all(|t| !t.text.trim().is_empty()), "a blank answer is not a turn: {history:?}");
    }

    fn read_back() -> Vec<Result<Value, String>> {
        vec![announce_answer("Telling x: y.", vec![reply()])]
    }

    #[test]
    fn a_tap_window_lasts_while_you_speak() {
        let (v, io, asked) = voice(vec![Ok(serde_json::json!({"say": "Done."}))]);
        v.tap();
        io.take();
        for words in ["tell", "tell hexgrid", "tell hexgrid to ship"] {
            advance(maya_core::wake::COMMAND_WAIT_MS - 1_000);
            v.partial(words);
        }
        advance(maya_core::wake::COMMAND_WAIT_MS - 1_000);
        v.heard("tell hexgrid to ship it");
        let asked = asked.lock().unwrap();
        assert!(matches!(&asked[..], [Request::Interpret { command, .. }] if command == "tell hexgrid to ship it"), "{asked:?}");
    }

    #[test]
    fn a_partial_during_the_window_keeps_its_deadline() {
        let (v, io, asked) = voice(read_back());
        v.tap();
        v.heard("tell x y");
        v.spoken(true);
        io.take();
        advance(window::WAIT_MS - 1_000);
        v.partial("ye");
        assert_eq!(v.view().state, "window");
        advance(1_001);
        v.ended();
        assert_eq!(executed(&asked), 1, "talking does not push the batch's deadline");
    }

    #[test]
    fn a_wake_retry_does_not_cut_off_a_tap() {
        let (v, io, _) = voice(vec![]);
        v.set_wake(true);
        v.failed("the recogniser is busy", false);
        let generation = v.begin_request();
        v.tap();
        io.take();
        v.retry_wake(generation);
        assert_eq!(io.take(), Vec::<String>::new(), "the tap's window is still open");
        advance(maya_core::wake::COMMAND_WAIT_MS + 1);
        v.retry_wake(generation);
        assert_eq!(io.take(), vec!["listen wake"]);
    }

    #[test]
    fn a_failure_inside_a_tap_window_ends_it_and_the_wake_loop_resumes() {
        let (v, io, _) = voice(vec![]);
        v.set_wake(true);
        v.tap();
        io.take();
        v.failed("no network for speech recognition", false);
        assert_eq!(io.take(), vec!["listen wake"], "the tap's recogniser is gone: back to the wake loop");
        assert_eq!(v.view().error.as_deref(), Some("no network for speech recognition"));
        assert_eq!(v.view().state, "listening");
        assert!(v.backoffs().is_empty(), "not a wake loop failure");
        advance(maya_core::wake::COMMAND_WAIT_MS + 1);
        v.retry_wake(v.begin_request());
        assert_eq!(io.take(), vec!["listen wake"], "a retry finds nothing in its way");
    }

    #[test]
    fn a_failure_inside_a_follow_up_window_lets_a_waiting_event_through() {
        let (v, io, _) = voice(vec![Ok(serde_json::json!({"say": "Which one?"}))]);
        v.set_wake(true);
        v.tap();
        v.heard("answer it");
        v.spoken(true);
        assert!(v.announce("laptop needs a decision"));
        io.take();
        v.failed("the recogniser is busy", false);
        assert_eq!(io.take(), vec!["say laptop needs a decision"]);
        v.spoken(true);
        assert_eq!(io.take(), vec!["listen wake"]);
    }

    #[test]
    fn a_second_tap_stops_listening() {
        let (v, io, asked) = voice(vec![]);
        v.tap();
        io.take();
        v.tap();
        assert_eq!(io.take(), vec!["stop listening"]);
        assert_eq!(v.view().state, "idle");
        v.heard("late words");
        assert!(asked.lock().unwrap().is_empty(), "nothing heard after the stop is a command");
    }

    #[test]
    fn a_second_tap_speaks_a_waiting_event_then_resumes_the_wake_loop() {
        let (v, io, _) = voice(vec![]);
        v.set_wake(true);
        v.tap();
        assert!(v.announce("laptop needs a decision"));
        io.take();
        v.tap();
        assert_eq!(io.take(), vec!["stop listening", "say laptop needs a decision"]);
        v.spoken(true);
        assert_eq!(io.take(), vec!["listen wake"]);
    }

    #[test]
    fn a_second_tap_after_a_question_stops_listening() {
        let (v, io, _) = voice(vec![Ok(serde_json::json!({"say": "Which one?"}))]);
        v.tap();
        v.heard("answer it");
        v.spoken(true);
        io.take();
        v.tap();
        assert_eq!(io.take(), vec!["stop listening"]);
        assert_eq!(v.view().state, "idle");
    }

    #[test]
    fn a_tap_during_the_window_stops_listening_but_keeps_the_batch() {
        let (v, io, asked) = voice(read_back());
        v.tap();
        v.heard("tell x y");
        v.spoken(true);
        io.take();
        v.tap();
        assert_eq!(io.take(), vec!["stop listening"]);
        assert_eq!(v.view().pending.as_deref(), Some("Telling x: y."));
        assert_eq!(v.view().state, "window");
        advance(window::WAIT_MS + 1);
        v.ended();
        assert_eq!(executed(&asked), 1, "it still runs when the window ends");
    }

    #[test]
    fn a_tap_after_the_window_ran_out_runs_the_batch() {
        let (v, io, asked) = voice(read_back());
        v.tap();
        v.heard("tell x y");
        v.spoken(true);
        v.tap();
        io.take();
        advance(window::WAIT_MS + 1);
        v.tap();
        assert_eq!(executed(&asked), 1);
        assert_eq!(v.view().pending, None);
    }

    #[test]
    fn a_tap_while_speaking_stops_and_listens() {
        let (v, io, _) = voice(vec![Ok(serde_json::json!({"say": "Two sessions need you."}))]);
        v.tap();
        v.heard("what needs me");
        io.take();
        v.tap();
        assert_eq!(io.take(), vec!["stop speaking", "listen"]);
        assert_eq!(v.view().state, "awaiting-command");
    }

    #[test]
    fn a_tap_during_the_announcement_cuts_it_off_and_starts_the_window() {
        let (v, io, asked) = voice(read_back());
        v.tap();
        v.heard("tell x y");
        io.take();
        v.tap();
        assert_eq!(io.take(), vec!["stop speaking", "listen"]);
        assert_eq!(v.view().state, "window");
        assert_eq!(v.view().pending.as_deref(), Some("Telling x: y."));
        v.heard("yes");
        assert_eq!(executed(&asked), 1);
    }

    #[test]
    fn a_stopped_window_still_runs_out_and_the_wake_loop_comes_back() {
        let (v, io, asked) = voice(read_back());
        v.set_wake(true);
        v.tap();
        v.heard("tell x y");
        v.spoken(true);
        let (generation, id) = v.window_timer();
        v.tap();
        io.take();
        assert!(v.expire_window(generation, id), "still inside its window: try again later");
        assert_eq!(io.take(), Vec::<String>::new());
        advance(window::WAIT_MS + 1);
        assert!(!v.expire_window(generation, id));
        assert_eq!(io.take(), vec!["stop listening", "listen wake"]);
        assert_eq!(v.view().pending, None);
        assert_eq!(executed(&asked), 1);
    }

    #[test]
    fn an_older_batch_s_timer_does_not_run_a_newer_batch() {
        let (v, io, asked) = voice(vec![
            announce_answer("Telling x: y.", vec![reply()]),
            announce_answer("Telling z: w.", vec![reply()]),
        ]);
        v.tap();
        v.heard("tell x y");
        v.spoken(true);
        let (generation, older) = v.window_timer();
        v.heard("Maya tell z w");
        v.spoken(true);
        let (_, newer) = v.window_timer();
        assert_ne!(older, newer);
        v.tap();
        io.take();
        advance(window::WAIT_MS + 1);
        assert!(!v.expire_window(generation, older));
        assert_eq!(executed(&asked), 0, "the older timer is stale");
        v.expire_window(generation, newer);
        assert_eq!(executed(&asked), 1);
    }

    #[test]
    fn a_queued_event_is_spoken_only_if_events_may_still_be_spoken() {
        for allowed in [true, false] {
            let (v, io, _) = voice(read_back());
            let gate_open = Arc::new(std::sync::atomic::AtomicBool::new(true));
            let unlocked = Arc::new(Mutex::new(vec![]));
            let (g, u, me) = (gate_open.clone(), unlocked.clone(), Arc::downgrade(&v));
            v.set_event_gate(Arc::new(move || {
                // Asked with the voice's lock free: the real gate calls into Android.
                u.lock().unwrap().push(me.upgrade().is_some_and(|v| v.inner.try_lock().is_ok()));
                g.load(std::sync::atomic::Ordering::SeqCst)
            }));
            v.set_wake(true);
            v.tap();
            v.heard("tell x y");
            assert!(v.announce("laptop needs a decision"));
            io.take();
            gate_open.store(allowed, std::sync::atomic::Ordering::SeqCst);
            v.cancel();
            v.spoken(true);
            let next = if allowed { "say laptop needs a decision" } else { "listen wake" };
            assert_eq!(io.take(), vec!["stop speaking", "stop listening", "say Cancelled.", next], "allowed: {allowed}");
            let unlocked = unlocked.lock().unwrap();
            assert!(!unlocked.is_empty() && unlocked.iter().all(|u| *u), "asked at playback, never under the lock: {unlocked:?}");
        }
    }

    /// Every view the page was shown, for checks on what was published.
    fn voice_shown() -> (Arc<Voice>, Arc<FakeIo>, Arc<Mutex<Vec<VoiceView>>>) {
        let io = Arc::new(FakeIo::default());
        let shown = Arc::new(Mutex::new(vec![]));
        let s = shown.clone();
        let ask: Ask = Arc::new(|_| Ok(Some(serde_json::json!({"say": "Hi."}))));
        let v = Voice::new_sync(io.clone(), ask, Arc::new(move |view: &VoiceView| s.lock().unwrap().push(view.clone())), now);
        v.screen(true);
        v.set_offer(None);
        io.take();
        shown.lock().unwrap().clear();
        (v, io, shown)
    }

    #[test]
    fn an_error_clears_on_a_tap_or_a_heard_command() {
        let (v, _, _) = voice(vec![Ok(serde_json::json!({"say": "Hi."}))]);
        v.failed("no network for speech recognition", false);
        assert!(v.view().error.is_some());
        v.tap();
        assert_eq!(v.view().error, None, "a tap clears it");
        v.failed("no network for speech recognition", false);
        v.heard("what needs me");
        assert_eq!(v.view().error, None, "a heard command clears it");
        v.failed(MIC_OFF, true);
        v.tap();
        assert_eq!(v.view().error, None, "a tap clears a refused microphone: it was asked for again");
    }

    #[test]
    fn a_spoken_line_clears_only_the_missing_voice() {
        let (v, _, _) = voice(vec![]);
        v.spoken(false);
        assert_eq!(v.view().error.as_deref(), Some(NO_TTS));
        v.spoken(true);
        assert_eq!(v.view().error, None, "the voice is there after all");
        v.failed("no network for speech recognition", false);
        v.spoken(true);
        assert_eq!(v.view().error.as_deref(), Some("no network for speech recognition"), "speaking says nothing about the recogniser");
        v.failed(MIC_OFF, true);
        v.spoken(true);
        assert_eq!(v.view().error.as_deref(), Some(MIC_OFF), "the Open settings button stays");
        v.spoken(false);
        assert_eq!(v.view().error.as_deref(), Some(MIC_OFF), "a failed line does not hide a refused microphone");
    }

    #[test]
    fn the_wake_word_stopped_error_stays_until_the_setting_changes() {
        let (v, _, _) = voice(vec![Ok(serde_json::json!({"say": "Hi."}))]);
        v.set_wake(true);
        for _ in 0..5 {
            v.failed("busy", false);
        }
        v.tap();
        v.heard("what needs me");
        v.spoken(true);
        assert_eq!(v.view().error.as_deref(), Some("Wake word stopped: busy"));
        v.set_wake(true);
        assert_eq!(v.view().error, None);
    }

    #[test]
    fn a_missing_voice_is_shown_on_screen_only() {
        let (v, _, shown) = voice_shown();
        v.spoken(false);
        let last = shown.lock().unwrap().last().cloned();
        assert_eq!(last.and_then(|s| s.error).as_deref(), Some(NO_TTS), "published, not just set");
        let (v, _, _) = voice_shown();
        v.screen(false);
        v.spoken(false);
        assert_eq!(v.view().error, None, "off screen, Android refuses to speak: not a missing voice");
    }
}
