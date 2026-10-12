# Promote an assistant to main — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A main Maya hands the main role to one of its assistants; every connected Maya, the old main included, then assists the new main without pairing again, and the phone stays linked as a listener.

**Architecture:** The main seals its paired list under a key derived from the promoted assistant's token and sends it as a `promote` command. The assistant becomes a main with that list and answers with its port. The main then sends `move { host, port }` to every other assistant that can follow, and saves itself as an assistant of the new main with an entry it minted into the list. Tokens never change, so the existing mutual HMAC handshake works against the new main.

**Tech Stack:** Rust (maya-core, Tauri 2 desktop and Android crates, the CLI), `tungstenite`, `hmac` 0.13 / `sha2` 0.11, new `chacha20poly1305` 0.11.0; vanilla TypeScript with vitest.

**Spec:** `docs/superpowers/specs/2026-10-12-promote-assistant-design.md`

## Global Constraints

- Protocol stays at version 1; every new field has a serde default (`hello.caps` empty when absent).
- Caps strings: `follow`, `lead`. Desktop and phone send both; `maya-cli` sends `follow`.
- Seal: HKDF-SHA256 of the token, salt empty, info `maya promote v1`, 32-byte key; ChaCha20-Poly1305, random 12-byte nonce; `sealed` = base64(nonce ‖ ciphertext); plaintext `{"assistants":[PairedAssistant…]}`.
- Promote timeout: 30 s.
- User-facing strings, verbatim:
  - `"<G> could not become the main: <error>."`
  - `"<G> did not answer; it may have become a main on its own. Check it before trying again."`
  - `"A handover is already running."`
  - `"The handover could not be opened."`
  - `"This Maya cannot become a main."`
  - `"This phone runs no sessions."`
  - `"Could not save this Maya as an assistant: <error>."`
  - `"The main Maya moved to another machine."`
  - Confirmation: `"Make <G> the main Maya? Every connected Maya will assist <G>, and so will <this Mac|this PC|this computer|this phone>."`, `"<name> cannot follow and stays with no main; pair it with <G> afterwards."`, `"<name> is offline; when it is back, set its Host to <G's address>. It does not need to pair again."`
  - Phone: `"Listening to <G>"`, `"Reconnecting…"`, button `Stop listening`.
  - Debug log under `network`: `"promoted to main by <M>; <n> assistants handed over"`.
- Never commit to `main`; work stays on `feat/promote-assistant`. Version bump to 0.16.0 is the branch's last commit, on its own, as `chore(release): 0.16.0`.
- Unix-only tests are gated with `#[cfg(unix)]`; paths are compared by component (Windows CI).

## Review Focus

1. **The promote result must leave G before its client stops.** If G's client sees a stop flag (or tears down) before the queued `result` is written, M times out and says G "did not answer" although G became the main. Task 4 pins this: the client ends itself only after draining the result.
2. **`move` must reach each B before M's server stops.** The server's connection loop checks `stopped()` before writing queued frames; stopping right after queueing `move` would drop it. Task 3 makes `move` end its connection like `bye` and has `promote` wait (≤ 3 s) for those connections to close.
3. **Promoting a Maya that is mid-pairing, or a second Make main click.** A second handover must be refused, and pairing must be closed so no new assistant pairs with M mid-handover and is lost. Task 3 tests both.
4. **G's port already taken.** G must put its old config back and stay an assistant of M, with nobody moved. Task 5 tests it end to end.
5. **A B running an older Maya (no `follow`).** It must not be sent `move`, and the confirmation must name it. Task 3 (server) and Task 9 (lines) test it.

---

### Task 1: Protocol — caps, `move`, `promote` and the seal

