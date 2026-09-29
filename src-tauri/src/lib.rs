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
pub mod launch;
pub mod model;
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

use config::Config;
use model::Card;
use std::sync::Mutex;
use std::time::Duration;
use store::{now_ms, Store};
use tauri::{AppHandle, Emitter, Manager, State as TauriState};

pub struct AppState {
    pub store: Mutex<Store>,
    pub notifier: Mutex<notify::Notifier>,
    pub reviews: Mutex<ReviewState>,
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
            Err(e) => r.error = Some(e),
        }
        r.clone()
    };
    let _ = app.emit("reviews", &snapshot);
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
    let (cards, wants_notify, speak, eleven) = {
        let state = app.state::<AppState>();
        let mut store = state.store.lock().unwrap();
        let cards = store.refresh(now_ms());
        let eleven = if store.config.speak_notifications { eleven_settings(&store) } else { None };
        (cards, store.config.notify_on_awaiting, store.config.speak_notifications, eleven)
    };
    // Track every refresh so a toggle-on later does not replay old events.
    let (fresh, finished) = {
        let state = app.state::<AppState>();
        let mut n = state.notifier.lock().unwrap();
        (n.take_new(&cards), n.take_finished(&cards))
    };
    // A Focus mode (Do Not Disturb and friends) keeps Maya quiet; banners are
    // left to macOS, which filters them by the Focus's own rules.
    let speak = speak && !((!fresh.is_empty() || !finished.is_empty()) && notify::focus_active());
    if wants_notify {
        for c in &fresh {
            // With a voice the banner stays silent; the sound is replaced, not doubled.
            notify::notify(c, !speak);
            if speak {
                if let Some(line) = notify::spoken_line(c) {
                    notify::speak(notify::Utterance { text: line, eleven: eleven.clone() });
                }
            }
        }
        if speak {
            for c in &finished {
                if let Some(line) = notify::spoken_line(c) {
                    notify::speak(notify::Utterance { text: line, eleven: eleven.clone() });
                }
            }
        }
    }
    let _ = app.emit("sessions", &cards);
}

#[tauri::command(async)]
fn list_sessions(state: TauriState<AppState>) -> Vec<Card> {
    state.store.lock().unwrap().refresh(now_ms())
}

#[tauri::command(async)]
fn focus_session(pid: i32) -> Result<(), String> {
    focus::focus_pid(pid)
}

#[tauri::command(async)]
fn session_history(state: TauriState<AppState>, session_id: String) -> Result<Vec<transcript::Turn>, String> {
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
fn send_reply(state: TauriState<AppState>, session_id: String, text: String) -> Result<(), String> {
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
fn answer_question(state: TauriState<AppState>, session_id: String, ask_id: u64, question_index: usize, option_index: usize) -> Result<(), String> {
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

/// Opens the session's pull request in the browser. The URL comes from the
/// PR cache, never from the page.
#[tauri::command(async)]
fn open_pr(state: TauriState<AppState>, session_id: String) -> Result<(), String> {
    let card = {
        let mut store = state.store.lock().unwrap();
        store.card_for(&session_id, now_ms()).ok_or("Session is no longer running.")?
    };
    let pr = card.pr.ok_or("No pull request is known for this session yet.")?;
    let ok = std::process::Command::new("open").arg(&pr.url).status().map_err(|e| format!("could not open the browser: {e}"))?;
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
fn set_session_option(state: TauriState<AppState>, session_id: String, setting: String, value: String) -> Result<(), String> {
    let text = answer::slash_command(&setting, &value)?;
    type_into_session(&state, &session_id, &text)
}

/// Types `/rename <name>` into the session's Terminal tab. The new name comes
/// back through the session registry on the next refresh.
#[tauri::command(async)]
fn rename_session(state: TauriState<AppState>, session_id: String, name: String) -> Result<(), String> {
    let text = answer::rename_command(&name)?;
    type_into_session(&state, &session_id, &text)
}

/// Types `/compact` into the session's Terminal tab.
#[tauri::command(async)]
fn compact_session(state: TauriState<AppState>, session_id: String) -> Result<(), String> {
    type_into_session(&state, &session_id, answer::COMPACT)
}

/// Sends Shift+Tab to the session's Terminal tab, cycling its permission mode.
#[tauri::command(async)]
fn cycle_session_mode(state: TauriState<AppState>, session_id: String) -> Result<(), String> {
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

#[derive(serde::Serialize)]
pub struct StartResult {
    pub dir: String,
    pub how: &'static str,
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

/// Past sessions of a project folder, newest first, with running ones marked.
#[tauri::command(async)]
fn list_resumable_sessions(state: TauriState<AppState>, dir: String) -> Result<Vec<resume::ResumableSession>, String> {
    let path = project_path(&state, &dir)?;
    let (claude_dir, running) = {
        let store = state.store.lock().unwrap();
        (store.claude_dir().to_path_buf(), store.live_session_ids())
    };
    Ok(resume::list_sessions(&claude_dir, &path.to_string_lossy(), &running))
}

/// Opens a Terminal in the folder running `claude --resume <id>`.
#[tauri::command(async)]
fn resume_session(state: TauriState<AppState>, dir: String, session_id: String) -> Result<(), String> {
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
fn list_project_dirs(state: TauriState<AppState>) -> Result<Vec<String>, String> {
    Ok(launch::list_project_dirs(&projects_root(&state)?))
}

#[tauri::command(async)]
fn start_session(state: TauriState<AppState>, dir: Option<String>, prompt: String, options: launch::LaunchOptions) -> Result<StartResult, String> {
    if prompt.trim().is_empty() {
        return Err("Type a prompt first.".into());
    }
    options.validate()?;
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
    Ok(StartResult { dir: target.to_string_lossy().into_owned(), how })
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
    let line = "Maya here. hexgrid needs a decision".to_string();
    match eleven {
        Some((dir, key, voice_id)) => voice::speak(&dir, &key, &voice_id, &line),
        None => {
            notify::say_builtin(&line);
            Ok(())
        }
    }
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
    {
        let mut store = state.store.lock().unwrap();
        config::save(&store.config_path(), &config)?;
        store.config = config.clone();
    }
    refresh_and_emit(&app);
    Ok(config)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let dir = claude_dir();
    let mut store = Store::new(dir.clone());
    store.compact_events();

    tauri::Builder::default()
        .manage(AppState { store: Mutex::new(store), notifier: Mutex::new(notify::Notifier::default()), reviews: Mutex::new(ReviewState::default()) })
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
            try_voice
        ])
        .setup(move |app| {
            focus::install_app_handle(app.handle().clone());
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
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
