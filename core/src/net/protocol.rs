//! The wire protocol between a main Maya and its assistants: JSON text
//! frames with a `type`, plus the pairing code and the HMAC handshake.

use crate::launch::LaunchOptions;
use crate::model::Card;
use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Sha256;
use std::collections::HashMap;

pub const PROTOCOL: u32 = 1;
pub const DEFAULT_PORT: u16 = 4127;
pub const CODE_TTL_MS: u64 = 5 * 60_000;
pub const LOCKOUT_MS: u64 = 5 * 60_000;
pub const LOCKOUT_AFTER: u32 = 3;
/// Larger frames are refused unparsed (a board with attachments stays well under).
pub const MAX_FRAME: usize = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    /// The file's path on the main, exactly as the reply's `Attached file:`
    /// line gives it; the assistant saves the copy under its file name and
    /// points that line at the copy.
    pub name: String,
    /// The file's bytes, base64.
    pub bytes: String,
}

/// Assistant → main.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Up {
    Hello { protocol: u32, app: String, name: String, hostname: String, platform: String, #[serde(default)] id: Option<String> },
    /// The code the user typed, and a nonce the main must answer in
    /// `welcome` to prove it knows the code too.
    Pair { code: String, #[serde(default)] nonce: String },
    /// The answer to the main's challenge, and this assistant's own nonce
    /// for the main to answer in `welcome`.
    Auth { mac: String, nonce: String },
    Board {
        cards: Vec<Card>,
        dirs: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "known_agents")]
        agents: Option<Vec<crate::agents::AgentInfo>>,
    },
    Result { id: u64, ok: bool, #[serde(default)] error: Option<String>, #[serde(default)] data: Option<Value> },
    Ping,
}

/// A board's agents, without the ones this Maya cannot read. A newer Maya
/// may list an agent this one has never heard of; failing on it would fail
/// the whole board and freeze every card from that machine.
fn known_agents<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Vec<crate::agents::AgentInfo>>, D::Error> {
    let raw: Option<Vec<Value>> = Option::deserialize(d)?;
    Ok(raw.map(|list| list.into_iter().filter_map(|a| serde_json::from_value(a).ok()).collect()))
}

