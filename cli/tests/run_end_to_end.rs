#![cfg(unix)]

//! `maya run` against a real main on localhost: pair, connect, show the
//! board, run a command through the terminal, type a reply into the
//! session's terminal, stop cleanly on a signal.

use maya_cli::{commands, run_cmd, status_file};
use maya_core::config::{NetworkConfig, PairedAssistant};
use maya_core::model::Card;
use maya_core::net::protocol::CommandKind;
use maya_core::net::server::{send_command_with, start_with, Notify};
use maya_core::net::NetworkStatus;
use maya_core::terminal::{Call, FakeTerminal};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Records what the main's server tells the app.
#[derive(Default)]
struct Recorder {
    paired: AtomicUsize,
    boards: AtomicUsize,
}

impl Notify for Recorder {
    fn board_changed(&self) {
        self.boards.fetch_add(1, Ordering::SeqCst);
    }
    fn board_seeded(&self, _: &[Card]) {}
    fn status_changed(&self, _: NetworkStatus) {}
    fn paired(&self, _: &PairedAssistant) -> Result<(), String> {
        self.paired.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn paired_list_changed(&self, _: &[PairedAssistant]) -> Result<(), String> {
        Ok(())
    }
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while !f() {
        assert!(Instant::now() < end, "{what}: not within 5 s");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn run_pairs_connects_runs_a_command_and_stops_on_a_signal() {
    let recorder = Arc::new(Recorder::default());
    let handle = start_with(recorder.clone(), Arc::new(Mutex::new(NetworkConfig { name: "Yhi".into(), ..Default::default() })), 0).unwrap();
    let port = handle.port();

    // A registry session owned by this (live) process, on /dev/pts/3.
    let pid = std::process::id() as i32;
    let (dir, store) = maya_core::actions::test_support::store_with_session("s1", pid);
    drop(store);
    let claude_dir = dir.path().to_path_buf();
    let maya_dir = claude_dir.join("maya");

    // 1. Pair.
    let (code, _) = handle.open_pairing(maya_core::now_ms());
    // The label is trimmed before it is sent and stored.
    assert_eq!(commands::pair(&claude_dir, "127.0.0.1", port, Some(" box "), &code), Ok(("Yhi".to_string(), "box".to_string())));
    assert_eq!(recorder.paired.load(Ordering::SeqCst), 1);
    let saved = maya_core::config::load(&maya_dir.join("config.json"));
    assert!(!saved.network.token.is_empty(), "the config on disk holds the token");
    assert_eq!(saved.network.name, "box");

    // 2. Run, with a fake terminal and a stop flag standing in for a signal.
    let fake = Arc::new(FakeTerminal::default());
    let stop = Arc::new(AtomicBool::new(false));
    let runner = {
        let (claude_dir, fake, stop) = (claude_dir.clone(), fake.clone(), stop.clone());
        std::thread::spawn(move || run_cmd::run_with(&claude_dir, fake, stop))
    };

    // 3. The main sees the assistant connected, with the session on its board.
    wait_until("connected", || handle.status().assistants.iter().any(|a| a.name == "box" && a.connected));
    wait_until("board", || handle.boards().iter().any(|b| b.machine == "box" && b.cards.iter().any(|c| c.session_id == "s1")));
    assert!(recorder.boards.load(Ordering::SeqCst) >= 1);
    let status = status_file::read(&maya_dir).expect("a status file while running");
    assert!(status.connected);
    assert_eq!(status.main_name.as_deref(), Some("Yhi"));

    // A second run, even in this same process, is refused while the first
    // holds the lock, and leaves the first's status file alone.
    assert_eq!(run_cmd::run_with(&claude_dir, Arc::new(FakeTerminal::default()), Arc::new(AtomicBool::new(false))), 2);
    assert_eq!(status_file::read(&maya_dir).map(|s| s.connected), Some(true));
    // Nor can pairing swap the credentials under it.
    assert_eq!(commands::pair(&claude_dir, "127.0.0.1", port, Some("box"), "123456"), Err((2, format!("stop `maya run` first (pid {})", std::process::id()))));

    // 4. A command from the main is typed through the terminal.
    assert_eq!(send_command_with(&handle.shared, "box", CommandKind::Compact { session: "s1".into() }, Duration::from_secs(5)), Ok(None));
    assert!(fake.calls.lock().unwrap().iter().any(|c| matches!(c, Call::Type { tty, text } if tty == "/dev/pts/3" && text == "/compact")));

    // 5. A reply from the main is typed into the session's terminal, as the user.
    let reply = CommandKind::Reply { session: "s1".into(), text: "hello".into(), attachments: vec![] };
    assert_eq!(send_command_with(&handle.shared, "box", reply, Duration::from_secs(5)), Ok(Some(serde_json::json!({"route": "typed"}))));
    assert!(fake.calls.lock().unwrap().iter().any(|c| matches!(c, Call::Type { tty, text } if tty == "/dev/pts/3" && text == "hello")));

    // 6. A signal ends the run with 0 and removes the status file.
    stop.store(true, Ordering::SeqCst);
    let end = Instant::now() + Duration::from_secs(5);
    while !runner.is_finished() {
        assert!(Instant::now() < end, "run did not stop within 5 s");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(runner.join().unwrap(), 0);
    assert_eq!(status_file::read(&maya_dir), None);
    wait_until("disconnected", || handle.status().assistants.iter().all(|a| !a.connected));
    handle.stop();
}

#[test]
fn run_stops_with_two_when_the_config_on_disk_stops_being_an_assistants() {
    let handle = start_with(Arc::new(Recorder::default()), Arc::new(Mutex::new(NetworkConfig { name: "Yhi".into(), ..Default::default() })), 0).unwrap();
    let (dir, store) = maya_core::actions::test_support::store_with_session("s1", std::process::id() as i32);
    drop(store);
    let claude_dir = dir.path().to_path_buf();
    let (code, _) = handle.open_pairing(maya_core::now_ms());
    commands::pair(&claude_dir, "127.0.0.1", handle.port(), Some("box"), &code).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let runner = {
        let claude_dir = claude_dir.clone();
        std::thread::spawn(move || run_cmd::run_with(&claude_dir, Arc::new(FakeTerminal::default()), stop))
    };
    wait_until("connected", || handle.status().assistants.iter().any(|a| a.name == "box" && a.connected));

    // Edited to role off while connected: the watcher sees it and the run
    // closes the link and exits 2, without waiting for a reconnect.
    let config_path = claude_dir.join("maya/config.json");
    let mut c = maya_core::config::load(&config_path);
    c.network.role = maya_core::config::NetworkRole::Off;
    maya_core::config::save(&config_path, &c).unwrap();
    wait_until("stopped", || runner.is_finished());
    assert_eq!(runner.join().unwrap(), 2);
    wait_until("disconnected", || handle.status().assistants.iter().all(|a| !a.connected));
    handle.stop();
}
