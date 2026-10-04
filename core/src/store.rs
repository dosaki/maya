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

/// What finds the sessions of `(pid, tty, harness)` processes, given the
/// Codex and Antigravity folders.
type Discover = dyn Fn(&[(i32, String, Harness)], &Path, &Path) -> Vec<ForeignSession> + Send;

pub struct Store {
    claude_dir: PathBuf,
    codex_dir: PathBuf,
    agy_dir: PathBuf,
    grok_dir: PathBuf,
    /// Kiro's sessions folder and its run folder (the turn markers).
    kiro_dir: PathBuf,
    kiro_run_dir: PathBuf,
    /// Where OpenCode's server writes its state file (`opencode/service.json` under it).
    opencode_state_dir: PathBuf,
    /// The open OpenCode terminals as `(pid, folder)`; replaceable in tests.
    opencode_windows: Box<dyn Fn() -> Vec<(i32, String)> + Send>,
    /// OpenCode's sessions as of the last refresh.
    opencode: Vec<crate::opencode::Fetched>,
    /// Sessions of other harnesses, keyed by pid, kept between refreshes so
    /// `lsof` runs once per process rather than every five seconds.
    foreign: std::collections::HashMap<i32, ForeignSession>,
    /// The process lister, replaceable in tests.
    processes: Box<dyn Fn() -> Vec<(i32, String, Harness)> + Send>,
    /// The process tree (pid, parent, tty), for agents whose session files
    /// name a pid with no terminal of its own; replaceable in tests.
    process_tree: Box<dyn Fn() -> Vec<foreign::Proc> + Send>,
    /// Finds the sessions of processes not yet known, from the Codex and
    /// Antigravity folders; replaceable in tests, where no `lsof` can.
    discover: Box<Discover>,
    events: EventLog,
    codex_events: EventLog,
    pub config: Config,
    alive: Box<dyn Fn(i32) -> bool + Send>,
    tails: TailCache,
    prs: PrCache,
    /// Compact the event log during refresh once it exceeds this many bytes.
    pub compact_threshold_bytes: u64,
    /// The last replies Maya sent each session, newest last: the session
    /// records them as messages from another session, and the history
    /// shows them as the user's own.
    sent: std::collections::HashMap<String, std::collections::VecDeque<String>>,
    /// Codex and Antigravity names, re-read only when their files change.
    names: NameFiles,
    /// Names chosen in the New session modal, waiting for their sessions.
    pending: crate::pending_names::PendingNames,
    /// Sessions to `/rename` now, accumulated until `take_due_renames` collects them.
    due_renames: Vec<(String, String)>,
    /// The agents installed on this machine, replaceable in tests.
    agents: std::sync::Arc<dyn Fn() -> Vec<crate::agents::AgentInfo> + Send + Sync>,
}

/// Text of the files names come from, read again only when they change.
#[derive(Default)]
struct NameFiles(std::collections::HashMap<PathBuf, (Option<std::time::SystemTime>, u64, std::sync::Arc<str>)>);

impl NameFiles {
    fn read(&mut self, path: &Path) -> std::sync::Arc<str> {
        let meta = std::fs::metadata(path).ok();
        let stamp = (meta.as_ref().and_then(|m| m.modified().ok()), meta.as_ref().map_or(0, |m| m.len()));
        if let Some((m, len, text)) = self.0.get(path) {
            if (*m, *len) == stamp {
                return text.clone();
            }
        }
        let text: std::sync::Arc<str> = std::fs::read_to_string(path).unwrap_or_default().into();
        self.0.insert(path.to_path_buf(), (stamp.0, stamp.1, text.clone()));
        text
    }
}

/// The sessions of `procs`, found the way this platform can: through `lsof`
/// on Unix, through the files each process holds or logs on Windows.
#[cfg(unix)]
fn discover_sessions(procs: &[(i32, String, Harness)], codex_dir: &Path, agy_dir: &Path) -> Vec<ForeignSession> {
    foreign::discover(procs, foreign::proc_info, codex_dir, agy_dir)
}

#[cfg(windows)]
fn discover_sessions(procs: &[(i32, String, Harness)], codex_dir: &Path, agy_dir: &Path) -> Vec<ForeignSession> {
    foreign::discover_from_files(procs, crate::win_process::holders, codex_dir, agy_dir)
}

/// Replies remembered per session.
const SENT_KEPT: usize = 50;

