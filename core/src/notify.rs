use crate::model::{AwaitKind, Card, State};
use crate::state::truncate;
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
#[cfg(any(test, target_os = "linux"))]
use std::path::Path;
#[cfg(all(unix, not(target_os = "android")))]
use std::process::{Command, Stdio};
use std::path::PathBuf;
use std::sync::{mpsc, LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Remembers which asks have been announced so each one notifies once.
/// The first refresh only primes it: asks already open when Maya starts
/// are not announced.
#[derive(Default)]
pub struct Notifier {
    seen: HashSet<(String, u64)>,
    finished: HashSet<(String, u64)>,
    primed: bool,
    primed_finished: bool,
}

impl Notifier {
    /// The cards whose ask is new since the last call.
    pub fn take_new(&mut self, cards: &[Card]) -> Vec<Card> {
        let mut out = Vec::new();
        for c in cards.iter().filter(|c| c.state == State::Awaiting) {
            if self.seen.insert((c.session_id.clone(), c.state_since)) && self.primed {
                out.push(c.clone());
            }
        }
        self.primed = true;
        out
    }

    /// The cards whose turn finished since the last call, once each. A card
    /// in `held` (its last message is still being judged) is announced on the
    /// first call that no longer holds it; before the first call has primed,
    /// held cards are recorded like the rest, so a turn from before startup
    /// stays quiet.
    pub fn take_finished(&mut self, cards: &[Card], held: &HashSet<String>) -> Vec<Card> {
        let mut out = Vec::new();
        for c in cards.iter().filter(|c| c.state == State::Completed) {
            if self.primed_finished && held.contains(&c.session_id) {
                continue;
            }
            if self.finished.insert((c.session_id.clone(), c.state_since)) && self.primed_finished {
                out.push(c.clone());
            }
        }
        self.primed_finished = true;
        out
    }

    /// Marks the asks and finished turns on `cards` as already announced,
    /// without announcing them: a board the main has never held (an
    /// assistant just paired, or the main just started) is not news.
    pub fn seed(&mut self, cards: &[Card]) {
        for c in cards {
            match c.state {
                State::Awaiting => self.seen.insert((c.session_id.clone(), c.state_since)),
                State::Completed => self.finished.insert((c.session_id.clone(), c.state_since)),
                _ => false,
            };
        }
    }
}

/// A board for another Maya, which holds no judging of its own: a finished
/// card whose session is in `judging` goes out as Working, so it is not
/// announced as finished just before its verdict makes it ask.
pub fn mask_judging(cards: &[Card], judging: &HashSet<String>) -> Vec<Card> {
    cards
        .iter()
        .map(|c| {
            let mut c = c.clone();
            if c.state == State::Completed && judging.contains(&c.session_id) {
                c.state = State::Working;
            }
            c
        })
        .collect()
}

/// The session name as speech: dashes and hashes become spaces.
pub(crate) fn spoken_name(name: &str) -> String {
    let spaced: String = name.chars().map(|c| if c == '-' || c == '_' || c == '#' { ' ' } else { c }).collect();
    spaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What Maya says for a card, or nothing for states that are not announced.
pub fn spoken_line(card: &Card) -> Option<String> {
    let name = spoken_name(&card.name);
    let name = match &card.machine {
        Some(m) => format!("{name} on {m}"),
        None => name,
    };
    match card.state {
        State::Awaiting => Some(format!("{name} needs a decision")),
        State::Completed => Some(format!("{name} is finished")),
        _ => None,
    }
}

/// Maya's voice: the first of these installed, all female English voices
/// that ship with macOS. Never the system default, which may be anything.
pub const VOICES: &[&str] = &["Samantha", "Karen", "Moira", "Tessa", "Kate", "Serena", "Ava", "Allison", "Zoe"];

/// The voice to use, from `say -v ?` output.
pub fn pick_voice(installed: &str) -> Option<String> {
    let names: Vec<&str> = installed.lines().filter_map(|l| l.split_whitespace().next()).collect();
    VOICES.iter().find(|v| names.contains(v)).map(|v| v.to_string())
}

#[cfg(target_os = "macos")]
static VOICE: OnceLock<Option<String>> = OnceLock::new();

#[cfg(target_os = "macos")]
fn voice() -> Option<String> {
    VOICE
        .get_or_init(|| {
            let out = Command::new("say").args(["-v", "?"]).output().ok()?;
            pick_voice(&String::from_utf8_lossy(&out.stdout))
        })
        .clone()
}

/// A voice that speaks over the network, with the settings it speaks with.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum RemoteVoice {
    Elevenlabs { maya_dir: PathBuf, key: String, voice_id: String },
    /// An OpenAI-compatible speech server at `base`.
    Server { maya_dir: PathBuf, base: String, voice: String, model: String, key: Option<String> },
}

impl RemoteVoice {
    /// The voice's name in what Maya reports.
    pub fn label(&self) -> &'static str {
        match self {
            RemoteVoice::Elevenlabs { .. } => "ElevenLabs",
            RemoteVoice::Server { .. } => "the voice server",
        }
    }

    /// Identifies the voice with these exact settings. The marks it keys
    /// live in memory, so the per-process std hasher is enough.
    fn fingerprint(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.hash(&mut h);
        h.finish()
    }
}

/// Written by hand so that no log ever shows a key.
impl std::fmt::Debug for RemoteVoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RemoteVoice::Elevenlabs { maya_dir, voice_id, .. } => f.debug_struct("Elevenlabs").field("maya_dir", maya_dir).field("voice_id", voice_id).finish_non_exhaustive(),
            RemoteVoice::Server { maya_dir, base, voice, model, .. } => {
                f.debug_struct("Server").field("maya_dir", maya_dir).field("base", base).field("voice", voice).field("model", model).finish_non_exhaustive()
            }
        }
    }
}

/// How far a remote voice may go for a line: only to its cache, or out to
/// the network when the line is not cached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    CacheOnly,
    Network,
}

/// What became of a line: the voice that spoke it, after the remote voices
/// that failed (`"ElevenLabs failed: 402."`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeechReport {
    pub spoken_by: &'static str,
    pub failures: Vec<String>,
}

impl SpeechReport {
    /// The failures and the voice that spoke, as one line.
    pub fn message(&self) -> String {
        let spoke = format!("Spoke with {}.", self.spoken_by);
        self.failures.iter().map(String::as_str).chain([spoke.as_str()]).collect::<Vec<_>>().join(" ")
    }
}

