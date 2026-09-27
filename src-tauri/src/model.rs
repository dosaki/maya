use serde::{Deserialize, Serialize};

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Awaiting,
    Working,
    Completed,
    Idle,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AwaitKind {
    Question,
    Plan,
    Permission,
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

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Awaiting {
    pub kind: AwaitKind,
    pub detail: String,
    /// The questions being asked, with their options; empty unless `kind` is `Question`.
    #[serde(default)]
    pub questions: Vec<Question>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
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
        };
        let json = serde_json::to_value(&card).unwrap();
        assert_eq!(json["sessionId"], "s1");
        assert_eq!(json["stateSince"], 1000);
        assert_eq!(json["state"], "awaiting");
        assert_eq!(json["awaiting"]["kind"], "permission");
    }
}
