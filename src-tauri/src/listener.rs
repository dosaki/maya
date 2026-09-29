//! Voice assistant: the listener thread, the wake-word conversation, spoken
//! replies with listening paused around them, and confirmed actions.
//!
//! Lock rule: never hold the store lock or the voice lock while speaking or
//! while the interpreter runs; take what is needed, release, then act.

use crate::store::now_ms;
use crate::{answer, config, ear, eleven_settings, focus, interpreter, launch, notify, wake};
use crate::{answer_question, list_resumable_sessions, resume_session, send_reply, start_session, type_into_session, AppState};
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
}

impl Default for VoiceStatus {
    fn default() -> Self {
        Self { listening: false, state: "off".into(), detail: String::new(), level: 0.0, heard: String::new(), said: String::new(), pending: None }
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

fn remember(app: &AppHandle, who: &str, text: &str) {
    let state = app.state::<AppState>();
    let mut v = state.voice.lock().unwrap();
    v.history.push(VoiceTurn { who: who.into(), text: text.into(), at: now_ms() });
    if v.history.len() > 40 {
        let extra = v.history.len() - 40;
        v.history.drain(..extra);
    }
}

/// Speaks a reply to the user now (blocking), with listening paused around it.
/// Blank text says nothing. False when run `generation` is no longer current,
/// before or after speaking.
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
        if let Some(e) = v.ear.as_mut() {
            e.pause();
        }
        if let Some(f) = v.flow.as_mut() {
            f.ignore_line(text);
        }
        v.status.said = text.to_string();
    }
    // History first, so a page that refetches it on this event sees the line.
    remember(app, "maya", text);
    emit_voice(app);
    let eleven = {
        let state = app.state::<AppState>();
        let store = state.store.lock().unwrap();
        eleven_settings(&store)
    };
    notify::speak_now(text, eleven);
    let state = app.state::<AppState>();
    let mut v = state.voice.lock().unwrap();
    if v.generation != generation {
        return false;
    }
    if let Some(e) = v.ear.as_mut() {
        e.resume();
    }
    true
}

