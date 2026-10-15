# Secure protocol between Mayas — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Maya's plain-text, token-authenticated WebSocket protocol
with protocol 2: SPAKE2 pairing over the six-digit code, Noise handshakes,
public keys as identity, and every frame encrypted and authenticated.

**Architecture:** Two new core modules carry the cryptography:
`core/src/net/identity.rs` (each Maya's X25519 key pair) and
`core/src/net/secure.rs` (the handshakes and the encrypted `Channel` that
wraps a tungstenite `WebSocket`). `server.rs` and `client.rs` run a
handshake right after the WebSocket upgrade and then exchange today's JSON
`Up`/`Down` messages through the `Channel` instead of text frames. Tokens,
HMAC challenges and the handover seal go away; the paired list carries
public keys. The desktop app keeps its private key in the OS key store
(`src-tauri/src/keystore.rs`, shared with the ElevenLabs key); the CLI and
the phone keep it in an owner-only `identity.key` beside their config.

**Tech Stack:** Rust 2021 (rustc 1.98), `snow` 0.10 (Noise), `spake2` 0.4
(RustCrypto), tungstenite 0.30, Tauri 2, vanilla TypeScript + vitest.

**Spec:** `docs/superpowers/specs/2026-10-14-secure-protocol-design.md`

## Global Constraints

- `PROTOCOL` becomes `2`. A protocol 2 Maya never sends or accepts a plain-text
  frame after the WebSocket upgrade, except the one plain `bye { reason:
  "protocol" }` it answers a protocol 1 peer with.
- Noise patterns, exactly: `Noise_XXpsk3_25519_ChaChaPoly_SHA256` (pairing,
  the joining Maya is the initiator, psk at location 3) and
  `Noise_IK_25519_ChaChaPoly_SHA256` (every later connection, the assistant
  is the initiator). Prologue for both: `b"maya protocol 2"`.
- SPAKE2: `Spake2::<Ed25519Group>`, the joining Maya `start_a`, the main
  `start_b`, identities `b"maya-join"` (A) and `b"maya-main"` (B), password
  = the six digits, trimmed.
- Handshake framing, exactly:
  - The joining Maya's first binary frame starts with a mode byte: `1` =
    pair (then SPAKE2's 33-byte message), `2` = connect (then the IK
    message). Its later handshake frames are bare Noise messages.
  - Every frame the main sends during the handshake starts with a kind
    byte: `0x00` = a handshake message follows (possibly empty), `0xFF` =
    a refusal, its UTF-8 reason follows. A refusal is never authenticated
    and only ever becomes an error message.
  - A pairing ends with the main's `[0x00]` once the joiner's last message
    proved the code, or `[0xFF]` + `"wrong or expired pairing code"`.
  - After the handshake there are no refusals: every frame is sealed.
- A pairing code allows 3 pairings that start SPAKE2 (`PAIR_TRIES = 3`);
  every one that does not reach `welcome` uses one up, from any address.
- Each Noise message carries at most 65 535 bytes, of which 16 are the tag.
  A frame's payload is one or more `[u16 big-endian length][Noise message]`.
  `MAX_FRAME` (64 MB) caps the joined plaintext.
- Public keys travel and are stored as standard base64 of 32 bytes.
- User-facing strings, verbatim:
  - `"Wrong or expired pairing code."` (exists, `client::WRONG_CODE`)
  - `"Code used up after 3 wrong tries — Regenerate."`
  - `"The main Maya's key has changed. If you reset it, pair again; otherwise someone may be impersonating it."`
  - `"The main runs an older Maya. Update it."`
  - `"The main Maya did not answer. If it runs an older Maya, update it."`
  - `"A frame failed authentication; connection closed."` (Debug tab)
  - `"This Maya was updated to secure pairing — pair again."`
  - `"Could not reach the key store: <reason>"`
- Never log, print, `Debug`-format or send a private key. `Keypair`'s
  `Debug` shows the public key only.
- Commits: conventional commits, each ending with
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Work on a branch;
  never commit to or push `main`.
- `cargo test -p maya-core` is the gate for Tasks 1–4. The apps (`maya`,
  `maya-mobile`, `maya-cli`) do not compile from Task 3 until Tasks 5–7
  update them; `cargo test --workspace` is green again from Task 7.

## Review Focus

1. **Two joiners race with the right code.** Both finish the XX handshake;
   only the first to send `hello` gets an entry, the second gets `bye
   "wrong or expired pairing code"`. Test in Task 3
   (`two_joiners_with_the_right_code_pair_one_machine`).
2. **A truncated or garbled frame.** A length prefix past the end, an empty
   frame, invalid UTF-8: `open_frame` returns `Forged`, never panics. Test in
   Task 2 (`garbled_frames_are_forged_not_panics`).
3. **`move` with a key that is not a key.** The assistant refuses to follow,
   keeps its old main and says why. Test in Task 3
   (`a_move_with_a_bad_key_is_not_followed`).
4. **Make main on an entry with no key** (a protocol 1 leftover that
   survived somehow). Refused with a message, nothing handed over. Test in
   Task 3 (`promote_refuses_an_entry_without_a_key`).
5. **A key store that answers with garbage** (a value that is not 32 bytes
   of base64). The desktop app reports "Could not reach the key store: …"
   and never overwrites the stored value with a fresh key. Test in Task 5
   (`a_garbled_stored_key_is_an_error_not_a_new_key`).

---

## File Structure

| File | Responsibility |
|---|---|
| `core/src/net/identity.rs` (new) | `Keypair`: generate, derive the public key, base64 encode/decode, load-or-create an owner-only key file |
| `core/src/net/secure.rs` (new) | The handshakes (`pair`, `connect`, `accept`), refusal frames, old-peer detection, frame sealing/opening, `Channel` |
| `core/src/net/tamper_tests.rs` (new, test only) | A WebSocket proxy that flips, replays, reorders and injects frames between a real client and server |
| `core/src/net/protocol.rs` | Message types for protocol 2; pairing window with tries; the token, MAC and seal code removed |
| `core/src/config.rs` | `PairedAssistant.key`, `NetworkConfig.main_key` / `pair_again`; protocol 1 pairings dropped on load |
| `core/src/net/server.rs` | Handshake after upgrade, identity by key, code tries, `Channel` everywhere, `move` with key, promote with keys |
| `core/src/net/client.rs` | Handshake after connect, `Channel` everywhere, new error messages, `pair_with`/`reconnect` take an identity |
| `core/src/net/promote.rs` | Handover list of keys; no seal |
| `core/src/net/mod.rs` | `NetworkStatus.code_used_up` / `pair_again`; `network_change` compares `main_key` |
| `src-tauri/src/keystore.rs` (new) | Keychain / Credential Manager / GNOME Keyring, found-or-not-found-or-error, for any item |
| `src-tauri/src/voice.rs` | ElevenLabs key through `keystore` |
| `src-tauri/src/net_app.rs`, `src-tauri/src/lib.rs` | Desktop identity, `network_new_key`, `network_forget_main`, status fields |
| `mobile/src/hub.rs`, `mobile/src/commands.rs`, `mobile/tests/round_trip.rs` | Phone identity file, `network_new_key` |
| `cli/src/commands.rs`, `cli/src/args.rs`, `cli/src/run_cmd.rs`, `cli/src/notify.rs`, `cli/src/lib.rs` | CLI identity file, `pair --new-key` |
| `src/settings.ts`, `src/mobile/network.ts` (+ tests) | Code used up, pair again note, New key, Forget main |
| `README.md`, `docs/DEVELOPING.md`, the spec | Trust section, layout, spec deviations |

---

### Task 1: Dependencies and the identity key pair

**Files:**
- Modify: `Cargo.toml` (workspace dependencies), `core/Cargo.toml`
- Create: `core/src/net/identity.rs`
- Modify: `core/src/net/mod.rs` (declare the module), `core/src/net/protocol.rs` (make `random` crate-visible)

**Interfaces:**
- Produces: `maya_core::net::identity::{Keypair, KEY_LEN, FILE, encode_key, decode_key, load_or_create, replace, BAD_KEY}`
  - `Keypair::generate() -> Keypair`
  - `Keypair::from_private([u8; 32]) -> Keypair`
  - `Keypair::from_secret(&str) -> Result<Keypair, String>` (base64 private key)
  - `Keypair::to_secret(&self) -> String`
  - `Keypair::private(&self) -> &[u8; 32]`
  - `Keypair::public: [u8; 32]` (field), `Keypair::public_b64(&self) -> String`
  - `encode_key(&[u8; 32]) -> String`, `decode_key(&str) -> Result<[u8; 32], String>`
  - `load_or_create(&Path) -> Result<Keypair, String>`, `replace(&Path) -> Result<Keypair, String>`
  - `pub const FILE: &str = "identity.key";`

- [ ] **Step 1: Add the crates**

In the root `Cargo.toml` `[workspace.dependencies]`, add beside `rand`:

```toml
snow = "0.10"
spake2 = "0.4"
```

In `core/Cargo.toml` `[dependencies]`, add:

```toml
snow.workspace = true
spake2.workspace = true
```

Run: `cargo build -p maya-core`
Expected: builds (the crates are unused so far; a warning is fine).

- [ ] **Step 2: Make the random helper crate-visible**

In `core/src/net/protocol.rs` change `fn random(n: usize) -> Vec<u8>` to
`pub(crate) fn random(n: usize) -> Vec<u8>`.

- [ ] **Step 3: Write the failing tests**

Create `core/src/net/identity.rs` with only the tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_pair_derives_its_public_key_and_round_trips_as_a_secret() {
        let k = Keypair::generate();
        let again = Keypair::from_secret(&k.to_secret()).unwrap();
        assert_eq!(again.public, k.public);
        assert_eq!(again.private(), k.private());
        assert_ne!(Keypair::generate().public, k.public, "fresh pairs differ");
        assert_eq!(decode_key(&k.public_b64()).unwrap(), k.public);
    }

    #[test]
    fn debug_shows_the_public_key_only() {
        let k = Keypair::generate();
        let shown = format!("{k:?}");
        assert!(shown.contains(&k.public_b64()), "{shown}");
        assert!(!shown.contains(&k.to_secret()), "the private key never prints");
    }

    #[test]
    fn a_key_that_is_not_32_bytes_of_base64_is_refused() {
        assert_eq!(decode_key("").unwrap_err(), BAD_KEY);
        assert_eq!(decode_key("not base64!").unwrap_err(), BAD_KEY);
        assert_eq!(decode_key("AAAA").unwrap_err(), BAD_KEY, "three bytes");
        assert!(Keypair::from_secret("AAAA").is_err());
    }

    #[test]
    fn the_key_file_is_made_once_owner_only_and_replaced_on_request() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("maya").join(FILE);
        let first = load_or_create(&path).unwrap();
        assert_eq!(load_or_create(&path).unwrap().public, first.public, "loaded, not made again");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let fresh = replace(&path).unwrap();
        assert_ne!(fresh.public, first.public);
        assert_eq!(load_or_create(&path).unwrap().public, fresh.public);
    }

    #[test]
    fn a_garbled_key_file_is_an_error_not_a_new_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        std::fs::write(&path, "garbage").unwrap();
        assert!(load_or_create(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "garbage", "the file is left alone");
    }
}
```

Add `pub mod identity;` to `core/src/net/mod.rs` after `pub mod promote;`.

- [ ] **Step 4: Run the tests to see them fail**

Run: `cargo test -p maya-core net::identity`
Expected: FAIL to compile, `Keypair` not found.

- [ ] **Step 5: Write the module**

Put this above the tests in `core/src/net/identity.rs`:

```rust
//! This Maya's long-term X25519 key pair: its identity on the network
//! (protocol 2). Peers pin the public half; the private half never leaves
//! this machine and is never logged.

use base64::Engine;
use snow::params::DHChoice;
use snow::resolvers::{CryptoResolver, DefaultResolver};
use std::path::Path;

pub const KEY_LEN: usize = 32;
/// The key file of the CLI and the phone, beside their `config.json`.
pub const FILE: &str = "identity.key";
pub const BAD_KEY: &str = "That is not a Maya key.";

#[derive(Clone)]
pub struct Keypair {
    private: [u8; KEY_LEN],
    pub public: [u8; KEY_LEN],
}

impl std::fmt::Debug for Keypair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Keypair({})", self.public_b64())
    }
}

impl Keypair {
    pub fn generate() -> Keypair {
        let bytes: [u8; KEY_LEN] = super::protocol::random(KEY_LEN).try_into().expect("32 random bytes");
        Keypair::from_private(bytes)
    }

    pub fn from_private(private: [u8; KEY_LEN]) -> Keypair {
        let mut dh = DefaultResolver.resolve_dh(&DHChoice::Curve25519).expect("snow resolves Curve25519");
        dh.set(&private);
        let public = dh.pubkey().try_into().expect("a 32-byte public key");
        Keypair { private, public }
    }

    /// The private key as stored: base64.
    pub fn from_secret(secret: &str) -> Result<Keypair, String> {
        decode_key(secret).map(Keypair::from_private)
    }

    pub fn to_secret(&self) -> String {
        encode_key(&self.private)
    }

    pub fn private(&self) -> &[u8; KEY_LEN] {
        &self.private
    }

    pub fn public_b64(&self) -> String {
        encode_key(&self.public)
    }
}

pub fn encode_key(key: &[u8; KEY_LEN]) -> String {
    base64::engine::general_purpose::STANDARD.encode(key)
}

pub fn decode_key(text: &str) -> Result<[u8; KEY_LEN], String> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(text.trim()).map_err(|_| BAD_KEY.to_string())?;
    bytes.try_into().map_err(|_| BAD_KEY.to_string())
}

/// The key pair in `path`, an owner-only file; made and saved there when
/// the file does not exist. A file that does not hold a key is an error,
/// never overwritten: a new key would silently drop every pairing.
pub fn load_or_create(path: &Path) -> Result<Keypair, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Keypair::from_secret(&text).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => replace(path),
        Err(e) => Err(format!("Could not read {}: {e}", path.display())),
    }
}

/// A fresh key pair, saved over `path`.
pub fn replace(path: &Path) -> Result<Keypair, String> {
    let k = Keypair::generate();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    crate::config::write_private(path, k.to_secret().as_bytes()).map_err(|e| format!("Could not save {}: {e}", path.display()))?;
    Ok(k)
}
```

- [ ] **Step 6: Run the tests to see them pass**

Run: `cargo test -p maya-core net::identity`
Expected: 5 passed.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock core/Cargo.toml core/src/net/identity.rs core/src/net/mod.rs core/src/net/protocol.rs
git commit -m "feat(net): give each Maya an X25519 identity key pair"
```

---

### Task 2: The secure channel: handshakes and sealed frames

