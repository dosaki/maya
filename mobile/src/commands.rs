//! The commands the page calls, by the names the desktop's page uses, so
//! the board, card, modal and dialogs run unchanged. Every session command
//! goes to the assistant that lists the session; nothing runs here.

use crate::hub::Hub;
use crate::android;
use maya_core::actions::{Route, StartResult};
use maya_core::agents::AgentInfo;
use maya_core::config::{Config, NetworkRole};
use maya_core::launch::LaunchOptions;
use maya_core::model::{Card, Harness};
use maya_core::net::merge;
use maya_core::net::protocol::CommandKind;
use maya_core::net::routing::{agents_reply, check_remote_agent, machines_of, remote_attachments, AgentsReply, MachineInfo};
use maya_core::net::NetworkStatus;
use maya_core::resume::ResumableSession;
use maya_core::store::now_ms;
use maya_core::transcript::Turn;
use maya_core::{attachments, log};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::Arc;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

type H<'a> = State<'a, Arc<Hub>>;

fn data<T: DeserializeOwned>(value: Option<Value>) -> Result<T, String> {
    serde_json::from_value(value.ok_or("The assistant sent no result.")?).map_err(|e| e.to_string())
}

/// The pickers always name a machine on the phone; a blank one is a page bug.
fn machine_arg(machine: Option<String>) -> Result<String, String> {
    machine.map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).ok_or_else(|| "Choose a machine first.".into())
}

#[tauri::command]
pub fn list_sessions(hub: H) -> Vec<Card> {
    hub.cards()
}

#[tauri::command(async)]
pub fn session_history(hub: H, session_id: String) -> Result<Vec<Turn>, String> {
    data(hub.send(&session_id, |session| CommandKind::History { session })?)
}

#[tauri::command(async)]
pub fn send_reply(hub: H, session_id: String, text: String, attachments: Vec<String>) -> Result<Option<Route>, String> {
    let attachments = remote_attachments(&attachments)?;
    hub.send(&session_id, |session| CommandKind::Reply { session, text, attachments }).map(|d| Route::from_data(d.as_ref()))
}

#[tauri::command(async)]
pub fn answer_question(hub: H, session_id: String, ask_id: u64, question_index: usize, option_index: usize) -> Result<(), String> {
    hub.send(&session_id, |session| CommandKind::Answer { session, ask_id, question: question_index, option: option_index }).map(|_| ())
}

#[tauri::command(async)]
pub fn set_session_option(hub: H, session_id: String, setting: String, value: String) -> Result<(), String> {
    hub.send(&session_id, |session| CommandKind::SetOption { session, setting, value }).map(|_| ())
}

#[tauri::command(async)]
pub fn cycle_session_mode(hub: H, session_id: String) -> Result<(), String> {
    hub.send(&session_id, |session| CommandKind::CycleMode { session }).map(|_| ())
}

#[tauri::command(async)]
pub fn send_slash_command(hub: H, session_id: String, text: String) -> Result<(), String> {
    hub.send(&session_id, |session| CommandKind::Slash { session, text }).map(|_| ())
}

#[tauri::command(async)]
pub fn rename_session(hub: H, session_id: String, name: String) -> Result<(), String> {
    hub.send(&session_id, |session| CommandKind::Rename { session, name }).map(|_| ())
}

#[tauri::command(async)]
pub fn compact_session(hub: H, session_id: String) -> Result<(), String> {
    hub.send(&session_id, |session| CommandKind::Compact { session }).map(|_| ())
}

#[tauri::command(async)]
pub fn close_session(hub: H, session_id: String) -> Result<(), String> {
    hub.send(&session_id, |session| CommandKind::Close { session }).map(|_| ())
}

/// Saves a picked file under the app's data so a reply can carry it.
#[tauri::command(async)]
pub fn save_attachment(hub: H, name: String, bytes: Vec<u8>) -> Result<String, String> {
    let path = attachments::save(&hub.maya_dir, &name, &bytes, now_ms())?;
    Ok(path.to_string_lossy().into_owned())
}

