//! `maya run`: the headless assistant. Keeps one connection to the main
//! open (reconnecting with backoff), runs its commands through tmux, and
//! pushes this machine's board whenever the sessions change.
//!
//! Exit codes: 0 after SIGINT or SIGTERM; 1 when the main removed this
//! assistant or the pairing is gone; 2 when it cannot start (not paired,
//! the main role, another run alive) or the config on disk stopped being
//! an assistant's while it ran.

use crate::executor::CliExecutor;
use crate::notify::CliNotify;
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
use std::time::Duration;

/// Refuses to start unless this machine is a paired assistant and no other
/// `maya run` is alive. A status file left by a dead run is ignored.
pub fn preflight(config: &Config, existing: Option<&RunStatus>, alive: &dyn Fn(u32) -> bool) -> Result<(), (i32, String)> {
    match config.network.role {
        NetworkRole::Main => return Err((2, "the CLI is assistant-only".into())),
        NetworkRole::Off => return Err((2, "run `maya pair` first".into())),
        NetworkRole::Assistant if config.network.token.is_empty() || config.network.assistant_id.is_empty() => return Err((2, "run `maya pair` first".into())),
        NetworkRole::Assistant => {}
    }
    if let Some(s) = existing {
        if s.pid != std::process::id() && alive(s.pid) {
            return Err((2, format!("maya run is already running (pid {})", s.pid)));
        }
    }
    Ok(())
}

/// How `run` exits once the client stopped: 0 for a signal, 2 when the
/// config is no longer an assistant's, 1 otherwise (removed, or unpaired).
pub fn exit_code_for(stopped_by_signal: bool, config: &Config) -> i32 {
    if stopped_by_signal {
        0
    } else if config.network.role != NetworkRole::Assistant {
        2
    } else {
        1
    }
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

pub fn run(claude_dir: &Path) -> i32 {
    let stop = Arc::new(AtomicBool::new(false));
    install_signal_handlers(stop.clone());
    run_with(claude_dir, Arc::new(Tmux::default()), stop)
}

/// `run` with the terminal and the stop flag given: setting `stop` is a signal.
pub fn run_with(claude_dir: &Path, terminal: Arc<dyn Terminal>, stop: Arc<AtomicBool>) -> i32 {
    let maya_dir = claude_dir.join("maya");
    let mut store = Store::new(claude_dir.to_path_buf());
    if let Err((code, msg)) = preflight(&store.config, status_file::read(&maya_dir).as_ref(), &|pid| maya_core::registry::pid_alive(pid as i32)) {
        eprintln!("{msg}");
        return code;
    }
    // Only once no other run is alive: it would be rewriting events.jsonl under that run.
    store.compact_events();
    let config_path = store.config_path();
    let _ = log::init(&maya_dir.join("maya-cli.log"));
    // A stdout gone (an SSH logout) must not panic the thread that logged.
    log::install_emitter(|l| {
        let _ = writeln!(std::io::stdout().lock(), "{}", log::file_line(l));
    });
    log::line("cli", format!("maya {} run started", env!("CARGO_PKG_VERSION")));
    match hook_install::status(claude_dir) {
        Ok(true) => {}
        _ => log::line("cli", "hooks are not installed: sessions will not be seen (run `maya hooks install`)"),
    }

    let store = Arc::new(Mutex::new(store));
    let exec = Arc::new(CliExecutor::new(store.clone(), terminal));
    let notify = Arc::new(CliNotify::new(maya_dir.clone(), stop.clone()));
    // Claims the status file at once, so a second run started before this
    // one connects sees it.
    notify.write(false, None, Some("connecting"));
    {
        let (s, due) = (store.clone(), exec.board_due.clone());
        let (sessions_dir, md) = (claude_dir.join("sessions"), maya_dir.clone());
        std::thread::spawn(move || {
            watcher::run(&sessions_dir, &md, Duration::from_secs(5), || {
                s.lock().unwrap().refresh(now_ms());
                due.store(true, Ordering::SeqCst);
            })
        });
    }
    let config_of = {
        let (s, path) = (store.clone(), config_path.clone());
        move || reload_network(&s, &path)
    };
    client::reconnect(&config_of, exec, notify.clone() as Arc<dyn ClientNotify>, &stop);

    let by_signal = stop.load(Ordering::SeqCst) && !notify.removed.load(Ordering::SeqCst);
    let code = exit_code_for(by_signal, &store.lock().unwrap().config);
    if by_signal {
        status_file::remove(&maya_dir);
    }
    if code == 2 {
        log::line("cli", "the config is no longer an assistant's; stopping");
    }
    log::line("cli", "stopped");
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preflight_refuses_the_main_role_an_unpaired_config_and_a_running_twin() {
        let mut c = Config::default();
        assert_eq!(preflight(&c, None, &|_| false), Err((2, "run `maya pair` first".into())));
        c.network.role = NetworkRole::Main;
        assert_eq!(preflight(&c, None, &|_| false), Err((2, "the CLI is assistant-only".into())));
        c.network.role = NetworkRole::Assistant;
        c.network.token = "t".into();
        c.network.assistant_id = "a".into();
        let twin = RunStatus { pid: 99, ..Default::default() };
        assert_eq!(preflight(&c, Some(&twin), &|pid| pid == 99), Err((2, "maya run is already running (pid 99)".into())));
        assert_eq!(preflight(&c, Some(&twin), &|_| false), Ok(())); // a stale file from a dead run is ignored
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
    fn exit_code_is_zero_for_a_signal_two_for_a_config_no_longer_an_assistants_else_one() {
        let mut c = Config::default();
        c.network.role = NetworkRole::Assistant;
        assert_eq!(exit_code_for(true, &c), 0);
        assert_eq!(exit_code_for(false, &c), 1);
        c.network.role = NetworkRole::Off;
        assert_eq!(exit_code_for(true, &c), 0);
        assert_eq!(exit_code_for(false, &c), 2);
        c.network.role = NetworkRole::Main;
        assert_eq!(exit_code_for(false, &c), 2);
    }
}