/// Runs a validated action through the same paths as the page's buttons;
/// the Ok text is what Maya says when the interpreter gave no line of its own.
fn execute_action(app: &AppHandle, action: &serde_json::Value) -> Result<String, String> {
    let state = app.state::<AppState>();
    let kind = action["kind"].as_str().unwrap_or("");
    let session = action["session"].as_str().unwrap_or("").to_string();
    match kind {
        "report" => Ok(String::new()),
        "focus" => {
            let pid = state.store.lock().unwrap().card_for(&session, now_ms()).map(|c| c.pid).ok_or("Session is no longer running.")?;
            focus::focus_pid(pid)?;
            Ok("Done.".into())
        }
        "compact" => {
            type_into_session(&state, &session, answer::COMPACT)?;
            Ok("Compacting.".into())
        }
        "reply" => {
            let text = action["text"].as_str().unwrap_or("").to_string();
            send_reply(state.clone(), session, text)?;
            Ok("Sent.".into())
        }
        "answer" => {
            // The ask the user confirmed, captured at validation: a newer ask is refused.
            let ask_id = action["askId"].as_u64().ok_or("The question has changed; ask me again.")?;
            let n = action["option"].as_u64().unwrap_or(1) as usize;
            answer_question(state.clone(), session, ask_id, 0, n.saturating_sub(1))?;
            Ok("Answered.".into())
        }
        "resume" => {
            let dir = action["dir"].as_str().unwrap_or("").to_string();
            let sessions = list_resumable_sessions(state.clone(), dir.clone())?;
            let latest = sessions.into_iter().find(|s| !s.running).ok_or("Nothing to resume there.")?;
            resume_session(state.clone(), dir, latest.id)?;
            Ok("Resuming.".into())
        }
        "start" => {
            let dir = action["dir"].as_str().unwrap_or("").to_string();
            let prompt = action["prompt"].as_str().unwrap_or("").to_string();
            start_session(state.clone(), Some(dir), prompt, launch::LaunchOptions::default())?;
            Ok("Started.".into())
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
fn reply_then_idle(app: &AppHandle, generation: u64, text: &str) {
    if reply_aloud(app, generation, text) {
        set_voice(app, generation, |st| st.state = "idle".into());
    }
}

/// Sends a spoken command to the interpreter and acts on its reply. Every
/// step after the interpreter returns checks that run `generation` is still
/// current, so a stop while she thinks leaves nothing behind.
fn interpret(app: &AppHandle, generation: u64, cmd: &str) {
    let started = set_voice(app, generation, |st| {
        st.state = "thinking".into();
        st.pending = None;
    });
    if !started {
        return;
    }
    let (cards, dirs, model) = {
        let state = app.state::<AppState>();
        let mut store = state.store.lock().unwrap();
        let cards = store.refresh(now_ms());
        let dirs = store.config.projects_dir_path().map(|r| launch::list_project_dirs(&r)).unwrap_or_default();
        (cards, dirs, store.config.interpreter_model.clone())
    };
    let history = recent_exchanges(app);
    let Some(binary) = launch::claude_binary() else {
        reply_then_idle(app, generation, "I can't find the claude command.");
        return;
    };
    let reply = match interpreter::run(&binary, &model, cmd, &cards, &history) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("interpreter: {e}");
            reply_then_idle(app, generation, "Sorry, I didn't catch that.");
            return;
        }
    };
    if !is_current(app, generation) {
        return;
    }
    // No action, or only a report: the spoken reply is the whole answer.
    let Some(proposed) = reply.action.as_ref().filter(|a| a["kind"] != "report") else {
        reply_then_idle(app, generation, &reply.say);
        return;
    };
    match interpreter::validate(proposed, &cards, &dirs) {
        Err(why) => reply_then_idle(app, generation, &why),
        Ok(action) if interpreter::needs_confirm(&action) => {
            let say = if reply.say.trim().is_empty() { "Shall I?".to_string() } else { reply.say.clone() };
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
            let said = match execute_action(app, &action) {
                Ok(s) if reply.say.trim().is_empty() => s,
                Ok(_) => reply.say.clone(),
                Err(e) => e,
            };
            reply_then_idle(app, generation, &said);
        }
    }
}

/// Handles one final segment heard during listener run `generation` (or a
/// yes/no from the page). Nothing happens once that run is stopped or replaced.
fn on_heard(app: &AppHandle, generation: u64, text: &str) {
    let effects = {
        let state = app.state::<AppState>();
        let mut v = state.voice.lock().unwrap();
        if v.generation != generation {
            return;
        }
        v.status.heard = text.to_string();
        let now = now_ms();
        let effects = match v.flow.as_mut() {
            Some(f) => f.on_segment(text, now),
            None => vec![],
        };
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
    remember(app, "user", text);
    for e in effects {
        match e {
            wake::Effect::Say(s) => {
                if !set_voice(app, generation, |st| st.state = "awaiting-command".into()) {
                    return;
                }
                reply_aloud(app, generation, &s);
            }
            wake::Effect::Interpret(cmd) => interpret(app, generation, &cmd),
            wake::Effect::Execute(action) => {
                let go = set_voice(app, generation, |st| {
                    st.pending = None;
                    st.state = "thinking".into();
                });
                if !go {
                    return;
                }
                let said = match execute_action(app, &action) {
                    Ok(s) => s,
                    Err(e) => e,
                };
                reply_then_idle(app, generation, &said);
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

/// True while `generation` is still the current listener run.
fn is_current(app: &AppHandle, generation: u64) -> bool {
    app.state::<AppState>().voice.lock().unwrap().generation == generation
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
    let device = app.state::<AppState>().store.lock().unwrap().config.microphone.clone();
    let (ear, rx) = match ear::Ear::spawn(device.as_deref()) {
        Ok(pair) => pair,
        Err(e) => {
            set_voice(app, generation, |s| {
                s.listening = false;
                s.state = "error".into();
                s.detail = e.clone();
            });
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
        for ev in rx {
            if !is_current(&handle, generation) {
                break;
            }
            match ev {
                ear::EarEvent::Level(l) => {
                    handle.state::<AppState>().voice.lock().unwrap().status.level = l;
                    let _ = handle.emit("voice-level", l);
                }
                ear::EarEvent::Partial(t) => {
                    set_voice(&handle, generation, |s| s.heard = t);
                }
                ear::EarEvent::Final(t) => {
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
                    on_heard(&handle, generation, &t);
                }
                ear::EarEvent::State { state, detail } if state == "error" || state == "exited" => {
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
                    set_voice(&handle, generation, |s| {
                        s.state = "error".into();
                        s.detail = if detail.is_empty() { "the listener stopped".into() } else { detail.clone() };
                    });
                    if wants {
                        if let Some(delay) = ear::restart_delay_ms(failures - 1) {
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
                    set_voice(&handle, generation, |s| s.detail = format!("microphone: {name}"));
                }
                _ => {}
            }
        }
    });
    Ok(())
}

fn stop_listening(app: &AppHandle) {
    let dead = {
        let state = app.state::<AppState>();
        let mut v = state.voice.lock().unwrap();
        v.generation += 1;
        v.flow = None;
        v.status = VoiceStatus { state: "off".into(), ..Default::default() };
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

/// The page's yes/no buttons; ignored unless a confirmation is waiting.
#[tauri::command(async)]
pub(crate) fn voice_confirm(app: AppHandle, yes: bool) {
    let generation = {
        let state = app.state::<AppState>();
        let v = state.voice.lock().unwrap();
        match v.flow.as_ref() {
            Some(f) if f.has_pending() => v.generation,
            _ => return,
        }
    };
    on_heard(&app, generation, if yes { "yes" } else { "no" });
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
    let out = std::process::Command::new(path).arg("--selftest").output().map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
