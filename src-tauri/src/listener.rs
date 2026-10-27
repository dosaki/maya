//! Voice assistant: the listener thread, the wake-word conversation, spoken
//! replies with listening paused around them, and announced actions that run
//! once the action window has passed.
//!
//! Lock rule: never hold the store, voice or window lock while speaking,
//! while the interpreter runs or while an action runs, and never take one of
//! them while holding another; take what is needed, release, then act.
//! `brain_app` extends the rule to the chat and brain locks.

use crate::store::now_ms;
use crate::{config, ear, interpreter, launch, log, notify, voice_chain, voices_wanted, wake};
use crate::{answer_pick, answer_question, compact_session, list_resumable_sessions, open_review_pr, resume_session, review_pr, send_reply, start_session, stop_session, AppState};
use maya_core::brain::chat::{ChatLog, ChatTurn};
use maya_core::window::{Batch, Via};
use serde_json::Value;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State as TauriState};

#[derive(Default)]
pub struct VoiceState {
    pub ear: Option<ear::Ear>,
    pub flow: Option<wake::Flow>,
    pub status: VoiceStatus,
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
    /// The announced batch waiting in the action window, as she said it.
    pub pending: Option<String>,
    /// When that batch runs (milliseconds since the epoch); None until her
    /// announcement has been said and the clock starts.
    pub until: Option<u64>,
    /// Turns ever added to the history; the page refetches it when this changes.
    pub turns: u64,
    /// The word she listens for, filled in from the config as the page reads it.
    pub word: String,
}

impl Default for VoiceStatus {
    fn default() -> Self {
        Self { listening: false, state: "off".into(), detail: String::new(), level: 0.0, heard: String::new(), said: String::new(), pending: None, until: None, turns: 0, word: wake::DEFAULT_WAKE_WORD.into() }
    }
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceTurn {
    pub who: String,
    pub text: String,
    pub at: u64,
}

/// The status as the page gets it. Takes the store lock, then the voice
/// lock, one after the other: the caller must hold neither.
fn page_status(state: &AppState) -> VoiceStatus {
    let word = state.store.lock().unwrap().config.listen_word();
    let mut status = state.voice.lock().unwrap().status.clone();
    status.word = word;
    status
}

pub(crate) fn emit_voice(app: &AppHandle) {
    let status = page_status(&app.state::<AppState>());
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

/// Adds a turn to the chat log, then sets the status counter to the log's
/// count. The chat lock is released before the voice lock is taken.
fn push_turn(chat: &Mutex<ChatLog>, voice: &Mutex<VoiceState>, who: &str, text: &str, at: u64, via: Via) {
    let turns = chat.lock().unwrap().push(ChatTurn { who: who.into(), text: text.into(), at, via });
    voice.lock().unwrap().status.turns = turns;
}

/// Remembers a turn; the caller's next `voice` event tells the page.
pub(crate) fn remember(app: &AppHandle, who: &str, text: &str, via: Via) {
    let state = app.state::<AppState>();
    let at = now_ms();
    push_turn(&state.chat, &state.voice, who, text, at, via);
    // No lock is held here: the page hears the turn, then the dot it may change.
    let turn = ChatTurn { who: who.into(), text: text.into(), at, via };
    let _ = app.emit("chat", &turn);
    if let Ok(turn) = serde_json::to_value(&turn) {
        crate::net_app::to_mirrors(app, crate::net::protocol::Down::Chat { turn });
    }
    crate::brain_app::emit_status(app);
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
    reply_aloud_as(app, generation, text, Via::Voice)
}

/// `reply_aloud`, with the line remembered as coming `via` that way.
pub(crate) fn reply_aloud_as(app: &AppHandle, generation: u64, text: &str, via: Via) -> bool {
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
    remember(app, "maya", text, via);
    emit_voice(app);
    let (muted, wanted, volume) = {
        let state = app.state::<AppState>();
        let store = state.store.lock().unwrap();
        (store.config.muted, voices_wanted(&store.config, store.claude_dir()), store.config.voice_loudness())
    };
    // Muted, the reply is only shown in the voice panel.
    if muted {
        return is_current(app, generation);
    }
    let voices = voice_chain(wanted);
    // No lock is held here: the speech hook takes the voice lock.
    let _ = notify::speak_and_wait(notify::Utterance { volume, ..notify::Utterance::new(text.to_string(), voices) });
    is_current(app, generation)
}

/// How a `resume_all` went: any session left behind makes it a failure, so
/// it is spoken after a batch (where only failures are).
fn resume_all_outcome(resumed: usize, failed: usize) -> Result<String, String> {
    let said = interpreter::resume_all_said(resumed, failed);
    if failed == 0 {
        Ok(said)
    } else {
        Err(said)
    }
}

/// Why "focus" refuses a card that runs on another machine: its pid means
/// nothing here, so the user is sent to open it there instead. None locally.
fn remote_focus_refusal(machine: Option<&str>) -> Option<String> {
    machine.map(|m| format!("That session runs on {m}; open it there."))
}

/// Runs a validated action through the same paths as the page's buttons;
/// the Ok text is what Maya says after an action that runs at once.
pub(crate) fn execute_action(app: &AppHandle, action: &serde_json::Value) -> Result<String, String> {
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
        "stop" => {
            stop_session(app.clone(), state.clone(), session)?;
            Ok("Stopping.".into())
        }
        "reply" if action["own"].as_bool() == Some(true) => {
            // The words go in the menu's own-words row, for the ask captured at validation.
            let ask_id = action["askId"].as_u64().ok_or("The question has changed; ask me again.")?;
            let text = action["text"].as_str().unwrap_or("").to_string();
            answer_pick(app.clone(), state.clone(), session, ask_id, 0, maya_core::answer::Pick::Own { text })?;
            Ok("Sent.".into())
        }
        "reply" => {
            let text = action["text"].as_str().unwrap_or("").to_string();
            send_reply(app.clone(), state.clone(), session, text, vec![])?;
            Ok("Sent.".into())
        }
        "answer" => {
            // The ask the user heard announced, captured at validation: a newer ask is refused.
            let ask_id = action["askId"].as_u64().ok_or("The question has changed; ask me again.")?;
            let n = action["option"].as_u64().unwrap_or(1) as usize;
            answer_question(app.clone(), state.clone(), session, ask_id, 0, n.saturating_sub(1))?;
            Ok("Answered.".into())
        }
        "resume" => {
            let dir = action["dir"].as_str().unwrap_or("").to_string();
            let brain = state.store.lock().unwrap().config.brain();
            let sessions = list_resumable_sessions(app.clone(), state.clone(), dir.clone(), machine.clone(), Some(brain))?;
            let latest = sessions.into_iter().find(|s| !s.running).ok_or("Nothing to resume there.")?;
            resume_session(app.clone(), state.clone(), dir, latest.id, machine, Some(brain))?;
            Ok("Resuming.".into())
        }
        "resume_all" => {
            let scope = action["scope"].as_str().unwrap_or("all");
            let rows = interpreter::resume_all_rows(&crate::remembered_app::rows(&state), scope);
            if rows.is_empty() {
                return Err("There's nothing to resume.".into());
            }
            let outcomes = crate::remembered_app::resume_rows(app, &rows);
            let failed = outcomes.iter().filter(|o| o.error.is_some()).count();
            resume_all_outcome(outcomes.len() - failed, failed)
        }
        "start" if action["scratch"].as_bool() == Some(true) => {
            let prompt = action["prompt"].as_str().unwrap_or("").to_string();
            let brain = state.store.lock().unwrap().config.brain();
            maya_core::actions::start_scratch(&crate::local(&state), prompt, launch::LaunchOptions { agent: brain, ..Default::default() })?;
            Ok("Started.".into())
        }
        "start" => {
            let dir = action["dir"].as_str().unwrap_or("").to_string();
            let prompt = action["prompt"].as_str().unwrap_or("").to_string();
            let brain = state.store.lock().unwrap().config.brain();
            start_session(app.clone(), state.clone(), Some(dir), prompt, launch::LaunchOptions { agent: brain, ..Default::default() }, machine)?;
            Ok("Started.".into())
        }
        "review" => {
            let repo = action["repo"].as_str().unwrap_or("").to_string();
            let number = action["number"].as_u64().unwrap_or(0);
            let forge = serde_json::from_value(action["forge"].clone()).ok();
            review_pr(state.clone(), forge, repo, number)?;
            Ok("Review started.".into())
        }
        "open" => {
            let repo = action["repo"].as_str().unwrap_or("").to_string();
            let number = action["number"].as_u64().unwrap_or(0);
            let forge = serde_json::from_value(action["forge"].clone()).ok();
            open_review_pr(state.clone(), forge, repo, number)?;
            Ok("Opened.".into())
        }
        other => Err(format!("I don't know how to {other}")),
    }
}

/// What a spoken command comes to before anything is said or run.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Interpreted {
    /// Say this; nothing runs.
    Say(String),
    /// Announce this, then run the actions once the window has passed.
    Announce { say: String, actions: Vec<Value> },
    /// Run this at once (focus, compact) and say what it did.
    Run(Value),
}