**Files:**
- Modify: `Cargo.toml` (workspace deps), `core/Cargo.toml`
- Modify: `core/src/net/protocol.rs`
- Modify: `core/src/net/server.rs` (`kind_name`, `admit`'s `Up::Hello` pattern, the 11 `Up::Hello {…}` literals in its tests)
- Modify: `core/src/net/client.rs` (the `Up::Hello` literal in `handshake`, `handle`'s match on `Down`)
- Modify: `src-tauri/src/net_app.rs` (`execute`'s match), `cli/src/executor.rs` (`execute`'s match)

**Interfaces:**
- Produces:
  - `pub const CAP_FOLLOW: &str = "follow"; pub const CAP_LEAD: &str = "lead";`
  - `Up::Hello { protocol, app, name, hostname, platform, id, caps: Vec<String> }`
  - `Down::Move { host: String, port: u16 }`
  - `CommandKind::Promote { sealed: String }`
  - `pub fn seal(token: &str, plaintext: &[u8]) -> String`
  - `pub fn open(token: &str, sealed: &str) -> Result<Vec<u8>, String>` (error text `"The handover could not be opened."`)
  - `pub const UNSEALABLE: &str = "The handover could not be opened.";`

- [ ] **Step 1: Add the crate.** In the workspace `Cargo.toml` `[workspace.dependencies]`, after `base64 = "0.23"`:

```toml
chacha20poly1305 = "0.11.0"
```

and in `core/Cargo.toml` `[dependencies]`, after `base64.workspace = true`:

```toml
chacha20poly1305.workspace = true
```

Run: `cargo check -p maya-core`. Expected: builds (the crate is unused so far).

- [ ] **Step 2: Write the failing tests** at the end of `protocol.rs`'s `mod tests`:

```rust
    #[test]
    fn a_hello_without_caps_has_none_and_caps_round_trip() {
        let old = r#"{"type":"hello","protocol":1,"app":"0.15.0","name":"n","hostname":"h","platform":"macos"}"#;
        let Up::Hello { caps, .. } = decode_up(old).unwrap() else { panic!() };
        assert!(caps.is_empty());
        let hello = Up::Hello { protocol: PROTOCOL, app: "x".into(), name: "n".into(), hostname: "h".into(), platform: "macos".into(), id: None, caps: vec![CAP_FOLLOW.into(), CAP_LEAD.into()] };
        let s = encode(&hello);
        assert!(s.contains(r#""caps":["follow","lead"]"#), "{s}");
        assert_eq!(decode_up(&s).unwrap(), hello);
    }

    #[test]
    fn move_and_promote_encode_and_decode() {
        let mv = Down::Move { host: "192.168.55.70".into(), port: 4127 };
        let s = encode(&mv);
        assert!(s.contains(r#""type":"move""#) && s.contains(r#""host":"192.168.55.70""#) && s.contains(r#""port":4127"#), "{s}");
        assert_eq!(decode_down(&s).unwrap(), mv);
        let p = Down::Command { id: 3, kind: CommandKind::Promote { sealed: "abc".into() } };
        let s = encode(&p);
        assert!(s.contains(r#""kind":"promote""#) && s.contains(r#""sealed":"abc""#), "{s}");
        assert_eq!(decode_down(&s).unwrap(), p);
    }

    #[test]
    fn a_seal_opens_only_with_its_token_and_unchanged() {
        let token = new_token();
        let sealed = seal(&token, b"{\"assistants\":[]}");
        assert_eq!(open(&token, &sealed).unwrap(), b"{\"assistants\":[]}");
        assert_eq!(open(&new_token(), &sealed).unwrap_err(), UNSEALABLE);
        let mut bytes = base64::engine::general_purpose::STANDARD.decode(&sealed).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        let tampered = base64::engine::general_purpose::STANDARD.encode(bytes);
        assert_eq!(open(&token, &tampered).unwrap_err(), UNSEALABLE);
        assert_eq!(open(&token, "not base64!").unwrap_err(), UNSEALABLE);
        assert_eq!(open(&token, "AAAA").unwrap_err(), UNSEALABLE, "shorter than a nonce");
        assert_ne!(seal(&token, b"x"), seal(&token, b"x"), "each seal has a fresh nonce");
    }
```

- [ ] **Step 3: Run them to see them fail**

Run: `cargo test -p maya-core --lib net::protocol`
Expected: compile errors (`caps`, `Down::Move`, `CommandKind::Promote`, `seal`, `open` unknown).

- [ ] **Step 4: Implement in `protocol.rs`.**

Under the constants:

```rust
/// A Maya with this cap follows the main to a new host on `move`.
pub const CAP_FOLLOW: &str = "follow";
/// A Maya with this cap can become a main when sent `promote`.
pub const CAP_LEAD: &str = "lead";
pub const UNSEALABLE: &str = "The handover could not be opened.";
```

`Up::Hello` gains, after `id`:

```rust
        /// What this Maya can do (`CAP_FOLLOW`, `CAP_LEAD`); an older Maya sends none.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        caps: Vec<String>,
```

`Down` gains, after `Command`:

```rust
    /// The main is now at `host:port`: save it and reconnect there with the
    /// same id and token. The main closes the connection after it.
    Move { host: String, port: u16 },
```

`CommandKind` gains, last:

```rust
    /// Become the main with the paired list sealed under this assistant's token (`seal`).
    Promote { sealed: String },
```

The seal, after `mac_matches`:

```rust
/// HKDF-SHA256 (RFC 5869) of the token with an empty salt and the info
/// `maya promote v1`: the key only this token's holders can derive.
fn promote_key(token: &str) -> [u8; 32] {
    let mut extract = Hmac::<Sha256>::new_from_slice(&[0u8; 32]).expect("hmac accepts any key length");
    extract.update(token.as_bytes());
    let prk = extract.finalize().into_bytes();
    let mut expand = Hmac::<Sha256>::new_from_slice(&prk).expect("hmac accepts any key length");
    expand.update(b"maya promote v1");
    expand.update(&[1u8]);
    let okm = expand.finalize().into_bytes();
    let mut key = [0u8; 32];
    key.copy_from_slice(&okm[..32]);
    key
}

/// Seals `plaintext` for whoever holds `token`: base64 of a random 12-byte
/// nonce followed by the ChaCha20-Poly1305 ciphertext.
pub fn seal(token: &str, plaintext: &[u8]) -> String {
    use chacha20poly1305::aead::{Aead, KeyInit};
    let cipher = chacha20poly1305::ChaCha20Poly1305::new_from_slice(&promote_key(token)).expect("a 32-byte key");
    let nonce: [u8; 12] = random(12).try_into().unwrap();
    let mut out = nonce.to_vec();
    out.extend(cipher.encrypt(&nonce.into(), plaintext).expect("encrypting in memory does not fail"));
    base64::engine::general_purpose::STANDARD.encode(out)
}

/// Opens a `seal`; any other token, or any changed byte, is `UNSEALABLE`.
pub fn open(token: &str, sealed: &str) -> Result<Vec<u8>, String> {
    use chacha20poly1305::aead::{Aead, KeyInit};
    let bytes = base64::engine::general_purpose::STANDARD.decode(sealed.trim()).map_err(|_| UNSEALABLE.to_string())?;
    if bytes.len() < 12 {
        return Err(UNSEALABLE.into());
    }
    let (nonce, body) = bytes.split_at(12);
    let nonce: [u8; 12] = nonce.try_into().unwrap();
    let cipher = chacha20poly1305::ChaCha20Poly1305::new_from_slice(&promote_key(token)).expect("a 32-byte key");
    cipher.decrypt(&nonce.into(), body).map_err(|_| UNSEALABLE.to_string())
}
```

If the 0.11 API differs (the nonce type's `From<[u8; 12]>`, `new_from_slice`), check it with context7 or docs.rs for `chacha20poly1305 0.11.0` and adjust only these two functions.

- [ ] **Step 5: Make the workspace compile.**
  - `server.rs` `kind_name`: add `CommandKind::Promote { .. } => "promote",`.
  - `server.rs` `admit`: the pattern `let Up::Hello { protocol, app, name, hostname, platform, id } = hello` becomes `let Up::Hello { protocol, app, name, hostname, platform, id, caps } = hello`, then `let _ = caps;` for now (Task 3 uses it).
  - `server.rs` tests: every `Up::Hello { … id: … }` literal gains `, caps: vec![]` before its closing `}` (11 of them: `hello()` helper and the ones in the tests).
  - `client.rs` `handshake`: the `Up::Hello { … id }` literal gains `caps: vec![]` (Task 4 fills it).
  - `client.rs` `handle`: add `Down::Move { .. } => log::line("network", "main: ignored move"),` (Task 4 replaces it).
  - `src-tauri/src/net_app.rs` `execute` and `cli/src/executor.rs` `execute`: add, last,

```rust
        // The client opens a handover itself (`client::handle`); it never reaches an executor.
        CommandKind::Promote { .. } => Err("This Maya cannot become a main.".into()),
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p maya-core --lib net::` then `cargo check --workspace`
Expected: all pass; the workspace builds.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock core/Cargo.toml core/src/net/protocol.rs core/src/net/server.rs core/src/net/client.rs src-tauri/src/net_app.rs cli/src/executor.rs
git commit -m "feat(net): caps in hello, move, promote and the sealed handover"
```

---

### Task 2: `promote.rs` — the handover list and the config rewrites

**Files:**
- Create: `core/src/net/promote.rs`
- Modify: `core/src/net/mod.rs` (`pub mod promote;`)

**Interfaces:**
- Consumes: `protocol::{seal, open, new_id, new_token}`, `config::{Config, NetworkRole, PairedAssistant}`, `net::local_hostname`.
- Produces:
  - `pub fn own_entry(main_name: &str) -> PairedAssistant` — new id and token, `name` = `main_name`, `hostname` = `local_hostname()`, `platform` = `std::env::consts::OS`, empty address, no `last_seen`.
  - `pub fn handover_list(paired: &[PairedAssistant], promoted: &str, me: &PairedAssistant) -> Vec<PairedAssistant>`
  - `pub fn seal_list(token: &str, list: &[PairedAssistant]) -> String`
  - `pub fn open_list(token: &str, sealed: &str) -> Result<Vec<PairedAssistant>, String>`
  - `pub fn as_main(c: &mut Config, list: Vec<PairedAssistant>)`
  - `pub fn as_assistant_of(c: &mut Config, host: &str, port: u16, me: &PairedAssistant)`

- [ ] **Step 1: Write the failing tests** in the new file (tests first, the functions as `todo!()` stubs so it compiles):

```rust
//! Handing the main role to an assistant: the list the new main gets, the
//! seal around it, and how each side's config changes.

use super::protocol::{new_id, new_token, open, seal, UNSEALABLE};
use crate::config::{Config, NetworkRole, PairedAssistant};
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, name: &str) -> PairedAssistant {
        PairedAssistant { id: id.into(), name: name.into(), hostname: "h".into(), platform: "macos".into(), token: format!("t-{id}"), address: "10.0.0.9".into(), last_seen: Some(5) }
    }

    #[test]
    fn the_list_leaves_the_new_main_out_and_adds_the_old_one() {
        let me = own_entry("desk");
        let list = handover_list(&[entry("g", "laptop"), entry("b", "mini")], "g", &me);
        assert_eq!(list.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["b", me.id.as_str()]);
        assert_eq!(list[0], entry("b", "mini"), "the others go as they are, token included");
        assert_eq!(me.name, "desk");
        assert!(me.address.is_empty() && me.last_seen.is_none());
        assert!(!me.id.is_empty() && !me.token.is_empty());
        assert_ne!(own_entry("desk").token, me.token, "a fresh token each time");
    }

    #[test]
    fn the_list_opens_with_the_new_mains_token_only() {
        let list = vec![entry("b", "mini")];
        let sealed = seal_list("t-g", &list);
        assert!(!sealed.contains("t-b"), "no token in the clear");
        assert_eq!(open_list("t-g", &sealed).unwrap(), list);
        assert_eq!(open_list("t-b", &sealed).unwrap_err(), UNSEALABLE);
    }

    #[test]
    fn the_new_main_keeps_its_port_and_name_and_forgets_its_link() {
        let mut c = Config::default();
        c.network.role = NetworkRole::Assistant;
        c.network.port = 5000;
        c.network.name = "laptop".into();
        c.network.main_host = "10.0.0.2".into();
        c.network.main_port = 4127;
        c.network.assistant_id = "g".into();
        c.network.token = "t-g".into();
        as_main(&mut c, vec![entry("b", "mini")]);
        assert_eq!(c.network.role, NetworkRole::Main);
        assert_eq!((c.network.port, c.network.name.as_str()), (5000, "laptop"));
        assert_eq!(c.network.assistants, vec![entry("b", "mini")]);
        assert!(c.network.main_host.is_empty() && c.network.main_port == 0 && c.network.assistant_id.is_empty() && c.network.token.is_empty());
    }

    #[test]
    fn the_old_main_becomes_an_assistant_with_its_own_entry_and_stops_listening() {
        let mut c = Config::default();
        c.network.role = NetworkRole::Main;
        c.network.assistants = vec![entry("g", "laptop")];
        c.listen = true;
        let me = own_entry("desk");
        as_assistant_of(&mut c, "10.0.0.9", 5000, &me);
        assert_eq!(c.network.role, NetworkRole::Assistant);
        assert_eq!((c.network.main_host.as_str(), c.network.main_port), ("10.0.0.9", 5000));
        assert_eq!((c.network.assistant_id.as_str(), c.network.token.as_str()), (me.id.as_str(), me.token.as_str()));
        assert!(c.network.assistants.is_empty());
        assert!(!c.listen, "the main listens for its assistants");
    }
}
```

Add `pub mod promote;` after `pub mod client;` in `core/src/net/mod.rs`.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p maya-core --lib net::promote`
Expected: FAIL (`not yet implemented` panics from the stubs).

- [ ] **Step 3: Implement** above the tests:

```rust
/// The handover's plaintext.
#[derive(Serialize, Deserialize)]
struct Handover {
    assistants: Vec<PairedAssistant>,
}

/// The entry the old main makes for itself in the list it hands over: it
/// connects to the new main with this id and token. Its address is filled
/// in by the new main at its first authentication.
pub fn own_entry(main_name: &str) -> PairedAssistant {
    PairedAssistant { id: new_id(), name: main_name.into(), hostname: super::local_hostname(), platform: std::env::consts::OS.into(), token: new_token(), address: String::new(), last_seen: None }
}

/// Every paired assistant but the one promoted, then the old main's own entry.
pub fn handover_list(paired: &[PairedAssistant], promoted: &str, me: &PairedAssistant) -> Vec<PairedAssistant> {
    paired.iter().filter(|p| p.id != promoted).cloned().chain(std::iter::once(me.clone())).collect()
}

pub fn seal_list(token: &str, list: &[PairedAssistant]) -> String {
    let plain = serde_json::to_vec(&Handover { assistants: list.to_vec() }).expect("a list serializes");
    seal(token, &plain)
}

pub fn open_list(token: &str, sealed: &str) -> Result<Vec<PairedAssistant>, String> {
    let plain = open(token, sealed)?;
    serde_json::from_slice::<Handover>(&plain).map(|h| h.assistants).map_err(|_| UNSEALABLE.to_string())
}

/// The promoted assistant's config: a main with the list, on its own port
/// and name, with its link to the old main gone.
pub fn as_main(c: &mut Config, list: Vec<PairedAssistant>) {
    let n = &mut c.network;
    n.role = NetworkRole::Main;
    n.assistants = list;
    n.main_host.clear();
    n.main_port = 0;
    n.assistant_id.clear();
    n.token.clear();
}

/// The old main's config: an assistant of `host:port` with the entry it
/// made for itself. Listening for "Maya" goes off, as pairing does: the
/// main listens for its assistants.
pub fn as_assistant_of(c: &mut Config, host: &str, port: u16, me: &PairedAssistant) {
    let n = &mut c.network;
    n.role = NetworkRole::Assistant;
    n.main_host = host.into();
    n.main_port = port;
    n.assistant_id = me.id.clone();
    n.token = me.token.clone();
    n.assistants.clear();
    c.listen = false;
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p maya-core --lib net::promote`
Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add core/src/net/promote.rs core/src/net/mod.rs
git commit -m "feat(net): the handover list and the config rewrites for a promotion"
```

---

### Task 3: Server — caps, `move` closing a connection, and `ServerHandle::promote`

**Files:**
- Modify: `core/src/net/mod.rs` (`AssistantStatus`)
- Modify: `core/src/net/server.rs`
- Modify: `src/settings.ts` (the `AssistantStatus` type only)

**Interfaces:**
- Consumes: `promote::{own_entry, handover_list, seal_list}`, `protocol::{CAP_FOLLOW, CAP_LEAD, Down::Move, CommandKind::Promote}`.
- Produces:
  - `AssistantStatus { …, can_lead: bool, can_follow: bool }` (JSON `canLead`, `canFollow`; both false while disconnected).
  - `pub struct Promotion { pub host: String, pub port: u16, pub me: PairedAssistant, pub moved: Vec<String>, pub left_behind: Vec<String> }`
  - `impl ServerHandle { pub fn promote(&self, id: &str) -> Result<Promotion, String> }`
  - `pub fn send_command_to(shared: &Arc<Mutex<Server>>, id: &str, kind: CommandKind, timeout: Duration) -> Result<Option<Value>, String>`
  - `pub(crate) const NO_ANSWER: &str = "did not answer in time";`

- [ ] **Step 1: Write the failing tests** in `server.rs`'s `mod tests` (they drive the server with raw sockets, like the tests around them):

```rust
    fn hello_caps(ws: &mut Client, name: &str, id: Option<&str>, caps: &[&str]) {
        send(ws, &Up::Hello { protocol: PROTOCOL, app: "t".into(), name: name.into(), hostname: "h".into(), platform: "macos".into(), id: id.map(str::to_string), caps: caps.iter().map(|c| c.to_string()).collect() });
    }

    /// Pairs, then reconnects with caps; returns the open socket, id and token.
    fn pair_with_caps(handle: &ServerHandle, port: u16, name: &str, caps: &[&str]) -> (Client, String, String) {
        let (ws, id, token) = pair_client(handle, port, name);
        drop(ws);
        wait_until(|| !handle.status().assistants.iter().any(|a| a.id == id && a.connected));
        let mut ws = connect(port);
        hello_caps(&mut ws, name, Some(&id), caps);
        let Down::Challenge { nonce } = recv(&mut ws) else { panic!("expected challenge") };
        answer(&mut ws, &token, &nonce);
        (ws, id, token)
    }

    #[test]
    fn caps_show_as_can_lead_and_can_follow_while_connected() {
        let (handle, port) = test_server();
        let (ws, id, _) = pair_with_caps(&handle, port, "laptop", &["follow", "lead"]);
        wait_until(|| handle.status().assistants.iter().any(|a| a.id == id && a.connected));
        let a = handle.status().assistants.into_iter().find(|a| a.id == id).unwrap();
        assert!(a.can_lead && a.can_follow);
        let json = serde_json::to_value(&a).unwrap();
        assert_eq!((json["canLead"].as_bool(), json["canFollow"].as_bool()), (Some(true), Some(true)));
        drop(ws);
        wait_until(|| !handle.status().assistants[0].connected);
        let a = &handle.status().assistants[0];
        assert!(!a.can_lead && !a.can_follow, "a disconnected Maya can do nothing");
        handle.stop();
    }

    #[test]
    fn promote_hands_the_sealed_list_over_moves_followers_and_names_the_rest() {
        let (handle, port) = test_server();
        let (mut g, g_id, g_token) = pair_with_caps(&handle, port, "laptop", &["follow", "lead"]);
        let (mut b, b_id, b_token) = pair_with_caps(&handle, port, "mini", &["follow"]);
        let (_old, old_id, _) = pair_with_caps(&handle, port, "old", &[]);
        handle.open_pairing(crate::store::now_ms());
        let h = handle.clone();
        let t = std::thread::spawn(move || h.promote(&g_id));
        let Down::Command { id: cmd, kind: CommandKind::Promote { sealed } } = recv(&mut g) else { panic!("expected promote") };
        assert!(handle.status().code.is_none(), "pairing closes for the handover");
        let list = crate::net::promote::open_list(&g_token, &sealed).unwrap();
        assert!(list.iter().any(|p| p.id == b_id && p.token == b_token), "B keeps its token");
        assert!(list.iter().any(|p| p.id == old_id));
        assert!(!list.iter().any(|p| p.token == g_token), "the new main is not in its own list");
        let me = list.last().unwrap().clone();
        send(&mut g, &Up::Result { id: cmd, ok: true, error: None, data: Some(serde_json::json!({ "port": 5001 })) });
        assert_eq!(recv(&mut b), Down::Move { host: "127.0.0.1".into(), port: 5001 });
        let out = t.join().unwrap().unwrap();
        assert_eq!((out.host.as_str(), out.port), ("127.0.0.1", 5001));
        assert_eq!(out.me, me);
        assert_eq!(out.moved, ["mini"]);
        assert_eq!(out.left_behind, ["old"], "a Maya without follow is not moved");
        assert!(!handle.status().assistants.iter().any(|a| a.id == b_id && a.connected), "a moved connection is closed");
        handle.stop();
    }

    #[test]
    fn promote_refuses_a_second_handover_and_one_that_cannot_lead() {
        let (handle, port) = test_server();
        let (mut g, g_id, _) = pair_with_caps(&handle, port, "laptop", &["follow", "lead"]);
        let (_c, c_id, _) = pair_with_caps(&handle, port, "cli", &["follow"]);
        assert_eq!(handle.promote(&c_id).unwrap_err(), "cli cannot become a main.");
        let h = handle.clone();
        let first = std::thread::spawn(move || h.promote(&g_id));
        let Down::Command { id: cmd, .. } = recv(&mut g) else { panic!("expected promote") };
        assert_eq!(handle.promote(&c_id).unwrap_err(), "A handover is already running.");
        send(&mut g, &Up::Result { id: cmd, ok: false, error: Some("Could not listen on port 4127: in use".into()), data: None });
        assert_eq!(first.join().unwrap().unwrap_err(), "laptop could not become the main: Could not listen on port 4127: in use.");
        assert!(handle.status().assistants.iter().all(|a| a.connected), "a failed handover moves nobody");
        handle.stop();
    }

    #[test]
    fn a_promote_nobody_answers_says_so_and_moves_nobody() {
        let (handle, port) = test_server();
        let (mut g, g_id, _) = pair_with_caps(&handle, port, "laptop", &["follow", "lead"]);
        let (_b, _, _) = pair_with_caps(&handle, port, "mini", &["follow"]);
        let err = handle.promote_with_timeout(&g_id, Duration::from_millis(300)).unwrap_err();
        assert_eq!(err, "laptop did not answer; it may have become a main on its own. Check it before trying again.");
        assert!(matches!(recv(&mut g), Down::Command { kind: CommandKind::Promote { .. }, .. }));
        assert!(handle.status().assistants.iter().all(|a| a.connected));
        handle.stop();
    }
```

In `src/settings.ts`, `AssistantStatus` gains:

```ts
  /** Connected and able to become the main (`lead`). */
  canLead?: boolean;
  /** Connected and able to follow a new main (`follow`); false for an older Maya. */
  canFollow?: boolean;
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p maya-core --lib net::server::tests::promote net::server::tests::caps net::server::tests::a_promote`
Expected: compile errors (`can_lead`, `promote`, `promote_with_timeout`).

- [ ] **Step 3: Implement.**

`core/src/net/mod.rs`, `AssistantStatus` gains after `note`:

```rust
    /// Connected and able to become the main.
    pub can_lead: bool,
    /// Connected and able to follow a new main; false for an older Maya.
    pub can_follow: bool,
```

and `paired_offline` sets `can_lead: false, can_follow: false`.

`server.rs`:

- `Peer` gains `caps: Vec<String>`; `admit` drops the `let _ = caps;` and inserts `Peer { app, caps: caps.into_iter().map(|c| clean(&c)).collect(), last_seen: now, unreadable_board: false }`.
- `Server` gains `handover: bool`, false in `Server::new`.
- `Server::status` computes, per entry:

```rust
                let connected = self.conns.contains_key(&p.id);
                let can = |cap: &str| connected && peer.is_some_and(|x| x.caps.iter().any(|c| c == cap));
```

and fills `connected`, `can_lead: can(CAP_LEAD)`, `can_follow: can(CAP_FOLLOW)`.
- `send_queued`: `let last = matches!(down, Down::Bye { .. } | Down::Move { .. });` and the returned reason is `if matches!(down, Down::Move { .. }) { "moved" } else { "removed" }` (compute before sending).
- Split `send_command_with`: keep its signature; it resolves `machine` to an id under the lock (`s.id_for(machine)`, else `Err(format!("{machine} is not connected"))`) and calls the new `send_command_to(shared, &id, kind, timeout)`, which does the rest of today's body with `machine` replaced by `lock(shared).label(id)`. Its timeout error becomes `format!("{machine} {NO_ANSWER}")` with

```rust
/// The end of the error when an assistant does not answer a command in time.
pub(crate) const NO_ANSWER: &str = "did not answer in time";
/// How long the new main has to answer `promote`.
const PROMOTE_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the old main waits for moved connections to close.
const MOVE_WAIT: Duration = Duration::from_secs(3);
```

- The handover, on `ServerHandle`:

```rust
/// What the old main needs once its assistant became the main: where the
/// new main is, the entry the old main made for itself, and who followed.
#[derive(Debug, Clone, PartialEq)]
pub struct Promotion {
    pub host: String,
    pub port: u16,
    pub me: PairedAssistant,
    pub moved: Vec<String>,
    pub left_behind: Vec<String>,
}

/// Clears `handover` when the handover ends, however it ends.
struct HandoverTurn<'a>(&'a Arc<Mutex<Server>>);

