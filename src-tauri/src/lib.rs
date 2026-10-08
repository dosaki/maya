pub use maya_core::{actions, answer, antigravity, attachments, codex, config, context, events, foreign, grok, hook_install, inbox, interpreter, kiro, launch, log, model, net, notify, opencode, pr, registry, resume, reviews, state, store, terminal, transcript, tty, watcher};

pub mod badge;
#[cfg(target_os = "macos")]
pub mod dock;
pub mod ear;
#[cfg(target_os = "macos")]
pub mod focus;
pub mod listener;
pub mod models;
pub mod net_app;
#[cfg(target_os = "macos")]
pub mod terminal_app;
#[cfg(target_os = "linux")]
pub mod terminal_linux;
#[cfg(windows)]
pub mod terminal_win;
pub mod voice;
pub mod wake;

/// This platform's terminal: `TERMINAL`, `open_terminal_with` and `focus_pid`.
#[cfg(target_os = "macos")]
pub(crate) use terminal_app as term;
#[cfg(windows)]
pub(crate) use terminal_win as term;
#[cfg(target_os = "linux")]
pub(crate) use terminal_linux as term;

/// This platform's one terminal, as the core's seam.
#[cfg(not(target_os = "linux"))]
fn terminal() -> &'static dyn terminal::Terminal {
    &term::TERMINAL
}
#[cfg(target_os = "linux")]
fn terminal() -> &'static dyn terminal::Terminal {
    &*term::TERMINAL
}

/// Opens an http(s) URL in the default browser.
#[cfg(target_os = "macos")]
fn open_in_browser(url: &str) -> Result<(), String> {
    let ok = std::process::Command::new("open").arg(url).status().map_err(|e| format!("could not open the browser: {e}"))?;
    if ok.success() {
        Ok(())
    } else {
        Err("The browser refused to open the link.".into())
    }
}

/// Opens an http(s) URL in the default browser. The URL is one argument to
/// `xdg-open`; no shell reads it. A browser started here outlives Maya, so
/// it starts without the AppImage's environment.
#[cfg(target_os = "linux")]
fn open_in_browser(url: &str) -> Result<(), String> {
    let mut cmd = std::process::Command::new("xdg-open");
    maya_core::launch::scrub_appimage_env(&mut cmd);
    let ok = cmd.arg(url).stdin(std::process::Stdio::null()).status().map_err(|e| format!("could not open the browser: {e}"))?;
    if ok.success() {
        Ok(())
    } else {
        Err("The browser refused to open the link.".into())
    }
}

/// Opens an http(s) URL in the default browser. ShellExecute takes the URL
/// as one argument; no shell reads it, so `&` and the like stay literal.
#[cfg(windows)]
fn open_in_browser(url: &str) -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (verb, file) = (wide("open"), wide(url));
    // SAFETY: NUL-terminated strings that outlive the call.
    let r = unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
    // Values above 32 mean success.
    if r as isize > 32 {
        Ok(())
    } else {
        Err("The browser refused to open the link.".into())
    }
}

use config::Config;
use listener::VoiceState;
use maya_core::agents;
use maya_core::net::routing::{agents_reply, check_remote_agent, machines_of, remote_attachments, AgentsReply, MachineInfo, ROUTE_TIMEOUT};
use model::{Card, Harness};
use net::merge;
use net::protocol::CommandKind;
use net::server::Notify;
use net::{NetChange, NetworkStatus};
use serde::Serialize;
use std::sync::Mutex;
use std::time::Duration;
use store::{now_ms, Store};
use tauri::{AppHandle, Emitter, Manager, State as TauriState};

/// A remote session's machine, when this Maya is the main and the session
/// belongs to one of its connected assistants; else `None` for a local one.
/// A live local session with the id wins, as it does on the board.
fn remote_machine_of(state: &AppState, session_id: &str) -> Option<String> {
    let local_ids = state.store.lock().unwrap().live_session_ids();
    merge::route(&local_ids, &remote_boards(state), session_id, now_ms())
}

/// Sends `kind` to `machine` and discards its (empty) result.
fn route_done(app: &AppHandle, machine: &str, kind: CommandKind) -> Result<(), String> {
    net_app::send_command(app, machine, kind, ROUTE_TIMEOUT).map(|_| ())
}

