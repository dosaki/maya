# Your Mayas: one trusted group of devices

## Intent

Today a Maya trusts other Mayas one pairing at a time, and that trust does
not survive a change of roles:

- An assistant knows one main: `NetworkConfig.main_host`, `main_port` and
  `main_key` (`core/src/config.rs`). Pairing with another main replaces it;
  nothing remembers the mains it knew before.
- Only the main keeps a list (`NetworkConfig.assistants`). Becoming an
  assistant clears it (`promote::as_assistant_of`); becoming the main by
  hand starts from an empty list.
- Make main hands the list over (`CommandKind::Promote`, `Down::Move`), but
  only to the Mayas connected at that moment. Any other switch of roles
  means typing host, port and a fresh code on every device again.

The user switches the main between machines and has to pair everything
again each time. This design makes every Maya remember every device in the
user's group, so a device paired once never needs a code again, whichever
machine is the main.

It is also the first of three steps toward reaching Maya from outside the
LAN without a VPN, because the user keeps Mullvad on on every device and
Android runs one VPN at a time:

1. **This design:** one trusted group, kept on every Maya.
2. Later: reach the main through a relay that both sides dial out to, so
   addresses stop mattering.
3. Later: a laptop that views and controls the main's board, as the phone
   does.

Steps 2 and 3 are out of scope here, but this design must not block them:
trust is by key, never by address.

## Goals

- Pair a device once, with a code, and it belongs to the group: every Maya
  in the group trusts it, and it trusts every Maya in the group.
- A device follows a new main with no code when the current main promotes
  it (Make main), including a device that was offline at the handover.
- Joining a main that is already in the group takes no code: pick it.
- Settings › Devices on every Maya (main, assistant, phone) lists the
  group, with a way to remove a device.
- Removing a device removes it from the whole group: no Maya trusts it
  any more, including those offline at the time, once they reconnect.

## Non-goals

- Reaching the main from outside the LAN (step 2).
- A laptop viewing and controlling the main's board (step 3).
- Changing pairing itself: SPAKE2 over a six-digit code on the main, then
  pinned keys and Noise, all stay as they are.
- A phone as main: still not allowed.

## Anti-goals

- **A device must never follow a main on its own say-so.** A Maya made the
  main by hand in Settings does not pull anyone along. A device moves to a
  new main only when (a) the main it trusts promoted it, or (b) the user
  told that device to join it. (The user's rule; it is what makes the
  shared list safe enough.)
- A removed device must not come back through a Maya that was offline when
  it was removed.
- No secret leaves a device: the group list carries public keys, names and
  labels only, as `Promote` does today.

## Approaches

1. **The main holds the list; every Maya keeps a numbered copy (chosen).**
   The main is the authority for the member list. Pairing adds; Remove,
   pressed on any Maya, goes through the main. The main sends the new list
   to every connected Maya; an offline one gets it when it reconnects.
   Removed devices stay as tombstones. A Maya takes a list only from its
   current main.
   *Trade-off:* a trusted device can do what a trusted device can; one that
   is stolen and made the main by its own hand pulls nobody along (anti-goal
   above), so the risk is no worse than today.
2. **A group key that signs each member.** Harder to forge a list, but the
   group key must live somewhere, move with the main and be recovered when
   that device is lost: a lot of new security code for little gain.
3. **Pairwise introductions, no shared list.** Every device keeps its own
   trust; removal must reach each one; no single list for Devices to show.

The user chose 1: "It's not so bad because a device can't become a main and
make everyone else follow it. The current main must promote it, or the user
must make each device join a main."

## Design

### The group, on every Maya

Every Maya keeps the group in its config, whatever its role:

- **Members:** one entry per device, the existing `PairedAssistant` fields
  (id, name, hostname, platform, key, address, last seen, mirror) plus
  `removed` (a tombstone). The key is the identity; name, hostname and
  address are labels.
