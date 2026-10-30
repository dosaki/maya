//! The sessions this machine remembers, so the ones that were running when
//! Maya or the machine stopped can be offered back after a restart. Kept in
//! `~/.claude/maya/remembered.json`. Pure logic: the store feeds it each
//! refresh (`observe`), the close path (`closed`) and the heartbeat.

use crate::model::{Card, Harness};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A session gone this long while Maya ran was closed by the user; one that
/// went within this of the last heartbeat went down with the machine.
pub const GRACE_MS: u64 = 120_000;
/// How often the heartbeat is written while Maya runs.
pub const HEARTBEAT_MS: u64 = 30_000;
/// The pause between two terminals one resume opens.
pub const RESUME_GAP_MS: u64 = 400;
/// How long an ignored offer is carried from run to run: a day after it
/// was first offered, judged by the last run's heartbeat.
pub const OFFER_TTL_MS: u64 = 24 * 60 * 60 * 1000;

/// What a remembered session is known by: its agent and its id.
pub type Key = (Harness, String);

/// `<claude dir>/maya/remembered.json`.
pub fn path_in(claude_dir: &Path) -> PathBuf {
    claude_dir.join("maya").join("remembered.json")
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// A session this machine saw running.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Remembered {
    pub agent: Harness,
    pub session_id: String,
    pub dir: String,
    pub name: String,
    pub first_seen_ms: u64,
    pub last_seen_ms: u64,
    /// When a refresh first missed it; None while it runs.
    #[serde(default)]
    pub gone_at_ms: Option<u64>,
    /// It was running when the last run stopped: offered until resumed,
    /// forgotten, or seen live again.
    #[serde(default, skip_serializing_if = "is_false")]
    pub offered: bool,
    /// When it was first offered; an offer is carried for `OFFER_TTL_MS`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offered_at_ms: Option<u64>,
    /// Why its last resume failed ("its folder is gone"…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// It ran in a terminal Maya hosted (Native), so it resumes in one.
    #[serde(default, skip_serializing_if = "is_false")]
    pub native: bool,
}

impl Remembered {
    pub fn key(&self) -> Key {
        (self.agent, self.session_id.clone())
    }
}

/// A session a refresh saw running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveSession {
    pub agent: Harness,
    pub session_id: String,
    pub dir: String,
    pub name: String,
    pub native: bool,
}

impl LiveSession {
    pub fn of(card: &Card) -> LiveSession {
        LiveSession { agent: card.harness, session_id: card.session_id.clone(), dir: card.cwd.clone(), name: card.name.clone(), native: card.native }
    }
}

