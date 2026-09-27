use crate::config::{self, Config};
use crate::events::EventLog;
use crate::model::Card;
use crate::registry::{self, RegistrySession};
use crate::state::{derive, DeriveInput};
use crate::transcript::TailCache;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub struct Store {
    claude_dir: PathBuf,
    events: EventLog,
    pub config: Config,
    alive: Box<dyn Fn(i32) -> bool + Send>,
    tails: TailCache,
    /// Compact the event log during refresh once it exceeds this many bytes.
    pub compact_threshold_bytes: u64,
}

pub const DEFAULT_COMPACT_THRESHOLD_BYTES: u64 = 5 * 1024 * 1024;

impl Store {
    pub fn new(claude_dir: PathBuf) -> Self {
        let config = config::load(&claude_dir.join("maya/config.json"));
        Self {
            events: EventLog::new(claude_dir.join("maya/events.jsonl")),
            claude_dir,
            config,
            alive: Box::new(registry::pid_alive),
            tails: TailCache::default(),
            compact_threshold_bytes: DEFAULT_COMPACT_THRESHOLD_BYTES,
        }
    }

    #[cfg(test)]
    pub fn with_alive(mut self, alive: impl Fn(i32) -> bool + Send + 'static) -> Self {
        self.alive = Box::new(alive);
        self
    }

    pub fn config_path(&self) -> PathBuf {
        self.claude_dir.join("maya/config.json")
    }

    pub fn claude_dir(&self) -> &Path {
        &self.claude_dir
    }

    fn registry(&self) -> Vec<RegistrySession> {
        registry::list(&self.claude_dir.join("sessions"), &*self.alive)
    }

    /// The live registry entry for a session id, if it is still running.
    pub fn session(&self, session_id: &str) -> Option<RegistrySession> {
        self.registry().into_iter().find(|s| s.session_id == session_id)
    }

    /// Hook-supplied transcript path when we have one, else the derived path.
    pub fn transcript_path_for(&self, s: &RegistrySession) -> PathBuf {
        match self.events.transcript_path_for(&s.session_id) {
            Some(p) => PathBuf::from(p),
            None => registry::transcript_path(&self.claude_dir, s),
        }
    }

    /// The current card for one session, or None when it is not running.
    pub fn card_for(&mut self, session_id: &str, now_ms: u64) -> Option<Card> {
        self.refresh(now_ms).into_iter().find(|c| c.session_id == session_id)
    }

    /// Drops event-log lines for sessions no longer in the registry.
    pub fn compact_events(&mut self) {
        let keep: HashSet<String> = self.registry().into_iter().map(|s| s.session_id).collect();
        let _ = self.events.compact(&keep);
    }

    pub fn refresh(&mut self, now_ms: u64) -> Vec<Card> {
        if self.events.file_len() > self.compact_threshold_bytes {
            self.compact_events();
        }
        let _ = self.events.read_new();
        let sessions = self.registry();
        let mut cards = Vec::with_capacity(sessions.len());
        let mut paths = Vec::with_capacity(sessions.len());
        for s in &sessions {
            let path = self.transcript_path_for(s);
            let tail = self.tails.get(&path);
            paths.push(path);
            cards.push(derive(&DeriveInput {
                registry: s,
                events: self.events.events_for(&s.session_id),
                transcript: &tail,
                now_ms,
                completed_timeout_ms: self.config.completed_timeout_ms(),
            }));
        }
        self.tails.retain(&paths);
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
    fn refresh_compacts_the_log_when_it_grows_past_the_threshold() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().to_path_buf();
        std::fs::create_dir_all(claude.join("sessions")).unwrap();
        std::fs::create_dir_all(claude.join("maya")).unwrap();
        std::fs::write(
            claude.join("sessions/7.json"),
            r#"{"pid":7,"sessionId":"s7","cwd":"/Users/x/dev/eye","name":"eye-7","status":"idle","startedAt":1,"statusUpdatedAt":100}"#,
        ).unwrap();
        let mut log = String::new();
        for i in 0..50 {
            log.push_str(&format!("{{\"session_id\":\"gone\",\"hook_event_name\":\"PostToolUse\",\"received_at\":{i}}}\n"));
        }
        std::fs::write(claude.join("maya/events.jsonl"), &log).unwrap();
        let mut store = Store::new(claude.clone()).with_alive(|_| true);
        store.compact_threshold_bytes = 1024;
        store.refresh(300);
        let after = std::fs::read_to_string(claude.join("maya/events.jsonl")).unwrap();
        assert!(after.is_empty(), "log should have been compacted: {} bytes", after.len());
    }

    #[test]
    fn session_lookup_and_transcript_path_prefer_hook_supplied_path() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().to_path_buf();
        std::fs::create_dir_all(claude.join("sessions")).unwrap();
        std::fs::create_dir_all(claude.join("maya")).unwrap();
        std::fs::write(claude.join("sessions/7.json"), r#"{"pid":7,"sessionId":"s7","cwd":"/Users/x/dev/eye","name":"eye-7","status":"idle"}"#).unwrap();
        std::fs::write(claude.join("maya/events.jsonl"), "{\"session_id\":\"s7\",\"hook_event_name\":\"Stop\",\"transcript_path\":\"/hooked/s7.jsonl\",\"received_at\":1}\n").unwrap();
        let mut store = Store::new(claude.clone()).with_alive(|_| true);
        store.refresh(2);
        let s = store.session("s7").expect("live session");
        assert_eq!(store.transcript_path_for(&s), PathBuf::from("/hooked/s7.jsonl"));
        assert!(store.session("nope").is_none());
    }

    #[test]
    fn card_for_returns_the_derived_card_or_none() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().to_path_buf();
        std::fs::create_dir_all(claude.join("sessions")).unwrap();
        std::fs::write(claude.join("sessions/7.json"), r#"{"pid":7,"sessionId":"s7","cwd":"/Users/x/dev/eye","name":"eye-7","status":"idle"}"#).unwrap();
        let mut store = Store::new(claude).with_alive(|_| true);
        assert_eq!(store.card_for("s7", 1).unwrap().name, "eye-7");
        assert!(store.card_for("nope", 1).is_none());
    }

    #[test]
    fn refresh_builds_cards_from_a_fake_claude_dir() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().to_path_buf();
        std::fs::create_dir_all(claude.join("sessions")).unwrap();
        std::fs::create_dir_all(claude.join("maya")).unwrap();
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
            claude.join("maya/events.jsonl"),
            "{\"session_id\":\"s7\",\"hook_event_name\":\"Stop\",\"received_at\":200}\n{\"session_id\":\"gone\",\"hook_event_name\":\"Stop\",\"received_at\":201}\n",
        ).unwrap();

        let mut store = Store::new(claude.clone()).with_alive(|_| true);
        store.compact_events();
        let cards = store.refresh(300);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].name, "eye-7");
        assert_eq!(cards[0].state, State::Completed);
        assert_eq!(cards[0].snippet, "All done.");

        let log = std::fs::read_to_string(claude.join("maya/events.jsonl")).unwrap();
        assert!(!log.contains("gone"));
    }
}