This module is the protocol's whole cryptography, so it is one task with
one review. The code below was compiled and its tests run against
`snow` 0.10.0 and `spake2` 0.4.0 while writing this plan (17 tests with
Task 1's, all passing, no warnings).

**Files:**
- Create: `core/src/net/secure.rs`
- Modify: `core/src/net/mod.rs` (declare the module)

**Interfaces:**
- Consumes: `identity::{Keypair, KEY_LEN}`, `protocol::MAX_FRAME`
- Produces (in `maya_core::net::secure`):
  - `pub fn pair<S: Read + Write>(ws: &mut WebSocket<S>, me: &Keypair, code: &str, deadline: Instant, stopped: &dyn Fn() -> bool) -> Result<(TransportState, [u8; 32]), Fail>` (the joining Maya; returns the main's key)
  - `pub fn connect<S: Read + Write>(ws: &mut WebSocket<S>, me: &Keypair, main: &[u8; 32], deadline: Instant, stopped: &dyn Fn() -> bool) -> Result<TransportState, Fail>` (an assistant)
  - `pub fn accept<S: Read + Write>(ws: &mut WebSocket<S>, me: &Keypair, deadline: Instant, stopped: &dyn Fn() -> bool, code: &mut dyn FnMut() -> Result<String, String>) -> Result<(TransportState, Accepted), Fail>` (the main; `code` is called once, only for a pairing, and gives the open code or the refusal reason)
  - `pub fn refuse<S: Read + Write>(ws: &mut WebSocket<S>, reason: &str)`
  - `pub enum Fail { OldPeer, Refused(String), Crypto, Io(String), Malformed }`
  - `pub struct Accepted { pub key: [u8; 32], pub paired: bool }`
  - `pub struct Channel<S> { pub ws: WebSocket<S>, .. }`: `Channel::new(ws, noise)`, `send(&mut self, &str) -> Result<(), String>`, `write(&mut self, &str) -> Result<(), String>` (no flush), `flush(&mut self) -> Result<(), String>`, `read(&mut self) -> Result<Got, ChannelError>`
  - `pub enum Got { Text(String), Control, Idle, Closed }`
  - `pub enum ChannelError { Forged, Io(String) }` (its `Display` for `Forged` is `FORGED`)
  - `pub fn seal_frame(&mut TransportState, &[u8]) -> Result<Vec<u8>, String>`, `pub fn open_frame(&mut TransportState, &[u8]) -> Result<String, ChannelError>`
  - consts `MODE_PAIR`, `MODE_CONNECT`, `KIND_MESSAGE`, `KIND_REFUSAL`, `WRONG_CODE_REASON` (`"wrong or expired pairing code"`), `UNKNOWN_KEY_REASON` (`"unknown key"`), `FORGED`
  - `pub(crate) fn is_timeout(&tungstenite::Error) -> bool`
  - test only: `pub(crate) fn transport_pair() -> (TransportState, TransportState)`

- [ ] **Step 1: Write the failing tests**

Create `core/src/net/secure.rs` with only these tests (they name what Step 3
adds), and add `pub mod secure;` to `core/src/net/mod.rs` after
`pub mod identity;`:

```rust
#[cfg(test)]
pub(crate) fn transport_pair() -> (TransportState, TransportState) {
    let (a, m) = (Keypair::generate(), Keypair::generate());
    let mut i = builder(CONNECT_PATTERN, &a).unwrap().remote_public_key(&m.public).unwrap().build_initiator().unwrap();
    let mut r = builder(CONNECT_PATTERN, &m).unwrap().build_responder().unwrap();
    absorb(&mut r, &message(&mut i).unwrap()).unwrap();
    absorb(&mut i, &message(&mut r).unwrap()).unwrap();
    (i.into_transport_mode().unwrap(), r.into_transport_mode().unwrap())
}

#[cfg(test)]
mod frame_tests {
    use super::*;

    #[test]
    fn a_frame_round_trips_small_empty_and_twenty_megabytes() {
        let (mut a, mut b) = transport_pair();
        for text in [String::new(), "hello".to_string(), "x".repeat(20 * 1024 * 1024)] {
            let frame = seal_frame(&mut a, text.as_bytes()).unwrap();
            assert_eq!(open_frame(&mut b, &frame).unwrap(), text);
        }
    }

    #[test]
    fn a_changed_byte_a_replay_and_a_reorder_are_forged() {
        let (mut a, mut b) = transport_pair();
        let mut flipped = seal_frame(&mut a, b"one").unwrap();
        *flipped.last_mut().unwrap() ^= 1;
        assert_eq!(open_frame(&mut b, &flipped), Err(ChannelError::Forged));

        let (mut a, mut b) = transport_pair();
        let first = seal_frame(&mut a, b"one").unwrap();
        assert_eq!(open_frame(&mut b, &first).unwrap(), "one");
        assert_eq!(open_frame(&mut b, &first), Err(ChannelError::Forged), "a replay");

        let (mut a, mut b) = transport_pair();
        let _first = seal_frame(&mut a, b"one").unwrap();
        let second = seal_frame(&mut a, b"two").unwrap();
        assert_eq!(open_frame(&mut b, &second), Err(ChannelError::Forged), "out of order");
    }

    #[test]
    fn garbled_frames_are_forged_not_panics() {
        let (mut a, mut b) = transport_pair();
        assert_eq!(open_frame(&mut b, &[]), Err(ChannelError::Forged), "empty");
        assert_eq!(open_frame(&mut b, &[0x00]), Err(ChannelError::Forged), "half a length");
        assert_eq!(open_frame(&mut b, &[0xFF, 0xFF, 1, 2, 3]), Err(ChannelError::Forged), "a length past the end");
        let bad_utf8 = seal_frame(&mut a, &[0xC3, 0x28]).unwrap();
        assert_eq!(open_frame(&mut b, &bad_utf8), Err(ChannelError::Forged), "not UTF-8");
    }

    #[test]
    fn a_large_frame_is_split_into_noise_sized_messages() {
        let (mut a, _) = transport_pair();
        let frame = seal_frame(&mut a, &vec![b'x'; CHUNK * 2 + 10]).unwrap();
        let first = u16::from_be_bytes([frame[0], frame[1]]) as usize;
        assert_eq!(first, NOISE_MAX, "a full chunk plus its tag");
        assert_eq!(frame.len(), 3 * 2 + (CHUNK * 2 + 10) + 3 * TAG);
    }
}

#[cfg(test)]
mod handshake_tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};
    use std::time::Duration;

    type Ws = WebSocket<TcpStream>;

    /// A WebSocket pair over loopback: (joiner side, main side).
    fn ws_pair() -> (Ws, Ws) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let joiner = std::thread::spawn(move || {
            let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
            tungstenite::client::client(format!("ws://127.0.0.1:{port}/"), stream).unwrap().0
        });
        let (stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
        let main = tungstenite::accept(stream).unwrap();
        (joiner.join().unwrap(), main)
    }

    fn soon() -> Instant {
        Instant::now() + Duration::from_secs(5)
    }

    type MainOut = (Ws, Result<(TransportState, Accepted), Fail>);

    /// The main's side on a thread, with `code` as the open code or the refusal.
    fn main_side(mut ws: Ws, me: Keypair, code: Result<String, String>) -> std::thread::JoinHandle<MainOut> {
        std::thread::spawn(move || {
            let mut code = Some(code);
            let out = accept(&mut ws, &me, soon(), &|| false, &mut || code.take().expect("asked once"));
            (ws, out)
        })
    }

    fn read_text<S: Read + Write>(c: &mut Channel<S>) -> String {
        let end = soon();
        loop {
            match c.read().unwrap() {
                Got::Text(t) => return t,
                _ if Instant::now() > end => panic!("nothing within 5 s"),
                _ => {}
            }
        }
    }

    #[test]
    fn the_same_code_pairs_and_both_learn_each_others_key() {
        let (mut j, m) = ws_pair();
        let (joiner, main) = (Keypair::generate(), Keypair::generate());
        let t = main_side(m, main.clone(), Ok("123456".into()));
        let (jt, main_key) = pair(&mut j, &joiner, " 123456 ", soon(), &|| false).unwrap();
        let (m, out) = t.join().unwrap();
        let (mt, accepted) = out.unwrap();
        assert_eq!(main_key, main.public);
        assert_eq!(accepted, Accepted { key: joiner.public, paired: true });
        let (mut jc, mut mc) = (Channel::new(j, jt), Channel::new(m, mt));
        jc.send("hello").unwrap();
        assert_eq!(read_text(&mut mc), "hello");
        mc.send("welcome").unwrap();
        assert_eq!(read_text(&mut jc), "welcome");
    }

    #[test]
    fn a_wrong_code_fails_on_the_main_and_the_joiner_hears_why() {
        let (mut j, m) = ws_pair();
        let t = main_side(m, Keypair::generate(), Ok("654321".into()));
        assert_eq!(pair(&mut j, &Keypair::generate(), "123456", soon(), &|| false).unwrap_err(), Fail::Refused(WRONG_CODE_REASON.into()));
        assert_eq!(t.join().unwrap().1.unwrap_err(), Fail::Crypto);
    }

    #[test]
    fn no_open_code_is_refused_before_any_spake() {
        let (mut j, m) = ws_pair();
        let t = main_side(m, Keypair::generate(), Err(WRONG_CODE_REASON.into()));
        assert_eq!(pair(&mut j, &Keypair::generate(), "123456", soon(), &|| false).unwrap_err(), Fail::Refused(WRONG_CODE_REASON.into()));
        assert_eq!(t.join().unwrap().1.unwrap_err(), Fail::Refused(WRONG_CODE_REASON.into()));
    }

    #[test]
    fn connecting_with_the_pinned_key_works_and_the_main_learns_who() {
        let (mut j, m) = ws_pair();
        let (assistant, main) = (Keypair::generate(), Keypair::generate());
        let t = main_side(m, main.clone(), Err("not pairing".into()));
        let jt = connect(&mut j, &assistant, &main.public, soon(), &|| false).unwrap();
        let (m, out) = t.join().unwrap();
        let (mt, accepted) = out.unwrap();
        assert_eq!(accepted, Accepted { key: assistant.public, paired: false });
        let (mut jc, mut mc) = (Channel::new(j, jt), Channel::new(m, mt));
        jc.send("hello").unwrap();
        assert_eq!(read_text(&mut mc), "hello");
    }

    #[test]
    fn a_main_without_the_pinned_key_refuses_by_name() {
        let (mut j, m) = ws_pair();
        let t = main_side(m, Keypair::generate(), Err("not pairing".into()));
        let pinned = Keypair::generate().public;
        assert_eq!(connect(&mut j, &Keypair::generate(), &pinned, soon(), &|| false).unwrap_err(), Fail::Refused(UNKNOWN_KEY_REASON.into()));
        assert_eq!(t.join().unwrap().1.unwrap_err(), Fail::Crypto);
    }

    #[test]
    fn a_text_frame_first_is_an_old_peer_both_ways() {
        let (mut j, mut m) = ws_pair();
        j.send(Message::text(r#"{"type":"hello","protocol":1}"#)).unwrap();
        assert_eq!(accept(&mut m, &Keypair::generate(), soon(), &|| false, &mut || panic!("no code for an old peer")).unwrap_err(), Fail::OldPeer);
        let (mut j, mut m) = ws_pair();
        let t = std::thread::spawn(move || {
            let end = soon();
            while Instant::now() < end && !matches!(m.read(), Ok(Message::Binary(_))) {}
            m.send(Message::text(r#"{"type":"challenge","nonce":"n"}"#)).unwrap();
            m
        });
        assert_eq!(connect(&mut j, &Keypair::generate(), &Keypair::generate().public, soon(), &|| false).unwrap_err(), Fail::OldPeer);
        drop(t.join());
    }

    #[test]
    fn silence_is_an_io_failure_by_the_deadline() {
        let (mut j, _m) = ws_pair();
        let out = connect(&mut j, &Keypair::generate(), &Keypair::generate().public, Instant::now() + Duration::from_millis(300), &|| false);
        assert!(matches!(out, Err(Fail::Io(_))), "{out:?}");
    }

    #[test]
    fn a_text_frame_after_the_handshake_is_forged() {
        let (mut j, m) = ws_pair();
        let main = Keypair::generate();
        let t = main_side(m, main.clone(), Err("not pairing".into()));
        let jt = connect(&mut j, &Keypair::generate(), &main.public, soon(), &|| false).unwrap();
        let (mut m, out) = t.join().unwrap();
        let _mt = out.unwrap().0;
        m.send(Message::text(r#"{"type":"command","id":1,"kind":"compact","session":"s"}"#)).unwrap();
        let mut jc = Channel::new(j, jt);
        let end = soon();
        let got = loop {
            match jc.read() {
                Ok(Got::Idle) if Instant::now() < end => {}
                other => break other,
            }
        };
        assert_eq!(got, Err(ChannelError::Forged));
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core net::secure`
Expected: FAIL to compile, `builder`, `seal_frame`, `pair` and the rest not found.

- [ ] **Step 3: Write the module**

Put this above the tests:

```rust
//! The secure channel between two Mayas (protocol 2). Pairing is SPAKE2
//! over the six-digit code, then a Noise `XXpsk3` handshake keyed by it;
//! every later connection is a Noise `IK` handshake against the main's
//! pinned key. After either, each WebSocket binary frame holds one or more
//! Noise transport messages, `[u16 length][message]`, so every byte is
//! encrypted and authenticated and a changed, replayed, reordered or
//! injected frame fails to open.

use super::identity::{Keypair, KEY_LEN};
use super::protocol::MAX_FRAME;
use snow::{Builder, HandshakeState, TransportState};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use std::io::{Read, Write};
use std::time::Instant;
use tungstenite::{Message, WebSocket};

const PAIR_PATTERN: &str = "Noise_XXpsk3_25519_ChaChaPoly_SHA256";
const CONNECT_PATTERN: &str = "Noise_IK_25519_ChaChaPoly_SHA256";
const PROLOGUE: &[u8] = b"maya protocol 2";
/// Noise's limit on one message, its 16-byte tag included.
const NOISE_MAX: usize = 65_535;
const TAG: usize = 16;
/// The most plaintext one Noise message carries.
const CHUNK: usize = NOISE_MAX - TAG;

/// The joining Maya's first frame starts with one of these.
pub const MODE_PAIR: u8 = 1;
pub const MODE_CONNECT: u8 = 2;
/// Every frame the main sends during the handshake starts with one of these.
pub const KIND_MESSAGE: u8 = 0x00;
/// A plain refusal, its reason as UTF-8 after it. Never authenticated, so
/// it only ever becomes an error message.
pub const KIND_REFUSAL: u8 = 0xFF;
pub const WRONG_CODE_REASON: &str = "wrong or expired pairing code";
pub const UNKNOWN_KEY_REASON: &str = "unknown key";
pub const FORGED: &str = "A frame failed authentication; connection closed.";
const SPAKE_LEN: usize = 33;
const JOINER: &[u8] = b"maya-join";
const MAIN: &[u8] = b"maya-main";

#[derive(Debug, Clone, PartialEq)]
pub enum ChannelError {
    /// A frame did not open: changed, replayed, reordered, injected or garbled.
    Forged,
    Io(String),
}

impl std::fmt::Display for ChannelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChannelError::Forged => f.write_str(FORGED),
            ChannelError::Io(e) => f.write_str(e),
        }
    }
}

/// What one read of a `Channel` gave.
#[derive(Debug, Clone, PartialEq)]
pub enum Got {
    Text(String),
    /// A ping or pong: the peer is alive, nothing to decode.
    Control,
    /// The read timed out (the connection loop's tick).
    Idle,
    Closed,
}

/// Why a handshake did not complete.
#[derive(Debug, Clone, PartialEq)]
pub enum Fail {
    /// The other side spoke plain text first: a protocol 1 Maya.
    OldPeer,
    /// The main refused, in plain text, with this reason.
    Refused(String),
    /// A handshake message did not decrypt: a wrong code, a changed key, tampering.
    Crypto,
    /// Timed out, closed, stopped or an I/O error.
    Io(String),
    /// A frame of no known shape.
    Malformed,
}

/// What the main learned from a handshake.
#[derive(Debug, Clone, PartialEq)]
pub struct Accepted {
    /// The other Maya's public key.
    pub key: [u8; KEY_LEN],
    /// It paired (with the code) rather than connected (with its key).
    pub paired: bool,
}

pub(crate) fn is_timeout(e: &tungstenite::Error) -> bool {
    matches!(e, tungstenite::Error::Io(io) if matches!(io.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut))
}

/// `plain` as one frame: Noise messages of at most `CHUNK` bytes of
/// plaintext each, each after its length. Empty plaintext is one message.
pub fn seal_frame(noise: &mut TransportState, plain: &[u8]) -> Result<Vec<u8>, String> {
    let chunks: Vec<&[u8]> = if plain.is_empty() { vec![&[][..]] } else { plain.chunks(CHUNK).collect() };
    let mut out = Vec::with_capacity(plain.len() + chunks.len() * (TAG + 2));
    let mut buf = vec![0u8; NOISE_MAX];
    for chunk in chunks {
        let n = noise.write_message(chunk, &mut buf).map_err(|e| format!("could not encrypt: {e}"))?;
        out.extend_from_slice(&(n as u16).to_be_bytes());
        out.extend_from_slice(&buf[..n]);
    }
    Ok(out)
}

/// The text a frame holds; anything that does not open is `Forged`.
pub fn open_frame(noise: &mut TransportState, frame: &[u8]) -> Result<String, ChannelError> {
    if frame.is_empty() {
        return Err(ChannelError::Forged);
    }
    let mut plain = Vec::new();
    let mut buf = vec![0u8; NOISE_MAX];
    let mut rest = frame;
    while !rest.is_empty() {
        let len = rest.get(..2).ok_or(ChannelError::Forged)?;
        let n = u16::from_be_bytes([len[0], len[1]]) as usize;
        let body = rest.get(2..2 + n).ok_or(ChannelError::Forged)?;
        let k = noise.read_message(body, &mut buf).map_err(|_| ChannelError::Forged)?;
        plain.extend_from_slice(&buf[..k]);
        if plain.len() > MAX_FRAME {
            return Err(ChannelError::Io(format!("frame too large ({} bytes)", plain.len())));
        }
        rest = &rest[2 + n..];
    }
    String::from_utf8(plain).map_err(|_| ChannelError::Forged)
}

/// A WebSocket after its handshake: JSON text in, JSON text out, sealed.
pub struct Channel<S> {
    pub ws: WebSocket<S>,
    noise: TransportState,
}

impl<S: Read + Write> Channel<S> {
    pub fn new(ws: WebSocket<S>, noise: TransportState) -> Channel<S> {
        Channel { ws, noise }
    }

    pub fn send(&mut self, text: &str) -> Result<(), String> {
        let frame = seal_frame(&mut self.noise, text.as_bytes())?;
        self.ws.send(Message::binary(frame)).map_err(|e| e.to_string())
    }

    /// Queues a frame without flushing; `flush` sends what is queued.
    pub fn write(&mut self, text: &str) -> Result<(), String> {
        let frame = seal_frame(&mut self.noise, text.as_bytes())?;
        self.ws.write(Message::binary(frame)).map_err(|e| e.to_string())
    }

    pub fn flush(&mut self) -> Result<(), String> {
        self.ws.flush().map_err(|e| e.to_string())
    }

    /// One read. A text frame after the handshake is never genuine.
    pub fn read(&mut self) -> Result<Got, ChannelError> {
        match self.ws.read() {
            Ok(Message::Binary(b)) => open_frame(&mut self.noise, &b).map(Got::Text),
            Ok(Message::Text(_)) => Err(ChannelError::Forged),
            Ok(Message::Close(_)) => Ok(Got::Closed),
            Ok(_) => Ok(Got::Control),
            Err(e) if is_timeout(&e) => Ok(Got::Idle),
            Err(e) => Err(ChannelError::Io(e.to_string())),
        }
    }
}

fn builder<'a>(pattern: &str, me: &'a Keypair) -> Result<Builder<'a>, Fail> {
    let params = pattern.parse().map_err(|_| Fail::Malformed)?;
    Builder::new(params).local_private_key(me.private()).and_then(|b| b.prologue(PROLOGUE)).map_err(|_| Fail::Malformed)
}

fn send_frame<S: Read + Write>(ws: &mut WebSocket<S>, bytes: Vec<u8>) -> Result<(), Fail> {
    ws.send(Message::binary(bytes)).map_err(|e| Fail::Io(e.to_string()))
}

/// The main's handshake message: `KIND_MESSAGE` then `body`.
fn send_message<S: Read + Write>(ws: &mut WebSocket<S>, body: &[u8]) -> Result<(), Fail> {
    let mut frame = vec![KIND_MESSAGE];
    frame.extend_from_slice(body);
    send_frame(ws, frame)
}

/// The main's plain refusal, during the handshake only.
pub fn refuse<S: Read + Write>(ws: &mut WebSocket<S>, reason: &str) {
    let mut frame = vec![KIND_REFUSAL];
    frame.extend_from_slice(reason.as_bytes());
    let _ = send_frame(ws, frame);
}

/// The next binary frame by `deadline`; a text frame is a protocol 1 peer.
fn read_frame<S: Read + Write>(ws: &mut WebSocket<S>, deadline: Instant, stopped: &dyn Fn() -> bool) -> Result<Vec<u8>, Fail> {
    loop {
        if stopped() {
            return Err(Fail::Io("stopped".into()));
        }
        match ws.read() {
            Ok(Message::Binary(b)) => return Ok(b.to_vec()),
            Ok(Message::Text(_)) => return Err(Fail::OldPeer),
            Ok(Message::Close(_)) => return Err(Fail::Io("closed".into())),
            Ok(_) => {}
            Err(e) if is_timeout(&e) => {
                if Instant::now() > deadline {
                    return Err(Fail::Io("timed out".into()));
                }
            }
            Err(e) => return Err(Fail::Io(e.to_string())),
        }
    }
}

/// The main's next handshake frame: its message, or its refusal as `Refused`.
fn from_main<S: Read + Write>(ws: &mut WebSocket<S>, deadline: Instant, stopped: &dyn Fn() -> bool) -> Result<Vec<u8>, Fail> {
    let frame = read_frame(ws, deadline, stopped)?;
    match frame.split_first() {
        Some((&KIND_MESSAGE, body)) => Ok(body.to_vec()),
        Some((&KIND_REFUSAL, reason)) => Err(Fail::Refused(String::from_utf8_lossy(reason).chars().filter(|c| !c.is_control()).take(80).collect())),
        _ => Err(Fail::Malformed),
    }
}

/// `hs`'s next message, empty payload.
fn message(hs: &mut HandshakeState) -> Result<Vec<u8>, Fail> {
    let mut buf = vec![0u8; NOISE_MAX];
    let n = hs.write_message(&[], &mut buf).map_err(|_| Fail::Crypto)?;
    buf.truncate(n);
    Ok(buf)
}

fn absorb(hs: &mut HandshakeState, msg: &[u8]) -> Result<(), Fail> {
    let mut buf = vec![0u8; NOISE_MAX];
    hs.read_message(msg, &mut buf).map(|_| ()).map_err(|_| Fail::Crypto)
}

/// The transport and the other side's static key, once the handshake is done.
fn finish(hs: HandshakeState) -> Result<(TransportState, [u8; KEY_LEN]), Fail> {
    let key: [u8; KEY_LEN] = hs.get_remote_static().and_then(|k| k.try_into().ok()).ok_or(Fail::Malformed)?;
    let t = hs.into_transport_mode().map_err(|_| Fail::Crypto)?;
    Ok((t, key))
}

fn spake_key(spake: Spake2<Ed25519Group>, theirs: &[u8]) -> Result<[u8; 32], Fail> {
    let key = spake.finish(theirs).map_err(|_| Fail::Crypto)?;
    key.try_into().map_err(|_| Fail::Crypto)
}

fn password(code: &str) -> Password {
    Password::new(code.trim().as_bytes())
}

/// The joining Maya's pairing: SPAKE2 over `code`, then `XXpsk3` keyed by
/// it, then the main's word that the code matched. Returns the transport
/// and the main's public key.
pub fn pair<S: Read + Write>(ws: &mut WebSocket<S>, me: &Keypair, code: &str, deadline: Instant, stopped: &dyn Fn() -> bool) -> Result<(TransportState, [u8; KEY_LEN]), Fail> {
    let (spake, ours) = Spake2::<Ed25519Group>::start_a(&password(code), &Identity::new(JOINER), &Identity::new(MAIN));
    let mut first = vec![MODE_PAIR];
    first.extend_from_slice(&ours);
    send_frame(ws, first)?;
    let psk = spake_key(spake, &from_main(ws, deadline, stopped)?)?;
    let mut hs = builder(PAIR_PATTERN, me)?.psk(3, &psk).and_then(|b| b.build_initiator()).map_err(|_| Fail::Malformed)?;
    send_frame(ws, message(&mut hs)?)?;
    absorb(&mut hs, &from_main(ws, deadline, stopped)?)?;
    send_frame(ws, message(&mut hs)?)?;
    // The main checks the code with that last message and says so.
    from_main(ws, deadline, stopped)?;
    finish(hs)
}

/// An assistant's connection: `IK` against the main's pinned key.
pub fn connect<S: Read + Write>(ws: &mut WebSocket<S>, me: &Keypair, main: &[u8; KEY_LEN], deadline: Instant, stopped: &dyn Fn() -> bool) -> Result<TransportState, Fail> {
    let mut hs = builder(CONNECT_PATTERN, me)?.remote_public_key(main).and_then(|b| b.build_initiator()).map_err(|_| Fail::Malformed)?;
    let mut first = vec![MODE_CONNECT];
    first.extend_from_slice(&message(&mut hs)?);
    send_frame(ws, first)?;
    absorb(&mut hs, &from_main(ws, deadline, stopped)?)?;
    finish(hs).map(|(t, _)| t)
}

/// The main's side of either handshake. `code` gives the open pairing code
/// (or why there is none) and is called only for a pairing, before SPAKE2.
pub fn accept<S: Read + Write>(ws: &mut WebSocket<S>, me: &Keypair, deadline: Instant, stopped: &dyn Fn() -> bool, code: &mut dyn FnMut() -> Result<String, String>) -> Result<(TransportState, Accepted), Fail> {
    let first = read_frame(ws, deadline, stopped)?;
    match first.split_first() {
        Some((&MODE_PAIR, theirs)) if theirs.len() == SPAKE_LEN => {
            let code = match code() {
                Ok(c) => c,
                Err(reason) => {
                    refuse(ws, &reason);
                    return Err(Fail::Refused(reason));
                }
            };
            let (spake, ours) = Spake2::<Ed25519Group>::start_b(&password(&code), &Identity::new(JOINER), &Identity::new(MAIN));
            send_message(ws, &ours)?;
            let psk = spake_key(spake, theirs)?;
            let mut hs = builder(PAIR_PATTERN, me)?.psk(3, &psk).and_then(|b| b.build_responder()).map_err(|_| Fail::Malformed)?;
            absorb(&mut hs, &read_frame(ws, deadline, stopped)?)?;
            send_message(ws, &message(&mut hs)?)?;
            // Only here does the joiner prove it knows the code (`psk3`).
            if let Err(f) = read_frame(ws, deadline, stopped).and_then(|m| absorb(&mut hs, &m)) {
                if f == Fail::Crypto {
                    refuse(ws, WRONG_CODE_REASON);
                }
                return Err(f);
            }
            send_message(ws, &[])?;
            let (t, key) = finish(hs)?;
            Ok((t, Accepted { key, paired: true }))
        }
        Some((&MODE_CONNECT, msg)) => {
            let mut hs = builder(CONNECT_PATTERN, me)?.build_responder().map_err(|_| Fail::Malformed)?;
            if absorb(&mut hs, msg).is_err() {
                // Sealed to another key: the assistant pinned someone else.
                refuse(ws, UNKNOWN_KEY_REASON);
                return Err(Fail::Crypto);
            }
            send_message(ws, &message(&mut hs)?)?;
            let (t, key) = finish(hs)?;
            Ok((t, Accepted { key, paired: false }))
        }
        _ => Err(Fail::Malformed),
    }
}
```

What each handshake sends, for a reviewer checking it against the Global
Constraints:

| Pairing | Joiner → main | Main → joiner |
|---|---|---|
| 1 | `[1]` + SPAKE2 A (33 bytes) | `[0x00]` + SPAKE2 B, or `[0xFF]` + reason (no code open, locked out) |
| 2 | XX `-> e` | `[0x00]` + XX `<- e, ee, s, es` |
| 3 | XX `-> s, se, psk` | `[0x00]` (the code matched) or `[0xFF]` + `wrong or expired pairing code` |

| Connection | Assistant → main | Main → assistant |
|---|---|---|
| 1 | `[2]` + IK `-> e, es, s, ss` | `[0x00]` + IK `<- e, ee, se`, or `[0xFF]` + `unknown key` |

After either, both sides are in transport mode and every frame is sealed.
The main recognises the assistant by `Accepted.key` and checks it against
its paired list only once `hello` arrives (Task 3), so an unknown key gets
an encrypted `bye "removed"` that only its holder can read.

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p maya-core net::secure`
Expected: 12 passed (4 frame tests, 8 handshake tests).

- [ ] **Step 5: Commit**

```bash
git add core/src/net/secure.rs core/src/net/mod.rs
git commit -m "feat(net): SPAKE2 and Noise handshakes, and frames sealed in a Channel"
```

---

### Task 3: The core speaks protocol 2

This task changes the message types, the config and the server and client
together: none of them compiles against the others' old shape. Work through
the steps in order; `cargo test -p maya-core` is the check at the end.

**Files:**
- Modify: `core/src/net/protocol.rs`, `core/src/config.rs`, `core/src/net/promote.rs`, `core/src/net/mod.rs`, `core/src/net/server.rs`, `core/src/net/client.rs`, `core/Cargo.toml`

**Interfaces:**
- Consumes: `identity::{Keypair, encode_key, decode_key}`, `secure::{Channel, Got, ChannelError, Fail, Accepted, accept, pair, connect, refuse, WRONG_CODE_REASON, UNKNOWN_KEY_REASON, FORGED}`
- Produces:
  - `config::PairedAssistant { id, name, hostname, platform, key: String, address, last_seen, mirror }` (no `token`)
  - `config::NetworkConfig { role, port, main_host, main_port, name, main_key: String, pair_again: bool, assistants }` (no `assistant_id`, no `token`)
  - `NetworkConfig::drop_insecure_pairings(&mut self)`
  - `protocol::PROTOCOL == 2`; `Up::Hello { protocol, app, name, hostname, platform, caps }`; `Down::Welcome { name, mirror }`; `Down::Move { host, port, key }`; `CommandKind::Promote { list: Vec<PairedAssistant> }`; `PairingWindow { code, expires_at_ms, tries_left }` with `take_try(&mut self, now_ms) -> bool`; `PAIR_TRIES: u32 = 3`
  - `server::start_with(notify, config_view, port, identity: Arc<Keypair>) -> Result<ServerHandle, String>`
  - `ServerHandle::public_key(&self) -> [u8; 32]`
  - `server::Promotion { host, port, main_key: String, moved, left_behind }`
  - `NetworkStatus.code_used_up: bool`, `NetworkStatus.pair_again: bool`
  - `client::ClientNotify::paired(&self, main_key: &str)`; `ClientNotify::moved(&self, host: &str, port: u16, key: &str) -> Result<(), String>`
  - `client::pair_with(exec, identity: Arc<Keypair>, host, port, name, code, caps) -> Result<(String /*main name*/, String /*main key*/), String>`
  - `client::reconnect(config_of, identity: Arc<Keypair>, exec, notify, stop)`
  - `client::run_once(config, identity: &Keypair, exec, notify, stop, code)`
  - `client::{KEY_CHANGED, OLD_MAIN, NO_ANSWER, PAIRING_FAILED}` consts; `net::CODE_USED_UP`
  - `promote::own_entry(main_name, mirror, key: &str)`, `promote::as_assistant_of(c, host, port, main_key: &str)`

- [ ] **Step 1: Protocol types**

In `core/src/net/protocol.rs`:

1. `pub const PROTOCOL: u32 = 2;` and add
   `pub const PAIR_TRIES: u32 = 3;` beside `LOCKOUT_AFTER`.
2. `Up::Hello`: remove the `id` field. Remove `Up::Pair` and `Up::Auth`.
3. `Down`: remove `Challenge` and `Paired`. `Welcome` becomes
   `Welcome { name: String, #[serde(default, skip_serializing_if = "Option::is_none")] mirror: Option<bool> }`
   with the doc: "`mirror` answers a Maya that asked with `CAP_MIRROR`: whether
   the main will mirror to it. Absent for anyone else." `Move` becomes
   `Move { host: String, port: u16, key: String }` with the doc "The main is
   now at `host:port` with the public key `key`: pin it and reconnect there.
   The main closes the connection after it."
4. `CommandKind::Promote` becomes
   `Promote { list: Vec<crate::config::PairedAssistant> }` with the doc
   "Become the main with this paired list (public keys only)." Its
   `PartialEq` derive still works: `PairedAssistant` derives `PartialEq, Eq`.
5. Delete `new_token`, `new_nonce`, `mac`, `mac_matches`, `promote_key`,
   `seal`, `open`, `NonceLog`, `UNSEALABLE` and the `use` lines they alone
   needed (`hmac`, `sha2`). Keep `new_id`, `random`, `Attempts`.
6. `PairingWindow`:

```rust
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
```

7. Tests in `protocol.rs`: delete `mac_rejects_wrong_token_and_reused_nonce`,
   `a_seal_opens_only_with_its_token_and_unchanged`,
   `tokens_and_nonces_are_random_and_long_enough`; in
   `messages_round_trip_with_a_type_tag` delete the `auth`, `pair` and
   proof-less-welcome assertions and remove `id: None` from the `Hello`;
   remove `mac: "cd".into()` from every `Welcome` and fix the
   `r#"{"type":"welcome","name":"desk","mac":"cd"}"#` decode to
   `r#"{"type":"welcome","name":"desk"}"#`; in
   `a_hello_without_caps_has_none_and_caps_round_trip` change the old frame's
   `"protocol":1` to `"protocol":2` and remove `id: None`. Replace
   `pairing_codes_are_six_digits_and_expire` and
   `move_and_promote_encode_and_decode` with:

```rust
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
```

- [ ] **Step 2: Config**

In `core/src/config.rs`:

```rust
/// A Maya the main has paired with; `key` is who it is.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PairedAssistant {
    pub id: String,
    pub name: String,
    pub hostname: String,
    pub platform: String,
    /// Its public key (base64), pinned at pairing; empty for a protocol 1
    /// entry, which `drop_insecure_pairings` removes.
    #[serde(default)]
    pub key: String,
    /// The peer's IP address as the main last saw it; shown, not trusted.
    #[serde(default)]
    pub address: String,
    // last_seen and mirror unchanged
}
```

`NetworkConfig`: delete `assistant_id` and `token`; add after `name`:

```rust
    /// The main's public key (base64), pinned at pairing; empty until then.
    pub main_key: String,
    /// Protocol 1 pairings were dropped on update: Settings says to pair again.
    pub pair_again: bool,
```

Add to `impl NetworkConfig` (create the `impl` block after the struct if
there is none):

```rust
impl NetworkConfig {
    /// Protocol 1 pairings (tokens, no keys) cannot connect any more: they
    /// go, and Settings tells the user to pair again.
    pub fn drop_insecure_pairings(&mut self) {
        let before = self.assistants.len();
        self.assistants.retain(|a| !a.key.is_empty());
        let stale_link = self.role == NetworkRole::Assistant && self.main_key.is_empty() && !self.main_host.trim().is_empty();
        if self.assistants.len() != before || stale_link {
            self.pair_again = true;
        }
    }
}
```

In `Config::migrate`, before `self`, add `self.network.drop_insecure_pairings();`.
Change the `save` doc comment to "Writes the config readable by its owner only."

Update every test in `config.rs` that builds a `PairedAssistant` with
`token: "t".into()` to `key: "k".into()`, and the one that sets
`c.network.token = "abc".into()` to `c.network.main_key = "abc".into()`.
Add:

```rust
    #[test]
    fn protocol_1_pairings_are_dropped_and_the_user_told_to_pair_again() {
        let old = r#"{"completedTimeoutMinutes":5,"network":{"role":"main","assistants":[{"id":"x","name":"laptop","hostname":"h","platform":"macos","token":"t"},{"id":"y","name":"mini","hostname":"h","platform":"macos","key":"a2V5"}]}}"#;
        let c = serde_json::from_str::<Config>(old).unwrap().migrate();
        assert_eq!(c.network.assistants.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["y"]);
        assert!(c.network.pair_again);
        let old_link = r#"{"completedTimeoutMinutes":5,"network":{"role":"assistant","mainHost":"desk.local","assistantId":"a","token":"t"}}"#;
        assert!(serde_json::from_str::<Config>(old_link).unwrap().migrate().network.pair_again);
        let fresh = Config::default().migrate();
        assert!(!fresh.network.pair_again, "nothing was paired, nothing to say");
    }
```

- [ ] **Step 3: Network status and change detection**

In `core/src/net/mod.rs`, add to `NetworkStatus` after `addresses`:

```rust
    /// The last code took three wrong tries; the main must regenerate one.
    pub code_used_up: bool,
    /// Protocol 1 pairings were dropped on update (`NetworkConfig::pair_again`).
    pub pair_again: bool,
```

In `network_change`, the `link` closure becomes
`(c.network.main_host.clone(), c.main_port(), c.network.name.clone(), c.network.main_key.clone())`.
In the test at the bottom that builds `PairedAssistant { … token: "t".into() … }`
use `key: "k".into()`.

Add `pub const CODE_USED_UP: &str = "Code used up after 3 wrong tries — Regenerate.";`
to `core/src/net/mod.rs` (the pages show it; it lives next to the status).

- [ ] **Step 4: Promote**

Replace `core/src/net/promote.rs`'s code above its tests with:

```rust
//! Handing the main role to an assistant: the list the new main gets and
//! how each side's config changes. The list carries public keys only, and
//! travels over the encrypted channel, so nothing secret moves.

use super::protocol::new_id;
use crate::config::{Config, NetworkRole, PairedAssistant};

/// The entry the old main makes for itself in the list it hands over: it
/// connects to the new main with its own key. Its address is filled in by
/// the new main at its first connection. `mirror` is true for a phone: the
/// user pressed Make main on it.
pub fn own_entry(main_name: &str, mirror: bool, key: &str) -> PairedAssistant {
    PairedAssistant { id: new_id(), name: main_name.into(), hostname: super::local_hostname(), platform: std::env::consts::OS.into(), key: key.into(), address: String::new(), last_seen: None, mirror }
}

/// Every paired assistant but the one promoted, then the old main's own entry.
pub fn handover_list(paired: &[PairedAssistant], promoted: &str, me: &PairedAssistant) -> Vec<PairedAssistant> {
    paired.iter().filter(|p| p.id != promoted).cloned().chain(std::iter::once(me.clone())).collect()
}

/// The promoted assistant's config: a main with the list, on its own port
/// and name, with its link to the old main gone.
pub fn as_main(c: &mut Config, list: Vec<PairedAssistant>) {
    let n = &mut c.network;
    n.role = NetworkRole::Main;
    n.assistants = list;
    n.main_host.clear();
    n.main_port = 0;
    n.main_key.clear();
}

/// The old main's config: an assistant of `host:port`, pinning the new
/// main's key. Listening for "Maya" goes off, as pairing does.
pub fn as_assistant_of(c: &mut Config, host: &str, port: u16, main_key: &str) {
    let n = &mut c.network;
    n.role = NetworkRole::Assistant;
    n.main_host = host.into();
    n.main_port = port;
    n.main_key = main_key.into();
    n.assistants.clear();
    c.listen = false;
}
```

Update `promote.rs`'s tests: drop every seal/open test; where a test calls
`own_entry(name, mirror)` add a third argument `"a2V5"`; where it calls
`as_assistant_of(c, host, port, &me)` pass `"a2V5"` and assert
`c.network.main_key == "a2V5"`; replace `token` with `key` in literals.

- [ ] **Step 5: Server — identity, keys and tries**

In `core/src/net/server.rs`:

1. Imports: replace the `protocol` import with
   `use super::protocol::{decode_up, encode, new_id, Attempts, CommandKind, Down, PairingWindow, Request, Up, CAP_FOLLOW, CAP_LEAD, CAP_MIRROR, MAX_FRAME, PROTOCOL};`
   and add
   `use super::identity::{encode_key, Keypair};` and
   `use super::secure::{self, Channel, ChannelError, Fail, Got};`.
2. `Server`: remove `nonces: NonceLog`; add `code_used_up: bool` with doc
   "The last code took its three tries without a pairing." Update
   `Server::new` accordingly. In `status`, add `code_used_up: self.code_used_up`
   to the `NetworkStatus { … }`.
3. `record_pairing` takes the key first and finds the machine by key:

```rust
    /// Records a pairing by the Maya with `key`. An entry with that key is
    /// the same machine pairing again: it keeps its id, takes the names
    /// just given, and its old board goes. Otherwise it is a new entry.
    /// Returns the entry, and what it was before (for `undo_pairing`).
    fn record_pairing(&mut self, key: &str, address: &str, name: &str, hostname: &str, platform: &str, mirror: bool) -> (PairedAssistant, Option<PairedAssistant>) {
        let again = self.paired.iter().position(|p| p.key == key);
        let Some(i) = again else {
            let a = PairedAssistant { id: new_id(), name: name.into(), hostname: hostname.into(), platform: platform.into(), key: key.into(), address: address.into(), last_seen: None, mirror };
            self.paired.push(a.clone());
            self.pairings_in_flight.insert(a.id.clone());
            return (a, None);
        };
        let previous = self.paired[i].clone();
        let p = &mut self.paired[i];
        p.name = name.into();
        p.hostname = hostname.into();
        p.platform = platform.into();
        p.address = address.into();
        p.mirror = mirror;
        let a = p.clone();
        self.pairings_in_flight.insert(a.id.clone());
        self.by_id.remove(&a.id);
        self.peers.remove(&a.id);
        self.sync_boards();
        (a, Some(previous))
    }
```

   Update `undo_pairing`'s doc: "an existing one gets its old names back".
4. `Ctx` gains `identity: Arc<Keypair>`. `start_with` gains a fourth
   parameter `identity: Arc<Keypair>` and stores it. `ServerHandle` gains:

```rust
    /// This main's public key, as its assistants pin it.
    pub fn public_key(&self) -> [u8; 32] {
        self.ctx.identity.public
    }
```

   `open_pairing` sets `s.code_used_up = false;` before storing the window.
5. Two new functions above `serve`:

```rust
/// One try at the open code for a pairing that is starting from `ip`, or
/// the reason to refuse it.
fn take_try(ctx: &Ctx, ip: &str) -> Result<String, String> {
    let mut s = lock(&ctx.shared);
    let now = now_ms();
    if s.attempts.locked(ip, now) {
        return Err("too many attempts".into());
    }
    match s.pairing.as_mut() {
        Some(w) if w.take_try(now) => Ok(w.code.clone()),
        _ => Err(secure::WRONG_CODE_REASON.into()),
    }
}

/// A pairing that took a try did not reach `welcome`: the address counts a
/// failure, and the code goes once its tries are gone. A code another
/// pairing already used is left alone.
fn pairing_failed(ctx: &Ctx, ip: &str) {
    {
        let mut s = lock(&ctx.shared);
        let Some(tries_left) = s.pairing.as_ref().map(|w| w.tries_left) else { return };
        s.attempts.failed(ip, now_ms());
        if tries_left == 0 {
            s.pairing = None;
            s.code_used_up = true;
            log::line("network", "pairing code used up after 3 wrong tries");
        }
    }
    ctx.announce();
}
```

- [ ] **Step 6: Server — the handshake in `serve` and `admit`**

1. Below `type Ws = WebSocket<TcpStream>;` add `type Chan = Channel<TcpStream>;`.
2. Replace `serve` with:

```rust
fn serve(ctx: &Ctx, stream: TcpStream, addr: SocketAddr) {
    let pre_auth = PreAuth(ctx, addr.ip());
    let deadline = Instant::now() + PRE_AUTH_DEADLINE;
    // Accepted sockets inherit the listener's non-blocking flag on macOS.
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(TICK));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_nodelay(true);
    let mut ws = match upgrade(ctx, stream, deadline) {
        Ok(ws) => ws,
        Err(e) => {
            log::line("network", format!("{addr}: no WebSocket handshake: {e}"));
            return;
        }
    };
    let ip = addr.ip().to_string();
    let mut took_try = false;
    let shook = secure::accept(&mut ws, &ctx.identity, deadline, &|| ctx.stopped(), &mut || {
        let code = take_try(ctx, &ip);
        took_try = code.is_ok();
        code
    });
    let (noise, accepted) = match shook {
        Ok(done) => done,
        Err(f) => {
            if f == Fail::OldPeer {
                // A protocol 1 Maya reads this as "runs a different version".
                let _ = ws.send(Message::text(encode(&Down::Bye { reason: "protocol".into() })));
            }
            if took_try {
                pairing_failed(ctx, &ip);
            }
            log::line("network", format!("{ip}: handshake failed: {f:?}"));
            finish_ws(&mut ws);
            return;
        }
    };
    let mut ch = Channel::new(ws, noise);
    let admitted = admit(ctx, &mut ch, addr, &accepted, deadline);
    if admitted.is_none() && took_try {
        pairing_failed(ctx, &ip);
    }
    drop(pre_auth);
    let Some(link) = admitted else {
        finish_ws(&mut ch.ws);
        return;
    };
    let reason = pump(ctx, &mut ch, &link);
    // Forgotten before the close completes: an assistant that waits for its
    // close to be answered (a client stopped for "Pair again") finds the
    // main already counting it as gone.
    leave(ctx, &link, &reason);
    finish_ws(&mut ch.ws);
}
```

3. Rename `finish(ws: &mut Ws)` to `finish_ws(ws: &mut Ws)` (body unchanged).
4. `send_down`, `bye`, `read_up`, `send_queued`, `pump`, `handle` take
   `ch: &mut Chan` instead of `ws: &mut Ws`:

```rust
fn send_down(ch: &mut Chan, down: &Down) -> Result<(), String> {
    ch.send(&encode(down)).map_err(|e| format!("write failed: {e}"))
}

fn bye(ch: &mut Chan, reason: &str) {
    let _ = send_down(ch, &Down::Bye { reason: reason.into() });
}

/// The next message that parses, by `deadline`; garbage is logged and skipped.
fn read_up(ctx: &Ctx, ch: &mut Chan, who: &str, deadline: Instant) -> Result<Up, String> {
    loop {
        if ctx.stopped() {
            return Err("the main stopped".into());
        }
        match ch.read() {
            Ok(Got::Text(t)) => match decode_up(&t) {
                Ok(up) => return Ok(up),
                Err(e) => log::line("network", format!("{who}: ignored a message: {e}")),
            },
            Ok(Got::Closed) => return Err("closed".into()),
            Ok(Got::Control) => {}
            Ok(Got::Idle) => {
                if Instant::now() > deadline {
                    return Err("no answer within 10 s of connecting".into());
                }
            }
            Err(ChannelError::Forged) => {
                log::line("network", format!("{who}: {}", secure::FORGED));
                return Err(secure::FORGED.into());
            }
            Err(ChannelError::Io(e)) => return Err(e),
        }
    }
}
```

   In `pump`, replace the `match ws.read() { … }` block with:

```rust
        match ch.read() {
            Ok(Got::Text(t)) => {
                deadline = Instant::now() + SILENCE;
                match decode_up(&t) {
                    Ok(up) => match handle(ctx, ch, link, up) {
                        Ok(()) => {}
                        // Removed while this frame was in flight: the `bye`
                        // `remove_assistant` queued (under the same lock
                        // `handle` checked) goes out before the close.
                        Err(e) if e == NOT_CURRENT => return send_queued(ch, link).unwrap_or(e),
                        Err(e) => return e,
                    },
                    Err(e) if unreadable_board(&t) => {
                        log::line("network", format!("{}: could not read its board: {e}", link.label));
                        keep_board_fresh(ctx, link);
                        mark_board(ctx, link, true);
                    }
                    Err(e) => log::line("network", format!("{}: ignored a message: {e}", link.label)),
                }
            }
            Ok(Got::Closed) => return "closed by the assistant".into(),
            Ok(Got::Control) => deadline = Instant::now() + SILENCE,
            Ok(Got::Idle) => {
                if Instant::now() > deadline {
                    return "silent for 15 s".into();
                }
            }
            Err(ChannelError::Forged) => {
                log::line("network", format!("{}: {}", link.label, secure::FORGED));
                return secure::FORGED.into();
            }
            Err(ChannelError::Io(e)) => return e,
        }
```

   Delete `skip_frame` (binary frames are now the protocol) and its uses.
   In `handle`, the last arm becomes
   `Up::Hello { .. } => log::line("network", format!("{}: ignored a hello after welcome", link.label)),`.
   Any `ws.set_config(...)` call becomes `ch.ws.set_config(...)`.
5. Replace `admit` with this version (registration block unchanged from
   "let (tx, rx) = channel();" down to the `match registered` —
   copy it across as is):

```rust
fn admit(ctx: &Ctx, ch: &mut Chan, addr: SocketAddr, accepted: &secure::Accepted, deadline: Instant) -> Option<Link> {
    let ip = addr.ip().to_string();
    let hello = match read_up(ctx, ch, &ip, deadline) {
        Ok(up) => up,
        Err(e) => {
            log::line("network", format!("{ip}: no hello: {e}"));
            return None;
        }
    };
    let Up::Hello { protocol, app, name, hostname, platform, caps } = hello else {
        log::line("network", format!("{ip}: first message was not a hello"));
        bye(ch, "protocol");
        return None;
    };
    if protocol != PROTOCOL {
        log::line("network", format!("{ip}: protocol {protocol}, expected {PROTOCOL}"));
        bye(ch, "protocol");
        return None;
    }
    let (hostname, platform, app) = (clean(&hostname), clean(&platform), clean(&app));
    if app != env!("CARGO_PKG_VERSION") {
        log::line("network", format!("{ip}: runs Maya {app}; this Mac runs {}", env!("CARGO_PKG_VERSION")));
    }
    let name = Some(clean(&name)).filter(|n| !n.is_empty()).or_else(|| Some(hostname.clone()).filter(|h| !h.is_empty())).unwrap_or_else(|| "assistant".into());
    let wants_mirror = caps.iter().any(|c| c == CAP_MIRROR);
    let key = encode_key(&accepted.key);
    let (id, pairing) = if accepted.paired {
        let p = pair(ctx, ch, &key, &ip, &name, &hostname, &platform, wants_mirror)?;
        (p.entry.id.clone(), Some(p))
    } else {
        let known = lock(&ctx.shared).paired.iter().find(|p| p.key == key).map(|p| p.id.clone());
        let Some(id) = known else {
            // A key this main does not know: removed, or paired with another main.
            log::line("network", format!("{name}: unknown key; told it was removed"));
            bye(ch, "removed");
            return None;
        };
        (id, None)
    };
    // … the registration block, unchanged …
```

   After the registration block, the `ws.set_config` becomes
   `ch.ws.set_config(…)` and the welcome is
   `send_down(ch, &Down::Welcome { name: ctx.main_name.clone(), mirror: offer })`.
   Every `bye(ws, …)` in the block becomes `bye(ch, …)`.
6. Delete `authenticate`. Replace `pair` with:

```rust
/// Records the pairing of a Maya that proved the code, unless the code
/// was used in the meantime (one code, one machine).
#[allow(clippy::too_many_arguments)]
fn pair(ctx: &Ctx, ch: &mut Chan, key: &str, ip: &str, name: &str, hostname: &str, platform: &str, mirror: bool) -> Option<NewPairing> {
    let outcome = {
        let mut s = lock(&ctx.shared);
        if s.pairing.is_none() {
            None
        } else {
            let pairing = s.record_pairing(key, ip, name, hostname, platform, mirror);
            s.pairing = None;
            Some(pairing)
        }
    };
    let Some((entry, previous)) = outcome else {
        log::line("network", format!("{ip}: pairing refused: the code was used meanwhile"));
        bye(ch, secure::WRONG_CODE_REASON);
        return None;
    };
    log::line("network", format!("{name}: {} from {ip}", if previous.is_some() { "paired again (same key)" } else { "paired" }));
    Some(NewPairing { entry, previous })
}
```

   Update `NewPairing`'s doc comment: "A pairing recorded but not yet saved:
   it is saved just before `welcome`, and undone if the save fails or
   `welcome` cannot be sent, so a Maya that was never welcomed leaves no
   entry behind."

- [ ] **Step 7: Server — promote with keys**

In `promote_with_timeout`:

```rust
        let (label, host, main_key, list) = {
            let mut s = lock(&self.shared);
            if s.handover {
                return Err("A handover is already running.".into());
            }
            let label = s.label(id);
            let entry = s.paired.iter().find(|p| p.id == id).cloned().ok_or_else(|| format!("{label} is no longer paired."))?;
            if crate::net::identity::decode_key(&entry.key).is_err() {
                return Err(format!("{label} has no key; pair it again."));
            }
            // … the connected, CAP_LEAD and in-flight checks, unchanged …
            s.handover = true;
            s.pairing = None;
            let me = super::promote::own_entry(&self.ctx.main_name, self.ctx.own_mirror.load(Ordering::SeqCst), &self.ctx.identity.public_b64());
            let list = super::promote::handover_list(&s.paired, id, &me);
            (label, entry.address, entry.key, list)
        };
```

The command becomes `CommandKind::Promote { list }` (no seal). The move is
`Down::Move { host: host.clone(), port, key: main_key.clone() }`. The
result is `Ok(Promotion { host, port, main_key, moved, left_behind })`, and
`Promotion`'s `me: PairedAssistant` field is replaced by
`pub main_key: String` with doc "The new main's public key, for the old
main to pin." Update the doc of `promote` ("sends it the paired list with
this main's own new entry").

- [ ] **Step 8: Client**

In `core/src/net/client.rs`:

1. Imports: replace the `protocol` import with
   `use super::protocol::{decode_down, encode, Attachment, CommandKind, Down, Request, Up, DEFAULT_PORT, MAX_FRAME, PROTOCOL};`
   and add `use super::identity::{decode_key, encode_key, Keypair};` and
   `use super::secure::{self, Channel, ChannelError, Fail, Got};`.
2. Messages: delete `MAIN_UNPROVEN` and `PAIRING_UNPROVEN`; add

```rust
pub const KEY_CHANGED: &str = "The main Maya's key has changed. If you reset it, pair again; otherwise someone may be impersonating it.";
pub const OLD_MAIN: &str = "The main runs an older Maya. Update it.";
pub const NO_ANSWER: &str = "The main Maya did not answer. If it runs an older Maya, update it.";
pub const PAIRING_FAILED: &str = "The pairing did not complete; try again with a new code.";
```

   and change `UNPROVEN_WAIT_MS`'s doc to "How long the client waits before
   trying again after `KEY_CHANGED`." `retry_wait_ms` matches `KEY_CHANGED`
   instead of `MAIN_UNPROVEN`.
3. `ClientNotify::paired` becomes `fn paired(&self, main_key: &str);` with
   doc "The main's public key, learned at pairing (pairing only)."
   `moved` becomes `fn moved(&self, _host: &str, _port: u16, _key: &str) -> Result<(), String>`
   with doc "The main is now at `host:port` with the public key `key`:
   save both. Runs on the connection thread." `promoted`'s doc drops
   "sealed".
4. `type Ws = WebSocket<TcpStream>;` stays; add `type Chan = Channel<TcpStream>;`.
   `send_up(ch: &mut Chan, up: &Up)` is `ch.send(&encode(up)).map_err(|e| format!("Lost the connection to the main Maya: {e}"))`.
   `read_down(ch: &mut Chan, stop, deadline)` is the server's `read_up`
   shape: `Got::Text` decodes (garbage logged and skipped), `Got::Closed`
   is `"The main Maya closed the connection."`, `Got::Idle` past the
   deadline is `"The main Maya did not answer in time."`, `Forged` logs
   `secure::FORGED` and returns it, `Io(e)` is
   `format!("Lost the connection to the main Maya: {e}")`.
   `finish(ws: &mut Ws)` keeps its signature; callers pass `&mut ch.ws`.
5. The handshake failure message:

```rust
/// What the user reads when the handshake did not complete.
fn fail_message(f: &Fail, pairing: bool) -> String {
    match f {
        Fail::OldPeer => OLD_MAIN.into(),
        Fail::Refused(reason) if reason == secure::UNKNOWN_KEY_REASON => KEY_CHANGED.into(),
        Fail::Refused(reason) => bye_message(reason),
        Fail::Crypto if pairing => PAIRING_FAILED.into(),
        Fail::Crypto => KEY_CHANGED.into(),
        Fail::Io(e) if e == "stopped" => "stopped".into(),
        Fail::Io(_) | Fail::Malformed => NO_ANSWER.into(),
    }
}
```

6. `run_once` and `connection` take `identity: &Keypair` after `config`:

```rust
pub fn run_once(config: &NetworkConfig, identity: &Keypair, exec: Arc<dyn Executor>, notify: Arc<dyn ClientNotify>, stop: &AtomicBool, code: Option<&str>) -> Result<(), String> {
    match connection(config, identity, &exec, &notify, stop, code) {
        // … unchanged …
    }
}

fn connection(config: &NetworkConfig, identity: &Keypair, exec: &Arc<dyn Executor>, notify: &Arc<dyn ClientNotify>, stop: &AtomicBool, code: Option<&str>) -> Result<(), String> {
    let main_key = match code {
        Some(_) => None,
        None => Some(decode_key(&config.main_key).map_err(|_| NOT_PAIRED.to_string())?),
    };
    let deadline = Instant::now() + SILENCE;
    let mut ws = connect(config, stop, deadline)?;
    let quit = || stopped(stop);
    let shook = match (code, main_key) {
        (Some(code), _) => secure::pair(&mut ws, identity, code, deadline, &quit).map(|(t, k)| (t, Some(k))),
        (None, Some(k)) => secure::connect(&mut ws, identity, &k, deadline, &quit).map(|t| (t, None)),
        (None, None) => unreachable!("a main key or a code"),
    };
    let (noise, learned) = match shook {
        Ok(done) => done,
        Err(f) => {
            finish(&mut ws);
            return Err(fail_message(&f, code.is_some()));
        }
    };
    let mut ch = Channel::new(ws, noise);
    let out = handshake(&mut ch, config, &**notify, stop, learned, deadline).and_then(|(main, mirror)| {
        log::line("network", format!("connected to {main}"));
        notify.connected(&main);
        notify.mirror_offer(mirror);
        pump(&mut ch, exec, notify, stop)
    });
    finish(&mut ch.ws);
    out
}
```

7. `handshake` becomes:

```rust
/// `hello`, then `welcome`; returns the main's name and its mirror offer.
/// `learned` is the main's key from a pairing, kept only once welcomed.
fn handshake(ch: &mut Chan, config: &NetworkConfig, notify: &dyn ClientNotify, stop: &AtomicBool, learned: Option<[u8; 32]>, deadline: Instant) -> Result<(String, Option<bool>), String> {
    let hostname = super::local_hostname();
    let hello = Up::Hello { protocol: PROTOCOL, app: env!("CARGO_PKG_VERSION").into(), name: display_name(&config.name, &hostname), hostname, platform: std::env::consts::OS.into(), caps: notify.caps() };
    send_up(ch, &hello)?;
    loop {
        match read_down(ch, stop, deadline)? {
            Down::Welcome { name, mirror } => {
                if let Some(key) = learned {
                    notify.paired(&encode_key(&key));
                }
                let name = clean(&name);
                return Ok((if name.is_empty() { "the main Maya".into() } else { name }, mirror));
            }
            Down::Bye { reason } => return Err(bye_message(&reason)),
            other => log::line("network", format!("main: ignored {} before welcome", encode(&other).chars().take(40).collect::<String>())),
        }
    }
}
```

   A wrong code never reaches `handshake`: the main says so before the
   handshake ends, and `secure::pair` returns
   `Fail::Refused("wrong or expired pairing code")`, which `fail_message`
   turns into `WRONG_CODE`. After the handshake there are no refusals.
   Add a `bye_message` arm: `"unknown key" => KEY_CHANGED.into(),`.
8. `pump(ch: &mut Chan, exec, notify, stop)` (no `token`): replace its
   `ws.send(Message::text(frame.clone()))` with `ch.send(&frame)` and its
   read `match` with the `Got` arms (Text → decode and `handle`;
   `Closed` → the two existing close arms; `Control` → `last_read = Instant::now()`;
   `Idle` → the silence check;
   `Forged` → log and `Err(secure::FORGED.into())`; `Io(_)` if leaving →
   `Ok(())`; `Io(e)` → the existing lost-connection error).
9. `handle(down, exec, notify, lane, tx, leaving)` (no `token`):
   - `Down::Command { id, kind: CommandKind::Promote { list } }`: spawn as
     today, calling `notify.promoted(list)` directly (no `open_list`).
   - `Down::Move { host, port, key }`:

```rust
        Down::Move { host, port, key } => {
            let host = clean(&host);
            if decode_key(&key).is_err() {
                log::line("network", format!("the main moved to {host}:{port} with no usable key; not following"));
                return Err(format!("Could not follow the main Maya to {host}: {}", super::identity::BAD_KEY));
            }
            log::line("network", format!("the main moved to {host}:{port}"));
            notify.moved(&host, port, &key).map_err(|e| format!("Could not follow the main Maya to {host}: {e}"))?;
            return Err(MOVED.into());
        }
```

   - Delete the `Down::Challenge | Down::Welcome | Down::Paired` arm; add
     `Down::Welcome { .. } => log::line("network", "main: ignored a welcome after welcome"),`.
10. `reconnect(config_of, identity: Arc<Keypair>, exec, notify, stop)`
    passes `&identity` to `run_once`. Its comment about "could not prove it
    holds the token" becomes "could not prove it holds its key".
11. `pair_with`:

```rust
/// The pairing handshake once. Ok((main_name, main_key)). One blocking
/// connection, at most `SILENCE` long once connected; it ends at
/// `welcome`, so no command reaches `exec`.
pub fn pair_with(exec: Arc<dyn Executor>, identity: Arc<Keypair>, host: &str, port: u16, name: &str, code: &str, caps: Vec<String>) -> Result<(String, String), String> {
    let config = NetworkConfig { role: NetworkRole::Assistant, main_host: host.trim().into(), main_port: port, name: name.trim().into(), ..Default::default() };
    let stop = Arc::new(AtomicBool::new(false));
    let notify = Arc::new(PairNotify { stop: stop.clone(), key: Mutex::new(None), main: Mutex::new(None), caps });
    log::line("network", format!("pairing with {}:{}", config.main_host, port_of(&config)));
    run_once(&config, &identity, exec, notify.clone(), &stop, Some(code)).map_err(|e| {
        log::line("network", format!("pairing failed: {e}"));
        e
    })?;
    let key = notify.key.lock().unwrap().take().ok_or("The main Maya did not pair this assistant.")?;
    let main = notify.main.lock().unwrap().take().unwrap_or_default();
    Ok((main, key))
}
```

    `PairNotify` keeps `key: Mutex<Option<String>>` instead of `creds`;
    its `paired(&self, main_key)` stores it.
12. `pair_after_stopping`'s doc: replace "take the new pairing for a second
    machine at the same address" with "find the pairing's key on a
    connected entry".
13. Client tests: `a_move_retries_at_once` uses `KEY_CHANGED` in place of
    `MAIN_UNPROVEN`. Others are unchanged.

- [ ] **Step 9: Drop the crates the core no longer uses**

Run: `grep -rn 'hmac\|sha2::\|chacha20poly1305' core/src`
Expected: no output. Then remove `hmac.workspace = true`,
`sha2.workspace = true` and `chacha20poly1305.workspace = true` from
`core/Cargo.toml`. If the grep finds a use, leave that crate in.

- [ ] **Step 10: Server tests — new helpers**

In `server.rs`'s `mod tests`, replace the `Client` alias and the helpers
`hello`, `hello_caps`, `pair_with_caps`, `pair_client_caps`, `pair_client`,
`answer`, `connect`, `send`, `recv` with:

```rust
    use crate::net::identity::Keypair;
    use crate::net::secure::{self, Channel, Got};

    pub(crate) type Client = Channel<MaybeTlsStream<TcpStream>>;
    pub(crate) type RawWs = WebSocket<MaybeTlsStream<TcpStream>>;

    fn soon() -> Instant {
        Instant::now() + Duration::from_secs(5)
    }

    /// A WebSocket to the test main, with a 5 s read timeout.
    pub(crate) fn connect(port: u16) -> RawWs {
        let (ws, _) = tungstenite::connect(format!("ws://127.0.0.1:{port}/")).unwrap();
        if let MaybeTlsStream::Plain(s) = ws.get_ref() {
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        }
        ws
    }

    pub(crate) fn hello_up(name: &str, caps: &[&str]) -> Up {
        Up::Hello { protocol: PROTOCOL, app: "t".into(), name: name.into(), hostname: "h".into(), platform: "macos".into(), caps: caps.iter().map(|c| c.to_string()).collect() }
    }

    /// Pairs a new Maya (key `me`) whose `hello` carries `caps`; returns its
    /// open channel, its entry id and the welcome's mirror offer.
    pub(crate) fn pair_as(handle: &ServerHandle, port: u16, me: &Keypair, name: &str, caps: &[&str]) -> (Client, String, Option<bool>) {
        let (code, _) = handle.open_pairing(crate::store::now_ms()).unwrap();
        let mut ws = connect(port);
        let (noise, main_key) = secure::pair(&mut ws, me, &code, soon(), &|| false).unwrap();
        assert_eq!(main_key, handle.public_key(), "the joiner learns the main's key");
        let mut ch = Channel::new(ws, noise);
        send(&mut ch, &hello_up(name, caps));
        let Down::Welcome { mirror, .. } = recv(&mut ch) else { panic!("expected welcome") };
        let key = crate::net::identity::encode_key(&me.public);
        let id = lock(&handle.shared).paired.iter().find(|p| p.key == key).map(|p| p.id.clone()).expect("an entry with this key");
        (ch, id, mirror)
    }

    /// Pairs a new Maya with a fresh key; returns its channel, id, key and offer.
    pub(crate) fn pair_client_caps(handle: &ServerHandle, port: u16, name: &str, caps: &[&str]) -> (Client, String, Keypair, Option<bool>) {
        let me = Keypair::generate();
        let (ch, id, offer) = pair_as(handle, port, &me, name, caps);
        (ch, id, me, offer)
    }

    pub(crate) fn pair_client(handle: &ServerHandle, port: u16, name: &str) -> (Client, String, Keypair) {
        let (ch, id, me, _) = pair_client_caps(handle, port, name, &[]);
        (ch, id, me)
    }

    /// Connects a paired Maya with its key and says `hello`; returns the
    /// channel before `welcome` is read.
    pub(crate) fn connect_as(handle: &ServerHandle, port: u16, me: &Keypair, name: &str, caps: &[&str]) -> Client {
        let mut ws = connect(port);
        let noise = secure::connect(&mut ws, me, &handle.public_key(), soon(), &|| false).unwrap();
        let mut ch = Channel::new(ws, noise);
        send(&mut ch, &hello_up(name, caps));
        ch
    }

    /// Pairs, then reconnects with caps; returns the welcomed channel, id and key.
    fn pair_with_caps(handle: &ServerHandle, port: u16, name: &str, caps: &[&str]) -> (Client, String, Keypair) {
        let (ch, id, me) = pair_client(handle, port, name);
        drop(ch);
        wait_until(|| !handle.status().assistants.iter().any(|a| a.id == id && a.connected));
        let mut ch = connect_as(handle, port, &me, name, caps);
        assert!(matches!(recv(&mut ch), Down::Welcome { .. }));
        (ch, id, me)
    }

    pub(crate) fn send(ch: &mut Client, up: &Up) {
        ch.send(&encode(up)).unwrap();
    }

    pub(crate) fn recv(ch: &mut Client) -> Down {
        let end = soon();
        loop {
            match ch.read().unwrap() {
                Got::Text(t) => return decode_down(&t).unwrap(),
                Got::Closed => panic!("closed"),
                _ if Instant::now() > end => panic!("nothing within 5 s"),
                _ => {}
            }
        }
    }
```

`test_server_with` passes `Arc::new(Keypair::generate())` as the fourth
argument of `start_with`. Remove `mac` from the test imports.

- [ ] **Step 11: Server tests — rewrite the ones that speak the handshake**

Mechanical changes across the module: every `(ws, id, token)` from
`pair_client` becomes `(ws, id, key)`; every reconnect that did
`connect(port)` + `hello(…, Some(&id))` + `Challenge` + `answer(…)`
becomes `let mut ws = connect_as(&handle, port, &key, name, &[]); assert!(matches!(recv(&mut ws), Down::Welcome { .. }));`;
`hello_caps(&mut ws, name, None, caps)` + `Pair` + `Paired` + `Welcome` is
`pair_client_caps`. `MaybeTlsStream::Plain(s) = ws.get_ref()` in `reset`
becomes `ws.ws.get_ref()`. `assistant_config(port, id, token)` becomes:

```rust
    fn assistant_config(port: u16, main: &ServerHandle) -> NetworkConfig {
        NetworkConfig { role: NetworkRole::Assistant, main_host: "127.0.0.1".into(), main_port: port, main_key: crate::net::identity::encode_key(&main.public_key()), ..Default::default() }
    }
```

and each `client::run_once(&config, …)` / `client::reconnect(…)` gains the
Maya's `&key` / `Arc::new(key)` identity argument. `FakeClientNotify`
keeps `key: Mutex<Option<String>>` instead of `creds` (its `paired` stores
the main key). `Leader::moved` records `(host, port, key)`.

The tests whose meaning changes, with their new bodies:

```rust
    #[test]
    fn pairs_snapshots_and_round_trips_a_command() {
        let (handle, port) = test_server();
        let (mut ws, id, key) = pair_client(&handle, port, "laptop");
        send(&mut ws, &Up::Board { cards: vec![card("r1", "remote")], dirs: vec!["proj".into()], agents: Some(vec![crate::agents::claude()]) });
        wait_until(|| handle.shared.lock().unwrap().boards.iter().any(|b| b.machine == "laptop" && b.cards.len() == 1));
        let shared = handle.shared.clone();
        let t = std::thread::spawn(move || send_command_with(&shared, "laptop", CommandKind::Compact { session: "r1".into() }, Duration::from_secs(5)));
        let Down::Command { id: cmd, kind } = recv(&mut ws) else { panic!("expected command") };
        assert_eq!(kind, CommandKind::Compact { session: "r1".into() });
        send(&mut ws, &Up::Result { id: cmd, ok: true, error: None, data: None });
        assert!(t.join().unwrap().is_ok());
        // Reconnect with the key: IK, no code, the same entry.
        drop(ws);
        wait_until(|| !handle.status().assistants.iter().any(|a| a.id == id && a.connected));
        let mut ws = connect_as(&handle, port, &key, "laptop", &[]);
        assert!(matches!(recv(&mut ws), Down::Welcome { .. }));
        assert_eq!(handle.status().assistants.len(), 1);
        handle.stop();
    }

    #[test]
    fn an_unknown_key_is_told_it_was_removed_and_a_wrong_code_is_refused() {
        let (handle, port) = test_server();
        let mut ws = connect_as(&handle, port, &Keypair::generate(), "x", &[]);
        assert!(matches!(recv(&mut ws), Down::Bye { reason } if reason == "removed"), "unknown key");
        let (code, _) = handle.open_pairing(now_ms()).unwrap();
        let wrong = if code == "000000" { "111111" } else { "000000" };
        let mut raw = connect(port);
        assert_eq!(secure::pair(&mut raw, &Keypair::generate(), wrong, soon(), &|| false).unwrap_err(), secure::Fail::Refused(secure::WRONG_CODE_REASON.into()));
        let err = send_command_with(&handle.shared, "ghost", CommandKind::Compact { session: "r".into() }, Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("not connected"), "{err}");
        handle.stop();
    }

    /// One pairing attempt with `code`; true when it was welcomed.
    fn try_code(port: u16, code: &str) -> bool {
        let mut raw = connect(port);
        let Ok((noise, _)) = secure::pair(&mut raw, &Keypair::generate(), code, soon(), &|| false) else { return false };
        let mut ch = Channel::new(raw, noise);
        send(&mut ch, &hello_up("guesser", &[]));
        let end = soon();
        loop {
            match ch.read() {
                Ok(Got::Text(t)) => return matches!(decode_down(&t), Ok(Down::Welcome { .. })),
                Ok(Got::Idle) if Instant::now() < end => {}
                _ => return false,
            }
        }
    }

    #[test]
    fn a_code_is_used_up_after_three_wrong_tries_from_any_address() {
        let (handle, port) = test_server();
        let (code, _) = handle.open_pairing(now_ms()).unwrap();
        let wrong = if code == "000000" { "111111" } else { "000000" };
        for _ in 0..3 {
            assert!(!try_code(port, wrong));
        }
        wait_until(|| handle.status().code_used_up);
        assert!(handle.status().code.is_none());
        assert!(!try_code(port, &code), "the right code no longer works");
        handle.open_pairing(now_ms()).unwrap();
        assert!(!handle.status().code_used_up, "a new code clears it");
        handle.stop();
    }

    #[test]
    fn an_abandoned_pairing_uses_a_try() {
        let (handle, port) = test_server();
        handle.open_pairing(now_ms()).unwrap();
        for _ in 0..3 {
            let mut raw = connect(port);
            // SPAKE2's first message, then gone.
            let (_, ours) = spake2::Spake2::<spake2::Ed25519Group>::start_a(&spake2::Password::new(b"123456"), &spake2::Identity::new(b"maya-join"), &spake2::Identity::new(b"maya-main"));
            let mut first = vec![secure::MODE_PAIR];
            first.extend(ours);
            raw.send(Message::binary(first)).unwrap();
            let _ = raw.read();
            drop(raw);
        }
        wait_until(|| handle.status().code_used_up);
        handle.stop();
    }

    #[test]
    fn two_joiners_with_the_right_code_pair_one_machine() {
        let (handle, port) = test_server();
        let (code, _) = handle.open_pairing(now_ms()).unwrap();
        let mut a = connect(port);
        let mut b = connect(port);
        let (na, _) = secure::pair(&mut a, &Keypair::generate(), &code, soon(), &|| false).unwrap();
        let (nb, _) = secure::pair(&mut b, &Keypair::generate(), &code, soon(), &|| false).unwrap();
        let (mut a, mut b) = (Channel::new(a, na), Channel::new(b, nb));
        send(&mut a, &hello_up("first", &[]));
        assert!(matches!(recv(&mut a), Down::Welcome { .. }));
        send(&mut b, &hello_up("second", &[]));
        assert!(matches!(recv(&mut b), Down::Bye { reason } if reason == secure::WRONG_CODE_REASON));
        assert_eq!(handle.status().assistants.len(), 1);
        handle.stop();
    }

    #[test]
    fn a_protocol_1_maya_is_answered_in_plain_text_that_it_can_show() {
        let (handle, port) = test_server();
        let mut raw = connect(port);
        raw.send(Message::text(r#"{"type":"hello","protocol":1,"app":"0.21.0","name":"old","hostname":"h","platform":"macos"}"#)).unwrap();
        let reason = loop {
            match raw.read().unwrap() {
                Message::Text(t) => break decode_down(&t).unwrap(),
                _ => continue,
            }
        };
        assert_eq!(reason, Down::Bye { reason: "protocol".into() });
        handle.stop();
    }
```

Rename and rewrite the address-identity tests to key identity:

```rust
    #[test]
    fn a_maya_pairing_again_with_its_key_keeps_its_entry() {
        let counter = Arc::new(Counter::default());
        let (handle, port) = test_server_with(counter.clone());
        let me = Keypair::generate();
        let (ws, first, _) = pair_as(&handle, port, &me, "desk", &[]);
        drop(ws);
        wait_until(|| !handle.status().assistants.iter().any(|a| a.connected));
        let (_ws, again, _) = pair_as(&handle, port, &me, "desk2", &[]);
        assert_eq!(again, first, "the same key: the same entry");
        assert_eq!(handle.status().assistants.len(), 1);
        assert_eq!(handle.status().assistants[0].name, "desk2", "names follow the machine");
        handle.stop();
    }

    #[test]
    fn two_mayas_at_one_address_are_two_entries_and_equal_names_carry_it() {
        let (handle, port) = test_server();
        let (_a, a, _) = pair_client(&handle, port, "laptop");
        let (_b, b, _) = pair_client(&handle, port, "laptop");
        assert_ne!(a, b);
        let names: Vec<String> = handle.status().assistants.iter().map(|s| s.name.clone()).collect();
        assert!(names.iter().all(|n| n.contains("127.0.0.1")), "{names:?}");
        handle.stop();
    }
```

Rewrite `promote_hands_the_sealed_list_over_moves_followers_and_names_the_rest`
as `promote_hands_the_keys_over_moves_followers_with_the_new_key_and_names_the_rest`:
the promoted side's `Down::Command { kind: CommandKind::Promote { list }, .. }`
now carries the list directly; assert every entry has a non-empty `key`,
none has a `token` in `encode(&list)`, the last entry's `key` equals
`encode_key(&handle.public_key())`, and each follower's `Down::Move` has
`key == ` the promoted entry's key. Add:

```rust
    #[test]
    fn promote_refuses_an_entry_without_a_key() {
        let (handle, port) = test_server();
        let (_g, g_id, _) = pair_with_caps(&handle, port, "laptop", &["follow", "lead"]);
        handle.shared.lock().unwrap().paired.iter_mut().for_each(|p| p.key.clear());
        assert_eq!(handle.promote(&g_id).unwrap_err(), "laptop has no key; pair it again.");
        assert!(!lock(&handle.shared).handover);
        handle.stop();
    }
```

Replace `fake_main` and `fake_pairing_main` and their two tests with:

```rust
    /// A stranger at the main's address: takes the first frame, answers it
    /// with `answer`, then sends a plain-text command. Returns its port.
    fn fake_main(answer: fn(&mut WebSocket<TcpStream>)) -> u16 {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            let _ = ws.read();
            answer(&mut ws);
            let _ = ws.send(Message::text(encode(&Down::Command { id: 1, kind: CommandKind::Compact { session: "r1".into() } })));
            let _ = ws.read();
        });
        port
    }

    #[test]
    fn the_client_keeps_nothing_from_a_main_that_does_not_know_the_code() {
        // It answers SPAKE2 with its own guess, then garbage where the
        // handshake goes on.
        let port = fake_main(|ws| {
            let (_, ours) = spake2::Spake2::<spake2::Ed25519Group>::start_b(&spake2::Password::new(b"999999"), &spake2::Identity::new(b"maya-join"), &spake2::Identity::new(b"maya-main"));
            let mut b = vec![secure::KIND_MESSAGE];
            b.extend(ours);
            let _ = ws.send(Message::binary(b));
            let _ = ws.read();
            let mut garbage = vec![secure::KIND_MESSAGE];
            garbage.extend([7u8; 96]);
            let _ = ws.send(Message::binary(garbage));
        });
        let exec = Arc::new(FakeExec::default());
        let notify = Arc::new(FakeClientNotify::default());
        let config = NetworkConfig { role: NetworkRole::Assistant, main_host: "127.0.0.1".into(), main_port: port, ..Default::default() };
        let out = client::run_once(&config, &Keypair::generate(), exec.clone(), notify.clone(), &AtomicBool::new(false), Some("123456"));
        assert_eq!(out, Err(client::PAIRING_FAILED.to_string()));
        assert!(notify.key.lock().unwrap().is_none(), "nothing is saved");
        assert!(notify.main.lock().unwrap().is_none(), "never reported as connected");
        assert!(exec.kinds.lock().unwrap().is_empty());
    }

    #[test]
    fn the_client_refuses_a_main_that_does_not_hold_the_pinned_key() {
        let port = fake_main(|ws| {
            let mut garbage = vec![secure::KIND_MESSAGE];
            garbage.extend([7u8; 48]);
            let _ = ws.send(Message::binary(garbage));
        });
        let exec = Arc::new(FakeExec::default());
        let notify = Arc::new(FakeClientNotify::default());
        let config = NetworkConfig { role: NetworkRole::Assistant, main_host: "127.0.0.1".into(), main_port: port, main_key: crate::net::identity::encode_key(&Keypair::generate().public), ..Default::default() };
        let out = client::run_once(&config, &Keypair::generate(), exec.clone(), notify.clone(), &AtomicBool::new(false), None);
        assert_eq!(out, Err(client::KEY_CHANGED.to_string()));
        assert!(exec.kinds.lock().unwrap().is_empty(), "no command from an impostor runs");
        assert!(notify.main.lock().unwrap().is_none());
        assert_eq!(*notify.errors.lock().unwrap(), [client::KEY_CHANGED]);
    }

    #[test]
    fn the_client_says_the_main_is_older_when_it_speaks_plain_text() {
        let port = fake_main(|ws| {
            let _ = ws.send(Message::text(r#"{"type":"challenge","nonce":"n"}"#));
        });
        let config = NetworkConfig { role: NetworkRole::Assistant, main_host: "127.0.0.1".into(), main_port: port, main_key: crate::net::identity::encode_key(&Keypair::generate().public), ..Default::default() };
        let out = client::run_once(&config, &Keypair::generate(), Arc::new(FakeExec::default()), Arc::new(FakeClientNotify::default()), &AtomicBool::new(false), None);
        assert_eq!(out, Err(client::OLD_MAIN.to_string()));
    }

    #[test]
    fn a_move_with_a_bad_key_is_not_followed() {
        let (handle, port) = test_server();
        let me = Keypair::generate();
        let (ws, id, _) = pair_as(&handle, port, &me, "laptop", &["follow"]);
        drop(ws);
        wait_until(|| !handle.status().assistants.iter().any(|a| a.connected));
        let leader = Arc::new(Leader { caps: vec!["follow".into()], ..Default::default() });
        let stop = Arc::new(AtomicBool::new(false));
        let config = assistant_config(port, &handle);
        let (l, s, k) = (leader.clone(), stop.clone(), me.clone());
        let run = std::thread::spawn(move || client::run_once(&config, &k, Arc::new(FakeExec::default()), l, &s, None));
        wait_until(|| handle.status().assistants.iter().any(|a| a.id == id && a.connected));
        let tx = handle.shared.lock().unwrap().conns.get(&id).unwrap().tx.clone();
        tx.send(Down::Move { host: "10.0.0.9".into(), port: 4127, key: "not a key".into() }).unwrap();
        let out = run.join().unwrap();
        assert!(out.unwrap_err().starts_with("Could not follow the main Maya to 10.0.0.9"));
        assert!(leader.moved.lock().unwrap().is_none(), "the old main is kept");
        stop.store(true, Ordering::SeqCst);
        handle.stop();
    }
```

Rewrite `an_assistant_gone_right_after_sending_pair_leaves_no_entry` as
`an_assistant_gone_right_after_its_hello_leaves_no_entry`: run
`secure::pair` to completion on a raw socket, `ch.write(&encode(&hello_up("ghost", &[])))`,
`ch.flush()`, then `reset(ch)` (whose body now reads
`let MaybeTlsStream::Plain(s) = ch.ws.get_ref() else { panic!("plain socket") };`);
keep the same assertions. `frames_are_small_until_welcome_and_large_after`
keeps its oversized-text first half unchanged (the WebSocket limit still
closes it) and its second half through `pair_client`.

`garbage_disconnects_wrong_macs_lockouts_removal_and_stop` becomes
`garbage_disconnects_lockouts_removal_and_stop`: delete its wrong-MAC part;
its lockout part makes three failed tries with `try_code(port, wrong)` and
asserts that a fourth attempt from the same address, after `open_pairing`,
is refused before SPAKE2:
`assert_eq!(secure::pair(&mut connect(port), &Keypair::generate(), &new_code, soon(), &|| false).unwrap_err(), secure::Fail::Refused("too many attempts".into()));`

Every other test only needs the mechanical changes above.

- [ ] **Step 12: Run the core tests**

Run: `cargo test -p maya-core`
Expected: all pass. A failure in `run_pairs_connects…` on macOS may be the
known flaky test (see memory); rerun once before debugging.

- [ ] **Step 13: Commit**

```bash
git add core/
git commit -m "feat(net)!: speak protocol 2: keys, SPAKE2 pairing, encrypted frames

Every Maya pairs again: protocol 1 pairings are dropped on load.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: A tampering proxy proves frames cannot be changed or injected

**Files:**
- Create: `core/src/net/tamper_tests.rs`
- Modify: `core/src/net/server.rs` (declare the file inside `mod tests`)

**Interfaces:**
- Consumes: `server::tests::{test_server, pair_as, assistant_config, FakeExec, FakeClientNotify, wait_until, card}` (make `FakeExec`, `FakeClientNotify` and `assistant_config` `pub(crate)` if they are not), `client::run_once`, `identity::Keypair`

- [ ] **Step 1: Declare the module**

At the end of `server.rs`'s `mod tests`, add:

```rust
    #[path = "tamper_tests.rs"]
    mod tamper;
```

- [ ] **Step 2: Write the tests and the proxy**

Create `core/src/net/tamper_tests.rs`:

```rust
//! A man in the middle between a real assistant and a real main, at the
//! WebSocket level: it relays every frame and does one thing wrong to the
//! frame numbered `at` in one direction. Each kind of tampering must close
//! the connection and run nothing.

use super::*;
use crate::net::identity::Keypair;
use std::net::TcpListener;
use std::sync::mpsc;

#[derive(Clone, Copy, Debug)]
enum Harm {
    /// Flip the last byte of the frame.
    Flip,
    /// Send the frame twice.
    Replay,
    /// Hold the frame and send the next one first.
    Reorder,
    /// Insert a plain-text command before the frame.
    InjectText,
    /// Insert 64 random bytes as a binary frame before it.
    InjectBinary,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Way {
    /// Main to assistant.
    Down,
    /// Assistant to main.
    Up,
}

/// What to forward for frame `m`, the `count`th going `dir`: harmed when it
/// is the chosen one, and a frame held back for `Reorder` after the next.
fn relay(m: Message, dir: Way, count: &mut usize, target: (Way, usize, Harm), held: &mut Option<Message>) -> Vec<Message> {
    let (way, at, harm) = target;
    let mut out = vec![];
    if dir == way && *count == at {
        match harm {
            Harm::Flip => {
                let mut b = m.into_data().to_vec();
                *b.last_mut().unwrap() ^= 1;
                out.push(Message::binary(b));
            }
            Harm::Replay => out.extend([m.clone(), m]),
            Harm::Reorder => *held = Some(m),
            Harm::InjectText => out.extend([Message::text(encode(&Down::Command { id: 99, kind: CommandKind::Compact { session: "r1".into() } })), m]),
            Harm::InjectBinary => out.extend([Message::binary(crate::net::protocol::random(64)), m]),
        }
    } else {
        out.push(m);
        if dir == way {
            if let Some(h) = held.take() {
                out.push(h);
            }
        }
    }
    *count += 1;
    out
}

/// Listens on a free port and relays one connection to `main_port`,
/// harming frame `at` (counted per direction from 0) going `way`.
fn proxy(main_port: u16, way: Way, at: usize, harm: Harm) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let (inbound, _) = listener.accept().unwrap();
        inbound.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        let mut assistant = tungstenite::accept(inbound).unwrap();
        let (mut main, _) = tungstenite::connect(format!("ws://127.0.0.1:{main_port}/")).unwrap();
        if let MaybeTlsStream::Plain(s) = main.get_ref() {
            s.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        }
        let target = (way, at, harm);
        let (mut ups, mut downs, mut held) = (0usize, 0usize, None);
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            match assistant.read() {
                Ok(m) if m.is_close() => return,
                Ok(m) => {
                    for m in relay(m, Way::Up, &mut ups, target, &mut held) {
                        if main.send(m).is_err() {
                            return;
                        }
                    }
                }
                Err(e) if crate::net::secure::is_timeout(&e) => {}
                Err(_) => return,
            }
            match main.read() {
                Ok(m) if m.is_close() => return,
                Ok(m) => {
                    for m in relay(m, Way::Down, &mut downs, target, &mut held) {
                        if assistant.send(m).is_err() {
                            return;
                        }
                    }
                }
                Err(e) if crate::net::secure::is_timeout(&e) => {}
                Err(_) => return,
            }
        }
    });
    port
}

