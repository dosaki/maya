//! Which Android notifications a board change posts and clears, the lines
//! they and the foreground service show, and what their buttons send. Pure:
//! the Android side is behind `Alerts`, so this is tested on the host.

use maya_core::answer::Pick;
use maya_core::model::{AwaitKind, Card, State};
use maya_core::net::protocol::CommandKind;
use maya_core::notify::body_for;
use serde::Serialize;
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

/// The channel for a notification the phone also speaks: no sound, so none
/// clashes with the speech (`silent()` is iOS-only in the plugin).
pub const SPOKEN_CHANNEL: &str = "spoken";

/// What a notification's button sends, through the main like the card's
/// own controls. `ask` is the card's `state_since`, so a button left on an
/// ask that has since moved on is refused rather than answering the next.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Answer {
    /// Picks a row of the ask's menu.
    Option { ask: u64, question: usize, index: usize },
    /// Types into the menu's own-words row; the button opens a text box.
    Own { ask: u64, question: usize },
    /// Sends what is typed, as the card's reply box does; the button opens a text box.
    Reply,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Button {
    pub label: String,
    #[serde(flatten)]
    pub answer: Answer,
}

/// The most buttons Android shows on a notification.
pub const MAX_BUTTONS: usize = 3;

/// The longest choice that still fits on a button with two others beside
/// it; Android shortens a longer one rather than wrap it.
pub const BUTTON_LABEL_CHARS: usize = 12;

/// The longest text a pulled-down notification shows; Android drops the
/// notification altogether when its extras grow too large.
pub const LONG_BODY_CHARS: usize = 4000;

/// One notification to show. `id` is stable per session, so a newer one
/// replaces the older in place; `session_id` rides along so a tap opens
/// the card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Post {
    pub id: i32,
    pub kind: Kind,
    pub title: String,
    pub body: String,
    /// What the notification shows pulled down: the whole ask with its
    /// choices, or the whole last message.
    pub long_body: String,
    pub buttons: Vec<Button>,
    pub session_id: String,
    /// What the phone says for it when it speaks events (`notify::spoken_line`).
    pub spoken: Option<String>,
    /// Posted on `SPOKEN_CHANNEL` because the phone is saying it aloud.
    pub quiet: bool,
    /// Replaces the notification on screen without a sound of its own.
    pub update: bool,
}

impl Post {
    pub fn channel_id(&self) -> &'static str {
        if self.quiet { SPOKEN_CHANNEL } else { self.kind.channel_id() }
    }
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
    /// The foreground service's text ("Joined to desk").
    fn service_line(&self, line: &str);
}

/// The foreground service's notification id (`KeepAliveService.NOTIFICATION_ID`).
pub const SERVICE_NOTIFICATION_ID: i32 = 1;

/// FNV-1a over the session id, folded to a positive i32 and kept clear of
/// the service's own id. The same id across runs, so a notification left
/// from an earlier run is replaced in place rather than doubled.
pub fn notification_id(session_id: &str) -> i32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in session_id.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    ((h & 0x7fff_ffff) as i32).max(SERVICE_NOTIFICATION_ID + 1)
}

/// The notifications an earlier run left on screen, to clear at startup.
/// `Posted` lives only in memory, so a decision taken or a session gone
/// while Maya was not running would otherwise leave its notification up
/// for good. The board shows whatever is still pending. The service's own
/// notification is not ours to clear.
pub fn leftovers(active: &[i32]) -> Vec<i32> {
    active.iter().copied().filter(|&id| id != SERVICE_NOTIFICATION_ID).collect()
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
    out.push(Action::Post(post_for(c, kind, body)));
}

fn post_for(c: &Card, kind: Kind, body: String) -> Post {
    let long_body = match kind {
        Kind::Decision => ask_text(c),
        Kind::Finished => body.clone(),
    };
    Post { id: notification_id(&c.session_id), kind, title: title_for(c), body, long_body, buttons: buttons_for(c, kind), session_id: c.session_id.clone(), spoken: maya_core::notify::spoken_line(c), quiet: false, update: false }
}

impl Posted {
    /// Whether this session's notification of this kind is still up.
    pub fn has(&self, session_id: &str, kind: Kind) -> bool {
        self.0.get(session_id) == Some(&kind)
    }
}

