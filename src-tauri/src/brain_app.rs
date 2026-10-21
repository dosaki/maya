//! Maya's own resident session, her brain: the driver and its event
//! thread, her card, and the glue from her session's rules
//! (`maya_core::brain::session`) to the listener, the action window, the
//! board, Chat and the phone. While her session is not running, every
//! request goes to the one-shot interpreter instead. The decisions with no
//! I/O in them live in `brain_route`.
//!
//! Lock rule (the listener's, extended): the brain lock is never held with
//! the store, voice, window, chat, reviews or network lock, nor while
//! speaking, while a model or an action runs, or while writing to the
//! driver. The driver sits behind its own lock, a leaf: it is taken alone,
//! only to write to the process or to shut it down. Each step takes what it
//! needs under one lock, copies it out and releases it before the next:
//! read the store, then the brain, then write to the driver, and post to
//! Chat or speak last.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::brain_route::{self, announcement, background_subagents, batch_notes, context_percent, failed_resume, follow_up, one_line, Outage, refusal_feedback, sort_actions, speaks, spoken, Feedback, PhoneReply, PhoneWaiters};
pub(crate) use crate::brain_route::{brain_card, handoff};
use crate::listener::{self, Interpreted};
use crate::store::now_ms;
use crate::{interpreter, launch, log, AppState};
use maya_core::brain::chat::Dot;
use maya_core::brain::driver::{self, Driver, Event, Launch};
use maya_core::brain::record::{self, Record, Start};
use maya_core::brain::session::{Out, Request, Session};
use maya_core::brain::watch::Watches;
use maya_core::brain::{notes, prompt};
use maya_core::model::{Card, Harness};
use maya_core::window::Via;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

const STOPPED: &str = "My session stopped; I'm using the quick interpreter until it's back.";
const RESUME_FAILED: &str = "I couldn't pick up my old session, so I've started fresh.";
/// The Chat turns a fresh session is primed with.
const PRIMER_TURNS: usize = 20;

type SharedDriver = Arc<Mutex<Box<dyn Driver>>>;

pub(crate) struct BrainState {
    /// Behind its own (leaf) lock: see the lock rule above.
    driver: Option<SharedDriver>,
    session: Session,
    record_path: PathBuf,
    watches: Watches,
    /// The board at the last refresh; None until the first refresh after a
    /// start, so a board not read yet is never compared with a real one.
    last_cards: Option<Vec<Card>>,
    /// Her session takes requests (from its spawn until it stops).
    ready: bool,
    /// Her session's id as she reported it (Ready); None until then.
    session_id: Option<String>,
    /// The id her session was started with, saved if she never reports one.
    assumed_id: Option<String>,
    /// Her session runs on this machine (started, and not an assistant).
    active: bool,
    /// Bumped on every start and shutdown: a replaced driver's events are ignored.
    generation: u64,
    agent: Harness,
    model: Option<String>,
    default_1m: bool,
    cwd: PathBuf,
    /// This run resumed a saved session; with `turn_seen`, tells a failed resume.
    resumed: bool,
    turn_seen: bool,
    /// When this run's driver was spawned, and when it was first sent a message.
    spawned_at: u64,
    first_sent_at: Option<u64>,
    /// Why a fresh session started, posted to Chat once her first turn ends.
    fresh_reason: Option<String>,
    /// The past conversation a fresh session reads first; its reply is not shown.
    primer: Option<String>,
    swallow_reply: bool,
    /// A watch fired while her session was down: wake her once it is back.
    wake_when_ready: bool,
    phones: PhoneWaiters,
    /// Replies in a row with refused actions.
    refusal_streak: u32,
    last_say: String,
    /// When her card last went working or idle.
    since: u64,
    /// Her session stopping on its own: the restart back-off and the one
    /// "my session stopped" line.
    outage: Outage,
}

impl BrainState {
    pub(crate) fn new(claude_dir: &Path) -> BrainState {
        let dir = brain_dir(claude_dir);
        BrainState {
            driver: None,
            session: Session::new(true),
            record_path: dir.join("record.json"),
            watches: Watches::load(dir.join("watches.json")),
            last_cards: None,
            ready: false,
            session_id: None,
            assumed_id: None,
            active: false,
            generation: 0,
            agent: Harness::ClaudeCode,
            model: None,
            default_1m: false,
            cwd: dir,
            resumed: false,
            turn_seen: false,
            spawned_at: 0,
            first_sent_at: None,
            fresh_reason: None,
            primer: None,
            swallow_reply: false,
            wake_when_ready: false,
            phones: PhoneWaiters::default(),
            refusal_streak: 0,
            last_say: String::new(),
            since: now_ms(),
            outage: Outage::default(),
        }
    }
}

