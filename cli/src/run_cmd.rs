//! `maya run`: the headless assistant. Keeps one connection to the main
//! open (reconnecting with backoff), runs its commands through tmux, and
//! pushes this machine's board whenever the sessions change.
//!
//! Exit codes: 0 after SIGINT or SIGTERM; 1 when the main removed this
//! assistant or the pairing is gone; 2 when it cannot start (not paired,
//! the main role, another run holding the lock) or the config on disk stopped being
//! an assistant's while it ran.

use crate::commands::{jq_found, NO_JQ};
use crate::executor::CliExecutor;
use crate::notify::CliNotify;
use crate::run_lock::{self, RunLock};
use crate::status_file::{self, RunStatus};
use crate::tmux::Tmux;
use maya_core::config::{Config, NetworkConfig, NetworkRole};
use maya_core::net::client::{self, ClientNotify};
use maya_core::store::Store;
use maya_core::terminal::Terminal;
use maya_core::{hook_install, log, now_ms, watcher};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

/// Refuses to start unless this machine is a paired assistant and no other
/// `maya run` holds the lock at `lock_path`; the lock, held for the run's
/// lifetime, otherwise. `existing` (the status file) only names the other
/// run's pid in the message: a file left by a dead run does not block.
pub fn preflight(config: &Config, lock_path: &Path, existing: Option<&RunStatus>) -> Result<RunLock, (i32, String)> {
    match config.network.role {
        NetworkRole::Main => return Err((2, "the CLI is assistant-only".into())),
        NetworkRole::Off => return Err((2, "run `maya pair` first".into())),
        NetworkRole::Assistant if config.network.token.is_empty() || config.network.assistant_id.is_empty() => return Err((2, "run `maya pair` first".into())),
        NetworkRole::Assistant => {}
    }
    match RunLock::try_take(lock_path) {
        Ok(Some(lock)) => Ok(lock),
        Ok(None) => Err((2, format!("maya run is already running{}", pid_suffix(existing)))),
        Err(e) => Err((2, e)),
    }
}

/// ` (pid N)` from the status file when there is one, else nothing.
pub fn pid_suffix(existing: Option<&RunStatus>) -> String {
    existing.map(|s| format!(" (pid {})", s.pid)).unwrap_or_default()
}

/// How `run` exits once the client stopped: 2 when the watcher saw the
/// config stop being an assistant's (`config_changed`, although that also
/// set `stop`), 0 for a signal, 2 when the config is no longer an
/// assistant's, 1 otherwise (removed, or unpaired).
pub fn exit_code_for(stopped_by_signal: bool, config_changed: bool, config: &Config) -> i32 {
    if config_changed {
        2
    } else if stopped_by_signal {
        0
    } else if config.network.role != NetworkRole::Assistant {
        2
    } else {
        1
    }
}

pub const NO_HOOKS: &str = "hooks are not installed: sessions will not be seen (run `maya hooks install`)";
pub const NO_PROJECTS_DIR: &str = "no projects directory: start and resume from the main will fail until you run `maya config projects-dir <path>`";

/// What `run` logs at start about a setup that will not fully work; it
/// keeps going regardless.
pub fn startup_warnings(config: &Config, hooks_installed: bool, jq_found: bool) -> Vec<&'static str> {
    let mut out = Vec::new();
    if !hooks_installed {
        out.push(NO_HOOKS);
    }
    if !jq_found {
        out.push(NO_JQ);
    }
    if config.projects_dir_path().is_none() {
        out.push(NO_PROJECTS_DIR);
    }
    out
}

static STOP: OnceLock<Arc<AtomicBool>> = OnceLock::new();

extern "C" fn on_signal(_: libc::c_int) {
    if let Some(s) = STOP.get() {
        s.store(true, Ordering::SeqCst)
    }
}

/// SIGINT and SIGTERM set `stop`; the client closes within a tick.
fn install_signal_handlers(stop: Arc<AtomicBool>) {
    let _ = STOP.set(stop);
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
    }
}

