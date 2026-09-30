# Built-in Whisper Speech Recognition Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let Maya's voice assistant listen through whisper.cpp on Macs where Apple's speech recognition is locked, with a Settings switch between System and Built-in recognition and a picker of downloadable models.

**Architecture:** The existing `maya-ear` sidecar gains a second engine behind the same audio capture and JSON line protocol, chosen by `--engine`. Whisper is linked from whisper.cpp's prebuilt xcframework (no cmake) and bundled inside Maya.app; models are downloaded on demand into `~/.claude/maya/models/` by a new Rust module with progress events. The Rust listener and the wake-word, confirmation and interpreter code are untouched except for passing the engine and model and refusing to start without a model.

**Tech Stack:** Swift 6 (`swiftc`, no Xcode project), whisper.cpp v1.9.2 xcframework (Metal), AVFoundation/AVAudioConverter, Rust (Tauri 2, serde, `curl` via `std::process::Command`), TypeScript + vitest.

**Spec:** `docs/superpowers/specs/2026-09-29-maya-builtin-recognition-design.md`

## Global Constraints

- Sidecar target stays `macos14.0`; the whisper framework's minimum is 13.3, so no change to `minimumSystemVersion`.
- whisper.cpp release pinned to **v1.9.2**, asset `whisper-v1.9.2-xcframework.zip`, sha256 `af74fed13ea7f2d5ca2a39d9f58ec177713fafd7cab63aef4e27b79f3ceca80b`. The zip unpacks to `build-apple/whisper.xcframework/`.
- Vendored framework lives in `vendor/whisper.xcframework` (git-ignored). Sidecar rpaths: `@executable_path/../Frameworks` first, then the absolute vendor slice directory.
- Model table (id, file, bytes, sha256), verbatim:
  - `tiny.en`, `ggml-tiny.en.bin`, 77704715, `921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f`
  - `base.en-q5_1`, `ggml-base.en-q5_1.bin`, 59721011, `4baf70dd0d7c4247ba2b81fafd9c01005ac77c2f9ef064e00dcf195d0e2fdd2f` (default)
  - `base.en`, `ggml-base.en.bin`, 147964211, `a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002`
  - `small.en-q5_1`, `ggml-small.en-q5_1.bin`, 190098681, `bfdff4894dcb76bbf647d56263ea2a96645423f1669176f4844a1bf8e478ad30`
- Download URL: `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/<file>`. Models dir: `<claude_dir>/maya/models/`. Downloads go to `<file>.part`, are verified (size and sha256) before rename, and a failed one is deleted.
- Config: `recognizer` (`"system"` default | `"builtin"`), `whisperModel` (default `"base.en-q5_1"`). camelCase in JSON as the rest of `Config`.
- Sidecar flags: `--engine system|whisper` (default `system`), `--model <path>`. New state `loading` before `listening`.
- VAD constants: 20 ms frames at 16 kHz (320 samples); speech starts after 3 consecutive frames above threshold; ends after 0.8 s (40 frames) below; hard cap 12 s (600 frames); pre-roll 0.3 s (15 frames); threshold = noise floor + 0.012 RMS, floor = running minimum of frame RMS decayed by +0.0005 per frame; partial every 1.5 s of open utterance.
- Filler deny list (trimmed, case-insensitive, punctuation stripped): `thank you`, `thanks for watching`, `blank_audio`, `you`, `bye`, `the end`, `subtitles by the amara.org community`. Any text whose entire content is one of these is dropped.
- Commit messages: conventional commits with trailer `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`. Never push.
- Bash in this harness refuses compound commands that mix `cd` with git and refuses variables inside complex constructs; run plain commands from the worktree. `cargo` needs `PATH="$HOME/.cargo/bin:$PATH"`.

## Review Focus

1. A model file that is truncated or corrupted on disk (a crashed download, a manual copy) must make the sidecar report `state: error` with the reason and the listener show it under the toggle, never a silent restart loop. Pinned by Task 5's `refuses_a_model_file_that_is_not_the_expected_size` and Task 2's load-failure path.
2. Long continuous speech (a phone call in the room) must be cut at 12 s and not grow memory without bound: Task 2's `caps_an_utterance_at_twelve_seconds`.
3. Whisper's silence hallucinations ("Thank you.") must never reach the wake-word flow: Task 2's `drops_known_fillers`.
4. Switching recogniser or model while listening must restart the run with the new engine and not leave two sidecars: Task 5's `changing_the_recogniser_restarts_a_running_listener` (a pure decision function) plus the existing generation checks.
5. A download that fails mid-way must leave no `.part` file that a later start could mistake for a model: Task 4's `a_failed_download_leaves_nothing_behind`.

---

### Task 1: Vendor the whisper.cpp framework in the sidecar build

**Files:**
- Modify: `scripts/build-ear.sh`
- Create: `scripts/fetch-whisper.sh`
- Modify: `.gitignore`
- Modify: `src-tauri/tauri.conf.json`

**Interfaces:**
- Produces: `vendor/whisper.xcframework/macos-arm64_x86_64/whisper.framework`; the sidecar is linked with `-framework whisper` and `import whisper` compiles (Task 2 relies on this).
- Produces: `bundle.macOS.frameworks` entry so `Maya.app/Contents/Frameworks/whisper.framework` exists after `pnpm tauri build`.

- [ ] **Step 1: Create the fetch script**

```sh
#!/bin/sh
# Fetches the pinned whisper.cpp xcframework into vendor/ (git-ignored).
set -eu
cd "$(dirname "$0")/.."
version="v1.9.2"
sha="af74fed13ea7f2d5ca2a39d9f58ec177713fafd7cab63aef4e27b79f3ceca80b"
zip="vendor/whisper-$version-xcframework.zip"
dest="vendor/whisper.xcframework"
if [ -d "$dest/macos-arm64_x86_64/whisper.framework" ]; then
  echo "whisper.xcframework $version already in vendor/"
  exit 0
fi
mkdir -p vendor
if [ ! -f "$zip" ]; then
  curl -fsSL -o "$zip" "https://github.com/ggml-org/whisper.cpp/releases/download/$version/whisper-$version-xcframework.zip"
fi
actual="$(shasum -a 256 "$zip" | cut -d' ' -f1)"
if [ "$actual" != "$sha" ]; then
  echo "checksum mismatch for $zip: $actual" >&2
  rm -f "$zip"
  exit 1
fi
rm -rf vendor/whisper-unzip "$dest"
mkdir -p vendor/whisper-unzip
unzip -q "$zip" -d vendor/whisper-unzip
mv vendor/whisper-unzip/build-apple/whisper.xcframework "$dest"
rm -rf vendor/whisper-unzip
echo "fetched whisper.xcframework $version"
```

- [ ] **Step 2: Link the sidecar against it**

Replace `scripts/build-ear.sh` with:

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
sh scripts/fetch-whisper.sh
slice="$PWD/vendor/whisper.xcframework/macos-arm64_x86_64"
mkdir -p src-tauri/binaries
swiftc -O -target "$arch-apple-macos14.0" \
  -framework Speech -framework AVFoundation -framework CoreAudio \
  -F "$slice" -framework whisper \
  -Xlinker -rpath -Xlinker "@executable_path/../Frameworks" \
  -Xlinker -rpath -Xlinker "$slice" \
  -o "src-tauri/binaries/maya-ear-$target" ear/vad.swift ear/main.swift
echo "built src-tauri/binaries/maya-ear-$target"
```

Note: `ear/vad.swift` is created in Task 2; until then create an empty `ear/vad.swift` containing only `// Voice activity detection (Task 2).` so the script runs.

- [ ] **Step 3: Ignore the vendor directory and bundle the framework**

Append to `.gitignore`:

```
# Vendored whisper.cpp framework (fetched by scripts/fetch-whisper.sh)
vendor/
```

In `src-tauri/tauri.conf.json`, change the `bundle` object to:

```json
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": [
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/128x128@2x.png",
      "icons/icon.icns",
      "icons/icon.ico"
    ],
    "externalBin": ["binaries/maya-ear"],
    "macOS": {
      "frameworks": ["../vendor/whisper.xcframework/macos-arm64_x86_64/whisper.framework"]
    }
  }
```

- [ ] **Step 4: Prove the link works**

Add to the top of `ear/main.swift`, after `import Speech`: `import whisper`. Run:

```
sh scripts/build-ear.sh
src-tauri/binaries/maya-ear-aarch64-apple-darwin --selftest
```

