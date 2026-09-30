//! The `pair`, `status`, `hooks`, `config` and `start` subcommands.

use crate::args::HooksOp;
use crate::executor::CliExecutor;
use crate::status_file;
use crate::tmux::Tmux;
use maya_core::actions;
use maya_core::config::{self, NetworkRole};
use maya_core::hook_install;
use maya_core::launch::LaunchOptions;
use maya_core::net::client::{self, Executor, WRONG_CODE};
use maya_core::registry::pid_alive;
use maya_core::store::Store;
use maya_core::terminal::Terminal;
use std::ffi::OsStr;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// The hook script pipes every event through `jq`; without it no session is seen.
pub const NO_JQ: &str = "jq is not installed: Claude Code's hooks need it, so sessions will not be seen";

/// True when `name` is an executable file in one of `path`'s folders.
pub fn on_path(name: &str, path: Option<&OsStr>) -> bool {
    let Some(path) = path else { return false };
    std::env::split_paths(path).any(|dir| {
        std::fs::metadata(dir.join(name)).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    })
}

/// True when `jq` is on this process's PATH.
pub fn jq_found() -> bool {
    on_path("jq", std::env::var_os("PATH").as_deref())
}

/// The pid of a live `maya run`, from its status file; a file left by a
/// dead run does not count.
fn live_run(claude_dir: &Path) -> Option<u32> {
    status_file::read(&claude_dir.join("maya")).map(|s| s.pid).filter(|pid| pid_alive(*pid as i32))
}

/// What `maya pair` prints once paired.
pub fn paired_message(main: &str, label: &str) -> String {
    format!("Paired with {main} as {label}")
}

/// Pairs with a main Maya at `host:port` using its six-digit `code`, stores
/// the assistant id and token and the link in the config, and returns the
/// main's name and the label this machine paired under. Errors carry the exit code: 2 while a `maya run` is live
/// (pairing would swap the credentials under it), else 1.
pub fn pair(claude_dir: &Path, host: &str, port: u16, name: Option<&str>, code: &str) -> Result<(String, String), (i32, String)> {
    if let Some(pid) = live_run(claude_dir) {
        return Err((2, format!("stop `maya run` first (pid {pid})")));
    }
    // The main only ever issues six ASCII digits: anything else is refused
    // here, with the main's own message, without contacting it.
    let code = code.trim();
    if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return Err((1, WRONG_CODE.into()));
    }
    let name = name.map(str::trim).filter(|n| !n.is_empty()).map(str::to_string).unwrap_or_else(maya_core::net::local_hostname);
    let store = Arc::new(Mutex::new(Store::new(claude_dir.to_path_buf())));
    let exec: Arc<dyn Executor> = Arc::new(CliExecutor::new(store.clone(), Arc::new(Tmux::default())));
    let (main, id, token) = client::pair_with(exec, host, port, &name, code).map_err(|e| (1, e))?;
    let mut s = store.lock().unwrap();
    s.config.network.role = NetworkRole::Assistant;
    s.config.network.main_host = host.trim().into();
    s.config.network.main_port = port;
    s.config.network.name = name.clone();
    s.config.network.assistant_id = id;
    s.config.network.token = token;
    s.config.listen = false;
    config::save(&s.config_path(), &s.config).map_err(|e| (1, e))?;
    Ok((main, name))
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
    fn a_pairing_reports_the_main_and_this_machines_label() {
        assert_eq!(paired_message("Yhi", "box"), "Paired with Yhi as box");
    }

    #[test]
    fn a_code_that_is_not_six_digits_is_refused_without_contacting_the_main() {
        let dir = claude_dir_with(NetworkRole::Off);
        // Port 1 has no main: reaching the network would say "Could not reach".
        for code in ["12345", "1234567", "12a456", "", "１２３４５６"] {
            assert_eq!(pair(dir.path(), "127.0.0.1", 1, Some("box"), code), Err((1, WRONG_CODE.to_string())), "{code:?}");
        }
    }

    #[test]
    fn pair_refuses_while_maya_run_is_live_and_leaves_the_config_alone() {
        let dir = claude_dir_with(NetworkRole::Off);
        let me = std::process::id();
        status_file::write(&dir.path().join("maya"), &RunStatus { pid: me, connected: true, ..Default::default() }).unwrap();
        assert_eq!(pair(dir.path(), "127.0.0.1", 1, Some("box"), "123456"), Err((2, format!("stop `maya run` first (pid {me})"))));
        assert_eq!(config::load(&dir.path().join("maya/config.json")).network.role, NetworkRole::Off);
    }

    #[test]
    fn a_status_file_left_by_a_dead_run_does_not_block_pairing() {
        let dir = claude_dir_with(NetworkRole::Off);
        status_file::write(&dir.path().join("maya"), &RunStatus { pid: 2_000_000_000, ..Default::default() }).unwrap();
        // Past the check: it tries the main, which is not there.
        let (code, err) = pair(dir.path(), "127.0.0.1", 1, Some("box"), "123456").unwrap_err();
        assert_eq!(code, 1);
        assert!(err.starts_with("Could not reach"), "{err}");
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
    fn on_path_finds_an_executable_file_in_any_path_entry() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let jq = b.path().join("jq");
        std::fs::write(&jq, "#!/bin/sh\n").unwrap();
        let path = std::env::join_paths([a.path(), b.path()]).unwrap();
        assert!(!on_path("jq", Some(&path)), "not executable yet");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&jq, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(on_path("jq", Some(&path)));
        assert!(!on_path("jq", Some(a.path().as_os_str())));
        assert!(!on_path("jq", None));
        assert_eq!(NO_JQ, "jq is not installed: Claude Code's hooks need it, so sessions will not be seen");
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
