//! The rows of Claude Code's own menus, so Maya can press the right one.

use crate::events::HookEvent;
use crate::model::{OwnRow, Question};

/// Adds the rows Claude Code puts after a question's options: "Type
/// something", then "Chat about this". A multi-select question has a Submit
/// row (Next before the last question) between them, and Maya does not type
/// on its own row yet, so it gets only the chat row. The order is Claude
/// Code's as of 2.1.316.
pub fn ask_rows(q: &mut Question) {
    let own = q.options.len();
    if q.multi_select {
        q.chat_row = Some(own + 2);
        return;
    }
    q.own = Some(OwnRow { label: "Type something".to_string(), row: own });
    q.chat_row = Some(own + 1);
}

/// The menu of a PermissionRequest. The CLI's rows for it are not verified yet, so the prompt stays answered in the terminal.
pub fn permission_question(_e: &HookEvent) -> Option<Question> {
    None
}

/// The menu of an ExitPlanMode prompt. The CLI's rows for it are not verified yet, so the prompt stays answered in the terminal.
pub fn plan_question(_e: &HookEvent) -> Option<Question> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Choice;
    use serde_json::json;

    fn q(n: usize, multi: bool) -> Question {
        Question {
            question: "Which?".into(),
            header: "H".into(),
            options: (0..n).map(|i| Choice { label: format!("O{i}"), description: String::new() }).collect(),
            multi_select: multi,
            own: None,
            chat_row: None,
        }
    }

    fn event(name: &str, tool: &str, input: serde_json::Value) -> HookEvent {
        serde_json::from_value(json!({"session_id": "s", "hook_event_name": name, "tool_name": tool, "tool_input": input})).unwrap()
    }

    #[test]
    fn ask_rows_follow_the_cli() {
        let mut one = q(2, false);
        ask_rows(&mut one);
        assert_eq!(one.own, Some(OwnRow { label: "Type something".into(), row: 2 }));
        assert_eq!(one.chat_row, Some(3));
    }

    #[test]
    fn a_multi_select_question_gets_its_chat_row_past_own_and_submit() {
        let mut m = q(3, true);
        ask_rows(&mut m);
        // Maya does not type on a multi-select's own row yet, so it has none.
        assert_eq!((m.own, m.chat_row), (None, Some(5)));
    }

    #[test]
    fn unknown_shapes_give_no_menu() {
        let perm = event("PermissionRequest", "Bash", json!({"command": "ls"}));
        let plan = event("PreToolUse", "ExitPlanMode", json!({"plan": "Do it"}));
        assert_eq!(permission_question(&perm), None);
        assert_eq!(plan_question(&plan), None);
    }
}
