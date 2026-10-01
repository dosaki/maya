use crate::model::{AwaitKind, Card, State};
use crate::state::truncate;
use std::collections::HashSet;
#[cfg(any(test, target_os = "linux"))]
use std::path::Path;
#[cfg(target_os = "linux")]
use std::path::PathBuf;
#[cfg(unix)]
use std::process::{Command, Stdio};
use std::sync::{mpsc, Mutex, OnceLock};

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

    /// The cards whose turn finished since the last call, once each.
    pub fn take_finished(&mut self, cards: &[Card]) -> Vec<Card> {
        let mut out = Vec::new();
        for c in cards.iter().filter(|c| c.state == State::Completed) {
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

/// The session name as speech: dashes and hashes become spaces.
fn spoken_name(name: &str) -> String {
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

/// A line to speak and the voice to use for it, decided when it is queued.
pub struct Utterance {
    pub text: String,
    /// ElevenLabs (maya dir, key, voice id) when configured; None for the built-in voice.
    pub eleven: Option<(std::path::PathBuf, String, String)>,
    /// When ElevenLabs fails, speak the line with the built-in voice instead.
    /// Off for "Try the voice", which reports the failure.
    pub fallback: bool,
    /// Told once the line has been spoken, with the ElevenLabs error if any.
    pub done: Option<mpsc::Sender<Result<(), String>>>,
}

impl Utterance {
    pub fn new(text: String, eleven: Option<(std::path::PathBuf, String, String)>) -> Utterance {
        Utterance { text, eleven, fallback: true, done: None }
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

/// Whether a Focus mode is on right now. Unreadable state counts as off.
#[cfg(target_os = "macos")]
pub fn focus_active() -> bool {
    let Some(home) = dirs::home_dir() else { return false };
    let path = home.join("Library/DoNotDisturb/DB/Assertions.json");
    let out = Command::new("plutil").args(["-convert", "json", "-o", "-"]).arg(&path).stdin(Stdio::null()).stderr(Stdio::null()).output();
    match out {
        Ok(o) if o.status.success() => focus_active_in(&String::from_utf8_lossy(&o.stdout)),
        _ => std::fs::read_to_string(&path).map(|t| focus_active_in(&t)).unwrap_or(false),
    }
}

static SPEECH: OnceLock<Mutex<mpsc::Sender<Utterance>>> = OnceLock::new();

/// Speaks `text` with ElevenLabs: `(maya_dir, key, voice_id, text)`.
pub type ElevenSpeaker = fn(&std::path::Path, &str, &str, &str) -> Result<(), String>;

static ELEVEN_SPEAKER: OnceLock<ElevenSpeaker> = OnceLock::new();

/// Installs the ElevenLabs speaker. Once per process; a second call is
/// ignored. Without one, every line is spoken with the built-in voice.
pub fn set_eleven_speaker(speak: ElevenSpeaker) {
    let _ = ELEVEN_SPEAKER.set(speak);
}

/// Speaks with the built-in female voice.
#[cfg(windows)]
fn say_builtin(line: &str) {
    crate::notify_win::say(line)
}

/// Speaks with the built-in female voice.
#[cfg(target_os = "macos")]
fn say_builtin(line: &str) {
    let mut cmd = Command::new("say");
    if let Some(v) = voice() {
        cmd.args(["-v", &v]);
    }
    let _ = cmd.arg(line).status();
}

/// Speaks one line with its chosen voice. An ElevenLabs failure falls back to
/// the built-in voice unless the utterance says not to.
fn speak_line(u: &Utterance) -> Result<(), String> {
    if let (Some((dir, key, voice_id)), Some(speak)) = (&u.eleven, ELEVEN_SPEAKER.get()) {
        match speak(dir, key, voice_id, &u.text) {
            Ok(()) => return Ok(()),
            Err(e) if !u.fallback => return Err(e),
            Err(_) => {}
        }
    }
    say_builtin(&u.text);
    Ok(())
}

/// Speaks `u` with `speak`, calling `hook` before and after, then tells
/// whoever waits for it.
fn deliver(u: Utterance, hook: Option<&dyn Fn(SpeechPhase)>, speak: &dyn Fn(&Utterance) -> Result<(), String>) {
    if let Some(h) = hook {
        h(SpeechPhase::Starting(u.text.clone()));
    }
    crate::log::line("speech", format!("saying: {}", u.text));
    let result = speak(&u);
    if let Err(e) = &result {
        crate::log::line("speech", format!("failed: {e}"));
    }
    if let Some(h) = hook {
        h(SpeechPhase::Finished);
    }
    if let Some(done) = u.done {
        let _ = done.send(result);
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

/// Queues an utterance and waits until it has been spoken; the result is the
/// ElevenLabs error when the line had no fallback. Never call it holding a
/// lock the speech hook takes.
pub fn speak_and_wait(mut u: Utterance) -> Result<(), String> {
    let (tx, rx) = mpsc::channel();
    u.done = Some(tx);
    speak(u);
    rx.recv().map_err(|_| "the speech queue stopped".to_string())?
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

#[cfg(windows)]
pub fn notify(card: &Card, sound: bool) {
    crate::notify_win::show_toast(&card.name, &subtitle_for(card), &body_for(card), sound);
}

#[cfg(target_os = "macos")]
pub fn notify(card: &Card, sound: bool) {
    let _ = Command::new("osascript").arg("-e").arg(applescript_notify(&card.name, &subtitle_for(card), &body_for(card), sound)).output();
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

/// Speaks with speech-dispatcher or eSpeak NG, whichever is installed;
/// without either, logs once and stays silent.
#[cfg(target_os = "linux")]
fn say_builtin(line: &str) {
    static WARNED: std::sync::Once = std::sync::Once::new();
    match speech_command(&std::env::var("PATH").unwrap_or_default()) {
        Some((bin, args)) => {
            let _ = Command::new(bin).args(&args).arg(line).stdin(Stdio::null()).status();
        }
        None => WARNED.call_once(|| crate::log::line("notify", "no speech synthesiser: install speech-dispatcher or espeak-ng")),
    }
}

/// A GNOME banner through `notify-send`, with the freedesktop "message"
/// sound unless the line is being spoken instead.
#[cfg(target_os = "linux")]
pub fn notify(card: &Card, sound: bool) {
    let icon = app_icon_path();
    let _ = Command::new("notify-send").args(notify_send_args(icon.as_deref(), &card.name, &subtitle_for(card), &body_for(card))).stdin(Stdio::null()).output();
    if sound {
        let _ = Command::new("canberra-gtk-play").args(["-i", "message"]).stdin(Stdio::null()).status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AwaitKind, Awaiting, Card, Harness, State};

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
            stale: false,
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
        assert!(n.take_finished(&[done.clone()]).is_empty(), "already finished at startup: quiet");
        let again = card("a", State::Completed, 200, "");
        assert_eq!(n.take_finished(&[again.clone()]).len(), 1);
        assert!(n.take_finished(&[again.clone()]).is_empty());
        assert!(n.take_finished(&[card("a", State::Working, 300, "")]).is_empty());
    }

    #[test]
    fn seeded_cards_are_never_announced_but_later_changes_are() {
        let mut n = Notifier::default();
        assert!(n.take_new(&[]).is_empty() && n.take_finished(&[]).is_empty(), "primed");
        let asked = card("a", State::Awaiting, 100, "Go?");
        let done = card("b", State::Completed, 100, "");
        n.seed(&[asked.clone(), done.clone(), card("c", State::Working, 100, "")]);
        assert!(n.take_new(&[asked.clone(), done.clone()]).is_empty(), "a seeded ask is not announced");
        assert!(n.take_finished(&[asked.clone(), done.clone()]).is_empty(), "a seeded finished turn is not announced");
        let asked_again = card("a", State::Awaiting, 200, "And now?");
        let done_again = card("b", State::Completed, 200, "");
        assert_eq!(n.take_new(&[asked_again.clone()]).len(), 1);
        assert_eq!(n.take_finished(&[done_again.clone()]).len(), 1);
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
            stale: false,
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
        let u = Utterance { done: Some(tx), ..Utterance::new("hexgrid needs a decision".into(), None) };
        deliver(u, Some(&hook), &|u: &Utterance| {
            log.lock().unwrap().push(format!("speak {}", u.text));
            Err("no voice".into())
        });
        assert_eq!(*log.lock().unwrap(), vec!["Starting(\"hexgrid needs a decision\")", "speak hexgrid needs a decision", "Finished"]);
        assert_eq!(rx.recv().unwrap(), Err("no voice".to_string()), "the speaker's result reaches the waiter");
        // Without a hook (nothing listening) the line is still spoken.
        let spoken = std::sync::Mutex::new(0);
        deliver(Utterance::new("x".into(), None), None, &|_| {
            *spoken.lock().unwrap() += 1;
            Ok(())
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
}