/// The built-in voice in a report: always last in the chain, always there.
const BUILTIN: &str = "the built-in voice";

/// A line to speak and the voices to try for it, decided when it is queued.
pub struct Utterance {
    pub text: String,
    /// The remote voices to try in order; the built-in voice speaks when
    /// none of them does.
    pub voices: Vec<RemoteVoice>,
    /// "Try the voice": every remote voice is tried over the network, even
    /// one marked down.
    pub trial: bool,
    /// Told once the line has been spoken, with the voice that spoke it.
    pub done: Option<mpsc::Sender<SpeechReport>>,
    /// How loud, in percent of the engine's own full volume (`Config::voice_loudness`).
    pub volume: u8,
}

impl Utterance {
    pub fn new(text: String, voices: Vec<RemoteVoice>) -> Utterance {
        Utterance { text, voices, trial: false, done: None, volume: 100 }
    }
}

/// Where an utterance is: the listener pauses its ear on `Starting` (and
/// ignores the line if it hears it back) and resumes on `Finished`.
#[derive(Debug, Clone, PartialEq)]
pub enum SpeechPhase {
    Starting(String),
    Finished,
}

type SpeechHook = Box<dyn Fn(SpeechPhase) + Send + Sync>;

static SPEECH_HOOK: OnceLock<SpeechHook> = OnceLock::new();

/// Installs the hook called around every utterance. Once per process; a
/// second call is ignored.
pub fn install_speech_hook(hook: impl Fn(SpeechPhase) + Send + Sync + 'static) {
    let _ = SPEECH_HOOK.set(Box::new(hook));
}

/// True when the Focus assertion store (`~/Library/DoNotDisturb/DB/Assertions.json`,
/// as JSON) holds an active record: a Focus mode such as Do Not Disturb is on.
pub fn focus_active_in(json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| v["data"][0]["storeAssertionRecords"].as_array().map(|a| !a.is_empty()))
        .unwrap_or(false)
}

/// Whether Windows is keeping quiet: Do not disturb, quiet time, a
/// presentation or a full-screen app. Unreadable state counts as off.
#[cfg(windows)]
pub fn focus_active() -> bool {
    crate::notify_win::quiet_now()
}

/// Whether the Focus assertion store at `path` says a Focus mode is on:
/// None when it cannot be read (macOS keeps it from apps without Full Disk
/// Access), off when there is none yet. The store is JSON on current macOS;
/// anything else goes through `plutil`.
#[cfg(any(test, target_os = "macos"))]
pub fn focus_status_at(path: &std::path::Path) -> Option<bool> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => return None,
        Err(_) => return Some(false),
    };
    if let Ok(text) = std::str::from_utf8(&bytes) {
        if serde_json::from_str::<serde_json::Value>(text).is_ok() {
            return Some(focus_active_in(text));
        }
    }
    #[cfg(target_os = "macos")]
    {
        let out = Command::new("plutil").args(["-convert", "json", "-o", "-"]).arg(path).stdin(Stdio::null()).stderr(Stdio::null()).output();
        if let Some(o) = out.ok().filter(|o| o.status.success()) {
            return Some(focus_active_in(&String::from_utf8_lossy(&o.stdout)));
        }
    }
    Some(false)
}

/// Whether a Focus mode is on right now: None when Maya cannot tell, which
/// on macOS means she has no Full Disk Access.
#[cfg(target_os = "macos")]
pub fn focus_status() -> Option<bool> {
    let home = dirs::home_dir()?;
    focus_status_at(&home.join("Library/DoNotDisturb/DB/Assertions.json"))
}

/// Whether a Focus mode is on right now. When Maya cannot tell, that is
/// logged once and counts as off.
#[cfg(target_os = "macos")]
pub fn focus_active() -> bool {
    static UNSEEN: std::sync::Once = std::sync::Once::new();
    focus_status().unwrap_or_else(|| {
        UNSEEN.call_once(|| crate::log::line("notify", "cannot read the Focus state (no Full Disk Access): Focus modes will not keep Maya quiet"));
        false
    })
}

/// The phone app never reads a desktop's quiet state: never quiet.
#[cfg(target_os = "android")]
pub fn focus_active() -> bool {
    false
}

/// Whether a Focus mode is on right now; Windows and GNOME always let Maya tell.
#[cfg(not(target_os = "macos"))]
pub fn focus_status() -> Option<bool> {
    Some(focus_active())
}

static SPEECH: OnceLock<Mutex<mpsc::Sender<Utterance>>> = OnceLock::new();

/// Speaks `(voice, text, volume)` with a remote voice, going no further than
/// `Reach`: with `CacheOnly` it plays a cached line and otherwise fails.
pub type RemoteSpeaker = fn(&RemoteVoice, &str, u8, Reach) -> Result<(), String>;

static REMOTE_SPEAKER: OnceLock<RemoteSpeaker> = OnceLock::new();

/// Installs the remote speaker. Once per process; a second call is
/// ignored. Without one, every line is spoken with the built-in voice.
pub fn set_remote_speaker(speak: RemoteSpeaker) {
    let _ = REMOTE_SPEAKER.set(speak);
}

/// How long a remote voice that failed is left out of the chain.
pub const DOWN_FOR: Duration = Duration::from_secs(300);

/// When each failed remote voice, by fingerprint, last failed. A voice
/// whose settings change has another fingerprint, so it is not down.
#[derive(Default)]
pub(crate) struct DownMarks(HashMap<u64, Instant>);

impl DownMarks {
    /// True when the voice failed less than `DOWN_FOR` before `now`.
    pub(crate) fn is_down(&self, fp: u64, now: Instant) -> bool {
        self.0.get(&fp).is_some_and(|at| now.saturating_duration_since(*at) < DOWN_FOR)
    }

    pub(crate) fn mark(&mut self, fp: u64, now: Instant) {
        self.0.insert(fp, now);
    }

    pub(crate) fn clear(&mut self, fp: u64) {
        self.0.remove(&fp);
    }
}

static DOWN: LazyLock<Mutex<DownMarks>> = LazyLock::new(Default::default);

/// Speaks with the built-in female voice.
#[cfg(windows)]
fn say_builtin(line: &str, volume: u8) {
    crate::notify_win::say(line, volume)
}

