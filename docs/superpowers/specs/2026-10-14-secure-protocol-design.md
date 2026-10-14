# A secure protocol between Mayas — design

Date: 2026-10-14
Status: implemented
Builds on: `2026-09-30-maya-remote-assistants-design.md`,
`2026-10-12-promote-assistant-design.md`,
`2026-10-14-phone-joins-a-main-design.md`

## Purpose

Make the connection between two Mayas safe on a network the user does not
trust: a public Wi-Fi, the internet behind a forwarded port, a tunnel, or a
future relay. Reaching the main from outside the LAN (Tailscale today, a
relay later) is a separate piece of work; this one makes whichever path is
used safe to use.

A Maya that pairs can drive every paired machine: it types replies into
Claude Code as the user, answers its permission questions and starts agents
with any prompt. Getting into the connection is remote code execution on
every machine, so the bar is "as safe as on the public internet", with no
reliance on the network being private.

Success: someone who can read, change, drop or inject traffic between two
Mayas cannot read any of it, cannot run anything, and cannot pair in the
user's place; a six-digit code cannot be guessed offline; a machine is
known by its key, not its address; a lost phone or a leaked key can be cut
off from Settings.

## Findings

- The main listens on `0.0.0.0:4127` (`core/src/net/server.rs`); every
  assistant, phone and CLI dials it over a plain WebSocket. Frames are JSON
  text, unencrypted, with no per-frame authentication. A machine on the
  path can read boards, conversations and attachments, and inject
  `command` frames into a connection that has already authenticated.
- Pairing sends the code in plain text in `pair`. The spec of 2026-09-30
  already notes that an active impostor at the typed host can read it and
  pair in the user's place.
- After pairing, the handshake is a mutual HMAC challenge-response over a
  32-byte token kept in `~/.claude/maya/config.json` (written 0600). The
  token never travels again; this part holds against a passive listener.
- Identity is the IP address: "same address, not connected" is the same
  machine pairing again (`server.rs:211`), and the wrong-code lockout and
  the two-handshakes-per-address limit are per IP (`server.rs:542`, `:703`).
  Behind a tunnel or relay every peer has one address.
- Make main seals the paired list, every assistant's token included, under
  the promoted machine's token (`core/src/net/promote.rs`), so the new main
  receives every other machine's secret.
- The desktop app already keeps a secret in the OS key store: the
  ElevenLabs key in the Keychain, Credential Manager or GNOME Keyring
  (`src-tauri/src/voice.rs`).
- `hello` (name, hostname, platform, caps) goes before authentication, in
  plain text.

## Decisions

- Encrypt and authenticate inside the WebSocket with the Noise protocol
  framework, not TLS: the protection runs end to end between two Mayas over
  any path, a future relay included, and needs no certificates.
- Pairing is a password-authenticated key exchange (SPAKE2) over the code.
- Protocol 2 refuses protocol 1 outright. There is no plain-text fallback
  and no transition release; every machine pairs once more.
- Each Maya's private key lives in the OS key store in the desktop app, and
  in the owner-only config on the CLI (SSH boxes and containers usually
  have no keyring) and on the phone (Android's app storage is sandboxed;
  Android Keystore cannot hand a raw key to the Noise library).
- Out of scope: the relay itself, and any change to how the main is
  reached. Local IPC (the inbox socket, the hook) is unchanged.

## Behaviour

### Identity

- On first run, or when it has none, a Maya makes one X25519 key pair. Its
  public key is its identity on the network and replaces the per-pairing
  token and the IP address as "which machine this is".
- The private key is stored as above. The desktop app reads it from the key
  store at network start; when the key store cannot be reached, the network
  stays off with "Could not reach the key store: <reason>." and nothing
  falls back to a file.
- The main keeps each paired machine's public key in its entry (replacing
  `token`); an assistant keeps the main's public key (replacing `token`).
  Public keys are not secret but must not be swapped, so they stay in the
  0600 config.