/// Hex SHA-256 of a reply as it reads trimmed: enough to recognise it in
/// the transcript, without keeping a second copy of what was said.
fn sent_hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.trim().as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

pub const DEFAULT_COMPACT_THRESHOLD_BYTES: u64 = 5 * 1024 * 1024;

/// Where OpenCode keeps its state: the XDG state dir (`~/.local/state`),
/// on Windows `%LOCALAPPDATA%\opencode\state`'s parent when that file
/// exists, else `~/.local/state` (unverified on Windows; see the README).
fn opencode_state_dir(home: &Path) -> PathBuf {
    #[cfg(windows)]
    if let Some(local) = dirs::data_local_dir() {
        if crate::opencode::service_file(&local.join("opencode/state")).exists() {
            return local.join("opencode/state");
        }
    }
    #[cfg(target_os = "linux")]
    if let Some(state) = dirs::state_dir() {
        return state;
    }
    home.join(".local/state")
}

/// The open OpenCode terminals: `opencode` processes on a tty, with the
/// folder each was opened in. Windows reads no process's folder.
#[cfg(unix)]
fn opencode_windows() -> Vec<(i32, String)> {
    foreign::list_process_tree()
        .into_iter()
        .filter(|p| p.command == "opencode" && p.tty.is_some())
        .filter_map(|p| Some((p.pid, foreign::proc_info(p.pid).cwd?)))
        .collect()
}

#[cfg(windows)]
fn opencode_windows() -> Vec<(i32, String)> {
    vec![]
}

impl Store {
    pub fn new(claude_dir: PathBuf) -> Self {
        let config = config::load(&claude_dir.join("maya/config.json"));
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        Self {
            codex_events: EventLog::new(claude_dir.join("maya/codex-events.jsonl")),
            events: EventLog::new(claude_dir.join("maya/events.jsonl")),
            claude_dir,
            codex_dir: crate::codex_hooks::dir(),
            agy_dir: home.join(".gemini/antigravity-cli"),
            grok_dir: home.join(".grok"),
            kiro_dir: home.join(".kiro/sessions/cli"),
            // `~/Library/Application Support` on macOS, `~/.local/share` on Linux, `%LOCALAPPDATA%` on Windows.
            kiro_run_dir: dirs::data_local_dir().unwrap_or_else(|| home.join(".local/share")).join("kiro-cli/run"),
            opencode_state_dir: opencode_state_dir(&home),
            opencode_windows: Box::new(opencode_windows),
            opencode: Vec::new(),
            foreign: std::collections::HashMap::new(),
            processes: Box::new(foreign::list_tui_processes),
            process_tree: Box::new(foreign::list_process_tree),
            discover: Box::new(discover_sessions),
            config,
            alive: Box::new(registry::pid_alive),
            tails: TailCache::default(),
            prs: PrCache::default(),
            compact_threshold_bytes: DEFAULT_COMPACT_THRESHOLD_BYTES,
            sent: Default::default(),
            names: NameFiles::default(),
            pending: Default::default(),
            due_renames: Default::default(),
            agents: std::sync::Arc::new(crate::agents::current),
        }
    }

    /// Where the hashes of replies Maya sent `session_id` are kept, so its
    /// history still shows them as the user's after Maya restarts. None for
    /// an id that is not a plain file name.
    fn sent_path(&self, session_id: &str) -> Option<PathBuf> {
        let plain = !session_id.is_empty() && session_id.len() <= 128 && session_id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        plain.then(|| self.claude_dir.join("maya").join("sent").join(format!("{session_id}.json")))
    }

    /// Remembers a reply Maya delivered to `session_id`: its hash, among the
    /// last `SENT_KEPT`, in memory and on disk.
    pub fn note_sent(&mut self, session_id: &str, text: &str) {
        let mut kept = self.sent_hashes(session_id);
        kept.push_back(sent_hash(text));
        while kept.len() > SENT_KEPT {
            kept.pop_front();
        }
        if let Some(path) = self.sent_path(session_id) {
            if let (Some(dir), Ok(json)) = (path.parent(), serde_json::to_string(&kept)) {
                let _ = std::fs::create_dir_all(dir).and_then(|_| config::write_private(&path, json.as_bytes()));
            }
        }
        self.sent.insert(session_id.to_string(), kept);
    }