/// Pairs a Maya, then runs a real client for it through a harming proxy.
/// Returns (the client's result, the commands it ran).
fn through(way: Way, at: usize, harm: Harm) -> (Result<(), String>, Vec<CommandKind>) {
    let (handle, port) = test_server();
    let me = Keypair::generate();
    let (ws, id, _) = pair_as(&handle, port, &me, "laptop", &[]);
    drop(ws);
    wait_until(|| !handle.status().assistants.iter().any(|a| a.connected));
    let via = proxy(port, way, at, harm);
    let mut config = assistant_config(via, &handle);
    config.name = "laptop".into();
    let exec = Arc::new(FakeExec::default());
    let stop = Arc::new(AtomicBool::new(false));
    let (e, s, k) = (exec.clone(), stop.clone(), me.clone());
    let run = std::thread::spawn(move || client::run_once(&config, &k, e, Arc::new(FakeClientNotify::default()), &s, None));
    // Once connected, the main sends one command: the frame after `welcome`.
    let shared = handle.shared.clone();
    let id2 = id.clone();
    std::thread::spawn(move || {
        let end = Instant::now() + Duration::from_secs(3);
        while Instant::now() < end && !shared.lock().unwrap().conns.contains_key(&id2) {
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = send_command_to(&shared, &id2, CommandKind::Rename { session: "r1".into(), name: "n".into() }, Duration::from_secs(2));
    });
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || tx.send(run.join().unwrap()));
    let out = rx.recv_timeout(Duration::from_secs(20)).unwrap_or(Ok(()));
    stop.store(true, Ordering::SeqCst);
    handle.stop();
    let ran = exec.kinds.lock().unwrap().clone();
    (out, ran)
}

