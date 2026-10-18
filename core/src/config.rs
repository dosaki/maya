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

/// A Maya the main has paired with; `key` is who it is.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PairedAssistant {
    pub id: String,
    pub name: String,
    pub hostname: String,
    pub platform: String,
    /// Its public key (base64), pinned at pairing; empty for a protocol 1
    /// entry, which `drop_insecure_pairings` removes.
    #[serde(default)]
    pub key: String,
    /// The peer's IP address as the main last saw it; shown, not trusted.
    #[serde(default)]
    pub address: String,
    /// When it last connected or disconnected (epoch millis), so a main
    /// restarted since still says when it last saw it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<u64>,
    /// May see and control every session in the pool: set at pairing when
    /// the Maya asked (`CAP_MIRROR`), and by the main's switch.
    #[serde(default)]
    pub mirror: bool,
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
    /// The main's public key (base64), pinned at pairing; empty until then.
    pub main_key: String,
    /// Protocol 1 pairings were dropped on update: Settings says to pair again.
    pub pair_again: bool,
    /// The main's paired assistants.
    pub assistants: Vec<PairedAssistant>,
    /// A protocol 1 assistant's token: read only to tell a pairing that
    /// was dropped from a host typed but never paired; never written.
    #[serde(default, rename = "token", skip_serializing)]
    pub legacy_token: String,
}

impl NetworkConfig {
    /// Protocol 1 pairings (tokens, no keys) cannot connect any more: they
    /// go, and Settings tells the user to pair again.
    pub fn drop_insecure_pairings(&mut self) {
        let before = self.assistants.len();
        self.assistants.retain(|a| !a.key.is_empty());
        // Whatever the role now: an assistant switched Off still held a link.
        let stale_link = self.main_key.is_empty() && !self.legacy_token.is_empty();
        if self.assistants.len() != before || stale_link {
            self.pair_again = true;
        }
    }
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
    /// Post a notification when a session's turn finishes. Read by the
    /// phone; the desktop announces finished turns under `notify_on_awaiting`.
    #[serde(default = "default_true")]
    pub notify_on_completed: bool,
    /// Post a notification when a PR assigned to the user is approved or
    /// has changes requested.
    #[serde(default = "default_true")]
    pub notify_on_pr_review: bool,
    /// Speak "<name> needs a decision" / "<name> is finished" with the system
    /// voice instead of playing the notification sound.
    #[serde(default = "default_true")]
    pub speak_notifications: bool,
    /// The phone listens for "Maya" while its app is on screen. The desktop ignores it.
    #[serde(default)]
    pub wake_word: bool,
    /// Make no sound at all: no spoken lines and silent banners. Banners,
    /// the badge and the Dock bounce still come.
    #[serde(default)]
    pub muted: bool,
    /// Which voice speaks: the built-in one or ElevenLabs.
    #[serde(default)]
    pub voice_provider: VoiceProvider,
    /// How loud Maya speaks, in percent; read through `voice_volume()`.
    #[serde(default = "default_voice_volume")]
    pub voice_volume: u8,
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
    /// GitHub and GitLab. None in a config from before 0.24 (and a fresh
    /// one): the first start fills it in after importing `gh`'s token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<Vec<GitConnection>>,
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

/// A code host Maya reads pull requests from.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum GitProvider {
    #[default]
    Github,
    Gitlab,
}

impl GitProvider {
    pub fn label(self) -> &'static str {
        match self {
            GitProvider::Github => "GitHub",
            GitProvider::Gitlab => "GitLab",
        }
    }

    /// The variable a new connection reads its token from.
    pub fn default_env(self) -> &'static str {
        match self {
            GitProvider::Github => "GH_TOKEN",
            GitProvider::Gitlab => "GITLAB_TOKEN",
        }
    }

    /// What goes between a repo and a number: `repo#12` on GitHub, `repo!12` on GitLab.
    pub fn sigil(self) -> char {
        match self {
            GitProvider::Github => '#',
            GitProvider::Gitlab => '!',
        }
    }
}

pub const GITHUB_URL: &str = "https://github.com";
pub const GITLAB_DEFAULT_URL: &str = "https://gitlab.com";

/// Where a connection's token comes from: the OS key store, or a variable
/// read from the login shell.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TokenSource {
    Stored,
    Env { name: String },
}

