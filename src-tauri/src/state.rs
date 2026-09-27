use crate::events::HookEvent;
use crate::model::{AwaitKind, Awaiting, Card, State};
use crate::registry::RegistrySession;
use crate::transcript::TranscriptTail;

pub const SNIPPET_CHARS: usize = 200;

pub struct DeriveInput<'a> {
    pub registry: &'a RegistrySession,
    pub events: &'a [HookEvent],
    pub transcript: &'a TranscriptTail,
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
        Some("AskUserQuestion") => Awaiting { kind: AwaitKind::Question, detail: question_detail(e) },
        Some("ExitPlanMode") => Awaiting { kind: AwaitKind::Plan, detail: "Plan approval".to_string() },
        _ => Awaiting { kind: AwaitKind::Permission, detail: permission_detail(e) },
    }
}

pub fn derive(i: &DeriveInput) -> Card {
    let r = i.registry;
    // (what is awaited, when, which agent asked: None = the main agent)
    let mut awaiting: Option<(Awaiting, u64, Option<String>)> = None;
    // (is_stop, timestamp) of the most recent main-agent Stop or UserPromptSubmit.
    let mut last_turn: Option<(bool, u64)> = None;

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
                    awaiting = Some((Awaiting { kind: AwaitKind::Permission, detail: "Permission prompt".to_string() }, e.received_at, e.agent_id.clone()));
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
                last_turn = Some((true, e.received_at));
            }
            "UserPromptSubmit" if e.agent_id.is_none() => {
                awaiting = None;
                last_turn = Some((false, e.received_at));
            }
            _ => {}
        }
    }

    if i.events.is_empty() {
        if let Some(q) = &i.transcript.open_question {
            awaiting = Some((Awaiting { kind: q.kind, detail: q.detail.clone() }, r.status_updated_at, None));
        }
    }

    let is_busy = matches!(r.status.as_str(), "busy" | "shell");

    let (state, state_since, aw) = if let Some((a, ts, _)) = awaiting {
        (State::Awaiting, ts, Some(a))
    } else if is_busy {
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
        derive(&DeriveInput { registry: r, events, transcript: t, now_ms: now, completed_timeout_ms: TIMEOUT })
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
        let evs = [tool_ev("PreToolUse", "AskUserQuestion", serde_json::json!({"questions": [{"question": "Which stack?"}]}), 10)];
        let c = run(&reg("busy", 0), &evs, &TranscriptTail::default(), 20);
        assert_eq!(c.state, State::Awaiting);
        let aw = c.awaiting.unwrap();
        assert_eq!(aw.kind, AwaitKind::Question);
        assert_eq!(aw.detail, "Which stack?");
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
    fn new_prompt_clears_awaiting_question() {
        let evs = [
            tool_ev("PreToolUse", "AskUserQuestion", serde_json::json!({"questions": [{"question": "Q"}]}), 10),
            ev("UserPromptSubmit", 11),
        ];
        assert_eq!(run(&reg("busy", 0), &evs, &TranscriptTail::default(), 20).state, State::Working);
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
            open_question: Some(OpenQuestion { kind: AwaitKind::Question, detail: "What?".into() }),
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
        let t = TranscriptTail { last_assistant_text: Some("y".repeat(500)), open_question: None };
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