impl Drop for HandoverTurn<'_> {
    fn drop(&mut self) {
        lock(self.0).handover = false;
    }
}

impl ServerHandle {
    /// Makes the assistant `id` the main: closes pairing, sends it the paired
    /// list (sealed under its token) with this main's own new entry, and once
    /// it answers with its port, sends `move` to every other connected
    /// assistant that can follow and waits for those connections to close.
    /// The caller then saves this Maya as an assistant of `host:port`.
    pub fn promote(&self, id: &str) -> Result<Promotion, String> {
        self.promote_with_timeout(id, PROMOTE_TIMEOUT)
    }

    pub fn promote_with_timeout(&self, id: &str, timeout: Duration) -> Result<Promotion, String> {
        let (label, host, token, list, me) = {
            let mut s = lock(&self.shared);
            if s.handover {
                return Err("A handover is already running.".into());
            }
            let label = s.label(id);
            let entry = s.paired.iter().find(|p| p.id == id).cloned().ok_or_else(|| format!("{label} is no longer paired."))?;
            if !s.conns.contains_key(id) {
                return Err(format!("{label} is not connected."));
            }
            if !s.peers.get(id).is_some_and(|p| p.caps.iter().any(|c| c == CAP_LEAD)) {
                return Err(format!("{label} cannot become a main."));
            }
            s.handover = true;
            s.pairing = None;
            let me = super::promote::own_entry(&self.ctx.main_name);
            let list = super::promote::handover_list(&s.paired, id, &me);
            (label, entry.address, entry.token, list, me)
        };
        let _turn = HandoverTurn(&self.shared);
        self.ctx.announce();
        log::line("network", format!("{label}: handing over the main role ({} assistants)", list.len()));
        let sealed = super::promote::seal_list(&token, &list);
        let data = send_command_to(&self.shared, id, CommandKind::Promote { sealed }, timeout).map_err(|e| {
            if e.ends_with(NO_ANSWER) {
                format!("{label} did not answer; it may have become a main on its own. Check it before trying again.")
            } else {
                format!("{label} could not become the main: {}.", e.trim_end_matches('.'))
            }
        })?;
        let port = data.as_ref().and_then(|d| d.get("port")).and_then(Value::as_u64).and_then(|p| u16::try_from(p).ok()).ok_or_else(|| format!("{label} could not become the main: it did not say its port."))?;
        let (mut moved, mut left_behind, mut moving) = (vec![], vec![], vec![]);
        {
            let s = lock(&self.shared);
            for (other, conn) in s.conns.iter().filter(|(other, _)| other.as_str() != id) {
                let follows = s.peers.get(other).is_some_and(|p| p.caps.iter().any(|c| c == CAP_FOLLOW));
                if follows && conn.tx.send(Down::Move { host: host.clone(), port }).is_ok() {
                    moved.push(s.label(other));
                    moving.push(other.clone());
                } else {
                    left_behind.push(s.label(other));
                }
            }
        }
        // `move` ends each connection once written; the server must not stop before.
        let end = Instant::now() + MOVE_WAIT;
        while Instant::now() < end && moving.iter().any(|m| lock(&self.shared).conns.contains_key(m)) {
            std::thread::sleep(TICK);
        }
        moved.sort();
        left_behind.sort();
        log::line("network", format!("{label} is the main now at {host}:{port}; moved {}", if moved.is_empty() { "nobody".to_string() } else { moved.join(", ") }));
        Ok(Promotion { host, port, me, moved, left_behind })
    }
}
```

The `Err` returns inside the locked block happen before `handover` is set, so they need no turn.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p maya-core --lib net::`
Expected: all pass, the earlier server tests included.

- [ ] **Step 5: Commit**

```bash
git add core/src/net/mod.rs core/src/net/server.rs src/settings.ts
git commit -m "feat(net): the main hands its role over and moves its followers"
```

---

### Task 4: Client — caps, opening `promote`, following `move`

**Files:**
- Modify: `core/src/net/client.rs`

