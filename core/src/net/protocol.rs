//! The wire protocol between a main Maya and its assistants: JSON messages
//! with a `type`, each sealed in a `secure::Channel`, plus the pairing code.

use crate::launch::LaunchOptions;
use crate::model::Card;
use base64::Engine;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub const PROTOCOL: u32 = 2;
pub const DEFAULT_PORT: u16 = 4127;
pub const CODE_TTL_MS: u64 = 5 * 60_000;
pub const LOCKOUT_MS: u64 = 5 * 60_000;
pub const LOCKOUT_AFTER: u32 = 3;
pub const PAIR_TRIES: u32 = 3;
/// Larger frames are refused unparsed (a board with attachments stays well under).
pub const MAX_FRAME: usize = 64 * 1024 * 1024;
/// Frames before `welcome` are small; a stranger cannot make a Maya buffer more.
pub const PRE_AUTH_FRAME: usize = 64 * 1024;
/// A Maya with this cap follows the main to a new host on `move`.
pub const CAP_FOLLOW: &str = "follow";
/// A Maya with this cap can become a main when sent `promote`.
pub const CAP_LEAD: &str = "lead";
/// A Maya with this cap wants the main's board (`mirror`) and sends `request`s.
pub const CAP_MIRROR: &str = "mirror";
/// A main with this cap interprets voice commands for a mirroring Maya
/// (`Request::Interpret`, `Request::Execute`).
pub const CAP_VOICE: &str = "voice";

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
    Hello {
        protocol: u32,
        app: String,
        name: String,
        hostname: String,
        platform: String,
        /// What this Maya can do (`CAP_FOLLOW`, `CAP_LEAD`); an older Maya sends none.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        caps: Vec<String>,
    },
    Board {
        cards: Vec<Card>,
        dirs: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "known_agents")]
        agents: Option<Vec<crate::agents::AgentInfo>>,
        /// What was running on this machine when its Maya last stopped; an older Maya sends none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        remembered: Option<Vec<crate::remembered::Remembered>>,
    },
    Result { id: u64, ok: bool, #[serde(default)] error: Option<String>, #[serde(default)] data: Option<Value> },
    /// Act on the pool through the main (a mirroring Maya only). `machine`
    /// names the target for kinds that need one; the main's own name, or
    /// none, means the main.
    Request { id: u64, #[serde(default)] machine: Option<String>, request: Request },
    Ping,
}

/// A board's agents, without the ones this Maya cannot read. A newer Maya
/// may list an agent this one has never heard of; failing on it would fail
/// the whole board and freeze every card from that machine.
fn known_agents<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Vec<crate::agents::AgentInfo>>, D::Error> {
    let raw: Option<Vec<Value>> = Option::deserialize(d)?;
    Ok(raw.map(|list| list.into_iter().filter_map(|a| serde_json::from_value::<crate::agents::AgentInfo>(a).ok()).filter(|a| a.harness != crate::model::Harness::Other).collect()))
}

/// Main → assistant.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Down {
    /// `mirror` answers a Maya that asked with `CAP_MIRROR`: whether the main
    /// will mirror to it. Absent for anyone else.
    Welcome {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mirror: Option<bool>,
        /// What this main can do for a mirroring Maya (`CAP_VOICE`); an older main sends none.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        caps: Vec<String>,
    },
    Bye { reason: String },
    Command { id: u64, #[serde(flatten)] kind: CommandKind },
    /// The main is now at `host:port` with the public key `key`: pin it and
    /// reconnect there. The main closes the connection after it.
    Move { host: String, port: u16, key: String },
    /// The main's board for a mirroring Maya; `allowed: false` (and nothing
    /// else) once the main's switch for it is off.
    Mirror {
        allowed: bool,
        #[serde(default)]
        cards: Vec<Card>,
        #[serde(default)]
        machines: Vec<crate::net::routing::MachineInfo>,
        /// The main's merged "was running" list; its own rows carry its name.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        remembered: Vec<crate::remembered::RememberedRow>,
    },
    /// The answer to the `request` with this id.
    Response { id: u64, ok: bool, #[serde(default)] error: Option<String>, #[serde(default)] data: Option<Value> },
    Pong,
}

/// One turn of a phone's voice conversation, oldest first.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceTurn {
    /// "user" or "maya".
    pub who: String,
    pub text: String,
}

/// What a mirroring Maya asks of the main: a command on the pool (never
/// `Promote`), or a listing the main's own page reads locally.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Request {
    Command(CommandKind),
    ListDirs,
    ListAgents,
    /// A spoken command for the main's agent; `history` is the phone's
    /// conversation so far. Answered with `{say, pending?: {say, action}}`.
    Interpret { command: String, #[serde(default)] history: Vec<VoiceTurn> },
    /// A validated action the user confirmed by voice; the main validates it
    /// again against its board before running it. Answered with `{say}`.
    Execute { action: Value },
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
    /// Forget these remembered sessions (Dismiss, Forget unticked).
    Forget { sessions: Vec<(crate::model::Harness, String)> },
    /// Become the main with this paired list (public keys only).
    Promote { list: Vec<crate::config::PairedAssistant> },
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

pub(crate) fn random(n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    rand::rng().fill_bytes(&mut b);
    b
}

pub fn new_id() -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random(12))
}

