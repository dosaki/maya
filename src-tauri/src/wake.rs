//! The wake word and the short conversation around a command: "Maya" then
//! a command, or a command in the same breath; a pending action that waits
//! for yes or no. Pure functions and a small state machine, no I/O.

pub const COMMAND_WAIT_MS: u64 = 8_000;
pub const CONFIRM_WAIT_MS: u64 = 10_000;

const WAKE_WORDS: &[&str] = &["maya", "maia", "mya", "my a"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wake {
    None,
    Bare,
    Command(String),
}

fn normalise(s: &str) -> String {
    s.to_lowercase().chars().map(|c| if c.is_alphanumeric() || c == '\'' || c == ' ' { c } else { ' ' }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Lowercases a single token and strips punctuation, without splitting it
/// on internal punctuation (so "hexgrid-one" and "24/7" stay one token).
fn normalise_token(t: &str) -> String {
    t.to_lowercase().chars().filter(|c| c.is_alphanumeric() || *c == '\'').collect()
}

/// The command after the wake word, if the segment contains one.
///
/// Matches the wake word on the original whitespace-split tokens (each
/// normalised on its own) rather than on a globally-normalised string, so a
/// hyphenated or slashed word right after the wake word can't absorb it.
pub fn extract(segment: &str) -> Wake {
    let original: Vec<&str> = segment.split_whitespace().collect();
    let normalised: Vec<String> = original.iter().map(|t| normalise_token(t)).collect();

    for start in 0..normalised.len() {
        for w in WAKE_WORDS {
            let w_tokens: Vec<&str> = w.split_whitespace().collect();
            let end = start + w_tokens.len();
            if end > normalised.len() {
                continue;
            }
            if normalised[start..end].iter().map(String::as_str).eq(w_tokens.iter().copied()) {
                let tail = &original[end..];
                if tail.is_empty() {
                    return Wake::Bare;
                }
                let command = tail.join(" ");
                let command = command.trim_start_matches(|c: char| c == ',' || c == '.' || c == ':').trim().to_string();
                return Wake::Command(command);
            }
        }
    }
    Wake::None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Yes,
    No,
    Other,
}

const FILLER_WORDS: &[&str] = &["please", "thanks", "thank", "you", "maya"];
const NO_WORDS: &[&str] = &["no", "nope", "not", "don't", "cancel", "stop", "abort", "never", "nevermind"];
const YES_PHRASES: &[&str] = &["yes", "yeah", "yep", "yup", "sure", "ok", "okay", "confirm", "do it", "go ahead", "go for it"];

/// Loose yes/no recognition over a short utterance. Declines win over
/// confirmations, and a "yes" must consume the whole (filler-stripped)
/// utterance, so a redirected or hedged reply falls through to `Other`.
pub fn answer(segment: &str) -> Answer {
    let n = normalise(segment);
    let words: Vec<&str> = n.split_whitespace().collect();
    if words.len() > 4 {
        return Answer::Other;
    }
    let remaining: Vec<&str> = words.into_iter().filter(|w| !FILLER_WORDS.contains(w)).collect();
    if remaining.iter().any(|w| NO_WORDS.contains(w)) {
        return Answer::No;
    }
    let mut rest = remaining.join(" ");
    if rest.is_empty() {
        return Answer::Other;
    }
    loop {
        let mut best: Option<&str> = None;
        for &p in YES_PHRASES {
            if rest == p || rest.starts_with(&format!("{p} ")) {
                if best.map_or(true, |b| p.len() > b.len()) {
                    best = Some(p);
                }
            }
        }
        match best {
            Some(p) => rest = rest[p.len()..].trim_start().to_string(),
            None => break,
        }
    }
    if rest.is_empty() {
        Answer::Yes
    } else {
        Answer::Other
    }
}

/// Words that may sit beside a no-word in a plain refusal ("never mind",
/// "not now", "don't do it", "cancel that").
const NO_COMPANIONS: &[&str] = &["mind", "now", "it", "that", "do"];

/// True when a command spoken after the wake word is only a yes or a no
/// ("Maya, yes", "Maya, cancel"), not a new request that happens to contain
/// a no-word ("Maya tell coral stop").
fn plain_answer(command: &str) -> bool {
    match answer(command) {
        Answer::Yes => true,
        Answer::No => normalise(command).split_whitespace().filter(|w| !FILLER_WORDS.contains(w)).all(|w| NO_WORDS.contains(&w) || NO_COMPANIONS.contains(&w)),
        Answer::Other => false,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    pub say: String,
    pub action: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Say(String),
    Interpret(String),
    Execute(serde_json::Value),
    Cancelled,
}

enum State {
    Idle,
    AwaitingCommand { until: u64 },
    AwaitingConfirm { pending: Pending, until: u64 },
}

pub struct Flow {
    state: State,
    ignore: Option<String>,
}

impl Flow {
    pub fn new() -> Flow {
        Flow { state: State::Idle, ignore: None }
    }

    /// Remembers Maya's last spoken line so hearing it back does nothing —
    /// but only when it's long enough not to collide with her own short
    /// prompts (like "Yes?", which would otherwise normalise to "yes" and
    /// swallow the user's real answer). The stored line is one-shot: it is
    /// cleared by the very next `on_segment` call whether or not it matched.
    pub fn ignore_line(&mut self, spoken: &str) {
        let n = normalise(spoken);
        if n.split_whitespace().count() >= 3 {
            self.ignore = Some(n);
        }
    }

    pub fn set_pending(&mut self, p: Pending, now_ms: u64) {
        self.state = State::AwaitingConfirm { pending: p, until: now_ms + CONFIRM_WAIT_MS };
    }

    /// True while a pending action is held, whether or not its window has run
    /// out; the next segment (or a new `set_pending`) settles it.
    pub fn has_pending(&self) -> bool {
        matches!(self.state, State::AwaitingConfirm { .. })
    }

    pub fn state(&self, now_ms: u64) -> &'static str {
        match &self.state {
            State::Idle => "idle",
            State::AwaitingCommand { until } if now_ms <= *until => "awaiting-command",
            State::AwaitingConfirm { until, .. } if now_ms <= *until => "awaiting-confirm",
            _ => "idle",
        }
    }

    /// Drops a pending action whose window has run out; true when one was dropped.
    pub fn drop_expired(&mut self, now_ms: u64) -> bool {
        let expired = self.has_pending() && self.state(now_ms) == "idle";
        if expired {
            self.state = State::Idle;
        }
        expired
    }

    fn expire(&mut self, now_ms: u64) {
        if self.state(now_ms) == "idle" {
            self.state = State::Idle;
        }
    }

    pub fn on_segment(&mut self, text: &str, now_ms: u64) -> Vec<Effect> {
        self.expire(now_ms);
        if let Some(ignored) = self.ignore.take() {
            if ignored == normalise(text) {
                return vec![];
            }
        }
        // With the wake word and a command, the command wins over a no-word
        // inside it: "Maya tell coral stop" is a new command, not a cancel.
        if let (State::AwaitingConfirm { .. }, Wake::Command(c)) = (&self.state, extract(text)) {
            if !plain_answer(&c) {
                self.state = State::Idle;
                return vec![Effect::Interpret(c)];
            }
        }
        match std::mem::replace(&mut self.state, State::Idle) {
            State::AwaitingConfirm { pending, until } => match answer(text) {
                Answer::Yes => vec![Effect::Execute(pending.action)],
                Answer::No => vec![Effect::Cancelled],
                Answer::Other => {
                    // A new command replaces the pending one; other chatter keeps it waiting.
                    match extract(text) {
                        Wake::Command(c) => vec![Effect::Interpret(c)],
                        Wake::Bare => {
                            self.state = State::AwaitingCommand { until: now_ms + COMMAND_WAIT_MS };
                            vec![Effect::Say("Yes?".into())]
                        }
                        Wake::None => {
                            self.state = State::AwaitingConfirm { pending, until };
                            vec![]
                        }
                    }
                }
            },
            State::AwaitingCommand { .. } => match extract(text) {
                Wake::Command(c) => vec![Effect::Interpret(c)],
                Wake::Bare => {
                    self.state = State::AwaitingCommand { until: now_ms + COMMAND_WAIT_MS };
                    vec![Effect::Say("Yes?".into())]
                }
                Wake::None => vec![Effect::Interpret(text.trim().to_string())],
            },
            State::Idle => match extract(text) {
                Wake::Command(c) => vec![Effect::Interpret(c)],
                Wake::Bare => {
                    self.state = State::AwaitingCommand { until: now_ms + COMMAND_WAIT_MS };
                    vec![Effect::Say("Yes?".into())]
                }
                Wake::None => vec![],
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_the_command_after_any_spelling_of_the_wake_word() {
        assert_eq!(extract("Maya, what's waiting on me?"), Wake::Command("what's waiting on me?".into()));
        assert_eq!(extract("hey maia tell hexgrid to go ahead"), Wake::Command("tell hexgrid to go ahead".into()));
        assert_eq!(extract("Mya compact the coral one"), Wake::Command("compact the coral one".into()));
        assert_eq!(extract("my a, focus on matters"), Wake::Command("focus on matters".into()));
        assert_eq!(extract("Maya"), Wake::Bare);
        assert_eq!(extract("Maya."), Wake::Bare);
        assert_eq!(extract("so I told her about it"), Wake::None);
        // The wake word inside a longer sentence still wakes her; the command is what follows.
        assert_eq!(extract("I told Maya yesterday about the deploy"), Wake::Command("yesterday about the deploy".into()));
        assert_eq!(extract("Maya tell hexgrid-one to go"), Wake::Command("tell hexgrid-one to go".into()));
        assert_eq!(extract("Maya, tell hexgrid to run 24/7 please"), Wake::Command("tell hexgrid to run 24/7 please".into()));
        assert_eq!(extract("the mayan calendar"), Wake::None);
        assert_eq!(extract("my apple is here"), Wake::None);
    }

    #[test]
    fn yes_and_no_are_recognised_loosely() {
        for s in ["yes", "Yes.", "yeah go ahead", "do it", "go ahead", "confirm", "yep"] {
            assert_eq!(answer(s), Answer::Yes, "{s}");
        }
        for s in ["no", "No.", "cancel", "stop", "don't", "never mind"] {
            assert_eq!(answer(s), Answer::No, "{s}");
        }
        assert_eq!(answer("actually tell coral instead"), Answer::Other);
        for s in ["don't do it", "not ok", "not sure", "no problem, go ahead"] {
            assert_eq!(answer(s), Answer::No, "{s}");
        }
        for s in ["okay so tell coral", "yes but wait", "sure, and then compact it"] {
            assert_eq!(answer(s), Answer::Other, "{s}");
        }
        assert_eq!(answer("yes please"), Answer::Yes);
        assert_eq!(answer("go for it maya"), Answer::Yes);
    }

    fn pending() -> Pending {
        Pending { say: "Telling hexgrid: go ahead. Yes?".into(), action: serde_json::json!({"kind": "reply", "session": "hexgrid", "text": "go ahead"}) }
    }

    #[test]
    fn bare_wake_word_waits_for_the_next_segment_then_times_out() {
        let mut f = Flow::new();
        assert_eq!(f.on_segment("Maya", 1000), vec![Effect::Say("Yes?".into())]);
        assert_eq!(f.state(1500), "awaiting-command");
        assert_eq!(f.on_segment("what's waiting", 2000), vec![Effect::Interpret("what's waiting".into())]);
        assert_eq!(f.state(2001), "idle");
        f.on_segment("Maya.", 5000);
        assert_eq!(f.state(5000 + COMMAND_WAIT_MS + 1), "idle");
        assert_eq!(f.on_segment("late words", 5000 + COMMAND_WAIT_MS + 1), vec![]);
    }

    #[test]
    fn a_pending_action_is_confirmed_cancelled_or_replaced_and_expires() {
        let mut f = Flow::new();
        f.set_pending(pending(), 1000);
        assert_eq!(f.state(1500), "awaiting-confirm");
        assert_eq!(f.on_segment("yes", 2000), vec![Effect::Execute(pending().action)]);
        assert_eq!(f.state(2001), "idle");
        f.set_pending(pending(), 3000);
        assert_eq!(f.on_segment("no", 3500), vec![Effect::Cancelled]);
        f.set_pending(pending(), 4000);
        // Anything else is a new command and drops the pending one.
        assert_eq!(f.on_segment("Maya tell coral yes instead", 4500), vec![Effect::Interpret("tell coral yes instead".into())]);
        f.set_pending(pending(), 6000);
        assert!(f.has_pending());
        // Past its window but before any segment: expired for state(), still held.
        assert_eq!(f.state(6000 + CONFIRM_WAIT_MS + 1), "idle");
        assert!(f.has_pending(), "the variant is still held until a segment or a restart clears it");
        assert_eq!(f.on_segment("yes", 6000 + CONFIRM_WAIT_MS + 1), vec![], "a late yes confirms nothing");
        assert!(!f.has_pending(), "the late segment settled it");
        assert_eq!(f.state(6000 + CONFIRM_WAIT_MS + 1), "idle");
        f.set_pending(pending(), 6000 + CONFIRM_WAIT_MS + 5);
        assert_eq!(f.on_segment("yes", 6000 + CONFIRM_WAIT_MS + 10), vec![Effect::Execute(pending().action)], "a restarted window confirms");
    }

    #[test]
    fn a_wake_word_with_a_command_replaces_the_pending_action_even_with_a_no_word() {
        let mut f = Flow::new();
        f.set_pending(pending(), 1000);
        assert_eq!(f.on_segment("Maya tell coral stop", 1500), vec![Effect::Interpret("tell coral stop".into())]);
        f.set_pending(pending(), 2000);
        assert_eq!(f.on_segment("Maya, don't touch hexgrid", 2500), vec![Effect::Interpret("don't touch hexgrid".into())]);
        // The wake word in front of a plain answer still answers.
        f.set_pending(pending(), 3000);
        assert_eq!(f.on_segment("Maya, cancel", 3500), vec![Effect::Cancelled]);
        f.set_pending(pending(), 4000);
        assert_eq!(f.on_segment("Maya yes", 4500), vec![Effect::Execute(pending().action)]);
        f.set_pending(pending(), 5000);
        assert_eq!(f.on_segment("no Maya", 5500), vec![Effect::Cancelled]);
        f.set_pending(pending(), 6000);
        assert_eq!(f.on_segment("stop", 6500), vec![Effect::Cancelled], "without the wake word a no-word still cancels");
    }

    #[test]
    fn an_expired_pending_action_can_be_dropped_once() {
        let mut f = Flow::new();
        f.set_pending(pending(), 1000);
        assert!(!f.drop_expired(1000 + CONFIRM_WAIT_MS), "still inside its window");
        assert!(f.has_pending());
        assert!(f.drop_expired(1000 + CONFIRM_WAIT_MS + 1));
        assert!(!f.has_pending());
        assert!(!f.drop_expired(1000 + CONFIRM_WAIT_MS + 2), "nothing left to drop");
    }

    #[test]
    fn her_own_last_line_is_ignored_once_but_short_prompts_never_are() {
        let mut f = Flow::new();
        f.ignore_line("Maya here. hexgrid needs a decision");
        assert_eq!(f.on_segment("Maya here hexgrid needs a decision", 1000), vec![]);
        // Consumed: the same words later are a real command.
        assert_eq!(f.on_segment("Maya here hexgrid needs a decision", 2000), vec![Effect::Interpret("here hexgrid needs a decision".into())]);
        f.set_pending(pending(), 3000);
        f.ignore_line("Yes?");
        assert_eq!(f.on_segment("yes", 3500), vec![Effect::Execute(pending().action)], "a short prompt is never stored");
    }
}
