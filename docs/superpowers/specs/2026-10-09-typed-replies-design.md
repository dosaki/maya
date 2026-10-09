# Typed replies: what you send through Maya counts as you — design

Date: 2026-10-09
Status: approved design, awaiting implementation plan
Builds on: `2026-09-28-eye-reply-modal-design.md`,
`2026-10-07-maya-android-companion-design.md`

## Purpose

A reply sent to a Claude Code session through Maya, from the phone, from a
paired Mac or from the desktop, must reach Claude as the user's own input.
Today it does not: Claude treats it as a message from another session, so
"approved" or "push it" sent from the phone cannot approve anything, and
the user has to go back to the Mac and type it again.

The same change lets the card history tell the user's turns, other
sessions' messages and Claude Code's internal lines apart, where today
some internal lines show as if the user typed them.

Success: the user types "approved" on the phone, and the session treats it
exactly as if it had been typed at the Mac.

## Findings

- Maya sends Claude Code replies through the session's inbox socket
  (`actions::send_reply` → `inbox::send`). Claude Code frames every inbox
  message as "Another Claude session sent a message" and, by design, such a
  message "never counts as your consent", "can't approve anything", and its
  slash commands "arrive as plain text"
  (https://code.claude.com/docs/en/cross-session-messaging.md, "How a
  session treats an incoming message"). No message type or token changes
  that. The fix is the channel, not the payload.
- Maya already types into session terminals: other agents' replies,
  question answers, `/compact`, renames and mode changes, through the
  `Terminal` trait (Terminal.app by AppleScript `do script … in tab`, tmux
  `send-keys`, the Windows console by `WriteConsoleInputW`). Typed text is
  the user's input to Claude.
- Every reply, wherever it was written, ends in `actions::send_reply` on the
  machine that runs the session: the phone and paired Macs reach it through
  a `Reply` command. One change there covers all of them.
- `Up::Result` already carries an optional `data` value, so a reply's
  outcome can be reported without changing the protocol.
- The phone draws cards with the desktop's `src/modal.ts`.
- In the user's last six transcripts, three internal line kinds show as
  user turns: the compaction summary (`isCompactSummary: true`), the
  interrupt marker (the text `[Request interrupted by user]`, sometimes
  with ` for tool use`), and pasted text wrapped in
  `<pasted_content id="…">…</pasted_content>`. Every turn shown as a peer
  message was the user's own reply through Maya.

## Decision

Always type a reply into a Claude Code session's terminal when Maya can
reach it; use the inbox only when it cannot, and say so.

Consequence, accepted: the phone's pairing, already HMAC-authenticated, can
now approve actions in the user's sessions.

## Behaviour

### Route

`send_reply` for a Claude Code session:

1. Checks the session is free to type into (`answer::check_free`): when
   Claude shows a choice (a permission prompt or an option picker), the
   reply is refused with "Answer the question on the card first." A prose
   question (`AwaitKind::Text`) does not block. This replaces today's
   behaviour, where an inbox reply queued behind the choice.
2. Types the reply into the session's terminal (below). On success the
   route is `typed`.
3. Falls back to the inbox only when typing failed before any key reached
   the terminal: no tty for the session, no Terminal.app tab on it (a
   session in iTerm, VS Code or Ghostty), a pane outside tmux, or no
   console to attach to. On success the route is `inbox`.
4. Fails when neither works: "Maya can't type into this session's terminal
   and it has no inbox. Use the terminal."

If typing fails after some lines went in, Maya does not resend through the
inbox (that would duplicate it) and reports "Part of the reply was typed
into the terminal; check it there."

OpenCode (its server) and the other agents (one typed line) keep their
current routes.

### Multi-line text

A typed newline sends the prompt in Claude Code, so each line but the last
is typed with Claude Code's documented continuation: the line, a trailing
`\`, then Enter, which inserts a newline instead of sending. The last line
is typed with a plain Enter, which sends. Each line is one `type_line` call
under the existing `TYPING` lock, so another typed action cannot land in
the middle. Blank lines are typed as a lone `\`. Carriage returns are
dropped. A last line that itself ends in `\` gets a trailing space, so its
Enter still sends instead of adding a line.

Typed replies are capped at 20,000 characters ("Message is too long to type
(over 20000 characters)."); the inbox keeps its 100,000.

Bracketed paste is out of scope; it can replace line-by-line typing later
if long replies prove slow.

### What `send_reply` returns

`Result<Route, String>` with `Route` `Typed` or `Inbox`. The Tauri command
and the `Reply` command handler put `{"route": "typed" | "inbox"}` in their
result's `data`. A client that does not read it (an older phone or Mac)
loses nothing; a client talking to an older main finds no route and shows
"Delivered" as today. Each send logs its route in `maya.log`.

### Reply box

- A Claude Code card always shows the reply box (today it hides without an
  inbox).
- After a send: "Sent as you" (green) for `typed`; "Sent as a message from
  another session: it can't approve anything" (amber) for `inbox`;
  "Delivered" when no route came back.
- The Awaiting Decision banner reads "This session is waiting for a
  decision. Answer it on the card first." in place of "A reply will queue
  behind it."

### Card history

Turn kinds become `user`, `assistant`, `tool`, `peer` and `notice`.

- A typed reply is a plain `user` turn. The relabelling of peer turns Maya
  sent (`actions.rs`, the `was_sent` filter) goes, with the store's
  sent-reply hashes (`note_sent`, `was_sent`, `sent_path`, `SENT_KEPT`).
  Files left in `~/.claude/maya/sent/` are harmless and left alone.
- A `peer` turn is labelled "From another session" (today "Message") and
  styled apart: dimmer, with a rule on its side. An inbox fallback reply
  shows this way too, which is how Claude treated it.
- `notice` is a short, centred, dim line with no speaker:
  - a user line with `isCompactSummary: true` → "Earlier conversation
    compacted" (the summary is not shown);
  - a user line whose text is `[Request interrupted by user]` or starts
    with `[Request interrupted by user` → "Interrupted".
- Pasted text: `<pasted_content …>` and `</pasted_content>` tags are
  removed from a user turn; the pasted text and anything typed around it
  stay, trimmed.
- A notice ends an assistant run like a user turn, so Claude's last text
  before an interrupt is kept as its answer rather than dropped as
  narration.

The parser is `maya-core`'s `transcript.rs`, so the desktop, the phone and
paired Macs all show the same history.

## Changes

- `core/src/actions.rs`: `send_reply` routes as above and returns `Route`;
  a `type_reply` helper types a multi-line reply; the `was_sent` relabel
  goes.
- `core/src/answer.rs`: `reply_lines(text) -> Vec<String>` (the
  continuation splitting), pure and tested, beside the other typed-key
  helpers.
- `core/src/store.rs`: the sent-reply hashes go.
- `core/src/transcript.rs`: `TurnKind::Notice`; compaction and interrupt
  lines become notices; paste tags are stripped.
- `src-tauri/src/net_app.rs` and `cli/src/executor.rs` (the two places an
  assistant runs a `Reply` command): the result carries the route in
  `data`.
- `src-tauri/src/lib.rs` (local and routed replies) and
  `mobile/src/commands.rs`: `send_reply` returns the route to the page.
- `src/modal.ts`, `src/types.ts`, `src/styles.css`: the reply box status
  and visibility, the banner text, the `peer` label and style, the `notice`
  turn.
- `README.md` ("Replies"), `docs/DEVELOPING.md`: replies are typed, the
  inbox is the fallback.

## Testing

- `reply_lines`: one line; several lines; blank lines; CRLF; a middle line
  and a last line ending in `\`.
- `send_reply` against the fake terminal (`terminal.rs`'s recording
  terminal): typed route and the exact lines typed; fallback to the inbox
  (a socket bound in a temp dir) when the terminal reports no tab; no
  fallback after a partial typing failure; refusal while a choice is
  pending; both routes missing; the 20,000-character cap.
- `parse_turns`: the compaction flag, both interrupt wordings, a paste with
  and without text around it, a peer message, a typed reply, and an
  interrupt keeping Claude's last text.
- `modal.test.ts`: the reply box shows for a Claude card without an inbox;
  each status text; the "From another session" label; a notice turn.
- Live, by the user (Claude cannot start agent sessions here): on
  Terminal.app, a multi-line reply arrives as one user message with its
  newlines, and "approved" from the phone is accepted by a session that
  asked for approval in prose. Also check what a middle line ending in `\`
  turns into, and adjust `reply_lines` if Claude Code drops that
  backslash. tmux is not installed on this Mac; the tmux
  route is covered by the unit tests unless the user installs it or runs
  the check in the Linux VM.

## Release

A `feat`: the pull request bumps the version to 0.15.0.