/// The ask in full, pulled down: what it asks, then each question with
/// its choices numbered as the terminal numbers them.
pub fn ask_text(card: &Card) -> String {
    let Some(aw) = &card.awaiting else { return body_for(card) };
    let detail = aw.detail.trim();
    let mut lines: Vec<String> = vec![];
    if !detail.is_empty() {
        lines.push(detail.to_string());
    }
    for q in &aw.questions {
        let question = q.question.trim();
        if !question.is_empty() && question != detail {
            lines.push(question.to_string());
        }
        lines.extend(q.options.iter().enumerate().map(|(i, o)| format!("{}. {}", i + 1, o.label)));
    }
    if lines.is_empty() {
        return body_for(card);
    }
    maya_core::state::truncate(&lines.join("\n"), LONG_BODY_CHARS)
}

/// The pulled-down text of a finished turn: its last message in full.
pub fn finished_text(text: &str) -> String {
    maya_core::state::truncate(text.trim(), LONG_BODY_CHARS)
}

/// The buttons for a notification. A finished turn and an ask in prose
/// take a reply. A single question takes its first choices (by number
/// when one is too long for its button), and its own-words row as a reply
/// when it has one; anything else (several
/// questions, a multi-select, an ask without a menu) is answered on the
/// card, where the reply box also waits for the menu to be answered first.
pub fn buttons_for(card: &Card, kind: Kind) -> Vec<Button> {
    let reply = || Button { label: "Reply".into(), answer: Answer::Reply };
    if kind == Kind::Finished {
        return vec![reply()];
    }
    let Some(aw) = card.awaiting.as_ref().filter(|_| card.state == State::Awaiting) else { return vec![] };
    let ask = card.state_since;
    match aw.questions.as_slice() {
        [] if aw.kind == AwaitKind::Text => vec![reply()],
        [q] if !q.multi_select => {
            let own = q.own.as_ref().map(|o| Button { label: o.label.clone(), answer: Answer::Own { ask, question: 0 } });
            let room = MAX_BUTTONS - usize::from(own.is_some());
            let shown = &q.options[..q.options.len().min(room)];
            // One choice too long and they all go by number, as `ask_text` numbers them.
            let numbered = shown.iter().any(|o| o.label.chars().count() > BUTTON_LABEL_CHARS);
            let mut out: Vec<Button> = shown.iter().enumerate().map(|(index, o)| Button { label: if numbered { (index + 1).to_string() } else { o.label.clone() }, answer: Answer::Option { ask, question: 0, index } }).collect();
            out.extend(own);
            out
        }
        _ => vec![],
    }
}

/// A button pressed on a notification, as the Android side reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pressed {
    pub session_id: String,
    pub answer: Answer,
    /// What was typed, for `Own` and `Reply`.
    pub text: String,
}

/// Reads the Android side's `{sessionId, kind, ask, question, index, text}`.
pub fn pressed(event: &serde_json::Value) -> Option<Pressed> {
    let session_id = event.get("sessionId")?.as_str().filter(|s| !s.is_empty())?.to_string();
    let num = |k: &str| event.get(k).and_then(serde_json::Value::as_u64);
    let answer = match event.get("kind")?.as_str()? {
        "option" => Answer::Option { ask: num("ask")?, question: num("question")? as usize, index: num("index")? as usize },
        "own" => Answer::Own { ask: num("ask")?, question: num("question")? as usize },
        "reply" => Answer::Reply,
        _ => return None,
    };
    let text = event.get("text").and_then(serde_json::Value::as_str).unwrap_or("").trim().to_string();
    // Android sends an empty reply when the box is submitted blank.
    if text.is_empty() && !matches!(answer, Answer::Option { .. }) {
        return None;
    }
    Some(Pressed { session_id, answer, text })
}

/// The command a pressed button sends to the main.
pub fn command_for(p: &Pressed) -> CommandKind {
    let session = p.session_id.clone();
    match p.answer {
        Answer::Option { ask, question, index } => CommandKind::Pick { session, ask_id: ask, question, pick: Pick::Option { index } },
        Answer::Own { ask, question } => CommandKind::Pick { session, ask_id: ask, question, pick: Pick::Own { text: p.text.clone() } },
        Answer::Reply => CommandKind::Reply { session, text: p.text.clone(), attachments: vec![] },
    }
}

/// The notification again after a button's answer failed to send: the same
/// buttons, to try again, under why it failed.
pub fn failed_post(card: &Card, kind: Kind, error: &str) -> Post {
    let mut p = post_for(card, kind, format!("Not sent: {error}"));
    p.long_body = format!("Not sent: {error}\n\n{}", match kind {
        Kind::Decision => ask_text(card),
        Kind::Finished => finished_body(card),
    });
    p.update = true;
    p
}

