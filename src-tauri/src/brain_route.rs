//! The brain glue's pure decisions: her card, where a reply goes, which of
//! her actions wait, run or are refused, the notes a batch leaves her, and
//! when her session restarts. `brain_app` does the I/O around them.

use std::collections::VecDeque;
use std::sync::mpsc::Sender;

use maya_core::brain::session::{Request, Session};
use maya_core::config::{Config, NetworkRole};
use maya_core::interpreter;
use maya_core::model::{Card, Harness, State};
use maya_core::window::Via;
use serde_json::Value;

/// Her card on the board: never awaiting or completed, so it never
/// announces; Working while a turn or a subagent of hers runs.
pub(crate) fn brain_card(session_id: &str, agent: Harness, cwd: &str, working: bool, job: Option<&str>, last_say: &str, since: u64) -> Card {
    let snippet = if working { job.map_or_else(|| "Working".to_string(), |j| format!("Working on: {j}")) } else { last_say.to_string() };
    Card {
        session_id: session_id.into(),
        pid: 0,
        name: "Maya".into(),
        cwd: cwd.into(),
        state: if working { State::Working } else { State::Idle },
        state_since: since,
        snippet: snippet.chars().take(200).collect(),
        awaiting: None,
        has_inbox: false,
        harness: agent,
        pr: None,
        context: None,
        machine: None,
        machine_address: None,
        machine_platform: None,
        terminal: None,
        stale: false,
        model: None,
        branch: None,
        brain: true,
    }
}

/// What an exited brain gives back: the request it was on, then the queue.
pub(crate) fn handoff(s: &mut Session) -> Vec<Request> {
    s.drain()
}

/// The line to say or post, trimmed; None when there is nothing to say.
pub(crate) fn spoken(text: &str) -> Option<String> {
    Some(text.trim()).filter(|t| !t.is_empty()).map(str::to_string)
}

/// Whether her line goes through the voice path: a reply to a spoken request
/// always does (the path itself only shows it when muted); her own lines
/// only when Maya is not muted; replies to typed and phone requests never.
pub(crate) fn speaks(via: Via, muted: bool) -> bool {
    match via {
        Via::Voice => true,
        Via::Maya => !muted,
        Via::Chat | Via::Phone => false,
    }
}

/// Her actions after validation.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Sorted {
    /// Wait in the window (`interpreter::waits`).
    pub announce: Vec<Value>,
    /// Run at once: focus, compact, open.
    pub run_now: Vec<Value>,
    /// `watch` actions, checked by `Watches::add`.
    pub watch: Vec<Value>,
    /// Watch ids to remove.
    pub unwatch: Vec<u64>,
    /// Why each refused action was refused, in order.
    pub refused: Vec<String>,
}

/// Validates each of her actions with `validate` (watch and unwatch skip it)
/// and sorts them by how they run. A report needs nothing.
pub(crate) fn sort_actions(actions: &[Value], validate: impl Fn(&Value) -> Result<Value, String>) -> Sorted {
    let mut s = Sorted::default();
    for a in actions {
        match a["kind"].as_str().unwrap_or("") {
            "report" => {}
            "watch" => s.watch.push(a.clone()),
            "unwatch" => match a["id"].as_u64() {
                Some(id) => s.unwatch.push(id),
                None => s.refused.push("unwatch needs the watch's number".into()),
            },
            _ => match validate(a) {
                Ok(v) if interpreter::waits(&v) => s.announce.push(v),
                Ok(v) => s.run_now.push(v),
                Err(why) => s.refused.push(why),
            },
        }
    }
    s
}

/// What Maya says to announce validated actions, built from the actions alone.
pub(crate) fn announcement(actions: &[Value]) -> String {
    actions.iter().map(|a| crate::listener::spoken_for(a).unwrap_or_else(|| "Running that.".into())).collect::<Vec<_>>().join(" ")
}

/// Refused actions go back to her this many times in a row; then the user is told.
pub(crate) const MAX_REFUSAL_ROUNDS: u32 = 2;

#[derive(Debug, PartialEq)]
pub(crate) enum Feedback {
    /// Sent to her as her next message.
    Ask(String),
    /// Posted to Chat: she kept getting it wrong.
    GiveUp(String),
}

