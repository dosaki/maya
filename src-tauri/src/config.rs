use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub completed_timeout_minutes: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self { completed_timeout_minutes: 30 }
    }
}

impl Config {
    pub fn completed_timeout_ms(&self) -> u64 {
        self.completed_timeout_minutes * 60_000
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
    fn round_trips_and_uses_camel_case() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nested/config.json");
        save(&p, &Config { completed_timeout_minutes: 5 }).unwrap();
        assert!(std::fs::read_to_string(&p).unwrap().contains("completedTimeoutMinutes"));
        assert_eq!(load(&p).completed_timeout_minutes, 5);
    }
}