// Frame numbers: going Up, 0 is the IK message and 1 is `hello`, 2 the
// first board. Going Down, 0 is the IK answer, 1 is `welcome`, 2 the command.

#[test]
fn a_changed_byte_either_way_closes_the_connection() {
    for way in [Way::Up, Way::Down] {
        let (out, ran) = through(way, 2, Harm::Flip);
        assert!(out.is_err(), "{way:?}: {out:?}");
        if way == Way::Down {
            assert!(ran.is_empty(), "a changed command never runs");
        }
    }
}

#[test]
fn a_replayed_or_reordered_frame_closes_the_connection() {
    for harm in [Harm::Replay, Harm::Reorder] {
        let (out, ran) = through(Way::Down, 1, harm);
        assert!(out.is_err(), "{harm:?}: {out:?}");
        assert!(ran.is_empty(), "{harm:?}: nothing runs");
    }
}

#[test]
fn an_injected_command_never_runs() {
    for harm in [Harm::InjectText, Harm::InjectBinary] {
        let (out, ran) = through(Way::Down, 2, harm);
        assert!(out.is_err(), "{harm:?}: {out:?}");
        assert!(ran.is_empty(), "{harm:?}: neither the injected command nor the real one after it runs");
    }
}

#[test]
fn an_untouched_relay_works() {
    // The control: the same path with no harm connects and runs the command.
    let (handle, port) = test_server();
    let me = Keypair::generate();
    let (ws, id, _) = pair_as(&handle, port, &me, "laptop", &[]);
    drop(ws);
    wait_until(|| !handle.status().assistants.iter().any(|a| a.connected));
    let via = proxy(port, Way::Down, usize::MAX, Harm::Flip);
    let config = assistant_config(via, &handle);
    let exec = Arc::new(FakeExec::default());
    let stop = Arc::new(AtomicBool::new(false));
    let (e, s, k) = (exec.clone(), stop.clone(), me.clone());
    let run = std::thread::spawn(move || client::run_once(&config, &k, e, Arc::new(FakeClientNotify::default()), &s, None));
    wait_until(|| handle.status().assistants.iter().any(|a| a.id == id && a.connected));
    send_command_to(&handle.shared, &id, CommandKind::Compact { session: "r1".into() }, Duration::from_secs(5)).unwrap();
    assert_eq!(*exec.kinds.lock().unwrap(), [CommandKind::Compact { session: "r1".into() }]);
    stop.store(true, Ordering::SeqCst);
    let _ = run.join();
    handle.stop();
}
```

`MaybeTlsStream` comes from `tungstenite::stream::MaybeTlsStream`, which
the parent test module already imports; `use super::*;` brings it in.

- [ ] **Step 3: Run the tests**

Run: `cargo test -p maya-core tamper`
Expected: 4 passed. If `an_untouched_relay_works` fails, the proxy is
wrong, not the protocol: fix the proxy first.

- [ ] **Step 4: Commit**

```bash
git add core/src/net/tamper_tests.rs core/src/net/server.rs
git commit -m "test(net): a man in the middle cannot change, replay or inject frames"
```

---

### Task 5: The desktop app: key store, identity, New key and Forget main

**Files:**
- Create: `src-tauri/src/keystore.rs`
- Modify: `src-tauri/src/voice.rs`, `src-tauri/src/net_app.rs`, `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `identity::Keypair`, `client::pair_with(…, identity, …)`, `server::start_with(…, identity)`, `Promotion.main_key`, `ClientNotify::{paired, moved}` new shapes, `NetworkConfig.{main_key, pair_again}`
- Produces:
  - `keystore::Item { service, account, label, linux_account }`, `keystore::ELEVENLABS`, `keystore::NETWORK_KEY`
  - `keystore::store(&Item, &str) -> Result<(), String>`
  - `keystore::load(&Item) -> Result<Option<String>, String>` (`Ok(None)`: not stored; `Err`: the store could not be read)
  - `net_app::identity(&AppHandle) -> Result<Arc<Keypair>, String>`
  - `net_app::identity_from(load, store) -> Result<Keypair, String>` (pure, testable)
  - Tauri commands `network_new_key`, `network_forget_main`
  - `AppState.identity: Mutex<Option<Arc<Keypair>>>`