/// GitHub or GitLab, switched on or off, with where its token comes from.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitConnection {
    pub provider: GitProvider,
    pub enabled: bool,
    /// GitLab only; None is `GITLAB_DEFAULT_URL`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    pub token: TokenSource,
}

impl GitConnection {
    /// GitHub on, GitLab off, each reading its default variable.
    pub fn default_for(provider: GitProvider) -> Self {
        GitConnection { provider, enabled: provider == GitProvider::Github, base_url: None, token: TokenSource::Env { name: provider.default_env().into() } }
    }

    /// The web address repos live under, without a trailing slash.
    pub fn web_base(&self) -> String {
        match self.provider {
            GitProvider::Github => GITHUB_URL.into(),
            GitProvider::Gitlab => self.base_url.as_deref().map(str::trim).filter(|u| !u.is_empty()).unwrap_or(GITLAB_DEFAULT_URL).trim_end_matches('/').to_string(),
        }
    }

    /// The variable to read, the provider's default when blank; None for a stored token.
    pub fn env_name(&self) -> Option<String> {
        match &self.token {
            TokenSource::Stored => None,
            TokenSource::Env { name } => {
                let name = name.trim();
                Some(if name.is_empty() { self.provider.default_env() } else { name }.to_string())
            }
        }
    }
}

/// True for a shell variable name: letters, digits and `_`, not starting
/// with a digit. Only such a name goes into the login-shell probe.
pub fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Why the Git settings cannot be saved, if they cannot.
pub fn validate_git(conns: &[GitConnection]) -> Result<(), String> {
    for c in conns {
        if let Some(url) = c.base_url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
            let host = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).and_then(|r| r.split('/').next()).filter(|h| !h.is_empty());
            if host.is_none() {
                return Err(format!("{} URL must be an http(s) address.", c.provider.label()));
            }
        }
        if let TokenSource::Env { name } = &c.token {
            let n = name.trim();
            if !n.is_empty() && !valid_env_name(n) {
                return Err(format!("The {} token variable must be a name like {}.", c.provider.label(), c.provider.default_env()));
            }
        }
    }
    Ok(())
}

/// The connections the first start saves: GitHub on with `gh`'s token when
/// it could be stored, else reading `GH_TOKEN`; GitLab off.
pub fn git_after_import(stored: bool) -> Vec<GitConnection> {
    let mut github = GitConnection::default_for(GitProvider::Github);
    if stored {
        github.token = TokenSource::Stored;
    }
    vec![github, GitConnection::default_for(GitProvider::Gitlab)]
}

/// The whisper model used until the user picks another.
pub const DEFAULT_MODEL: &str = "base.en-q5_1";

fn default_true() -> bool {
    true
}

/// The quietest the voice goes: at 0 a line would still play, in silence.
pub const MIN_VOICE_VOLUME: u8 = 10;
/// The engines' own volume, in percent, that the slider's 100% plays at:
/// their full volume was too loud.
pub const FULL_VOICE_LOUDNESS: u8 = 60;

fn default_voice_volume() -> u8 {
    100
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
            notify_on_completed: true,
            notify_on_pr_review: true,
            speak_notifications: true,
            wake_word: false,
            muted: false,
            voice_provider: Default::default(),
            voice_volume: default_voice_volume(),
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
            git: None,
            network: NetworkConfig::default(),
        }
    }
}

impl Config {
    /// The voice volume in percent, kept within `MIN_VOICE_VOLUME`..=100.
    pub fn voice_volume(&self) -> u8 {
        self.voice_volume.clamp(MIN_VOICE_VOLUME, 100)
    }

    /// What the voice engines are told, in percent of their own full
    /// volume: the slider's value scaled so that 100% is `FULL_VOICE_LOUDNESS`.
    pub fn voice_loudness(&self) -> u8 {
        (u16::from(self.voice_volume()) * u16::from(FULL_VOICE_LOUDNESS) / 100) as u8
    }

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
        self.network.drop_insecure_pairings();
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