**Interfaces:**
- Consumes: `promote::open_list`, `protocol::{Down::Move, CommandKind::Promote}`.
- Produces, on `ClientNotify` (all with defaults, so every existing implementor still compiles):
  - `fn caps(&self) -> Vec<String> { vec![] }`
  - `fn promoted(&self, assistants: Vec<PairedAssistant>) -> Result<u16, String> { Err(CANNOT_LEAD.into()) }`
  - `fn moved(&self, host: &str, port: u16) -> Result<(), String> { Err(CANNOT_FOLLOW.into()) }`
- Constants: `pub const MOVED: &str = "The main Maya moved to another machine.";`, `pub const CANNOT_LEAD: &str = "This Maya cannot become a main.";`, `pub const CANNOT_FOLLOW: &str = "This Maya cannot follow the main to another machine.";`
- Behaviour: after a successful `promote` result is written, `run_once` returns `Ok(())` (so `reconnect` ends). On `move`, `moved` saves and `run_once` returns `Err(MOVED)`; `reconnect` tries again at once with the config read afresh.

- [ ] **Step 1: Write the failing tests.** In `server.rs`'s `mod tests` there is a client-side test notifier (around the `fn removed(&self)` at line ~1704) and `fake_main`. Add a new one next to them:

```rust
    /// A client-side notifier that records `moved` and answers `promoted`.
    #[derive(Default)]
    struct Leader {
        caps: Vec<String>,
        moved: Mutex<Option<(String, u16)>>,
        promoted: Mutex<Option<Vec<crate::config::PairedAssistant>>>,
        promote_result: Mutex<Option<Result<u16, String>>>,
        disconnects: Mutex<Vec<String>>,
    }

    impl client::ClientNotify for Leader {
        fn paired(&self, _: &str, _: &str) {}
        fn connected(&self, _: &str) {}
        fn disconnected(&self, e: &str) {
            self.disconnects.lock().unwrap().push(e.into());
        }
        fn removed(&self) {}
        fn caps(&self) -> Vec<String> {
            self.caps.clone()
        }
        fn promoted(&self, list: Vec<crate::config::PairedAssistant>) -> Result<u16, String> {
            *self.promoted.lock().unwrap() = Some(list);
            self.promote_result.lock().unwrap().clone().unwrap_or(Ok(6000))
        }
        fn moved(&self, host: &str, port: u16) -> Result<(), String> {
            *self.moved.lock().unwrap() = Some((host.into(), port));
            Ok(())
        }
    }

    struct NoSessions;
    impl client::Executor for NoSessions {
        fn execute(&self, _: CommandKind) -> Result<Option<Value>, String> {
            Err("no".into())
        }
        fn board(&self) -> (Vec<Card>, Vec<String>) {
            (vec![], vec![])
        }
    }

    fn assistant_config(port: u16, id: &str, token: &str) -> NetworkConfig {
        NetworkConfig { role: NetworkRole::Assistant, main_host: "127.0.0.1".into(), main_port: port, assistant_id: id.into(), token: token.into(), ..Default::default() }
    }

    #[test]
    fn a_client_says_its_caps_opens_promote_answers_with_its_port_and_ends() {
        let (handle, port) = test_server();
        let (ws, id, token) = pair_client(&handle, port, "laptop");
        drop(ws);
        wait_until(|| !handle.status().assistants[0].connected);
        let notify = Arc::new(Leader { caps: vec!["follow".into(), "lead".into()], ..Default::default() });
        let (config, n, stop) = (assistant_config(port, &id, &token), notify.clone(), Arc::new(AtomicBool::new(false)));
        let s = stop.clone();
        let run = std::thread::spawn(move || client::run_once(&config, Arc::new(NoSessions), n, &s, None));
        wait_until(|| handle.status().assistants[0].can_lead);
        let out = handle.promote(&id).unwrap();
        assert_eq!(out.port, 6000, "the port the new main gave");
        assert_eq!(notify.promoted.lock().unwrap().as_ref().unwrap().last().unwrap(), &out.me);
        assert_eq!(run.join().unwrap(), Ok(()), "a promoted client ends without an error");
        assert!(notify.disconnects.lock().unwrap().is_empty());
        handle.stop();
    }

    #[test]
    fn a_client_that_cannot_become_the_main_says_why_and_stays() {
        let (handle, port) = test_server();
        let (ws, id, token) = pair_client(&handle, port, "laptop");
        drop(ws);
        wait_until(|| !handle.status().assistants[0].connected);
        let notify = Arc::new(Leader { caps: vec!["follow".into(), "lead".into()], promote_result: Mutex::new(Some(Err("Could not listen on port 6000: in use".into()))), ..Default::default() });
        let (config, n, stop) = (assistant_config(port, &id, &token), notify.clone(), Arc::new(AtomicBool::new(false)));
        let s = stop.clone();
        let run = std::thread::spawn(move || client::run_once(&config, Arc::new(NoSessions), n, &s, None));
        wait_until(|| handle.status().assistants[0].can_lead);
        assert_eq!(handle.promote(&id).unwrap_err(), "laptop could not become the main: Could not listen on port 6000: in use.");
        assert!(handle.status().assistants[0].connected, "still an assistant of this main");
        stop.store(true, Ordering::SeqCst);
        let _ = run.join();
        handle.stop();
    }

    #[test]
    fn a_client_told_to_move_saves_the_new_host_and_reconnects_at_once() {
        let (handle, port) = test_server();
        let (ws, id, token) = pair_client(&handle, port, "mini");
        drop(ws);
        wait_until(|| !handle.status().assistants[0].connected);
        let notify = Arc::new(Leader { caps: vec!["follow".into()], ..Default::default() });
        let (config, n, stop) = (assistant_config(port, &id, &token), notify.clone(), Arc::new(AtomicBool::new(false)));
        let s = stop.clone();
        let run = std::thread::spawn(move || client::run_once(&config, Arc::new(NoSessions), n, &s, None));
        wait_until(|| handle.status().assistants[0].connected);
        lock(&handle.shared).conns.get(&id).unwrap().tx.send(Down::Move { host: "10.0.0.9".into(), port: 5001 }).unwrap();
        assert_eq!(run.join().unwrap(), Err(client::MOVED.into()));
        assert_eq!(*notify.moved.lock().unwrap(), Some(("10.0.0.9".into(), 5001)));
        handle.stop();
    }

    #[test]
    fn an_older_client_without_caps_cannot_be_promoted() {
        let (handle, port) = test_server();
        let (ws, id, token) = pair_client(&handle, port, "old");
        drop(ws);
        wait_until(|| !handle.status().assistants[0].connected);
        let (config, stop) = (assistant_config(port, &id, &token), Arc::new(AtomicBool::new(false)));
        let s = stop.clone();
        let run = std::thread::spawn(move || client::run_once(&config, Arc::new(NoSessions), Arc::new(Leader::default()), &s, None));
        wait_until(|| handle.status().assistants[0].connected);
        assert!(!handle.status().assistants[0].can_lead && !handle.status().assistants[0].can_follow);
        stop.store(true, Ordering::SeqCst);
        let _ = run.join();
        handle.stop();
    }
```

In `client.rs`'s own `mod tests`, the reconnect rule:

```rust
    #[test]
    fn a_move_retries_at_once() {
        assert_eq!(retry_wait_ms(MOVED, 4), 0);
        assert_eq!(retry_wait_ms(MAIN_UNPROVEN, 0), 30_000);
        assert_eq!(retry_wait_ms("Lost the connection", 2), 4_000);
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p maya-core --lib net::`
Expected: compile errors (`caps`, `promoted`, `moved` not trait methods; `MOVED`, `retry_wait_ms` unknown).

- [ ] **Step 3: Implement in `client.rs`.**

Constants after `PAIRING_UNPROVEN`:

```rust
/// Why a connection ended when the main moved; `reconnect` tries the new host at once.
pub const MOVED: &str = "The main Maya moved to another machine.";
pub const CANNOT_LEAD: &str = "This Maya cannot become a main.";
pub const CANNOT_FOLLOW: &str = "This Maya cannot follow the main to another machine.";
```

`ClientNotify` gains:

```rust
    /// What this Maya can do, sent in `hello` (`CAP_FOLLOW`, `CAP_LEAD`).
    fn caps(&self) -> Vec<String> {
        vec![]
    }
    /// The main handed this Maya its paired list: become the main with it.
    /// `Ok(port)` once its server listens; on `Err` nothing changed. Runs on
    /// a command thread; the client ends by itself once the answer is out,
    /// so this must not stop or join the client.
    fn promoted(&self, _assistants: Vec<crate::config::PairedAssistant>) -> Result<u16, String> {
        Err(CANNOT_LEAD.into())
    }
    /// The main is now at `host:port`: save it (the id and token stay).
    /// Runs on the connection thread.
    fn moved(&self, _host: &str, _port: u16) -> Result<(), String> {
        Err(CANNOT_FOLLOW.into())
    }
```

Threading the notifier through: `run_once` calls `connection(config, &exec, &notify, stop, code)`; `connection` takes `notify: &Arc<dyn ClientNotify>`, passes `&**notify` to `handshake` and `notify` plus `&config.token` to `pump`. `handshake`'s hello gets `caps: notify.caps()`.

`pump(ws, exec, notify, token, stop)` gains `let leaving = Arc::new(AtomicBool::new(false));` and its loop head becomes:

```rust
        // Read before draining, so a promote answer queued just before the
        // flag was set is written before the connection ends.
        let leave = leaving.load(Ordering::SeqCst);
        while let Ok(up) = rx.try_recv() {
            send_up(ws, &up)?;
        }
        if leave {
            log::line("network", "this Maya is the main now; the client ends");
            return Ok(());
        }
```

and calls `handle(down, exec, notify, token, &tx, &leaving)?`.

`handle` gains, before the general `Down::Command` arm:

```rust
        Down::Command { id, kind: CommandKind::Promote { sealed } } => {
            log::line("network", format!("command {id} promote"));
            let (notify, out, leaving, token) = (notify.clone(), tx.clone(), leaving.clone(), token.to_string());
            // On its own thread, as every command: becoming the main starts a server.
            let spawned = std::thread::Builder::new().name("net-promote".into()).spawn(move || {
                match super::promote::open_list(&token, &sealed).and_then(|list| notify.promoted(list)) {
                    Ok(port) => {
                        let _ = out.send(Up::Result { id, ok: true, error: None, data: Some(serde_json::json!({ "port": port })) });
                        leaving.store(true, Ordering::SeqCst);
                    }
                    Err(e) => {
                        log::line("network", format!("command {id} promote failed: {e}"));
                        let _ = out.send(Up::Result { id, ok: false, error: Some(e), data: None });
                    }
                }
            });
            if let Err(e) = spawned {
                let _ = tx.send(Up::Result { id, ok: false, error: Some(format!("no thread for the command: {e}")), data: None });
            }
        }
```

and replaces Task 1's `Down::Move` arm with:

```rust
        Down::Move { host, port } => {
            let host = clean(&host);
            log::line("network", format!("the main moved to {host}:{port}"));
            notify.moved(&host, port).map_err(|e| format!("Could not follow the main Maya to {host}: {e}"))?;
            return Err(MOVED.into());
        }
```

`reconnect` uses a helper for its wait:

```rust
/// How long `reconnect` waits after a failure: none after a move, 30 s
/// after a main that could not prove the token, else the backoff.
fn retry_wait_ms(error: &str, attempt: u32) -> u64 {
    match error {
        MOVED => 0,
        MAIN_UNPROVEN => UNPROVEN_WAIT_MS,
        _ => backoff_ms(attempt),
    }
}
```