/// Speaks with the built-in female voice.
#[cfg(target_os = "macos")]
fn say_builtin(line: &str, volume: u8) {
    let mut cmd = Command::new("say");
    if let Some(v) = voice() {
        cmd.args(["-v", &v]);
    }
    let _ = cmd.arg(say_text(line, volume)).status();
}

/// The line for `say`, led by its `volm` embedded command below full volume.
#[cfg(any(test, target_os = "macos"))]
fn say_text(line: &str, volume: u8) -> String {
    if volume >= 100 {
        return line.to_string();
    }
    format!("[[volm {:.2}]] {line}", f32::from(volume) / 100.0)
}

/// The phone app does not speak.
#[cfg(target_os = "android")]
fn say_builtin(_line: &str, _volume: u8) {}

/// The function that speaks one line with a remote voice, as `speak_chain` takes it.
type RemoteFn<'a> = dyn Fn(&RemoteVoice, &str, u8, Reach) -> Result<(), String> + 'a;

/// Speaks `u` with the first voice in its chain that can. A voice marked
/// down only plays a cached line, silently moving on when it has none
/// (unless this is a trial); any other voice goes to the network, and a
/// failure marks it down and is reported. The built-in voice speaks when no
/// remote voice did.
fn speak_chain(u: &Utterance, marks: &mut DownMarks, now: Instant, remote: &RemoteFn, builtin: &dyn Fn(&str, u8)) -> SpeechReport {
    let mut failures = Vec::new();
    for v in &u.voices {
        let fp = v.fingerprint();
        if !u.trial && marks.is_down(fp, now) {
            if remote(v, &u.text, u.volume, Reach::CacheOnly).is_ok() {
                return SpeechReport { spoken_by: v.label(), failures };
            }
            continue;
        }
        match remote(v, &u.text, u.volume, Reach::Network) {
            Ok(()) => {
                marks.clear(fp);
                return SpeechReport { spoken_by: v.label(), failures };
            }
            Err(e) => {
                marks.mark(fp, now);
                failures.push(format!("{} failed: {}.", capitalised(v.label()), e.trim_end_matches('.')));
            }
        }
    }
    builtin(&u.text, u.volume);
    SpeechReport { spoken_by: BUILTIN, failures }
}

/// `s` with its first letter upper-cased, to start a sentence.
fn capitalised(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// Speaks one line through its chain of voices, with the installed remote
/// speaker; without one, the built-in voice speaks.
fn speak_line(u: &Utterance) -> SpeechReport {
    let Some(remote) = REMOTE_SPEAKER.get() else {
        say_builtin(&u.text, u.volume);
        return SpeechReport { spoken_by: BUILTIN, failures: vec![] };
    };
    let now = Instant::now();
    // Held across synthesis and playback: safe only because the speech thread is DOWN's sole user.
    let mut marks = DOWN.lock().unwrap_or_else(|e| e.into_inner());
    speak_chain(u, &mut marks, now, remote, &say_builtin)
}

/// Speaks `u` with `speak`, calling `hook` before and after, then tells
/// whoever waits for it.
fn deliver(u: Utterance, hook: Option<&dyn Fn(SpeechPhase)>, speak: &dyn Fn(&Utterance) -> SpeechReport) {
    if let Some(h) = hook {
        h(SpeechPhase::Starting(u.text.clone()));
    }
    crate::log::line("speech", format!("saying: {}", u.text));
    let report = speak(&u);
    for f in &report.failures {
        crate::log::line("speech", f.clone());
    }
    if let Some(h) = hook {
        h(SpeechPhase::Finished);
    }
    if let Some(done) = u.done {
        let _ = done.send(report);
    }
}

/// Queues an utterance. Every line Maya says (announcements, replies, the
/// voice test) goes through this one queue and is spoken one after another
/// on a background thread, so voices never overlap and the speech hook
/// pauses listening around each one. The queue itself ignores Focus modes:
/// unsolicited announcements are filtered before they are queued.
pub fn speak(u: Utterance) {
    let tx = SPEECH.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Utterance>();
        std::thread::spawn(move || {
            for u in rx {
                let hook = SPEECH_HOOK.get().map(|h| h.as_ref() as &dyn Fn(SpeechPhase));
                deliver(u, hook, &speak_line);
            }
        });
        Mutex::new(tx)
    });
    if let Ok(tx) = tx.lock() {
        let _ = tx.send(u);
    }
}

/// Queues an utterance and waits until it has been spoken, for the report
/// of which voice spoke it; an error only when the queue stopped. Never
/// call it holding a lock the speech hook takes.
pub fn speak_and_wait(mut u: Utterance) -> Result<SpeechReport, String> {
    let (tx, rx) = mpsc::channel();
    u.done = Some(tx);
    speak(u);
    rx.recv().map_err(|_| "the speech queue stopped".to_string())
}