- The address is still recorded and shown ("Gnowee (macos, 192.168.55.70)
  — Connected") but no longer decides identity: a `pair` from the address
  of an existing entry is a new entry unless it presents that entry's key.

### Pairing

The main shows a six-digit code, valid for five minutes, as today. The
joining Maya types it.

1. Over the WebSocket, both sides run SPAKE2 on the code (the main as one
   side, the joining Maya as the other, with fixed identities `maya-main`
   and `maya-join`). Each derives a 32-byte key; the keys match only if
   both used the same code. A listener learns nothing about the code and
   cannot test guesses offline.
2. That key is the pre-shared key of a Noise `XXpsk3` handshake
   (`Noise_XXpsk3_25519_ChaChaPoly_SHA256`), the joining Maya as the
   initiator. Each side sends its static public key encrypted; the
   handshake completes only if the keys from step 1 matched. `psk3` mixes
   the key in only at the initiator's last message, so the main sends
   nothing that depends on the code before the joiner has proved it.
3. Inside the new channel the joining Maya sends `hello`. The main makes
   the entry with the joiner's public key, saves it, and sends `welcome`.
   The joiner saves the main's public key. Saving and taking back on
   failure follow today's order (`paired` is gone; the entry is saved
   before `welcome`, taken back if `welcome` cannot be sent, and the joiner
   saves nothing until `welcome` arrives).
4. Every pairing that starts SPAKE2 and does not reach `welcome` counts
   against the code, from any address, whether it failed or was
   abandoned. After
   three failures the code is used up: the main drops it and shows "Code
   used up after 3 wrong tries — Regenerate." A guesser gets three tries in
   a million per code. The per-address lockout stays, only as flood
   protection.

A wrong code shows "Wrong or expired pairing code." on the joining Maya, as
today.

### Every later connection

- A Noise `IK` handshake (`Noise_IK_25519_ChaChaPoly_SHA256`): the
  assistant knows the main's pinned key; the main learns the assistant's
  key from the handshake and looks it up. An unknown or removed key is
  refused.
- After the handshake every message is a Noise transport message:
  encrypted, authenticated and in order. A changed, replayed, reordered or
  injected message fails to decrypt, and the connection closes.
- `hello` moves inside the channel. Before the handshake completes nothing
  but handshake messages travel; a listener learns only that two Mayas
  talked, and the main's static key.
- The main's `welcome` no longer carries a `mac`: completing `IK` proves
  the main holds its private key.
- The pre-auth limits stay (eight handshakes at once, two per address).

### Framing

- Every protocol 2 frame is a WebSocket binary frame. Its payload is one or
  more Noise messages, each prefixed with its length as a big-endian u16.
  Noise messages top out at 65 535 bytes; a reply with attachments can be
  20 MB, so a message larger than one Noise message is split across
  several, and the receiver joins them before decoding the JSON inside.
  `MAX_FRAME` (64 MB) still caps the joined plaintext.
- The JSON inside is today's `Up` and `Down` messages, without the
  handshake ones: `challenge`, `auth`, `pair` and `paired` go; `hello`,
  `welcome`, `bye` and everything after keep their shapes. `PROTOCOL`
  becomes 2.

### Older Mayas

- A protocol 1 Maya opens with a plain JSON text frame. A protocol 2 main
  that receives a text frame answers with a plain `bye { reason: "This
  main needs a newer Maya. Update it." }`, which an old Maya shows as it
  shows any `bye`, and closes.
