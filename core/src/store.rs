use crate::config::{self, Config};
use crate::events::EventLog;
use crate::foreign::{self, ForeignSession};
use crate::model::Harness;
use crate::model::{Card, PullRequest};
use crate::pr::PrCache;
use crate::registry::{self, RegistrySession};
use crate::state::{derive, DeriveInput};
use crate::transcript::TailCache;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub struct Store {
    claude_dir: PathBuf,
    codex_dir: PathBuf,
    agy_dir: PathBuf,
    grok_dir: PathBuf,
    /// Sessions of other harnesses, keyed by pid, kept between refreshes so
    /// `lsof` runs once per process rather than every five seconds.
    foreign: std::collections::HashMap<i32, ForeignSession>,
    /// The process lister, replaceable in tests.
    processes: Box<dyn Fn() -> Vec<(i32, String, Harness)> + Send>,
    events: EventLog,
    pub config: Config,
    alive: Box<dyn Fn(i32) -> bool + Send>,
    tails: TailCache,
    prs: PrCache,
    /// Compact the event log during refresh once it exceeds this many bytes.
    pub compact_threshold_bytes: u64,
}

pub const DEFAULT_COMPACT_THRESHOLD_BYTES: u64 = 5 * 1024 * 1024;

impl Store {
    pub fn new(claude_dir: PathBuf) -> Self {
        let config = config::load(&claude_dir.join("maya/config.json"));
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        Self {
            events: EventLog::new(claude_dir.join("maya/events.jsonl")),
            claude_dir,
            codex_dir: home.join(".codex"),
            agy_dir: home.join(".gemini/antigravity-cli"),
            grok_dir: home.join(".grok"),
            foreign: std::collections::HashMap::new(),
            processes: Box::new(foreign::list_tui_processes),
            config,
            alive: Box::new(registry::pid_alive),
            tails: TailCache::default(),
            prs: PrCache::default(),
            compact_threshold_bytes: DEFAULT_COMPACT_THRESHOLD_BYTES,
        }
    }

    #[cfg(test)]
    pub fn with_alive(mut self, alive: impl Fn(i32) -> bool + Send + 'static) -> Self {
        self.alive = Box::new(alive);
        // Tests never see the machine's real codex/agy/grok sessions.
        self.processes = Box::new(Vec::new);
        self.grok_dir = PathBuf::from("/nonexistent/grok");
        self
    }

    #[cfg(test)]
    pub fn with_foreign(mut self, codex_dir: PathBuf, agy_dir: PathBuf, sessions: Vec<ForeignSession>) -> Self {
        self.codex_dir = codex_dir;
        self.agy_dir = agy_dir.clone();
        self.grok_dir = agy_dir.join("no-grok");
        let pids: Vec<(i32, String, Harness)> = sessions.iter().map(|s| (s.pid, s.tty.clone().unwrap_or_default(), s.harness)).collect();
        self.foreign = sessions.into_iter().map(|s| (s.pid, s)).collect();
        self.processes = Box::new(move || pids.clone());
        self
    }

    /// Re-discovers foreign sessions: new pids are looked up, gone pids dropped.
    fn refresh_foreign(&mut self) {
        let procs = (self.processes)();
        let live: std::collections::HashSet<i32> = procs.iter().map(|p| p.0).collect();
        self.foreign.retain(|pid, _| live.contains(pid));
        let fresh: Vec<_> = procs.into_iter().filter(|p| !self.foreign.contains_key(&p.0)).collect();
        if !fresh.is_empty() {
            for s in foreign::discover(&fresh, foreign::proc_info, &self.codex_dir, &self.agy_dir) {
                self.foreign.insert(s.pid, s);
            }
        }
        // Grok keeps a registry of its own: cheap to read every refresh.
        let grok = foreign::grok_sessions(&self.grok_dir, &*self.alive, &|pid| crate::tty::tty_for_pid(pid).ok());
        let grok_pids: std::collections::HashSet<i32> = grok.iter().map(|s| s.pid).collect();
        self.foreign.retain(|pid, s| s.harness != Harness::Grok || grok_pids.contains(pid));
        for s in grok {
            self.foreign.entry(s.pid).or_insert(s);
        }
    }