/// What the interpreter sees and checks an action against.
pub(crate) struct VoiceContext {
    pub cards: Vec<maya_core::model::Card>,
    pub dirs: Vec<String>,
    pub machines: Vec<String>,
    pub prs: Vec<maya_core::reviews::ReviewPr>,
    pub remembered: Vec<maya_core::remembered::RememberedRow>,
}

/// The board, the project dirs (a remote one as "<dir> (on <machine>)"),
/// the connected machines and the PRs, as the page shows them now.
pub(crate) fn voice_context(app: &AppHandle) -> VoiceContext {
    let state = app.state::<AppState>();
    let mut dirs = {
        let store = state.store.lock().unwrap();
        store.config.projects_dir_path().map(|r| launch::list_project_dirs(&r)).unwrap_or_default()
    };
    // Taken after `store` is released (lock order: `store` then `network`).
    let cards = crate::merged_cards(&state);
    let mut machines = vec![];
    for board in crate::remote_boards(&state).into_iter().filter(|b| b.connected) {
        for d in &board.dirs {
            dirs.push(format!("{d} (on {})", board.machine));
        }
        machines.push(board.machine);
    }
    let prs = state.reviews.lock().unwrap().prs.clone();
    let remembered = crate::remembered_app::rows(&state);
    VoiceContext { cards, dirs, machines, prs, remembered }
}

/// The interpreter's reply as one of the three outcomes. The announcement
/// is what will run (`spoken_for`), never the model's `say`.
pub(crate) fn decide(reply: &interpreter::Reply, ctx: &VoiceContext) -> Interpreted {
    let Some(proposed) = reply.action.as_ref().filter(|a| a["kind"] != "report") else {
        return Interpreted::Say(reply.say.clone());
    };
    let validated = if proposed["kind"] == "resume_all" { interpreter::validate_resume_all(proposed, &ctx.remembered) } else { interpreter::validate(proposed, &ctx.cards, &ctx.dirs, &ctx.machines, &ctx.prs) };
    match validated {
        Err(why) => Interpreted::Say(why),
        Ok(action) if interpreter::waits(&action) => {
            let say = spoken_for(&action).unwrap_or_else(|| "Running that.".into());
            Interpreted::Announce { say, actions: vec![action] }
        }
        Ok(action) => Interpreted::Run(action),
    }
}

/// (user, maya) pairs from the last 12 turns, at most 6, oldest first.
pub(crate) fn exchanges<'a>(turns: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<(String, String)> {
    let turns: Vec<(&str, &str)> = turns.into_iter().collect();
    let mut pairs = Vec::new();
    let mut last_user: Option<&str> = None;
    for (who, text) in &turns[turns.len().saturating_sub(12)..] {
        if *who == "user" {
            last_user = Some(text);
        } else if let Some(u) = last_user.take() {
            pairs.push((u.to_string(), text.to_string()));
        }
    }
    let skip = pairs.len().saturating_sub(6);
    pairs.split_off(skip)
}