/// Her working folder, which also holds her record, chat and watches.
fn brain_dir(claude_dir: &Path) -> PathBuf {
    claude_dir.join("maya").join("brain")
}

/// The brain's card, appended to the board the window sees. None until her
/// session exists.
pub(crate) fn card(state: &AppState) -> Option<Card> {
    let b = state.brain.lock().unwrap();
    let id = b.session_id.as_deref()?;
    Some(brain_card(id, b.agent, &b.cwd.to_string_lossy(), b.session.working(), b.session.job(), &b.last_say, b.since))
}

/// What the `brain` event and `brain_status` carry: the Chat tab's dot, the
/// batch waiting in the action window, what she is working on, and whether
/// her session takes requests.
#[derive(Clone, serde::Serialize)]
pub(crate) struct BrainStatus {
    dot: Dot,
    pending: Option<PendingView>,
    job: Option<String>,
    ready: bool,
}

#[derive(Clone, serde::Serialize)]
struct PendingView {
    say: String,
    /// When it runs; None until her announcement has been said.
    until: Option<u64>,
}

/// The status now. One lock at a time: window, then brain, then chat.
pub(crate) fn status(app: &AppHandle) -> BrainStatus {
    let state = app.state::<AppState>();
    let pending = state.window.lock().unwrap().pending().map(|b| PendingView { say: b.say.clone(), until: (b.until != u64::MAX).then_some(b.until) });
    let (working, job, ready) = {
        let b = state.brain.lock().unwrap();
        (b.session.working(), b.session.job().map(String::from), b.ready)
    };
    let dot = state.chat.lock().unwrap().dot(working);
    BrainStatus { dot, pending, job, ready }
}

/// Tells the page what she has waiting. Takes the window, brain and chat
/// locks in turn: the caller must hold none.
pub(crate) fn emit_status(app: &AppHandle) {
    let _ = app.emit("brain", status(app));
}

#[tauri::command]
pub(crate) fn brain_status(app: AppHandle) -> BrainStatus {
    status(&app)
}

/// The Chat tab's Stop button.
#[tauri::command(async)]
pub(crate) fn brain_stop(app: AppHandle) {
    stop(&app, Via::Chat);
}

/// True while her session runs and takes requests.
pub(crate) fn ready(app: &AppHandle) -> bool {
    app.state::<AppState>().brain.lock().unwrap().ready
}

/// Starts her session: on a main (or unnetworked) machine only, resuming the
/// saved one when `record::decide` says so.
pub(crate) fn start(app: &AppHandle) {
    start_run(app, None);
}

