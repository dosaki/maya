use crate::claude_menus;
use crate::events::HookEvent;
use crate::model::{parse_questions, AwaitKind, Awaiting, Card, Harness, State};
use crate::registry::RegistrySession;
use crate::transcript::TranscriptTail;

pub const SNIPPET_CHARS: usize = 200;

/// What is known about whether the agent's last message asks the user something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prose {
    /// No verdict: read the message with `asking_line`.
    Unjudged,
    /// A verdict is on its way: no ask meanwhile.
    Judging,
    /// The message asks; the line to show.
    Asks(String),
    /// The message asks nothing.
    Done,
}

pub struct DeriveInput<'a> {
    pub registry: &'a RegistrySession,
    pub events: &'a [HookEvent],
    pub transcript: &'a TranscriptTail,
    pub prose: &'a Prose,
    pub now_ms: u64,
    pub completed_timeout_ms: u64,
}

/// Trims to `max_chars` characters, replacing the last one with `…` when cut.
pub fn truncate(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Phrases that mark a sentence as waiting on the user, even without a
/// question mark. Matched on whole words.
const DECISION_PHRASES: &[&str] = &[
    "let me know",
    "say the word",
    "which do you",
    "do you want",
    "should i",
    "shall i",
    "want me to",
    "your call",
    "tell me which",
    "if you'd like",
    "if you would like",
    "if you want",
    "if you say",
    "say go",
    "say so",
    "tell me",
    "up to you",
    "your decision",
    "after you run",
    "once you run",
    "please run",
    "when you're ready",
];

/// The lines of `text` that carry prose: not inside a code fence, not a table
/// row, quote or `! ` command. Also the first `! ` command line, fenced or not.
fn prose_lines(text: &str) -> (Vec<&str>, Option<&str>) {
    let mut prose = Vec::new();
    let mut command = None;
    let mut fenced = false;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if line.starts_with("```") {
            fenced = !fenced;
        } else if line.starts_with("! ") {
            command.get_or_insert(line);
        } else if !fenced && !line.starts_with('|') && !line.starts_with('>') {
            prose.push(line);
        }
    }
    (prose, command)
}

