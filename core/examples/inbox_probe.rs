//! Sends one message into a running session's inbox, the way Maya replies:
//! `cargo run -p maya-core --example inbox_probe -- <pid> "<message>"`.
//! Lists the live sessions when run without arguments.
fn main() {
    let sessions = maya_core::claude_dir().join("sessions");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [pid, text] = args.as_slice() else {
        for s in maya_core::registry::list(&sessions, &maya_core::registry::pid_alive) {
            println!("{}\t{}\t{}\t{}", s.pid, s.name, s.cwd, s.messaging_socket_path.as_deref().unwrap_or("(no inbox)"));
        }
        eprintln!("usage: inbox_probe <pid> <message>");
        std::process::exit(2);
    };
    let pid: i32 = pid.parse().expect("pid");
    let s = maya_core::registry::list(&sessions, &maya_core::registry::pid_alive).into_iter().find(|s| s.pid == pid).expect("no live session with that pid");
    let socket = s.messaging_socket_path.expect("session has no inbox");
    // `--env-token`: authenticate with this process's own
    // CLAUDE_CODE_MESSAGING_TOKEN, as a hook or Bash command of that session may.
    #[cfg(windows)]
    let result = match std::env::var("MAYA_PROBE_ENV_TOKEN").ok().and(std::env::var("CLAUDE_CODE_MESSAGING_TOKEN").ok()) {
        Some(token) => maya_core::inbox::send_with_token(std::path::Path::new(&socket), pid, &token, text),
        None => maya_core::inbox::send(std::path::Path::new(&socket), pid, text),
    };
    #[cfg(not(windows))]
    let result = maya_core::inbox::send(std::path::Path::new(&socket), pid, text);
    match result {
        Ok(()) => println!("sent to {} ({})", s.name, s.cwd),
        Err(e) => {
            eprintln!("failed: {e}");
            std::process::exit(1);
        }
    }
}
