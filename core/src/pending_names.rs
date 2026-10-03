//! Names chosen in the New session modal for agents that take none when
//! they start (Codex, Antigravity, Grok Build). The name shows on the new
//! session's card at once; `/rename` is typed once the session is free.
//! Typing earlier is unsafe: keys sent to a TUI that is not at its input
//! box act as shortcuts.

use crate::model::{Card, Harness, State};
use std::collections::HashSet;
use std::path::Path;

/// How long a name waits for its session to appear.
pub const MATCH_WINDOW_MS: u64 = 10 * 60 * 1000;

#[derive(Debug, Clone)]
pub struct PendingName {
    harness: Harness,
    cwd: String,
    name: String,
    launched_ms: u64,
    /// Sessions running at launch: none of them is the new one.
    known: HashSet<String>,
    /// The session, once matched; set from the start for Grok's `--session-id`.
    session_id: Option<String>,
    /// Its card has been on the board.
    seen: bool,
    seen_working: bool,
    renamed: bool,
}

impl PendingName {
    /// `harness` is never `Harness::ClaudeCode` in practice: that agent takes
    /// a name on the command line and so never needs a pending entry.
    /// `cwd` is kept as its physical path: the agent reports the folder it
    /// runs in with symlinks resolved, so a projects folder reached through
    /// one (macOS's /tmp, a linked ~/dev) would otherwise never match.
    pub fn new(harness: Harness, cwd: &str, name: &str, launched_ms: u64, known: Vec<String>, session_id: Option<String>) -> Self {
        let cwd = physical(cwd).unwrap_or_else(|| cwd.to_string());
        Self { harness, cwd, name: name.to_string(), launched_ms, known: known.into_iter().collect(), session_id, seen: false, seen_working: false, renamed: false }
    }
}

/// `dir` with symlinks resolved as far as the filesystem allows: when `dir`
/// itself does not exist yet (a review's clone folder, named before `gh
/// repo clone` creates it), the nearest existing ancestor is canonicalized
/// and the remaining, not-yet-existing components are rejoined onto it
/// untouched. None when no ancestor below the filesystem root exists: the
/// root always canonicalizes (on Windows to the current drive, as
/// `\\?\C:\`), which would rewrite a folder that is simply not there.
fn physical(dir: &str) -> Option<String> {
    let path = Path::new(dir);
    if let Ok(p) = std::fs::canonicalize(path) {
        return Some(p.to_string_lossy().into_owned());
    }
    let mut suffix = Vec::new();
    let mut cur = path;
    while let Some(parent) = cur.parent() {
        suffix.push(cur.file_name()?.to_os_string());
        if parent.parent().is_none() {
            return None;
        }
        if let Ok(base) = std::fs::canonicalize(parent) {
            let mut out = base;
            for comp in suffix.iter().rev() {
                out.push(comp);
            }
            return Some(out.to_string_lossy().into_owned());
        }
        cur = parent;
    }
    None
}

/// Ready for typed input: not running a turn and not asking anything.
pub fn is_free(card: &Card) -> bool {
    matches!(card.state, State::Completed | State::Idle)
}

fn same_dir(a: &str, b: &str) -> bool {
    a.trim_end_matches(['/', '\\']) == b.trim_end_matches(['/', '\\'])
}

/// True when a card's folder is the pending name's (already physical)
/// target. The card's folder is resolved only when the plain comparison
/// fails, and this runs only while a name is unmatched, so quiet refreshes
/// cost no file-system calls.
fn is_target(card_cwd: &str, target: &str) -> bool {
    same_dir(card_cwd, target) || physical(card_cwd).is_some_and(|p| same_dir(&p, target))
}