/// The six-digit code shown on the main while pairing is open; it allows
/// `PAIR_TRIES` pairings that start, from any address.
pub struct PairingWindow {
    pub code: String,
    pub expires_at_ms: u64,
    pub tries_left: u32,
}

impl PairingWindow {
    pub fn new(now_ms: u64) -> PairingWindow {
        let n = u32::from_le_bytes(random(4).try_into().unwrap()) % 1_000_000;
        PairingWindow { code: format!("{n:06}"), expires_at_ms: now_ms + CODE_TTL_MS, tries_left: PAIR_TRIES }
    }

    /// Takes one try for a pairing that is starting; false once the code
    /// expired or its tries are gone.
    pub fn take_try(&mut self, now_ms: u64) -> bool {
        if now_ms > self.expires_at_ms || self.tries_left == 0 {
            return false;
        }
        self.tries_left -= 1;
        true
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
    fn requests_responses_and_mirrors_round_trip_and_older_frames_still_decode() {
        use crate::net::routing::MachineInfo;
        let req = Up::Request { id: 7, machine: Some("laptop".into()), request: Request::Command(CommandKind::Compact { session: "s1".into() }) };
        assert_eq!(decode_up(&encode(&req)).unwrap(), req);
        let dirs = Up::Request { id: 8, machine: None, request: Request::ListDirs };
        assert_eq!(decode_up(&encode(&dirs)).unwrap(), dirs);
        let resp = Down::Response { id: 7, ok: false, error: Some("no".into()), data: None };
        assert_eq!(decode_down(&encode(&resp)).unwrap(), resp);
        let mirror = Down::Mirror { allowed: true, cards: vec![], machines: vec![MachineInfo { name: "desk".into(), hostname: "desk".into(), platform: "macos".into(), connected: true }], remembered: vec![] };
        assert_eq!(decode_down(&encode(&mirror)).unwrap(), mirror);
        assert_eq!(decode_down(r#"{"type":"mirror","allowed":false}"#).unwrap(), Down::Mirror { allowed: false, cards: vec![], machines: vec![], remembered: vec![] });
        let offered = Down::Welcome { name: "desk".into(), mirror: Some(true), caps: vec![] };
        assert_eq!(decode_down(&encode(&offered)).unwrap(), offered);
        assert!(!encode(&Down::Welcome { name: "desk".into(), mirror: None, caps: vec![] }).contains("mirror"), "no offer, no field");
        assert_eq!(decode_down(r#"{"type":"welcome","name":"desk"}"#).unwrap(), Down::Welcome { name: "desk".into(), mirror: None, caps: vec![] });
    }

    #[test]
    fn voice_requests_round_trip() {
        let ask = Up::Request { id: 7, machine: None, request: Request::Interpret { command: "what needs me".into(), history: vec![VoiceTurn { who: "user".into(), text: "hi".into() }, VoiceTurn { who: "maya".into(), text: "Hello.".into() }] } };
        assert_eq!(decode_up(&encode(&ask)).unwrap(), ask);
        let run = Up::Request { id: 8, machine: None, request: Request::Execute { action: serde_json::json!({"kind": "answer", "session": "s1", "option": 1}) } };
        assert_eq!(decode_up(&encode(&run)).unwrap(), run);
    }

    #[test]
    fn welcome_carries_caps_only_when_there_are_some() {
        let voiced = Down::Welcome { name: "desk".into(), mirror: Some(true), caps: vec![CAP_VOICE.into()] };
        assert_eq!(decode_down(&encode(&voiced)).unwrap(), voiced);
        assert!(!encode(&Down::Welcome { name: "desk".into(), mirror: None, caps: vec![] }).contains("caps"));
    }

    #[test]
    fn an_old_welcome_without_caps_still_decodes() {
        assert_eq!(decode_down(r#"{"type":"welcome","name":"desk","mirror":true}"#).unwrap(), Down::Welcome { name: "desk".into(), mirror: Some(true), caps: vec![] });
    }

    #[test]
    fn resume_without_an_agent_means_claude_code() {
        let k: CommandKind = serde_json::from_str(r#"{"kind":"resume","dir":"maya","session":"abc"}"#).unwrap();
        assert_eq!(k, CommandKind::Resume { dir: "maya".into(), session: "abc".into(), agent: crate::model::Harness::ClaudeCode });
        let k: CommandKind = serde_json::from_str(r#"{"kind":"list_resumable","dir":"maya","agent":"grok"}"#).unwrap();
        assert_eq!(k, CommandKind::ListResumable { dir: "maya".into(), agent: crate::model::Harness::Grok });
    }

    #[test]
    fn messages_round_trip_with_a_type_tag() {
        let up = Up::Hello { protocol: PROTOCOL, app: "0.1.0".into(), name: "laptop".into(), hostname: "tiagos-mbp".into(), platform: "macos".into(), caps: vec![] };
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
        let welcome = Down::Welcome { name: "desk".into(), mirror: None, caps: vec![] };
        assert_eq!(decode_down(&encode(&welcome)).unwrap(), welcome);
    }

    #[test]
    fn a_board_from_an_older_maya_has_no_agents() {
        let old = r#"{"type":"board","cards":[],"dirs":["proj"]}"#;
        let Up::Board { agents, .. } = serde_json::from_str::<Up>(old).unwrap() else { panic!() };
        assert_eq!(agents, None);
        let new = encode(&Up::Board { cards: vec![], dirs: vec![], agents: Some(vec![crate::agents::claude()]), remembered: None });
        assert!(new.contains("\"agents\":[{\"harness\":\"claude-code\""), "{new}");
    }

    #[test]
    fn a_card_from_an_agent_this_maya_does_not_know_still_decodes() {
        use crate::model::{Card, Harness, State};
        let card = Card { session_id: "s1".into(), pid: 1, name: "s1".into(), cwd: "/x".into(), state: State::Idle, state_since: 0, snippet: String::new(), awaiting: None, has_inbox: false, harness: Harness::Kiro, pr: None, context: None, machine: None, machine_address: None, machine_platform: None, terminal: None, stale: false, model: None };
        let text = encode(&Up::Board { cards: vec![card], dirs: vec![], agents: None, remembered: None }).replace("\"harness\":\"kiro\"", "\"harness\":\"future-agent\"");
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
    fn pairing_codes_are_six_digits_with_three_tries_until_they_expire() {
        let mut w = PairingWindow::new(1_000);
        assert_eq!(w.code.len(), 6);
        assert!(w.code.chars().all(|c| c.is_ascii_digit()));
        assert!(w.take_try(1_000 + CODE_TTL_MS));
        assert!(w.take_try(2_000));
        assert!(w.take_try(2_000));
        assert!(!w.take_try(2_000), "three tries, then none");
        let mut late = PairingWindow::new(1_000);
        assert!(!late.take_try(1_000 + CODE_TTL_MS + 1), "expired");
        assert_eq!(late.tries_left, PAIR_TRIES, "an expired code takes no try");
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
    fn a_hello_without_caps_has_none_and_caps_round_trip() {
        let old = r#"{"type":"hello","protocol":2,"app":"0.15.0","name":"n","hostname":"h","platform":"macos"}"#;
        let Up::Hello { caps, .. } = decode_up(old).unwrap() else { panic!() };
        assert!(caps.is_empty());
        let hello = Up::Hello { protocol: PROTOCOL, app: "x".into(), name: "n".into(), hostname: "h".into(), platform: "macos".into(), caps: vec![CAP_FOLLOW.into(), CAP_LEAD.into()] };
        let s = encode(&hello);
        assert!(s.contains(r#""caps":["follow","lead"]"#), "{s}");
        assert_eq!(decode_up(&s).unwrap(), hello);
    }

    #[test]
    fn move_carries_the_new_main_s_key_and_promote_carries_the_list() {
        let mv = Down::Move { host: "192.168.55.70".into(), port: 4127, key: "a2V5".into() };
        let s = encode(&mv);
        assert!(s.contains(r#""type":"move""#) && s.contains(r#""key":"a2V5""#), "{s}");
        assert_eq!(decode_down(&s).unwrap(), mv);
        let entry = crate::config::PairedAssistant { id: "a1".into(), name: "laptop".into(), hostname: "h".into(), platform: "macos".into(), key: "a2V5".into(), address: String::new(), last_seen: None, mirror: false };
        let p = Down::Command { id: 3, kind: CommandKind::Promote { list: vec![entry] } };
        let s = encode(&p);
        assert!(s.contains(r#""kind":"promote""#) && s.contains(r#""key":"a2V5""#) && !s.contains("token"), "{s}");
        assert_eq!(decode_down(&s).unwrap(), p);
    }

    fn was(id: &str) -> crate::remembered::Remembered {
        crate::remembered::Remembered { agent: crate::model::Harness::ClaudeCode, session_id: id.into(), dir: "/Users/x/dev/calibre".into(), name: "calibre-88".into(), first_seen_ms: 1, last_seen_ms: 2, gone_at_ms: None, offered: true, offered_at_ms: None, error: None }
    }

    #[test]
    fn a_board_carries_what_was_running_and_an_older_board_has_none() {
        let board = Up::Board { cards: vec![], dirs: vec![], agents: None, remembered: Some(vec![was("s1")]) };
        let s = encode(&board);
        assert!(s.contains(r#""remembered":[{"agent":"claude-code","sessionId":"s1""#), "{s}");
        assert_eq!(decode_up(&s).unwrap(), board);
        let Up::Board { remembered, .. } = decode_up(r#"{"type":"board","cards":[],"dirs":[]}"#).unwrap() else { panic!() };
        assert_eq!(remembered, None);
        assert!(!encode(&Up::Board { cards: vec![], dirs: vec![], agents: None, remembered: None }).contains("remembered"));
    }

    #[test]
    fn a_board_with_fields_and_agents_this_maya_does_not_know_still_parses() {
        let newer = r#"{"type":"board","cards":[],"dirs":[],"remembered":[{"agent":"future-agent","sessionId":"x","dir":"/x","name":"x","firstSeenMs":1,"lastSeenMs":2,"somethingNew":true}],"alsoNew":1}"#;
        let Up::Board { remembered, .. } = decode_up(newer).unwrap() else { panic!() };
        assert_eq!(remembered.unwrap()[0].agent, crate::model::Harness::Other);
    }

    #[test]
    fn forget_names_each_session_by_agent_and_id() {
        let f = Down::Command { id: 4, kind: CommandKind::Forget { sessions: vec![(crate::model::Harness::Codex, "c1".into())] } };
        let s = encode(&f);
        assert!(s.contains(r#""kind":"forget","sessions":[["codex","c1"]]"#), "{s}");
        assert_eq!(decode_down(&s).unwrap(), f);
    }

    #[test]
    fn a_mirror_carries_the_remembered_rows_only_when_there_are_some() {
        let rows = vec![crate::remembered::RememberedRow { machine: Some("desk".into()), waiting: false, entry: was("s1") }];
        let m = Down::Mirror { allowed: true, cards: vec![], machines: vec![], remembered: rows };
        assert_eq!(decode_down(&encode(&m)).unwrap(), m);
        assert!(!encode(&Down::Mirror { allowed: false, cards: vec![], machines: vec![], remembered: vec![] }).contains("remembered"));
    }
}
