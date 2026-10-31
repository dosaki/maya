use crate::asks::{self, AskCache, Verdict};
use crate::collect;
use crate::config::{self, Config};
use crate::events::EventLog;
use crate::foreign::{self, ForeignSession};
use crate::model::Harness;
use crate::model::{Card, PullRequest, State};
use crate::pr::PrCache;
use crate::remembered::{Key, LiveSession, Memory, Remembered};
use crate::registry::{self, RegistrySession};
use crate::state::{derive, DeriveInput, Prose};
use crate::transcript::TailCache;
use crate::usage::{AccountLimits, SessionUsage};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// What finds the sessions of `(pid, tty, harness)` processes, given the
/// Codex and Antigravity folders.
type Discover = dyn Fn(&[(i32, String, Harness)], &Path, &Path) -> Vec<ForeignSession> + Send;

/// Told the roots of the Native terminals a refresh found cards under.
pub type NativeSeen = Box<dyn Fn(&[i32]) + Send>;

pub struct Store {
    claude_dir: PathBuf,
    codex_dir: PathBuf,
    agy_dir: PathBuf,
    grok_dir: PathBuf,
    /// Kiro's sessions folder and its run folder (the turn markers).
    kiro_dir: PathBuf,
    kiro_run_dir: PathBuf,
    /// OpenCode's server state file (`<state dir>/opencode/service.json`, or
    /// wherever the platform keeps it).
    opencode_state_file: PathBuf,
    /// The open OpenCode terminals as `(pid, folder)`; replaceable in tests.
    opencode_windows: Box<dyn Fn() -> Vec<(i32, String)> + Send>,
    /// The pids of the programs Maya started in terminals she hosts; a
    /// session under one of them is Native. Replaced by the native host.
    native_roots: Box<dyn Fn() -> Vec<i32> + Send>,
    /// Told, on each refresh with Native sessions, the roots that a card was
    /// found under. Replaced by the native host.
    native_seen: NativeSeen,
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
    /// Claude Code transcripts' running token totals.
    totals: crate::usage::TranscriptTotals,
    /// What the Claude Code status lines last recorded, every file in `maya/`.
    recorded: crate::statusline::RecordedCache,
    prs: PrCache,
    /// Antigravity's and Kiro's account limits, read by collectors off the lock.
    agy_limits: LimitsSlot,
    kiro_limits: LimitsSlot,
    /// Grok's tokens and cost per session id, from `grok usage`.
    grok_usage: std::collections::HashMap<String, GrokSlot>,
    /// Compact the event log during refresh once it exceeds this many bytes.
    pub compact_threshold_bytes: u64,
    /// Codex and Antigravity names, re-read only when their files change.
    names: NameFiles,
    /// Names chosen in the New session modal, waiting for their sessions.
    pending: crate::pending_names::PendingNames,
    /// Sessions to `/rename` now, accumulated until `take_due_renames` collects them.
    due_renames: Vec<(String, String)>,
    /// The agents installed on this machine, replaceable in tests.
    agents: std::sync::Arc<dyn Fn() -> Vec<crate::agents::AgentInfo> + Send + Sync>,
    /// The sessions this machine remembers across restarts, once
    /// `start_remembering` ran (the app and `maya run` do; other CLI
    /// commands and most tests do not).
    memory: Option<Memory>,
    /// The last save of the memory failed: the streak was logged already.
    memory_save_failing: bool,
    /// The session id refresh drops from the cards and from memory (Maya's own brain).
    hidden_session: Option<String>,
    /// Ask a model whether a finished message asks the user something (the app
    /// does); off, the wording check decides.
    pub judge_asks: bool,
    asks: AskCache,
    /// `(key, text)` of messages to judge, until `take_due_asks` collects them.
    due_asks: Vec<(String, String)>,
    /// Keys handed out or waiting to be, until their verdict or failure arrives.
    queued: HashSet<String>,
    /// The sessions held on their judging by the last refresh.
    judging: HashSet<String>,
    asks_save_failing: bool,
}

