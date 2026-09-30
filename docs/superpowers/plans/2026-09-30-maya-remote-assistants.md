# Main and Assistant Mayas Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A main Maya shows and drives the sessions of assistant Mayas on other computers over one WebSocket per assistant, paired with a code, while assistants go quiet.

**Architecture:** A pure protocol module (messages, pairing code, HMAC handshake) and a pure merge module (remote boards into one card list, stale and expiry rules, routing by session id) sit under `src-tauri/src/net/`. The main runs a `tungstenite` server with a thread per assistant; the assistant runs a client thread that sends board snapshots and executes commands with the same local functions its own buttons use. Every existing session command in `lib.rs` routes a remote session id to its assistant, so the page changes only where machines are visible: a remote glyph on cards, no Terminal button, a Machine picker in the two dialogs, and a Network section in Settings. The main never touches an assistant's machine; messages carry session ids, text, option numbers, folder names and base64 file bytes only.

**Tech Stack:** Rust (Tauri 2, `tungstenite` 0.30, `hmac` 0.13, `sha2` 0.11, `rand` 0.10, `base64` 0.23, serde), std threads and channels (no async runtime), TypeScript + vitest.

**Spec:** `docs/superpowers/specs/2026-09-30-maya-remote-assistants-design.md`

## Global Constraints

- Protocol version `1`; default port `4127`; pairing code six digits, valid five minutes; token 32 random bytes (base64); nonce 16 random bytes (base64); MAC = hex HMAC-SHA256(token, nonce); three wrong codes from one address in five minutes lock it out for five minutes.
- Command timeout 30 s; `ping` every 10 s; a machine's cards are `stale` after 30 s without a snapshot or on disconnect; a machine disappears after 5 min without a snapshot; snapshots at most once a second; reconnect backoff 1, 2, 4, 8, 16 s then every 30 s.
- Messages are JSON text frames with a `type` (snake_case) field; commands carry a `kind` field. Exact shapes are in Task 1.
- The main never reads an assistant's files, sockets or terminals: every remote action is a `command` the assistant executes.
- `Card` gains `machine: Option<String>` (None locally) and `stale: bool`; the assistant sends cards with `machine` unset and the main fills it in.
- Assistant mode suppresses notifications and speech, and forces `listen` off with the toggle disabled and the note "The main Maya notifies and listens for this machine."
- Spoken and notified lines for remote sessions: `"<name> on <machine> needs a decision"` / `"… is finished"`. Board summary line: `name | harness | state | id | project | machine | asks…` with `machine` = `this mac` for local cards. Duplicate machine names display as `name (hostname)`.
- Commit messages: conventional commits with trailer `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`. Never push; never commit to `main` (work on `feat/remote-mayas`).
- Bash in this harness refuses compound commands mixing `cd` with git and variables inside complex constructs; run plain commands from the repo root. `cargo` needs `PATH="$HOME/.cargo/bin:$PATH"`; run it from `src-tauri`. `pnpm tauri build` and `cargo test` need the sidecar (`pnpm ear:build` once).

## Review Focus

1. A malformed or hostile frame from the network must never panic either side: Task 1's `decode_rejects_garbage_and_wrong_types`, and Task 3's connection thread wraps every decode in a match that logs and continues.
2. A stale or forged authentication must fail closed: Task 1's `mac_rejects_wrong_token_and_reused_nonce`, Task 3's integration test with a wrong MAC.
3. Two assistants with the same name must stay distinguishable and route correctly: Task 2's `duplicate_names_get_the_hostname` and `routes_a_session_id_to_its_machine`.
4. A command whose assistant disconnects mid-flight must return an error within the timeout, not hang the modal: Task 3's `a_command_to_a_disconnected_assistant_fails_fast`.
5. Attachments on a remote reply must reach the assistant's disk and the text must point at the assistant's paths, not the main's: Task 5's `remote_reply_rewrites_attachment_paths`.

---

### Task 1: Protocol, pairing and handshake (pure)

**Files:**
- Create: `src-tauri/src/net/mod.rs`, `src-tauri/src/net/protocol.rs`
- Modify: `src-tauri/Cargo.toml`, `src-tauri/src/lib.rs` (`pub mod net;`), `src-tauri/src/model.rs` (Deserialize derives, `machine`, `stale`), `src-tauri/src/context.rs` (Deserialize), `src-tauri/src/launch.rs` (`LaunchOptions` Serialize)

**Interfaces:**
- Produces: everything in `protocol.rs` below; `Card { machine: Option<String>, stale: bool }` with `Deserialize` on `Card`, `State`, `AwaitKind`, `Harness`, `PullRequest`, `Awaiting`, `context::ContextUsage`; `Serialize` on `launch::LaunchOptions`.

- [ ] **Step 1: Add the crates**

In `src-tauri/Cargo.toml` `[dependencies]` add:

```toml
tungstenite = "0.30"
hmac = "0.13"
sha2 = "0.11"
rand = "0.10"
base64 = "0.23"
```

Run `cargo fetch` from `src-tauri` to confirm they resolve (if `hmac`/`sha2` at those versions do not build together, use the latest pair that does and note it in the report).

- [ ] **Step 2: Model changes**

In `model.rs`: add `Deserialize` to the derives of `State`, `AwaitKind`, `Harness`, `PullRequest`, `Awaiting` and `Card`; add to `Card`:

```rust
    /// The assistant machine this card came from; None for a local session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    /// True when the machine has not reported for a while or is disconnected.
    #[serde(default)]
    pub stale: bool,
```

