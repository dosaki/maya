//! Maya's ear on Windows and Linux: the microphone through cpal (WASAPI,
//! ALSA), speech found by the energy VAD and transcribed by whisper.cpp,
//! streamed as JSON lines. It speaks the Swift ear's protocol
//! (`ear/main.swift`) line for line, so the app's listener drives either one:
//!
//! - arguments: `[--device <name>] --engine whisper --model <path>`, or
//!   `--selftest`;
//! - stdout: `devices`, `device`, `state`, `partial`, `final`, `level` and
//!   `note` objects, one per line;
//! - stdin: `pause`, `resume`, `quit` (EOF quits too);
//! - exit codes: 3 audio, 4 no microphone, 7 model missing, 8 model load.

#[cfg(any(windows, target_os = "linux"))]
fn main() {
    ear::run()
}

#[cfg(not(any(windows, target_os = "linux")))]
fn main() {
    eprintln!("This ear runs on Windows and Linux; macOS has the Swift one in ear/.");
    std::process::exit(2)
}

#[cfg(any(windows, target_os = "linux"))]
mod ear {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use maya_ear::resample::{downmix, Resampler, TARGET_RATE};
    use maya_ear::vad::{is_filler, Vad, VadEvent, FRAME_SAMPLES, PRE_ROLL_FRAMES};
    use serde_json::{json, Value};
    use std::io::{BufRead, Write};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{mpsc, Arc, Mutex};
    use std::time::Instant;
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    static OUT: Mutex<()> = Mutex::new(());

    fn emit(v: Value) {
        let _g = OUT.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{v}");
        let _ = out.flush();
    }

    fn state(s: &str, detail: Option<&str>) {
        match detail {
            Some(d) => emit(json!({"type": "state", "state": s, "detail": d})),
            None => emit(json!({"type": "state", "state": s})),
        }
    }

    fn note(text: String) {
        emit(json!({"type": "note", "text": text}));
    }

    fn fail(detail: &str, code: i32) -> ! {
        state("error", Some(detail));
        std::process::exit(code)
    }

    fn arg(args: &[String], name: &str) -> Option<String> {
        args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
    }

    fn device_name(d: &cpal::Device) -> Option<String> {
        d.description().ok().map(|d| d.name().to_string())
    }

    fn inputs(host: &cpal::Host) -> Vec<(String, cpal::Device)> {
        host.input_devices().map(|ds| ds.filter_map(|d| Some((device_name(&d)?, d))).collect()).unwrap_or_default()
    }

