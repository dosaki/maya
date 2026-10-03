use crate::model::Harness;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum NetworkRole {
    #[default]
    Off,
    Main,
    Assistant,
}

/// An assistant the main has paired with; the token is what it must prove it holds.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PairedAssistant {
    pub id: String,
    pub name: String,
    pub hostname: String,
    pub platform: String,
    pub token: String,
    /// The peer's IP address as the main saw it at pairing or at its last
    /// authentication; what tells two machines apart (names are labels).
    #[serde(default)]
    pub address: String,
    /// When it last connected or disconnected (epoch millis), so a main
    /// restarted since still says when it last saw it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct NetworkConfig {
    pub role: NetworkRole,
    /// The main's listening port; 0 means `protocol::DEFAULT_PORT`.
    pub port: u16,
    pub main_host: String,
    pub main_port: u16,
    /// The assistant's display name; blank means the hostname.
    pub name: String,
    /// Minted by the main at pairing.
    pub assistant_id: String,
    /// The assistant's token, from pairing.
    pub token: String,
    /// The main's paired assistants.
    pub assistants: Vec<PairedAssistant>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub completed_timeout_minutes: u64,
    /// Folder whose subfolders are offered when starting a new session, e.g. `~/dev`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projects_dir: Option<String>,
    /// Post a system notification when a session starts waiting on the user.
    #[serde(default = "default_true")]
    pub notify_on_awaiting: bool,
    /// Speak "<name> needs a decision" / "<name> is finished" with the system
    /// voice instead of playing the notification sound.
    #[serde(default = "default_true")]
    pub speak_notifications: bool,
    /// Make no sound at all: no spoken lines and silent banners. Banners,
    /// the badge and the Dock bounce still come.
    #[serde(default)]
    pub muted: bool,
    /// Which voice speaks: the built-in one or ElevenLabs.
    #[serde(default)]
    pub voice_provider: VoiceProvider,
    /// The ElevenLabs voice to use; the key itself lives in the Keychain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elevenlabs_voice_id: Option<String>,
    /// Where PR review clones go when the project checkout is missing or busy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clones_dir: Option<String>,
    /// Listen for the "Maya" wake word while the app runs.
    #[serde(default)]
    pub listen: bool,
    /// Microphone name for the listener; None picks the built-in one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub microphone: Option<String>,
    /// The agent that powers Maya: it interprets voice commands, picks
    /// folders for "Let Maya choose", and is the default for new, resumed
    /// and review sessions. None until the first-start modal or Settings
    /// sets it; `brain()` then reads Claude Code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<Harness>,
    /// That agent's model id; "" means the agent's own default.
    #[serde(default)]
    pub agent_model: String,
    /// The prompt a review session starts with; "" means the built-in one
    /// (`reviews::DEFAULT_REVIEW_PROMPT`).
    #[serde(default)]
    pub review_prompt: String,
    /// The voice interpreter's model before 0.10, kept only to be read once
    /// by `migrate`; never written again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interpreter_model: Option<String>,
    /// System (Apple) or Built-in (whisper.cpp) recognition.
    #[serde(default)]
    pub recognizer: Recognizer,
    /// Whisper model id for the built-in recogniser.
    #[serde(default = "default_whisper_model")]
    pub whisper_model: String,
    #[serde(default)]
    pub network: NetworkConfig,
}

/// Which speech recogniser the listener uses.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Recognizer {
    #[default]
    System,
    Builtin,
}

pub const DEFAULT_CLONES_DIR: &str = "~/dev/reviews";

/// Which voice speaks Maya's lines: the built-in `say`, or ElevenLabs.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum VoiceProvider {
    #[default]
    Builtin,
    Elevenlabs,
}

/// The whisper model used until the user picks another.
pub const DEFAULT_MODEL: &str = "base.en-q5_1";

fn default_true() -> bool {
    true
}

fn default_whisper_model() -> String {
    DEFAULT_MODEL.into()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            completed_timeout_minutes: 30,
            projects_dir: None,
            notify_on_awaiting: true,
            speak_notifications: true,
            muted: false,
            voice_provider: Default::default(),
            elevenlabs_voice_id: None,
            clones_dir: None,
            listen: false,
            microphone: None,
            agent: None,
            agent_model: String::new(),
            review_prompt: String::new(),
            interpreter_model: None,
            recognizer: if cfg!(any(windows, target_os = "linux")) { Recognizer::Builtin } else { Recognizer::System },
            whisper_model: DEFAULT_MODEL.into(),
            network: NetworkConfig::default(),
        }
    }
}