`Awaiting.questions` already has `#[serde(default)]`; give `Awaiting.detail` none (required). In `context.rs` add `Deserialize` to `ContextUsage`. In `launch.rs` add `Serialize` to `LaunchOptions`. Grep every `Card {` literal in tests (`grep -rn "Card {" src-tauri/src`) and add `machine: None, stale: false`.

- [ ] **Step 3: Write the failing tests** (`protocol.rs` test module)

```rust
#[cfg(test)]
mod tests {
    use super::*;

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
    }

    #[test]
    fn tokens_and_nonces_are_random_and_long_enough() {
        assert_ne!(new_token(), new_token());
        assert!(base64::engine::general_purpose::STANDARD.decode(new_token()).unwrap().len() == 32);
        assert!(base64::engine::general_purpose::STANDARD.decode(new_nonce()).unwrap().len() == 16);
    }
}
```

- [ ] **Step 4: Run them to see them fail**

`PATH="$HOME/.cargo/bin:$PATH" cargo test net::` from `src-tauri`. Expected: compile errors for the missing items.

- [ ] **Step 5: Implement `net/mod.rs` and `net/protocol.rs`**

`net/mod.rs`:

```rust
//! Main and assistant Mayas over the local network.
pub mod protocol;
```

`net/protocol.rs`:

```rust
//! The wire protocol between a main Maya and its assistants: JSON text
//! frames with a `type`, plus the pairing code and the HMAC handshake.

use crate::launch::LaunchOptions;
use crate::model::Card;
use base64::Engine;
use hmac::{Hmac, Mac};
use rand::RngCore;
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
    pub name: String,
    /// The file's bytes, base64.
    pub bytes: String,
}

/// Assistant → main.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Up {
    Hello { protocol: u32, app: String, name: String, hostname: String, platform: String, #[serde(default)] id: Option<String> },
    Pair { code: String },
    Auth { mac: String },
    Board { cards: Vec<Card>, dirs: Vec<String> },
    Result { id: u64, ok: bool, #[serde(default)] error: Option<String>, #[serde(default)] data: Option<Value> },
    Ping,
}

/// Main → assistant.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Down {
    Challenge { nonce: String },
    Welcome { name: String },
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
    Rename { session: String, name: String },
    SetOption { session: String, setting: String, value: String },
    CycleMode { session: String },
    Start { dir: Option<String>, prompt: String, options: LaunchOptions },
    Resume { dir: String, session: String },
    ListResumable { dir: String },
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

/// Constant-time check of a MAC the assistant sent.
pub fn mac_matches(token: &str, nonce: &str, given: &str) -> bool {
    let Ok(given) = (0..given.len()).step_by(2).map(|i| u8::from_str_radix(given.get(i..i + 2).unwrap_or("zz"), 16)).collect::<Result<Vec<u8>, _>>() else {
        return false;
    };
    let mut m = Hmac::<Sha256>::new_from_slice(token.as_bytes()).expect("hmac accepts any key length");
    m.update(nonce.as_bytes());
    m.verify_slice(&given).is_ok()
}

/// Nonces the main has issued; each answers once.
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
        let v = self.failures.entry(addr.to_string()).or_default();
        v.retain(|t| now_ms.saturating_sub(*t) < LOCKOUT_MS);
        v.push(now_ms);
    }

    pub fn locked(&self, addr: &str, now_ms: u64) -> bool {
        self.failures.get(addr).map_or(false, |v| v.iter().filter(|t| now_ms.saturating_sub(**t) < LOCKOUT_MS).count() >= LOCKOUT_AFTER as usize)
    }
}
```

Adjust the `rand` API to the crate version that resolved (`rand::rng()` in 0.9+/0.10; `thread_rng()` earlier).

- [ ] **Step 6: Run the tests until green, then the whole suite**

`cargo test net::` then `cargo test`. Expected: all green (the `Card {` literal updates keep the rest compiling).

- [ ] **Step 7: Commit**

```
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/net src-tauri/src/lib.rs src-tauri/src/model.rs src-tauri/src/context.rs src-tauri/src/launch.rs
git commit -m "feat(net): protocol messages, pairing code and HMAC handshake"
```

---

### Task 2: Config and the board merge (pure)

**Files:**
- Modify: `src-tauri/src/config.rs`
- Create: `src-tauri/src/net/merge.rs`
- Modify: `src-tauri/src/net/mod.rs` (`pub mod merge;`)

**Interfaces:**
- Produces: `config::NetworkRole { Off, Main, Assistant }` (lowercase), `config::PairedAssistant { id, name, hostname, platform, token }`, `config::NetworkConfig { role, port, main_host, main_port, name, assistant_id, token, assistants }` all camelCase with defaults, `Config.network: NetworkConfig`; `merge::RemoteBoard { machine, hostname, platform, cards, dirs, received_at, connected }`, `merge::STALE_MS`, `merge::EXPIRE_MS`, `merge::merged(local, remotes, now) -> Vec<Card>`, `merge::display_names(&[(name, hostname)]) -> Vec<String>`, `merge::machine_of(remotes, session_id) -> Option<String>`, `merge::dirs_of(remotes, machine) -> Vec<String>`.

- [ ] **Step 1: Failing tests**

In `config.rs` tests:

```rust
    #[test]
    fn network_defaults_to_off_and_round_trips() {
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes": 5}"#).unwrap();
        assert_eq!(c.network.role, NetworkRole::Off);
        assert_eq!(c.network.port, 0, "0 means the default port");
        assert!(c.network.assistants.is_empty());
        let mut c = Config::default();
        c.network.role = NetworkRole::Assistant;
        c.network.main_host = "10.0.0.2".into();
        c.network.token = "abc".into();
        c.network.assistants.push(PairedAssistant { id: "x".into(), name: "laptop".into(), hostname: "h".into(), platform: "macos".into(), token: "t".into() });
        let text = serde_json::to_string(&c).unwrap();
        assert!(text.contains("\"role\":\"assistant\"") && text.contains("\"mainHost\":\"10.0.0.2\"") && text.contains("\"assistants\":[{"), "{text}");
        assert_eq!(serde_json::from_str::<Config>(&text).unwrap(), c);
    }
```

In `merge.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Card, Harness, State};

    fn card(id: &str, name: &str) -> Card {
        Card { session_id: id.into(), pid: 1, name: name.into(), cwd: "/x/p".into(), state: State::Idle, state_since: 0, snippet: "".into(), awaiting: None, has_inbox: true, harness: Harness::ClaudeCode, pr: None, context: None, machine: None, stale: false }
    }

    fn board(machine: &str, hostname: &str, ids: &[&str], received_at: u64, connected: bool) -> RemoteBoard {
        RemoteBoard { machine: machine.into(), hostname: hostname.into(), platform: "macos".into(), cards: ids.iter().map(|i| card(i, i)).collect(), dirs: vec!["proj".into()], received_at, connected }
    }

    #[test]
    fn remote_cards_follow_local_ones_and_carry_their_machine() {
        let m = merged(vec![card("l1", "local")], &[board("laptop", "h1", &["r1", "r2"], 1_000, true)], 1_500);
        assert_eq!(m.iter().map(|c| c.session_id.as_str()).collect::<Vec<_>>(), ["l1", "r1", "r2"]);
        assert_eq!(m[0].machine, None);
        assert_eq!(m[1].machine.as_deref(), Some("laptop"));
        assert!(!m[1].stale);
    }

    #[test]
    fn stale_after_thirty_seconds_or_disconnect_and_gone_after_five_minutes() {
        let fresh = merged(vec![], &[board("a", "h", &["r"], 1_000, true)], 1_000 + STALE_MS);
        assert!(!fresh[0].stale);
        let stale = merged(vec![], &[board("a", "h", &["r"], 1_000, true)], 1_000 + STALE_MS + 1);
        assert!(stale[0].stale);
        let dropped = merged(vec![], &[board("a", "h", &["r"], 1_000, false)], 1_500);
        assert!(dropped[0].stale, "a disconnect greys at once");
        assert!(merged(vec![], &[board("a", "h", &["r"], 1_000, false)], 1_000 + EXPIRE_MS + 1).is_empty());
    }

    #[test]
    fn duplicate_names_get_the_hostname() {
        let names = display_names(&[("laptop".into(), "h1".into()), ("laptop".into(), "h2".into()), ("desk".into(), "h3".into())]);
        assert_eq!(names, ["laptop (h1)", "laptop (h2)", "desk"]);
    }

    #[test]
    fn routes_a_session_id_to_its_machine() {
        let boards = [board("a", "h1", &["r1"], 0, true), board("b", "h2", &["r2"], 0, true)];
        assert_eq!(machine_of(&boards, "r2").as_deref(), Some("b"));
        assert_eq!(machine_of(&boards, "l1"), None);
        assert_eq!(dirs_of(&boards, "a"), vec!["proj".to_string()]);
        assert!(dirs_of(&boards, "zzz").is_empty());
    }
}
```

- [ ] **Step 2: Run to see them fail** (`cargo test network_defaults`, `cargo test merge::`).

- [ ] **Step 3: Implement**

`config.rs` additions:

```rust
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum NetworkRole {
    #[default]
    Off,
    Main,
    Assistant,
}

/// An assistant the main has paired with; the token is what it must prove it holds.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PairedAssistant {
    pub id: String,
    pub name: String,
    pub hostname: String,
    pub platform: String,
    pub token: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct NetworkConfig {
    pub role: NetworkRole,
    /// The main's listening port; 0 means `protocol::DEFAULT_PORT`.
    pub port: u16,
    pub main_host: String,
    pub main_port: u16,
    /// The assistant's display name; blank means the hostname.
    pub name: String,
    /// Minted by the main at pairing.
    pub assistant_id: String,
    /// The assistant's token, from pairing.
    pub token: String,
    /// The main's paired assistants.
    pub assistants: Vec<PairedAssistant>,
}
```

Add to `Config`: `#[serde(default)] pub network: NetworkConfig,` and to `Default for Config`: `network: NetworkConfig::default()`. Add to `impl Config`: `pub fn listen_port(&self) -> u16 { if self.network.port == 0 { crate::net::protocol::DEFAULT_PORT } else { self.network.port } }` and `pub fn main_port(&self) -> u16` likewise from `main_port`.

`net/merge.rs`:

