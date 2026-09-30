// ear/main.swift — Maya's ear: on-device speech recognition streamed as JSON lines.
import AVFoundation
import Foundation
import Speech
import whisper

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
/// The banned list is absolute: it is applied before honouring a preferred name,
/// so a banned `--device` request falls through to the automatic choice.
func pickDevice(_ names: [String], preferred: String?) -> String? {
    let banned = ["blackhole", "remote sound", "soundflower", "loopback"]
    let ok = names.filter { n in !banned.contains { n.lowercased().contains($0) } }
    if let p = preferred, ok.contains(p) { return p }
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
    /// True once `begin` has finished getting ready to transcribe.
    var ready: Bool { get }
    /// Called on every tap callback while paused — the same thread as
    /// `accept`, so a recogniser that buffers state across calls can defer
    /// clearing it to the next `accept` without any cross-thread mutation.
    func paused()
}

/// Apple's on-device recogniser: today's behaviour, unchanged.
final class SystemRecogniser: Recogniser {
    let recognizer = SFSpeechRecognizer(locale: Locale(identifier: "en-GB"))!
    var request: SFSpeechAudioBufferRecognitionRequest?
    var task: SFSpeechRecognitionTask?
    var lastPartial = ""
    /// When `lastPartial` last changed; a transcript quiet for `endpointAfter` is an utterance.
    var lastChangeAt = Date()
    /// Set by the 50 s timer; the rotation waits for a quiet moment.
    var rotateDue = false
    var restartTimer: Timer?
    var endpointTimer: Timer?
    /// When the current request began, and how many requests in a row died
    /// within two seconds: recognition that fails instantly (Dictation off,
    /// a missing on-device model) is reported, then given up on.
    var requestBegan = Date()
    var instantFailures = 0
    var lastErrorMessage = ""
    let endpointAfter: TimeInterval = 1.2
    var ready = false

    func begin(format: AVAudioFormat) {
        beginRequest()
        // On-device requests are capped around a minute: rotate before the cap,
        // but only at a quiet moment so a sentence is never split.
        restartTimer = Timer.scheduledTimer(withTimeInterval: 50, repeats: true) { _ in
            self.rotateDue = true
            self.tick()
        }
        // The recogniser keeps one growing transcript across pauses, so Maya
        // end-points utterances herself: a partial unchanged for 1.2 s is final.
        endpointTimer = Timer.scheduledTimer(withTimeInterval: 0.2, repeats: true) { _ in self.tick() }
        Out.emit(["type": "state", "state": "listening"])
        ready = true
    }

    func accept(_ buffer: AVAudioPCMBuffer) {
        request?.append(buffer)
    }

    func paused() {}

    /// Emits a quiet partial as final and starts a new request; also performs
    /// a due rotation once nothing is being said.
    func tick() {
        let quiet = Date().timeIntervalSince(lastChangeAt) >= endpointAfter
        if !lastPartial.isEmpty {
            if quiet { rotate() }
        } else if rotateDue {
            rotate()
        }
    }

    func beginRequest() {
        let req = SFSpeechAudioBufferRecognitionRequest()
        req.shouldReportPartialResults = true
        req.requiresOnDeviceRecognition = true
        request = req
        lastPartial = ""
        lastChangeAt = Date()
        requestBegan = Date()
        // Captured by the completion handler below so it can tell whether it belongs to
        // the still-current task. A superseded task (rotate() clears self.task before
        // cancelling) must not emit or restart anything: its late final would repeat
        // the segment rotate() already emitted. Handlers run on the main queue.
        var newTask: SFSpeechRecognitionTask?
        newTask = recognizer.recognitionTask(with: req) { result, error in
            guard self.task === newTask else { return }
            if let r = result {
                self.instantFailures = 0
                let text = r.bestTranscription.formattedString
                if r.isFinal {
                    if !text.isEmpty { Out.emit(["type": "final", "text": text]) }
                    self.lastPartial = ""
                    // This request is finished: listen on with a new one.
                    self.task = nil
                    DispatchQueue.main.async { self.beginRequestIfIdle() }
                    return
                } else if text != self.lastPartial {
                    self.lastPartial = text
                    self.lastChangeAt = Date()
                    Out.emit(["type": "partial", "text": text])
                }
            }
            if let e = error {
                // The request ended: a silence timeout after a long life is
                // routine; an instant failure is not, and five in a row is fatal.
                let lived = Date().timeIntervalSince(self.requestBegan)
                let ns = e as NSError
                let message = "\(ns.domain) \(ns.code): \(e.localizedDescription)"
                // On-device recognition rides on Dictation: with it off, every
                // request fails at once. Say what to do and stop.
                if ns.domain == "kLSRErrorDomain" && ns.code == 201 {
                    Out.emit(["type": "state", "state": "error", "detail": "Dictation is off. Turn it on in System Settings \u{203A} Keyboard \u{203A} Dictation, then listen again."]); exit(6)
                }
                if lived < 2 { self.instantFailures += 1 } else { self.instantFailures = 0 }
                if self.instantFailures >= 5 {
                    Out.emit(["type": "state", "state": "error", "detail": "speech recognition keeps failing: \(message)"]); exit(5)
                }
                if message != self.lastErrorMessage || lived < 2 {
                    self.lastErrorMessage = message
                    Out.emit(["type": "state", "state": "warning", "detail": "recognition ended after \(String(format: "%.1f", lived)) s: \(message)"])
                }
                if !self.lastPartial.isEmpty { Out.emit(["type": "final", "text": self.lastPartial]) }
                self.lastPartial = ""
                self.task = nil
                DispatchQueue.main.async { self.beginRequestIfIdle() }
            }
        }
        task = newTask
    }

