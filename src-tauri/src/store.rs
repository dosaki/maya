use crate::config::{self, Config};
use crate::events::EventLog;
use crate::model::Card;
use crate::registry::{self, RegistrySession};
use crate::state::{derive, DeriveInput};
use crate::transcript::{self, TAIL_BYTES};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub struct Store {
    claude_dir: PathBuf,
    events: EventLog,
    pub config: Config,
    alive: Box<dyn Fn(i32) -> bool + Send>,
}

impl Store {
    pub fn new(claude_dir: PathBuf) -> Self {
        let config = config::load(&claude_dir.join("eye/config.json"));
        Self {
            events: EventLog::new(claude_dir.join("eye/events.jsonl")),
            claude_dir,
            config,
            alive: Box::new(registry::pid_alive),
        }
    }

    #[cfg(test)]
    pub fn with_alive(mut self, alive: impl Fn(i32) -> bool + Send + 'static) -> Self {
        self.alive = Box::new(alive);
        self
    }

    pub fn config_path(&self) -> PathBuf {
        self.claude_dir.join("eye/config.json")
    }

    pub fn claude_dir(&self) -> &Path {
        &self.claude_dir
    }

    fn registry(&self) -> Vec<RegistrySession> {
        registry::list(&self.claude_dir.join("sessions"), &*self.alive)
    }

    /// Drops event-log lines for sessions no longer in the registry.
    pub fn compact_events(&mut self) {
        let keep: HashSet<String> = self.registry().into_iter().map(|s| s.session_id).collect();
        let _ = self.events.compact(&keep);
    }

    pub fn refresh(&mut self, now_ms: u64) -> Vec<Card> {
        let _ = self.events.read_new();
        let sessions = self.registry();
        let mut cards = Vec::with_capacity(sessions.len());
        for s in &sessions {
            let path = match self.events.transcript_path_for(&s.session_id) {
                Some(p) => PathBuf::from(p),
                None => registry::transcript_path(&self.claude_dir, s),
            };
            let tail = transcript::read_tail(&path, TAIL_BYTES);
            cards.push(derive(&DeriveInput {
                registry: s,
                events: self.events.events_for(&s.session_id),
                transcript: &tail,
                now_ms,
                completed_timeout_ms: self.config.completed_timeout_ms(),
            }));
        }
        cards
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;

    #[test]
    fn refresh_builds_cards_from_a_fake_claude_dir() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().to_path_buf();
        std::fs::create_dir_all(claude.join("sessions")).unwrap();
        std::fs::create_dir_all(claude.join("eye")).unwrap();
        std::fs::create_dir_all(claude.join("projects/-Users-x-dev-eye")).unwrap();

        std::fs::write(
            claude.join("sessions/7.json"),
            r#"{"pid":7,"sessionId":"s7","cwd":"/Users/x/dev/eye","name":"eye-7","status":"idle","startedAt":1,"statusUpdatedAt":100}"#,
        ).unwrap();
        std::fs::write(
            claude.join("projects/-Users-x-dev-eye/s7.jsonl"),
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"All done."}]}}"#,
        ).unwrap();
        std::fs::write(
            claude.join("eye/events.jsonl"),
            "{\"session_id\":\"s7\",\"hook_event_name\":\"Stop\",\"received_at\":200}\n{\"session_id\":\"gone\",\"hook_event_name\":\"Stop\",\"received_at\":201}\n",
        ).unwrap();

        let mut store = Store::new(claude.clone()).with_alive(|_| true);
        store.compact_events();
        let cards = store.refresh(300);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].name, "eye-7");
        assert_eq!(cards[0].state, State::Completed);
        assert_eq!(cards[0].snippet, "All done.");

        let log = std::fs::read_to_string(claude.join("eye/events.jsonl")).unwrap();
        assert!(!log.contains("gone"));
    }
}