fn applescript_string(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// A Notification Center banner, with the default sound unless it is being spoken instead.
pub fn applescript_notify(title: &str, subtitle: &str, body: &str, sound: bool) -> String {
    format!(
        "display notification \"{}\" with title \"{}\" subtitle \"{}\"{}",
        applescript_string(body),
        applescript_string(title),
        applescript_string(subtitle),
        if sound { " sound name \"default\"" } else { "" }
    )
}

/// What the notification says: the ask itself, or its kind when the detail is blank.
pub fn body_for(card: &Card) -> String {
    let Some(aw) = &card.awaiting else { return "Waiting for a decision".into() };
    let detail = aw.detail.trim().trim_matches(|c| c == '*' || c == '_' || c == ' ');
    if !detail.is_empty() {
        return truncate(detail, 200);
    }
    match aw.kind {
        AwaitKind::Question | AwaitKind::Text => "Waiting for an answer",
        AwaitKind::Plan => "Waiting for plan approval",
        AwaitKind::Permission => "Waiting for a permission decision",
    }
    .into()
}

/// The banner's subtitle: the project, with the machine appended for a
/// remote card so a decision on another Mac is never mistaken for a local one.
pub fn subtitle_for(card: &Card) -> String {
    let project = card.cwd.rsplit('/').find(|s| !s.is_empty()).unwrap_or(&card.cwd);
    match &card.machine {
        Some(m) => format!("{project} on {m}"),
        None => project.to_string(),
    }
}

/// A session's banner: its name, the project and what it waits on.
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
pub fn notify(card: &Card, sound: bool) {
    banner(&card.name, &subtitle_for(card), &body_for(card), sound);
}

#[cfg(windows)]
pub fn banner(title: &str, subtitle: &str, body: &str, sound: bool) {
    crate::notify_win::show_toast(title, subtitle, body, sound);
}

#[cfg(target_os = "macos")]
pub fn banner(title: &str, subtitle: &str, body: &str, sound: bool) {
    let _ = Command::new("osascript").arg("-e").arg(applescript_notify(title, subtitle, body, sound)).output();
}

/// `notify-send`'s arguments for a banner from Maya: the app name, the
/// icon when there is one, the session as the title and the subtitle and
/// body on two lines (GNOME banners have no subtitle).
#[cfg(any(test, target_os = "linux"))]
pub fn notify_send_args(icon: Option<&Path>, title: &str, subtitle: &str, body: &str) -> Vec<String> {
    let mut a = vec!["--app-name".to_string(), "Maya".to_string()];
    if let Some(i) = icon {
        a.extend(["--icon".to_string(), i.to_string_lossy().into_owned()]);
    }
    let text = if subtitle.is_empty() {
        body.to_string()
    } else if body.is_empty() {
        subtitle.to_string()
    } else {
        format!("{subtitle}\n{body}")
    };
    a.extend(["--".to_string(), title.to_string(), text]);
    a
}

/// True when `gsettings get org.gnome.desktop.notifications show-banners`
/// says `false`: GNOME's Do Not Disturb is on. Anything else (true, an
/// error, no GNOME) counts as off.
#[cfg(any(test, target_os = "linux"))]
pub fn dnd_from_gsettings(out: &str) -> bool {
    out.trim() == "false"
}

/// The speech synthesiser on `path` (a `PATH` string) and its arguments
/// before the line: speech-dispatcher's `spd-say -w` (waits until spoken),
/// else `espeak-ng`; None when neither is installed.
#[cfg(any(test, target_os = "linux"))]
pub fn speech_command(path: &str) -> Option<(&'static str, Vec<String>)> {
    if crate::launch::find_on_path(path, "spd-say").is_some() {
        return Some(("spd-say", vec!["-w".to_string()]));
    }
    crate::launch::find_on_path(path, "espeak-ng").map(|_| ("espeak-ng", Vec::new()))
}

/// The volume for a Linux synthesiser, on its own scale; none at full
/// volume: speech-dispatcher's `-i` runs -100..100 with 0 its normal level,
/// eSpeak NG's `-a` amplitude 0..200 with 100 its default.
#[cfg(any(test, target_os = "linux"))]
pub fn volume_args(bin: &str, volume: u8) -> Vec<String> {
    if volume >= 100 {
        return vec![];
    }
    match bin {
        "spd-say" => vec!["-i".into(), (i32::from(volume) - 100).to_string()],
        "espeak-ng" => vec!["-a".into(), volume.to_string()],
        _ => vec![],
    }
}

/// Maya's icon for banners: inside the AppImage (`$APPDIR`) or where the
/// .deb installs it; None when the file is not there.
#[cfg(target_os = "linux")]
pub fn app_icon_path() -> Option<PathBuf> {
    let rel = "usr/share/icons/hicolor/128x128/apps/maya.png";
    let p = match std::env::var("APPDIR") {
        Ok(d) => Path::new(&d).join(rel),
        Err(_) => Path::new("/").join(rel),
    };
    p.is_file().then_some(p)
}

/// Whether GNOME's Do Not Disturb is on. Unreadable state counts as off.
#[cfg(target_os = "linux")]
pub fn focus_active() -> bool {
    Command::new("gsettings")
        .args(["get", "org.gnome.desktop.notifications", "show-banners"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|o| dnd_from_gsettings(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or(false)
}

/// The fixed part of a line's speech deadline.
#[cfg(any(test, target_os = "linux"))]
const SPEECH_BASE: std::time::Duration = std::time::Duration::from_secs(15);

/// How long a line may take to speak: 15 s plus 1 s per 12 characters.
#[cfg(any(test, target_os = "linux"))]
pub fn speech_deadline(line: &str) -> std::time::Duration {
    speech_deadline_from(SPEECH_BASE, line)
}

#[cfg(any(test, target_os = "linux"))]
fn speech_deadline_from(base: std::time::Duration, line: &str) -> std::time::Duration {
    base + std::time::Duration::from_secs((line.chars().count() / 12) as u64)
}

/// Runs the synthesiser `bin` on `line` and waits for it, up to `base` plus
/// 1 s per 12 characters. With no audio sink `spd-say -w` and `espeak-ng`
/// never return, which would block the speech queue (and leave the ear
/// paused) for good, so past the deadline the process is killed, an
/// `spd-say` is told to cancel what it queued, and the line is logged.
/// True when the synthesiser finished in time.
#[cfg(target_os = "linux")]
pub(crate) fn speak_within(bin: &Path, args: &[String], line: &str, base: std::time::Duration) -> bool {
    use std::time::{Duration, Instant};
    let Ok(mut child) = Command::new(bin).args(args).arg("--").arg(line).stdin(Stdio::null()).spawn() else {
        return false;
    };
    let limit = speech_deadline_from(base, line);
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            _ => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    let name = bin.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if name == "spd-say" {
        // On a thread of its own and killed after 2 s: with no audio sink
        // the cancel hangs too, and must neither block the queue nor
        // linger.
        if let Ok(mut cancel) = Command::new(bin).arg("-C").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(2);
                while matches!(cancel.try_wait(), Ok(None)) && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(100));
                }
                let _ = cancel.kill();
                let _ = cancel.wait();
            });
        }
    }
    crate::log::line("speech", format!("{name} did not finish in {} s; stopped", limit.as_secs()));
    false
}

/// Speaks with speech-dispatcher or eSpeak NG, whichever is installed;
/// without either, logs once and stays silent.
#[cfg(target_os = "linux")]
fn say_builtin(line: &str, volume: u8) {
    static WARNED: std::sync::Once = std::sync::Once::new();
    match speech_command(&std::env::var("PATH").unwrap_or_default()) {
        Some((bin, mut args)) => {
            args.extend(volume_args(bin, volume));
            speak_within(Path::new(bin), &args, line, SPEECH_BASE);
        }
        None => WARNED.call_once(|| crate::log::line("notify", "no speech synthesiser: install speech-dispatcher or espeak-ng")),
    }
}

