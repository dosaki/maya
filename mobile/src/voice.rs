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
/// Whether a board event may be spoken now (Speak on, the phone not silenced).
pub type Gate = Arc<dyn Fn() -> bool + Send + Sync>;

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
    /// `carry_on` found the conversation idle with an event waiting;
    /// `play_queued` asks the gate and speaks it. Busy meanwhile.
    playing: bool,
    generation: u64,
    /// Bumped by each confirmation held or armed: an older window's
    /// expiry timer finds it changed and does nothing.
    epoch: u64,
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
            after: After::Idle, events: VecDeque::new(), playing: false, generation: 0, epoch: 0, failures: 0, backoffs: vec![], view: VoiceView { state: "off".into(), ..Default::default() },
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
    /// yes or no when she was reading an action back. While a command
    /// window listens it stops listening; a waiting confirmation stops
    /// listening too but keeps its action and its buttons. Otherwise it
    /// listens for one command.
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
            if let After::Confirm(p) = std::mem::replace(&mut i.after, After::Idle) {
                self.arm_confirm(&mut i, p);
                self.io.listen(false);
                self.publish(&mut i);
                return;
            }
        } else {
            if i.flow.drop_expired(now) {
                i.view.pending = None;
            }
            if i.flow.has_pending() {
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
        // Still talking: a command window lasts until the words stop. A
        // confirmation keeps its own window and its pending action.
        let now = (self.now)();
        if i.flow.state(now) == "awaiting-command" {
            i.flow.await_reply(now);
        }
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
        self.failed_now(message, fatal);
        self.play_queued();
    }

    fn failed_now(&self, message: &str, fatal: bool) {
        let mut i = self.inner.lock().unwrap();
        if !Self::active(&i) {
            return;
        }
        // Inside a tap, follow-up or confirmation window the failed
        // recogniser was that window's, not the wake loop's: end the window
        // as without the wake word, and carry on (the wake loop resumes).
        let window = i.flow.state((self.now)()) != "idle";
        if fatal || !i.wake || window {
            i.view.error = Some(message.to_string());
            i.flow.reset();
            // A fatal error would fail again at once: no wake loop this session.
            if fatal {
                i.wake = false;
            }
            if !i.speaking && !i.thinking {
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
    /// or a confirmation listens in a window of its own; the wake loop
    /// comes back by itself once that window closes, since every way one
    /// closes carries on: heard, ended, a failure (`failed` ends the
    /// window), a second tap, or a confirmation's own expiry timer.
    fn retry_wake(&self, generation: u64) {
        let i = self.inner.lock().unwrap();
        let idle = i.flow.state((self.now)()) == "idle";
        if i.generation == generation && Self::active(&i) && i.wake && !i.speaking && !i.thinking && idle {
            self.io.listen(true);
        }
    }

    /// Starts a confirmation's answer window, and a timer that ends it
    /// even when nothing is listening (a tap stopped the mic): the timer
    /// thread holds only a `Weak`, and the generation and epoch tell it
    /// whether this window is still the one waiting.
    fn arm_confirm(&self, i: &mut Inner, p: Pending) {
        i.flow.set_pending(p, (self.now)());
        i.epoch += 1;
        if self.sync {
            return;
        }
        let (generation, epoch) = (i.generation, i.epoch);
        let me = self.me.lock().unwrap().clone();
        std::thread::spawn(move || {
            let mut wait = maya_core::wake::CONFIRM_WAIT_MS + 1;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(wait));
                match me.upgrade() {
                    Some(v) if v.expire_pending(generation, epoch) => wait = 250,
                    _ => break,
                }
            }
        });
    }

    /// A confirmation's timer ran out: as `ended` does for an expired
    /// action, say "OK." and carry on once it is said. True when the
    /// window is still open (a clock that lagged the sleep): try again.
    fn expire_pending(&self, generation: u64, epoch: u64) -> bool {
        let mut i = self.inner.lock().unwrap();
        // Speaking or thinking, what follows settles the action itself.
        if i.generation != generation || i.epoch != epoch || !Self::active(&i) || i.speaking || i.thinking || !i.flow.has_pending() {
            return false;
        }
        if !i.flow.drop_expired((self.now)()) {
            return true;
        }
        i.view.pending = None;
        self.say(&mut i, "OK.", After::Idle);
        self.publish(&mut i);
        false
    }

    /// The generation and epoch a confirmation's timer now carries; a test seam.
    #[cfg(test)]
    fn confirm_epoch(&self) -> (u64, u64) {
        let i = self.inner.lock().unwrap();
        (i.generation, i.epoch)
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
            After::Confirm(p) => {
                self.arm_confirm(&mut i, p);
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
        let execute = matches!(request, Request::Execute { .. });
        // The user has moved on: a read-back still being said must not re-arm
        // its confirmation when it finishes.
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
        let pending = data.get("pending").filter(|p| !p.is_null()).map(|p| Pending { say: p["say"].as_str().unwrap_or(&say).to_string(), action: p["action"].clone() });
        // The turn is what Maya says: the read-back when there is one.
        push(&mut i.history, "maya", pending.as_ref().map_or(&say, |p| &p.say));
        match pending {
            Some(p) => {
                // Held while it is read back; `spoken` starts its window.
                i.flow.hold_pending(p.clone());
                i.epoch += 1;
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

    #[test]
    fn a_yes_during_the_read_back_runs_the_action_once() {
        let action = serde_json::json!({"kind": "reply", "session": "s1"});
        let io = Arc::new(FakeIo::default());
        let asked = Arc::new(Mutex::new(vec![]));
        let me: Arc<Mutex<std::sync::Weak<Voice>>> = Arc::new(Mutex::new(std::sync::Weak::new()));
        let (a, m) = (asked.clone(), me.clone());
        let ask: Ask = Arc::new(move |r: Request| {
            let execute = matches!(r, Request::Execute { .. });
            a.lock().unwrap().push(r);
            if !execute {
                return Ok(Some(serde_json::json!({"say": "Telling x: y. Yes?", "pending": {"say": "Telling x: y. Yes?", "action": action.clone()}})));
            }
            // The read-back finishes while the main runs the action.
            let v = m.lock().unwrap().upgrade();
            if let Some(v) = v {
                v.spoken(true);
            }
            Ok(Some(serde_json::json!({"say": "Sent."})))
        });
        let v = Voice::new_sync(io.clone(), ask, Arc::new(|_| {}), now);
        *me.lock().unwrap() = Arc::downgrade(&v);
        v.screen(true);
        v.set_offer(None);
        v.tap();
        v.heard("tell x y");
        io.take();
        v.confirm(true);
        assert_eq!(io.take(), vec!["stop listening", "stop speaking", "say Sent."]);
        v.spoken(true);
        assert_eq!(io.take(), Vec::<String>::new(), "no mic reopened for a confirm");
        assert_eq!(v.view().pending, None);
        v.heard("okay");
        let asked = asked.lock().unwrap();
        assert_eq!(asked.iter().filter(|r| matches!(r, Request::Execute { .. })).count(), 1);
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
    fn history_records_the_read_back_that_was_spoken() {
        let action = serde_json::json!({"kind": "reply", "session": "s1"});
        let (v, _, asked) = voice(vec![
            Ok(serde_json::json!({"say": "I'll tell x.", "pending": {"say": "Telling x: y. Yes?", "action": action}})),
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
            assert_eq!(history.last().unwrap().text, "Telling x: y. Yes?");
        }
        v.tap();
        v.heard("q3");
        let Request::Interpret { history, .. } = &asked.lock().unwrap()[2] else { panic!() };
        assert!(history.iter().all(|t| !t.text.trim().is_empty()), "a blank answer is not a turn: {history:?}");
    }

    fn read_back() -> Vec<Result<Value, String>> {
        let action = serde_json::json!({"kind": "reply", "session": "s1"});
        vec![
            Ok(serde_json::json!({"say": "Telling x: y. Yes?", "pending": {"say": "Telling x: y. Yes?", "action": action}})),
            Ok(serde_json::json!({"say": "Sent."})),
        ]
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
    fn a_partial_during_a_read_back_keeps_it_a_confirmation() {
        let (v, io, asked) = voice(read_back());
        v.tap();
        v.heard("tell x y");
        v.spoken(true);
        io.take();
        v.partial("ye");
        assert_eq!(v.view().state, "awaiting-confirm");
        v.heard("yes");
        assert!(matches!(&asked.lock().unwrap()[1], Request::Execute { .. }));
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
    fn a_tap_during_a_confirmation_stops_listening_but_keeps_it() {
        let (v, io, asked) = voice(read_back());
        v.tap();
        v.heard("tell x y");
        v.spoken(true);
        io.take();
        v.tap();
        assert_eq!(io.take(), vec!["stop listening"]);
        assert_eq!(v.view().pending.as_deref(), Some("Telling x: y. Yes?"));
        assert_eq!(v.view().state, "awaiting-confirm");
        v.confirm(true);
        assert!(matches!(&asked.lock().unwrap()[1], Request::Execute { .. }));
    }

    #[test]
    fn a_tap_after_a_stopped_confirmation_ran_out_starts_a_command() {
        let (v, io, _) = voice(read_back());
        v.tap();
        v.heard("tell x y");
        v.spoken(true);
        v.tap();
        io.take();
        advance(maya_core::wake::CONFIRM_WAIT_MS + 1);
        v.tap();
        assert_eq!(io.take(), vec!["listen"]);
        assert_eq!(v.view().pending, None);
        assert_eq!(v.view().state, "awaiting-command");
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
    fn a_tap_during_the_read_back_listens_for_yes_or_no() {
        let (v, io, asked) = voice(read_back());
        v.tap();
        v.heard("tell x y");
        io.take();
        v.tap();
        assert_eq!(io.take(), vec!["stop speaking", "listen"]);
        assert_eq!(v.view().state, "awaiting-confirm");
        assert_eq!(v.view().pending.as_deref(), Some("Telling x: y. Yes?"));
        v.heard("yes");
        assert!(matches!(&asked.lock().unwrap()[1], Request::Execute { .. }));
    }

    #[test]
    fn a_yes_during_a_long_read_back_still_confirms() {
        let (v, io, asked) = voice(read_back());
        v.tap();
        v.heard("tell x y");
        io.take();
        advance(maya_core::wake::CONFIRM_WAIT_MS + 1);
        assert_eq!(v.view().pending.as_deref(), Some("Telling x: y. Yes?"), "Yes and No still show");
        v.confirm(true);
        assert!(matches!(asked.lock().unwrap().get(1), Some(Request::Execute { .. })), "the window starts when the read-back ends");
    }

    #[test]
    fn the_answer_window_starts_when_the_read_back_ends() {
        let (v, io, asked) = voice(read_back());
        v.tap();
        v.heard("tell x y");
        advance(maya_core::wake::CONFIRM_WAIT_MS + 1);
        v.spoken(true);
        io.take();
        assert_eq!(v.view().state, "awaiting-confirm");
        advance(maya_core::wake::CONFIRM_WAIT_MS - 1);
        v.heard("yes");
        assert!(matches!(asked.lock().unwrap().get(1), Some(Request::Execute { .. })));
    }

    #[test]
    fn a_stopped_confirmation_still_runs_out_and_the_wake_loop_comes_back() {
        let (v, io, asked) = voice(read_back());
        v.set_wake(true);
        v.tap();
        v.heard("tell x y");
        v.spoken(true);
        let (generation, epoch) = v.confirm_epoch();
        v.tap();
        io.take();
        assert!(v.expire_pending(generation, epoch), "still inside its window: try again later");
        assert_eq!(io.take(), Vec::<String>::new());
        advance(maya_core::wake::CONFIRM_WAIT_MS + 1);
        assert!(!v.expire_pending(generation, epoch));
        assert_eq!(io.take(), vec!["say OK."]);
        assert_eq!(v.view().pending, None);
        v.spoken(true);
        assert_eq!(io.take(), vec!["listen wake"]);
        assert_eq!(asked.lock().unwrap().len(), 1, "nothing ran");
    }

    #[test]
    fn an_older_window_s_timer_does_not_end_a_newer_confirmation() {
        let action = serde_json::json!({"kind": "reply", "session": "s1"});
        let (v, io, _) = voice(vec![
            Ok(serde_json::json!({"say": "Telling x: y. Yes?", "pending": {"say": "Telling x: y. Yes?", "action": action}})),
            Ok(serde_json::json!({"say": "Telling z: w. Yes?", "pending": {"say": "Telling z: w. Yes?", "action": action}})),
        ]);
        v.tap();
        v.heard("tell x y");
        v.spoken(true);
        let (generation, older) = v.confirm_epoch();
        v.heard("Maya tell z w");
        v.spoken(true);
        let (_, newer) = v.confirm_epoch();
        v.tap();
        io.take();
        advance(maya_core::wake::CONFIRM_WAIT_MS + 1);
        assert!(!v.expire_pending(generation, older));
        assert_eq!(io.take(), Vec::<String>::new(), "the older timer is stale");
        v.expire_pending(generation, newer);
        assert_eq!(io.take(), vec!["say OK."]);
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
            v.confirm(false);
            v.spoken(true);
            let next = if allowed { "say laptop needs a decision" } else { "listen wake" };
            assert_eq!(io.take(), vec!["stop listening", "say OK.", next], "allowed: {allowed}");
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
