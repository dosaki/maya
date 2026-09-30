//! The `pair`, `status` and `hooks` subcommands.

use crate::args::HooksOp;
use crate::executor::CliExecutor;
use crate::status_file;
use crate::tmux::Tmux;
use maya_core::config::{self, NetworkRole};
use maya_core::hook_install;
use maya_core::net::client::{self, Executor};
use maya_core::registry::pid_alive;
use maya_core::store::Store;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Pairs with a main Maya at `host:port` using its six-digit `code`, stores
/// the assistant id and token and the link in the config, and returns the
/// main's name.
pub fn pair(claude_dir: &Path, host: &str, port: u16, name: Option<&str>, code: &str) -> Result<String, String> {
    let name = name.map(str::to_string).unwrap_or_else(maya_core::net::local_hostname);
    let store = Arc::new(Mutex::new(Store::new(claude_dir.to_path_buf())));
    let exec: Arc<dyn Executor> = Arc::new(CliExecutor::new(store.clone(), Arc::new(Tmux::default())));
    let (main, id, token) = client::pair_with(exec, host, port, &name, code)?;
    let mut s = store.lock().unwrap();
    s.config.network.role = NetworkRole::Assistant;
    s.config.network.main_host = host.trim().into();
    s.config.network.main_port = port;
    s.config.network.name = name;
    s.config.network.assistant_id = id;
    s.config.network.token = token;
    s.config.listen = false;
    config::save(&s.config_path(), &s.config)?;
    Ok(main)
}

/// The printed report for `maya status`.
pub fn status(claude_dir: &Path) -> Result<String, String> {
    let mut store = Store::new(claude_dir.to_path_buf());
    let role = store.config.network.role;
    let mut out = String::new();
    match role {
        NetworkRole::Off => out.push_str("role: off\n"),
        NetworkRole::Main => out.push_str("role: main\n"),
        NetworkRole::Assistant => {
            out.push_str("role: assistant\n");
            out.push_str(&format!("main: {}:{}\n", store.config.network.main_host, store.config.main_port()));
            out.push_str(&format!("name: {}\n", store.config.network.name));
        }
    }

    let run_status = status_file::read(&claude_dir.join("maya"));
    let run_line = match (role, run_status) {
        (NetworkRole::Assistant, Some(s)) if pid_alive(s.pid as i32) && s.connected => {
            format!("run: connected to {} (pid {})", s.main_name.as_deref().unwrap_or("the main Maya"), s.pid)
        }
        (NetworkRole::Assistant, Some(s)) if pid_alive(s.pid as i32) && !s.connected => {
            format!("run: retrying (pid {}): {}", s.pid, s.error.as_deref().unwrap_or(""))
        }
        _ => "run: not running".to_string(),
    };
    out.push_str(&run_line);
    out.push('\n');

    let sessions = store.refresh(maya_core::now_ms()).len();
    out.push_str(&format!("sessions: {sessions}"));
    Ok(out)
}

/// Installs, removes or reports the Claude Code hooks Maya listens to.
pub fn hooks(claude_dir: &Path, op: HooksOp) -> Result<String, String> {
    match op {
        HooksOp::Install => hook_install::install_to(claude_dir).map(|_| "hooks installed".to_string()),
        HooksOp::Remove => hook_install::remove_from(claude_dir).map(|_| "hooks removed".to_string()),
        HooksOp::Status => hook_install::status(claude_dir).map(|installed| if installed { "hooks: installed".to_string() } else { "hooks: not installed".to_string() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status_file::RunStatus;
    use maya_core::config::Config;

    fn claude_dir_with(role: NetworkRole) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let mut c = Config::default();
        c.network.role = role;
        config::save(&dir.path().join("maya/config.json"), &c).unwrap();
        dir
    }

    #[test]
    fn status_reports_off_and_not_running() {
        let dir = claude_dir_with(NetworkRole::Off);
        let report = status(dir.path()).unwrap();
        assert!(report.starts_with("role: off"), "{report}");
        assert!(report.contains("run: not running"), "{report}");
    }

    #[test]
    fn status_reports_a_connected_assistant() {
        let dir = claude_dir_with(NetworkRole::Assistant);
        status_file::write(&dir.path().join("maya"), &RunStatus { pid: 1, connected: true, main_name: Some("Yhi".into()), error: None, updated_ms: 1 }).unwrap();
        let report = status(dir.path()).unwrap();
        assert!(report.contains("run: connected to Yhi (pid 1)"), "{report}");
    }

    #[test]
    fn hooks_status_reports_installed_state() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.json"), "{}").unwrap();
        assert_eq!(hooks(dir.path(), HooksOp::Status).unwrap(), "hooks: not installed");
        hooks(dir.path(), HooksOp::Install).unwrap();
        assert_eq!(hooks(dir.path(), HooksOp::Status).unwrap(), "hooks: installed");
    }
}