/// A GNOME banner through `notify-send`, with the freedesktop "message"
/// sound unless the line is being spoken instead.
#[cfg(target_os = "linux")]
pub fn banner(title: &str, subtitle: &str, body: &str, sound: bool) {
    static MISSING: std::sync::Once = std::sync::Once::new();
    let icon = app_icon_path();
    if let Err(e) = Command::new("notify-send").args(notify_send_args(icon.as_deref(), title, subtitle, body)).stdin(Stdio::null()).output() {
        crate::log::missing_once(&MISSING, &e, "notify", "notify-send is not installed, so no banners: install libnotify-bin");
    }
    if sound {
        // Reaped on a thread: the sound must not hold up the watcher.
        if let Ok(mut child) = Command::new("canberra-gtk-play").args(["-i", "message"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AwaitKind, Awaiting, Card, Harness, State};

    #[test]
    fn an_utterance_speaks_at_full_volume_unless_told() {
        assert_eq!(Utterance::new("x".into(), vec![]).volume, 100);
    }

    fn eleven() -> RemoteVoice {
        RemoteVoice::Elevenlabs { maya_dir: "/m".into(), key: "k".into(), voice_id: "v".into() }
    }

    fn server() -> RemoteVoice {
        RemoteVoice::Server { maya_dir: "/m".into(), base: "http://h:1".into(), voice: "default".into(), model: String::new(), key: None }
    }

    /// Remote voices that answer from a script per label and record each
    /// call, and a built-in voice that records what it said.
    #[derive(Default)]
    struct Voices {
        answers: std::cell::RefCell<std::collections::HashMap<&'static str, Result<(), String>>>,
        calls: std::cell::RefCell<Vec<(&'static str, Reach)>>,
        builtin: std::cell::RefCell<Vec<String>>,
    }

    impl Voices {
        fn answer(&self, label: &'static str, result: Result<(), &str>) {
            self.answers.borrow_mut().insert(label, result.map_err(String::from));
        }

        fn speak(&self, u: &Utterance, marks: &mut DownMarks, now: Instant) -> SpeechReport {
            let remote = |v: &RemoteVoice, _: &str, _: u8, reach: Reach| {
                self.calls.borrow_mut().push((v.label(), reach));
                self.answers.borrow().get(v.label()).cloned().unwrap_or(Ok(()))
            };
            let builtin = |text: &str, _: u8| self.builtin.borrow_mut().push(text.to_string());
            speak_chain(u, marks, now, &remote, &builtin)
        }

        fn take_calls(&self) -> Vec<(&'static str, Reach)> {
            std::mem::take(&mut *self.calls.borrow_mut())
        }
    }

    const ELEVEN: &str = "ElevenLabs";
    const SERVER: &str = "the voice server";

    fn line() -> Utterance {
        Utterance::new("hexgrid needs a decision".into(), vec![eleven(), server()])
    }

    #[test]
    fn the_chain_goes_elevenlabs_then_server_then_builtin() {
        let v = Voices::default();
        v.answer(ELEVEN, Err("402"));
        v.answer(SERVER, Ok(()));
        let report = v.speak(&line(), &mut DownMarks::default(), Instant::now());
        assert_eq!(report, SpeechReport { spoken_by: SERVER, failures: vec!["ElevenLabs failed: 402.".into()] });
        assert!(v.builtin.borrow().is_empty(), "the built-in voice is not used");
        assert_eq!(v.take_calls(), [(ELEVEN, Reach::Network), (SERVER, Reach::Network)]);
    }

    #[test]
    fn with_every_remote_voice_failing_the_builtin_speaks() {
        let v = Voices::default();
        v.answer(ELEVEN, Err("a"));
        v.answer(SERVER, Err("b"));
        let report = v.speak(&line(), &mut DownMarks::default(), Instant::now());
        assert_eq!(report.spoken_by, "the built-in voice");
        assert_eq!(report.failures.len(), 2);
        assert_eq!(*v.builtin.borrow(), ["hexgrid needs a decision"]);
        assert_eq!(report.message(), "ElevenLabs failed: a. The voice server failed: b. Spoke with the built-in voice.");
    }

    #[test]
    fn with_no_remote_voice_the_builtin_speaks() {
        let v = Voices::default();
        let report = v.speak(&Utterance::new("x".into(), vec![]), &mut DownMarks::default(), Instant::now());
        assert_eq!(report, SpeechReport { spoken_by: "the built-in voice", failures: vec![] });
        assert_eq!(report.message(), "Spoke with the built-in voice.");
        assert_eq!(*v.builtin.borrow(), ["x"]);
    }

    #[test]
    fn a_failed_voice_is_skipped_for_five_minutes_but_its_cache_still_plays() {
        let v = Voices::default();
        let mut marks = DownMarks::default();
        let t0 = Instant::now();
        v.answer(ELEVEN, Err("402"));
        v.speak(&line(), &mut marks, t0);
        v.take_calls();
        let report = v.speak(&line(), &mut marks, t0 + Duration::from_secs(60));
        assert_eq!(v.take_calls(), [(ELEVEN, Reach::CacheOnly), (SERVER, Reach::Network)], "only the cache is tried while down");
        assert_eq!(report, SpeechReport { spoken_by: SERVER, failures: vec![] }, "a cache miss while down is not a failure");
        assert!(marks.is_down(eleven().fingerprint(), t0 + Duration::from_secs(299)));
        v.speak(&line(), &mut marks, t0 + DOWN_FOR);
        assert_eq!(v.take_calls()[0], (ELEVEN, Reach::Network), "five minutes on, it is tried again");
        // A cached line plays while down.
        let mut marks = DownMarks::default();
        v.speak(&line(), &mut marks, t0);
        v.answer(ELEVEN, Ok(()));
        v.take_calls();
        let report = v.speak(&line(), &mut marks, t0 + Duration::from_secs(1));
        assert_eq!(report.spoken_by, ELEVEN);
        assert_eq!(v.take_calls(), [(ELEVEN, Reach::CacheOnly)]);
    }

    #[test]
    fn a_cache_miss_while_down_does_not_renew_the_mark() {
        let v = Voices::default();
        let mut marks = DownMarks::default();
        let t0 = Instant::now();
        v.answer(ELEVEN, Err("402"));
        v.speak(&line(), &mut marks, t0);
        v.speak(&line(), &mut marks, t0 + Duration::from_secs(200));
        v.take_calls();
        v.speak(&line(), &mut marks, t0 + Duration::from_secs(300));
        assert_eq!(v.take_calls()[0], (ELEVEN, Reach::Network));
    }

    #[test]
    fn changed_settings_are_not_down() {
        let v = Voices::default();
        let mut marks = DownMarks::default();
        let t0 = Instant::now();
        v.answer(ELEVEN, Err("402"));
        v.speak(&line(), &mut marks, t0);
        v.take_calls();
        let other = RemoteVoice::Elevenlabs { maya_dir: "/m".into(), key: "k".into(), voice_id: "w".into() };
        v.speak(&Utterance::new("x".into(), vec![other]), &mut marks, t0 + Duration::from_secs(1));
        assert_eq!(v.take_calls(), [(ELEVEN, Reach::Network)]);
    }

    #[test]
    fn a_trial_ignores_marks_and_a_success_clears_them() {
        let v = Voices::default();
        let mut marks = DownMarks::default();
        let t0 = Instant::now();
        v.answer(ELEVEN, Err("402"));
        v.speak(&line(), &mut marks, t0);
        v.take_calls();
        v.answer(ELEVEN, Ok(()));
        let report = v.speak(&Utterance { trial: true, ..line() }, &mut marks, t0 + Duration::from_secs(1));
        assert_eq!(report.spoken_by, ELEVEN);
        assert_eq!(v.take_calls(), [(ELEVEN, Reach::Network)], "a trial ignores the mark");
        v.speak(&line(), &mut marks, t0 + Duration::from_secs(2));
        assert_eq!(v.take_calls(), [(ELEVEN, Reach::Network)], "the success cleared the mark");
    }

    #[test]
    fn a_remote_voice_never_shows_its_key() {
        let shown = format!("{:?}", eleven());
        assert!(!shown.contains("k\"") && !shown.contains("key: \"k\""), "{shown}");
        let keyed = RemoteVoice::Server { maya_dir: "/m".into(), base: "http://h:1".into(), voice: "default".into(), model: String::new(), key: Some("sk-secret".into()) };
        let shown = format!("{keyed:?}");
        assert!(!shown.contains("sk-secret"), "{shown}");
    }

    #[test]
    fn say_is_told_the_volume_by_an_embedded_command() {
        assert_eq!(say_text("hexgrid needs a decision", 100), "hexgrid needs a decision", "full volume speaks as before");
        assert_eq!(say_text("hexgrid needs a decision", 80), "[[volm 0.80]] hexgrid needs a decision");
        assert_eq!(say_text("x", 10), "[[volm 0.10]] x");
    }

    #[test]
    fn linux_synthesisers_are_told_the_volume_their_own_way() {
        assert!(volume_args("spd-say", 100).is_empty(), "full volume speaks as before");
        assert!(volume_args("espeak-ng", 100).is_empty());
        // speech-dispatcher's scale is -100..100 with 0 its normal level.
        assert_eq!(volume_args("spd-say", 80), ["-i", "-20"]);
        assert_eq!(volume_args("spd-say", 10), ["-i", "-90"]);
        // eSpeak NG's amplitude is 0..200 with 100 its default.
        assert_eq!(volume_args("espeak-ng", 80), ["-a", "80"]);
        assert!(volume_args("something-else", 50).is_empty());
    }

    fn card(id: &str, state: State, since: u64, detail: &str) -> Card {
        Card {
            session_id: id.into(),
            pid: 1,
            name: format!("{id}-name"),
            cwd: format!("/Users/x/dev/{id}"),
            state,
            state_since: since,
            snippet: "".into(),
            awaiting: if state == State::Awaiting { Some(Awaiting { kind: AwaitKind::Text, detail: detail.into(), questions: vec![] }) } else { None },
            has_inbox: true,
            harness: Harness::ClaudeCode,
            pr: None,
            context: None,
            machine: None,
            machine_address: None, machine_platform: None,
            terminal: None,
            stale: false, model: None, branch: None, brain: false, native: false, shell: false, usage: None, limits: None, status_line: None, live: false
        }
    }

    #[test]
    fn first_refresh_primes_without_notifying_then_each_new_ask_notifies_once() {
        let mut n = Notifier::default();
        let already = card("a", State::Awaiting, 100, "Old question?");
        assert!(n.take_new(&[already.clone(), card("b", State::Working, 100, "")]).is_empty(), "asks open at startup are not announced");
        // Same ask again: nothing. A new session asking: once.
        assert!(n.take_new(&[already.clone()]).is_empty());
        let fresh = card("b", State::Awaiting, 200, "Go ahead?");
        let got = n.take_new(&[already.clone(), fresh.clone()]);
        assert_eq!(got.iter().map(|c| c.session_id.as_str()).collect::<Vec<_>>(), vec!["b"]);
        assert!(n.take_new(&[already.clone(), fresh.clone()]).is_empty());
        // The same session asking something new (a later state_since) notifies again.
        let again = card("b", State::Awaiting, 300, "And this?");
        assert_eq!(n.take_new(&[again.clone()]).len(), 1);
        // A session that leaves and comes back with the same ask timestamp is not re-announced.
        assert!(n.take_new(&[]).is_empty());
        assert!(n.take_new(&[again.clone()]).is_empty());
    }

    #[test]
    fn finished_turns_are_announced_once_like_asks() {
        let mut n = Notifier::default();
        let done = card("a", State::Completed, 100, "");
        assert!(n.take_finished(&[done.clone()], &HashSet::new()).is_empty(), "already finished at startup: quiet");
        let again = card("a", State::Completed, 200, "");
        assert_eq!(n.take_finished(&[again.clone()], &HashSet::new()).len(), 1);
        assert!(n.take_finished(&[again.clone()], &HashSet::new()).is_empty());
        assert!(n.take_finished(&[card("a", State::Working, 300, "")], &HashSet::new()).is_empty());
    }

    #[test]
    fn a_turn_held_at_startup_stays_quiet_after_its_verdict() {
        let mut n = Notifier::default();
        let held: HashSet<String> = ["a".to_string()].into();
        let done = card("a", State::Completed, 100, "");
        assert!(n.take_finished(&[done.clone()], &held).is_empty());
        assert!(n.take_finished(&[done], &HashSet::new()).is_empty());
    }

    #[test]
    fn a_judging_turn_goes_out_as_working_until_its_verdict() {
        let judging: HashSet<String> = ["a".to_string(), "c".to_string()].into();
        let cards = [card("a", State::Completed, 100, ""), card("b", State::Completed, 100, ""), card("c", State::Idle, 100, "")];
        let out = mask_judging(&cards, &judging);
        assert_eq!(out.iter().map(|c| (c.session_id.as_str(), c.state, c.state_since)).collect::<Vec<_>>(), [("a", State::Working, 100), ("b", State::Completed, 100), ("c", State::Idle, 100)]);
        assert_eq!(mask_judging(&cards, &HashSet::new()), cards);
    }

    #[test]
    fn a_held_turn_is_announced_once_when_released() {
        let mut n = Notifier::default();
        assert!(n.take_finished(&[], &HashSet::new()).is_empty(), "primed");
        let held: HashSet<String> = ["a".to_string()].into();
        let done = card("a", State::Completed, 100, "");
        assert!(n.take_finished(&[done.clone()], &held).is_empty());
        assert!(n.take_finished(&[done.clone()], &held).is_empty());
        assert_eq!(n.take_finished(&[done.clone()], &HashSet::new()).len(), 1);
        assert!(n.take_finished(&[done], &HashSet::new()).is_empty());
    }

    #[test]
    fn seeded_cards_are_never_announced_but_later_changes_are() {
        let mut n = Notifier::default();
        assert!(n.take_new(&[]).is_empty() && n.take_finished(&[], &HashSet::new()).is_empty(), "primed");
        let asked = card("a", State::Awaiting, 100, "Go?");
        let done = card("b", State::Completed, 100, "");
        n.seed(&[asked.clone(), done.clone(), card("c", State::Working, 100, "")]);
        assert!(n.take_new(&[asked.clone(), done.clone()]).is_empty(), "a seeded ask is not announced");
        assert!(n.take_finished(&[asked.clone(), done.clone()], &HashSet::new()).is_empty(), "a seeded finished turn is not announced");
        let asked_again = card("a", State::Awaiting, 200, "And now?");
        let done_again = card("b", State::Completed, 200, "");
        assert_eq!(n.take_new(&[asked_again.clone()]).len(), 1);
        assert_eq!(n.take_finished(&[done_again.clone()], &HashSet::new()).len(), 1);
    }

    fn card_awaiting(name: &str) -> Card {
        Card {
            session_id: "x".into(),
            pid: 1,
            name: name.into(),
            cwd: "/Users/x/dev/x".into(),
            state: State::Awaiting,
            state_since: 1,
            snippet: "".into(),
            awaiting: Some(Awaiting { kind: AwaitKind::Text, detail: "".into(), questions: vec![] }),
            has_inbox: true,
            harness: Harness::ClaudeCode,
            pr: None,
            context: None,
            machine: None,
            machine_address: None, machine_platform: None,
            terminal: None,
            stale: false, model: None, branch: None, brain: false, native: false, shell: false, usage: None, limits: None, status_line: None, live: false
        }
    }

    #[test]
    fn remote_sessions_are_announced_with_their_machine() {
        let mut c = card_awaiting("hexgrid");
        c.machine = Some("laptop".into());
        assert_eq!(spoken_line(&c).unwrap(), "hexgrid on laptop needs a decision");
    }

    #[test]
    fn the_banner_subtitle_names_the_machine_for_a_remote_card() {
        let local = card("a", State::Awaiting, 1, "");
        assert_eq!(subtitle_for(&local), "a");
        let mut remote = card("b", State::Awaiting, 1, "");
        remote.machine = Some("laptop".into());
        assert_eq!(subtitle_for(&remote), "b on laptop");
    }

    #[test]
    fn spoken_lines_are_short_and_say_the_name_without_dashes() {
        let ask = card("hexgrid-d3", State::Awaiting, 1, "Approve this plan?");
        assert_eq!(spoken_line(&ask).as_deref(), Some("hexgrid d3 name needs a decision"));
        let done = card("coral", State::Completed, 1, "");
        assert_eq!(spoken_line(&done).as_deref(), Some("coral name is finished"));
        assert_eq!(spoken_line(&card("x", State::Working, 1, "")), None);
        let mut named = card("x", State::Awaiting, 1, "q");
        named.name = "review bedrock #451".into();
        assert_eq!(spoken_line(&named).as_deref(), Some("review bedrock 451 needs a decision"));
    }

    #[test]
    fn voice_is_the_first_installed_female_english_voice() {
        let list = "Daniel              en_GB    # Hello! My name is Daniel.\nKaren               en_AU    # Hello!\nSamantha            en_US    # Hello!\n";
        assert_eq!(pick_voice(list).as_deref(), Some("Samantha"));
        assert_eq!(pick_voice("Daniel en_GB # x\nTessa en_ZA # y\n").as_deref(), Some("Tessa"));
        assert_eq!(pick_voice("Daniel en_GB # x\n"), None);
    }

    #[test]
    fn focus_is_unknown_only_when_macos_will_not_let_maya_read_the_store() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("Assertions.json");
        assert_eq!(focus_status_at(&p), Some(false), "no store yet: no Focus has ever been on");
        std::fs::write(&p, r#"{"data":[{"storeAssertionRecords":[{"assertionUUID":"x"}]}]}"#).unwrap();
        assert_eq!(focus_status_at(&p), Some(true));
        std::fs::write(&p, r#"{"data":[{"storeAssertionRecords":[]}]}"#).unwrap();
        assert_eq!(focus_status_at(&p), Some(false));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o000)).unwrap();
            // Without Full Disk Access macOS refuses the read the same way.
            assert_eq!(focus_status_at(&p), None);
        }
    }

    #[test]
    fn focus_is_active_when_the_assertion_store_has_records() {
        let on = r#"{"data":[{"storeAssertionRecords":[{"assertionUUID":"x","assertionDetails":{"assertionDetailsModeIdentifier":"com.apple.donotdisturb.mode.default"}}]}]}"#;
        assert!(focus_active_in(on));
        assert!(!focus_active_in(r#"{"data":[{"storeAssertionRecords":[]}]}"#));
        assert!(!focus_active_in(r#"{"data":[{}]}"#));
        assert!(!focus_active_in("garbage"));
    }

    #[test]
    fn each_line_is_spoken_between_the_hook_calls_and_then_reported_done() {
        let log = std::sync::Mutex::new(Vec::<String>::new());
        let hook = |p: SpeechPhase| log.lock().unwrap().push(format!("{p:?}"));
        let (tx, rx) = mpsc::channel();
        let u = Utterance { done: Some(tx), ..Utterance::new("hexgrid needs a decision".into(), vec![]) };
        let failed = SpeechReport { spoken_by: "the built-in voice", failures: vec!["ElevenLabs failed: no voice.".into()] };
        deliver(u, Some(&hook), &|u: &Utterance| {
            log.lock().unwrap().push(format!("speak {}", u.text));
            failed.clone()
        });
        assert_eq!(*log.lock().unwrap(), vec!["Starting(\"hexgrid needs a decision\")", "speak hexgrid needs a decision", "Finished"]);
        assert_eq!(rx.recv().unwrap(), failed, "the speaker's report reaches the waiter");
        // Without a hook (nothing listening) the line is still spoken.
        let spoken = std::sync::Mutex::new(0);
        deliver(Utterance::new("x".into(), vec![]), None, &|_| {
            *spoken.lock().unwrap() += 1;
            SpeechReport { spoken_by: "the built-in voice", failures: vec![] }
        });
        assert_eq!(*spoken.lock().unwrap(), 1);
    }

    #[test]
    fn banner_script_can_be_silent() {
        assert!(applescript_notify("t", "s", "b", true).contains("sound name"));
        assert!(!applescript_notify("t", "s", "b", false).contains("sound name"));
    }

    #[test]
    fn script_escapes_text_and_names_the_session_project_and_ask() {
        let s = applescript_notify("eye-1", "eye", "Bash: rm -rf \"x\" \\ y", true);
        assert!(s.starts_with("display notification \"Bash: rm -rf \\\"x\\\" \\\\ y\""), "{s}");
        assert!(s.contains("with title \"eye-1\""));
        assert!(s.contains("subtitle \"eye\""));
        assert!(s.contains("sound name \"default\""));
    }

    #[test]
    fn body_uses_the_ask_detail_and_falls_back_to_the_kind() {
        let c = card("a", State::Awaiting, 1, "Create it as drafted?");
        assert_eq!(body_for(&c), "Create it as drafted?");
        let mut p = card("a", State::Awaiting, 1, "");
        p.awaiting = Some(Awaiting { kind: AwaitKind::Permission, detail: "".into(), questions: vec![] });
        assert_eq!(body_for(&p), "Waiting for a permission decision");
        let long = card("a", State::Awaiting, 1, &"x".repeat(300));
        assert!(body_for(&long).chars().count() <= 200);
        // Markdown emphasis around the ask is noise in a banner.
        let bold = card("a", State::Awaiting, 1, "**Would you like me to commit?**");
        assert_eq!(body_for(&bold), "Would you like me to commit?");
        let under = card("a", State::Awaiting, 1, "_Ready?_ ");
        assert_eq!(body_for(&under), "Ready?");
    }

    #[test]
    fn notify_send_args_carry_app_name_icon_title_and_two_line_body() {
        let args = notify_send_args(Some(Path::new("/usr/share/icons/hicolor/128x128/apps/maya.png")), "collector", "needs a decision", "Allow Bash?");
        assert_eq!(args, ["--app-name", "Maya", "--icon", "/usr/share/icons/hicolor/128x128/apps/maya.png", "--", "collector", "needs a decision\nAllow Bash?"].map(String::from).to_vec());
        assert_eq!(notify_send_args(None, "a", "", "b"), ["--app-name", "Maya", "--", "a", "b"].map(String::from).to_vec());
    }

    #[test]
    fn dnd_is_only_an_explicit_false_from_gsettings() {
        assert!(dnd_from_gsettings("false\n"));
        assert!(!dnd_from_gsettings("true\n"));
        assert!(!dnd_from_gsettings(""));
        assert!(!dnd_from_gsettings("No such schema"));
    }

    #[cfg(unix)]
    #[test]
    fn speech_prefers_spd_say_then_espeak_then_nothing() {
        let d = tempfile::tempdir().unwrap();
        let exe = |name: &str| {
            let p = d.path().join(name);
            std::fs::write(&p, "#!/bin/sh\n").unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        assert_eq!(speech_command(""), None);
        exe("espeak-ng");
        assert_eq!(speech_command(&d.path().to_string_lossy()).map(|c| c.0), Some("espeak-ng"));
        exe("spd-say");
        assert_eq!(speech_command(&d.path().to_string_lossy()), Some(("spd-say", vec!["-w".to_string()])));
    }

    #[test]
    fn speech_gets_fifteen_seconds_plus_one_per_twelve_characters() {
        assert_eq!(speech_deadline(""), std::time::Duration::from_secs(15));
        assert_eq!(speech_deadline(&"a".repeat(120)), std::time::Duration::from_secs(25));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_synthesiser_that_never_returns_is_stopped_and_cancelled_at_the_deadline() {
        let d = tempfile::tempdir().unwrap();
        let marker = d.path().join("cancelled");
        let fake = d.path().join("spd-say");
        // The cancel hangs as well, as it does with no audio sink.
        std::fs::write(&fake, format!("#!/bin/sh\nif [ \"$1\" = -C ]; then echo $$ > '{}'; exec sleep 60; fi\nexec sleep 60\n", marker.display())).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let started = std::time::Instant::now();
        assert!(!speak_within(&fake, &["-w".to_string()], "x", std::time::Duration::from_secs(1)));
        assert!(started.elapsed() < std::time::Duration::from_secs(3), "took {:?}", started.elapsed());
        let waited = std::time::Instant::now();
        let written = || std::fs::read_to_string(&marker).map(|s| s.ends_with('\n')).unwrap_or(false);
        while !written() && waited.elapsed() < std::time::Duration::from_secs(2) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(written(), "spd-say -C was not run");
        let cancel = std::path::PathBuf::from(format!("/proc/{}", std::fs::read_to_string(&marker).unwrap().trim()));
        let waited = std::time::Instant::now();
        while cancel.exists() && waited.elapsed() < std::time::Duration::from_secs(4) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(!cancel.exists(), "a hung spd-say -C is stopped too");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_synthesiser_that_returns_in_time_is_left_alone() {
        let d = tempfile::tempdir().unwrap();
        let fake = d.path().join("espeak-ng");
        std::fs::write(&fake, "#!/bin/sh\nexit 0\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(speak_within(&fake, &[], "hello", std::time::Duration::from_secs(5)));
    }
}