/// The sentences of `line`, split at `.`, `?` or `!` followed by whitespace.
/// A bracketed aside stays with the sentence before it.
fn sentences(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut depth) = (0, 0i32);
    let mut chars = line.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            '.' | '?' | '!' if depth == 0 => {
                if let Some(&(_, next)) = chars.peek() {
                    if next.is_whitespace() {
                        let rest = line[i + 1..].trim_start();
                        if !rest.starts_with('(') && !rest.starts_with('[') {
                            out.push(line[start..=i].trim());
                            start = i + 1;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out.push(line[start..].trim());
    out.retain(|s| !s.is_empty());
    out
}

/// `line` without any bracketed asides at its end, such as `(e.g. "skip #2")`
/// or `[y/n]`, so the question mark before them is seen.
fn strip_trailing_asides(line: &str) -> &str {
    let mut s = line.trim_end();
    loop {
        let Some(close) = s.chars().last() else { return s };
        let open = match close {
            ')' => '(',
            ']' => '[',
            _ => return s,
        };
        let Some(start) = s.rfind(open) else { return s };
        s = s[..start].trim_end();
    }
}

/// The last sentence of `text`, anywhere in it, that asks the user for a
/// decision: it ends with a question mark (ignoring trailing markdown, quotes
/// or emoji) or contains one of `DECISION_PHRASES`. Code, tables and quotes
/// are skipped. Failing that, the first `! ` command line, as the ask to run it.
pub fn asking_line(text: &str) -> Option<String> {
    let (prose, command) = prose_lines(text);
    let asks = |s: &str| {
        let core = strip_trailing_asides(s).trim_end_matches(|c: char| !c.is_alphanumeric() && c != '?');
        if core.ends_with('?') {
            return true;
        }
        let words: Vec<String> = s.replace('’', "'").to_lowercase().split(|c: char| !c.is_alphanumeric() && c != '\'').filter(|w| !w.is_empty()).map(str::to_string).collect();
        let padded = format!(" {} ", words.join(" "));
        DECISION_PHRASES.iter().any(|p| padded.contains(&format!(" {p} ")))
    };
    prose
        .iter()
        .rev()
        .flat_map(|l| sentences(l).into_iter().rev())
        .find(|s| asks(s))
        .map(|s| balanced_bold(s).to_string())
        .or_else(|| command.map(str::to_string))
}

/// `s` without a `**` at its end or start that a sentence split left unmatched.
fn balanced_bold(s: &str) -> &str {
    if s.matches("**").count() % 2 == 0 {
        return s;
    }
    s.strip_suffix("**").map(str::trim_end).or_else(|| s.strip_prefix("**").map(str::trim_start)).unwrap_or(s)
}

fn permission_detail(e: &HookEvent) -> String {
    let tool = e.tool_name.clone().unwrap_or_else(|| "Tool".to_string());
    let input = e.tool_input.as_ref();
    let arg = input
        .and_then(|i| i.get("command").or_else(|| i.get("file_path")).or_else(|| i.get("path")))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    match arg {
        Some(a) => format!("{tool}: {}", truncate(a.trim(), 120)),
        None => tool,
    }
}

fn question_detail(e: &HookEvent) -> String {
    e.tool_input
        .as_ref()
        .and_then(|i| i["questions"][0]["question"].as_str())
        .unwrap_or("Question")
        .to_string()
}

fn awaiting_for_tool(e: &HookEvent) -> Awaiting {
    match e.tool_name.as_deref() {
        Some("AskUserQuestion") => Awaiting {
            kind: AwaitKind::Question,
            detail: question_detail(e),
            questions: e
                .tool_input
                .as_ref()
                .map(parse_questions)
                .unwrap_or_default()
                .into_iter()
                .map(|mut q| {
                    claude_menus::ask_rows(&mut q);
                    q
                })
                .collect(),
        },
        Some("ExitPlanMode") => Awaiting { kind: AwaitKind::Plan, detail: "Plan approval".to_string(), questions: claude_menus::plan_question(e).into_iter().collect() },
        _ => Awaiting { kind: AwaitKind::Permission, detail: permission_detail(e), questions: claude_menus::permission_question(e).into_iter().collect() },
    }
}

pub fn derive(i: &DeriveInput) -> Card {
    let r = i.registry;
    // (what is awaited, when, which agent asked: None = the main agent)
    let mut awaiting: Option<(Awaiting, u64, Option<String>)> = None;
    // (is_stop, timestamp) of the most recent main-agent Stop or UserPromptSubmit.
    let mut last_turn: Option<(bool, u64)> = None;
    // An Esc after the latest turn start or end: the ask that turn left is over.
    let mut interrupted = false;

    for e in i.events {
        match e.hook_event_name.as_str() {
            "PermissionRequest" => {
                awaiting = Some((awaiting_for_tool(e), e.received_at, e.agent_id.clone()));
            }
            "PreToolUse" if matches!(e.tool_name.as_deref(), Some("AskUserQuestion") | Some("ExitPlanMode")) => {
                awaiting = Some((awaiting_for_tool(e), e.received_at, e.agent_id.clone()));
            }
            "Notification" if e.notification_type.as_deref() == Some("permission_prompt") => {
                if awaiting.is_none() {
                    awaiting = Some((Awaiting { kind: AwaitKind::Permission, detail: "Permission prompt".to_string(), questions: vec![] }, e.received_at, e.agent_id.clone()));
                }
            }
            // A tool finishing only resolves a prompt raised by the same agent:
            // a background subagent's tool calls must not hide the main agent's prompt.
            "PostToolUse" | "PostToolUseFailure" | "PermissionDenied" => {
                if awaiting.as_ref().map_or(false, |(_, _, agent)| *agent == e.agent_id) {
                    awaiting = None;
                }
            }
            "Stop" if e.agent_id.is_none() => {
                awaiting = None;
                interrupted = false;
                last_turn = Some((true, e.received_at));
            }
            // Written by Maya when it presses Esc: Claude's hooks say nothing of an interrupt.
            "MayaInterrupt" if e.agent_id.is_none() => {
                awaiting = None;
                interrupted = true;
            }
            "UserPromptSubmit" if e.agent_id.is_none() => {
                awaiting = None;
                interrupted = false;
                last_turn = Some((false, e.received_at));
            }
            _ => {}
        }
    }

    // A resumed session may have no Stop for its last turn, or only an older
    // prompt: its transcript shows when the turn ended.
    if r.status != "busy" {
        if let Some(t) = &i.transcript.turn_end {
            if last_turn.map_or(true, |(_, ts)| t.at_ms > ts) {
                last_turn = Some((true, t.at_ms));
            }
        }
    }

    if i.events.is_empty() {
        if let Some(q) = &i.transcript.open_question {
            awaiting = Some((Awaiting { kind: q.kind, detail: q.detail.clone(), questions: q.questions.clone() }, r.status_updated_at, None));
        }
    }

    // "shell": the turn has ended but a background command it started still runs.
    let is_busy = r.status == "busy";
    let shell_running = r.status == "shell";
    // A turn that ended by asking in prose waits for a reply, however long ago
    // and whatever it left running in the background.
    let asked = match last_turn {
        Some((true, ts)) if !is_busy && !interrupted => match i.prose {
            Prose::Asks(line) => Some((line.clone(), ts)),
            Prose::Unjudged => {
                let text = i.transcript.turn_end.as_ref().map(|t| t.text.as_str()).or(i.transcript.last_assistant_text.as_deref());
                text.and_then(asking_line).map(|line| (line, ts))
            }
            Prose::Judging | Prose::Done => None,
        },
        _ => None,
    };

    let (state, state_since, aw) = if let Some((a, ts, _)) = awaiting {
        (State::Awaiting, ts, Some(a))
    } else if is_busy {
        (State::Working, r.status_updated_at, None)
    } else if let Some((line, ts)) = asked {
        (State::Awaiting, ts, Some(Awaiting { kind: AwaitKind::Text, detail: truncate(&line, SNIPPET_CHARS), questions: vec![] }))
    } else if shell_running {
        (State::Working, r.status_updated_at, None)
    } else if let Some((true, ts)) = last_turn {
        if i.now_ms.saturating_sub(ts) < i.completed_timeout_ms {
            (State::Completed, ts, None)
        } else {
            (State::Idle, ts + i.completed_timeout_ms, None)
        }
    } else {
        (State::Idle, r.status_updated_at, None)
    };

    // Working on the shell alone, with no turn behind it.
    let shell = shell_running && state == State::Working;
    Card {
        session_id: r.session_id.clone(),
        pid: r.pid,
        name: r.name.clone(),
        cwd: r.cwd.clone(),
        state,
        state_since,
        snippet: truncate(i.transcript.last_assistant_text.as_deref().unwrap_or(""), SNIPPET_CHARS),
        awaiting: aw,
        has_inbox: r.messaging_socket_path.is_some(),
        harness: Harness::ClaudeCode,
        pr: None,
        context: None,
        machine: None,
        machine_address: None, machine_platform: None,
        terminal: None,
        stale: false, model: None, branch: None, brain: false, shell
    }
}

#[cfg(test)]
mod tests {
    use super::*;
use crate::events::HookEvent;
    use crate::model::{AwaitKind, State};
    use crate::registry::RegistrySession;
    use crate::transcript::{OpenQuestion, TranscriptTail};

    const MIN: u64 = 60_000;
    const TIMEOUT: u64 = 30 * MIN;

    fn reg(status: &str, status_updated_at: u64) -> RegistrySession {
        RegistrySession {
            pid: 7,
            session_id: "s".into(),
            cwd: "/Users/x/dev/eye".into(),
            name: "eye-1".into(),
            status: status.into(),
            started_at: 0,
            status_updated_at,
            entrypoint: "cli".into(),
            messaging_socket_path: None,
            tty: None,
        }
    }

    fn ev(name: &str, ts: u64) -> HookEvent {
        HookEvent {
            session_id: "s".into(),
            hook_event_name: name.into(),
            tool_name: None,
            tool_input: None,
            notification_type: None,
            transcript_path: None,
            agent_id: None,
            received_at: ts,
        }
    }

    fn tool_ev(name: &str, tool: &str, input: serde_json::Value, ts: u64) -> HookEvent {
        HookEvent { tool_name: Some(tool.into()), tool_input: Some(input), ..ev(name, ts) }
    }

    fn run(r: &RegistrySession, events: &[HookEvent], t: &TranscriptTail, now: u64) -> Card {
        run_prose(r, events, t, &Prose::Unjudged, now)
    }

    fn run_prose(r: &RegistrySession, events: &[HookEvent], t: &TranscriptTail, prose: &Prose, now: u64) -> Card {
        derive(&DeriveInput { registry: r, events, transcript: t, prose, now_ms: now, completed_timeout_ms: TIMEOUT })
    }

    fn said(text: &str) -> TranscriptTail {
        TranscriptTail { last_assistant_text: Some(text.into()), ..Default::default() }
    }

    #[test]
    fn a_judged_ask_stays_awaiting_long_after_the_turn() {
        let hours = 5 * 3_600_000;
        let c = run_prose(&reg("idle", 0), &[ev("Stop", 0)], &said("Which way?"), &Prose::Asks("Which way?".into()), hours);
        assert_eq!(c.state, State::Awaiting);
        assert_eq!(c.state_since, 0);
        let a = c.awaiting.unwrap();
        assert_eq!(a.kind, AwaitKind::Text);
        assert_eq!(a.detail, "Which way?");
    }

    #[test]
    fn a_done_verdict_overrides_a_question_mark() {
        let t = said("Why? Because X.");
        let fresh = run_prose(&reg("idle", 0), &[ev("Stop", 0)], &t, &Prose::Done, 1000);
        assert_eq!(fresh.state, State::Completed);
        let old = run_prose(&reg("idle", 0), &[ev("Stop", 0)], &t, &Prose::Done, TIMEOUT + 1);
        assert_eq!(old.state, State::Idle);
    }

    #[test]
    fn judging_and_unjudged_fall_back_as_documented() {
        let t = said("Shall I go on?");
        let events = [ev("Stop", 0)];
        assert_eq!(run_prose(&reg("idle", 0), &events, &t, &Prose::Judging, 1000).state, State::Completed);
        assert_eq!(run_prose(&reg("idle", 0), &events, &t, &Prose::Judging, TIMEOUT + 1).state, State::Idle);
        assert_eq!(run_prose(&reg("idle", 0), &events, &t, &Prose::Unjudged, TIMEOUT + 1).state, State::Awaiting);
    }

    #[test]
    fn a_resumed_session_with_no_events_takes_its_turn_end_from_the_transcript() {
        let t = TranscriptTail { turn_end: Some(crate::transcript::TurnEnd { at_ms: 1_000, text: "Run these".into() }), ..Default::default() };
        let asks = run_prose(&reg("idle", 5), &[], &t, &Prose::Asks("Run these".into()), 5_000_000);
        assert_eq!(asks.state, State::Awaiting);
        assert_eq!(asks.state_since, 1_000);
        let done = run_prose(&reg("idle", 5), &[], &t, &Prose::Done, 1_000 + TIMEOUT + 5);
        assert_eq!(done.state, State::Idle);
        assert_eq!(done.state_since, 1_000 + TIMEOUT);
    }

    #[test]
    fn a_transcript_turn_end_after_the_last_prompt_event_is_the_turn_end() {
        let t = TranscriptTail { turn_end: Some(crate::transcript::TurnEnd { at_ms: 200, text: "Which way?".into() }), ..Default::default() };
        let asks = Prose::Asks("Which way?".into());
        let c = run_prose(&reg("idle", 5), &[ev("UserPromptSubmit", 100)], &t, &asks, 5_000_000);
        assert_eq!(c.state, State::Awaiting);
        assert_eq!(c.state_since, 200);
        let c = run_prose(&reg("idle", 5), &[ev("UserPromptSubmit", 300)], &t, &asks, 5_000_000);
        assert_ne!(c.state, State::Awaiting);
        let c = run_prose(&reg("idle", 5), &[ev("Stop", 250)], &t, &asks, 5_000_000);
        assert_eq!(c.state_since, 250);
    }

    #[test]
    fn a_reply_event_clears_a_judged_ask() {
        let events = [ev("Stop", 0), ev("UserPromptSubmit", 10)];
        let c = run_prose(&reg("idle", 10), &events, &said("Which way?"), &Prose::Asks("Which way?".into()), 20);
        assert_ne!(c.state, State::Awaiting);
    }

    #[test]
    fn an_interrupt_ends_the_ask_whatever_the_verdict() {
        let asks = Prose::Asks("Which way?".into());
        let t = said("Which way?");
        for prose in [&asks, &Prose::Unjudged] {
            let c = run_prose(&reg("idle", 0), &[ev("Stop", 0), ev("MayaInterrupt", 10)], &t, prose, 5_000_000);
            assert_eq!(c.state, State::Idle);
            assert_eq!(c.state_since, TIMEOUT);
        }
        let tail = TranscriptTail { turn_end: Some(crate::transcript::TurnEnd { at_ms: 1_000, text: "Which way?".into() }), ..said("Which way?") };
        let c = run_prose(&reg("idle", 5), &[ev("MayaInterrupt", 10)], &tail, &asks, 5_000_000);
        assert_ne!(c.state, State::Awaiting);
        let again = run_prose(&reg("idle", 0), &[ev("Stop", 0), ev("MayaInterrupt", 10), ev("Stop", 20)], &t, &asks, 5_000_000);
        assert_eq!(again.state, State::Awaiting);
    }

    #[test]
    fn a_busy_session_is_working_whatever_the_verdict() {
        let t = TranscriptTail { turn_end: Some(crate::transcript::TurnEnd { at_ms: 1_000, text: "Which way?".into() }), ..said("Which way?") };
        let c = run_prose(&reg("busy", 2_000), &[], &t, &Prose::Asks("Which way?".into()), 3_000);
        assert_eq!(c.state, State::Working);
    }

    #[test]
    fn busy_registry_without_events_is_working() {
        let c = run(&reg("busy", 1000), &[], &TranscriptTail::default(), 5000);
        assert_eq!(c.state, State::Working);
        assert_eq!(c.state_since, 1000);
        assert_eq!(c.name, "eye-1");
        assert_eq!(c.pid, 7);
    }

    #[test]
    fn registry_sessions_run_under_claude_code() {
        let c = run(&reg("idle", 1), &[], &TranscriptTail::default(), 5);
        assert_eq!(c.harness, crate::model::Harness::ClaudeCode);
        assert_eq!(c.pr, None, "the PR comes from the cache, not from derive");
    }

    #[test]
    fn a_shell_left_running_marks_the_card() {
        let c = run(&reg("shell", 1), &[], &TranscriptTail::default(), 5);
        assert!(c.state == State::Working && c.shell);
        assert!(!run(&reg("busy", 1), &[], &TranscriptTail::default(), 5).shell);
    }

    #[test]
    fn shell_registry_is_working() {
        assert_eq!(run(&reg("shell", 1), &[], &TranscriptTail::default(), 5).state, State::Working);
    }

    #[test]
    fn idle_registry_without_events_is_idle() {
        let c = run(&reg("idle", 1000), &[], &TranscriptTail::default(), 5000);
        assert_eq!(c.state, State::Idle);
        assert_eq!(c.state_since, 1000);
    }

    #[test]
    fn permission_request_is_awaiting_until_post_tool_use() {
        let asked = [tool_ev("PermissionRequest", "Bash", serde_json::json!({"command": "rm -rf build"}), 2000)];
        let c = run(&reg("busy", 1000), &asked, &TranscriptTail::default(), 3000);
        assert_eq!(c.state, State::Awaiting);
        assert_eq!(c.state_since, 2000);
        let aw = c.awaiting.unwrap();
        assert_eq!(aw.kind, AwaitKind::Permission);
        assert_eq!(aw.detail, "Bash: rm -rf build");

        let approved = [asked[0].clone(), tool_ev("PostToolUse", "Bash", serde_json::json!({}), 2500)];
        let c = run(&reg("busy", 1000), &approved, &TranscriptTail::default(), 3000);
        assert_eq!(c.state, State::Working);
        assert!(c.awaiting.is_none());
    }

    #[test]
    fn subagent_post_tool_use_does_not_clear_main_agent_awaiting() {
        let mut sub = tool_ev("PostToolUse", "Bash", serde_json::json!({}), 2500);
        sub.agent_id = Some("agent-x".into());
        let evs = [tool_ev("PermissionRequest", "Bash", serde_json::json!({"command": "rm -rf build"}), 2000), sub];
        let c = run(&reg("busy", 1000), &evs, &TranscriptTail::default(), 3000);
        assert_eq!(c.state, State::Awaiting);
        assert_eq!(c.awaiting.unwrap().detail, "Bash: rm -rf build");
    }

    #[test]
    fn subagent_permission_is_cleared_by_that_subagent_post_tool_use() {
        let mut ask = tool_ev("PermissionRequest", "Bash", serde_json::json!({"command": "ls"}), 2000);
        ask.agent_id = Some("agent-x".into());
        let mut done = tool_ev("PostToolUse", "Bash", serde_json::json!({}), 2500);
        done.agent_id = Some("agent-x".into());
        assert_eq!(run(&reg("busy", 1000), &[ask.clone()], &TranscriptTail::default(), 3000).state, State::Awaiting);
        assert_eq!(run(&reg("busy", 1000), &[ask, done], &TranscriptTail::default(), 3000).state, State::Working);
    }

    #[test]
    fn permission_request_for_ask_user_question_keeps_the_question_text() {
        let evs = [
            tool_ev("PreToolUse", "AskUserQuestion", serde_json::json!({"questions": [{"question": "Which stack?"}]}), 10),
            tool_ev("PermissionRequest", "AskUserQuestion", serde_json::json!({"questions": [{"question": "Which stack?"}]}), 11),
        ];
        let aw = run(&reg("busy", 0), &evs, &TranscriptTail::default(), 20).awaiting.unwrap();
        assert_eq!(aw.kind, AwaitKind::Question);
        assert_eq!(aw.detail, "Which stack?");

        let evs = [tool_ev("PermissionRequest", "ExitPlanMode", serde_json::json!({"plan": "x"}), 11)];
        let aw = run(&reg("busy", 0), &evs, &TranscriptTail::default(), 20).awaiting.unwrap();
        assert_eq!(aw.kind, AwaitKind::Plan);
        assert_eq!(aw.detail, "Plan approval");
    }

    #[test]
    fn subagent_stop_does_not_complete_the_main_session() {
        let mut stop = ev("Stop", 2000);
        stop.agent_id = Some("agent-x".into());
        let c = run(&reg("idle", 2100), &[ev("UserPromptSubmit", 1000), stop], &TranscriptTail::default(), 3000);
        assert_eq!(c.state, State::Idle);
    }

    #[test]
    fn card_reports_whether_the_session_has_an_inbox() {
        let mut r = reg("idle", 0);
        assert!(!run(&r, &[], &TranscriptTail::default(), 1).has_inbox);
        r.messaging_socket_path = Some("/tmp/cc-socks/7.sock".into());
        assert!(run(&r, &[], &TranscriptTail::default(), 1).has_inbox);
    }

    #[test]
    fn permission_denied_clears_awaiting() {
        let evs = [tool_ev("PermissionRequest", "Edit", serde_json::json!({"file_path": "/a/b.rs"}), 1), ev("PermissionDenied", 2)];
        assert_eq!(run(&reg("busy", 0), &evs, &TranscriptTail::default(), 3).state, State::Working);
    }

    #[test]
    fn edit_permission_detail_uses_file_path() {
        let evs = [tool_ev("PermissionRequest", "Edit", serde_json::json!({"file_path": "/a/b.rs"}), 1)];
        assert_eq!(run(&reg("busy", 0), &evs, &TranscriptTail::default(), 3).awaiting.unwrap().detail, "Edit: /a/b.rs");
    }

    #[test]
    fn ask_user_question_is_awaiting_with_question_text() {
        let evs = [tool_ev("PreToolUse", "AskUserQuestion", serde_json::json!({"questions": [{"question": "Which stack?", "header": "Stack", "options": [{"label": "Tauri", "description": "Rust"}]}]}), 10)];
        let c = run(&reg("busy", 0), &evs, &TranscriptTail::default(), 20);
        assert_eq!(c.state, State::Awaiting);
        let aw = c.awaiting.unwrap();
        assert_eq!(aw.kind, AwaitKind::Question);
        assert_eq!(aw.detail, "Which stack?");
        assert_eq!(aw.questions[0].options[0].label, "Tauri");
        assert_eq!(aw.questions[0].own.as_ref().map(|o| o.row), Some(1));
        assert_eq!(aw.questions[0].chat_row, Some(2));
    }

    #[test]
    fn question_without_options_is_still_awaiting_with_no_buttons() {
        let evs = [tool_ev("PreToolUse", "AskUserQuestion", serde_json::json!({}), 10)];
        let aw = run(&reg("busy", 0), &evs, &TranscriptTail::default(), 20).awaiting.unwrap();
        assert_eq!(aw.kind, AwaitKind::Question);
        assert!(aw.questions.is_empty());
    }

    #[test]
    fn exit_plan_mode_is_awaiting_plan() {
        let evs = [tool_ev("PreToolUse", "ExitPlanMode", serde_json::json!({}), 10)];
        let aw = run(&reg("busy", 0), &evs, &TranscriptTail::default(), 20).awaiting.unwrap();
        assert_eq!(aw.kind, AwaitKind::Plan);
    }

    #[test]
    fn other_pre_tool_use_is_not_awaiting() {
        let evs = [tool_ev("PreToolUse", "Bash", serde_json::json!({}), 10)];
        assert_eq!(run(&reg("busy", 0), &evs, &TranscriptTail::default(), 20).state, State::Working);
    }

    #[test]
    fn notification_permission_prompt_counts_as_awaiting() {
        let mut e = ev("Notification", 10);
        e.notification_type = Some("permission_prompt".into());
        let c = run(&reg("busy", 0), &[e], &TranscriptTail::default(), 20);
        assert_eq!(c.state, State::Awaiting);
        assert_eq!(c.awaiting.unwrap().kind, AwaitKind::Permission);
    }

    #[test]
    fn a_maya_interrupt_clears_the_ask() {
        let evs = [tool_ev("PermissionRequest", "Bash", serde_json::json!({"command": "git push"}), 10), ev("MayaInterrupt", 11)];
        let c = run(&reg("busy", 0), &evs, &TranscriptTail::default(), 20);
        assert!(c.awaiting.is_none());
        assert_eq!(c.state, State::Working);
    }

    #[test]
    fn new_prompt_clears_awaiting_question() {
        let evs = [
            tool_ev("PreToolUse", "AskUserQuestion", serde_json::json!({"questions": [{"question": "Q"}]}), 10),
            ev("UserPromptSubmit", 11),
        ];
        assert_eq!(run(&reg("busy", 0), &evs, &TranscriptTail::default(), 20).state, State::Working);
    }

    #[test]
    fn asking_line_finds_the_ask_anywhere_in_the_message() {
        assert_eq!(asking_line("**Which way should I build it?**\n- A: one.\n- B: two.\n\nI recommend A. It is safer. It costs more."), Some("**Which way should I build it?**".into()));
        assert_eq!(asking_line("Option 2 is a question about who opens PRs — your call, not mine.\n\nAnd it isn't collector-specific."), Some("Option 2 is a question about who opens PRs — your call, not mine.".into()));
        assert_eq!(asking_line("I'll only do it if you say so and tell me which thread to use.\n\nOtherwise, try the checks yourself."), Some("I'll only do it if you say so and tell me which thread to use.".into()));
        assert_eq!(asking_line("All green. If you'd like that fixed, I can do it in a separate PR."), Some("If you'd like that fixed, I can do it in a separate PR.".into()));
        assert_eq!(asking_line("It's a single AWS call, and I can make it now if you say go."), Some("It's a single AWS call, and I can make it now if you say go.".into()));
        assert_eq!(asking_line("Run these:\n```\n! cd ~/dev/x && sh a.sh\n```\nAfter you run them, I'll check the policies."), Some("After you run them, I'll check the policies.".into()));
        // Kept from the old test: closing lines, code fences, trailing asides.
        assert_eq!(asking_line("Ready?\n```\ncode\n```"), Some("Ready?".into()));
        assert_eq!(asking_line("Go ahead? [y/n]"), Some("Go ahead? [y/n]".into()));
        assert_eq!(asking_line("**Please review the plan. Which way should I build it?**\n\nI recommend A."), Some("Which way should I build it?".into()));
        assert_eq!(asking_line("**Shall I ship it? It is ready.**"), Some("Shall I ship it?".into()));
        assert_eq!(asking_line("If you’d like that fixed, I can do it."), Some("If you’d like that fixed, I can do it.".into()));
        assert_eq!(asking_line("Run:\n! sh a.sh\nDone."), Some("! sh a.sh".into()));
        let aside = "Approve this plan, or amend any rows? (e.g. \"skip #2\")";
        assert_eq!(asking_line(aside), Some(aside.into()));
        // A question inside code, a table or a quote is not an ask.
        assert_eq!(asking_line("```\nwhy?\n```\nDone."), None);
        assert_eq!(asking_line("| a? | b |\n|---|---|\n\nAll done."), None);
        assert_eq!(asking_line("> Should we?\n\nFixed and pushed."), None);
        assert_eq!(asking_line("Done.\n```\nwhy?"), None);
        assert_eq!(asking_line("All done."), None);
        assert_eq!(asking_line(""), None);
    }

    #[test]
    fn stop_with_a_question_in_the_text_is_awaiting_a_reply_and_never_decays() {
        let evs = [ev("UserPromptSubmit", 1000), ev("Stop", 2000)];
        let tail = TranscriptTail { last_assistant_text: Some("PR body here.\n\nCreate it as drafted?".into()), ..Default::default() };
        let c = run(&reg("idle", 2100), &evs, &tail, 3000);
        assert_eq!(c.state, State::Awaiting);
        assert_eq!(c.state_since, 2000);
        let aw = c.awaiting.unwrap();
        assert_eq!(aw.kind, AwaitKind::Text);
        assert_eq!(aw.detail, "Create it as drafted?");
        assert!(aw.questions.is_empty());
        let later = run(&reg("idle", 2100), &evs, &tail, 2000 + TIMEOUT * 10);
        assert_eq!(later.state, State::Awaiting);
        // A reply clears it.
        let answered = [ev("UserPromptSubmit", 1000), ev("Stop", 2000), ev("UserPromptSubmit", 4000)];
        assert_eq!(run(&reg("busy", 4001), &answered, &tail, 5000).state, State::Working);
        // A Stop whose text does not ask stays Completed.
        let plain = TranscriptTail { last_assistant_text: Some("All done.".into()), ..Default::default() };
        assert_eq!(run(&reg("idle", 2100), &evs, &plain, 3000).state, State::Completed);
    }

    #[test]
    fn stop_with_a_question_while_a_background_shell_runs_is_awaiting() {
        // The background command's tool events after the Stop do not hide the question.
        let evs = [ev("UserPromptSubmit", 1000), ev("Stop", 2000), tool_ev("PostToolUse", "Bash", serde_json::json!({}), 2500)];
        let tail = TranscriptTail { last_assistant_text: Some("Say the word and I'll start with the delete.".into()), ..Default::default() };
        let c = run(&reg("shell", 2100), &evs, &tail, 3000);
        assert_eq!(c.state, State::Awaiting);
        assert_eq!(c.state_since, 2000);
        assert_eq!(c.awaiting.unwrap().kind, AwaitKind::Text);
        // Without a question the background shell keeps it Working.
        let plain = TranscriptTail { last_assistant_text: Some("Watching the deploy.".into()), ..Default::default() };
        assert_eq!(run(&reg("shell", 2100), &evs, &plain, 3000).state, State::Working);
        // A turn still in progress stays Working whatever the last text says.
        assert_eq!(run(&reg("busy", 2100), &evs, &tail, 3000).state, State::Working);
    }

    #[test]
    fn stop_makes_completed_then_decays_to_idle() {
        let evs = [ev("UserPromptSubmit", 1000), ev("Stop", 2000)];
        let c = run(&reg("idle", 2100), &evs, &TranscriptTail::default(), 3000);
        assert_eq!(c.state, State::Completed);
        assert_eq!(c.state_since, 2000);

        let c = run(&reg("idle", 2100), &evs, &TranscriptTail::default(), 2000 + TIMEOUT + 1);
        assert_eq!(c.state, State::Idle);
        assert_eq!(c.state_since, 2000 + TIMEOUT);
    }

    #[test]
    fn stop_then_new_prompt_is_working() {
        let evs = [ev("Stop", 2000), ev("UserPromptSubmit", 3000)];
        let c = run(&reg("busy", 3001), &evs, &TranscriptTail::default(), 4000);
        assert_eq!(c.state, State::Working);
    }

    #[test]
    fn stop_but_registry_still_busy_is_working() {
        let evs = [ev("Stop", 2000)];
        assert_eq!(run(&reg("busy", 1000), &evs, &TranscriptTail::default(), 2001).state, State::Working);
    }

    #[test]
    fn transcript_fallback_detects_question_only_without_events() {
        let t = TranscriptTail {
            last_assistant_text: Some("One question first.".into()),
            open_question: Some(OpenQuestion { kind: AwaitKind::Question, detail: "What?".into(), questions: vec![] }),
            ..Default::default()
        };
        let c = run(&reg("busy", 500), &[], &t, 600);
        assert_eq!(c.state, State::Awaiting);
        assert_eq!(c.awaiting.unwrap().detail, "What?");
        assert_eq!(c.snippet, "One question first.");

        let c = run(&reg("busy", 500), &[tool_ev("PostToolUse", "Bash", serde_json::json!({}), 550)], &t, 600);
        assert_eq!(c.state, State::Working);
    }

    #[test]
    fn snippet_is_truncated_to_200_chars() {
        let t = TranscriptTail { last_assistant_text: Some("y".repeat(500)), ..Default::default() };
        let c = run(&reg("idle", 0), &[], &t, 1);
        assert_eq!(c.snippet.chars().count(), 200);
        assert!(c.snippet.ends_with('…'));
    }

    #[test]
    fn truncate_keeps_short_strings_and_counts_chars() {
        assert_eq!(truncate("héllo", 10), "héllo");
        assert_eq!(truncate("héllo wörld", 5), "héll…");
    }
}
