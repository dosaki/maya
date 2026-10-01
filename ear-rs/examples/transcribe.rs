//! Runs a 16-bit PCM WAV through the ear's pipeline without a microphone:
//! resampled to 16 kHz, cut into utterances by the VAD, each transcribed as
//! the ear would. For checking a model or a build:
//! `cargo run -p maya-ear --release --example transcribe -- <model.bin> <file.wav>`.

#[cfg(windows)]
fn main() {
    use maya_ear::resample::{downmix, Resampler};
    use maya_ear::vad::{is_filler, Vad, VadEvent, FRAME_SAMPLES, PRE_ROLL_FRAMES};
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    let args: Vec<String> = std::env::args().skip(1).collect();
    let [model, wav] = args.as_slice() else {
        eprintln!("usage: transcribe <model.bin> <file.wav>");
        std::process::exit(2)
    };
    let (rate, channels, samples) = read_wav(&std::fs::read(wav).expect("read the wav"));
    let audio = Resampler::new(rate).push(&downmix(&samples, channels));
    // A second of silence after the file, so the last utterance ends by silence.
    let audio: Vec<f32> = audio.into_iter().chain(std::iter::repeat_n(0.0, 16_000)).collect();

    whisper_rs::install_logging_hooks();
    let ctx = WhisperContext::new_with_params(model, WhisperContextParameters::default()).expect("load the model");
    let mut state = ctx.create_state().expect("a whisper state");
    let transcribe = |state: &mut whisper_rs::WhisperState, audio: &[f32]| -> String {
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_print_progress(false);
        params.set_no_timestamps(true);
        params.set_single_segment(true);
        params.set_suppress_blank(true);
        params.set_suppress_nst(true);
        params.set_n_threads(4);
        params.set_language(Some("en"));
        state.full(params, audio).expect("whisper_full");
        state.as_iter().filter_map(|s| s.to_str_lossy().ok().map(|t| t.into_owned())).collect::<String>().trim().to_string()
    };

    let (mut vad, mut pre_roll, mut utterance) = (Vad::default(), Vec::new(), Vec::new());
    for frame in audio.chunks_exact(FRAME_SAMPLES) {
        match vad.push(maya_ear::rms(frame)) {
            VadEvent::None if vad.speaking => utterance.extend_from_slice(frame),
            VadEvent::None => {
                pre_roll.extend_from_slice(frame);
                let keep = PRE_ROLL_FRAMES * FRAME_SAMPLES;
                if pre_roll.len() > keep {
                    pre_roll.drain(..pre_roll.len() - keep);
                }
            }
            VadEvent::Started => {
                utterance = std::mem::take(&mut pre_roll);
                utterance.extend_from_slice(frame);
            }
            VadEvent::PartialDue => utterance.extend_from_slice(frame),
            VadEvent::Ended(reason) => {
                utterance.extend_from_slice(frame);
                let t0 = std::time::Instant::now();
                let text = transcribe(&mut state, &utterance);
                println!("utterance {:.1} s ({reason}) in {:.2} s: {text:?}{}", utterance.len() as f64 / 16_000.0, t0.elapsed().as_secs_f64(), if is_filler(&text) { " [filler]" } else { "" });
                utterance.clear();
            }
        }
    }
}

/// (sample rate, channels, samples in -1..1) of a 16-bit PCM WAV.
#[cfg(windows)]
fn read_wav(bytes: &[u8]) -> (u32, usize, Vec<f32>) {
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    assert!(&bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE", "not a WAV file");
    let (mut i, mut rate, mut channels) = (12, 0, 0);
    while i + 8 <= bytes.len() {
        let (id, len) = (&bytes[i..i + 4], u32_at(i + 4) as usize);
        let body = i + 8;
        if id == b"fmt " {
            assert_eq!(u16_at(body + 14), 16, "16-bit PCM only");
            channels = u16_at(body + 2) as usize;
            rate = u32_at(body + 4);
        } else if id == b"data" {
            let data = &bytes[body..(body + len).min(bytes.len())];
            return (rate, channels, data.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect());
        }
        i = body + len + (len & 1);
    }
    panic!("no data chunk")
}

#[cfg(not(windows))]
fn main() {
    eprintln!("The Rust ear runs on Windows.");
}
