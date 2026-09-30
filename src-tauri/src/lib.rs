pub mod answer;
pub mod antigravity;
pub mod attachments;
pub mod codex;
pub mod config;
pub mod dock;
pub mod ear;
pub mod foreign;
pub mod grok;
pub mod context;
pub mod events;
pub mod focus;
pub mod hook_install;
pub mod inbox;
pub mod interpreter;
pub mod launch;
pub mod listener;
pub mod log;
pub mod model;
pub mod models;
pub mod net;
pub mod notify;
pub mod pr;
pub mod registry;
pub mod resume;
pub mod reviews;
pub mod state;
pub mod store;
pub mod transcript;
pub mod voice;
pub mod wake;
pub mod watcher;

use base64::Engine;
use config::Config;
use listener::VoiceState;
use model::Card;
use net::merge;
use net::protocol::{Attachment, CommandKind};
use net::server::Notify;
use net::{NetChange, NetworkStatus};
use serde::Serialize;
use std::sync::Mutex;
use std::time::Duration;
use store::{now_ms, Store};
use tauri::{AppHandle, Emitter, Manager, State as TauriState};

/// How long the main waits for a command's result from an assistant.
const ROUTE_TIMEOUT: Duration = Duration::from_secs(30);

/// A remote session's machine, when this Maya is the main and the session
/// belongs to one of its connected assistants; else `None` for a local one.
fn remote_machine_of(state: &AppState, session_id: &str) -> Option<String> {
    merge::machine_of(&remote_boards(state), session_id)
}

/// Sends `kind` to `machine` and discards its (empty) result.
fn route_done(app: &AppHandle, machine: &str, kind: CommandKind) -> Result<(), String> {
    net::server::send_command(app, machine, kind, ROUTE_TIMEOUT).map(|_| ())
}

/// Sends `kind` to `machine` and deserialises its result into `T`.
fn route_data<T: for<'de> serde::Deserialize<'de>>(app: &AppHandle, machine: &str, kind: CommandKind) -> Result<T, String> {
    let value = net::server::send_command(app, machine, kind, ROUTE_TIMEOUT)?;
    let value = value.ok_or("The assistant sent no result.")?;
    serde_json::from_value(value).map_err(|e| e.to_string())
}

/// Reads and base64-encodes local files for a remote reply's attachments;
/// refuses a missing file, one over 20 MB, or files over 20 MB together
/// (the command must fit in one frame). `name` on each `Attachment` is the
/// path exactly as given, since the assistant matches on it.
fn remote_attachments(paths: &[String]) -> Result<Vec<Attachment>, String> {
    const MAX_BYTES: u64 = 20 * 1024 * 1024;
    let mut total = 0u64;
    for p in paths {
        let meta = std::fs::metadata(p).map_err(|_| format!("Attachment not found: {p}"))?;
        if meta.len() > MAX_BYTES {
            return Err("The file is too large (over 20 MB).".into());
        }
        total += meta.len();
    }
    if total > MAX_BYTES {
        return Err("Attachments total more than 20 MB.".into());
    }
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        let bytes = std::fs::read(p).map_err(|_| format!("Attachment not found: {p}"))?;
        out.push(Attachment { name: p.clone(), bytes: base64::engine::general_purpose::STANDARD.encode(bytes) });
    }
    Ok(out)
}

/// Builds the `Start` command a "+" dialog on a remote machine sends.
fn start_kind_for(dir: Option<String>, prompt: String, options: launch::LaunchOptions) -> CommandKind {
    CommandKind::Start { dir, prompt, options }
}

/// What Settings' Machine pickers show: each connected assistant's display
/// name, host and platform.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MachineInfo {
    pub name: String,
    pub hostname: String,
    pub platform: String,
    pub connected: bool,
}

#[tauri::command]
fn list_machines(state: TauriState<AppState>) -> Vec<MachineInfo> {
    network_status_of(&state).assistants.into_iter().map(|a| MachineInfo { name: a.name, hostname: a.hostname, platform: a.platform, connected: a.connected }).collect()
}

pub struct AppState {
    pub store: Mutex<Store>,
    pub notifier: Mutex<notify::Notifier>,
    pub reviews: Mutex<ReviewState>,
    pub voice: Mutex<VoiceState>,
    /// Lock order: `network`, then the server's own mutex, then `store`.
    pub network: Mutex<net::NetworkState>,
}

/// The server's line to the app: repaints the board, tells the page about
/// pairing and connections, and keeps the paired list in the config file.
pub(crate) struct TauriNetNotify {
    pub app: AppHandle,
}

impl TauriNetNotify {
    fn save_assistants(&self, f: impl FnOnce(&mut Vec<config::PairedAssistant>)) {
        let state = self.app.state::<AppState>();
        let mut store = state.store.lock().unwrap();
        f(&mut store.config.network.assistants);
        if let Err(e) = config::save(&store.config_path(), &store.config) {
            log::line("network", format!("could not save the paired assistants: {e}"));
        }
    }
}

impl Notify for TauriNetNotify {
    fn board_changed(&self) {
        refresh_and_emit(&self.app);
    }

    fn status_changed(&self, status: NetworkStatus) {
        let _ = self.app.emit("network", &status);
    }

    fn paired(&self, assistant: &config::PairedAssistant) {
        self.save_assistants(|list| {
            list.retain(|a| a.id != assistant.id);
            list.push(assistant.clone());
        });
    }

    fn paired_list_changed(&self, assistants: &[config::PairedAssistant]) {
        self.save_assistants(|list| *list = assistants.to_vec());
    }
}

