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
        shell: false,
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

/// The parts of her reply as one line to say or post, blank parts left
/// out: said as one utterance, a short part ("Sure.") never plays alone.
pub(crate) fn one_line<S: AsRef<str>>(parts: &[S]) -> String {
    parts.iter().map(|p| p.as_ref().trim()).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ")
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

/// A resumed session that ends before its first turn, and within
/// `RESUME_GRACE_MS` of its spawn or of the first message sent to it (a bad
/// resume may fail only once that message arrives), did not resume; any
/// other exit is an ordinary crash.
pub(crate) fn failed_resume(resumed: bool, turn_seen: bool, ran_ms: u64, since_first_message_ms: Option<u64>) -> bool {
    resumed && !turn_seen && (ran_ms < RESUME_GRACE_MS || since_first_message_ms.is_some_and(|m| m < RESUME_GRACE_MS))
}

/// The id to save her record with when her agent names her session
/// (`Ready`): only once her first turn has saved one, and only when it
/// names another (Antigravity relaunched on a conversation it could not
/// open, or afresh after a stop before it named one). Before her first
/// turn, that turn saves it.
pub(crate) fn resave_on_ready(turn_seen: bool, saved: Option<&str>, named: &str) -> Option<String> {
    (turn_seen && saved != Some(named)).then(|| named.to_string())
}

/// How long after her session's `n`th exit in a row (from 0) it starts again.
const RESTART_DELAYS: [u64; 4] = [30, 60, 120, 300];

/// Her session stopping on its own, again and again: each restart waits
/// longer (30 s, 60 s, 120 s, then 300 s), and Chat hears about it once,
/// until a turn ends.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Outage {
    exits: u32,
    said: bool,
}

impl Outage {
    /// Her session exited: how long to wait before starting it again, and
    /// whether to post "my session stopped".
    pub(crate) fn exited(&mut self) -> (std::time::Duration, bool) {
        let secs = RESTART_DELAYS[(self.exits as usize).min(RESTART_DELAYS.len() - 1)];
        self.exits += 1;
        (std::time::Duration::from_secs(secs), self.say_once())
    }

    /// Whether to post that her session stopped: once per outage.
    pub(crate) fn say_once(&mut self) -> bool {
        !std::mem::replace(&mut self.said, true)
    }

    /// A turn ended: she works again, and the outage is over.
    pub(crate) fn turn_ended(&mut self) {
        *self = Outage::default();
    }
}

/// What she says to "stop": "Cancelled." for a batch that was waiting in
/// the window, "Stopped." for her own work, both, or "Nothing to stop.".
pub(crate) fn stop_line(cancelled_batch: bool, stopped_work: bool) -> &'static str {
    match (cancelled_batch, stopped_work) {
        (true, true) => "Cancelled and stopped.",
        (true, false) => "Cancelled.",
        (false, true) => "Stopped.",
        (false, false) => "Nothing to stop.",
    }
}

/// Whether a reply may announce or run its actions: only when no stop came
/// since its turn began (`turn_epoch` is the stop count then).
pub(crate) fn may_act(turn_epoch: u64, stop_epoch: u64) -> bool {
    turn_epoch == stop_epoch
}

/// How long an interrupted turn has to end.
pub(crate) const INTERRUPT_GRACE_MS: u64 = 10_000;
/// How long a turn may go without a line from her process.
pub(crate) const SILENT_TURN_MS: u64 = 600_000;

/// When she was last heard from: the later of her last parsed event (or
/// the last message sent) and the last line her process wrote (0 for a
/// driver with no output clock). A long turn that keeps writing lines no
/// event comes from is alive.
pub(crate) fn heard_at(last_event: u64, last_output: u64) -> u64 {
    last_event.max(last_output)
}

/// Why her session must be restarted now, if it must: her turn did not end
/// within `INTERRUPT_GRACE_MS` of an interrupt, or her process said nothing
/// for `SILENT_TURN_MS` while a turn ran (`last_heard` is the later of her
/// last line and the last message sent). Either way voice would go dead.
pub(crate) fn stalled(now: u64, in_turn: bool, last_heard: u64, interrupted_at: Option<u64>) -> Option<&'static str> {
    if !in_turn {
        return None;
    }
    if interrupted_at.is_some_and(|t| now.saturating_sub(t) >= INTERRUPT_GRACE_MS) {
        return Some("her turn did not stop");
    }
    (now.saturating_sub(last_heard) >= SILENT_TURN_MS).then_some("her turn went silent")
}