- [ ] **Step 1: Write the failing identity tests**

In `src-tauri/src/net_app.rs` tests (add a `#[cfg(test)] mod tests` if
there is none):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn a_stored_key_is_used_and_none_stored_makes_and_stores_one() {
        let k = Keypair::generate();
        let got = identity_from(|| Ok(Some(k.to_secret())), |_| panic!("not stored again")).unwrap();
        assert_eq!(got.public, k.public);
        let stored = RefCell::new(None);
        let made = identity_from(|| Ok(None), |s| {
            *stored.borrow_mut() = Some(s.to_string());
            Ok(())
        })
        .unwrap();
        assert_eq!(Keypair::from_secret(stored.borrow().as_ref().unwrap()).unwrap().public, made.public);
    }

    #[test]
    fn a_garbled_stored_key_is_an_error_not_a_new_key() {
        let out = identity_from(|| Ok(Some("garbage".into())), |_| panic!("never overwritten"));
        assert!(out.unwrap_err().starts_with("Could not reach the key store: "));
        let out = identity_from(|| Err("the keyring is locked".into()), |_| panic!("never overwritten"));
        assert_eq!(out.unwrap_err(), "Could not reach the key store: the keyring is locked");
    }
}
```

Run: `cargo test -p maya net_app`
Expected: FAIL to compile (`identity_from` missing; the app also has the
Task 3 compile errors — fix those in the steps below before rerunning).

- [ ] **Step 2: The key store module**

Create `src-tauri/src/keystore.rs`, moving the platform code out of
`voice.rs` and making a lookup tell "not stored" from "could not read":

```rust
//! Secrets in the OS key store: the login Keychain on macOS, Credential
//! Manager on Windows, GNOME Keyring through `secret-tool` on Linux. A
//! secret never goes on a command line: `security -i` and `secret-tool`
//! read it from stdin.

