# Maya Voice Assistant Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Say "Maya, …" and have her answer aloud and act on the board, with listening kept on the Mac.

**Architecture:** A Swift sidecar (`maya-ear`) does on-device speech recognition and streams JSON lines to Maya. Rust modules turn segments into wake-word commands (`wake.rs`), interpret a command with a one-shot `claude -p` call that returns one validated action (`interpreter.rs`), and run the sidecar and the confirmation flow (`ear.rs`, `lib.rs`). The page shows an indicator and a small panel (`src/voice.ts`).

**Tech Stack:** Swift (Speech, AVFoundation) for the sidecar; Rust with serde_json for parsing; Claude Code print mode with Haiku as the interpreter; the existing voice path (`say` or ElevenLabs) for replies; vitest for the page.

**Spec:** `docs/superpowers/specs/2026-09-29-maya-voice-assistant-design.md`

## Global Constraints

- Wake word is "Maya"; the recogniser may spell it "maya", "maia", "mya" or "my a". All must wake her.
- Audio never leaves the Mac. Only the command text, the board summary and the last six exchanges go to the model.
- Listening is off by default; the Settings toggle is labelled "Listen for 'Maya'".
- The listener must never pick a device whose name contains "BlackHole" or "Remote Sound"; the built-in microphone is preferred.
- Sends, option answers, starts and resumes require a spoken "yes" within ten seconds; report, focus and compact run at once.
- The interpreter is `claude -p` with `--model haiku` by default, `--strict-mcp-config --disable-slash-commands --no-session-persistence --max-turns 1 --output-format json`.
- Listening pauses while Maya speaks.
- A reply to the user is spoken even under a Focus mode; announcements stay silent under Focus as today.

## Review Focus

- A `final` segment that contains "Maya" in the middle of unrelated talk ("I told Maya yesterday …") must not run a command that damages anything: the interpreter may return `report` or `null`, and every acting command is confirmed first. Pinned in Task 3 (extraction) and Task 4 (validation).
- The interpreter returning malformed JSON, or JSON naming a session that is not on the board, must produce a spoken "I couldn't …" and no action. Pinned in Task 4.
- Two wake words in quick succession, or a confirmation that arrives after the ten-second window, must not confirm a stale action. Pinned in Task 3.
- The sidecar dying (crash, permission denied) must set the indicator to error and not spin the CPU restarting it. Pinned in Task 5 (restart backoff test).
- Maya hearing her own reply must not wake her: listening is paused around speech, and a segment that equals her last spoken line is ignored. Pinned in Task 5.

---

### Task 1: The listener sidecar (`maya-ear`)

**Files:**
- Create: `ear/main.swift`
- Create: `scripts/build-ear.sh`
- Modify: `package.json` (script `ear:build`)
- Modify: `.gitignore` (ignore `src-tauri/binaries/`)
- Modify: `src-tauri/tauri.conf.json` (`bundle.externalBin`)

**Interfaces:**
- Produces: a binary that prints one JSON object per line to stdout:
  - `{"type":"state","state":"listening"|"paused"|"error","detail":"…"}`
  - `{"type":"partial","text":"…"}` and `{"type":"final","text":"…"}`
  - `{"type":"level","value":0.0-1.0}` a few times a second
  - `{"type":"devices","names":["…"]}` once at start
- Consumes on stdin, one command per line: `pause`, `resume`, `quit`.
- Flags: `--device <name>` to force a microphone; `--selftest` prints `{"type":"selftest","available":bool,"onDevice":bool,"speechAuth":int,"devices":[…]}` and exits.

- [ ] **Step 1: Write the Swift source**

```swift
// ear/main.swift — Maya's ear: on-device speech recognition streamed as JSON lines.
import AVFoundation
import Foundation
import Speech

struct Out {
    static let lock = NSLock()
    static func emit(_ obj: [String: Any]) {
        lock.lock(); defer { lock.unlock() }
        if let d = try? JSONSerialization.data(withJSONObject: obj), let s = String(data: d, encoding: .utf8) {
            print(s); fflush(stdout)
        }
    }
}

func inputDevices() -> [AVCaptureDevice] {
    AVCaptureDevice.DiscoverySession(deviceTypes: [.microphone, .external], mediaType: .audio, position: .unspecified).devices
}

/// Built-in first, then any input that is not a virtual or remote device.
func pickDevice(_ names: [String], preferred: String?) -> String? {
    if let p = preferred, names.contains(p) { return p }
    let banned = ["blackhole", "remote sound", "soundflower", "loopback"]
    let ok = names.filter { n in !banned.contains { n.lowercased().contains($0) } }
    if let b = ok.first(where: { $0.lowercased().contains("macbook") || $0.lowercased().contains("built-in") }) { return b }
    return ok.first
}

let args = CommandLine.arguments
let names = inputDevices().map { $0.localizedName }
if args.contains("--selftest") {
    let r = SFSpeechRecognizer(locale: Locale(identifier: "en-GB"))
    Out.emit(["type": "selftest", "available": r?.isAvailable ?? false, "onDevice": r?.supportsOnDeviceRecognition ?? false,
              "speechAuth": SFSpeechRecognizer.authorizationStatus().rawValue, "devices": names])
    exit(0)
}
var preferred: String? = nil
if let i = args.firstIndex(of: "--device"), i + 1 < args.count { preferred = args[i + 1] }
Out.emit(["type": "devices", "names": names])

final class Ear {
    let engine = AVAudioEngine()
    let recognizer = SFSpeechRecognizer(locale: Locale(identifier: "en-GB"))!
    var request: SFSpeechAudioBufferRecognitionRequest?
    var task: SFSpeechRecognitionTask?
    var paused = false
    var lastPartial = ""
    var restartTimer: Timer?

    func start() {
        SFSpeechRecognizer.requestAuthorization { status in
            guard status == .authorized else {
                Out.emit(["type": "state", "state": "error", "detail": "speech recognition not authorised (\(status.rawValue))"]); exit(2)
            }
            DispatchQueue.main.async { self.startAudio() }
        }
    }

    func startAudio() {
        if let name = pickDevice(names, preferred: preferred), let dev = inputDevices().first(where: { $0.localizedName == name }) {
            var id = AudioDeviceID(0)
            if let uid = dev.uniqueID as CFString? {
                var addr = AudioObjectPropertyAddress(mSelector: kAudioHardwarePropertyTranslateUIDToDevice, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
                var uidRef: CFString = uid
                var size = UInt32(MemoryLayout<AudioDeviceID>.size)
                _ = withUnsafeMutablePointer(to: &uidRef) { p in
                    AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &addr, UInt32(MemoryLayout<CFString>.size), p, &size, &id)
                }
            }
            if id != 0, let unit = engine.inputNode.audioUnit {
                var devId = id
                AudioUnitSetProperty(unit, kAudioOutputUnitProperty_CurrentDevice, kAudioUnitScope_Global, 0, &devId, UInt32(MemoryLayout<AudioDeviceID>.size))
            }
            Out.emit(["type": "state", "state": "device", "detail": name])
        }
        let input = engine.inputNode
        let format = input.outputFormat(forBus: 0)
        input.installTap(onBus: 0, bufferSize: 2048, format: format) { buffer, _ in
            guard !self.paused else { return }
            self.request?.append(buffer)
            if let ch = buffer.floatChannelData?[0] {
                let n = Int(buffer.frameLength)
                var sum: Float = 0
                for i in 0..<n { sum += ch[i] * ch[i] }
                let rms = n > 0 ? sqrt(sum / Float(n)) : 0
                Out.emit(["type": "level", "value": min(1.0, Double(rms) * 8)])
            }
        }
        engine.prepare()
        do { try engine.start() } catch {
            Out.emit(["type": "state", "state": "error", "detail": "audio engine: \(error.localizedDescription)"]); exit(3)
        }
        beginRequest()
        // On-device requests are capped around a minute: rotate before the cap.
        restartTimer = Timer.scheduledTimer(withTimeInterval: 50, repeats: true) { _ in self.rotate() }
        Out.emit(["type": "state", "state": "listening"])
    }

    func beginRequest() {
        let req = SFSpeechAudioBufferRecognitionRequest()
        req.shouldReportPartialResults = true
        req.requiresOnDeviceRecognition = true
        request = req
        lastPartial = ""
        task = recognizer.recognitionTask(with: req) { result, error in
            if let r = result {
                let text = r.bestTranscription.formattedString
                if r.isFinal {
                    if !text.isEmpty { Out.emit(["type": "final", "text": text]) }
                    self.lastPartial = ""
                } else if text != self.lastPartial {
                    self.lastPartial = text
                    Out.emit(["type": "partial", "text": text])
                }
            }
            if error != nil && self.task != nil {
                // The request ended (silence timeout or rotation): flush the partial as final and start again.
                if !self.lastPartial.isEmpty { Out.emit(["type": "final", "text": self.lastPartial]) }
                DispatchQueue.main.async { self.beginRequest() }
            }
        }
    }

    func rotate() {
        if !lastPartial.isEmpty { Out.emit(["type": "final", "text": lastPartial]) }
        request?.endAudio()
        task?.cancel()
        task = nil
        beginRequest()
    }

    func setPaused(_ p: Bool) {
        paused = p
        Out.emit(["type": "state", "state": p ? "paused" : "listening"])
    }
}

let ear = Ear()
ear.start()

DispatchQueue.global().async {
    while let line = readLine() {
        switch line.trimmingCharacters(in: .whitespaces) {
        case "pause": DispatchQueue.main.async { ear.setPaused(true) }
        case "resume": DispatchQueue.main.async { ear.setPaused(false) }
        case "quit": exit(0)
        default: break
        }
    }
    exit(0)
}
RunLoop.main.run()
```

