#![cfg(unix)]

use maya_cli::tmux::Tmux;
use maya_core::terminal::Terminal;
use std::time::{Duration, Instant};

fn tmux_available() -> bool {
    std::process::Command::new("tmux").arg("-V").output().map(|o| o.status.success()).unwrap_or(false)
}

/// Kills the tmux session on drop, so a failed assert above it still
/// cleans up the session instead of leaking it.
struct KillSessionGuard(String);

impl Drop for KillSessionGuard {
    fn drop(&mut self) {
        let _ = std::process::Command::new("tmux").args(["kill-session", "-t", &self.0]).output();
    }
}

#[test]
fn opens_a_session_finds_it_by_tty_and_types_into_it() {
    if !tmux_available() { eprintln!("tmux not installed; skipping"); return; }
    let label = format!("mayatest-{}", std::process::id());
    let _guard = KillSessionGuard(label.clone());
    let t = Tmux::default();
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("typed.txt");
    // A shell that records what is typed into it.
    let cmd = format!("cat > {}", maya_core::launch::shell_single_quote(&out.to_string_lossy()));
    assert_eq!(t.open(&cmd, dir.path(), &label).unwrap().as_deref(), Some(label.as_str()));
    let tty = String::from_utf8(std::process::Command::new("tmux").args(["display-message", "-p", "-t", &label, "#{pane_tty}"]).output().unwrap().stdout).unwrap().trim().to_string();
    assert_eq!(t.name_for_tty(&tty).as_deref(), Some(label.as_str()));
    assert_eq!(t.names_for_ttys(&[tty.clone(), "/dev/pts/none".into()]).get(&tty), Some(&label));
    assert!(t.focus(&tty).unwrap_err().ends_with(&format!("tmux attach -t {label}")));
    t.type_line(&tty, "hello").unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if std::fs::read_to_string(&out).unwrap_or_default().starts_with("hello\n") { break; }
        assert!(Instant::now() < deadline, "timed out waiting for the typed output to appear");
        std::thread::sleep(Duration::from_millis(50));
    }
}
