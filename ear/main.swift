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
Out.emit(["type": "devices", "names": names])

final class Ear {
    let engine = AVAudioEngine()
    let recognizer = SFSpeechRecognizer(locale: Locale(identifier: "en-GB"))!
    var request: SFSpeechAudioBufferRecognitionRequest?
    var task: SFSpeechRecognitionTask?
    var paused = false
    var lastPartial = ""
    /// When `lastPartial` last changed; a transcript quiet for `endpointAfter` is an utterance.
    var lastChangeAt = Date()
    /// Set by the 50 s timer; the rotation waits for a quiet moment.
    var rotateDue = false
    var restartTimer: Timer?
    /// When the current request began, and how many requests in a row died
    /// within two seconds: recognition that fails instantly (Dictation off,
    /// a missing on-device model) is reported, then given up on.
    var requestBegan = Date()
    var instantFailures = 0
    var lastErrorMessage = ""
    var endpointTimer: Timer?
    var lastLevelAt: TimeInterval = 0
    let endpointAfter: TimeInterval = 1.2

    func start() {
        SFSpeechRecognizer.requestAuthorization { status in
            guard status == .authorized else {
                Out.emit(["type": "state", "state": "error", "detail": "speech recognition not authorised (\(status.rawValue))"]); exit(2)
            }
            DispatchQueue.main.async { self.startAudio() }
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
            guard !self.paused else { return }
            self.request?.append(buffer)
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
    }

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