#[derive(Debug, Default)]
pub struct PendingNames {
    entries: Vec<PendingName>,
    /// Every session id an entry has ever been matched to, kept after that
    /// entry drops. Without this, a session freed by one dropped name could
    /// be claimed right back by a different, still-unmatched name — showing
    /// the wrong name on it and typing `/rename` into the wrong session.
    /// Cleared once no unmatched entry remains to contend for anything.
    claimed_ever: HashSet<String>,
    /// What happened to each name, for Maya's log: a name that never got
    /// typed must say why. Collected here so `apply` stays free of the log.
    events: Vec<String>,
}

impl PendingNames {
    pub fn add(&mut self, p: PendingName) {
        self.entries.push(p);
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The names waiting, in no particular order.
    pub fn names(&self) -> Vec<String> {
        self.entries.iter().map(|p| p.name.clone()).collect()
    }

    /// The folders waiting, in no particular order: each already resolved
    /// physically by `PendingName::new`.
    pub fn cwds(&self) -> Vec<String> {
        self.entries.iter().map(|p| p.cwd.clone()).collect()
    }

    /// Drops the name waiting for `session_id`: the user renamed it themselves.
    pub fn forget(&mut self, session_id: &str) {
        let events = &mut self.events;
        self.entries.retain(|p| {
            let mine = p.session_id.as_deref() == Some(session_id);
            if mine {
                events.push(format!("dropped {:?}: the user renamed {session_id}", p.name));
            }
            !mine
        });
    }

    /// Makes the name for `session_id` due again the next time its session
    /// is free: it was due, but the session got busy before `/rename` could
    /// be typed, and keys typed into a busy TUI act as shortcuts.
    pub fn requeue(&mut self, session_id: &str) {
        for p in self.entries.iter_mut().filter(|p| p.session_id.as_deref() == Some(session_id)) {
            p.renamed = false;
        }
    }

    /// What happened to the names since the last call, oldest first.
    pub fn take_events(&mut self) -> Vec<String> {
        std::mem::take(&mut self.events)
    }

    /// Matches names to new sessions, shows them on `cards`, and returns
    /// `(session id, name)` for each session to `/rename` now. A name is
    /// dropped once the agent's own name is it, once its session has ended,
    /// or when no session came within `MATCH_WINDOW_MS`.
    pub fn apply(&mut self, cards: &mut [Card], now_ms: u64) -> Vec<(String, String)> {
        let mut claimed = self.claimed_ever.clone();
        claimed.extend(self.entries.iter().filter_map(|p| p.session_id.clone()));
        // An entry past its window matches nothing: a session appearing that
        // late is not the one launched, and the pass below drops the entry.
        for p in self.entries.iter_mut().filter(|p| p.session_id.is_none() && now_ms.saturating_sub(p.launched_ms) < MATCH_WINDOW_MS) {
            // The folder check goes last: it is the one that may touch the disk.
            let found = cards.iter().find(|c| c.harness == p.harness && !p.known.contains(&c.session_id) && !claimed.contains(&c.session_id) && is_target(&c.cwd, &p.cwd));
            if let Some(c) = found {
                claimed.insert(c.session_id.clone());
                self.claimed_ever.insert(c.session_id.clone());
                p.session_id = Some(c.session_id.clone());
                self.events.push(format!("matched {:?} to {}", p.name, c.session_id));
            }
        }
        let mut due = Vec::new();
        let events = &mut self.events;
        self.entries.retain_mut(|p| {
            let waiting = now_ms.saturating_sub(p.launched_ms) < MATCH_WINDOW_MS;
            let Some(id) = p.session_id.clone() else {
                if !waiting {
                    events.push(format!("dropped {:?}: no new {:?} session appeared in {} within ten minutes", p.name, p.harness, p.cwd));
                }
                return waiting;
            };
            let Some(card) = cards.iter_mut().find(|c| c.session_id == id) else {
                // Gone after it was seen: ended. Never seen (Grok's id): still coming.
                if p.seen {
                    let why = if p.renamed { "ended before the agent showed the new name" } else { "ended before it was free to rename" };
                    events.push(format!("dropped {:?}: {id} {why}", p.name));
                } else if !waiting {
                    events.push(format!("dropped {:?}: its session {id} never appeared within ten minutes", p.name));
                }
                return !p.seen && waiting;
            };
            p.seen = true;
            // Once renamed, the only thing left to wait for is the agent's
            // own name catching up; once it has, there is nothing more to do.
            if p.renamed && card.name == p.name {
                events.push(format!("{:?} is now {id}'s own name", p.name));
                return false;
            }
            card.name = p.name.clone();
            p.seen_working |= card.state == State::Working;
            // A turn must have run: before that the first prompt may still be arriving.
            if !p.renamed && is_free(card) && (p.seen_working || !card.snippet.is_empty()) {
                p.renamed = true;
                events.push(format!("{:?} is due for {id}: it is free", p.name));
                due.push((id, p.name.clone()));
            }
            true
        });
        if self.entries.iter().all(|p| p.session_id.is_some()) {
            self.claimed_ever.clear();
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Card, Harness, State};

    fn card(id: &str, harness: Harness, cwd: &str, state: State, snippet: &str) -> Card {
        Card { session_id: id.into(), pid: 1, name: format!("auto-{id}"), cwd: cwd.into(), state, state_since: 0, snippet: snippet.into(), awaiting: None, has_inbox: false, harness, pr: None, context: None, machine: None, machine_address: None, machine_platform: None, terminal: None, stale: false }
    }

    fn pending(name: &str, known: &[&str]) -> PendingName {
        PendingName::new(Harness::Codex, "/dev/a", name, 1_000, known.iter().map(|s| s.to_string()).collect(), None)
    }

    #[test]
    fn a_new_session_shows_the_name_at_once_and_is_renamed_once_free() {
        let mut p = PendingNames::default();
        p.add(pending("Fix CI", &["old"]));
        let mut cards = vec![card("old", Harness::Codex, "/dev/a", State::Idle, "x"), card("new", Harness::Codex, "/dev/a/", State::Working, "")];
        assert!(p.apply(&mut cards, 2_000).is_empty(), "working: not yet");
        assert_eq!(cards[1].name, "Fix CI");
        assert_eq!(cards[0].name, "auto-old", "a session running before the launch is never it");
        cards[1].state = State::Completed;
        assert_eq!(p.apply(&mut cards, 3_000), vec![("new".to_string(), "Fix CI".to_string())]);
        assert!(p.apply(&mut cards, 4_000).is_empty(), "only once");
        assert_eq!(cards[1].name, "Fix CI", "still shown until the agent's own name is it");
        cards[1].name = "Fix CI".into();
        p.apply(&mut cards, 5_000);
        assert!(p.is_empty());
    }

    #[test]
    fn a_session_that_has_not_started_its_turn_is_not_renamed() {
        // Idle with no reply yet: the first prompt may still be on its way in.
        let mut p = PendingNames::default();
        p.add(pending("Fix CI", &[]));
        let mut cards = vec![card("new", Harness::Codex, "/dev/a", State::Idle, "")];
        assert!(p.apply(&mut cards, 2_000).is_empty());
        cards[0].snippet = "Hi!".into();
        assert_eq!(p.apply(&mut cards, 3_000).len(), 1);
    }

    #[test]
    fn two_names_in_one_folder_claim_different_sessions() {
        let mut p = PendingNames::default();
        p.add(pending("First", &[]));
        p.add(pending("Second", &[]));
        let mut cards = vec![card("s1", Harness::Codex, "/dev/a", State::Working, ""), card("s2", Harness::Codex, "/dev/a", State::Working, "")];
        p.apply(&mut cards, 2_000);
        assert_eq!((cards[0].name.as_str(), cards[1].name.as_str()), ("First", "Second"));
    }

    #[test]
    fn only_the_same_agent_and_folder_match() {
        let mut p = PendingNames::default();
        p.add(pending("Fix CI", &[]));
        let mut cards = vec![card("g", Harness::Grok, "/dev/a", State::Working, ""), card("b", Harness::Codex, "/dev/b", State::Working, "")];
        p.apply(&mut cards, 2_000);
        assert!(cards.iter().all(|c| c.name.starts_with("auto-")));
    }

    #[test]
    fn a_grok_name_waits_for_its_own_session_id() {
        let mut p = PendingNames::default();
        p.add(PendingName::new(Harness::Grok, "/dev/a", "Fix CI", 1_000, vec![], Some("g-1".into())));
        let mut cards = vec![card("other", Harness::Grok, "/dev/a", State::Working, "")];
        p.apply(&mut cards, 2_000);
        assert_eq!(cards[0].name, "auto-other");
        cards.push(card("g-1", Harness::Grok, "/dev/a", State::Working, ""));
        p.apply(&mut cards, 3_000);
        assert_eq!(cards[1].name, "Fix CI");
    }

    #[test]
    fn names_are_dropped_when_the_session_ends_or_never_comes() {
        let mut p = PendingNames::default();
        p.add(pending("Ends", &[]));
        let mut cards = vec![card("s1", Harness::Codex, "/dev/a", State::Working, "")];
        p.apply(&mut cards, 2_000);
        p.apply(&mut [], 3_000);
        assert!(p.is_empty(), "its session ended");
        p.add(pending("Never", &[]));
        p.apply(&mut [], 1_000 + MATCH_WINDOW_MS - 1);
        assert!(!p.is_empty());
        p.apply(&mut [], 1_000 + MATCH_WINDOW_MS);
        assert!(p.is_empty(), "unmatched for ten minutes");
    }

    #[test]
    fn a_session_appearing_once_the_window_is_over_is_not_named() {
        let mut p = PendingNames::default();
        p.add(pending("Late", &[]));
        let mut cards = vec![card("s1", Harness::Codex, "/dev/a", State::Working, "")];
        assert!(p.apply(&mut cards, 1_000 + MATCH_WINDOW_MS).is_empty());
        assert_eq!(cards[0].name, "auto-s1");
        assert!(p.is_empty());
        assert_eq!(p.take_events(), vec!["dropped \"Late\": no new Codex session appeared in /dev/a within ten minutes".to_string()]);
    }

    #[test]
    fn forget_drops_the_name_of_one_session() {
        let mut p = PendingNames::default();
        p.add(pending("Fix CI", &[]));
        let mut cards = vec![card("s1", Harness::Codex, "/dev/a", State::Working, "")];
        p.apply(&mut cards, 2_000);
        p.forget("s1");
        assert!(p.is_empty());
    }

    #[test]
    fn a_dropped_name_does_not_free_its_session_for_another_pending_name() {
        let mut p = PendingNames::default();
        // A, then B, launched into the same folder before s1 exists: s1 is
        // not in B's known sessions, so nothing yet stops B from matching it.
        p.add(pending("A", &[]));
        p.add(pending("B", &[]));
        let mut cards = vec![card("s1", Harness::Codex, "/dev/a", State::Working, "")];
        p.apply(&mut cards, 2_000);
        assert_eq!(cards[0].name, "A", "A claims s1 first; B is still unmatched");
        cards[0].state = State::Completed;
        assert_eq!(p.apply(&mut cards, 3_000), vec![("s1".to_string(), "A".to_string())]);
        // The agent's own name catches up: A's entry drops.
        cards[0].name = "A".into();
        p.apply(&mut cards, 4_000);
        // B is still unmatched. A fresh s1 card, the agent's own again, must
        // stay unclaimed by B rather than being renamed out from under A.
        let mut cards = vec![card("s1", Harness::Codex, "/dev/a", State::Idle, "")];
        cards[0].name = "A".into();
        p.apply(&mut cards, 5_000);
        assert_eq!(cards[0].name, "A", "s1 belongs to A; B must not claim it");
    }

    #[test]
    fn a_freshly_derived_card_keeps_showing_the_name_until_the_agent_catches_up() {
        // Store::refresh derives a brand new Card every call; pending_names
        // must not depend on its own override from a previous call still
        // being on the card, the way the other tests' reused `cards` vec
        // does.
        let mut p = PendingNames::default();
        p.add(pending("Fix CI", &[]));
        let mut cards = vec![card("new", Harness::Codex, "/dev/a", State::Working, "")];
        p.apply(&mut cards, 2_000);
        let mut cards = vec![card("new", Harness::Codex, "/dev/a", State::Completed, "")];
        assert_eq!(p.apply(&mut cards, 3_000), vec![("new".to_string(), "Fix CI".to_string())]);
        // Freshly derived, still under its own old name: must be shown "Fix
        // CI" again, and the rename must still be tracked as not yet landed.
        let mut cards = vec![card("new", Harness::Codex, "/dev/a", State::Idle, "x")];
        p.apply(&mut cards, 4_000);
        assert_eq!(cards[0].name, "Fix CI");
        assert!(!p.is_empty());
        // The agent's own name is "Fix CI" for real now: nothing left to do.
        let mut cards = vec![card("new", Harness::Codex, "/dev/a", State::Idle, "x")];
        cards[0].name = "Fix CI".into();
        p.apply(&mut cards, 5_000);
        assert!(p.is_empty());
    }

    #[test]
    fn every_match_due_name_and_drop_is_told_with_its_reason() {
        let mut p = PendingNames::default();
        p.add(pending("Ends early", &[]));
        let mut cards = vec![card("s1", Harness::Codex, "/dev/a", State::Working, "")];
        p.apply(&mut cards, 2_000);
        assert_eq!(p.take_events(), vec!["matched \"Ends early\" to s1".to_string()]);
        p.apply(&mut [], 3_000);
        assert_eq!(p.take_events(), vec!["dropped \"Ends early\": s1 ended before it was free to rename".to_string()]);

        p.add(pending("Lands", &[]));
        let mut cards = vec![card("s2", Harness::Codex, "/dev/a", State::Completed, "Hi")];
        p.apply(&mut cards, 4_000);
        assert_eq!(p.take_events(), vec!["matched \"Lands\" to s2".to_string(), "\"Lands\" is due for s2: it is free".to_string()]);
        cards[0].name = "Lands".into();
        p.apply(&mut cards, 5_000);
        assert_eq!(p.take_events(), vec!["\"Lands\" is now s2's own name".to_string()]);

        p.add(pending("Never", &[]));
        p.apply(&mut [], 1_000 + MATCH_WINDOW_MS);
        assert_eq!(p.take_events(), vec!["dropped \"Never\": no new Codex session appeared in /dev/a within ten minutes".to_string()]);

        p.add(PendingName::new(Harness::Grok, "/dev/a", "Grok", 1_000, vec![], Some("g-1".into())));
        p.apply(&mut [], 1_000 + MATCH_WINDOW_MS);
        assert_eq!(p.take_events(), vec!["dropped \"Grok\": its session g-1 never appeared within ten minutes".to_string()]);

        p.add(pending("Mine", &[]));
        let mut cards = vec![card("s3", Harness::Codex, "/dev/a", State::Working, "")];
        p.apply(&mut cards, 6_000);
        p.take_events();
        p.forget("s3");
        assert_eq!(p.take_events(), vec!["dropped \"Mine\": the user renamed s3".to_string()]);
        assert!(p.take_events().is_empty(), "each event is taken once");
    }

    #[test]
    fn a_requeued_name_is_due_again_once_its_session_is_free() {
        let mut p = PendingNames::default();
        p.add(pending("Fix CI", &[]));
        let mut cards = vec![card("new", Harness::Codex, "/dev/a", State::Completed, "Hi")];
        assert_eq!(p.apply(&mut cards, 2_000).len(), 1);
        // It went busy before `/rename` could be typed: put it back.
        p.requeue("new");
        cards[0].state = State::Awaiting;
        assert!(p.apply(&mut cards, 3_000).is_empty(), "not while it asks something");
        assert_eq!(cards[0].name, "Fix CI", "still shown meanwhile");
        cards[0].state = State::Completed;
        assert_eq!(p.apply(&mut cards, 4_000), vec![("new".to_string(), "Fix CI".to_string())]);
        assert!(p.apply(&mut cards, 5_000).is_empty(), "once again");
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_reached_through_a_symlink_still_matches() {
        // A projects folder behind a symlink (macOS's /tmp, a linked ~/dev):
        // the agent reports its physical folder, Maya may hold the linked one.
        let t = tempfile::tempdir().unwrap();
        let real = t.path().join("real");
        std::fs::create_dir_all(real.join("proj")).unwrap();
        let link = t.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let physical = std::fs::canonicalize(real.join("proj")).unwrap().to_string_lossy().into_owned();
        let linked = link.join("proj").to_string_lossy().into_owned();

        let mut p = PendingNames::default();
        p.add(PendingName::new(Harness::Codex, &linked, "Linked target", 1_000, vec![], None));
        let mut cards = vec![card("a", Harness::Codex, &format!("{physical}/"), State::Working, "")];
        p.apply(&mut cards, 2_000);
        assert_eq!(cards[0].name, "Linked target", "Maya's target goes through the link");

        let mut p = PendingNames::default();
        p.add(PendingName::new(Harness::Codex, &physical, "Linked card", 1_000, vec![], None));
        let mut cards = vec![card("b", Harness::Codex, &linked, State::Working, "")];
        p.apply(&mut cards, 2_000);
        assert_eq!(cards[0].name, "Linked card", "the card's folder goes through the link");
    }

    #[test]
    fn a_path_whose_only_existing_ancestor_is_the_root_is_left_alone() {
        // On Windows "/" canonicalizes to the current drive, so resolving
        // through the root would turn a folder like "/dev/a" into
        // "\\?\D:\dev\a". A path with no real ancestor is kept as given.
        assert_eq!(physical("/maya-no-such-dir-xyz/a"), None);
        let p = PendingName::new(Harness::Codex, "/maya-no-such-dir-xyz/a", "x", 1_000, vec![], None);
        assert_eq!(p.cwd, "/maya-no-such-dir-xyz/a");
    }

    #[cfg(unix)]
    #[test]
    fn a_not_yet_cloned_folder_behind_a_symlink_is_still_resolved() {
        // A review clone is named before `gh repo clone` creates its folder:
        // `cwd` must still resolve through a symlinked ancestor (macOS's
        // /tmp, a linked clones directory), by canonicalizing the nearest
        // existing ancestor and rejoining the rest untouched.
        let t = tempfile::tempdir().unwrap();
        let real = t.path().join("real");
        std::fs::create_dir_all(&real).unwrap();
        let link = t.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let canonical_real = std::fs::canonicalize(&real).unwrap();

        let target = link.join("not-yet-cloned");
        let p = PendingName::new(Harness::Codex, &target.to_string_lossy(), "review x #1", 1_000, vec![], None);
        assert_eq!(p.cwd, canonical_real.join("not-yet-cloned").to_string_lossy().into_owned());
    }

    #[test]
    fn free_means_completed_or_idle() {
        assert!(is_free(&card("a", Harness::Codex, "/", State::Completed, "")));
        assert!(is_free(&card("a", Harness::Codex, "/", State::Idle, "")));
        assert!(!is_free(&card("a", Harness::Codex, "/", State::Working, "")));
        assert!(!is_free(&card("a", Harness::Codex, "/", State::Awaiting, "")));
    }
}
