//! Maya's ear: the `maya-ear` sidecar, its JSON lines, and the microphone choice.

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;

#[derive(Debug, Clone, PartialEq)]
pub enum EarEvent {
    State { state: String, detail: String },
    Partial(String),
    Final(String),
    Level(f64),
    Devices(Vec<String>),
    Device(String),
    Note(String),
    Other,
}

pub fn parse_line(line: &str) -> Option<EarEvent> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    Some(match v["type"].as_str()? {
        "final" => EarEvent::Final(v["text"].as_str()?.to_string()),
        "partial" => EarEvent::Partial(v["text"].as_str()?.to_string()),
        "level" => EarEvent::Level(v["value"].as_f64()?),
        "state" => EarEvent::State { state: v["state"].as_str().unwrap_or("").to_string(), detail: v["detail"].as_str().unwrap_or("").to_string() },
        "devices" => EarEvent::Devices(v["names"].as_array().map(|a| a.iter().filter_map(|n| n.as_str().map(str::to_string)).collect()).unwrap_or_default()),
        "device" => EarEvent::Device(v["name"].as_str()?.to_string()),
        "note" => EarEvent::Note(v["text"].as_str()?.to_string()),
        _ => EarEvent::Other,
    })
}

/// The message the sidecar sends when macOS refuses recognition because
/// Dictation is off; the listener looks for it to tell a locked switch apart.
pub const DICTATION_OFF: &str = "Dictation is off.";

/// What to tell the user when Dictation is off: a locked switch (a management
/// profile sets `allowDictation` to false) needs their IT admin, not a click.
pub fn dictation_advice(managed_off: bool) -> &'static str {
    if managed_off {
        "Dictation is disabled by a management profile on this Mac, so on-device speech recognition cannot run. Ask your IT admin to allow Dictation (allowDictation), or switch Speech recognition to Built-in in Settings."
    } else {
        "Dictation is off. Turn it on in System Settings \u{203A} Keyboard \u{203A} Dictation, then listen again, or switch Speech recognition to Built-in in Settings."
    }
}

/// True when `defaults read` of the managed applicationaccess domain says
/// Dictation is not allowed (the output is `0`).
pub fn managed_dictation_off(defaults_output: &str) -> bool {
    defaults_output.trim() == "0"
}