/// The network config for the next connect attempt, read afresh from
/// `path` into the store: a config edited to another role stops the client
/// with NOT_PAIRED. A missing `config.json` is an unpair signal: the role
/// goes `Off` and the credentials are cleared, so the client fails with
/// NOT_PAIRED. A file that exists but does not parse, or any other read
/// error (half-written, or broken by hand), keeps the last good config.
fn reload_network(store: &Mutex<Store>, path: &Path) -> NetworkConfig {
    let mut s = store.lock().unwrap();
    match std::fs::read_to_string(path) {
        Ok(t) => match serde_json::from_str::<Config>(&t) {
            Ok(c) => s.config = c,
            Err(e) => log::line("cli", format!("config.json unreadable, keeping the last good one: {e}")),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::line("cli", "config.json is gone; treating this as unpaired");
            s.config.network.role = NetworkRole::Off;
            s.config.network.token.clear();
            s.config.network.assistant_id.clear();
        }
        Err(e) => log::line("cli", format!("config.json unreadable, keeping the last good one: {e}")),
    }
    let mut n = s.config.network.clone();
    if n.role != NetworkRole::Assistant {
        n.token.clear();
        n.assistant_id.clear();
    }
    n
}

/// What changes when `config.json` is written, replaced or removed (`None`).
type ConfigStamp = Option<(u64, Option<SystemTime>, u64)>;

fn config_stamp(path: &Path) -> ConfigStamp {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| (m.ino(), m.modified().ok(), m.len()))
}

/// The watcher's look at the config: reloads it into the store (through
/// `reload_network`) when it changed since `seen`, and says whether the
/// role is still Assistant. The watcher fires on every write under
/// `~/.claude/maya`, this run's own log and status file included, so an
/// unchanged file is not read again (an unreadable one would log, and so
/// fire the watcher, forever).
fn config_still_assistant(store: &Mutex<Store>, path: &Path, seen: &mut ConfigStamp) -> bool {
    let now = config_stamp(path);
    if *seen != now {
        *seen = now;
        reload_network(store, path);
    }
    store.lock().unwrap().config.network.role == NetworkRole::Assistant
}

pub fn run(claude_dir: &Path) -> i32 {
    let stop = Arc::new(AtomicBool::new(false));
    install_signal_handlers(stop.clone());
    run_with(claude_dir, Arc::new(Tmux::default()), stop)
}

