//! JSON-RPC envelopes, one object per line: what ACP agents (`"jsonrpc":
//! "2.0"` on every message) and Codex's app server (no version field)
//! both speak.

use serde_json::{json, Value};

pub const METHOD_NOT_FOUND: i64 = -32601;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Envelope {
    /// Every message carries `"jsonrpc": "2.0"` (ACP).
    Versioned,
    /// No version field (Codex's app server).
    Bare,
}

impl Envelope {
    fn line(self, mut v: Value) -> String {
        if self == Envelope::Versioned {
            v["jsonrpc"] = json!("2.0");
        }
        format!("{v}\n")
    }

    pub fn request(self, id: u64, method: &str, params: Value) -> String {
        self.line(json!({"id": id, "method": method, "params": params}))
    }

    /// A notification; `None` leaves `params` out.
    pub fn notification(self, method: &str, params: Option<Value>) -> String {
        let mut v = json!({"method": method});
        if let Some(p) = params {
            v["params"] = p;
        }
        self.line(v)
    }

    pub fn reply(self, id: &Value, result: Value) -> String {
        self.line(json!({"id": id, "result": result}))
    }

    pub fn reply_error(self, id: &Value, code: i64, message: &str) -> String {
        self.line(json!({"id": id, "error": {"code": code, "message": message}}))
    }
}

/// One line from the other side, sorted.
#[derive(Debug, PartialEq)]
pub enum Incoming {
    Request { id: Value, method: String, params: Value },
    Notification { method: String, params: Value },
    /// An answer to one of ours. `id` is None when it is not a number (ours
    /// always are); `error` is the error's message.
    Response { id: Option<u64>, result: Value, error: Option<String> },
}

/// None for a line that is not JSON or not a message (a JSON value that
/// is not an object included: nothing here indexes into one mutably).
pub fn parse(line: &str) -> Option<Incoming> {
    let Value::Object(mut v) = serde_json::from_str::<Value>(line.trim()).ok()? else { return None };
    let id = v.remove("id").filter(|i| !i.is_null());
    let params = v.remove("params").unwrap_or_default();
    match (v.get("method").and_then(Value::as_str), id) {
        (Some(m), Some(id)) => Some(Incoming::Request { id, method: m.to_string(), params }),
        (Some(m), None) => Some(Incoming::Notification { method: m.to_string(), params }),
        (None, Some(id)) => {
            let error = v.get("error").filter(|e| !e.is_null()).map(|e| e["message"].as_str().unwrap_or("an error").to_string());
            Some(Incoming::Response { id: id.as_u64(), result: v.remove("result").unwrap_or_default(), error })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json_of(line: &str) -> Value {
        assert!(line.ends_with('\n'));
        serde_json::from_str(line.trim_end()).unwrap()
    }

    #[test]
    fn a_versioned_envelope_says_2_0_and_a_bare_one_does_not() {
        let v = json_of(&Envelope::Versioned.request(3, "session/new", json!({"a": 1})));
        assert_eq!(v, json!({"jsonrpc": "2.0", "id": 3, "method": "session/new", "params": {"a": 1}}));
        let v = json_of(&Envelope::Bare.request(3, "thread/start", json!({})));
        assert_eq!(v, json!({"id": 3, "method": "thread/start", "params": {}}));
        assert_eq!(json_of(&Envelope::Bare.notification("initialized", None)), json!({"method": "initialized"}));
        assert_eq!(json_of(&Envelope::Versioned.notification("session/cancel", Some(json!({"sessionId": "s"})))), json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": "s"}}));
        assert_eq!(json_of(&Envelope::Bare.reply(&json!("r1"), json!({"ok": true}))), json!({"id": "r1", "result": {"ok": true}}));
        assert_eq!(json_of(&Envelope::Bare.reply_error(&json!(7), METHOD_NOT_FOUND, "nope")), json!({"id": 7, "error": {"code": -32601, "message": "nope"}}));
    }

    #[test]
    fn incoming_lines_are_sorted_by_id_and_method() {
        assert_eq!(parse(r#"{"id":"r7","method":"fs/read","params":{"p":1}}"#), Some(Incoming::Request { id: json!("r7"), method: "fs/read".into(), params: json!({"p": 1}) }));
        assert_eq!(parse(r#"{"method":"turn/started","params":{"x":1},"id":null}"#), Some(Incoming::Notification { method: "turn/started".into(), params: json!({"x": 1}) }));
        assert_eq!(parse(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s"}}"#), Some(Incoming::Response { id: Some(2), result: json!({"sessionId": "s"}), error: None }));
        assert_eq!(parse(r#"{"id":2,"error":{"code":-1,"message":"bad"}}"#), Some(Incoming::Response { id: Some(2), result: Value::Null, error: Some("bad".into()) }));
        assert_eq!(parse("not json"), None);
        assert_eq!(parse("{}"), None);
        for not_an_object in ["42", "[1]", "true", "\"x\"", "null"] {
            assert_eq!(parse(not_an_object), None, "{not_an_object}");
        }
    }
}