/// The assistants' last snapshots when this Maya is the main; else none.
fn remote_boards(state: &AppState) -> Vec<merge::RemoteBoard> {
    let server = state.network.lock().unwrap().server.clone();
    server.map(|s| s.boards()).unwrap_or_default()
}

/// The last PR list fetched, and the error from the last attempt if it failed.
#[derive(Default, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewState {
    pub prs: Vec<reviews::ReviewPr>,
    pub error: Option<String>,
    pub fetched_at: Option<u64>,
}

fn poll_reviews(app: &AppHandle) {
    let result = reviews::fetch();
    let state = app.state::<AppState>();
    let snapshot = {
        let mut r = state.reviews.lock().unwrap();
        match result {
            Ok(prs) => {
                r.prs = prs;
                r.error = None;
                r.fetched_at = Some(now_ms());
            }
            Err(e) => {
                log::line("app", format!("pull requests: {e}"));
                r.error = Some(e);
            }
        }
        r.clone()
    };
    let _ = app.emit("reviews", &snapshot);
}

/// The lines kept for the Debug tab, oldest first.
#[tauri::command]
fn log_lines() -> Vec<log::Line> {
    log::lines()
}

/// Forgets the lines shown on the Debug tab; the file keeps everything.
#[tauri::command]
fn log_clear() {
    log::clear();
}

/// Where this launch's log file is.
#[tauri::command]
fn log_path(state: TauriState<AppState>) -> String {
    state.store.lock().unwrap().claude_dir().join("maya").join("maya.log").display().to_string()
}