use std::process::Stdio;

pub struct Item {
    /// The Keychain service and the Credential Manager target.
    pub service: &'static str,
    /// The Keychain account and the Credential Manager user name.
    pub account: &'static str,
    /// What GNOME Keyring shows.
    pub label: &'static str,
    /// The `account` attribute on Linux (`service` is always `maya` there).
    pub linux_account: &'static str,
}

pub const ELEVENLABS: Item = Item { service: "maya-elevenlabs", account: "api-key", label: "Maya ElevenLabs", linux_account: "elevenlabs" };
pub const NETWORK_KEY: Item = Item { service: "maya-network-key", account: "private-key", label: "Maya network key", linux_account: "network-key" };

/// `security` exits with this when the item is not in the keychain.
#[cfg(target_os = "macos")]
const ERR_SEC_ITEM_NOT_FOUND: i32 = 44;

#[cfg(target_os = "macos")]
pub fn store(item: &Item, secret: &str) -> Result<(), String> {
    use std::io::Write;
    let secret = secret.trim();
    if secret.is_empty() || secret.contains(char::is_whitespace) || secret.contains('"') {
        return Err("The secret cannot be stored as given.".into());
    }
    let mut child = maya_core::command("security").arg("-i").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().map_err(|e| format!("could not run security: {e}"))?;
    let line = format!("add-generic-password -U -s {} -a {} -w {}\n", item.service, item.account, secret);
    child.stdin.take().ok_or("no stdin for security")?.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let err = String::from_utf8_lossy(&out.stderr);
    if out.status.success() && err.trim().is_empty() {
        Ok(())
    } else {
        Err(format!("Keychain refused it: {}", err.trim()))
    }
}