In its `Err(e)` arm: `if e == MOVED { attempt = 0; }` before `let wait = retry_wait_ms(&e, attempt);`, and only `attempt = attempt.saturating_add(1)` when `e != MOVED`. With `wait == 0` it skips the sleep loop.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p maya-core --lib net::` then `cargo check --workspace`
Expected: all pass; the workspace builds (the defaults cover `TauriClientNotify`, `CliNotify`, `PairNotify`, the mobile test's `Quiet`).

- [ ] **Step 5: Commit**

```bash
git add core/src/net/client.rs core/src/net/server.rs
git commit -m "feat(net): the assistant opens a handover, follows a move, and says its caps"
```

---

### Task 5: Core — the whole handover end to end

**Files:**
- Modify: `core/src/net/server.rs` (`mod tests`)

**Interfaces:**
- Consumes: everything from Tasks 1–4; `start_with`, `client::reconnect`, `promote::{as_main, as_assistant_of}`.

This is the test that proves the pieces fit: three real clients, two real servers.

- [ ] **Step 1: Write the test**

```rust
    /// A Maya that plays either side: its client hands `promoted` a real
    /// server, and `moved` rewrites the config `reconnect` reads.
    struct Mobile {
        config: Arc<Mutex<NetworkConfig>>,
        server: Mutex<Option<ServerHandle>>,
        counter: Arc<Counter>,
        /// Fails `promoted` as a taken port would.
        port_taken: bool,
    }

    impl client::ClientNotify for Mobile {
        fn paired(&self, _: &str, _: &str) {}
        fn connected(&self, _: &str) {}
        fn disconnected(&self, _: &str) {}
        fn removed(&self) {}
        fn caps(&self) -> Vec<String> {
            vec!["follow".into(), "lead".into()]
        }
        fn promoted(&self, list: Vec<crate::config::PairedAssistant>) -> Result<u16, String> {
            if self.port_taken {
                return Err("Could not listen on port 4127: Address already in use. Choose another port.".into());
            }
            let mut c = crate::config::Config::default();
            c.network = self.config.lock().unwrap().clone();
            crate::net::promote::as_main(&mut c, list);
            let view = Arc::new(Mutex::new(c.network.clone()));
            let handle = start_with(self.counter.clone(), view, 0)?;
            let _ = self.counter.server.set(handle.shared.clone());
            *self.config.lock().unwrap() = c.network;
            let port = handle.port();
            *self.server.lock().unwrap() = Some(handle);
            Ok(port)
        }
        fn moved(&self, host: &str, port: u16) -> Result<(), String> {
            let mut c = self.config.lock().unwrap();
            c.main_host = host.into();
            c.main_port = port;
            Ok(())
        }
    }

    /// Pairs a `Mobile` with `handle` and runs its client until `stop`.
    fn run_mobile(handle: &ServerHandle, port: u16, name: &str, port_taken: bool) -> (Arc<Mobile>, Arc<AtomicBool>, std::thread::JoinHandle<()>) {
        let (ws, id, token) = pair_client(handle, port, name);
        drop(ws);
        wait_until(|| !handle.status().assistants.iter().any(|a| a.id == id && a.connected));
        let config = Arc::new(Mutex::new(NetworkConfig { name: name.into(), ..assistant_config(port, &id, &token) }));
        let m = Arc::new(Mobile { config: config.clone(), server: Mutex::new(None), counter: Arc::new(Counter::default()), port_taken });
        let stop = Arc::new(AtomicBool::new(false));
        let (n, s) = (m.clone(), stop.clone());
        let t = std::thread::spawn(move || client::reconnect(&move || config.lock().unwrap().clone(), Arc::new(NoSessions), n, &s));
        wait_until(|| handle.status().assistants.iter().any(|a| a.id == id && a.connected));
        (m, stop, t)
    }

    #[test]
    fn a_promoted_assistant_becomes_the_main_and_everyone_follows_it_with_their_old_tokens() {
        let (handle, port) = test_server();
        let (g, g_stop, g_thread) = run_mobile(&handle, port, "laptop", false);
        let (b, b_stop, b_thread) = run_mobile(&handle, port, "mini", false);
        let g_id = g.config.lock().unwrap().assistant_id.clone();
        let out = handle.promote(&g_id).unwrap();
        g_thread.join().unwrap();
        let new_main = g.server.lock().unwrap().clone().expect("the laptop runs a server");
        assert_eq!(out.port, new_main.port());
        assert_eq!(out.moved, ["mini"]);
        // B reconnects to the laptop by itself, with the token it always had.
        wait_until(|| new_main.status().assistants.iter().any(|a| a.name == "mini" && a.connected));
        assert_eq!(b.config.lock().unwrap().main_port, out.port);
        // The old main joins as an assistant with the entry it made.
        let mut old = crate::config::Config::default();
        crate::net::promote::as_assistant_of(&mut old, &out.host, out.port, &out.me);
        handle.stop();
        let (m_stop, m_cfg) = (Arc::new(AtomicBool::new(false)), Arc::new(Mutex::new(old.network)));
        let s = m_stop.clone();
        let m_thread = std::thread::spawn(move || client::reconnect(&move || m_cfg.lock().unwrap().clone(), Arc::new(NoSessions), Arc::new(Leader::default()), &s));
        wait_until(|| new_main.status().assistants.iter().any(|a| a.id == out.me.id && a.connected));
        assert!(g.counter.announced.lock().unwrap().is_empty(), "nothing on the first boards is announced");
        for s in [&b_stop, &m_stop, &g_stop] {
            s.store(true, Ordering::SeqCst);
        }
        let _ = (b_thread.join(), m_thread.join());
        new_main.stop();
    }

    #[test]
    fn an_assistant_whose_port_is_taken_stays_an_assistant_and_nobody_moves() {
        let (handle, port) = test_server();
        let (g, g_stop, g_thread) = run_mobile(&handle, port, "laptop", true);
        let (_b, b_stop, b_thread) = run_mobile(&handle, port, "mini", false);
        let g_id = g.config.lock().unwrap().assistant_id.clone();
        let err = handle.promote(&g_id).unwrap_err();
        assert!(err.starts_with("laptop could not become the main: Could not listen on port 4127"), "{err}");
        assert!(g.server.lock().unwrap().is_none());
        assert!(handle.status().assistants.iter().all(|a| a.connected), "both still assist this main");
        for s in [&g_stop, &b_stop] {
            s.store(true, Ordering::SeqCst);
        }
        let _ = (g_thread.join(), b_thread.join());
        handle.stop();
    }
```

- [ ] **Step 2: Run them**

Run: `cargo test -p maya-core --lib net::server::tests::a_promoted net::server::tests::an_assistant_whose`
Expected: PASS. If the first fails at "B reconnects", check the `move` reaches B before `promote` returns (Task 3's `send_queued` change) and that `reconnect` re-reads `config` after `MOVED` (Task 4).

- [ ] **Step 3: Commit**

```bash
git add core/src/net/server.rs
git commit -m "test(net): a whole handover between real clients and servers"
```

---

### Task 6: Desktop app — becoming the main, following, and Make main

**Files:**
- Modify: `src-tauri/src/net_app.rs`
- Modify: `src-tauri/src/lib.rs` (new command `network_promote`, registered in `generate_handler!`)

**Interfaces:**
- Consumes: `ServerHandle::promote`, `promote::{as_main, as_assistant_of}`, `client::{CAP_FOLLOW, CAP_LEAD}` (from `protocol`), `crate::update_config`, `net_app::start_server`.
- Produces: Tauri command `network_promote(id: String) -> Result<NetworkStatus, String>` (async).

The desktop crate has no unit tests around `AppHandle`; the logic it runs is the core's (Tasks 2–5). This task is checked by `cargo check`, `cargo test --workspace`, and by hand in Task 11.

- [ ] **Step 1: `TauriClientNotify` gains the three methods.**

```rust
    fn caps(&self) -> Vec<String> {
        vec![CAP_FOLLOW.into(), CAP_LEAD.into()]
    }

    /// Starts this Maya's server with the list, then saves the config as a
    /// main without `apply_config_change`: that would stop this very client
    /// before its answer is out. The client ends by itself once it is.
    /// Nothing is saved when the server cannot start, and the server stops
    /// again when the save fails, so either way this Maya stays an assistant.
    fn promoted(&self, list: Vec<config::PairedAssistant>) -> Result<u16, String> {
        let state = self.app.state::<AppState>();
        let (mut after, path) = {
            let store = state.store.lock().unwrap();
            (store.config.clone(), store.config_path())
        };
        let handed = list.len().saturating_sub(1);
        promote::as_main(&mut after, list);
        let handle = start_with(Arc::new(crate::TauriNetNotify { app: self.app.clone() }), Arc::new(Mutex::new(after.network.clone())), after.listen_port())?;
        if let Err(e) = config::save(&path, &after) {
            handle.stop();
            return Err(format!("Could not save the settings: {e}"));
        }
        let port = handle.port();
        state.store.lock().unwrap().config.network = after.network.clone();
        let by = {
            let mut net = state.network.lock().unwrap();
            let by = net.status.assistant.main_name.take().unwrap_or_else(|| "the main Maya".into());
            if let Some(old) = net.server.replace(handle) {
                old.stop();
            }
            net.status.main_error = None;
            net.status.assistant = Default::default();
            // Dropped, not stopped: stopping would end the connection before the answer is written.
            net.client.take();
            by
        };
        log::line("network", format!("promoted to main by {by}; {handed} assistants handed over"));
        let _ = self.app.emit("network", crate::network_status_of(&state));
        crate::refresh_and_emit(&self.app);
        Ok(port)
    }

    /// Saves the new main's host and port. The config change restarts the
    /// client against it (`RestartAssistant`), which stops this one.
    fn moved(&self, host: &str, port: u16) -> Result<(), String> {
        crate::update_config(&self.app, |c| {
            c.network.main_host = host.into();
            c.network.main_port = port;
        })
        .map(|_| ())
    }
```

Imports to add at the top: `use crate::config;` (already imported as `crate::config::NetworkRole`; extend that line), `use crate::net::promote;`, `use crate::net::protocol::{CAP_FOLLOW, CAP_LEAD};`. `network_status_of` and `refresh_and_emit` live in the crate root, so this child module already reaches them. `handed` counts the list without the old main's own entry; `by` is the main name the client last reported.

- [ ] **Step 2: `network_promote` in `lib.rs`**, after `network_remove_assistant`:

```rust
/// Makes a connected assistant the main, moves the others to it, then saves
/// this Maya as its assistant, which stops the server and starts the client.
#[tauri::command(async)]
fn network_promote(app: AppHandle, state: TauriState<AppState>, id: String) -> Result<NetworkStatus, String> {
    let server = state.network.lock().unwrap().server.clone().ok_or("Turn on \"Act as main Maya\" first.")?;
    let out = server.promote(&id)?;
    update_config(&app, |c| net::promote::as_assistant_of(c, &out.host, out.port, &out.me)).map_err(|e| format!("Could not save this Maya as an assistant: {}.", e.trim_end_matches('.')))?;
    log::line("network", format!("now an assistant of {}:{}", out.host, out.port));
    Ok(network_status_of(&state))
}
```

Register `network_promote` in `generate_handler!` next to `network_remove_assistant`.

- [ ] **Step 3: Build and test**

Run: `cargo check -p maya && cargo clippy -p maya -- -D warnings && cargo test --workspace`
Expected: clean; all tests pass. (The package name of `src-tauri/` is in its `Cargo.toml`; use it if it is not `maya`.)

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/net_app.rs src-tauri/src/lib.rs
git commit -m "feat: the desktop Maya can be made the main and can hand the role over"
```

