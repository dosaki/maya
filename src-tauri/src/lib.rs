pub mod answer;
pub mod config;
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
pub mod state;
pub mod store;
pub mod transcript;
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
}

fn claude_dir() -> std::path::PathBuf {
    dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/")).join(".claude")
}

fn refresh_and_emit(app: &AppHandle) {
    let (cards, wants_notify) = {
        let state = app.state::<AppState>();
        let mut store = state.store.lock().unwrap();
        (store.refresh(now_ms()), store.config.notify_on_awaiting)
    };
    // Track every refresh so a toggle-on later does not replay old asks.
    let fresh = app.state::<AppState>().notifier.lock().unwrap().take_new(&cards);
    if wants_notify {
        for c in &fresh {
            notify::notify(c);
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
    let path = {
        let store = state.store.lock().unwrap();
        let s = store.session(&session_id).ok_or("Session is no longer running.")?;
        store.transcript_path_for(&s)
    };
    Ok(transcript::read_turns(&path, 30))
}

#[tauri::command(async)]
fn send_reply(state: TauriState<AppState>, session_id: String, text: String) -> Result<(), String> {
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
        .manage(AppState { store: Mutex::new(store), notifier: Mutex::new(notify::Notifier::default()) })
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
            open_pr,
            list_project_dirs,
            start_session,
            hook_status,
            install_hook,
            remove_hook,
            get_config,
            set_config
        ])
        .setup(move |app| {
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
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