/// Maya's follow-up to one of her replies (its refused actions): internal,
/// and carrying that reply's via, so her answer goes back the same way.
pub(crate) fn follow_up(via: Via, text: &str) -> Request {
    Request::internal(text, via)
}

/// What a role change does to her brain, described for the log. Every Maya
/// keeps her own: on an assistant it sleeps, and nothing of it is reset.
/// The choices themselves are made elsewhere: `runs_brain` (dormant or
/// not, in `brain_app::start_run`) and `record::decide` (resume or fresh,
/// which also starts fresh after a brain or model change).
#[derive(Debug, PartialEq)]
pub(crate) enum Wake {
    /// This machine became an assistant: her session stops, and her record,
    /// chat and watches stay, the record marked dormant.
    Dormant,
    /// It stopped being an assistant: she resumes her own record; with
    /// `catch_up`, her first turn is the old main's summary.
    Resume { catch_up: bool },
    /// The same with no record: she starts fresh, primed with her Chat.
    Fresh { catch_up: bool },
    /// The role change does not touch her session.
    Unchanged,
}

/// What a move from `before` to `after` does to her brain, for the log;
/// `has_record`: she ran here before; `summary`: the old main sent one.
pub(crate) fn wake(before: NetworkRole, after: NetworkRole, has_record: bool, summary: bool) -> Wake {
    match (before == NetworkRole::Assistant, after == NetworkRole::Assistant) {
        (false, true) => Wake::Dormant,
        (true, false) if has_record => Wake::Resume { catch_up: summary },
        (true, false) => Wake::Fresh { catch_up: summary },
        _ => Wake::Unchanged,
    }
}

/// Where the handover summary for the new main comes from.
#[derive(Debug, PartialEq)]
pub(crate) enum SummarySource {
    /// Her brain writes it (bounded by `handover::ASK_MS`).
    Ask,
    /// A plain digest of her Chat: her brain is not running.
    Digest,
}

pub(crate) fn summary_source(brain_ready: bool) -> SummarySource {
    if brain_ready {
        SummarySource::Ask
    } else {
        SummarySource::Digest
    }
}

/// Her written summary, or the digest when she gave none (timeout, error,
/// a blank reply).
pub(crate) fn pick_summary(asked: Option<String>, digest: impl FnOnce() -> String) -> String {
    asked.map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).unwrap_or_else(digest)
}

/// Her first message: the primer of a fresh session, then the catch-up.
pub(crate) fn first_turn(primer: Option<String>, catch_up: Option<String>) -> Option<String> {
    match (primer, catch_up) {
        (Some(p), Some(c)) => Some(format!("{p}\n{c}")),
        (p, c) => p.or(c),
    }
}

/// Her handover summary being written: the request, who waits for its
/// reply (none once they stopped waiting), and whether her turn on it runs.
pub(crate) struct SummaryWait {
    text: String,
    tx: Option<Sender<String>>,
    in_turn: bool,
}

impl SummaryWait {
    pub(crate) fn new(text: &str, tx: Sender<String>) -> SummaryWait {
        SummaryWait { text: text.to_string(), tx: Some(tx), in_turn: false }
    }

    /// `req` is going to her: her turn on the summary runs when it is the
    /// summary request.
    pub(crate) fn delivered(&mut self, req: &Request) {
        if req.internal && req.via == Via::Maya && req.text == self.text {
            self.in_turn = true;
        }
    }

    /// Whether a reply that came back `via` is the summary.
    pub(crate) fn takes(&self, via: Via) -> bool {
        via == Via::Maya && self.in_turn
    }

    /// The summary came: to whoever still waits.
    pub(crate) fn answer(self, say: &str) {
        if let Some(tx) = self.tx {
            let _ = tx.send(say.to_string());
        }
    }

    /// Nobody waits any more; a late reply is still swallowed.
    pub(crate) fn gave_up(&mut self) {
        self.tx = None;
    }
}

/// The summary wait after "stop": a stop ends her turn with no reply and
/// clears her queue, so the summary will not come. The wait goes (its
/// waiter hears at once and uses the digest), and no later reply of hers
/// is taken for it.
pub(crate) fn summary_after_stop(_wait: Option<SummaryWait>) -> Option<SummaryWait> {
    None
}

