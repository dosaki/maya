use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Awaiting,
    Working,
    Completed,
    Idle,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AwaitKind {
    Question,
    Plan,
    Permission,
    /// A question asked in prose at the end of a turn, with no picker in the terminal.
    Text,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub label: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub question: String,
    #[serde(default)]
    pub header: String,
    #[serde(default)]
    pub options: Vec<Choice>,
    #[serde(default)]
    pub multi_select: bool,
}

/// Reads `input["questions"]` from an AskUserQuestion tool input. Anything
/// missing or malformed yields an empty list rather than an error. All or
/// nothing: a partial list would shift indices away from the terminal's picker.
pub fn parse_questions(input: &serde_json::Value) -> Vec<Question> {
    input
        .get("questions")
        .and_then(|q| q.as_array())
        .map(|arr| arr.iter().map(|q| serde_json::from_value::<Question>(q.clone())).collect::<Result<Vec<_>, _>>().unwrap_or_default())
        .unwrap_or_default()
}

/// The agent runner a session belongs to. Only Claude Code has a session
/// source today; a new harness adds a variant here and its own reader.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Harness {
    ClaudeCode,
    Codex,
    Antigravity,
    Grok,
}

/// A new session runs Claude Code unless the caller picks another agent.
impl Default for Harness {
    fn default() -> Self {
        Harness::ClaudeCode
    }
}

/// The pull request for a session directory's current branch.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub number: u64,
    pub url: String,
    /// "open", "draft", "merged" or "closed".
    pub state: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Awaiting {
    pub kind: AwaitKind,
    pub detail: String,
    /// The questions being asked, with their options; empty unless `kind` is `Question`.
    #[serde(default)]
    pub questions: Vec<Question>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub session_id: String,
    pub pid: i32,
    pub name: String,
    pub cwd: String,
    pub state: State,
    /// Epoch millis when the session entered `state`.
    pub state_since: u64,
    /// Last assistant text, trimmed to 200 chars.
    pub snippet: String,
    pub awaiting: Option<Awaiting>,
    /// True when the session has an inbox socket the app can post replies to.
    pub has_inbox: bool,
    pub harness: Harness,
    /// The PR for the directory's branch, once the background lookup has run.
    pub pr: Option<PullRequest>,
    /// Context window usage after the last assistant turn.
    pub context: Option<crate::context::ContextUsage>,
    /// The assistant machine this card came from; None for a local session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    /// That machine's IP address as the main sees it; None for a local session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_address: Option<String>,
    /// That machine's platform as it reported it (`macos`, `linux`,
    /// `windows`); None for a local session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_platform: Option<String>,
    /// The assistant's terminal name for this session (a tmux session), if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<String>,
    /// True when the machine has not reported for a while or is disconnected.
    #[serde(default)]
    pub stale: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_questions_from_tool_input_tolerantly() {
        let v = serde_json::json!({"questions": [
            {"question": "Size?", "header": "Size", "options": [{"label": "S", "description": "small"}, {"label": "L"}]},
            {"question": "Toppings?", "header": "Top", "multiSelect": true, "options": [{"label": "A", "description": "a"}]}
        ]});
        let q = parse_questions(&v);
        assert_eq!(q.len(), 2);
        assert_eq!(q[0].options[1], Choice { label: "L".into(), description: "".into() });
        assert!(!q[0].multi_select);
        assert!(q[1].multi_select);
        assert!(parse_questions(&serde_json::json!({})).is_empty());
        assert!(parse_questions(&serde_json::json!({"questions": "nope"})).is_empty());
        // One bad element must not shift the indices of the others: all or nothing.
        let mixed = serde_json::json!({"questions": [{"header": "no question field"}, {"question": "ok?", "options": []}]});
        assert!(parse_questions(&mixed).is_empty());
        let json = serde_json::to_value(&q[1]).unwrap();
        assert_eq!(json["multiSelect"], true);
    }

    #[test]
    fn card_serialises_camel_case_and_lowercase_enums() {
        let card = Card {
            session_id: "s1".into(),
            pid: 42,
            name: "eye-3b".into(),
            cwd: "/Users/x/dev/eye".into(),
            state: State::Awaiting,
            state_since: 1000,
            snippet: "hi".into(),
            awaiting: Some(Awaiting { kind: AwaitKind::Permission, detail: "Bash: rm -rf".into(), questions: vec![] }),
            has_inbox: false,
            harness: Harness::ClaudeCode,
            pr: Some(PullRequest { number: 781, url: "https://github.com/o/r/pull/781".into(), state: "merged".into() }),
            context: Some(crate::context::ContextUsage { used: 124_000, window: 200_000, percent: 62 }),
            machine: None,
            machine_address: None, machine_platform: None,
            terminal: None,
            stale: false,
        };
        let json = serde_json::to_value(&card).unwrap();
        assert_eq!(json["context"]["percent"], 62);
        assert_eq!(json["context"]["window"], 200_000);
        assert_eq!(json["harness"], "claude-code");
        assert_eq!(json["pr"]["number"], 781);
        assert_eq!(json["pr"]["state"], "merged");
        assert_eq!(json["sessionId"], "s1");
        assert_eq!(json["stateSince"], 1000);
        assert_eq!(json["state"], "awaiting");
        assert_eq!(json["awaiting"]["kind"], "permission");
        assert_eq!(serde_json::to_value(AwaitKind::Text).unwrap(), "text");
    }
}