/// `start`; with `forced_fresh`, a fresh session whatever the record says,
/// posting that line once it is ready.
fn start_run(app: &AppHandle, forced_fresh: Option<&str>) {
    // A start or shutdown that lands while this one spawns wins: see below.
    let generation_at_start = app.state::<AppState>().brain.lock().unwrap().generation;
    let (runs, agent, model, claude_dir, projects) = {
        let state = app.state::<AppState>();
        let store = state.store.lock().unwrap();
        let c = &store.config;
        (brain_route::runs_brain(c), c.brain(), c.brain_model().map(String::from), store.claude_dir().to_path_buf(), c.projects_dir_path())
    };
    if !runs {
        log::line("brain", "this machine is an assistant: the main runs her session");
        return;
    }
    let binary = match launch::find_binary(agent) {
        Ok(b) => b,
        Err(e) => {
            log::line("brain", format!("her session cannot start: {e}"));
            return;
        }
    };
    let dir = brain_dir(&claude_dir);
    let record_path = dir.join("record.json");
    let start = match forced_fresh {
        Some(why) => Start::Fresh { reason: Some(why.to_string()) },
        None => record::decide(record::load(&record_path).as_ref(), agent, model.as_deref()),
    };
    let (resume, reason) = match start {
        Start::Resume(id) => (Some(id), None),
        Start::Fresh { reason } => (None, reason),
    };
    // A fresh session reads the recent conversation first.
    let primer = resume.is_none().then(|| prompt::primer(&app.state::<AppState>().chat.lock().unwrap().last(PRIMER_TURNS)));
    let background = background_subagents(agent);
    let mut writable: Vec<PathBuf> = projects.into_iter().collect();
    writable.extend(launch::scratch_dir("x").parent().map(Path::to_path_buf));
    let l = Launch { agent, binary, model: model.clone(), cwd: dir.clone(), system_prompt: prompt::system_prompt(background), resume: resume.clone(), writable };
    let default_1m = std::fs::read_to_string(claude_dir.join("settings.json")).map(|t| maya_core::context::default_is_1m(&t)).unwrap_or(false);
    // Her old session stays hidden until the next Ready names the new one.
    if let Some(id) = &resume {
        hide(app, id.clone());
    }
    let (tx, rx) = channel();
    let spawned = match driver::spawn(&l, tx) {
        Ok(d) => d,
        Err(e) => {
            log::line("brain", format!("her session cannot start: {e}"));
            return;
        }
    };
    // A fresh session is hidden by the id it was started with until her
    // Ready names it.
    if let Some(id) = spawned.assumed_session_id().filter(|_| resume.is_none()) {
        hide(app, id);
    }
    log::line("brain", match &resume {
        Some(id) => format!("resuming her session {id}"),
        None => "starting her session fresh".to_string(),
    });
    // Read apart from the brain lock (lock rule); a role change since bumps
    // the generation through `shutdown`, which the check below sees too.
    let still_runs = brain_route::runs_brain(&app.state::<AppState>().store.lock().unwrap().config);
    let installed = {
        let state = app.state::<AppState>();
        let mut b = state.brain.lock().unwrap();
        if !still_runs || b.generation != generation_at_start {
            drop(b);
            // Shut down with no lock held: it may wait for the process.
            return superseded(spawned);
        }
        let assumed = spawned.assumed_session_id();
        b.generation += 1;
        let old = b.driver.replace(Arc::new(Mutex::new(spawned)));
        // Notes the last run left ride along with her first message.
        let carried = b.session.take_notes();
        b.session = Session::new(background);
        for n in carried {
            b.session.note(n);
        }
        b.record_path = record_path;
        b.last_cards = None;
        // She takes requests from now: her agent may report her session
        // (Ready) only once the first message arrives. Whatever she has when
        // she dies before her first turn goes back to the quick interpreter.
        b.ready = true;
        b.session_id = None;
        b.assumed_id = assumed;
        b.active = true;
        b.agent = agent;
        b.model = model;
        b.default_1m = default_1m;
        b.cwd = dir;
        b.resumed = resume.is_some();
        b.turn_seen = false;
        b.spawned_at = now_ms();
        b.first_sent_at = None;
        b.fresh_reason = reason;
        b.primer = primer.clone();
        b.swallow_reply = false;
        b.since = now_ms();
        // A fresh session reads the past conversation first; a watch that
        // fired while she was down wakes her.
        let mut outs = Vec::new();
        if let Some(p) = primer {
            outs.extend(b.session.submit(Request::new(&p, Via::Maya)));
        }
        if std::mem::take(&mut b.wake_when_ready) {
            outs.extend(b.session.wake());
        }
        (b.generation, old, outs)
    };
    let (generation, old, outs) = installed;
    if let Some(old) = old {
        old.lock().unwrap().shutdown();
    }
    let handle = app.clone();
    std::thread::spawn(move || events(&handle, generation, rx));
    act(app, outs);
    emit_status(app);
}

/// A driver spawned for a start that another start or a shutdown overtook.
fn superseded(mut d: Box<dyn Driver>) {
    log::line("brain", "another start or a shutdown came first; this new session stops");
    d.shutdown();
}

/// The event thread of driver run `generation`; it ends with the driver.
fn events(app: &AppHandle, generation: u64, rx: Receiver<Event>) {
    for e in rx {
        let last = matches!(e, Event::Exited(_));
        if !on_event(app, generation, e) || last {
            break;
        }
    }
}

/// What an exit leaves to do once the brain lock is released.
struct Exit {
    drained: Vec<Request>,
    phones: Vec<Option<Sender<PhoneReply>>>,
    failed_resume: bool,
    say_stopped: bool,
    /// How long to wait before starting her again (`Outage`).
    restart_after: Duration,
    driver: Option<SharedDriver>,
}

