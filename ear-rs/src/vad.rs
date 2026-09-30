//! Energy-based voice activity detection, as `ear/vad.swift`.

#[derive(Debug, Clone, PartialEq)]
pub enum VadEvent {
    None,
    Started,
    PartialDue,
    Ended(&'static str),
}

/// Frames of 20 ms. Speech starts after `START_LOUD_FRAMES` frames above
/// the threshold and ends after `END_QUIET_FRAMES` below it, or at
/// `MAX_FRAMES`. The threshold follows the room: `floor` is the running
/// minimum of frame RMS, nudged up by `FLOOR_DECAY` every frame, so a
/// louder room is absorbed within a second or two and a quieter one at
/// once; speech is `floor + MARGIN`.
#[derive(Debug, Clone)]
pub struct Vad {
    pub floor: f32,
    pub speaking: bool,
    loud_run: u32,
    quiet_run: u32,
    frames_in_utterance: u32,
    frames_since_partial: u32,
}

pub const FRAME_SAMPLES: usize = 320;
pub const PRE_ROLL_FRAMES: usize = 30;
pub const END_QUIET_FRAMES: u32 = 40;
pub const MAX_FRAMES: u32 = 600;
pub const PARTIAL_EVERY_FRAMES: u32 = 75;
pub const START_LOUD_FRAMES: u32 = 3;
pub const MARGIN: f32 = 0.004;
pub const FLOOR_DECAY: f32 = 0.0005;

impl Default for Vad {
    fn default() -> Self {
        Vad { floor: 1.0, speaking: false, loud_run: 0, quiet_run: 0, frames_in_utterance: 0, frames_since_partial: 0 }
    }
}

impl Vad {
    pub fn push(&mut self, rms: f32) -> VadEvent {
        self.floor = (self.floor + FLOOR_DECAY).min(rms);
        let loud = rms > self.floor + MARGIN;
        if !self.speaking {
            self.loud_run = if loud { self.loud_run + 1 } else { 0 };
            if self.loud_run >= START_LOUD_FRAMES {
                self.speaking = true;
                self.loud_run = 0;
                self.quiet_run = 0;
                self.frames_in_utterance = 0;
                self.frames_since_partial = 0;
                return VadEvent::Started;
            }
            return VadEvent::None;
        }
        self.frames_in_utterance += 1;
        self.frames_since_partial += 1;
        self.quiet_run = if loud { 0 } else { self.quiet_run + 1 };
        if self.quiet_run >= END_QUIET_FRAMES {
            self.speaking = false;
            return VadEvent::Ended("silence");
        }
        if self.frames_in_utterance >= MAX_FRAMES {
            self.speaking = false;
            return VadEvent::Ended("cap");
        }
        if self.frames_since_partial >= PARTIAL_EVERY_FRAMES {
            self.frames_since_partial = 0;
            return VadEvent::PartialDue;
        }
        VadEvent::None
    }
}

const FILLERS: &[&str] = &["thank you", "thanks for watching", "blank_audio", "you", "bye", "the end", "subtitles by the amara.org community"];

/// Whisper invents these on silence; a result that is nothing but one of
/// them is noise.
pub fn is_filler(text: &str) -> bool {
    let cleaned: String = text.to_lowercase().chars().filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '_' | '.')).collect();
    let cleaned = cleaned.trim().trim_matches('.');
    cleaned.is_empty() || FILLERS.contains(&cleaned)
}

#[cfg(test)]
mod tests {
    //! The Swift ear's `vadtest.swift`, case for case.
    use super::*;

    fn drive(v: &mut Vad, rms: f32, frames: usize) -> Vec<VadEvent> {
        (0..frames).map(|_| v.push(rms)).filter(|e| *e != VadEvent::None).collect()
    }

    #[test]
    fn starts_after_three_loud_frames_not_one() {
        let mut v = Vad::default();
        drive(&mut v, 0.002, 50);
        assert!(drive(&mut v, 0.05, 2).is_empty());
        assert_eq!(drive(&mut v, 0.05, 1), [VadEvent::Started]);
        assert!(v.speaking);
    }

    #[test]
    fn ends_after_forty_quiet_frames() {
        let mut v = Vad::default();
        drive(&mut v, 0.002, 50);
        drive(&mut v, 0.05, 10);
        assert!(drive(&mut v, 0.002, 39).is_empty());
        assert_eq!(drive(&mut v, 0.002, 1), [VadEvent::Ended("silence")]);
        assert!(!v.speaking);
    }

    #[test]
    fn a_loud_frame_resets_the_quiet_count() {
        let mut v = Vad::default();
        drive(&mut v, 0.002, 50);
        drive(&mut v, 0.05, 10);
        drive(&mut v, 0.002, 30);
        drive(&mut v, 0.05, 1);
        assert!(drive(&mut v, 0.002, 36).is_empty());
    }

    #[test]
    fn caps_at_twelve_seconds_with_partials_every_one_and_a_half() {
        let mut v = Vad::default();
        drive(&mut v, 0.002, 50);
        assert_eq!(drive(&mut v, 0.06, 3), [VadEvent::Started]);
        let events: Vec<VadEvent> = (0..600).map(|i| v.push(if i % 2 == 0 { 0.06 } else { 0.02 })).filter(|e| *e != VadEvent::None).collect();
        assert!(events.contains(&VadEvent::Ended("cap")));
        assert!(!events.contains(&VadEvent::Ended("silence")));
        assert_eq!(events.iter().filter(|e| **e == VadEvent::PartialDue).count(), 7);
    }

    #[test]
    fn a_constant_level_becomes_the_floor() {
        let mut v = Vad::default();
        drive(&mut v, 0.002, 50);
        drive(&mut v, 0.03, 3);
        assert!(drive(&mut v, 0.03, 200).contains(&VadEvent::Ended("silence")));
    }

    #[test]
    fn fillers_are_dropped_and_sentences_kept() {
        assert!(is_filler("Thank you."));
        assert!(is_filler(" [BLANK_AUDIO] "));
        assert!(is_filler("you"));
        assert!(is_filler(""));
        assert!(!is_filler("Thank you, Maya, tell hexgrid to go ahead"));
        assert!(!is_filler("Maya what's waiting on me"));
    }
}