Expected: the build prints `fetched whisper.xcframework v1.9.2` (first time) and `built …`; the selftest prints its JSON line and exits 0 (the dylib resolves through the vendor rpath).

Then `PATH="$HOME/.cargo/bin:$PATH" pnpm tauri build --bundles app` and check:

```
ls src-tauri/target/release/bundle/macos/Maya.app/Contents/Frameworks/
otool -L src-tauri/target/release/bundle/macos/Maya.app/Contents/MacOS/maya-ear | grep whisper
```

Expected: `whisper.framework` is listed, and the sidecar references `@rpath/whisper.framework/Versions/Current/whisper`.

- [ ] **Step 5: Commit**

```
git add scripts/build-ear.sh scripts/fetch-whisper.sh .gitignore src-tauri/tauri.conf.json ear/main.swift ear/vad.swift
git commit -m "build: link the listener against whisper.cpp's xcframework and bundle it"
```

---

### Task 2: Voice activity detection and filler filter (pure Swift, tested)

**Files:**
- Create: `ear/vad.swift`
- Create: `ear/vadtest.swift`
- Create: `scripts/test-ear.sh`
- Modify: `package.json` (add `"ear:test": "sh scripts/test-ear.sh"`)

**Interfaces:**
- Produces:
  ```swift
  enum VadEvent { case none, started, partialDue, ended(reason: String) }
  struct Vad {
      static let frameSamples = 320       // 20 ms at 16 kHz
      var floor: Float = 1.0
      var speaking = false
      var loudRun = 0, quietRun = 0, framesInUtterance = 0, framesSincePartial = 0
      mutating func push(rms: Float) -> VadEvent   // one frame
      static let preRollFrames = 15, endQuietFrames = 40, maxFrames = 600, partialEveryFrames = 75, startLoudFrames = 3
      static let margin: Float = 0.012, floorDecay: Float = 0.0005
  }
  func isFiller(_ text: String) -> Bool
  ```

- [ ] **Step 1: Write the failing tests**

`ear/vadtest.swift`:

```swift
// Tests for the voice activity detector and filler filter: `sh scripts/test-ear.sh`.
import Foundation

var failures = 0
func check(_ cond: Bool, _ what: String) {
    if !cond { failures += 1; print("FAIL: \(what)") }
}

func drive(_ vad: inout Vad, _ rms: Float, _ frames: Int) -> [VadEvent] {
    var out: [VadEvent] = []
    for _ in 0..<frames {
        let e = vad.push(rms: rms)
        if case .none = e { continue }
        out.append(e)
    }
    return out
}

func isStarted(_ e: VadEvent) -> Bool { if case .started = e { return true }; return false }
func isEnded(_ e: VadEvent, _ reason: String) -> Bool { if case .ended(let r) = e { return r == reason }; return false }
func isPartial(_ e: VadEvent) -> Bool { if case .partialDue = e { return true }; return false }

// Quiet room, then speech: starts after three loud frames, not one.
do {
    var v = Vad()
    _ = drive(&v, 0.002, 50)                       // settle the floor
    check(drive(&v, 0.05, 2).isEmpty, "two loud frames do not start")
    let e = drive(&v, 0.05, 1)
    check(e.count == 1 && isStarted(e[0]), "third loud frame starts the utterance")
    check(v.speaking, "speaking after start")
}

// Ends after 0.8 s (40 frames) of quiet, not before.
do {
    var v = Vad()
    _ = drive(&v, 0.002, 50)
    _ = drive(&v, 0.05, 10)
    check(drive(&v, 0.002, 39).isEmpty, "39 quiet frames keep it open")
    let e = drive(&v, 0.002, 1)
    check(e.count == 1 && isEnded(e[0], "silence"), "40th quiet frame ends it")
    check(!v.speaking, "not speaking after end")
}

// A loud frame in the middle resets the quiet count.
do {
    var v = Vad()
    _ = drive(&v, 0.002, 50)
    _ = drive(&v, 0.05, 10)
    _ = drive(&v, 0.002, 30)
    _ = drive(&v, 0.05, 1)
    check(drive(&v, 0.002, 39).isEmpty, "quiet run restarts after a loud frame")
}

// Cap at 12 s (600 frames) of continuous speech. Real speech swings between
// loud and soft frames, so alternate levels after the start; a constant level
// would become the floor (see the next test) and end by silence instead.
do {
    var v = Vad()
    _ = drive(&v, 0.002, 50)
    let start = drive(&v, 0.06, 3)
    check(start.count == 1 && isStarted(start[0]), "three loud frames start the long utterance")
    var events: [VadEvent] = []
    for i in 0..<600 {
        let e = v.push(rms: i % 2 == 0 ? 0.06 : 0.02)
        if case .none = e { continue }
        events.append(e)
    }
    check(events.contains { isEnded($0, "cap") }, "caps an utterance at twelve seconds")
    check(!events.contains { isEnded($0, "silence") }, "alternating speech is not silence")
    check(events.filter { isPartial($0) }.count == 7, "partials every 1.5 s while open: got \(events.filter { isPartial($0) }.count)")
}

// The floor adapts upward slowly so a noisy room does not stay 'speaking' forever.
do {
    var v = Vad()
    _ = drive(&v, 0.002, 50)
    _ = drive(&v, 0.03, 3)                         // starts
    let events = drive(&v, 0.03, 200)              // constant noise at the same level
    check(events.contains { isEnded($0, "silence") }, "a constant level becomes the floor and ends the utterance")
}

// Fillers.
check(isFiller("Thank you."), "thank you is a filler")
check(isFiller(" [BLANK_AUDIO] "), "blank audio is a filler")
check(isFiller("you"), "bare you is a filler")
check(!isFiller("Thank you, Maya, tell hexgrid to go ahead"), "a sentence containing a filler is kept")
check(!isFiller("Maya what's waiting on me"), "a command is kept")

if failures > 0 { print("\(failures) failure(s)"); exit(1) }
print("ear tests passed")
```

`scripts/test-ear.sh`:

```sh
#!/bin/sh
# Compiles and runs the sidecar's pure-Swift tests.
set -eu
cd "$(dirname "$0")/.."
out="$(mktemp -d)"
swiftc -O -o "$out/vadtest" ear/vad.swift ear/vadtest.swift
"$out/vadtest"
```

Add to `package.json` scripts: `"ear:test": "sh scripts/test-ear.sh"`.

- [ ] **Step 2: Run it to see it fail**

Run: `sh scripts/test-ear.sh`
Expected: compile errors (`Vad`, `VadEvent`, `isFiller` undefined).

- [ ] **Step 3: Implement `ear/vad.swift`**

```swift
// ear/vad.swift — energy-based voice activity detection for the Whisper engine.
import Foundation

enum VadEvent {
    case none
    case started
    case partialDue
    case ended(reason: String)
}

/// Frames of 20 ms. Speech starts after `startLoudFrames` frames above the
/// threshold and ends after `endQuietFrames` below it, or at `maxFrames`.
/// The threshold follows the room: `floor` is the running minimum of frame
/// RMS, nudged upward each frame so a change of room is forgotten in a
/// minute or two; speech is `floor + margin`.
struct Vad {
    static let frameSamples = 320
    static let preRollFrames = 15
    static let endQuietFrames = 40
    static let maxFrames = 600
    static let partialEveryFrames = 75
    static let startLoudFrames = 3
    static let margin: Float = 0.012
    static let floorDecay: Float = 0.0005

    var floor: Float = 1.0
    var speaking = false
    var loudRun = 0
    var quietRun = 0
    var framesInUtterance = 0
    var framesSincePartial = 0

    mutating func push(rms: Float) -> VadEvent {
        floor = min(floor + Vad.floorDecay, rms)
        let loud = rms > floor + Vad.margin
        if !speaking {
            loudRun = loud ? loudRun + 1 : 0
            if loudRun >= Vad.startLoudFrames {
                speaking = true
                loudRun = 0
                quietRun = 0
                framesInUtterance = 0
                framesSincePartial = 0
                return .started
            }
            return .none
        }
        framesInUtterance += 1
        framesSincePartial += 1
        quietRun = loud ? 0 : quietRun + 1
        if quietRun >= Vad.endQuietFrames {
            speaking = false
            return .ended(reason: "silence")
        }
        if framesInUtterance >= Vad.maxFrames {
            speaking = false
            return .ended(reason: "cap")
        }
        if framesSincePartial >= Vad.partialEveryFrames {
            framesSincePartial = 0
            return .partialDue
        }
        return .none
    }
}

private let fillers: Set<String> = ["thank you", "thanks for watching", "blank_audio", "you", "bye", "the end", "subtitles by the amara.org community"]

/// Whisper invents these on silence; a result that is nothing but one of them is noise.
func isFiller(_ text: String) -> Bool {
    let cleaned = text.lowercased().filter { $0.isLetter || $0.isNumber || $0 == " " || $0 == "_" || $0 == "." }
        .trimmingCharacters(in: .whitespacesAndNewlines)
        .trimmingCharacters(in: CharacterSet(charactersIn: "."))
    return cleaned.isEmpty || fillers.contains(cleaned)
}
```