---

### Task 7: CLI — following a move

**Files:**
- Modify: `cli/src/notify.rs`

**Interfaces:**
- Consumes: `ClientNotify::{caps, moved}`, `protocol::CAP_FOLLOW`, `config::{load, save}`.
- Produces: `CliNotify::caps() == ["follow"]`; `moved` rewrites `<maya_dir>/config.json`'s `mainHost`/`mainPort`, keeping everything else.

- [ ] **Step 1: Write the failing test** at the end of `cli/src/notify.rs` (add `#[cfg(test)] mod tests` if there is none):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use maya_core::config::{self, Config, NetworkRole};

    #[test]
    fn a_move_saves_the_new_host_and_keeps_the_id_and_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut c = Config::default();
        c.network.role = NetworkRole::Assistant;
        c.network.main_host = "10.0.0.2".into();
        c.network.assistant_id = "id".into();
        c.network.token = "tok".into();
        config::save(&path, &c).unwrap();
        let n = CliNotify::new(dir.path().to_path_buf(), Arc::new(AtomicBool::new(false)));
        assert_eq!(n.caps(), ["follow"]);
        n.moved("10.0.0.9", 5001).unwrap();
        let after = config::load(&path);
        assert_eq!((after.network.main_host.as_str(), after.network.main_port), ("10.0.0.9", 5001));
        assert_eq!((after.network.assistant_id.as_str(), after.network.token.as_str()), ("id", "tok"));
        assert_eq!(after.network.role, NetworkRole::Assistant);
    }
}
```

If `tempfile` is not a dev-dependency of `cli/`, add `tempfile.workspace = true` under `[dev-dependencies]` in `cli/Cargo.toml`.

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p maya-cli notify`
Expected: FAIL (`caps` is `[]`, `moved` errs "This Maya cannot follow…").

- [ ] **Step 3: Implement** in `impl ClientNotify for CliNotify`:

```rust
    fn caps(&self) -> Vec<String> {
        vec![maya_core::net::protocol::CAP_FOLLOW.into()]
    }

    /// Rewrites the main's host and port in `config.json`; `run` reads it
    /// again for the next connection, which `reconnect` makes at once.
    fn moved(&self, host: &str, port: u16) -> Result<(), String> {
        let path = self.maya_dir.join("config.json");
        let mut c = maya_core::config::load(&path);
        c.network.main_host = host.into();
        c.network.main_port = port;
        maya_core::config::save(&path, &c).map_err(|e| format!("could not save config.json: {e}"))
    }
```

Check the config path the CLI uses (`Store::config_path()`); if it is not `<maya_dir>/config.json`, use the same function.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p maya-cli`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add cli/src/notify.rs cli/Cargo.toml Cargo.lock
git commit -m "feat(cli): follow the main to a new host"
```

---

### Task 8: Phone hub — listening, promoting, and being promoted

**Files:**
- Modify: `mobile/src/hub.rs`
- Modify: `mobile/src/commands.rs` (`network_promote`, `listen_stop`)
- Modify: `mobile/src/lib.rs` (register both; start the client at launch when the role is Assistant)
- Test: `mobile/tests/round_trip.rs`

**Interfaces:**
- Consumes: `ServerHandle::promote`, `promote::{as_main, as_assistant_of}`, `client::{reconnect, ClientHandle, ClientNotify, Executor, NOT_PAIRED, REMOVED}`, `net::AssistantLink`.
- Produces on `Hub`:
  - `pub client: Mutex<Option<ClientHandle>>`, `pub link: Mutex<AssistantLink>`
  - `pub fn start_client(self: &Arc<Self>)`
  - `pub fn stop_listening(&self) -> Result<NetworkStatus, String>`
  - `pub fn promote(self: &Arc<Self>, id: &str) -> Result<NetworkStatus, String>`
  - `status()` returns `role: Assistant` with `assistant: link` while the client runs.
- Lock order addition: `client` and `link` are taken alone, never while holding `server` or `settings`.

- [ ] **Step 1: Write the failing tests** in `mobile/tests/round_trip.rs`. Reuse its `Recording`, `Page`, `wait_until`, `free_port`, and the way its existing tests build a hub (copy that construction into a helper if there is none):

```rust
/// A desktop-like assistant: its client becomes a main on `promote`.
struct Lead {
    config: Arc<Mutex<NetworkConfig>>,
    server: Mutex<Option<maya_core::net::server::ServerHandle>>,
}

impl ClientNotify for Lead {
    fn paired(&self, _: &str, _: &str) {}
    fn connected(&self, _: &str) {}
    fn disconnected(&self, _: &str) {}
    fn removed(&self) {}
    fn caps(&self) -> Vec<String> {
        vec!["follow".into(), "lead".into()]
    }
    fn promoted(&self, list: Vec<maya_core::config::PairedAssistant>) -> Result<u16, String> {
        let mut c = Config::default();
        c.network = self.config.lock().unwrap().clone();
        maya_core::net::promote::as_main(&mut c, list);
        let h = maya_core::net::server::start_with(Arc::new(NoNotify), Arc::new(Mutex::new(c.network.clone())), 0)?;
        *self.config.lock().unwrap() = c.network;
        let port = h.port();
        *self.server.lock().unwrap() = Some(h);
        Ok(port)
    }
}

struct NoNotify;
impl maya_core::net::server::Notify for NoNotify {
    fn board_changed(&self) {}
    fn board_seeded(&self, _: &[Card]) {}
    fn status_changed(&self, _: NetworkStatus) {}
    fn paired(&self, _: &maya_core::config::PairedAssistant) -> Result<(), String> {
        Ok(())
    }
    fn paired_list_changed(&self, _: &[maya_core::config::PairedAssistant]) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn the_phone_hands_the_main_role_over_listens_and_is_made_the_main_again() {
    let (hub, page, _dir) = new_hub(free_port());
    hub.start_server().unwrap();
    let port = hub.settings.lock().unwrap().config.listen_port();
    let code = hub.pairing_code().unwrap().code.unwrap().code;
    let exec: Arc<dyn Executor> = Arc::new(FakeAssistant { cards: Mutex::new(vec![]), due: AtomicBool::new(false), ran: Mutex::new(vec![]) });
    let (_, id, token) = pair_with(exec.clone(), "127.0.0.1", port, "laptop", &code).unwrap();
    let config = Arc::new(Mutex::new(NetworkConfig { role: NetworkRole::Assistant, main_host: "127.0.0.1".into(), main_port: port, name: "laptop".into(), assistant_id: id.clone(), token, port: free_port(), ..Default::default() }));
    let lead = Arc::new(Lead { config: config.clone(), server: Mutex::new(None) });
    let stop = Arc::new(AtomicBool::new(false));
    let (n, s, cfg) = (lead.clone(), stop.clone(), config.clone());
    let laptop = std::thread::spawn(move || maya_core::net::client::reconnect(&move || cfg.lock().unwrap().clone(), exec, n, &s));
    wait_until("the laptop can lead", || hub.status().assistants.iter().any(|a| a.id == id && a.can_lead));

    // The phone makes the laptop the main and listens to it.
    let status = hub.promote(&id).unwrap();
    assert_eq!(status.role, NetworkRole::Assistant);
    laptop.join().unwrap();
    let new_main = lead.server.lock().unwrap().clone().unwrap();
    wait_until("the phone listens to the laptop", || hub.status().assistant.connected);
    let phone = new_main.status().assistants.into_iter().find(|a| a.connected).unwrap();
    assert!(phone.can_lead, "the phone can be made the main again");
    assert!(hub.cards().is_empty());
    assert!(page.statuses.lock().unwrap().iter().any(|s| s.role == NetworkRole::Assistant));

    // The laptop makes the phone the main again.
    let back = new_main.promote(&phone.id).unwrap();
    wait_until("the phone runs its server", || hub.status().role == NetworkRole::Main);
    assert_eq!(back.port, hub.settings.lock().unwrap().config.listen_port());
    assert!(hub.client.lock().unwrap().is_none());
    new_main.stop();
    hub.stop_server();
}

#[test]
fn stop_listening_turns_the_phone_off_and_forgets_its_link() {
    let (hub, _page, _dir) = new_hub(free_port());
    hub.update_config(|c| {
        c.network.role = NetworkRole::Assistant;
        c.network.main_host = "127.0.0.1".into();
        c.network.main_port = free_port();
        c.network.assistant_id = "id".into();
        c.network.token = "tok".into();
    })
    .unwrap();
    hub.start_client();
    let status = hub.stop_listening().unwrap();
    assert_eq!(status.role, NetworkRole::Off);
    let c = hub.settings.lock().unwrap().config.network.clone();
    assert_eq!(c.role, NetworkRole::Off);
    assert!(c.assistant_id.is_empty() && c.token.is_empty() && c.main_host.is_empty());
    assert!(hub.client.lock().unwrap().is_none());
}
```

The hub helper, built as the file's first test builds its hub:

```rust
fn new_hub(port: u16) -> (Arc<Hub>, Arc<Page>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let maya_dir = dir.path().join("maya");
    let mut config = Config::default();
    config.network.port = port;
    config.network.name = "Pixel".into();
    config.network.role = NetworkRole::Main;
    let settings = Settings { path: maya_dir.join("config.json"), config };
    let page = Arc::new(Page::default());
    let hub = Hub::new(maya_dir, settings, Arc::new(Recording::default()), page.clone());
    (hub, page, dir)
}
```

Also a unit test in `mobile/src/hub.rs` for the executor:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use maya_core::net::client::Executor;

    #[test]
    fn a_listening_phone_has_no_sessions_and_runs_no_commands() {
        let e = NoSessions;
        assert_eq!(e.board(), (vec![], vec![]));
        assert_eq!(e.execute(CommandKind::Compact { session: "s".into() }).unwrap_err(), "This phone runs no sessions.");
    }
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p maya-mobile`
Expected: compile errors (`promote`, `start_client`, `stop_listening`, `client`, `NoSessions` unknown).

- [ ] **Step 3: Implement in `hub.rs`.**

`Hub` gains `pub client: Mutex<Option<ClientHandle>>` and `pub link: Mutex<AssistantLink>` (both `Default` in `with_clock`).

```rust
/// The phone while it listens: no sessions, so every command but
/// `promote` (which the client opens itself) is refused.
pub(crate) struct NoSessions;

impl Executor for NoSessions {
    fn execute(&self, _: CommandKind) -> Result<Option<Value>, String> {
        Err("This phone runs no sessions.".into())
    }
    fn board(&self) -> (Vec<Card>, Vec<String>) {
        (vec![], vec![])
    }
}

/// The client's line to the hub.
struct HubClientNotify {
    hub: Weak<Hub>,
    stop: Arc<std::sync::atomic::AtomicBool>,
}

