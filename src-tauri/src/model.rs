use serde::Serialize;

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

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Awaiting {
    pub kind: AwaitKind,
    pub detail: String,
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
    fn card_serialises_camel_case_and_lowercase_enums() {
        let card = Card {
            session_id: "s1".into(),
            pid: 42,
            name: "eye-3b".into(),
            cwd: "/Users/x/dev/eye".into(),
            state: State::Awaiting,
            state_since: 1000,
            snippet: "hi".into(),
            awaiting: Some(Awaiting { kind: AwaitKind::Permission, detail: "Bash: rm -rf".into() }),
            has_inbox: false,
        };
        let json = serde_json::to_value(&card).unwrap();
        assert_eq!(json["sessionId"], "s1");
        assert_eq!(json["stateSince"], 1000);
        assert_eq!(json["state"], "awaiting");
        assert_eq!(json["awaiting"]["kind"], "permission");
    }
}
