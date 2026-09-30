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

func runTests() {
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
        check(drive(&v, 0.002, 36).isEmpty, "quiet run restarts after a loud frame")
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
}

@main
struct TestRunner {
    static func main() {
        runTests()
    }
}