impl HubClientNotify {
    fn set(&self, f: impl FnOnce(&mut AssistantLink)) {
        if self.stop.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let Some(h) = self.hub.upgrade() else { return };
        let line = {
            let mut l = h.link.lock().unwrap();
            f(&mut l);
            match (&l.main_name, l.connected) {
                (Some(m), true) => format!("Listening to {m}"),
                _ => "Reconnecting…".to_string(),
            }
        };
        h.alerts.service_line(&line);
        h.sink.network(&h.status());
    }
}

impl ClientNotify for HubClientNotify {
    fn paired(&self, _: &str, _: &str) {}
    fn connected(&self, main_name: &str) {
        let name = main_name.to_string();
        self.set(|l| *l = AssistantLink { connected: true, main_name: Some(name), error: None, retrying: false });
    }
    fn disconnected(&self, error: &str) {
        let retrying = error != NOT_PAIRED && error != REMOVED;
        self.set(|l| {
            l.connected = false;
            l.error = Some(error.to_string());
            l.retrying = retrying;
        });
    }
    fn removed(&self) {
        self.set(|l| {
            l.connected = false;
            l.error = Some(REMOVED.into());
            l.retrying = false;
        });
    }
    fn caps(&self) -> Vec<String> {
        vec![CAP_FOLLOW.into(), CAP_LEAD.into()]
    }
    /// Saves the config as a main with the list and starts the server; if
    /// it cannot listen, the old config goes back. The client ends by
    /// itself once the answer is out, so it is dropped here, not stopped.
    fn promoted(&self, list: Vec<PairedAssistant>) -> Result<u16, String> {
        let h = self.hub.upgrade().ok_or("the app is gone")?;
        let before = h.settings.lock().unwrap().config.clone();
        h.update_config(|c| promote::as_main(c, list))?;
        if let Err(e) = h.start_server() {
            let _ = h.update_config(|c| *c = before);
            return Err(e);
        }
        h.client.lock().unwrap().take();
        *h.link.lock().unwrap() = AssistantLink::default();
        log::line("network", "promoted to main");
        let port = h.handle().map(|s| s.port()).ok_or("The server is stopped.")?;
        h.sink.network(&h.status());
        Ok(port)
    }
    fn moved(&self, host: &str, port: u16) -> Result<(), String> {
        let h = self.hub.upgrade().ok_or("the app is gone")?;
        h.update_config(|c| {
            c.network.main_host = host.into();
            c.network.main_port = port;
        })
        .map(|_| ())
    }
}
```

Note `start_server` calls `stop_server`, which calls `refresh` and `sink.network`; that is fine on the command thread (no lock held). Make `handle()` `pub(crate)` if needed.

On `Hub`:

```rust
    /// Listens to the main in the config: an assistant with no sessions, so
    /// that main can make this phone the main again. The keep-awake service runs meanwhile.
    pub fn start_client(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let handle = ClientHandle::spawn(move |stop| {
            let notify: Arc<dyn ClientNotify> = Arc::new(HubClientNotify { hub: weak.clone(), stop: stop.clone() });
            let config_of = move || weak.upgrade().map(|h| h.settings.lock().unwrap().config.network.clone()).unwrap_or_default();
            reconnect(&config_of, Arc::new(NoSessions), notify, &stop);
        });
        *self.link.lock().unwrap() = AssistantLink { retrying: true, ..Default::default() };
        if let Some(old) = self.client.lock().unwrap().replace(handle) {
            old.stop();
        }
        self.sink.service_start("Reconnecting…");
        self.sink.network(&self.status());
    }

    /// Stops listening: the role goes Off and the link is forgotten.
    pub fn stop_listening(&self) -> Result<NetworkStatus, String> {
        if let Some(c) = self.client.lock().unwrap().take() {
            c.stop();
            self.sink.service_stop();
        }
        *self.link.lock().unwrap() = AssistantLink::default();
        self.update_config(|c| {
            c.network.role = NetworkRole::Off;
            c.network.main_host.clear();
            c.network.main_port = 0;
            c.network.assistant_id.clear();
            c.network.token.clear();
        })?;
        let status = self.status();
        self.sink.network(&status);
        Ok(status)
    }

    /// Makes a connected assistant the main, then listens to it.
    pub fn promote(self: &Arc<Self>, id: &str) -> Result<NetworkStatus, String> {
        let server = self.handle().ok_or("Start the server first.")?;
        let out = server.promote(id)?;
        self.update_config(|c| promote::as_assistant_of(c, &out.host, out.port, &out.me)).map_err(|e| format!("Could not save this Maya as an assistant: {}.", e.trim_end_matches('.')))?;
        self.stop_server();
        self.start_client();
        Ok(self.status())
    }
```

`status()`: before matching on the server, `if self.client.lock().unwrap().is_some() { return NetworkStatus { role: NetworkRole::Assistant, assistant: self.link.lock().unwrap().clone(), ..Default::default() }; }`.

Imports: `maya_core::net::client::{reconnect, ClientHandle, ClientNotify, Executor, NOT_PAIRED, REMOVED}`, `maya_core::net::protocol::{CAP_FOLLOW, CAP_LEAD}`, `maya_core::net::{promote, AssistantLink}`.

`commands.rs`:

```rust
#[tauri::command(async)]
pub fn network_promote(hub: H, id: String) -> Result<NetworkStatus, String> {
    hub.promote(&id)
}

#[tauri::command(async)]
pub fn listen_stop(hub: H) -> Result<NetworkStatus, String> {
    hub.stop_listening()
}
```

`lib.rs`: register `commands::network_promote, commands::listen_stop`; in `setup`, after `was_main`, add the same block for `NetworkRole::Assistant` calling `hub.start_client()` (on the main thread is fine; it only spawns).

- [ ] **Step 4: Run the tests**

Run: `cargo test -p maya-mobile`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add mobile/src/hub.rs mobile/src/commands.rs mobile/src/lib.rs mobile/tests/round_trip.rs
git commit -m "feat(mobile): the phone hands the main role over, listens, and can be made the main again"
```

---

### Task 9: Page — the confirmation's lines and Make main on the desktop

**Files:**
- Create: `src/promote.ts`, `src/promote.test.ts`
- Modify: `src/settings.ts` (model fields, handlers, the paired list rendering, the `network` listener)
- Test: the existing settings test file (`src/settings.test.ts`; find it with `ls src/*settings*.test.ts`)

**Interfaces:**
- Consumes: `AssistantStatus.canLead`, `canFollow` (Task 3), command `network_promote` (Task 6), `thisComputerLower()` from `src/platform.ts`.
- Produces: `export function promoteLines(assistants: AssistantStatus[], id: string, thisDevice: string): string[]` — the headline first, then one line per Maya left behind, then one per offline Maya.

- [ ] **Step 1: Write the failing tests** `src/promote.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { promoteLines } from "./promote";
import type { AssistantStatus } from "./settings";

const a = (id: string, name: string, more: Partial<AssistantStatus> = {}): AssistantStatus => ({ id, name, hostname: "h", platform: "macos", address: `10.0.0.${id.length}`, connected: true, lastSeen: null, canLead: true, canFollow: true, ...more });

describe("promoteLines", () => {
  it("says who will assist, who cannot follow and who is offline", () => {
    const list = [a("g", "laptop", { address: "192.168.55.70" }), a("bb", "mini"), a("ccc", "old", { canFollow: false, canLead: false }), a("dddd", "nas", { connected: false, canFollow: false, canLead: false })];
    expect(promoteLines(list, "g", "this Mac")).toEqual([
      "Make laptop the main Maya? Every connected Maya will assist laptop, and so will this Mac.",
      "old cannot follow and stays with no main; pair it with laptop afterwards.",
      "nas is offline; when it is back, set its Host to 192.168.55.70. It does not need to pair again.",
    ]);
  });

  it("is only the headline when everyone follows", () => {
    expect(promoteLines([a("g", "laptop"), a("bb", "mini")], "g", "this phone")).toEqual(["Make laptop the main Maya? Every connected Maya will assist laptop, and so will this phone."]);
  });
});
```

In the settings test file, following how its other Network tests render the panel and read buttons:

Add `onMakeMain: vi.fn(), onPromote: vi.fn(), onPromoteCancel: vi.fn(),` to the file's `handlers()`, then:

```ts
describe("Make main", () => {
  const laptop = { id: "g", name: "laptop", hostname: "laptop", platform: "macos", address: "192.168.55.70", connected: true, lastSeen: null, canLead: true, canFollow: true };
  const cli = { ...laptop, id: "c", name: "cli", canLead: false };
  const nas = { ...laptop, id: "n", name: "nas", connected: false, canLead: false, canFollow: false };
  const network = { role: "main" as const, code: null, assistants: [laptop, cli, nas], assistant: { connected: false, mainName: null, error: null } };

  it("shows only for a connected assistant that can lead", () => {
    const h = handlers();
    const el = renderSettings({ ...voiceBase, networkRole: "main", network }, h);
    const buttons = [...el.querySelectorAll<HTMLButtonElement>("button[data-action=make-main]")];
    expect(buttons.map((b) => b.dataset.id)).toEqual(["g"]);
    buttons[0].click();
    expect(h.onMakeMain).toHaveBeenCalledWith("g");
    expect(el.querySelector("[data-for=promote-confirm]")).toBeNull();
  });

  it("confirms with who stays behind before handing over", () => {
    const h = handlers();
    const el = renderSettings({ ...voiceBase, networkRole: "main", network, networkPromote: "g" }, h);
    const confirm = el.querySelector<HTMLElement>("[data-for=promote-confirm]")!;
    expect(confirm.textContent).toContain("Make laptop the main Maya? Every connected Maya will assist laptop");
    expect(confirm.textContent).toContain("nas is offline; when it is back, set its Host to 192.168.55.70.");
    confirm.querySelector<HTMLButtonElement>("button[data-action=promote-confirm]")!.click();
    expect(h.onPromote).toHaveBeenCalledWith("g");
    confirm.querySelector<HTMLButtonElement>("button[data-action=promote-cancel]")!.click();
    expect(h.onPromoteCancel).toHaveBeenCalled();
  });

  it("disables the buttons while the handover runs", () => {
    const el = renderSettings({ ...voiceBase, networkRole: "main", network, networkPromote: "g", networkPromoting: true }, handlers());
    expect(el.querySelector<HTMLButtonElement>("button[data-action=promote-confirm]")!.disabled).toBe(true);
    expect(el.querySelector<HTMLButtonElement>("button[data-action=promote-confirm]")!.textContent).toBe("Handing over…");
    expect([...el.querySelectorAll<HTMLButtonElement>("button[data-action=remove-assistant]")].every((b) => b.disabled)).toBe(true);
  });
});
```

- [ ] **Step 2: Run them to see them fail**

Run: `pnpm test -- src/promote.test.ts src/settings.test.ts`
Expected: FAIL (module `./promote` missing; no `make-main` button).

- [ ] **Step 3: Implement** `src/promote.ts`:

```ts
// The confirmation shown before a main hands its role to an assistant:
// what will happen, who cannot follow, and who is offline.

import type { AssistantStatus } from "./settings";

export function promoteLines(assistants: AssistantStatus[], id: string, thisDevice: string): string[] {
  const g = assistants.find((a) => a.id === id);
  if (!g) return [];
  const others = assistants.filter((a) => a.id !== id);
  return [
    `Make ${g.name} the main Maya? Every connected Maya will assist ${g.name}, and so will ${thisDevice}.`,
    ...others.filter((a) => a.connected && !a.canFollow).map((a) => `${a.name} cannot follow and stays with no main; pair it with ${g.name} afterwards.`),
    ...others.filter((a) => !a.connected).map((a) => `${a.name} is offline; when it is back, set its Host to ${g.address}. It does not need to pair again.`),
  ];
}
```