/// The note the read-only Chat on an assistant shows.
pub(crate) fn dormant_note(main: Option<&str>) -> String {
    match main.map(str::trim).filter(|m| !m.is_empty()) {
        Some(m) => format!("Dormant: {m}'s Maya is awake."),
        None => "Dormant: the main Maya is awake.".into(),
    }
}

/// The quiet line the old main's Chat gets once it hands the role over.
pub(crate) fn dormant_line(main: &str) -> String {
    format!("I'm dormant now; {main} is awake.")
}

/// The new main's Chat line with the old main's summary.
pub(crate) fn caught_up_line(by: &str, summary: &str) -> String {
    format!("Caught up from {by}: {summary}")
}

/// What `chat_send` says on an assistant, where her brain is dormant.
pub(crate) const DORMANT_CHAT: &str = "Maya's brain is dormant on this machine; talk to the main Maya.";

pub(crate) fn chat_refusal(c: &Config) -> Option<&'static str> {
    (!runs_brain(c)).then_some(DORMANT_CHAT)
}

/// Her card while she is dormant (Debug mode shows it): greyed, idle.
pub(crate) fn dormant_card(session_id: &str, agent: Harness, cwd: &str, since: u64) -> Card {
    Card { stale: true, ..brain_card(session_id, agent, cwd, false, None, "Dormant", since) }
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

/// What a phone hears when her reply takes longer than it waits.
pub(crate) const PHONE_STILL_WORKING: &str = "I'm still working on that; the answer will be in Chat on the computer.";

/// The note her actions for a phone leave when no phone took them: they did
/// not run, and why. `gave_up`: the phone stopped waiting; otherwise it was
/// never waiting for this reply (it got a busy line, or this answers one of
/// Maya's follow-ups). None when there were no actions.
pub(crate) fn phone_dropped_note(gave_up: bool, actions: &[Value]) -> Option<String> {
    if actions.is_empty() {
        return None;
    }
    let why = if gave_up { "The phone stopped waiting" } else { "The phone was not waiting for this reply" };
    Some(format!("{why}, so these did not run: {}", announcement(actions)))
}

/// One phone request's place in line.
#[derive(Debug)]
pub(crate) enum Waiter {
    /// The phone waits for her reply.
    Waiting(Sender<PhoneReply>),
    /// The phone stopped waiting: her reply came too late.
    GaveUp,
    /// No phone waits: it already got a busy line, or the request is one of
    /// Maya's follow-ups.
    NotWaiting,
}

/// The phones waiting for her replies, one entry per phone request in the
/// order they were asked; the session answers them in that order. An entry
/// whose phone stopped waiting keeps its place, so the replies behind it
/// still reach their own phones.
#[derive(Default)]
pub(crate) struct PhoneWaiters {
    entries: VecDeque<(u64, Waiter)>,
    next: u64,
}

impl PhoneWaiters {
    pub(crate) fn push(&mut self, waiter: Option<Sender<PhoneReply>>) -> u64 {
        self.next += 1;
        self.entries.push_back((self.next, waiter.map_or(Waiter::NotWaiting, Waiter::Waiting)));
        self.next
    }

    /// The phone of request `id` stopped waiting: its reply goes to Chat only.
    pub(crate) fn give_up(&mut self, id: u64) {
        if let Some((_, w)) = self.entries.iter_mut().find(|(i, w)| *i == id && matches!(w, Waiter::Waiting(_))) {
            *w = Waiter::GaveUp;
        }
    }

    /// Takes request `id`'s waiter to answer it now, leaving its place in line.
    pub(crate) fn take(&mut self, id: u64) -> Option<Sender<PhoneReply>> {
        let (_, w) = self.entries.iter_mut().find(|(i, _)| *i == id)?;
        match std::mem::replace(w, Waiter::NotWaiting) {
            Waiter::Waiting(tx) => Some(tx),
            other => {
                *w = other;
                None
            }
        }
    }

    /// The oldest request's place, which is gone after this.
    pub(crate) fn pop(&mut self) -> Waiter {
        self.entries.pop_front().map_or(Waiter::NotWaiting, |(_, w)| w)
    }

    /// Every waiter still waiting, oldest first, and the line emptied.
    pub(crate) fn drain(&mut self) -> Vec<Option<Sender<PhoneReply>>> {
        self.entries
            .drain(..)
            .map(|(_, w)| match w {
                Waiter::Waiting(tx) => Some(tx),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_renamed_after_her_first_turn_is_saved_again() {
        assert_eq!(resave_on_ready(false, None, "c-1"), None, "her first turn saves it");
        assert_eq!(resave_on_ready(false, Some("c-0"), "c-1"), None, "her first turn saves it");
        assert_eq!(resave_on_ready(true, Some("c-1"), "c-1"), None, "the same session: nothing new");
        assert_eq!(resave_on_ready(true, Some("c-1"), "c-2").as_deref(), Some("c-2"));
        assert_eq!(resave_on_ready(true, None, "c-2").as_deref(), Some("c-2"));
    }
    use maya_core::brain::session::{Request, Session};
    use maya_core::config::{Config, NetworkRole};
    use maya_core::model::{Harness, State};
    use maya_core::window::Via;
    use serde_json::json;

    #[test]
    fn her_words_and_what_ran_make_one_line() {
        assert_eq!(one_line(&["Sure.", "", " Focusing api-fix. "]), "Sure. Focusing api-fix.");
        assert_eq!(one_line(&["  ", ""]), "");
        assert_eq!(one_line(&["", "Telling api-fix: open the PR."]), "Telling api-fix: open the PR.");
    }

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
    fn a_resume_that_dies_soon_after_its_spawn_or_its_first_message_and_before_its_first_turn_failed() {
        let g = RESUME_GRACE_MS;
        assert!(failed_resume(true, false, 0, None));
        assert!(failed_resume(true, false, g - 1, None));
        assert!(!failed_resume(true, false, g, None), "a later exit with nothing sent is a crash: resume again");
        assert!(failed_resume(true, false, 10 * g, Some(g - 1)), "a bad resume may fail only once the first message arrives");
        assert!(!failed_resume(true, false, 10 * g, Some(g)));
        assert!(!failed_resume(true, true, 1_000, Some(1_000)), "a turn ended: it resumed");
        assert!(!failed_resume(false, false, 1_000, Some(1_000)), "a fresh session that dies is a plain exit");
    }

    #[test]
    fn restarts_back_off_to_five_minutes_and_an_outage_is_reported_once() {
        let mut o = Outage::default();
        let secs = |d: std::time::Duration| d.as_secs();
        let (d, say) = o.exited();
        assert_eq!((secs(d), say), (30, true));
        let (d, say) = o.exited();
        assert_eq!((secs(d), say), (60, false), "one line per outage");
        assert_eq!(secs(o.exited().0), 120);
        assert_eq!(secs(o.exited().0), 300);
        assert_eq!(secs(o.exited().0), 300, "capped");
        assert!(!o.say_once());
        o.turn_ended();
        assert_eq!((secs(o.exited().0), true), (30, true));
        o.turn_ended();
        assert!(o.say_once(), "a stall reports a new outage");
        assert!(!o.say_once());
        assert_eq!(o.exited(), (std::time::Duration::from_secs(30), false), "the stall's outage was reported already");
    }

    #[test]
    fn a_turn_that_ignores_a_stop_or_goes_silent_restarts_her_session() {
        let t = 1_000_000;
        assert_eq!(stalled(t, false, 0, Some(0)), None, "no turn runs: nothing to wait for");
        assert_eq!(stalled(t, true, t, Some(t - INTERRUPT_GRACE_MS + 1)), None);
        assert_eq!(stalled(t, true, t, Some(t - INTERRUPT_GRACE_MS)), Some("her turn did not stop"));
        assert_eq!(stalled(t, true, t - SILENT_TURN_MS + 1, None), None);
        assert_eq!(stalled(t, true, t - SILENT_TURN_MS, None), Some("her turn went silent"));
        assert_eq!(stalled(5, true, 10, Some(10)), None, "a clock that went back is not a stall");
    }

    #[test]
    fn a_long_turn_that_keeps_writing_is_not_silent() {
        let t = 10_000_000;
        // No parsed event for longer than the limit, but her process wrote a line a second ago.
        let heard = heard_at(t - SILENT_TURN_MS - 1, t - 1_000);
        assert_eq!(heard, t - 1_000);
        assert_eq!(stalled(t, true, heard, None), None);
        assert_eq!(heard_at(t - 5, 0), t - 5, "a driver with no output clock");
        assert_eq!(stalled(t, true, heard_at(t - SILENT_TURN_MS, t - SILENT_TURN_MS - 1), None), Some("her turn went silent"));
    }

    #[test]
    fn a_stop_says_what_it_stopped_and_nothing_to_stop_only_when_nothing_was() {
        assert_eq!(stop_line(true, false), "Cancelled.");
        assert_eq!(stop_line(false, true), "Stopped.");
        assert_eq!(stop_line(true, true), "Cancelled and stopped.");
        assert_eq!(stop_line(false, false), "Nothing to stop.");
    }

    #[test]
    fn a_reply_whose_turn_began_before_the_latest_stop_runs_nothing() {
        assert!(may_act(3, 3));
        assert!(!may_act(2, 3));
    }

    #[test]
    fn follow_ups_go_back_internally_the_way_the_request_came() {
        assert_eq!(follow_up(Via::Chat, "m"), Request::internal("m", Via::Chat));
        assert_eq!(follow_up(Via::Phone, "m").via, Via::Phone);
        assert!(follow_up(Via::Maya, "m").internal);
    }

    #[test]
    fn the_logged_wake_matches_what_runs_brain_and_record_decide_do() {
        use NetworkRole::*;
        // Dormant exactly where `runs_brain` says she does not run.
        for (before, after) in [(Main, Assistant), (Off, Assistant), (Assistant, Main), (Assistant, Off)] {
            let c = Config { network: maya_core::config::NetworkConfig { role: after, ..Default::default() }, ..Default::default() };
            assert_eq!(wake(before, after, true, false) == Wake::Dormant, !runs_brain(&c), "{before:?} -> {after:?}");
        }
        assert_eq!(wake(Main, Assistant, true, false), Wake::Dormant);
        assert_eq!(wake(Off, Assistant, false, false), Wake::Dormant);
        assert_eq!(wake(Assistant, Main, true, false), Wake::Resume { catch_up: false }, "her own record, not fresh");
        assert_eq!(wake(Assistant, Main, true, true), Wake::Resume { catch_up: true });
        assert_eq!(wake(Assistant, Main, false, true), Wake::Fresh { catch_up: true }, "never ran a brain: fresh");
        assert_eq!(wake(Assistant, Off, false, false), Wake::Fresh { catch_up: false });
        assert_eq!(wake(Main, Main, true, false), Wake::Unchanged);
        assert_eq!(wake(Off, Main, true, false), Wake::Unchanged);
        assert_eq!(wake(Assistant, Assistant, true, false), Wake::Unchanged);
    }

    #[test]
    fn the_summary_comes_from_her_brain_when_ready_else_a_digest() {
        assert_eq!(summary_source(true), SummarySource::Ask);
        assert_eq!(summary_source(false), SummarySource::Digest);
    }

    #[test]
    fn a_blank_or_missing_brain_summary_falls_back_to_the_digest() {
        assert_eq!(pick_summary(Some(" Did #52. ".into()), || "digest".into()), "Did #52.");
        assert_eq!(pick_summary(Some("  ".into()), || "digest".into()), "digest");
        assert_eq!(pick_summary(None, || "digest".into()), "digest");
    }

    #[test]
    fn a_catch_up_follows_the_primer_or_is_her_first_turn_alone() {
        assert_eq!(first_turn(None, None), None);
        assert_eq!(first_turn(Some("P".into()), None).as_deref(), Some("P"));
        assert_eq!(first_turn(None, Some("C".into())).as_deref(), Some("C"));
        assert_eq!(first_turn(Some("P".into()), Some("C".into())).as_deref(), Some("P\nC"));
    }

    #[test]
    fn only_her_turn_on_the_summary_request_answers_it() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut w = SummaryWait::new("Write a summary", tx);
        assert!(!w.takes(Via::Maya), "not sent yet: a reply of hers is not the summary");
        w.delivered(&Request::new("Write a summary", Via::Maya));
        w.delivered(&Request::internal("other", Via::Maya));
        assert!(!w.takes(Via::Maya), "only the internal summary request");
        w.delivered(&Request::internal("Write a summary", Via::Maya));
        assert!(w.takes(Via::Maya));
        assert!(!w.takes(Via::Chat));
        w.answer("Did #52.");
        assert_eq!(rx.try_recv().unwrap(), "Did #52.");
    }

    #[test]
    fn a_stop_during_the_summary_ends_the_wait_at_once_and_takes_no_later_reply() {
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let mut w = SummaryWait::new("Write a summary", tx);
        w.delivered(&Request::internal("Write a summary", Via::Maya));
        let after = summary_after_stop(Some(w));
        assert!(after.is_none(), "no later Maya reply is taken for the summary");
        assert!(matches!(rx.try_recv(), Err(std::sync::mpsc::TryRecvError::Disconnected)), "the waiter goes to the digest at once");
    }

    #[test]
    fn a_late_summary_is_swallowed_with_nobody_waiting() {
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let mut w = SummaryWait::new("s", tx);
        w.gave_up();
        assert!(matches!(rx.try_recv(), Err(std::sync::mpsc::TryRecvError::Disconnected)));
        w.delivered(&Request::internal("s", Via::Maya));
        assert!(w.takes(Via::Maya), "still taken, so it is never said");
        w.answer("late");
    }

    #[test]
    fn the_dormant_lines_name_the_main_or_say_the_main_maya() {
        assert_eq!(dormant_note(Some("desk")), "Dormant: desk's Maya is awake.");
        assert_eq!(dormant_note(None), "Dormant: the main Maya is awake.");
        assert_eq!(dormant_note(Some(" ")), "Dormant: the main Maya is awake.");
        assert_eq!(dormant_line("laptop"), "I'm dormant now; laptop is awake.");
        assert_eq!(caught_up_line("desk", "Did #52."), "Caught up from desk: Did #52.");
    }

    #[test]
    fn a_dormant_card_is_hers_greyed_idle_and_says_dormant() {
        let c = dormant_card("u", Harness::ClaudeCode, "/x", 7);
        assert!(c.brain && c.stale);
        assert_eq!((c.state, c.snippet.as_str(), c.state_since), (State::Idle, "Dormant", 7));
        assert!(c.awaiting.is_none());
    }

    #[test]
    fn chat_is_refused_on_an_assistant_only() {
        let mut c = Config::default();
        assert_eq!(chat_refusal(&c), None);
        c.network.role = NetworkRole::Assistant;
        assert_eq!(chat_refusal(&c), Some(DORMANT_CHAT));
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
        assert!(matches!(w.pop(), Waiter::GaveUp), "the first request's phone gave up");
        let Waiter::Waiting(tx) = w.pop() else { panic!("the second still waits") };
        tx.send(("two".into(), vec![])).unwrap();
        assert!(rx1.try_recv().is_err());
        assert_eq!(rx2.try_recv().unwrap().0, "two");
        assert!(matches!(w.pop(), Waiter::NotWaiting));
    }

    #[test]
    fn a_busy_answer_takes_the_waiter_but_keeps_the_place_in_line() {
        let mut w = PhoneWaiters::default();
        let (tx, _rx) = std::sync::mpsc::channel();
        let a = w.push(Some(tx));
        let (tx2, _rx2) = std::sync::mpsc::channel();
        w.push(Some(tx2));
        w.push(None);
        assert!(w.take(a).is_some());
        assert!(matches!(w.pop(), Waiter::NotWaiting), "the busy request's reply goes to Chat only, and the phone did not give up");
        assert!(matches!(w.pop(), Waiter::Waiting(_)));
        assert!(matches!(w.pop(), Waiter::NotWaiting), "a follow-up has no phone waiting");
        assert_eq!(w.drain().len(), 0);
    }

    #[test]
    fn a_late_phone_reply_drops_its_actions_with_the_true_reason() {
        let a = [json!({"kind":"reply","name":"api-fix","text":"open the PR"})];
        assert_eq!(phone_dropped_note(true, &a).as_deref(), Some("The phone stopped waiting, so these did not run: Telling api-fix: open the PR."));
        assert_eq!(phone_dropped_note(false, &a).as_deref(), Some("The phone was not waiting for this reply, so these did not run: Telling api-fix: open the PR."));
        assert_eq!(phone_dropped_note(true, &[]), None);
        assert!(PHONE_STILL_WORKING.contains("Chat on the computer"));
    }

    #[test]
    fn handoff_is_the_in_flight_request_then_the_queue() {
        let mut s = Session::new(true);
        s.submit(Request::new("a", Via::Phone));
        s.submit(Request::new("b", Via::Chat));
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