/// What to do about refused actions after `streak` rounds of refusals in a row.
pub(crate) fn refusal_feedback(refused: &[String], streak: u32) -> Option<Feedback> {
    if refused.is_empty() {
        return None;
    }
    let list = refused.join("; ");
    Some(if streak < MAX_REFUSAL_ROUNDS {
        Feedback::Ask(format!("<Maya> These actions were refused: {list}. Fix them, or tell the user why you can't."))
    } else {
        Feedback::GiveUp(format!("I couldn't get that right: {list}."))
    })
}

/// How soon after its spawn a resumed session must end to count as a failed resume.
pub(crate) const RESUME_GRACE_MS: u64 = 15_000;

/// A resumed session that ends within `RESUME_GRACE_MS` of its spawn and
/// before its first turn did not resume; a later exit is an ordinary crash.
pub(crate) fn failed_resume(resumed: bool, turn_seen: bool, ran_ms: u64) -> bool {
    resumed && !turn_seen && ran_ms < RESUME_GRACE_MS
}

/// Why her session starts fresh whatever the saved record says: this
/// machine just became the main (from an assistant), and her old session
/// files belong to the machine that was main.
pub(crate) fn forced_fresh(before: NetworkRole, after: NetworkRole) -> Option<&'static str> {
    (before == NetworkRole::Assistant && after == NetworkRole::Main).then_some("This machine is now main, so I've started fresh.")
}

/// Her session runs on every machine but an assistant.
pub(crate) fn runs_brain(c: &Config) -> bool {
    c.network.role != NetworkRole::Assistant
}

/// A settings change that starts, stops or replaces her session: the role
/// moving to or from assistant, or a new brain or model where she runs.
pub(crate) fn needs_restart(before: &Config, after: &Config) -> bool {
    runs_brain(before) != runs_brain(after) || (runs_brain(after) && (before.brain() != after.brain() || before.brain_model() != after.brain_model()))
}

/// Whether `agent`'s resident brain hands long jobs to background subagents
/// (the constraints' list); the others get the busy queue.
pub(crate) fn background_subagents(agent: Harness) -> bool {
    matches!(agent, Harness::ClaudeCode | Harness::Grok | Harness::OpenCode | Harness::Antigravity)
}

/// One note per action of a batch that ran, for her next message.
pub(crate) fn batch_notes(results: &[(Value, Result<String, String>)]) -> Vec<String> {
    results
        .iter()
        .map(|(a, r)| {
            let what = crate::listener::spoken_for(a).unwrap_or_else(|| a["kind"].as_str().unwrap_or("action").to_string());
            match r {
                Ok(_) => format!("Done: {what}"),
                Err(e) => format!("Failed: {what} ({e})"),
            }
        })
        .collect()
}

/// How full her context is, in percent, from a turn's token count.
pub(crate) fn context_percent(model: Option<&str>, tokens: Option<u64>, default_1m: bool) -> Option<u8> {
    tokens.map(|t| maya_core::context::usage(model.unwrap_or(""), t, default_1m).percent)
}

/// Her reply to a phone: what she said and her validated actions.
pub(crate) type PhoneReply = (String, Vec<Value>);

/// The phones waiting for her replies, one entry per phone request in the
/// order they were asked; the session answers them in that order. An entry
/// whose phone stopped waiting keeps its place, so the replies behind it
/// still reach their own phones.
#[derive(Default)]
pub(crate) struct PhoneWaiters {
    entries: VecDeque<(u64, Option<Sender<PhoneReply>>)>,
    next: u64,
}

impl PhoneWaiters {
    pub(crate) fn push(&mut self, waiter: Option<Sender<PhoneReply>>) -> u64 {
        self.next += 1;
        self.entries.push_back((self.next, waiter));
        self.next
    }

    /// The phone of request `id` stopped waiting: its reply goes to Chat only.
    pub(crate) fn give_up(&mut self, id: u64) {
        let _ = self.take(id);
    }

    /// Takes request `id`'s waiter, leaving its place in line.
    pub(crate) fn take(&mut self, id: u64) -> Option<Sender<PhoneReply>> {
        self.entries.iter_mut().find(|(i, _)| *i == id).and_then(|(_, w)| w.take())
    }

    /// The oldest request's waiter, if it still waits; its place is gone.
    pub(crate) fn pop(&mut self) -> Option<Sender<PhoneReply>> {
        self.entries.pop_front().and_then(|(_, w)| w)
    }