Check the arithmetic before running. Floor test: at 0.03 constant, the floor rises by 0.0005 per frame from 0.002, crossing 0.03 − 0.012 = 0.018 after 32 frames; 40 quiet frames later the utterance ends, well inside 200 frames. Cap test: the floor follows the running minimum, so with frames alternating 0.06 and 0.02 it settles at 0.02 (threshold 0.032); the 0.06 frames stay loud and reset the quiet run every other frame, so nothing ends by silence. The `.started` frame does not count towards the utterance, so the 600 pushes in the loop take `framesInUtterance` from 1 to 600: partials at 75, 150, …, 525 (seven), then the cap at 600, which is checked before the partial rule.

- [ ] **Step 4: Run the tests until green**

Run: `sh scripts/test-ear.sh`
Expected: `ear tests passed`. If the partial count differs, recount: 600 frames with a partial every 75 gives 8 partialDue events unless the cap fires on frame 600 first; the cap check comes before the partial check only when `framesInUtterance >= 600`, which is frame 600, and frame 600 is also a partial boundary (600 = 8 × 75), so the cap wins and 7 partials are emitted. Adjust the test only if the arithmetic, not the code, is wrong.

- [ ] **Step 5: Commit**

```
git add ear/vad.swift ear/vadtest.swift scripts/test-ear.sh package.json
git commit -m "feat(ear): voice activity detector and filler filter, with tests"
```

---

### Task 3: The Whisper engine in the sidecar

**Files:**
- Modify: `ear/main.swift`

**Interfaces:**
- Consumes: `Vad`, `VadEvent`, `isFiller` (Task 2); `import whisper` (Task 1).
- Produces: `maya-ear --engine whisper --model <path> [--device <name>]` emitting the same `devices`, `device`, `level`, `state`, `partial`, `final` lines; new `state: loading`; errors `model file not found: <path>` (exit 7) and `could not load model: <path>` (exit 8). `--engine system` (or no flag) behaves exactly as today.

- [ ] **Step 1: Restructure `main.swift` around a `Recogniser` protocol**

Replace the file's contents from `final class Ear {` to the end with the following (everything above, `Out`, `inputDevices`, `pickDevice`, the `--selftest` block and `preferred`, stays). Argument parsing gains the engine and model:

```swift
var engineName = "system"
if let i = args.firstIndex(of: "--engine"), i + 1 < args.count { engineName = args[i + 1] }
var modelPath: String? = nil
if let i = args.firstIndex(of: "--model"), i + 1 < args.count { modelPath = args[i + 1] }
Out.emit(["type": "devices", "names": names])

/// One of the two ways audio becomes text. Both receive every tap buffer
/// while not paused and emit `partial` and `final` lines themselves.
protocol Recogniser: AnyObject {
    /// Called once the audio engine runs; the recogniser emits `listening` when ready.
    func begin(format: AVAudioFormat)
    func accept(_ buffer: AVAudioPCMBuffer)
}

/// Apple's on-device recogniser: today's behaviour, unchanged.
final class SystemRecogniser: Recogniser {
    let recognizer = SFSpeechRecognizer(locale: Locale(identifier: "en-GB"))!
    var request: SFSpeechAudioBufferRecognitionRequest?
    var task: SFSpeechRecognitionTask?
    var lastPartial = ""
    var lastChangeAt = Date()
    var rotateDue = false
    var restartTimer: Timer?
    var endpointTimer: Timer?
    var requestBegan = Date()
    var instantFailures = 0
    var lastErrorMessage = ""
    let endpointAfter: TimeInterval = 1.2

    func begin(format: AVAudioFormat) {
        beginRequest()
        restartTimer = Timer.scheduledTimer(withTimeInterval: 50, repeats: true) { _ in
            self.rotateDue = true
            self.tick()
        }
        endpointTimer = Timer.scheduledTimer(withTimeInterval: 0.2, repeats: true) { _ in self.tick() }
        Out.emit(["type": "state", "state": "listening"])
    }

    func accept(_ buffer: AVAudioPCMBuffer) {
        request?.append(buffer)
    }

    // tick(), beginRequest(), beginRequestIfIdle(), rotate(): moved verbatim
    // from the old Ear class (they reference only the fields above).
}
```

Move `tick`, `beginRequest`, `beginRequestIfIdle` and `rotate` from the old `Ear` into `SystemRecogniser` unchanged (they only use the fields listed).

