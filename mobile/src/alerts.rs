//! Which Android notifications a board change posts and clears, and the
//! lines they and the foreground service show. Pure: the Android side is
//! behind `Alerts`, so this is tested on the host.

use maya_core::model::{Card, State};
use maya_core::notify::body_for;
use std::collections::HashMap;

/// Which channel a notification goes on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Decision,
    Finished,
}

impl Kind {
    pub fn channel_id(self) -> &'static str {
        match self {
            Kind::Decision => "decisions",
            Kind::Finished => "finished",
        }
    }
}

/// One notification to show. `id` is stable per session, so a newer one
/// replaces the older in place; `session_id` rides along so a tap opens
/// the card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Post {
    pub id: i32,
    pub kind: Kind,
    pub title: String,
    pub body: String,
    pub session_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Post(Post),
    Clear(i32),
}

/// The two Network-screen switches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Switches {
    pub awaiting: bool,
    pub completed: bool,
}

/// The notifications on screen, by session id: which kind each is.
#[derive(Default, Debug)]
pub struct Posted(HashMap<String, Kind>);

/// What shows and clears notifications and keeps the service's line: the
/// Android side, or a logger on the desktop.
pub trait Alerts: Send + Sync {
    fn post(&self, post: &Post);
    fn clear(&self, id: i32);
    /// The foreground service's text ("Main for 2 assistants, 1 connected").
    fn service_line(&self, line: &str);
}

/// FNV-1a over the session id, folded to a positive i32 and kept clear of
/// 1, the service's own id. The same id across runs, so a notification left
/// from an earlier run is replaced or cleared rather than doubled.
pub fn notification_id(session_id: &str) -> i32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in session_id.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    ((h & 0x7fff_ffff) as i32).max(2)
}

/// "hexgrid on maya-mini needs a decision" / "… is finished"; the machine is
/// the label `merge::merged` set, so two assistants of one name read apart.
pub fn title_for(card: &Card) -> String {
    let who = match &card.machine {
        Some(m) => format!("{} on {m}", card.name),
        None => card.name.clone(),
    };
    match card.state {
        State::Awaiting => format!("{who} needs a decision"),
        State::Completed => format!("{who} is finished"),
        _ => who,
    }
}

/// The last assistant text, or a stock line when there is none.
pub fn finished_body(card: &Card) -> String {
    let s = card.snippet.trim();
    if s.is_empty() {
        "Finished its turn".into()
    } else {
        s.chars().take(200).collect()
    }
}

/// The notifications to post and clear for this board. Clears come first,
/// for every posted session whose card moved on or left the board (a
/// decision taken at the desk); then one post per new ask and per finished
/// turn, each behind its switch.
pub fn plan(cards: &[Card], fresh: &[Card], finished: &[Card], posted: &mut Posted, switches: Switches) -> Vec<Action> {
    let mut out = vec![];
    let now: HashMap<&str, State> = cards.iter().map(|c| (c.session_id.as_str(), c.state)).collect();
    posted.0.retain(|id, kind| {
        let still = matches!((now.get(id.as_str()), *kind), (Some(State::Awaiting), Kind::Decision) | (Some(State::Completed), Kind::Finished));
        if !still {
            out.push(Action::Clear(notification_id(id)));
        }
        still
    });
    if switches.awaiting {
        for c in fresh {
            push(&mut out, posted, c, Kind::Decision, body_for(c));
        }
    }
    if switches.completed {
        for c in finished {
            push(&mut out, posted, c, Kind::Finished, finished_body(c));
        }
    }
    out
}

fn push(out: &mut Vec<Action>, posted: &mut Posted, c: &Card, kind: Kind, body: String) {
    posted.0.insert(c.session_id.clone(), kind);
    out.push(Action::Post(Post { id: notification_id(&c.session_id), kind, title: title_for(c), body, session_id: c.session_id.clone() }));
}

/// The foreground service's line.
pub fn service_line(paired: usize, connected: usize) -> String {
    match (paired, connected) {
        (0, _) => "No assistants paired yet".into(),
        (1, 1) => "Main for 1 assistant, connected".into(),
        (1, _) => "Main for 1 assistant, not connected".into(),
        (n, c) => format!("Main for {n} assistants, {c} connected"),
    }
}