/// Asks macOS whether a management profile forbids Dictation.
pub fn dictation_managed_off() -> bool {
    Command::new("defaults")
        .args(["read", "/Library/Managed Preferences/com.apple.applicationaccess", "allowDictation"])
        .output()
        .map(|o| o.status.success() && managed_dictation_off(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or(false)
}

const VIRTUAL: &[&str] = &["blackhole", "remote sound", "soundflower", "loopback"];

/// The preferred device when present, else the built-in microphone, else
/// the first input that is not a virtual or remote device.
pub fn pick_microphone(names: &[String], preferred: Option<&str>) -> Option<String> {
    if let Some(p) = preferred {
        if names.iter().any(|n| n == p) && !VIRTUAL.iter().any(|v| p.to_lowercase().contains(v)) {
            return Some(p.to_string());
        }
    }
    let ok: Vec<&String> = names.iter().filter(|n| !VIRTUAL.iter().any(|v| n.to_lowercase().contains(v))).collect();
    ok.iter().find(|n| n.to_lowercase().contains("macbook") || n.to_lowercase().contains("built-in")).or(ok.first()).map(|n| n.to_string())
}

/// The sidecar next to the running executable (a bundled app), else the dev
/// build under `<manifest>/binaries/` when a manifest dir is given.
pub fn sidecar_path_in(exe_dir: &Path, triple: &str, manifest_dir: Option<&Path>) -> Option<PathBuf> {
    let bundled = exe_dir.join("maya-ear");
    if bundled.is_file() {
        return Some(bundled);
    }
    let dev = manifest_dir?.join("binaries").join(format!("maya-ear-{triple}"));
    dev.is_file().then_some(dev)
}

pub fn sidecar_path() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let triple = format!("{}-apple-darwin", std::env::consts::ARCH);
    // Only a debug build looks in the source tree; a release build would
    // otherwise carry the build machine's path (cfg, not cfg!, so the
    // string is not compiled in at all).
    #[cfg(debug_assertions)]
    let manifest = Some(Path::new(env!("CARGO_MANIFEST_DIR")));
    #[cfg(not(debug_assertions))]
    let manifest = None;
    sidecar_path_in(&exe_dir, &triple, manifest)
}

/// Delay before restarting a listener that exited, by consecutive failure count.
pub fn restart_delay_ms(failures: u32) -> Option<u64> {
    (failures < 5).then(|| 1_000u64 << failures)
}

/// Which recogniser the sidecar runs, and the model file when it is whisper.
pub enum EngineArgs {
    System,
    Whisper { model: PathBuf },
}

pub struct Ear {
    child: Child,
    stdin: ChildStdin,
}

impl Ear {
    /// Starts the sidecar; events arrive on the returned channel until it exits.
    pub fn spawn(device: Option<&str>, engine: EngineArgs) -> Result<(Ear, mpsc::Receiver<EarEvent>), String> {
        let path = sidecar_path().ok_or("The listener (maya-ear) is not built. Run `pnpm ear:build`.")?;
        let mut cmd = Command::new(path);
        if let Some(d) = device {
            cmd.args(["--device", d]);
        }
        match engine {
            EngineArgs::System => {
                cmd.args(["--engine", "system"]);
            }
            EngineArgs::Whisper { model } => {
                cmd.args(["--engine", "whisper", "--model"]).arg(model);
            }
        }
        let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| format!("could not start the listener: {e}"))?;
        let stdin = child.stdin.take().ok_or("no stdin for the listener")?;
        let stdout = child.stdout.take().ok_or("no stdout for the listener")?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(ev) = parse_line(&line) {
                    if tx.send(ev).is_err() {
                        break;
                    }
                }
            }
            let _ = tx.send(EarEvent::State { state: "exited".into(), detail: String::new() });
        });
        Ok((Ear { child, stdin }, rx))
    }

    fn send(&mut self, cmd: &str) {
        let _ = writeln!(self.stdin, "{cmd}");
        let _ = self.stdin.flush();
    }

    pub fn pause(&mut self) {
        self.send("pause");
    }

    pub fn resume(&mut self) {
        self.send("resume");
    }

    pub fn stop(&mut self) {
        self.send("quit");
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_managed_zero_means_dictation_is_locked_off() {
        use super::*;
        assert!(managed_dictation_off("0\n"));
        assert!(!managed_dictation_off("1\n"));
        assert!(!managed_dictation_off(""));
        assert!(dictation_advice(true).contains("IT admin"));
        assert!(dictation_advice(false).contains("System Settings"));
        assert!(dictation_advice(true).ends_with("or switch Speech recognition to Built-in in Settings."));
        assert!(dictation_advice(false).ends_with("or switch Speech recognition to Built-in in Settings."));
    }

    use super::*;

    #[test]
    fn parses_each_event_kind_and_ignores_noise() {
        assert!(matches!(parse_line(r#"{"type":"final","text":"maya hello"}"#), Some(EarEvent::Final(t)) if t == "maya hello"));
        assert!(matches!(parse_line(r#"{"type":"partial","text":"ma"}"#), Some(EarEvent::Partial(t)) if t == "ma"));
        assert!(matches!(parse_line(r#"{"type":"level","value":0.25}"#), Some(EarEvent::Level(v)) if (v - 0.25).abs() < 1e-9));
        assert!(matches!(parse_line(r#"{"type":"state","state":"error","detail":"nope"}"#), Some(EarEvent::State { state, detail }) if state == "error" && detail == "nope"));
        assert!(matches!(parse_line(r#"{"type":"devices","names":["A","B"]}"#), Some(EarEvent::Devices(n)) if n == vec!["A".to_string(), "B".to_string()]));
        assert!(matches!(parse_line(r#"{"type":"device","name":"MacBook Pro Microphone"}"#), Some(EarEvent::Device(n)) if n == "MacBook Pro Microphone"));
        assert!(matches!(parse_line(r#"{"type":"note","text":"model loaded in 1.2 s"}"#), Some(EarEvent::Note(t)) if t == "model loaded in 1.2 s"));
        assert!(matches!(parse_line(r#"{"type":"selftest"}"#), Some(EarEvent::Other)));
        assert!(parse_line("not json").is_none());
        assert!(parse_line("").is_none());
    }

    #[test]
    fn microphone_preference_avoids_virtual_devices_and_prefers_built_in() {
        let names = vec!["BlackHole 2ch".to_string(), "MacBook Pro Microphone".to_string(), "Splashtop Remote Sound".to_string(), "USB Mic".to_string()];
        assert_eq!(pick_microphone(&names, None).as_deref(), Some("MacBook Pro Microphone"));
        assert_eq!(pick_microphone(&names, Some("USB Mic")).as_deref(), Some("USB Mic"));
        assert_eq!(pick_microphone(&names, Some("Gone")).as_deref(), Some("MacBook Pro Microphone"));
        assert_eq!(pick_microphone(&["BlackHole 2ch".to_string()], None), None);
        assert_eq!(pick_microphone(&["USB Mic".to_string(), "BlackHole 2ch".to_string()], None).as_deref(), Some("USB Mic"));
        assert_eq!(pick_microphone(&names, Some("BlackHole 2ch")).as_deref(), Some("MacBook Pro Microphone"), "a banned preferred device falls through");
        assert_eq!(pick_microphone(&[], None), None);
    }

    #[test]
    fn restart_backoff_grows_and_gives_up() {
        assert_eq!(restart_delay_ms(0), Some(1_000));
        assert_eq!(restart_delay_ms(1), Some(2_000));
        assert_eq!(restart_delay_ms(2), Some(4_000));
        assert_eq!(restart_delay_ms(4), Some(16_000));
        assert_eq!(restart_delay_ms(5), None, "after five crashes the indicator shows error and stays off");
    }

    #[test]
    fn sidecar_path_prefers_the_binary_next_to_the_executable() {
        let t = tempfile::tempdir().unwrap();
        let exe_dir = t.path().join("Contents/MacOS");
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::write(exe_dir.join("maya-ear"), "").unwrap();
        assert_eq!(sidecar_path_in(&exe_dir, "aarch64-apple-darwin", Some(t.path())), Some(exe_dir.join("maya-ear")));
        std::fs::remove_file(exe_dir.join("maya-ear")).unwrap();
        let dev = t.path().join("binaries");
        std::fs::create_dir_all(&dev).unwrap();
        std::fs::write(dev.join("maya-ear-aarch64-apple-darwin"), "").unwrap();
        assert_eq!(sidecar_path_in(&exe_dir, "aarch64-apple-darwin", Some(t.path())), Some(dev.join("maya-ear-aarch64-apple-darwin")));
        assert_eq!(sidecar_path_in(&exe_dir, "aarch64-apple-darwin", None), None, "a release build never looks in the build machine's tree");
        std::fs::remove_file(dev.join("maya-ear-aarch64-apple-darwin")).unwrap();
        assert_eq!(sidecar_path_in(&exe_dir, "aarch64-apple-darwin", Some(t.path())), None);
    }
}