```swift
/// whisper.cpp on this Mac. The tap's audio is converted to 16 kHz mono,
/// cut into utterances by the VAD, and transcribed on one background queue.
final class WhisperRecogniser: Recogniser {
    let modelPath: String
    var ctx: OpaquePointer?
    var converter: AVAudioConverter?
    let target = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 16000, channels: 1, interleaved: false)!
    var vad = Vad()
    /// Samples not yet framed, then the last `preRollFrames` frames, then the open utterance.
    var pending: [Float] = []
    var preRoll: [Float] = []
    var utterance: [Float] = []
    let work = DispatchQueue(label: "maya.whisper")
    var busy = false

    init(modelPath: String) { self.modelPath = modelPath }

    func begin(format: AVAudioFormat) {
        converter = AVAudioConverter(from: format, to: target)
        guard FileManager.default.fileExists(atPath: modelPath) else {
            Out.emit(["type": "state", "state": "error", "detail": "model file not found: \(modelPath)"]); exit(7)
        }
        Out.emit(["type": "state", "state": "loading"])
        work.async {
            var cparams = whisper_context_default_params()
            cparams.use_gpu = true
            let t0 = Date()
            guard let c = whisper_init_from_file_with_params(self.modelPath, cparams) else {
                Out.emit(["type": "state", "state": "error", "detail": "could not load model: \(self.modelPath)"]); exit(8)
            }
            self.ctx = c
            Out.emit(["type": "note", "text": String(format: "model loaded in %.1f s", Date().timeIntervalSince(t0))])
            Out.emit(["type": "state", "state": "listening"])
        }
    }

    func accept(_ buffer: AVAudioPCMBuffer) {
        guard let converter = converter else { return }
        let ratio = target.sampleRate / buffer.format.sampleRate
        let capacity = AVAudioFrameCount(Double(buffer.frameLength) * ratio) + 32
        guard let out = AVAudioPCMBuffer(pcmFormat: target, frameCapacity: capacity) else { return }
        var consumed = false
        var error: NSError?
        converter.convert(to: out, error: &error) { _, status in
            if consumed { status.pointee = .noDataNow; return nil }
            consumed = true
            status.pointee = .haveData
            return buffer
        }
        guard error == nil, let ch = out.floatChannelData?[0] else { return }
        pending.append(contentsOf: UnsafeBufferPointer(start: ch, count: Int(out.frameLength)))
        while pending.count >= Vad.frameSamples {
            let frame = Array(pending[0..<Vad.frameSamples])
            pending.removeFirst(Vad.frameSamples)
            step(frame)
        }
    }

    private func step(_ frame: [Float]) {
        var sum: Float = 0
        for s in frame { sum += s * s }
        let rms = sqrt(sum / Float(frame.count))
        switch vad.push(rms: rms) {
        case .none:
            if vad.speaking {
                utterance.append(contentsOf: frame)
            } else {
                preRoll.append(contentsOf: frame)
                let keep = Vad.preRollFrames * Vad.frameSamples
                if preRoll.count > keep { preRoll.removeFirst(preRoll.count - keep) }
            }
        case .started:
            utterance = preRoll + frame
            preRoll = []
        case .partialDue:
            utterance.append(contentsOf: frame)
            transcribe(Array(utterance), final: false)
        case .ended(let reason):
            utterance.append(contentsOf: frame)
            let audio = utterance
            utterance = []
            Out.emit(["type": "note", "text": String(format: "utterance %.1f s (%@)", Double(audio.count) / 16000, reason)])
            transcribe(audio, final: true)
        }
    }

    /// A partial is skipped while the previous transcription still runs; a
    /// final always runs, queued behind it.
    private func transcribe(_ audio: [Float], final: Bool) {
        if !final && busy { return }
        busy = true
        work.async {
            defer { self.busy = false }
            guard let ctx = self.ctx else { return }
            var params = whisper_full_default_params(WHISPER_SAMPLING_GREEDY)
            params.print_progress = false
            params.print_realtime = false
            params.print_timestamps = false
            params.no_timestamps = true
            params.single_segment = true
            params.suppress_blank = true
            params.n_threads = 4
            params.language = UnsafePointer(strdup("en"))
            let t0 = Date()
            let rc = audio.withUnsafeBufferPointer { whisper_full(ctx, params, $0.baseAddress, Int32(audio.count)) }
            guard rc == 0 else {
                Out.emit(["type": "state", "state": "warning", "detail": "whisper_full failed: \(rc)"]); return
            }
            var text = ""
            for i in 0..<whisper_full_n_segments(ctx) { text += String(cString: whisper_full_get_segment_text(ctx, i)) }
            text = text.trimmingCharacters(in: .whitespacesAndNewlines)
            if final {
                Out.emit(["type": "note", "text": String(format: "transcribed %.1f s in %.2f s", Double(audio.count) / 16000, Date().timeIntervalSince(t0))])
            }
            if isFiller(text) {
                if final && !text.isEmpty { Out.emit(["type": "note", "text": "dropped filler: \(text)"]) }
                return
            }
            Out.emit(["type": final ? "final" : "partial", "text": text])
        }
    }
}

final class Ear {
    let engine = AVAudioEngine()
    let recogniser: Recogniser
    var paused = false
    var lastLevelAt: TimeInterval = 0

    init(recogniser: Recogniser) { self.recogniser = recogniser }

    func start() {
        if recogniser is SystemRecogniser {
            SFSpeechRecognizer.requestAuthorization { status in
                guard status == .authorized else {
                    Out.emit(["type": "state", "state": "error", "detail": "speech recognition not authorised (\(status.rawValue))"]); exit(2)
                }
                DispatchQueue.main.async { self.startAudio() }
            }
        } else {
            startAudio()
        }
    }

    func startAudio() {
        // (device selection block: unchanged from today, ending with Out.emit(["type": "device", …]))
        let input = engine.inputNode
        let format = input.outputFormat(forBus: 0)
        input.installTap(onBus: 0, bufferSize: 2048, format: format) { buffer, _ in
            guard !self.paused else { return }
            self.recogniser.accept(buffer)
            // (level meter block: unchanged from today)
        }
        engine.prepare()
        do { try engine.start() } catch {
            Out.emit(["type": "state", "state": "error", "detail": "audio engine: \(error.localizedDescription)"]); exit(3)
        }
        recogniser.begin(format: format)
    }

    func setPaused(_ p: Bool) {
        paused = p
        Out.emit(["type": "state", "state": p ? "paused" : "listening"])
    }
}

let recogniser: Recogniser
switch engineName {
case "whisper":
    guard let m = modelPath else {
        Out.emit(["type": "state", "state": "error", "detail": "--engine whisper needs --model <path>"]); exit(7)
    }
    recogniser = WhisperRecogniser(modelPath: m)
default:
    recogniser = SystemRecogniser()
}
let ear = Ear(recogniser: recogniser)
ear.start()
// (stdin loop and RunLoop.main.run(): unchanged)
```

The System path must keep the microphone request flow: `startAudio` still runs the device selection and the tap; `SystemRecogniser.begin` starts the request and timers. Diff the System path against today's code line by line before committing; behaviour must be identical.

- [ ] **Step 2: Add a `note` event to the Rust parser so the log can show model and timing lines**

In `src-tauri/src/ear.rs`, add `Note(String)` to `EarEvent`, parse `"note" => EarEvent::Note(v["text"].as_str()?.to_string())`, and add to the parser test: `assert!(matches!(parse_line(r#"{"type":"note","text":"model loaded in 1.2 s"}"#), Some(EarEvent::Note(t)) if t == "model loaded in 1.2 s"));`. In `src-tauri/src/listener.rs`'s event loop add an arm before `_ => {}`:

```rust
                ear::EarEvent::Note(text) => {
                    log::line("ear", text);
                }
```

Run `PATH="$HOME/.cargo/bin:$PATH" cargo test` from `src-tauri`; expected all green.

- [ ] **Step 3: Build and try the sidecar alone**

```
sh scripts/build-ear.sh
sh scripts/test-ear.sh
src-tauri/binaries/maya-ear-aarch64-apple-darwin --engine whisper --model /private/tmp/claude-501/-Users-tiagocorreia-dev-maya/2898aac1-15b9-4386-8ad9-8f49bb10931b/scratchpad/ggml-base.en-q5_1.bin
```

(That scratchpad model was downloaded during the spike; if missing, `curl -fsSL -o /tmp/ggml-base.en-q5_1.bin https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en-q5_1.bin`.) From a terminal the microphone may not be granted to the shell, so the meaningful checks are: `devices`, `device`, `state: loading`, the `note` with the load time, `state: listening`, and level lines; a spoken sentence, if the mic works in the terminal, produces `partial` then `final`. Type `quit` + Enter to stop. Record what was observed in the report.

- [ ] **Step 4: Commit**

```
git add ear/main.swift src-tauri/src/ear.rs src-tauri/src/listener.rs
git commit -m "feat(ear): whisper engine behind --engine, with notes for the log"
```

---

### Task 4: Model store and download in Rust

**Files:**
- Create: `src-tauri/src/models.rs`
- Modify: `src-tauri/src/lib.rs` (register module and commands)

**Interfaces:**
- Produces:
  ```rust
  pub struct WhisperModel { pub id: &'static str, pub file: &'static str, pub bytes: u64, pub sha256: &'static str, pub label: &'static str }
  pub const MODELS: &[WhisperModel];
  pub const DEFAULT_MODEL: &str = "base.en-q5_1";
  pub fn model(id: &str) -> Option<&'static WhisperModel>;
  pub fn models_dir(claude_dir: &Path) -> PathBuf;               // <claude_dir>/maya/models
  pub fn model_path(claude_dir: &Path, id: &str) -> Option<PathBuf>;
  pub fn is_downloaded(claude_dir: &Path, id: &str) -> bool;      // file exists with the exact byte count
  pub fn verify(path: &Path, m: &WhisperModel) -> Result<(), String>;   // size then sha256 (streamed)
  pub fn download(claude_dir: &Path, id: &str, progress: &dyn Fn(u64, u64)) -> Result<PathBuf, String>;
  pub fn remove(claude_dir: &Path, id: &str) -> Result<(), String>;
  #[derive(Serialize)] pub struct ModelInfo { id, label, bytes, downloaded: bool }
  pub fn list(claude_dir: &Path) -> Vec<ModelInfo>;
  ```
  Tauri commands: `list_whisper_models() -> Vec<ModelInfo>`, `download_whisper_model(id) -> Result<(), String>` (async; emits `voice-model` `{ id, received, total }` about every 500 ms and once at the end), `remove_whisper_model(id) -> Result<(), String>` (refuses the model in use while `recognizer == builtin`: "That model is in use; pick another first.").
  Event names: `voice-model`.

