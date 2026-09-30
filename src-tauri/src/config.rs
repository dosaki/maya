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
    /// Which voice speaks: the built-in one or ElevenLabs.
    #[serde(default)]
    pub voice_provider: crate::voice::VoiceProvider,
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
    /// Model alias for the voice interpreter.
    #[serde(default = "default_interpreter_model")]
    pub interpreter_model: String,
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

fn default_true() -> bool {
    true
}

fn default_interpreter_model() -> String {
    "haiku".into()
}

fn default_whisper_model() -> String {
    crate::models::DEFAULT_MODEL.into()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            completed_timeout_minutes: 30,
            projects_dir: None,
            notify_on_awaiting: true,
            speak_notifications: true,
            voice_provider: Default::default(),
            elevenlabs_voice_id: None,
            clones_dir: None,
            listen: false,
            microphone: None,
            interpreter_model: "haiku".into(),
            recognizer: Recognizer::System,
            whisper_model: crate::models::DEFAULT_MODEL.into(),
            network: NetworkConfig::default(),
        }
    }
}

impl Config {
    pub fn completed_timeout_ms(&self) -> u64 {
        self.completed_timeout_minutes * 60_000
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
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(path: &Path, config: &Config) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn voice_provider_defaults_to_builtin_and_round_trips() {
        assert_eq!(Config::default().voice_provider, crate::voice::VoiceProvider::Builtin);
        assert!(Config::default().elevenlabs_voice_id.is_none());
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes": 5, "voiceProvider": "elevenlabs", "elevenlabsVoiceId": "abc"}"#).unwrap();
        assert_eq!(c.voice_provider, crate::voice::VoiceProvider::Elevenlabs);
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
    fn voice_settings_default_off_with_haiku() {
        let c = Config::default();
        assert!(!c.listen);
        assert!(c.microphone.is_none());
        assert_eq!(c.interpreter_model, "haiku");
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes": 5, "listen": true, "microphone": "USB Mic", "interpreterModel": "sonnet"}"#).unwrap();
        assert!(c.listen);
        assert_eq!(c.microphone.as_deref(), Some("USB Mic"));
        assert_eq!(c.interpreter_model, "sonnet");
    }

    #[test]
    fn recogniser_defaults_to_system_and_round_trips() {
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes": 5}"#).unwrap();
        assert_eq!(c.recognizer, Recognizer::System);
        assert_eq!(c.whisper_model, "base.en-q5_1");
        let c = Config { recognizer: Recognizer::Builtin, whisper_model: "tiny.en".into(), ..Default::default() };
        let text = serde_json::to_string(&c).unwrap();
        assert!(text.contains("\"recognizer\":\"builtin\""));
        assert!(text.contains("\"whisperModel\":\"tiny.en\""));
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
