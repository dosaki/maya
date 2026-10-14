# The Phone Joins a Main — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A phone pairs with a running main Maya, shows the main's whole board with notifications, acts on any session through the main, and stays in the pool so Make main can promote it.

**Architecture:** Additive protocol messages: the main sends `mirror { allowed, cards, machines }` to a Maya that asked with the `mirror` cap and whose paired entry allows it, and answers that Maya's `request { id, machine?, request }` with `response`. The server hands each request to a new `Notify::request` on a worker thread. The desktop main answers it with the same local executor and routing its page uses; a phone main forwards it. On the phone, a `RequestLane` on the client carries the page's commands up, and the hub shows the mirrored cards as its board.

**Tech Stack:** Rust (maya_core, Tauri 2 desktop `src-tauri/`, Tauri 2 Android `mobile/`), tungstenite WebSockets, serde; vanilla TypeScript pages with vitest.

**Spec:** `docs/superpowers/specs/2026-10-14-phone-joins-a-main-design.md`

## Global Constraints

- The protocol stays at version 1 (`PROTOCOL`); every new field is `#[serde(default)]`, so an older Maya's frames still decode.
- The cap is the string `"mirror"` (`CAP_MIRROR`). The phone sends `follow`, `lead`, `mirror`; the desktop and `maya-cli` never send `mirror`.
- User-facing strings, exactly:
  - `"<main> has not allowed this Maya to control its sessions."` (request refused, server side)
  - `"That cannot be requested."` (a `promote` sent as a request)
  - `"This phone runs no sessions."` (unchanged)
  - `"Not connected to the main Maya."` and `"Lost the connection to the main Maya."` (the phone's request lane)
  - `"Sees and controls all sessions"` (the main's switch)
  - `"<main> has not allowed this phone to see its sessions. Turn on \"Sees and controls all sessions\" for <phone> in <main>'s Settings → Network."`
  - `"<main> runs an older Maya that cannot share its sessions. Update it."`
  - `"Connected to <main>"`, `"Reconnecting…"`, button `"Leave"`, section `"Join a main"`, button `"Join"`.
- Mirrors go out at most once every 300 ms per connection (`MIRROR_GAP`); a phone request waits at most 40 s (`REQUEST_TIMEOUT`).
- Lock order is unchanged: `Notify` is never called with the server's mutex held, except the two saves. Hub locks `client`, `link`, `mirror`, `notifier` are taken alone.
- The release is `0.21.0` (main is `0.20.0`; this PR has `feat` commits). Set it with `sh scripts/set-version.sh 0.21.0`, check `Cargo.lock` changed, commit as `chore(release): 0.21.0`.
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Never commit to `main`; work on `feat/phone-joins-main`.
- Rust tests run from the repo root: `cargo test -p maya-core`, `cargo test -p maya-mobile`, `cargo test -p maya`. Page tests: `pnpm test`. Type check: `pnpm exec tsc --noEmit`.

## Review Focus

1. The main goes away while a phone request waits: the request fails at once with "Lost the connection to the main Maya.", not after 40 s. Test in Task 3.
2. Two requests in flight at once each get their own answer (ids are matched, not order). Test in Task 3.
3. Joining with a wrong code on a phone that was the main: the phone's server comes back and its role stays Main. Test in Task 6.
4. A reconnect does not re-announce what is already waiting: the first mirror after each connect seeds the notifier. Test in Task 6.
5. The main's switch turned off while the phone shows its board: the board empties, no request gets through, and turning it on brings the board back. Tests in Tasks 2 and 6.

---

### Task 1: Protocol, config and status types

**Files:**
- Modify: `core/src/config.rs:17-31` (`PairedAssistant`)
- Modify: `core/src/net/protocol.rs` (cap, `Welcome.mirror`, `Up::Request`, `Request`, `Down::Mirror`, `Down::Response`)
- Modify: `core/src/net/routing.rs` (`MachineInfo` Deserialize, `Target`, `target_of`, `agent_of`)
- Modify: `core/src/net/mod.rs` (`AssistantStatus`, `AssistantLink`, `MirrorAccess`, `paired_offline`)
- Modify: `core/src/net/promote.rs:17-19` (`own_entry`)
- Modify (compile fixes only): `core/src/net/server.rs`, `core/src/net/client.rs`, `src-tauri/src/lib.rs`, `src-tauri/src/net_app.rs`, `mobile/src/hub.rs`
- Test: the `#[cfg(test)]` modules of `protocol.rs`, `routing.rs`, `promote.rs`, `config.rs`

**Interfaces:**
- Produces:
  - `pub const CAP_MIRROR: &str = "mirror";` in `protocol.rs`
  - `PairedAssistant.mirror: bool`
  - `Down::Welcome { name: String, mac: String, mirror: Option<bool> }`
  - `Up::Request { id: u64, machine: Option<String>, request: Request }`
  - `pub enum Request { Command(CommandKind), ListDirs, ListAgents }` (snake_case, externally tagged)
  - `Down::Mirror { allowed: bool, cards: Vec<Card>, machines: Vec<MachineInfo> }`
  - `Down::Response { id: u64, ok: bool, error: Option<String>, data: Option<Value> }`
  - `pub enum Target { Session(String), Machine }`, `pub fn target_of(kind: &CommandKind) -> Target`, `pub fn agent_of(kind: &CommandKind) -> Option<Harness>` in `routing.rs`
  - `AssistantStatus.mirror: bool`, `AssistantStatus.wants_mirror: bool` (JSON `mirror`, `wantsMirror`)
  - `pub enum MirrorAccess { Allowed, Refused, Unsupported }` (lowercase JSON), `AssistantLink.mirror: Option<MirrorAccess>`
  - `pub fn own_entry(main_name: &str, mirror: bool) -> PairedAssistant`

- [ ] **Step 1: Write the failing tests**

In `core/src/net/protocol.rs`'s test module add:

```rust
    #[test]
    fn requests_responses_and_mirrors_round_trip_and_older_frames_still_decode() {
        use crate::net::routing::MachineInfo;
        let req = Up::Request { id: 7, machine: Some("laptop".into()), request: Request::Command(CommandKind::Compact { session: "s1".into() }) };
        assert_eq!(decode_up(&encode(&req)).unwrap(), req);
        let dirs = Up::Request { id: 8, machine: None, request: Request::ListDirs };
        assert_eq!(decode_up(&encode(&dirs)).unwrap(), dirs);
        let resp = Down::Response { id: 7, ok: false, error: Some("no".into()), data: None };
        assert_eq!(decode_down(&encode(&resp)).unwrap(), resp);
        let mirror = Down::Mirror { allowed: true, cards: vec![], machines: vec![MachineInfo { name: "desk".into(), hostname: "desk".into(), platform: "macos".into(), connected: true }] };
        assert_eq!(decode_down(&encode(&mirror)).unwrap(), mirror);
        assert_eq!(decode_down(r#"{"type":"mirror","allowed":false}"#).unwrap(), Down::Mirror { allowed: false, cards: vec![], machines: vec![] });
        let offered = Down::Welcome { name: "desk".into(), mac: "cd".into(), mirror: Some(true) };
        assert_eq!(decode_down(&encode(&offered)).unwrap(), offered);
        assert!(!encode(&Down::Welcome { name: "desk".into(), mac: "cd".into(), mirror: None }).contains("mirror"), "no offer, no field");
        assert_eq!(decode_down(r#"{"type":"welcome","name":"desk","mac":"cd"}"#).unwrap(), Down::Welcome { name: "desk".into(), mac: "cd".into(), mirror: None });
    }
```

In the same module, change the existing line that builds `Down::Welcome { name: "desk".into(), mac: "cd".into() }` (around line 294) and the one comparing with `Down::Welcome { name: "desk".into(), mac: String::new() }` (around line 296) to add `, mirror: None`.

In `core/src/net/routing.rs`'s test module add:

```rust
    #[test]
    fn a_command_names_its_session_or_needs_a_machine_and_some_name_an_agent() {
        use crate::net::protocol::CommandKind;
        use crate::launch::LaunchOptions;
        assert_eq!(target_of(&CommandKind::Reply { session: "s1".into(), text: "hi".into(), attachments: vec![] }), Target::Session("s1".into()));
        assert_eq!(target_of(&CommandKind::History { session: "s2".into() }), Target::Session("s2".into()));
        assert_eq!(target_of(&CommandKind::ListResumable { dir: "/d".into(), agent: Harness::Codex }), Target::Machine);
        assert_eq!(agent_of(&CommandKind::ListResumable { dir: "/d".into(), agent: Harness::Codex }), Some(Harness::Codex));
        let options = LaunchOptions { agent: Harness::Kiro, ..Default::default() };
        assert_eq!(agent_of(&CommandKind::Start { dir: None, prompt: "go".into(), options }), Some(Harness::Kiro));
        assert_eq!(agent_of(&CommandKind::Compact { session: "s1".into() }), None);
    }
```

In `core/src/net/promote.rs`'s test module add:

```rust
    #[test]
    fn only_a_phone_s_own_entry_may_mirror() {
        assert!(own_entry("Pixel", true).mirror);
        assert!(!own_entry("desk", false).mirror);
    }
```

and change its existing `own_entry("desk")` calls (lines 72, 79, 114) to `own_entry("desk", false)`.

In `core/src/config.rs`'s test module (create `#[cfg(test)] mod tests { use super::*; … }` at the end of the file if there is none) add:

```rust
    #[test]
    fn a_paired_entry_saved_before_mirroring_existed_may_not_mirror() {
        let old = r#"{"id":"a","name":"laptop","hostname":"h","platform":"macos","token":"t"}"#;
        let a: PairedAssistant = serde_json::from_str(old).unwrap();
        assert!(!a.mirror);
    }
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core 2>&1 | tail -20`
Expected: compile errors: no `Up::Request`, `Request`, `Down::Mirror`, `Down::Response`, no field `mirror`, `target_of`, `agent_of`, `own_entry` takes 1 argument.

- [ ] **Step 3: Implement the types**

`core/src/config.rs`, inside `PairedAssistant` after `last_seen`:

```rust
    /// May see and control every session in the pool: set at pairing when
    /// the Maya asked (`CAP_MIRROR`), and by the main's switch.
    #[serde(default)]
    pub mirror: bool,
```

`core/src/net/protocol.rs`, after `CAP_LEAD`:

```rust
/// A Maya with this cap wants the main's board (`mirror`) and sends `request`s.
pub const CAP_MIRROR: &str = "mirror";
```

In `Up`, after `Result { … }`:

```rust
    /// Act on the pool through the main (a mirroring Maya only). `machine`
    /// names the target for kinds that need one; the main's own name, or
    /// none, means the main.
    Request { id: u64, #[serde(default)] machine: Option<String>, request: Request },
```

In `Down`, change `Welcome` and add two variants before `Pong`:

```rust
    /// `mac` is `mac(token, nonce)` for the assistant's `auth` nonce: the
    /// main's proof that it holds the token. Right after pairing it is
    /// `mac(code, nonce)` for the `pair` nonce: proof it knows the code.
    /// `mirror` answers a Maya that asked with `CAP_MIRROR`: whether the main
    /// will mirror to it. Absent for anyone else, and from an older main.
    Welcome { name: String, #[serde(default)] mac: String, #[serde(default, skip_serializing_if = "Option::is_none")] mirror: Option<bool> },
```

```rust
    /// The main's board for a mirroring Maya; `allowed: false` (and nothing
    /// else) once the main's switch for it is off.
    Mirror { allowed: bool, #[serde(default)] cards: Vec<Card>, #[serde(default)] machines: Vec<crate::net::routing::MachineInfo> },
    /// The answer to the `request` with this id.
    Response { id: u64, ok: bool, #[serde(default)] error: Option<String>, #[serde(default)] data: Option<Value> },
```

After `CommandKind`:

```rust
/// What a mirroring Maya asks of the main: a command on the pool (never
/// `Promote`), or a listing the main's own page reads locally.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Request {
    Command(CommandKind),
    ListDirs,
    ListAgents,
}
```

`core/src/net/routing.rs`: change `#[derive(Serialize, Clone, Debug, PartialEq)]` on `MachineInfo` to `#[derive(Serialize, serde::Deserialize, Clone, Debug, PartialEq)]`, and add after `machines_of`:

```rust
/// Where a command goes: the machine that lists its session, or the
/// machine the caller names (start, resume and the listings).
#[derive(Debug, PartialEq)]
pub enum Target {
    Session(String),
    Machine,
}

pub fn target_of(kind: &super::protocol::CommandKind) -> Target {
    use super::protocol::CommandKind::*;
    match kind {
        Reply { session, .. } | Answer { session, .. } | Compact { session } | Close { session } | Rename { session, .. } | SetOption { session, .. } | CycleMode { session } | Slash { session, .. } | History { session } => Target::Session(session.clone()),
        Start { .. } | Resume { .. } | ListResumable { .. } | Promote { .. } => Target::Machine,
    }
}

/// The agent a command starts or lists, for `check_remote_agent`.
pub fn agent_of(kind: &super::protocol::CommandKind) -> Option<Harness> {
    use super::protocol::CommandKind::*;
    match kind {
        Start { options, .. } => Some(options.agent),
        Resume { agent, .. } | ListResumable { agent, .. } => Some(*agent),
        _ => None,
    }
}
```

`core/src/net/mod.rs`: in `AssistantStatus` after `can_follow`:

```rust
    /// The main lets it see and control every session (its paired entry).
    pub mirror: bool,
    /// Connected and asked to mirror (`CAP_MIRROR`): the main shows its switch.
    pub wants_mirror: bool,
```

In `AssistantLink` after `retrying`:

```rust
    /// What the main said about mirroring at the last connect; `None` before.
    pub mirror: Option<MirrorAccess>,
```

and below it:

```rust
/// Whether the main shares its board with this Maya.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MirrorAccess {
    Allowed,
    /// The main's switch for this Maya is off.
    Refused,
    /// The main runs an older Maya that cannot mirror.
    Unsupported,
}
```

In `paired_offline`, add `mirror: p.mirror, wants_mirror: false` to the `AssistantStatus` literal.

`core/src/net/promote.rs`:

```rust
/// The entry the old main makes for itself in the list it hands over: it
/// connects to the new main with this id and token. Its address is filled
/// in by the new main at its first authentication. `mirror` is true for a
/// phone: the user pressed Make main on it.
pub fn own_entry(main_name: &str, mirror: bool) -> PairedAssistant {
    PairedAssistant { id: new_id(), name: main_name.into(), hostname: super::local_hostname(), platform: std::env::consts::OS.into(), token: new_token(), address: String::new(), last_seen: None, mirror }
}
```

- [ ] **Step 4: Fix the compile errors elsewhere**

Run `cargo build --workspace 2>&1 | grep -E "^error" -A6 | head -80` and fix each site. These are the expected ones; behaviour comes in later tasks:
- Every `PairedAssistant { … }` literal (in `server.rs`, `config.rs`, `mod.rs`, `src-tauri/src/lib.rs`, `mobile/src/hub.rs` and their tests): add `mirror: false`.
- Every `AssistantStatus { … }` literal (`server.rs:277`, `routing.rs:177-178`): add `mirror: false, wants_mirror: false` (`server.rs` gets its real values in Task 2).
- Every full `AssistantLink { connected: true, main_name: Some(name), error: None, retrying: false }` (`src-tauri/src/net_app.rs:104`, `mobile/src/hub.rs:412`): add `mirror: None`.
- `server.rs:702`: `Down::Welcome { name: ctx.main_name.clone(), mac: proof, mirror: None }`. Test fakes at `server.rs:2100` and `2124`: add `mirror: None`.
- `server.rs:1129`: `super::promote::own_entry(&self.ctx.main_name, false)`.
- `server.rs` `handle()`: add an arm before `Up::Hello { .. } | …`:
  ```rust
        Up::Request { id, .. } => send_down(ws, &Down::Response { id, ok: false, error: Some("This Maya cannot answer requests.".into()), data: None })?,
  ```
- `client.rs:373`: `Down::Welcome { name, mac: proof, .. } => {`.
- `client.rs` `handle()`: add an arm before the handshake arm:
  ```rust
        Down::Mirror { .. } | Down::Response { .. } => log::line("network", "main: ignored a mirror message"),
  ```
- `server.rs` `kind_name` already covers every `CommandKind`; nothing to do.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p maya-core 2>&1 | tail -5` and `cargo build --workspace 2>&1 | tail -3`
Expected: all pass; the workspace builds.

- [ ] **Step 6: Commit**

```bash
git add core src-tauri mobile
git commit -m "feat: protocol and status types for mirroring a main's board

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: The server mirrors, answers requests and keeps the switch

**Files:**
- Modify: `core/src/net/server.rs`
- Test: `core/src/net/server.rs` test module

**Interfaces:**
- Consumes: everything Task 1 produces.
- Produces:
  - `Notify::request(&self, machine: Option<&str>, request: Request) -> Result<Option<Value>, String>` (default: `Err("This Maya cannot answer requests.")`); `machine` is `None` for the main itself.
  - `ServerHandle::mirror(&self, cards: Vec<Card>)`: the main's merged board, local cards with `machine: None`.
  - `ServerHandle::set_mirror(&self, id: &str, on: bool) -> Result<(), String>`
  - `ServerHandle::set_own_mirror(&self, on: bool)`: the entry this main makes for itself at a handover may mirror (a phone sets it).
  - Pairing saves `mirror: true` when the pairing `hello` carried `mirror`; `welcome.mirror` is `Some(entry.mirror)` for a Maya that asked, else `None`.
  - Status: `mirror` = the entry's flag, `wants_mirror` = connected and asked.

- [ ] **Step 1: Write the failing tests**

In the test module, give `Counter` a request log. Add the field `pub requests: Mutex<Vec<(Option<String>, Request)>>,` to `Counter` and this method to its `impl Notify for Counter`:

```rust
        fn request(&self, machine: Option<&str>, request: Request) -> Result<Option<Value>, String> {
            if let Request::Command(CommandKind::Compact { session }) = &request {
                if session == "slow" {
                    std::thread::sleep(Duration::from_millis(800));
                }
            }
            self.requests.lock().unwrap().push((machine.map(str::to_string), request));
            Ok(Some(serde_json::json!("done")))
        }
```

Add `Request` to the test module's `use crate::net::protocol::{…}` line.

Replace `pair_client` with a version that takes caps, keeping the old name as a wrapper:

```rust
    /// Pairs a new assistant whose `hello` carries `caps`; returns its open
    /// socket, id, token and the welcome's mirror offer.
    pub(crate) fn pair_client_caps(handle: &ServerHandle, port: u16, name: &str, caps: &[&str]) -> (Client, String, String, Option<bool>) {
        let (code, _) = handle.open_pairing(crate::store::now_ms()).unwrap();
        let mut ws = connect(port);
        hello_caps(&mut ws, name, None, caps);
        let nonce = crate::net::protocol::new_nonce();
        send(&mut ws, &Up::Pair { code: code.clone(), nonce: nonce.clone() });
        let Down::Paired { id, token } = recv(&mut ws) else { panic!("expected paired") };
        let Down::Welcome { mac: proof, mirror, .. } = recv(&mut ws) else { panic!("expected welcome") };
        assert!(crate::net::protocol::mac_matches(&code, &nonce, &proof), "the main proves it knows the code");
        (ws, id, token, mirror)
    }

    /// Pairs a new assistant; returns its open socket, id and token.
    pub(crate) fn pair_client(handle: &ServerHandle, port: u16, name: &str) -> (Client, String, String) {
        let (ws, id, token, _) = pair_client_caps(handle, port, name, &[]);
        (ws, id, token)
    }
```

Then add these tests:

```rust
    #[test]
    fn a_maya_that_asks_to_mirror_is_allowed_at_pairing_and_told_so_in_welcome() {
        let counter = Arc::new(Counter::default());
        let (handle, port) = test_server_with(counter.clone());
        let (_phone, phone_id, _, offer) = pair_client_caps(&handle, port, "Pixel", &["follow", "lead", "mirror"]);
        assert_eq!(offer, Some(true));
        let (_laptop, laptop_id, _, offer) = pair_client_caps(&handle, port, "laptop", &["follow", "lead"]);
        assert_eq!(offer, None, "a Maya that did not ask is told nothing");
        wait_until(|| handle.status().assistants.iter().filter(|a| a.connected).count() == 2);
        let status = handle.status();
        let phone = status.assistants.iter().find(|a| a.id == phone_id).unwrap();
        assert!(phone.mirror && phone.wants_mirror);
        let laptop = status.assistants.iter().find(|a| a.id == laptop_id).unwrap();
        assert!(!laptop.mirror && !laptop.wants_mirror);
        assert!(counter.saved.lock().unwrap().iter().any(|a| a.id == phone_id && a.mirror), "the permission is saved");
        handle.stop();
    }

    #[test]
    fn the_main_mirrors_its_board_with_its_own_cards_named_and_the_receiver_left_out() {
        let (handle, port) = test_server();
        let (mut phone, _, _, _) = pair_client_caps(&handle, port, "Pixel", &["mirror"]);
        let (_laptop, _, _) = pair_client(&handle, port, "laptop");
        wait_until(|| handle.status().assistants.iter().filter(|a| a.connected).count() == 2);
        let mut remote = card("r1", "fix");
        remote.machine = Some("laptop".into());
        handle.mirror(vec![card("l1", "local"), remote]);
        let Down::Mirror { allowed, cards, machines } = recv(&mut phone) else { panic!("expected a mirror") };
        let main = main_name_of(&NetworkConfig::default());
        assert!(allowed);
        assert_eq!(cards[0].machine.as_deref(), Some(main.as_str()), "the main's own cards carry its name");
        assert_eq!(cards[1].machine.as_deref(), Some("laptop"));
        assert_eq!(machines[0].name, main);
        assert!(machines[0].connected);
        assert!(machines.iter().any(|m| m.name == "laptop"));
        assert!(!machines.iter().any(|m| m.name == "Pixel"), "the receiver is not a machine to pick");
        handle.stop();
    }

    #[test]
    fn turning_the_switch_off_tells_the_maya_and_refuses_its_requests_and_on_mirrors_again() {
        let counter = Arc::new(Counter::default());
        let (handle, port) = test_server_with(counter.clone());
        let (mut phone, id, _, _) = pair_client_caps(&handle, port, "Pixel", &["mirror"]);
        wait_until(|| handle.status().assistants.iter().any(|a| a.id == id && a.connected));
        handle.set_mirror(&id, false).unwrap();
        assert!(!counter.saved.lock().unwrap().iter().find(|a| a.id == id).unwrap().mirror, "the switch is saved");
        assert_eq!(recv(&mut phone), Down::Mirror { allowed: false, cards: vec![], machines: vec![] });
        send(&mut phone, &Up::Request { id: 1, machine: None, request: Request::Command(CommandKind::Compact { session: "s1".into() }) });
        let Down::Response { id: 1, ok: false, error: Some(e), .. } = recv(&mut phone) else { panic!("expected a refusal") };
        assert!(e.ends_with("has not allowed this Maya to control its sessions."), "{e}");
        assert!(counter.requests.lock().unwrap().is_empty());
        handle.mirror(vec![card("l1", "local")]);
        handle.set_mirror(&id, true).unwrap();
        assert!(matches!(recv(&mut phone), Down::Mirror { allowed: true, .. }));
        assert_eq!(handle.set_mirror("nobody", true).unwrap_err(), "That Maya is no longer paired.");
        handle.stop();
    }

    #[test]
    fn an_allowed_maya_s_requests_reach_the_app_off_the_connection_thread() {
        let counter = Arc::new(Counter::default());
        let (handle, port) = test_server_with(counter.clone());
        let (mut phone, _, _, _) = pair_client_caps(&handle, port, "Pixel", &["mirror"]);
        let main = main_name_of(&NetworkConfig::default());
        send(&mut phone, &Up::Request { id: 1, machine: None, request: Request::Command(CommandKind::Compact { session: "slow".into() }) });
        send(&mut phone, &Up::Ping);
        assert_eq!(recv(&mut phone), Down::Pong, "a slow request does not hold up the connection");
        assert_eq!(recv(&mut phone), Down::Response { id: 1, ok: true, error: None, data: Some(serde_json::json!("done")) });
        send(&mut phone, &Up::Request { id: 2, machine: Some(main.clone()), request: Request::ListDirs });
        assert!(matches!(recv(&mut phone), Down::Response { id: 2, ok: true, .. }));
        send(&mut phone, &Up::Request { id: 3, machine: Some("laptop".into()), request: Request::ListAgents });
        assert!(matches!(recv(&mut phone), Down::Response { id: 3, ok: true, .. }));
        let seen: Vec<Option<String>> = counter.requests.lock().unwrap().iter().map(|(m, _)| m.clone()).collect();
        assert_eq!(seen, [None, None, Some("laptop".to_string())], "the main's own name means the main");
        send(&mut phone, &Up::Request { id: 4, machine: None, request: Request::Command(CommandKind::Promote { sealed: "x".into() }) });
        assert_eq!(recv(&mut phone), Down::Response { id: 4, ok: false, error: Some("That cannot be requested.".into()), data: None });
        handle.stop();
    }

    #[test]
    fn a_phone_main_hands_over_an_entry_for_itself_that_may_mirror() {
        let (handle, port) = test_server();
        handle.set_own_mirror(true);
        let (mut g, g_id, g_token) = pair_with_caps(&handle, port, "laptop", &["follow", "lead"]);
        let h = handle.clone();
        let t = std::thread::spawn(move || h.promote(&g_id));
        let Down::Command { id: cmd, kind: CommandKind::Promote { sealed } } = recv(&mut g) else { panic!("expected promote") };
        let list = crate::net::promote::open_list(&g_token, &sealed).unwrap();
        assert!(list.last().unwrap().mirror);
        send(&mut g, &Up::Result { id: cmd, ok: true, error: None, data: Some(serde_json::json!({ "port": 5001 })) });
        assert!(t.join().unwrap().unwrap().me.mirror);
        handle.stop();
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p maya-core net::server 2>&1 | tail -20`
Expected: compile errors: no `mirror`, `set_mirror` or `set_own_mirror` on `ServerHandle`, no `request` in `Notify`.

- [ ] **Step 3: Implement**

Imports: add `Request, CAP_MIRROR` to the `use super::protocol::{…}` line, and `use super::routing::MachineInfo;` and `use crate::model::Card;`.

Constants, beside `TICK`:

```rust
/// Mirrors go out at most this often per connection; the last change still goes.
const MIRROR_GAP: Duration = Duration::from_millis(300);
```

`Notify`, after `paired_list_changed`:

```rust
    /// A mirroring Maya's request, run on a worker thread with no lock held.
    /// `machine` is `None` for this main itself. The result is what this
    /// main's own page would get for the same action.
    fn request(&self, _machine: Option<&str>, _request: Request) -> Result<Option<Value>, String> {
        Err("This Maya cannot answer requests.".into())
    }
```

`Server`: add fields

```rust
    /// The main's last board for mirroring Mayas, newest first by serial.
    mirror: Option<MirrorBoard>,
    mirror_serial: u64,
```

initialise them in `Server::new` as `mirror: None, mirror_serial: 0`, and add above `impl Server`:

```rust
/// The main's board as its page shows it; local cards have no `machine`.
struct MirrorBoard {
    serial: u64,
    cards: Vec<Card>,
}
```

Add to `impl Server`:

```rust
    /// The entry `id` lets its Maya mirror.
    fn may_mirror(&self, id: &str) -> bool {
        self.paired.iter().any(|p| p.id == id && p.mirror)
    }

    /// The machines a mirroring Maya can pick: this main first, then every
    /// paired assistant but `receiver`.
    fn mirror_machines(&self, main_name: &str, receiver: &str) -> Vec<MachineInfo> {
        let me = MachineInfo { name: main_name.into(), hostname: super::local_hostname(), platform: std::env::consts::OS.into(), connected: true };
        let others = self.paired.iter().zip(self.labels()).filter(|(p, _)| p.id != receiver).map(|(p, (id, label))| MachineInfo { name: label, hostname: p.hostname.clone(), platform: p.platform.clone(), connected: self.conns.contains_key(&id) });
        std::iter::once(me).chain(others).collect()
    }

    /// Makes the next tick send the board again to every mirroring Maya.
    fn bump_mirror(&mut self) {
        if let Some(board) = self.mirror.take() {
            self.mirror_serial += 1;
            self.mirror = Some(MirrorBoard { serial: self.mirror_serial, cards: board.cards });
        }
    }
```

`record_pairing` gets a `mirror: bool` parameter: the new entry's literal gets `mirror`, and the "same machine again" branch sets `p.mirror = mirror;` beside `p.platform = …`. `pair()` gets `mirror: bool` after `platform` and passes it on. In `admit`, before the `match id`:

```rust
    let wants_mirror = caps.iter().any(|c| c == CAP_MIRROR);
```

and the `None =>` arm calls `pair(ctx, ws, &ip, &name, &hostname, &platform, wants_mirror, deadline)`.

In `admit`'s registered block, the `Ok(())` arm returns the offer too: change its type to `Result<(u64, String, Option<bool>), Option<String>>`, and its last line to:

```rust
                    let offer = wants_mirror.then(|| s.may_mirror(&id));
                    Ok((serial, s.label(&id), offer))
```

then `let (serial, label, offer) = match registered { … }` and the welcome becomes `Down::Welcome { name: ctx.main_name.clone(), mac: proof, mirror: offer }`.

`Link` gains three fields, set in `admit` as `wants_mirror, mirror_sent: Cell::new(0), mirror_at: Cell::new(None)`:

```rust
    /// Its `hello` asked for the main's board.
    wants_mirror: bool,
    /// The serial of the last mirror sent on this connection, and when.
    mirror_sent: std::cell::Cell<u64>,
    mirror_at: std::cell::Cell<Option<Instant>>,
```

Add after `send_queued`:

```rust
/// The mirror this connection is owed, if any: a newer board than the last
/// one sent, no sooner than `MIRROR_GAP` after it, to a Maya allowed it.
fn mirror_due(ctx: &Ctx, link: &Link) -> Option<Down> {
    if !link.wants_mirror || link.mirror_at.get().is_some_and(|t| t.elapsed() < MIRROR_GAP) {
        return None;
    }
    let s = lock(&ctx.shared);
    let board = s.mirror.as_ref().filter(|b| b.serial != link.mirror_sent.get())?;
    if !s.may_mirror(&link.id) {
        return None;
    }
    let cards = board
        .cards
        .iter()
        .filter(|c| c.machine.as_deref() != Some(link.label.as_str()))
        .map(|c| {
            let mut c = c.clone();
            if c.machine.is_none() {
                c.machine = Some(ctx.main_name.clone());
                c.machine_platform = Some(std::env::consts::OS.into());
            }
            c
        })
        .collect();
    link.mirror_sent.set(board.serial);
    link.mirror_at.set(Some(Instant::now()));
    Some(Down::Mirror { allowed: true, cards, machines: s.mirror_machines(&ctx.main_name, &link.id) })
}
```

In `pump`, right after the `send_queued` check:

```rust
        if let Some(mirror) = mirror_due(ctx, link) {
            if let Err(e) = send_down(ws, &mirror) {
                return e;
            }
        }
```

In `handle`, replace the Task 1 placeholder arm with:

```rust
        Up::Request { id, machine, request } => {
            let (allowed, tx) = {
                let mut s = lock(&ctx.shared);
                s.touch(&link.id);
                let tx = s.conns.get(&link.id).filter(|c| c.serial == link.serial).map(|c| c.tx.clone());
                (link.wants_mirror && s.may_mirror(&link.id), tx)
            };
            let Some(tx) = tx else { return Err(NOT_CURRENT.into()) };
            let refusal = if !allowed {
                Some(format!("{} has not allowed this Maya to control its sessions.", ctx.main_name))
            } else if matches!(request, Request::Command(CommandKind::Promote { .. })) {
                Some("That cannot be requested.".to_string())
            } else {
                None
            };
            if let Some(error) = refusal {
                send_down(ws, &Down::Response { id, ok: false, error: Some(error), data: None })?;
                return Ok(());
            }
            // The main's own name, like none, means the main.
            let machine = machine.map(|m| clean(&m)).filter(|m| !m.is_empty() && *m != ctx.main_name);
            let (notify, label) = (ctx.notify.clone(), link.label.clone());
            let spawned = std::thread::Builder::new().name("net-request".into()).spawn(move || {
                let down = match notify.request(machine.as_deref(), request) {
                    Ok(data) => Down::Response { id, ok: true, error: None, data },
                    Err(e) => {
                        log::line("network", format!("{label}: request {id} failed: {e}"));
                        Down::Response { id, ok: false, error: Some(e), data: None }
                    }
                };
                let _ = tx.send(down);
            });
            if let Err(e) = spawned {
                send_down(ws, &Down::Response { id, ok: false, error: Some(format!("no thread for the request: {e}")), data: None })?;
            }
        }
```

`Ctx` gains `own_mirror: AtomicBool`, initialised `AtomicBool::new(false)` in `start_with`. In `promote_with_timeout` use `super::promote::own_entry(&self.ctx.main_name, self.ctx.own_mirror.load(Ordering::SeqCst))`.

`Server::status`: the `AssistantStatus` literal gets `mirror: p.mirror, wants_mirror: can(CAP_MIRROR)`.

`ServerHandle`, beside `boards()`:

```rust
    /// The main's board as its page shows it (local cards with no machine),
    /// for every mirroring Maya; each connection sends it on its next tick.
    pub fn mirror(&self, cards: Vec<Card>) {
        let mut s = lock(&self.shared);
        s.mirror_serial += 1;
        s.mirror = Some(MirrorBoard { serial: s.mirror_serial, cards });
    }

    /// Lets the Maya `id` see and control every session, or stops it. Saved
    /// first; if that fails nothing changes. Off tells a connected Maya at
    /// once; on sends it the board again.
    pub fn set_mirror(&self, id: &str, on: bool) -> Result<(), String> {
        let (label, status) = {
            let mut s = lock(&self.shared);
            let mut after = s.paired.clone();
            let entry = after.iter_mut().find(|p| p.id == id).ok_or("That Maya is no longer paired.")?;
            entry.mirror = on;
            self.ctx.notify.paired_list_changed(&after)?;
            s.paired = after;
            if on {
                s.bump_mirror();
            } else if let Some(c) = s.conns.get(id) {
                let _ = c.tx.send(Down::Mirror { allowed: false, cards: vec![], machines: vec![] });
            }
            (s.label(id), s.status(now_ms()))
        };
        log::line("network", format!("{label}: {}", if on { "may see and control every session" } else { "no longer sees the sessions" }));
        self.ctx.notify.status_changed(status);
        // The app refreshes and mirrors the board (it may hold none yet).
        if on {
            self.ctx.notify.board_changed();
        }
        Ok(())
    }

    /// The entry this main makes for itself at a handover may mirror: a
    /// phone, where the user pressed Make main.
    pub fn set_own_mirror(&self, on: bool) {
        self.ctx.own_mirror.store(on, Ordering::SeqCst);
    }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p maya-core 2>&1 | tail -5`
Expected: all pass, the existing promote and pairing tests included.

- [ ] **Step 5: Commit**

```bash
git add core/src/net/server.rs
git commit -m "feat: the main mirrors its board and answers requests from an allowed Maya

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: The client's request lane and mirror callbacks

**Files:**
- Modify: `core/src/net/client.rs`
- Modify: `src-tauri/src/net_app.rs:65`, `cli/src/commands.rs:64`, `mobile/tests/round_trip.rs:140,234,323` (the new `pair_with` argument)
- Test: `core/src/net/client.rs` test module (`check_pair_args`), `core/src/net/server.rs` test module (client against a server)

**Interfaces:**
- Consumes: Task 1's messages; Task 2's server (`mirror`, `Notify::request`).
- Produces:
  - `pub fn pair_with(exec, host, port, name, code, caps: Vec<String>) -> Result<(String, String, String), String>`
  - `pub fn check_pair_args(host: &str, code: &str) -> Result<(String, String), String>`
  - `pub struct RequestLane` with `pub fn request(&self, machine: Option<String>, request: Request, timeout: Duration) -> Result<Option<Value>, String>`
  - `pub const NOT_CONNECTED: &str`, `pub const LOST: &str`, `pub const REQUEST_TIMEOUT: Duration` (40 s)
  - `ClientNotify` gains, with no-op defaults: `fn mirror_offer(&self, offered: Option<bool>)` (called right after `connected`), `fn mirrored(&self, allowed: bool, cards: Vec<Card>, machines: Vec<MachineInfo>)`, `fn lane(&self) -> Option<Arc<RequestLane>>`

- [ ] **Step 1: Write the failing tests**

In `client.rs`'s test module:

```rust
    #[test]
    fn a_pairing_form_needs_a_host_and_six_digits() {
        assert_eq!(check_pair_args(" desk.local ", "483 921"), Ok(("desk.local".into(), "483921".into())));
        assert_eq!(check_pair_args("", "483921").unwrap_err(), "Type the main Maya's host name or address.");
        assert_eq!(check_pair_args("http://desk", "483921").unwrap_err(), "Type the main Maya's host name or address.");
        assert_eq!(check_pair_args("desk", "48392").unwrap_err(), "The pairing code has six digits.");
    }
```

In `server.rs`'s test module, beside the other client tests:

```rust
    #[derive(Default)]
    struct Mirrorer {
        lane: Arc<client::RequestLane>,
        offers: Mutex<Vec<Option<bool>>>,
        mirrors: Mutex<Vec<(bool, Vec<Card>)>>,
    }

    impl client::ClientNotify for Mirrorer {
        fn paired(&self, _: &str, _: &str) {}
        fn connected(&self, _: &str) {}
        fn disconnected(&self, _: &str) {}
        fn removed(&self) {}
        fn caps(&self) -> Vec<String> {
            vec!["mirror".into()]
        }
        fn mirror_offer(&self, offered: Option<bool>) {
            self.offers.lock().unwrap().push(offered);
        }
        fn mirrored(&self, allowed: bool, cards: Vec<Card>, _: Vec<MachineInfo>) {
            self.mirrors.lock().unwrap().push((allowed, cards));
        }
        fn lane(&self) -> Option<Arc<client::RequestLane>> {
            Some(self.lane.clone())
        }
    }

    /// Pairs a mirroring client with `handle` and runs it until `stop`.
    fn run_mirrorer(handle: &ServerHandle, port: u16) -> (Arc<Mirrorer>, Arc<AtomicBool>, std::thread::JoinHandle<()>) {
        let (code, _) = handle.open_pairing(crate::store::now_ms()).unwrap();
        let (_, id, token) = client::pair_with(Arc::new(NoSessions), "127.0.0.1", port, "Pixel", &code, vec!["mirror".into()]).unwrap();
        let m = Arc::new(Mirrorer::default());
        let stop = Arc::new(AtomicBool::new(false));
        let config = assistant_config(port, &id, &token);
        let (n, s) = (m.clone(), stop.clone());
        let t = std::thread::spawn(move || client::reconnect(&move || config.clone(), Arc::new(NoSessions), n, &s));
        wait_until(|| m.offers.lock().unwrap().as_slice() == [Some(true)]);
        (m, stop, t)
    }

    #[test]
    fn a_mirroring_client_hears_the_offer_gets_the_board_and_its_requests_answered() {
        let counter = Arc::new(Counter::default());
        let (handle, port) = test_server_with(counter.clone());
        let (m, stop, t) = run_mirrorer(&handle, port);
        handle.mirror(vec![card("l1", "local")]);
        wait_until(|| m.mirrors.lock().unwrap().iter().any(|(ok, cards)| *ok && cards.iter().any(|c| c.session_id == "l1")));
        let lane = m.lane.clone();
        let slow = {
            let l = lane.clone();
            std::thread::spawn(move || l.request(None, Request::Command(CommandKind::Compact { session: "slow".into() }), Duration::from_secs(5)))
        };
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(lane.request(Some("laptop".into()), Request::ListAgents, Duration::from_secs(5)), Ok(Some(serde_json::json!("done"))));
        assert_eq!(slow.join().unwrap(), Ok(Some(serde_json::json!("done"))), "each answer reaches its own caller");
        stop.store(true, Ordering::SeqCst);
        t.join().unwrap();
        assert_eq!(lane.request(None, Request::ListDirs, Duration::from_secs(1)).unwrap_err(), client::NOT_CONNECTED);
        handle.stop();
    }

    #[test]
    fn a_request_waiting_when_the_main_goes_away_fails_at_once() {
        let (handle, port) = test_server();
        let (m, stop, t) = run_mirrorer(&handle, port);
        let lane = m.lane.clone();
        let began = Instant::now();
        let waiting = std::thread::spawn(move || lane.request(None, Request::Command(CommandKind::Compact { session: "slow".into() }), Duration::from_secs(30)));
        std::thread::sleep(Duration::from_millis(100));
        handle.stop();
        assert_eq!(waiting.join().unwrap().unwrap_err(), client::LOST);
        assert!(began.elapsed() < Duration::from_secs(5), "not left to time out");
        stop.store(true, Ordering::SeqCst);
        t.join().unwrap();
    }
```

(`NoSessions` and `assistant_config` already exist in this test module; add `use crate::net::routing::MachineInfo;` if the module lacks it.)

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p maya-core 2>&1 | tail -20`
Expected: compile errors: no `RequestLane`, `check_pair_args`, `mirror_offer`, `pair_with` takes 5 arguments.

- [ ] **Step 3: Implement**

Imports in `client.rs`: add `Request` to the `protocol` use list, and `use super::routing::MachineInfo;`, `use std::collections::HashMap;`, `use std::sync::mpsc::Receiver;` only if needed.

Constants:

```rust
/// What a request reads when no connection to the main is up.
pub const NOT_CONNECTED: &str = "Not connected to the main Maya.";
/// What a waiting request reads when its connection drops.
pub const LOST: &str = "Lost the connection to the main Maya.";
/// How long a request waits: the main itself waits up to 30 s on an assistant.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(40);
```

`ClientNotify`, after `moved`:

```rust
    /// What the main said at `welcome` about mirroring (`Some(true)`: it
    /// will; `Some(false)`: its switch is off; `None`: it did not say).
    /// Called right after `connected`.
    fn mirror_offer(&self, _offered: Option<bool>) {}
    /// A board from the main, or `allowed: false` once its switch went off.
    fn mirrored(&self, _allowed: bool, _cards: Vec<Card>, _machines: Vec<MachineInfo>) {}
    /// Where this Maya's requests to the main go out; `None` sends none.
    fn lane(&self) -> Option<Arc<RequestLane>> {
        None
    }
```

The lane, after `ClientNotify`:

```rust
/// Requests to the main from this Maya's page, matched to their answers by
/// id. Each connection attaches its outbox while it runs; when it ends,
/// every request still waiting fails with `LOST`.
#[derive(Default)]
pub struct RequestLane {
    inner: Mutex<Lane>,
}

#[derive(Default)]
struct Lane {
    out: Option<Sender<Up>>,
    /// Which connection attached `out`, so an older one cannot detach a newer.
    generation: u64,
    next: u64,
    waiting: HashMap<u64, Sender<Result<Option<Value>, String>>>,
}

impl RequestLane {
    /// Sends `request` to the main and waits up to `timeout` for its answer.
    pub fn request(&self, machine: Option<String>, request: Request, timeout: Duration) -> Result<Option<Value>, String> {
        let (id, rx) = {
            let mut l = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let out = l.out.clone().ok_or(NOT_CONNECTED)?;
            l.next += 1;
            let id = l.next;
            let (tx, rx) = channel();
            l.waiting.insert(id, tx);
            if out.send(Up::Request { id, machine, request }).is_err() {
                l.waiting.remove(&id);
                return Err(NOT_CONNECTED.into());
            }
            (id, rx)
        };
        match rx.recv_timeout(timeout) {
            Ok(r) => r,
            Err(_) => {
                self.inner.lock().unwrap_or_else(|e| e.into_inner()).waiting.remove(&id);
                Err("The main Maya did not answer in time.".into())
            }
        }
    }

    fn attach(&self, out: Sender<Up>) -> u64 {
        let mut l = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        l.generation += 1;
        l.out = Some(out);
        l.generation
    }

    fn detach(&self, generation: u64) {
        let mut l = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if l.generation != generation {
            return;
        }
        l.out = None;
        for (_, w) in l.waiting.drain() {
            let _ = w.send(Err(LOST.into()));
        }
    }

    fn resolve(&self, id: u64, result: Result<Option<Value>, String>) {
        let waiter = self.inner.lock().unwrap_or_else(|e| e.into_inner()).waiting.remove(&id);
        if let Some(w) = waiter {
            let _ = w.send(result);
        }
    }
}

/// Detaches the lane when the connection's pump ends, however it ends.
struct Attached(Option<(Arc<RequestLane>, u64)>);

impl Drop for Attached {
    fn drop(&mut self) {
        if let Some((lane, generation)) = &self.0 {
            lane.detach(*generation);
        }
    }
}
```

`check_pair_args`, after `display_name`:

```rust
/// The host and code a pairing form gave, checked: a host with no spaces
/// or slashes, and six digits (spaces dropped).
pub fn check_pair_args(host: &str, code: &str) -> Result<(String, String), String> {
    let host = host.trim();
    if host.is_empty() || host.chars().any(|c| c.is_whitespace() || c == '/') {
        return Err("Type the main Maya's host name or address.".into());
    }
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return Err("The pairing code has six digits.".into());
    }
    Ok((host.to_string(), code))
}
```

`handshake` returns `Result<(String, Option<bool>), String>`: its `Down::Welcome { name, mac: proof, mirror }` arm ends with `return Ok((if name.is_empty() { "the main Maya".into() } else { name }, mirror));`. In `connection`:

```rust
    let out = handshake(&mut ws, config, &**notify, stop, code, deadline).and_then(|(main, mirror)| {
        log::line("network", format!("connected to {main}"));
        notify.connected(&main);
        notify.mirror_offer(mirror);
        pump(&mut ws, exec, notify, &config.token, stop)
    });
```

In `pump`, after `let (tx, rx) = channel::<Up>();`:

```rust
    let lane = notify.lane();
    let _attached = Attached(lane.as_ref().map(|l| (l.clone(), l.attach(tx.clone()))));
```

and pass `&lane` to `handle`: `handle(down, exec, notify, &lane, token, &tx, &leaving)`. In `handle` add the parameter `lane: &Option<Arc<RequestLane>>` and replace the Task 1 placeholder arm with:

```rust
        Down::Mirror { allowed, cards, machines } => notify.mirrored(allowed, cards, machines),
        Down::Response { id, ok, error, data } => match lane {
            Some(l) => l.resolve(id, if ok { Ok(data) } else { Err(error.unwrap_or_else(|| "The request failed.".into())) }),
            None => log::line("network", format!("main: response {id} with no request lane")),
        },
```

`pair_with` takes `caps: Vec<String>` last, and `PairNotify` keeps them:

```rust
pub fn pair_with(exec: Arc<dyn Executor>, host: &str, port: u16, name: &str, code: &str, caps: Vec<String>) -> Result<(String, String, String), String> {
    …
    let notify = Arc::new(PairNotify { stop: stop.clone(), creds: Mutex::new(None), main: Mutex::new(None), caps });
```

with `caps: Vec<String>,` in `PairNotify` and in its impl:

```rust
    fn caps(&self) -> Vec<String> {
        self.caps.clone()
    }
```

Update the callers: `src-tauri/src/net_app.rs:65` and `cli/src/commands.rs:64` pass `vec![]` (as today: they ask for nothing at pairing); the three `pair_with(…)` calls in `mobile/tests/round_trip.rs` pass `vec![]`.

In `src-tauri/src/lib.rs` `network_pair`, replace the host and code checks with:

```rust
    let (host, code) = net::client::check_pair_args(&host, &code)?;
    let host = host.as_str();
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p maya-core 2>&1 | tail -5`, then `cargo build --workspace 2>&1 | tail -3`
Expected: all pass; the workspace builds.

- [ ] **Step 5: Commit**

```bash
git add core cli src-tauri mobile/tests/round_trip.rs
git commit -m "feat: a client can ask the main for its board and send it requests

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: The desktop main serves mirrors, requests and the switch

**Files:**
- Modify: `src-tauri/src/lib.rs` (`TauriNetNotify::request`, `refresh_and_emit`, `network_set_mirror`, visibility of `remote_machine_of` and `remote_boards`, the handler list in `run`)
- Modify: `src-tauri/src/net_app.rs` (`serve_request`)

**Interfaces:**
- Consumes: `Notify::request`, `ServerHandle::mirror`, `ServerHandle::set_mirror` (Task 2); `target_of`, `agent_of`, `Request` (Task 1); `net_app::execute` and `net_app::send_command` (existing).
- Produces: Tauri command `network_set_mirror(id: String, on: bool) -> Result<NetworkStatus, String>`, used by Task 7.

- [ ] **Step 1: Implement `serve_request`**

In `src-tauri/src/net_app.rs` add the imports `use crate::net::protocol::Request;`, `use maya_core::net::routing::{agent_of, agents_reply, check_remote_agent, target_of, Target, ROUTE_TIMEOUT};` and `use crate::net::merge;`, then:

```rust
/// Answers a mirroring Maya's request as this Maya's own page would:
/// a session's command goes to whichever machine lists it, a machine's
/// command to the machine named (`None` is this Mac), and the listings
/// come from the same places the page reads.
pub fn serve_request(app: &AppHandle, machine: Option<&str>, request: Request) -> Result<Option<Value>, String> {
    let state = app.state::<AppState>();
    match request {
        Request::ListDirs => match machine {
            Some(m) => to_data(merge::dirs_of(&crate::remote_boards(&state), m)),
            None => to_data(actions::list_project_dirs(&crate::local(&state))?),
        },
        Request::ListAgents => match machine {
            Some(m) => to_data(agents_reply(false, merge::agents_of(&crate::remote_boards(&state), m), Vec::new)),
            None => to_data(agents_reply(true, None, maya_core::agents::current)),
        },
        Request::Command(kind) => match target_of(&kind) {
            Target::Session(session) => match crate::remote_machine_of(&state, &session) {
                Some(m) => send_command(app, &m, kind, ROUTE_TIMEOUT),
                None => execute(app, kind),
            },
            Target::Machine => match machine {
                Some(m) => {
                    if let Some(agent) = agent_of(&kind) {
                        check_remote_agent(agent, &merge::agents_of(&crate::remote_boards(&state), m), m)?;
                    }
                    send_command(app, m, kind, ROUTE_TIMEOUT)
                }
                None => execute(app, kind),
            },
        },
    }
}
```

(`execute` already saves a reply's attachments for a local session, refuses `Promote`, and returns the same data an assistant would.)

- [ ] **Step 2: Wire it into the app**

In `src-tauri/src/lib.rs`:
- change `fn remote_machine_of` and `fn remote_boards` to `pub(crate) fn`.
- in `impl Notify for TauriNetNotify` add:
  ```rust
    fn request(&self, machine: Option<&str>, request: net::protocol::Request) -> Result<Option<serde_json::Value>, String> {
        net_app::serve_request(&self.app, machine, request)
    }
  ```
- in `refresh_and_emit`, after `let _ = app.emit("sessions", &cards);`:
  ```rust
    // Mirroring Mayas see what this page shows.
    let server = app.state::<AppState>().network.lock().unwrap().server.clone();
    if let Some(s) = server {
        s.mirror(cards.clone());
    }
  ```
- after `network_promote`, add:
  ```rust
  /// Lets a paired Maya see and control every session, or stops it. With the
  /// server down only the saved list changes.
  #[tauri::command]
  fn network_set_mirror(app: AppHandle, state: TauriState<AppState>, id: String, on: bool) -> Result<NetworkStatus, String> {
      let server = state.network.lock().unwrap().server.clone();
      match server {
          Some(s) => s.set_mirror(&id, on)?,
          None => TauriNetNotify { app: app.clone() }.save_assistants(|list| {
              if let Some(a) = list.iter_mut().find(|a| a.id == id) {
                  a.mirror = on;
              }
          })?,
      }
      Ok(network_status_of(&state))
  }
  ```
- add `network_set_mirror,` to `tauri::generate_handler![…]` in `run`, beside `network_promote`.

- [ ] **Step 3: Build and run the desktop tests**

Run: `cargo build -p maya 2>&1 | tail -3 && cargo test -p maya 2>&1 | tail -5`
Expected: builds; all tests pass. (The routing decisions are covered by Task 1's `target_of`/`agent_of` tests and Task 2's request tests; `serve_request` itself needs a running app and is checked by hand in Task 9.)

- [ ] **Step 4: Commit**

```bash
git add src-tauri
git commit -m "feat: the desktop main mirrors its board and answers a phone's requests

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: A phone main serves mirrors, requests and the switch

**Files:**
- Modify: `mobile/src/hub.rs` (`serve_request`, `HubNotify::request`, mirror in `refresh`, `set_own_mirror` in `start_server`, `set_mirror`)
- Modify: `mobile/src/commands.rs` (`network_set_mirror`), `mobile/src/lib.rs` (register it)
- Test: `mobile/tests/round_trip.rs`

**Interfaces:**
- Consumes: Task 2's server API, Task 3's `RequestLane` and `pair_with` caps, Task 1's `target_of`.
- Produces: `Hub::serve_request(&self, machine: Option<&str>, request: Request) -> Result<Option<Value>, String>`, `Hub::set_mirror(&self, id: &str, on: bool) -> Result<NetworkStatus, String>`; command `network_set_mirror(id, on)`.

- [ ] **Step 1: Write the failing test**

In `mobile/tests/round_trip.rs` add (imports: `maya_core::net::client::{reconnect, RequestLane}`, `maya_core::net::protocol::Request`, `maya_core::net::routing::MachineInfo`):

```rust
/// A second phone, as a core client that mirrors.
#[derive(Default)]
struct Mirrorer {
    lane: Arc<RequestLane>,
    mirrors: Mutex<Vec<(bool, Vec<Card>)>>,
}

impl ClientNotify for Mirrorer {
    fn paired(&self, _: &str, _: &str) {}
    fn connected(&self, _: &str) {}
    fn disconnected(&self, _: &str) {}
    fn removed(&self) {}
    fn caps(&self) -> Vec<String> {
        vec!["mirror".into()]
    }
    fn mirrored(&self, allowed: bool, cards: Vec<Card>, _: Vec<MachineInfo>) {
        self.mirrors.lock().unwrap().push((allowed, cards));
    }
    fn lane(&self) -> Option<Arc<RequestLane>> {
        Some(self.lane.clone())
    }
}

#[test]
fn a_phone_main_mirrors_its_board_and_forwards_a_mirroring_maya_s_reply() {
    let (hub, _page, _dir) = new_hub(free_port());
    hub.start_server().unwrap();
    let port = hub.settings.lock().unwrap().config.listen_port();
    // The laptop, with one session.
    let code = hub.pairing_code().unwrap().code.unwrap().code;
    let laptop = Arc::new(FakeAssistant { cards: Mutex::new(vec![card("s1", State::Idle, 1_000)]), due: AtomicBool::new(false), ran: Mutex::new(vec![]) });
    let (_, id, token) = pair_with(laptop.clone(), "127.0.0.1", port, "laptop", &code, vec![]).unwrap();
    let link = NetworkConfig { role: NetworkRole::Assistant, main_host: "127.0.0.1".into(), main_port: port, name: "laptop".into(), assistant_id: id, token, ..Default::default() };
    let stop = Arc::new(AtomicBool::new(false));
    let laptop_thread = {
        let (laptop, stop) = (laptop.clone(), stop.clone());
        std::thread::spawn(move || run_once(&link, laptop, Arc::new(Quiet), &stop, None))
    };
    wait_until("the laptop's board", || hub.cards().iter().any(|c| c.session_id == "s1"));
    // A second phone joins and mirrors.
    let code = hub.pairing_code().unwrap().code.unwrap().code;
    let (_, pid, ptoken) = pair_with(Arc::new(FakeAssistant { cards: Mutex::new(vec![]), due: AtomicBool::new(false), ran: Mutex::new(vec![]) }), "127.0.0.1", port, "Fold", &code, vec!["mirror".into()]).unwrap();
    let m = Arc::new(Mirrorer::default());
    let pcfg = NetworkConfig { role: NetworkRole::Assistant, main_host: "127.0.0.1".into(), main_port: port, name: "Fold".into(), assistant_id: pid.clone(), token: ptoken, ..Default::default() };
    let pthread = {
        let (m, stop) = (m.clone(), stop.clone());
        std::thread::spawn(move || reconnect(&move || pcfg.clone(), Arc::new(NoExec), m, &stop))
    };
    wait_until("the mirror", || m.mirrors.lock().unwrap().iter().any(|(ok, cards)| *ok && cards.iter().any(|c| c.session_id == "s1" && c.machine.as_deref() == Some("laptop"))));
    m.lane.request(None, Request::Command(CommandKind::Reply { session: "s1".into(), text: "from the fold".into(), attachments: vec![] }), Duration::from_secs(5)).unwrap();
    wait_until("the forwarded reply", || laptop.ran.lock().unwrap().iter().any(|k| matches!(k, CommandKind::Reply { text, .. } if text == "from the fold")));
    let err = m.lane.request(Some("Pixel".into()), Request::Command(CommandKind::ListResumable { dir: "/d".into(), agent: Harness::ClaudeCode }), Duration::from_secs(5)).unwrap_err();
    assert_eq!(err, "This phone runs no sessions.", "the phone main's own name means the phone");
    // The switch off: the second phone is told and refused.
    hub.set_mirror(&pid, false).unwrap();
    wait_until("the refusal", || m.mirrors.lock().unwrap().iter().any(|(ok, _)| !ok));
    assert!(m.lane.request(None, Request::ListDirs, Duration::from_secs(5)).unwrap_err().ends_with("has not allowed this Maya to control its sessions."));
    stop.store(true, Ordering::SeqCst);
    let _ = (laptop_thread.join(), pthread.join());
    hub.stop_server();
}
```

with this executor beside `Quiet`:

```rust
struct NoExec;
impl Executor for NoExec {
    fn execute(&self, _: CommandKind) -> Result<Option<Value>, String> {
        Err("no".into())
    }
    fn board(&self) -> (Vec<Card>, Vec<String>) {
        (vec![], vec![])
    }
}
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p maya-mobile a_phone_main_mirrors 2>&1 | tail -15`
Expected: compile error: no method `set_mirror` on `Hub`.

- [ ] **Step 3: Implement**

`mobile/src/hub.rs` imports: add `Request` to the `protocol` use, and `use maya_core::net::routing::{agents_reply, target_of, Target};`.

In `Hub::refresh`, after `let cards = self.cards();`:

```rust
        // A Maya that mirrors this phone sees what the page shows.
        if let Some(s) = self.handle() {
            s.mirror(cards.clone());
        }
```

In `start_server`'s `Ok(handle)` arm, before storing it: `handle.set_own_mirror(true);` (the user pressed Make main on this phone, so the entry it hands itself may mirror).

Add to `impl Hub`:

```rust
    /// A mirroring Maya's request, as a main: every session and machine is
    /// an assistant's; this phone itself runs nothing.
    pub fn serve_request(&self, machine: Option<&str>, request: Request) -> Result<Option<Value>, String> {
        let to_data = |v: serde_json::Result<Value>| v.map(Some).map_err(|e| e.to_string());
        match request {
            Request::ListDirs => to_data(serde_json::to_value(machine.map(|m| merge::dirs_of(&self.boards(), m)).unwrap_or_default())),
            Request::ListAgents => to_data(serde_json::to_value(agents_reply(false, machine.and_then(|m| merge::agents_of(&self.boards(), m)), Vec::new))),
            Request::Command(kind) => match target_of(&kind) {
                Target::Session(session) => self.send(&session, |_| kind),
                Target::Machine => self.send_to(machine.ok_or("This phone runs no sessions.")?, kind),
            },
        }
    }

    /// Lets a paired Maya see and control every session, or stops it.
    pub fn set_mirror(&self, id: &str, on: bool) -> Result<NetworkStatus, String> {
        match self.handle() {
            Some(s) => s.set_mirror(id, on)?,
            None => self.save_assistants(|list| {
                if let Some(a) = list.iter_mut().find(|a| a.id == id) {
                    a.mirror = on;
                }
            })?,
        }
        Ok(self.status())
    }
```

In `impl Notify for HubNotify` add:

```rust
    fn request(&self, machine: Option<&str>, request: Request) -> Result<Option<Value>, String> {
        let Some(h) = self.0.upgrade() else { return Err("the app is gone".into()) };
        h.serve_request(machine, request)
    }
```

`mobile/src/commands.rs`, after `network_promote`:

```rust
#[tauri::command(async)]
pub fn network_set_mirror(hub: H, id: String, on: bool) -> Result<NetworkStatus, String> {
    hub.set_mirror(&id, on)
}
```

and `commands::network_set_mirror,` in `mobile/src/lib.rs`'s handler list after `commands::network_promote,`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p maya-mobile 2>&1 | tail -5`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add mobile
git commit -m "feat: a phone main mirrors its board and forwards a mirroring Maya's requests

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: The phone joins a main and mirrors it

**Files:**
- Modify: `mobile/src/hub.rs` (mirror state, `joined`, `cards`, `join`, `stop_client`, `send`/`send_to`, `HubClientNotify`)
- Modify: `mobile/src/commands.rs` (`network_pair`, listings, `server_start`), `mobile/src/lib.rs` (register `network_pair`)
- Test: `mobile/tests/round_trip.rs`

**Interfaces:**
- Consumes: Tasks 1–3 (`RequestLane`, `REQUEST_TIMEOUT`, `check_pair_args`, `pair_with` caps, `MirrorAccess`, `ClientNotify` callbacks).
- Produces: `Hub::join(self: &Arc<Self>, host: &str, port: u16, name: &str, code: &str) -> Result<NetworkStatus, String>`, `Hub::joined(&self) -> bool`, `Hub::stop_client(&self)`, `Hub::request(&self, machine: Option<&str>, request: Request) -> Result<Option<Value>, String>`, `pub mirror: Mutex<MirrorView>` with `pub struct MirrorView { pub cards: Vec<Card>, pub machines: Vec<MachineInfo> }`; command `network_pair(host, port, name, code) -> Result<NetworkStatus, String>`. The status of a joined phone: `role: Assistant`, `assistant.mirror` set from the welcome.

- [ ] **Step 1: Write the failing tests**

In `mobile/tests/round_trip.rs` (imports: `maya_core::net::MirrorAccess`, `maya_core::net::server::{start_with, ServerHandle}`):

```rust
/// A core main that records the requests it is asked.
#[derive(Default)]
struct MainNotify {
    requests: Mutex<Vec<(Option<String>, Request)>>,
}

impl maya_core::net::server::Notify for MainNotify {
    fn board_changed(&self) {}
    fn board_seeded(&self, _: &[Card]) {}
    fn status_changed(&self, _: NetworkStatus) {}
    fn paired(&self, _: &maya_core::config::PairedAssistant) -> Result<(), String> {
        Ok(())
    }
    fn paired_list_changed(&self, _: &[maya_core::config::PairedAssistant]) -> Result<(), String> {
        Ok(())
    }
    fn request(&self, machine: Option<&str>, request: Request) -> Result<Option<Value>, String> {
        self.requests.lock().unwrap().push((machine.map(str::to_string), request));
        Ok(None)
    }
}

fn desk() -> (ServerHandle, u16, Arc<MainNotify>) {
    let notify = Arc::new(MainNotify::default());
    let config = NetworkConfig { name: "desk".into(), ..Default::default() };
    let handle = start_with(notify.clone(), Arc::new(Mutex::new(config)), 0).unwrap();
    let port = handle.port();
    (handle, port, notify)
}

#[test]
fn the_phone_joins_a_main_mirrors_it_quietly_acts_through_it_and_leaves() {
    let (main, port, notify) = desk();
    let (hub, _page, alerts, _dir) = new_hub_recording(free_port());
    let code = main.open_pairing(maya_core::store::now_ms()).unwrap().0;
    let status = hub.join("127.0.0.1", port, "Pixel", &code).unwrap();
    assert_eq!(status.role, NetworkRole::Assistant);
    wait_until("the offer", || hub.status().assistant.mirror == Some(MirrorAccess::Allowed));
    let phone_id = main.status().assistants[0].id.clone();
    assert!(main.status().assistants[0].mirror);

    // What already waits is on the board but not announced.
    main.mirror(vec![card("s1", State::Awaiting, 1_000)]);
    wait_until("the mirrored card", || hub.cards().iter().any(|c| c.session_id == "s1" && c.machine.as_deref() == Some("desk")));
    std::thread::sleep(Duration::from_millis(200));
    assert!(alerts.posts.lock().unwrap().is_empty(), "the first mirror after a connect is seeded, not announced");
    // A new ask is.
    main.mirror(vec![card("s1", State::Awaiting, 1_000), card("s2", State::Awaiting, 2_000)]);
    wait_until("one notification", || alerts.posts.lock().unwrap().len() == 1);
    assert_eq!(alerts.posts.lock().unwrap()[0].title, "s2 on desk needs a decision");
    assert_eq!(hub.mirror.lock().unwrap().machines[0].name, "desk");

    // A command goes to the main.
    hub.send("s1", |session| CommandKind::Compact { session }).unwrap();
    assert!(matches!(&notify.requests.lock().unwrap()[0], (None, Request::Command(CommandKind::Compact { session })) if session == "s1"));

    // The switch off empties the board; on brings it back.
    main.set_mirror(&phone_id, false).unwrap();
    wait_until("the refusal", || hub.cards().is_empty() && hub.status().assistant.mirror == Some(MirrorAccess::Refused));
    main.set_mirror(&phone_id, true).unwrap();
    main.mirror(vec![card("s1", State::Idle, 3_000)]);
    wait_until("the board again", || hub.cards().iter().any(|c| c.session_id == "s1"));

    // The main goes away: the cards stay, greyed.
    main.stop();
    wait_until("stale cards", || !hub.cards().is_empty() && hub.cards().iter().all(|c| c.stale));

    let status = hub.stop_listening().unwrap();
    assert_eq!(status.role, NetworkRole::Off);
    assert!(hub.cards().is_empty());
}
```

`new_hub` has to hand back the `Recording` for that. Replace it with:

```rust
fn new_hub_recording(port: u16) -> (Arc<Hub>, Arc<Page>, Arc<Recording>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let maya_dir = dir.path().join("maya");
    let mut config = Config::default();
    config.network.port = port;
    config.network.name = "Pixel".into();
    config.network.role = NetworkRole::Main;
    let settings = Settings { path: maya_dir.join("config.json"), config };
    let page = Arc::new(Page::default());
    let alerts = Arc::new(Recording::default());
    let hub = Hub::new(maya_dir, settings, alerts.clone(), page.clone());
    (hub, page, alerts, dir)
}

fn new_hub(port: u16) -> (Arc<Hub>, Arc<Page>, tempfile::TempDir) {
    let (hub, page, _, dir) = new_hub_recording(port);
    (hub, page, dir)
}
```

Also add:

```rust
#[test]
fn a_join_with_a_wrong_code_puts_a_main_phone_s_server_back() {
    let (main, port, _) = desk();
    let (hub, _page, _dir) = new_hub(free_port());
    hub.start_server().unwrap();
    // No pairing window is open on the main: any code is wrong.
    let err = hub.join("127.0.0.1", port, "Pixel", "123456").unwrap_err();
    assert_eq!(err, "Wrong or expired pairing code.");
    assert_eq!(hub.status().role, NetworkRole::Main);
    assert!(hub.client.lock().unwrap().is_none());
    assert_eq!(hub.join("", port, "Pixel", "123456").unwrap_err(), "Type the main Maya's host name or address.");
    hub.stop_server();
    main.stop();
}
```

In the existing `the_phone_hands_the_main_role_over_listens_and_is_made_the_main_again`, after `wait_until("the phone listens to the laptop", …)` add:

```rust
    wait_until("the laptop offers its board", || hub.status().assistant.mirror == Some(MirrorAccess::Allowed));
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p maya-mobile 2>&1 | tail -15`
Expected: compile errors: no `join`, no field `mirror` on `Hub`.

- [ ] **Step 3: Implement the hub**

Imports in `hub.rs`: `use maya_core::net::client::{check_pair_args, pair_with, RequestLane, REQUEST_TIMEOUT};`, `use maya_core::net::protocol::{CAP_MIRROR, DEFAULT_PORT};`, `use maya_core::net::routing::MachineInfo;`, `use maya_core::net::MirrorAccess;`, `use std::sync::atomic::{AtomicBool, Ordering};`.

Add beside `Settings`:

```rust
/// The main's board while this phone has joined it.
#[derive(Default, Clone)]
pub struct MirrorView {
    pub cards: Vec<Card>,
    pub machines: Vec<MachineInfo>,
}

/// What this phone says it can do: follow a moved main, become the main,
/// and mirror the main's board.
fn phone_caps() -> Vec<String> {
    vec![CAP_FOLLOW.into(), CAP_LEAD.into(), CAP_MIRROR.into()]
}
```

`Hub` gains (update the lock-order comment to list `mirror` and `lane` with the locks taken alone):

```rust
    /// The joined main's last board; empty otherwise.
    pub mirror: Mutex<MirrorView>,
    /// Where the page's commands go while joined.
    pub lane: Arc<RequestLane>,
    /// The first mirror since the last connect was seeded into the notifier.
    mirror_seeded: AtomicBool,
```

initialised in `with_clock` as `mirror: Mutex::new(MirrorView::default()), lane: Arc::new(RequestLane::default()), mirror_seeded: AtomicBool::new(false)`.

Replace `cards()`:

```rust
    /// The board: the joined main's mirror (greyed while the link is down),
    /// or every connected assistant's cards; never local ones.
    pub fn cards(&self) -> Vec<Card> {
        if self.joined() {
            let connected = self.link.lock().unwrap().connected;
            let mut cards = self.mirror.lock().unwrap().cards.clone();
            for c in &mut cards {
                c.stale |= !connected;
            }
            return cards;
        }
        merge::merged(vec![], &self.boards(), (self.now)())
    }

    /// This phone listens to a main (joined, or after handing its role away).
    pub fn joined(&self) -> bool {
        self.client.lock().unwrap().is_some()
    }

    /// Sends a request to the joined main and waits for its answer.
    pub fn request(&self, machine: Option<&str>, request: Request) -> Result<Option<Value>, String> {
        self.lane.request(machine.map(str::to_string), request, REQUEST_TIMEOUT)
    }
```

Change `send` and `send_to` to go up while joined:

```rust
    pub fn send(&self, session_id: &str, kind: impl FnOnce(String) -> CommandKind) -> Result<Option<Value>, String> {
        if self.joined() {
            return self.request(None, Request::Command(kind(session_id.to_string())));
        }
        let machine = merge::route(&[], &self.boards(), session_id, (self.now)()).ok_or("Session is no longer running.")?;
        self.send_to(&machine, kind(session_id.to_string()))
    }

    pub fn send_to(&self, machine: &str, kind: CommandKind) -> Result<Option<Value>, String> {
        if self.joined() {
            return self.request(Some(machine), Request::Command(kind));
        }
        let shared = self.handle().map(|s| s.shared.clone()).ok_or("The server is stopped.")?;
        send_command_with(&shared, machine, kind, ROUTE_TIMEOUT)
    }
```

Add `join` and `stop_client`, and make `stop_listening` use `stop_client`:

```rust
    /// Pairs with a running main and listens to it. A phone that was the
    /// main stops its server first and gets it back if the pairing fails.
    pub fn join(self: &Arc<Self>, host: &str, port: u16, name: &str, code: &str) -> Result<NetworkStatus, String> {
        let (host, code) = check_pair_args(host, code)?;
        let port = if port == 0 { DEFAULT_PORT } else { port };
        let name = match name.trim() {
            "" => self.settings.lock().unwrap().config.network.name.clone(),
            n => n.to_string(),
        };
        let was_main = self.handle().is_some();
        self.stop_server();
        self.stop_client();
        let paired = pair_with(Arc::new(NoSessions), &host, port, &name, &code, phone_caps()).and_then(|(main, id, token)| {
            self.update_config(|c| {
                c.network.role = NetworkRole::Assistant;
                c.network.main_host = host.clone();
                c.network.main_port = port;
                c.network.name = name.clone();
                c.network.assistant_id = id;
                c.network.token = token;
            })
            .map(|_| main)
        });
        match paired {
            Ok(main) => {
                log::line("network", format!("joined {main}"));
                self.start_client();
                Ok(self.status())
            }
            Err(e) => {
                if was_main {
                    let _ = self.start_server();
                }
                Err(e)
            }
        }
    }

    /// Stops listening to the main without changing the config; the board
    /// and the link are forgotten.
    pub fn stop_client(&self) {
        let client = self.client.lock().unwrap().take();
        if let Some(c) = client {
            c.stop_and_join();
            self.sink.service_stop();
        }
        *self.link.lock().unwrap() = AssistantLink::default();
        *self.mirror.lock().unwrap() = MirrorView::default();
        self.refresh();
    }
```

In `stop_listening`, replace the block from `let client = …` to `*self.link.lock().unwrap() = AssistantLink::default();` with `self.stop_client();`.

`HubClientNotify`: change the service line in `set` to `(Some(m), true) => format!("Connected to {m}"),`; in `connected` and `disconnected` and `removed`, call `h.refresh()` after `self.set(…)` so the cards grey or come back (get `h` with `if let Some(h) = self.hub.upgrade() { h.refresh(); }`); `caps()` returns `phone_caps()`; and add:

```rust
    fn mirror_offer(&self, offered: Option<bool>) {
        let Some(h) = self.hub.upgrade() else { return };
        h.mirror_seeded.store(false, Ordering::SeqCst);
        let access = match offered {
            Some(true) => MirrorAccess::Allowed,
            Some(false) => MirrorAccess::Refused,
            None => MirrorAccess::Unsupported,
        };
        if access != MirrorAccess::Allowed {
            *h.mirror.lock().unwrap() = MirrorView::default();
        }
        self.set(|l| l.mirror = Some(access));
        h.refresh();
    }
    fn mirrored(&self, allowed: bool, cards: Vec<Card>, machines: Vec<MachineInfo>) {
        if self.stop.load(Ordering::SeqCst) {
            return;
        }
        let Some(h) = self.hub.upgrade() else { return };
        if !allowed {
            *h.mirror.lock().unwrap() = MirrorView::default();
            self.set(|l| l.mirror = Some(MirrorAccess::Refused));
            h.refresh();
            return;
        }
        // What already waits when the link comes up is not news.
        if !h.mirror_seeded.swap(true, Ordering::SeqCst) {
            h.notifier.lock().unwrap().seed(&cards);
        }
        *h.mirror.lock().unwrap() = MirrorView { cards, machines };
        self.set(|l| l.mirror = Some(MirrorAccess::Allowed));
        h.refresh();
    }
    fn lane(&self) -> Option<Arc<RequestLane>> {
        self.hub.upgrade().map(|h| h.lane.clone())
    }
```

(`mirrored` sets the access too, since turning the switch on sends a mirror without a new `welcome`.)

In `promoted`, after `h.client.lock().unwrap().take();`, also clear the board: `*h.mirror.lock().unwrap() = MirrorView::default();`.

- [ ] **Step 4: Implement the commands**

`mobile/src/commands.rs` (import `maya_core::net::protocol::Request`):

```rust
/// Pairs this phone with a running main and listens to it.
#[tauri::command(async)]
pub fn network_pair(hub: H, host: String, port: u16, name: String, code: String) -> Result<NetworkStatus, String> {
    hub.join(&host, port, &name, &code)
}
```

`list_machines`:

```rust
#[tauri::command]
pub fn list_machines(hub: H) -> Vec<MachineInfo> {
    if hub.joined() {
        return hub.mirror.lock().unwrap().machines.clone();
    }
    machines_of(&hub.status())
}
```

`list_project_dirs` and `list_agents` become `#[tauri::command(async)]` with a joined branch:

```rust
#[tauri::command(async)]
pub fn list_project_dirs(hub: H, machine: Option<String>) -> Result<Vec<String>, String> {
    let machine = machine_arg(machine)?;
    if hub.joined() {
        return data(hub.request(Some(&machine), Request::ListDirs)?);
    }
    Ok(merge::dirs_of(&hub.boards(), &machine))
}

#[tauri::command(async)]
pub fn list_agents(hub: H, machine: Option<String>) -> AgentsReply {
    let machine = machine.filter(|m| !m.is_empty());
    if hub.joined() {
        return hub.request(machine.as_deref(), Request::ListAgents).and_then(data).unwrap_or_else(|e| {
            log::line("network", format!("could not list the agents: {e}"));
            agents_reply(false, None, Vec::new)
        });
    }
    let remote: Option<Vec<AgentInfo>> = machine.as_deref().and_then(|m| merge::agents_of(&hub.boards(), m));
    agents_reply(false, remote, Vec::new)
}
```

In `list_resumable_sessions`, `resume_session` and `start_session`, wrap the `check_remote_agent(…)?;` line as `if !hub.joined() { check_remote_agent(…)?; }` (the main checks it when joined).

`server_start`: first line `hub.stop_client();`.

Register `commands::network_pair,` in `mobile/src/lib.rs` beside `commands::network_status,`, and change the doc comment at the top of `mobile/src/lib.rs` to:

```rust
//! Maya for Android: a main Maya with no sessions of its own, or a phone
//! that joins a main and mirrors its board. The server, pairing, merging
//! and routing are `maya_core`'s; this crate is the Tauri commands the page
//! calls, and the Android glue: the foreground service and the notifications.
```

and `start_client`'s doc comment in `hub.rs` to: "Listens to the main in the config and mirrors its board; the keep-awake service runs meanwhile. That main can make this phone the main."

- [ ] **Step 5: Run the tests**

Run: `cargo test -p maya-mobile 2>&1 | tail -5`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add mobile
git commit -m "feat: the phone joins a main, mirrors its board and acts through it

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: The desktop's per-Maya switch

**Files:**
- Modify: `src/settings.ts` (types, the assistant row, handlers)
- Test: `src/settings.test.ts`

**Interfaces:**
- Consumes: `network_set_mirror` (Task 4); `AssistantStatus.mirror`/`wantsMirror` and `AssistantLink.mirror` JSON (Task 1).
- Produces: TS types used by Task 8: `AssistantStatus.mirror?: boolean`, `AssistantStatus.wantsMirror?: boolean`, `AssistantLink.mirror?: "allowed" | "refused" | "unsupported" | null`.

- [ ] **Step 1: Write the failing test**

In `src/settings.test.ts`, add `onMirror: vi.fn(),` to the object `handlers` (line 35) returns, and add:

```ts
describe("the mirror switch", () => {
  const phone = { id: "p", name: "Pixel", hostname: "localhost", platform: "android", address: "192.168.1.30", connected: true, lastSeen: null, canLead: true, canFollow: true, mirror: true, wantsMirror: true };
  const laptop = { id: "g", name: "laptop", hostname: "laptop", platform: "macos", address: "192.168.55.70", connected: true, lastSeen: null, mirror: false, wantsMirror: false };
  const network = { role: "main" as const, code: null, assistants: [phone, laptop], assistant: { connected: false, mainName: null, error: null } };

  it("shows for a Maya that asked to mirror, says Android, and saves on change", () => {
    const h = handlers();
    const el = renderSettings({ ...voiceBase, networkRole: "main", network }, h);
    const boxes = [...el.querySelectorAll<HTMLInputElement>("input[data-action=mirror]")];
    expect(boxes.map((b) => b.dataset.id)).toEqual(["p"]);
    expect(boxes[0].checked).toBe(true);
    expect(boxes[0].parentElement!.textContent).toContain("Sees and controls all sessions");
    expect(el.textContent).toContain("Pixel (Android, 192.168.1.30)");
    boxes[0].checked = false;
    boxes[0].dispatchEvent(new Event("change"));
    expect(h.onMirror).toHaveBeenCalledWith("p", false);
  });
});
```

- [ ] **Step 2: Run it to see it fail**

Run: `pnpm test src/settings.test.ts 2>&1 | tail -15`
Expected: FAIL: no `input[data-action=mirror]`.

- [ ] **Step 3: Implement**

In `src/settings.ts`:
- `AssistantStatus`: add
  ```ts
  /** The main lets it see and control every session. */
  mirror?: boolean;
  /** Connected and asked to mirror: its switch shows. */
  wantsMirror?: boolean;
  ```
- `AssistantLink`: add
  ```ts
  /** What the main said about mirroring at the last connect. */
  mirror?: "allowed" | "refused" | "unsupported" | null;
  ```
- `SettingsHandlers`: add `onMirror(id: string, on: boolean): void;` after `onMakeMain`.
- in the assistant row, the info line becomes
  ```ts
      info.textContent = `${a.name} (${[a.platform === "android" ? "Android" : a.platform, a.address].filter(Boolean).join(", ")}) — ${a.connected ? "Connected" : a.lastSeen !== null ? `Last seen ${formatAge(a.lastSeen, nowMs)} ago` : "Never connected"}`;
  ```
  and after the Make main button block, before `list.append(row);`:
  ```ts
      if (a.wantsMirror || a.mirror) {
        const sw = document.createElement("label");
        sw.className = "settings__switch";
        const box = document.createElement("input");
        box.type = "checkbox";
        box.dataset.action = "mirror";
        box.dataset.id = a.id;
        box.checked = !!a.mirror;
        box.disabled = !!model.networkPromoting;
        box.addEventListener("change", () => h.onMirror(a.id, box.checked));
        sw.append(box, document.createTextNode(" Sees and controls all sessions"));
        row.append(sw);
      }
  ```
- in the handlers object (near `onRemoveAssistant`, line ~1240):
  ```ts
    onMirror: (id, on) => void run(async () => { model.network = await invoke<NetworkStatus>("network_set_mirror", { id, on }); }),
  ```

- [ ] **Step 4: Run the page tests and the type check**

Run: `pnpm test 2>&1 | tail -6 && pnpm exec tsc --noEmit`
Expected: all pass, no type errors.

- [ ] **Step 5: Commit**

```bash
git add src/settings.ts src/settings.test.ts
git commit -m "feat: a switch on the main for each Maya that asks to see every session

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: The phone's Join screens

**Files:**
- Modify: `src/mobile/setup.ts`, `src/mobile/network.ts`, `src/mobile/main.ts:~85` (`paintHint`)
- Test: `src/mobile/setup.test.ts`, `src/mobile/network.test.ts`

**Interfaces:**
- Consumes: commands `network_pair(host, port, name, code)`, `network_set_mirror(id, on)`, `listen_stop` (Tasks 5–6); TS types from Task 7.
- Produces: nothing later tasks use.

- [ ] **Step 1: Write the failing tests**

`src/mobile/setup.test.ts`: in the two existing `renderSetup({ … })` model literals add `mode: "main", refused: null`, change `const h = { onStart: vi.fn() };` to `const h = { onStart: vi.fn(), onJoin: vi.fn(), onMode: vi.fn() };`, and add:

```ts
  it("shows a joined phone's board, or why the main shares nothing", () => {
    const joined: NetworkStatus = { ...off, role: "assistant", assistant: { connected: true, mainName: "desk", error: null, mirror: "allowed" } };
    expect(isFirstRun(joined)).toBe(false);
    expect(setupView(joined, false)).toBe("none");
    expect(setupView({ ...joined, assistant: { ...joined.assistant, mirror: "refused" } }, false)).toBe("board");
    expect(setupView({ ...joined, assistant: { ...joined.assistant, mirror: "unsupported" } }, false)).toBe("board");
    const h = { onStart: vi.fn(), onJoin: vi.fn(), onMode: vi.fn() };
    const refused = renderSetup({ firstRun: false, mode: "main", name: "Pixel 8", port: 4127, error: null, busy: false, refused: { main: "desk", older: false } }, h);
    expect(refused.textContent).toContain(`desk has not allowed this phone to see its sessions. Turn on "Sees and controls all sessions" for Pixel 8 in desk's Settings → Network.`);
    expect(refused.querySelector("button")).toBeNull();
    const older = renderSetup({ firstRun: false, mode: "main", name: "Pixel 8", port: 4127, error: null, busy: false, refused: { main: "desk", older: true } }, h);
    expect(older.textContent).toContain("desk runs an older Maya that cannot share its sessions. Update it.");
  });

  it("offers Be the main or Join a main on the first run, and joins with what was typed", () => {
    const h = { onStart: vi.fn(), onJoin: vi.fn(), onMode: vi.fn() };
    const first = renderSetup({ firstRun: true, mode: "main", name: "Pixel 8", port: 4127, error: null, busy: false, refused: null }, h);
    first.querySelector<HTMLButtonElement>("button[data-action=mode-join]")!.click();
    expect(h.onMode).toHaveBeenCalledWith("join");
    const join = renderSetup({ firstRun: true, mode: "join", name: "Pixel 8", port: 4127, error: "Wrong or expired pairing code.", busy: false, refused: null }, h);
    expect(join.querySelector("button[data-action=start]")).toBeNull();
    expect(join.textContent).toContain("Wrong or expired pairing code.");
    join.querySelector<HTMLInputElement>("input[name=host]")!.value = "192.168.1.10";
    join.querySelector<HTMLInputElement>("input[name=code]")!.value = "483921";
    join.querySelector<HTMLButtonElement>("button[data-action=join]")!.click();
    expect(h.onJoin).toHaveBeenCalledWith("Pixel 8", "192.168.1.10", 4127, "483921");
  });
```

`src/mobile/network.test.ts`: add `mainHost: "", mainPort: 0,` to `base`; add `onJoin: vi.fn(), onMirror: vi.fn()` to `handlers()`; rewrite the first test as:

```ts
  it("shows Connected to with the main's address and Leave while the phone has joined a main", () => {
    const h = handlers();
    const joined: NetworkModel = { ...base, mainHost: "192.168.1.10", mainPort: 4127, status: { role: "assistant", code: null, assistants: [], assistant: { connected: true, mainName: "laptop", error: null } } };
    const el = renderNetwork(joined, h, NOW);
    expect(el.textContent).toContain("Connected to laptop");
    expect(el.textContent).toContain("192.168.1.10:4127");
    expect(el.querySelector("button[data-action=start]")).toBeNull();
    expect(el.querySelector("button[data-action=join]")).toBeNull();
    expect(el.querySelector(".network__code")).toBeNull();
    el.querySelector<HTMLButtonElement>("button[data-action=leave]")!.click();
    expect(h.onStopListening).toHaveBeenCalled();
  });
```

and add:

```ts
  it("offers Join a main while the phone is off, and keeps what was typed across a repaint", () => {
    const h = handlers();
    const off: NetworkModel = { ...base, status: { ...running, role: "off", code: null, assistants: [] } };
    const el = renderNetwork(off, h, NOW);
    el.querySelector<HTMLInputElement>("input[name=host]")!.value = "desk.local";
    el.querySelector<HTMLInputElement>("input[name=code]")!.value = "483921";
    const edits = carryEdits(el, off);
    const again = renderNetwork(off, h, NOW);
    restoreEdits(again, edits);
    expect(again.querySelector<HTMLInputElement>("input[name=host]")!.value).toBe("desk.local");
    again.querySelector<HTMLButtonElement>("button[data-action=join]")!.click();
    expect(h.onJoin).toHaveBeenCalledWith("desk.local", 4127, "483921");
    expect(renderNetwork(base, h, NOW).querySelector("button[data-action=join]")).toBeNull();
  });

  it("shows the mirror switch for a Maya that asked, and Android by name", () => {
    const h = handlers();
    const fold = { id: "f", name: "Fold", hostname: "localhost", platform: "android", address: "192.168.1.31", connected: true, lastSeen: null, mirror: false, wantsMirror: true };
    const el = renderNetwork({ ...base, status: { ...running, assistants: [fold] } }, h, NOW);
    expect(el.textContent).toContain("Android");
    const box = el.querySelector<HTMLInputElement>("input[data-action=mirror]")!;
    expect(box.checked).toBe(false);
    box.checked = true;
    box.dispatchEvent(new Event("change"));
    expect(h.onMirror).toHaveBeenCalledWith("f", true);
  });
```

- [ ] **Step 2: Run them to see them fail**

Run: `pnpm test src/mobile 2>&1 | tail -20`
Expected: FAIL on the new tests (no join button, no mirror switch, no `refused` card).

- [ ] **Step 3: Implement `network.ts`**

- `PLATFORM_NAMES` gains `android: "Android"`.
- `NetworkModel` gains:
  ```ts
  /** The joined main's host and port, from the config. */
  mainHost: string;
  mainPort: number;
  ```
- `NetworkHandlers` gains `onJoin(host: string, port: number, code: string): void;` and `onMirror(id: string, on: boolean): void;`.
- Add helpers above `renderNetwork`:
  ```ts
  const JOIN_HINT = "Open Settings → Network on the main, choose Show pairing code, and type the code it shows.";

  function input(label: string, name: string, value: string, type = "text"): { label: HTMLLabelElement; input: HTMLInputElement } {
    const l = el("label", "", `${label} `);
    const i = el("input", "");
    i.name = name;
    i.type = type;
    i.value = value;
    l.append(i);
    return { label: l, input: i };
  }

  function joinSection(h: NetworkHandlers): HTMLElement {
    const join = section("Join a main");
    const host = input("Host", "host", "");
    const port = input("Port", "joinPort", "4127", "number");
    const code = input("Pairing code", "code", "");
    code.input.inputMode = "numeric";
    join.append(host.label, port.label, code.label, el("p", "settings__hint", JOIN_HINT));
    join.append(button("Join", "join", () => h.onJoin(host.input.value, Number(port.input.value) || 0, code.input.value), true));
    return join;
  }
  ```
- In `renderNetwork`: the listening block becomes
  ```ts
  if (listening) {
    const link = m.status.assistant;
    phone.append(el("p", "network__listening", link.connected && link.mainName ? `Connected to ${link.mainName}` : "Reconnecting…"));
    if (m.mainHost) phone.append(el("p", "settings__hint", `${m.mainHost}:${m.mainPort || 4127}`));
    if (!link.connected && link.error) phone.append(el("p", "settings__hint", link.error));
    phone.append(button("Leave", "leave", h.onStopListening));
  }
  ```
  after `root.append(phone);` add `if (!running && !listening) root.append(joinSection(h));`, and in the assistant loop, after `if (a.note) …`:
  ```ts
    if (a.wantsMirror || a.mirror) {
      const sw = el("label", "settings__switch");
      const box = el("input", "");
      box.type = "checkbox";
      box.dataset.action = "mirror";
      box.dataset.id = a.id;
      box.checked = !!a.mirror;
      box.addEventListener("change", () => h.onMirror(a.id, box.checked));
      sw.append(box, document.createTextNode(" Sees and controls all sessions"));
      text.append(sw);
    }
  ```
- Replace `carryEdits` and `restoreEdits` (the existing tests' `edits.name`/`edits.port` keep working):
  ```ts
  const CARRIED = ["name", "port", "host", "joinPort", "code"] as const;
  type Carried = (typeof CARRIED)[number];
  export type Edits = Partial<Record<Carried, string>> & { focus?: Carried };

  /** Carry forward typed values across repaints: each field that differs from what a fresh render shows, and the focused one. */
  export function carryEdits(pane: ParentNode, model: { name: string; port: number }): Edits {
    const fresh: Record<Carried, string> = { name: model.name, port: String(model.port), host: "", joinPort: "4127", code: "" };
    const edits: Edits = {};
    const doc = pane.ownerDocument;
    for (const f of CARRIED) {
      const input = pane.querySelector<HTMLInputElement>(`input[name=${f}]`);
      if (!input) continue;
      if (input.value !== fresh[f]) edits[f] = input.value;
      if (doc && doc.activeElement === input) edits.focus = f;
    }
    return edits;
  }

  /** Restore carried edits into the pane after a repaint. */
  export function restoreEdits(pane: ParentNode, edits: Edits): void {
    for (const f of CARRIED) {
      const input = pane.querySelector<HTMLInputElement>(`input[name=${f}]`);
      if (!input) continue;
      const v = edits[f];
      if (v !== undefined) input.value = v;
      if (edits.focus === f) input.focus();
    }
  }
  ```
- `initNetwork`: `ConfigView.network` gains `mainHost?: string; mainPort?: number;`; the model starts with `mainHost: "", mainPort: 0`; factor the config read into
  ```ts
  const loadConfig = async () => {
    config = await invoke<ConfigView>("get_config");
    model.name = config.network.name;
    model.port = config.network.port || 4127;
    model.mainHost = config.network.mainHost ?? "";
    model.mainPort = config.network.mainPort ?? 0;
    model.notifyAwaiting = config.notifyOnAwaiting;
    model.notifyCompleted = config.notifyOnCompleted;
  };
  ```
  used by the startup IIFE; and add handlers
  ```ts
    onJoin: (host, port, code) => void busy(async () => {
      setStatus(await invoke<NetworkStatus>("network_pair", { host, port, name: model.name, code }));
      await loadConfig();
    }),
    onMirror: (id, on) => void busy(async () => setStatus(await invoke<NetworkStatus>("network_set_mirror", { id, on }))),
  ```
  and `onStopListening` reloads the config after `listen_stop` (`await loadConfig();`).

- [ ] **Step 4: Implement `setup.ts`**

- `SetupModel` gains:
  ```ts
  /** The first run's choice: run the server here, or join a main. */
  mode: "main" | "join";
  /** Joined, but the main shares no sessions: whose, and whether it is too old to. */
  refused: { main: string; older: boolean } | null;
  ```
- ```ts
  /** Nothing has ever paired and the phone is neither a main nor joined: say what Maya is for. */
  export function isFirstRun(status: NetworkStatus): boolean {
    return status.role === "off" && status.assistants.length === 0;
  }

  /** Which view a status calls for; `ran` is whether the phone was a main or joined since the page loaded. */
  export function setupView(status: NetworkStatus, ran: boolean): "none" | "overlay" | "board" {
    if (status.role === "main") return "none";
    if (status.role === "assistant") {
      const m = status.assistant.mirror;
      return m === "refused" || m === "unsupported" ? "board" : "none";
    }
    return isFirstRun(status) && !ran ? "overlay" : "board";
  }

  /** Why a joined phone's board is empty. */
  export function refusedLine(main: string, phone: string, older: boolean): string {
    return older
      ? `${main} runs an older Maya that cannot share its sessions. Update it.`
      : `${main} has not allowed this phone to see its sessions. Turn on "Sees and controls all sessions" for ${phone} in ${main}'s Settings → Network.`;
  }
  ```
- `renderSetup(m, h: { onStart(name: string, port: number): void; onJoin(name: string, host: string, port: number, code: string): void; onMode(mode: "main" | "join"): void })`:
  - when `m.refused`: a card with class `setup__card setup__card--stopped`, an `h2` "No sessions to show", a `p` with `refusedLine(m.refused.main, m.name, m.refused.older)`, and nothing else; return it.
  - on a first run: the title "Maya for Android", then two buttons in a `div.setup__modes`: "Be the main" (`data-action=mode-main`) and "Join a main" (`data-action=mode-join`), each with `aria-pressed` set from `m.mode`, calling `h.onMode`. In `main` mode, today's blurb, Name, Port and Start. In `join` mode, the blurb "Join a Maya that is already the main, to see and answer every session it shows.", then Name (`name`), Host (`host`), Port (`joinPort`, value `String(m.port)`), Pairing code (`code`, `inputMode = "numeric"`), the line "Open Settings → Network on the main, choose Show pairing code, and type the code it shows.", and a primary button "Join" (`data-action=join`, "Joining…" and disabled while `m.busy`) that calls `h.onJoin(nameInput.value, hostInput.value, Number(portInput.value) || 0, codeInput.value)`.
  - not a first run: unchanged (the stopped card with Start).
  - the error line stays last on every card.
- `initSetup`: the model starts with `mode: "main", refused: null`; `paint(status)` sets `ran = true` for role `main` or `assistant`, and
  ```ts
    const m = status.assistant.mirror;
    model.refused = status.role === "assistant" && (m === "refused" || m === "unsupported") ? { main: status.assistant.mainName ?? "The main Maya", older: m === "unsupported" } : null;
  ```
  `show()` passes `{ onStart: …, onJoin: (name, host, port, code) => void join(name, host, port, code), onMode: (mode) => { model.mode = mode; show(); } }`; and add
  ```ts
  const join = async (name: string, host: string, port: number, code: string) => {
    model.name = name;
    model.busy = true;
    model.error = null;
    show();
    try {
      if (!(await isPermissionGranted().catch(() => true))) await requestPermission().catch(() => undefined);
      hooks.onPermissionAsked();
      model.busy = false;
      paint(await invoke<NetworkStatus>("network_pair", { host, port, name, code }));
      return;
    } catch (e) {
      model.error = String(e);
    }
    model.busy = false;
    show();
  };
  ```
- Update the file's header comment: "The phone's views when it shows no board of its own: on a first run, an overlay that runs the server here or joins a main; once it has run, the board says the server is stopped; and a joined phone whose main shares nothing says why."

`src/mobile/main.ts` `paintHint`: change its first line to `if (!status || status.role !== "main" || status.assistants.length > 0) return;` (a joined phone has no assistants of its own).

Add to `src/mobile/mobile.css`: `.setup__modes { display: flex; gap: 8px; margin: 8px 0 12px; }` and `.setup__modes button[aria-pressed=true] { border-color: var(--gold); color: var(--gold-bright); }`.

- [ ] **Step 5: Run the page tests and the type check**

Run: `pnpm test 2>&1 | tail -6 && pnpm exec tsc --noEmit`
Expected: all pass, no type errors.

- [ ] **Step 6: Commit**

```bash
git add src/mobile
git commit -m "feat: the phone's Join a main screens and the main's switch on the phone

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: Docs, version, and the whole-branch check

**Files:**
- Modify: `docs/DEVELOPING.md:245-250`, `README.md:29`, `README.md:175-188`, `docs/superpowers/specs/2026-10-14-phone-joins-a-main-design.md` (Status line)
- Modify (by script): `src-tauri/tauri.conf.json`, `mobile/tauri.conf.json`, `package.json`, `Cargo.toml`, `Cargo.lock`

- [ ] **Step 1: Update the docs**

`docs/DEVELOPING.md`, replace "It is only ever a main, or, after it handed the main role to an assistant, a listener with no sessions that the new main can make the main again." with:

"It is a main, or it joins a main (with a pairing code, or by handing its own main role to an assistant) and mirrors that main's board: the main sends it `mirror` frames and answers its `request`s, as long as the main's \"Sees and controls all sessions\" switch for it is on. A main can make a joined phone the main again."

`README.md` line 29 becomes:

"- Comes as an Android app too: a main Maya in your pocket, or a phone that joins your computer's Maya, shows every session and notifies you when one needs you."

`README.md`, the Android paragraph: replace "The phone is a **main** Maya and nothing else: it runs no agents and shows the sessions of the assistants paired with it." with "The phone runs no agents. It is either the **main**, with your computers paired to it, or it **joins** a computer that is already the main and shows that main's whole board." After step 3 of the list add:

"To join a computer that is already the main instead, choose **Join a main** on the first screen (or in the Network tab when the server is off), type the computer's address, port 4127 and the code from its Settings › Network › Show pairing code, and press **Join**. The computer lists the phone with a **Sees and controls all sessions** switch, on from the start; turn it off to stop the phone seeing and acting on the sessions."

In the spec, set `Status: implemented`.

- [ ] **Step 2: Bump the version**

```bash
git fetch -q origin && git show origin/main:package.json | grep '"version"'
```

If `main` is still `0.20.0`: `PATH="$HOME/.cargo/bin:$PATH" sh scripts/set-version.sh 0.21.0`, then `git status --short` must list `Cargo.lock`; if it does not, run `PATH="$HOME/.cargo/bin:$PATH" cargo update --workspace --offline`. If `main` moved, bump its minor version instead.

- [ ] **Step 3: Run everything**

```bash
cargo test --workspace 2>&1 | grep -E "^test result|FAILED|panicked" | head -20
pnpm test 2>&1 | tail -4
pnpm exec tsc --noEmit
cargo clippy --workspace --all-targets 2>&1 | grep -E "^(warning|error)" | sort | uniq -c | head
```

Expected: every test result `ok`, vitest all passing, no type errors, no new clippy warnings against `main` (compare with `git stash; cargo clippy …; git stash pop` if unsure).

- [ ] **Step 4: Commit**

```bash
git add docs README.md
git commit -m "docs: the phone can join a main and mirror its board

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
git add src-tauri/tauri.conf.json mobile/tauri.conf.json package.json Cargo.toml Cargo.lock
git commit -m "chore(release): 0.21.0

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 5: The hand check (with the user; not automatable here)**

Ask the user to run, on the Mac (main), the laptop (assistant) and the phone:
1. On the phone, Join a main with the Mac's address and code. The phone's board shows the Mac's and the laptop's sessions.
2. Reply from the phone to a session on the Mac and to one on the laptop; both arrive.
3. A session on the laptop starts waiting on a decision; the phone posts a notification.
4. Turn the Mac's switch for the phone off: the phone's board shows why; on: it comes back.
5. On the Mac, Make main beside the phone; the Mac and the laptop follow the phone. Then, on the phone, Make main beside the Mac.

Record the result in the PR; do not call the feature verified before it is done.