- **Version:** a number the main raises on every change to the list.
- **Main:** which member is the main now (by key), and where it was last
  reached (host, port). This replaces `main_host`, `main_port` and
  `main_key` as the source of truth; the fields can stay as they are if the
  builder prefers, filled from it.

A Maya's own entry is in its list too, so the list is the same on every
Maya.

### Joining the group: pairing

Pairing stays as it is: the main shows a code; a device types host, port
and code; SPAKE2 proves both keys. After it, the main adds the device to
the list, raises the version and sends the new list to every connected
Maya, the new device included. The new device keeps the whole list, so it
trusts every member from then on.

Pairing again with a removed device's key (with a new code) clears its
tombstone: a code is the only way back. So that a stale copy cannot remove
it again, each entry carries a generation that the main raises when it
adds, removes or pairs again; in a merge the entry with the higher
generation wins, and on a tie removed wins. (My call.) A removal the main
takes from a member's copy gets the main's own next version as its
generation, so a copy with a forged, huge number cannot keep out a device
that pairs again.

### Keeping the copies in step

- The main sends the list (version and members) when a Maya connects and
  whenever it changes. A Maya replaces its copy with a list from its
  current main whose version is higher than its own.
- When a Maya connects to a main, it sends its own copy too. The main
  merges it into its list entry by entry, by generation (above). If that
  changed anything, the main raises its own version by one (it never
  takes the copy's) and sends the list out. This is what keeps a removal from being
  undone by a Maya that was away. Entries the main never had are not
  added (see "Only a main's own pairing adds a member").
- A Maya never takes a list from anyone but its current main. When the
  user joins a main by hand (Join or a code), that main becomes the
  authority, and its list replaces the device's copy after the merge,
  whatever the two versions were.
- A main still refuses a key that is not in its list before any frame is
  exchanged, as today (`server.rs`): a list a device carries never gets it
  in.

### Removing a device

Devices shows Remove beside each member but the Maya itself and the main,
on every Maya. To remove the main, make another Maya the main first. (My
call.) On the main, it marks the member removed, raises the version, sends
the list, and closes the removed device's connection with `Bye` "removed"
as today. On an assistant or a phone, Remove asks the main to do it (a new
request); with no main connected, the button is disabled and says why.

A Maya that receives a list in which it is itself removed drops its link to
the main and shows that it was removed from the group, as `REMOVED` does
today.

A Maya refuses a connection from a removed key, whatever its role.

### Following a promoted main

Make main works as today for the Mayas connected at the time: the main
sends `Promote` with the list to the new main, and `Move` to everyone else.
Two changes:

- The list it hands over is the whole group with its version, and the new
  main keeps raising that version.
- **A Maya offline at the handover catches up through the old main.** The
  old main, now an assistant of the new main, keeps a listener on its old
  port that answers only group members, with its member list and `Move`
  to the new main (host, port, key). A device that comes back connects
  there as before, takes the list (the old main was its trusted main, and
  the list carries a main paired after the device went away), checks that
  the new main's key is in it, and moves. This is the user's rule (the
  main it trusted promoted the new one), with no new signing key.
  The listener points only at the main it promoted. It follows a later
  promotion of that main (the new main hands the role on: the old main is
  then told the next one, as a connected assistant is), and stops when
  its Maya becomes the main again, joins a main by hand (Join or a code),
  is removed, or turns networking off. (My call: see "Calls made" below.)

### Joining a main by hand, with no code

When the user wants a device to follow a main it was not promoted to (they
made a Maya the main by hand, say, or the old main is gone), Devices on
that device offers **Join** beside each member that is a computer. Join
asks for the member's host and port, prefilled with its last-known address
and port, and connects with the pinned key: no code. If that Maya is not a
main, or does not answer, Join says so.

Settings › Network keeps "Assistant to a main Maya" with host, port and
code for a main that is not yet in the group.

### Settings › Devices

A new Devices section in Settings, on desktop and phone:

- One row per member that is not removed: name (with hostname when names
  clash, as today), platform, last-known address, when it was last seen,
  and a badge on the main and on "This device".
- On the main, the phone rows keep their **Sees and controls all sessions**
  switch (moved here from Network, or shown in both: builder's call).
- Remove beside each row but This device; Join beside computers on a Maya
  that is not the main.
- Repaint in place with `morph` (see CLAUDE.md's lore), since `last seen`
  changes often.

Settings › Network keeps the role, port, name, pairing code and status
line, and loses the paired-assistant list, which moves to Devices.

The `maya` command gets `maya devices` (list) and `maya devices join
<name or id> <host>[:port]` (no code), to match. Remove is in Settings ›
Devices, on a computer or on the phone.

### Migration

On first start of the new version:

- A main turns its `assistants` into members, adds itself, version 1.
- An assistant or a phone with a `main_key` makes a list of itself and the
  main, version 0; the main's list replaces it at the first connection.
  (One the main had lost is refused before that, as today, and pairs
  again.)
- Mayas on older versions keep working with a main as today but do not get
  the list (README already says to run the same version everywhere). An
  older main sends no list, and a newer assistant keeps working with the
  copy it has.

## Constraints

- The protocol stays backward-readable: new `Down`/`Up` variants and
  `#[serde(default)]` fields, as the protocol already does for `mirror`
  and `caps`. Frames stay within `MAX_FRAME`.
- Keys stay where they are (Keychain, Credential Manager, GNOME Keyring,
  or `identity.key`); the list holds public keys only.
- The identity key is X25519, which cannot sign, which is why catch-up
  goes through the old main's authenticated link and not through a signed
  note any member could pass on.
- Tests first (the repo's practice): the list's merge and version rules,
  tombstones, a promotion with a device offline, removal from an
  assistant, migration of each role's config, the Devices view.

## Calls made (not discussed with the user)

- Catch-up for an offline device is through the old main's listener, not
  a signed handover note that any member could relay. I said "a handover
  note signed by the main it trusted" in the conversation; signing needs a
  second key per device, since the identity key is X25519. Through the old
  main, the proof is the authenticated link itself. Cost: a device that
  was offline catches up only if the old main is reachable; if not, Join
  (no code) does it.
- Every Maya, phones included, can press Remove; it goes through the main.
  No Remove on the main's row.
- Entries carry a generation, so pairing again beats an older removal.
- A merge raises the main's own version by one and stamps the removals it
  takes with it; a hand join makes the joined main the authority.
- The old main's redirect sends its list with `Move`, follows only
  promotions, and stops on a hand join, a code pairing, removal or Off.
- A main keeps refusing unknown keys before any frame, as today.
- Join asks for host and port, prefilled from the last-known address,
  because a member's address is only a label until step 2.
- `maya devices` and `maya devices join` for the command line; no remove
  there.
- The phone switch's place (Devices, Network or both) is left to the
  builder.

## Only a main's own pairing adds a member (the user's decision)

A copy a member brings can carry removals and updated labels to the main,
never new members: an entry the main has never had is ignored in the merge.
Only a code pairing on the main (or a promotion's handover, which is the
main's own list) adds a member. So removing a stolen device cuts off
everything it could reach: it could not have slipped in a key of its own.

The user's case for why this costs little: with two groups, A (main) + B
and C (main) + D, C or D joins A's group at once if A already trusts it
(it is in A's list, not removed). Only a device A has never trusted needs
a code. The merge also does not bring D in when C joins: D pairs, or
joins later on its own if A already trusts it.

## Left to the builder

- Field and message names, and whether `main_host`/`main_port`/`main_key`
  stay as fields or are derived.
- How the old main's redirect listener shares code with `server.rs`.
- Copy for the Devices rows, buttons and errors, in the style of the
  existing Settings text.
- The release notes and README updates (README's Network and Android
  sections describe the old behaviour).
