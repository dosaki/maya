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

/// The command after the wake word, if the segment contains one.
pub fn extract(segment: &str) -> Wake {
    let norm = normalise(segment);
    let padded = format!(" {norm} ");
    for w in WAKE_WORDS {
        if let Some(i) = padded.find(&format!(" {w} ")) {
            let after = padded[i + w.len() + 2..].trim();
            // Keep the original casing of the command: take the same number of words from the source.
            let words_after = after.split_whitespace().count();
            if words_after == 0 {
                return Wake::Bare;
            }
            let original: Vec<&str> = segment.split_whitespace().collect();
            let tail = original[original.len().saturating_sub(words_after)..].join(" ");
            let tail = tail.trim_start_matches(|c: char| c == ',' || c == '.' || c == ':').trim().to_string();
            return Wake::Command(tail);
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

pub fn answer(segment: &str) -> Answer {
    let n = normalise(segment);
    let words: Vec<&str> = n.split_whitespace().collect();
    let text = words.join(" ");
    let yes = ["yes", "yeah", "yep", "yup", "do it", "go ahead", "confirm", "sure", "ok", "okay"];
    let no = ["no", "nope", "cancel", "stop", "don't", "never mind", "nevermind", "abort"];
    if words.len() <= 4 {
        if yes.iter().any(|y| text == *y || text.starts_with(&format!("{y} ")) || text.ends_with(&format!(" {y}")) || text.contains(&format!(" {y} "))) {
            return Answer::Yes;
        }
        if no.iter().any(|y| text == *y || text.starts_with(&format!("{y} ")) || text.ends_with(&format!(" {y}")) || text.contains(&format!(" {y} "))) {
            return Answer::No;
        }
    }
    Answer::Other
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

    /// Remembers Maya's last spoken line so hearing it back does nothing.
    pub fn ignore_line(&mut self, spoken: &str) {
        self.ignore = Some(normalise(spoken));
    }

    pub fn set_pending(&mut self, p: Pending, now_ms: u64) {
        self.state = State::AwaitingConfirm { pending: p, until: now_ms + CONFIRM_WAIT_MS };
    }

    pub fn state(&self, now_ms: u64) -> &'static str {
        match &self.state {
            State::Idle => "idle",
            State::AwaitingCommand { until } if now_ms <= *until => "awaiting-command",
            State::AwaitingConfirm { until, .. } if now_ms <= *until => "awaiting-confirm",
            _ => "idle",
        }
    }

    fn expire(&mut self, now_ms: u64) {
        if self.state(now_ms) == "idle" {
            self.state = State::Idle;
        }
    }

    pub fn on_segment(&mut self, text: &str, now_ms: u64) -> Vec<Effect> {
        self.expire(now_ms);
        if self.ignore.as_deref() == Some(normalise(text).as_str()) {
            return vec![];
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
        assert_eq!(f.on_segment("yes", 6000 + CONFIRM_WAIT_MS + 1), vec![], "a late yes confirms nothing");
        assert_eq!(f.state(6000 + CONFIRM_WAIT_MS + 1), "idle");
    }

    #[test]
    fn her_own_last_line_is_ignored() {
        let mut f = Flow::new();
        f.ignore_line("Maya here. hexgrid needs a decision");
        assert_eq!(f.on_segment("Maya here hexgrid needs a decision", 1000), vec![]);
        assert_eq!(f.on_segment("Maya what's up", 2000), vec![Effect::Interpret("what's up".into())]);
    }
}