/// Sends `kind` to `machine` and deserialises its result into `T`.
fn route_data<T: for<'de> serde::Deserialize<'de>>(app: &AppHandle, machine: &str, kind: CommandKind) -> Result<T, String> {
    let value = net_app::send_command(app, machine, kind, ROUTE_TIMEOUT)?;
    let value = value.ok_or("The assistant sent no result.")?;
    serde_json::from_value(value).map_err(|e| e.to_string())
}

/// Builds the `Start` command a "+" dialog on a remote machine sends.
fn start_kind_for(dir: Option<String>, prompt: String, options: launch::LaunchOptions) -> CommandKind {
    CommandKind::Start { dir, prompt, options }
}

#[tauri::command]
fn list_machines(state: TauriState<AppState>) -> Vec<MachineInfo> {
    machines_of(&network_status_of(&state))
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
    fn save_assistants(&self, f: impl FnOnce(&mut Vec<config::PairedAssistant>)) -> Result<(), String> {
        let state = self.app.state::<AppState>();
        let mut store = state.store.lock().unwrap();
        let path = store.config_path();
        save_paired(&path, &mut store.config, f)
    }
}

/// Changes the paired list with `f` and saves the config to `path`; the
/// config in memory changes only once the save succeeded.
fn save_paired(path: &std::path::Path, config: &mut Config, f: impl FnOnce(&mut Vec<config::PairedAssistant>)) -> Result<(), String> {
    let mut next = config.clone();
    f(&mut next.network.assistants);
    if let Err(e) = config::save(path, &next) {
        log::line("network", format!("could not save the paired assistants: {e}"));
        return Err(format!("Could not save the paired assistants: {e}"));
    }
    *config = next;
    Ok(())
}

impl Notify for TauriNetNotify {
    fn board_changed(&self) {
        refresh_and_emit(&self.app);
    }

    fn board_seeded(&self, cards: &[Card]) {
        self.app.state::<AppState>().notifier.lock().unwrap().seed(cards);
    }

    fn status_changed(&self, status: NetworkStatus) {
        let _ = self.app.emit("network", &status);
    }

    fn paired(&self, assistant: &config::PairedAssistant) -> Result<(), String> {
        // A machine pairing again keeps its entry, and its place in the list.
        self.save_assistants(|list| match list.iter_mut().find(|a| a.id == assistant.id) {
            Some(a) => *a = assistant.clone(),
            None => list.push(assistant.clone()),
        })
    }