#[tauri::command]
fn list_whisper_models(state: TauriState<AppState>) -> Vec<models::ModelInfo> {
    let claude = state.store.lock().unwrap().claude_dir().to_path_buf();
    models::list(&claude)
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ModelProgress {
    id: String,
    received: u64,
    total: u64,
}

/// Downloads one model, reporting progress as `voice-model` events.
#[tauri::command(async)]
fn download_whisper_model(app: AppHandle, state: TauriState<AppState>, id: String) -> Result<(), String> {
    if models::is_in_flight(&id) {
        return Err("That model is already downloading.".into());
    }
    let claude = state.store.lock().unwrap().claude_dir().to_path_buf();
    log::line("app", format!("downloading whisper model {id}"));
    let handle = app.clone();
    let name = id.clone();
    models::download(&claude, &id, &move |received, total| {
        let _ = handle.emit("voice-model", ModelProgress { id: name.clone(), received, total });
    })
    .map(|_| {
        log::line("app", format!("whisper model {id} ready"));
        let config = state.store.lock().unwrap().config.clone();
        if listener::should_start_after_download(&config, &id) {
            log::line("app", "model downloaded; starting the listener");
            state.voice.lock().unwrap().failures = 0;
            let handle = app.clone();
            std::thread::spawn(move || {
                let _ = listener::start_listening(&handle);
            });
        }
    })
    .map_err(|e| {
        log::line("app", format!("whisper model {id}: {e}"));
        e
    })
}

#[tauri::command]
fn remove_whisper_model(state: TauriState<AppState>, id: String) -> Result<(), String> {
    let (claude, in_use) = {
        let store = state.store.lock().unwrap();
        (store.claude_dir().to_path_buf(), store.config.recognizer == config::Recognizer::Builtin && store.config.whisper_model == id)
    };
    if in_use {
        return Err("That model is in use; pick another first.".into());
    }
    models::remove(&claude, &id)
}

/// The clones directory with `~` expanded, so the page can recognise review clones.
#[tauri::command]
fn clones_dir(state: TauriState<AppState>) -> String {
    state.store.lock().unwrap().config.clones_dir_path().to_string_lossy().into_owned()
}

#[tauri::command(async)]
fn list_review_prs(state: TauriState<AppState>) -> ReviewState {
    state.reviews.lock().unwrap().clone()
}

fn review_pr_for(state: &TauriState<AppState>, repo: &str, number: u64) -> Result<reviews::ReviewPr, String> {
    state.reviews.lock().unwrap().prs.iter().find(|p| p.repo == repo && p.number == number).cloned().ok_or("That pull request is no longer on the list.".into())
}

/// Opens a listed PR in the browser; the URL comes from the fetched list.
#[tauri::command(async)]
fn open_review_pr(state: TauriState<AppState>, repo: String, number: u64) -> Result<(), String> {
    let pr = review_pr_for(&state, &repo, number)?;
    let ok = std::process::Command::new("open").arg(&pr.url).status().map_err(|e| format!("could not open the browser: {e}"))?;
    if ok.success() {
        Ok(())
    } else {
        Err("The browser refused to open the pull request.".into())
    }
}

/// Opens a Terminal that reviews a listed PR with /should-i-approve, in the
/// project checkout when it is free, else in a clone under the clones dir.
#[tauri::command(async)]
fn review_pr(state: TauriState<AppState>, repo: String, number: u64) -> Result<String, String> {
    let pr = review_pr_for(&state, &repo, number)?;
    let (projects, clones, live) = {
        let store = state.store.lock().unwrap();
        let live: Vec<String> = store.live_cwds();
        (store.config.projects_dir_path(), store.config.clones_dir_path(), live)
    };
    let projects = projects.ok_or("Set a projects directory in Settings first.")?;
    let target = reviews::resolve_target(&projects, &clones, &pr.repo, pr.number, &live);
    launch::open_terminal_with(&reviews::shell_command(&target, &pr.repo, pr.number))?;
    Ok(target.dir.to_string_lossy().into_owned())
}

fn claude_dir() -> std::path::PathBuf {
    dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/")).join(".claude")
}

/// The ElevenLabs settings for one utterance, when the provider is chosen
/// and both a key and a voice exist; else None for the built-in voice.
fn eleven_settings(store: &Store) -> Option<(std::path::PathBuf, String, String)> {
    let cfg = &store.config;
    let key = voice::load_key();
    if !voice::use_elevenlabs(cfg.voice_provider, key.is_some(), cfg.elevenlabs_voice_id.as_deref()) {
        return None;
    }
    Some((store.claude_dir().join("maya"), key?, cfg.elevenlabs_voice_id.clone()?))
}

fn refresh_and_emit(app: &AppHandle) {
    let (cards, wants_notify, speak, eleven, assistant) = {
        let state = app.state::<AppState>();
        let mut store = state.store.lock().unwrap();
        let cards = store.refresh(now_ms());
        let eleven = if store.config.speak_notifications { eleven_settings(&store) } else { None };
        (cards, store.config.notify_on_awaiting, store.config.speak_notifications, eleven, store.config.network.role == config::NetworkRole::Assistant)
    };
    // Remote cards join after the store lock is released (lock order), so
    // the notifier below announces remote decisions too.
    let cards = merge::merged(cards, &remote_boards(&app.state::<AppState>()), now_ms());
    // Track every refresh so a toggle-on later does not replay old events.
    let (fresh, finished) = {
        let state = app.state::<AppState>();
        let mut n = state.notifier.lock().unwrap();
        (n.take_new(&cards), n.take_finished(&cards))
    };
    // A Focus mode (Do Not Disturb and friends) keeps Maya quiet; banners are
    // left to macOS, which filters them by the Focus's own rules.
    // An assistant stays quiet: its main notifies and speaks for it.
    let focus = !assistant && (!fresh.is_empty() || !finished.is_empty()) && notify::focus_active();
    if focus {
        log::line("app", "focus mode is on: announcements stay silent");
    }
    let speak = speak && !focus;
    if wants_notify && !assistant {
        for c in &fresh {
            // With a voice the banner stays silent; the sound is replaced, not doubled.
            notify::notify(c, !speak);
            if speak {
                if let Some(line) = notify::spoken_line(c) {
                    notify::speak(notify::Utterance::new(line, eleven.clone()));
                }
            }
        }
        if speak {
            for c in &finished {
                if let Some(line) = notify::spoken_line(c) {
                    notify::speak(notify::Utterance::new(line, eleven.clone()));
                }
            }
        }
    }
    let _ = app.emit("sessions", &cards);
    if assistant {
        net::client::push_board(app);
    }
}

/// Local and remote cards merged: what the board shows and what the voice
/// interpreter reasons about. Locks `store` only long enough to refresh it,
/// releasing it before `remote_boards` takes `network` (lock order: never
/// hold `store` while taking `network`).
pub(crate) fn merged_cards(state: &AppState) -> Vec<Card> {
    let cards = state.store.lock().unwrap().refresh(now_ms());
    merge::merged(cards, &remote_boards(state), now_ms())
}

#[tauri::command(async)]
fn list_sessions(state: TauriState<AppState>) -> Vec<Card> {
    merged_cards(&state)
}

/// What the Network section of Settings shows.
fn network_status_of(state: &AppState) -> NetworkStatus {
    let (server, stored) = {
        let n = state.network.lock().unwrap();
        (n.server.clone(), n.status.clone())
    };
    match server {
        Some(s) => s.status(),
        None => {
            let (role, paired) = {
                let store = state.store.lock().unwrap();
                (store.config.network.role, store.config.network.assistants.clone())
            };
            let main = role == config::NetworkRole::Main;
            // A main whose server is down still lists its assistants, disconnected.
            let assistants = if main { net::paired_offline(&paired) } else { vec![] };
            let main_error = if main { stored.main_error.clone() } else { None };
            NetworkStatus { role, code: None, assistants, main_error, ..stored }
        }
    }
}

#[tauri::command]
fn network_status(state: TauriState<AppState>) -> NetworkStatus {
    network_status_of(&state)
}

/// Opens pairing, or regenerates the code when it is already open.
#[tauri::command]
fn network_pairing_code(state: TauriState<AppState>) -> Result<NetworkStatus, String> {
    let (server, main_error) = {
        let n = state.network.lock().unwrap();
        (n.server.clone(), n.status.main_error.clone())
    };
    let Some(server) = server else {
        let main = state.store.lock().unwrap().config.network.role == config::NetworkRole::Main;
        return Err(match main_error {
            Some(e) if main => e,
            _ => "Turn on \"Act as main Maya\" first.".into(),
        });
    };
    server.open_pairing(now_ms());
    Ok(server.status())
}

/// Forgets a paired assistant; a connected one is told and closed.
#[tauri::command]
fn network_remove_assistant(app: AppHandle, state: TauriState<AppState>, id: String) -> NetworkStatus {
    // The handle is cloned out so the network lock is not held while the
    // server notifies (its adapter takes `network` again to repaint).
    let server = state.network.lock().unwrap().server.clone();
    match server {
        Some(s) => s.remove_assistant(&id),
        None => {
            TauriNetNotify { app: app.clone() }.save_assistants(|list| list.retain(|a| a.id != id));
            log::line("network", format!("{id}: removed"));
        }
    }
    network_status_of(&state)
}

/// Starts the main's server on the configured port, replacing any running
/// one; with `show_code` (the role just turned on) it opens pairing at once.
fn start_main(app: &AppHandle, show_code: bool) {
    stop_main(app);
    let port = app.state::<AppState>().store.lock().unwrap().config.listen_port();
    // A server just stopped lets go of the port within a tick; retry briefly.
    let mut started = net::server::start(app.clone(), port);
    for _ in 0..5 {
        if started.is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(150));
        started = net::server::start(app.clone(), port);
    }
    match started {
        Ok(handle) => {
            // Another start may have finished meanwhile; the displaced server stops.
            let displaced = {
                let state = app.state::<AppState>();
                let mut n = state.network.lock().unwrap();
                n.status.main_error = None;
                n.server.replace(handle)
            };
            if let Some(old) = displaced {
                old.stop();
            }
            if show_code {
                let server = app.state::<AppState>().network.lock().unwrap().server.clone();
                if let Some(server) = server {
                    server.open_pairing(now_ms());
                }
            }
        }
        Err(e) => {
            log::line("network", &e);
            // Shown under the role in Settings until a start succeeds.
            app.state::<AppState>().network.lock().unwrap().status.main_error = Some(e);
        }
    }
    let _ = app.emit("network", network_status_of(&app.state::<AppState>()));
}