/// Asks Maya's agent what `cmd` means. Speaks nothing and stores nothing:
/// the desktop listener and a phone's request both act on the outcome.
pub(crate) fn interpret_text(app: &AppHandle, cmd: &str, history: &[(String, String)]) -> Interpreted {
    let (brain, model, maya_dir) = {
        let state = app.state::<AppState>();
        let store = state.store.lock().unwrap();
        (store.config.brain(), store.config.brain_model().map(String::from), store.claude_dir().join("maya"))
    };
    let ctx = voice_context(app);
    let Ok(binary) = launch::find_binary(brain) else {
        return Interpreted::Say(format!("I can't find the {} command.", launch::binary_name(brain)));
    };
    match interpreter::run(brain, &binary, model.as_deref(), cmd, &ctx.cards, &ctx.prs, now_ms(), history, &maya_dir, interpreter::TIMEOUT) {
        Ok(reply) => decide(&reply, &ctx),
        Err(interpreter::RunError::TimedOut) => Interpreted::Say("Sorry, that took too long.".into()),
        Err(_) => Interpreted::Say("Sorry, I didn't catch that.".into()),
    }
}

/// The last exchanges as (user, maya) pairs, oldest first, for the interpreter.
pub(crate) fn recent_exchanges(app: &AppHandle) -> Vec<(String, String)> {
    let state = app.state::<AppState>();
    let turns = state.chat.lock().unwrap().last(12);
    exchanges(turns.iter().map(|t| (t.who.as_str(), t.text.as_str())))
}

