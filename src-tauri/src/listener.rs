//! Voice assistant: the listener thread, the wake-word conversation, spoken
//! replies with listening paused around them, and confirmed actions.
//!
//! Lock rule: never hold the store lock or the voice lock while speaking or
//! while the interpreter runs; take what is needed, release, then act.

use crate::store::now_ms;
use crate::{config, ear, eleven_settings, eleven_wanted, interpreter, launch, log, notify, wake};
use crate::{answer_question, compact_session, list_resumable_sessions, open_review_pr, resume_session, review_pr, send_reply, start_session, AppState};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State as TauriState};

#[derive(Default)]
pub struct VoiceState {
    pub ear: Option<ear::Ear>,
    pub flow: Option<wake::Flow>,
    pub status: VoiceStatus,
    pub history: Vec<VoiceTurn>,
    pub failures: u32,
    /// Bumped on every start and stop, so a listener thread from an earlier
    /// run ignores its sidecar's last events instead of touching the new one.
    pub generation: u64,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceStatus {
    pub listening: bool,
    pub state: String,
    pub detail: String,
    pub level: f64,
    pub heard: String,
    pub said: String,
    pub pending: Option<String>,
    /// Turns ever added to the history; the page refetches it when this changes.
    pub turns: u64,
}

impl Default for VoiceStatus {
    fn default() -> Self {
        Self { listening: false, state: "off".into(), detail: String::new(), level: 0.0, heard: String::new(), said: String::new(), pending: None, turns: 0 }
    }
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceTurn {
    pub who: String,
    pub text: String,
    pub at: u64,
}

fn emit_voice(app: &AppHandle) {
    let status = app.state::<AppState>().voice.lock().unwrap().status.clone();
    let _ = app.emit("voice", &status);
}

/// Updates the status and emits it, but only while listener run `generation`
/// is still current; false means the run was stopped or replaced, and the
/// caller drops the rest of its turn quietly.
fn set_voice(app: &AppHandle, generation: u64, f: impl FnOnce(&mut VoiceStatus)) -> bool {
    {
        let state = app.state::<AppState>();
        let mut v = state.voice.lock().unwrap();
        if v.generation != generation {
            return false;
        }
        f(&mut v.status);
    }
    emit_voice(app);
    true
}

/// Adds a turn to the history (the last 40 are kept) and counts it.
fn push_turn(v: &mut VoiceState, who: &str, text: &str, at: u64) {
    v.history.push(VoiceTurn { who: who.into(), text: text.into(), at });
    if v.history.len() > 40 {
        let extra = v.history.len() - 40;
        v.history.drain(..extra);
    }
    v.status.turns += 1;
}

/// Remembers a turn; the caller's next `voice` event tells the page.
fn remember(app: &AppHandle, who: &str, text: &str) {
    let state = app.state::<AppState>();
    let mut v = state.voice.lock().unwrap();
    push_turn(&mut v, who, text, now_ms());
}

/// What the speech hook does to the voice state: pause the ear and remember
/// the line (so hearing it back does nothing) as an utterance starts, resume
/// the ear when it ends.
fn on_speech(v: &mut VoiceState, phase: &notify::SpeechPhase) {
    match phase {
        notify::SpeechPhase::Starting(text) => {
            // A short prompt ("Yes?") leaves the ear open so the user can
            // talk over it; the flow strips her words from what comes back.
            if pauses_ear_for(text) {
                if let Some(e) = v.ear.as_mut() {
                    e.pause();
                }
            }
            if let Some(f) = v.flow.as_mut() {
                f.ignore_line(text);
            }
        }
        notify::SpeechPhase::Finished => {
            if let Some(e) = v.ear.as_mut() {
                e.resume();
            }
        }
    }
}

/// Installs the hook that pauses listening around every line Maya says:
/// replies, queued announcements and the voice test all share one queue.
pub(crate) fn install_speech_hook(app: AppHandle) {
    notify::install_speech_hook(move |phase| {
        let state = app.state::<AppState>();
        let mut v = state.voice.lock().unwrap();
        on_speech(&mut v, &phase);
    });
}

/// Speaks a reply to the user (blocking until spoken) through the shared
/// speech queue, which pauses listening around it and is not held back by a
/// Focus mode. Blank text, or a muted Maya, says nothing. False when run
/// `generation` is no longer current, before or after speaking.
fn reply_aloud(app: &AppHandle, generation: u64, text: &str) -> bool {
    if text.trim().is_empty() {
        return is_current(app, generation);
    }
    {
        let state = app.state::<AppState>();
        let mut v = state.voice.lock().unwrap();
        if v.generation != generation {
            return false;
        }
        v.status.said = text.to_string();
    }
    // History first, so a page that refetches it on this event sees the line.
    remember(app, "maya", text);
    emit_voice(app);
    let (muted, eleven) = {
        let state = app.state::<AppState>();
        let store = state.store.lock().unwrap();
        (store.config.muted, eleven_wanted(&store.config, store.claude_dir()))
    };
    // Muted, the reply is only shown in the voice panel.
    if muted {
        return is_current(app, generation);
    }
    let eleven = eleven_settings(eleven);
    // No lock is held here: the speech hook takes the voice lock.
    let _ = notify::speak_and_wait(notify::Utterance::new(text.to_string(), eleven));
    is_current(app, generation)
}

/// Why "focus" refuses a card that runs on another machine: its pid means
/// nothing here, so the user is sent to open it there instead. None locally.
fn remote_focus_refusal(machine: Option<&str>) -> Option<String> {
    machine.map(|m| format!("That session runs on {m}; open it there."))
}

/// Runs a validated action through the same paths as the page's buttons;
/// the Ok text is what Maya says after a confirmed action.
fn execute_action(app: &AppHandle, action: &serde_json::Value) -> Result<String, String> {
    let state = app.state::<AppState>();
    let kind = action["kind"].as_str().unwrap_or("");
    let session = action["session"].as_str().unwrap_or("").to_string();
    // The machine the validated action runs on; None (or blank) means local.
    let machine = action["machine"].as_str().filter(|m| !m.is_empty()).map(str::to_string);
    match kind {
        "report" => Ok(String::new()),
        "focus" => {
            // A remote card's pid is meaningless here: there is nothing local to
            // bring forward, so send the user to the machine that has it.
            if let Some(msg) = remote_focus_refusal(machine.as_deref()) {
                return Err(msg);
            }
            let pid = state.store.lock().unwrap().card_for(&session, now_ms()).map(|c| c.pid).ok_or("Session is no longer running.")?;
            crate::term::focus_pid(pid)?;
            Ok("Done.".into())
        }
        "compact" => {
            // Routed like reply/answer: a remote card's compact must run on
            // its own machine, never typed into a local (nonexistent) tty.
            compact_session(app.clone(), state.clone(), session)?;
            Ok("Compacting.".into())
        }
        "reply" => {
            let text = action["text"].as_str().unwrap_or("").to_string();
            send_reply(app.clone(), state.clone(), session, text, vec![])?;
            Ok("Sent.".into())
        }
        "answer" => {
            // The ask the user confirmed, captured at validation: a newer ask is refused.
            let ask_id = action["askId"].as_u64().ok_or("The question has changed; ask me again.")?;
            let n = action["option"].as_u64().unwrap_or(1) as usize;
            answer_question(app.clone(), state.clone(), session, ask_id, 0, n.saturating_sub(1))?;
            Ok("Answered.".into())
        }
        "resume" => {
            let dir = action["dir"].as_str().unwrap_or("").to_string();
            let sessions = list_resumable_sessions(app.clone(), state.clone(), dir.clone(), machine.clone())?;
            let latest = sessions.into_iter().find(|s| !s.running).ok_or("Nothing to resume there.")?;
            resume_session(app.clone(), state.clone(), dir, latest.id, machine)?;
            Ok("Resuming.".into())
        }
        "start" => {
            let dir = action["dir"].as_str().unwrap_or("").to_string();
            let prompt = action["prompt"].as_str().unwrap_or("").to_string();
            start_session(app.clone(), state.clone(), Some(dir), prompt, launch::LaunchOptions::default(), machine)?;
            Ok("Started.".into())
        }
        "review" => {
            let repo = action["repo"].as_str().unwrap_or("").to_string();
            let number = action["number"].as_u64().unwrap_or(0);
            review_pr(state.clone(), repo, number)?;
            Ok("Review started.".into())
        }
        "open" => {
            let repo = action["repo"].as_str().unwrap_or("").to_string();
            let number = action["number"].as_u64().unwrap_or(0);
            open_review_pr(state.clone(), repo, number)?;
            Ok("Opened.".into())
        }
        other => Err(format!("I don't know how to {other}")),
    }
}

/// The last exchanges as (user, maya) pairs, oldest first, for the interpreter:
/// pairs from the last 12 turns, at most 6 of them.
fn recent_exchanges(app: &AppHandle) -> Vec<(String, String)> {
    let state = app.state::<AppState>();
    let v = state.voice.lock().unwrap();
    let mut pairs = Vec::new();
    let mut last_user: Option<String> = None;
    let start = v.history.len().saturating_sub(12);
    for t in &v.history[start..] {
        if t.who == "user" {
            last_user = Some(t.text.clone());
        } else if let Some(u) = last_user.take() {
            pairs.push((u, t.text.clone()));
        }
    }
    let skip = pairs.len().saturating_sub(6);
    pairs.split_off(skip)
}

/// Says `text`, then shows idle; both only while run `generation` is current.
fn reply_then_idle(app: &AppHandle, generation: u64, text: &str, inbox: Option<&Inbox>) {
    if !reply_aloud(app, generation, text) {
        return;
    }
    if !asks_question(text) {
        set_voice(app, generation, |st| st.state = "idle".into());
        return;
    }
    // She asked something: listen for the answer without a wake word.
    // Whatever was heard while she thought and spoke is not the answer.
    if let Some(inbox) = inbox {
        let n = inbox.discard_heard();
        if n > 0 {
            log::line("listener", format!("discarded {n} segment(s) heard before the question ended"));
        }
    }
    let state = app.state::<AppState>();
    let mut v = state.voice.lock().unwrap();
    if v.generation != generation {
        return;
    }
    if let Some(f) = v.flow.as_mut() {
        f.await_reply(now_ms());
        v.status.state = "awaiting-command".into();
        log::line("wake", "waiting for the answer to her question");
    }
    drop(v);
    emit_voice(app);
}

/// Lines of three words or more mute the ear while spoken; shorter prompts
/// ("Yes?", "Cancelled.") are over before a pause would help and the user
/// often talks across them.
pub(crate) fn pauses_ear_for(text: &str) -> bool {
    text.split_whitespace().count() >= 3
}

/// A spoken line that ends in a question mark expects an answer.
pub(crate) fn asks_question(text: &str) -> bool {
    text.trim_end().ends_with('?')
}

/// Sends a spoken command to the interpreter and acts on its reply. Every
/// step after the interpreter returns checks that run `generation` is still
/// current, so a stop while she thinks leaves nothing behind.
fn interpret(app: &AppHandle, generation: u64, cmd: &str, inbox: Option<&Inbox>) {
    let started = set_voice(app, generation, |st| {
        st.state = "thinking".into();
        st.pending = None;
    });
    if !started {
        return;
    }
    let (dirs_local, model, maya_dir) = {
        let state = app.state::<AppState>();
        let store = state.store.lock().unwrap();
        let dirs_local = store.config.projects_dir_path().map(|r| launch::list_project_dirs(&r)).unwrap_or_default();
        (dirs_local, store.config.agent_model.clone(), store.claude_dir().join("maya"))
    };
    // Cards and remote dirs come from the same merged board `list_sessions`
    // shows, taken after `store` is released (lock order: `store` then `network`).
    let state = app.state::<AppState>();
    let cards = crate::merged_cards(&state);
    let mut dirs = dirs_local;
    let mut machines = vec![];
    for board in crate::remote_boards(&state).into_iter().filter(|b| b.connected) {
        for d in &board.dirs {
            dirs.push(format!("{d} (on {})", board.machine));
        }
        machines.push(board.machine);
    }
    let prs = app.state::<AppState>().reviews.lock().unwrap().prs.clone();
    let history = recent_exchanges(app);
    let Some(binary) = launch::claude_binary() else {
        reply_then_idle(app, generation, "I can't find the claude command.", inbox);
        return;
    };
    let reply = match interpreter::run(&binary, &model, cmd, &cards, &prs, &history, &maya_dir, interpreter::TIMEOUT) {
        Ok(r) => r,
        Err(interpreter::RunError::TimedOut) => {
            reply_then_idle(app, generation, "Sorry, that took too long.", inbox);
            return;
        }
        Err(_) => {
            reply_then_idle(app, generation, "Sorry, I didn't catch that.", inbox);
            return;
        }
    };
    if !is_current(app, generation) {
        log::line("listener", "run replaced while thinking; reply dropped");
        return;
    }
    // No action, or only a report: the spoken reply is the whole answer.
    let Some(proposed) = reply.action.as_ref().filter(|a| a["kind"] != "report") else {
        log::line("action", "report only; nothing to run");
        reply_then_idle(app, generation, &reply.say, inbox);
        return;
    };
    match interpreter::validate(proposed, &cards, &dirs, &machines, &prs) {
        Err(why) => {
            log::line("action", format!("rejected: {why}"));
            reply_then_idle(app, generation, &why, inbox)
        }
        Ok(action) if interpreter::needs_confirm(&action) => {
            // The read-back is what will run, never the model's `say`, so a
            // yes always confirms the action the user heard.
            let say = spoken_for(&action).unwrap_or_else(|| "Shall I?".into());
            log::line("action", format!("needs confirmation: {action}"));
            let pending = wake::Pending { say: say.clone(), action };
            // Stored before the read-back so a tap on the page can answer it.
            let accepted = {
                let state = app.state::<AppState>();
                let mut v = state.voice.lock().unwrap();
                if v.generation != generation {
                    return;
                }
                match v.flow.as_mut() {
                    Some(f) => {
                        f.set_pending(pending.clone(), now_ms());
                        v.status.pending = Some(say.clone());
                        v.status.state = "awaiting-confirm".into();
                        true
                    }
                    None => false,
                }
            };
            if !accepted {
                return;
            }
            emit_voice(app);
            if !reply_aloud(app, generation, &say) {
                return;
            }
            // Anything heard before the read-back ended (an "okay" said to
            // someone while she was thinking) sits queued and would confirm
            // an action the user had not heard yet. Throw it away: a person
            // who spoke before the read-back must say it again, which is
            // safer than a stray word confirming.
            if let Some(inbox) = inbox {
                let n = inbox.discard_heard();
                if n > 0 {
                    log::line("listener", format!("discarded {n} segment(s) heard before the read-back ended"));
                }
            }
            // The confirmation window starts when the read-back ends (the ear
            // was paused while she spoke), unless it was answered meanwhile.
            // A read-back longer than the window must still restart it.
            let state = app.state::<AppState>();
            let mut v = state.voice.lock().unwrap();
            if v.generation == generation {
                if let Some(f) = v.flow.as_mut().filter(|f| f.has_pending()) {
                    f.set_pending(pending, now_ms());
                }
            }
        }
        Ok(action) => {
            // Focus and compact run at once and say what they did.
            log::line("action", format!("running now: {action}"));
            let said = match execute_action(app, &action) {
                Ok(s) => spoken_for(&action).unwrap_or(s),
                Err(e) => {
                    log::line("action", format!("failed: {e}"));
                    e
                }
            };
            reply_then_idle(app, generation, &said, inbox);
        }
    }
}

/// Handles one final segment heard during listener run `generation` (or a
/// yes/no from the page). Nothing happens once that run is stopped or replaced.
fn on_heard(app: &AppHandle, generation: u64, text: &str, inbox: Option<&Inbox>) {
    let effects = {
        let state = app.state::<AppState>();
        let mut v = state.voice.lock().unwrap();
        if v.generation != generation {
            return;
        }
        v.status.heard = text.to_string();
        let now = now_ms();
        let before = v.flow.as_ref().map_or("none", |f| f.state(now)).to_string();
        let effects = match v.flow.as_mut() {
            Some(f) => f.on_segment(text, now),
            None => vec![],
        };
        log::line("wake", format!("heard ({before}): {text:?} → {}", describe_effects(&effects)));
        // Nothing to do, but a pending confirmation may still be waiting
        // (chatter, or her own line heard back): show the flow's state.
        // When not listening, "off" or "error" stays as it is.
        if effects.is_empty() && v.status.listening {
            let shown = v.flow.as_ref().map_or("idle", |f| f.state(now));
            v.status.state = shown.into();
            if shown != "awaiting-confirm" {
                v.status.pending = None;
            }
        }
        effects
    };
    if effects.is_empty() {
        emit_voice(app);
        return;
    }
    if let Some(turn) = user_turn(&effects) {
        remember(app, "user", &turn);
    }
    for e in effects {
        match e {
            wake::Effect::Say(s) => {
                if !set_voice(app, generation, |st| st.state = "awaiting-command".into()) {
                    return;
                }
                reply_aloud(app, generation, &s);
            }
            wake::Effect::Interpret(cmd) => interpret(app, generation, &cmd, inbox),
            wake::Effect::Execute(action) => {
                let go = set_voice(app, generation, |st| {
                    st.pending = None;
                    st.state = "thinking".into();
                });
                if !go {
                    return;
                }
                log::line("action", format!("confirmed, running: {action}"));
                let said = match execute_action(app, &action) {
                    Ok(s) => s,
                    Err(e) => {
                        log::line("action", format!("failed: {e}"));
                        e
                    }
                };
                reply_then_idle(app, generation, &said, inbox);
            }
            wake::Effect::Cancelled => {
                let shown = set_voice(app, generation, |st| {
                    st.pending = None;
                    st.state = "idle".into();
                });
                if !shown {
                    return;
                }
                reply_aloud(app, generation, "Cancelled.");
            }
        }
    }
}

/// The listener thread's view of the ear's events, able to throw away what
/// was heard so far without losing a state change such as the sidecar exiting.
pub(crate) struct Inbox {
    rx: std::sync::mpsc::Receiver<ear::EarEvent>,
    kept: std::cell::RefCell<std::collections::VecDeque<ear::EarEvent>>,
}

impl Inbox {
    pub(crate) fn new(rx: std::sync::mpsc::Receiver<ear::EarEvent>) -> Inbox {
        Inbox { rx, kept: Default::default() }
    }