impl Config {
    pub fn completed_timeout_ms(&self) -> u64 {
        self.completed_timeout_minutes * 60_000
    }

    /// The agent that powers Maya; Claude Code until one is chosen.
    pub fn brain(&self) -> Harness {
        self.agent.unwrap_or(Harness::ClaudeCode)
    }

    /// Maya's agent's model, or None for the agent's own default.
    pub fn brain_model(&self) -> Option<&str> {
        Some(self.agent_model.trim()).filter(|m| !m.is_empty())
    }

    /// Carries a pre-0.10 `interpreterModel` into `agentModel`, only when
    /// Maya's agent is (still) Claude Code, whose aliases it named.
    pub fn migrate(mut self) -> Config {
        if let Some(m) = self.interpreter_model.take() {
            if self.agent_model.trim().is_empty() && self.brain() == Harness::ClaudeCode {
                self.agent_model = m;
            }
        }
        self
    }

    /// The projects directory with `~` expanded, if configured.
    pub fn projects_dir_path(&self) -> Option<PathBuf> {
        self.projects_dir.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(expand_home)
    }

    /// The clones directory with `~` expanded; `DEFAULT_CLONES_DIR` when unset or blank.
    pub fn clones_dir_path(&self) -> PathBuf {
        expand_home(self.clones_dir.as_deref().map(str::trim).filter(|s| !s.is_empty()).unwrap_or(DEFAULT_CLONES_DIR))
    }

    pub fn listen_port(&self) -> u16 {
        if self.network.port == 0 {
            crate::net::protocol::DEFAULT_PORT
        } else {
            self.network.port
        }
    }

    pub fn main_port(&self) -> u16 {
        if self.network.main_port == 0 {
            crate::net::protocol::DEFAULT_PORT
        } else {
            self.network.main_port
        }
    }
}

/// Expands a leading `~` or `~/` to the home directory.
pub fn expand_home(s: &str) -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    if s == "~" {
        home
    } else if let Some(rest) = s.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(s)
    }
}

/// Missing or unreadable file gives `Config::default()`.
pub fn load(path: &Path) -> Config {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<Config>(&t).ok()).unwrap_or_default().migrate().for_this_platform()
}

impl Config {
    /// This config as this platform can run it: Windows and Linux have no
    /// system recogniser Maya drives, so the listener there always runs the
    /// built-in one.
    pub fn for_this_platform(mut self) -> Config {
        if cfg!(any(windows, target_os = "linux")) {
            self.recognizer = Recognizer::Builtin;
        }
        self
    }
}

/// Writes the config readable by its owner only: it holds the network token.
pub fn save(path: &Path, config: &Config) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    write_private(path, text.as_bytes()).map_err(|e| e.to_string())
}

/// Writes `bytes` to `path` with mode 0600: created that way, and an
/// existing file's mode is tightened before it is written.
#[cfg(unix)]
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    f.write_all(bytes)
}