/// Only a session its agent can resume by a plain id is kept.
fn recordable(s: &LiveSession) -> bool {
    s.agent != Harness::Other && crate::resume::plain_session_id(&s.session_id) && !s.dir.is_empty()
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Record {
    #[serde(default)]
    heartbeat_ms: u64,
    #[serde(default)]
    sessions: Vec<Remembered>,
}

/// The file and what this run made of it.
pub struct Memory {
    path: PathBuf,
    record: Record,
    /// Something changed since the last save.
    dirty: bool,
}

impl Memory {
    /// The file at `path`. Missing reads as empty; unreadable too, logged,
    /// so it never blocks startup.
    pub fn load(path: PathBuf) -> Memory {
        let record = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                crate::log::line("remembered", format!("{} is unreadable, starting empty: {e}", path.display()));
                Record::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Record::default(),
            Err(e) => {
                crate::log::line("remembered", format!("{} is unreadable, starting empty: {e}", path.display()));
                Record::default()
            }
        };
        Memory { path, record, dirty: false }
    }

    /// Writes the file, readable by its owner only, when something changed.
    pub fn save(&mut self) -> Result<(), String> {
        if !self.dirty {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(&self.record).map_err(|e| e.to_string())?;
        // Written beside the file and renamed over it: a power cut mid-write
        // leaves the old file whole, never a cut-off one.
        let mut tmp = self.path.clone().into_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        let written = crate::config::write_private(&tmp, text.as_bytes()).and_then(|_| std::fs::rename(&tmp, &self.path));
        if let Err(e) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(e.to_string());
        }
        self.dirty = false;
        Ok(())
    }

    /// At startup, before the first refresh: what the last run left is
    /// judged against its last heartbeat. Still running when it stopped, or
    /// gone within `GRACE_MS` of its last heartbeat: was running, offered.
    /// Gone earlier: the user closed it. Returns what is offered.
    pub fn classify_at_start(&mut self, now_ms: u64) -> Vec<Remembered> {
        let heartbeat = self.record.heartbeat_ms;
        self.record.sessions.retain_mut(|e| {
            if e.agent == Harness::Other {
                return false;
            }
            if e.offered {
                // A file from before offers were dated: dated now, once.
                let at = *e.offered_at_ms.get_or_insert(now_ms);
                return heartbeat.saturating_sub(at) <= OFFER_TTL_MS;
            }
            let running = match e.gone_at_ms {
                None => true,
                Some(gone) => heartbeat.saturating_sub(gone) <= GRACE_MS,
            };
            if running {
                e.offered = true;
                e.offered_at_ms = Some(now_ms);
                e.gone_at_ms = None;
                e.error = None;
            }
            running
        });
        self.record.heartbeat_ms = now_ms;
        self.dirty = true;
        self.was_running()
    }

    /// One refresh's live sessions: new ones are added, known ones follow
    /// their name and folder (and are no longer offered), missing ones are
    /// marked gone, and one gone longer than `GRACE_MS` while this run
    /// watched is dropped. Offered ones that are not live are left alone.
    pub fn observe(&mut self, live: &[LiveSession], now_ms: u64) {
        let mut seen: Vec<Key> = Vec::new();
        for s in live.iter().filter(|s| recordable(s)) {
            let key = (s.agent, s.session_id.clone());
            if seen.contains(&key) {
                continue;
            }
            seen.push(key.clone());
            match self.record.sessions.iter_mut().find(|e| e.key() == key) {
                Some(e) => {
                    if e.name != s.name || e.dir != s.dir || e.native != s.native || e.gone_at_ms.is_some() || e.offered || e.error.is_some() {
                        e.name = s.name.clone();
                        e.dir = s.dir.clone();
                        e.native = s.native;
                        e.gone_at_ms = None;
                        e.offered = false;
                        e.offered_at_ms = None;
                        e.error = None;
                        self.dirty = true;
                    }
                    // Kept in memory: the heartbeat's write carries it.
                    e.last_seen_ms = now_ms;
                }
                None => {
                    self.record.sessions.push(Remembered { agent: s.agent, session_id: s.session_id.clone(), dir: s.dir.clone(), name: s.name.clone(), first_seen_ms: now_ms, last_seen_ms: now_ms, gone_at_ms: None, offered: false, offered_at_ms: None, error: None, native: s.native });
                    self.dirty = true;
                }
            }
        }
        let before = self.record.sessions.len();
        let mut marked = false;
        self.record.sessions.retain_mut(|e| {
            if e.offered || seen.contains(&e.key()) {
                return true;
            }
            match e.gone_at_ms {
                None => {
                    e.gone_at_ms = Some(now_ms);
                    marked = true;
                    true
                }
                Some(gone) => now_ms.saturating_sub(gone) <= GRACE_MS,
            }
        });
        if marked || self.record.sessions.len() != before {
            self.dirty = true;
        }
    }

    /// Closed from its card: forgotten at once.
    pub fn closed(&mut self, agent: Harness, session_id: &str) {
        self.forget(&[(agent, session_id.to_string())]);
    }

    pub fn forget(&mut self, keys: &[Key]) {
        let before = self.record.sessions.len();
        self.record.sessions.retain(|e| !keys.contains(&e.key()));
        if self.record.sessions.len() != before {
            self.dirty = true;
        }
    }

    /// Keeps why its resume failed, until it is dismissed.
    pub fn failed(&mut self, key: &Key, reason: &str) {
        if let Some(e) = self.record.sessions.iter_mut().find(|e| e.key() == *key) {
            e.error = Some(reason.to_string());
            self.dirty = true;
        }
    }

    /// This run's last sign of life.
    pub fn heartbeat(&mut self, now_ms: u64) {
        self.record.heartbeat_ms = now_ms;
        self.dirty = true;
    }

    /// What is offered, oldest `last_seen_ms` first.
    pub fn was_running(&self) -> Vec<Remembered> {
        let mut out: Vec<Remembered> = self.record.sessions.iter().filter(|e| e.offered).cloned().collect();
        out.sort_by_key(|e| e.last_seen_ms);
        out
    }

    pub fn entry(&self, agent: Harness, session_id: &str) -> Option<Remembered> {
        self.record.sessions.iter().find(|e| e.agent == agent && e.session_id == session_id).cloned()
    }

    pub fn heartbeat_ms(&self) -> u64 {
        self.record.heartbeat_ms
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn sessions(&self) -> &[Remembered] {
        &self.record.sessions
    }
}

/// One "was running" entry as the banner lists it: the machine it runs on
/// (None: this one) and whether that machine is away.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RememberedRow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    #[serde(default)]
    pub waiting: bool,
    #[serde(flatten)]
    pub entry: Remembered,
}

