//! Exercises the Linux notification, focus and speech paths by hand, the way
//! the Tauri commands drive `notify.rs`, but from a terminal so it can run
//! headless in the VM: `cargo run -p maya-core --example linux_probe`.
//! Builds one Awaiting card named "collector", sends a `notify-send`
//! banner for it, prints whether GNOME's Do Not Disturb looks active, and
//! speaks one line, waiting for it to finish. A stub on every other OS so
//! `cargo build --workspace` still compiles it there.
#[cfg(target_os = "linux")]
fn main() {
    use maya_core::model::{AwaitKind, Awaiting, Card, Harness, State};
    use maya_core::notify::{self, Utterance};

    let card = Card {
        session_id: "linux-probe".into(),
        pid: std::process::id() as i32,
        name: "collector".into(),
        cwd: "/home/ubuntu/proj/collector".into(),
        state: State::Awaiting,
        state_since: 1,
        snippet: "".into(),
        awaiting: Some(Awaiting { kind: AwaitKind::Text, detail: "Ready to merge?".into(), questions: vec![] }),
        has_inbox: false,
        harness: Harness::ClaudeCode,
        pr: None,
        context: None,
        machine: None,
        machine_address: None,
        machine_platform: None,
        terminal: None,
        stale: false,
    };

    println!("notify::notify(&card, false)");
    notify::notify(&card, false);

    println!("focus_active: {}", notify::focus_active());

    println!("speak_and_wait: speaking one line");
    match notify::speak_and_wait(Utterance::new("collector needs a decision".into(), None)) {
        Ok(()) => println!("speak_and_wait: ok"),
        Err(e) => println!("speak_and_wait: {e}"),
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    println!("linux_probe: nothing to probe on this OS");
}
