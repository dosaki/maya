//! Board changes since the brain's last turn, as short notes.

use crate::model::{Card, State};

pub(crate) fn state_word(s: State) -> String {
    serde_json::to_value(s).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
}

/// One note for a card that changed state between two boards, if it did.
pub(crate) fn state_note(before: &Card, after: &Card) -> Option<String> {
    (before.state != after.state).then(|| format!("{} moved to {}", after.name, state_word(after.state)))
}

/// What changed on the board between `before` and `after`. Brain cards are ignored.
pub fn changes(before: &[Card], after: &[Card]) -> Vec<String> {
    let mut notes = Vec::new();
    for b in before.iter().filter(|c| !c.brain) {
        match after.iter().find(|a| a.session_id == b.session_id) {
            Some(a) => notes.extend(state_note(b, a)),
            None => notes.push(format!("{} ended", b.name)),
        }
    }
    for a in after.iter().filter(|c| !c.brain) {
        if !before.iter().any(|b| b.session_id == a.session_id) {
            notes.push(format!("new session {} ({})", a.name, state_word(a.state)));
        }
    }
    notes
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::Harness;

    pub(crate) fn card(id: &str, name: &str, state: State) -> Card {
        Card {
            session_id: id.into(), pid: 1, name: name.into(), cwd: format!("/x/{id}"), state, state_since: 0, snippet: "".into(),
            awaiting: None, has_inbox: true, harness: Harness::ClaudeCode, pr: None, context: None, machine: None, machine_address: None,
            machine_platform: None, terminal: None, stale: false, model: None, branch: None, brain: false,
        }
    }

    #[test]
    fn a_state_change_becomes_a_note() {
        let before = [card("1", "api-fix", State::Working)];
        let after = [card("1", "api-fix", State::Completed)];
        assert_eq!(changes(&before, &after), vec!["api-fix moved to completed".to_string()]);
    }

    #[test]
    fn an_ended_and_a_new_session_are_noted_and_brain_cards_are_not() {
        let mut brain = card("b", "maya", State::Working);
        brain.brain = true;
        let before = [card("1", "api-fix", State::Working), brain.clone()];
        let mut brain_after = brain;
        brain_after.state = State::Idle;
        let after = [card("2", "docs", State::Working), brain_after];
        assert_eq!(changes(&before, &after), vec!["api-fix ended".to_string(), "new session docs (working)".to_string()]);
    }

    #[test]
    fn an_unchanged_board_has_no_notes() {
        let b = [card("1", "api-fix", State::Working)];
        assert!(changes(&b, &b).is_empty());
    }
}