/// `run` with the terminal and the stop flag given: setting `stop` is a signal.
pub fn run_with(claude_dir: &Path, terminal: Arc<dyn Terminal>, stop: Arc<AtomicBool>) -> i32 {
    let maya_dir = claude_dir.join("maya");
    let mut store = Store::new(claude_dir.to_path_buf());
    // Held until this function returns: a second run is refused meanwhile.
    let _lock = match preflight(&store.config, &run_lock::path(&maya_dir), status_file::read(&maya_dir).as_ref()) {
        Ok(lock) => lock,
        Err((code, msg)) => {
            eprintln!("{msg}");
            return code;
        }
    };
    // Only once no other run holds the lock: it would be rewriting events.jsonl under that run.
    store.compact_events();
    let config_path = store.config_path();
    let _ = log::init(&maya_dir.join("maya-cli.log"));
    // A stdout gone (an SSH logout) must not panic the thread that logged.
    log::install_emitter(|l| {
        let _ = writeln!(std::io::stdout().lock(), "{}", log::file_line(l));
    });
    log::line("cli", format!("maya {} run started", env!("CARGO_PKG_VERSION")));
    for w in startup_warnings(&store.config, matches!(hook_install::status(claude_dir), Ok(true)), jq_found()) {
        log::line("cli", w);
    }

    let store = Arc::new(Mutex::new(store));
    let exec = Arc::new(CliExecutor::new(store.clone(), terminal));
    let notify = Arc::new(CliNotify::new(maya_dir.clone(), stop.clone()));
    // Claims the status file at once, so a second run started before this
    // one connects sees it.
    notify.write(false, None, Some("connecting"));
    // Set by the watcher when the config on disk stops being an assistant's.
    let config_changed = Arc::new(AtomicBool::new(false));
    {
        let (s, due) = (store.clone(), exec.board_due.clone());
        let (sessions_dir, md) = (claude_dir.join("sessions"), maya_dir.clone());
        let (path, changed, stop) = (config_path.clone(), config_changed.clone(), stop.clone());
        // The config as loaded at start counts as seen.
        let mut seen = config_stamp(&path);
        std::thread::spawn(move || {
            watcher::run(&sessions_dir, &md, Duration::from_secs(5), || {
                s.lock().unwrap().refresh(now_ms());
                due.store(true, Ordering::SeqCst);
                if !config_still_assistant(&s, &path, &mut seen) && !changed.swap(true, Ordering::SeqCst) {
                    log::line("cli", "the config is no longer an assistant's; stopping");
                    stop.store(true, Ordering::SeqCst);
                }
            })
        });
    }
    let config_of = {
        let (s, path) = (store.clone(), config_path.clone());
        move || reload_network(&s, &path)
    };
    client::reconnect(&config_of, exec, notify.clone() as Arc<dyn ClientNotify>, &stop);

    let by_signal = stop.load(Ordering::SeqCst) && !notify.removed.load(Ordering::SeqCst);
    let changed = config_changed.load(Ordering::SeqCst);
    let code = exit_code_for(by_signal, changed, &store.lock().unwrap().config);
    if by_signal {
        status_file::remove(&maya_dir);
    }
    // The watcher logged it already when it was the one that saw it.
    if code == 2 && !changed {
        log::line("cli", "the config is no longer an assistant's; stopping");
    }
    log::line("cli", "stopped");
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preflight_refuses_the_main_role_and_an_unpaired_config() {
        let d = tempfile::tempdir().unwrap();
        let lock = run_lock::path(d.path());
        let mut c = Config::default();
        assert_eq!(preflight(&c, &lock, None).err(), Some((2, "run `maya pair` first".into())));
        c.network.role = NetworkRole::Main;
        assert_eq!(preflight(&c, &lock, None).err(), Some((2, "the CLI is assistant-only".into())));
        c.network.role = NetworkRole::Assistant;
        c.network.token = "t".into();
        c.network.assistant_id = "a".into();
        assert!(preflight(&c, &lock, None).is_ok());
    }

    #[test]
    fn preflight_refuses_while_another_run_holds_the_lock_and_ignores_a_stale_status_file() {
        let d = tempfile::tempdir().unwrap();
        let lock = run_lock::path(d.path());
        let mut c = Config::default();
        c.network.role = NetworkRole::Assistant;
        c.network.token = "t".into();
        c.network.assistant_id = "a".into();
        let twin = RunStatus { pid: 99, ..Default::default() };
        // A status file with no run holding the lock is left by a dead run.
        let held = preflight(&c, &lock, Some(&twin)).expect("a stale status file does not block");
        assert_eq!(preflight(&c, &lock, Some(&twin)).err(), Some((2, "maya run is already running (pid 99)".into())));
        assert_eq!(preflight(&c, &lock, None).err(), Some((2, "maya run is already running".into())));
        drop(held);
        drop(run_lock::take_once_free(&lock).expect("free once the first run's lock is dropped"));
        assert!(preflight(&c, &lock, None).is_ok());
    }

    #[test]
    fn an_unreadable_config_keeps_the_last_good_credentials() {
        let (dir, store) = maya_core::actions::test_support::store_with_session("s1", 4242);
        let path = {
            let mut s = store.lock().unwrap();
            s.config.network.role = NetworkRole::Assistant;
            s.config.network.token = "t".into();
            s.config.network.assistant_id = "a".into();
            s.config_path()
        };
        std::fs::write(&path, "{").unwrap();
        let n = reload_network(&store, &path);
        assert_eq!((n.role, n.token.as_str(), n.assistant_id.as_str()), (NetworkRole::Assistant, "t", "a"));
        // A config that parses and is no longer an assistant's clears them.
        std::fs::write(&path, r#"{"completedTimeoutMinutes":10,"network":{"role":"off","token":"t","assistantId":"a"}}"#).unwrap();
        let n = reload_network(&store, &path);
        assert_eq!((n.token.as_str(), n.assistant_id.as_str()), ("", ""));
        assert_eq!(store.lock().unwrap().config.network.role, NetworkRole::Off);
        drop(dir);
    }

    #[test]
    fn a_missing_config_clears_the_credentials_and_sets_the_role_off() {
        let (dir, store) = maya_core::actions::test_support::store_with_session("s1", 4242);
        let path = {
            let mut s = store.lock().unwrap();
            s.config.network.role = NetworkRole::Assistant;
            s.config.network.token = "t".into();
            s.config.network.assistant_id = "a".into();
            s.config_path()
        };
        std::fs::remove_file(&path).unwrap();
        let n = reload_network(&store, &path);
        assert_eq!((n.role, n.token.as_str(), n.assistant_id.as_str()), (NetworkRole::Off, "", ""));
        drop(dir);
    }

    #[test]
    fn run_warns_at_start_about_missing_hooks_jq_and_projects_directory() {
        let mut c = Config::default();
        assert_eq!(startup_warnings(&c, false, false), vec![NO_HOOKS, NO_JQ, NO_PROJECTS_DIR]);
        c.projects_dir = Some("/p".into());
        assert_eq!(startup_warnings(&c, true, true), Vec::<&str>::new());
        assert_eq!(NO_PROJECTS_DIR, "no projects directory: start and resume from the main will fail until you run `maya config projects-dir <path>`");
    }

    #[test]
    fn exit_code_is_zero_for_a_signal_two_for_a_config_no_longer_an_assistants_else_one() {
        let mut c = Config::default();
        c.network.role = NetworkRole::Assistant;
        assert_eq!(exit_code_for(true, false, &c), 0);
        assert_eq!(exit_code_for(false, false, &c), 1);
        c.network.role = NetworkRole::Off;
        assert_eq!(exit_code_for(true, false, &c), 0);
        assert_eq!(exit_code_for(false, false, &c), 2);
        c.network.role = NetworkRole::Main;
        assert_eq!(exit_code_for(false, false, &c), 2);
        // The watcher saw the role change and set `stop` itself: 2, not 0.
        assert_eq!(exit_code_for(true, true, &c), 2);
        c.network.role = NetworkRole::Assistant;
        assert_eq!(exit_code_for(true, true, &c), 2);
    }

    #[test]
    fn the_watcher_reloads_a_changed_config_and_sees_a_role_that_is_no_longer_an_assistants() {
        let (dir, store) = maya_core::actions::test_support::store_with_session("s1", 4242);
        let path = store.lock().unwrap().config_path();
        let assistant = r#"{"completedTimeoutMinutes":10,"network":{"role":"assistant","token":"t","assistantId":"a"}}"#;
        std::fs::write(&path, assistant).unwrap();
        let mut seen = None;
        assert!(config_still_assistant(&store, &path, &mut seen));
        assert_eq!(store.lock().unwrap().config.network.role, NetworkRole::Assistant);
        // Unchanged since the last look: not read again, so an in-memory
        // change stands.
        store.lock().unwrap().config.network.role = NetworkRole::Main;
        assert!(!config_still_assistant(&store, &path, &mut seen));
        store.lock().unwrap().config.network.role = NetworkRole::Assistant;
        // Edited to role off: reloaded, and no longer an assistant's.
        std::fs::write(&path, r#"{"completedTimeoutMinutes":10,"network":{"role":"off"}}"#).unwrap();
        assert!(!config_still_assistant(&store, &path, &mut seen));
        assert_eq!(store.lock().unwrap().config.network.role, NetworkRole::Off);
        // Removed: treated as unpaired.
        std::fs::write(&path, assistant).unwrap();
        assert!(config_still_assistant(&store, &path, &mut seen));
        std::fs::remove_file(&path).unwrap();
        assert!(!config_still_assistant(&store, &path, &mut seen));
        assert_eq!(store.lock().unwrap().config.network.role, NetworkRole::Off);
        drop(dir);
    }
}