    pub fn run() {
        let args: Vec<String> = std::env::args().skip(1).collect();
        // whisper.cpp logs through these hooks, which go nowhere without a
        // logger, instead of writing to stderr.
        whisper_rs::install_logging_hooks();
        let host = cpal::default_host();
        let devices = inputs(&host);
        let names: Vec<String> = devices.iter().map(|(n, _)| n.clone()).collect();
        if args.iter().any(|a| a == "--selftest") {
            // No system recogniser on Windows and Linux: only the built-in one.
            emit(json!({"type": "selftest", "available": false, "onDevice": false, "speechAuth": 0, "devices": names}));
            return;
        }
        emit(json!({"type": "devices", "names": names}));
        let engine = arg(&args, "--engine").unwrap_or_else(|| "whisper".into());
        if engine != "whisper" {
            fail("only the built-in (whisper) recogniser runs on Windows and Linux", 7);
        }
        let Some(model) = arg(&args, "--model") else { fail("--engine whisper needs --model <path>", 7) };
        // Checked before the microphone opens: a missing model is always fatal.
        if !std::path::Path::new(&model).is_file() {
            fail(&format!("model file not found: {model}"), 7);
        }
        let default = host.default_input_device().as_ref().and_then(device_name);
        let Some(name) = maya_ear::pick_device(&names, arg(&args, "--device").as_deref(), default.as_deref()) else {
            fail("no non-virtual microphone", 4)
        };
        let device = devices.into_iter().find(|(n, _)| *n == name).map(|(_, d)| d).unwrap_or_else(|| fail("no non-virtual microphone", 4));
        emit(json!({"type": "device", "name": name}));

        let paused = Arc::new(AtomicBool::new(false));
        let epoch = Arc::new(AtomicU64::new(0));
        let ready = Arc::new(AtomicBool::new(false));
        let (audio_tx, audio_rx) = mpsc::channel::<Vec<f32>>();
        let stream = open_stream(&device, audio_tx).unwrap_or_else(|e| fail(&format!("audio engine: {e}"), 3));
        if let Err(e) = stream.play() {
            fail(&format!("audio engine: {e}"), 3);
        }
        let rate = device.default_input_config().map(|c| c.sample_rate()).unwrap_or(48_000);

        let (job_tx, job_rx) = mpsc::channel::<Job>();
        // Set while a transcription is queued or running; a partial is
        // skipped while it is, a final always queues.
        let busy = Arc::new(AtomicBool::new(false));
        {
            let (ready, epoch, busy) = (ready.clone(), epoch.clone(), busy.clone());
            std::thread::spawn(move || transcriber(&model, job_rx, ready, epoch, busy));
        }
        {
            let (paused, epoch) = (paused.clone(), epoch.clone());
            std::thread::spawn(move || listen(audio_rx, rate, paused, epoch, busy, job_tx));
        }

        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            match line.trim() {
                "pause" => {
                    // One epoch per pause: anything in flight from before it is dropped.
                    if !paused.swap(true, Ordering::SeqCst) {
                        epoch.fetch_add(1, Ordering::SeqCst);
                    }
                    state("paused", None);
                }
                "resume" => {
                    paused.store(false, Ordering::SeqCst);
                    state(if ready.load(Ordering::SeqCst) { "listening" } else { "loading" }, None);
                }
                "quit" => break,
                _ => {}
            }
        }
        drop(stream);
        let _ = std::io::stdout().flush();
        std::process::exit(0)
    }

    /// Opens `device` at its default format and sends every buffer, as mono
    /// f32, to `tx`.
    fn open_stream(device: &cpal::Device, tx: mpsc::Sender<Vec<f32>>) -> Result<cpal::Stream, String> {
        let supported = device.default_input_config().map_err(|e| e.to_string())?;
        let channels = supported.channels() as usize;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let err = |e: cpal::Error| state("warning", Some(&format!("audio: {e}")));
        let stream = match format {
            cpal::SampleFormat::F32 => device.build_input_stream(config, move |d: &[f32], _: &_| drop(tx.send(downmix(d, channels))), err, None),
            cpal::SampleFormat::I16 => device.build_input_stream(
                config,
                move |d: &[i16], _: &_| {
                    let f: Vec<f32> = d.iter().map(|s| *s as f32 / 32768.0).collect();
                    let _ = tx.send(downmix(&f, channels));
                },
                err,
                None,
            ),
            cpal::SampleFormat::I32 => device.build_input_stream(
                config,
                move |d: &[i32], _: &_| {
                    let f: Vec<f32> = d.iter().map(|s| *s as f32 / 2_147_483_648.0).collect();
                    let _ = tx.send(downmix(&f, channels));
                },
                err,
                None,
            ),
            other => return Err(format!("unsupported sample format {other}")),
        };
        stream.map_err(|e| e.to_string())
    }

    /// One transcription: the audio, whether it is the utterance's end, and
    /// the pause epoch when it was queued.
    struct Job {
        audio: Vec<f32>,
        last: bool,
        epoch: u64,
    }

    /// Frames the audio, runs the VAD, meters the level and queues
    /// transcriptions; mirrors the Swift `WhisperRecogniser`'s tap side.
    fn listen(rx: mpsc::Receiver<Vec<f32>>, rate: u32, paused: Arc<AtomicBool>, epoch: Arc<AtomicU64>, busy: Arc<AtomicBool>, jobs: mpsc::Sender<Job>) {
        let mut resampler = Resampler::new(rate);
        let (mut vad, mut pending, mut pre_roll, mut utterance) = (Vad::default(), Vec::<f32>::new(), Vec::<f32>::new(), Vec::<f32>::new());
        let mut was_paused = false;
        let mut last_level = Instant::now();
        for buf in rx {
            if paused.load(Ordering::SeqCst) {
                was_paused = true;
                continue;
            }
            if was_paused {
                // Nothing heard across a pause carries into what follows it.
                was_paused = false;
                resampler = Resampler::new(rate);
                vad = Vad::default();
                pending.clear();
                pre_roll.clear();
                utterance.clear();
            }
            if last_level.elapsed().as_millis() >= 200 {
                last_level = Instant::now();
                emit(json!({"type": "level", "value": maya_ear::level(maya_ear::rms(&buf))}));
            }
            pending.extend(resampler.push(&buf));
            while pending.len() >= FRAME_SAMPLES {
                let frame: Vec<f32> = pending.drain(..FRAME_SAMPLES).collect();
                let queue = |audio: Vec<f32>, last: bool| {
                    if busy.swap(true, Ordering::SeqCst) && !last {
                        return;
                    }
                    let _ = jobs.send(Job { audio, last, epoch: epoch.load(Ordering::SeqCst) });
                };
                match vad.push(maya_ear::rms(&frame)) {
                    VadEvent::None if vad.speaking => utterance.extend(&frame),
                    VadEvent::None => {
                        pre_roll.extend(&frame);
                        let keep = PRE_ROLL_FRAMES * FRAME_SAMPLES;
                        if pre_roll.len() > keep {
                            pre_roll.drain(..pre_roll.len() - keep);
                        }
                    }
                    VadEvent::Started => {
                        utterance = std::mem::take(&mut pre_roll);
                        utterance.extend(&frame);
                    }
                    VadEvent::PartialDue => {
                        utterance.extend(&frame);
                        queue(utterance.clone(), false);
                    }
                    VadEvent::Ended(reason) => {
                        utterance.extend(&frame);
                        let audio = std::mem::take(&mut utterance);
                        note(format!("utterance {:.1} s ({reason})", audio.len() as f64 / TARGET_RATE as f64));
                        queue(audio, true);
                    }
                }
            }
        }
    }

    /// Loads the model, then transcribes each job in order.
    fn transcriber(model: &str, jobs: mpsc::Receiver<Job>, ready: Arc<AtomicBool>, epoch: Arc<AtomicU64>, busy: Arc<AtomicBool>) {
        state("loading", None);
        let t0 = Instant::now();
        let ctx = WhisperContext::new_with_params(model, WhisperContextParameters::default()).unwrap_or_else(|e| fail(&format!("could not load model: {model} ({e})"), 8));
        let mut st = ctx.create_state().unwrap_or_else(|e| fail(&format!("could not load model: {model} ({e})"), 8));
        note(format!("model loaded in {:.1} s", t0.elapsed().as_secs_f64()));
        ready.store(true, Ordering::SeqCst);
        state("listening", None);
        let threads = std::thread::available_parallelism().map(|n| n.get().clamp(1, 4)).unwrap_or(4) as i32;
        let mut last_partial = String::new();
        let mut last_epoch = epoch.load(Ordering::SeqCst);
        for job in jobs {
            let now = epoch.load(Ordering::SeqCst);
            if now != last_epoch {
                last_epoch = now;
                last_partial.clear();
            }
            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_timestamps(false);
            params.set_no_timestamps(true);
            params.set_single_segment(true);
            params.set_suppress_blank(true);
            params.set_suppress_nst(true);
            params.set_n_threads(threads);
            params.set_language(Some("en"));
            let t0 = Instant::now();
            let result = st.full(params, &job.audio);
            busy.store(false, Ordering::SeqCst);
            if let Err(e) = result {
                state("warning", Some(&format!("whisper_full failed: {e}")));
                continue;
            }
            let text: String = st.as_iter().filter_map(|s| s.to_str_lossy().ok().map(|t| t.into_owned())).collect::<String>().trim().to_string();
            if job.last {
                note(format!("transcribed {:.1} s in {:.2} s", job.audio.len() as f64 / TARGET_RATE as f64, t0.elapsed().as_secs_f64()));
            }
            // A pause while this ran means the listener may already have
            // drained what it heard for a read-back: this result must never
            // surface, or a stray word could confirm the pending action.
            if epoch.load(Ordering::SeqCst) != job.epoch {
                note("dropped a result from before a pause".into());
                continue;
            }
            if is_filler(&text) {
                if job.last && !text.is_empty() {
                    note(format!("dropped filler: {text}"));
                }
                continue;
            }
            if !job.last {
                if text == last_partial {
                    continue;
                }
                last_partial = text.clone();
            }
            emit(json!({"type": if job.last { "final" } else { "partial" }, "text": text}));
        }
    }

}