/// What became of one row of a resume; `error` is its reason.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResumeOutcome {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    pub agent: Harness,
    pub session_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ResumeOutcome {
    pub fn of(machine: Option<&str>, e: &Remembered, result: &Result<(), String>) -> ResumeOutcome {
        ResumeOutcome { machine: machine.map(String::from), agent: e.agent, session_id: e.session_id.clone(), name: e.name.clone(), error: result.as_ref().err().cloned() }
    }
}

/// The rows of one resume by where they run: this machine's, each connected
/// machine's (oldest first in each), and the ones whose machine is away.
#[derive(Debug, Default, PartialEq)]
pub struct ResumePlan {
    pub here: Vec<Remembered>,
    pub remote: Vec<(String, Vec<Remembered>)>,
    pub waiting: Vec<RememberedRow>,
}

pub fn plan(rows: &[RememberedRow]) -> ResumePlan {
    let mut p = ResumePlan::default();
    for r in rows {
        match (&r.machine, r.waiting) {
            (_, true) => p.waiting.push(r.clone()),
            (None, false) => p.here.push(r.entry.clone()),
            (Some(m), false) => match p.remote.iter_mut().find(|(n, _)| n == m) {
                Some((_, list)) => list.push(r.entry.clone()),
                None => p.remote.push((m.clone(), vec![r.entry.clone()])),
            },
        }
    }
    p.here.sort_by_key(|e| e.last_seen_ms);
    for (_, list) in &mut p.remote {
        list.sort_by_key(|e| e.last_seen_ms);
    }
    p
}

/// The rows as this Maya knows their machines, not as a page sent them: a
/// row of a machine `connected` denies is waiting, and a row of this one is
/// never waiting.
pub fn settle(rows: &[RememberedRow], connected: &dyn Fn(&str) -> bool) -> Vec<RememberedRow> {
    rows.iter()
        .map(|r| {
            let waiting = r.machine.as_deref().is_some_and(|m| !connected(m));
            RememberedRow { waiting, ..r.clone() }
        })
        .collect()
}

/// The stored entries `asked` names: same agent, session and folder. A
/// request for anything else is dropped; nothing is resumed from the page's
/// own say-so.
pub fn trusted(stored: &[Remembered], asked: &[Remembered]) -> Vec<Remembered> {
    asked.iter().filter_map(|a| stored.iter().find(|s| s.key() == a.key() && s.dir == a.dir).cloned()).collect()
}

/// The rows' keys by machine (None: this one), in first-seen order, with
/// whether that machine is away.
pub fn keys_by_machine(rows: &[RememberedRow]) -> Vec<(Option<String>, bool, Vec<Key>)> {
    let mut out: Vec<(Option<String>, bool, Vec<Key>)> = Vec::new();
    for r in rows {
        match out.iter_mut().find(|(m, _, _)| *m == r.machine) {
            Some((_, waiting, keys)) => {
                *waiting |= r.waiting;
                keys.push(r.entry.key());
            }
            None => out.push((r.machine.clone(), r.waiting, vec![r.entry.key()])),
        }
    }
    out
}

/// What a row whose machine is away says, and why Resume all skipped it.
pub fn waiting_for(machine: &str) -> String {
    format!("waiting for {machine}")
}

