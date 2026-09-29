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
    var restartTimer: Timer?
    var lastLevelAt: TimeInterval = 0

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
        // Captured by the completion handler below so it can tell whether it belongs to
        // the still-current task; a handler whose task has since been superseded (by
        // rotate(), which clears self.task before cancelling) must not restart anything.
        var newTask: SFSpeechRecognitionTask?
        newTask = recognizer.recognitionTask(with: req) { result, error in
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
            if error != nil && self.task === newTask {
                // The request ended (silence timeout): flush the partial as final and start again.
                if !self.lastPartial.isEmpty { Out.emit(["type": "final", "text": self.lastPartial]) }
                DispatchQueue.main.async { self.beginRequest() }
            }
        }
        task = newTask
    }

    func rotate() {
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