fn open_web(app: &AppHandle, url: &str) -> Result<(), String> {
    if !attachments::is_web_url(url) {
        return Err("Only web links can be opened.".into());
    }
    app.opener().open_url(url.trim(), None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    open_web(&app, &url)
}

/// The session's pull request, from the assistant's card, never from the page.
#[tauri::command(async)]
pub fn open_pr(app: AppHandle, hub: H, session_id: String) -> Result<(), String> {
    let card = hub.cards().into_iter().find(|c| c.session_id == session_id).ok_or("Session is no longer running.")?;
    let pr = card.pr.ok_or("No pull request is known for this session yet.")?;
    open_web(&app, &pr.url)
}

#[tauri::command]
pub fn list_machines(hub: H) -> Vec<MachineInfo> {
    machines_of(&hub.status())
}

#[tauri::command]
pub fn list_project_dirs(hub: H, machine: Option<String>) -> Result<Vec<String>, String> {
    let machine = machine_arg(machine)?;
    Ok(merge::dirs_of(&hub.boards(), &machine))
}

#[tauri::command]
pub fn list_agents(hub: H, machine: Option<String>) -> AgentsReply {
    let remote: Option<Vec<AgentInfo>> = machine.as_deref().filter(|m| !m.is_empty()).and_then(|m| merge::agents_of(&hub.boards(), m));
    agents_reply(false, remote, Vec::new)
}

#[tauri::command(async)]
pub fn list_resumable_sessions(hub: H, dir: String, machine: Option<String>, agent: Option<Harness>) -> Result<Vec<ResumableSession>, String> {
    let machine = machine_arg(machine)?;
    let agent = agent.unwrap_or_default();
    check_remote_agent(agent, &merge::agents_of(&hub.boards(), &machine), &machine)?;
    data(hub.send_to(&machine, CommandKind::ListResumable { dir, agent })?)
}

#[tauri::command(async)]
pub fn resume_session(hub: H, dir: String, session_id: String, machine: Option<String>, agent: Option<Harness>) -> Result<(), String> {
    let machine = machine_arg(machine)?;
    let agent = agent.unwrap_or_default();
    check_remote_agent(agent, &merge::agents_of(&hub.boards(), &machine), &machine)?;
    hub.send_to(&machine, CommandKind::Resume { dir, session: session_id, agent }).map(|_| ())
}

#[tauri::command(async)]
pub fn start_session(hub: H, dir: Option<String>, prompt: String, options: LaunchOptions, machine: Option<String>) -> Result<StartResult, String> {
    let machine = machine_arg(machine)?;
    if prompt.trim().is_empty() {
        return Err("Type a prompt first.".into());
    }
    options.validate_shape()?;
    check_remote_agent(options.agent, &merge::agents_of(&hub.boards(), &machine), &machine)?;
    data(hub.send_to(&machine, CommandKind::Start { dir, prompt, options })?)
}

#[tauri::command]
pub fn network_status(hub: H) -> NetworkStatus {
    hub.status()
}

#[tauri::command]
pub fn network_pairing_code(hub: H) -> Result<NetworkStatus, String> {
    hub.pairing_code()
}

#[tauri::command(async)]
pub fn network_remove_assistant(hub: H, id: String) -> Result<NetworkStatus, String> {
    hub.remove_assistant(&id)
}

#[tauri::command]
pub fn get_config(hub: H) -> Config {
    hub.settings.lock().unwrap().config.clone()
}

/// Saves what the Network screen edits: name, port and the two switches.
/// The role and the paired list stay the server's. A new name or port on a
/// running server restarts it; assistants reconnect within seconds.
#[tauri::command(async)]
pub fn set_config(hub: H, config: Config) -> Result<Config, String> {
    let before = hub.settings.lock().unwrap().config.clone();
    let saved = hub.update_config(|c| {
        if !config.network.name.trim().is_empty() {
            c.network.name = config.network.name.trim().to_string();
        }
        c.network.port = config.network.port;
        c.notify_on_awaiting = config.notify_on_awaiting;
        c.notify_on_completed = config.notify_on_completed;
    })?;
    let running = hub.server.lock().unwrap().is_some();
    if running && (before.listen_port() != saved.listen_port() || before.network.name != saved.network.name) {
        log::line("network", "settings changed; restarting the server");
        hub.start_server()?;
    }
    Ok(saved)
}

#[tauri::command]
pub fn log_lines() -> Vec<log::Line> {
    log::lines()
}

#[tauri::command]
pub fn log_clear() {
    log::clear()
}

#[tauri::command]
pub fn log_path(hub: H) -> String {
    hub.maya_dir.join("maya.log").to_string_lossy().into_owned()
}

/// Starts the server, remembers that it should run, and opens pairing.
#[tauri::command(async)]
pub fn server_start(hub: H) -> Result<NetworkStatus, String> {
    hub.update_config(|c| c.network.role = NetworkRole::Main)?;
    hub.start_server()?;
    hub.pairing_code()
}

#[tauri::command(async)]
pub fn server_stop(hub: H) -> Result<NetworkStatus, String> {
    hub.stop_server();
    hub.update_config(|c| c.network.role = NetworkRole::Off)?;
    Ok(hub.status())
}

#[tauri::command]
pub fn local_addresses(app: AppHandle) -> Vec<String> {
    android::local_addresses(&app)
}

#[tauri::command(async)]
pub fn request_battery_exemption(app: AppHandle) -> Result<(), String> {
    android::request_battery_exemption(&app)
}

#[tauri::command]
pub fn notifications_allowed(app: AppHandle) -> bool {
    android::notifications_allowed(&app)
}

/// The session of the last tapped notification, once. The id Android hands
/// back is matched to a card; after Android killed the app the card comes
/// back only once its assistant reconnects, so this waits for it a while.
#[tauri::command(async)]
pub fn pending_tap(app: AppHandle, hub: H) -> Option<String> {
    let id = android::pending_tap(&app)?;
    let found = hub.wait_for_notification(id, TAP_WAIT);
    if found.is_none() {
        log::line("android", format!("tapped notification {id}: no card for it"));
    }
    found
}

/// How long a tapped notification waits for its card.
const TAP_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_machine_must_be_named() {
        assert_eq!(machine_arg(None).unwrap_err(), "Choose a machine first.");
        assert_eq!(machine_arg(Some("  ".into())).unwrap_err(), "Choose a machine first.");
        assert_eq!(machine_arg(Some(" laptop ".into())).unwrap(), "laptop");
    }

    #[test]
    fn missing_data_is_an_error_not_a_panic() {
        assert_eq!(data::<Vec<Turn>>(None).unwrap_err(), "The assistant sent no result.");
        assert!(data::<Vec<Turn>>(Some(serde_json::json!([]))).unwrap().is_empty());
    }

}