/// Stops the main's server, if running, and drops the remote cards.
fn stop_main(app: &AppHandle) {
    let (server, had_error) = {
        let state = app.state::<AppState>();
        let mut n = state.network.lock().unwrap();
        (n.server.take(), n.status.main_error.take().is_some())
    };
    if let Some(s) = &server {
        s.stop();
    }
    if server.is_some() || had_error {
        let _ = app.emit("network", network_status_of(&app.state::<AppState>()));
    }
}

/// Starts the assistant's client with the stored link, replacing any running one.
fn start_assistant(app: &AppHandle) {
    let handle = net::client::start(app.clone());
    let displaced = {
        let state = app.state::<AppState>();
        let mut n = state.network.lock().unwrap();
        n.status.assistant = Default::default();
        n.client.replace(handle)
    };
    if let Some(old) = displaced {
        old.stop();
    }
    let _ = app.emit("network", network_status_of(&app.state::<AppState>()));
}

/// Stops the assistant's client, if running.
fn stop_assistant(app: &AppHandle) {
    let client = {
        let state = app.state::<AppState>();
        let mut n = state.network.lock().unwrap();
        let client = n.client.take();
        if client.is_some() {
            n.status.assistant = Default::default();
        }
        client
    };
    if let Some(c) = client {
        c.stop();
        let _ = app.emit("network", network_status_of(&app.state::<AppState>()));
    }
}

/// Pairs this Maya with a main as its assistant and starts the client.
#[tauri::command(async)]
fn network_pair(app: AppHandle, state: TauriState<AppState>, host: String, port: u16, name: String, code: String) -> Result<NetworkStatus, String> {
    let host = host.trim();
    if host.is_empty() || host.chars().any(|c| c.is_whitespace() || c == '/') {
        return Err("Type the main Maya's host name or address.".into());
    }
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return Err("The pairing code has six digits.".into());
    }
    let port = if port == 0 { net::protocol::DEFAULT_PORT } else { port };
    net::client::pair(&app, host, port, &name, &code)?;
    Ok(network_status_of(&state))
}

#[tauri::command(async)]
fn focus_session(pid: i32) -> Result<(), String> {
    focus::focus_pid(pid)
}

#[tauri::command(async)]
fn session_history(app: AppHandle, state: TauriState<AppState>, session_id: String) -> Result<Vec<transcript::Turn>, String> {
    if let Some(machine) = remote_machine_of(&state, &session_id) {
        return route_data(&app, &machine, CommandKind::History { session: session_id });
    }
    let (path, foreign) = {
        let store = state.store.lock().unwrap();
        if let Some(f) = store.foreign(&session_id) {
            (f.transcript_path.clone(), Some(f))
        } else {
            let s = store.session(&session_id).ok_or("Session is no longer running.")?;
            (store.transcript_path_for(&s), None)
        }
    };
    match foreign {
        Some(f) => Ok(foreign::turns_for(&f, 30)),
        None => Ok(transcript::read_turns(&path, 30)),
    }
}

#[tauri::command(async)]
fn send_reply(app: AppHandle, state: TauriState<AppState>, session_id: String, text: String, attachments: Vec<String>) -> Result<(), String> {
    if let Some(machine) = remote_machine_of(&state, &session_id) {
        let attachments = remote_attachments(&attachments)?;
        return route_done(&app, &machine, CommandKind::Reply { session: session_id, text, attachments });
    }
    // Other harnesses have no inbox: the reply is typed into their tty as one line.
    let foreign = state.store.lock().unwrap().foreign(&session_id);
    if let Some(f) = foreign {
        let card = state.store.lock().unwrap().card_for(&session_id, now_ms()).ok_or("Session is no longer running.")?;
        answer::check_free(&card)?;
        let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            return Err("Message is empty.".into());
        }
        let tty = match f.tty {
            Some(t) => t,
            None => focus::tty_for_pid(f.pid)?,
        };
        return answer::type_into_tty(&tty, &line);
    }
    let (socket, pid) = {
        let store = state.store.lock().unwrap();
        let s = store.session(&session_id).ok_or("Session is no longer running.")?;
        let socket = s.messaging_socket_path.clone().ok_or("This session has no inbox. Use the terminal.")?;
        (socket, s.pid)
    };
    inbox::send(std::path::Path::new(&socket), pid, &text)
}

