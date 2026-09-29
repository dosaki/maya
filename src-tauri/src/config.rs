use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
    /// Where PR review clones go when the project checkout is missing or busy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clones_dir: Option<String>,
}

pub const DEFAULT_CLONES_DIR: &str = "~/dev/reviews";

fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self { completed_timeout_minutes: 30, projects_dir: None, notify_on_awaiting: true, speak_notifications: true, clones_dir: None }
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
    fn round_trips_and_uses_camel_case() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nested/config.json");
        save(&p, &Config { completed_timeout_minutes: 5, projects_dir: None, ..Default::default() }).unwrap();
        assert!(std::fs::read_to_string(&p).unwrap().contains("completedTimeoutMinutes"));
        assert_eq!(load(&p).completed_timeout_minutes, 5);
    }
}
