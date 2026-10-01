//! Mono audio at any rate to the 16 kHz whisper.cpp wants, streaming.
//!
//! Each output sample is the mean of the input samples in its window (a box
//! filter, which keeps most aliasing out of speech), found by stepping a
//! fractional position through the input: enough for recognition, with no
//! state beyond the unfinished window.

pub const TARGET_RATE: u32 = 16_000;

pub struct Resampler {
    /// Input samples per output sample.
    step: f64,
    /// Where the next output window ends, in input samples from `acc`'s start.
    next_end: f64,
    acc: Vec<f32>,
}

impl Resampler {
    pub fn new(input_rate: u32) -> Resampler {
        let step = input_rate as f64 / TARGET_RATE as f64;
        Resampler { step, next_end: step, acc: Vec::new() }
    }

    /// Takes the next input samples and returns the 16 kHz ones they complete.
    pub fn push(&mut self, input: &[f32]) -> Vec<f32> {
        self.acc.extend_from_slice(input);
        let mut out = Vec::with_capacity((input.len() as f64 / self.step) as usize + 1);
        let mut start = 0.0f64;
        while self.next_end <= self.acc.len() as f64 {
            let a = start.floor() as usize;
            let b = (self.next_end.ceil() as usize).min(self.acc.len()).max(a + 1);
            let window = &self.acc[a..b];
            out.push(window.iter().sum::<f32>() / window.len() as f32);
            start = self.next_end;
            self.next_end += self.step;
        }
        // Keep only the unfinished window.
        let used = start.floor() as usize;
        self.acc.drain(..used);
        self.next_end -= used as f64;
        out
    }
}

/// Interleaved frames of `channels` channels, averaged into mono.
pub fn downmix(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved.chunks_exact(channels).map(|f| f.iter().sum::<f32>() / channels as f32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixteen_khz_passes_through() {
        let mut r = Resampler::new(16_000);
        let input: Vec<f32> = (0..100).map(|i| i as f32).collect();
        assert_eq!(r.push(&input), input);
    }

    #[test]
    fn forty_eight_khz_is_every_three_samples_averaged() {
        let mut r = Resampler::new(48_000);
        assert_eq!(r.push(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]), [2.0, 5.0]);
        // The seventh sample waits for its window to complete.
        assert_eq!(r.push(&[8.0, 9.0]), [8.0]);
    }

    #[test]
    fn any_rate_gives_the_right_count_across_chunks() {
        for rate in [44_100u32, 48_000, 22_050, 96_000, 8_000] {
            let mut r = Resampler::new(rate);
            let (mut n, chunk) = (0, (rate / 100) as usize);
            for _ in 0..100 {
                n += r.push(&vec![0.5; chunk]).len();
            }
            // As many 16 kHz samples as the input's length gives, give or take a window.
            let expected = (100 * chunk) as f64 * TARGET_RATE as f64 / rate as f64;
            assert!((n as f64 - expected).abs() <= 2.0, "{rate}: {n} for {expected}");
        }
    }

    #[test]
    fn a_constant_signal_stays_constant() {
        let mut r = Resampler::new(44_100);
        assert!(r.push(&vec![0.25; 4410]).iter().all(|s| (s - 0.25).abs() < 1e-6));
    }

    #[test]
    fn downmix_averages_channels() {
        assert_eq!(downmix(&[1.0, 3.0, 2.0, 4.0], 2), [2.0, 3.0]);
        assert_eq!(downmix(&[1.0, 2.0], 1), [1.0, 2.0]);
    }
}