    /// A deferred restart after a request ended on its own; skipped when
    /// rotate() has already begun a new one in the meantime.
    func beginRequestIfIdle() {
        if task == nil { beginRequest() }
    }

    /// Ends the current request, emitting its partial as final, and begins a new one.
    func rotate() {
        rotateDue = false
        if !lastPartial.isEmpty { Out.emit(["type": "final", "text": lastPartial]) }
        request?.endAudio()
        let old = task
        task = nil
        old?.cancel()
        beginRequest()
    }
}

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
    /// Set by `paused()` on the tap thread; consumed by `accept`, also on the
    /// tap thread, so `pending`/`preRoll`/`utterance`/`vad` are only ever
    /// touched from that one thread and never race a pause against a buffer.
    private var needsReset = false
    /// The last emitted partial's text, so an unchanged partial is not
    /// re-emitted (and re-logged) every 1.5 s in a noisy room.
    private var lastPartialText = ""
    /// Bumped once per pause, under `epochLock`. A transcription already
    /// queued on `work` when the pause lands can still finish after it — the
    /// Rust side may have already drained "heard" for the read-back by
    /// then — so the safety rule "nothing before the read-back confirms an
    /// action" needs every result to know whether a pause happened while it
    /// was in flight.
    private let epochLock = NSLock()
    private var epoch = 0

    init(modelPath: String) { self.modelPath = modelPath }

    var ready: Bool { ctx != nil }

    func begin(format: AVAudioFormat) {
        converter = AVAudioConverter(from: format, to: target)
        Out.emit(["type": "state", "state": "loading"])
        work.async {
            var cparams = whisper_context_default_params()
            cparams.use_gpu = true
            let t0 = Date()
            guard let c = whisper_init_from_file_with_params(self.modelPath, cparams) else {
                Out.emit(["type": "state", "state": "error", "detail": "could not load model: \(self.modelPath)"])
                // Same _exit path as quit(): whisper.cpp's Metal backend can
                // abort in its own static destructors once a context has
                // held GPU residency sets, and exit() would run those.
                fflush(stdout); _exit(8)
            }
            self.ctx = c
            Out.emit(["type": "note", "text": String(format: "model loaded in %.1f s", Date().timeIntervalSince(t0))])
            Out.emit(["type": "state", "state": "listening"])
        }
    }

    /// Called on the tap thread for every buffer while paused; only records
    /// that a reset is due — see `needsReset`. Bumps the pause epoch once
    /// per pause (not on every buffer while it stays paused), so any
    /// transcription already queued for this utterance is dropped instead
    /// of possibly landing after the read-back.
    func paused() {
        if !needsReset {
            epochLock.lock(); epoch += 1; epochLock.unlock()
        }
        needsReset = true
    }

    func accept(_ buffer: AVAudioPCMBuffer) {
        // Deferred from `paused()`: cleared here, on the same (tap) thread,
        // so nothing said during the gap bleeds into what's transcribed
        // after resuming, without mutating these buffers from two threads.
        if needsReset {
            needsReset = false
            pending = []
            preRoll = []
            utterance = []
            vad = Vad()
            lastPartialText = ""
        }
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
        // Captured now, on the tap thread, at the moment this transcription
        // is queued — not when it finishes — so a pause landing while it
        // runs on `work` is detected below.
        epochLock.lock(); let capturedEpoch = epoch; epochLock.unlock()
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
            params.suppress_nst = true
            params.n_threads = 4
            let t0 = Date()
            let rc: Int32 = "en".withCString { lang in
                params.language = lang
                return audio.withUnsafeBufferPointer { whisper_full(ctx, params, $0.baseAddress, Int32(audio.count)) }
            }
            guard rc == 0 else {
                Out.emit(["type": "state", "state": "warning", "detail": "whisper_full failed: \(rc)"]); return
            }
            var text = ""
            for i in 0..<whisper_full_n_segments(ctx) { text += String(cString: whisper_full_get_segment_text(ctx, i)) }
            text = text.trimmingCharacters(in: .whitespacesAndNewlines)
            if final {
                Out.emit(["type": "note", "text": String(format: "transcribed %.1f s in %.2f s", Double(audio.count) / 16000, Date().timeIntervalSince(t0))])
            }
            // Just before emitting: if a pause happened while this ran, the
            // Rust side may already have drained "heard" for a read-back, so
            // this result — final or partial — must never surface and risk
            // confirming a pending action with a stray word from before it.
            self.epochLock.lock(); let epochChanged = self.epoch != capturedEpoch; self.epochLock.unlock()
            if epochChanged {
                Out.emit(["type": "note", "text": "dropped a result from before a pause"])
                return
            }
            if isFiller(text) {
                if final && !text.isEmpty { Out.emit(["type": "note", "text": "dropped filler: \(text)"]) }
                return
            }
            if !final {
                if text == self.lastPartialText { return }
                self.lastPartialText = text
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
        // Never fall back to the system default input: it may be a virtual device.
        guard let name = pickDevice(names, preferred: preferred) else {
            Out.emit(["type": "state", "state": "error", "detail": "no non-virtual microphone"]); exit(4)
        }
        if let dev = inputDevices().first(where: { $0.localizedName == name }) {
            var id = AudioDeviceID(0)
            var addr = AudioObjectPropertyAddress(mSelector: kAudioHardwarePropertyTranslateUIDToDevice, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
            var uidRef: CFString = dev.uniqueID as CFString
            var size = UInt32(MemoryLayout<AudioDeviceID>.size)
            _ = withUnsafeMutablePointer(to: &uidRef) { p in
                AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &addr, UInt32(MemoryLayout<CFString>.size), p, &size, &id)
            }
            if id != 0, let unit = engine.inputNode.audioUnit {
                var devId = id
                AudioUnitSetProperty(unit, kAudioOutputUnitProperty_CurrentDevice, kAudioUnitScope_Global, 0, &devId, UInt32(MemoryLayout<AudioDeviceID>.size))
            }
            Out.emit(["type": "device", "name": name])
        }
        let input = engine.inputNode
        let format = input.outputFormat(forBus: 0)
        input.installTap(onBus: 0, bufferSize: 2048, format: format) { buffer, _ in
            guard !self.paused else {
                // Still called on every buffer while paused so a recogniser
                // can notice the pause on this same thread (see `paused()`).
                self.recogniser.paused()
                return
            }
            self.recogniser.accept(buffer)
            if let ch = buffer.floatChannelData?[0] {
                let n = Int(buffer.frameLength)
                var sum: Float = 0
                for i in 0..<n { sum += ch[i] * ch[i] }
                let rms = n > 0 ? sqrt(sum / Float(n)) : 0
                let now = Date().timeIntervalSince1970
                if now - self.lastLevelAt >= 0.2 {
                    self.lastLevelAt = now
                    // Typical speaking RMS is small (roughly 0.05-0.15), so scale it up by
                    // 8x to fill a 0-1 range that's usable for a UI level meter.
                    Out.emit(["type": "level", "value": min(1.0, Double(rms) * 8)])
                }
            }
        }
        engine.prepare()
        do { try engine.start() } catch {
            Out.emit(["type": "state", "state": "error", "detail": "audio engine: \(error.localizedDescription)"]); exit(3)
        }
        recogniser.begin(format: format)
    }

    func setPaused(_ p: Bool) {
        paused = p
        if p {
            Out.emit(["type": "state", "state": "paused"])
        } else {
            Out.emit(["type": "state", "state": recogniser.ready ? "listening" : "loading"])
        }
    }
}