/// Whether the `tick`th turn (from 1) of a loop ticking every `tick_ms`
/// writes the heartbeat: every `HEARTBEAT_MS`.
pub fn heartbeat_due(tick: u64, tick_ms: u64) -> bool {
    tick > 0 && tick_ms > 0 && (tick * tick_ms) % HEARTBEAT_MS == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Harness;

    fn live(id: &str) -> LiveSession {
        LiveSession { agent: Harness::ClaudeCode, session_id: id.into(), dir: format!("/Users/x/dev/{id}"), name: format!("{id}-name"), native: false }
    }

    fn entry(id: &str) -> Remembered {
        Remembered { agent: Harness::ClaudeCode, session_id: id.into(), dir: format!("/Users/x/dev/{id}"), name: format!("{id}-name"), first_seen_ms: 1, last_seen_ms: 2, gone_at_ms: None, offered: true, offered_at_ms: None, error: None, native: false }
    }

    fn memory() -> (tempfile::TempDir, Memory) {
        let t = tempfile::tempdir().unwrap();
        let m = Memory::load(path_in(t.path()));
        (t, m)
    }

    fn ids(list: &[Remembered]) -> Vec<&str> {
        list.iter().map(|e| e.session_id.as_str()).collect()
    }

    #[test]
    fn only_a_stored_session_in_its_stored_folder_is_trusted() {
        let stored = vec![entry("a"), entry("b")];
        let moved = Remembered { dir: "/etc".into(), ..entry("a") };
        let got = trusted(&stored, &[entry("a"), moved, entry("zz")]);
        assert_eq!(ids(&got), ["a"]);
        assert_eq!(got[0].dir, "/Users/x/dev/a");
    }

    #[test]
    fn settle_marks_a_disconnected_machine_waiting_whatever_the_page_said() {
        let row = |m: Option<&str>, waiting| RememberedRow { machine: m.map(String::from), waiting, entry: entry("a") };
        let got = settle(&[row(Some("desk"), false), row(Some("lap"), true), row(None, true)], &|m| m == "lap");
        assert_eq!(got.iter().map(|r| r.waiting).collect::<Vec<_>>(), [true, false, false]);
    }

    #[test]
    fn a_session_is_added_on_first_sight_and_follows_its_name_and_folder() {
        let (_t, mut m) = memory();
        m.classify_at_start(1);
        m.observe(&[live("a")], 1_000);
        let a = m.entry(Harness::ClaudeCode, "a").unwrap();
        assert_eq!((a.first_seen_ms, a.last_seen_ms, a.gone_at_ms, a.offered), (1_000, 1_000, None, false));
        let moved = LiveSession { dir: "/Users/x/dev/two".into(), name: "renamed".into(), ..live("a") };
        m.observe(&[moved], 6_000);
        let a = m.entry(Harness::ClaudeCode, "a").unwrap();
        assert_eq!((a.dir.as_str(), a.name.as_str(), a.first_seen_ms, a.last_seen_ms), ("/Users/x/dev/two", "renamed", 1_000, 6_000));
    }

    #[test]
    fn observe_records_that_a_session_is_native() {
        let (_t, mut m) = memory();
        m.classify_at_start(1);
        m.observe(&[LiveSession { native: true, ..live("a") }], 1_000);
        assert!(m.entry(Harness::ClaudeCode, "a").unwrap().native);
        m.observe(&[live("a")], 2_000);
        assert!(!m.entry(Harness::ClaudeCode, "a").unwrap().native);
    }

    #[test]
    fn other_agents_odd_ids_and_duplicates_are_never_recorded_twice_or_at_all() {
        let (_t, mut m) = memory();
        m.classify_at_start(1);
        let other = LiveSession { agent: Harness::Other, ..live("o1") };
        m.observe(&[other, live("../etc"), live("ok"), live("ok")], 2);
        assert_eq!(ids(m.sessions()), ["ok"]);
    }

    #[test]
    fn a_vanished_session_is_marked_gone_comes_back_and_is_dropped_after_the_grace() {
        let (_t, mut m) = memory();
        m.classify_at_start(1_000);
        m.observe(&[live("a"), live("b")], 2_000);
        m.observe(&[live("a")], 10_000);
        assert_eq!(m.entry(Harness::ClaudeCode, "b").unwrap().gone_at_ms, Some(10_000));
        m.observe(&[live("a"), live("b")], 20_000);
        assert_eq!(m.entry(Harness::ClaudeCode, "b").unwrap().gone_at_ms, None, "back: live again");
        m.observe(&[live("a")], 30_000);
        m.observe(&[live("a")], 30_000 + GRACE_MS);
        assert!(m.entry(Harness::ClaudeCode, "b").is_some(), "not past the grace yet");
        m.observe(&[live("a")], 30_001 + GRACE_MS);
        assert!(m.entry(Harness::ClaudeCode, "b").is_none(), "gone for longer than the grace while Maya ran: the user closed it");
        assert!(m.was_running().is_empty());
    }

    #[test]
    fn a_session_closed_from_its_card_is_dropped_at_once() {
        let (_t, mut m) = memory();
        m.classify_at_start(1);
        m.observe(&[live("a"), live("b")], 2);
        m.closed(Harness::ClaudeCode, "a");
        assert_eq!(ids(m.sessions()), ["b"]);
    }

    #[test]
    fn at_start_what_was_live_or_gone_within_the_grace_of_the_last_heartbeat_was_running() {
        let t = tempfile::tempdir().unwrap();
        let path = path_in(t.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let record = serde_json::json!({
            "heartbeatMs": 1_000_000,
            "sessions": [
                { "agent": "claude-code", "sessionId": "live", "dir": "/x", "name": "live", "firstSeenMs": 1, "lastSeenMs": 900_000, "goneAtMs": null },
                { "agent": "codex", "sessionId": "late", "dir": "/x", "name": "late", "firstSeenMs": 1, "lastSeenMs": 950_000, "goneAtMs": 950_000 },
                { "agent": "grok", "sessionId": "old", "dir": "/x", "name": "old", "firstSeenMs": 1, "lastSeenMs": 700_000, "goneAtMs": 700_000 },
                { "agent": "kiro", "sessionId": "before", "dir": "/x", "name": "before", "firstSeenMs": 1, "lastSeenMs": 100, "goneAtMs": null, "offered": true },
                { "agent": "future-agent", "sessionId": "f", "dir": "/x", "name": "f", "firstSeenMs": 1, "lastSeenMs": 5 }
            ]
        });
        std::fs::write(&path, record.to_string()).unwrap();
        let mut m = Memory::load(path);
        let offered = m.classify_at_start(2_000_000);
        assert_eq!(ids(&offered), ["before", "live", "late"], "oldest last-seen first; the long-gone one and the unknown agent are dropped");
        assert!(offered.iter().all(|e| e.offered && e.gone_at_ms.is_none()));
        assert!(offered.iter().all(|e| e.offered_at_ms == Some(2_000_000)), "offered now; one carried from a file with no date is dated now");
        assert_eq!(m.heartbeat_ms(), 2_000_000);
    }

    #[test]
    fn an_offered_session_seen_live_again_is_no_longer_offered() {
        let (t, mut m) = memory();
        m.classify_at_start(1);
        m.observe(&[live("a")], 2);
        m.heartbeat(3);
        m.save().unwrap();
        // The next run, over the file this one left.
        let mut next = Memory::load(path_in(t.path()));
        assert_eq!(ids(&next.classify_at_start(10)), ["a"]);
        next.observe(&[], 11);
        assert_eq!(ids(&next.was_running()), ["a"], "still offered while it is not running");
        next.observe(&[live("a")], 12);
        assert!(next.was_running().is_empty(), "resumed by hand: it leaves the list on its own");
        assert_eq!(next.entry(Harness::ClaudeCode, "a").unwrap().gone_at_ms, None);
    }

    #[test]
    fn an_ignored_offer_is_carried_for_a_day_then_dropped() {
        const HOUR: u64 = 60 * 60 * 1000;
        let (t, mut m) = memory();
        m.classify_at_start(1);
        m.observe(&[live("a")], 2);
        m.heartbeat(3);
        m.save().unwrap();
        // Run 1: offers A; the user ignores the banner.
        let start = 10_000;
        let mut run1 = Memory::load(path_in(t.path()));
        assert_eq!(ids(&run1.classify_at_start(start)), ["a"]);
        run1.heartbeat(start + HOUR);
        run1.save().unwrap();
        // Run 2: its last sign of life is still within a day of the offer.
        let mut run2 = Memory::load(path_in(t.path()));
        assert_eq!(ids(&run2.classify_at_start(start + 2 * HOUR)), ["a"], "within a day: still offered");
        run2.heartbeat(start + 24 * HOUR + 1);
        run2.save().unwrap();
        // Run 3: the offer outlived a day of runs: dropped.
        let mut run3 = Memory::load(path_in(t.path()));
        assert!(run3.classify_at_start(start + 25 * HOUR).is_empty(), "an offer ignored for more than a day is not carried");
        assert!(run3.entry(Harness::ClaudeCode, "a").is_none());
    }

    #[test]
    fn a_clock_that_moved_backwards_neither_panics_nor_drops_sessions() {
        let t = tempfile::tempdir().unwrap();
        let path = path_in(t.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // The heartbeat is older than the moment the session went: the clock was put back.
        let record = serde_json::json!({ "heartbeatMs": 1_000, "sessions": [
            { "agent": "claude-code", "sessionId": "a", "dir": "/x", "name": "a", "firstSeenMs": 1, "lastSeenMs": 5_000, "goneAtMs": 5_000 }
        ]});
        std::fs::write(&path, record.to_string()).unwrap();
        let mut m = Memory::load(path);
        assert_eq!(ids(&m.classify_at_start(500)), ["a"], "now is before everything stored");
        m.observe(&[live("b")], 400);
        m.observe(&[], 300);
        assert_eq!(m.entry(Harness::ClaudeCode, "b").unwrap().gone_at_ms, Some(300));
        m.observe(&[], 100);
        assert!(m.entry(Harness::ClaudeCode, "b").is_some(), "time going backwards is not time gone");
        assert_eq!(ids(&m.was_running()), ["a"]);
    }

    #[test]
    fn the_file_round_trips_and_is_written_only_when_something_changed() {
        let (t, mut m) = memory();
        m.classify_at_start(1);
        m.observe(&[live("a")], 2);
        m.save().unwrap();
        let path = path_in(t.path());
        let back = Memory::load(path.clone());
        assert_eq!(back.sessions(), m.sessions());
        assert_eq!(back.heartbeat_ms(), 1);
        std::fs::remove_file(&path).unwrap();
        m.observe(&[live("a")], 3);
        m.save().unwrap();
        assert!(!path.exists(), "only lastSeen moved: nothing written");
        assert_eq!(m.entry(Harness::ClaudeCode, "a").unwrap().last_seen_ms, 3);
        m.heartbeat(30_000);
        m.save().unwrap();
        let back = Memory::load(path);
        assert_eq!((back.heartbeat_ms(), back.entry(Harness::ClaudeCode, "a").unwrap().last_seen_ms), (30_000, 3), "the heartbeat's write carries lastSeen");
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let (t, mut m) = memory();
        m.classify_at_start(1);
        m.save().unwrap();
        assert_eq!(std::fs::metadata(path_in(t.path())).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn a_save_leaves_no_temporary_file_and_a_whole_target() {
        let (t, mut m) = memory();
        m.classify_at_start(1);
        m.observe(&[live("a")], 2);
        m.save().unwrap();
        let path = path_in(t.path());
        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, ["remembered.json"]);
        assert_eq!(Memory::load(path).sessions().len(), 1);
    }

    #[test]
    fn a_session_gone_while_the_clock_read_before_the_start_is_still_pruned() {
        let (_t, mut m) = memory();
        m.classify_at_start(1_000);
        m.observe(&[live("b")], 400);
        m.observe(&[], 300);
        m.observe(&[], 300 + GRACE_MS + 1);
        assert!(m.entry(Harness::ClaudeCode, "b").is_none());
    }

    #[test]
    fn an_unreadable_or_cut_off_file_reads_as_empty_and_the_next_save_writes_a_good_one() {
        let t = tempfile::tempdir().unwrap();
        let path = path_in(t.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"heartbeatMs":123,"sessions":[{"agent":"claude-code","sessionId":"a","di"#).unwrap();
        let mut m = Memory::load(path.clone());
        assert!(m.sessions().is_empty());
        assert!(m.classify_at_start(5).is_empty());
        m.save().unwrap();
        assert_eq!(Memory::load(path.clone()).heartbeat_ms(), 5, "a good file replaced the cut-off one");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(Memory::load(path).sessions().is_empty(), "a directory in its place reads as empty too");
    }

    #[test]
    fn forget_and_failed_change_only_the_named_sessions() {
        let (_t, mut m) = memory();
        m.classify_at_start(1);
        m.observe(&[live("a"), live("b")], 2);
        m.failed(&(Harness::ClaudeCode, "b".into()), "its folder is gone");
        m.failed(&(Harness::Codex, "b".into()), "not this one");
        assert_eq!(m.entry(Harness::ClaudeCode, "b").unwrap().error.as_deref(), Some("its folder is gone"));
        m.forget(&[(Harness::ClaudeCode, "a".into()), (Harness::Codex, "b".into())]);
        assert_eq!(ids(m.sessions()), ["b"]);
    }

    #[test]
    fn a_resume_plan_splits_rows_by_machine_oldest_first() {
        let row = |m: Option<&str>, waiting: bool, id: &str, seen: u64| RememberedRow { machine: m.map(String::from), waiting, entry: Remembered { last_seen_ms: seen, ..entry(id) } };
        let p = plan(&[row(None, false, "l2", 20), row(Some("Gnowee"), false, "g1", 30), row(None, false, "l1", 10), row(Some("pi"), true, "p1", 5), row(Some("Gnowee"), false, "g0", 1)]);
        assert_eq!(ids(&p.here), ["l1", "l2"]);
        assert_eq!(p.remote.len(), 1);
        assert_eq!((p.remote[0].0.as_str(), ids(&p.remote[0].1)), ("Gnowee", vec!["g0", "g1"]));
        assert_eq!(p.waiting.iter().map(|r| r.entry.session_id.as_str()).collect::<Vec<_>>(), ["p1"]);
        assert_eq!(waiting_for("pi"), "waiting for pi");
    }

    #[test]
    fn keys_are_grouped_by_machine_with_whether_it_is_away() {
        let row = |m: Option<&str>, waiting: bool, id: &str| RememberedRow { machine: m.map(String::from), waiting, entry: entry(id) };
        let groups = keys_by_machine(&[row(None, false, "a"), row(Some("pi"), true, "p"), row(None, false, "b")]);
        assert_eq!(groups, vec![(None, false, vec![(Harness::ClaudeCode, "a".to_string()), (Harness::ClaudeCode, "b".to_string())]), (Some("pi".to_string()), true, vec![(Harness::ClaudeCode, "p".to_string())])]);
    }

    #[test]
    fn a_row_is_the_entry_with_its_machine_beside_it() {
        let r = RememberedRow { machine: Some("Gnowee".into()), waiting: true, entry: entry("g1") };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!((v["machine"].as_str(), v["waiting"].as_bool(), v["sessionId"].as_str(), v["agent"].as_str()), (Some("Gnowee"), Some(true), Some("g1"), Some("claude-code")));
        assert_eq!(serde_json::from_value::<RememberedRow>(v).unwrap(), r);
        let local: RememberedRow = serde_json::from_str(r#"{"agent":"codex","sessionId":"c1","dir":"/x","name":"c","firstSeenMs":1,"lastSeenMs":2}"#).unwrap();
        assert_eq!((local.machine, local.waiting, local.entry.offered), (None, false, false));
        let out = ResumeOutcome::of(Some("pi"), &entry("p"), &Err("its folder is gone".into()));
        assert_eq!((out.machine.as_deref(), out.name.as_str(), out.error.as_deref()), (Some("pi"), "p-name", Some("its folder is gone")));
    }

    #[test]
    fn the_heartbeat_is_due_every_thirty_seconds_of_ticks() {
        let due: Vec<u64> = (1..=9).filter(|t| heartbeat_due(*t, 10_000)).collect();
        assert_eq!(due, [3, 6, 9]);
        assert!(!heartbeat_due(0, 10_000));
    }
}