/// The finished message of a Claude Code session: its turn end, else its last text.
fn claude_text(tail: &crate::transcript::TranscriptTail) -> Option<&str> {
    tail.turn_end.as_ref().map(|t| t.text.as_str()).or(tail.last_assistant_text.as_deref())
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

/// One harness's account limits as its collector last read them.
#[derive(Default)]
struct LimitsSlot {
    limits: Option<AccountLimits>,
    /// When the collector was last handed out or last answered.
    ran_ms: Option<u64>,
    /// A window opened on a stale reading: due at the next hand-out.
    nudged: bool,
    /// When a failed read was last logged.
    logged_ms: Option<u64>,
}

impl LimitsSlot {
    /// Due while a card is on the board, when nudged or `gap_ms` after the
    /// last run; marks it started.
    fn take_due(&mut self, on_board: bool, gap_ms: u64, now_ms: u64) -> bool {
        let due = on_board && (self.nudged || self.ran_ms.is_none_or(|t| now_ms.saturating_sub(t) >= gap_ms));
        self.nudged = false;
        if due {
            self.ran_ms = Some(now_ms);
        }
        due
    }
}

/// One Grok session's usage as `grok usage` last told it.
#[derive(Default)]
struct GrokSlot {
    usage: Option<SessionUsage>,
    /// When the run was last handed out or last answered.
    ran_ms: Option<u64>,
    /// The events file's mtime when the last run was handed out.
    seen: Option<std::time::SystemTime>,
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

pub const DEFAULT_COMPACT_THRESHOLD_BYTES: u64 = 5 * 1024 * 1024;

/// OpenCode's server state file: under the XDG state dir (`~/.local/state`
/// by default); on Windows `%LOCALAPPDATA%\opencode\state\service.json`
/// when that file exists, else the same `~/.local/state` layout (unverified
/// on Windows; see the README).
pub(crate) fn opencode_state_file(home: &Path) -> PathBuf {
    #[cfg(windows)]
    if let Some(local) = dirs::data_local_dir() {
        let candidate = local.join("opencode").join("state").join("service.json");
        if candidate.exists() {
            return candidate;
        }
    }
    #[cfg(target_os = "linux")]
    if let Some(state) = dirs::state_dir() {
        return crate::opencode::service_file(&state);
    }
    crate::opencode::service_file(&home.join(".local/state"))
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
            asks: AskCache::load(claude_dir.join("maya/asks.json")),
            claude_dir,
            codex_dir: crate::codex_hooks::dir(),
            agy_dir: home.join(".gemini/antigravity-cli"),
            grok_dir: home.join(".grok"),
            kiro_dir: home.join(".kiro/sessions/cli"),
            // `~/Library/Application Support` on macOS, `~/.local/share` on Linux, `%LOCALAPPDATA%` on Windows.
            kiro_run_dir: dirs::data_local_dir().unwrap_or_else(|| home.join(".local/share")).join("kiro-cli/run"),
            opencode_state_file: opencode_state_file(&home),
            opencode_windows: Box::new(opencode_windows),
            native_roots: Box::new(Vec::new),
            native_seen: Box::new(|_| ()),
            opencode: Vec::new(),
            foreign: std::collections::HashMap::new(),
            processes: Box::new(foreign::list_tui_processes),
            process_tree: Box::new(foreign::list_process_tree),
            discover: Box::new(discover_sessions),
            config,
            alive: Box::new(registry::pid_alive),
            tails: TailCache::default(),
            totals: Default::default(),
            recorded: Default::default(),
            prs: PrCache::default(),
            agy_limits: LimitsSlot::default(),
            kiro_limits: LimitsSlot::default(),
            grok_usage: std::collections::HashMap::new(),
            compact_threshold_bytes: DEFAULT_COMPACT_THRESHOLD_BYTES,
            names: NameFiles::default(),
            pending: Default::default(),
            due_renames: Default::default(),
            agents: std::sync::Arc::new(crate::agents::current),
            memory: None,
            memory_save_failing: false,
            hidden_session: None,
            judge_asks: false,
            due_asks: Vec::new(),
            queued: HashSet::new(),
            judging: HashSet::new(),
            asks_save_failing: false,
        }
    }

    /// Sets the session id `refresh` leaves out of its cards and out of what
    /// the store remembers; `None` shows every session again.
    pub fn hide_session(&mut self, id: Option<String>) {
        self.hidden_session = id;
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_alive(mut self, alive: impl Fn(i32) -> bool + Send + 'static) -> Self {
        self.alive = Box::new(alive);
        // Tests never see the machine's real codex/agy/grok sessions.
        self.processes = Box::new(Vec::new);
        self.process_tree = Box::new(Vec::new);
        self.grok_dir = PathBuf::from("/nonexistent/grok");
        self.kiro_dir = PathBuf::from("/nonexistent/kiro");
        self.opencode_state_file = PathBuf::from("/nonexistent/state/opencode/service.json");
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
        self.opencode_state_file = crate::opencode::service_file(&state_dir);
        self.opencode_windows = Box::new(move || windows.clone());
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_opencode_file(mut self, state_file: PathBuf) -> Self {
        self.opencode_state_file = state_file;
        self
    }

    /// OpenCode's server when its state file names one that is alive.
    pub fn opencode_service(&self) -> Option<crate::opencode::Service> {
        crate::opencode::read_service(&self.opencode_state_file()).filter(|s| (self.alive)(s.pid))
    }

    pub fn opencode_state_file(&self) -> PathBuf {
        self.opencode_state_file.clone()
    }

    /// The OpenCode card with this id, as of the last refresh.
    pub fn opencode_card(&self, session_id: &str) -> Option<Card> {
        self.opencode.iter().find(|f| f.session.id == session_id).map(|f| f.card.clone())
    }

    /// The OpenCode session and what the server said about it, as of the last refresh.
    pub fn opencode_session(&self, session_id: &str) -> Option<(crate::opencode::SessionInfo, crate::opencode::Live)> {
        self.opencode.iter().find(|f| f.session.id == session_id).map(|f| (f.session.clone(), f.live.clone()))
    }

    /// Every process with its parent and tty, for walking from a session's
    /// tty to its terminal tab's (`tty::outer_tty`).
    pub fn process_tree(&self) -> Vec<foreign::Proc> {
        (self.process_tree)()
    }

    /// Where to ask for the pids of the programs Maya hosts, once per refresh.
    pub fn set_native_roots(&mut self, roots: Box<dyn Fn() -> Vec<i32> + Send>) {
        self.native_roots = roots;
    }

    /// Where to tell, on each refresh, the roots a card was found under.
    pub fn set_native_seen(&mut self, seen: NativeSeen) {
        self.native_seen = seen;
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
    pub fn with_process_tree(mut self, procs: Vec<foreign::Proc>) -> Self {
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
        // An OpenCode session is running only while the server runs a turn in it; a
        // quiet one on the board can be resumed.
        self.registry().into_iter().map(|s| s.session_id).chain(self.foreign.values().map(|s| s.session_id.clone())).chain(self.opencode.iter().filter(|f| f.live.running).map(|f| f.session.id.clone())).collect()
    }

    /// Working directories of every live session, any harness.
    pub fn live_cwds(&self) -> Vec<String> {
        self.registry().into_iter().map(|s| s.cwd).chain(self.foreign.values().map(|s| s.cwd.clone())).chain(self.opencode.iter().map(|f| f.card.cwd.clone())).collect()
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

    /// One Claude Code session's card, without refreshing the other agents
    /// (OpenCode's server among them): enough to know whether it is free.
    pub fn claude_card(&mut self, session_id: &str, now_ms: u64) -> Option<Card> {
        let _ = self.events.read_new();
        let s = self.session(session_id)?;
        let path = self.transcript_path_for(&s);
        let tail = self.tails.get(&path);
        let prose = self.prose_for(&s.session_id, claude_text(&tail)).map_or(Prose::Unjudged, |(_, p)| p);
        Some(derive(&DeriveInput { registry: &s, events: self.events.events_for(&s.session_id), transcript: &tail, prose: &prose, now_ms, completed_timeout_ms: self.config.completed_timeout_ms() }))
    }

    /// Records that Maya pressed Esc in a Claude session: the ask the turn
    /// left open is over, and no hook of Claude's says so.
    pub fn mark_interrupted(&mut self, session_id: &str, now_ms: u64) {
        let payload = serde_json::json!({ "session_id": session_id, "hook_event_name": "MayaInterrupt" }).to_string();
        if let Err(e) = crate::hook_install::append_record_named(&self.claude_dir.join("maya"), "events.jsonl", &payload, now_ms) {
            crate::log::line("stop", format!("could not record the interrupt of {session_id}: {e}"));
        }
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

    /// Forgets every PR lookup: the Git settings changed.
    pub fn clear_prs(&mut self) {
        self.prs.clear();
    }

    /// Records the branch checked out in `dir`, looked up beside its PR.
    pub fn set_branch(&mut self, dir: &str, branch: Option<String>) {
        self.prs.set_branch(dir, branch);
    }

    pub fn set_pr(&mut self, dir: &str, pr: Option<PullRequest>, now_ms: u64) {
        self.prs.set(dir, pr, now_ms);
    }

    pub const GROK_USAGE_MIN_GAP_MS: u64 = 60_000;
    pub const AGY_LIMITS_GAP_MS: u64 = 300_000;
    pub const KIRO_LIMITS_GAP_MS: u64 = 600_000;
    pub const NUDGE_STALE_MS: u64 = 60_000;
    pub const LIMITS_LOG_GAP_MS: u64 = 3_600_000;

    fn limits_slot(&mut self, h: Harness) -> Option<&mut LimitsSlot> {
        match h {
            Harness::Antigravity => Some(&mut self.agy_limits),
            Harness::Kiro => Some(&mut self.kiro_limits),
            _ => None,
        }
    }

    /// Marks the harness's limits due now when older than `NUDGE_STALE_MS` (a window opened).
    pub fn nudge_limits(&mut self, h: Harness, now_ms: u64) {
        if let Some(slot) = self.limits_slot(h) {
            if slot.ran_ms.is_none_or(|t| now_ms.saturating_sub(t) >= Self::NUDGE_STALE_MS) {
                slot.nudged = true;
            }
        }
    }

    /// What the collectors should run now; marks what it hands out as started.
    /// Limits are read while a card of that harness is on the board: every
    /// gap, or at once when nudged. A Grok session's usage is read when its
    /// events file changed since the last run, at most once a minute.
    pub fn usage_due(&mut self, now_ms: u64) -> collect::Due {
        let on_board = |h: Harness| self.foreign.values().any(|s| s.harness == h);
        let (agy, kiro) = (on_board(Harness::Antigravity), on_board(Harness::Kiro));
        let mut due = collect::Due { antigravity: self.agy_limits.take_due(agy, Self::AGY_LIMITS_GAP_MS, now_ms), kiro: self.kiro_limits.take_due(kiro, Self::KIRO_LIMITS_GAP_MS, now_ms), grok: vec![] };
        let grok: Vec<(String, PathBuf)> = self.foreign.values().filter(|s| s.harness == Harness::Grok).map(|s| (s.session_id.clone(), s.transcript_path.clone())).collect();
        self.grok_usage.retain(|id, _| grok.iter().any(|(g, _)| g == id));
        for (id, events) in grok {
            let mtime = std::fs::metadata(&events).and_then(|m| m.modified()).ok();
            let slot = self.grok_usage.entry(id.clone()).or_default();
            let rested = slot.ran_ms.is_none_or(|t| now_ms.saturating_sub(t) >= Self::GROK_USAGE_MIN_GAP_MS);
            if mtime.is_some() && mtime != slot.seen && rested {
                slot.seen = mtime;
                slot.ran_ms = Some(now_ms);
                due.grok.push(id);
            }
        }
        due.grok.sort();
        due
    }

    /// The harness's limits as its collector read them; `None` clears them.
    pub fn set_limits(&mut self, h: Harness, limits: Option<AccountLimits>, now_ms: u64) {
        if let Some(slot) = self.limits_slot(h) {
            slot.limits = limits;
            slot.ran_ms = Some(now_ms);
        }
    }

    /// The harness's limits could not be read. The last good reading stays,
    /// so one timed-out read does not blank the card; with none ever read
    /// the limits stay `None`. True when the failure is to be logged: at most
    /// once an hour per harness.
    pub fn limits_failed(&mut self, h: Harness, now_ms: u64) -> bool {
        let Some(slot) = self.limits_slot(h) else { return false };
        slot.ran_ms = Some(now_ms);
        let log = slot.logged_ms.is_none_or(|t| now_ms.saturating_sub(t) >= Self::LIMITS_LOG_GAP_MS);
        if log {
            slot.logged_ms = Some(now_ms);
        }
        log
    }

    /// A Grok session's usage from `grok usage`; `None` (the run failed)
    /// keeps the last reading, since a session's usage only grows.
    pub fn set_grok_usage(&mut self, session_id: &str, usage: Option<SessionUsage>, now_ms: u64) {
        let slot = self.grok_usage.entry(session_id.to_string()).or_default();
        slot.ran_ms = Some(now_ms);
        match usage {
            Some(u) => slot.usage = Some(u),
            // A run that brought nothing (the binary missing, a timeout, an
            // unparseable answer) keeps the last reading and forgets the mtime
            // it was handed out for, so the usual one-minute retry happens.
            None => slot.seen = None,
        }
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
        self.judging.clear();
        let mut used = HashSet::new();
        self.recorded.scan(&self.claude_dir.join("maya"), now_ms);
        let mut budget = crate::usage::TOTALS_BUDGET_BYTES;
        for s in &sessions {
            let path = self.transcript_path_for(s);
            let tail = self.tails.get(&path);
            let totals = self.totals.get(&path, &mut budget);
            paths.push(path);
            let text = claude_text(&tail);
            let looked = self.prose_for(&s.session_id, text);
            let mut card = derive(&DeriveInput {
                registry: s,
                events: self.events.events_for(&s.session_id),
                transcript: &tail,
                prose: looked.as_ref().map_or(&Prose::Unjudged, |(_, p)| p),
                now_ms,
                completed_timeout_ms: self.config.completed_timeout_ms(),
            });
            if let Some((key, prose)) = looked {
                self.track_prose(&mut used, &s.session_id, text, key, &prose, &card);
            }
            card.pr = self.prs.get(&s.cwd);
            card.branch = self.prs.branch(&s.cwd);
            // Claude Code names its model in the transcript; the viewer's status line shows it.
            card.model = tail.model.clone();
            let rec = self.recorded.get(&s.session_id);
            // The recorded context is what Claude Code's own status line shows; Maya's guess fills in without one.
            card.context = match rec.as_ref().and_then(|r| r.context_used_percent.zip(r.context_window_size)) {
                Some((percent, window)) => Some(crate::context::ContextUsage { used: window.saturating_mul(percent.into()) / 100, window, percent }),
                None => tail.context_tokens.map(|used| crate::context::usage(tail.model.as_deref().unwrap_or(""), used, default_1m)),
            };
            let mut usage = crate::usage::session_usage_of(&totals);
            let counted = !totals.priced.is_empty() || !totals.unpriced.is_empty();
            let recorded_cost = rec.as_ref().and_then(|r| r.total_cost_usd);
            if let Some(usd) = recorded_cost {
                usage.cost = Some(crate::usage::Cost { micros: (usd * 1e6).round() as u64, unit: crate::usage::CostUnit::Usd, estimated: false, models: vec![] });
            }
            // Nothing counted yet is no usage, not zeros.
            card.usage = (counted || recorded_cost.is_some()).then_some(usage);
            card.status_line = rec.map(|r| r.line).filter(|l| !l.is_empty());
            cards.push(card);
        }
        self.tails.retain(&paths);
        self.totals.retain(&paths);
        // One machine is one account: the newest reading on disk, a session that
        // ended included, is every Claude Code card's limits.
        let limits = self.recorded.limits(crate::statusline::account_email(&self.claude_dir));
        for card in &mut cards {
            card.limits = limits.clone();
        }
        let timeout = self.config.completed_timeout_ms();
        let foreign: Vec<ForeignSession> = self.foreign.values().cloned().collect();
        for s in &foreign {
            let mut tail = foreign::tail_for(s);
            if s.harness == Harness::Kiro {
                crate::kiro::apply_live(&mut tail, &self.kiro_run_dir, s.pid, &s.transcript_path, now_ms);
                let meta = self.names.read(&s.transcript_path.with_extension("json"));
                crate::kiro::apply_credits(&mut tail, &meta);
            }
            if s.harness == Harness::Codex {
                crate::codex_hooks::apply(&mut tail, self.codex_events.events_for(&s.session_id));
            }
            let text = tail.last_agent_text.as_deref().filter(|_| !tail.working);
            let looked = self.prose_for(&s.session_id, text);
            let mut card = foreign::derive(s, &tail, looked.as_ref().map_or(&Prose::Unjudged, |(_, p)| p), now_ms, timeout);
            if let Some((key, prose)) = looked {
                self.track_prose(&mut used, &s.session_id, text, key, &prose, &card);
            }
            card.pr = self.prs.get(&s.cwd);
            card.branch = self.prs.branch(&s.cwd);
            cards.push(card);
        }
        // One machine is one Codex account: the newest reading is every Codex card's limits.
        let codex_limits = cards.iter().filter(|c| c.harness == Harness::Codex).filter_map(|c| c.limits.clone()).max_by_key(|l| l.read_at);
        for card in cards.iter_mut().filter(|c| c.harness == Harness::Codex) {
            card.limits = codex_limits.clone();
        }
        // What the collectors read off the lock: account limits per harness, Grok's usage per session.
        for card in &mut cards {
            match card.harness {
                Harness::Antigravity => card.limits = self.agy_limits.limits.clone(),
                Harness::Kiro => card.limits = self.kiro_limits.limits.clone(),
                Harness::Grok => card.usage = self.grok_usage.get(&card.session_id).and_then(|g| g.usage.clone()),
                _ => {}
            }
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
        cards.extend(self.opencode.iter().map(|f| {
            let mut card = f.card.clone();
            card.pr = self.prs.get(&card.cwd);
            card.branch = self.prs.branch(&card.cwd);
            card
        }));
        if let Some(hidden) = &self.hidden_session {
            cards.retain(|c| &c.session_id != hidden);
        }
        let roots = (self.native_roots)();
        if !roots.is_empty() {
            let parents = crate::native::parents(&self.process_tree());
            let mut seen = Vec::new();
            for card in &mut cards {
                let root = crate::native::root_of(card.pid, &roots, &parents);
                card.native = root.is_some();
                seen.extend(root);
            }
            (self.native_seen)(&seen);
        }
        let due = self.pending.apply(&mut cards, now_ms);
        self.due_renames.extend(due);
        self.log_pending_events();
        if let Some(m) = self.memory.as_mut() {
            let live: Vec<LiveSession> = cards.iter().map(LiveSession::of).collect();
            m.observe(&live, now_ms);
        }
        self.save_memory();
        self.asks.retain(&used);
        self.save_asks();
        cards
    }

    /// What the cache and the judge settings say about `text`, the finished
    /// message of session `id`: its key and `Asks`/`Done` from a cached
    /// verdict, or `Judging` when it should be judged. `None` leaves the
    /// wording check to decide.
    fn prose_for(&self, id: &str, text: Option<&str>) -> Option<(String, Prose)> {
        let text = text.filter(|_| self.hidden_session.as_deref() != Some(id))?;
        let key = asks::key(id, text);
        if let Some(v) = self.asks.get(&key) {
            let line = || v.line.clone().or_else(|| crate::state::asking_line(text)).or_else(|| text.lines().map(str::trim).rfind(|l| !l.is_empty()).map(str::to_string));
            let prose = if v.asks { Prose::Asks(line().unwrap_or_default()) } else { Prose::Done };
            return Some((key, prose));
        }
        self.judge_asks.then_some((key, Prose::Judging))
    }

    /// Books what `prose_for` found once the card is derived: a verdict in use
    /// is kept; a finished card held on `Judging` (or working only on a shell
    /// its turn left) is queued and counted as judging. A card that works or
    /// waits on a prompt needs no verdict.
    fn track_prose(&mut self, used: &mut HashSet<String>, id: &str, text: Option<&str>, key: String, prose: &Prose, card: &Card) {
        if *prose != Prose::Judging {
            used.insert(key);
        } else if matches!(card.state, State::Completed | State::Idle) || (card.state == State::Working && card.shell) {
            if let Some(text) = text.filter(|_| self.queued.insert(key.clone())) {
                self.due_asks.push((key, text.to_string()));
            }
            self.judging.insert(id.to_string());
        }
    }

    /// The messages to judge now as `(key, text)`; each key is handed out once.
    pub fn take_due_asks(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.due_asks)
    }

    pub fn set_ask_verdict(&mut self, key: String, v: Verdict) {
        self.queued.remove(&key);
        self.asks.insert(key, v);
        self.save_asks();
    }

    /// Writes the verdicts when they changed; a streak of failed saves is
    /// logged once, like the memory's.
    fn save_asks(&mut self) {
        match self.asks.save() {
            Ok(()) => self.asks_save_failing = false,
            Err(e) => {
                if !self.asks_save_failing {
                    crate::log::line("asks", format!("could not save the verdicts to {}: {e}", self.asks.path().display()));
                }
                self.asks_save_failing = true;
            }
        }
    }

    /// The judge gave no usable verdict on `text`: the wording check's is
    /// kept as its verdict, so it is not judged again after a restart.
    pub fn set_ask_failed(&mut self, key: String, text: &str) {
        let line = crate::state::asking_line(text);
        self.set_ask_verdict(key, Verdict { asks: line.is_some(), line });
    }

    /// The sessions the last refresh held on a verdict still to come.
    pub fn judging_sessions(&self) -> HashSet<String> {
        self.judging.clone()
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

    /// Starts remembering this machine's sessions: reads what the last run
    /// left, keeps what was running when it stopped, and saves. Called
    /// once, before the first refresh. Returns what is offered.
    pub fn start_remembering(&mut self, now_ms: u64) -> Vec<Remembered> {
        let mut m = Memory::load(crate::remembered::path_in(&self.claude_dir));
        let offered = m.classify_at_start(now_ms);
        self.memory = Some(m);
        self.save_memory();
        offered
    }

    /// Writes the memory when it changed; a failure is logged, never fatal.
    /// A failed save is retried on every refresh, so a streak of failures
    /// is logged once, and again only after a save succeeded.
    fn save_memory(&mut self) {
        if let Some(m) = self.memory.as_mut() {
            match m.save() {
                Ok(()) => self.memory_save_failing = false,
                Err(e) => {
                    if !self.memory_save_failing {
                        crate::log::line("remembered", format!("could not save {}: {e}", m.path().display()));
                    }
                    self.memory_save_failing = true;
                }
            }
        }
    }

    /// What was running when Maya last stopped and is not running now.
    pub fn was_running(&self) -> Vec<Remembered> {
        self.memory.as_ref().map(Memory::was_running).unwrap_or_default()
    }

    pub fn remembered_entry(&self, agent: Harness, session_id: &str) -> Option<Remembered> {
        self.memory.as_ref()?.entry(agent, session_id)
    }

    /// Closed from its card (here or relayed from the main): forgotten at once.
    pub fn session_closed(&mut self, agent: Harness, session_id: &str) {
        if let Some(m) = self.memory.as_mut() {
            m.closed(agent, session_id);
        }
        self.save_memory();
    }

    pub fn forget_remembered(&mut self, keys: &[Key]) {
        if let Some(m) = self.memory.as_mut() {
            m.forget(keys);
        }
        self.save_memory();
    }

    pub fn remembered_failed(&mut self, key: &Key, reason: &str) {
        if let Some(m) = self.memory.as_mut() {
            m.failed(key, reason);
        }
        self.save_memory();
    }

    /// This run's last sign of life, every `remembered::HEARTBEAT_MS`.
    pub fn heartbeat(&mut self, now_ms: u64) {
        if let Some(m) = self.memory.as_mut() {
            m.heartbeat(now_ms);
        }
        self.save_memory();
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
        let pr = PullRequest { number: 5, url: "https://github.com/o/r/pull/5".into(), state: "open".into(), forge: Default::default() };
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
    fn codex_limits_are_shared_by_every_codex_card() {
        use crate::foreign::ForeignSession;
        use crate::model::Harness;
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join("claude");
        std::fs::create_dir_all(claude.join("sessions")).unwrap();
        let fixture = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/codex/token_count.jsonl")).unwrap();
        let with = dir.path().join("with.jsonl");
        std::fs::write(&with, &fixture).unwrap();
        let without = dir.path().join("without.jsonl");
        std::fs::write(&without, "{\"timestamp\":\"2026-10-29T09:00:00.000Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\"}}\n").unwrap();
        let mk = |pid, id: &str, path| ForeignSession { harness: Harness::Codex, pid, tty: Some(format!("/dev/ttys0{pid}")), session_id: id.into(), cwd: "/Users/x/dev/one".into(), name: id.into(), transcript_path: path };
        let mut store = Store::new(claude).with_alive(|_| true).with_foreign(dir.path().join("codex"), dir.path().join("agy"), vec![mk(77, "aaa", with), mk(78, "bbb", without)]);
        let cards = store.refresh(1_790_671_680_000);
        assert_eq!(cards.len(), 2);
        let a = cards.iter().find(|c| c.session_id == "aaa").unwrap();
        let b = cards.iter().find(|c| c.session_id == "bbb").unwrap();
        assert!(a.limits.is_some());
        assert_eq!(a.limits, b.limits);
        assert!(b.usage.is_none());
        assert_eq!(a.model.as_deref(), Some("gpt-6.1-sol"));
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
    fn the_opencode_state_file_is_a_whole_path_wherever_the_platform_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("claude")).with_alive(|_| false).with_opencode(dir.path().join("state"), vec![]);
        assert_eq!(store.opencode_state_file(), dir.path().join("state/opencode/service.json"));
        let store = store.with_opencode_file(dir.path().join("elsewhere/service.json"));
        assert_eq!(store.opencode_state_file(), dir.path().join("elsewhere/service.json"));
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
        let live = store.live_session_ids();
        assert!(!live.iter().any(|id| id == "ses_efe568c4dffeefYKhkbkEX18Qm"), "a completed session is not running: Resume may offer it");
        assert!(store.live_cwds().iter().any(|d| d == "/Users/tiagocorreia"), "its folder counts as occupied");
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

#[test]
    fn a_session_under_a_native_root_is_marked_native() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().to_path_buf();
        std::fs::create_dir_all(claude.join("sessions")).unwrap();
        std::fs::write(
            claude.join("sessions/30.json"),
            r#"{"pid":30,"sessionId":"a","cwd":"/Users/x/dev/eye","name":"n","status":"idle","startedAt":1,"statusUpdatedAt":100}"#,
        ).unwrap();
        let tree = vec![foreign::Proc { pid: 10, ppid: 1, tty: None, command: String::new() }, foreign::Proc { pid: 20, ppid: 10, tty: None, command: String::new() }, foreign::Proc { pid: 30, ppid: 20, tty: None, command: String::new() }];
        let mut store = Store::new(claude).with_alive(|_| true).with_process_tree(tree);
        store.set_native_roots(Box::new(|| vec![10, 99]));
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let s = seen.clone();
        store.set_native_seen(Box::new(move |roots| s.lock().unwrap().push(roots.to_vec())));
        assert!(store.refresh(1_000).iter().find(|c| c.session_id == "a").unwrap().native);
        assert_eq!(*seen.lock().unwrap(), [vec![10]], "the roots a card was seen under, once per refresh");
        store.set_native_roots(Box::new(Vec::new));
        assert!(!store.refresh(2_000).iter().find(|c| c.session_id == "a").unwrap().native);
        assert_eq!(seen.lock().unwrap().len(), 1, "no roots, nothing to tell");
    }

    #[test]
    fn a_hidden_session_is_neither_shown_nor_remembered() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().to_path_buf();
        std::fs::create_dir_all(claude.join("sessions")).unwrap();
        std::fs::create_dir_all(claude.join("maya")).unwrap();
        for (pid, id) in [(7, "a"), (8, "brain-id")] {
            std::fs::write(
                claude.join(format!("sessions/{pid}.json")),
                format!(r#"{{"pid":{pid},"sessionId":"{id}","cwd":"/Users/x/dev/eye","name":"n-{pid}","status":"idle","startedAt":1,"statusUpdatedAt":100}}"#),
            ).unwrap();
        }
        let mut first = Store::new(claude.clone()).with_alive(|_| true);
        first.start_remembering(1_000);
        first.hide_session(Some("brain-id".into()));
        let cards = first.refresh(2_000);
        assert!(!cards.is_empty());
        assert!(cards.iter().all(|c| c.session_id != "brain-id"));
        first.heartbeat(30_000);
        drop(first);
        // The machine restarted: only the shown session is offered back.
        let mut second = Store::new(claude).with_alive(|_| false);
        let offered = second.start_remembering(400_000);
        assert!(offered.iter().any(|e| e.session_id == "a"));
        assert!(offered.iter().all(|e| e.session_id != "brain-id"));
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

    fn claude_with_session(root: &Path) -> PathBuf {
        let claude = root.join("claude");
        std::fs::create_dir_all(claude.join("sessions")).unwrap();
        std::fs::create_dir_all(claude.join("maya")).unwrap();
        std::fs::write(claude.join("sessions/7.json"), r#"{"pid":7,"sessionId":"s7","cwd":"/Users/x/dev/eye","name":"eye-7","status":"idle"}"#).unwrap();
        claude
    }

    #[test]
    fn a_session_running_when_maya_stopped_is_offered_at_the_next_start() {
        let t = tempfile::tempdir().unwrap();
        let claude = claude_with_session(t.path());
        let mut first = Store::new(claude.clone()).with_alive(|_| true);
        assert!(first.start_remembering(1_000).is_empty());
        first.refresh(2_000);
        first.heartbeat(30_000);
        drop(first);
        // The machine restarted: the session's process is gone.
        let mut second = Store::new(claude).with_alive(|_| false);
        let offered = second.start_remembering(400_000);
        assert_eq!(offered.iter().map(|e| (e.session_id.as_str(), e.dir.as_str(), e.name.as_str())).collect::<Vec<_>>(), [("s7", "/Users/x/dev/eye", "eye-7")]);
        second.refresh(401_000);
        assert_eq!(second.was_running().len(), 1, "still offered while it is not running");
        second.forget_remembered(&[(Harness::ClaudeCode, "s7".into())]);
        assert!(second.was_running().is_empty());
    }

    #[test]
    fn sessions_that_outlived_a_restart_of_maya_alone_are_not_offered() {
        let t = tempfile::tempdir().unwrap();
        let claude = claude_with_session(t.path());
        let mut first = Store::new(claude.clone()).with_alive(|_| true);
        first.start_remembering(1_000);
        first.refresh(2_000);
        drop(first);
        // Only Maya was quit and reopened: the session still runs.
        let mut second = Store::new(claude).with_alive(|_| true);
        assert_eq!(second.start_remembering(60_000).len(), 1, "judged before the first refresh");
        second.refresh(61_000);
        assert!(second.was_running().is_empty(), "it is live: nothing to resume");
    }

    #[test]
    fn a_save_that_keeps_failing_is_logged_once_until_one_succeeds() {
        let t = tempfile::tempdir().unwrap();
        let claude = claude_with_session(t.path());
        let maya = claude.join("maya");
        let path = crate::remembered::path_in(&claude);
        let logged = || crate::log::lines().iter().filter(|l| l.source == "remembered" && l.text.starts_with(&format!("could not save {}", path.display()))).count();
        // A file where the folder should be: every save fails.
        std::fs::remove_dir_all(&maya).unwrap();
        std::fs::write(&maya, "").unwrap();
        let mut store = Store::new(claude.clone()).with_alive(|_| true);
        store.start_remembering(1_000);
        store.refresh(2_000);
        store.heartbeat(30_000);
        assert_eq!(logged(), 1, "one line for the whole streak, not one per refresh");
        std::fs::remove_file(&maya).unwrap();
        store.heartbeat(60_000);
        assert!(path.exists(), "saved again once it can be");
        std::fs::remove_dir_all(&maya).unwrap();
        std::fs::write(&maya, "").unwrap();
        store.heartbeat(90_000);
        store.heartbeat(120_000);
        assert_eq!(logged(), 2, "a new streak after a success is logged again, once");
    }

    #[test]
    fn a_store_that_does_not_remember_writes_no_file() {
        let t = tempfile::tempdir().unwrap();
        let claude = claude_with_session(t.path());
        let mut store = Store::new(claude.clone()).with_alive(|_| true);
        store.refresh(1);
        store.heartbeat(2);
        store.session_closed(Harness::ClaudeCode, "s7");
        assert!(!crate::remembered::path_in(&claude).exists());
        assert!(store.was_running().is_empty());
    }

    #[test]
    fn refresh_attaches_the_branch_and_a_claude_sessions_model() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().to_path_buf();
        std::fs::create_dir_all(claude.join("sessions")).unwrap();
        std::fs::create_dir_all(claude.join("projects/-Users-x-dev-eye")).unwrap();
        std::fs::write(claude.join("sessions/7.json"), r#"{"pid":7,"sessionId":"s7","cwd":"/Users/x/dev/eye","name":"eye-7","status":"idle"}"#).unwrap();
        std::fs::write(
            claude.join("projects/-Users-x-dev-eye/s7.jsonl"),
            r#"{"type":"assistant","message":{"model":"claude-opus-5[1m]","usage":{"input_tokens":10,"cache_read_input_tokens":0,"cache_creation_input_tokens":0},"content":[{"type":"text","text":"hi"}]}}"#,
        ).unwrap();
        let mut store = Store::new(claude).with_alive(|_| true);
        let card = store.refresh(1).remove(0);
        assert_eq!(card.model.as_deref(), Some("claude-opus-5[1m]"));
        assert_eq!(card.branch, None, "not looked up yet");
        store.set_branch("/Users/x/dev/eye", Some("feat/x".into()));
        assert_eq!(store.refresh(2)[0].branch.as_deref(), Some("feat/x"));
    }

    const SAY_GO: &str = "I can make it now if you say go.";

    /// A Claude dir with session `s7`, idle after a Stop, whose turn ended on `SAY_GO`.
    fn finished_turn(claude: &Path) -> u64 {
        std::fs::create_dir_all(claude.join("sessions")).unwrap();
        std::fs::create_dir_all(claude.join("maya")).unwrap();
        std::fs::create_dir_all(claude.join("projects/-Users-x-dev-eye")).unwrap();
        std::fs::write(claude.join("sessions/7.json"), r#"{"pid":7,"sessionId":"s7","cwd":"/Users/x/dev/eye","name":"eye-7","status":"idle","startedAt":1,"statusUpdatedAt":100}"#).unwrap();
        let ts = "2026-10-28T13:01:14.734Z";
        std::fs::write(
            claude.join("projects/-Users-x-dev-eye/s7.jsonl"),
            format!(r#"{{"type":"assistant","message":{{"stop_reason":"end_turn","content":[{{"type":"text","text":"{SAY_GO}"}}]}},"timestamp":"{ts}"}}"#),
        ).unwrap();
        let at = crate::codex::ms_of(ts).unwrap();
        std::fs::write(claude.join("maya/events.jsonl"), format!("{{\"session_id\":\"s7\",\"hook_event_name\":\"Stop\",\"received_at\":{at}}}\n")).unwrap();
        at + 1_000
    }

    #[test]
    fn a_finished_turn_is_judged_once_and_its_verdict_is_kept_across_stores() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().to_path_buf();
        let now = finished_turn(&claude);
        let mut store = Store::new(claude.clone()).with_alive(|_| true);
        store.judge_asks = true;
        assert_eq!(store.refresh(now)[0].state, State::Completed);
        assert_eq!(store.judging_sessions(), ["s7".to_string()].into_iter().collect());
        let due = store.take_due_asks();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].1, SAY_GO);
        assert!(store.take_due_asks().is_empty());
        assert_eq!(store.refresh(now)[0].state, State::Completed, "still judging");
        assert!(store.take_due_asks().is_empty(), "a queued key is not handed out again");

        store.set_ask_verdict(due[0].0.clone(), Verdict { asks: true, line: Some(SAY_GO.into()) });
        let card = store.refresh(now).remove(0);
        assert_eq!((card.state, card.awaiting.unwrap().detail.as_str()), (State::Awaiting, SAY_GO));
        assert!(store.judging_sessions().is_empty());
        assert!(store.take_due_asks().is_empty());

        let mut next = Store::new(claude).with_alive(|_| true);
        next.judge_asks = true;
        assert_eq!(next.refresh(now)[0].state, State::Awaiting);
        assert!(next.take_due_asks().is_empty());
    }

    #[test]
    fn a_verdict_that_does_not_ask_lets_the_card_complete_and_a_changed_message_is_judged_again() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().to_path_buf();
        let now = finished_turn(&claude);
        let mut store = Store::new(claude.clone()).with_alive(|_| true);
        store.judge_asks = true;
        store.refresh(now);
        let key = store.take_due_asks().remove(0).0;
        store.set_ask_verdict(key, Verdict { asks: false, line: None });
        assert_eq!(store.refresh(now)[0].state, State::Completed);
        assert!(store.judging_sessions().is_empty());
        let t = claude.join("projects/-Users-x-dev-eye/s7.jsonl");
        std::fs::write(&t, std::fs::read_to_string(&t).unwrap().replace("say go", "say stop")).unwrap();
        store.refresh(now);
        assert_eq!(store.take_due_asks().len(), 1);
    }

    #[test]
    fn a_failed_judgement_falls_back_to_the_wording_check() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().to_path_buf();
        let now = finished_turn(&claude);
        let mut store = Store::new(claude.clone()).with_alive(|_| true);
        store.judge_asks = true;
        store.refresh(now);
        let (key, text) = store.take_due_asks().remove(0);
        store.set_ask_failed(key, &text);
        let card = store.refresh(now).remove(0);
        assert_eq!((card.state, card.awaiting.unwrap().detail.as_str()), (State::Awaiting, SAY_GO));
        assert!(store.judging_sessions().is_empty());
        assert!(store.take_due_asks().is_empty());

        let mut next = Store::new(claude).with_alive(|_| true);
        next.judge_asks = true;
        assert_eq!(next.refresh(now)[0].state, State::Awaiting);
        assert!(next.take_due_asks().is_empty(), "a failed message is not judged again after a restart");
    }

    #[test]
    fn a_verdict_that_cannot_be_saved_is_logged_once() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().to_path_buf();
        let now = finished_turn(&claude);
        // A folder where the verdicts file should be: every save fails.
        std::fs::create_dir_all(claude.join("maya/asks.json")).unwrap();
        let logged = || crate::log::lines().iter().filter(|l| l.source == "asks" && l.text.contains("could not save the verdicts") && l.text.contains(&*claude.to_string_lossy())).count();
        let mut store = Store::new(claude.clone()).with_alive(|_| true);
        store.judge_asks = true;
        store.refresh(now);
        let (key, text) = store.take_due_asks().remove(0);
        store.set_ask_verdict(key.clone(), Verdict { asks: false, line: None });
        store.set_ask_verdict(key.clone(), Verdict { asks: true, line: None });
        store.set_ask_failed(key, &text);
        store.refresh(now);
        assert_eq!(logged(), 1);
    }

    #[test]
    fn a_finished_turn_is_judged_while_its_background_shell_runs() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().to_path_buf();
        let now = finished_turn(&claude);
        std::fs::write(claude.join("sessions/7.json"), r#"{"pid":7,"sessionId":"s7","cwd":"/Users/x/dev/eye","name":"eye-7","status":"shell","startedAt":1,"statusUpdatedAt":100}"#).unwrap();
        let mut store = Store::new(claude).with_alive(|_| true);
        store.judge_asks = true;
        let card = store.refresh(now).remove(0);
        assert!(card.state == State::Working && card.shell);
        assert_eq!(store.judging_sessions(), ["s7".to_string()].into_iter().collect());
        let due = store.take_due_asks();
        assert_eq!(due.len(), 1);
        store.set_ask_verdict(due[0].0.clone(), Verdict { asks: true, line: Some(SAY_GO.into()) });
        assert_eq!(store.refresh(now)[0].state, State::Awaiting);
        assert!(store.judging_sessions().is_empty());
    }

    #[test]
    fn without_a_judge_the_wording_check_decides() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().to_path_buf();
        let now = finished_turn(&claude);
        let mut store = Store::new(claude).with_alive(|_| true);
        assert_eq!(store.refresh(now)[0].state, State::Awaiting);
        assert!(store.take_due_asks().is_empty());
        assert!(store.judging_sessions().is_empty());
    }

    #[test]
    fn the_hidden_session_is_never_judged() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().to_path_buf();
        let now = finished_turn(&claude);
        let mut store = Store::new(claude).with_alive(|_| true);
        store.judge_asks = true;
        store.hide_session(Some("s7".into()));
        store.refresh(now);
        assert!(store.take_due_asks().is_empty());
    }

    #[test]
    fn a_foreign_session_that_stopped_is_judged() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join("claude");
        std::fs::create_dir_all(claude.join("sessions")).unwrap();
        let rollout = dir.path().join("rollout.jsonl");
        let line = |ts: &str, payload: &str| format!("{{\"timestamp\":\"{ts}\",\"type\":\"event_msg\",\"payload\":{payload}}}\n");
        let said = line("2026-09-29T08:47:58.953Z", r#"{"type":"task_started","turn_id":"t1"}"#)
            + &line("2026-09-29T08:48:00.953Z", &format!(r#"{{"type":"task_complete","turn_id":"t1","last_agent_message":"{SAY_GO}"}}"#));
        std::fs::write(&rollout, said).unwrap();
        let s = ForeignSession { harness: Harness::Codex, pid: 77, tty: None, session_id: "aaa".into(), cwd: "/Users/x/dev/one".into(), name: "Fix CI".into(), transcript_path: rollout.clone() };
        let mut store = Store::new(claude).with_alive(|_| true).with_foreign(dir.path().join("codex"), dir.path().join("agy"), vec![s]);
        store.judge_asks = true;
        let now = 1_790_671_681_000;
        let cards = store.refresh(now);
        assert_eq!(cards[0].state, State::Completed);
        assert_eq!(store.judging_sessions(), ["aaa".to_string()].into_iter().collect());
        let due = store.take_due_asks();
        assert_eq!((due.len(), due[0].1.as_str()), (1, SAY_GO));
        store.set_ask_verdict(due[0].0.clone(), Verdict { asks: true, line: None });
        let card = store.refresh(now).remove(0);
        assert_eq!((card.state, card.awaiting.unwrap().detail.as_str()), (State::Awaiting, SAY_GO));
    }

    const SID_A: &str = "4f1c2a7e-0b1d-4c3e-9a8f-1234567890ab";
    const SID_B: &str = "00000000-0000-0000-0000-000000000002";

    fn assistant_line(model: &str, input: u64, output: u64) -> String {
        format!(
            r#"{{"type":"assistant","message":{{"model":"{model}","usage":{{"input_tokens":{input},"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"output_tokens":{output}}},"content":[{{"type":"text","text":"hi"}}]}}}}"#
        )
    }

    /// Live session `sid` (pid `pid`, in /Users/x/dev/eye) in the Claude dir `claude`, its transcript holding `lines`.
    fn claude_session(claude: &Path, pid: i32, sid: &str, lines: &[String]) {
        std::fs::create_dir_all(claude.join("sessions")).unwrap();
        std::fs::create_dir_all(claude.join("projects/-Users-x-dev-eye")).unwrap();
        std::fs::write(
            claude.join(format!("sessions/{pid}.json")),
            format!(r#"{{"pid":{pid},"sessionId":"{sid}","cwd":"/Users/x/dev/eye","name":"eye-{pid}","status":"idle"}}"#),
        ).unwrap();
        let body: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(claude.join(format!("projects/-Users-x-dev-eye/{sid}.jsonl")), body).unwrap();
    }

    /// The status line fixture's payload recorded for `sid` at `at`, with `line` as the printed line.
    fn recording(sid: &str, line: &str, at: u64) -> crate::statusline::Recorded {
        let payload: serde_json::Value = serde_json::from_str(include_str!("../fixtures/statusline/payload.json")).unwrap();
        let mut r = crate::statusline::recorded_from(&payload, line, at).unwrap();
        r.session_id = sid.into();
        r
    }

    #[test]
    fn a_recorded_status_line_fills_limits_cost_context_and_the_line() {
        use crate::context::ContextUsage;
        use crate::usage::{Cost, CostUnit, TokenCounts};
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join(".claude");
        claude_session(&claude, 7, SID_A, &[assistant_line("claude-opus-5-5", 100, 20), assistant_line("claude-opus-5-5", 5, 1)]);
        std::fs::write(dir.path().join(".claude.json"), r#"{"oauthAccount":{"emailAddress":"me@example.com"},"numStartups":3}"#).unwrap();
        let line = "\u{1b}[1mOpus\u{1b}[0m 12%";
        crate::statusline::write_recorded(&claude.join("maya"), &recording(SID_A, line, 1_000)).unwrap();
        let mut store = Store::new(claude).with_alive(|_| true);
        let card = store.refresh(2_000).remove(0);
        assert_eq!(store.recorded.reads, 1, "one file, read once for the card and the limits");
        assert_eq!(store.refresh(2_500).remove(0), card);
        assert_eq!(store.recorded.reads, 1, "a second refresh with unchanged files reads none of them");
        assert_eq!(card.status_line.as_deref(), Some(line));
        let limits = card.limits.unwrap();
        assert_eq!(limits.windows[0].label, "5h");
        assert_eq!(limits.account.as_deref(), Some("me@example.com"));
        assert_eq!(limits.read_at, 1_000);
        assert_eq!(card.context, Some(ContextUsage { used: 120_000, window: 1_000_000, percent: 12 }));
        let usage = card.usage.unwrap();
        assert_eq!(usage.cost, Some(Cost { micros: 4_123_400, unit: CostUnit::Usd, estimated: false, models: vec![] }));
        assert_eq!(usage.tokens, TokenCounts { input: 105, output: 21, ..Default::default() });
    }

    #[test]
    fn without_a_recording_cost_is_estimated_from_the_transcript_and_limits_are_none() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join(".claude");
        claude_session(&claude, 7, SID_A, &[assistant_line("claude-opus-5-5", 100, 20), assistant_line("claude-opus-5-5", 5, 1)]);
        claude_session(&claude, 8, SID_B, &[]);
        let mut store = Store::new(claude).with_alive(|_| true);
        let cards = store.refresh(1);
        let fresh = cards.iter().find(|c| c.session_id == SID_B).unwrap();
        assert!(fresh.usage.is_none(), "nothing counted yet is no usage, not zeros");
        let card = cards.into_iter().find(|c| c.session_id == SID_A).unwrap();
        let usage = card.usage.unwrap();
        assert_eq!((usage.tokens.input, usage.tokens.output), (105, 21));
        let cost = usage.cost.unwrap();
        assert!(cost.estimated);
        assert!(cost.micros > 0);
        assert_eq!(cost.models, vec!["claude-opus-5-5".to_string()]);
        assert!(card.limits.is_none());
        assert!(card.status_line.is_none());
    }

    #[test]
    fn the_newest_recording_sets_every_claude_cards_limits() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join(".claude");
        let maya = claude.join("maya");
        claude_session(&claude, 7, SID_A, &[assistant_line("claude-opus-5-5", 1, 1)]);
        claude_session(&claude, 8, SID_B, &[assistant_line("claude-opus-5-5", 1, 1)]);
        crate::statusline::write_recorded(&maya, &recording(SID_A, "a", 1_000)).unwrap();
        let mut store = Store::new(claude).with_alive(|_| true);
        let cards = store.refresh(2_000);
        let card = |sid: &str| cards.iter().find(|c| c.session_id == sid).unwrap().clone();
        assert!(card(SID_A).limits.is_some());
        assert_eq!(card(SID_B).limits, card(SID_A).limits);
        assert_eq!(card(SID_B).status_line, None, "the line stays the session's own");

        let mut newer = recording(SID_B, "b", 3_000);
        newer.rate_limits = Some(serde_json::json!({"five_hour": {"used_percentage": 5}}));
        crate::statusline::write_recorded(&maya, &newer).unwrap();
        let cards = store.refresh(4_000);
        for c in &cards {
            let limits = c.limits.clone().unwrap();
            assert_eq!((limits.read_at, limits.windows[0].percent), (3_000, 5), "{}", c.session_id);
        }

        // A session that has ended may hold the freshest reading.
        let mut ended = recording("00000000-0000-0000-0000-0000000000ff", "gone", 5_000);
        ended.rate_limits = Some(serde_json::json!({"five_hour": {"used_percentage": 9}}));
        crate::statusline::write_recorded(&maya, &ended).unwrap();
        let cards = store.refresh(6_000);
        assert_eq!(cards.len(), 2);
        for c in &cards {
            let limits = c.limits.clone().unwrap();
            assert_eq!((limits.read_at, limits.windows[0].percent), (5_000, 9), "{}", c.session_id);
        }
    }

    #[test]
    fn recordings_without_rate_limits_give_every_claude_card_no_limits() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join(".claude");
        claude_session(&claude, 7, SID_A, &[assistant_line("claude-opus-5-5", 1, 1)]);
        claude_session(&claude, 8, SID_B, &[assistant_line("claude-opus-5-5", 1, 1)]);
        let mut api_key = recording(SID_A, "a", 1_000);
        api_key.rate_limits = None;
        crate::statusline::write_recorded(&claude.join("maya"), &api_key).unwrap();
        let mut store = Store::new(claude).with_alive(|_| true);
        let cards = store.refresh(2_000);
        assert_eq!(cards.len(), 2);
        for c in &cards {
            let limits = c.limits.clone().expect("recorded, so read: no limits rather than not read yet");
            assert!(limits.windows.is_empty(), "{}", c.session_id);
            assert_eq!(limits.read_at, 1_000);
        }
    }

    fn grok_events(dir: &Path, mtime_secs: u64) -> PathBuf {
        let events = dir.join("events.jsonl");
        std::fs::write(&events, "{}\n").unwrap();
        std::fs::File::options().write(true).open(&events).unwrap().set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(mtime_secs)).unwrap();
        events
    }

    #[test]
    fn grok_usage_is_due_once_per_minute_and_only_after_the_events_file_changed() {
        let t = tempfile::tempdir().unwrap();
        let events = grok_events(t.path(), 1_000);
        let s = ForeignSession { harness: Harness::Grok, pid: 11, tty: None, session_id: "g1".into(), cwd: "/x".into(), name: "g".into(), transcript_path: events.clone() };
        let mut store = Store::new(t.path().join("claude")).with_alive(|_| true).with_foreign(t.path().join("codex"), t.path().join("agy"), vec![s]);
        let now = 10_000_000;
        assert_eq!(store.usage_due(now), crate::collect::Due { antigravity: false, kiro: false, grok: vec!["g1".into()] }, "never run");
        assert!(store.usage_due(now + 120_000).grok.is_empty(), "the events file has not changed");
        grok_events(t.path(), 2_000);
        assert_eq!(store.usage_due(now + 130_000).grok, vec!["g1".to_string()], "changed, and the last run is over a minute old");
        grok_events(t.path(), 3_000);
        assert!(store.usage_due(now + 150_000).grok.is_empty(), "changed, but the last run is 20 s old");
        assert_eq!(store.usage_due(now + 190_000).grok, vec!["g1".to_string()]);
        // A run that failed is retried after the gap without the file changing.
        store.set_grok_usage("g1", None, now + 191_000);
        assert!(store.usage_due(now + 200_000).grok.is_empty(), "the failed run is 9 s old");
        assert_eq!(store.usage_due(now + 260_000).grok, vec!["g1".to_string()], "retried after the gap, same file");
        store.set_grok_usage("g1", Some(SessionUsage { tokens: Default::default(), cost: None }), now + 261_000);
        assert!(store.usage_due(now + 400_000).grok.is_empty(), "a good run waits for the file to change");
    }

    fn agy_session(dir: &Path, pid: i32, id: &str) -> ForeignSession {
        ForeignSession { harness: Harness::Antigravity, pid, tty: Some(format!("ttys0{pid}")), session_id: id.into(), cwd: "/x".into(), name: id.into(), transcript_path: dir.join(format!("{id}.jsonl")) }
    }

    #[test]
    fn nudged_limits_are_due_at_once_when_stale() {
        let t = tempfile::tempdir().unwrap();
        let mut store = Store::new(t.path().join("claude")).with_alive(|_| true).with_foreign(t.path().join("codex"), t.path().join("agy"), vec![agy_session(t.path(), 43, "a1")]);
        let now = 10_000_000;
        assert!(store.usage_due(now).antigravity, "never read, and an Antigravity card is on the board");
        assert!(!store.usage_due(now + 1_000).antigravity);
        store.nudge_limits(Harness::Antigravity, now + 30_000);
        assert!(!store.usage_due(now + 30_000).antigravity, "a reading under a minute old is fresh enough");
        store.nudge_limits(Harness::Antigravity, now + 61_000);
        assert!(store.usage_due(now + 61_000).antigravity, "a window opened on a stale reading");
        assert!(!store.usage_due(now + 62_000).antigravity, "the nudge is spent");
        assert!(store.usage_due(now + 61_000 + Store::AGY_LIMITS_GAP_MS).antigravity, "every five minutes while the card is on the board");
        store.nudge_limits(Harness::Kiro, now + 400_000);
        assert!(!store.usage_due(now + 400_000).kiro, "no Kiro card, no Kiro run");
    }

    #[test]
    fn set_limits_stamps_every_card_of_that_harness() {
        use crate::usage::LimitWindow;
        let t = tempfile::tempdir().unwrap();
        let codex = ForeignSession { harness: Harness::Codex, pid: 42, tty: Some("ttys001".into()), session_id: "c1".into(), cwd: "/x".into(), name: "c1".into(), transcript_path: t.path().join("rollout.jsonl") };
        let sessions = vec![agy_session(t.path(), 43, "a1"), agy_session(t.path(), 44, "a2"), codex];
        let mut store = Store::new(t.path().join("claude")).with_alive(|_| true).with_foreign(t.path().join("codex"), t.path().join("agy"), sessions);
        let limits = AccountLimits { account: None, windows: vec![LimitWindow { label: "Gemini 7d".into(), percent: 40, resets_at: Some(1_793_877_569) }], read_at: 5 };
        store.set_limits(Harness::Antigravity, Some(limits.clone()), 5);
        let cards = store.refresh(10);
        let of = |id: &str| cards.iter().find(|c| c.session_id == id).unwrap().limits.clone();
        assert_eq!((of("a1"), of("a2"), of("c1")), (Some(limits.clone()), Some(limits), None));
        store.set_limits(Harness::Antigravity, None, 20);
        assert!(store.refresh(30).iter().all(|c| c.limits.is_none()), "None clears the limits");
    }

    #[test]
    fn a_failed_limits_read_keeps_the_last_reading_and_is_logged_once_an_hour() {
        use crate::usage::LimitWindow;
        let t = tempfile::tempdir().unwrap();
        let mut store = Store::new(t.path().join("claude")).with_alive(|_| true).with_foreign(t.path().join("codex"), t.path().join("agy"), vec![agy_session(t.path(), 43, "a1")]);
        let now = 10_000_000;
        assert!(store.limits_failed(Harness::Antigravity, now), "the first failure is logged");
        assert!(store.refresh(now).iter().all(|c| c.limits.is_none()), "never read: no limits");
        let limits = AccountLimits { account: None, windows: vec![LimitWindow { label: "Gemini 7d".into(), percent: 40, resets_at: None }], read_at: now + 1 };
        store.set_limits(Harness::Antigravity, Some(limits.clone()), now + 1);
        assert!(!store.limits_failed(Harness::Antigravity, now + 300_000), "under an hour since the last log");
        assert_eq!(store.refresh(now + 300_000)[0].limits, Some(limits), "the last good reading stays");
        assert!(store.limits_failed(Harness::Antigravity, now + Store::LIMITS_LOG_GAP_MS), "an hour on, logged again");
        assert!(store.limits_failed(Harness::Kiro, now + 1), "each harness logs on its own");
        assert!(!store.limits_failed(Harness::Grok, now), "no limits slot, nothing to log");
    }

    #[test]
    fn grok_usage_reaches_its_card_and_a_failed_run_keeps_the_last_reading() {
        use crate::usage::{Cost, CostUnit, TokenCounts};
        let t = tempfile::tempdir().unwrap();
        let grok = t.path().join("grok");
        let sdir = crate::grok::session_dir(&grok, "/Users/x", "g1");
        std::fs::create_dir_all(&sdir).unwrap();
        std::fs::write(grok.join("active_sessions.json"), r#"[{"session_id":"g1","pid":11,"cwd":"/Users/x"}]"#).unwrap();
        let mut store = Store::new(t.path().join("claude")).with_alive(|pid| pid == 11).with_agent_dirs(t.path().join("codex"), t.path().join("agy"), grok);
        let usage = SessionUsage { tokens: TokenCounts { input: 7, output: 3, ..Default::default() }, cost: Some(Cost { micros: 40_362, unit: CostUnit::Usd, estimated: false, models: vec![] }) };
        assert!(store.refresh(1).iter().find(|c| c.session_id == "g1").unwrap().usage.is_none());
        store.set_grok_usage("g1", Some(usage.clone()), 2);
        assert_eq!(store.refresh(3).iter().find(|c| c.session_id == "g1").unwrap().usage, Some(usage.clone()));
        store.set_grok_usage("g1", None, 4);
        assert_eq!(store.refresh(5).iter().find(|c| c.session_id == "g1").unwrap().usage, Some(usage));
    }
}
