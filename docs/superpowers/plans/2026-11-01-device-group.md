# Your Mayas: One Trusted Group — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every Maya keeps the user's whole group of trusted devices, so a device paired once never needs a code again: it follows a promoted main (even after being offline), joins a known main with no code, and a removal reaches every Maya.

**Architecture:** The main stays the authority for the group (members plus removed tombstones, each stamped with a generation). It sends the group (`Down::Group`) after `Welcome` and on every change; each device stores it in its own config and sends its copy back in `Hello`, from which the main takes only removals. A promoted-away main keeps a redirect listener on its old port that tells returning members where the new main is. Settings › Devices (desktop and phone) shows the group.

**Tech Stack:** Rust (`core`, `src-tauri`, `mobile`, `cli`), serde, Noise/SPAKE2 transport already in `core/src/net/secure.rs`; TypeScript UI with vitest.

**Spec:** `docs/superpowers/specs/2026-11-01-device-group-design.md`

## Global Constraints

- `PROTOCOL` stays `2` (`core/src/net/protocol.rs:12`). Only new enum variants and `#[serde(default)]` fields; an older Maya must keep working (it ignores unknown frames).
- The group carries public keys, names and labels only. Never a private key.
- A device takes a group only from its current main (or the old main's redirect listener, which was its main).
- A main refuses a key not among its live members before any frame, as today (`bye "removed"`, `server.rs:851`). A tombstoned key counts as unknown.
- Only a main's own code pairing adds a member. A copy from a member carries removals only; entries the main never had are ignored.
- A device moves to a new main only on `Move` from the main it trusts (or that main's redirect listener), and only if the new main's key is a live member of the group it holds.
- `Notify::paired` and `Notify::paired_list_changed` are called with the server mutex held and must not take it (`server.rs:73`). Lock order: `network`, then server mutex, then `store` (`src-tauri/src/lib.rs:156`).
- Repaints that touch hovered rows use `morph` from `src/morph.ts`, never `replaceChildren` (CLAUDE.md lore).
- Tests first in every task. Full checks: `cargo test --workspace`, `pnpm vitest run`, `pnpm tsc --noEmit`.
- The PR bumps to `0.45.0` (main is `0.44.3`, this ships a `feat`) with `sh scripts/set-version.sh 0.45.0` and a `RELEASE_NOTES.md` section, in one `chore(release): 0.45.0` commit.

## Review Focus

1. A removed device that reconnects after the main restarts must stay refused: the main seeds its tombstones from config at start. (Test in Task 3.)
2. A device offline while it was removed comes back carrying a copy in which it is still a live member: it must stay removed and be told so. (Test in Task 3.)
3. A device removed, then paired again with a code, must not be removed again by a stale copy carrying the older tombstone. (Test in Task 1.)
4. An old main that the user later joins by hand to another main must stop redirecting, so no device follows a main nobody promoted. (Test in Task 6.)
5. A Maya upgraded from 0.44 as an assistant must show its main in Devices before its first connection, and still connect. (Test in Task 7.)

---

### Task 1: The group model and its rules

**Files:**
- Modify: `core/src/config.rs:17-59` (`PairedAssistant`, `NetworkConfig`)
- Create: `core/src/net/group.rs`
- Modify: `core/src/net/mod.rs` (add `pub mod group;`, `devices` on `NetworkStatus`)
- Modify: every `PairedAssistant { .. }` literal the compiler lists (add `generation: 0`)

**Interfaces:**
- Produces, in `core/src/config.rs`:
  - `PairedAssistant.generation: u64` (`#[serde(default)]`): the group version at which this entry was last added, paired again or removed.
  - `NetworkConfig.removed: Vec<PairedAssistant>` (tombstones), `NetworkConfig.group_version: u64`, `NetworkConfig.redirect_port: u16` (0 = no redirect listener). `assistants` is now documented as "this Maya's copy of the group: every live member but itself"; its name stays for config compatibility.
  - `impl NetworkConfig { pub fn end_redirect(&mut self) }` sets `redirect_port = 0`.
- Produces, in `core/src/net/group.rs`:
  - `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)] #[serde(rename_all = "camelCase", default)] pub struct GroupCopy { pub version: u64, pub members: Vec<PairedAssistant>, pub removed: Vec<PairedAssistant> }`
  - `pub fn of_config(n: &NetworkConfig) -> GroupCopy`
  - `pub fn add(g: &mut GroupCopy, entry: PairedAssistant) -> PairedAssistant`: version + 1, stamps `generation = version`, drops a tombstone with the same key, replaces a live entry with the same key keeping its `id`, else pushes. Returns the stored entry.
  - `pub fn remove(g: &mut GroupCopy, key: &str) -> Option<PairedAssistant>`: moves the live entry to `removed` with `generation = version + 1`, `mirror = false`; version + 1. `None` when no live entry has the key.
  - `pub fn merge_copy(g: &mut GroupCopy, theirs: &GroupCopy) -> bool`: for each tombstone in `theirs.removed` whose key is a live member of `g` with `generation <= tombstone.generation`, remove it (keeping the tombstone's generation); raise a known tombstone's generation if theirs is higher. Ignores `theirs.members` and unknown keys. If anything changed, `g.version = max(g.version, theirs.version) + 1`. Returns whether it changed.
  - `pub fn adopt(n: &mut NetworkConfig, my_key: &str, copy: &GroupCopy)`: `assistants = copy.members` without `my_key`, `removed = copy.removed`, `group_version = copy.version`.
  - `pub fn may_follow(n: &NetworkConfig, seen: Option<&GroupCopy>, key: &str) -> bool`: with `seen`, true when `key` is in `seen.members` and not in `seen.removed`; without, true when `key` is in `n.assistants` and not in `n.removed`, or when `n.assistants` is empty (an older main that sends no group).
  - `pub fn is_phone(p: &PairedAssistant) -> bool` (`p.platform == "android"`).
  - `pub fn join(n: &mut NetworkConfig, my_key: &str, key: &str, host: &str, port: u16) -> Result<(), String>`: sets role `Assistant`, `main_host = host.trim()`, `main_port` (0 → `protocol::DEFAULT_PORT`), `main_key = key`, `pair_again = false`, `end_redirect()`. Errors, exactly: key is `my_key` → `"This is this device."`; not a live member → `"That device is not in your group."`; `is_phone` → `"A phone cannot be the main Maya."`; blank host → `"Type the main Maya's host name or address."`.
  - `pub fn me(n: &NetworkConfig, key: &str) -> PairedAssistant`: this Maya's own entry. `id = key`, name from `n.name` (hostname when blank), `hostname = super::local_hostname()`, `platform = std::env::consts::OS`.
  - `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)] #[serde(rename_all = "camelCase")] pub struct DeviceRow { pub id: String, pub key: String, pub name: String, pub hostname: String, pub platform: String, pub address: String, pub last_seen: Option<u64>, pub is_main: bool, pub is_self: bool }`
  - `pub fn device_rows(n: &NetworkConfig, me: &PairedAssistant) -> Vec<DeviceRow>`: `me` first (`is_self`, `is_main` when role is `Main`), then each live member in list order, `is_main` when role is `Assistant` and its key is `n.main_key`. Tombstones never appear.
- Produces, in `core/src/net/mod.rs`: `NetworkStatus.devices: Vec<group::DeviceRow>` (`#[serde(default)]`).

- [ ] **Step 1: Write the failing tests** in `group.rs`'s `mod tests`, with an `entry(key, name)` helper:
  - `add_stamps_generation_and_keeps_the_id_of_a_known_key`: add A, add A again with another name → one member, same id, `generation == 2`, `version == 2`.
  - `remove_makes_a_tombstone`: add A, remove "A" → `members` empty, `removed[0].generation == 2`, `!removed[0].mirror`, `version == 2`; `remove(&mut g, "nope") == None`.
  - `a_stale_tombstone_does_not_remove_a_device_paired_again` (Review Focus 3): add A (gen 1), remove A (gen 2), add A again (gen 3); `merge_copy` with theirs `removed = [A gen 2]` → returns false, A still a member.
  - `a_newer_tombstone_removes_and_raises_the_version_above_both`: g has A gen 1 at version 1; theirs `{version: 7, removed: [A gen 4]}` → true, A removed, `g.version == 8`.
  - `a_tie_goes_to_removed`: A gen 3 live, theirs tombstone gen 3 → removed.
  - `members_from_a_copy_are_never_added`: theirs `members = [B]` → false, B absent.
  - `adopt_drops_my_own_entry`: copy members `[me, A]` → `assistants == [A]`.
  - `may_follow_cases`: seen with B live → true; seen with B in `removed` → false; no seen and empty `assistants` → true; no seen and `assistants = [A]`, key B → false.
  - `join_rejects`: each of the four errors above, exact strings; and a good join sets role, host, port 4127 for port 0, key, `redirect_port == 0`.
  - `device_rows_marks_self_and_main_and_hides_tombstones`.
- [ ] **Step 2: Run** `cargo test -p maya-core net::group` → FAIL (module missing).
- [ ] **Step 3: Implement** the fields and `group.rs` as specified; add `generation: 0` to each literal the compiler reports.
- [ ] **Step 4: Run** `cargo test -p maya-core` → PASS.
- [ ] **Step 5: Commit** `feat(net): keep the group of trusted devices in every config`.

### Task 2: The wire frames

**Files:**
- Modify: `core/src/net/protocol.rs` (`Up::Hello`, `Down`, `Request`, `CommandKind::Promote`)

**Interfaces:**
- Consumes: `group::GroupCopy` (Task 1).
- Produces:
  - `Up::Hello { .., #[serde(default, skip_serializing_if = "Option::is_none")] group: Option<GroupCopy> }`
  - `Down::Group(GroupCopy)`: the main's group, including the main's own entry (from `group::me`) among `members`.
  - `Request::RemoveMember { key: String }`
  - `CommandKind::Promote { list, summary, #[serde(default)] removed: Vec<PairedAssistant>, #[serde(default)] version: u64 }`

- [ ] **Step 1: Write the failing tests** in `protocol.rs`'s tests:
  - `group_round_trips`: `decode_down(&encode(&Down::Group(g)))` equals it, and the JSON has `"type":"group"`.
  - `an_old_hello_decodes_with_no_group`: a hello JSON without `group` decodes with `group == None`.
  - `an_old_promote_decodes_with_no_tombstones`: a promote command JSON without `removed`/`version` decodes with empty `removed`, `version == 0`.
  - `remove_member_request_round_trips`.
- [ ] **Step 2: Run** `cargo test -p maya-core net::protocol` → FAIL.
- [ ] **Step 3: Implement** the variants and fields; fix the exhaustive matches and `Up::Hello` destructuring the compiler reports (`server.rs` `admit`, client `handshake`) by ignoring the new fields for now (`..`).
- [ ] **Step 4: Run** `cargo test --workspace` → PASS.
- [ ] **Step 5: Commit** `feat(net): frames for the group`.

### Task 3: The main keeps, sends and merges the group

**Files:**
- Modify: `core/src/net/server.rs` (`Notify`, `Server`, `start_with`, `admit`, `pair`/`record_pairing`, `remove_assistant`, `handle`, `leave`, `status`, `promote_inner`)
- Modify: `core/src/net/mod.rs` (`paired_offline` ignores nothing new; it takes live members only, as today)
- Modify: `Notify` implementors: `src-tauri/src/lib.rs:204` (`TauriNetNotify`), `server.rs` `Counter`, `mobile/tests/round_trip.rs:180`, `cli/tests/run_end_to_end.rs:26`
- Test: `core/src/net/server.rs` `mod tests`

**Interfaces:**
- Consumes: Task 1 (`GroupCopy`, `add`, `remove`, `merge_copy`, `me`), Task 2 frames.
- Produces:
  - `Notify::paired(&self, assistant: &PairedAssistant, group: &GroupCopy) -> Result<(), String>` and `Notify::paired_list_changed(&self, group: &GroupCopy) -> Result<(), String>`: `group` is the stored form (no main entry). `TauriNetNotify` saves `assistants`, `removed`, `group_version` from it.
  - `Server::new(group: GroupCopy, own: PairedAssistant, main_name: String)`; private fields `removed: Vec<PairedAssistant>`, `version: u64`, `own: PairedAssistant`; `fn stored(&self) -> GroupCopy`, `fn wire(&self) -> GroupCopy` (stored plus `own` in `members`), `fn send_group_to_all(&self)` (queues `Down::Group(self.wire())` on every `conns` entry).
  - `start_with` seeds from `group::of_config(&c)` and `group::me(&c, &identity.public_b64())`.
  - `ServerHandle::remove_member(&self, key: &str) -> Result<(), String>`: refuses `own.key` with `pub const MAIN_NOT_REMOVABLE: &str = "The main cannot be removed; make another Maya the main first."`; otherwise `group::remove`, saves (save before change, as today), sends `Bye "removed"` to that connection, drops its board/peer, sends the group to all, emits status and board. `remove_assistant(id)` looks up the key and calls it.
  - In `admit`, for a key that connected (not paired): under the lock, `merge_copy` the hello's `group`; if changed, save and send the group to the others; if the connecting key is now a tombstone, `bye "removed"`. After `Welcome`, `send_down(ch, &Down::Group(wire))`.
  - `record_pairing` goes through `group::add` (so a tombstone with the same key is dropped and the generation stamped); after the save, the group goes to all.
  - In `handle`, `Up::Request { request: Request::RemoveMember { key }, .. }` is answered before the mirror gate, for any connected member: `remove_member` then `Down::Response`.
  - `promote_inner` hands over `promote::handover(&s.stored(), id, &s.own)` (Task 5 renames the helper; until then build the same list inline).

- [ ] **Step 1: Write the failing tests** with the existing helpers (`test_server_with`, `pair_client`, `connect_as`, `recv`, `wait_until`, `Counter`):
  - `a_member_gets_the_group_after_welcome`: pair A → after `Welcome`, the next frame is `Down::Group` whose members hold A and the main's own entry (`id == key`).
  - `pairing_sends_the_group_to_everyone_connected`: A connected, pair B → A receives a group with B.
  - `remove_member_tombstones_and_tells_the_others`: A and B connected; `remove_member(A.key)` → A gets `Bye "removed"`, B gets a group with A in `removed`, `Counter` saved a group with A in `removed`.
  - `a_removed_key_is_refused_after_a_restart` (Review Focus 1): start a server seeded with A as a tombstone → A connecting gets `Bye "removed"`.
  - `a_member_removed_while_away_is_told_on_return` (Review Focus 2): A paired, disconnect, `remove_member(A)`, A connects with a hello whose `group` has A live at a lower generation → `Bye "removed"`.
  - `a_copy_from_a_member_removes_but_never_adds`: B connects with a copy whose `removed` holds A (higher generation) and whose `members` hold an unknown C → A removed, C absent, version raised.
  - `remove_member_request_works_without_mirroring`: a non-mirroring assistant sends `Request::RemoveMember` for B → `Response { ok: true }`, B removed.
  - `the_main_cannot_be_removed`: request with the main's own key → `Response { ok: false, error: MAIN_NOT_REMOVABLE }`.
- [ ] **Step 2: Run** `cargo test -p maya-core net::server` → FAIL.
- [ ] **Step 3: Implement** as specified; keep `Counter`'s under-lock check for both save methods.
- [ ] **Step 4: Run** `cargo test --workspace` → PASS.
- [ ] **Step 5: Commit** `feat(net): the main shares the group and removes from it`.

### Task 4: A device keeps the group and follows only a trusted main

**Files:**
- Modify: `core/src/net/client.rs` (`ClientNotify`, `handshake`, `handle`, `reconnect`, constants)
- Modify: `ClientNotify` implementors that must change for the new `promoted` signature (Task 5 uses it): `src-tauri/src/net_app.rs:149`, test `Leader`, `Mobile` in `server.rs`
- Test: `core/src/net/client.rs` and `server.rs` tests

**Interfaces:**
- Consumes: Task 1 (`of_config`, `may_follow`), Task 2 frames.
- Produces:
  - `ClientNotify::group(&self, _group: GroupCopy) {}` (default no-op): the app saves it with `group::adopt`.
  - `ClientNotify::promoted(&self, group: GroupCopy, summary: Option<String>) -> Result<u16, String>` (was `Vec<PairedAssistant>`).
  - `pub const NOT_IN_GROUP: &str = "The main Maya moved to a Maya this one does not trust; pair with it.";` `reconnect` stops on it, as on `REMOVED`.
  - `handshake` sends `group: Some(group::of_config(config))` in `Hello`.
  - In `handle`: `Down::Group(g)`: when `g.removed` holds this Maya's key → `Err(REMOVED.into())`; otherwise remember it for this connection and call `notify.group(g)`. `Down::Move`: when `!group::may_follow(config, seen.as_ref(), &key)` → `Err(NOT_IN_GROUP.into())` without calling `notify.moved`. `handle` gains the connection's config, this Maya's key and a `&mut Option<GroupCopy>`.
  - `Down::Command { Promote { list, removed, version, summary } }` calls `notify.promoted(GroupCopy { version, members: list, removed }, summary)`.

- [ ] **Step 1: Write the failing tests:**
  - `the_device_saves_the_group_it_is_sent`: a `FakeClientNotify` recording `group` calls; connect to a test main → one call whose members hold the main's entry.
  - `a_group_that_removes_me_stops_the_link`: `fake_main` sending `Welcome` then a group with the client's key in `removed` → `run_once` returns `Err(REMOVED)`.
  - `a_move_to_an_unknown_key_is_refused`: `fake_main` sending `Welcome`, a group without key K, then `Move { key: K }` → `Err(NOT_IN_GROUP)`, `moved` never called.
  - `a_move_from_an_old_main_with_no_group_is_followed`: no group frame, empty `assistants`, `Move` → `moved` called (back-compat).
  - `reconnect_stops_on_not_in_group`.
- [ ] **Step 2: Run** `cargo test -p maya-core net::client` → FAIL.
- [ ] **Step 3: Implement** as specified.
- [ ] **Step 4: Run** `cargo test --workspace` → PASS (the existing promote end-to-end test at `server.rs:3063` must still pass: the promoted Maya is in the group every follower received).
- [ ] **Step 5: Commit** `feat(net): a device keeps the group and follows only a trusted main`.

### Task 5: Make main hands over the whole group and keeps it

**Files:**
- Modify: `core/src/net/promote.rs`
- Modify: `core/src/net/server.rs` (`promote_inner`)
- Modify: `src-tauri/src/net_app.rs:192` (`TauriClientNotify::promoted`), `src-tauri/src/lib.rs:865` (`network_promote`)

**Interfaces:**
- Consumes: Task 1 `GroupCopy`, Task 4 `promoted` signature.
- Produces:
  - `pub fn handover(group: &GroupCopy, promoted: &str, me: &PairedAssistant) -> GroupCopy` (replaces `handover_list`): members minus the promoted id, plus `me`; `removed` and `version` as they are. `own_entry` is replaced by `group::me` (stable id = key).
  - `pub fn as_main(c: &mut Config, group: GroupCopy)`: role `Main`, `assistants`/`removed`/`group_version` from it, main fields cleared, `end_redirect()`.
  - `pub fn as_assistant_of(c: &mut Config, host: &str, port: u16, main_key: &str)`: role `Assistant`, pins the key, **keeps** `assistants` and `removed`, sets `redirect_port = c.listen_port()` (the port it listened on as main), `listen = false`.

- [ ] **Step 1: Write the failing tests** in `promote.rs`:
  - `handover_keeps_tombstones_and_version_and_adds_me`.
  - `the_old_main_keeps_the_group_and_its_port_for_redirects`: config as main on port 4200 with members `[A, B]` → after `as_assistant_of`, `assistants == [A, B]`, `redirect_port == 4200`.
  - `the_new_main_takes_the_group_and_stops_redirecting`: `redirect_port = 4200` before `as_main` → 0 after; tombstones and version taken.
- [ ] **Step 2: Run** `cargo test -p maya-core net::promote` → FAIL.
- [ ] **Step 3: Implement**; update the two app callers.
- [ ] **Step 4: Run** `cargo test --workspace` → PASS.
- [ ] **Step 5: Commit** `feat(net): Make main hands over the whole group`.

### Task 6: The old main redirects members it promoted away from

**Files:**
- Modify: `core/src/net/server.rs` (redirect mode)
- Modify: `core/src/net/mod.rs` (`NetChange`, `network_change`, `NetworkState`)
- Modify: `src-tauri/src/lib.rs` (`apply_config_change`, start/stop of the listener, `set_config`)
- Test: `server.rs`, `mod.rs` tests

**Interfaces:**
- Consumes: Tasks 1–5.
- Produces:
  - `pub struct MoveTarget { pub host: String, pub port: u16, pub key: String }`
  - `pub fn start_redirect(config_view: Arc<Mutex<NetworkConfig>>, port: u16, identity: Arc<Keypair>, to: MoveTarget) -> Result<ServerHandle, String>`: a `Server` with `redirect: Some(to)` and a private no-op `Notify`. In `admit`, a redirecting server refuses pairing, refuses unknown and tombstoned keys as a main does, and for a live member sends `Welcome`, `Down::Group(wire)`, `Down::Move { host, port, key }`, then closes. It merges nothing and keeps no boards.
  - `NetChange::{StartRedirect, StopRedirect, RestartRedirect}`; `network_change` compares `redirect_of(c) = (role == Assistant && redirect_port != 0 && !main_key.is_empty()).then(|| (redirect_port, main_host, main_port, main_key))` before and after.
  - `NetworkState.redirect: Option<ServerHandle>`; the app starts it after `StopMain` frees the port, with the same bind retry as `start_main` (`lib.rs:954`).
  - `pub fn settle_role_change(before: &NetworkConfig, after: &mut NetworkConfig)` in `core/src/net/mod.rs`: when the role changed, `after.end_redirect()`. Called only from the `set_config` command (the UI's role switch), never from `update_config`, which `network_promote` uses.
  - `end_redirect()` is also called by: `net_app::pair` (code pairing), `group::join` (Task 1), `network_forget_main`, `TauriClientNotify::removed`.

- [ ] **Step 1: Write the failing tests:**
  - `a_returning_member_is_moved_to_the_new_main` (`server.rs`): start a redirect server holding A's key with target T, whose group also holds T; A runs `run_once` against it → `moved(T.host, T.port, T.key)` called.
  - `the_redirect_hands_over_a_main_paired_after_the_device_left`: A's own config lacks T; the redirect's group has T → A still moves (it adopts the group first).
  - `a_stranger_gets_no_redirect`: unknown key → `Bye "removed"`, no `Move`.
  - `network_change_starts_and_stops_the_redirect` (`mod.rs`): Main → Assistant with `redirect_port` set → `[StopMain, StartRedirect]` (with `StartAssistant`); main fields changed (a later promotion) → `RestartRedirect`; `redirect_port` 0 → `StopRedirect`.
  - `a_hand_join_ends_the_redirect` (Review Focus 4): config redirecting to T; `group::join` to member U → `redirect_port == 0` and `network_change` yields `StopRedirect`.
  - `switching_role_by_hand_ends_the_redirect`: `settle_role_change` Assistant → Off clears it; same role leaves it.
- [ ] **Step 2: Run** `cargo test -p maya-core` → FAIL.
- [ ] **Step 3: Implement** core, then the app's start/stop in `apply_config_change` and the `end_redirect` calls listed above.
- [ ] **Step 4: Run** `cargo test --workspace` → PASS.
- [ ] **Step 5: Commit** `feat(net): the old main points returning devices to the new main`.

### Task 7: Desktop app: save the group, Join, Remove, migration, status

**Files:**
- Modify: `core/src/config.rs:515` (`Config::migrate`)
- Modify: `src-tauri/src/net_app.rs` (`TauriClientNotify::group`, `lane`)
- Modify: `src-tauri/src/lib.rs` (`AppState.lane`, `network_status_of`, new commands, handler registration at `lib.rs:1775-1781`)

**Interfaces:**
- Consumes: Tasks 1–6.
- Produces:
  - `Config::migrate`: role `Assistant` with a `main_key` and no member holding that key → push `PairedAssistant { id: new_id(), name: main_host, hostname: main_host, platform: "", key: main_key, address: main_host, last_seen: None, mirror: false, generation: 0 }`.
  - `TauriClientNotify::group(g)`: `update_config(|c| group::adopt(&mut c.network, &my_key, &g))`. `TauriClientNotify::lane()` returns `Some(state.lane.clone())`; `AppState.lane: Arc<RequestLane>`.
  - `network_status_of` sets `status.devices = group::device_rows(&config.network, &group::me(&config.network, &my_key))` for every role.
  - `#[tauri::command] fn network_remove_member(app: AppHandle, state: TauriState<AppState>, key: String) -> Result<NetworkStatus, String>`: with a running main server, `remove_member(&key)`; as an assistant, `state.lane.request(None, Request::RemoveMember { key }, REQUEST_TIMEOUT)`, mapping `NOT_CONNECTED` to `"Connect to the main Maya to remove a device."`; otherwise that same error.
  - `#[tauri::command] fn network_join(app: AppHandle, state: TauriState<AppState>, key: String, host: String, port: u16) -> Result<NetworkStatus, String>`: validates with `group::join` on a clone, then `update_config` with it (the client restarts through `network_change`).
  - `network_remove_assistant` (main rows) keeps its name and now tombstones through `remove_member`; with the server down it tombstones in config with `group::remove`.

- [ ] **Step 1: Write the failing tests** in `config.rs`:
  - `an_assistant_from_0_44_lists_its_main` (Review Focus 5): JSON of a 0.44 assistant config (role, main host/port/key, no members) → after `load`, `assistants` holds one entry with that key, and `device_rows` marks it `is_main`.
  - `migration_does_not_duplicate_the_main`.
- [ ] **Step 2: Run** `cargo test -p maya-core config` → FAIL.
- [ ] **Step 3: Implement** the migration and the app wiring.
- [ ] **Step 4: Run** `cargo test --workspace` and `cargo build -p maya` → PASS.
- [ ] **Step 5: Commit** `feat(app): join and remove devices in the group`.

### Task 8: Settings › Devices on the desktop

**Files:**
- Create: `src/devices.ts`, `src/devices.test.ts`
- Modify: `src/settings.ts` (types at 27-74, `SETTINGS_GROUPS` 345-356, `renderSettings` 592-639 and the Network block 1459-1535, handlers 1996-2040, paint at 1864)
- Modify: `src/settings.test.ts` (move the paired-list tests at 531, 758 and `describe("Make main")` 1001 to `devices.test.ts`)

**Interfaces:**
- Consumes: `NetworkStatus.devices` (Task 7), commands `network_remove_member`, `network_join`, `network_remove_assistant`, `network_set_mirror`, `network_promote`.
- Produces:
  - TS `export interface DeviceRow { id: string; key: string; name: string; hostname: string; platform: string; address: string; lastSeen: number | null; isMain: boolean; isSelf: boolean }` and `devices: DeviceRow[]` on `NetworkStatus` (default `[]`).
  - `export interface DevicesModel { joining: string | null; joinHost: string; joinPort: number; promote: string | null; promoting: boolean }`
  - `export interface DevicesHandlers { onRemoveMember(key: string): void; onJoinAsk(key: string): void; onJoin(key: string, host: string, port: number): void; onJoinCancel(): void; onRemoveAssistant(id: string): void; onMakeMain(id: string): void; onMirror(id: string, on: boolean): void; onPromote(id: string): void; onPromoteCancel(): void }`
  - `export function renderDevices(net: NetworkStatus, m: DevicesModel, h: DevicesHandlers, nowMs: number): HTMLElement`:
    - Always the "This device" row first (from `net.devices`, `isSelf`).
    - On a main: the rows of `net.assistants`, moved from `settings.ts:1459-1535` with their selectors kept (`data-action=remove-assistant`, `make-main`, `mirror`, `data-for=promote-confirm`).
    - Otherwise: the other `net.devices` rows, a "Main" badge on `isMain`; `button[data-action=remove-member][data-key]` on rows that are neither self nor main, disabled with title `"Connect to the main Maya to remove a device."` while `!net.assistant.connected`; `button[data-action=join][data-key]` on rows that are not self, not `platform === "android"`, and not the connected main. Join reveals `input[name=join-host]` (prefilled with the row's `address`), `input[name=join-port]` (4127) and a confirm `button[data-action=join-confirm]`.
    - Each row carries `data-key` (or `data-id` on main rows) so `morph` keeps it.
  - `SETTINGS_GROUPS` gains `["devices", "Devices"]` right after `network`; the Network section on a main shows the hint `"Paired devices are in Settings › Devices."` where the list was.
  - The Settings paint patches in place with `morph` instead of `panel.replaceChildren`, keeping the focus behaviour its tests pin.

- [ ] **Step 1: Write the failing tests** in `devices.test.ts`:
  - `shows this device first, then the main with a badge` (assistant status).
  - `offers Remove on other members and disables it while the main is unreachable`.
  - `offers no Join on a phone, on this device, or on the connected main`.
  - `Join asks for host and port prefilled from the last address, then calls onJoin` (click join → inputs `"192.168.1.5"`, `"4127"`; click confirm → `onJoin(key, "192.168.1.5", 4127)`).
  - `on a main it lists the paired devices with Remove, Make main and the phone switch` (moved tests).
  - In `settings.test.ts`: `the Devices section follows Network in the sidebar`; `a status repaint keeps the same row element` (render, change `lastSeen`, repaint through the settings paint → `===` same row node).
- [ ] **Step 2: Run** `pnpm vitest run src/devices.test.ts src/settings.test.ts` → FAIL.
- [ ] **Step 3: Implement** `src/devices.ts`, wire handlers in `initSettings` (`invoke("network_remove_member", { key })`, `invoke("network_join", { key, host, port })`), and switch the paint to `morph`.
- [ ] **Step 4: Run** `pnpm vitest run` and `pnpm tsc --noEmit` → PASS.
- [ ] **Step 5: Commit** `feat(ui): Settings › Devices`.

### Task 9: The phone keeps the group and shows Devices

**Files:**
- Modify: `mobile/src/hub.rs` (`HubClientNotify::group`, `Hub::remove_member`, `Hub::join_member`, `Hub::status`)
- Modify: `mobile/src/commands.rs`, `mobile/src/lib.rs:126-128`
- Modify: `src/mobile/network.ts` (`PhoneGroup`, `PHONE_GROUPS` line 20, `CARDS` 203, `NetworkHandlers`), `src/mobile/network.test.ts`
- Test: `mobile/tests/round_trip.rs`

**Interfaces:**
- Consumes: Tasks 1–4, `renderDevices` (Task 8).
- Produces:
  - `HubClientNotify::group(g)`: `hub.update_config(|c| group::adopt(&mut c.network, &my_key, &g))`.
  - `Hub::remove_member(&self, key: &str) -> Result<NetworkStatus, String>` via `self.request(None, Request::RemoveMember { key })`, `NOT_CONNECTED` → `"Connect to the main Maya to remove a device."`.
  - `Hub::join_member(self: &Arc<Self>, key: &str, host: &str, port: u16) -> Result<NetworkStatus, String>`: `group::join` on the config, then `start_client`.
  - `Hub::status` fills `devices` with `group::device_rows`.
  - Commands `network_remove_member(hub, key: String)` and `network_join(hub, key: String, host: String, port: u16)`, registered.
  - `PhoneGroup` gains `"devices"` after `"network"`; its card renders `renderDevices` (phone is never a main) with handlers invoking those commands.

- [ ] **Step 1: Write the failing tests:**
  - `round_trip.rs`: `the_phone_saves_the_group_and_lists_the_main` (join a test main → the phone's config `assistants` holds the main's key; `status().devices` marks it `is_main`).
  - `round_trip.rs`: `the_phone_removes_a_member_through_the_main`.
  - `network.test.ts`: `the Devices card lists the group and calls network_join`.
- [ ] **Step 2: Run** `cargo test -p maya-mobile` and `pnpm vitest run src/mobile` → FAIL.
- [ ] **Step 3: Implement**.
- [ ] **Step 4: Run** both again → PASS.
- [ ] **Step 5: Commit** `feat(mobile): Devices on the phone`.

### Task 10: `maya devices` on the command line

**Files:**
- Modify: `cli/src/args.rs` (`Cmd`, `parse`, `USAGE`), `cli/src/lib.rs:14` (dispatch), `cli/src/commands.rs`, `cli/src/notify.rs:33` (`CliNotify::group`)

**Interfaces:**
- Consumes: Task 1 (`device_rows`, `join`, `me`, `adopt`).
- Produces:
  - `Cmd::Devices(DevicesOp)`, `pub enum DevicesOp { List, Join { device: String, host: String, port: u16 } }`. Usage lines: `maya devices` and `maya devices join <name or id> <host>[:port]`.
  - `pub fn devices(claude_dir: &Path) -> Result<String, String>`: one line per row, `"<name>  <platform>  <address>"` plus `"  (main)"` / `"  (this device)"`.
  - `pub fn devices_join(claude_dir: &Path, device: &str, host: &str, port: u16) -> Result<String, String>`: finds the member by id, or by name ignoring case (two matches → `"Two devices are called <name>; use its id."`), runs `group::join` with this machine's key, saves, and returns `"Joined <name>."` (a running `maya run` picks the change up through `run_cmd::reload_network`).
  - `CliNotify::group(g)` saves with `group::adopt`.
  - No `maya devices remove`: removal needs the live link; the README says to remove from any desktop or phone. (Plan call.)

- [ ] **Step 1: Write the failing tests:** `args.rs`: `parses_devices_and_devices_join` (port defaults to 4127; `host:4200` splits); `commands.rs`: `devices_lists_main_and_self`, `devices_join_by_name`, `devices_join_unknown_name_fails`.
- [ ] **Step 2: Run** `cargo test -p maya-cli` → FAIL.
- [ ] **Step 3: Implement**.
- [ ] **Step 4: Run** `cargo test --workspace` → PASS.
- [ ] **Step 5: Commit** `feat(cli): maya devices`.

### Task 11: Docs and release

**Files:**
- Modify: `README.md` (Network section ~385-460: Trust, Make main, a new Devices bullet, Outside the LAN unchanged; Android section ~206-244: replace "If the main's Wi-Fi address changes, join again with the new one" with Devices › Join; headless section: `maya devices`)
- Modify: `RELEASE_NOTES.md`, version files via `scripts/set-version.sh`

- [ ] **Step 1:** Update README to describe: the group on every Maya, pairing once, following a promoted main (also after being offline, through the old main), Join with no code, Remove from any Maya removes everywhere, the main cannot be removed (make another main first), only a code pairing on the main adds a device.
- [ ] **Step 2:** Commit `docs: the trusted group of devices`.
- [ ] **Step 3:** `git fetch origin && git show origin/main:package.json | grep version` → bump from that version's minor (expected `0.45.0`): `sh scripts/set-version.sh 0.45.0`; add `# 0.45.0` with `## New` bullets in plain words (Settings › Devices; pair once, no code after a new main; Join; Remove everywhere; phone Devices; `maya devices`).
- [ ] **Step 4:** Run `cargo test --workspace`, `pnpm vitest run`, `pnpm tsc --noEmit` → PASS.
- [ ] **Step 5:** Commit `chore(release): 0.45.0`.