#[tauri::command(async)]
fn answer_question(app: AppHandle, state: TauriState<AppState>, session_id: String, ask_id: u64, question_index: usize, option_index: usize) -> Result<(), String> {
    if let Some(machine) = remote_machine_of(&state, &session_id) {
        return route_done(&app, &machine, CommandKind::Answer { session: session_id, ask_id, question: question_index, option: option_index });
    }
    let card = {
        let mut store = state.store.lock().unwrap();
        store.card_for(&session_id, now_ms()).ok_or("Session is no longer running.")?
    };
    answer::check(&card, ask_id, question_index, option_index, now_ms())?;
    let count = card.awaiting.as_ref().map(|a| a.questions.len()).unwrap_or(0);
    let tty = focus::tty_for_pid(card.pid)?;
    answer::type_into_tty(&tty, &answer::keys_for_option(option_index))?;
    if answer::needs_submit(question_index, count) {
        std::thread::sleep(Duration::from_millis(answer::SUBMIT_DELAY_MS));
        answer::type_into_tty(&tty, "")?;
    }
    Ok(())
}

/// The pull request link to open for `card`: only a web address, since a
/// remote card's link comes from its assistant and `open` also takes file
/// paths and app schemes.
fn pr_link(card: &Card) -> Result<String, String> {
    let pr = card.pr.as_ref().ok_or("No pull request is known for this session yet.")?;
    if !attachments::is_web_url(&pr.url) {
        return Err("That pull request's link is not a web address.".into());
    }
    Ok(pr.url.trim().to_string())
}

/// Opens the session's pull request in the browser, on this Mac. The URL
/// comes from the PR cache (or, for a remote card, the assistant's card),
/// never from the page.
#[tauri::command(async)]
fn open_pr(state: TauriState<AppState>, session_id: String) -> Result<(), String> {
    let card = if remote_machine_of(&state, &session_id).is_some() {
        merged_cards(&state).into_iter().find(|c| c.session_id == session_id && c.machine.is_some()).ok_or("Session is no longer running.")?
    } else {
        let mut store = state.store.lock().unwrap();
        store.card_for(&session_id, now_ms()).ok_or("Session is no longer running.")?
    };
    let url = pr_link(&card)?;
    let ok = std::process::Command::new("open").arg(&url).status().map_err(|e| format!("could not open the browser: {e}"))?;
    if ok.success() {
        Ok(())
    } else {
        Err("The browser refused to open the pull request.".into())
    }
}

/// Looks up PRs for session directories whose result is missing or stale,
/// outside the store lock, and repaints when anything was learned.
fn poll_pull_requests(app: &AppHandle) {
    let due = {
        let state = app.state::<AppState>();
        let mut store = state.store.lock().unwrap();
        store.pr_dirs_due(now_ms())
    };
    if due.is_empty() {
        return;
    }
    for dir in due {
        let pr = pr::lookup(std::path::Path::new(&dir));
        let state = app.state::<AppState>();
        state.store.lock().unwrap().set_pr(&dir, pr, now_ms());
    }
    refresh_and_emit(app);
}

/// Saves a pasted file under the Maya directory and returns its path.
#[tauri::command(async)]
fn save_attachment(state: TauriState<AppState>, name: String, bytes: Vec<u8>) -> Result<String, String> {
    let maya_dir = state.store.lock().unwrap().claude_dir().join("maya");
    let path = attachments::save(&maya_dir, &name, &bytes, now_ms())?;
    Ok(path.to_string_lossy().into_owned())
}

/// Opens an http(s) link from a rendered message in the browser.
#[tauri::command(async)]
fn open_url(url: String) -> Result<(), String> {
    if !attachments::is_web_url(&url) {
        return Err("Only web links can be opened.".into());
    }
    let ok = std::process::Command::new("open").arg(url.trim()).status().map_err(|e| format!("could not open the browser: {e}"))?;
    if ok.success() {
        Ok(())
    } else {
        Err("The browser refused to open the link.".into())
    }
}

/// Types `/model x` or `/effort y` into the session's Terminal tab.
#[tauri::command(async)]
fn set_session_option(app: AppHandle, state: TauriState<AppState>, session_id: String, setting: String, value: String) -> Result<(), String> {
    if let Some(machine) = remote_machine_of(&state, &session_id) {
        return route_done(&app, &machine, CommandKind::SetOption { session: session_id, setting, value });
    }
    let text = answer::slash_command(&setting, &value)?;
    type_into_session(&state, &session_id, &text)
}

/// Types `/rename <name>` into the session's Terminal tab. The new name comes
/// back through the session registry on the next refresh.
#[tauri::command(async)]
fn rename_session(app: AppHandle, state: TauriState<AppState>, session_id: String, name: String) -> Result<(), String> {
    if let Some(machine) = remote_machine_of(&state, &session_id) {
        return route_done(&app, &machine, CommandKind::Rename { session: session_id, name });
    }
    let text = answer::rename_command(&name)?;
    type_into_session(&state, &session_id, &text)
}

/// Types `/compact` into the session's Terminal tab.
#[tauri::command(async)]
fn compact_session(app: AppHandle, state: TauriState<AppState>, session_id: String) -> Result<(), String> {
    if let Some(machine) = remote_machine_of(&state, &session_id) {
        return route_done(&app, &machine, CommandKind::Compact { session: session_id });
    }
    type_into_session(&state, &session_id, answer::COMPACT)
}

/// Sends Shift+Tab to the session's Terminal tab, cycling its permission mode.
#[tauri::command(async)]
fn cycle_session_mode(app: AppHandle, state: TauriState<AppState>, session_id: String) -> Result<(), String> {
    if let Some(machine) = remote_machine_of(&state, &session_id) {
        return route_done(&app, &machine, CommandKind::CycleMode { session: session_id });
    }
    type_into_session(&state, &session_id, answer::SHIFT_TAB)
}