#[cfg(target_os = "macos")]
pub fn load(item: &Item) -> Result<Option<String>, String> {
    let out = maya_core::command("security").args(["find-generic-password", "-s", item.service, "-a", item.account, "-w"]).stdin(Stdio::null()).stderr(Stdio::piped()).output().map_err(|e| format!("could not run security: {e}"))?;
    match out.status.code() {
        Some(0) => Ok(Some(String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())),
        Some(ERR_SEC_ITEM_NOT_FOUND) => Ok(None),
        _ => Err(format!("Keychain: {}", String::from_utf8_lossy(&out.stderr).trim())),
    }
}
```

   `security -i` reports a failed command on stderr but may still exit 0,
   hence the stderr check in `store`.

   Windows: move `store_credential`, `load_credential` and `wide` from
   `voice.rs`, taking `item.service` / `item.account`; `load` returns
   `Ok(None)` when `CredReadW` fails and `GetLastError()` is
   `ERROR_NOT_FOUND` (both from `windows_sys::Win32::Foundation`, whose
   feature the workspace already enables) and
   `Err(format!("Credential Manager: {}", std::io::Error::last_os_error()))`
   for any other failure.

   Linux: move `store_key` / `load_key` as `store(item, secret)` /
   `load(item)`, with the arguments
   `["store", "--label", item.label, "service", "maya", "account", item.linux_account]`
   and `["lookup", "service", "maya", "account", item.linux_account]`.
   `load`: exit 0 with output is `Ok(Some(..))`; a non-zero exit with empty
   stderr is `Ok(None)` (secret-tool's "not found"); a non-zero exit with
   stderr, or failing to run it, is `Err(...)` with the stderr.
   Move `secret_tool_store_args` / `secret_tool_lookup_args` here as
   `secret_tool_args(item) -> (Vec<String>, Vec<String>)` and move their
   test from `voice.rs`.

In `voice.rs`, delete the moved code and make
`store_key(key)` call `crate::keystore::store(&crate::keystore::ELEVENLABS, key)`
and `load_key()` return
`crate::keystore::load(&crate::keystore::ELEVENLABS).ok().flatten()`.
Add `pub mod keystore;` to `src-tauri/src/lib.rs` beside `pub mod voice;`.

- [ ] **Step 3: The desktop identity**

In `src-tauri/src/lib.rs`, add to `AppState`:

```rust
    /// This Maya's network key pair, read from the key store at first use.
    pub identity: Mutex<Option<Arc<maya_core::net::identity::Keypair>>>,
```

and initialise it `Mutex::new(None)` where `AppState` is built.

In `src-tauri/src/net_app.rs`:

```rust
use maya_core::net::identity::Keypair;

const KEY_STORE: &str = "Could not reach the key store: ";

/// The key pair from `load`, or a new one saved with `store` when none is
/// stored. A value that is not a key, or a store that cannot be read, is an
/// error: a new key would silently drop every pairing.
pub fn identity_from(load: impl FnOnce() -> Result<Option<String>, String>, store: impl FnOnce(&str) -> Result<(), String>) -> Result<Keypair, String> {
    match load().map_err(|e| format!("{KEY_STORE}{e}"))? {
        Some(secret) => Keypair::from_secret(&secret).map_err(|e| format!("{KEY_STORE}{e}")),
        None => {
            let k = Keypair::generate();
            store(&k.to_secret()).map_err(|e| format!("{KEY_STORE}{e}"))?;
            Ok(k)
        }
    }
}

/// This Maya's key pair, read once and kept for the app's life.
pub fn identity(app: &AppHandle) -> Result<Arc<Keypair>, String> {
    let state = app.state::<AppState>();
    let mut held = state.identity.lock().unwrap();
    if let Some(k) = held.as_ref() {
        return Ok(k.clone());
    }
    let k = Arc::new(identity_from(|| crate::keystore::load(&crate::keystore::NETWORK_KEY), |s| crate::keystore::store(&crate::keystore::NETWORK_KEY, s))?);
    *held = Some(k.clone());
    Ok(k)
}
```

Then:
- `start_server(app, port)`: `let id = identity(&app)?;` and pass it as
  `start_with`'s fourth argument.
- `start(app)`: inside the spawned body,
  `match identity(&app) { Ok(id) => reconnect(&move || …, id, exec, notify, &stop), Err(e) => notify.disconnected(&e) }`.
- `pair(app, …)`: `let (main, main_key) = pair_with(exec, identity(app)?, host, port, name, code, vec![])?;`
  and in the `update_config` closure set `c.network.main_key = main_key;`
  and `c.network.pair_again = false;` (remove the `assistant_id`/`token`
  lines). Update its doc: "stores the main's key and the link".
- `TauriClientNotify::paired(&self, _: &str) {}`.
- `promoted`: `start_with(…, after.listen_port(), identity(&self.app)?)`.
- `moved(&self, host, port, key)`: also set `c.network.main_key = key.into();`.

In `src-tauri/src/lib.rs`:
- `TauriNetNotify::paired` (the server saved a new pairing): in the same
  config change, set `config.network.pair_again = false;`. Read
  `save_paired`'s shape and set the flag on the config it saves.
- The config-from-the-page guard at `lib.rs:1090` keeps the server's and
  pairing's fields:

```rust
        config.network.assistants = before.network.assistants.clone();
        config.network.main_key = before.network.main_key.clone();
        config.network.pair_again = before.network.pair_again;
```

- `network_promote`:
  `update_config(&app, |c| net::promote::as_assistant_of(c, &out.host, out.port, &out.main_key))`.
- `network_status_of`: set `status.pair_again` from
  `state.store.lock().unwrap().config.network.pair_again` (take `store`
  after releasing `network`, per the lock order comment).
- New commands (register both in the `invoke_handler` list next to
  `network_pair`):

```rust
/// Drops every pairing and makes a fresh key pair: for a key that may have
/// leaked. The role goes Off; the user turns Main or Assistant on again.
#[tauri::command]
fn network_new_key(app: AppHandle, state: TauriState<AppState>) -> Result<NetworkStatus, String> {
    // The pairings go first: if the new key cannot be stored, the old key
    // stays and nothing is half-changed but the (already dropped) pairings.
    update_config(&app, |c| {
        c.network.role = config::NetworkRole::Off;
        c.network.assistants.clear();
        c.network.main_key.clear();
        c.network.main_host.clear();
        c.network.main_port = 0;
        c.network.pair_again = false;
    })?;
    let k = maya_core::net::identity::Keypair::generate();
    keystore::store(&keystore::NETWORK_KEY, &k.to_secret()).map_err(|e| format!("Could not reach the key store: {e}"))?;
    *state.identity.lock().unwrap() = Some(Arc::new(k));
    log::line("network", "new key made; every pairing dropped");
    Ok(network_status_of(&state))
}

/// Forgets the main: its key, host and port go and the role goes Off.
#[tauri::command]
fn network_forget_main(app: AppHandle, state: TauriState<AppState>) -> Result<NetworkStatus, String> {
    update_config(&app, |c| {
        c.network.role = config::NetworkRole::Off;
        c.network.main_key.clear();
        c.network.main_host.clear();
        c.network.main_port = 0;
    })?;
    Ok(network_status_of(&state))
}
```

- Fix the remaining compile errors: test literals with `token: "t".into()`
  become `key: "k".into()`.

- [ ] **Step 4: Run the app tests**

Run: `cargo test -p maya`
Expected: all pass, the two new identity tests included.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/
git commit -m "feat(app): keep the network key in the OS key store; New key and Forget main"
```

---

### Task 6: The phone

**Files:**
- Modify: `mobile/src/hub.rs`, `mobile/src/commands.rs`, the file that registers the phone's commands (find it with `grep -rn network_pair mobile/src`), `mobile/tests/round_trip.rs`

**Interfaces:**
- Consumes: `identity::{Keypair, load_or_create, replace, FILE}`, the Task 3 core signatures
- Produces: `Hub::identity(&self) -> Result<Arc<Keypair>, String>`, `Hub::new_key(&self) -> Result<NetworkStatus, String>`, command `network_new_key`

- [ ] **Step 1: Write the failing test**

In `mobile/tests/round_trip.rs`, add (it uses the file's `new_hub` and
`free_port` helpers):

```rust
#[test]
fn the_phone_keeps_one_key_until_new_key_drops_every_pairing() {
    let (hub, _page, dir) = new_hub(free_port());
    let first = hub.identity().unwrap();
    assert_eq!(hub.identity().unwrap().public, first.public, "kept");
    assert!(dir.path().join("maya").join(maya_core::net::identity::FILE).exists());
    hub.update_config(|c| c.network.main_key = "a2V5".into()).unwrap();
    hub.new_key().unwrap();
    assert_ne!(hub.identity().unwrap().public, first.public);
    let config = hub.settings.lock().unwrap().config.clone();
    assert!(config.network.main_key.is_empty() && config.network.assistants.is_empty());
    assert_eq!(config.network.role, NetworkRole::Off);
}
```

Run: `cargo test -p maya-mobile --test round_trip the_phone_keeps_one_key`
Expected: FAIL to compile.

- [ ] **Step 2: Implement**

In `hub.rs`:
- `Hub` gains `identity: Mutex<Option<Arc<Keypair>>>` (initialised `None`
  in `with_clock`).
- Add:

```rust
    /// This phone's key pair, from `identity.key` beside its config.
    pub fn identity(&self) -> Result<Arc<Keypair>, String> {
        let mut held = self.identity.lock().unwrap();
        if let Some(k) = held.as_ref() {
            return Ok(k.clone());
        }
        let k = Arc::new(identity::load_or_create(&self.maya_dir.join(identity::FILE))?);
        *held = Some(k.clone());
        Ok(k)
    }

    /// Drops every pairing and makes a fresh key: the server and the link
    /// stop and the role goes Off.
    pub fn new_key(&self) -> Result<NetworkStatus, String> {
        self.stop_server();
        self.stop_client();
        self.update_config(|c| {
            c.network.role = NetworkRole::Off;
            c.network.assistants.clear();
            c.network.main_key.clear();
            c.network.main_host.clear();
            c.network.main_port = 0;
            c.network.pair_again = false;
        })?;
        let k = identity::replace(&self.maya_dir.join(identity::FILE))?;
        *self.identity.lock().unwrap() = Some(Arc::new(k));
        let status = self.status();
        self.sink.network(&status);
        Ok(status)
    }
```

- `start_with(…)` calls (two, at `hub.rs:248` and `:254`) get
  `self.identity()?` as the fourth argument (map the error into
  `main_error` the way a bind failure is).
