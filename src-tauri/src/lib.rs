pub mod config;
pub mod events;
pub mod focus;
pub mod hook_install;
pub mod inbox;
pub mod model;
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
}

fn claude_dir() -> std::path::PathBuf {
    dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/")).join(".claude")
}

fn refresh_and_emit(app: &AppHandle) {
    let cards = {
        let state = app.state::<AppState>();
        let mut store = state.store.lock().unwrap();
        store.refresh(now_ms())
    };
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
        .manage(AppState { store: Mutex::new(store) })
        .invoke_handler(tauri::generate_handler![
            list_sessions,
            focus_session,
            session_history,
            send_reply,
            hook_status,
            install_hook,
            remove_hook,
            get_config,
            set_config
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let sessions_dir = dir.join("sessions");
            let eye_dir = dir.join("eye");
            std::thread::spawn(move || {
                watcher::run(&sessions_dir, &eye_dir, Duration::from_secs(5), || refresh_and_emit(&handle));
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