/// Runs one driver event through her session and acts on what it decides.
/// False once run `generation` has been replaced.
fn on_event(app: &AppHandle, generation: u64, e: Event) -> bool {
    let mut outs = Vec::new();
    let mut hide_id = None;
    let mut line = None;
    let mut exit = None;
    {
        let state = app.state::<AppState>();
        let mut b = state.brain.lock().unwrap();
        if b.generation != generation {
            return false;
        }
        let was_working = b.session.working();
        match &e {
            // Her agent names her session: that id is hidden and, once a turn
            // has ended, saved.
            Event::Ready { session_id } => {
                b.session_id = Some(session_id.clone());
                hide_id = Some(session_id.clone());
            }
            Event::TurnEnded { context_tokens, .. } | Event::TurnFailed { context_tokens } => {
                // Her first turn proves the session: only now is it saved to
                // resume, and a fresh start explained.
                if !std::mem::replace(&mut b.turn_seen, true) {
                    match b.session_id.clone().or_else(|| b.assumed_id.clone()) {
                        Some(id) => {
                            let r = Record { agent: b.agent, model: b.model.clone(), session_id: id };
                            if let Err(err) = record::save(&b.record_path, &r) {
                                log::line("brain", format!("could not save her record: {err}"));
                            }
                        }
                        None => log::line("brain", "her session has no id to save"),
                    }
                    line = b.fresh_reason.take();
                }
                b.outage.turn_ended();
                let percent = context_percent(b.model.as_deref(), *context_tokens, b.default_1m);
                outs = b.session.on_event(&e, percent);
            }
            Event::Exited(why) => {
                log::line("brain", format!("her session ended: {why}"));
                b.session.on_event(&e, None);
                let now = now_ms();
                let failed = failed_resume(b.resumed, b.turn_seen, now.saturating_sub(b.spawned_at), b.first_sent_at.map(|t| now.saturating_sub(t)));
                let (restart_after, say_stopped) = if failed { (Duration::ZERO, false) } else { b.outage.exited() };
                let drained = handoff(&mut b.session);
                let phones = b.phones.drain();
                b.ready = false;
                b.session_id = None;
                b.primer = None;
                b.swallow_reply = false;
                exit = Some(Exit { drained, phones, failed_resume: failed, say_stopped, restart_after, driver: b.driver.take() });
            }
            _ => outs = b.session.on_event(&e, None),
        }
        if b.session.working() != was_working {
            b.since = now_ms();
        }
    }
    if let Some(id) = hide_id {
        hide(app, id);
    }
    if let Some(l) = line {
        post(app, &l, Via::Maya);
    }
    act(app, outs);
    if let Some(x) = exit {
        exited(app, generation, x);
    }
    crate::refresh_and_emit(app);
    emit_status(app);
    true
}

/// Her session ended on its own: the quick interpreter takes what she had,
/// and her session starts again (at once and fresh after a failed resume,
/// else after `Outage`'s back-off).
fn exited(app: &AppHandle, generation: u64, x: Exit) {
    drop(x.driver);
    if x.say_stopped {
        post(app, STOPPED, Via::Maya);
    }
    hand_to_fallback(app, x.drained, x.phones);
    let handle = app.clone();
    if x.failed_resume {
        log::line("brain", "the resume failed; starting fresh");
        std::thread::spawn(move || start_run(&handle, Some(RESUME_FAILED)));
        return;
    }
    let wait = x.restart_after;
    std::thread::spawn(move || {
        log::line("brain", format!("starting her session again in {} s", wait.as_secs()));
        std::thread::sleep(wait);
        let due = {
            let state = handle.state::<AppState>();
            let b = state.brain.lock().unwrap();
            b.generation == generation && b.driver.is_none() && b.active
        };
        if due {
            start_run(&handle, None);
        }
    });
}

/// Leaves her session out of the board and of what the store remembers. It
/// stays hidden after her process ends, until the next Ready names a new one.
fn hide(app: &AppHandle, id: String) {
    app.state::<AppState>().store.lock().unwrap().hide_session(Some(id));
}

/// Sends a request to her session; the reply comes back later, the way the
/// request came. While her session is not running the quick interpreter
/// answers it instead.
pub(crate) fn ask(app: &AppHandle, text: &str, via: Via) {
    submit(app, Request::new(text, via), None);
}