fn type_into_session(state: &TauriState<AppState>, session_id: &str, text: &str) -> Result<(), String> {
    let card = {
        let mut store = state.store.lock().unwrap();
        store.card_for(session_id, now_ms()).ok_or("Session is no longer running.")?
    };
    if card.harness != model::Harness::ClaudeCode {
        return Err("That command is only available for Claude Code sessions.".into());
    }
    answer::check_free(&card)?;
    let tty = focus::tty_for_pid(card.pid)?;
    answer::type_into_tty(&tty, text)
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct StartResult {
    pub dir: String,
    pub how: String,
}

fn projects_root(state: &TauriState<AppState>) -> Result<std::path::PathBuf, String> {
    let root = state.store.lock().unwrap().config.projects_dir_path().ok_or("Set a projects directory in Settings first.")?;
    if !root.is_dir() {
        return Err(format!("Projects directory does not exist: {}.", root.display()));
    }
    Ok(root)
}

/// The project folder `dir` as an absolute path, refusing anything not listed.
fn project_path(state: &TauriState<AppState>, dir: &str) -> Result<std::path::PathBuf, String> {
    let root = projects_root(state)?;
    if !launch::list_project_dirs(&root).iter().any(|d| d == dir) {
        return Err("That folder is not in the projects directory.".into());
    }
    Ok(root.join(dir))
}

/// `None` or `""` means the machine argument was not given: this Mac.
fn is_local(machine: &Option<String>) -> bool {
    machine.as_deref().map_or(true, str::is_empty)
}

/// Past sessions of a project folder, newest first, with running ones marked.
#[tauri::command(async)]
fn list_resumable_sessions(app: AppHandle, state: TauriState<AppState>, dir: String, machine: Option<String>) -> Result<Vec<resume::ResumableSession>, String> {
    if !is_local(&machine) {
        let machine = machine.unwrap();
        return route_data(&app, &machine, CommandKind::ListResumable { dir });
    }
    let path = project_path(&state, &dir)?;
    let (claude_dir, running) = {
        let store = state.store.lock().unwrap();
        (store.claude_dir().to_path_buf(), store.live_session_ids())
    };
    Ok(resume::list_sessions(&claude_dir, &path.to_string_lossy(), &running))
}

/// Opens a Terminal in the folder running `claude --resume <id>`.
#[tauri::command(async)]
fn resume_session(app: AppHandle, state: TauriState<AppState>, dir: String, session_id: String, machine: Option<String>) -> Result<(), String> {
    if !is_local(&machine) {
        let machine = machine.unwrap();
        return route_done(&app, &machine, CommandKind::Resume { dir, session: session_id });
    }
    let path = project_path(&state, &dir)?;
    let (claude_dir, running) = {
        let store = state.store.lock().unwrap();
        (store.claude_dir().to_path_buf(), store.live_session_ids())
    };
    if running.contains(&session_id) {
        return Err("That session is already running.".into());
    }
    if !resume::transcript_exists(&claude_dir, &path.to_string_lossy(), &session_id) {
        return Err("No such session in that folder.".into());
    }
    launch::open_terminal_with(&resume::resume_command(&path, &session_id))
}

#[tauri::command(async)]
fn list_project_dirs(state: TauriState<AppState>, machine: Option<String>) -> Result<Vec<String>, String> {
    if !is_local(&machine) {
        let machine = machine.unwrap();
        return Ok(merge::dirs_of(&remote_boards(&state), &machine));
    }
    Ok(launch::list_project_dirs(&projects_root(&state)?))
}

#[tauri::command(async)]
fn start_session(app: AppHandle, state: TauriState<AppState>, dir: Option<String>, prompt: String, options: launch::LaunchOptions, machine: Option<String>) -> Result<StartResult, String> {
    if prompt.trim().is_empty() {
        return Err("Type a prompt first.".into());
    }
    options.validate()?;
    if !is_local(&machine) {
        let machine = machine.unwrap();
        return route_data(&app, &machine, start_kind_for(dir, prompt, options));
    }
    let root = projects_root(&state)?;
    let maya_dir = state.store.lock().unwrap().claude_dir().join("maya");
    let dirs = launch::list_project_dirs(&root);
    let picked = match dir {
        Some(_) => None,
        None => {
            let binary = launch::claude_binary().ok_or("Could not find the claude command.")?;
            launch::classify(&binary, &root, &prompt, &dirs, launch::CLASSIFIER_TIMEOUT)
        }
    };
    let (target, how) = launch::resolve_target(&root, &dirs, dir.as_deref(), picked.as_deref())?;
    let file = launch::write_prompt_file(&maya_dir, &prompt)?;
    launch::open_terminal(&target, &file, &options)?;
    Ok(StartResult { dir: target.to_string_lossy().into_owned(), how: how.to_string() })
}

#[tauri::command(async)]
fn hook_status(state: TauriState<AppState>) -> Result<bool, String> {
    let dir = state.store.lock().unwrap().claude_dir().to_path_buf();
    hook_install::status(&dir)
}

#[tauri::command(async)]
fn install_hook(state: TauriState<AppState>) -> Result<bool, String> {
    let dir = state.store.lock().unwrap().claude_dir().to_path_buf();
    hook_install::install_to(&dir)?;
    hook_install::status(&dir)
}

#[tauri::command(async)]
fn remove_hook(state: TauriState<AppState>) -> Result<bool, String> {
    let dir = state.store.lock().unwrap().claude_dir().to_path_buf();
    hook_install::remove_from(&dir)?;
    hook_install::status(&dir)
}

/// Stores the ElevenLabs key in the Keychain.
#[tauri::command(async)]
fn set_elevenlabs_key(key: String) -> Result<(), String> {
    voice::store_key(&key)
}

#[tauri::command(async)]
fn has_elevenlabs_key() -> bool {
    voice::load_key().is_some()
}

/// The account's voices, needing a stored key.
#[tauri::command(async)]
fn list_elevenlabs_voices() -> Result<Vec<voice::Voice>, String> {
    let key = voice::load_key().ok_or("Save an ElevenLabs API key first.")?;
    voice::list_voices(&key)
}

/// Speaks a sample line with the current voice settings.
#[tauri::command(async)]
fn try_voice(state: TauriState<AppState>) -> Result<(), String> {
    let eleven = {
        let store = state.store.lock().unwrap();
        eleven_settings(&store)
    };
    // Through the one speech queue, so listening pauses and she does not
    // wake herself on "Maya here".
    let line = "Maya here. hexgrid needs a decision".to_string();
    notify::speak_and_wait(notify::Utterance { fallback: false, ..notify::Utterance::new(line, eleven) })
}

#[tauri::command]
fn get_config(state: TauriState<AppState>) -> Config {
    state.store.lock().unwrap().config.clone()
}

#[tauri::command(async)]
fn set_config(app: AppHandle, state: TauriState<AppState>, config: Config) -> Result<Config, String> {
    if config.completed_timeout_minutes == 0 {
        return Err("Completed timeout must be at least 1 minute".into());
    }
    if let Some(dir) = config.projects_dir_path() {
        if !dir.is_dir() {
            return Err(format!("Projects directory does not exist: {}.", dir.display()));
        }
    }
    let mut config = config;
    // The main notifies and listens for an assistant.
    if config.network.role == config::NetworkRole::Assistant {
        config.listen = false;
    }
    let before = {
        let mut store = state.store.lock().unwrap();
        let before = store.config.clone();
        // The paired list belongs to the server and the assistant's id and
        // token to pairing; a page holding an older config must not undo them.
        config.network.assistants = before.network.assistants.clone();
        config.network.assistant_id = before.network.assistant_id.clone();
        config.network.token = before.network.token.clone();
        config::save(&store.config_path(), &config)?;
        store.config = config.clone();
        before
    };
    apply_config_change(&app, &before, &config);
    refresh_and_emit(&app);
    Ok(config)
}

/// Changes the stored config with `f`, saves it, and starts or stops what the change asks for.
pub(crate) fn update_config(app: &AppHandle, f: impl FnOnce(&mut Config)) -> Result<Config, String> {
    let (before, after) = {
        let state = app.state::<AppState>();
        let mut store = state.store.lock().unwrap();
        let before = store.config.clone();
        let mut after = before.clone();
        f(&mut after);
        config::save(&store.config_path(), &after)?;
        store.config = after.clone();
        (before, after)
    };
    apply_config_change(app, &before, &after);
    refresh_and_emit(app);
    Ok(after)
}

/// Starts, stops or restarts the listener, the server and the client after a config change.
fn apply_config_change(app: &AppHandle, before: &Config, config: &Config) {
    let state = app.state::<AppState>();
    match listener::listening_change(before, config) {
        listener::ListenChange::Restart => {
            log::line("listener", "settings changed; restarting");
            state.voice.lock().unwrap().failures = 0;
            let handle = app.clone();
            std::thread::spawn(move || {
                let _ = listener::start_listening(&handle);
            });
        }
        listener::ListenChange::Start => {
            state.voice.lock().unwrap().failures = 0;
            let handle = app.clone();
            std::thread::spawn(move || {
                let _ = listener::start_listening(&handle);
            });
        }
        listener::ListenChange::Stop => listener::stop_listening(app),
        listener::ListenChange::None => {}
    }
    for change in net::network_change(before, config) {
        match change {
            NetChange::StartMain | NetChange::RestartMain => {
                log::line("network", "settings changed; starting the main's server");
                // Turning the role on shows a pairing code without a click.
                start_main(app, change == NetChange::StartMain);
            }
            NetChange::StopMain => stop_main(app),
            NetChange::StartAssistant | NetChange::RestartAssistant => {
                log::line("network", "settings changed; starting the assistant's client");
                start_assistant(app);
            }
            NetChange::StopAssistant => stop_assistant(app),
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let dir = claude_dir();
    let mut store = Store::new(dir.clone());
    store.compact_events();

    tauri::Builder::default()
        .manage(AppState { store: Mutex::new(store), notifier: Mutex::new(notify::Notifier::default()), reviews: Mutex::new(ReviewState::default()), voice: Mutex::new(VoiceState::default()), network: Mutex::new(net::NetworkState::default()) })
        .invoke_handler(tauri::generate_handler![
            list_sessions,
            focus_session,
            session_history,
            send_reply,
            answer_question,
            set_session_option,
            cycle_session_mode,
            rename_session,
            compact_session,
            save_attachment,
            open_url,
            open_pr,
            list_review_prs,
            clones_dir,
            open_review_pr,
            review_pr,
            list_project_dirs,
            list_resumable_sessions,
            resume_session,
            start_session,
            hook_status,
            install_hook,
            remove_hook,
            get_config,
            set_config,
            set_elevenlabs_key,
            has_elevenlabs_key,
            list_elevenlabs_voices,
            try_voice,
            listener::voice_listen,
            listener::voice_confirm,
            listener::voice_status,
            listener::voice_history,
            listener::voice_selftest,
            log_lines,
            log_clear,
            log_path,
            list_whisper_models,
            download_whisper_model,
            remove_whisper_model,
            network_status,
            network_pairing_code,
            network_remove_assistant,
            network_pair,
            list_machines
        ])
        .setup(move |app| {
            let log_path = dir.join("maya").join("maya.log");
            if let Err(e) = log::init(&log_path) {
                eprintln!("{e}");
            }
            let log_handle = app.handle().clone();
            log::install_emitter(move |l| {
                let _ = log_handle.emit("log", l);
            });
            log::line("app", format!("Maya {} started; log at {}", env!("CARGO_PKG_VERSION"), log_path.display()));
            focus::install_app_handle(app.handle().clone());
            listener::install_speech_hook(app.handle().clone());
            dock::set_dock_icon();
            let handle = app.handle().clone();
            let sessions_dir = dir.join("sessions");
            let maya_dir = dir.join("maya");
            std::thread::spawn(move || {
                watcher::run(&sessions_dir, &maya_dir, Duration::from_secs(5), || refresh_and_emit(&handle));
            });
            let pr_handle = app.handle().clone();
            std::thread::spawn(move || loop {
                poll_pull_requests(&pr_handle);
                std::thread::sleep(Duration::from_secs(10));
            });
            let review_handle = app.handle().clone();
            std::thread::spawn(move || loop {
                poll_reviews(&review_handle);
                std::thread::sleep(Duration::from_secs(120));
            });
            let role = app.state::<AppState>().store.lock().unwrap().config.network.role;
            match role {
                config::NetworkRole::Main => {
                    let net_handle = app.handle().clone();
                    std::thread::spawn(move || start_main(&net_handle, false));
                }
                config::NetworkRole::Assistant => start_assistant(app.handle()),
                config::NetworkRole::Off => {}
            }
            // The main listens for an assistant.
            if app.state::<AppState>().store.lock().unwrap().config.listen && role != config::NetworkRole::Assistant {
                let voice_handle = app.handle().clone();
                std::thread::spawn(move || {
                    let _ = listener::start_listening(&voice_handle);
                });
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|_app, event| {
            // A stalled model download otherwise keeps its curl child alive
            // under launchd after Maya quits, writing a 60-190 MB `.part`
            // file to nowhere.
            if let tauri::RunEvent::Exit = event {
                models::abort_all();
            }
        });
}

#[cfg(test)]
mod route_tests {
    use super::*;

    #[test]
    fn remote_attachments_encodes_a_small_file() {
        let dir = std::env::temp_dir().join(format!("maya-route-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.txt");
        std::fs::write(&path, b"hello there").unwrap();
        let path_str = path.to_string_lossy().into_owned();

        let out = remote_attachments(&[path_str.clone()]).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, path_str, "the name carries the main's full local path exactly");
        let decoded = base64::engine::general_purpose::STANDARD.decode(&out[0].bytes).unwrap();
        assert_eq!(decoded, b"hello there");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn remote_attachments_refuses_a_missing_file() {
        let err = remote_attachments(&["/no/such/file-for-maya-tests.txt".to_string()]).unwrap_err();
        assert!(err.contains("Attachment not found"), "{err}");
    }

    #[test]
    fn remote_attachments_refuses_a_file_over_20_mb() {
        let dir = std::env::temp_dir().join(format!("maya-route-test-big-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.bin");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(20 * 1024 * 1024 + 1).unwrap();
        drop(file);
        let path_str = path.to_string_lossy().into_owned();

        let err = remote_attachments(&[path_str]).unwrap_err();
        assert_eq!(err, "The file is too large (over 20 MB).");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn remote_attachments_refuses_files_over_20_mb_together() {
        let dir = std::env::temp_dir().join(format!("maya-route-test-total-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths: Vec<String> = (0..2)
            .map(|i| {
                let path = dir.join(format!("part{i}.bin"));
                std::fs::File::create(&path).unwrap().set_len(11 * 1024 * 1024).unwrap();
                path.to_string_lossy().into_owned()
            })
            .collect();
        assert!(remote_attachments(&paths[..1]).is_ok(), "one alone fits");
        assert_eq!(remote_attachments(&paths).unwrap_err(), "Attachments total more than 20 MB.");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_pull_request_link_opens_only_when_it_is_a_web_address() {
        let card = |url: &str| Card {
            session_id: "r1".into(),
            pid: 1,
            name: "x".into(),
            cwd: "/x".into(),
            state: model::State::Idle,
            state_since: 0,
            snippet: "".into(),
            awaiting: None,
            has_inbox: true,
            harness: model::Harness::ClaudeCode,
            pr: Some(model::PullRequest { number: 7, url: url.into(), state: "open".into() }),
            context: None,
            machine: Some("laptop".into()),
            stale: false,
        };
        assert_eq!(pr_link(&card(" https://github.com/o/r/pull/7 ")), Ok("https://github.com/o/r/pull/7".to_string()));
        assert_eq!(pr_link(&card("file:///Applications/Calculator.app")).unwrap_err(), "That pull request's link is not a web address.");
        assert_eq!(pr_link(&card("/Applications/Calculator.app")).unwrap_err(), "That pull request's link is not a web address.");
        assert_eq!(pr_link(&Card { pr: None, ..card("") }).unwrap_err(), "No pull request is known for this session yet.");
    }

    #[test]
    fn start_kind_for_builds_a_start_command() {
        let options = launch::LaunchOptions::default();
        let kind = start_kind_for(Some("proj".into()), "do the thing".into(), options.clone());
        match kind {
            CommandKind::Start { dir, prompt, options: got } => {
                assert_eq!(dir.as_deref(), Some("proj"));
                assert_eq!(prompt, "do the thing");
                assert_eq!(got, options);
            }
            other => panic!("expected a Start command, got {other:?}"),
        }
    }
}
