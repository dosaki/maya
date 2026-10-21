//! The wake word and the short conversation around a command: "Maya" (or
//! the word the user chose) then a command, or a command in the same breath;
//! the action window after Maya announces something, where a stop, a yes or
//! a correction settles it. Pure functions and a small
//! state machine, no I/O.

pub const COMMAND_WAIT_MS: u64 = 8_000;

/// The wake word when the user has not chosen one.
pub const DEFAULT_WAKE_WORD: &str = "Maya";

/// How the recogniser may spell "Maya"; all of them wake her.
const MAYA_SPELLINGS: &[&str] = &["maya", "maia", "mya", "my a"];

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

/// The word Maya answers to: `typed` trimmed, or "Maya" when it holds no
/// letter or digit.
pub fn wake_word(typed: Option<&str>) -> String {
    match typed.map(str::trim) {
        Some(t) if !normalise(t).is_empty() => t.to_string(),
        _ => DEFAULT_WAKE_WORD.to_string(),
    }
}

/// What a segment must contain to wake her: "Maya" in any of the
/// recogniser's spellings, or a chosen word as typed.
fn spellings(word: &str) -> Vec<String> {
    let n = normalise(word);
    if n.is_empty() || n == "maya" {
        MAYA_SPELLINGS.iter().map(|s| s.to_string()).collect()
    } else {
        vec![n]
    }
}

/// Words that may sit beside a yes or a no without changing it: "please",
/// "thanks" and the wake word itself ("yes Maya").
fn fillers(word: &str) -> Vec<String> {
    let n = normalise(word);
    let n = if n.is_empty() { "maya".to_string() } else { n };
    FILLER_WORDS.iter().map(|s| s.to_string()).chain(n.split_whitespace().map(String::from)).collect()
}

