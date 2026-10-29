//! Asks `kiro-cli acp` for its usage card and prints every line it sends:
//! `cargo run -p maya-core --example kiro_usage_probe [-- <method> <command-json>...]`.
//! The probe sends `initialize`, `session/new` (home, no MCP servers), then
//! one `<method>` request per `<command-json>`, each as
//! `{"sessionId": .., "command": <command-json>}`, and waits up to 20 s.
//! The defaults are `_kiro.dev/commands/execute` and
//! `{"command":"usage","args":{}}`, the shape Kiro 2.34 answers (a bare
//! `"usage"` is refused: its commands are an adjacently tagged enum).
//! `-- --maya` instead runs what Maya's collector runs
//! (`kiro::acp_usage`, then `kiro::parse_usage_result`) and prints both.
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use maya_core::brain::{acp, jsonrpc::Envelope};
use serde_json::{json, Value};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--maya") {
        let result = maya_core::kiro::acp_usage(Duration::from_secs(15));
        println!("{result:?}");
        println!("{:?}", result.ok().and_then(|r| maya_core::kiro::parse_usage_result(&r, maya_core::now_ms())));
        return;
    }
    let method = args.first().cloned().unwrap_or_else(|| "_kiro.dev/commands/execute".into());
    let commands: Vec<Value> = match args.get(1..) {
        Some(c) if !c.is_empty() => c.iter().map(|c| serde_json::from_str(c).expect("command JSON")).collect(),
        _ => vec![json!({"command": "usage", "args": {}})],
    };
    let home = dirs::home_dir().expect("home directory");
    let mut child = Command::new("kiro-cli").arg("acp").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().expect("spawn kiro-cli acp");
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let start = Instant::now();
    let send = |stdin: &mut std::process::ChildStdin, line: String| {
        print!(">> {line}");
        stdin.write_all(line.as_bytes()).unwrap();
        stdin.flush().unwrap();
    };
    send(&mut stdin, acp::initialize(1));
    send(&mut stdin, Envelope::Versioned.request(2, "session/new", json!({"cwd": home.to_string_lossy(), "mcpServers": []})));
    let deadline = start + Duration::from_secs(20);
    let mut answered_at: Option<Instant> = None;
    let mut session: Option<String> = None;
    while Instant::now() < deadline && answered_at.is_none_or(|t| t.elapsed() < Duration::from_secs(3)) {
        let Ok(line) = rx.recv_timeout(Duration::from_millis(200)) else { continue };
        println!("<< [{:>5} ms] {line}", start.elapsed().as_millis());
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        if v["id"] == 2 {
            let Some(sid) = v["result"]["sessionId"].as_str() else {
                println!("!! session/new failed");
                break;
            };
            session = Some(sid.to_string());
            for (i, command) in commands.iter().enumerate() {
                send(&mut stdin, Envelope::Versioned.request(3 + i as u64, &method, json!({"sessionId": sid, "command": command})));
            }
        } else if v["id"] == 2 + commands.len() as u64 && v.get("method").is_none() {
            answered_at = Some(Instant::now());
        }
    }
    // Let Kiro's agent save and exit on stdin's close, then forget the session.
    drop(stdin);
    let grace = Instant::now() + Duration::from_secs(3);
    while rx.recv_timeout(grace.saturating_duration_since(Instant::now())).is_ok() {}
    let _ = child.kill();
    let _ = child.wait();
    if let Some(id) = session {
        maya_core::kiro::forget_probe_session(&home.join(".kiro/sessions/cli"), &id);
    }
}