- [ ] **Step 2: Write the build script and hook it into package.json**

`scripts/build-ear.sh`:

```sh
#!/bin/sh
# Builds Maya's ear for the given target triple (default: this machine).
set -eu
cd "$(dirname "$0")/.."
target="${1:-$(rustc -vV | sed -n 's/^host: //p')}"
case "$target" in
  aarch64-apple-darwin) arch=arm64 ;;
  x86_64-apple-darwin) arch=x86_64 ;;
  *) echo "unsupported target $target" >&2; exit 1 ;;
esac
mkdir -p src-tauri/binaries
swiftc -O -target "$arch-apple-macos13.0" -framework Speech -framework AVFoundation -framework CoreAudio \
  -o "src-tauri/binaries/maya-ear-$target" ear/main.swift
echo "built src-tauri/binaries/maya-ear-$target"
```

In `package.json` scripts add `"ear:build": "sh scripts/build-ear.sh"`. In `.gitignore` add `src-tauri/binaries/`. In `src-tauri/tauri.conf.json` under `bundle` add `"externalBin": ["binaries/maya-ear"]`.

- [ ] **Step 3: Build and self-test**

Run: `chmod +x scripts/build-ear.sh && pnpm ear:build && src-tauri/binaries/maya-ear-$(rustc -vV | sed -n 's/^host: //p') --selftest`
Expected: one JSON line with `"available":true,"onDevice":true` and the device names.

- [ ] **Step 4: Listen once, by hand**

Run: `src-tauri/binaries/maya-ear-aarch64-apple-darwin` then speak "Maya, hello" and press Ctrl-C.
Expected: a `devices` line, a `state: device` line naming the MacBook microphone, `state: listening`, then `partial` and `final` lines containing the words. The first run shows macOS's speech recognition permission dialog; approve it.

- [ ] **Step 5: Commit**

```bash
git add ear/main.swift scripts/build-ear.sh package.json .gitignore src-tauri/tauri.conf.json
git commit -m "feat: maya-ear listener sidecar with on-device speech recognition"
```

---

### Task 2: Sidecar events and microphone preference (`ear.rs`)

**Files:**
- Create: `src-tauri/src/ear.rs`
- Modify: `src-tauri/src/lib.rs` (add `pub mod ear;`)

**Interfaces:**
- Produces:
  - `pub enum EarEvent { State { state: String, detail: String }, Partial(String), Final(String), Level(f64), Devices(Vec<String>), Other }`
  - `pub fn parse_line(line: &str) -> Option<EarEvent>`
  - `pub fn pick_microphone(names: &[String], preferred: Option<&str>) -> Option<String>`
  - `pub fn sidecar_path() -> Option<PathBuf>` (next to the running executable, else `src-tauri/binaries/maya-ear-<host triple>` in dev)
  - `pub struct Ear { child, stdin }` with `pub fn spawn(device: Option<&str>) -> Result<(Ear, mpsc::Receiver<EarEvent>), String>`, `pub fn pause(&mut self)`, `pub fn resume(&mut self)`, `pub fn stop(&mut self)`

- [ ] **Step 1: Write the failing tests**

Append to `src-tauri/src/ear.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_event_kind_and_ignores_noise() {
        assert!(matches!(parse_line(r#"{"type":"final","text":"maya hello"}"#), Some(EarEvent::Final(t)) if t == "maya hello"));
        assert!(matches!(parse_line(r#"{"type":"partial","text":"ma"}"#), Some(EarEvent::Partial(t)) if t == "ma"));
        assert!(matches!(parse_line(r#"{"type":"level","value":0.25}"#), Some(EarEvent::Level(v)) if (v - 0.25).abs() < 1e-9));
        assert!(matches!(parse_line(r#"{"type":"state","state":"error","detail":"nope"}"#), Some(EarEvent::State { state, detail }) if state == "error" && detail == "nope"));
        assert!(matches!(parse_line(r#"{"type":"devices","names":["A","B"]}"#), Some(EarEvent::Devices(n)) if n == vec!["A".to_string(), "B".to_string()]));
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
    }

    #[test]
    fn sidecar_path_prefers_the_binary_next_to_the_executable() {
        let t = tempfile::tempdir().unwrap();
        let exe_dir = t.path().join("Contents/MacOS");
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::write(exe_dir.join("maya-ear"), "").unwrap();
        assert_eq!(sidecar_path_in(&exe_dir, "aarch64-apple-darwin", t.path()), Some(exe_dir.join("maya-ear")));
        std::fs::remove_file(exe_dir.join("maya-ear")).unwrap();
        let dev = t.path().join("binaries");
        std::fs::create_dir_all(&dev).unwrap();
        std::fs::write(dev.join("maya-ear-aarch64-apple-darwin"), "").unwrap();
        assert_eq!(sidecar_path_in(&exe_dir, "aarch64-apple-darwin", t.path()), Some(dev.join("maya-ear-aarch64-apple-darwin")));
        std::fs::remove_file(dev.join("maya-ear-aarch64-apple-darwin")).unwrap();
        assert_eq!(sidecar_path_in(&exe_dir, "aarch64-apple-darwin", t.path()), None);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test ear::` (after adding `pub mod ear;` to `lib.rs`)
Expected: compile errors naming `parse_line`, `pick_microphone`, `sidecar_path_in`, `EarEvent`.

- [ ] **Step 3: Write the implementation**

Top of `src-tauri/src/ear.rs`:

```rust
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
        _ => EarEvent::Other,
    })
}

const VIRTUAL: &[&str] = &["blackhole", "remote sound", "soundflower", "loopback"];

/// The preferred device when present, else the built-in microphone, else
/// the first input that is not a virtual or remote device.
pub fn pick_microphone(names: &[String], preferred: Option<&str>) -> Option<String> {
    if let Some(p) = preferred {
        if names.iter().any(|n| n == p) {
            return Some(p.to_string());
        }
    }
    let ok: Vec<&String> = names.iter().filter(|n| !VIRTUAL.iter().any(|v| n.to_lowercase().contains(v))).collect();
    ok.iter().find(|n| n.to_lowercase().contains("macbook") || n.to_lowercase().contains("built-in")).or(ok.first()).map(|n| n.to_string())
}

/// The sidecar next to the running executable (a bundled app), else the dev
/// build under `<manifest>/binaries/`.
pub fn sidecar_path_in(exe_dir: &Path, triple: &str, manifest_dir: &Path) -> Option<PathBuf> {
    let bundled = exe_dir.join("maya-ear");
    if bundled.is_file() {
        return Some(bundled);
    }
    let dev = manifest_dir.join("binaries").join(format!("maya-ear-{triple}"));
    dev.is_file().then_some(dev)
}

pub fn sidecar_path() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let triple = format!("{}-apple-darwin", std::env::consts::ARCH);
    sidecar_path_in(&exe_dir, &triple, Path::new(env!("CARGO_MANIFEST_DIR")))
}

pub struct Ear {
    child: Child,
    stdin: ChildStdin,
}

impl Ear {
    /// Starts the sidecar; events arrive on the returned channel until it exits.
    pub fn spawn(device: Option<&str>) -> Result<(Ear, mpsc::Receiver<EarEvent>), String> {
        let path = sidecar_path().ok_or("The listener (maya-ear) is not built. Run `pnpm ear:build`.")?;
        let mut cmd = Command::new(path);
        if let Some(d) = device {
            cmd.args(["--device", d]);
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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test ear::`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/ear.rs src-tauri/src/lib.rs
git commit -m "feat: ear module: sidecar events, microphone choice and process control"
```

---

### Task 3: Wake word and the conversation flow (`wake.rs`)

**Files:**
- Create: `src-tauri/src/wake.rs`
- Modify: `src-tauri/src/lib.rs` (add `pub mod wake;`)

**Interfaces:**
- Produces:
  - `pub enum Wake { None, Bare, Command(String) }` and `pub fn extract(segment: &str) -> Wake`
  - `pub enum Answer { Yes, No, Other }` and `pub fn answer(segment: &str) -> Answer`
  - `pub struct Pending { pub say: String, pub action: serde_json::Value }`
  - `pub struct Flow { … }` with `pub fn new() -> Flow`, `pub fn on_segment(&mut self, text: &str, now_ms: u64) -> Vec<Effect>`, `pub fn set_pending(&mut self, p: Pending, now_ms: u64)`, `pub fn ignore_line(&mut self, spoken: &str)`, `pub fn state(&self, now_ms: u64) -> &'static str` ("idle" | "awaiting-command" | "awaiting-confirm")
  - `pub enum Effect { Say(String), Interpret(String), Execute(serde_json::Value), Cancelled }`
  - constants `COMMAND_WAIT_MS = 8_000`, `CONFIRM_WAIT_MS = 10_000`

- [ ] **Step 1: Write the failing tests**