- [ ] **Step 1: Write the failing tests** (in `models.rs`'s test module)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn the_table_has_the_four_models_with_the_default_marked() {
        assert_eq!(MODELS.len(), 4);
        assert_eq!(model("base.en-q5_1").unwrap().file, "ggml-base.en-q5_1.bin");
        assert_eq!(model("base.en-q5_1").unwrap().bytes, 59_721_011);
        assert!(model("nope").is_none());
        assert_eq!(DEFAULT_MODEL, "base.en-q5_1");
    }

    #[test]
    fn paths_live_under_maya_models() {
        let p = model_path(Path::new("/Users/x/.claude"), "tiny.en").unwrap();
        assert_eq!(p, Path::new("/Users/x/.claude/maya/models/ggml-tiny.en.bin"));
        assert!(model_path(Path::new("/x"), "nope").is_none());
    }

    #[test]
    fn downloaded_means_present_with_the_exact_size() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path();
        assert!(!is_downloaded(claude, "tiny.en"));
        let p = model_path(claude, "tiny.en").unwrap();
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"short").unwrap();
        assert!(!is_downloaded(claude, "tiny.en"), "a truncated file is not a model");
    }

    #[test]
    fn verify_checks_size_then_sha256() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("m.bin");
        let m = WhisperModel { id: "t", file: "m.bin", bytes: 3, sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad", label: "t" };
        std::fs::write(&p, b"ab").unwrap();
        assert!(verify(&p, &m).unwrap_err().contains("size"));
        std::fs::write(&p, b"abc").unwrap();
        assert!(verify(&p, &m).is_ok(), "sha256 of abc");
        std::fs::write(&p, b"abd").unwrap();
        assert!(verify(&p, &m).unwrap_err().contains("checksum"));
    }

    #[test]
    fn a_failed_download_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path();
        // An unreachable URL: the fetch fails fast.
        let m = WhisperModel { id: "t", file: "never.bin", bytes: 1, sha256: "00", label: "t" };
        let err = fetch_to(&models_dir(claude), &m, "http://127.0.0.1:9/never.bin", &|_, _| {}).unwrap_err();
        assert!(!err.is_empty());
        assert!(!models_dir(claude).join("never.bin.part").exists());
        assert!(!models_dir(claude).join("never.bin").exists());
    }

    #[test]
    fn list_reports_downloaded_state() {
        let dir = tempfile::tempdir().unwrap();
        let l = list(dir.path());
        assert_eq!(l.len(), 4);
        assert!(l.iter().all(|m| !m.downloaded));
        assert_eq!(l[1].id, "base.en-q5_1");
    }

    #[test]
    fn remove_deletes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = model_path(dir.path(), "tiny.en").unwrap();
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::File::create(&p).unwrap().write_all(b"x").unwrap();
        remove(dir.path(), "tiny.en").unwrap();
        assert!(!p.exists());
        assert!(remove(dir.path(), "tiny.en").is_ok(), "removing a missing model is fine");
    }
}
```

- [ ] **Step 2: Run to see them fail**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test models::` from `src-tauri` (after adding `pub mod models;` to `lib.rs`). Expected: compile errors for the missing items.

- [ ] **Step 3: Implement `models.rs`**

sha256 without a new crate: shell out to `shasum -a 256 <path>` (macOS ships it), parse the first token. Download with `curl -fL --progress-bar`-free approach: spawn `curl -fsSL -o <part> <url>` and poll the `.part` file size every 500 ms from a thread for progress until the child exits. This keeps the crate dependency-free.

```rust
//! Whisper models for the built-in recogniser: the table, where they live,
//! and downloading them once, verified, into `<claude_dir>/maya/models/`.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct WhisperModel {
    pub id: &'static str,
    pub file: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    pub label: &'static str,
}

pub const MODELS: &[WhisperModel] = &[
    WhisperModel { id: "tiny.en", file: "ggml-tiny.en.bin", bytes: 77_704_715, sha256: "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f", label: "Tiny (78 MB, fastest)" },
    WhisperModel { id: "base.en-q5_1", file: "ggml-base.en-q5_1.bin", bytes: 59_721_011, sha256: "4baf70dd0d7c4247ba2b81fafd9c01005ac77c2f9ef064e00dcf195d0e2fdd2f", label: "Base, quantised (60 MB, recommended)" },
    WhisperModel { id: "base.en", file: "ggml-base.en.bin", bytes: 147_964_211, sha256: "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002", label: "Base (148 MB)" },
    WhisperModel { id: "small.en-q5_1", file: "ggml-small.en-q5_1.bin", bytes: 190_098_681, sha256: "bfdff4894dcb76bbf647d56263ea2a96645423f1669176f4844a1bf8e478ad30", label: "Small, quantised (190 MB, most accurate)" },
];

pub const DEFAULT_MODEL: &str = "base.en-q5_1";
const BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

pub fn model(id: &str) -> Option<&'static WhisperModel> {
    MODELS.iter().find(|m| m.id == id)
}

pub fn models_dir(claude_dir: &Path) -> PathBuf {
    claude_dir.join("maya").join("models")
}

pub fn model_path(claude_dir: &Path, id: &str) -> Option<PathBuf> {
    model(id).map(|m| models_dir(claude_dir).join(m.file))
}

/// Present with the exact byte count. The checksum is checked at download
/// time; re-hashing 200 MB on every listener start would be too slow.
pub fn is_downloaded(claude_dir: &Path, id: &str) -> bool {
    match (model(id), model_path(claude_dir, id)) {
        (Some(m), Some(p)) => std::fs::metadata(&p).map(|md| md.len() == m.bytes).unwrap_or(false),
        _ => false,
    }
}

fn sha256_of(path: &Path) -> Result<String, String> {
    let out = Command::new("shasum").args(["-a", "256"]).arg(path).output().map_err(|e| format!("could not run shasum: {e}"))?;
    if !out.status.success() {
        return Err(format!("shasum failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).split_whitespace().next().unwrap_or("").to_string())
}

pub fn verify(path: &Path, m: &WhisperModel) -> Result<(), String> {
    let len = std::fs::metadata(path).map_err(|e| format!("could not read {}: {e}", path.display()))?.len();
    if len != m.bytes {
        return Err(format!("wrong size: {len} bytes, expected {}", m.bytes));
    }
    let actual = sha256_of(path)?;
    if actual != m.sha256 {
        return Err(format!("checksum mismatch: {actual}"));
    }
    Ok(())
}

/// Fetches `url` to `<dir>/<file>.part`, reporting (received, total) while
/// curl runs, verifies it and renames it into place. Anything that fails
/// removes the part file.
pub fn fetch_to(dir: &Path, m: &WhisperModel, url: &str, progress: &dyn Fn(u64, u64)) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let part = dir.join(format!("{}.part", m.file));
    let target = dir.join(m.file);
    let _ = std::fs::remove_file(&part);
    let result = (|| {
        let mut child = Command::new("curl")
            .args(["-fsSL", "-o"])
            .arg(&part)
            .arg(url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not run curl: {e}"))?;
        loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                if !status.success() {
                    let mut err = String::new();
                    if let Some(mut e) = child.stderr.take() {
                        use std::io::Read;
                        let _ = e.read_to_string(&mut err);
                    }
                    return Err(format!("download failed: {}", if err.trim().is_empty() { format!("curl exited with {status}") } else { err.trim().to_string() }));
                }
                break;
            }
            let got = std::fs::metadata(&part).map(|md| md.len()).unwrap_or(0);
            progress(got, m.bytes);
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        verify(&part, m)?;
        std::fs::rename(&part, &target).map_err(|e| format!("could not move the model into place: {e}"))?;
        progress(m.bytes, m.bytes);
        Ok(target.clone())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

pub fn download(claude_dir: &Path, id: &str, progress: &dyn Fn(u64, u64)) -> Result<PathBuf, String> {
    let m = model(id).ok_or_else(|| format!("unknown model {id}"))?;
    fetch_to(&models_dir(claude_dir), m, &format!("{BASE_URL}/{}", m.file), progress)
}

pub fn remove(claude_dir: &Path, id: &str) -> Result<(), String> {
    let p = model_path(claude_dir, id).ok_or_else(|| format!("unknown model {id}"))?;
    match std::fs::remove_file(&p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("could not remove {}: {e}", p.display())),
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
    pub bytes: u64,
    pub downloaded: bool,
}

pub fn list(claude_dir: &Path) -> Vec<ModelInfo> {
    MODELS.iter().map(|m| ModelInfo { id: m.id.into(), label: m.label.into(), bytes: m.bytes, downloaded: is_downloaded(claude_dir, m.id) }).collect()
}
```

- [ ] **Step 4: Register the commands in `lib.rs`**

Add `pub mod models;` beside the other modules. Add, next to the `log_*` commands:

```rust
#[tauri::command]
fn list_whisper_models(state: TauriState<AppState>) -> Vec<models::ModelInfo> {
    let claude = state.store.lock().unwrap().claude_dir().to_path_buf();
    models::list(&claude)
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ModelProgress {
    id: String,
    received: u64,
    total: u64,
}

/// Downloads one model, reporting progress as `voice-model` events.
#[tauri::command(async)]
fn download_whisper_model(app: AppHandle, state: TauriState<AppState>, id: String) -> Result<(), String> {
    let claude = state.store.lock().unwrap().claude_dir().to_path_buf();
    log::line("app", format!("downloading whisper model {id}"));
    let handle = app.clone();
    let name = id.clone();
    models::download(&claude, &id, &move |received, total| {
        let _ = handle.emit("voice-model", ModelProgress { id: name.clone(), received, total });
    })
    .map(|_| log::line("app", format!("whisper model {id} ready")))
    .map_err(|e| {
        log::line("app", format!("whisper model {id}: {e}"));
        e
    })
}

#[tauri::command]
fn remove_whisper_model(state: TauriState<AppState>, id: String) -> Result<(), String> {
    let (claude, in_use) = {
        let store = state.store.lock().unwrap();
        (store.claude_dir().to_path_buf(), store.config.recognizer == config::Recognizer::Builtin && store.config.whisper_model == id)
    };
    if in_use {
        return Err("That model is in use; pick another first.".into());
    }
    models::remove(&claude, &id)
}
```