let recogniser: Recogniser
switch engineName {
case "whisper":
    guard let m = modelPath else {
        Out.emit(["type": "state", "state": "error", "detail": "--engine whisper needs --model <path>"]); exit(7)
    }
    // Checked here, before the microphone (and so the audio engine) ever
    // starts: a missing model is always fatal, so there is no reason to ask
    // for the mic first.
    guard FileManager.default.fileExists(atPath: m) else {
        Out.emit(["type": "state", "state": "error", "detail": "model file not found: \(m)"]); exit(7)
    }
    recogniser = WhisperRecogniser(modelPath: m)
default:
    recogniser = SystemRecogniser()
}
let ear = Ear(recogniser: recogniser)
ear.start()

// _exit(0), not exit(0): whisper.cpp's Metal backend aborts inside its own
// static destructors when exit() runs them, so quitting skips that path
// entirely (no teardown needed: nothing here holds anything worth freeing
// once the process is going away), after flushing the one thing this
// process buffers (stdout).
func quit() -> Never {
    fflush(stdout)
    _exit(0)
}

DispatchQueue.global().async {
    while let line = readLine() {
        switch line.trimmingCharacters(in: .whitespaces) {
        case "pause": DispatchQueue.main.async { ear.setPaused(true) }
        case "resume": DispatchQueue.main.async { ear.setPaused(false) }
        case "quit": quit()
        default: break
        }
    }
    quit()
}
RunLoop.main.run()