/// `ask` for a phone, waiting up to `timeout` for her reply: what she said
/// and her validated actions. None when she took too long or was stopped.
pub(crate) fn ask_and_wait(app: &AppHandle, text: &str, timeout: Duration) -> Option<PhoneReply> {
    let (tx, rx) = channel();
    let id = submit(app, Request::new(text, Via::Phone), Some(tx));
    match rx.recv_timeout(timeout) {
        Ok(reply) => Some(reply),
        Err(_) => {
            if let Some(id) = id {
                app.state::<AppState>().brain.lock().unwrap().phones.give_up(id);
            }
            None
        }
    }
}

/// Queues or delivers a request; the phone request's place in line, if any.
fn submit(app: &AppHandle, req: Request, waiter: Option<Sender<PhoneReply>>) -> Option<u64> {
    let via = req.via;
    // Spoken turns are remembered by the listener as they are heard; Maya's
    // own follow-ups are not the user's words.
    if !req.internal && matches!(via, Via::Chat | Via::Phone) {
        listener::remember(app, "user", &req.text, via);
        listener::emit_voice(app);
    }
    let mut busy = None;
    let (outs, id) = {
        let state = app.state::<AppState>();
        let mut b = state.brain.lock().unwrap();
        if !b.ready {
            drop(b);
            hand_to_fallback(app, vec![req], vec![waiter]);
            return None;
        }
        if !req.internal && via != Via::Maya {
            b.refusal_streak = 0;
        }
        // A follow-up on a phone's request answers in its turn too, to no one waiting.
        let id = (via == Via::Phone).then(|| b.phones.push(waiter));
        let outs = b.session.submit(req);
        // A busy line answers the phone now; her real reply goes to Chat.
        if let (Some(id), Some(Out::Busy { say, .. })) = (id, outs.iter().find(|o| matches!(o, Out::Busy { .. }))) {
            busy = b.phones.take(id).map(|tx| (tx, say.clone()));
        }
        (outs, id)
    };
    if let Some((tx, say)) = busy {
        let _ = tx.send((say, vec![]));
    }
    act(app, outs);
    emit_status(app);
    id
}

/// "Maya, stop": stops her running turn, her subagents and her queue.
pub(crate) fn stop(app: &AppHandle, via: Via) {
    let (outs, phones) = {
        let state = app.state::<AppState>();
        let mut b = state.brain.lock().unwrap();
        if !b.ready {
            (vec![Out::Said { via, say: "Nothing to stop.".into() }], vec![])
        } else {
            // A stopped primer turn ends with no reply to swallow.
            b.swallow_reply = false;
            let outs = b.session.stop(via);
            let phones = if outs.contains(&Out::Interrupt) { b.phones.drain() } else { vec![] };
            (outs, phones)
        }
    };
    for tx in phones.into_iter().flatten() {
        let _ = tx.send(("Stopped.".into(), vec![]));
    }
    act(app, outs);
    emit_status(app);
}

/// The board after a refresh, brain cards left out: what changed since the
/// last one becomes her notes, and a fired watch wakes her.
pub(crate) fn board_changed(app: &AppHandle, after: &[Card]) {
    let outs = {
        let state = app.state::<AppState>();
        let mut b = state.brain.lock().unwrap();
        if !b.active {
            b.last_cards = None;
            return;
        }
        let Some(before) = b.last_cards.replace(after.to_vec()) else { return };
        for n in notes::changes(&before, after) {
            b.session.note(n);
        }
        let fired = b.watches.fired(&before, after);
        if fired.is_empty() {
            return;
        }
        for f in fired {
            b.session.note(f);
        }
        if b.ready {
            b.session.wake()
        } else {
            b.wake_when_ready = true;
            vec![]
        }
    };
    act(app, outs);
}

/// A batch has run: each action's outcome becomes a note for her next message.
pub(crate) fn batch_done(app: &AppHandle, results: &[(Value, Result<String, String>)]) {
    let notes = batch_notes(results);
    let state = app.state::<AppState>();
    let mut b = state.brain.lock().unwrap();
    if b.active {
        for n in notes {
            b.session.note(n);
        }
    }
}