/// Says `text`, then shows idle; both only while run `generation` is current.
pub(crate) fn reply_then_idle(app: &AppHandle, generation: u64, text: &str, inbox: Option<&Inbox>) {
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

/// Her reply to a spoken request that went to her session: said aloud (only
/// shown when muted) for whoever listens now, then the flow's next state as
/// `reply_then_idle` sets it; with the listener off it is only said. Blocks
/// while speaking: the caller must hold no lock.
pub(crate) fn answer_aloud(app: &AppHandle, text: &str) {
    let (generation, listening) = {
        let state = app.state::<AppState>();
        let v = state.voice.lock().unwrap();
        (v.generation, v.status.listening)
    };
    if listening {
        reply_then_idle(app, generation, text, None);
    } else {
        reply_aloud(app, generation, text);
    }
}

/// Sends a spoken command to her session when it runs, else to the one-shot
/// interpreter, and acts on the interpreter's reply. Every step after the
/// interpreter returns checks that run `generation` is still current, so a
/// stop while she thinks leaves nothing behind. Her session's reply comes
/// later, through `brain_app`; the state shows "thinking" until then.
fn interpret(app: &AppHandle, generation: u64, cmd: &str, inbox: Option<&Inbox>) {
    let started = set_voice(app, generation, |st| {
        st.state = "thinking".into();
        st.pending = None;
        st.until = None;
    });
    if !started {
        return;
    }
    if crate::brain_app::ready(app) {
        log::line("action", "asking her session");
        crate::brain_app::ask(app, cmd, Via::Voice);
        return;
    }
    let outcome = interpret_text(app, cmd, &recent_exchanges(app));
    act_on(app, generation, outcome, inbox);
}

/// Says, announces or runs what the interpreter made of a spoken command,
/// for listener run `generation`.
pub(crate) fn act_on(app: &AppHandle, generation: u64, outcome: Interpreted, inbox: Option<&Inbox>) {
    if !is_current(app, generation) {
        log::line("listener", "run replaced while thinking; reply dropped");
        return;
    }
    match outcome {
        Interpreted::Say(s) => {
            log::line("action", "nothing to run");
            reply_then_idle(app, generation, &s, inbox)
        }
        Interpreted::Announce { say, actions } => {
            log::line("action", format!("announcing: {}", actions.iter().map(Value::to_string).collect::<Vec<_>>().join(", ")));
            announce(app, Some(generation), say, actions, Via::Voice);
            // Anything heard before the announcement ended (an "okay" said to
            // someone while she was thinking) sits queued and would settle a
            // batch the user had not heard yet. Throw it away: a person who
            // spoke before the announcement must say it again, which is safer
            // than a stray word running or correcting it.
            if let Some(inbox) = inbox {
                let n = inbox.discard_heard();
                if n > 0 {
                    log::line("listener", format!("discarded {n} segment(s) heard before the announcement ended"));
                }
            }
        }
        Interpreted::Run(action) => {
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

/// Handles one final segment heard during listener run `generation`.
/// Nothing happens once that run is stopped or replaced.
fn on_heard(app: &AppHandle, generation: u64, text: &str, inbox: Option<&Inbox>) {
    // Read first: the store lock is not held with the voice lock.
    let word = app.state::<AppState>().store.lock().unwrap().config.listen_word();
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
            Some(f) => {
                f.set_word(&word);
                f.on_segment(text, now)
            }
            None => vec![],
        };
        log::line("wake", format!("heard ({before}): {text:?} → {}", describe_effects(&effects)));
        // Nothing to do, but an announced batch may still be waiting
        // (chatter, or her own line heard back): show the flow's state.
        // When not listening, "off" or "error" stays as it is.
        if effects.is_empty() && v.status.listening {
            let shown = v.flow.as_ref().map_or("idle", |f| f.state(now));
            v.status.state = shown.into();
            if shown != "window" {
                v.status.pending = None;
                v.status.until = None;
            }
        }
        effects
    };
    if effects.is_empty() {
        emit_voice(app);
        return;
    }
    if let Some(turn) = user_turn(&effects, &word) {
        remember(app, "user", &turn, Via::Voice);
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
            wake::Effect::RunNow => {
                let taken = app.state::<AppState>().window.lock().unwrap().take_now();
                match taken {
                    Some(batch) => {
                        log::line("action", format!("batch {} runs now on a yes", batch.id));
                        run_now(app, batch);
                    }
                    // Its timer took it a moment ago: it is running already.
                    None => settle(app, "idle"),
                }
            }
            wake::Effect::Correct(cmd) => {
                let dropped = app.state::<AppState>().window.lock().unwrap().cancel();
                if let Some(batch) = dropped {
                    log::line("action", format!("batch {} dropped for a correction", batch.id));
                }
                settle(app, "idle");
                interpret(app, generation, &cmd, inbox);
            }
            wake::Effect::Cancelled => {
                let dropped = app.state::<AppState>().window.lock().unwrap().cancel();
                if let Some(batch) = &dropped {
                    log::line("action", format!("batch {} cancelled", batch.id));
                }
                settle(app, "idle");
                // Nothing left to cancel: its timer took it as the stop was heard.
                reply_aloud(app, generation, if dropped.is_some() { "Cancelled." } else { "Too late." });
            }
            wake::Effect::Stop => crate::brain_app::stop(app, Via::Voice),
        }
    }
}

/// Shows the announced batch in the panel; `until` once its clock runs.
/// The state reads "window" only while listening: "off" and "error" stay.
fn show_window(v: &mut VoiceState, say: &str, until: Option<u64>) {
    v.status.pending = Some(say.to_string());
    v.status.until = until;
    if v.status.listening {
        v.status.state = "window".into();
    }
}

/// Ends the flow's window and clears the panel's batch, once it ran or was
/// dropped; a state still showing the window becomes `next`.
fn close_window(v: &mut VoiceState, next: &str) {
    if let Some(f) = v.flow.as_mut() {
        f.close_window();
    }
    v.status.pending = None;
    v.status.until = None;
    if v.status.listening && v.status.state == "window" {
        v.status.state = next.into();
    }
}

/// `close_window` on the voice state, then tells the page and her Chat.
/// Takes the voice lock: the caller must hold none.
pub(crate) fn settle(app: &AppHandle, next: &str) {
    {
        let state = app.state::<AppState>();
        let mut v = state.voice.lock().unwrap();
        close_window(&mut v, next);
    }
    emit_voice(app);
    crate::brain_app::emit_status(app);
}

pub(crate) fn current_generation(app: &AppHandle) -> u64 {
    app.state::<AppState>().voice.lock().unwrap().generation
}

/// Says `text` aloud for whoever is listening now (a muted Maya only shows
/// it). Blocks while speaking: the caller must hold no lock.
pub(crate) fn say(app: &AppHandle, text: &str) {
    reply_aloud(app, current_generation(app), text);
}

/// Drops batch `id` if it is still the one waiting.
fn drop_batch(app: &AppHandle, id: u64) {
    let state = app.state::<AppState>();
    let mut w = state.window.lock().unwrap();
    if w.pending().map(|b| b.id) == Some(id) {
        w.cancel();
    }
}

/// Takes a batch that was asked for by voice out of the window: once its
/// listener run stops or restarts, nobody can stop it by voice, and a request
/// in the next run must not join it and inherit its deadline. A batch from
/// Chat, the phone or Maya herself stays.
fn drop_voice_batch(w: &mut maya_core::window::Window) -> Option<Batch> {
    match w.pending() {
        Some(b) if b.origin == Via::Voice => w.cancel(),
        _ => None,
    }
}

/// `drop_voice_batch` when listening stops or restarts, saying why in the
/// log. Takes the window lock: the caller must hold none.
fn drop_voice_batch_because(app: &AppHandle, why: &str) {
    let dropped = drop_voice_batch(&mut app.state::<AppState>().window.lock().unwrap());
    if let Some(batch) = dropped {
        log::line("action", format!("dropped announced action (batch {}, {:?}): {why}", batch.id, batch.say));
        crate::brain_app::emit_status(app);
    }
}

/// Whether a batch from `origin` is announced and its failures reported
/// aloud (a muted Maya only shows them): a spoken request's and her own.
/// A typed or phone request's go to Chat as text.
pub(crate) fn aloud(origin: Via) -> bool {
    matches!(origin, Via::Voice | Via::Maya)
}

/// `announce_after` with nothing said before the announcement.
pub(crate) fn announce(app: &AppHandle, generation: Option<u64>, say: String, actions: Vec<Value>, origin: Via) {
    announce_after(app, generation, "", say, actions, origin);
}

/// Announces `actions` and runs them once the window has passed, unless a
/// stop, a correction or the Cancel button drops them first. Opens a batch
/// or joins the open one (`Window::open`); a batch from another user surface
/// is cancelled first and "Cancelled: …" is posted to Chat, so two surfaces
/// never share one window. A spoken request, or one from Maya herself, is
/// said aloud (a muted Maya only shows it) and stopped by voice; the clock
/// starts only once the announcement has been said, so the user always gets
/// the whole window after hearing it. A voice request's batch is dropped
/// when its listener run stops or restarts (`drop_voice_batch`), during the
/// announcement or after it. Blocks while speaking: the caller must hold no lock.
///
/// `lead`, her own words, goes first in the same utterance (or Chat line),
/// so a short reply of hers is never said on its own just before the
/// window opens. A typed request's announcement is posted to Chat.
pub(crate) fn announce_after(app: &AppHandle, generation: Option<u64>, lead: &str, say: String, actions: Vec<Value>, origin: Via) {
    let state = app.state::<AppState>();
    let (id, shown, cancelled) = {
        let mut w = state.window.lock().unwrap();
        let (id, cancelled) = w.open(say.clone(), actions, origin);
        (id, w.pending().map(|b| b.say.clone()).unwrap_or_default(), cancelled)
    };
    if let Some(old) = cancelled {
        log::line("action", format!("batch {} cancelled by a request from elsewhere ({origin:?})", old.id));
        if matches!(old.origin, Via::Voice | Via::Maya) {
            close_window(&mut state.voice.lock().unwrap(), "idle");
        }
        remember(app, "maya", &format!("Cancelled: {}", old.say), old.origin);
    }
    log::line("action", format!("batch {id} announced: {say}"));
    let spoken = aloud(origin);
    let line = crate::brain_route::one_line(&[lead, say.as_str()]);
    // Only the listener run that heard the request opens its flow's window.
    let ours = |v: &VoiceState| generation.map_or(true, |g| v.generation == g);
    if spoken {
        // Open before she speaks, so a stop or the Cancel button works from
        // the first word; the deadline is set once she has finished.
        let mut v = state.voice.lock().unwrap();
        if ours(&v) {
            if let Some(f) = v.flow.as_mut() {
                f.open_window(u64::MAX);
            }
            show_window(&mut v, &shown, None);
        }
    }
    if !spoken {
        // A typed request leaves a record of what will run.
        remember(app, "maya", &line, origin);
    }
    emit_voice(app);
    crate::brain_app::emit_status(app);
    if spoken {
        let run = generation.unwrap_or_else(|| current_generation(app));
        if !reply_aloud(app, run, &line) && origin == Via::Voice {
            log::line("action", format!("listener run replaced while announcing; batch {id} dropped"));
            drop_batch(app, id);
            crate::brain_app::emit_status(app);
            return;
        }
    }
    let started = {
        let mut w = state.window.lock().unwrap();
        w.start_clock(id, now_ms()).map(|until| (until, w.pending().map(|b| b.say.clone()).unwrap_or_default()))
    };
    // Cancelled, corrected or run while she spoke: whoever did it settled the panel.
    let Some((until, shown)) = started else { return };
    if spoken {
        let mut v = state.voice.lock().unwrap();
        // A cancel between the clock starting and here has closed the window already.
        if ours(&v) && v.flow.as_ref().map_or(false, |f| f.state(now_ms()) == "window") {
            if let Some(f) = v.flow.as_mut() {
                f.open_window(until);
            }
            show_window(&mut v, &shown, Some(until));
        }
    }
    emit_voice(app);
    crate::brain_app::emit_status(app);
    let handle = app.clone();
    std::thread::spawn(move || wait_then_run(&handle, id, until));
}

/// The timer of batch `id`: sleeps until just past its deadline, then runs it
/// unless it was cancelled, corrected, dropped when listening stopped, or
/// already run.
fn wait_then_run(app: &AppHandle, id: u64, mut until: u64) {
    let batch = loop {
        std::thread::sleep(Duration::from_millis(until.saturating_sub(now_ms()) + 50));
        let state = app.state::<AppState>();
        let mut w = state.window.lock().unwrap();
        match w.pending().map(|b| (b.id, b.until)) {
            Some((pending, deadline)) if pending == id => match w.take_due(id, now_ms()) {
                Some(batch) => break batch,
                // The clock went back: wait for the deadline again.
                None => until = deadline,
            },
            _ => return,
        }
    };
    log::line("action", format!("batch {} runs: its window passed", batch.id));
    run_now(app, batch);
}

/// Runs a batch taken from the window and reports how it went.
fn run_now(app: &AppHandle, batch: Batch) {
    let origin = batch.origin;
    settle(app, "thinking");
    let results = run_batch(app, batch);
    on_batch_done(app, results, origin);
}

/// Runs each action of `batch` in order, through the page's own paths.
pub(crate) fn run_batch(app: &AppHandle, batch: Batch) -> Vec<(Value, Result<String, String>)> {
    batch
        .actions
        .into_iter()
        .map(|action| {
            log::line("action", format!("running: {action}"));
            let outcome = execute_action(app, &action);
            if let Err(e) = &outcome {
                log::line("action", format!("failed: {e}"));
            }
            (action, outcome)
        })
        .collect()
}

/// What she says once a batch ran: only its failures, since the
/// announcement already said what it would do. None when all went well.
pub(crate) fn failures_said(results: &[(Value, Result<String, String>)]) -> Option<String> {
    let failed: Vec<String> = results.iter().filter_map(|(_, r)| r.as_ref().err()).map(|e| sentence(e)).collect();
    (!failed.is_empty()).then(|| failed.join(" "))
}

/// A batch has run: leaves her a note per action, reports its failures
/// (`report_failures`), then shows idle.
pub(crate) fn on_batch_done(app: &AppHandle, results: Vec<(Value, Result<String, String>)>, origin: Via) {
    crate::brain_app::batch_done(app, &results);
    report_failures(app, &results, origin);
    {
        let state = app.state::<AppState>();
        let mut v = state.voice.lock().unwrap();
        if v.status.listening && v.status.state == "thinking" {
            v.status.state = v.flow.as_ref().map_or("idle", |f| f.state(now_ms())).into();
        }
    }
    emit_voice(app);
}

/// Says a batch's failures aloud when it was spoken or hers (a muted Maya
/// only shows them), and posts them to Chat when it was typed or came from
/// the phone. Blocks while speaking: the caller must hold no lock.
pub(crate) fn report_failures(app: &AppHandle, results: &[(Value, Result<String, String>)], origin: Via) {
    let Some(line) = failures_said(results) else { return };
    if aloud(origin) {
        say(app, &line);
    } else {
        remember(app, "maya", &line, origin);
        emit_voice(app);
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
/// (an announced batch shown, partial transcript, last spoken line) so nothing
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
        v.status.until = None;
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
    drop_voice_batch_because(app, "listening restarted");
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
            v.status.until = None;
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
    drop_voice_batch_because(app, "listening stopped");
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

/// The page's Cancel button: drops the announced batch while it waits. A
/// spoken one is answered aloud, as a spoken stop would be.
#[tauri::command(async)]
pub(crate) fn window_cancel(app: AppHandle) {
    let dropped = app.state::<AppState>().window.lock().unwrap().cancel();
    let Some(batch) = dropped else { return };
    log::line("action", format!("batch {} cancelled from the page", batch.id));
    settle(&app, "idle");
    if matches!(batch.origin, Via::Voice | Via::Maya) {
        say(&app, "Cancelled.");
    }
}

/// The whole conversation, voice and typed, oldest first.
#[tauri::command]
pub(crate) fn chat_history(state: TauriState<AppState>) -> Vec<ChatTurn> {
    state.chat.lock().unwrap().last(maya_core::brain::chat::KEEP)
}

/// The Chat tab's input. An open batch is dropped first: typing is a
/// correction. Her session answers; without it the quick interpreter does.
#[tauri::command(async)]
pub(crate) fn chat_send(app: AppHandle, text: String) -> Result<(), String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(());
    }
    // On an assistant her brain is dormant and Chat is read-only.
    if let Some(why) = crate::brain_route::chat_refusal(&app.state::<AppState>().store.lock().unwrap().config) {
        return Err(why.into());
    }
    window_cancel(app.clone());
    crate::brain_app::ask(&app, text, Via::Chat);
    Ok(())
}

/// The user has looked at Chat: the dot stops being "unread".
#[tauri::command]
pub(crate) fn chat_seen(app: AppHandle) {
    app.state::<AppState>().chat.lock().unwrap().seen(now_ms());
    crate::brain_app::emit_status(&app);
}

#[tauri::command]
pub(crate) fn voice_status(state: TauriState<AppState>) -> VoiceStatus {
    page_status(&state)
}

#[tauri::command]
pub(crate) fn voice_history(state: TauriState<AppState>) -> Vec<VoiceTurn> {
    let turns = state.chat.lock().unwrap().last(40);
    turns.into_iter().map(|t| VoiceTurn { who: t.who, text: t.text, at: t.at }).collect()
}

#[tauri::command(async)]
pub(crate) fn voice_selftest() -> Result<String, String> {
    let path = ear::sidecar_path().ok_or("The listener (maya-ear) is not built. Run `pnpm ear:build`.")?;
    let out = maya_core::command(path).arg("--selftest").output().map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

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
            wake::Effect::RunNow => "run now".into(),
            wake::Effect::Correct(c) => format!("correction {c:?}"),
            wake::Effect::Cancelled => "cancelled".into(),
            wake::Effect::Stop => "stop".into(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The user's turn to remember for a heard segment's effects: the command
/// after the wake word (or the correction), or the yes, no or stop she
/// recognised. Never the raw segment, which can hold talk from before the
/// wake word; the history goes to the model, and only the command text may.
pub(crate) fn user_turn(effects: &[wake::Effect], word: &str) -> Option<String> {
    effects.iter().find_map(|e| match e {
        wake::Effect::Interpret(cmd) => Some(cmd.clone()),
        wake::Effect::Say(_) => Some(word.into()),
        wake::Effect::Correct(cmd) => Some(cmd.clone()),
        wake::Effect::RunNow => Some("yes".into()),
        wake::Effect::Cancelled => Some("no".into()),
        wake::Effect::Stop => Some("stop".into()),
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

/// `owner/name` spoken as just `name`.
fn repo_name(repo: &str) -> &str {
    repo.rsplit('/').next().unwrap_or(repo)
}

/// What Maya says for a validated action, built from the action itself and
/// never from the model's `say`: the announcement for actions that wait in
/// the window, the line for focus and compact. None for a report, whose
/// answer is the model's `say`.
pub(crate) fn spoken_for(action: &serde_json::Value) -> Option<String> {
    let s = |k: &str| action[k].as_str().unwrap_or("").trim().to_string();
    // " on <machine>" after the name (or the dir, for start/resume) when the
    // action runs on a remote machine; nothing for a local one.
    let on_machine = action["machine"].as_str().filter(|m| !m.is_empty()).map(|m| format!(" on {m}")).unwrap_or_default();
    Some(match action["kind"].as_str()? {
        "reply" => format!("Telling {}{on_machine}: {}", s("name"), sentence(&s("text"))),
        "answer" => format!("Answering {}{on_machine} with \"{}\".", s("name"), s("label")),
        "start" if action["scratch"].as_bool() == Some(true) => format!("Starting a session in a scratch folder: {}", sentence(&s("prompt"))),
        "start" => format!("Starting a session in {}{on_machine}: {}", s("dir"), sentence(&s("prompt"))),
        // Resume always picks the newest session that is not running.
        "resume" => format!("Resuming the latest {}{on_machine} session.", s("dir")),
        "resume_all" => interpreter::resume_all_announcement(action),
        "focus" => format!("Focusing {}{on_machine}.", s("name")),
        "compact" => format!("Compacting {}{on_machine}.", s("name")),
        "stop" => format!("Stopping {}{on_machine}.", s("name")),
        "review" => sentence(&format!("Reviewing {} #{}, {}", repo_name(&s("repo")), action["number"].as_u64().unwrap_or(0), s("title"))),
        "open" => format!("Opening {} #{}.", repo_name(&s("repo")), action["number"].as_u64().unwrap_or(0)),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use maya_core::model::{AwaitKind, Awaiting, Card, Harness, State};

    fn card(id: &str, name: &str, ask: &str) -> Card {
        Card {
            session_id: id.into(), pid: 1, name: name.into(), cwd: format!("/Users/x/dev/{name}"), state: State::Awaiting, state_since: 42, snippet: "".into(),
            awaiting: Some(Awaiting { kind: AwaitKind::Text, detail: ask.into(), questions: vec![] }),
            has_inbox: true, harness: Harness::ClaudeCode, pr: None, context: None, machine: None, machine_address: None, machine_platform: None, terminal: None, stale: false, model: None, branch: None, brain: false, shell: false,
        }
    }

    fn ctx() -> VoiceContext {
        VoiceContext { cards: vec![card("s1", "hexgrid", "Ship it?")], dirs: vec![], machines: vec![], prs: vec![], remembered: vec![] }
    }

    fn reply(say: &str, action: Option<serde_json::Value>) -> interpreter::Reply {
        interpreter::Reply { say: say.into(), action, confirm: false }
    }

    #[test]
    fn a_report_is_only_said() {
        assert_eq!(decide(&reply("Two sessions need you.", None), &ctx()), Interpreted::Say("Two sessions need you.".into()));
        assert_eq!(decide(&reply("All quiet.", Some(serde_json::json!({"kind": "report"}))), &ctx()), Interpreted::Say("All quiet.".into()));
    }

    #[test]
    fn a_reply_is_announced_without_asking() {
        let out = decide(&reply("", Some(json!({"kind":"reply","session":"hexgrid","text":"go ahead"}))), &ctx());
        match out {
            Interpreted::Announce { say, actions } => {
                assert_eq!(say, "Telling hexgrid: go ahead.");
                assert!(!say.ends_with("Yes?"));
                assert_eq!(actions.len(), 1);
                assert_eq!(actions[0]["session"], "s1", "the announced action is the validated one");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn announcements_and_failures_are_said_for_voice_and_her_and_posted_for_chat_and_the_phone() {
        assert!(aloud(Via::Voice));
        assert!(aloud(Via::Maya));
        assert!(!aloud(Via::Chat));
        assert!(!aloud(Via::Phone));
    }

    #[test]
    fn compact_still_runs_at_once() {
        let r = reply("Compacting.", Some(json!({"kind": "compact", "session": "hexgrid"})));
        assert!(matches!(decide(&r, &ctx()), Interpreted::Run(a) if a["kind"] == "compact"));
    }

    #[test]
    fn spoken_stop() {
        assert_eq!(spoken_for(&json!({"kind":"stop","session":"id-1","name":"hexgrid"})).unwrap(), "Stopping hexgrid.");
        assert_eq!(spoken_for(&json!({"kind":"stop","session":"id-1","name":"hexgrid","machine":"laptop"})).unwrap(), "Stopping hexgrid on laptop.");
        let r = reply("", Some(json!({"kind": "stop", "session": "hexgrid"})));
        let mut c = ctx();
        c.cards[0].state = maya_core::model::State::Working;
        c.cards[0].awaiting = None;
        assert!(matches!(decide(&r, &c), Interpreted::Run(a) if a["kind"] == "stop"), "a stop runs at once");
    }

    fn with_question(own: bool, kind: AwaitKind) -> VoiceContext {
        use maya_core::model::{Choice, OwnRow, Question};
        let mut c = ctx();
        let aw = c.cards[0].awaiting.as_mut().unwrap();
        aw.kind = kind;
        aw.questions = vec![Question { question: "Which?".into(), header: "".into(), multi_select: false, own: own.then(|| OwnRow { label: "Type something.".into(), row: 3 }), chat_row: None, options: vec![Choice { label: "A".into(), description: "".into() }] }];
        c
    }

    #[test]
    fn a_spoken_reply_to_a_question_is_its_own_answer() {
        let r = reply("", Some(json!({"kind":"reply","session":"hexgrid","text":"MariaDB"})));
        let Interpreted::Announce { actions, .. } = decide(&r, &with_question(true, AwaitKind::Question)) else { panic!("expected an announcement") };
        assert_eq!((actions[0]["own"].as_bool(), actions[0]["askId"].as_u64()), (Some(true), Some(42)));
    }

    #[test]
    fn a_spoken_reply_to_a_permission_is_refused() {
        let r = reply("", Some(json!({"kind":"reply","session":"hexgrid","text":"yes"})));
        assert_eq!(decide(&r, &with_question(false, AwaitKind::Permission)), Interpreted::Say("Pick one of its numbered options, or answer it in the terminal.".into()));
    }

    #[test]
    fn an_action_on_nothing_says_why() {
        let r = reply("Done.", Some(serde_json::json!({"kind": "reply", "session": "nope", "text": "x"})));
        assert!(matches!(decide(&r, &ctx()), Interpreted::Say(_)));
    }

    #[test]
    fn exchanges_pair_the_last_twelve_turns_into_at_most_six() {
        let turns: Vec<(String, String)> = (0..20).flat_map(|i| [("user".to_string(), format!("u{i}")), ("maya".to_string(), format!("m{i}"))]).collect();
        let pairs = exchanges(turns.iter().map(|(w, t)| (w.as_str(), t.as_str())));
        assert_eq!(pairs.len(), 6);
        assert_eq!(pairs.last().unwrap(), &("u19".to_string(), "m19".to_string()));
        assert_eq!(pairs.first().unwrap(), &("u14".to_string(), "m14".to_string()));
    }

    use serde_json::json;

    fn remembered_row(machine: Option<&str>, id: &str) -> maya_core::remembered::RememberedRow {
        maya_core::remembered::RememberedRow { machine: machine.map(String::from), waiting: false, entry: maya_core::remembered::Remembered { agent: Harness::ClaudeCode, session_id: id.into(), dir: "/x".into(), name: id.into(), first_seen_ms: 1, last_seen_ms: 2, gone_at_ms: None, offered: true, offered_at_ms: None, error: None } }
    }

    #[test]
    fn resume_all_is_announced_with_the_split_and_nothing_to_resume_is_only_said() {
        let asked = Some(json!({"kind": "resume_all", "scope": "all"}));
        let with_rows = VoiceContext { remembered: vec![remembered_row(None, "a"), remembered_row(Some("Gnowee"), "g")], ..ctx() };
        let Interpreted::Announce { say, actions } = decide(&reply("", asked.clone()), &with_rows) else { panic!("expected an announcement") };
        assert_eq!(say, "Resuming 2 sessions, 1 here and 1 on Gnowee.");
        assert_eq!(actions[0]["count"], 2);
        assert_eq!(decide(&reply("", asked), &ctx()), Interpreted::Say("There's nothing to resume.".into()));
    }

    #[test]
    fn announcements_come_from_the_validated_action_and_never_ask() {
        let reply = json!({"kind":"reply","session":"id-1","name":"hexgrid-d3","text":"go ahead and ship it"});
        assert_eq!(spoken_for(&reply).unwrap(), "Telling hexgrid-d3: go ahead and ship it.");
        let asked = json!({"kind":"reply","session":"id-1","name":"coral","text":"ready?"});
        assert_eq!(spoken_for(&asked).unwrap(), "Telling coral: ready?");
        let answer = json!({"kind":"answer","session":"id-1","name":"hexgrid","option":2,"label":"SQLite","askId":5});
        assert_eq!(spoken_for(&answer).unwrap(), "Answering hexgrid with \"SQLite\".");
        let start = json!({"kind":"start","dir":"maya","prompt":"fix the build"});
        assert_eq!(spoken_for(&start).unwrap(), "Starting a session in maya: fix the build.");
        let resume = json!({"kind":"resume","dir":"maya"});
        assert_eq!(spoken_for(&resume).unwrap(), "Resuming the latest maya session.");
        assert_eq!(spoken_for(&json!({"kind":"focus","session":"id-1","name":"coral"})).unwrap(), "Focusing coral.");
        assert_eq!(spoken_for(&json!({"kind":"compact","session":"id-1","name":"coral"})).unwrap(), "Compacting coral.");
        assert_eq!(spoken_for(&json!({"kind":"review","repo":"dosaki/collector","number":14,"title":"Add ERD overlay"})).unwrap(), "Reviewing collector #14, Add ERD overlay.");
        assert_eq!(spoken_for(&json!({"kind":"open","repo":"dosaki/collector","number":14,"title":"Add ERD overlay"})).unwrap(), "Opening collector #14.");
        assert_eq!(spoken_for(&json!({"kind":"start","scratch":true,"prompt":"try the new parser"})).unwrap(), "Starting a session in a scratch folder: try the new parser.");
        assert_eq!(spoken_for(&json!({"kind":"report"})), None, "a report speaks the model's own answer");
    }

    #[test]
    fn announcements_name_the_machine() {
        assert_eq!(spoken_for(&json!({"kind":"reply","session":"id","name":"hexgrid","machine":"laptop","text":"go"})).unwrap(), "Telling hexgrid on laptop: go.");
        assert_eq!(spoken_for(&json!({"kind":"start","dir":"maya","machine":"laptop","prompt":"fix it"})).unwrap(), "Starting a session in maya on laptop: fix it.");
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
        assert_eq!(user_turn(&heard, "Maya").as_deref(), Some("what's waiting"));
        assert_eq!(user_turn(&[Effect::Say("Yes?".into())], "Maya").as_deref(), Some("Maya"));
        assert_eq!(user_turn(&[Effect::Say("Yes?".into())], "Jarvis").as_deref(), Some("Jarvis"), "a bare wake is the word she was called by");
        assert_eq!(user_turn(&[Effect::RunNow], "Maya").as_deref(), Some("yes"));
        assert_eq!(user_turn(&[Effect::Cancelled], "Maya").as_deref(), Some("no"));
        assert_eq!(user_turn(&[Effect::Correct("the other one".into())], "Maya").as_deref(), Some("the other one"), "a correction is remembered as the new command");
        assert_eq!(user_turn(&[Effect::Stop], "Maya").as_deref(), Some("stop"));
        assert_eq!(user_turn(&[], "Maya"), None);
    }

    #[test]
    fn only_failures_are_spoken_after_a_batch() {
        let sent = (json!({"kind":"reply"}), Ok("Sent.".to_string()));
        let changed = (json!({"kind":"answer"}), Err("The question has changed; ask me again.".to_string()));
        let gone = (json!({"kind":"focus"}), Err("Session is no longer running".to_string()));
        assert_eq!(failures_said(&[sent.clone()]), None, "the announcement already said it");
        assert_eq!(failures_said(&[sent, changed.clone()]).as_deref(), Some("The question has changed; ask me again."));
        assert_eq!(failures_said(&[changed, gone]).as_deref(), Some("The question has changed; ask me again. Session is no longer running."));
    }

    #[test]
    fn a_voice_batch_is_dropped_when_listening_stops_and_never_joined_by_the_next_run() {
        use maya_core::window::Window;
        let mut w = Window::default();
        let (old, _) = w.open("Telling coral: go.".into(), vec![json!({"kind":"reply"})], Via::Voice);
        w.start_clock(old, 0);
        assert_eq!(drop_voice_batch(&mut w).map(|b| b.id), Some(old));
        assert!(w.pending().is_none());
        // The next run's request opens a batch of its own, with a full window.
        let (new, _) = w.open("Telling hexgrid: ship it.".into(), vec![json!({"kind":"reply"})], Via::Voice);
        assert_ne!(new, old);
        assert_eq!(w.start_clock(new, 10_000), Some(10_000 + maya_core::window::WAIT_MS));
        assert_eq!(w.pending().unwrap().actions.len(), 1);
        // A batch from Chat, the phone or Maya herself does not hang on the microphone.
        for origin in [Via::Chat, Via::Phone, Via::Maya] {
            let mut w = Window::default();
            w.open("x".into(), vec![json!(1)], origin);
            assert!(drop_voice_batch(&mut w).is_none(), "{origin:?}");
            assert!(w.pending().is_some());
        }
        assert!(drop_voice_batch(&mut Window::default()).is_none());
    }

    #[test]
    fn a_resume_all_that_left_any_behind_is_a_failure_and_is_spoken() {
        assert_eq!(resume_all_outcome(5, 0), Ok("Resuming 5.".to_string()));
        assert_eq!(resume_all_outcome(4, 1), Err("Resumed 4; one couldn't be resumed.".to_string()));
        assert_eq!(resume_all_outcome(0, 2), Err("None of them could be resumed.".to_string()));
        let ran = |r| vec![(json!({"kind":"resume_all"}), r)];
        assert_eq!(failures_said(&ran(resume_all_outcome(5, 0))), None);
        assert_eq!(failures_said(&ran(resume_all_outcome(4, 1))).as_deref(), Some("Resumed 4; one couldn't be resumed."));
        assert_eq!(failures_said(&ran(resume_all_outcome(0, 2))).as_deref(), Some("None of them could be resumed."));
    }

    #[test]
    fn closing_the_window_clears_what_the_panel_shows() {
        let mut v = VoiceState { flow: Some(wake::Flow::new()), ..Default::default() };
        v.status.listening = true;
        v.flow.as_mut().unwrap().open_window(u64::MAX);
        show_window(&mut v, "Telling coral: go.", Some(5_000));
        assert_eq!((v.status.state.as_str(), v.status.pending.as_deref(), v.status.until), ("window", Some("Telling coral: go."), Some(5_000)));
        close_window(&mut v, "thinking");
        assert_eq!(v.flow.as_ref().unwrap().state(1), "idle", "a stop after the batch ran settles nothing");
        assert_eq!((v.status.state.as_str(), v.status.pending, v.status.until), ("thinking", None, None));
        // Not listening, the state stays as it is ("off").
        let mut off = VoiceState::default();
        show_window(&mut off, "Telling coral: go.", None);
        close_window(&mut off, "idle");
        assert_eq!((off.status.state.as_str(), off.status.pending), ("off", None));
    }

    #[test]
    fn each_remembered_turn_bumps_the_counter_even_once_the_log_is_full() {
        let chat = Mutex::new(ChatLog::in_memory());
        let voice = Mutex::new(VoiceState::default());
        for i in 0..505 {
            push_turn(&chat, &voice, "user", &format!("turn {i}"), i, Via::Voice);
        }
        assert_eq!(chat.lock().unwrap().last(1000).len(), 500, "the log keeps the last 500");
        assert_eq!(chat.lock().unwrap().last(1)[0].text, "turn 504");
        assert_eq!(voice.lock().unwrap().status.turns, 505, "the page refetches whenever this changes");
    }

    #[test]
    fn speech_queued_before_the_announcement_is_discarded_but_state_changes_are_kept() {
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
        assert!(pauses_ear_for("Telling hexgrid: go ahead."));
        assert!(pauses_ear_for("Nothing is waiting on you."));
    }

    #[test]
    fn a_reply_that_ends_in_a_question_mark_asks_for_a_follow_up() {
        assert!(asks_question("Which one: Hexgrid 1 or Hexgrid 2?"));
        assert!(asks_question("I couldn't find voice test. Did you mean voice-test? "));
        assert!(!asks_question("Nothing is waiting on you."));
        // An announcement never asks: she waits for a stop, not a yes.
        assert!(!asks_question("Telling hexgrid: go ahead."));
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