```rust
//! Local and remote cards as one board, and where a session id lives.

use crate::model::Card;

pub const STALE_MS: u64 = 30_000;
pub const EXPIRE_MS: u64 = 5 * 60_000;

/// The last snapshot from one assistant.
#[derive(Clone, Debug)]
pub struct RemoteBoard {
    pub machine: String,
    pub hostname: String,
    pub platform: String,
    pub cards: Vec<Card>,
    pub dirs: Vec<String>,
    pub received_at: u64,
    pub connected: bool,
}

/// Local cards first, then each machine's cards with `machine` set and
/// `stale` when the snapshot is old or the link is down; machines quiet for
/// `EXPIRE_MS` are left out.
pub fn merged(local: Vec<Card>, remotes: &[RemoteBoard], now_ms: u64) -> Vec<Card> {
    let mut out = local;
    for b in remotes {
        let age = now_ms.saturating_sub(b.received_at);
        if age > EXPIRE_MS {
            continue;
        }
        let stale = !b.connected || age > STALE_MS;
        for c in &b.cards {
            let mut c = c.clone();
            c.machine = Some(b.machine.clone());
            c.stale = stale;
            out.push(c);
        }
    }
    out
}

/// Names as shown on cards: a name shared by two machines gets its hostname.
pub fn display_names(pairs: &[(String, String)]) -> Vec<String> {
    pairs
        .iter()
        .map(|(name, host)| {
            let dup = pairs.iter().filter(|(n, _)| n == name).count() > 1;
            if dup { format!("{name} ({host})") } else { name.clone() }
        })
        .collect()
}

pub fn machine_of(remotes: &[RemoteBoard], session_id: &str) -> Option<String> {
    remotes.iter().find(|b| b.cards.iter().any(|c| c.session_id == session_id)).map(|b| b.machine.clone())
}

pub fn dirs_of(remotes: &[RemoteBoard], machine: &str) -> Vec<String> {
    remotes.iter().find(|b| b.machine == machine).map(|b| b.dirs.clone()).unwrap_or_default()
}
```

- [ ] **Step 4: Green, then commit**

```
git add src-tauri/src/config.rs src-tauri/src/net
git commit -m "feat(net): network config and the board merge"
```

---

### Task 3: The main's server

**Files:**
- Create: `src-tauri/src/net/server.rs`
- Modify: `src-tauri/src/net/mod.rs`, `src-tauri/src/lib.rs` (`AppState.network`, commands, setup)

**Interfaces:**
- Consumes: Task 1 protocol, Task 2 config and merge.
- Produces:
  ```rust
  pub struct NetworkState { pub server: Option<ServerHandle>, pub client: Option<ClientHandle /* Task 4 */>, pub status: NetworkStatus }
  pub struct ServerHandle { pub shared: Arc<Mutex<Server>>, stop: Arc<AtomicBool> }   // impl ServerHandle { pub fn stop(&self) }
  pub struct Server { pub boards: Vec<RemoteBoard>, pub pairing: Option<PairingWindow>, /* private: connections, pending results, attempts, nonces, next_id */ }
  pub fn start(app: AppHandle, port: u16) -> Result<ServerHandle, String>
  pub fn send_command(app: &AppHandle, machine: &str, kind: CommandKind, timeout: Duration) -> Result<Option<Value>, String>
  impl ServerHandle { pub fn open_pairing(&self, now_ms: u64) -> (String, u64) /* code, expires_at_ms; Regenerate reuses it */; pub fn remove_assistant(&self, id: &str); pub fn port(&self) -> u16; pub fn stop(&self) }
  #[derive(Serialize, Clone, Default)] #[serde(rename_all = "camelCase")]
  pub struct NetworkStatus { pub role: NetworkRole, pub code: Option<PairingCode { code, expires_at }>, pub assistants: Vec<AssistantStatus { id, name, hostname, platform, connected, last_seen: Option<u64> }>, pub assistant: AssistantLink { connected: bool, main_name: Option<String>, error: Option<String> } }
  ```
  Tauri commands: `network_status() -> NetworkStatus`, `network_pairing_code() -> NetworkStatus` (opens or regenerates), `network_remove_assistant(id) -> NetworkStatus`. Event `network` with `NetworkStatus` on every change. Every assistant's snapshot causes `refresh_and_emit`.

- [ ] **Step 1: Failing integration test** (in `server.rs`, `#[cfg(test)]`, using a raw `tungstenite::connect` client against a server started on port 0)

The server needs an `AppHandle` for emits; factor the core so tests can run it without Tauri: `Server` plus a `Notify` trait (`fn board_changed(&self)`, `fn status_changed(&self, status: NetworkStatus)`, `fn paired(&self, assistant: &PairedAssistant)`, `fn paired_list_changed(&self, assistants: &[PairedAssistant])`) implemented by a Tauri adapter in `lib.rs` and by a counter in tests. `start_with(notify: Arc<dyn Notify>, config_view: Arc<Mutex<NetworkConfig>>, port: u16) -> Result<ServerHandle, String>` is the real entry; `start(app, port)` wraps it with the Tauri adapter and the store's config (pairing writes the new assistant into `config.network.assistants` and saves).