/// Her brain, its model or this machine's role changed: her session stops
/// and starts again (fresh or resumed, as `record::decide` says, or fresh
/// with `forced_fresh` posted once she is ready), or stays stopped on an
/// assistant.
pub(crate) fn restart(app: &AppHandle, forced_fresh: Option<&str>) {
    log::line("brain", "her settings changed; restarting her session");
    shutdown(app);
    start_run(app, forced_fresh);
}

/// Stops her session on purpose: no "stopped" line and no restart; the
/// quick interpreter takes what she had.
fn shutdown(app: &AppHandle) {
    let (driver, drained, phones) = {
        let state = app.state::<AppState>();
        let mut b = state.brain.lock().unwrap();
        b.generation += 1;
        b.active = false;
        b.ready = false;
        b.session_id = None;
        b.last_cards = None;
        b.primer = None;
        b.swallow_reply = false;
        b.wake_when_ready = false;
        (b.driver.take(), handoff(&mut b.session), b.phones.drain())
    };
    if let Some(d) = driver {
        d.lock().unwrap().shutdown();
    }
    hand_to_fallback(app, drained, phones);
    crate::refresh_and_emit(app);
    emit_status(app);
}

/// Maya is quitting: her process ends with it.
pub(crate) fn quit(app: &AppHandle) {
    let driver = {
        let state = app.state::<AppState>();
        let mut b = state.brain.lock().unwrap();
        b.generation += 1;
        b.ready = false;
        b.driver.take()
    };
    if let Some(d) = driver {
        d.lock().unwrap().shutdown();
    }
}

/// Acts on her session's decisions, in order. The caller holds no lock.
fn act(app: &AppHandle, outs: Vec<Out>) {
    for o in outs {
        match o {
            Out::Deliver(r) => deliver(app, r),
            Out::Reply { via, say, actions, errors } => reply(app, via, &say, &actions, errors),
            Out::Busy { via, say } | Out::Said { via, say } => say_line(app, via, &say),
            Out::Interrupt => {
                let _ = with_driver(app, "interrupt her", |d| d.interrupt());
                let dropped = app.state::<AppState>().window.lock().unwrap().cancel();
                if let Some(batch) = dropped {
                    log::line("action", format!("batch {} dropped by a stop", batch.id));
                    listener::settle(app, "idle");
                }
            }
            Out::Compact => {
                if !with_driver(app, "compact her", |d| d.compact()) {
                    // A driver that compacts itself (or could not be written to): the
                    // compaction turn ends here, so the queue does not wait on it.
                    let next = app.state::<AppState>().brain.lock().unwrap().session.compact_skipped();
                    act(app, next);
                }
            }
        }
    }
}

/// Writes to her process under the driver's own lock, taken alone. False
/// when it could not.
fn with_driver(app: &AppHandle, what: &str, f: impl FnOnce(&mut dyn Driver) -> Result<(), String>) -> bool {
    let driver = app.state::<AppState>().brain.lock().unwrap().driver.clone();
    let Some(driver) = driver else {
        log::line("brain", format!("could not {what}: her session is not running"));
        return false;
    };
    let mut d = driver.lock().unwrap();
    match f(d.as_mut()) {
        Ok(()) => true,
        Err(e) => {
            log::line("brain", format!("could not {what}: {e}"));
            false
        }
    }
}

/// Sends her the next turn: the request (none when a watch woke her), the
/// board as it is now, the pull requests and the notes since her last turn.
/// The primer of a fresh session goes as it is.
fn deliver(app: &AppHandle, r: Option<Request>) {
    let (primer, notes) = {
        let state = app.state::<AppState>();
        let mut b = state.brain.lock().unwrap();
        let primer = match &r {
            Some(req) if req.via == Via::Maya && !req.internal && b.primer.as_deref() == Some(req.text.as_str()) => {
                b.swallow_reply = true;
                b.primer.take()
            }
            _ => None,
        };
        let notes = if primer.is_some() { vec![] } else { b.session.take_notes() };
        b.first_sent_at.get_or_insert_with(now_ms);
        (primer, notes)
    };
    let text = match primer {
        Some(p) => p,
        None => {
            let state = app.state::<AppState>();
            let cards = crate::merged_cards(&state);
            let prs = state.reviews.lock().unwrap().prs.clone();
            // Maya's follow-ups are labelled as hers, not as the user's words.
            prompt::turn(r.as_ref().map(|r| (r.text.as_str(), if r.internal { Via::Maya } else { r.via })), &interpreter::board_summary(&cards, now_ms()), &interpreter::pr_summary(&prs), &notes)
        }
    };
    let _ = with_driver(app, "send her a message", |d| d.send(&text));
}