(`config::Recognizer` and `whisper_model` come from Task 5; to keep this task green on its own, implement Task 5's config step first if the reviewer prefers, or land the `in_use` check as `false` with a `// Task 5 wires the in-use check` comment and finish it in Task 5. The controller's ruling: do the config fields in this task's Step 4 as well, exactly as Task 5 Step 1 specifies, and Task 5 then skips that step.)

Register `list_whisper_models, download_whisper_model, remove_whisper_model` in `generate_handler!`. Add `use serde::Serialize;` if not already imported in `lib.rs`.

- [ ] **Step 5: Run the suite**

`PATH="$HOME/.cargo/bin:$PATH" cargo test` from `src-tauri`; expected all green including the seven new tests.

- [ ] **Step 6: Commit**

```
git add src-tauri/src/models.rs src-tauri/src/lib.rs src-tauri/src/config.rs
git commit -m "feat: whisper model table, verified downloads and removal"
```

---

### Task 5: Config fields, engine selection and restarts in the listener

**Files:**
- Modify: `src-tauri/src/config.rs`
- Modify: `src-tauri/src/ear.rs` (spawn signature)
- Modify: `src-tauri/src/listener.rs`
- Modify: `src-tauri/src/lib.rs` (`set_config` restart)

**Interfaces:**
- Consumes: `models::{model_path, is_downloaded, model}` (Task 4).
- Produces: `config::Recognizer { System, Builtin }` (serde lowercase), `Config.recognizer`, `Config.whisper_model: String`; `ear::Ear::spawn(device: Option<&str>, engine: EngineArgs)` where `pub enum EngineArgs { System, Whisper { model: PathBuf } }`; `listener::listening_change(before: &Config, after: &Config) -> ListenChange` with `enum ListenChange { None, Start, Stop, Restart }`; `listener::builtin_model_check(claude_dir, &Config) -> Result<PathBuf, String>` producing "Download the <label> model in Settings first." when missing.

- [ ] **Step 1: Config fields (skip if Task 4 already added them)**

In `config.rs`:

```rust
/// Which speech recogniser the listener uses.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Recognizer {
    #[default]
    System,
    Builtin,
}
```

Add to `Config` (after `interpreter_model`):

```rust
    /// System (Apple) or Built-in (whisper.cpp) recognition.
    #[serde(default)]
    pub recognizer: Recognizer,
    /// Whisper model id for the built-in recogniser.
    #[serde(default = "default_whisper_model")]
    pub whisper_model: String,
```

with `fn default_whisper_model() -> String { crate::models::DEFAULT_MODEL.into() }`, and extend `Default for Config` with `recognizer: Recognizer::System, whisper_model: crate::models::DEFAULT_MODEL.into()`. Test:

```rust
    #[test]
    fn recogniser_defaults_to_system_and_round_trips() {
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes": 5}"#).unwrap();
        assert_eq!(c.recognizer, Recognizer::System);
        assert_eq!(c.whisper_model, "base.en-q5_1");
        let c = Config { recognizer: Recognizer::Builtin, whisper_model: "tiny.en".into(), ..Default::default() };
        let text = serde_json::to_string(&c).unwrap();
        assert!(text.contains("\"recognizer\":\"builtin\""));
        assert!(text.contains("\"whisperModel\":\"tiny.en\""));
    }
```

Grep for every `Config { … }` literal in tests across the crate (`grep -rn "Config {" src-tauri/src`) and add the two fields or `..Default::default()`.

- [ ] **Step 2: Failing tests for the pure decisions** (in `listener.rs`'s test module)

```rust
    #[test]
    fn changing_the_recogniser_restarts_a_running_listener() {
        use crate::config::{Config, Recognizer};
        let off = Config::default();
        let on = Config { listen: true, ..Default::default() };
        let on_builtin = Config { listen: true, recognizer: Recognizer::Builtin, ..Default::default() };
        let on_tiny = Config { listen: true, recognizer: Recognizer::Builtin, whisper_model: "tiny.en".into(), ..Default::default() };
        let on_mic = Config { listen: true, microphone: Some("USB".into()), ..Default::default() };
        assert_eq!(listening_change(&off, &on), ListenChange::Start);
        assert_eq!(listening_change(&on, &off), ListenChange::Stop);
        assert_eq!(listening_change(&on, &on_builtin), ListenChange::Restart);
        assert_eq!(listening_change(&on_builtin, &on_tiny), ListenChange::Restart);
        assert_eq!(listening_change(&on, &on_mic), ListenChange::Restart);
        assert_eq!(listening_change(&off, &Config { recognizer: Recognizer::Builtin, ..Default::default() }), ListenChange::None, "no run to restart while off");
        assert_eq!(listening_change(&on, &Config { listen: true, completed_timeout_minutes: 5, ..Default::default() }), ListenChange::None);
    }

    #[test]
    fn refuses_a_model_file_that_is_not_the_expected_size() {
        use crate::config::{Config, Recognizer};
        let dir = tempfile::tempdir().unwrap();
        let c = Config { recognizer: Recognizer::Builtin, ..Default::default() };
        let err = builtin_model_check(dir.path(), &c).unwrap_err();
        assert!(err.contains("Download the Base, quantised (60 MB, recommended) model in Settings first."), "{err}");
        let p = crate::models::model_path(dir.path(), "base.en-q5_1").unwrap();
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"truncated").unwrap();
        assert!(builtin_model_check(dir.path(), &c).is_err(), "a truncated file is refused");
        let sys = Config::default();
        assert!(builtin_model_check(dir.path(), &sys).is_err(), "system recogniser has no model path");
    }
```

Run `cargo test listener::`; expected: compile errors.

- [ ] **Step 3: Implement**

In `listener.rs`:

```rust
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ListenChange {
    None,
    Start,
    Stop,
    Restart,
}

/// What a config save means for the listener: a change of engine, model or
/// microphone while listening restarts the run with the new settings.
pub(crate) fn listening_change(before: &config::Config, after: &config::Config) -> ListenChange {
    match (before.listen, after.listen) {
        (false, true) => ListenChange::Start,
        (true, false) => ListenChange::Stop,
        (false, false) => ListenChange::None,
        (true, true) => {
            if before.recognizer != after.recognizer || before.whisper_model != after.whisper_model || before.microphone != after.microphone {
                ListenChange::Restart
            } else {
                ListenChange::None
            }
        }
    }
}

/// The model file the built-in recogniser will load, or why it cannot start.
pub(crate) fn builtin_model_check(claude_dir: &std::path::Path, c: &config::Config) -> Result<std::path::PathBuf, String> {
    if c.recognizer != config::Recognizer::Builtin {
        return Err("the system recogniser needs no model".into());
    }
    let m = crate::models::model(&c.whisper_model).ok_or_else(|| format!("Unknown model {}; pick one in Settings.", c.whisper_model))?;
    if !crate::models::is_downloaded(claude_dir, m.id) {
        return Err(format!("Download the {} model in Settings first.", m.label));
    }
    Ok(crate::models::model_path(claude_dir, m.id).unwrap())
}
```

In `ear.rs`:

```rust
pub enum EngineArgs {
    System,
    Whisper { model: PathBuf },
}

impl Ear {
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
        // (rest unchanged)
```

In `start_listening`, replace the device/spawn lines with:

```rust
    let (device, engine) = {
        let state = app.state::<AppState>();
        let store = state.store.lock().unwrap();
        let device = store.config.microphone.clone();
        let engine = if store.config.recognizer == config::Recognizer::Builtin {
            match builtin_model_check(store.claude_dir(), &store.config) {
                Ok(model) => ear::EngineArgs::Whisper { model },
                Err(why) => {
                    drop(store);
                    log::line("listener", format!("built-in recogniser cannot start: {why}"));
                    set_voice(app, generation, |s| {
                        s.listening = false;
                        s.state = "error".into();
                        s.detail = why.clone();
                    });
                    emit_voice(app);
                    return Err(why);
                }
            }
        } else {
            ear::EngineArgs::System
        };
        (device, engine)
    };
    log::line("listener", format!("starting run {generation} ({}, microphone: {})", match &engine { ear::EngineArgs::System => "system".to_string(), ear::EngineArgs::Whisper { model } => format!("whisper {}", model.display()) }, device.as_deref().unwrap_or("automatic")));
    let (ear, rx) = match ear::Ear::spawn(device.as_deref(), engine) {
```

Also handle the new `loading` state in the event loop: the generic `State` arm already logs it; additionally set the page's state so the indicator shows it is not ready yet:

```rust
                ear::EarEvent::State { state, detail } if state == "loading" => {
                    log::line("ear", "loading the model");
                    set_voice(&handle, generation, |s| s.detail = "loading the model…".into());
                    let _ = detail;
                }
```

(place it before the generic `State` arm). When `listening` arrives the existing code path leaves `detail` as the microphone line; make the `Device` arm and this arm consistent: on `State { state: "listening" }` clear a "loading the model…" detail: add to the generic State arm `if state == "listening" { set_voice(&handle, generation, |s| if s.detail.starts_with("loading") { s.detail.clear() }); }`.

Update the Dictation message in `ear::dictation_advice` to end with "…or switch Speech recognition to Built-in in Settings." for both branches (and the test's `contains` checks still pass).

In `lib.rs` `set_config`, replace the body after validation with:

```rust
    let before = {
        let mut store = state.store.lock().unwrap();
        let before = store.config.clone();
        config::save(&store.config_path(), &config)?;
        store.config = config.clone();
        before
    };
    match listener::listening_change(&before, &config) {
        listener::ListenChange::Restart => {
            log::line("listener", "settings changed; restarting");
            let handle = app.clone();
            std::thread::spawn(move || {
                let _ = listener::start_listening(&handle);
            });
        }
        listener::ListenChange::Start => {
            let handle = app.clone();
            std::thread::spawn(move || {
                let _ = listener::start_listening(&handle);
            });
        }
        listener::ListenChange::Stop => listener::stop_listening(&app),
        listener::ListenChange::None => {}
    }
    refresh_and_emit(&app);
    Ok(config)
```

`stop_listening` becomes `pub(crate)`. `start_listening` already replaces a running ear (generation bump + stop of the old one), so Restart is just a start.

- [ ] **Step 4: Run the suite and a real start**

`PATH="$HOME/.cargo/bin:$PATH" cargo test` green. Then `pnpm tauri build --bundles app`, launch the bundle, and in the log confirm `starting run 1 (system, microphone: automatic)`.

- [ ] **Step 5: Commit**

```
git add src-tauri/src/config.rs src-tauri/src/ear.rs src-tauri/src/listener.rs src-tauri/src/lib.rs
git commit -m "feat: choose the recogniser and model in config; restart the listener when they change"
```

---

### Task 6: Settings: recogniser switch and model picker

**Files:**
- Modify: `src/settings.ts`
- Modify: `src/settings.test.ts`
- Modify: `src/settings-flow.test.ts`
- Modify: `src/styles.css`

**Interfaces:**
- Consumes: commands `list_whisper_models`, `download_whisper_model`, `remove_whisper_model`, event `voice-model` `{ id, received, total }` (Task 4); config fields `recognizer`, `whisperModel` (Task 5).
- Produces: `SettingsModel` gains `recognizer: "system" | "builtin"`, `whisperModel: string`, `models: ModelInfo[]`, `downloading: { id: string; received: number; total: number } | null`; handlers `onRecognizer(r)`, `onWhisperModel(id)`, `onDownloadModel(id)`, `onRemoveModel(id)`.

- [ ] **Step 1: Failing tests** (append to `settings.test.ts`; extend the `voiceBase` fixture with `recognizer: "system" as const, whisperModel: "base.en-q5_1", models: [], downloading: null` and the handlers helper with `onRecognizer: vi.fn(), onWhisperModel: vi.fn(), onDownloadModel: vi.fn(), onRemoveModel: vi.fn()`)

```ts
  const models = [
    { id: "tiny.en", label: "Tiny (78 MB, fastest)", bytes: 77704715, downloaded: false },
    { id: "base.en-q5_1", label: "Base, quantised (60 MB, recommended)", bytes: 59721011, downloaded: true },
  ];

  it("offers System and Built-in recognition, System first", () => {
    const h = handlers();
    const el = renderSettings({ ...voiceBase, models }, h);
    const sel = el.querySelector<HTMLSelectElement>("select[name=recognizer]")!;
    expect([...sel.options].map((o) => o.value)).toEqual(["system", "builtin"]);
    expect(sel.value).toBe("system");
    expect(el.querySelector("select[name=whisperModel]")).toBeNull();
    sel.value = "builtin";
    sel.dispatchEvent(new Event("change"));
    expect(h.onRecognizer).toHaveBeenCalledWith("builtin");
  });

  it("with Built-in, lists the models, shows which are downloaded, and downloads or removes", () => {
    const h = handlers();
    const el = renderSettings({ ...voiceBase, recognizer: "builtin", models }, h);
    const sel = el.querySelector<HTMLSelectElement>("select[name=whisperModel]")!;
    expect([...sel.options].map((o) => o.textContent)).toEqual(["Tiny (78 MB, fastest)", "Base, quantised (60 MB, recommended) ✓"]);
    expect(sel.value).toBe("base.en-q5_1");
    expect(el.querySelector("button[data-action=download-model]")).toBeNull();
    const remove = el.querySelector<HTMLButtonElement>("button[data-action=remove-model]")!;
    expect(remove.disabled).toBe(true);
    sel.value = "tiny.en";
    sel.dispatchEvent(new Event("change"));
    expect(h.onWhisperModel).toHaveBeenCalledWith("tiny.en");

    const tiny = renderSettings({ ...voiceBase, recognizer: "builtin", whisperModel: "tiny.en", models }, h);
    const dl = tiny.querySelector<HTMLButtonElement>("button[data-action=download-model]")!;
    expect(dl.textContent).toContain("Download");
    expect(dl.textContent).toContain("78 MB");
    dl.click();
    expect(h.onDownloadModel).toHaveBeenCalledWith("tiny.en");
    expect(tiny.querySelector(".settings__hint")?.textContent).toContain("Download");
  });

  it("shows download progress and disables the button meanwhile", () => {
    const el = renderSettings({ ...voiceBase, recognizer: "builtin", whisperModel: "tiny.en", models, downloading: { id: "tiny.en", received: 38852357, total: 77704715 } }, handlers());
    const bar = el.querySelector<HTMLProgressElement>("progress[name=modelDownload]")!;
    expect(bar.value).toBe(38852357);
    expect(bar.max).toBe(77704715);
    expect(el.querySelector<HTMLButtonElement>("button[data-action=download-model]")!.disabled).toBe(true);
    expect(el.querySelector(".settings__progress")?.textContent).toContain("50%");
  });
```

In `settings-flow.test.ts`, add `list_whisper_models` to the mocked invoke (`Promise.resolve([])`) and a test:

```ts
  it("applies voice-model progress events to the picker", async () => {
    const config = { completedTimeoutMinutes: 30, projectsDir: null, clonesDir: null, notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin", elevenlabsVoiceId: null, listen: false, microphone: null, interpreterModel: "haiku", recognizer: "builtin", whisperModel: "tiny.en" };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([{ id: "tiny.en", label: "Tiny", bytes: 100, downloaded: false }]);
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await initSettings();
    await flush();
    document.getElementById("settings")!.hidden = false;
    const onProgress = eventListen.mock.calls.find((c) => c[0] === "voice-model")![1] as (e: { payload: { id: string; received: number; total: number } }) => void;
    onProgress({ payload: { id: "tiny.en", received: 25, total: 100 } });
    expect(document.querySelector<HTMLProgressElement>("progress[name=modelDownload]")!.value).toBe(25);
  });
```

Run `pnpm exec vitest run src/settings.test.ts src/settings-flow.test.ts`; expected: the four new tests fail.

- [ ] **Step 2: Implement in `settings.ts`**

Model and handler additions:

```ts
export interface ModelInfo {
  id: string;
  label: string;
  bytes: number;
  downloaded: boolean;
}
export type Recognizer = "system" | "builtin";
// SettingsModel += recognizer: Recognizer; whisperModel: string; models: ModelInfo[]; downloading: { id: string; received: number; total: number } | null;
// SettingsHandlers += onRecognizer(r: Recognizer): void; onWhisperModel(id: string): void; onDownloadModel(id: string): void; onRemoveModel(id: string): void;
// ConfigJson += recognizer: Recognizer; whisperModel: string;
```

In `renderSettings`, in the Voice assistant section right after the listen toggle (and its error line), before the microphone picker:

```ts
  const recLabel = document.createElement("label");
  recLabel.textContent = "Speech recognition";
  const rec = document.createElement("select");
  rec.name = "recognizer";
  for (const [v, text] of [["system", "System (Apple)"], ["builtin", "Built-in (Whisper, runs on this Mac)"]] as const) {
    const o = document.createElement("option");
    o.value = v;
    o.textContent = text;
    rec.append(o);
  }
  rec.value = model.recognizer;
  rec.addEventListener("change", () => h.onRecognizer(rec.value === "builtin" ? "builtin" : "system"));
  recLabel.append(rec);
  assistant.append(recLabel);

  if (model.recognizer === "builtin") {
    const mb = (n: number) => `${Math.round(n / 1_000_000)} MB`;
    const chosen = model.models.find((m) => m.id === model.whisperModel);
    const modelLabel = document.createElement("label");
    modelLabel.textContent = "Model";
    const sel = document.createElement("select");
    sel.name = "whisperModel";
    for (const m of model.models) {
      const o = document.createElement("option");
      o.value = m.id;
      o.textContent = m.downloaded ? `${m.label} ✓` : m.label;
      sel.append(o);
    }
    sel.value = model.whisperModel;
    sel.addEventListener("change", () => h.onWhisperModel(sel.value));
    modelLabel.append(sel);
    assistant.append(modelLabel);

    const row = document.createElement("div");
    row.className = "settings__row";
    if (chosen && !chosen.downloaded) {
      const dl = document.createElement("button");
      dl.type = "button";
      dl.dataset.action = "download-model";
      dl.textContent = `Download (${mb(chosen.bytes)})`;
      dl.disabled = model.downloading !== null;
      dl.addEventListener("click", () => h.onDownloadModel(chosen.id));
      row.append(dl);
      if (!model.downloading) {
        const hint = document.createElement("div");
        hint.className = "settings__hint";
        hint.textContent = "Download the model once; it stays on this Mac.";
        row.append(hint);
      }
    }
    if (chosen?.downloaded) {
      const rm = document.createElement("button");
      rm.type = "button";
      rm.dataset.action = "remove-model";
      rm.textContent = "Remove";
      rm.disabled = true;
      rm.title = "The model in use cannot be removed; pick another first.";
      row.append(rm);
    }
    for (const m of model.models) {
      if (!m.downloaded || m.id === model.whisperModel) continue;
      const rm = document.createElement("button");
      rm.type = "button";
      rm.dataset.action = "remove-model";
      rm.dataset.model = m.id;
      rm.textContent = `Remove ${m.label.split(" (")[0]}`;
      rm.addEventListener("click", () => h.onRemoveModel(m.id));
      row.append(rm);
    }
    assistant.append(row);
    if (model.downloading) {
      const bar = document.createElement("progress");
      bar.setAttribute("name", "modelDownload");
      bar.max = model.downloading.total;
      bar.value = model.downloading.received;
      const pct = document.createElement("div");
      pct.className = "settings__progress";
      pct.textContent = `Downloading… ${Math.round((100 * model.downloading.received) / Math.max(1, model.downloading.total))}%`;
      assistant.append(bar, pct);
    }
  }
```

Note: `progress.max` and `.value` are numbers in jsdom; set them as numbers.

In `initSettings`: model init gains `recognizer: "system", whisperModel: "base.en-q5_1", models: [], downloading: null`; `saveConfig` mirrors `c.recognizer` and `c.whisperModel`; the initial load mirrors them too and calls `loadModels()`:

```ts
  const loadModels = async () => {
    try {
      model.models = await invoke<ModelInfo[]>("list_whisper_models");
    } catch {
      model.models = [];
    }
  };
```

Handlers:

```ts
    onRecognizer: (r) => void run(() => saveConfig({ recognizer: r })),
    onWhisperModel: (id) => void run(() => saveConfig({ whisperModel: id })),
    onDownloadModel: (id) => {
      model.downloading = { id, received: 0, total: model.models.find((m) => m.id === id)?.bytes ?? 0 };
      paint();
      void run(async () => {
        try {
          await invoke("download_whisper_model", { id });
        } finally {
          model.downloading = null;
          await loadModels();
        }
      });
    },
    onRemoveModel: (id) => void run(async () => { await invoke("remove_whisper_model", { id }); await loadModels(); }),
```

Event:

```ts
  await listen<{ id: string; received: number; total: number }>("voice-model", (e) => {
    model.downloading = e.payload.received >= e.payload.total ? null : e.payload;
    if (!panel.hidden) paint();
  });
```

CSS: `.settings__row { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; } .settings__hint, .settings__progress { color: var(--muted); font-size: 12.5px; } .settings progress { width: 100%; }`.

- [ ] **Step 3: Run the whole frontend**

`pnpm exec vitest run` and `pnpm exec tsc --noEmit`; expected green.

- [ ] **Step 4: Commit**

```
git add src/settings.ts src/settings.test.ts src/settings-flow.test.ts src/styles.css
git commit -m "feat(settings): choose System or Built-in recognition and download Whisper models"
```

---

### Task 7: CI, README and the spec's Dictation message

**Files:**
- Modify: `.github/workflows/release.yml`
- Modify: `README.md`

- [ ] **Step 1: Workflow**

The "Build the listener (both architectures)" step already runs `build-ear.sh`, which now fetches the framework. Add the Swift tests to the Test step:

```yaml
      - name: Test
        run: |
          pnpm test
          sh scripts/test-ear.sh
          cd src-tauri && cargo test
```

Run `actionlint .github/workflows/release.yml` if installed; otherwise a careful read.

- [ ] **Step 2: README**

In the Voice section, after the macOS 14 paragraph, replace the Dictation sentence with:

```
Speech recognition is Apple's by default, which needs Dictation turned on
(System Settings › Keyboard › Dictation). On a Mac where that switch is
locked by a management profile, choose **Built-in (Whisper)** under
Settings › Voice assistant › Speech recognition and download a model
(60 MB recommended). Recognition then runs on this Mac through whisper.cpp;
the one-off model download from Hugging Face is its only network access.
```

In Develop and Build, note that `pnpm ear:build` also fetches whisper.cpp's framework (53 MB, once, into `vendor/`) and that `pnpm ear:test` runs the sidecar's tests.

- [ ] **Step 3: Commit**

```
git add .github/workflows/release.yml README.md
git commit -m "docs: built-in recognition in the README; sidecar tests in CI"
```

---

### Task 8: End to end on this Mac (needs the user present)

**Files:** none unless fixes are needed.

- [ ] **Step 1:** `sh scripts/build-ear.sh && PATH="$HOME/.cargo/bin:$PATH" pnpm tauri build --bundles app`, quit the running Maya, `open` the new bundle.
- [ ] **Step 2:** Settings › Voice assistant: set Speech recognition to Built-in, keep the recommended model, press Download; watch the progress bar reach 100 % and the model show ✓. Check `~/.claude/maya/models/ggml-base.en-q5_1.bin` is 59721011 bytes.
- [ ] **Step 3:** Tick "Listen for 'Maya'". The Debug tab should show `starting run … (whisper …)`, `loading the model`, `model loaded in … s`, `listening`. Timing: expect a few seconds on first load.
- [ ] **Step 4:** Say "Maya, what's waiting on me?". Expect `partial` lines while speaking, a `final`, the wake line, the interpreter exchange, and a spoken answer. Note the transcription time from the `transcribed … in … s` line.
- [ ] **Step 5:** Say "Maya, tell voice-test to go ahead" against a scratch session; confirm with "yes"; repeat with "no". Press Try voice and confirm she does not wake herself.
- [ ] **Step 6:** Switch the recogniser back to System while listening and confirm a restart in the log (and the Dictation message under the toggle on this Mac), then back to Built-in.
- [ ] **Step 7:** Fix anything found, with a test where a unit is testable, and commit each fix separately.