    /// The next event, blocking; None once the ear's reader has gone.
    pub(crate) fn next(&self) -> Option<ear::EarEvent> {
        let kept = self.kept.borrow_mut().pop_front();
        kept.or_else(|| self.rx.recv().ok())
    }

    /// Discards every transcript and level event queued so far; state and
    /// device events are kept for `next`.
    /// Returns how many final segments were thrown away.
    pub(crate) fn discard_heard(&self) -> usize {
        let mut finals = 0;
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                ear::EarEvent::Final(_) => finals += 1,
                ear::EarEvent::Partial(_) | ear::EarEvent::Level(_) => {}
                other => self.kept.borrow_mut().push_back(other),
            }
        }
        finals
    }
}

/// True while `generation` is still the current listener run.
fn is_current(app: &AppHandle, generation: u64) -> bool {
    app.state::<AppState>().voice.lock().unwrap().generation == generation
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ListenChange {
    None,
    Start,
    Stop,
    Restart,
}

/// What a config save means for the listener: a change of engine, model or
/// microphone while listening restarts the run with the new settings.
pub(crate) fn listening_change(before: &config::Config, after: &config::Config) -> ListenChange {
    match (before.listen, after.listen) {
        (false, true) => ListenChange::Start,
        (true, false) => ListenChange::Stop,
        (false, false) => ListenChange::None,
        (true, true) => {
            // A whisper_model change only matters once Builtin is the active
            // recogniser; the picker is hidden under System, but a stale
            // config value must not force a needless restart.
            let model_changed = before.whisper_model != after.whisper_model && after.recognizer == config::Recognizer::Builtin;
            if before.recognizer != after.recognizer || model_changed || before.microphone != after.microphone {
                ListenChange::Restart
            } else {
                ListenChange::None
            }
        }
    }
}

/// The model file the built-in recogniser will load, or why it cannot start.
pub(crate) fn builtin_model_check(claude_dir: &std::path::Path, c: &config::Config) -> Result<std::path::PathBuf, String> {
    if c.recognizer != config::Recognizer::Builtin {
        return Err("the system recogniser needs no model".into());
    }
    let m = crate::models::model(&c.whisper_model).ok_or_else(|| format!("Unknown model {}; pick one in Settings.", c.whisper_model))?;
    if !crate::models::is_downloaded(claude_dir, m.id) {
        return Err(format!("Download the {} model in Settings first.", m.label));
    }
    Ok(crate::models::model_path(claude_dir, m.id).unwrap())
}

/// Whether a model download that just finished should (re)start the
/// listener: the user wants to listen, has chosen the built-in recogniser,
/// and downloaded the model it is configured to use.
pub(crate) fn should_start_after_download(config: &config::Config, id: &str) -> bool {
    config.listen && config.recognizer == config::Recognizer::Builtin && config.whisper_model == id
}

/// Shows `why` as an error and clears whatever a previous run left behind
/// (a pending confirmation, partial transcript, last spoken line) so nothing
/// stale can still be answered while the listener is not running; mirrors
/// what `stop_listening` clears. A no-op once `generation` is no longer
/// current.
fn fail_to_start(app: &AppHandle, generation: u64, why: &str) {
    {
        let state = app.state::<AppState>();
        let mut v = state.voice.lock().unwrap();
        if v.generation != generation {
            return;
        }
        v.flow = None;
        v.status.listening = false;
        v.status.state = "error".into();
        v.status.detail = why.to_string();
        v.status.pending = None;
        v.status.heard.clear();
        v.status.said.clear();
    }
    emit_voice(app);
}

/// Starts the sidecar and the thread that reads it, replacing any running one.
pub(crate) fn start_listening(app: &AppHandle) -> Result<(), String> {
    let (generation, old) = {
        let state = app.state::<AppState>();
        let mut v = state.voice.lock().unwrap();
        v.generation += 1;
        (v.generation, v.ear.take())
    };
    // Ear has no Drop: stop the one being replaced.
    if let Some(mut e) = old {
        e.stop();
    }
    let (device, engine) = {
        let state = app.state::<AppState>();
        let store = state.store.lock().unwrap();
        let device = store.config.microphone.clone();
        let engine = if store.config.recognizer == config::Recognizer::Builtin {
            match builtin_model_check(store.claude_dir(), &store.config) {
                Ok(model) => ear::EngineArgs::Whisper { model },
                Err(why) => {
                    drop(store);
                    log::line("listener", format!("built-in recogniser cannot start: {why}"));
                    fail_to_start(app, generation, &why);
                    return Err(why);
                }
            }
        } else {
            ear::EngineArgs::System
        };
        (device, engine)
    };
    log::line(
        "listener",
        format!(
            "starting run {generation} ({}, microphone: {})",
            match &engine {
                ear::EngineArgs::System => "system".to_string(),
                ear::EngineArgs::Whisper { model } => format!("whisper {}", model.display()),
            },
            device.as_deref().unwrap_or("automatic")
        ),
    );
    let (ear, rx) = match ear::Ear::spawn(device.as_deref(), engine) {
        Ok(pair) => pair,
        Err(e) => {
            log::line("listener", format!("could not start the sidecar: {e}"));
            fail_to_start(app, generation, &e);
            return Err(e);
        }
    };
    // Install it only if no other start or stop happened while it spawned.
    let (installed, displaced) = {
        let state = app.state::<AppState>();
        let mut v = state.voice.lock().unwrap();
        if v.generation != generation {
            (Some(ear), None)
        } else {
            let displaced = v.ear.replace(ear);
            v.flow = Some(wake::Flow::new());
            v.status.listening = true;
            v.status.state = "idle".into();
            v.status.detail.clear();
            v.status.pending = None;
            v.status.heard.clear();
            v.status.said.clear();
            v.status.level = 0.0;
            (None, displaced)
        }
    };
    // Ear has no Drop: stop whichever one is not kept, outside the lock.
    if let Some(mut e) = displaced {
        e.stop();
    }
    if let Some(mut superseded) = installed {
        superseded.stop();
        return Ok(());
    }
    emit_voice(app);
    let handle = app.clone();
    std::thread::spawn(move || {
        let mut heard_any = false;
        // Sound is logged as a peak per few seconds, only when there was some,
        // so the log shows whether the microphone hears anything at all.
        let mut peak = 0.0f64;
        let mut peak_since = std::time::Instant::now();
        // Remembered so the "loading the model…" and "listening" details
        // still name the microphone instead of losing it, the way the
        // System engine's detail never does.
        let mut device_name: Option<String> = None;
        let inbox = Inbox::new(rx);
        while let Some(ev) = inbox.next() {
            if !is_current(&handle, generation) {
                break;
            }
            match ev {
                ear::EarEvent::Level(l) => {
                    handle.state::<AppState>().voice.lock().unwrap().status.level = l;
                    let _ = handle.emit("voice-level", l);
                    peak = peak.max(l);
                    if peak_since.elapsed() >= Duration::from_secs(3) {
                        if peak >= 0.05 {
                            log::line("ear", format!("sound: peak level {peak:.2}"));
                        }
                        peak = 0.0;
                        peak_since = std::time::Instant::now();
                    }
                }
                ear::EarEvent::Partial(t) => {
                    log::line("ear", format!("partial: {t:?}"));
                    set_voice(&handle, generation, |s| s.heard = t);
                }
                ear::EarEvent::Final(t) => {
                    log::line("ear", format!("final: {t:?}"));
                    if !heard_any {
                        // The first segment of a run shows it is healthy:
                        // crashes are counted afresh from here.
                        heard_any = true;
                        let st = handle.state::<AppState>();
                        let mut v = st.voice.lock().unwrap();
                        if v.generation == generation {
                            v.failures = 0;
                        }
                    }
                    on_heard(&handle, generation, &t, Some(&inbox));
                }
                ear::EarEvent::State { state, detail } if state == "error" || state == "exited" => {
                    log::line("ear", format!("{state}: {}", if detail.is_empty() { "(no detail)" } else { detail.as_str() }));
                    let wants = handle.state::<AppState>().store.lock().unwrap().config.listen;
                    let (dead, failures) = {
                        let st = handle.state::<AppState>();
                        let mut v = st.voice.lock().unwrap();
                        if v.generation != generation {
                            break;
                        }
                        v.status.listening = false;
                        v.failures += 1;
                        (v.ear.take(), v.failures)
                    };
                    // Ear has no Drop: reap the child before letting it go.
                    if let Some(mut e) = dead {
                        e.stop();
                    }
                    // A locked Dictation switch needs a different message from an off one.
                    let shown = if detail.starts_with(ear::DICTATION_OFF) {
                        let managed = ear::dictation_managed_off();
                        if managed {
                            log::line("listener", "a management profile sets allowDictation to false");
                        }
                        ear::dictation_advice(managed).to_string()
                    } else if detail.is_empty() {
                        "the listener stopped".into()
                    } else {
                        detail.clone()
                    };
                    set_voice(&handle, generation, |s| {
                        s.state = "error".into();
                        s.detail = shown;
                    });
                    if wants {
                        if let Some(delay) = ear::restart_delay_ms(failures - 1) {
                            log::line("listener", format!("restart {failures} in {delay} ms"));
                            std::thread::sleep(Duration::from_millis(delay));
                            let still_wanted = handle.state::<AppState>().store.lock().unwrap().config.listen;
                            if still_wanted && is_current(&handle, generation) {
                                let _ = start_listening(&handle);
                            }
                        }
                    }
                    break;
                }
                ear::EarEvent::Device(name) => {
                    log::line("ear", format!("microphone: {name}"));
                    device_name = Some(name.clone());
                    set_voice(&handle, generation, |s| s.detail = format!("microphone: {name}"));
                }
                ear::EarEvent::State { state, .. } if state == "loading" => {
                    log::line("ear", "loading the model");
                    let detail = match &device_name {
                        Some(d) => format!("microphone: {d} (loading the model…)"),
                        None => "loading the model…".into(),
                    };
                    set_voice(&handle, generation, |s| s.detail = detail);
                }
                ear::EarEvent::State { state, detail } => {
                    log::line("ear", if detail.is_empty() { state.clone() } else { format!("{state}: {detail}") });
                    if state == "listening" {
                        let restored = device_name.as_ref().map(|d| format!("microphone: {d}")).unwrap_or_default();
                        set_voice(&handle, generation, |s| {
                            if s.detail.contains("loading the model") {
                                s.detail = restored.clone();
                            }
                        });
                    }
                }
                ear::EarEvent::Note(text) => {
                    log::line("ear", text);
                }
                _ => {}
            }
        }
    });
    Ok(())
}

pub(crate) fn stop_listening(app: &AppHandle) {
    log::line("listener", "stopping");
    let dead = {
        let state = app.state::<AppState>();
        let mut v = state.voice.lock().unwrap();
        v.generation += 1;
        v.flow = None;
        // The history outlives a run, and so does its counter.
        v.status = VoiceStatus { state: "off".into(), turns: v.status.turns, ..Default::default() };
        v.failures = 0;
        v.ear.take()
    };
    // Ear has no Drop: always stop it.
    if let Some(mut e) = dead {
        e.stop();
    }
    emit_voice(app);
}

#[tauri::command(async)]
pub(crate) fn voice_listen(app: AppHandle, state: TauriState<AppState>, on: bool) -> Result<(), String> {
    {
        let mut store = state.store.lock().unwrap();
        if on && store.config.network.role == config::NetworkRole::Assistant {
            return Err("The main Maya notifies and listens for this machine.".into());
        }
        store.config.listen = on;
        config::save(&store.config_path(), &store.config)?;
    }
    if on {
        state.voice.lock().unwrap().failures = 0;
        start_listening(&app)
    } else {
        stop_listening(&app);
        Ok(())
    }
}

/// The page's yes/no buttons; ignored unless a confirmation is waiting. A
/// tap after the window ran out drops the action and says so.
#[tauri::command(async)]
pub(crate) fn voice_confirm(app: AppHandle, yes: bool) {
    let (generation, expired) = {
        let state = app.state::<AppState>();
        let mut v = state.voice.lock().unwrap();
        let generation = v.generation;
        let expired = match v.flow.as_mut() {
            Some(f) if f.has_pending() => f.drop_expired(now_ms()),
            _ => return,
        };
        if expired {
            v.status.pending = None;
            v.status.state = "idle".into();
        }
        (generation, expired)
    };
    if expired {
        emit_voice(&app);
        reply_aloud(&app, generation, "That expired. Ask me again.");
        return;
    }
    on_heard(&app, generation, if yes { "yes" } else { "no" }, None);
}

#[tauri::command]
pub(crate) fn voice_status(state: TauriState<AppState>) -> VoiceStatus {
    state.voice.lock().unwrap().status.clone()
}

#[tauri::command]
pub(crate) fn voice_history(state: TauriState<AppState>) -> Vec<VoiceTurn> {
    state.voice.lock().unwrap().history.clone()
}

#[tauri::command(async)]
pub(crate) fn voice_selftest() -> Result<String, String> {
    let path = ear::sidecar_path().ok_or("The listener (maya-ear) is not built. Run `pnpm ear:build`.")?;
    let out = maya_core::command(path).arg("--selftest").output().map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The user's turn to remember for a heard segment's effects: the command
/// after the wake word, or the yes or no she recognised. Never the raw
/// segment, which can hold talk from before the wake word; the history goes
/// to the model, and only the command text may.
/// One line for the log: what the flow decided to do with a segment.
pub(crate) fn describe_effects(effects: &[wake::Effect]) -> String {
    if effects.is_empty() {
        return "nothing".into();
    }
    effects
        .iter()
        .map(|e| match e {
            wake::Effect::Say(s) => format!("say {s:?}"),
            wake::Effect::Interpret(c) => format!("command {c:?}"),
            wake::Effect::Execute(a) => format!("confirmed {}", a["kind"].as_str().unwrap_or("?")),
            wake::Effect::Cancelled => "cancelled".into(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn user_turn(effects: &[wake::Effect]) -> Option<String> {
    effects.iter().find_map(|e| match e {
        wake::Effect::Interpret(cmd) => Some(cmd.clone()),
        wake::Effect::Say(_) => Some("Maya".into()),
        wake::Effect::Execute(_) => Some("yes".into()),
        wake::Effect::Cancelled => Some("no".into()),
    })
}

/// `text` closed with a full stop unless it already ends a sentence.
fn sentence(text: &str) -> String {
    let t = text.trim();
    if t.ends_with(['.', '!', '?']) {
        t.to_string()
    } else {
        format!("{t}.")
    }
}

/// What Maya says for a validated action, built from the action itself and
/// never from the model's `say`: the read-back for actions that need a yes,
/// the line for focus and compact. None for a report, whose answer is the
/// model's `say`.
/// `owner/name` spoken as just `name`.
fn repo_name(repo: &str) -> &str {
    repo.rsplit('/').next().unwrap_or(repo)
}

pub(crate) fn spoken_for(action: &serde_json::Value) -> Option<String> {
    let s = |k: &str| action[k].as_str().unwrap_or("").trim().to_string();
    // " on <machine>" after the name (or the dir, for start/resume) when the
    // action runs on a remote machine; nothing for a local one.
    let on_machine = action["machine"].as_str().filter(|m| !m.is_empty()).map(|m| format!(" on {m}")).unwrap_or_default();
    Some(match action["kind"].as_str()? {
        "reply" => format!("Telling {}{on_machine}: {} Yes?", s("name"), sentence(&s("text"))),
        "answer" => format!("Answering {}{on_machine} with \"{}\". Yes?", s("name"), s("label")),
        "start" => format!("Starting a session in {}{on_machine}: {} Yes?", s("dir"), sentence(&s("prompt"))),
        // Resume always picks the newest session that is not running.
        "resume" => format!("Resuming the latest {}{on_machine} session. Yes?", s("dir")),
        "focus" => format!("Focusing {}{on_machine}.", s("name")),
        "compact" => format!("Compacting {}{on_machine}.", s("name")),
        "review" => format!("Reviewing {} #{}, {}. Yes?", repo_name(&s("repo")), action["number"].as_u64().unwrap_or(0), s("title")),
        "open" => format!("Opening {} #{}.", repo_name(&s("repo")), action["number"].as_u64().unwrap_or(0)),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn read_backs_come_from_the_validated_action() {
        let reply = json!({"kind":"reply","session":"id-1","name":"hexgrid-d3","text":"go ahead and ship it"});
        assert_eq!(spoken_for(&reply).unwrap(), "Telling hexgrid-d3: go ahead and ship it. Yes?");
        let asked = json!({"kind":"reply","session":"id-1","name":"coral","text":"ready?"});
        assert_eq!(spoken_for(&asked).unwrap(), "Telling coral: ready? Yes?");
        let answer = json!({"kind":"answer","session":"id-1","name":"hexgrid","option":2,"label":"SQLite","askId":5});
        assert_eq!(spoken_for(&answer).unwrap(), "Answering hexgrid with \"SQLite\". Yes?");
        let start = json!({"kind":"start","dir":"maya","prompt":"fix the build"});
        assert_eq!(spoken_for(&start).unwrap(), "Starting a session in maya: fix the build. Yes?");
        let resume = json!({"kind":"resume","dir":"maya"});
        assert_eq!(spoken_for(&resume).unwrap(), "Resuming the latest maya session. Yes?");
        assert_eq!(spoken_for(&json!({"kind":"focus","session":"id-1","name":"coral"})).unwrap(), "Focusing coral.");
        assert_eq!(spoken_for(&json!({"kind":"compact","session":"id-1","name":"coral"})).unwrap(), "Compacting coral.");
        assert_eq!(spoken_for(&json!({"kind":"review","repo":"dosaki/collector","number":14,"title":"Add ERD overlay"})).unwrap(), "Reviewing collector #14, Add ERD overlay. Yes?");
        assert_eq!(spoken_for(&json!({"kind":"open","repo":"dosaki/collector","number":14,"title":"Add ERD overlay"})).unwrap(), "Opening collector #14.");
        assert_eq!(spoken_for(&json!({"kind":"report"})), None, "a report speaks the model's own answer");
    }

    #[test]
    fn read_backs_name_the_machine() {
        assert_eq!(spoken_for(&json!({"kind":"reply","session":"id","name":"hexgrid","machine":"laptop","text":"go"})).unwrap(), "Telling hexgrid on laptop: go. Yes?");
        assert_eq!(spoken_for(&json!({"kind":"start","dir":"maya","machine":"laptop","prompt":"fix it"})).unwrap(), "Starting a session in maya on laptop: fix it. Yes?");
    }

    #[test]
    fn a_remote_session_cannot_be_focused_here() {
        assert_eq!(remote_focus_refusal(Some("laptop")).as_deref(), Some("That session runs on laptop; open it there."));
        assert_eq!(remote_focus_refusal(None), None);
    }

    #[test]
    fn only_the_command_or_the_answer_is_remembered_never_the_segment() {
        use wake::{Effect, Flow};
        let mut f = Flow::new();
        // Talk before the wake word stays out of the history.
        let heard = f.on_segment("the password is hunter2 anyway Maya what's waiting", 1000);
        assert_eq!(user_turn(&heard).as_deref(), Some("what's waiting"));
        assert_eq!(user_turn(&[Effect::Say("Yes?".into())]).as_deref(), Some("Maya"));
        assert_eq!(user_turn(&[Effect::Execute(json!({"kind":"reply"}))]).as_deref(), Some("yes"));
        assert_eq!(user_turn(&[Effect::Cancelled]).as_deref(), Some("no"));
        assert_eq!(user_turn(&[]), None);
    }

    #[test]
    fn each_remembered_turn_bumps_the_counter_even_once_the_history_is_full() {
        let mut v = VoiceState::default();
        for i in 0..45 {
            push_turn(&mut v, "user", &format!("turn {i}"), i);
        }
        assert_eq!(v.history.len(), 40, "the history keeps the last 40");
        assert_eq!(v.history.last().unwrap().text, "turn 44");
        assert_eq!(v.status.turns, 45, "the page refetches whenever this changes");
    }

    #[test]
    fn speech_queued_before_the_read_back_is_discarded_but_state_changes_are_kept() {
        use ear::EarEvent;
        let (tx, rx) = std::sync::mpsc::channel();
        let inbox = Inbox::new(rx);
        tx.send(EarEvent::Partial("ok".into())).unwrap();
        tx.send(EarEvent::Final("okay".into())).unwrap();
        tx.send(EarEvent::Level(0.3)).unwrap();
        tx.send(EarEvent::State { state: "exited".into(), detail: String::new() }).unwrap();
        tx.send(EarEvent::Final("sure".into())).unwrap();
        inbox.discard_heard();
        tx.send(EarEvent::Final("yes".into())).unwrap();
        assert_eq!(inbox.next(), Some(EarEvent::State { state: "exited".into(), detail: String::new() }), "a sidecar exit is never lost");
        assert_eq!(inbox.next(), Some(EarEvent::Final("yes".into())), "what comes after the drain is heard");
        drop(tx);
        assert_eq!(inbox.next(), None);
    }

    #[test]
    fn a_line_she_starts_to_say_is_not_taken_as_a_command_when_heard_back() {
        use crate::notify::SpeechPhase;
        let mut v = VoiceState { flow: Some(wake::Flow::new()), ..Default::default() };
        // The voice test and announcements go through the same hook as replies.
        on_speech(&mut v, &SpeechPhase::Starting("Maya here. hexgrid needs a decision".into()));
        on_speech(&mut v, &SpeechPhase::Finished);
        assert_eq!(v.flow.as_mut().unwrap().on_segment("Maya here hexgrid needs a decision", 1000), vec![]);
    }

    #[test]
    fn only_lines_of_three_words_or_more_mute_the_ear() {
        assert!(!pauses_ear_for("Yes?"));
        assert!(!pauses_ear_for("Cancelled."));
        assert!(pauses_ear_for("Telling hexgrid: go ahead. Yes?"));
        assert!(pauses_ear_for("Nothing is waiting on you."));
    }

    #[test]
    fn a_reply_that_ends_in_a_question_mark_asks_for_a_follow_up() {
        assert!(asks_question("Which one: Hexgrid 1 or Hexgrid 2?"));
        assert!(asks_question("I couldn't find voice test. Did you mean voice-test? "));
        assert!(!asks_question("Nothing is waiting on you."));
        // A read-back is a question too; the confirm window handles it before this check runs.
        assert!(asks_question("Telling hexgrid: ready? Yes?"));
        assert!(!asks_question(""));
    }

    #[test]
    fn changing_the_recogniser_restarts_a_running_listener() {
        use crate::config::{Config, Recognizer};
        // Starts from System, which Windows does not have, to cover both.
        let base = Config { recognizer: Recognizer::System, ..Default::default() };
        let off = base.clone();
        let on = Config { listen: true, ..base.clone() };
        let on_builtin = Config { listen: true, recognizer: Recognizer::Builtin, ..base.clone() };
        let on_tiny = Config { listen: true, recognizer: Recognizer::Builtin, whisper_model: "tiny.en".into(), ..base.clone() };
        let on_mic = Config { listen: true, microphone: Some("USB".into()), ..base.clone() };
        assert_eq!(listening_change(&off, &on), ListenChange::Start);
        assert_eq!(listening_change(&on, &off), ListenChange::Stop);
        assert_eq!(listening_change(&on, &on_builtin), ListenChange::Restart);
        assert_eq!(listening_change(&on_builtin, &on_tiny), ListenChange::Restart);
        assert_eq!(listening_change(&on, &on_mic), ListenChange::Restart);
        assert_eq!(listening_change(&off, &Config { recognizer: Recognizer::Builtin, ..base.clone() }), ListenChange::None, "no run to restart while off");
        assert_eq!(listening_change(&on, &Config { listen: true, completed_timeout_minutes: 5, ..base.clone() }), ListenChange::None);
        // The model picker is hidden under System: a stale whisperModel
        // value changing must not restart a System listener.
        assert_eq!(
            listening_change(&on, &Config { listen: true, whisper_model: "tiny.en".into(), ..base.clone() }),
            ListenChange::None,
            "a model change under System is not a restart"
        );
    }

    #[test]
    fn refuses_a_model_file_that_is_not_the_expected_size() {
        use crate::config::{Config, Recognizer};
        let dir = tempfile::tempdir().unwrap();
        let c = Config { recognizer: Recognizer::Builtin, ..Default::default() };
        let err = builtin_model_check(dir.path(), &c).unwrap_err();
        assert!(err.contains("Download the Base, quantised (60 MB, recommended) model in Settings first."), "{err}");
        let p = crate::models::model_path(dir.path(), "base.en-q5_1").unwrap();
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"truncated").unwrap();
        assert!(builtin_model_check(dir.path(), &c).is_err(), "a truncated file is refused");
        let sys = Config::default();
        assert!(builtin_model_check(dir.path(), &sys).is_err(), "system recogniser has no model path");
    }

    #[test]
    fn builtin_model_check_accepts_an_exact_size_file_and_rejects_an_unknown_model() {
        use crate::config::{Config, Recognizer};
        let dir = tempfile::tempdir().unwrap();
        let c = Config { recognizer: Recognizer::Builtin, ..Default::default() };
        let p = crate::models::model_path(dir.path(), "base.en-q5_1").unwrap();
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::File::create(&p).unwrap().set_len(59_721_011).unwrap();
        assert_eq!(builtin_model_check(dir.path(), &c), Ok(p));
        let unknown = Config { recognizer: Recognizer::Builtin, whisper_model: "nope".into(), ..Default::default() };
        let err = builtin_model_check(dir.path(), &unknown).unwrap_err();
        assert!(err.contains("Unknown model"), "{err}");
    }

    #[test]
    fn should_start_after_download_needs_listening_builtin_and_a_matching_model() {
        use crate::config::{Config, Recognizer};
        let base = Config { listen: true, recognizer: Recognizer::Builtin, whisper_model: "tiny.en".into(), ..Default::default() };
        assert!(should_start_after_download(&base, "tiny.en"));
        assert!(!should_start_after_download(&Config { listen: false, ..base.clone() }, "tiny.en"), "not while off");
        assert!(!should_start_after_download(&Config { recognizer: Recognizer::System, ..base.clone() }, "tiny.en"), "not for the system recogniser");
        assert!(!should_start_after_download(&base, "base.en-q5_1"), "not for a different model");
    }
}
