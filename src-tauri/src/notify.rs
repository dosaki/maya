use crate::model::{AwaitKind, Card, State};
use crate::state::truncate;
use std::collections::HashSet;
use std::process::Command;

/// Remembers which asks have been announced so each one notifies once.
/// The first refresh only primes it: asks already open when Maya starts
/// are not announced.
#[derive(Default)]
pub struct Notifier {
    seen: HashSet<(String, u64)>,
    primed: bool,
}

impl Notifier {
    /// The cards whose ask is new since the last call.
    pub fn take_new(&mut self, cards: &[Card]) -> Vec<Card> {
        let mut out = Vec::new();
        for c in cards.iter().filter(|c| c.state == State::Awaiting) {
            if self.seen.insert((c.session_id.clone(), c.state_since)) && self.primed {
                out.push(c.clone());
            }
        }
        self.primed = true;
        out
    }
}

fn applescript_string(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// A Notification Center banner with a sound.
pub fn applescript_notify(title: &str, subtitle: &str, body: &str) -> String {
    format!(
        "display notification \"{}\" with title \"{}\" subtitle \"{}\" sound name \"default\"",
        applescript_string(body),
        applescript_string(title),
        applescript_string(subtitle)
    )
}

/// What the notification says: the ask itself, or its kind when the detail is blank.
pub fn body_for(card: &Card) -> String {
    let Some(aw) = &card.awaiting else { return "Waiting for a decision".into() };
    let detail = aw.detail.trim().trim_matches(|c| c == '*' || c == '_' || c == ' ');
    if !detail.is_empty() {
        return truncate(detail, 200);
    }
    match aw.kind {
        AwaitKind::Question | AwaitKind::Text => "Waiting for an answer",
        AwaitKind::Plan => "Waiting for plan approval",
        AwaitKind::Permission => "Waiting for a permission decision",
    }
    .into()
}

pub fn notify(card: &Card) {
    let project = card.cwd.rsplit('/').find(|s| !s.is_empty()).unwrap_or(&card.cwd);
    let _ = Command::new("osascript").arg("-e").arg(applescript_notify(&card.name, project, &body_for(card))).output();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AwaitKind, Awaiting, Card, Harness, State};

    fn card(id: &str, state: State, since: u64, detail: &str) -> Card {
        Card {
            session_id: id.into(),
            pid: 1,
            name: format!("{id}-name"),
            cwd: format!("/Users/x/dev/{id}"),
            state,
            state_since: since,
            snippet: "".into(),
            awaiting: if state == State::Awaiting { Some(Awaiting { kind: AwaitKind::Text, detail: detail.into(), questions: vec![] }) } else { None },
            has_inbox: true,
            harness: Harness::ClaudeCode,
            pr: None,
            context: None,
        }
    }

    #[test]
    fn first_refresh_primes_without_notifying_then_each_new_ask_notifies_once() {
        let mut n = Notifier::default();
        let already = card("a", State::Awaiting, 100, "Old question?");
        assert!(n.take_new(&[already.clone(), card("b", State::Working, 100, "")]).is_empty(), "asks open at startup are not announced");
        // Same ask again: nothing. A new session asking: once.
        assert!(n.take_new(&[already.clone()]).is_empty());
        let fresh = card("b", State::Awaiting, 200, "Go ahead?");
        let got = n.take_new(&[already.clone(), fresh.clone()]);
        assert_eq!(got.iter().map(|c| c.session_id.as_str()).collect::<Vec<_>>(), vec!["b"]);
        assert!(n.take_new(&[already.clone(), fresh.clone()]).is_empty());
        // The same session asking something new (a later state_since) notifies again.
        let again = card("b", State::Awaiting, 300, "And this?");
        assert_eq!(n.take_new(&[again.clone()]).len(), 1);
        // A session that leaves and comes back with the same ask timestamp is not re-announced.
        assert!(n.take_new(&[]).is_empty());
        assert!(n.take_new(&[again.clone()]).is_empty());
    }

    #[test]
    fn script_escapes_text_and_names_the_session_project_and_ask() {
        let s = applescript_notify("eye-1", "eye", "Bash: rm -rf \"x\" \\ y");
        assert!(s.starts_with("display notification \"Bash: rm -rf \\\"x\\\" \\\\ y\""), "{s}");
        assert!(s.contains("with title \"eye-1\""));
        assert!(s.contains("subtitle \"eye\""));
        assert!(s.contains("sound name \"default\""));
    }

    #[test]
    fn body_uses_the_ask_detail_and_falls_back_to_the_kind() {
        let c = card("a", State::Awaiting, 1, "Create it as drafted?");
        assert_eq!(body_for(&c), "Create it as drafted?");
        let mut p = card("a", State::Awaiting, 1, "");
        p.awaiting = Some(Awaiting { kind: AwaitKind::Permission, detail: "".into(), questions: vec![] });
        assert_eq!(body_for(&p), "Waiting for a permission decision");
        let long = card("a", State::Awaiting, 1, &"x".repeat(300));
        assert!(body_for(&long).chars().count() <= 200);
        // Markdown emphasis around the ask is noise in a banner.
        let bold = card("a", State::Awaiting, 1, "**Would you like me to commit?**");
        assert_eq!(body_for(&bold), "Would you like me to commit?");
        let under = card("a", State::Awaiting, 1, "_Ready?_ ");
        assert_eq!(body_for(&under), "Ready?");
    }
}