```rust
    #[test]
    fn pairs_snapshots_and_round_trips_a_command() {
        let (handle, port) = test_server();               // start_with on port 0, returns the bound port
        let (code, _) = handle.open_pairing(0);
        let mut ws = connect(port);
        send(&mut ws, &Up::Hello { protocol: PROTOCOL, app: "t".into(), name: "laptop".into(), hostname: "h".into(), platform: "macos".into(), id: None });
        send(&mut ws, &Up::Pair { code });
        let Down::Paired { id, token } = recv(&mut ws) else { panic!("expected paired") };
        assert!(matches!(recv(&mut ws), Down::Welcome { .. }));
        send(&mut ws, &Up::Board { cards: vec![card("r1", "remote")], dirs: vec!["proj".into()] });
        wait_until(|| handle.shared.lock().unwrap().boards.iter().any(|b| b.machine == "laptop" && b.cards.len() == 1));
        // A command from the main reaches the client and its result comes back.
        let shared = handle.shared.clone();
        let t = std::thread::spawn(move || send_command_with(&shared, "laptop", CommandKind::Compact { session: "r1".into() }, Duration::from_secs(5)));
        let Down::Command { id: cmd, kind } = recv(&mut ws) else { panic!("expected command") };
        assert_eq!(kind, CommandKind::Compact { session: "r1".into() });
        send(&mut ws, &Up::Result { id: cmd, ok: true, error: None, data: None });
        assert!(t.join().unwrap().is_ok());
        // Reconnect with the token: challenge-response, no code.
        drop(ws);
        let mut ws = connect(port);
        send(&mut ws, &Up::Hello { protocol: PROTOCOL, app: "t".into(), name: "laptop".into(), hostname: "h".into(), platform: "macos".into(), id: Some(id.clone()) });
        let Down::Challenge { nonce } = recv(&mut ws) else { panic!("expected challenge") };
        send(&mut ws, &Up::Auth { mac: mac(&token, &nonce) });
        assert!(matches!(recv(&mut ws), Down::Welcome { .. }));
        handle.stop();
    }

    #[test]
    fn a_wrong_mac_or_code_is_refused_and_a_command_to_a_disconnected_assistant_fails_fast() {
        let (handle, port) = test_server();
        let mut ws = connect(port);
        send(&mut ws, &Up::Hello { protocol: PROTOCOL, app: "t".into(), name: "x".into(), hostname: "h".into(), platform: "macos".into(), id: Some("nobody".into()) });
        assert!(matches!(recv(&mut ws), Down::Bye { .. }), "unknown assistant");
        let mut ws = connect(port);
        send(&mut ws, &Up::Hello { protocol: PROTOCOL, app: "t".into(), name: "x".into(), hostname: "h".into(), platform: "macos".into(), id: None });
        send(&mut ws, &Up::Pair { code: "000000".into() });
        assert!(matches!(recv(&mut ws), Down::Bye { reason } if reason.contains("pairing code")));
        let err = send_command_with(&handle.shared, "ghost", CommandKind::Compact { session: "r".into() }, Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("not connected"), "{err}");
        handle.stop();
    }
```

Helpers `test_server`, `connect`, `send`, `recv`, `wait_until` (poll every 20 ms, 5 s cap) live in the test module.

- [ ] **Step 2: See them fail**, then implement `server.rs`:

Design, concretely:
- `Server` fields: `boards: Vec<RemoteBoard>`, `pairing: Option<PairingWindow>`, `conns: HashMap<String /*assistant id*/, mpsc::Sender<Down>>`, `pending: HashMap<u64, mpsc::Sender<Result<Option<Value>, String>>>`, `next_id: u64`, `attempts: Attempts`, `nonces: NonceLog`, `paired: Vec<PairedAssistant>` (mirror of config), `names: HashMap<String, (String, String)>` (id → name, hostname).
- `start_with` binds `TcpListener` (port 0 allowed for tests; report the bound port through `ServerHandle::port()`), sets `set_nonblocking(false)`, spawns the accept loop which checks `stop` between accepts (use a 500 ms `SO_RCVTIMEO`-style poll by setting the listener non-blocking and sleeping 100 ms when `WouldBlock`). Each accepted stream: `tungstenite::accept` then `handle_connection(shared, notify, stream, peer_addr)` on its own thread.
- `handle_connection`: read the first frame; must be `Hello` with `protocol == PROTOCOL` (else `Bye {reason:"protocol"}`). If `id` is `Some` and paired: send `Challenge`, expect `Auth`, verify `mac_matches` and `nonces.first_use`; else `Bye {reason:"authentication failed"}`. If `id` is `None`: expect `Pair`; check `attempts.locked(addr)` → `Bye {"too many attempts"}`; check the window `accepts` → on failure `attempts.failed`, `Bye {"wrong or expired pairing code"}`; on success mint `new_id()` and `new_token()`, push `PairedAssistant`, call `notify.paired(assistant)` (persists to config in the Tauri adapter), send `Paired`. Then `Welcome { name: <main's name: hostname> }`, register `conns[id] = tx`, mark the board connected, and loop: a reader thread pushes `Up` frames into the connection; `Board` replaces/creates `boards[machine]` with `received_at = now`, `connected = true`, then `notify.board_changed()`; `Result` resolves `pending[id]`; `Ping` answers `Pong`; writer side drains `rx` for `Down` frames. Set the stream read timeout to 15 s so a silent peer is dropped (the client pings every 10 s). On any error or close: `conns.remove`, board `connected = false`, `notify.status_changed`, and every pending id owned by that connection fails with "assistant disconnected".
- `send_command_with(shared, machine, kind, timeout)`: find the assistant id by machine name; if not in `conns` → `Err("<machine> is not connected")`; allocate id, insert a `pending` sender, send `Down::Command`, `recv_timeout(timeout)` → `Err("<machine> did not answer in time")` on timeout (and remove pending).
- `open_pairing`: new `PairingWindow`, `notify.status_changed`. `remove_assistant`: drop from `paired`, send `Bye {"removed"}` and close, remove its board, `notify.paired_list_changed`.
- Status: `NetworkStatus` built from `paired`, `conns`, `boards` and `pairing`.

In `lib.rs`: `AppState.network: Mutex<NetworkState>`; the Tauri `Notify` adapter: `board_changed` → `refresh_and_emit(app)`; `status_changed` → `app.emit("network", status)`; `paired` → append to `store.config.network.assistants` and `config::save`. `refresh_and_emit` and `list_sessions` merge: after `store.refresh`, `let boards = state.network.lock().unwrap().server.as_ref().map(|s| s.shared.lock().unwrap().boards.clone()).unwrap_or_default(); let cards = merge::merged(cards, &boards, now_ms());`. Also make `Store::refresh` results include remote cards for the notifier (`take_new`/`take_finished` run on the merged list so remote decisions notify). In `setup`, if `config.network.role == Main` start the server on `config.listen_port()`; in `set_config`, when the role or port changed, stop and start as needed (a `network_change(before, after) -> NetChange { None, StartMain, StopMain, RestartMain, StartAssistant, StopAssistant, RestartAssistant }` pure function with a test). Register the three commands.