/// Her reply: said back the way the request came, her actions validated and
/// then announced, run or handed to the waiting phone, and refused ones
/// sent back to her.
fn reply(app: &AppHandle, via: Via, say: &str, actions: &[Value], errors: Vec<String>) {
    {
        let state = app.state::<AppState>();
        let mut b = state.brain.lock().unwrap();
        if via == Via::Maya && std::mem::take(&mut b.swallow_reply) {
            log::line("brain", "she has read the past conversation");
            return;
        }
        if let Some(s) = spoken(say) {
            b.last_say = s;
        }
    }
    let ctx = listener::voice_context(app);
    let sorted = sort_actions(actions, |a| {
        if a["kind"] == "resume_all" {
            interpreter::validate_resume_all(a, &ctx.remembered)
        } else {
            interpreter::validate(a, &ctx.cards, &ctx.dirs, &ctx.machines, &ctx.prs)
        }
    });
    let mut refused = errors;
    refused.extend(sorted.refused);
    let (feedback, waiter) = {
        let state = app.state::<AppState>();
        let mut b = state.brain.lock().unwrap();
        for w in &sorted.watch {
            match b.watches.add(w, &ctx.cards) {
                Ok(id) => b.session.note(format!("Watch {id} is set.")),
                Err(e) => refused.push(e),
            }
        }
        for id in &sorted.unwatch {
            if b.watches.remove(*id) {
                b.session.note(format!("Watch {id} is removed."));
            } else {
                refused.push(format!("There is no watch {id}"));
            }
        }
        let feedback = refusal_feedback(&refused, b.refusal_streak);
        b.refusal_streak = if refused.is_empty() { 0 } else { b.refusal_streak + 1 };
        let waiter = (via == Via::Phone).then(|| b.phones.pop());
        (feedback, waiter)
    };
    match waiter {
        // The phone announces, waits out the window and runs them itself.
        Some(waiter) => {
            let said = spoken(say);
            if let Some(s) = &said {
                post(app, s, Via::Phone);
            }
            let mut acts = sorted.announce;
            acts.extend(sorted.run_now);
            let sent = waiter.is_some_and(|tx| tx.send((said.unwrap_or_default(), acts.clone())).is_ok());
            if !sent && !acts.is_empty() {
                log::line("brain", "the phone stopped waiting; her actions for it are dropped");
                let note = format!("The phone stopped waiting, so these did not run: {}", announcement(&acts));
                app.state::<AppState>().brain.lock().unwrap().session.note(note);
            }
        }
        None => {
            // Her words, what the at-once actions did, then the announcement:
            // one utterance (or Chat line), so a short "Sure." is never said
            // alone, heard back and taken for the user's answer.
            let mut lines = vec![say.to_string()];
            if !sorted.run_now.is_empty() {
                let results: Vec<(Value, Result<String, String>)> = sorted.run_now.into_iter().map(|a| {
                    let r = listener::execute_action(app, &a);
                    (a, r)
                }).collect();
                for (a, r) in &results {
                    lines.push(match r {
                        Ok(ok) => listener::spoken_for(a).unwrap_or_else(|| ok.clone()),
                        Err(e) => e.clone(),
                    });
                }
                batch_done(app, &results);
            }
            let lead = one_line(&lines);
            if sorted.announce.is_empty() {
                say_line(app, via, &lead);
            } else {
                listener::announce_after(app, None, &lead, announcement(&sorted.announce), sorted.announce, via);
            }
        }
    }
    match feedback {
        Some(Feedback::Ask(m)) => {
            submit(app, follow_up(via, &m), None);
        }
        Some(Feedback::GiveUp(m)) => post(app, &m, Via::Maya),
        None => {}
    }
}

/// Her line, back the way the request came: spoken for a spoken request (or
/// one of her own when Maya is not muted), posted to Chat in every case.
/// Blank lines are neither said nor posted.
fn say_line(app: &AppHandle, via: Via, text: &str) {
    let line = spoken(text);
    if via == Via::Voice {
        // A blank reply still ends the "thinking" the request started.
        listener::answer_aloud(app, line.as_deref().unwrap_or(""));
        return;
    }
    let Some(line) = line else { return };
    let muted = app.state::<AppState>().store.lock().unwrap().config.muted;
    if speaks(via, muted) {
        listener::reply_aloud_as(app, listener::current_generation(app), &line, via);
    } else {
        post(app, &line, via);
    }
}

