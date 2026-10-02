//! Names chosen in the New session modal for agents that take none when
//! they start (Codex, Antigravity, Grok Build). The name shows on the new
//! session's card at once; `/rename` is typed once the session is free.
//! Typing earlier is unsafe: keys sent to a TUI that is not at its input
//! box act as shortcuts.

use crate::model::{Card, Harness, State};
use std::collections::HashSet;

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
    pub fn new(harness: Harness, cwd: &str, name: &str, launched_ms: u64, known: Vec<String>, session_id: Option<String>) -> Self {
        Self { harness, cwd: cwd.to_string(), name: name.to_string(), launched_ms, known: known.into_iter().collect(), session_id, seen: false, seen_working: false, renamed: false }
    }
}

/// Ready for typed input: not running a turn and not asking anything.
pub fn is_free(card: &Card) -> bool {
    matches!(card.state, State::Completed | State::Idle)
}

fn same_dir(a: &str, b: &str) -> bool {
    a.trim_end_matches(['/', '\\']) == b.trim_end_matches(['/', '\\'])
}

#[derive(Debug, Default)]
pub struct PendingNames(Vec<PendingName>);

impl PendingNames {
    pub fn add(&mut self, p: PendingName) {
        self.0.push(p);
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Drops the name waiting for `session_id`: the user renamed it themselves.
    pub fn forget(&mut self, session_id: &str) {
        self.0.retain(|p| p.session_id.as_deref() != Some(session_id));
    }

    /// Matches names to new sessions, shows them on `cards`, and returns
    /// `(session id, name)` for each session to `/rename` now. A name is
    /// dropped once the agent's own name is it, once its session has ended,
    /// or when no session came within `MATCH_WINDOW_MS`.
    pub fn apply(&mut self, cards: &mut [Card], now_ms: u64) -> Vec<(String, String)> {
        let mut claimed: HashSet<String> = self.0.iter().filter_map(|p| p.session_id.clone()).collect();
        for p in self.0.iter_mut().filter(|p| p.session_id.is_none()) {
            let found = cards.iter().find(|c| c.harness == p.harness && same_dir(&c.cwd, &p.cwd) && !p.known.contains(&c.session_id) && !claimed.contains(&c.session_id));
            if let Some(c) = found {
                claimed.insert(c.session_id.clone());
                p.session_id = Some(c.session_id.clone());
            }
        }
        let mut due = Vec::new();
        self.0.retain_mut(|p| {
            let waiting = now_ms.saturating_sub(p.launched_ms) < MATCH_WINDOW_MS;
            let Some(id) = p.session_id.clone() else { return waiting };
            let Some(card) = cards.iter_mut().find(|c| c.session_id == id) else {
                // Gone after it was seen: ended. Never seen (Grok's id): still coming.
                return !p.seen && waiting;
            };
            p.seen = true;
            // Once renamed, the only thing left to wait for is the agent's
            // own name catching up; once it has, there is nothing more to do.
            if p.renamed && card.name == p.name {
                return false;
            }
            card.name = p.name.clone();
            p.seen_working |= card.state == State::Working;
            // A turn must have run: before that the first prompt may still be arriving.
            if !p.renamed && is_free(card) && (p.seen_working || !card.snippet.is_empty()) {
                p.renamed = true;
                due.push((id, p.name.clone()));
            }
            true
        });
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
    fn forget_drops_the_name_of_one_session() {
        let mut p = PendingNames::default();
        p.add(pending("Fix CI", &[]));
        let mut cards = vec![card("s1", Harness::Codex, "/dev/a", State::Working, "")];
        p.apply(&mut cards, 2_000);
        p.forget("s1");
        assert!(p.is_empty());
    }

    #[test]
    fn free_means_completed_or_idle() {
        assert!(is_free(&card("a", Harness::Codex, "/", State::Completed, "")));
        assert!(is_free(&card("a", Harness::Codex, "/", State::Idle, "")));
        assert!(!is_free(&card("a", Harness::Codex, "/", State::Working, "")));
        assert!(!is_free(&card("a", Harness::Codex, "/", State::Awaiting, "")));
    }
}