Append to `src-tauri/src/wake.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_the_command_after_any_spelling_of_the_wake_word() {
        assert_eq!(extract("Maya, what's waiting on me?"), Wake::Command("what's waiting on me?".into()));
        assert_eq!(extract("hey maia tell hexgrid to go ahead"), Wake::Command("tell hexgrid to go ahead".into()));
        assert_eq!(extract("Mya compact the coral one"), Wake::Command("compact the coral one".into()));
        assert_eq!(extract("my a, focus on matters"), Wake::Command("focus on matters".into()));
        assert_eq!(extract("Maya"), Wake::Bare);
        assert_eq!(extract("Maya."), Wake::Bare);
        assert_eq!(extract("so I told her about it"), Wake::None);
        // The wake word inside a longer sentence still wakes her; the command is what follows.
        assert_eq!(extract("I told Maya yesterday about the deploy"), Wake::Command("yesterday about the deploy".into()));
    }

    #[test]
    fn yes_and_no_are_recognised_loosely() {
        for s in ["yes", "Yes.", "yeah go ahead", "do it", "go ahead", "confirm", "yep"] {
            assert_eq!(answer(s), Answer::Yes, "{s}");
        }
        for s in ["no", "No.", "cancel", "stop", "don't", "never mind"] {
            assert_eq!(answer(s), Answer::No, "{s}");
        }
        assert_eq!(answer("actually tell coral instead"), Answer::Other);
    }

    fn pending() -> Pending {
        Pending { say: "Telling hexgrid: go ahead. Yes?".into(), action: serde_json::json!({"kind": "reply", "session": "hexgrid", "text": "go ahead"}) }
    }

    #[test]
    fn bare_wake_word_waits_for_the_next_segment_then_times_out() {
        let mut f = Flow::new();
        assert_eq!(f.on_segment("Maya", 1000), vec![Effect::Say("Yes?".into())]);
        assert_eq!(f.state(1500), "awaiting-command");
        assert_eq!(f.on_segment("what's waiting", 2000), vec![Effect::Interpret("what's waiting".into())]);
        assert_eq!(f.state(2001), "idle");
        f.on_segment("Maya.", 5000);
        assert_eq!(f.state(5000 + COMMAND_WAIT_MS + 1), "idle");
        assert_eq!(f.on_segment("late words", 5000 + COMMAND_WAIT_MS + 1), vec![]);
    }

    #[test]
    fn a_pending_action_is_confirmed_cancelled_or_replaced_and_expires() {
        let mut f = Flow::new();
        f.set_pending(pending(), 1000);
        assert_eq!(f.state(1500), "awaiting-confirm");
        assert_eq!(f.on_segment("yes", 2000), vec![Effect::Execute(pending().action)]);
        assert_eq!(f.state(2001), "idle");
        f.set_pending(pending(), 3000);
        assert_eq!(f.on_segment("no", 3500), vec![Effect::Cancelled]);
        f.set_pending(pending(), 4000);
        // Anything else is a new command and drops the pending one.
        assert_eq!(f.on_segment("Maya tell coral yes instead", 4500), vec![Effect::Interpret("tell coral yes instead".into())]);
        f.set_pending(pending(), 6000);
        assert_eq!(f.on_segment("yes", 6000 + CONFIRM_WAIT_MS + 1), vec![], "a late yes confirms nothing");
        assert_eq!(f.state(6000 + CONFIRM_WAIT_MS + 1), "idle");
    }

    #[test]
    fn her_own_last_line_is_ignored() {
        let mut f = Flow::new();
        f.ignore_line("Maya here. hexgrid needs a decision");
        assert_eq!(f.on_segment("Maya here hexgrid needs a decision", 1000), vec![]);
        assert_eq!(f.on_segment("Maya what's up", 2000), vec![Effect::Interpret("what's up".into())]);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test wake::`
Expected: compile errors for the missing items.

- [ ] **Step 3: Write the implementation**

Top of `src-tauri/src/wake.rs`:

```rust
//! The wake word and the short conversation around a command: "Maya" then
//! a command, or a command in the same breath; a pending action that waits
//! for yes or no. Pure functions and a small state machine, no I/O.

pub const COMMAND_WAIT_MS: u64 = 8_000;
pub const CONFIRM_WAIT_MS: u64 = 10_000;

const WAKE_WORDS: &[&str] = &["maya", "maia", "mya", "my a"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wake {
    None,
    Bare,
    Command(String),
}

fn normalise(s: &str) -> String {
    s.to_lowercase().chars().map(|c| if c.is_alphanumeric() || c == '\'' || c == ' ' { c } else { ' ' }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The command after the wake word, if the segment contains one.
pub fn extract(segment: &str) -> Wake {
    let norm = normalise(segment);
    let padded = format!(" {norm} ");
    for w in WAKE_WORDS {
        if let Some(i) = padded.find(&format!(" {w} ")) {
            let after = padded[i + w.len() + 2..].trim();
            // Keep the original casing of the command: take the same number of words from the source.
            let words_after = after.split_whitespace().count();
            if words_after == 0 {
                return Wake::Bare;
            }
            let original: Vec<&str> = segment.split_whitespace().collect();
            let tail = original[original.len().saturating_sub(words_after)..].join(" ");
            let tail = tail.trim_start_matches(|c: char| c == ',' || c == '.' || c == ':').trim().to_string();
            return Wake::Command(tail);
        }
    }
    Wake::None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Yes,
    No,
    Other,
}

pub fn answer(segment: &str) -> Answer {
    let n = normalise(segment);
    let words: Vec<&str> = n.split_whitespace().collect();
    let text = words.join(" ");
    let yes = ["yes", "yeah", "yep", "yup", "do it", "go ahead", "confirm", "sure", "ok", "okay"];
    let no = ["no", "nope", "cancel", "stop", "don't", "never mind", "nevermind", "abort"];
    if words.len() <= 4 {
        if yes.iter().any(|y| text == *y || text.starts_with(&format!("{y} ")) || text.ends_with(&format!(" {y}")) || text.contains(&format!(" {y} "))) {
            return Answer::Yes;
        }
        if no.iter().any(|y| text == *y || text.starts_with(&format!("{y} ")) || text.ends_with(&format!(" {y}")) || text.contains(&format!(" {y} "))) {
            return Answer::No;
        }
    }
    Answer::Other
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    pub say: String,
    pub action: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Say(String),
    Interpret(String),
    Execute(serde_json::Value),
    Cancelled,
}

enum State {
    Idle,
    AwaitingCommand { until: u64 },
    AwaitingConfirm { pending: Pending, until: u64 },
}

pub struct Flow {
    state: State,
    ignore: Option<String>,
}

impl Flow {
    pub fn new() -> Flow {
        Flow { state: State::Idle, ignore: None }
    }

    /// Remembers Maya's last spoken line so hearing it back does nothing.
    pub fn ignore_line(&mut self, spoken: &str) {
        self.ignore = Some(normalise(spoken));
    }

    pub fn set_pending(&mut self, p: Pending, now_ms: u64) {
        self.state = State::AwaitingConfirm { pending: p, until: now_ms + CONFIRM_WAIT_MS };
    }

    pub fn state(&self, now_ms: u64) -> &'static str {
        match &self.state {
            State::Idle => "idle",
            State::AwaitingCommand { until } if now_ms <= *until => "awaiting-command",
            State::AwaitingConfirm { until, .. } if now_ms <= *until => "awaiting-confirm",
            _ => "idle",
        }
    }

    fn expire(&mut self, now_ms: u64) {
        if self.state(now_ms) == "idle" {
            self.state = State::Idle;
        }
    }

    pub fn on_segment(&mut self, text: &str, now_ms: u64) -> Vec<Effect> {
        self.expire(now_ms);
        if self.ignore.as_deref() == Some(normalise(text).as_str()) {
            return vec![];
        }
        match std::mem::replace(&mut self.state, State::Idle) {
            State::AwaitingConfirm { pending, until } => match answer(text) {
                Answer::Yes => vec![Effect::Execute(pending.action)],
                Answer::No => vec![Effect::Cancelled],
                Answer::Other => {
                    // A new command replaces the pending one; other chatter keeps it waiting.
                    match extract(text) {
                        Wake::Command(c) => vec![Effect::Interpret(c)],
                        Wake::Bare => {
                            self.state = State::AwaitingCommand { until: now_ms + COMMAND_WAIT_MS };
                            vec![Effect::Say("Yes?".into())]
                        }
                        Wake::None => {
                            self.state = State::AwaitingConfirm { pending, until };
                            vec![]
                        }
                    }
                }
            },
            State::AwaitingCommand { .. } => match extract(text) {
                Wake::Command(c) => vec![Effect::Interpret(c)],
                Wake::Bare => {
                    self.state = State::AwaitingCommand { until: now_ms + COMMAND_WAIT_MS };
                    vec![Effect::Say("Yes?".into())]
                }
                Wake::None => vec![Effect::Interpret(text.trim().to_string())],
            },
            State::Idle => match extract(text) {
                Wake::Command(c) => vec![Effect::Interpret(c)],
                Wake::Bare => {
                    self.state = State::AwaitingCommand { until: now_ms + COMMAND_WAIT_MS };
                    vec![Effect::Say("Yes?".into())]
                }
                Wake::None => vec![],
            },
        }
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test wake::`
Expected: 5 passed. If `extracts_the_command…` fails on punctuation, adjust the trailing trim in `extract` only.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/wake.rs src-tauri/src/lib.rs
git commit -m "feat: wake word extraction and the yes/no conversation flow"
```

---

### Task 4: The interpreter (`interpreter.rs`)

**Files:**
- Create: `src-tauri/src/interpreter.rs`
- Modify: `src-tauri/src/lib.rs` (add `pub mod interpreter;`)

**Interfaces:**
- Consumes: `model::Card` and `launch::list_project_dirs`.
- Produces:
  - `pub fn board_summary(cards: &[Card]) -> String`
  - `pub fn system_prompt() -> String`
  - `pub fn user_prompt(command: &str, summary: &str, history: &[(String, String)]) -> String`
  - `pub struct Reply { pub say: String, pub action: Option<serde_json::Value>, pub confirm: bool }`
  - `pub fn parse_reply(claude_json: &str) -> Result<Reply, String>` (takes `claude -p --output-format json` output)
  - `pub fn validate(action: &serde_json::Value, cards: &[Card], dirs: &[String]) -> Result<serde_json::Value, String>` (returns the action with `session` replaced by the matched session id)
  - `pub fn needs_confirm(action: &serde_json::Value) -> bool`
  - `pub fn run(binary: &Path, model: &str, command: &str, cards: &[Card], history: &[(String, String)]) -> Result<Reply, String>`

- [ ] **Step 1: Write the failing tests**

Append to `src-tauri/src/interpreter.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AwaitKind, Awaiting, Card, Harness, State};

    fn card(id: &str, name: &str, state: State, ask: Option<&str>) -> Card {
        Card {
            session_id: id.into(), pid: 1, name: name.into(), cwd: format!("/Users/x/dev/{id}"), state, state_since: 0, snippet: "".into(),
            awaiting: ask.map(|a| Awaiting { kind: AwaitKind::Text, detail: a.into(), questions: vec![] }),
            has_inbox: true, harness: Harness::ClaudeCode, pr: None, context: None,
        }
    }

    #[test]
    fn summary_lists_each_session_on_one_line() {
        let s = board_summary(&[card("a", "hexgrid-d3", State::Awaiting, Some("Push now?")), card("b", "Coral4 Loop", State::Working, None)]);
        assert!(s.contains("hexgrid-d3 | claude-code | awaiting | a | asks: Push now?"));
        assert!(s.contains("Coral4 Loop | claude-code | working | b"));
    }

    #[test]
    fn prompt_carries_command_summary_and_history() {
        let p = user_prompt("what's waiting", "hexgrid | …", &[("Maya what's up".into(), "Nothing much.".into())]);
        assert!(p.contains("what's waiting"));
        assert!(p.contains("hexgrid | …"));
        assert!(p.contains("User: Maya what's up"));
        assert!(p.contains("Maya: Nothing much."));
        assert!(system_prompt().contains("\"kind\""));
    }

    #[test]
    fn parses_the_json_inside_claude_print_output() {
        let out = r#"{"type":"result","subtype":"success","is_error":false,"result":"```json\n{\"say\":\"Telling hexgrid: go ahead.\",\"action\":{\"kind\":\"reply\",\"session\":\"hexgrid\",\"text\":\"go ahead\"},\"confirm\":true}\n```"}"#;
        let r = parse_reply(out).unwrap();
        assert_eq!(r.say, "Telling hexgrid: go ahead.");
        assert_eq!(r.action.unwrap()["kind"], "reply");
        assert!(r.confirm);
        let plain = r#"{"type":"result","result":"{\"say\":\"Nothing is waiting.\",\"action\":null,\"confirm\":false}"}"#;
        assert!(parse_reply(plain).unwrap().action.is_none());
        assert!(parse_reply(r#"{"type":"result","is_error":true,"result":"boom"}"#).unwrap_err().contains("boom"));
        assert!(parse_reply(r#"{"type":"result","result":"I am not JSON"}"#).is_err());
    }

    #[test]
    fn validation_matches_sessions_loosely_and_rejects_the_unknown() {
        let cards = vec![card("a", "hexgrid-d3", State::Awaiting, Some("Push now?")), card("b", "Coral4 Loop", State::Working, None)];
        let dirs = vec!["maya".to_string()];
        let ok = validate(&serde_json::json!({"kind":"reply","session":"hexgrid","text":"go"}), &cards, &dirs).unwrap();
        assert_eq!(ok["session"], "a");
        let ok = validate(&serde_json::json!({"kind":"focus","session":"coral 4 loop"}), &cards, &dirs).unwrap();
        assert_eq!(ok["session"], "b");
        assert!(validate(&serde_json::json!({"kind":"focus","session":"nautilus"}), &cards, &dirs).unwrap_err().contains("nautilus"));
        assert!(validate(&serde_json::json!({"kind":"start","dir":"nowhere","prompt":"x"}), &cards, &dirs).unwrap_err().contains("nowhere"));
        assert!(validate(&serde_json::json!({"kind":"start","dir":"maya","prompt":"x"}), &cards, &dirs).is_ok());
        assert!(validate(&serde_json::json!({"kind":"answer","session":"a","option":3}), &cards, &dirs).unwrap_err().contains("option"));
        assert!(validate(&serde_json::json!({"kind":"dance"}), &cards, &dirs).unwrap_err().contains("dance"));
        assert!(validate(&serde_json::json!({"kind":"report"}), &cards, &dirs).is_ok());
    }

    #[test]
    fn only_acting_commands_need_confirmation() {
        for k in ["reply", "answer", "start", "resume"] {
            assert!(needs_confirm(&serde_json::json!({"kind": k})), "{k}");
        }
        for k in ["report", "focus", "compact"] {
            assert!(!needs_confirm(&serde_json::json!({"kind": k})), "{k}");
        }
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test interpreter::`
Expected: compile errors for the missing items.

- [ ] **Step 3: Write the implementation**

Top of `src-tauri/src/interpreter.rs`:

```rust
//! Turns a spoken command into one action with a one-shot `claude -p` call.
//! The model sees the board summary and the last exchanges; Maya validates
//! whatever comes back before acting on it.

use crate::model::Card;
use serde_json::{json, Value};
use std::path::Path;
use std::process::{Command, Stdio};

pub struct Reply {
    pub say: String,
    pub action: Option<Value>,
    pub confirm: bool,
}

/// One line per session: name | harness | state | id | project [| asks: …].
pub fn board_summary(cards: &[Card]) -> String {
    cards
        .iter()
        .map(|c| {
            let harness = serde_json::to_value(c.harness).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
            let state = serde_json::to_value(c.state).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
            let project = c.cwd.rsplit('/').find(|s| !s.is_empty()).unwrap_or("");
            let ask = c.awaiting.as_ref().map(|a| format!(" | asks: {}", a.detail)).unwrap_or_default();
            format!("{} | {harness} | {state} | {} | {project}{ask}", c.name, c.session_id)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn system_prompt() -> String {
    r#"You are Maya, a voice assistant for a board of coding-agent sessions. The user spoke a short command. Answer with ONE JSON object and nothing else:
{"say": "<one short spoken sentence>", "action": <action or null>, "confirm": <true|false>}
Actions (use the session's name or id from the board):
{"kind":"report"}                                   nothing to do; the answer is in say
{"kind":"reply","session":"<name>","text":"<text>"} send text to that session
{"kind":"answer","session":"<name>","option":<1-based number>} pick an option on its open question
{"kind":"focus","session":"<name>"}                 bring its terminal forward
{"kind":"compact","session":"<name>"}               compact its context
{"kind":"resume","dir":"<project folder>"}          resume the latest session of that folder
{"kind":"start","dir":"<project folder>","prompt":"<text>"} start a new session
Set confirm to true for reply, answer, start and resume, and phrase say as a read-back ending in "Yes?". Keep say under 20 words. If the request is unclear or names nothing on the board, use report and say what you could not find."#.to_string()
}

pub fn user_prompt(command: &str, summary: &str, history: &[(String, String)]) -> String {
    let mut p = String::new();
    if !history.is_empty() {
        p.push_str("Recent exchanges:\n");
        for (u, m) in history {
            p.push_str(&format!("User: {u}\nMaya: {m}\n"));
        }
        p.push('\n');
    }
    p.push_str("Board (name | harness | state | id | project | asks):\n");
    p.push_str(summary);
    p.push_str("\n\nCommand: ");
    p.push_str(command);
    p
}

fn json_in(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    serde_json::from_str(&text[start..=end]).ok()
}

/// The reply inside `claude -p --output-format json` output.
pub fn parse_reply(claude_json: &str) -> Result<Reply, String> {
    let v: Value = serde_json::from_str(claude_json).map_err(|e| format!("claude output was not JSON: {e}"))?;
    let result = v["result"].as_str().unwrap_or("");
    if v["is_error"].as_bool() == Some(true) {
        return Err(format!("claude failed: {result}"));
    }
    let r = json_in(result).ok_or_else(|| format!("no JSON reply in: {}", result.chars().take(120).collect::<String>()))?;
    Ok(Reply {
        say: r["say"].as_str().unwrap_or("").trim().to_string(),
        action: match &r["action"] {
            Value::Null => None,
            a => Some(a.clone()),
        },
        confirm: r["confirm"].as_bool().unwrap_or(false),
    })
}

fn loose(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

fn find_session<'a>(cards: &'a [Card], wanted: &str) -> Option<&'a Card> {
    let w = loose(wanted);
    if w.is_empty() {
        return None;
    }
    cards.iter().find(|c| c.session_id == wanted).or_else(|| cards.iter().find(|c| loose(&c.name) == w)).or_else(|| cards.iter().find(|c| loose(&c.name).contains(&w) || w.contains(&loose(&c.name))))
}

pub fn needs_confirm(action: &Value) -> bool {
    matches!(action["kind"].as_str(), Some("reply") | Some("answer") | Some("start") | Some("resume"))
}

/// The action with its session resolved to an id, or why it cannot run.
pub fn validate(action: &Value, cards: &[Card], dirs: &[String]) -> Result<Value, String> {
    let mut a = action.clone();
    let kind = a["kind"].as_str().unwrap_or("").to_string();
    match kind.as_str() {
        "report" => Ok(a),
        "reply" | "focus" | "compact" | "answer" => {
            let wanted = a["session"].as_str().unwrap_or("").to_string();
            let c = find_session(cards, &wanted).ok_or_else(|| format!("I couldn't find a session called {wanted}"))?;
            if kind == "answer" {
                let n = a["option"].as_u64().unwrap_or(0) as usize;
                let count = c.awaiting.as_ref().and_then(|w| w.questions.first()).map(|q| q.options.len()).unwrap_or(0);
                if n == 0 || n > count {
                    return Err(format!("{} has no option {n}", c.name));
                }
            }
            if kind == "reply" && a["text"].as_str().map_or(true, |t| t.trim().is_empty()) {
                return Err("There was nothing to send".into());
            }
            a["session"] = Value::String(c.session_id.clone());
            Ok(a)
        }
        "resume" | "start" => {
            let dir = a["dir"].as_str().unwrap_or("").to_string();
            let found = dirs.iter().find(|d| loose(d) == loose(&dir)).ok_or_else(|| format!("I couldn't find a project called {dir}"))?;
            a["dir"] = Value::String(found.clone());
            if kind == "start" && a["prompt"].as_str().map_or(true, |t| t.trim().is_empty()) {
                return Err("I need a prompt to start a session".into());
            }
            Ok(a)
        }
        other => Err(format!("I don't know how to {other}")),
    }
}

/// Runs the interpreter. `binary` is the claude executable.
pub fn run(binary: &Path, model: &str, command: &str, cards: &[Card], history: &[(String, String)]) -> Result<Reply, String> {
    let out = Command::new(binary)
        .args(["-p", "--model", model, "--output-format", "json", "--strict-mcp-config", "--disable-slash-commands", "--no-session-persistence", "--max-turns", "1", "--append-system-prompt"])
        .arg(system_prompt())
        .arg(user_prompt(command, &board_summary(cards), history))
        .env_clear()
        .envs(crate::launch::clean_env(std::env::vars()))
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("could not run claude: {e}"))?;
    parse_reply(&String::from_utf8_lossy(&out.stdout))
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test interpreter::`
Expected: 5 passed.

- [ ] **Step 5: Try the real interpreter once**

Run (from `src-tauri`, with a Claude login on the machine):

```bash
cat >> src/interpreter.rs <<'EOF'
#[cfg(test)]
mod probe {
    #[test]
    #[ignore]
    fn real_call() {
        let bin = crate::launch::claude_binary().unwrap();
        let cards = vec![];
        let r = super::run(&bin, "haiku", "what's waiting on me?", &cards, &[]).unwrap();
        println!("SAY {} ACTION {:?} CONFIRM {}", r.say, r.action, r.confirm);
    }
}
EOF
cargo test real_call -- --ignored --nocapture | grep SAY
```

Expected: a `SAY …` line with `ACTION Some({"kind":"report"})` or `None`. Then delete the probe module before committing.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/interpreter.rs src-tauri/src/lib.rs
git commit -m "feat: voice interpreter: prompt, claude -p call, reply parsing and validation"
```

---

### Task 5: Wiring the voice loop (`lib.rs`, `config.rs`)

**Files:**
- Modify: `src-tauri/src/config.rs` (fields `listen: bool` default false, `microphone: Option<String>`, `interpreter_model: String` default "haiku")
- Modify: `src-tauri/src/lib.rs` (voice state, commands, the listener thread, action execution)
- Modify: `src-tauri/src/notify.rs` (a `speak_and_wait(text)` that speaks synchronously with the configured voice, for replies)

**Interfaces:**
- Produces commands: `voice_listen(on: bool) -> Result<(), String>`, `voice_confirm(yes: bool)`, `voice_status() -> VoiceStatus`, `voice_history() -> Vec<VoiceTurn>`, `voice_selftest() -> Result<String, String>` (runs the sidecar `--selftest`).
- Emits event `voice` with `VoiceStatus { listening: bool, state: "off"|"idle"|"awaiting-command"|"awaiting-confirm"|"thinking"|"error", detail: String, level: f64, heard: String, said: String, pending: Option<String> }`.
- `VoiceTurn { who: "user"|"maya", text: String, at: u64 }`.

- [ ] **Step 1: Write the failing tests (config and restart backoff)**

In `src-tauri/src/config.rs` tests:

```rust
    #[test]
    fn voice_settings_default_off_with_haiku() {
        let c = Config::default();
        assert!(!c.listen);
        assert!(c.microphone.is_none());
        assert_eq!(c.interpreter_model, "haiku");
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes": 5, "listen": true, "microphone": "USB Mic", "interpreterModel": "sonnet"}"#).unwrap();
        assert!(c.listen);
        assert_eq!(c.microphone.as_deref(), Some("USB Mic"));
        assert_eq!(c.interpreter_model, "sonnet");
    }
```

In `src-tauri/src/ear.rs` tests:

```rust
    #[test]
    fn restart_backoff_grows_and_gives_up() {
        assert_eq!(restart_delay_ms(0), Some(1_000));
        assert_eq!(restart_delay_ms(1), Some(2_000));
        assert_eq!(restart_delay_ms(2), Some(4_000));
        assert_eq!(restart_delay_ms(4), Some(16_000));
        assert_eq!(restart_delay_ms(5), None, "after five crashes the indicator shows error and stays off");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cd src-tauri && cargo test voice_settings restart_backoff`
Expected: compile errors for the missing fields and `restart_delay_ms`.

- [ ] **Step 3: Config fields and backoff**

In `config.rs` struct:

```rust
    /// Listen for the "Maya" wake word while the app runs.
    #[serde(default)]
    pub listen: bool,
    /// Microphone name for the listener; None picks the built-in one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub microphone: Option<String>,
    /// Model alias for the voice interpreter.
    #[serde(default = "default_interpreter_model")]
    pub interpreter_model: String,
```

Add `fn default_interpreter_model() -> String { "haiku".into() }` and the three fields to `Default` (`listen: false, microphone: None, interpreter_model: "haiku".into()`).

In `ear.rs`:

```rust
/// Delay before restarting a listener that exited, by consecutive failure count.
pub fn restart_delay_ms(failures: u32) -> Option<u64> {
    (failures < 5).then(|| 1_000u64 << failures)
}
```

- [ ] **Step 4: The voice state, commands and loop in `lib.rs`**

Add to `AppState`: `pub voice: Mutex<VoiceState>` and:

```rust
#[derive(Default)]
pub struct VoiceState {
    pub ear: Option<ear::Ear>,
    pub flow: Option<wake::Flow>,
    pub status: VoiceStatus,
    pub history: Vec<VoiceTurn>,
    pub failures: u32,
}

#[derive(Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceStatus {
    pub listening: bool,
    pub state: String,
    pub detail: String,
    pub level: f64,
    pub heard: String,
    pub said: String,
    pub pending: Option<String>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceTurn {
    pub who: String,
    pub text: String,
    pub at: u64,
}
```

Helpers:

```rust
fn emit_voice(app: &AppHandle) {
    let status = app.state::<AppState>().voice.lock().unwrap().status.clone();
    let _ = app.emit("voice", &status);
}

fn set_voice(app: &AppHandle, f: impl FnOnce(&mut VoiceStatus)) {
    {
        let state = app.state::<AppState>();
        let mut v = state.voice.lock().unwrap();
        f(&mut v.status);
    }
    emit_voice(app);
}

fn remember(app: &AppHandle, who: &str, text: &str) {
    let state = app.state::<AppState>();
    let mut v = state.voice.lock().unwrap();
    v.history.push(VoiceTurn { who: who.into(), text: text.into(), at: now_ms() });
    if v.history.len() > 40 {
        v.history.drain(..v.history.len() - 40);
    }
}

/// Speaks a reply to the user now (blocking), with listening paused around it.
fn reply_aloud(app: &AppHandle, text: &str) {
    {
        let state = app.state::<AppState>();
        let mut v = state.voice.lock().unwrap();
        if let Some(e) = v.ear.as_mut() { e.pause(); }
        if let Some(f) = v.flow.as_mut() { f.ignore_line(text); }
        v.status.said = text.to_string();
    }
    emit_voice(app);
    remember(app, "maya", text);
    let eleven = { let state = app.state::<AppState>(); let store = state.store.lock().unwrap(); eleven_settings(&store) };
    notify::speak_now(text, eleven);
    let state = app.state::<AppState>();
    if let Some(e) = state.voice.lock().unwrap().ear.as_mut() { e.resume(); }
}
```

In `notify.rs` add a synchronous variant used only for replies:

```rust
/// Speaks now and returns when done; replies to the user go this way so
/// listening can pause around them.
pub fn speak_now(text: &str, eleven: Option<(std::path::PathBuf, String, String)>) {
    let spoken = match &eleven {
        Some((dir, key, voice_id)) => crate::voice::speak(dir, key, voice_id, text).is_ok(),
        None => false,
    };
    if !spoken {
        say_builtin(text);
    }
}
```

Action execution, reusing existing paths:

```rust
fn execute_action(app: &AppHandle, action: &serde_json::Value) -> Result<String, String> {
    let state = app.state::<AppState>();
    let kind = action["kind"].as_str().unwrap_or("");
    let session = action["session"].as_str().unwrap_or("").to_string();
    match kind {
        "report" => Ok(String::new()),
        "focus" => {
            let pid = state.store.lock().unwrap().card_for(&session, now_ms()).map(|c| c.pid).ok_or("Session is no longer running.")?;
            focus::focus_pid(pid)?;
            Ok("Done.".into())
        }
        "compact" => {
            type_into_session(&state, &session, answer::COMPACT)?;
            Ok("Compacting.".into())
        }
        "reply" => {
            let text = action["text"].as_str().unwrap_or("").to_string();
            send_reply(state.clone(), session, text)?;
            Ok("Sent.".into())
        }
        "answer" => {
            let card = state.store.lock().unwrap().card_for(&session, now_ms()).ok_or("Session is no longer running.")?;
            let n = action["option"].as_u64().unwrap_or(1) as usize;
            answer_question(state.clone(), session, card.state_since, 0, n - 1)?;
            Ok("Answered.".into())
        }
        "resume" => {
            let dir = action["dir"].as_str().unwrap_or("").to_string();
            let sessions = list_resumable_sessions(state.clone(), dir.clone())?;
            let latest = sessions.into_iter().find(|s| !s.running).ok_or("Nothing to resume there.")?;
            resume_session(state.clone(), dir, latest.id)?;
            Ok("Resuming.".into())
        }
        "start" => {
            let dir = action["dir"].as_str().unwrap_or("").to_string();
            let prompt = action["prompt"].as_str().unwrap_or("").to_string();
            start_session(state.clone(), Some(dir), prompt, launch::LaunchOptions::default())?;
            Ok("Started.".into())
        }
        other => Err(format!("I don't know how to {other}")),
    }
}
```

The `send_reply`, `answer_question`, `list_resumable_sessions`, `resume_session` and `start_session` commands already exist; if their signatures take `TauriState<AppState>` by value, call them with `state.clone()` as above (Tauri's `State` is `Clone`). If the compiler objects, factor each body into a plain function taking `&AppState` and call that from both the command and here.

Handling one final segment:

```rust
fn on_heard(app: &AppHandle, text: &str) {
    let effects = {
        let state = app.state::<AppState>();
        let mut v = state.voice.lock().unwrap();
        v.status.heard = text.to_string();
        match v.flow.as_mut() {
            Some(f) => f.on_segment(text, now_ms()),
            None => vec![],
        }
    };
    if effects.is_empty() {
        set_voice(app, |s| s.state = "idle".into());
        return;
    }
    remember(app, "user", text);
    for e in effects {
        match e {
            wake::Effect::Say(s) => {
                set_voice(app, |st| st.state = "awaiting-command".into());
                reply_aloud(app, &s);
            }
            wake::Effect::Interpret(cmd) => {
                set_voice(app, |st| { st.state = "thinking".into(); st.pending = None; });
                let (cards, dirs, model, binary) = {
                    let state = app.state::<AppState>();
                    let mut store = state.store.lock().unwrap();
                    let cards = store.refresh(now_ms());
                    let dirs = store.config.projects_dir_path().map(|r| launch::list_project_dirs(&r)).unwrap_or_default();
                    (cards, dirs, store.config.interpreter_model.clone(), launch::claude_binary())
                };
                let history: Vec<(String, String)> = {
                    let state = app.state::<AppState>();
                    let v = state.voice.lock().unwrap();
                    let mut pairs = Vec::new();
                    let mut last_user: Option<String> = None;
                    for t in v.history.iter().rev().take(12).collect::<Vec<_>>().into_iter().rev() {
                        if t.who == "user" { last_user = Some(t.text.clone()); }
                        else if let Some(u) = last_user.take() { pairs.push((u, t.text.clone())); }
                    }
                    pairs.into_iter().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect()
                };
                let Some(binary) = binary else { reply_aloud(app, "I can't find the claude command."); continue; };
                let reply = match interpreter::run(&binary, &model, &cmd, &cards, &history) {
                    Ok(r) => r,
                    Err(e) => { reply_aloud(app, "Sorry, I didn't catch that."); eprintln!("interpreter: {e}"); set_voice(app, |st| st.state = "idle".into()); continue; }
                };
                match reply.action.as_ref().map(|a| interpreter::validate(a, &cards, &dirs)) {
                    None | Some(Ok(_)) if reply.action.as_ref().map_or(true, |a| a["kind"] == "report") => {
                        reply_aloud(app, &reply.say);
                        set_voice(app, |st| st.state = "idle".into());
                    }
                    Some(Ok(action)) if interpreter::needs_confirm(&action) => {
                        let say = if reply.say.is_empty() { "Shall I?".to_string() } else { reply.say.clone() };
                        {
                            let state = app.state::<AppState>();
                            let mut v = state.voice.lock().unwrap();
                            if let Some(f) = v.flow.as_mut() { f.set_pending(wake::Pending { say: say.clone(), action: action.clone() }, now_ms()); }
                            v.status.pending = Some(say.clone());
                        }
                        set_voice(app, |st| st.state = "awaiting-confirm".into());
                        reply_aloud(app, &say);
                    }
                    Some(Ok(action)) => {
                        let said = match execute_action(app, &action) { Ok(s) => if reply.say.is_empty() { s } else { reply.say.clone() }, Err(e) => e };
                        reply_aloud(app, &said);
                        set_voice(app, |st| st.state = "idle".into());
                    }
                    Some(Err(why)) => {
                        reply_aloud(app, &why);
                        set_voice(app, |st| st.state = "idle".into());
                    }
                }
            }
            wake::Effect::Execute(action) => {
                set_voice(app, |st| { st.pending = None; st.state = "thinking".into(); });
                let said = match execute_action(app, &action) { Ok(s) => s, Err(e) => e };
                reply_aloud(app, &said);
                set_voice(app, |st| st.state = "idle".into());
            }
            wake::Effect::Cancelled => {
                set_voice(app, |st| { st.pending = None; st.state = "idle".into(); });
                reply_aloud(app, "Cancelled.");
            }
        }
    }
}
```

The listener thread and the commands:

```rust
fn start_listening(app: &AppHandle) -> Result<(), String> {
    let device = app.state::<AppState>().store.lock().unwrap().config.microphone.clone();
    let (ear, rx) = ear::Ear::spawn(device.as_deref())?;
    {
        let state = app.state::<AppState>();
        let mut v = state.voice.lock().unwrap();
        v.ear = Some(ear);
        v.flow = Some(wake::Flow::new());
        v.status.listening = true;
        v.status.state = "idle".into();
        v.status.detail.clear();
    }
    emit_voice(app);
    let handle = app.clone();
    std::thread::spawn(move || {
        for ev in rx {
            match ev {
                ear::EarEvent::Level(l) => {
                    let state = handle.state::<AppState>();
                    state.voice.lock().unwrap().status.level = l;
                }
                ear::EarEvent::Partial(t) => set_voice(&handle, |s| s.heard = t),
                ear::EarEvent::Final(t) => on_heard(&handle, &t),
                ear::EarEvent::State { state, detail } if state == "error" || state == "exited" => {
                    let wants = handle.state::<AppState>().store.lock().unwrap().config.listen;
                    let failures = {
                        let st = handle.state::<AppState>();
                        let mut v = st.voice.lock().unwrap();
                        v.ear = None;
                        v.status.listening = false;
                        v.failures += 1;
                        v.failures
                    };
                    set_voice(&handle, |s| { s.state = "error".into(); s.detail = if detail.is_empty() { "the listener stopped".into() } else { detail.clone() }; });
                    if wants {
                        if let Some(delay) = ear::restart_delay_ms(failures - 1) {
                            std::thread::sleep(Duration::from_millis(delay));
                            let _ = start_listening(&handle);
                        }
                    }
                    break;
                }
                ear::EarEvent::State { state, detail } => set_voice(&handle, |s| { if state == "device" { s.detail = format!("microphone: {detail}"); } }),
                _ => {}
            }
        }
    });
    Ok(())
}

fn stop_listening(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut v = state.voice.lock().unwrap();
    if let Some(mut e) = v.ear.take() {
        e.stop();
    }
    v.flow = None;
    v.status = VoiceStatus { state: "off".into(), ..Default::default() };
    v.failures = 0;
    drop(v);
    emit_voice(app);
}

#[tauri::command(async)]
fn voice_listen(app: AppHandle, state: TauriState<AppState>, on: bool) -> Result<(), String> {
    {
        let mut store = state.store.lock().unwrap();
        store.config.listen = on;
        config::save(&store.config_path(), &store.config)?;
    }
    if on { start_listening(&app) } else { stop_listening(&app); Ok(()) }
}

#[tauri::command(async)]
fn voice_confirm(app: AppHandle, yes: bool) {
    on_heard(&app, if yes { "yes" } else { "no" });
}

#[tauri::command]
fn voice_status(state: TauriState<AppState>) -> VoiceStatus {
    state.voice.lock().unwrap().status.clone()
}

#[tauri::command]
fn voice_history(state: TauriState<AppState>) -> Vec<VoiceTurn> {
    state.voice.lock().unwrap().history.clone()
}

#[tauri::command(async)]
fn voice_selftest() -> Result<String, String> {
    let path = ear::sidecar_path().ok_or("The listener (maya-ear) is not built. Run `pnpm ear:build`.")?;
    let out = std::process::Command::new(path).arg("--selftest").output().map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
```

Register the five commands in `generate_handler!`, add `voice: Mutex::new(VoiceState::default())` to `.manage(AppState { … })`, and in `setup` after the other threads: `if app.state::<AppState>().store.lock().unwrap().config.listen { let h = app.handle().clone(); std::thread::spawn(move || { let _ = start_listening(&h); }); }`.

- [ ] **Step 5: Build and run the tests**

Run: `cd src-tauri && cargo test`
Expected: all pass, including the two new ones. Fix borrow errors by binding `app.state::<AppState>()` to a local before locking, as done elsewhere in `lib.rs`.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/config.rs src-tauri/src/ear.rs src-tauri/src/lib.rs src-tauri/src/notify.rs
git commit -m "feat: voice loop: listener thread, wake-word flow, interpreter and confirmed actions"
```

---

### Task 6: Indicator, panel and settings (`src/voice.ts`, `src/settings.ts`)

**Files:**
- Create: `src/voice.ts`, `src/voice.test.ts`
- Modify: `index.html` (indicator button in the top bar, panel host), `src/main.ts` (wire events), `src/settings.ts` and `src/settings.test.ts` (listen toggle, microphone, interpreter model), `src/styles.css`

**Interfaces:**
- Consumes events `voice` with the `VoiceStatus` shape and commands `voice_listen`, `voice_confirm`, `voice_history`, `voice_status`, `voice_selftest` from Task 5.
- Produces: `renderIndicator(status)`, `renderVoicePanel(status, turns, handlers)`, `initVoice()`.

- [ ] **Step 1: Write the failing tests**

`src/voice.test.ts`:

```ts
import { describe, expect, it, vi } from "vitest";
import { renderIndicator, renderVoicePanel, type VoiceStatus } from "./voice";

const status = (over: Partial<VoiceStatus> = {}): VoiceStatus => ({ listening: true, state: "idle", detail: "", level: 0, heard: "", said: "", pending: null, ...over });

describe("voice indicator", () => {
  it("reflects each state in a class and a label", () => {
    expect(renderIndicator(status({ listening: false, state: "off" })).className).toContain("voice--off");
    expect(renderIndicator(status()).className).toContain("voice--idle");
    expect(renderIndicator(status({ state: "awaiting-command" })).className).toContain("voice--awaiting");
    expect(renderIndicator(status({ state: "awaiting-confirm" })).className).toContain("voice--awaiting");
    expect(renderIndicator(status({ state: "thinking" })).className).toContain("voice--thinking");
    const err = renderIndicator(status({ state: "error", detail: "no microphone" }));
    expect(err.className).toContain("voice--error");
    expect(err.title).toContain("no microphone");
    expect(renderIndicator(status({ level: 0.5 })).style.getPropertyValue("--level")).toBe("0.5");
  });
});

describe("voice panel", () => {
  it("shows the exchange, the pending read-back with yes and no, and a listen toggle", () => {
    const h = { onConfirm: vi.fn(), onListen: vi.fn() };
    const el = renderVoicePanel(status({ heard: "tell hexgrid to go ahead", said: "Telling hexgrid: go ahead. Yes?", pending: "Telling hexgrid: go ahead. Yes?" }), [{ who: "user", text: "Maya what's waiting", at: 1 }, { who: "maya", text: "Nothing is waiting.", at: 2 }], h);
    expect([...el.querySelectorAll(".voice__turn")].map((t) => t.textContent)).toEqual(["Maya what's waiting", "Nothing is waiting."]);
    expect(el.querySelector(".voice__pending")?.textContent).toContain("go ahead");
    el.querySelector<HTMLButtonElement>("button[data-action=voice-yes]")!.click();
    expect(h.onConfirm).toHaveBeenCalledWith(true);
    el.querySelector<HTMLButtonElement>("button[data-action=voice-no]")!.click();
    expect(h.onConfirm).toHaveBeenCalledWith(false);
    const toggle = el.querySelector<HTMLInputElement>("input[name=listen]")!;
    expect(toggle.checked).toBe(true);
    toggle.checked = false;
    toggle.dispatchEvent(new Event("change"));
    expect(h.onListen).toHaveBeenCalledWith(false);
    const quiet = renderVoicePanel(status(), [], h);
    expect(quiet.querySelector(".voice__pending")).toBeNull();
    expect(quiet.querySelector(".voice__empty")?.textContent).toContain("Say \"Maya\"");
  });
});
```

In `src/settings.test.ts`, extend the handlers with `onListen: vi.fn(), onMicrophone: vi.fn(), onInterpreter: vi.fn()`, add `listen: false, microphone: "", microphones: [], interpreterModel: "haiku"` to the base model, and add:

```ts
  it("has the listen toggle, a microphone picker and an interpreter model", () => {
    const h = handlers();
    const el = renderSettings({ ...voiceBase, listen: true, microphone: "USB Mic", microphones: ["MacBook Pro Microphone", "USB Mic"], interpreterModel: "haiku" }, h);
    const listen = el.querySelector<HTMLInputElement>("input[name=listen]")!;
    expect(listen.checked).toBe(true);
    expect(listen.closest("label")?.textContent).toContain("Listen for");
    listen.checked = false;
    listen.dispatchEvent(new Event("change"));
    expect(h.onListen).toHaveBeenCalledWith(false);
    const mic = el.querySelector<HTMLSelectElement>("select[name=microphone]")!;
    expect([...mic.options].map((o) => o.textContent)).toEqual(["Built-in (recommended)", "MacBook Pro Microphone", "USB Mic"]);
    expect(mic.value).toBe("USB Mic");
    mic.value = "";
    mic.dispatchEvent(new Event("change"));
    expect(h.onMicrophone).toHaveBeenCalledWith("");
    const model = el.querySelector<HTMLSelectElement>("select[name=interpreterModel]")!;
    expect([...model.options].map((o) => o.value)).toEqual(["haiku", "sonnet", "opus"]);
    model.value = "sonnet";
    model.dispatchEvent(new Event("change"));
    expect(h.onInterpreter).toHaveBeenCalledWith("sonnet");
  });
```

- [ ] **Step 2: Run to verify they fail**

Run: `pnpm test -- src/voice src/settings`
Expected: `src/voice.test.ts` fails to load; the settings test fails on the missing controls.

- [ ] **Step 3: Write `src/voice.ts`**

```ts
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface VoiceStatus {
  listening: boolean;
  state: "off" | "idle" | "awaiting-command" | "awaiting-confirm" | "thinking" | "error";
  detail: string;
  level: number;
  heard: string;
  said: string;
  pending: string | null;
}

export interface VoiceTurn {
  who: "user" | "maya";
  text: string;
  at: number;
}

export interface VoiceHandlers {
  onConfirm(yes: boolean): void;
  onListen(on: boolean): void;
}

const LABEL: Record<VoiceStatus["state"], string> = {
  off: "Not listening. Click to listen for \"Maya\".",
  idle: "Listening for \"Maya\".",
  "awaiting-command": "Yes? Say your command.",
  "awaiting-confirm": "Waiting for yes or no.",
  thinking: "Thinking…",
  error: "Listener stopped",
};

/** The top-bar microphone button; its class carries the state, --level the input level. */
export function renderIndicator(s: VoiceStatus): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
  b.dataset.action = "voice";
  const cls = s.state === "awaiting-command" || s.state === "awaiting-confirm" ? "awaiting" : s.state;
  b.className = `voice voice--${cls}`;
  b.title = s.state === "error" && s.detail ? `${LABEL.error}: ${s.detail}` : LABEL[s.state];
  b.setAttribute("aria-label", b.title);
  b.style.setProperty("--level", String(Math.max(0, Math.min(1, s.level))));
  b.innerHTML = '<svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><rect x="5.5" y="1.5" width="5" height="8" rx="2.5" fill="currentColor"/><path d="M3.5 7.5 a4.5 4.5 0 0 0 9 0 M8 12 V14.5 M5.5 14.5 H10.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/></svg>';
  return b;
}

export function renderVoicePanel(s: VoiceStatus, turns: VoiceTurn[], h: VoiceHandlers): HTMLElement {
  const root = document.createElement("div");
  root.className = "voice-panel";
  const head = document.createElement("label");
  head.className = "settings__check";
  const toggle = document.createElement("input");
  toggle.type = "checkbox";
  toggle.name = "listen";
  toggle.checked = s.listening;
  toggle.addEventListener("change", () => h.onListen(toggle.checked));
  head.append(toggle, document.createTextNode(" Listen for \"Maya\""));
  root.append(head);
  if (s.state === "error" && s.detail) {
    const err = document.createElement("div");
    err.className = "voice__error";
    err.textContent = s.detail;
    root.append(err);
  }
  const list = document.createElement("div");
  list.className = "voice__turns";
  if (turns.length === 0) {
    const empty = document.createElement("div");
    empty.className = "voice__empty";
    empty.textContent = 'Say "Maya" and then what you need: what\'s waiting, reply to a session, focus, compact, resume or start one.';
    list.append(empty);
  }
  for (const t of turns) {
    const row = document.createElement("div");
    row.className = `voice__turn voice__turn--${t.who}`;
    row.textContent = t.text;
    list.append(row);
  }
  root.append(list);
  if (s.pending) {
    const pending = document.createElement("div");
    pending.className = "voice__pending";
    pending.textContent = s.pending;
    const yes = document.createElement("button");
    yes.type = "button";
    yes.className = "card__btn card__btn--primary";
    yes.dataset.action = "voice-yes";
    yes.textContent = "Yes";
    yes.addEventListener("click", () => h.onConfirm(true));
    const no = document.createElement("button");
    no.type = "button";
    no.className = "card__btn";
    no.dataset.action = "voice-no";
    no.textContent = "No";
    no.addEventListener("click", () => h.onConfirm(false));
    pending.append(document.createElement("br"), yes, no);
    root.append(pending);
  }
  return root;
}

/** Mounts the indicator and panel and keeps them current. */
export async function initVoice(): Promise<void> {
  const host = document.getElementById("voice-host");
  const panel = document.getElementById("voice-panel");
  if (!host || !panel) return;
  let status: VoiceStatus = { listening: false, state: "off", detail: "", level: 0, heard: "", said: "", pending: null };
  let turns: VoiceTurn[] = [];
  const handlers: VoiceHandlers = {
    onConfirm: (yes) => void invoke("voice_confirm", { yes }),
    onListen: (on) => void invoke("voice_listen", { on }).catch((e) => { status = { ...status, state: "error", detail: String(e) }; paint(); }),
  };
  const paint = () => {
    host.replaceChildren(renderIndicator(status));
    if (!panel.hidden) panel.replaceChildren(renderVoicePanel(status, turns, handlers));
  };
  host.addEventListener("click", () => {
    panel.hidden = !panel.hidden;
    if (!panel.hidden) void invoke<VoiceTurn[]>("voice_history").then((t) => { turns = t; paint(); });
    paint();
  });
  await listen<VoiceStatus>("voice", (e) => {
    const was = status.said;
    status = e.payload;
    if (status.said !== was || status.heard) void invoke<VoiceTurn[]>("voice_history").then((t) => { turns = t; paint(); });
    paint();
  });
  status = await invoke<VoiceStatus>("voice_status");
  paint();
}
```

In `index.html` add `<span id="voice-host"></span>` after the tabs and `<aside id="voice-panel" class="settings" hidden></aside>` after the settings aside. In `src/main.ts` call `void initVoice();` next to `initSettings()`.

Styles:

```css
.voice { display: inline-flex; align-items: center; justify-content: center; width: 28px; height: 28px; border-radius: 999px; border: 1px solid var(--border); background: var(--panel); color: var(--muted); cursor: pointer; position: relative; margin-left: 8px; }
.voice::after { content: ""; position: absolute; inset: -3px; border-radius: 999px; border: 2px solid currentColor; opacity: calc(var(--level, 0) * 0.9); pointer-events: none; }
.voice--idle { color: var(--completed); }
.voice--awaiting { color: var(--awaiting); }
.voice--thinking { color: var(--working); }
.voice--error { color: #e5484d; }
.voice--off { color: var(--muted); opacity: .6; }
.voice-panel { display: flex; flex-direction: column; gap: 10px; }
.voice__turns { display: flex; flex-direction: column; gap: 6px; max-height: 40vh; overflow-y: auto; }
.voice__turn { padding: 6px 10px; border-radius: 8px; font-size: 13px; }
.voice__turn--user { background: color-mix(in srgb, var(--working) 14%, transparent); align-self: flex-end; }
.voice__turn--maya { background: var(--bg); align-self: flex-start; }
.voice__pending { padding: 8px 10px; border-radius: 8px; background: color-mix(in srgb, var(--awaiting) 14%, transparent); font-size: 13px; }
.voice__pending button { margin: 6px 6px 0 0; }
.voice__empty, .voice__error { color: var(--muted); font-size: 13px; }
.voice__error { color: #e5484d; }
```

- [ ] **Step 4: Settings controls**

In `settings.ts` add to the model `listen: boolean; microphone: string; microphones: string[]; interpreterModel: string;`, to the handlers `onListen(on: boolean)`, `onMicrophone(name: string)`, `onInterpreter(model: string)`, and to `ConfigJson` `listen: boolean; microphone?: string | null; interpreterModel: string;`. Render, after the speak toggle:

```ts
  const listenLabel = document.createElement("label");
  listenLabel.className = "settings__check";
  const listenBox = document.createElement("input");
  listenBox.type = "checkbox";
  listenBox.name = "listen";
  listenBox.checked = model.listen;
  listenBox.addEventListener("change", () => h.onListen(listenBox.checked));
  listenLabel.append(listenBox, document.createTextNode(' Listen for "Maya" (on-device speech recognition)'));
  root.append(listenLabel);

  const micLabel = document.createElement("label");
  micLabel.textContent = "Microphone";
  const mic = document.createElement("select");
  mic.name = "microphone";
  const auto = document.createElement("option");
  auto.value = "";
  auto.textContent = "Built-in (recommended)";
  mic.append(auto);
  for (const name of model.microphones) {
    const o = document.createElement("option");
    o.value = name;
    o.textContent = name;
    mic.append(o);
  }
  mic.value = model.microphones.includes(model.microphone) ? model.microphone : "";
  mic.addEventListener("change", () => h.onMicrophone(mic.value));
  micLabel.append(mic);
  root.append(micLabel);

  const modelLabel = document.createElement("label");
  modelLabel.textContent = "Voice interpreter";
  const modelSel = document.createElement("select");
  modelSel.name = "interpreterModel";
  for (const [v, text] of [["haiku", "Haiku (fast)"], ["sonnet", "Sonnet"], ["opus", "Opus"]] as const) {
    const o = document.createElement("option");
    o.value = v;
    o.textContent = text;
    modelSel.append(o);
  }
  modelSel.value = model.interpreterModel;
  modelSel.addEventListener("change", () => h.onInterpreter(modelSel.value));
  modelLabel.append(modelSel);
  root.append(modelLabel);
```

Handlers in `initSettings`: `onListen: (on) => void run(async () => { await invoke("voice_listen", { on }); model.listen = on; })`, `onMicrophone: (name) => void run(() => saveConfig({ microphone: name || null }))`, `onInterpreter: (m) => void run(() => saveConfig({ interpreterModel: m }))`. Load `microphones` once from `voice_selftest`: parse its JSON and take `devices`, ignoring errors. Include `listen`, `microphone` and `interpreterModel` in `saveConfig`'s payload and copy them back from the response, as the other fields are.

- [ ] **Step 5: Run the tests and type check**

Run: `pnpm test && npx tsc --noEmit`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add src/voice.ts src/voice.test.ts src/settings.ts src/settings.test.ts src/main.ts index.html src/styles.css
git commit -m "feat: voice indicator, panel and settings"
```

---

### Task 7: Release builds the listener

**Files:**
- Modify: `.github/workflows/release.yml`

- [ ] **Step 1: Add the ear builds before the Tauri builds**

After "Install dependencies":

```yaml
      - name: Build the listener (both architectures)
        run: |
          sh scripts/build-ear.sh aarch64-apple-darwin
          sh scripts/build-ear.sh x86_64-apple-darwin
```

- [ ] **Step 2: Lint and commit**

Run: `actionlint .github/workflows/release.yml`
Expected: clean.

```bash
git add .github/workflows/release.yml
git commit -m "ci: build the maya-ear listener for both architectures"
```

- [ ] **Step 3: Push and watch the run**

Run: `git push origin main && gh run watch --exit-status`
Expected: success; the release's `.app.tar.gz` contains `Contents/MacOS/maya-ear` (check with `tar -tzf` on a downloaded asset).

---

### Task 8: End to end

- [ ] **Step 1: Build and start**

Run: `pnpm ear:build && (cd src-tauri && cargo build) && ./src-tauri/target/debug/maya` with vite running. In Settings turn on "Listen for Maya". Approve the speech recognition permission dialog on first use. The indicator turns green.

- [ ] **Step 2: A question**

Say: "Maya, what's waiting on me?"
Expected: the indicator turns amber then blue; she answers aloud within a few seconds with the sessions in Awaiting Decision, or that nothing is waiting; the panel shows both lines.

- [ ] **Step 3: A confirmed send**

Start a scratch session named `voice-test` (`claude --name voice-test -- "Reply ready and stop"`), then say: "Maya, tell voice test to say hello".
Expected: she reads back "Telling voice-test: say hello. Yes?" and the indicator stays amber. Say "yes". She says "Sent." and the scratch session receives the message (its terminal shows it). Say "Maya, tell voice test goodbye", then "no": she says "Cancelled." and nothing is sent.

- [ ] **Step 4: Focus mode and self-hearing**

Turn on Do Not Disturb, say "Maya, what's waiting on me?": she still answers. Let a scratch session finish: no announcement. Turn Focus off. Confirm that her own replies never wake her (the panel shows no user turn matching her last line).

- [ ] **Step 5: Commit any fixes, then push**

```bash
git add -A && git commit -m "fix: voice assistant end-to-end adjustments" && git push origin main
```
