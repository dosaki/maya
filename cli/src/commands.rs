//! The `pair`, `status`, `hooks`, `config` and `start` subcommands.

use crate::args::HooksOp;
use crate::executor::CliExecutor;
use crate::status_file;
use crate::tmux::Tmux;
use maya_core::actions;
use maya_core::config::{self, NetworkRole};
use maya_core::hook_install;
use maya_core::launch::LaunchOptions;
use maya_core::net::client::{self, Executor};
use maya_core::registry::pid_alive;
use maya_core::store::Store;
use maya_core::terminal::Terminal;
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

    match store.config.projects_dir_path() {
        Some(p) => out.push_str(&format!("projects: {}\n", p.display())),
        None => out.push_str("projects: unset\n"),
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

/// `maya config projects-dir <path>`: saves the folder start and resume
/// pick projects from. A `~` is kept and expanded when read, as the app
/// does; any other relative path is made absolute against the current
/// folder. Refused, and not saved, when the folder does not exist.
pub fn set_projects_dir(claude_dir: &Path, dir: &str) -> Result<String, String> {
    let dir = dir.trim();
    let stored = if dir.starts_with('~') || Path::new(dir).is_absolute() {
        dir.to_string()
    } else {
        std::env::current_dir().map_err(|e| e.to_string())?.join(dir).to_string_lossy().into_owned()
    };
    let path = claude_dir.join("maya/config.json");
    let mut c = config::load(&path);
    c.projects_dir = Some(stored);
    let resolved = c.projects_dir_path().ok_or("A projects directory needs a path.")?;
    if !resolved.is_dir() {
        return Err(format!("Projects directory does not exist: {}.", resolved.display()));
    }
    config::save(&path, &c)?;
    Ok(format!("projects: {}", resolved.display()))
}

/// Opens a session in a project folder through tmux: `maya start [dir] [prompt]`.
pub fn start(claude_dir: &Path, dir: Option<String>, prompt: Option<String>, options: LaunchOptions) -> Result<String, String> {
    start_with(claude_dir, dir, prompt, options, Arc::new(Tmux::default()))
}

pub fn start_with(claude_dir: &Path, dir: Option<String>, prompt: Option<String>, options: LaunchOptions, terminal: Arc<dyn Terminal>) -> Result<String, String> {
    let store = Mutex::new(Store::new(claude_dir.to_path_buf()));
    let l = actions::Local { store: &store, terminal: &*terminal };
    let r = actions::start_session(&l, dir, prompt.unwrap_or_default(), options)?;
    let name = Path::new(&r.dir).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(r.dir.clone());
    Ok(match r.terminal { Some(t) => format!("started in {name}: tmux attach -t {t}"), None => format!("started in {name}") })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status_file::RunStatus;
    use maya_core::actions::test_support::store_with_projects;
    use maya_core::config::Config;
    use maya_core::terminal::FakeTerminal;

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
    fn status_reports_the_projects_directory_or_unset() {
        let dir = claude_dir_with(NetworkRole::Assistant);
        assert!(status(dir.path()).unwrap().contains("\nprojects: unset\n"));
        let projects = dir.path().join("projects");
        std::fs::create_dir_all(&projects).unwrap();
        set_projects_dir(dir.path(), &projects.to_string_lossy()).unwrap();
        let report = status(dir.path()).unwrap();
        assert!(report.contains(&format!("\nprojects: {}\n", projects.display())), "{report}");
    }

    #[test]
    fn config_projects_dir_saves_the_path_and_keeps_the_rest_of_the_config() {
        let dir = claude_dir_with(NetworkRole::Assistant);
        let projects = dir.path().join("projects");
        std::fs::create_dir_all(&projects).unwrap();
        assert_eq!(set_projects_dir(dir.path(), &projects.to_string_lossy()), Ok(format!("projects: {}", projects.display())));
        let saved = config::load(&dir.path().join("maya/config.json"));
        assert_eq!(saved.projects_dir_path(), Some(projects.clone()));
        assert_eq!(saved.network.role, NetworkRole::Assistant);
    }

    #[test]
    fn config_projects_dir_keeps_a_tilde_for_later_expansion_and_refuses_a_missing_folder() {
        let dir = claude_dir_with(NetworkRole::Off);
        // `~` alone is the home folder, which exists.
        assert!(set_projects_dir(dir.path(), "~").is_ok());
        assert_eq!(config::load(&dir.path().join("maya/config.json")).projects_dir.as_deref(), Some("~"));
        let missing = dir.path().join("nope");
        assert_eq!(set_projects_dir(dir.path(), &missing.to_string_lossy()), Err(format!("Projects directory does not exist: {}.", missing.display())));
        assert_eq!(config::load(&dir.path().join("maya/config.json")).projects_dir.as_deref(), Some("~"), "a refused path is not saved");
    }

    #[test]
    fn hooks_status_reports_installed_state() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.json"), "{}").unwrap();
        assert_eq!(hooks(dir.path(), HooksOp::Status).unwrap(), "hooks: not installed");
        hooks(dir.path(), HooksOp::Install).unwrap();
        assert_eq!(hooks(dir.path(), HooksOp::Status).unwrap(), "hooks: installed");
    }

    #[test]
    fn start_prints_the_tmux_session_and_needs_a_prompt() {
        let (dir, _store) = store_with_projects(&["proj"]);
        let fake = Arc::new(FakeTerminal::default());
        let out = start_with(dir.path(), Some("proj".into()), Some("hello".into()), LaunchOptions::default(), fake.clone()).unwrap();
        assert!(out.starts_with("started in proj: tmux attach -t maya-"), "{out}");
        assert_eq!(start_with(dir.path(), Some("proj".into()), None, LaunchOptions::default(), fake).unwrap_err(), "Type a prompt first.");
    }
}