/// Main → assistant.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Down {
    Challenge { nonce: String },
    /// `mac` is `mac(token, nonce)` for the assistant's `auth` nonce: the
    /// main's proof that it holds the token. Right after pairing it is
    /// `mac(code, nonce)` for the `pair` nonce: proof it knows the code.
    Welcome { name: String, #[serde(default)] mac: String },
    Paired { id: String, token: String },
    Bye { reason: String },
    Command { id: u64, #[serde(flatten)] kind: CommandKind },
    Pong,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CommandKind {
    Reply { session: String, text: String, #[serde(default)] attachments: Vec<Attachment> },
    Answer { session: String, ask_id: u64, question: usize, option: usize },
    Compact { session: String },
    Close { session: String },
    Rename { session: String, name: String },
    SetOption { session: String, setting: String, value: String },
    CycleMode { session: String },
    Slash { session: String, text: String },
    Start { dir: Option<String>, prompt: String, options: LaunchOptions },
    Resume { dir: String, session: String, #[serde(default)] agent: crate::model::Harness },
    ListResumable { dir: String, #[serde(default)] agent: crate::model::Harness },
    History { session: String },
}

pub fn encode<T: Serialize>(m: &T) -> String {
    serde_json::to_string(m).unwrap_or_default()
}

fn decode<T: for<'de> Deserialize<'de>>(s: &str) -> Result<T, String> {
    if s.len() > MAX_FRAME {
        return Err(format!("frame too large ({} bytes)", s.len()));
    }
    serde_json::from_str(s).map_err(|e| format!("bad message: {e}"))
}

pub fn decode_up(s: &str) -> Result<Up, String> {
    decode(s)
}

pub fn decode_down(s: &str) -> Result<Down, String> {
    decode(s)
}

fn random(n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    rand::rng().fill_bytes(&mut b);
    b
}

pub fn new_token() -> String {
    base64::engine::general_purpose::STANDARD.encode(random(32))
}

pub fn new_nonce() -> String {
    base64::engine::general_purpose::STANDARD.encode(random(16))
}

pub fn new_id() -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random(12))
}

/// Hex HMAC-SHA256 of `nonce` under `token`.
pub fn mac(token: &str, nonce: &str) -> String {
    let mut m = Hmac::<Sha256>::new_from_slice(token.as_bytes()).expect("hmac accepts any key length");
    m.update(nonce.as_bytes());
    m.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

/// Constant-time check of a MAC the other side sent.
pub fn mac_matches(token: &str, nonce: &str, given: &str) -> bool {
    let Ok(given) = (0..given.len()).step_by(2).map(|i| u8::from_str_radix(given.get(i..i + 2).unwrap_or("zz"), 16)).collect::<Result<Vec<u8>, _>>() else {
        return false;
    };
    let mut m = Hmac::<Sha256>::new_from_slice(token.as_bytes()).expect("hmac accepts any key length");
    m.update(nonce.as_bytes());
    m.verify_slice(&given).is_ok()
}

/// Nonces the main has issued; each answers once. The real replay guard is
/// that every challenge carries a fresh random nonce; this log is belt and
/// braces against a nonce ever being accepted twice.
#[derive(Default)]
pub struct NonceLog {
    used: HashMap<String, u64>,
}

impl NonceLog {
    pub fn first_use(&mut self, nonce: &str, now_ms: u64) -> bool {
        self.used.retain(|_, t| now_ms.saturating_sub(*t) < 10 * 60_000);
        self.used.insert(nonce.to_string(), now_ms).is_none()
    }
}

/// The six-digit code shown on the main while pairing is open.
pub struct PairingWindow {
    pub code: String,
    pub expires_at_ms: u64,
}

impl PairingWindow {
    pub fn new(now_ms: u64) -> PairingWindow {
        let n = u32::from_le_bytes(random(4).try_into().unwrap()) % 1_000_000;
        PairingWindow { code: format!("{n:06}"), expires_at_ms: now_ms + CODE_TTL_MS }
    }

    pub fn accepts(&self, code: &str, now_ms: u64) -> bool {
        now_ms <= self.expires_at_ms && code.trim() == self.code
    }
}

/// Wrong pairing codes per address; three in five minutes lock it out.
#[derive(Default)]
pub struct Attempts {
    failures: HashMap<String, Vec<u64>>,
}

impl Attempts {
    pub fn failed(&mut self, addr: &str, now_ms: u64) {
        // Old failures age out, and an address with none left is forgotten.
        self.failures.retain(|_, v| {
            v.retain(|t| now_ms.saturating_sub(*t) < LOCKOUT_MS);
            !v.is_empty()
        });
        self.failures.entry(addr.to_string()).or_default().push(now_ms);
    }

    pub fn locked(&self, addr: &str, now_ms: u64) -> bool {
        self.failures.get(addr).map_or(false, |v| v.iter().filter(|t| now_ms.saturating_sub(**t) < LOCKOUT_MS).count() >= LOCKOUT_AFTER as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_without_an_agent_means_claude_code() {
        let k: CommandKind = serde_json::from_str(r#"{"kind":"resume","dir":"maya","session":"abc"}"#).unwrap();
        assert_eq!(k, CommandKind::Resume { dir: "maya".into(), session: "abc".into(), agent: crate::model::Harness::ClaudeCode });
        let k: CommandKind = serde_json::from_str(r#"{"kind":"list_resumable","dir":"maya","agent":"grok"}"#).unwrap();
        assert_eq!(k, CommandKind::ListResumable { dir: "maya".into(), agent: crate::model::Harness::Grok });
    }

    #[test]
    fn messages_round_trip_with_a_type_tag() {
        let up = Up::Hello { protocol: PROTOCOL, app: "0.1.0".into(), name: "laptop".into(), hostname: "tiagos-mbp".into(), platform: "macos".into(), id: None };
        let s = encode(&up);
        assert!(s.contains("\"type\":\"hello\""), "{s}");
        assert_eq!(decode_up(&s).unwrap(), up);
        let down = Down::Command { id: 7, kind: CommandKind::Reply { session: "s1".into(), text: "go".into(), attachments: vec![] } };
        let s = encode(&down);
        assert!(s.contains("\"type\":\"command\"") && s.contains("\"kind\":\"reply\"") && s.contains("\"id\":7"), "{s}");
        assert_eq!(decode_down(&s).unwrap(), down);
        let result = Up::Result { id: 7, ok: false, error: Some("no".into()), data: None };
        assert_eq!(decode_up(&encode(&result)).unwrap(), result);
        let slash = Down::Command { id: 8, kind: CommandKind::Slash { session: "s1".into(), text: "/review".into() } };
        let s = encode(&slash);
        assert!(s.contains("\"kind\":\"slash\"") && s.contains("\"text\":\"/review\""), "{s}");
        assert_eq!(decode_down(&s).unwrap(), slash);
        // The handshake is mutual: `auth` carries the assistant's nonce and `welcome` the main's proof.
        let auth = Up::Auth { mac: "ab".into(), nonce: "n1".into() };
        let s = encode(&auth);
        assert!(s.contains("\"type\":\"auth\"") && s.contains("\"nonce\":\"n1\""), "{s}");
        assert_eq!(decode_up(&s).unwrap(), auth);
        let welcome = Down::Welcome { name: "desk".into(), mac: "cd".into() };
        assert_eq!(decode_down(&encode(&welcome)).unwrap(), welcome);
        assert_eq!(decode_down(r#"{"type":"welcome","name":"desk"}"#).unwrap(), Down::Welcome { name: "desk".into(), mac: String::new() }, "a welcome without a proof still decodes; the assistant refuses it");
        // Pairing is mutual too: `pair` carries a nonce the main answers in `welcome`.
        let pair = Up::Pair { code: "123456".into(), nonce: "n2".into() };
        let s = encode(&pair);
        assert!(s.contains("\"type\":\"pair\"") && s.contains("\"code\":\"123456\"") && s.contains("\"nonce\":\"n2\""), "{s}");
        assert_eq!(decode_up(&s).unwrap(), pair);
        assert!(decode_up(r#"{"type":"auth","mac":"ab"}"#).is_err(), "an auth without the assistant's nonce is refused");
    }

    #[test]
    fn a_board_from_an_older_maya_has_no_agents() {
        let old = r#"{"type":"board","cards":[],"dirs":["proj"]}"#;
        let Up::Board { agents, .. } = serde_json::from_str::<Up>(old).unwrap() else { panic!() };
        assert_eq!(agents, None);
        let new = encode(&Up::Board { cards: vec![], dirs: vec![], agents: Some(vec![crate::agents::claude()]) });
        assert!(new.contains("\"agents\":[{\"harness\":\"claude-code\""), "{new}");
    }

    #[test]
    fn a_card_from_an_agent_this_maya_does_not_know_still_decodes() {
        use crate::model::{Card, Harness, State};
        let card = Card { session_id: "s1".into(), pid: 1, name: "s1".into(), cwd: "/x".into(), state: State::Idle, state_since: 0, snippet: String::new(), awaiting: None, has_inbox: false, harness: Harness::Kiro, pr: None, context: None, machine: None, machine_address: None, machine_platform: None, terminal: None, stale: false };
        let text = encode(&Up::Board { cards: vec![card], dirs: vec![], agents: None }).replace("\"harness\":\"kiro\"", "\"harness\":\"future-agent\"");
        let Up::Board { cards, .. } = decode_up(&text).unwrap() else { panic!() };
        assert_eq!(cards[0].harness, Harness::Other);
    }

    #[test]
    fn a_board_listing_an_agent_this_maya_does_not_know_still_decodes() {
        // A newer Maya with one more agent must not freeze every card from its machine.
        let board = r#"{"type":"board","cards":[],"dirs":[],"agents":[{"harness":"future","models":[]},{"harness":"claude-code","models":[]}]}"#;
        let Up::Board { agents, .. } = decode_up(board).unwrap() else { panic!() };
        let claude = crate::agents::AgentInfo { harness: crate::model::Harness::ClaudeCode, models: vec![], efforts: vec![], modes: vec![] };
        assert_eq!(agents, Some(vec![claude]), "the unknown one is dropped, the rest kept");
        let Up::Board { agents, .. } = decode_up(r#"{"type":"board","cards":[],"dirs":[]}"#).unwrap() else { panic!() };
        assert_eq!(agents, None, "an old board still has none");
        let Up::Board { agents, .. } = decode_up(r#"{"type":"board","cards":[],"dirs":[],"agents":null}"#).unwrap() else { panic!() };
        assert_eq!(agents, None);
    }

    #[test]
    fn decode_rejects_garbage_and_wrong_types() {
        assert!(decode_up("not json").is_err());
        assert!(decode_up(r#"{"type":"dance"}"#).is_err());
        assert!(decode_up(r#"{"type":"result","id":"seven"}"#).is_err());
        assert!(decode_down(r#"{"type":"command","id":1,"kind":"explode"}"#).is_err());
        assert!(decode_up(&"x".repeat(MAX_FRAME + 1)).is_err(), "oversized frames are refused before parsing");
    }

    #[test]
    fn pairing_codes_are_six_digits_and_expire() {
        let w = PairingWindow::new(1_000);
        assert_eq!(w.code.len(), 6);
        assert!(w.code.chars().all(|c| c.is_ascii_digit()));
        assert!(w.accepts(&w.code, 1_000 + CODE_TTL_MS));
        assert!(!w.accepts(&w.code, 1_000 + CODE_TTL_MS + 1));
        assert!(!w.accepts("000000", 1_000) || w.code == "000000");
        assert!(w.accepts(&format!(" {} ", w.code), 2_000), "whitespace is ignored");
    }

    #[test]
    fn mac_rejects_wrong_token_and_reused_nonce() {
        let token = new_token();
        let nonce = new_nonce();
        let good = mac(&token, &nonce);
        assert!(mac_matches(&token, &nonce, &good));
        assert!(!mac_matches(&new_token(), &nonce, &good));
        assert!(!mac_matches(&token, &new_nonce(), &good));
        assert!(!mac_matches(&token, &nonce, ""));
        let mut seen = NonceLog::default();
        assert!(seen.first_use(&nonce, 1_000));
        assert!(!seen.first_use(&nonce, 2_000), "a nonce answers once");
    }

    #[test]
    fn three_wrong_codes_lock_an_address_out() {
        let mut a = Attempts::default();
        for _ in 0..2 {
            a.failed("10.0.0.5", 1_000);
        }
        assert!(!a.locked("10.0.0.5", 1_000));
        a.failed("10.0.0.5", 1_000);
        assert!(a.locked("10.0.0.5", 1_000));
        assert!(!a.locked("10.0.0.6", 1_000));
        assert!(!a.locked("10.0.0.5", 1_000 + LOCKOUT_MS + 1));
        a.failed("10.0.0.7", 1_000 + LOCKOUT_MS + 1);
        assert_eq!(a.failures.len(), 1, "addresses whose failures aged out are pruned");
    }

    #[test]
    fn tokens_and_nonces_are_random_and_long_enough() {
        assert_ne!(new_token(), new_token());
        assert!(base64::engine::general_purpose::STANDARD.decode(new_token()).unwrap().len() == 32);
        assert!(base64::engine::general_purpose::STANDARD.decode(new_nonce()).unwrap().len() == 16);
    }
}