`src/settings.ts`:
- `SettingsModel` gains `networkPromote?: string | null;` (the assistant whose confirmation is open) and `networkPromoting?: boolean;`.
- Handlers interface gains `onMakeMain(id: string): void; onPromote(id: string): void; onPromoteCancel(): void;`.
- In the paired list loop, after the Remove button:

```ts
      if (a.connected && a.canLead) {
        const mk = document.createElement("button");
        mk.type = "button";
        mk.dataset.action = "make-main";
        mk.dataset.id = a.id;
        mk.textContent = "Make main";
        mk.disabled = !!model.networkPromoting;
        mk.addEventListener("click", () => h.onMakeMain(a.id));
        row.append(mk);
      }
      list.append(row);
      if (model.networkPromote === a.id) {
        const confirm = document.createElement("div");
        confirm.className = "settings__confirm";
        confirm.dataset.for = "promote-confirm";
        for (const line of promoteLines(net.assistants, a.id, thisComputerLower())) {
          const p = document.createElement("p");
          p.textContent = line;
          confirm.append(p);
        }
        const yes = document.createElement("button");
        yes.type = "button";
        yes.dataset.action = "promote-confirm";
        yes.textContent = model.networkPromoting ? "Handing over…" : "Make main";
        yes.disabled = !!model.networkPromoting;
        yes.addEventListener("click", () => h.onPromote(a.id));
        const no = document.createElement("button");
        no.type = "button";
        no.dataset.action = "promote-cancel";
        no.textContent = "Cancel";
        no.disabled = !!model.networkPromoting;
        no.addEventListener("click", () => h.onPromoteCancel());
        confirm.append(yes, no);
        list.append(confirm);
      }
```

(move the existing `list.append(row)` into this block rather than appending twice). While `networkPromoting` is set, also disable the row's Remove buttons and the Regenerate button.
- The handlers object:

```ts
    onMakeMain: (id) => {
      model.networkPromote = id;
      paint();
    },
    onPromoteCancel: () => {
      model.networkPromote = null;
      paint();
    },
    onPromote: (id) => {
      if (model.networkPromoting) return;
      model.networkPromoting = true;
      paint();
      void run(async () => {
        try {
          model.network = await invoke<NetworkStatus>("network_promote", { id });
          model.networkRole = model.network.role;
          model.networkPromote = null;
          refetchAndPaint();
        } finally {
          model.networkPromoting = false;
        }
      });
    },
```

(`run` already shows a thrown error where Remove's errors show; check that it does, and that `refetchAndPaint` is in scope there, moving its declaration up if needed.)
- In the `network` listener, after `model.network = e.payload;`: when the role or `assistant.mainName` differs from the previous payload's, call `refetchAndPaint()` so the role, Host and Port fields follow a promotion or a move made from another machine.
- Add `.settings__confirm` to `src/styles.css` next to `.settings__assistant`: the same padding as a row, a 1px border in the existing border token, 8px gap between its children.

- [ ] **Step 4: Run the tests**

Run: `pnpm test`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/promote.ts src/promote.test.ts src/settings.ts src/settings.test.ts src/styles.css
git commit -m "feat: Make main in Settings, with a confirmation that names who stays behind"
```

---

### Task 10: Phone page — Make main and the Listening screen

**Files:**
- Modify: `src/mobile/network.ts`, `src/mobile/network.test.ts`

**Interfaces:**
- Consumes: `promoteLines` (Task 9), commands `network_promote`, `listen_stop` (Task 8), `NetworkStatus.assistant`.
- Produces: `NetworkModel.promote: string | null`; `NetworkHandlers.onMakeMain(id)`, `onPromote(id)`, `onPromoteCancel()`, `onStopListening()`.

- [ ] **Step 1: Write the failing tests** in `src/mobile/network.test.ts`, using its existing model builder and handler fakes:

Add `onMakeMain: vi.fn(), onPromote: vi.fn(), onPromoteCancel: vi.fn(), onStopListening: vi.fn()` to the file's `handlers()`, and `promote: null` to `base`. Then, inside `describe("renderNetwork", …)`:

```ts
  it("shows Listening with Stop listening while the phone assists a main", () => {
    const h = handlers();
    const listening: NetworkModel = { ...base, status: { role: "assistant", code: null, assistants: [], assistant: { connected: true, mainName: "laptop", error: null } } };
    const el = renderNetwork(listening, h, NOW);
    expect(el.textContent).toContain("Listening to laptop");
    expect(el.querySelector("button[data-action=start]")).toBeNull();
    expect(el.querySelector(".network__code")).toBeNull();
    el.querySelector<HTMLButtonElement>("button[data-action=stop-listening]")!.click();
    expect(h.onStopListening).toHaveBeenCalled();
  });

  it("says Reconnecting with the last error while it cannot reach the main", () => {
    const lost: NetworkModel = { ...base, status: { role: "assistant", code: null, assistants: [], assistant: { connected: false, mainName: null, error: "The main Maya stopped answering.", retrying: true } } };
    const text = renderNetwork(lost, handlers(), NOW).textContent;
    expect(text).toContain("Reconnecting…");
    expect(text).toContain("The main Maya stopped answering.");
  });

  it("offers Make main for a connected assistant that can lead and confirms with the lines", () => {
    const h = handlers();
    const laptop = { id: "g", name: "laptop", hostname: "h", platform: "macos", address: "192.168.55.70", connected: true, lastSeen: null, canLead: true, canFollow: true };
    const cli = { ...laptop, id: "c", name: "cli", canLead: false };
    const m: NetworkModel = { ...base, status: { ...running, assistants: [laptop, cli] }, promote: "g" };
    const el = renderNetwork(m, h, NOW);
    expect([...el.querySelectorAll<HTMLButtonElement>("button[data-action=make-main]")].length).toBe(1);
    expect(el.textContent).toContain("Make laptop the main Maya? Every connected Maya will assist laptop, and so will this phone.");
    el.querySelector<HTMLButtonElement>("button[data-action=promote-confirm]")!.click();
    expect(h.onPromote).toHaveBeenCalledWith("g");
  });
```

- [ ] **Step 2: Run them to see them fail**

Run: `pnpm test -- src/mobile/network.test.ts`
Expected: FAIL.

- [ ] **Step 3: Implement** in `renderNetwork`:
- `const listening = m.status.role === "assistant";`. When listening, the "This phone" section shows the name field and Save as today, but in place of Start/Stop server a status line and one button:

```ts
  if (listening) {
    const link = m.status.assistant;
    const line = link.connected && link.mainName ? `Listening to ${link.mainName}` : "Reconnecting…";
    phone.append(el("p", "network__listening", line));
    if (!link.connected && link.error) phone.append(el("p", "settings__hint", link.error));
    phone.append(button("Stop listening", "stop-listening", h.onStopListening));
  }
```

and the `row` holds only Save then. The Pairing and Assistants sections are not shown while listening.
- In the assistants loop, after Remove:

```ts
    if (a.connected && a.canLead) r.append(button("Make main", "make-main", () => h.onMakeMain(a.id)));
    assistants.append(r);
    if (m.promote === a.id) {
      const c = el("div", "network__confirm");
      c.dataset.for = "promote-confirm";
      for (const line of promoteLines(m.status.assistants, a.id, "this phone")) c.append(el("p", "", line));
      c.append(button(m.busy ? "Handing over…" : "Make main", "promote-confirm", () => h.onPromote(a.id), true), button("Cancel", "promote-cancel", h.onPromoteCancel));
      assistants.append(c);
    }
```

(move the existing `assistants.append(r)` into this block).
- Handlers in the page's setup:

```ts
    onMakeMain: (id) => { model.promote = id; paint(); },
    onPromoteCancel: () => { model.promote = null; paint(); },
    onPromote: (id) => void busy(async () => { setStatus(await invoke<NetworkStatus>("network_promote", { id })); model.promote = null; }),
    onStopListening: () => void busy(async () => setStatus(await invoke<NetworkStatus>("listen_stop"))),
```

- Add `.network__confirm` and `.network__listening` to `src/mobile/mobile.css` beside `.network__assistant`.

- [ ] **Step 4: Run the tests**

Run: `pnpm test`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/mobile/network.ts src/mobile/network.test.ts src/mobile/mobile.css
git commit -m "feat(mobile): Make main and the Listening screen on the phone"
```

---

### Task 11: Docs, version, and the check on real machines

**Files:**
- Modify: `docs/DEVELOPING.md` (the net layout paragraph: `promote.rs`; the Android section: the phone can listen), `README.md` (the network section: Make main), the spec's `Status:` line to `implemented`
- Modify (via script): `src-tauri/tauri.conf.json`, `package.json`, `Cargo.toml`, `Cargo.lock`, `mobile/tauri.conf.json` if `set-version.sh` covers it

- [ ] **Step 1: Docs.** In `docs/DEVELOPING.md`, where `core/src/net/` is described (`protocol.rs`, `server.rs`, `client.rs`…), add: "`promote.rs` (handing the main role to an assistant: the list, its seal, the config rewrites)". In the Android section's first paragraph, after "It is only ever a main", add: "or, after it handed the main role to an assistant, a listener with no sessions that the new main can make the main again". In `README.md`'s network section, add a short paragraph: on the main, Settings › Network › Make main beside a connected assistant hands it the role; every connected Maya follows it with no new pairing; an offline one needs its Host set to the new main's address. Set the spec's `Status:` to `implemented`.

- [ ] **Step 2: Full check**

Run: `cargo test --workspace && cargo clippy --workspace -- -D warnings && pnpm test`
Expected: all pass, no warnings.

- [ ] **Step 3: Commit the docs**

```bash
git add docs/DEVELOPING.md README.md docs/superpowers/specs/2026-10-12-promote-assistant-design.md
git commit -m "docs: promoting an assistant to main"
```

- [ ] **Step 4: Bump the version** from `main`'s version (`git show main:package.json | grep '"version"'`; 0.15.0 today, so 0.16.0 for a `feat`):

```bash
sh scripts/set-version.sh 0.16.0
git diff --stat   # Cargo.lock must be in it; if not, run `cargo check` (cargo may be off sh's PATH) and look again
git add -A && git commit -m "chore(release): 0.16.0"
```

- [ ] **Step 5: By hand, with the user** (the laptop "Gnowee" is 192.168.55.70 over SSH; its UI and the phone need the user):
  1. Mac is main, laptop and phone assist it. On the Mac, Make main beside the laptop; read the confirmation; confirm.
  2. The laptop's Settings show "Act as main Maya" with the Mac and the phone in its list; its board shows the Mac's sessions; the Mac shows "Connected to Gnowee"; the phone shows "Listening to Gnowee".
  3. On the laptop, Make main beside the phone. The phone shows its server running with the laptop and the Mac; their cards appear on the phone.
  4. On the phone, Make main beside the Mac. The Mac is the main again with the laptop and the phone.
  5. Nothing asked for a pairing code at any step, and no machine announced a session that was already waiting.
