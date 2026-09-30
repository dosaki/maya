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
/// RMS, nudged upward by `floorDecay` (0.0005) every 20 ms frame — 0.025
/// RMS/s — so a quieter room is followed within a second or two, and a
/// louder one immediately (`floor` is clamped down to `rms`, not decayed
/// up to it); speech is `floor + margin`.
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