/// The foreground service's line: the main this phone has joined.
pub fn service_line(main: &str, connected: bool) -> String {
    if connected {
        format!("Joined to {main}")
    } else {
        format!("Reconnecting to {main}…")
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
            branch: None, brain: false, shell: false,
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
    fn a_post_carries_the_line_to_speak_and_starts_loud() {
        let mut posted = Posted::default();
        let mut c = card("s1", State::Awaiting, 1);
        c.machine = None;
        let actions = plan(&[c.clone()], &[c], &[], &mut posted, ON);
        let Action::Post(p) = &actions[0] else { panic!("expected a post") };
        assert_eq!(p.spoken.as_deref(), Some("hexgrid needs a decision"));
        assert!(!p.quiet);
        assert_eq!(p.channel_id(), "decisions");
        assert_eq!(Post { quiet: true, ..p.clone() }.channel_id(), SPOKEN_CHANNEL);
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
    fn leftovers_are_every_active_notification_but_the_services() {
        assert_eq!(leftovers(&[1, 42, 7]), vec![42, 7]);
        assert!(leftovers(&[SERVICE_NOTIFICATION_ID]).is_empty());
        assert!(leftovers(&[]).is_empty());
    }

    #[test]
    fn the_service_line_names_the_main() {
        assert_eq!(service_line("desk", true), "Joined to desk");
        assert_eq!(service_line("desk", false), "Reconnecting to desk…");
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

    fn menu(labels: &[&str], own: bool) -> maya_core::model::Question {
        maya_core::model::Question {
            question: "Which one?".into(),
            header: "Pick".into(),
            options: labels.iter().map(|l| maya_core::model::Choice { label: l.to_string(), description: String::new() }).collect(),
            multi_select: false,
            own: own.then(|| maya_core::model::OwnRow { label: "Type something".into(), row: labels.len() }),
            chat_row: None,
        }
    }

    fn asking(questions: Vec<maya_core::model::Question>) -> Card {
        let mut c = card("a", State::Awaiting, 7);
        c.awaiting = Some(Awaiting { kind: AwaitKind::Question, detail: "Run `cargo test`?".into(), questions });
        c
    }

    #[test]
    fn pulled_down_an_ask_shows_its_question_and_every_choice() {
        let c = asking(vec![menu(&["Yes", "Yes, and don't ask again", "No"], false)]);
        assert_eq!(ask_text(&c), "Run `cargo test`?\nWhich one?\n1. Yes\n2. Yes, and don't ask again\n3. No");
        let mut bare = c.clone();
        bare.awaiting = None;
        assert_eq!(ask_text(&bare), "Waiting for a decision");
        let mut posted = Posted::default();
        let Action::Post(p) = &plan(&[c.clone()], &[c], &[], &mut posted, ON)[0] else { panic!() };
        assert!(p.long_body.ends_with("3. No"), "{}", p.long_body);
        assert_eq!(p.body, "Run `cargo test`?");
    }

    #[test]
    fn a_single_question_gets_its_first_choices_and_its_own_words_row() {
        let three = buttons_for(&asking(vec![menu(&["A", "B", "C", "D"], false)]), Kind::Decision);
        assert_eq!(three.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(), ["A", "B", "C"]);
        assert_eq!(three[2].answer, Answer::Option { ask: 7, question: 0, index: 2 });
        let with_own = buttons_for(&asking(vec![menu(&["A", "B", "C"], true)]), Kind::Decision);
        assert_eq!(with_own.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(), ["A", "B", "Type something"]);
        assert_eq!(with_own[2].answer, Answer::Own { ask: 7, question: 0 });
    }

    #[test]
    fn choices_too_long_for_a_button_are_numbered_as_the_text_numbers_them() {
        let long = asking(vec![menu(&["Commit, bump, open PR", "No", "Keep testing first"], true)]);
        let buttons = buttons_for(&long, Kind::Decision);
        assert_eq!(buttons.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(), ["1", "2", "Type something"]);
        assert_eq!(buttons[1].answer, Answer::Option { ask: 7, question: 0, index: 1 });
        assert!(ask_text(&long).contains("1. Commit, bump, open PR\n2. No"));
        // Every label fits: they stay.
        let short = buttons_for(&asking(vec![menu(&["Yes", "Always allow", "No"], false)]), Kind::Decision);
        assert_eq!(short.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(), ["Yes", "Always allow", "No"]);
    }

    #[test]
    fn asks_answered_only_on_the_card_get_no_buttons() {
        assert!(buttons_for(&asking(vec![menu(&["A"], false), menu(&["B"], false)]), Kind::Decision).is_empty());
        let mut multi = menu(&["A", "B"], true);
        multi.multi_select = true;
        assert!(buttons_for(&asking(vec![multi]), Kind::Decision).is_empty());
        // A permission ask whose menu is not known: answered on the card.
        assert!(buttons_for(&card("a", State::Awaiting, 1), Kind::Decision).is_empty());
    }

    #[test]
    fn a_prose_ask_and_a_finished_turn_take_a_reply() {
        let mut prose = card("a", State::Awaiting, 1);
        prose.awaiting = Some(Awaiting { kind: AwaitKind::Text, detail: "Which branch?".into(), questions: vec![] });
        assert_eq!(buttons_for(&prose, Kind::Decision), vec![Button { label: "Reply".into(), answer: Answer::Reply }]);
        assert_eq!(buttons_for(&card("a", State::Completed, 1), Kind::Finished), vec![Button { label: "Reply".into(), answer: Answer::Reply }]);
    }

    #[test]
    fn buttons_go_to_android_flat() {
        let b = Button { label: "Yes".into(), answer: Answer::Option { ask: 7, question: 0, index: 1 } };
        assert_eq!(serde_json::to_value(&b).unwrap(), serde_json::json!({"label": "Yes", "kind": "option", "ask": 7, "question": 0, "index": 1}));
        let r = Button { label: "Reply".into(), answer: Answer::Reply };
        assert_eq!(serde_json::to_value(&r).unwrap(), serde_json::json!({"label": "Reply", "kind": "reply"}));
    }

    #[test]
    fn a_pressed_button_becomes_the_cards_own_command() {
        let opt = pressed(&serde_json::json!({"sessionId": "s1", "kind": "option", "ask": 7, "question": 0, "index": 2})).unwrap();
        let CommandKind::Pick { session, ask_id: 7, question: 0, pick: Pick::Option { index: 2 } } = command_for(&opt) else { panic!("{opt:?}") };
        assert_eq!(session, "s1");
        let own = pressed(&serde_json::json!({"sessionId": "s1", "kind": "own", "ask": 7, "question": 0, "text": " rename it "})).unwrap();
        let CommandKind::Pick { pick: Pick::Own { text }, .. } = command_for(&own) else { panic!("{own:?}") };
        assert_eq!(text, "rename it");
        let reply = pressed(&serde_json::json!({"sessionId": "s1", "kind": "reply", "text": "now push it"})).unwrap();
        let CommandKind::Reply { text, attachments, .. } = command_for(&reply) else { panic!("{reply:?}") };
        assert_eq!((text.as_str(), attachments.len()), ("now push it", 0));
    }

    #[test]
    fn a_blank_reply_or_a_garbled_event_sends_nothing() {
        assert_eq!(pressed(&serde_json::json!({"sessionId": "s1", "kind": "reply", "text": "  "})), None);
        assert_eq!(pressed(&serde_json::json!({"sessionId": "s1", "kind": "own", "ask": 7, "question": 0})), None);
        assert_eq!(pressed(&serde_json::json!({"sessionId": "", "kind": "reply", "text": "x"})), None);
        assert_eq!(pressed(&serde_json::json!({"sessionId": "s1", "kind": "option", "question": 0, "index": 1})), None);
        assert_eq!(pressed(&serde_json::json!({"sessionId": "s1", "kind": "dance"})), None);
    }

    #[test]
    fn a_failed_answer_reposts_with_why_and_the_same_buttons() {
        let c = asking(vec![menu(&["Yes", "No"], false)]);
        let p = failed_post(&c, Kind::Decision, "Not joined to a main.");
        assert_eq!(p.body, "Not sent: Not joined to a main.");
        assert!(p.long_body.starts_with("Not sent: Not joined to a main.\n\nRun `cargo test`?"), "{}", p.long_body);
        assert_eq!(p.buttons, buttons_for(&c, Kind::Decision));
        assert_eq!(p.id, notification_id("a"));
    }

    #[test]
    fn the_full_text_is_capped() {
        assert_eq!(finished_text(&"x".repeat(5000)).chars().count(), LONG_BODY_CHARS);
        assert_eq!(finished_text("  done  "), "done");
    }
}