/// The command after the wake word, if the segment contains one of `spellings`.
///
/// Matches the wake word on the original whitespace-split tokens (each
/// normalised on its own) rather than on a globally-normalised string, so a
/// hyphenated or slashed word right after the wake word can't absorb it.
fn extract(segment: &str, spellings: &[String]) -> Wake {
    let original: Vec<&str> = segment.split_whitespace().collect();
    let normalised: Vec<String> = original.iter().map(|t| normalise_token(t)).collect();

    for start in 0..normalised.len() {
        for w in spellings {
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

const FILLER_WORDS: &[&str] = &["please", "thanks", "thank", "you"];
const NO_WORDS: &[&str] = &["no", "nope", "not", "don't", "cancel", "stop", "abort", "never", "nevermind"];
const YES_PHRASES: &[&str] = &["yes", "yeah", "yep", "yup", "sure", "ok", "okay", "confirm", "do it", "go ahead", "go for it"];

/// Loose yes/no recognition over a short utterance. Declines win over
/// confirmations, and a "yes" must consume the whole (filler-stripped)
/// utterance, so a redirected or hedged reply falls through to `Other`.
/// `fillers` are dropped first.
fn answer(segment: &str, fillers: &[String]) -> Answer {
    let n = normalise(segment);
    let words: Vec<&str> = n.split_whitespace().collect();
    if words.len() > 4 {
        return Answer::Other;
    }
    let remaining: Vec<&str> = words.into_iter().filter(|w| !fillers.iter().any(|f| f == w)).collect();
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

/// Words that stop an announced action when they are the whole utterance.
const STOP_WORDS: &[&str] = &["no", "nope", "stop", "wait", "cancel", "abort", "hold", "never", "nevermind", "don't", "not", "nah"];
/// Words that may sit beside a stop word ("hold on", "never mind", "stop that").
const STOP_COMPANIONS: &[&str] = &["on", "mind", "now", "it", "that", "do", "thanks", "yet"];

fn is_stop_with(text: &str, fillers: &[String]) -> bool {
    let n = normalise(text);
    let words: Vec<&str> = n.split_whitespace().filter(|w| !fillers.iter().any(|f| f == w)).collect();
    words.len() <= 4 && words.iter().any(|w| STOP_WORDS.contains(w)) && words.iter().all(|w| STOP_WORDS.contains(w) || STOP_COMPANIONS.contains(w))
}

/// Sounds a recogniser hears in silence or hesitation; never a correction.
const NOISE_WORDS: &[&str] = &["um", "uh", "er", "erm", "ah", "hmm", "mm", "mhm"];

/// True when a segment heard in the window is only her own announcement
/// coming back (every word is one of hers) or noise, so it settles nothing.
fn is_echo(ignored: &str, text: &str) -> bool {
    let n = normalise(text);
    let ours: Vec<&str> = ignored.split_whitespace().collect();
    n.split_whitespace().all(|w| NOISE_WORDS.contains(&w) || ours.contains(&w))
}

/// True when a short utterance is only a stop: no, stop, wait, cancel, hold
/// on, never mind, don't. Loose like `answer`, but a request that merely holds
/// one of these words ("tell coral to stop") is not a stop.
pub fn is_stop(text: &str) -> bool {
    is_stop_with(text, &fillers(DEFAULT_WAKE_WORD))
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Say(String),
    Interpret(String),
    /// The user stopped the announced batch.
    Cancelled,
    /// The user said yes: run the announced batch now.
    RunNow,
    /// The user corrected the announced batch: cancel it and interpret this.
    Correct(String),
    /// "Maya, stop" with nothing announced: stop what she is doing.
    Stop,
}

enum State {
    Idle,
    /// The next segment is a command without the wake word. `woken`: a bare
    /// wake word opened it, so a plain stop stops her; otherwise she asked a
    /// question, and "no" is its answer.
    AwaitingCommand { until: u64, woken: bool },
    /// An announced batch waits to run; the next segment settles it.
    Window { until: u64 },
}

/// Her lines kept for the echo filter; older ones are dropped.
const MAX_IGNORED: usize = 8;

pub struct Flow {
    state: State,
    /// Every line she said since the last segment that did something,
    /// normalised, oldest first.
    ignore: Vec<String>,
    spellings: Vec<String>,
    fillers: Vec<String>,
}

impl Flow {
    /// A flow that answers to "Maya".
    pub fn new() -> Flow {
        Flow::for_word(DEFAULT_WAKE_WORD)
    }

    pub fn for_word(word: &str) -> Flow {
        Flow { state: State::Idle, ignore: Vec::new(), spellings: spellings(word), fillers: fillers(word) }
    }

    /// Answers to `word` from the next segment on; the conversation so far stays.
    pub fn set_word(&mut self, word: &str) {
        self.spellings = spellings(word);
        self.fillers = fillers(word);
    }

    /// Forgets the conversation, keeping the word.
    pub fn reset(&mut self) {
        self.state = State::Idle;
        self.ignore.clear();
    }

    /// Remembers a line Maya says so hearing it back does nothing. The ear
    /// stays open while she says a short line, so the user may talk over it
    /// ("Yes?") and the next segment can start with her words: `on_segment`
    /// strips them from the front. Every line she says is kept until a
    /// segment does something (a new request, or an answer that settles the
    /// window) or the window closes: a short reply of hers ("Sure.", "On
    /// it.") heard back during the window must never run or correct it.
    /// Outside a window a line heard back is dropped once and forgotten.
    pub fn ignore_line(&mut self, spoken: &str) {
        let n = normalise(spoken);
        if n.is_empty() {
            return;
        }
        self.ignore.retain(|l| *l != n);
        self.ignore.push(n);
        if self.ignore.len() > MAX_IGNORED {
            self.ignore.remove(0);
        }
    }

    /// Maya just asked the user something: the next segment within
    /// `COMMAND_WAIT_MS` is taken as the answer, no wake word needed.
    pub fn await_reply(&mut self, now_ms: u64) {
        self.state = State::AwaitingCommand { until: now_ms + COMMAND_WAIT_MS, woken: false };
    }

    /// An announced batch waits until `until` (milliseconds): the next
    /// segment stops it, runs it now or corrects it.
    pub fn open_window(&mut self, until: u64) {
        self.state = State::Window { until };
    }

    /// The batch ran or was cancelled from elsewhere (a button, the phone):
    /// the window ends, and her lines with it.
    pub fn close_window(&mut self) {
        if matches!(self.state, State::Window { .. }) {
            self.state = State::Idle;
            self.ignore.clear();
        }
    }

    pub fn state(&self, now_ms: u64) -> &'static str {
        match &self.state {
            State::Idle => "idle",
            State::AwaitingCommand { until, .. } if now_ms <= *until => "awaiting-command",
            State::Window { until } if now_ms < *until => "window",
            _ => "idle",
        }
    }

    fn expire(&mut self, now_ms: u64) {
        if self.state(now_ms) == "idle" {
            self.state = State::Idle;
        }
    }

    /// True when a segment is a plain stop or a plain yes.
    fn settles(&self, text: &str) -> bool {
        is_stop_with(text, &self.fillers) || answer(text, &self.fillers) == Answer::Yes
    }

    /// A command after the wake word: a plain stop stops her, anything else is
    /// interpreted.
    fn command(&self, c: String) -> Vec<Effect> {
        if is_stop_with(&c, &self.fillers) {
            vec![Effect::Stop]
        } else {
            vec![Effect::Interpret(c)]
        }
    }

    /// Listens for a command without the wake word, after a bare wake word.
    fn woken(&mut self, now_ms: u64) -> Vec<Effect> {
        self.state = State::AwaitingCommand { until: now_ms + COMMAND_WAIT_MS, woken: true };
        vec![Effect::Say("Yes?".into())]
    }

    pub fn on_segment(&mut self, text: &str, now_ms: u64) -> Vec<Effect> {
        self.expire(now_ms);
        // In a window her lines stay until it closes: the recogniser may
        // split or garble them over several segments.
        let in_window = matches!(self.state, State::Window { .. });
        let seg = normalise(text);
        // One of her lines heard back whole is dropped, whatever it says.
        if let Some(i) = self.ignore.iter().position(|l| *l == seg) {
            if !in_window {
                self.ignore.remove(i);
            }
            return vec![];
        }
        // The user talked over a line of hers: it is taken off the front.
        let over = self.ignore.iter().enumerate().filter(|(_, l)| seg.starts_with(&format!("{l} "))).max_by_key(|(_, l)| l.len()).map(|(i, l)| (i, seg[l.len()..].trim().to_string()));
        let owned;
        let text = match over {
            Some((i, rest)) => {
                if !in_window {
                    self.ignore.remove(i);
                }
                owned = rest;
                owned.as_str()
            }
            None => text,
        };
        // Only her words, or noise: nothing to settle. A stop or a yes always
        // wins, even when her announcement holds the word.
        if in_window && is_echo(&self.ignore.join(" "), text) && !self.settles(text) {
            return vec![];
        }
        let effects = self.settle_segment(text, now_ms);
        if !effects.is_empty() {
            self.ignore.clear();
        }
        effects
    }

    fn settle_segment(&mut self, text: &str, now_ms: u64) -> Vec<Effect> {
        match std::mem::replace(&mut self.state, State::Idle) {
            State::Window { .. } => {
                let found = extract(text, &self.spellings);
                let said = match &found {
                    Wake::Command(c) => c.clone(),
                    _ => text.trim().to_string(),
                };
                if is_stop_with(&said, &self.fillers) {
                    vec![Effect::Cancelled]
                } else if answer(&said, &self.fillers) == Answer::Yes {
                    vec![Effect::RunNow]
                } else if found == Wake::Bare {
                    let mut out = vec![Effect::Cancelled];
                    out.extend(self.woken(now_ms));
                    out
                } else {
                    vec![Effect::Correct(said)]
                }
            }
            State::AwaitingCommand { woken, .. } => match extract(text, &self.spellings) {
                Wake::Command(c) => self.command(c),
                Wake::Bare => self.woken(now_ms),
                // "Maya." "Yes?" "Stop.": she stops. After her question a "no" is its answer.
                Wake::None if woken && is_stop_with(text, &self.fillers) => vec![Effect::Stop],
                Wake::None => vec![Effect::Interpret(text.trim().to_string())],
            },
            State::Idle => match extract(text, &self.spellings) {
                Wake::Command(c) => self.command(c),
                Wake::Bare => self.woken(now_ms),
                Wake::None => vec![],
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn maya() -> Vec<String> {
        spellings("Maya")
    }

    #[test]
    fn extracts_the_command_after_any_spelling_of_the_wake_word() {
        assert_eq!(extract("Maya, what's waiting on me?", &maya()), Wake::Command("what's waiting on me?".into()));
        assert_eq!(extract("hey maia tell hexgrid to go ahead", &maya()), Wake::Command("tell hexgrid to go ahead".into()));
        assert_eq!(extract("Mya compact the coral one", &maya()), Wake::Command("compact the coral one".into()));
        assert_eq!(extract("my a, focus on matters", &maya()), Wake::Command("focus on matters".into()));
        assert_eq!(extract("Maya", &maya()), Wake::Bare);
        assert_eq!(extract("Maya.", &maya()), Wake::Bare);
        assert_eq!(extract("so I told her about it", &maya()), Wake::None);
        // The wake word inside a longer sentence still wakes her; the command is what follows.
        assert_eq!(extract("I told Maya yesterday about the deploy", &maya()), Wake::Command("yesterday about the deploy".into()));
        assert_eq!(extract("Maya tell hexgrid-one to go", &maya()), Wake::Command("tell hexgrid-one to go".into()));
        assert_eq!(extract("Maya, tell hexgrid to run 24/7 please", &maya()), Wake::Command("tell hexgrid to run 24/7 please".into()));
        assert_eq!(extract("the mayan calendar", &maya()), Wake::None);
        assert_eq!(extract("my apple is here", &maya()), Wake::None);
    }

    #[test]
    fn yes_and_no_are_recognised_loosely() {
        for s in ["yes", "Yes.", "yeah go ahead", "do it", "go ahead", "confirm", "yep"] {
            assert_eq!(answer(s, &fillers("Maya")), Answer::Yes, "{s}");
        }
        for s in ["no", "No.", "cancel", "stop", "don't", "never mind"] {
            assert_eq!(answer(s, &fillers("Maya")), Answer::No, "{s}");
        }
        assert_eq!(answer("actually tell coral instead", &fillers("Maya")), Answer::Other);
        for s in ["don't do it", "not ok", "not sure", "no problem, go ahead"] {
            assert_eq!(answer(s, &fillers("Maya")), Answer::No, "{s}");
        }
        for s in ["okay so tell coral", "yes but wait", "sure, and then compact it"] {
            assert_eq!(answer(s, &fillers("Maya")), Answer::Other, "{s}");
        }
        assert_eq!(answer("yes please", &fillers("Maya")), Answer::Yes);
        assert_eq!(answer("go for it maya", &fillers("Maya")), Answer::Yes);
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
    fn after_she_asks_a_question_the_next_segment_is_the_answer_without_a_wake_word() {
        let mut f = Flow::new();
        f.await_reply(1_000);
        assert_eq!(f.state(1_000), "awaiting-command");
        assert_eq!(f.on_segment("the second one", 3_000), vec![Effect::Interpret("the second one".into())]);
        assert_eq!(f.state(3_000), "idle");
        // The window closes after COMMAND_WAIT_MS: chatter is ignored again.
        f.await_reply(10_000);
        assert_eq!(f.on_segment("the second one", 10_000 + COMMAND_WAIT_MS + 1), vec![]);
        // A wake word inside the window still works as usual.
        f.await_reply(30_000);
        assert_eq!(f.on_segment("Maya what's waiting", 31_000), vec![Effect::Interpret("what's waiting".into())]);
    }

    #[test]
    fn in_the_window_stop_words_cancel_yes_runs_now_and_anything_else_corrects() {
        let mut f = Flow::new();
        f.open_window(10_000);
        assert_eq!(f.on_segment("no", 1), vec![Effect::Cancelled]);
        assert_eq!(f.state(2), "idle", "every answer closes the window");
        f.open_window(10_000);
        assert_eq!(f.on_segment("go ahead", 1), vec![Effect::RunNow]);
        f.open_window(10_000);
        assert_eq!(f.on_segment("the other one", 1), vec![Effect::Correct("the other one".into())]);
        f.open_window(10_000);
        assert_eq!(f.on_segment("Maya tell coral to stop", 1), vec![Effect::Correct("tell coral to stop".into())]);
        f.open_window(10_000);
        assert_eq!(f.on_segment("Maya, cancel", 1), vec![Effect::Cancelled]);
        f.open_window(10_000);
        assert_eq!(f.on_segment("Maya yes", 1), vec![Effect::RunNow]);
        f.open_window(10_000);
        assert_eq!(f.on_segment("wait", 1), vec![Effect::Cancelled]);
        f.open_window(10_000);
        assert_eq!(f.on_segment("hold on", 1), vec![Effect::Cancelled]);
    }

    #[test]
    fn a_bare_wake_word_in_the_window_cancels_it_and_listens() {
        let mut f = Flow::new();
        f.open_window(10_000);
        assert_eq!(f.on_segment("Maya", 1), vec![Effect::Cancelled, Effect::Say("Yes?".into())]);
        assert_eq!(f.state(2), "awaiting-command");
    }

    #[test]
    fn her_announcement_heard_back_is_not_a_correction() {
        let mut f = Flow::new();
        f.ignore_line("Telling api-fix: open the PR.");
        f.open_window(10_000);
        assert_eq!(f.on_segment("telling api fix open the pr", 1), vec![]);
        assert_eq!(f.state(2), "window", "the window stays open");
        // Even a short line of hers is not an answer in the window.
        f.ignore_line("Yes?");
        assert_eq!(f.on_segment("Yes?", 3), vec![]);
        assert_eq!(f.state(4), "window");
    }

    #[test]
    fn a_split_or_garbled_echo_and_noise_leave_the_window_open() {
        let mut f = Flow::new();
        f.ignore_line("Telling api-fix: open the PR.");
        f.open_window(10_000);
        assert_eq!(f.on_segment("telling api fix", 1), vec![]);
        assert_eq!(f.on_segment("open the pr", 2), vec![]);
        assert_eq!(f.on_segment("um", 3), vec![], "filler-only segments are dropped");
        assert_eq!(f.on_segment("api fix", 4), vec![], "a variant made of her words is still her");
        assert_eq!(f.state(5), "window");
        // A real correction holds a word she did not say.
        assert_eq!(f.on_segment("the other one", 6), vec![Effect::Correct("the other one".into())]);
        assert_eq!(f.state(7), "idle");
        // Closing the window ends the echo filter.
        f.ignore_line("Telling api-fix: open the PR.");
        f.open_window(10_000);
        f.close_window();
        // Her line is gone: the same words are now an answer, not an echo.
        f.await_reply(8);
        assert_eq!(f.on_segment("telling api fix open the pr", 9), vec![Effect::Interpret("telling api fix open the pr".into())]);
    }

    #[test]
    fn a_stop_or_a_yes_is_never_taken_for_her_echo() {
        let mut f = Flow::new();
        f.ignore_line("Telling coral: stop the build");
        f.open_window(10_000);
        assert_eq!(f.on_segment("stop", 1), vec![Effect::Cancelled]);
        f.ignore_line("Telling coral: stop the build");
        f.open_window(10_000);
        assert_eq!(f.on_segment("not now", 1), vec![Effect::Cancelled]);
        f.ignore_line("Telling coral: go ahead");
        f.open_window(10_000);
        assert_eq!(f.on_segment("go ahead", 1), vec![Effect::RunNow]);
        // The rest of her line is still dropped.
        f.ignore_line("Telling coral: stop the build");
        f.open_window(10_000);
        assert_eq!(f.on_segment("the build", 1), vec![]);
    }

    #[test]
    fn not_now_is_a_stop_but_not_that_one_is_a_correction() {
        for s in ["not now", "not yet", "nah"] {
            assert!(is_stop(s), "{s}");
        }
        assert!(!is_stop("not that one"));
        let mut f = Flow::new();
        f.open_window(10_000);
        assert_eq!(f.on_segment("not now", 1), vec![Effect::Cancelled]);
        f.open_window(10_000);
        assert_eq!(f.on_segment("not that one", 1), vec![Effect::Correct("not that one".into())]);
        assert_eq!(f.on_segment("Maya, not now", 2), vec![Effect::Stop]);
        assert_eq!(f.on_segment("Maya, nah", 3), vec![Effect::Stop]);
    }

    #[test]
    fn maya_stop_outside_the_window_stops_her() {
        let mut f = Flow::new();
        assert_eq!(f.on_segment("Maya, stop", 1), vec![Effect::Stop]);
        assert_eq!(f.on_segment("Maya stop that", 1), vec![Effect::Stop]);
        assert_eq!(f.on_segment("Maya tell coral to stop", 1), vec![Effect::Interpret("tell coral to stop".into())]);
    }

    #[test]
    fn an_expired_window_reads_idle() {
        let mut f = Flow::new();
        f.open_window(100);
        assert_eq!(f.state(99), "window");
        assert_eq!(f.state(100), "idle", "the deadline itself is out of the window, as in Window::take_due");
        assert_eq!(f.state(101), "idle");
        assert_eq!(f.on_segment("no", 101), vec![], "a late word cancels nothing");
        f.open_window(100);
        f.close_window();
        assert_eq!(f.state(1), "idle");
    }

    #[test]
    fn stop_words_are_loose_but_a_request_that_holds_one_is_not_a_stop() {
        for s in ["no", "Stop.", "wait", "cancel", "hold on", "never mind", "don't", "stop that", "no thanks"] {
            assert!(is_stop(s), "{s}");
        }
        for s in ["tell coral to stop", "yes", "the other one", ""] {
            assert!(!is_stop(s), "{s}");
        }
    }

    #[test]
    fn her_short_prompt_is_stripped_from_the_front_of_what_the_user_says_over_it() {
        // The ear keeps listening while she says "Yes?", so the segment can
        // start with her own word followed by the user's command.
        let mut f = Flow::new();
        f.await_reply(1_000);
        f.ignore_line("Yes?");
        assert_eq!(f.on_segment("Yes? Tell Collector 2 to approve.", 2_000), vec![Effect::Interpret("tell collector 2 to approve".into())]);
        // Her "Yes?" heard back on its own means nothing while she waits for a command.
        f.await_reply(3_000);
        f.ignore_line("Yes?");
        assert_eq!(f.on_segment("Yes?", 3_500), vec![]);
        assert_eq!(f.state(3_600), "awaiting-command", "the window stays open");
        // Without an ignored line, a segment is untouched.
        assert_eq!(f.on_segment("tell coral to stop", 4_000), vec![Effect::Interpret("tell coral to stop".into())]);
    }

    #[test]
    fn her_own_last_line_outside_the_window_is_ignored_once() {
        let mut f = Flow::new();
        f.ignore_line("Maya here. hexgrid needs a decision");
        assert_eq!(f.on_segment("Maya here hexgrid needs a decision", 1000), vec![]);
        // Consumed: the same words later are a real command.
        assert_eq!(f.on_segment("Maya here hexgrid needs a decision", 2000), vec![Effect::Interpret("here hexgrid needs a decision".into())]);
        f.ignore_line("Yes?");
        assert_eq!(f.on_segment("yes", 3500), vec![], "a short prompt of hers is dropped as well");
    }

    #[test]
    fn every_line_she_said_since_the_request_is_ignored_until_the_window_closes() {
        let mut f = Flow::new();
        // Her short reply, then the announcement: "sure" heard late is still hers.
        f.ignore_line("Sure.");
        f.open_window(10_000);
        f.ignore_line("Telling api-fix: open the PR.");
        assert_eq!(f.on_segment("sure", 1), vec![]);
        assert_eq!(f.state(2), "window", "her own yes runs nothing");
        f.ignore_line("On it.");
        assert_eq!(f.on_segment("on it", 3), vec![], "her own words are never a correction");
        f.ignore_line("Done.");
        assert_eq!(f.on_segment("Done.", 4), vec![]);
        assert_eq!(f.state(5), "window");
        // Closing the window forgets them: the user's own "sure" now runs it.
        f.close_window();
        f.open_window(10_000);
        assert_eq!(f.on_segment("sure", 6), vec![Effect::RunNow]);
    }

    #[test]
    fn a_new_request_forgets_her_earlier_lines() {
        let mut f = Flow::new();
        f.ignore_line("Sure.");
        f.ignore_line("Okay.");
        assert_eq!(f.on_segment("Maya, compact coral", 1), vec![Effect::Interpret("compact coral".into())]);
        f.open_window(10_000);
        assert_eq!(f.on_segment("sure", 2), vec![Effect::RunNow], "she has not said it since this request");
    }

    #[test]
    fn a_stop_after_a_bare_wake_word_stops_her_but_a_no_to_her_question_is_an_answer() {
        let mut f = Flow::new();
        assert_eq!(f.on_segment("Maya.", 1_000), vec![Effect::Say("Yes?".into())]);
        assert_eq!(f.on_segment("stop", 2_000), vec![Effect::Stop]);
        assert_eq!(f.on_segment("Maya", 3_000), vec![Effect::Say("Yes?".into())]);
        assert_eq!(f.on_segment("never mind", 4_000), vec![Effect::Stop]);
        // She asked something: "no" is the answer, not a stop.
        f.await_reply(5_000);
        assert_eq!(f.on_segment("no", 6_000), vec![Effect::Interpret("no".into())]);
        // A bare wake word in the window cancels it and listens; a stop then stops her.
        f.open_window(20_000);
        assert_eq!(f.on_segment("Maya", 7_000), vec![Effect::Cancelled, Effect::Say("Yes?".into())]);
        assert_eq!(f.on_segment("stop", 8_000), vec![Effect::Stop]);
        // A real command after a bare wake word is still a command.
        f.on_segment("Maya", 9_000);
        assert_eq!(f.on_segment("tell coral to stop", 10_000), vec![Effect::Interpret("tell coral to stop".into())]);
    }

    #[test]
    fn a_blank_or_wordless_choice_is_maya_and_a_real_one_is_trimmed() {
        assert_eq!(wake_word(None), "Maya");
        assert_eq!(wake_word(Some("")), "Maya");
        assert_eq!(wake_word(Some("  ,. ")), "Maya");
        assert_eq!(wake_word(Some("  Jarvis ")), "Jarvis");
        assert_eq!(wake_word(Some("Hey Jarvis")), "Hey Jarvis");
    }

    #[test]
    fn a_chosen_word_wakes_her_as_typed_and_maya_no_longer_does() {
        let mut f = Flow::for_word("Jarvis");
        assert_eq!(f.on_segment("Jarvis, what's waiting on me?", 1_000), vec![Effect::Interpret("what's waiting on me?".into())]);
        assert_eq!(f.on_segment("jarvis.", 2_000), vec![Effect::Say("Yes?".into())]);
        assert_eq!(f.on_segment("what's waiting", 3_000), vec![Effect::Interpret("what's waiting".into())]);
        assert_eq!(f.on_segment("Maya what's waiting", 4_000), vec![], "Maya is just a word now");
        assert_eq!(f.on_segment("mya what's waiting", 5_000), vec![], "and so are her spellings");
        // Several words must come together, in order.
        let mut f = Flow::for_word("Hey Jarvis");
        assert_eq!(f.on_segment("hey, Jarvis, compact coral", 1_000), vec![Effect::Interpret("compact coral".into())]);
        assert_eq!(f.on_segment("Jarvis compact coral", 2_000), vec![]);
    }

    #[test]
    fn a_chosen_word_beside_a_yes_or_a_no_still_answers() {
        let mut f = Flow::for_word("Jarvis");
        f.open_window(10_000);
        assert_eq!(f.on_segment("Jarvis, yes", 1_500), vec![Effect::RunNow]);
        f.open_window(10_000);
        assert_eq!(f.on_segment("no Jarvis", 2_500), vec![Effect::Cancelled]);
        f.open_window(10_000);
        assert_eq!(f.on_segment("Jarvis tell coral stop", 3_500), vec![Effect::Correct("tell coral stop".into())]);
        assert_eq!(f.on_segment("Jarvis stop", 4_000), vec![Effect::Stop]);
    }

    #[test]
    fn a_new_word_applies_from_the_next_segment_and_a_reset_keeps_it() {
        let mut f = Flow::new();
        f.on_segment("Maya", 1_000);
        f.set_word("Jarvis");
        assert_eq!(f.state(1_500), "awaiting-command", "the open window survives the change");
        assert_eq!(f.on_segment("Maya compact coral", 9_500 + COMMAND_WAIT_MS), vec![]);
        assert_eq!(f.on_segment("Jarvis compact coral", 20_000), vec![Effect::Interpret("compact coral".into())]);
        f.on_segment("Jarvis", 30_000);
        f.reset();
        assert_eq!(f.state(30_001), "idle");
        assert_eq!(f.on_segment("Jarvis compact coral", 31_000), vec![Effect::Interpret("compact coral".into())]);
        f.set_word("  ");
        assert_eq!(f.on_segment("my a compact coral", 32_000), vec![Effect::Interpret("compact coral".into())], "blank is Maya again");
    }
}