    /// One GitHub and one GitLab connection, in that order: the saved ones,
    /// with defaults for any missing.
    pub fn git_connections(&self) -> Vec<GitConnection> {
        [GitProvider::Github, GitProvider::Gitlab]
            .into_iter()
            .map(|p| self.git.as_ref().and_then(|g| g.iter().find(|c| c.provider == p)).cloned().unwrap_or_else(|| GitConnection::default_for(p)))
            .collect()
    }

    /// Trims the Git settings as typed: the URL's spaces, trailing slashes
    /// and a pasted `/api/v4` (blank becomes None), and the variable name.
    pub fn normalize_git(&mut self) {
        for c in self.git.iter_mut().flatten() {
            c.base_url = c.base_url.as_deref().map(|u| {
                let u = u.trim().trim_end_matches('/');
                u.strip_suffix("/api/v4").unwrap_or(u).trim_end_matches('/').to_string()
            }).filter(|u| !u.is_empty());
            if let TokenSource::Env { name } = &mut c.token {
                *name = name.trim().to_string();
            }
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

/// Writes the config readable by its owner only.
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

    #[test]
    fn a_paired_entry_saved_before_mirroring_existed_may_not_mirror() {
        let old = r#"{"id":"a","name":"laptop","hostname":"h","platform":"macos","token":"t"}"#;
        let a: PairedAssistant = serde_json::from_str(old).unwrap();
        assert!(!a.mirror);
    }

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
    fn the_wake_word_defaults_off_and_round_trips() {
        assert!(!Config::default().wake_word);
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes":30,"wakeWord":true}"#).unwrap();
        assert!(c.wake_word);
        let old: Config = serde_json::from_str(r#"{"completedTimeoutMinutes":30}"#).unwrap();
        assert!(!old.wake_word);
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
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes": 5, "listen": true, "microphone": "USB Mic"}"#).unwrap();
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
    fn voice_volume_defaults_to_full_and_stays_in_range() {
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes": 30}"#).unwrap();
        assert_eq!(c.voice_volume(), 100, "a config from before the slider reads as the slider's 100%");
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes": 30, "voiceVolume": 60}"#).unwrap();
        assert_eq!(c.voice_volume(), 60);
        assert_eq!(serde_json::to_value(&c).unwrap()["voiceVolume"], 60);
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes": 30, "voiceVolume": 3}"#).unwrap();
        assert_eq!(c.voice_volume(), MIN_VOICE_VOLUME, "never so quiet that a line plays in silence");
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes": 30, "voiceVolume": 200}"#).unwrap();
        assert_eq!(c.voice_volume(), 100);
    }

    #[test]
    fn the_slider_full_volume_is_sixty_percent_of_the_engines() {
        let at = |v: u8| Config { voice_volume: v, ..Config::default() }.voice_loudness();
        assert_eq!(Config::default().voice_loudness(), 60, "100% on the slider is what 60% used to be");
        assert_eq!(at(100), 60);
        assert_eq!(at(50), 30);
        assert_eq!(at(10), 6);
        assert_eq!(at(0), 6, "clamped to the slider's minimum first");
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
        c.network.main_key = "abc".into();
        c.network.assistants.push(PairedAssistant { id: "x".into(), name: "laptop".into(), hostname: "h".into(), platform: "macos".into(), key: "k".into(), address: "192.168.55.70".into(), last_seen: Some(1_700_000_000_000), mirror: false });
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

    #[test]
    fn protocol_1_pairings_are_dropped_and_the_user_told_to_pair_again() {
        let old = r#"{"completedTimeoutMinutes":5,"network":{"role":"main","assistants":[{"id":"x","name":"laptop","hostname":"h","platform":"macos","token":"t"},{"id":"y","name":"mini","hostname":"h","platform":"macos","key":"a2V5"}]}}"#;
        let c = serde_json::from_str::<Config>(old).unwrap().migrate();
        assert_eq!(c.network.assistants.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["y"]);
        assert!(c.network.pair_again);
        let old_link = r#"{"completedTimeoutMinutes":5,"network":{"role":"assistant","mainHost":"desk.local","assistantId":"a","token":"t"}}"#;
        assert!(serde_json::from_str::<Config>(old_link).unwrap().migrate().network.pair_again);
        let switched_off = r#"{"completedTimeoutMinutes":5,"network":{"role":"off","mainHost":"desk.local","assistantId":"a","token":"t"}}"#;
        assert!(serde_json::from_str::<Config>(switched_off).unwrap().migrate().network.pair_again, "an old link switched Off is told too");
        let typed_only = r#"{"completedTimeoutMinutes":5,"network":{"role":"assistant","mainHost":"desk.local"}}"#;
        assert!(!serde_json::from_str::<Config>(typed_only).unwrap().migrate().network.pair_again, "a host typed but never paired");
        let resaved = serde_json::to_string(&serde_json::from_str::<Config>(old_link).unwrap()).unwrap();
        assert!(!resaved.contains("token"), "the protocol 1 token is never written back");
        let fresh = Config::default().migrate();
        assert!(!fresh.network.pair_again, "nothing was paired, nothing to say");
    }

    #[test]
    fn notify_on_pr_review_defaults_on_and_reads_from_older_files() {
        assert!(Config::default().notify_on_pr_review);
        let older: Config = serde_json::from_str(r#"{"completedTimeoutMinutes": 30}"#).unwrap();
        assert!(older.notify_on_pr_review, "a file written before the field reads as on");
        let text = serde_json::to_string(&Config { notify_on_pr_review: false, ..Config::default() }).unwrap();
        assert!(text.contains(r#""notifyOnPrReview":false"#));
    }

    #[test]
    fn notify_on_completed_defaults_on_and_reads_from_older_files() {
        assert!(Config::default().notify_on_completed);
        let older: Config = serde_json::from_str(r#"{"completedTimeoutMinutes": 30, "notifyOnAwaiting": false}"#).unwrap();
        assert!(older.notify_on_completed, "a file written before the field reads as on");
        assert!(!older.notify_on_awaiting);
        let text = serde_json::to_string(&Config { notify_on_completed: false, ..Config::default() }).unwrap();
        assert!(text.contains(r#""notifyOnCompleted":false"#));
    }

    #[test]
    fn git_defaults_to_github_on_reading_gh_token_and_gitlab_off() {
        let c = Config::default();
        assert_eq!(c.git, None, "no git field until the first start imports gh's token");
        let conns = c.git_connections();
        assert_eq!(conns.len(), 2);
        assert_eq!((conns[0].provider, conns[0].enabled), (GitProvider::Github, true));
        assert_eq!(conns[0].token, TokenSource::Env { name: "GH_TOKEN".into() });
        assert_eq!((conns[1].provider, conns[1].enabled, conns[1].base_url.clone()), (GitProvider::Gitlab, false, None));
        assert_eq!(conns[1].token, TokenSource::Env { name: "GITLAB_TOKEN".into() });
    }

    #[test]
    fn an_older_config_reads_with_no_git_field_and_round_trips_once_set() {
        let older: Config = serde_json::from_str(r#"{"completedTimeoutMinutes":30}"#).unwrap();
        assert_eq!(older.git, None);
        let set = Config { git: Some(git_after_import(true)), ..Config::default() };
        let text = serde_json::to_string(&set).unwrap();
        assert!(text.contains(r#""git":[{"provider":"github","enabled":true,"token":{"kind":"stored"}}"#), "{text}");
        assert!(text.contains(r#"{"provider":"gitlab","enabled":false,"token":{"kind":"env","name":"GITLAB_TOKEN"}}"#), "{text}");
        let back: Config = serde_json::from_str(&text).unwrap();
        assert_eq!(back.git, set.git);
    }

    #[test]
    fn git_connections_fills_in_a_missing_provider_and_keeps_the_order() {
        let only_gitlab = GitConnection { provider: GitProvider::Gitlab, enabled: true, base_url: Some("https://gitlab.acme.io".into()), token: TokenSource::Stored };
        let c = Config { git: Some(vec![only_gitlab.clone()]), ..Config::default() };
        let conns = c.git_connections();
        assert_eq!(conns[0], GitConnection::default_for(GitProvider::Github));
        assert_eq!(conns[1], only_gitlab);
    }

    #[test]
    fn the_import_stores_github_as_a_pasted_token_or_falls_back_to_gh_token() {
        assert_eq!(git_after_import(true)[0].token, TokenSource::Stored);
        assert_eq!(git_after_import(false)[0].token, TokenSource::Env { name: "GH_TOKEN".into() });
        assert!(git_after_import(true)[0].enabled);
        assert!(!git_after_import(true)[1].enabled, "GitLab starts off");
    }

    #[test]
    fn web_base_is_github_or_the_gitlab_url_defaulting_to_gitlab_com() {
        assert_eq!(GitConnection::default_for(GitProvider::Github).web_base(), "https://github.com");
        let mut gl = GitConnection::default_for(GitProvider::Gitlab);
        assert_eq!(gl.web_base(), "https://gitlab.com");
        gl.base_url = Some("  ".into());
        assert_eq!(gl.web_base(), "https://gitlab.com", "blank is the default");
        gl.base_url = Some("https://acme.io/gitlab/".into());
        assert_eq!(gl.web_base(), "https://acme.io/gitlab");
    }

    #[test]
    fn env_name_is_the_default_when_blank() {
        let mut c = GitConnection::default_for(GitProvider::Github);
        c.token = TokenSource::Env { name: " ".into() };
        assert_eq!(c.env_name().as_deref(), Some("GH_TOKEN"));
        c.token = TokenSource::Env { name: " MY_TOKEN ".into() };
        assert_eq!(c.env_name().as_deref(), Some("MY_TOKEN"));
        c.token = TokenSource::Stored;
        assert_eq!(c.env_name(), None);
    }

    #[test]
    fn env_names_are_letters_digits_and_underscores_only() {
        for ok in ["GH_TOKEN", "_x", "a1"] {
            assert!(valid_env_name(ok), "{ok}");
        }
        for bad in ["", "1A", "GH TOKEN", "GH_TOKEN; rm -rf ~", "$(id)", "A-B", "É"] {
            assert!(!valid_env_name(bad), "{bad}");
        }
    }

    #[test]
    fn validate_git_refuses_a_bad_url_or_variable_name() {
        let gl = |url: &str| GitConnection { base_url: Some(url.into()), ..GitConnection::default_for(GitProvider::Gitlab) };
        assert!(validate_git(&[gl("https://gitlab.acme.io")]).is_ok());
        assert!(validate_git(&[gl("http://10.0.0.5:8080/gitlab")]).is_ok());
        assert!(validate_git(&[gl("")]).is_ok(), "blank means gitlab.com");
        assert_eq!(validate_git(&[gl("gitlab.acme.io")]).unwrap_err(), "GitLab URL must be an http(s) address.");
        assert_eq!(validate_git(&[gl("https://")]).unwrap_err(), "GitLab URL must be an http(s) address.");
        let bad_env = GitConnection { token: TokenSource::Env { name: "GH_TOKEN; rm -rf ~".into() }, ..GitConnection::default_for(GitProvider::Github) };
        assert_eq!(validate_git(&[bad_env]).unwrap_err(), "The GitHub token variable must be a name like GH_TOKEN.");
        let blank_env = GitConnection { token: TokenSource::Env { name: "".into() }, ..GitConnection::default_for(GitProvider::Github) };
        assert!(validate_git(&[blank_env]).is_ok(), "blank means the default");
    }

    #[test]
    fn normalize_git_trims_the_url_and_a_pasted_api_suffix_and_the_name() {
        let mut c = Config {
            git: Some(vec![
                GitConnection { token: TokenSource::Env { name: " GH_TOKEN ".into() }, ..GitConnection::default_for(GitProvider::Github) },
                GitConnection { base_url: Some(" https://gitlab.acme.io/api/v4/ ".into()), ..GitConnection::default_for(GitProvider::Gitlab) },
            ]),
            ..Config::default()
        };
        c.normalize_git();
        let g = c.git.unwrap();
        assert_eq!(g[0].token, TokenSource::Env { name: "GH_TOKEN".into() });
        assert_eq!(g[1].base_url.as_deref(), Some("https://gitlab.acme.io"));
        let mut blank = Config { git: Some(vec![GitConnection { base_url: Some("  ".into()), ..GitConnection::default_for(GitProvider::Gitlab) }]), ..Config::default() };
        blank.normalize_git();
        assert_eq!(blank.git.unwrap()[0].base_url, None);
        let mut none = Config::default();
        none.normalize_git();
        assert_eq!(none.git, None, "nothing to normalise stays unimported");
    }
}