    /// The foreign session with this id, if it is still running.
    pub fn foreign(&self, session_id: &str) -> Option<ForeignSession> {
        self.foreign.values().find(|s| s.session_id == session_id).cloned()
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

    /// Ids of every live session, any harness.
    pub fn live_session_ids(&self) -> Vec<String> {
        self.registry().into_iter().map(|s| s.session_id).chain(self.foreign.values().map(|s| s.session_id.clone())).collect()
    }

    /// Working directories of every live session, any harness.
    pub fn live_cwds(&self) -> Vec<String> {
        self.registry().into_iter().map(|s| s.cwd).chain(self.foreign.values().map(|s| s.cwd.clone())).collect()
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

    /// Directories of live sessions whose PR lookup is missing or stale.
    /// Dead sessions' directories are forgotten at the same time.
    pub fn pr_dirs_due(&mut self, now_ms: u64) -> Vec<String> {
        let mut dirs: Vec<String> = self.live_cwds();
        dirs.sort();
        dirs.dedup();
        self.prs.retain(&dirs);
        self.prs.due(&dirs, now_ms)
    }

    pub fn set_pr(&mut self, dir: &str, pr: Option<PullRequest>, now_ms: u64) {
        self.prs.set(dir, pr, now_ms);
    }

    /// True when the user's settings pick a 1M-context model by default.
    fn default_window_is_1m(&self) -> bool {
        std::fs::read_to_string(self.claude_dir.join("settings.json")).map(|t| crate::context::default_is_1m(&t)).unwrap_or(false)
    }

    pub fn refresh(&mut self, now_ms: u64) -> Vec<Card> {
        if self.events.file_len() > self.compact_threshold_bytes {
            self.compact_events();
        }
        let default_1m = self.default_window_is_1m();
        let _ = self.events.read_new();
        self.refresh_foreign();
        let sessions = self.registry();
        let mut cards = Vec::with_capacity(sessions.len() + self.foreign.len());
        let mut paths = Vec::with_capacity(sessions.len());
        for s in &sessions {
            let path = self.transcript_path_for(s);
            let tail = self.tails.get(&path);
            paths.push(path);
            let mut card = derive(&DeriveInput {
                registry: s,
                events: self.events.events_for(&s.session_id),
                transcript: &tail,
                now_ms,
                completed_timeout_ms: self.config.completed_timeout_ms(),
            });
            card.pr = self.prs.get(&s.cwd);
            card.context = tail.context_tokens.map(|used| crate::context::usage(tail.model.as_deref().unwrap_or(""), used, default_1m));
            cards.push(card);
        }
        self.tails.retain(&paths);
        let timeout = self.config.completed_timeout_ms();
        for s in self.foreign.values() {
            let tail = foreign::tail_for(s);
            let mut card = foreign::derive(s, &tail, now_ms, timeout);
            card.pr = self.prs.get(&s.cwd);
            cards.push(card);
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
    fn refresh_attaches_the_cached_pull_request_by_directory() {
        use crate::model::PullRequest;
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().to_path_buf();
        std::fs::create_dir_all(claude.join("sessions")).unwrap();
        std::fs::write(claude.join("sessions/7.json"), r#"{"pid":7,"sessionId":"s7","cwd":"/Users/x/dev/eye","name":"eye-7","status":"idle"}"#).unwrap();
        std::fs::write(claude.join("sessions/8.json"), r#"{"pid":8,"sessionId":"s8","cwd":"/Users/x/dev/other","name":"o-8","status":"idle"}"#).unwrap();
        let mut store = Store::new(claude).with_alive(|_| true);
        assert!(store.refresh(1).iter().all(|c| c.pr.is_none()));
        // Both live directories are due for a lookup at first.
        let mut due = store.pr_dirs_due(1);
        due.sort();
        assert_eq!(due, vec!["/Users/x/dev/eye".to_string(), "/Users/x/dev/other".to_string()]);
        let pr = PullRequest { number: 5, url: "https://github.com/o/r/pull/5".into(), state: "open".into() };
        store.set_pr("/Users/x/dev/eye", Some(pr.clone()), 1);
        store.set_pr("/Users/x/dev/other", None, 1);
        let cards = store.refresh(2);
        assert_eq!(cards.iter().find(|c| c.session_id == "s7").unwrap().pr, Some(pr));
        assert!(cards.iter().find(|c| c.session_id == "s8").unwrap().pr.is_none());
        assert!(store.pr_dirs_due(2).is_empty(), "a known result, including none, is not retried at once");
        assert_eq!(store.pr_dirs_due(2 + crate::pr::TTL_MS).len(), 2);
    }

    #[test]
    fn refresh_attaches_context_usage_using_the_settings_default_window() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().to_path_buf();
        std::fs::create_dir_all(claude.join("sessions")).unwrap();
        std::fs::create_dir_all(claude.join("projects/-Users-x-dev-eye")).unwrap();
        std::fs::write(claude.join("sessions/7.json"), r#"{"pid":7,"sessionId":"s7","cwd":"/Users/x/dev/eye","name":"eye-7","status":"idle"}"#).unwrap();
        std::fs::write(
            claude.join("projects/-Users-x-dev-eye/s7.jsonl"),
            r#"{"type":"assistant","message":{"model":"claude-opus-5","usage":{"input_tokens":0,"cache_read_input_tokens":150000,"cache_creation_input_tokens":0},"content":[{"type":"text","text":"hi"}]}}"#,
        ).unwrap();
        let mut store = Store::new(claude.clone()).with_alive(|_| true);
        let ctx = store.refresh(1)[0].context.clone().unwrap();
        assert_eq!((ctx.used, ctx.window, ctx.percent), (150_000, 200_000, 75));
        std::fs::write(claude.join("settings.json"), r#"{"model": "claude-opus-5[1m]"}"#).unwrap();
        let ctx = store.refresh(2)[0].context.clone().unwrap();
        assert_eq!((ctx.window, ctx.percent), (1_000_000, 15));
    }

    #[test]
    fn refresh_includes_foreign_sessions_with_their_own_state() {
        use crate::foreign::ForeignSession;
        use crate::model::{Harness, State};
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join("claude");
        std::fs::create_dir_all(claude.join("sessions")).unwrap();
        let codex = dir.path().join("codex");
        let agy = dir.path().join("agy");
        let rollout = dir.path().join("rollout.jsonl");
        std::fs::write(&rollout, "{\"timestamp\":\"2026-09-29T08:47:58.953Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\",\"turn_id\":\"t1\"}}\n").unwrap();
        let s = ForeignSession { harness: Harness::Codex, pid: 77, tty: Some("/dev/ttys009".into()), session_id: "aaa".into(), cwd: "/Users/x/dev/one".into(), name: "Fix CI".into(), transcript_path: rollout };
        let mut store = Store::new(claude).with_alive(|_| true).with_foreign(codex, agy, vec![s]);
        let cards = store.refresh(1_790_671_680_000);
        assert_eq!(cards.len(), 1);
        assert_eq!((cards[0].harness, cards[0].state, cards[0].name.as_str(), cards[0].pid), (Harness::Codex, State::Working, "Fix CI", 77));
        assert!(store.foreign("aaa").is_some());
        assert_eq!(store.live_session_ids(), vec!["aaa".to_string()]);
        assert_eq!(store.card_for("aaa", 1_790_671_680_000).unwrap().session_id, "aaa");
    }

    #[test]
    fn grok_sessions_come_from_its_registry_when_the_pid_is_alive() {
        let dir = tempfile::tempdir().unwrap();
        let grok = dir.path().join("grok");
        let sdir = crate::grok::session_dir(&grok, "/Users/x", "g1");
        std::fs::create_dir_all(&sdir).unwrap();
        std::fs::write(grok.join("active_sessions.json"), r#"[{"session_id":"g1","pid":11,"cwd":"/Users/x"},{"session_id":"g2","pid":12,"cwd":"/Users/x"}]"#).unwrap();
        std::fs::write(sdir.join("summary.json"), r#"{"generated_title":"Testing Grok Build"}"#).unwrap();
        std::fs::write(sdir.join("events.jsonl"), "{\"ts\":\"2026-09-29T14:02:15.012Z\",\"type\":\"turn_started\"}\n").unwrap();
        std::fs::write(sdir.join("chat_history.jsonl"), "{\"type\":\"assistant\",\"content\":\"Hi\"}\n").unwrap();
        let found = foreign::grok_sessions(&grok, &|pid| pid == 11, &|_| Some("/dev/ttys008".into()));
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].harness, found[0].pid, found[0].name.as_str(), found[0].session_id.as_str()), (Harness::Grok, 11, "Testing Grok Build", "g1"));
        let tail = foreign::tail_for(&found[0]);
        assert!(tail.working);
        assert_eq!(tail.last_agent_text.as_deref(), Some("Hi"));
        assert_eq!(foreign::turns_for(&found[0], 10).len(), 1);
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