    /// Every waiter still waiting, oldest first, and the line emptied.
    pub(crate) fn drain(&mut self) -> Vec<Option<Sender<PhoneReply>>> {
        self.entries.drain(..).map(|(_, w)| w).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maya_core::brain::session::{Request, Session};
    use maya_core::config::{Config, NetworkRole};
    use maya_core::model::{Harness, State};
    use maya_core::window::Via;
    use serde_json::json;

    #[test]
    fn a_blank_reply_is_neither_said_nor_posted() {
        assert_eq!(spoken("  \n "), None);
        assert_eq!(spoken(" Done. \n"), Some("Done.".to_string()));
    }

    #[test]
    fn a_spoken_request_is_answered_aloud_her_own_lines_only_when_not_muted_and_typed_ones_never() {
        assert!(speaks(Via::Voice, false));
        assert!(speaks(Via::Voice, true), "the voice path shows a muted reply in the panel");
        assert!(speaks(Via::Maya, false));
        assert!(!speaks(Via::Maya, true));
        for via in [Via::Chat, Via::Phone] {
            assert!(!speaks(via, false), "{via:?}");
        }
    }

    fn check(a: &serde_json::Value) -> Result<serde_json::Value, String> {
        match a["session"].as_str() {
            Some("ghost") => Err("I couldn't find a session called ghost".into()),
            _ => {
                let mut v = a.clone();
                v["name"] = json!("api-fix");
                Ok(v)
            }
        }
    }

    #[test]
    fn her_actions_are_validated_and_sorted_into_waiting_running_watches_and_refused() {
        let actions = [
            json!({"kind":"reply","session":"api-fix","text":"open the PR"}),
            json!({"kind":"focus","session":"api-fix"}),
            json!({"kind":"report"}),
            json!({"kind":"watch","session":"api-fix","note":"n"}),
            json!({"kind":"unwatch","id":3}),
            json!({"kind":"unwatch","id":"x"}),
            json!({"kind":"reply","session":"ghost","text":"hi"}),
        ];
        let s = sort_actions(&actions, check);
        assert_eq!(s.announce, vec![json!({"kind":"reply","session":"api-fix","text":"open the PR","name":"api-fix"})]);
        assert_eq!(s.run_now, vec![json!({"kind":"focus","session":"api-fix","name":"api-fix"})]);
        assert_eq!(s.watch, vec![json!({"kind":"watch","session":"api-fix","note":"n"})]);
        assert_eq!(s.unwatch, vec![3]);
        assert_eq!(s.refused, vec!["unwatch needs the watch's number".to_string(), "I couldn't find a session called ghost".to_string()]);
    }

    #[test]
    fn the_announcement_is_built_from_the_validated_actions() {
        let a = [json!({"kind":"reply","name":"api-fix","text":"open the PR"}), json!({"kind":"compact","name":"docs"})];
        assert_eq!(announcement(&a), "Telling api-fix: open the PR. Compacting docs.");
    }

    #[test]
    fn refusals_go_back_to_her_twice_then_to_the_user() {
        assert_eq!(refusal_feedback(&[], 0), None);
        let r = vec!["a".to_string(), "b".to_string()];
        assert_eq!(refusal_feedback(&r, 0), Some(Feedback::Ask("<Maya> These actions were refused: a; b. Fix them, or tell the user why you can't.".into())));
        assert!(matches!(refusal_feedback(&r, MAX_REFUSAL_ROUNDS - 1), Some(Feedback::Ask(_))));
        assert_eq!(refusal_feedback(&r, MAX_REFUSAL_ROUNDS), Some(Feedback::GiveUp("I couldn't get that right: a; b.".into())));
    }

    #[test]
    fn a_resume_that_dies_soon_and_before_its_first_turn_failed() {
        assert!(failed_resume(true, false, 0));
        assert!(failed_resume(true, false, RESUME_GRACE_MS - 1));
        assert!(!failed_resume(true, false, RESUME_GRACE_MS), "a later exit is a crash: resume again");
        assert!(!failed_resume(true, true, 1_000));
        assert!(!failed_resume(false, false, 1_000), "a fresh session that dies is a plain exit");
    }

    #[test]
    fn becoming_main_from_assistant_starts_fresh_and_says_so() {
        assert_eq!(forced_fresh(NetworkRole::Assistant, NetworkRole::Main), Some("This machine is now main, so I've started fresh."));
        assert_eq!(forced_fresh(NetworkRole::Off, NetworkRole::Main), None);
        assert_eq!(forced_fresh(NetworkRole::Main, NetworkRole::Main), None);
        assert_eq!(forced_fresh(NetworkRole::Assistant, NetworkRole::Off), None, "only the move into main");
        assert_eq!(forced_fresh(NetworkRole::Main, NetworkRole::Assistant), None);
    }

    #[test]
    fn her_session_runs_off_assistants_and_restarts_on_a_brain_model_or_role_change() {
        let main = Config { agent: Some(Harness::ClaudeCode), ..Default::default() };
        assert!(runs_brain(&main));
        let mut assistant = main.clone();
        assistant.network.role = NetworkRole::Assistant;
        assert!(!runs_brain(&assistant));
        assert!(needs_restart(&main, &assistant));
        assert!(needs_restart(&assistant, &main));
        let mut promoted = main.clone();
        promoted.network.role = NetworkRole::Main;
        assert!(!needs_restart(&main, &promoted), "off and main both run her");
        assert!(needs_restart(&main, &Config { agent: Some(Harness::Grok), ..main.clone() }));
        assert!(needs_restart(&main, &Config { agent_model: "sonnet".into(), ..main.clone() }));
        assert!(!needs_restart(&main, &Config { muted: true, ..main.clone() }));
        assert!(!needs_restart(&assistant, &Config { agent: Some(Harness::Grok), ..assistant.clone() }), "an assistant never runs her");
    }

    #[test]
    fn a_batch_leaves_her_a_done_or_failed_note_per_action() {
        let results = vec![
            (json!({"kind":"reply","name":"api-fix","text":"open the PR"}), Ok("Sent.".to_string())),
            (json!({"kind":"start","dir":"web","prompt":"fix it"}), Err("Session is no longer running.".to_string())),
        ];
        assert_eq!(batch_notes(&results), vec!["Done: Telling api-fix: open the PR.".to_string(), "Failed: Starting a session in web: fix it. (Session is no longer running.)".to_string()]);
    }

    #[test]
    fn context_percent_uses_the_brains_model_window() {
        assert_eq!(context_percent(None, None, false), None);
        assert_eq!(context_percent(None, Some(100_000), false), Some(50));
        assert_eq!(context_percent(Some("opus[1m]"), Some(100_000), false), Some(10));
        assert_eq!(context_percent(None, Some(100_000), true), Some(10));
    }

    #[test]
    fn phone_replies_reach_their_waiters_in_order_and_one_that_gave_up_gets_nothing() {
        let mut w = PhoneWaiters::default();
        let (tx1, rx1) = std::sync::mpsc::channel();
        let (tx2, rx2) = std::sync::mpsc::channel();
        let a = w.push(Some(tx1));
        let _b = w.push(Some(tx2));
        w.give_up(a);
        assert!(w.pop().is_none(), "the first request's waiter gave up");
        w.pop().unwrap().send(("two".into(), vec![])).unwrap();
        assert!(rx1.try_recv().is_err());
        assert_eq!(rx2.try_recv().unwrap().0, "two");
        assert!(w.pop().is_none());
    }

    #[test]
    fn a_busy_answer_takes_the_waiter_but_keeps_the_place_in_line() {
        let mut w = PhoneWaiters::default();
        let (tx, _rx) = std::sync::mpsc::channel();
        let a = w.push(Some(tx));
        let (tx2, _rx2) = std::sync::mpsc::channel();
        w.push(Some(tx2));
        assert!(w.take(a).is_some());
        assert!(w.pop().is_none(), "the busy request's reply goes to Chat only");
        assert!(w.pop().is_some());
        assert_eq!(w.drain().len(), 0);
    }

    #[test]
    fn handoff_is_the_in_flight_request_then_the_queue() {
        let mut s = Session::new(true);
        s.submit(Request { text: "a".into(), via: Via::Phone });
        s.submit(Request { text: "b".into(), via: Via::Chat });
        assert_eq!(handoff(&mut s).into_iter().map(|r| r.via).collect::<Vec<_>>(), vec![Via::Phone, Via::Chat]);
    }

    #[test]
    fn a_working_card_shows_her_job_and_an_idle_one_her_last_line() {
        let w = brain_card("u", Harness::ClaudeCode, "/x", true, Some("refactor"), "old", 7);
        assert_eq!((w.snippet.as_str(), w.state_since, w.session_id.as_str(), w.cwd.as_str()), ("Working on: refactor", 7, "u", "/x"));
        let i = brain_card("u", Harness::ClaudeCode, "/x", false, None, &"x".repeat(300), 7);
        assert_eq!(i.snippet.chars().count(), 200);
        assert_eq!(i.state, State::Idle);
    }
}