- A protocol 2 Maya that receives a text frame (an old main's `challenge`)
  closes and shows "The main runs an older Maya. Update it."
- On upgrade a Maya drops its protocol 1 pairings (the tokens), makes its
  key pair, and Settings › Network shows "This Maya was updated to secure
  pairing — pair again." until it is paired.

### Make main

- `promote` carries the paired list as ids, names, hostnames, platforms,
  mirror flags and public keys. No secrets travel, and the extra seal goes
  (`seal`, `open`, `seal_list`, `open_list`, `UNSEALABLE`); the channel is
  already encrypted.
- The old main's own entry in the list is its public key. It becomes an
  assistant of the promoted machine, whose key it already holds from
  their pairing.
- Every other assistant gets `move { host, port, key }` and pins `key` as
  its main's. It accepts that key only because the instruction arrived
  from the main it already pins, over the channel.
- An assistant offline during the handover behaves as today; nothing but
  its pinned main can give it a new main key.

### Cutting a machine off

- **Remove** on the main deletes that entry and its key; the machine's
  next handshake fails. The phone's "Sees and controls all sessions"
  switch is unchanged.
- **Forget main** on an assistant or phone deletes the pinned main key and
  the main's address, for a lost or stolen main.
- **New key** (Settings › Network, and `maya-cli pair --new-key`) makes a
  fresh key pair and drops every pairing on that Maya, for when the key may
  have leaked. The user then pairs again.

### When the main's key changes

An assistant whose `IK` handshake fails against its pinned key shows "The
main Maya's key has changed. If you reset it, pair again; otherwise someone
may be impersonating it." It keeps retrying every 30 s, which is harmless,
and never re-pins on its own.

### Messages and logs

- A transport message that fails to decrypt closes the connection; the
  Debug tab logs "A frame failed authentication; connection closed." The
  usual reconnect backoff follows.
- Nothing in a refusal before the handshake completes says whether a key
  is known.

### Phone and CLI

- Everything is in `core/src/net/`; `mobile/` and `cli/` get it through
  `maya_core`. `pair_with` keeps its signature apart from returning the
  main's public key in place of a token.
- The phone keeps its private key in the app's config; the CLI in the 0600
  config. The desktop app's key store access moves from `voice.rs` into a
  small `src-tauri/src/keystore.rs` that both the voice key and the network
  key use.

## Ready for a relay

The protection runs between the two Mayas, so a future relay only forwards
opaque frames, routed by a hash of the main's public key. It could drop
traffic but never read or forge it. This spec builds no relay; Tailscale or
a forwarded port carries the connection until one exists.

## Dependencies

- `snow` for Noise.
- RustCrypto's `spake2`, also used by `magic-wormhole.rs`. To our knowledge
  neither crate has had an independent audit; that residual risk is far
  smaller than writing the cryptography ourselves.

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
- The main confirms a pairing with a sealed frame holding `paired`, which
  the joiner opens before it trusts the main's key (a security review found
  the plain confirmation byte let `pair()` return an unconfirmed key).
- Each sealed message's plaintext starts with a flag byte (`0x01` more,
  `0x00` last), so frame boundaries are authenticated: two frames merged
  into one, or one frame cut in two, no longer open.
- Handshake and connection deadlines are checked on every read, and
  WebSocket pings never count as liveness, so a peer sending pings cannot
  hold a handshake slot or a connection open.
- A refusal's reason text is never shown: known reasons map to fixed
  sentences, any other to "The main Maya refused the connection."
- On Linux, a key the keyring holds but would not unlock is an error, not
  "no key", so a dismissed unlock prompt never replaces the key.
- `maya pair --new-key` keeps the old key and pairing unless the new
  pairing, its config and its key file are all saved; New key on the phone
  changes nothing if a step fails.
- `maya run` refuses to start with a pairing but no `identity.key`.

## Testing

- `core/src/net/` unit tests:
  - pairing completes only with the same code; the code is used up after
    three failed or abandoned pairings from any mix of addresses;
  - `IK` refuses an unknown key and a removed one;
  - a changed main key is reported, and not re-pinned;
  - protocol 1 is detected both ways, with the messages above;
  - a 20 MB attachment round-trips across split Noise messages;
  - `promote` carries keys only, and `move` pins the new main's key;
  - the upgrade drops protocol 1 pairings and makes a key pair.
- A tampering proxy between client and server in the tests flips a byte,
  replays a frame, reorders two frames and injects one. Each closes the
  connection, and no command runs.
- The existing server and client tests, `mobile/tests/round_trip.rs` and
  the CLI tests move to protocol 2.
- The key store on Windows and Linux through CI and the Linux VM.
- By hand on the Mac, the laptop and the phone: pair each, drive each,
  Make main, Remove, New key; capture port 4127 with `tcpdump` and confirm
  none of it is readable.