/// The phone's name as assistants see it: the device model, or "Android" when that is blank.
pub fn default_name(model: &str) -> String {
    let m = model.trim();
    if m.is_empty() {
        "Android".to_string()
    } else {
        m.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maya_core::model::{AwaitKind, Awaiting, Harness};

    fn card(id: &str, state: State, since: u64) -> Card {
        Card {
            session_id: id.into(),
            pid: 0,
            name: "hexgrid".into(),
            cwd: "/home/u/dev/hexgrid".into(),
            state,
            state_since: since,
            snippet: "All done, the branch is pushed.".into(),
            awaiting: (state == State::Awaiting).then(|| Awaiting { kind: AwaitKind::Permission, detail: "Run `cargo test`?".into(), questions: vec![] }),
            has_inbox: true,
            harness: Harness::ClaudeCode,
            pr: None,
            context: None,
            machine: Some("maya-mini (10.0.0.2)".into()),
            machine_address: Some("10.0.0.2".into()),
            machine_platform: Some("macos".into()),
            terminal: None,
            stale: false,
            model: None,
        }
    }

    const ON: Switches = Switches { awaiting: true, completed: true };

    #[test]
    fn titles_carry_the_machine_label_as_merged() {
        assert_eq!(title_for(&card("a", State::Awaiting, 1)), "hexgrid on maya-mini (10.0.0.2) needs a decision");
        assert_eq!(title_for(&card("a", State::Completed, 1)), "hexgrid on maya-mini (10.0.0.2) is finished");
        let mut local = card("a", State::Awaiting, 1);
        local.machine = None;
        assert_eq!(title_for(&local), "hexgrid needs a decision");
    }

    #[test]
    fn a_new_ask_posts_once_on_the_decisions_channel_with_the_ask_as_body() {
        let c = card("a", State::Awaiting, 1);
        let mut posted = Posted::default();
        let out = plan(&[c.clone()], &[c.clone()], &[], &mut posted, ON);
        assert_eq!(out.len(), 1);
        let Action::Post(p) = &out[0] else { panic!("{out:?}") };
        assert_eq!((p.kind, p.id, p.session_id.as_str()), (Kind::Decision, notification_id("a"), "a"));
        assert_eq!(p.body, "Run `cargo test`?");
        // The same board again: nothing new, nothing cleared.
        assert!(plan(&[c], &[], &[], &mut posted, ON).is_empty());
    }

    #[test]
    fn a_decision_taken_elsewhere_clears_its_notification() {
        let asked = card("a", State::Awaiting, 1);
        let mut posted = Posted::default();
        plan(&[asked.clone()], &[asked], &[], &mut posted, ON);
        let working = card("a", State::Working, 2);
        assert_eq!(plan(&[working], &[], &[], &mut posted, ON), vec![Action::Clear(notification_id("a"))]);
        // Gone from the board entirely: also cleared, once.
        let asked = card("b", State::Awaiting, 1);
        plan(&[asked.clone()], &[asked], &[], &mut posted, ON);
        assert_eq!(plan(&[], &[], &[], &mut posted, ON), vec![Action::Clear(notification_id("b"))]);
        assert!(plan(&[], &[], &[], &mut posted, ON).is_empty());
    }

    #[test]
    fn awaiting_then_finished_swaps_channels_and_the_switches_gate_each() {
        let asked = card("a", State::Awaiting, 1);
        let mut posted = Posted::default();
        plan(&[asked.clone()], &[asked], &[], &mut posted, ON);
        let done = card("a", State::Completed, 2);
        let out = plan(&[done.clone()], &[], &[done.clone()], &mut posted, ON);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], Action::Clear(notification_id("a")));
        let Action::Post(p) = &out[1] else { panic!("{out:?}") };
        assert_eq!((p.kind, p.body.as_str()), (Kind::Finished, "All done, the branch is pushed."));
        // Finished off: the finish is neither posted nor remembered.
        let mut quiet = Posted::default();
        assert!(plan(&[done.clone()], &[], &[done], &mut quiet, Switches { awaiting: true, completed: false }).is_empty());
        let asked = card("c", State::Awaiting, 1);
        assert!(plan(&[asked.clone()], &[asked], &[], &mut quiet, Switches { awaiting: false, completed: true }).is_empty());
    }

    #[test]
    fn notification_ids_are_stable_positive_and_never_the_services() {
        assert_eq!(notification_id("abc"), notification_id("abc"));
        assert_ne!(notification_id("abc"), notification_id("abd"));
        assert!(notification_id("") >= 2);
        for id in ["1", "x", "session-9"] {
            assert!(notification_id(id) >= 2, "{id}");
        }
    }

    #[test]
    fn service_lines_count_assistants() {
        assert_eq!(service_line(0, 0), "No assistants paired yet");
        assert_eq!(service_line(1, 1), "Main for 1 assistant, connected");
        assert_eq!(service_line(1, 0), "Main for 1 assistant, not connected");
        assert_eq!(service_line(3, 2), "Main for 3 assistants, 2 connected");
    }

    #[test]
    fn the_default_name_is_the_model_or_android() {
        assert_eq!(default_name(" Pixel 8 "), "Pixel 8");
        assert_eq!(default_name(""), "Android");
        assert_eq!(default_name("   "), "Android");
    }

    #[test]
    fn a_finished_turn_with_no_snippet_still_has_a_body() {
        let mut c = card("a", State::Completed, 1);
        c.snippet = "  ".into();
        assert_eq!(finished_body(&c), "Finished its turn");
        c.snippet = "x".repeat(300);
        assert_eq!(finished_body(&c).chars().count(), 200);
    }
}