- [ ] **Step 3: Green**, `cargo test` whole suite; commit:

```
git add src-tauri/src/net src-tauri/src/lib.rs
git commit -m "feat(net): the main Maya's server: pairing, snapshots, commands"
```

---

### Task 4: The assistant's client

**Files:**
- Create: `src-tauri/src/net/client.rs`
- Modify: `src-tauri/src/net/mod.rs`, `src-tauri/src/lib.rs`, `src-tauri/src/listener.rs` (listen forced off), `src-tauri/src/attachments.rs` (reuse `save`)

**Interfaces:**
- Consumes: protocol, config, the local session functions in `lib.rs` (`send_reply`, `answer_question`, `compact_session`, `rename_session`, `set_session_option`, `cycle_session_mode`, `session_history`, `start_session`, `resume_session`, `list_resumable_sessions`, `list_project_dirs`).
- Produces: `ClientHandle { stop }`, `client::start(app: AppHandle) -> ClientHandle`, `client::pair(app, host, port, name, code) -> Result<String /*main name*/, String>` (one blocking connection that pairs and stores `assistant_id` + `token` in config, then the normal client starts), `client::push_board(app)` called from `refresh_and_emit` (throttled inside to one a second), `client::execute(app, kind) -> Result<Option<Value>, String>` (pure dispatch to the local functions), Tauri command `network_pair(host, port, name, code) -> Result<NetworkStatus, String>`.

- [ ] **Step 1: Failing tests**

`execute` is testable without a network: it takes `&AppHandle`, so test its pure helper `rewrite_attachments(text: &str, saved: &[(String /*original path*/, PathBuf /*local*/)]) -> String` that replaces `Attached file: <original>` lines with the local path, and `backoff_ms(attempt) -> u64` (1 000, 2 000, 4 000, 8 000, 16 000, then 30 000). Tests:

```rust
    #[test]
    fn backoff_doubles_to_sixteen_seconds_then_every_thirty() {
        assert_eq!((0..7).map(backoff_ms).collect::<Vec<_>>(), [1_000, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000]);
    }

    #[test]
    fn remote_reply_rewrites_attachment_paths() {
        let text = "look at this\nAttached file: /Users/main/.claude/maya/attachments/a.png";
        let out = rewrite_attachments(text, &[("/Users/main/.claude/maya/attachments/a.png".into(), std::path::PathBuf::from("/Users/asst/.claude/maya/attachments/1-a.png"))]);
        assert_eq!(out, "look at this\nAttached file: /Users/asst/.claude/maya/attachments/1-a.png");
    }

    #[test]
    fn a_hello_carries_the_name_or_the_hostname() {
        assert_eq!(display_name("", "tiagos-mbp"), "tiagos-mbp");
        assert_eq!(display_name("  laptop ", "tiagos-mbp"), "laptop");
    }
```

And an integration test in `server.rs`'s module (it already has the server helpers): start a test server, open pairing, run `client::run_once(config, notify, stop)` (the connection routine, factored so tests can call it with a `NetworkConfig` and a fake executor `Arc<dyn Executor>` whose `execute(kind)` records the kind and returns `Ok(None)`), and assert the server got a board (the fake executor also provides `board()`), then send a `Compact` command from the server side and assert the executor recorded it and the server got `Ok`.

- [ ] **Step 2: Implement**

- `run_once(config: &NetworkConfig, exec: Arc<dyn Executor>, notify: Arc<dyn ClientNotify>, stop: &AtomicBool) -> Result<(), String>`: connect `ws://{host}:{port}/`, send `Hello` (with `id` when paired), handle `Challenge` → `Auth`, or `Pair` when `config.token` is empty and a code is provided (pairing path used by `pair`), then `Welcome` → `notify.connected(main_name)`, send the first `Board`, and loop: a reader thread for `Down` frames; `Command` → `exec.execute(kind)` on a fresh thread → `Result`; `Bye` → return `Err(reason)` and, when the reason is `removed`, `notify.removed()`; every 10 s send `Ping`; on `push_board` requests (an `mpsc` the handle owns) send `Board` if at least 1 s since the last one (else schedule one). Read timeout 15 s → treat as disconnect.
- `start(app)` spawns a thread looping `run_once` with `backoff_ms(attempt)` between failures, resetting `attempt` after a connection lasted 60 s; `stop` ends it. The Tauri `Executor` calls the local functions through `app.state()`; `Reply` with attachments: decode base64, `attachments::save(&maya_dir, &name, &bytes, now_ms())` each, `rewrite_attachments`, then `send_reply`. `History` returns `serde_json::to_value(turns)`; `ListResumable` returns the list; `Start` returns `StartResult`.
- Assistant quiet mode: in `refresh_and_emit`, when `config.network.role == Assistant`, skip notifications and speech and call `net::client::push_board(app)` after emitting. In `listener::voice_listen`, refuse with "The main Maya notifies and listens for this machine." when the role is Assistant; `set_config` forces `listen = false` when the role becomes Assistant and stops listening.
- `network_pair` command: validates host and code, runs a one-shot `run_once` in pairing mode with a 15 s deadline, saves `assistant_id`, `token`, `main_host`, `main_port`, `name` and `role = Assistant`, starts the client, returns the status.
- Status: `NetworkState.status.assistant` updated by `ClientNotify` (`connected(main_name)`, `disconnected(error)`, `removed()`), each emitting `network`.