/// Posts her line to Chat without saying it.
fn post(app: &AppHandle, text: &str, via: Via) {
    if let Some(t) = spoken(text) {
        listener::remember(app, "maya", &t, via);
        listener::emit_voice(app);
    }
}

/// Hands requests her session will not answer to the quick interpreter, in
/// order, on a thread of their own; each phone request with its waiter.
fn hand_to_fallback(app: &AppHandle, drained: Vec<Request>, phones: Vec<Option<Sender<PhoneReply>>>) {
    if drained.is_empty() {
        return;
    }
    let mut phones = phones.into_iter();
    let jobs: Vec<(Request, Option<Sender<PhoneReply>>)> = drained
        .into_iter()
        .map(|r| {
            let waiter = if r.via == Via::Phone { phones.next().flatten() } else { None };
            (r, waiter)
        })
        .collect();
    let handle = app.clone();
    std::thread::spawn(move || {
        for (r, waiter) in jobs {
            fallback(&handle, r, waiter);
        }
    });
}

/// Answers one request with the one-shot interpreter, the way the listener,
/// Chat and the phone did before her session existed.
fn fallback(app: &AppHandle, r: Request, waiter: Option<Sender<PhoneReply>>) {
    if r.internal {
        log::line("brain", "her session is gone; Maya's follow-up to it is dropped");
        return;
    }
    log::line("brain", format!("the quick interpreter takes a {:?} request", r.via));
    match r.via {
        // Maya's own messages to her session mean nothing to the interpreter.
        Via::Maya => {}
        Via::Voice => {
            let (generation, listening) = {
                let state = app.state::<AppState>();
                let v = state.voice.lock().unwrap();
                (v.generation, v.status.listening)
            };
            if !listening {
                log::line("brain", "listening stopped; the spoken request is dropped");
                return;
            }
            let outcome = listener::interpret_text(app, &r.text, &listener::recent_exchanges(app));
            listener::act_on(app, generation, outcome, None);
        }
        Via::Chat => match listener::interpret_text(app, &r.text, &listener::recent_exchanges(app)) {
            Interpreted::Say(s) => post(app, &s, Via::Chat),
            Interpreted::Announce { say, actions } => listener::announce(app, None, say, actions, Via::Chat),
            Interpreted::Run(action) => {
                let line = match listener::execute_action(app, &action) {
                    Ok(ok) => listener::spoken_for(&action).unwrap_or(ok),
                    Err(e) => e,
                };
                post(app, &line, Via::Chat);
            }
        },
        Via::Phone => {
            let reply = match listener::interpret_text(app, &r.text, &[]) {
                Interpreted::Say(s) => (s, vec![]),
                Interpreted::Announce { actions, .. } => (String::new(), actions),
                Interpreted::Run(action) => (String::new(), vec![action]),
            };
            let unsent = match waiter {
                Some(tx) => tx.send(reply).err().map(|e| e.0),
                None => Some(reply),
            };
            // Nobody waits any more: what was said goes to Chat; actions are dropped.
            if let Some((say, _)) = unsent {
                post(app, &say, Via::Phone);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maya_core::brain::session::{Request, Session};
    use maya_core::model::{Harness, State};

    fn req(text: &str, via: Via) -> Request {
        Request::new(text, via)
    }

    #[test]
    fn the_brain_card_is_named_maya_flagged_and_never_awaiting_or_completed() {
        let c = brain_card("u", Harness::ClaudeCode, "/x/maya/brain", false, None, "Done.", 5);
        assert!(c.brain);
        assert_eq!(c.name, "Maya");
        assert_eq!(c.state, State::Idle);
        assert!(c.awaiting.is_none());
        assert_eq!(brain_card("u", Harness::ClaudeCode, "/x", true, Some("refactor"), "", 5).state, State::Working);
    }
    #[test]
    fn an_exited_brain_hands_its_queue_back() {
        let mut s = Session::new(true);
        s.submit(req("a", Via::Chat)); s.submit(req("b", Via::Voice));
        let back = handoff(&mut s); // brain_app helper: drain() on Exited
        assert_eq!(back.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), vec!["a", "b"]);
    }
}