    /// Hashes of the replies Maya delivered to `session_id`, oldest first.
    fn sent_hashes(&self, session_id: &str) -> std::collections::VecDeque<String> {
        if let Some(k) = self.sent.get(session_id) {
            return k.clone();
        }
        self.sent_path(session_id).and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    /// Whether Maya delivered `text` to `session_id`, in this run or an earlier one.
    pub fn was_sent(&self, session_id: &str, text: &str) -> bool {
        self.sent_hashes(session_id).contains(&sent_hash(text))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_alive(mut self, alive: impl Fn(i32) -> bool + Send + 'static) -> Self {
        self.alive = Box::new(alive);
        // Tests never see the machine's real codex/agy/grok sessions.
        self.processes = Box::new(Vec::new);
        self.process_tree = Box::new(Vec::new);
        self.grok_dir = PathBuf::from("/nonexistent/grok");
        self.kiro_dir = PathBuf::from("/nonexistent/kiro");
        self.opencode_state_dir = PathBuf::from("/nonexistent/state");
        self.opencode_windows = Box::new(Vec::new);
        // Nor do they list the machine's agents, which runs each one.
        self.agents = std::sync::Arc::new(|| vec![crate::agents::claude()]);
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_agents(mut self, list: Vec<crate::agents::AgentInfo>) -> Self {
        self.agents = std::sync::Arc::new(move || list.clone());
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn pending_apply_for_test(&mut self, cards: &mut [Card]) {
        self.pending.apply(cards, now_ms());
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn has_pending_name_for_test(&self, name: &str) -> bool {
        self.pending.names().iter().any(|n| n == name)
    }

    /// The folders waiting on a pending name, each already resolved
    /// physically (see `pending_names::physical`).
    #[cfg(any(test, feature = "test-support"))]
    pub fn pending_cwds_for_test(&self) -> Vec<String> {
        self.pending.cwds()
    }

    /// Where the installed agents come from; called without the store's lock
    /// held, since the first listing can take seconds.
    pub fn agents_source(&self) -> std::sync::Arc<dyn Fn() -> Vec<crate::agents::AgentInfo> + Send + Sync> {
        self.agents.clone()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_foreign(mut self, codex_dir: PathBuf, agy_dir: PathBuf, sessions: Vec<ForeignSession>) -> Self {
        self.codex_dir = codex_dir;
        self.agy_dir = agy_dir.clone();
        self.grok_dir = agy_dir.join("no-grok");
        self.kiro_dir = agy_dir.join("no-kiro");
        let pids: Vec<(i32, String, Harness)> = sessions.iter().map(|s| (s.pid, s.tty.clone().unwrap_or_default(), s.harness)).collect();
        self.foreign = sessions.into_iter().map(|s| (s.pid, s)).collect();
        self.processes = Box::new(move || pids.clone());
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_opencode(mut self, state_dir: PathBuf, windows: Vec<(i32, String)>) -> Self {
        self.opencode_state_dir = state_dir;
        self.opencode_windows = Box::new(move || windows.clone());
        self
    }

    /// OpenCode's server when its state file names one that is alive.
    pub fn opencode_service(&self) -> Option<crate::opencode::Service> {
        crate::opencode::read_service(&self.opencode_state_file()).filter(|s| (self.alive)(s.pid))
    }

    pub fn opencode_state_file(&self) -> PathBuf {
        crate::opencode::service_file(&self.opencode_state_dir)
    }

    /// The OpenCode card with this id, as of the last refresh.
    pub fn opencode_card(&self, session_id: &str) -> Option<Card> {
        self.opencode.iter().find(|f| f.session.id == session_id).map(|f| f.card.clone())
    }

    /// The OpenCode session and what the server said about it, as of the last refresh.
    pub fn opencode_session(&self, session_id: &str) -> Option<(crate::opencode::SessionInfo, crate::opencode::Live)> {
        self.opencode.iter().find(|f| f.session.id == session_id).map(|f| (f.session.clone(), f.live.clone()))
    }

    /// The newest OpenCode terminal open on `dir`.
    pub fn opencode_window(&self, dir: &str) -> Option<i32> {
        (self.opencode_windows)().into_iter().filter(|(_, d)| d == dir).map(|(pid, _)| pid).max()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_kiro(mut self, sessions_dir: PathBuf, run_dir: PathBuf, procs: Vec<foreign::Proc>) -> Self {
        self.kiro_dir = sessions_dir;
        self.kiro_run_dir = run_dir;
        self.process_tree = Box::new(move || procs.clone());
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_agent_dirs(mut self, codex: PathBuf, agy: PathBuf, grok: PathBuf) -> Self {
        self.codex_dir = codex;
        self.agy_dir = agy;
        self.grok_dir = grok;
        self
    }

    /// Processes running alongside the ones the store already knows, which
    /// the next refresh discovers as `sessions`: a session started by hand
    /// moments ago that no refresh has looked up yet.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_processes(mut self, sessions: Vec<ForeignSession>) -> Self {
        let mut pids: Vec<(i32, String, Harness)> = self.foreign.values().map(|s| (s.pid, s.tty.clone().unwrap_or_default(), s.harness)).collect();
        pids.extend(sessions.iter().map(|s| (s.pid, s.tty.clone().unwrap_or_default(), s.harness)));
        self.processes = Box::new(move || pids.clone());
        self.discover = Box::new(move |fresh, _, _| sessions.iter().filter(|s| fresh.iter().any(|p| p.0 == s.pid)).cloned().collect());
        self
    }

    /// Re-discovers foreign sessions: new pids are looked up, gone pids dropped.
    fn refresh_foreign(&mut self) {
        let procs = (self.processes)();
        let live: std::collections::HashSet<i32> = procs.iter().map(|p| p.0).collect();
        self.foreign.retain(|pid, _| live.contains(pid));
        // On Windows an agy process's log is re-read every refresh, cheaply, so
        // a conversation it starts or switches to replaces the one it had.
        let recheck = |h: Harness| cfg!(windows) && h == Harness::Antigravity;
        let fresh: Vec<_> = procs.into_iter().filter(|p| !self.foreign.contains_key(&p.0) || recheck(p.2)).collect();
        if !fresh.is_empty() {
            for s in (self.discover)(&fresh, &self.codex_dir, &self.agy_dir) {
                self.foreign.insert(s.pid, s);
            }
        }
        // Grok keeps a registry of its own: cheap to read every refresh.
        let grok = foreign::grok_sessions(&self.grok_dir, &*self.alive, &|pid| crate::tty::tty_for_pid(pid).ok());
        let grok_pids: std::collections::HashSet<i32> = grok.iter().map(|s| s.pid).collect();
        self.foreign.retain(|pid, s| s.harness != Harness::Grok || grok_pids.contains(pid));
        for s in grok {
            // Grok's title changes with `/rename`: keep the newest.
            self.foreign.entry(s.pid).and_modify(|e| e.name = s.name.clone()).or_insert(s);
        }
        // Kiro's locks name a pid too, and its title changes with `/rename`.
        let kiro = foreign::kiro_sessions(&self.kiro_dir, &(self.process_tree)(), std::process::id() as i32);
        let kiro_pids: std::collections::HashSet<i32> = kiro.iter().map(|s| s.pid).collect();
        self.foreign.retain(|pid, s| s.harness != Harness::Kiro || kiro_pids.contains(pid));
        for s in kiro {
            self.foreign.entry(s.pid).and_modify(|e| e.name = s.name.clone()).or_insert(s);
        }
        self.refresh_names();
    }

    /// Re-reads Codex and Antigravity names, which change with `/rename`.
    fn refresh_names(&mut self) {
        let has = |foreign: &std::collections::HashMap<i32, ForeignSession>, h: Harness| foreign.values().any(|s| s.harness == h);
        if has(&self.foreign, Harness::Codex) {
            let index = self.names.read(&self.codex_dir.join("session_index.jsonl"));
            for s in self.foreign.values_mut().filter(|s| s.harness == Harness::Codex) {
                if let Some(n) = crate::codex::thread_name(&index, &s.session_id) {
                    s.name = n;
                }
            }
        }
        if has(&self.foreign, Harness::Antigravity) {
            let history = self.names.read(&self.agy_dir.join("history.jsonl"));
            for s in self.foreign.values_mut().filter(|s| s.harness == Harness::Antigravity) {
                if let Some(n) = crate::antigravity::conversation_name(&history, &s.session_id) {
                    s.name = n;
                }
            }
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

    /// Where each agent keeps its sessions.
    pub fn agent_dirs(&self) -> crate::resume::AgentDirs {
        crate::resume::AgentDirs { claude: self.claude_dir.clone(), codex: self.codex_dir.clone(), agy: self.agy_dir.clone(), grok: self.grok_dir.clone(), kiro: self.kiro_dir.clone() }
    }

    fn registry(&self) -> Vec<RegistrySession> {
        registry::list(&self.claude_dir.join("sessions"), &*self.alive)
    }

    /// Ids of every live session, any harness.
    pub fn live_session_ids(&self) -> Vec<String> {
        self.registry().into_iter().map(|s| s.session_id).chain(self.foreign.values().map(|s| s.session_id.clone())).chain(self.opencode.iter().map(|f| f.session.id.clone())).collect()
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

    /// Drops event-log lines for sessions no longer in the registry. A Codex
    /// thread is kept while its writer lock exists, which holds from its
    /// start, whether or not a refresh has matched it to a process yet (at
    /// startup none has).
    pub fn compact_events(&mut self) {
        let keep: HashSet<String> = self.registry().into_iter().map(|s| s.session_id).collect();
        let _ = self.events.compact(&keep);
        let mut keep: HashSet<String> = crate::codex::locked_threads(&self.codex_dir).into_iter().map(|(id, _)| id).collect();
        keep.extend(self.foreign.values().filter(|s| s.harness == Harness::Codex).map(|s| s.session_id.clone()));
        let _ = self.codex_events.compact(&keep);
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
        if self.events.file_len() > self.compact_threshold_bytes || self.codex_events.file_len() > self.compact_threshold_bytes {
            self.compact_events();
        }
        let default_1m = self.default_window_is_1m();
        let _ = self.events.read_new();
        let _ = self.codex_events.read_new();
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
            let mut tail = foreign::tail_for(s);
            if s.harness == Harness::Kiro {
                crate::kiro::apply_live(&mut tail, &self.kiro_run_dir, s.pid, &s.transcript_path, now_ms);
            }
            if s.harness == Harness::Codex {
                crate::codex_hooks::apply(&mut tail, self.codex_events.events_for(&s.session_id));
            }
            let mut card = foreign::derive(s, &tail, now_ms, timeout);
            card.pr = self.prs.get(&s.cwd);
            cards.push(card);
        }
        // OpenCode's sessions live in its server: asked on every refresh while it runs.
        self.opencode = match self.opencode_service() {
            Some(svc) => {
                let client = crate::opencode::Client::for_poll(&svc);
                let windows = (self.opencode_windows)();
                let data_dir = self.claude_dir.join("maya").to_string_lossy().into_owned();
                // The context limit comes from the listing when one has landed; a refresh never waits for it.
                let models: Vec<crate::agents::ModelInfo> = crate::agents::cached().unwrap_or_default().into_iter().find(|a| a.harness == Harness::OpenCode).map(|a| a.models).unwrap_or_default();
                crate::opencode::fetch(&client, svc.pid, &windows, &data_dir, now_ms, timeout, |p, m| models.iter().find(|x| x.id == format!("{p}/{m}")).and_then(|x| x.context))
            }
            None => vec![],
        };
        cards.extend(self.opencode.iter().map(|f| f.card.clone()));
        let due = self.pending.apply(&mut cards, now_ms);
        self.due_renames.extend(due);
        self.log_pending_events();
        cards
    }

    /// Writes what happened to the pending names to Maya's log, so a name
    /// that was never typed says why.
    fn log_pending_events(&mut self) {
        for e in self.pending.take_events() {
            crate::log::line("rename", e);
        }
    }

    pub fn add_pending_name(&mut self, p: crate::pending_names::PendingName) {
        self.pending.add(p);
    }

    pub fn forget_pending_name(&mut self, session_id: &str) {
        self.pending.forget(session_id);
        self.due_renames.retain(|(id, _)| id != session_id);
        self.log_pending_events();
    }

    /// Puts a due name back to wait: its session got busy before `/rename`
    /// was typed. It is due again once a refresh sees the session free.
    pub fn requeue_rename(&mut self, session_id: &str) {
        self.pending.requeue(session_id);
    }

    /// Sessions to `/rename` now, each returned once.
    pub fn take_due_renames(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.due_renames)
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_maya_sent_are_remembered_per_session_and_capped() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::new(dir.path().to_path_buf());
        store.note_sent("a", "  hello
");
        assert!(store.was_sent("a", "hello"));
        assert!(!store.was_sent("b", "hello"), "per session");
        assert!(!store.was_sent("a", "hell"));
        for i in 0..SENT_KEPT {
            store.note_sent("a", &i.to_string());
        }
        assert!(!store.was_sent("a", "hello"), "the oldest goes once more than {SENT_KEPT} are kept");
        assert!(store.was_sent("a", "0"));
        // A restarted Maya still knows them, from hashes rather than the text.
        let again = Store::new(dir.path().to_path_buf());
        assert!(again.was_sent("a", "0") && again.was_sent("a", &(SENT_KEPT - 1).to_string()));
        assert!(!again.was_sent("a", "hello"));
        let saved = std::fs::read_to_string(dir.path().join("maya/sent/a.json")).unwrap();
        assert!(!saved.contains("\"0\""), "no reply text on disk: {saved}");
        // An id that is not a file name is remembered for this run only.
        store.note_sent("../x", "hi");
        assert!(store.was_sent("../x", "hi"));
        assert!(!dir.path().join("maya/x.json").exists());
    }
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
        crate::hook_install::append_record_named(&store.claude_dir.join("maya"), "codex-events.jsonl",
            r#"{"session_id":"aaa","hook_event_name":"PermissionRequest","tool_input":{"command":"cargo test"}}"#, 1_800_000_000_000).unwrap();
        let card = store.card_for("aaa", 1_800_000_000_001).unwrap();
        assert_eq!(card.state, State::Awaiting);
        assert_eq!(card.awaiting.unwrap().detail, "Approve: cargo test");
        crate::hook_install::append_record_named(&store.claude_dir.join("maya"), "codex-events.jsonl",
            r#"{"session_id":"aaa","hook_event_name":"Stop"}"#, 1_800_000_000_002).unwrap();
        assert_eq!(store.card_for("aaa", 1_800_000_000_003).unwrap().state, State::Completed);
    }

    #[test]
    fn opencode_sessions_come_from_its_server_and_a_dead_pid_makes_no_call() {
        let (base, hits) = crate::opencode::fake::fake_server(vec![
            ("GET /api/session?limit=50", include_str!("../fixtures/opencode/sessions.json")),
            ("GET /api/session/active", "{\"data\":{\"ses_efe57d285ffeRMX70aswKZ9qCO\":{\"type\":\"running\"}}}"),
            ("GET /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/permission", "{\"data\":[]}"),
            ("GET /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/form", "{\"data\":[]}"),
            ("GET /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/message?limit=5&order=desc", include_str!("../fixtures/opencode/messages.json")),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir_all(state.join("opencode")).unwrap();
        let me = std::process::id();
        std::fs::write(state.join("opencode/service.json"), format!("{{\"url\":\"{base}\",\"pid\":{me},\"password\":\"pw\"}}")).unwrap();
        let mut store = Store::new(dir.path().join("claude")).with_alive(move |pid| pid == me as i32).with_opencode(state.clone(), vec![(31, "/Users/tiagocorreia".into())]);
        let cards = store.refresh(1_791_029_170_000);
        let c = cards.iter().find(|c| c.session_id == "ses_efe57d285ffeRMX70aswKZ9qCO").expect("the running session");
        assert_eq!((c.harness, c.state, c.pid, c.snippet.as_str(), c.name.as_str()), (Harness::OpenCode, State::Working, 31, "Hi", "Saying \"Hi\" request"));
        assert!(store.live_session_ids().contains(&c.session_id));
        assert!(cards.iter().filter(|c| c.harness == Harness::OpenCode).count() >= 2, "the completed ones within the window show too");
        assert!(store.opencode_card("ses_efe57d285ffeRMX70aswKZ9qCO").is_some());
        assert_eq!(store.opencode_window("/Users/tiagocorreia"), Some(31));
        // The server died but left its file: nothing is asked.
        std::fs::write(state.join("opencode/service.json"), format!("{{\"url\":\"{base}\",\"pid\":999999,\"password\":\"pw\"}}")).unwrap();
        let before = hits.lock().unwrap().len();
        let cards = store.refresh(1_791_029_170_000);
        assert!(cards.iter().all(|c| c.harness != Harness::OpenCode));
        assert_eq!(hits.lock().unwrap().len(), before, "no request to a dead server");
        assert!(store.opencode_service().is_none());
    }

    #[test]
    fn kiro_sessions_come_from_their_locks_and_show_working_from_the_marker() {
        let dir = tempfile::tempdir().unwrap();
        let (sessions, run) = (dir.path().join("kiro"), dir.path().join("kiro-run"));
        std::fs::create_dir_all(run.join("turn-markers")).unwrap();
        std::fs::create_dir_all(&sessions).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/kiro");
        let id = "efcba1da-5c0b-4b39-96f9-9a4740ead331";
        for (from, to) in [("lock.json", "lock"), ("session.json", "json"), ("events.jsonl", "jsonl")] {
            std::fs::copy(fixtures.join(from), sessions.join(format!("{id}.{to}"))).unwrap();
        }
        std::fs::copy(fixtures.join("marker.json"), run.join("turn-markers/95441-1791030218653.json")).unwrap();
        let alive = crate::kiro::markers(&run)[0].alive_ms;
        let procs = foreign::parse_ps_tree("95245 93903 ttys010 kiro-cli chat\n95295 95245 ttys010 kiro-cli-chat chat\n95441 95295 ttys010 bun tui.js chat\n95508 95441 ?? kiro-cli-chat acp\n");
        let mut store = Store::new(dir.path().join("claude")).with_alive(|_| true).with_kiro(sessions, run, procs);
        let now = alive + 500;
        let cards = store.refresh(now);
        let k = cards.iter().find(|c| c.harness == Harness::Kiro).expect("a Kiro card");
        assert_eq!((k.pid, k.name.as_str(), k.cwd.as_str(), k.state), (95441, "run ls", "/Users/tiagocorreia", State::Working));
        assert_eq!(k.snippet, "shell: mkdir -p /tmp/kiro-probe && sleep 90");
        assert_eq!(k.context.as_ref().map(|c| c.percent), Some(1));
        assert!(store.live_session_ids().contains(&id.to_string()));
        // The marker's heartbeat stops: the turn is over, the card is Completed.
        let later = now + crate::kiro::MARKER_STALE_MS + 1;
        let k = store.refresh(later).into_iter().find(|c| c.harness == Harness::Kiro).unwrap();
        assert_eq!(k.state, State::Completed);
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

    use crate::foreign::ForeignSession;

    #[test]
    fn a_codex_rename_reaches_the_card_on_the_next_refresh() {
        let t = tempfile::tempdir().unwrap();
        let codex = t.path().join("codex");
        std::fs::create_dir_all(&codex).unwrap();
        let rollout = codex.join("rollout.jsonl");
        std::fs::write(&rollout, "").unwrap();
        std::fs::write(codex.join("session_index.jsonl"), "{\"id\":\"c1\",\"thread_name\":\"Say hi\"}\n").unwrap();
        let s = ForeignSession { harness: Harness::Codex, pid: 42, tty: Some("ttys001".into()), session_id: "c1".into(), cwd: "/x".into(), name: "Say hi".into(), transcript_path: rollout };
        let mut store = Store::new(t.path().join("claude")).with_alive(|_| true).with_foreign(codex.clone(), t.path().join("agy"), vec![s]);
        assert_eq!(store.refresh(1).iter().find(|c| c.session_id == "c1").unwrap().name, "Say hi");
        std::fs::write(codex.join("session_index.jsonl"), "{\"id\":\"c1\",\"thread_name\":\"Say hi\"}\n{\"id\":\"c1\",\"thread_name\":\"Fix CI\"}\n").unwrap();
        assert_eq!(store.refresh(2).iter().find(|c| c.session_id == "c1").unwrap().name, "Fix CI");
    }

    #[test]
    fn an_antigravity_rename_reaches_the_card_on_the_next_refresh() {
        let t = tempfile::tempdir().unwrap();
        let agy = t.path().join("agy");
        std::fs::create_dir_all(&agy).unwrap();
        let s = ForeignSession { harness: Harness::Antigravity, pid: 43, tty: Some("ttys002".into()), session_id: "a1".into(), cwd: "/x".into(), name: "agy-43".into(), transcript_path: agy.join("t.jsonl") };
        let mut store = Store::new(t.path().join("claude")).with_alive(|_| true).with_foreign(t.path().join("codex"), agy.clone(), vec![s]);
        std::fs::write(agy.join("history.jsonl"), "{\"display\":\"/rename Fix CI\",\"timestamp\":3,\"workspace\":\"/x\",\"conversationId\":\"a1\",\"type\":\"slash_command\"}\n").unwrap();
        assert_eq!(store.refresh(1).iter().find(|c| c.session_id == "a1").unwrap().name, "Fix CI");
    }
}