- `reconnect(&config_of, …)` (at `:305`) gets the identity: fetch it before
  spawning; on error, set the link's error to it and do not spawn.
- `join`: `pair_with(Arc::new(NoSessions), self.identity()?, &host, port, &name, &code, phone_caps()).and_then(|(main, main_key)| { self.update_config(|c| { …; c.network.main_key = main_key; c.network.pair_again = false; }) … })`
  (remove the `assistant_id`/`token` lines).
- `stop_listening`: replace the `assistant_id`/`token` clears with
  `c.network.main_key.clear();`.
- `promote`: `promote::as_assistant_of(c, &out.host, out.port, &out.main_key)`.
- `HubClientNotify::paired(&self, _: &str) {}`;
  `moved(&self, host, port, key)` also sets `c.network.main_key = key.into();`.
- The server `Notify::paired` adapter: also `c.network.pair_again = false`.
- `status()`: fill `pair_again` from the config.
- Test literals: `token: "t".into()` → `key: "k".into()`.

In `mobile/src/commands.rs`, add beside `network_pair`:

```rust
#[tauri::command]
pub fn network_new_key(hub: H) -> Result<NetworkStatus, String> {
    hub.new_key()
}
```

and register it where the other `network_*` commands are registered.

In `mobile/tests/round_trip.rs`:
- `start_with(…)` calls get `Arc::new(Keypair::generate())`.
- `let (main_name, id, token) = pair_with(assistant.clone(), "127.0.0.1", port, "laptop", &code, vec![])`
  becomes `let me = Arc::new(Keypair::generate()); let (main_name, main_key) = pair_with(assistant.clone(), me.clone(), "127.0.0.1", port, "laptop", &code, vec![])`,
  and the `NetworkConfig { … assistant_id: id, token, … }` literals use
  `main_key`; the `reconnect`/`run_once` calls pass `me`.
- The test `ClientNotify` impls: `fn paired(&self, _: &str) {}`; any
  `moved` gains the `key` argument.

- [ ] **Step 3: Run the phone tests**

Run: `cargo test -p maya-mobile`
Expected: all pass.

- [ ] **Step 4: Commit**

```bash
git add mobile/
git commit -m "feat(mobile): keep the phone's key in identity.key; New key"
```

---

### Task 7: The CLI

**Files:**
- Modify: `cli/src/commands.rs`, `cli/src/args.rs`, `cli/src/lib.rs`, `cli/src/run_cmd.rs`, `cli/src/notify.rs`

**Interfaces:**
- Consumes: `identity::{load_or_create, replace, FILE}`, Task 3 client signatures
- Produces: `Cmd::Pair { host, port, code, name, new_key: bool }`; `commands::pair(claude_dir, host, port, name, code, new_key)`

- [ ] **Step 1: Write the failing tests**

In `cli/src/args.rs` tests:

```rust
    #[test]
    fn pair_takes_a_new_key_flag() {
        assert_eq!(parse(&a("pair 10.0.0.5 --code 483921 --new-key")).unwrap(), Cmd::Pair { host: "10.0.0.5".into(), port: 4127, code: "483921".into(), name: None, new_key: true });
    }
```

and add `new_key: false` to the two existing `Cmd::Pair` expectations.

In `cli/src/run_cmd.rs` tests, the preflight tests that set
`c.network.token`/`assistant_id` set `c.network.main_key = "a2V5".into()`
instead, and the assertions read `n.main_key`.

Run: `cargo test -p maya-cli`
Expected: FAIL to compile.

- [ ] **Step 2: Implement**

- `args.rs`: `Cmd::Pair` gains `new_key: bool`; `parse_pair` accepts
  `--new-key`. Add to `USAGE`'s pair line: `[--new-key]`.
- `commands.rs` `pair(…, new_key: bool)`:

```rust
    let key_path = claude_dir.join("maya").join(maya_core::net::identity::FILE);
    let identity = if new_key { maya_core::net::identity::replace(&key_path) } else { maya_core::net::identity::load_or_create(&key_path) }.map_err(|e| (1, e))?;
    let (main, main_key) = client::pair_with(exec, Arc::new(identity), host, port, &name, code, vec![]).map_err(|e| (1, e))?;
```

  then set `s.config.network.main_key = main_key;` and
  `s.config.network.pair_again = false;` (remove `assistant_id`/`token`).
- `lib.rs`: pass `new_key` through.
- `run_cmd.rs`:
  - `preflight`: the paired check is
    `NetworkRole::Assistant if config.network.main_key.is_empty() => return Err((2, "run `maya pair` first".into())),`.
  - Before `client::reconnect`, load the identity with
    `load_or_create(&claude_dir.join("maya").join(FILE))`, exit 2 with its
    error if it fails, and pass `Arc::new(identity)`.
  - `reload_network`: `serde_json::from_str::<Config>(&t)` becomes
    `.map(Config::migrate)`; the two clears become
    `s.config.network.main_key.clear();` / `n.main_key.clear();`.
- `notify.rs`: `fn paired(&self, _main_key: &str) {}`; `moved(&self, host,
  port, key)` also saves `c.network.main_key = key.into()`; its test sets
  and asserts `main_key` instead of `assistant_id`/`token`.

- [ ] **Step 3: Run the whole workspace**

Run: `cargo test --workspace`
Expected: all pass. (Known flaky: `run_pairs_connects…` on macOS; rerun once.)

- [ ] **Step 4: Commit**

```bash
git add cli/
git commit -m "feat(cli): keep the CLI's key in identity.key; pair --new-key"
```

---

### Task 8: Settings › Network on the desktop and the phone

**Files:**
- Modify: `src/settings.ts`, `src/settings.test.ts`, `src/main.ts` (if handlers are wired there), `src/mobile/network.ts`, `src/mobile/network.test.ts`

**Interfaces:**
- Consumes: `NetworkStatus.codeUsedUp`, `NetworkStatus.pairAgain`, commands `network_new_key`, `network_forget_main` (desktop), `network_new_key` (phone), the config JSON's `network.mainKey`

- [ ] **Step 1: Write the failing tests**

In `src/settings.test.ts`, add the new handlers to `handlers()`:
`onNewKeyAsk: vi.fn(), onNewKey: vi.fn(), onNewKeyCancel: vi.fn(), onForgetMain: vi.fn(),`
and add:

```ts
describe("secure pairing in Settings › Network", () => {
  const main = (extra: object = {}) => ({ role: "main" as const, code: null, assistants: [], assistant: { connected: false, mainName: null, error: null }, ...extra });

  it("says the code is used up on a main", () => {
    const el = renderSettings({ ...voiceBase, networkRole: "main", network: main({ codeUsedUp: true }) }, handlers());
    expect(el.textContent).toContain("Code used up after 3 wrong tries — Regenerate.");
    const fresh = renderSettings({ ...voiceBase, networkRole: "main", network: main() }, handlers());
    expect(fresh.textContent).not.toContain("Code used up");
  });

  it("tells an updated Maya to pair again", () => {
    const el = renderSettings({ ...voiceBase, networkRole: "main", network: main({ pairAgain: true }) }, handlers());
    expect(el.querySelector("[data-for=pair-again-note]")?.textContent).toBe("This Maya was updated to secure pairing — pair again.");
  });

  it("offers New key, which asks before it drops every pairing", () => {
    const h = handlers();
    const el = renderSettings({ ...voiceBase, networkRole: "main", network: main() }, h);
    el.querySelector<HTMLButtonElement>("button[data-action=new-key]")!.click();
    expect(h.onNewKeyAsk).toHaveBeenCalled();
    expect(h.onNewKey).not.toHaveBeenCalled();
    const asking = renderSettings({ ...voiceBase, networkRole: "main", network: main(), networkNewKeyConfirm: true }, h);
    expect(asking.textContent).toContain("Drop every pairing and make a new key?");
    asking.querySelector<HTMLButtonElement>("button[data-action=new-key-confirm]")!.click();
    expect(h.onNewKey).toHaveBeenCalled();
    asking.querySelector<HTMLButtonElement>("button[data-action=new-key-cancel]")!.click();
    expect(h.onNewKeyCancel).toHaveBeenCalled();
    const off = renderSettings({ ...voiceBase }, h);
    expect(off.querySelector("button[data-action=new-key]")).toBeNull();
  });

  it("offers Forget main on a paired assistant only", () => {
    const h = handlers();
    const paired = renderSettings({ ...voiceBase, networkRole: "assistant", networkPaired: true, networkMainHost: "desk.local" }, h);
    paired.querySelector<HTMLButtonElement>("button[data-action=forget-main]")!.click();
    expect(h.onForgetMain).toHaveBeenCalled();
    const fresh = renderSettings({ ...voiceBase, networkRole: "assistant" }, h);
    expect(fresh.querySelector("button[data-action=forget-main]")).toBeNull();
  });
});
```

In `src/mobile/network.test.ts`, add
`onNewKeyAsk: vi.fn(), onNewKey: vi.fn(), onNewKeyCancel: vi.fn()` to its
`handlers()`, and:

```ts
describe("secure pairing on the phone", () => {
  it("says the code is used up, and that an updated phone must pair again", () => {
    const el = renderNetwork({ ...base, status: { ...running, code: null, codeUsedUp: true, pairAgain: true } }, handlers(), NOW);
    expect(el.textContent).toContain("Code used up after 3 wrong tries — Regenerate.");
    expect(el.textContent).toContain("This Maya was updated to secure pairing — pair again.");
  });

  it("offers New key, which asks before it drops every pairing", () => {
    const h = handlers();
    renderNetwork(base, h, NOW).querySelector<HTMLButtonElement>("button[data-action=new-key]")!.click();
    expect(h.onNewKeyAsk).toHaveBeenCalled();
    const asking = renderNetwork({ ...base, newKey: true }, h, NOW);
    expect(asking.textContent).toContain("Drop every pairing and make a new key?");
    asking.querySelector<HTMLButtonElement>("button[data-action=new-key-confirm]")!.click();
    expect(h.onNewKey).toHaveBeenCalled();
  });
});
```

Run: `pnpm test`
Expected: the new tests FAIL.

- [ ] **Step 2: Implement**

`src/settings.ts`:
- `NetworkStatus` gains `codeUsedUp?: boolean;` and `pairAgain?: boolean;`.
- `NetworkConfigJson` (line ~213): replace `assistantId: string; token: string;`
  with `mainKey: string; pairAgain?: boolean;`, and the default object at
  line ~1161 uses `mainKey: ""` in place of `assistantId: "", token: ""`.
- `model.networkPaired = !!c.network?.mainKey;` (line ~1105); update the
  `networkPaired` doc: "The config holds the main's key: choosing Assistant
  reconnects without pairing."
- `SettingsModel` gains `networkNewKeyConfirm?: boolean;` ("New key's
  confirmation is open").
- `SettingsHandlers` gains `onNewKeyAsk(): void; onNewKey(): void;
  onNewKeyCancel(): void; onForgetMain(): void;`. Where the handlers are
  built (the `onRemoveAssistant` neighbourhood, `settings.ts:1261`):
  `onNewKeyAsk` sets `model.networkNewKeyConfirm = true` and repaints;
  `onNewKeyCancel` clears it and repaints; `onNewKey` clears it and runs
  `model.network = await invoke<NetworkStatus>("network_new_key"); model.networkPaired = false;`
  through `run(...)`; `onForgetMain` runs
  `model.network = await invoke<NetworkStatus>("network_forget_main"); model.networkPaired = false;`.
- Main panel: where the code is shown, when `!net.code && net.codeUsedUp`
  show a `div.settings__hint` with
  `"Code used up after 3 wrong tries — Regenerate."`.
- Both roles: when `net.pairAgain`, show a `div.settings__hint` with
  `data-for="pair-again-note"` and the text
  `"This Maya was updated to secure pairing — pair again."` at the top of
  the Network section.
- Both roles (Main and Assistant, not Off): a `button.settings__link`
  `data-action="new-key"` "New key" calling `h.onNewKeyAsk()`. While
  `model.networkNewKeyConfirm`, it is replaced by the text "Drop every
  pairing and make a new key?" with a button `data-action="new-key-confirm"`
  "Drop and make a new key" (`h.onNewKey()`) and `data-action="new-key-cancel"`
  "Cancel" (`h.onNewKeyCancel()`). Follow the Make main confirmation's markup.
- Assistant panel: when `model.networkPaired && !model.networkRepair`, next
  to "Pair again", a `button.settings__link` `data-action="forget-main"`
  "Forget main" that calls `h.onForgetMain()`.

`src/mobile/network.ts`: `NetworkStatus` gains `codeUsedUp?: boolean;
pairAgain?: boolean;`, `NetworkModel` gains `newKey?: boolean;`, and the
handlers gain `onNewKeyAsk`, `onNewKey`, `onNewKeyCancel`, wired in
`initNetwork` like `onMakeMain` / `onPromote` / `onPromoteCancel` (`onNewKey`
invokes `network_new_key` and stores the status it returns). Render the
same `codeUsedUp` hint, `pairAgain` note and New key confirmation as the
desktop. The phone already has Leave (`onStopListening`), which forgets the
main, so it needs no Forget main.

- [ ] **Step 3: Run the page tests**

Run: `pnpm test`
Expected: all pass.

- [ ] **Step 4: Check it in the real app**

Follow the project's run-and-verify memory: start vite and the debug
binary, open Settings › Network, screenshot Main with a code, regenerate,
and confirm New key's confirmation appears and Cancel closes it. Do not
press "Drop and make a new key" on the user's own machine without asking.

- [ ] **Step 5: Commit**

```bash
git add src/
git commit -m "feat(settings): show code used up, the pair-again note, New key and Forget main"
```

---

### Task 9: Docs, spec deviations and the version

**Files:**
- Modify: `README.md`, `docs/DEVELOPING.md`, `docs/superpowers/specs/2026-10-14-secure-protocol-design.md`, the version files via `scripts/set-version.sh`

- [ ] **Step 1: README**

Replace the "Trust" bullet (around `README.md:284`) and the "Plain text on
the LAN" bullet (around `:315`) with:

```markdown
- **Trust.** Pairing runs SPAKE2 over the six-digit code: whatever answers
  at the host you typed learns nothing about the code unless it is the Maya
  showing it, and a code allows three tries before it is used up. Each
  Maya then pins the other's public key; a machine is known by its key,
  not its address.
- **Encrypted.** Every frame between two Mayas is encrypted and
  authenticated (Noise, ChaCha20-Poly1305). A changed, replayed or injected
  frame closes the connection. The desktop app keeps its private key in
  the Keychain, Credential Manager or GNOME Keyring; the CLI and the phone
  in an owner-only `identity.key` beside their config. Any program running
  as you can read the CLI's file, and on macOS the Keychain item too
  (through `security`), so keep a leaked machine's key out with Remove on
  the main, or New key on the machine itself.
- **Outside the LAN.** Maya does not open anything to the internet. Reach
  a main from outside through a VPN such as Tailscale or WireGuard and
  pair with its address there.
```

- [ ] **Step 2: DEVELOPING.md**

In the Layout's `core/src/` entry, after `` `net/` with `protocol.rs` ``,
change "(message types, encode/decode, pairing and HMAC)" to "(message
types, encode/decode, the pairing window)", and add
"`identity.rs` (this Maya's key pair), `secure.rs` (SPAKE2 pairing, the
Noise handshakes and the encrypted `Channel`)," before `server.rs`. In the
`src-tauri/src/` entry add "`keystore.rs` (the OS key store)". In the
mobile section, after the `adb shell run-as` sentence, add: "Its network
key is `maya/identity.key` in the same directory."

- [ ] **Step 3: The spec's deviations**

In the spec, set `Status: implemented` and add a section before
"Testing":

```markdown
## Changes from this design

- The CLI's and the phone's private key is in `identity.key` beside their
  config, not in `config.json`, so it never reaches a page that reads or
  saves the config.
- A protocol 1 Maya reaching a protocol 2 main reads the main's existing
  `bye { reason: "protocol" }`: "The main Maya runs a different version of
  Maya."
- A protocol 1 main ignores binary frames and says nothing, so a protocol 2
  Maya reaching one shows "The main Maya did not answer. If it runs an
  older Maya, update it." ("The main runs an older Maya. Update it." shows
  when a main does speak plain text.)
- New key sets the role to Off as well as dropping the pairings.
- Keychain writes go through `security -i` on stdin, so a secret never
  appears on a command line. A Keychain item that `security` wrote is
  readable by any process that runs `security` as the user: the key store
  keeps the key out of files an agent might `cat`, not away from a process
  determined to read it.
```

- [ ] **Step 4: The version**

This is a breaking change before 1.0: bump the minor version from `main`'s.
Run `git fetch origin && git show origin/main:package.json | grep '"version"'`
to read it (0.21.0 at the time of writing, so 0.22.0), then:

```bash
sh scripts/set-version.sh 0.22.0
git diff --stat   # Cargo.lock must be among the changed files (see memory: set-version may leave it)
```

If `Cargo.lock` did not change, run `cargo update -w` and check again.

- [ ] **Step 5: Commit**

```bash
git add README.md docs/
git commit -m "docs: describe the secure protocol and record where it differs from the design"
git add package.json src-tauri/tauri.conf.json Cargo.toml Cargo.lock
git commit -m "chore(release): 0.22.0"
```

- [ ] **Step 6: Final check**

Run: `cargo test --workspace && pnpm test`
Expected: all pass.

Then the hand check the spec asks for (needs the user, the laptop and the
phone): pair each Maya, drive a session from each, Make main and back,
Remove, New key, and capture the traffic with
`sudo tcpdump -i any -A port 4127` while a reply is sent: no reply text,
session name or JSON may be readable.
```