    fn paired_list_changed(&self, assistants: &[config::PairedAssistant]) -> Result<(), String> {
        self.save_assistants(|list| *list = assistants.to_vec())
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
    open_in_browser(&pr.url)
}

/// Opens a terminal that reviews a listed PR with Maya's agent and the
/// review prompt from Settings, in the project checkout when it is free,
/// else in a clone under the clones dir.
#[tauri::command(async)]
fn review_pr(state: TauriState<AppState>, repo: String, number: u64) -> Result<String, String> {
    let pr = review_pr_for(&state, &repo, number)?;
    actions::start_review(&local(&state), &pr)
}

fn claude_dir() -> std::path::PathBuf {
    dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/")).join(".claude")
}

/// Maya's folder and the ElevenLabs voice id when the config asks for that
/// voice; None for the built-in one. Read under the store lock; the key is
/// looked up only when this says yes, and after the lock is released.
pub(crate) fn eleven_wanted(cfg: &Config, claude_dir: &std::path::Path) -> Option<(std::path::PathBuf, String)> {
    // A key is assumed here: looking it up is what this check spares.
    if !voice::use_elevenlabs(cfg.voice_provider, true, cfg.elevenlabs_voice_id.as_deref()) {
        return None;
    }
    Some((claude_dir.join("maya"), cfg.elevenlabs_voice_id.clone()?))
}

/// The ElevenLabs settings for one utterance, when `eleven_wanted` said yes
/// and a key is stored; else None for the built-in voice. Never call it
/// holding the store lock: the key lookup can block (on Linux a locked
/// GNOME keyring asks to be unlocked).
pub(crate) fn eleven_settings(wanted: Option<(std::path::PathBuf, String)>) -> Option<(std::path::PathBuf, String, String)> {
    let (maya_dir, voice_id) = wanted?;
    Some((maya_dir, voice::load_key()?, voice_id))
}

/// The local cards as the board shows them: on Linux each carries the name
/// of the tmux session it runs in, so the page knows which ones a terminal
/// window can attach to.
pub(crate) fn local_cards(store: &mut store::Store) -> Vec<Card> {
    #[allow(unused_mut)]
    let mut cards = store.refresh(now_ms());
    #[cfg(target_os = "linux")]
    fill_terminal_names(store, &mut cards);
    cards
}

/// Forgets the tty of every pid no longer on the board: a pid that is gone
/// may come back as another process.
#[cfg(any(test, target_os = "linux"))]
fn prune_ttys(cache: &mut std::collections::HashMap<i32, Option<String>>, live_pids: &std::collections::HashSet<i32>) {
    cache.retain(|pid, _| live_pids.contains(pid));
}

/// Sets each card's `terminal` to its tmux session's name, as the CLI's
/// board does: the tty from the registry (or the foreign session), else
/// from the pid, cached per pid (a pid with no terminal too, so it is not
/// asked again each refresh); then one `tmux list-panes` for them all.
#[cfg(target_os = "linux")]
fn fill_terminal_names(store: &store::Store, cards: &mut [Card]) {
    static TTYS: Mutex<Option<std::collections::HashMap<i32, Option<String>>>> = Mutex::new(None);
    let mut cache = TTYS.lock().unwrap();
    let cache = cache.get_or_insert_with(Default::default);
    prune_ttys(cache, &cards.iter().map(|c| c.pid).collect());
    let mut ttys = Vec::with_capacity(cards.len());
    for c in cards.iter() {
        let known = store.session(&c.session_id).and_then(|s| s.tty).or_else(|| store.foreign(&c.session_id).and_then(|f| f.tty));
        let tty = match known {
            Some(t) => Some(t),
            None => cache.entry(c.pid).or_insert_with(|| tty::tty_for_pid(c.pid).ok()).clone(),
        };
        ttys.push(tty);
    }
    let wanted: Vec<String> = ttys.iter().flatten().cloned().collect();
    let names = terminal().names_for_ttys(&wanted);
    for (c, tty) in cards.iter_mut().zip(ttys) {
        c.terminal = tty.and_then(|t| names.get(&t).cloned());
    }
}

/// How an announcement is made: whether it is spoken, and whether its
/// banner plays a sound. A Focus mode or mute keeps Maya from speaking.
/// The voice replaces the banner's sound rather than doubling it; under a
/// Focus mode the sound is left to macOS, which filters banners by the
/// Focus's own rules; muted, there is none.
fn announcement(speak_notifications: bool, muted: bool, focus: bool) -> (bool, bool) {
    let spoken = speak_notifications && !muted && !focus;
    (spoken, !spoken && !muted)
}

fn refresh_and_emit(app: &AppHandle) {
    let (cards, wants_notify, speak, muted, eleven_voice, assistant) = {
        let state = app.state::<AppState>();
        let mut store = state.store.lock().unwrap();
        let cards = local_cards(&mut store);
        let c = &store.config;
        let eleven = if c.speak_notifications && !c.muted { eleven_wanted(c, store.claude_dir()) } else { None };
        (cards, c.notify_on_awaiting, c.speak_notifications, c.muted, eleven, c.network.role == config::NetworkRole::Assistant)
    };
    // Due names are typed into the terminals with no lock held.
    actions::run_due_renames(&local(&app.state::<AppState>()));
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
    let (speak, banner_sound) = announcement(speak, muted, focus);
    // The app icon shows how many sessions wait, and asks for a look (a
    // Dock bounce) under the same rules as the banner.
    badge::show(app, badge::awaiting_count(&cards));
    if wants_notify && !assistant && !focus && !fresh.is_empty() {
        badge::bounce(app);
    }
    if wants_notify && !assistant {
        // The key is looked up only when there is something to say.
        let eleven = if speak && (!fresh.is_empty() || !finished.is_empty()) { eleven_settings(eleven_voice) } else { None };
        for c in &fresh {
            notify::notify(c, banner_sound);
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
        net_app::push_board(app);
    }
}

/// Local and remote cards merged: what the board shows and what the voice
/// interpreter reasons about. Locks `store` only long enough to refresh it,
/// releasing it before `remote_boards` takes `network` (lock order: never
/// hold `store` while taking `network`).
pub(crate) fn merged_cards(state: &AppState) -> Vec<Card> {
    let cards = local_cards(&mut state.store.lock().unwrap());
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

/// Forgets a paired assistant; a connected one is told and closed. The
/// shorter list is saved first: if that fails, nothing changes and the
/// page gets the error.
#[tauri::command]
fn network_remove_assistant(app: AppHandle, state: TauriState<AppState>, id: String) -> Result<NetworkStatus, String> {
    // The handle is cloned out so the network lock is not held while the
    // server notifies (its adapter takes `network` again to repaint).
    let server = state.network.lock().unwrap().server.clone();
    match server {
        Some(s) => s.remove_assistant(&id)?,
        None => {
            TauriNetNotify { app: app.clone() }.save_assistants(|list| list.retain(|a| a.id != id))?;
            log::line("network", format!("{id}: removed"));
        }
    }
    Ok(network_status_of(&state))
}

/// Starts the main's server on the configured port, replacing any running
/// one; with `show_code` (the role just turned on) it opens pairing at once.
fn start_main(app: &AppHandle, show_code: bool) {
    stop_main(app);
    let port = app.state::<AppState>().store.lock().unwrap().config.listen_port();
    // A server just stopped lets go of the port within a tick; retry briefly.
    let mut started = net_app::start_server(app.clone(), port);
    for _ in 0..5 {
        if started.is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(150));
        started = net_app::start_server(app.clone(), port);
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
    let handle = net_app::start(app.clone());
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
    // "Pair again" while connected: the running client goes first, and its
    // connection with it, so the main reuses this machine's entry instead of
    // pairing a second machine at the same address. No lock is held while
    // it winds down (its notifier takes `network`).
    let running = state.network.lock().unwrap().client.take();
    let had_client = running.is_some();
    if let Err(e) = net::client::pair_after_stopping(running, || net_app::pair(&app, host, port, &name, &code)) {
        // The old pairing still stands: its client comes back.
        let assistant = state.store.lock().unwrap().config.network.role == config::NetworkRole::Assistant;
        if had_client && assistant {
            start_assistant(&app);
        }
        return Err(e);
    }
    Ok(network_status_of(&state))
}

/// This Maya's sessions over this platform's terminal, for the core's local actions.
pub(crate) fn local(state: &AppState) -> actions::Local<'_> {
    actions::Local { store: &state.store, terminal: terminal() }
}

/// Brings forward the terminal of the local session running as `pid`.
#[tauri::command(async)]
fn focus_session(state: TauriState<AppState>, pid: i32) -> Result<(), String> {
    let session_id = state.store.lock().unwrap().refresh(now_ms()).into_iter().find(|c| c.pid == pid && c.machine.is_none()).map(|c| c.session_id);
    match session_id {
        Some(id) => actions::focus_session(&local(&state), &id),
        None => term::focus_pid(pid),
    }
}

#[tauri::command(async)]
fn session_history(app: AppHandle, state: TauriState<AppState>, session_id: String) -> Result<Vec<transcript::Turn>, String> {
    if let Some(machine) = remote_machine_of(&state, &session_id) {
        return route_data(&app, &machine, CommandKind::History { session: session_id });
    }
    actions::session_history(&local(&state), &session_id)
}

#[tauri::command(async)]
fn send_reply(app: AppHandle, state: TauriState<AppState>, session_id: String, text: String, attachments: Vec<String>) -> Result<(), String> {
    if let Some(machine) = remote_machine_of(&state, &session_id) {
        let attachments = remote_attachments(&attachments)?;
        return route_done(&app, &machine, CommandKind::Reply { session: session_id, text, attachments });
    }
    actions::send_reply(&local(&state), &session_id, &text)
}

#[tauri::command(async)]
fn answer_question(app: AppHandle, state: TauriState<AppState>, session_id: String, ask_id: u64, question_index: usize, option_index: usize) -> Result<(), String> {
    if let Some(machine) = remote_machine_of(&state, &session_id) {
        return route_done(&app, &machine, CommandKind::Answer { session: session_id, ask_id, question: question_index, option: option_index });
    }
    actions::answer_question(&local(&state), &session_id, ask_id, question_index, option_index)
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
    open_in_browser(&url)
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
    open_in_browser(url.trim())
}

/// Types `/model x` or `/effort y` into the session's Terminal tab.
#[tauri::command(async)]
fn set_session_option(app: AppHandle, state: TauriState<AppState>, session_id: String, setting: String, value: String) -> Result<(), String> {
    if let Some(machine) = remote_machine_of(&state, &session_id) {
        return route_done(&app, &machine, CommandKind::SetOption { session: session_id, setting, value });
    }
    actions::set_session_option(&local(&state), &session_id, &setting, &value)
}

/// Types `/rename <name>` into the session's Terminal tab. The new name comes
/// back through the session registry on the next refresh.
#[tauri::command(async)]
fn rename_session(app: AppHandle, state: TauriState<AppState>, session_id: String, name: String) -> Result<(), String> {
    if let Some(machine) = remote_machine_of(&state, &session_id) {
        return route_done(&app, &machine, CommandKind::Rename { session: session_id, name });
    }
    actions::rename_session(&local(&state), &session_id, &name)
}

/// Types `/compact` into the session's Terminal tab.
#[tauri::command(async)]
fn compact_session(app: AppHandle, state: TauriState<AppState>, session_id: String) -> Result<(), String> {
    if let Some(machine) = remote_machine_of(&state, &session_id) {
        return route_done(&app, &machine, CommandKind::Compact { session: session_id });
    }
    actions::compact_session(&local(&state), &session_id)
}

/// Ends an idle or completed session and closes its terminal.
#[tauri::command(async)]
fn close_session(app: AppHandle, state: TauriState<AppState>, session_id: String) -> Result<(), String> {
    if let Some(machine) = remote_machine_of(&state, &session_id) {
        return route_done(&app, &machine, CommandKind::Close { session: session_id });
    }
    actions::close_session(&local(&state), &session_id)
}

/// Types a `/command` or `!` shell line from the composer into the session's terminal.
#[tauri::command(async)]
fn send_slash_command(app: AppHandle, state: TauriState<AppState>, session_id: String, text: String) -> Result<(), String> {
    if let Some(machine) = remote_machine_of(&state, &session_id) {
        return route_done(&app, &machine, CommandKind::Slash { session: session_id, text });
    }
    actions::send_slash_command(&local(&state), &session_id, &text)
}

/// Sends Shift+Tab to the session's Terminal tab, cycling its permission mode.
#[tauri::command(async)]
fn cycle_session_mode(app: AppHandle, state: TauriState<AppState>, session_id: String) -> Result<(), String> {
    if let Some(machine) = remote_machine_of(&state, &session_id) {
        return route_done(&app, &machine, CommandKind::CycleMode { session: session_id });
    }
    actions::cycle_session_mode(&local(&state), &session_id)
}

/// `None` or `""` means the machine argument was not given: this Mac.
fn is_local(machine: &Option<String>) -> bool {
    machine.as_deref().is_none_or(str::is_empty)
}

/// Past sessions of a project folder for `agent`, newest first, with running ones marked.
#[tauri::command(async)]
fn list_resumable_sessions(app: AppHandle, state: TauriState<AppState>, dir: String, machine: Option<String>, agent: Option<Harness>) -> Result<Vec<resume::ResumableSession>, String> {
    let agent = agent.unwrap_or_default();
    if !is_local(&machine) {
        let machine = machine.unwrap();
        check_remote_agent(agent, &merge::agents_of(&remote_boards(&state), &machine), &machine)?;
        return route_data(&app, &machine, CommandKind::ListResumable { dir, agent });
    }
    actions::list_resumable_sessions(&local(&state), agent, &dir)
}

/// Opens a terminal in the folder resuming the session with `agent`.
#[tauri::command(async)]
fn resume_session(app: AppHandle, state: TauriState<AppState>, dir: String, session_id: String, machine: Option<String>, agent: Option<Harness>) -> Result<(), String> {
    let agent = agent.unwrap_or_default();
    if !is_local(&machine) {
        let machine = machine.unwrap();
        check_remote_agent(agent, &merge::agents_of(&remote_boards(&state), &machine), &machine)?;
        return route_done(&app, &machine, CommandKind::Resume { dir, session: session_id, agent });
    }
    actions::resume_session(&local(&state), agent, &dir, &session_id)
}

#[tauri::command(async)]
fn list_project_dirs(state: TauriState<AppState>, machine: Option<String>) -> Result<Vec<String>, String> {
    if !is_local(&machine) {
        let machine = machine.unwrap();
        return Ok(merge::dirs_of(&remote_boards(&state), &machine));
    }
    actions::list_project_dirs(&local(&state))
}

/// The agents the chosen machine can start, with their models. Locally the
/// first call waits for the listing (seconds); a remote's comes with its board.
#[tauri::command(async)]
fn list_agents(state: TauriState<AppState>, machine: Option<String>) -> AgentsReply {
    let local = is_local(&machine);
    let remote = if local { None } else { merge::agents_of(&remote_boards(&state), machine.as_deref().unwrap_or_default()) };
    agents_reply(local, remote, agents::current)
}

#[tauri::command(async)]
fn start_session(app: AppHandle, state: TauriState<AppState>, dir: Option<String>, prompt: String, options: launch::LaunchOptions, machine: Option<String>) -> Result<actions::StartResult, String> {
    if !is_local(&machine) {
        if prompt.trim().is_empty() {
            return Err("Type a prompt first.".into());
        }
        options.validate_shape()?;
        let machine = machine.unwrap();
        check_remote_agent(options.agent, &merge::agents_of(&remote_boards(&state), &machine), &machine)?;
        return route_data(&app, &machine, start_kind_for(dir, prompt, options));
    }
    actions::start_session(&local(&state), dir, prompt, options)
}

#[tauri::command(async)]
fn codex_hook_status() -> Result<bool, String> {
    maya_core::codex_hooks::status(&maya_core::codex_hooks::dir())
}

#[tauri::command(async)]
fn install_codex_hook() -> Result<bool, String> {
    maya_core::codex_hooks::install_to(&maya_core::codex_hooks::dir())
}

#[tauri::command(async)]
fn remove_codex_hook() -> Result<bool, String> {
    maya_core::codex_hooks::remove_from(&maya_core::codex_hooks::dir())
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
        eleven_wanted(&store.config, store.claude_dir())
    };
    let eleven = eleven_settings(eleven);
    // Through the one speech queue, so listening pauses and she does not
    // wake herself on "Maya here".
    let line = "Maya here. hexgrid needs a decision".to_string();
    notify::speak_and_wait(notify::Utterance { fallback: false, ..notify::Utterance::new(line, eleven) })
}

/// Mutes or unmutes Maya: muted, she makes no sound but still notifies.
#[tauri::command(async)]
fn set_muted(app: AppHandle, muted: bool) -> Result<Config, String> {
    log::line("app", if muted { "muted" } else { "unmuted" });
    update_config(&app, |c| c.muted = muted)
}

/// False when Maya cannot tell whether a Focus mode is on: on macOS, when
/// she has no Full Disk Access to read the Focus state.
#[tauri::command(async)]
fn focus_visible() -> bool {
    notify::focus_status().is_some()
}

/// Opens System Settings › Privacy & Security › Full Disk Access, where
/// Maya can be let read the Focus state.
#[tauri::command(async)]
fn open_full_disk_access() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let ok = std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")
            .status()
            .map_err(|e| format!("could not open System Settings: {e}"))?;
        if ok.success() {
            Ok(())
        } else {
            Err("System Settings did not open.".into())
        }
    }
    #[cfg(not(target_os = "macos"))]
    Err("Full Disk Access is a macOS setting.".into())
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
    let mut config = config.for_this_platform();
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
        // Mute belongs to the top-bar button, which may have saved it after
        // the page fetched this config.
        config.muted = before.muted;
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
            set_muted,
            focus_visible,
            open_full_disk_access,
            list_sessions,
            focus_session,
            session_history,
            send_reply,
            answer_question,
            set_session_option,
            cycle_session_mode,
            send_slash_command,
            rename_session,
            compact_session,
            close_session,
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
            list_agents,
            codex_hook_status,
            install_codex_hook,
            remove_codex_hook,
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
            #[cfg(target_os = "macos")]
            focus::install_app_handle(app.handle().clone());
            notify::set_eleven_speaker(voice::speak);
            listener::install_speech_hook(app.handle().clone());
            #[cfg(target_os = "macos")]
            dock::set_dock_icon();
            // Starts the agents' first listing, so the start form finds it ready.
            let _ = agents::snapshot();
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
    fn the_elevenlabs_key_is_wanted_only_for_the_elevenlabs_provider_with_a_voice() {
        let dir = std::path::Path::new("/home/u/.claude");
        let cfg = |provider, voice: Option<&str>| Config { voice_provider: provider, elevenlabs_voice_id: voice.map(String::from), ..Default::default() };
        assert_eq!(eleven_wanted(&cfg(voice::VoiceProvider::Builtin, Some("v")), dir), None);
        assert_eq!(eleven_wanted(&cfg(voice::VoiceProvider::Elevenlabs, None), dir), None);
        assert_eq!(eleven_wanted(&cfg(voice::VoiceProvider::Elevenlabs, Some("  ")), dir), None);
        assert_eq!(eleven_wanted(&cfg(voice::VoiceProvider::Elevenlabs, Some("v")), dir), Some((dir.join("maya"), "v".to_string())));
        assert_eq!(eleven_settings(None), None, "no key lookup when the voice is not wanted");
    }

    #[test]
    fn mute_silences_the_voice_and_the_banner_and_focus_only_the_voice() {
        // (speak setting, muted, focus) -> (spoken, banner sound)
        assert_eq!(announcement(true, false, false), (true, false), "the voice replaces the banner's sound");
        assert_eq!(announcement(false, false, false), (false, true));
        assert_eq!(announcement(true, false, true), (false, true), "a Focus mode leaves banner sounds to macOS");
        assert_eq!(announcement(true, true, false), (false, false));
        assert_eq!(announcement(false, true, false), (false, false));
        assert_eq!(announcement(true, true, true), (false, false));
    }

    #[test]
    fn prune_ttys_forgets_pids_no_longer_on_the_board() {
        let mut cache: std::collections::HashMap<i32, Option<String>> = [(1, Some("/dev/pts/1".to_string())), (2, Some("/dev/pts/2".to_string())), (4, None)].into();
        prune_ttys(&mut cache, &[2, 3, 4].into());
        assert_eq!(cache, [(2, Some("/dev/pts/2".to_string())), (4, None)].into());
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
            machine_address: None, machine_platform: None,
            terminal: None,
            stale: false, model: None
        };
        assert_eq!(pr_link(&card(" https://github.com/o/r/pull/7 ")), Ok("https://github.com/o/r/pull/7".to_string()));
        assert_eq!(pr_link(&card("file:///Applications/Calculator.app")).unwrap_err(), "That pull request's link is not a web address.");
        assert_eq!(pr_link(&card("/Applications/Calculator.app")).unwrap_err(), "That pull request's link is not a web address.");
        assert_eq!(pr_link(&Card { pr: None, ..card("") }).unwrap_err(), "No pull request is known for this session yet.");
    }

    #[test]
    fn a_paired_list_that_cannot_be_saved_is_not_changed() {
        let dir = std::env::temp_dir().join(format!("maya-save-paired-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = |id: &str| config::PairedAssistant { id: id.into(), name: id.into(), hostname: "h".into(), platform: "macos".into(), token: "t".into(), address: "10.0.0.5".into(), last_seen: None };
        let mut config = Config::default();
        config.network.assistants = vec![p("a1"), p("b2")];
        // A file where the config's directory should be: the save fails.
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        let err = save_paired(&blocker.join("maya/config.json"), &mut config, |l| l.retain(|a| a.id != "a1")).unwrap_err();
        assert!(err.starts_with("Could not save the paired assistants: "), "{err}");
        assert_eq!(config.network.assistants.len(), 2, "the removal is not made");
        let path = dir.join("maya/config.json");
        assert_eq!(save_paired(&path, &mut config, |l| l.retain(|a| a.id != "a1")), Ok(()));
        assert_eq!(config.network.assistants.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["b2"]);
        assert_eq!(config::load(&path).network.assistants.len(), 1, "and it is on disk");
        std::fs::remove_dir_all(&dir).ok();
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

    #[test]
    fn an_older_remote_offers_claude_and_no_name() {
        let r = agents_reply(false, None, || panic!("not local"));
        assert_eq!(r.agents, vec![maya_core::agents::claude()]);
        assert!(!r.names);
        let r = agents_reply(false, Some(vec![maya_core::agents::claude(), maya_core::agents::info_for(Harness::Grok, None)]), || panic!("not local"));
        assert_eq!(r.agents.len(), 2);
        assert!(r.names);
        assert!(agents_reply(true, None, || vec![maya_core::agents::claude()]).names);
    }

    #[test]
    fn a_remote_start_needs_the_agent_on_that_machine() {
        assert!(check_remote_agent(Harness::ClaudeCode, &None, "laptop").is_ok());
        assert!(check_remote_agent(Harness::Codex, &None, "laptop").unwrap_err().contains("laptop"));
        let grok_only = Some(vec![maya_core::agents::claude(), maya_core::agents::info_for(Harness::Grok, None)]);
        assert!(check_remote_agent(Harness::Grok, &grok_only, "laptop").is_ok());
        assert!(check_remote_agent(Harness::Codex, &grok_only, "laptop").unwrap_err().contains("not installed"));
    }
}