/// Writes `bytes` to `path`. On Windows the file inherits the ACL of its
/// folder under the user's profile, which only the user (and SYSTEM and
/// administrators) can read.
#[cfg(windows)]
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn save_leaves_the_config_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("maya/config.json");
        save(&p, &Config::default()).unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        // A config an older version left world-readable is tightened too.
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        save(&p, &Config::default()).unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(load(&p), Config::default());
    }

    #[test]
    fn defaults_to_thirty_minutes_when_missing() {
        let c = load(Path::new("/nonexistent/config.json"));
        assert_eq!(c.completed_timeout_minutes, 30);
        assert_eq!(c.completed_timeout_ms(), 1_800_000);
    }

    #[test]
    fn expands_home_and_round_trips_projects_dir() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(expand_home("~/dev"), home.join("dev"));
        assert_eq!(expand_home("/abs/path"), Path::new("/abs/path"));
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes": 5}"#).unwrap();
        assert!(c.projects_dir.is_none());
        let c = Config { completed_timeout_minutes: 5, projects_dir: Some("~/dev".into()), ..Default::default() };
        let text = serde_json::to_string(&c).unwrap();
        assert!(text.contains("\"projectsDir\":\"~/dev\""));
        assert_eq!(c.projects_dir_path(), Some(home.join("dev")));
    }

    #[test]
    fn notifications_default_on_and_round_trip() {
        assert!(Config::default().notify_on_awaiting);
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes": 5}"#).unwrap();
        assert!(c.notify_on_awaiting, "older config files without the field keep notifying");
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes": 5, "notifyOnAwaiting": false}"#).unwrap();
        assert!(!c.notify_on_awaiting);
        assert!(serde_json::to_string(&c).unwrap().contains("\"notifyOnAwaiting\":false"));
    }

    #[test]
    fn speaking_defaults_on_and_round_trips() {
        assert!(Config::default().speak_notifications);
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes": 5}"#).unwrap();
        assert!(c.speak_notifications);
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes": 5, "speakNotifications": false}"#).unwrap();
        assert!(!c.speak_notifications);
        assert!(serde_json::to_string(&c).unwrap().contains("\"speakNotifications\":false"));
    }

    #[test]
    fn muting_defaults_off_and_round_trips() {
        assert!(!Config::default().muted);
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes": 5}"#).unwrap();
        assert!(!c.muted, "older config files without the field are not muted");
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes": 5, "muted": true}"#).unwrap();
        assert!(c.muted);
        assert!(serde_json::to_string(&c).unwrap().contains("\"muted\":true"));
    }

    #[test]
    fn voice_provider_defaults_to_builtin_and_round_trips() {
        assert_eq!(Config::default().voice_provider, VoiceProvider::Builtin);
        assert!(Config::default().elevenlabs_voice_id.is_none());
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes": 5, "voiceProvider": "elevenlabs", "elevenlabsVoiceId": "abc"}"#).unwrap();
        assert_eq!(c.voice_provider, VoiceProvider::Elevenlabs);
        assert_eq!(c.elevenlabs_voice_id.as_deref(), Some("abc"));
        let text = serde_json::to_string(&c).unwrap();
        assert!(text.contains("\"voiceProvider\":\"elevenlabs\""));
        assert!(text.contains("\"elevenlabsVoiceId\":\"abc\""));
    }

    #[test]
    fn clones_dir_defaults_under_home_and_expands() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(Config::default().clones_dir_path(), home.join("dev/reviews"));
        let c = Config { clones_dir: Some("~/tmp/clones".into()), ..Default::default() };
        assert_eq!(c.clones_dir_path(), home.join("tmp/clones"));
        let blank = Config { clones_dir: Some("  ".into()), ..Default::default() };
        assert_eq!(blank.clones_dir_path(), home.join("dev/reviews"));
        assert!(serde_json::to_string(&c).unwrap().contains("\"clonesDir\":\"~/tmp/clones\""));
    }

    #[test]
    fn voice_settings_default_off() {
        let c = Config::default();
        assert!(!c.listen);
        assert!(c.microphone.is_none());
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes": 5, "listen": true, "microphone": "USB Mic", "interpreterModel": "sonnet"}"#).unwrap();
        assert!(c.listen);
        assert_eq!(c.microphone.as_deref(), Some("USB Mic"));
    }

    #[test]
    fn agent_defaults_to_none_and_the_brain_to_claude_code() {
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes":30}"#).unwrap();
        assert_eq!(c.agent, None);
        assert_eq!(c.brain(), crate::model::Harness::ClaudeCode);
        assert_eq!(c.brain_model(), None);
        assert_eq!(c.review_prompt, "");
        let text = serde_json::to_string(&c).unwrap();
        assert!(!text.contains("\"agent\""), "an unset agent is not written: {text}");
        assert!(!text.contains("interpreterModel"), "{text}");
    }

    #[test]
    fn an_old_interpreter_model_becomes_the_claude_agent_model_only() {
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes":30,"interpreterModel":"sonnet"}"#).unwrap();
        let c = c.migrate();
        assert_eq!(c.agent_model, "sonnet");
        assert_eq!(c.interpreter_model, None);
        assert_eq!(c.brain_model(), Some("sonnet"));
        let codex: Config = serde_json::from_str(r#"{"completedTimeoutMinutes":30,"agent":"codex","interpreterModel":"sonnet"}"#).unwrap();
        assert_eq!(codex.migrate().agent_model, "", "a Claude alias is not carried to another agent");
        let kept: Config = serde_json::from_str(r#"{"completedTimeoutMinutes":30,"agentModel":"haiku","interpreterModel":"sonnet"}"#).unwrap();
        assert_eq!(kept.migrate().agent_model, "haiku", "a model already chosen wins");
    }

    #[test]
    fn agent_model_and_review_prompt_round_trip() {
        let c = Config { agent: Some(crate::model::Harness::Grok), agent_model: "grok-4.7".into(), review_prompt: "/should-i-approve".into(), ..Default::default() };
        let text = serde_json::to_string(&c).unwrap();
        assert!(text.contains("\"agent\":\"grok\""), "{text}");
        assert!(text.contains("\"agentModel\":\"grok-4.7\""), "{text}");
        assert!(text.contains("\"reviewPrompt\":\"/should-i-approve\""), "{text}");
        let back: Config = serde_json::from_str(&text).unwrap();
        assert_eq!(back.brain(), crate::model::Harness::Grok);
        assert_eq!(back.brain_model(), Some("grok-4.7"));
        assert_eq!(Config { agent_model: "   ".into(), ..Default::default() }.brain_model(), None, "blank means the agent's default");
    }

    #[test]
    fn recogniser_defaults_to_system_and_round_trips() {
        let c: Config = serde_json::from_str::<Config>(r#"{"completedTimeoutMinutes": 5}"#).unwrap().for_this_platform();
        assert_eq!(c.recognizer, if cfg!(any(windows, target_os = "linux")) { Recognizer::Builtin } else { Recognizer::System });
        assert_eq!(c.whisper_model, "base.en-q5_1");
        let c = Config { recognizer: Recognizer::Builtin, whisper_model: "tiny.en".into(), ..Default::default() };
        let text = serde_json::to_string(&c).unwrap();
        assert!(text.contains("\"recognizer\":\"builtin\""));
        assert!(text.contains("\"whisperModel\":\"tiny.en\""));
    }

    #[test]
    fn windows_and_linux_always_run_the_built_in_recogniser() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.json");
        save(&p, &Config { recognizer: Recognizer::System, ..Default::default() }).unwrap();
        let expected = if cfg!(any(windows, target_os = "linux")) { Recognizer::Builtin } else { Recognizer::System };
        assert_eq!(load(&p).recognizer, expected);
        assert_eq!(Config { recognizer: Recognizer::System, ..Default::default() }.for_this_platform().recognizer, expected);
    }

    #[test]
    fn round_trips_and_uses_camel_case() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nested/config.json");
        save(&p, &Config { completed_timeout_minutes: 5, projects_dir: None, ..Default::default() }).unwrap();
        assert!(std::fs::read_to_string(&p).unwrap().contains("completedTimeoutMinutes"));
        assert_eq!(load(&p).completed_timeout_minutes, 5);
    }

    #[test]
    fn network_defaults_to_off_and_round_trips() {
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes": 5}"#).unwrap();
        assert_eq!(c.network.role, NetworkRole::Off);
        assert_eq!(c.network.port, 0, "0 means the default port");
        assert!(c.network.assistants.is_empty());
        let mut c = Config::default();
        c.network.role = NetworkRole::Assistant;
        c.network.main_host = "10.0.0.2".into();
        c.network.token = "abc".into();
        c.network.assistants.push(PairedAssistant { id: "x".into(), name: "laptop".into(), hostname: "h".into(), platform: "macos".into(), token: "t".into(), address: "192.168.55.70".into(), last_seen: Some(1_700_000_000_000) });
        let text = serde_json::to_string(&c).unwrap();
        assert!(text.contains("\"role\":\"assistant\"") && text.contains("\"mainHost\":\"10.0.0.2\"") && text.contains("\"assistants\":[{"), "{text}");
        assert!(text.contains("\"address\":\"192.168.55.70\"") && text.contains("\"lastSeen\":1700000000000"), "{text}");
        assert_eq!(serde_json::from_str::<Config>(&text).unwrap(), c);
    }

    #[test]
    fn a_paired_assistant_saved_before_addresses_loads_without_one() {
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes": 5, "network": {"assistants": [{"id": "x", "name": "laptop", "hostname": "h", "platform": "macos", "token": "t"}]}}"#).unwrap();
        assert_eq!(c.network.assistants[0].address, "");
        assert_eq!(c.network.assistants[0].last_seen, None);
    }
}