- [ ] **Step 3: Green, commit**

```
git add src-tauri/src/net src-tauri/src/lib.rs src-tauri/src/listener.rs
git commit -m "feat(net): the assistant Maya's client: pairing, snapshots, executing commands"
```

---

### Task 5: Routing every session command through the main

**Files:**
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Produces: each of `session_history`, `send_reply`, `answer_question`, `compact_session`, `rename_session`, `set_session_option`, `cycle_session_mode` first calls `remote_machine_of(&state, &session_id) -> Option<String>` (merge::machine_of over the server's boards) and, when `Some(machine)`, returns `net::server::send_command(&app, &machine, CommandKind::…, Duration::from_secs(30))` mapped to the command's result type; `send_reply` gains `attachments: Vec<String>` (local paths, default empty) that the remote path reads and base64-encodes (20 MB cap each) into `Attachment`s; `start_session`, `resume_session`, `list_resumable_sessions`, `list_project_dirs` gain `machine: Option<String>` (None or "" = local), forwarding to the machine when set (`list_project_dirs` for a remote machine returns `merge::dirs_of`); new `list_machines() -> Vec<MachineInfo { name, hostname, platform, connected }>`; `focus_session` for a remote id returns `Err("That session runs on <machine>; open it there.")`.

- [ ] **Step 1: Failing tests** for the pure routing helper `routed(kind_for_local: …)` is awkward; instead test `remote_attachments(paths) -> Result<Vec<Attachment>, String>` (reads files, refuses a missing file and one over 20 MB) and `start_args_for(machine, dir, prompt, options) -> CommandKind` (a trivial builder) in `lib.rs` tests; the routing itself is exercised by Task 8's end-to-end run and by the Task 3/4 integration tests.

- [ ] **Step 2: Implement** as described; keep the local paths byte-for-byte as today (diff them before committing). Update the `generate_handler!` list and `src/modal.ts`'s `send_reply` call to pass `attachments: current.model.attachments.map(a => a.path)`.

- [ ] **Step 3: `cargo test`, `pnpm test`, commit**

```
git add src-tauri/src/lib.rs src/modal.ts
git commit -m "feat(net): session commands route to the assistant that owns the session"
```

---

### Task 6: The page: remote cards, Machine pickers, Network settings

**Files:**
- Modify: `src/types.ts` (`Card.machine?: string | null; Card.stale?: boolean`), `src/card.ts`, `src/card.test.ts`, `src/modal.ts`, `src/main.ts`, `src/newsession.ts`, `src/newsession.test.ts`, `src/resume.ts`, `src/resume.test.ts`, `src/settings.ts`, `src/settings.test.ts`, `src/settings-flow.test.ts`, `src/styles.css`
- Create: `src/assets/icons/remote.svg` (a small monitor-with-signal glyph, 16×16, `currentColor`), registered in `src/icons.ts`

**Interfaces:**
- Consumes: commands `network_status`, `network_pairing_code`, `network_remove_assistant`, `network_pair`, `list_machines`, the `machine` argument on `list_project_dirs`, `list_resumable_sessions`, `resume_session`, `start_session`; event `network`.

- [ ] **Step 1: Failing tests**

`card.test.ts`:
```ts
  it("marks a remote card with the machine and drops the Terminal button", () => {
    const el = renderCard({ ...base, machine: "laptop", stale: false }, 0);
    expect(el.querySelector(".card__remote")?.getAttribute("title")).toBe("Runs on laptop");
    expect(el.querySelector(".card__project")?.textContent).toBe("proj on laptop");
    expect(el.querySelector("button[data-action=terminal]")).toBeNull();
    expect(el.querySelector("button[data-action=reply]")).not.toBeNull();
    expect(renderCard({ ...base, machine: "laptop", stale: true }, 0).classList.contains("card--stale")).toBe(true);
    expect(renderCard(base, 0).querySelector("button[data-action=terminal]")).not.toBeNull();
  });
```

`newsession.test.ts` and `resume.test.ts`: with `machines: [{ name: "This Mac", value: "" }, { name: "laptop", value: "laptop" }]` in the model, a `select[name=machine]` renders with those options, "This Mac" selected; changing it calls `onMachine("laptop")`; `onStart`/`onResume` receive the machine. With one machine, no picker renders.

`settings.test.ts`: a "Network" section with a `select[name=networkRole]` (off / main / assistant); role main shows the port input, the pairing code (`.settings__code`, text "483 921" when `code = "483921"`), a Regenerate button (`data-action=regenerate-code`) and the assistants list with Remove buttons (`data-action=remove-assistant`, `data-id`); role assistant shows host, port, name and code inputs plus a Pair button (`data-action=pair`) and the status line; the listen checkbox is disabled with the note "The main Maya notifies and listens for this machine." when the role is assistant.

`settings-flow.test.ts`: a `network` event repaints the Network section (the status line changes from "Reconnecting…" to "Connected to desk").

- [ ] **Step 2: Implement**

- `card.ts`: when `card.machine`, append `iconElement("remote", 12)` inside a `span.card__remote` with `title = "Runs on <machine>"` after the name, subtitle `"<project> on <machine>"`, skip the Terminal button, add `card--stale` when `stale`.
- `modal.ts`: no "Open terminal" button in the banner for a remote card; the title shows the machine after the name. `main.ts`: the terminal action on a remote card shows a toast "That session runs on <machine>".
- `newsession.ts` / `resume.ts`: `machines` in the model, loaded from `list_machines`; a `select[name=machine]` before the folder select when more than one; changing it reloads `list_project_dirs` with `machine`; `onStart(machine, dir, prompt, options)` and `onResume(machine, dir, sessionId)` pass `machine` to the commands. "Let Claude choose" stays local-only (disabled with a hint for a remote machine).
- `settings.ts`: the Network section as tested, driven by `NetworkStatus` from `network_status` and the `network` event; `onRole`, `onPort`, `onMainHost`, `onMainPort`, `onName` save config; `onRegenerate` → `network_pairing_code`; `onRemove` → `network_remove_assistant`; `onPair` → `network_pair`. Listen toggle disabled with the note when role is assistant.
- CSS: `.card__remote { color: var(--gold); margin-left: 6px; }`, `.card--stale { opacity: .55; }`, `.settings__code { font: 600 22px ui-monospace, monospace; letter-spacing: .15em; color: var(--gold-bright); }`.

- [ ] **Step 3: `pnpm test`, `pnpm exec tsc --noEmit`, commit**

```
git add src src/assets/icons/remote.svg
git commit -m "feat(net): remote cards, machine pickers and the Network settings"
```

---

### Task 7: Notifications and voice know about machines

**Files:**
- Modify: `src-tauri/src/notify.rs`, `src-tauri/src/interpreter.rs`, `src-tauri/src/listener.rs`

**Interfaces:**
- Produces: `notify::spoken_line` says `"<name> on <machine> needs a decision"` for remote cards; `notify::notify` puts `<project> on <machine>` in the banner subtitle; `interpreter::board_summary` gains the machine column (`this mac` locally); `find_session` accepts `"<name> on <machine>"` (split on the last " on " when the right side matches a machine) and disambiguates with `"Which one: <name> on <machine> or <name> on this Mac?"`; `spoken_for` read-backs add ` on <machine>` after the name for remote actions; the dirs list for start/resume by voice includes `"<dir> (on <machine>)"` entries and `validate` maps those to `machine` + `dir` in the action (`execute_action` passes `machine` through).

- [ ] **Step 1: Failing tests**

```rust
    // notify.rs
    #[test]
    fn remote_sessions_are_announced_with_their_machine() {
        let mut c = card_awaiting("hexgrid");
        c.machine = Some("laptop".into());
        assert_eq!(spoken_line(&c).unwrap(), "hexgrid on laptop needs a decision");
    }
    // interpreter.rs
    #[test]
    fn summary_and_matching_know_the_machine() {
        let mut a = card("a", "hexgrid", State::Idle, None);
        let mut b = card("b", "hexgrid", State::Idle, None);
        b.machine = Some("laptop".into());
        let s = board_summary(&[a.clone(), b.clone()]);
        assert!(s.contains("| proj-a | this mac"), "{s}");
        assert!(s.contains("| proj-b | laptop"), "{s}");
        assert_eq!(find_session(&[a.clone(), b.clone()], "hexgrid on laptop").unwrap().session_id, "b");
        assert_eq!(find_session(&[a.clone(), b.clone()], "hexgrid on this mac").unwrap().session_id, "a");
        let err = find_session(&[a, b], "hexgrid").unwrap_err();
        assert_eq!(err, "Which one: hexgrid on this Mac or hexgrid on laptop?");
    }
    // listener.rs
    #[test]
    fn read_backs_name_the_machine() {
        assert_eq!(spoken_for(&json!({"kind":"reply","session":"id","name":"hexgrid","machine":"laptop","text":"go"})).unwrap(), "Telling hexgrid on laptop: go. Yes?");
        assert_eq!(spoken_for(&json!({"kind":"start","dir":"maya","machine":"laptop","prompt":"fix it"})).unwrap(), "Starting a session in maya on laptop: fix it. Yes?");
    }
```

- [ ] **Step 2: Implement**, keeping all existing tests green (local cards produce exactly today's strings). `validate` copies `c.machine` into `a["machine"]` for session actions; for `start`/`resume`, a dir spelled `"<dir> (on <machine>)"` sets `a["machine"]` and `a["dir"]`; the listener's `interpret` builds the voice dir list as local dirs plus `dirs_of` each board with the suffix, and `execute_action` passes `machine` to `start_session`/`resume_session`/`list_resumable_sessions`.

- [ ] **Step 3: `cargo test`, commit**

```
git add src-tauri/src/notify.rs src-tauri/src/interpreter.rs src-tauri/src/listener.rs
git commit -m "feat(net): announcements and voice name the machine a session runs on"
```

---

### Task 8: Docs

**Files:**
- Modify: `README.md` (a "Several Macs" section under What Maya does and a "Network" subsection after Voice: roles, pairing, what travels, plain text on the LAN), `docs/DEVELOPING.md` (the `net/` modules in Layout)

- [ ] Write both, keeping the README's voice; commit `docs: main and assistant Mayas`.

---

### Task 9: End to end on two Macs (needs the user)

- [ ] Build signed bundles on both machines (or copy the bundle), set one as main, pair the other with the code.
- [ ] See the assistant's cards on the main with the glyph and no Terminal button; open one, read its conversation, reply with an attachment, answer an option question; compact; rename.
- [ ] Start a session on the assistant from the main's "+" with the Machine picker; resume one.
- [ ] Have a remote session ask a question: the main announces "… on <machine> needs a decision"; the assistant stays silent; "Maya, tell hexgrid on laptop to go ahead" works from the main.
- [ ] Quit the assistant: its cards grey within 30 s and vanish after 5 min; restart it: they return without re-pairing. Remove it from the main: it reports "Removed by the main Maya; pair again."
- [ ] Fix what fails, with a test where a unit is testable, one commit per fix.
