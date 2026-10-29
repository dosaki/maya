# Session usage and limits

Date: 2026-10-29. Status: reviewed by a builder check, awaiting approval.

## Intent

Every session window on Maya's board shows three things about the session
that opens it: how full its context is (shown today), how close the
account it runs under is to its limits, and what the session has cost so
far. The numbers sit in the window's header as a strip of small meters,
in both the chat and the terminal look, and the terminal look's bottom
line becomes the session's real Claude Code status line.

### Why

The user runs many sessions, across several agents and several machines,
and each agent on each machine is signed into its own account. Today the
only way to know whether a session is about to hit its limit, or how
heavy it has been, is to find its terminal and ask the agent. Maya exists
so that the user does not have to find the right terminal first.

### Who

The user of Maya, on every machine Maya runs on: a main, its assistants
and the phone (the phone is out of scope for this round; see Non-goals).

## Goals

- Each window shows, for its session: context, the account's limit
  windows with how full they are and when they reset, and the session's
  cost so far, in dollars (estimated where the agent gives only tokens)
  or in the agent's credits.
- Every agent on the board is covered to the extent its data allows:
  Claude Code, Codex, Antigravity, Kiro CLI, Grok Build and OpenCode.
- Sessions on assistant machines show the same numbers on the main.
- The header strip fits the window at any width, giving up detail in
  stages rather than wrapping or overflowing.
- The terminal look's bottom line shows the Claude Code status line the
  user configured, as the session's own terminal shows it, without
  running the status line command a second time.
- Where an agent does not share a number, the window says so in a muted
  way rather than leaving a hole.

## Non-goals

- Two Claude Code accounts on one machine (sessions under a second
  `CLAUDE_CONFIG_DIR`). Maya watches one Claude folder per machine. The
  data model carries the account label per session so this slots in
  later without redoing the window.
- Showing usage on the board's cards, in the Sessions tab or on the
  phone. This round is the session window only.
- Warnings, notifications or voice announcements about limits.
- Exact billing. The dollar figures for Claude Code and Codex are
  estimates from list prices and are labelled "est.".
- Reading any agent's sign-in token. Maya never handles credentials for
  this feature.
- Fetching prices from the network. The price table ships with Maya.
- Maya's own brain session: it runs headless and never gets a status
  line, and it is not on the board outside Debug mode.

## Anti-goals

- A status line install that changes what the user sees in their
  terminal. The wrapper must be invisible: same output, same timing.
- Background work that slows the board. The watcher runs every second;
  nothing added to it may read a whole transcript on every tick or spawn
  a process on every tick.
- A dollar figure that looks like a bill. Estimates are labelled.

## Constraints

- Claude Code gives its limits to one place only: the `rate_limits`
  object of the status line payload (5-hour and 7-day windows, each with
  `used_percentage` and `resets_at`). It is not in transcripts or hook
  payloads. The usage endpoint was rejected: undocumented, throttled, and
  it needs the sign-in token.
- The status line payload reaches a session's status line command only
  while that session is talking or on its `refreshInterval`. Numbers
  from an idle session are as old as its last message.
- Maya reads agent files from the watcher tick, every second, and must
  keep doing so cheaply. Transcripts can be hundreds of megabytes;
  today Maya reads only their tail.
- Assistants send their cards to the main as JSON (`core/src/net`).
  New card fields must be optional with serde defaults so mixed
  versions keep working.
- Kiro's account limits have no official route. The only route is its
  ACP server's `_kiro.dev/commands/execute` with the `usage` command,
  which renders the same card as `/usage`. It may break with a Kiro
  release.
- Grok Build shares its account limits nowhere outside its own TUI.
- Antigravity answers `agy -p "/usage" --output-format json` with its
  limit buckets per model group, with no agent turn and no saved
  conversation. Verified on 1.4.10, signed out included.

## Approaches considered

### Where the numbers live

1. **On the card, read by each agent's module in `core/`** (chosen).
   Each agent's module already reads that agent's files for the card's
   context; each one grows a usage reading, one new module collects
   account limits per agent on its own cadence, and both are stamped on
   the card. Cards already travel from assistants to the main, so
   remote sessions come for free, and the modal draws what the card
   carries.
2. **Fetched when a window opens.** Nothing on the card; the modal asks
   the backend for the open session only. Spares background work, but
   needs a new request type for the main to ask an assistant, and the
   per-account cache has to exist anyway. Rejected.

### Claude Code's limits

1. **The status line payload** (chosen). Maya installs a wrapper as the
   status line command, the way she installs her hook. It records the
   payload and the rendered line, then runs the user's own command
   untouched. Official numbers, no network, no credential, each reading
   tagged with the session that produced it (which is what tells two
   accounts apart later). Against it: a settings change at install, and
   numbers refresh only while a session is talking.
2. **The usage endpoint with the sign-in token.** Works with no session
   running, but undocumented, throttled hard, and Maya would handle
   Claude Code's credential. Rejected.

### The window

The user chose a strip of small meters inside the header, shrinking in
stages with the window's width, over chips in the header and over a
single chip with a popover. In the terminal look the strip stays in the
header and the bottom line is the real status line.

## Design

### What a window shows

In the header, between the harness badge and the model/branch text, a
strip of meters. Each meter is a label, a small bar and a percentage;
limit windows add their reset time. Bars use the card meter's bands:
the context meter's yellow, blue for a limit window, red from 75% for
any window. At the right of the strip, the cost: `$4.12 est.` with the
token totals beside it, `$0.40` when the agent reported the figure
itself, `12.4 credits` for Kiro.

Per agent, the strip holds:

| Agent | Limit meters | Cost |
|---|---|---|
| Claude Code | `5h`, `7d` from the recorded payload | Claude Code's own figure, `cost.total_cost_usd` from the recorded payload; before the first payload, or without the wrapper, estimated dollars from the transcript's tokens |
| Codex | `5h`, `7d` from the session file's `token_count` lines; window labels come from `window_minutes` (300 → `5h`, 10080 → `7d`, otherwise `<n>h` or `<n>d`) | estimated dollars from `total_token_usage` |
| Antigravity | one meter per model group from print mode, labelled by group and window (`Gemini 7d`, `Claude+GPT 7d`) | none this round (see What is left to the builder) |
| Kiro CLI | the month's credits from the `/usage` card, labelled `month`, when the ACP route works; otherwise "limits not shared" | credits, summed from the session file's `metering_usage` |
| Grok Build | "limits not shared", muted | dollars from `grok usage <session-id>` |
| OpenCode | none, muted "no limits" | dollars and tokens from its server's session record |

Hovering a meter shows the full numbers: `5-hour limit 78% · resets in
1h 12m · <account>`; the cost's tooltip lists tokens by kind and, when
estimated, the model priced. A Claude Code cost from the payload is not
labelled "est.": it is Claude Code's own figure and covers subagents;
the transcript estimate is labelled "est." and its tooltip says "main
conversation only", since subagent transcripts live in files of their
own that Maya does not read. The account label is the signed-in email
when Maya can read it without touching a secret (Claude Code:
`oauthAccount.emailAddress` in `.claude.json` beside the Claude folder),
otherwise the machine's name.

Shrink stages, by the header's available width, measured in the
frontend with a `ResizeObserver` on the header (not media queries, since
the panel is resizable on its own):

1. Wide: everything, including reset times and token totals.
2. Medium: reset times and token totals go; bars narrow.
3. Narrow: bars go; percentages keep their colour. The model/branch
   text is hidden before the strip is.
4. Phone-narrow: one chip, `62% · 78% · $4.12`, coloured by the hottest
   window, everything in its tooltip. The project name goes before the
   chip does.

The exact pixel thresholds are the builder's; the mockups used about
1100, 850, 650 and 480 px.

The terminal look draws the same strip in its palette. Its bottom line,
today Maya's own `model · folder · branch · PR · ctx %`, becomes the
session's recorded status line when there is one (Claude Code sessions
with the wrapper installed and at least one payload recorded), and stays
Maya's own line otherwise, with the limits and cost appended to it:
`… · ctx 62% · 5h 78% · 7d 41% · $4.12 est.`.

The recorded status line carries ANSI colour. The viewer renders SGR
bold, dim, the 16 and 256 colours and truecolor, and strips everything
else. No other escapes are honoured.

### The card

`Card` gains three optional fields, all skipped when absent:

```rust
/// What the session has used so far.
pub usage: Option<SessionUsage>,
/// The limits of the account the session runs under.
pub limits: Option<AccountLimits>,
/// The session's own status line as its terminal last drew it (Claude Code).
pub status_line: Option<String>,

pub struct SessionUsage {
    pub input: u64,          // uncached input tokens
    pub cached_input: u64,   // cache reads
    pub cache_write: u64,    // cache creation
    pub output: u64,         // output, reasoning included
    pub cost: Option<Cost>,
}
pub struct Cost {
    pub amount: f64,
    pub unit: CostUnit,      // Usd | Credits
    pub estimated: bool,     // true when Maya priced the tokens
    /// The models priced, for the tooltip; empty when not estimated.
    pub models: Vec<String>,
}
pub struct AccountLimits {
    pub account: Option<String>,   // email or machine name
    pub windows: Vec<LimitWindow>,
    pub read_at: u64,              // epoch millis of the reading
}
pub struct LimitWindow {
    pub label: String,             // "5h", "7d", "Gemini 7d", "month"
    pub percent: u8,               // used, 0 to 100
    pub resets_at: Option<u64>,    // epoch seconds
}
```

`windows` is a list, not two fixed fields, so Antigravity's groups and
Kiro's month fit without special cases in the frontend. The TypeScript
`Card` in `src/types.ts` mirrors it.

A card with `limits: None` on an agent that can have them (Claude Code
before any payload, Kiro when the route fails) shows "limits not read
yet" muted; `Grok` and OpenCode show "limits not shared" and "no
limits". A Claude Code payload recorded without `rate_limits` (an API
key, or a gateway) gives `limits: Some` with no windows, shown as "no
limits". The frontend decides the wording from the harness and the
field, not from a flag.

`Cost.amount` is stored as integer micro-units (millionths of a dollar
or of a credit) so `Card` keeps deriving `Eq`.

### Collecting, per agent

All collection runs in `core/`, on the machine the session runs on. The
main gets assistants' numbers inside their cards, as it gets their
context today.

**Claude Code.**

- Limits, the cost, the context and the status line come from the
  wrapper (next section), which writes
  `<claude_dir>/maya/usage-<session_id>.json` with the payload's
  `rate_limits`, `cost`, `context_window`, `model`, `version`, the epoch
  of the write and the rendered line. The files sit in `maya/` itself,
  which the watcher already watches (non-recursively, with the 5 s tick
  as the backstop). The state builder reads the file for each card's
  session. A session with no file shows no limits.
- When the file exists, its `context_window` (`context_window_size` and
  `used_percentage`) replaces Maya's guessed window for that card's
  context, so the strip and the recorded status line never disagree,
  and its `cost.total_cost_usd` is the card's cost, `estimated: false`.
- Because limits are per account and one machine is one account this
  round, every Claude Code card on the machine shows the newest reading
  across all usage files, not only its own: an idle session shows what a
  talking session just learned. The reading's `read_at` is its file's
  write time.
- Tokens come from the transcript: every main-chain `assistant` record
  with a real model (not `<synthetic>`) adds its `usage` fields. Maya
  keeps running totals per transcript path together with the byte
  offset read so far, in memory, and reads only the bytes appended since
  the last tick. A transcript that shrinks or is replaced starts over.
  The first read of a long transcript may take a moment; it runs on the
  watcher's existing background path, never on the UI thread.
- Without a recorded payload, cost is estimated from those totals per
  model with the price table. The 1M-context surcharge (prices change past 200k input tokens on
  some models) is applied per record when that record's input plus
  cache tokens exceed 200k, since the transcript records each call.

**Codex.** The session file Maya already tails carries, in each
`event_msg` of type `token_count`, `info.total_token_usage` (input,
cached input, cache writes, output, reasoning) and `rate_limits` with
`primary` and `secondary` windows (`used_percent`, `window_minutes`,
`resets_at` in epoch seconds). The newest line with a non-null
`rate_limits` is the account reading, shared across the machine's Codex
cards like Claude's. `rate_limits` is null for proxied or `exec`
sessions; then no limits. Cost is estimated from the totals with the
price table, keyed by the session's model from its newest
`turn_context` line (`payload.model`); `session_meta` does not name it.

**Antigravity.** A collector runs `agy -p "/usage" --output-format json`
with a 20 s timeout, at most every 5 minutes while an Antigravity card
is on the board, and once when an Antigravity window opens if the
reading is older than 60 s. The answer's `command.data.groups[]` give
`name` and `buckets[]` with `window`, `remaining_fraction` and
`reset_time` (ISO 8601). Each bucket becomes a window labelled
`<short group name> <window>` with `percent = round((1 - remaining) *
100)`. Group names are shortened by a small table ("Gemini Models" →
"Gemini", "Claude and GPT models" → "Claude+GPT"); unknown groups keep
their name. A signed-out `agy` still answers, with 100% remaining;
that is shown as is.

**Kiro CLI.** Credits per session come from the session's `<id>.json`:
the sum of `metering_usage[].value` across
`session_state.conversation_metadata.user_turn_metadatas[]`, unit
`credits` (`core/fixtures/kiro/session.json` carries them). Account
limits come from a collector that spawns `kiro-cli acp`, sends
`initialize`, `session/new`, then `_kiro.dev/commands/execute` with
`{"command": "usage"}`, parses the card's text for the credits used, the
plan cap and the reset date, and exits. It runs at most every 10
minutes while a Kiro card is on the board. When the spawn, the method or
the parse fails, Maya logs it once per hour and the window shows
"limits not shared". This route is unofficial; the builder spikes it
first (see What is left to the builder).

**Grok Build.** Cost and tokens come from `grok usage <session-id>`,
which prints JSON with `session.inputTokens`, `outputTokens`,
`cachedReadTokens`, `cacheCreationTokens`, `reasoningTokens` and
`costUsdTicks` (1 USD = 10^10 ticks; verified against the model's
list price). It runs when the session's transcript changed since the
last run, at most once a minute per session, with a 10 s timeout. No
limits.

**OpenCode.** The server's session record Maya already fetches carries
`tokens` (input, output, reasoning, cache read and write) and `cost`.
Both go on the card as reported, `estimated: false`. No limits.

### Off the watcher's path

`Store::refresh` runs under the store mutex. Every spawn (`agy`,
`kiro-cli acp`, `grok usage`) and the first full read of a transcript
run on threads of their own that fill caches, and `refresh` only reads
the caches. Nothing added to `refresh` may block on a process or read a
whole file.

### The price table

`core/src/pricing.rs`: a table of list prices per model id prefix, in
dollars per million tokens for uncached input, cache reads, cache
writes and output, plus the over-200k rates where a model has them.
It covers the Claude models Claude Code can run and the models Codex can
run. The builder takes the values from the providers' pricing pages at
build time, not from memory, and notes the date in the file. A model
that matches no row yields tokens only, no cost, and the tooltip says
"no price for <model>". The table is updated with Maya's releases.

### The status line wrapper

Install, in Settings → Sessions beside "Install Claude hook", as a
second button, "Record usage from the status line", with its own
installed/not-installed line. It is also offered on first run, after
the hook. My call: separate from the hook, because it edits a setting
the user may have made by hand.

The headless CLI gets the same, as `maya statusline install|remove|status`
beside its `hooks` subcommand, because the user runs Maya as the CLI on
some machines and those machines' Claude Code sessions must show their
limits on the main like any other. Session discovery does not depend on
the wrapper in either mode: a machine without it still shows every
session, only without limits and without the recorded status line.

Installing:

1. Finds which settings file holds the `statusLine` that wins. Claude
   Code takes it from managed settings, then `settings.local.json`,
   then the project's `.claude/settings.json`, then the user's
   `settings.json`, and a `statusLine` is one value, not merged. Maya
   installs into the user-level file that holds it (`settings.local.json`
   or `settings.json`), or into `settings.json` when none does. When a
   managed or project file holds one, install is refused and the
   Settings line says which file, so the user can move it.
2. If the entry is a `command` entry, its `command` string is saved as
   the inner command, in `<claude_dir>/maya/statusline-inner` (one
   line). Every other key of the entry (`padding`, `refreshInterval`,
   `hideVimModeIndicator`, anything else) is carried over unchanged to
   the new entry; `subagentStatusLine` is left alone. If there is no
   `statusLine`, the inner command is empty.
3. Writes `statusLine.command` to the wrapper: on macOS and Linux
   `"$HOME/.claude/maya/maya-statusline"`, a portable POSIX `sh` script
   written by Maya that needs only `jq`, which the hook already requires
   there; the same script serves the app and the CLI. On Windows it is
   `maya-hook.exe` with a `statusline` argument, as the hook is.
   Reinstalling over an existing wrapper does not overwrite the saved
   inner command.
4. Records the install so `is_installed` can report it. `is_installed`
   is true only when the winning entry's command is the wrapper.

Uninstalling restores the saved inner command (or removes `statusLine`
when there was none) in the file that holds the wrapper, and deletes
the usage files.

The wrapper, on each run:

1. Reads the payload from stdin.
2. Runs the inner command with the payload on its stdin, if there is
   one, capturing its stdout. Its stderr passes through. If the inner
   command fails or is missing, the captured output is empty.
3. Writes `<claude_dir>/maya/usage-<session_id>.json` atomically
   (write a temp file, rename): `{"at": <epoch ms>, "sessionId",
   "model", "version", "contextWindow", "rateLimits", "cost",
   "line": "<captured output>"}`. The session id is taken from the
   payload and checked to be a UUID before it names a file.
4. Prints the captured output, so the terminal shows exactly what it
   would have.

The wrapper adds the write of one small file to each status line
render. It must not add noticeable delay: no network, no `jq` chains
beyond one call.

Maya's hook uninstall does not touch the status line, and vice versa.

The status line runs on each assistant message, on `refreshInterval`,
and when a limit window resets, and only in trusted workspaces, so a
session in an untrusted folder records nothing and shows "limits not
read yet".

### Where the frontend gets it

The modal's model carries the card; `renderModal` (shared with the phone, which gets the strip at its
narrowest stage with no work of its own) builds the strip from
`card.context`, `card.limits` and `card.usage`, and `statusLine(card)`
returns `card.status_line` when set. The strip is its own module,
`src/usage.ts`, with the shrink stages, the tooltips, the per-harness
"not shared" wording and the ANSI rendering, each a pure function the
tests can hit.

### Tests

- Rust: the CLI's `statusline` subcommand against a temp Claude folder;
  parsing fixtures for each agent's file or output (a Codex
  `token_count` line, an `agy` usage answer, a Kiro session file with
  metering, a `grok usage` answer, a status line payload); the
  incremental transcript totals (append, shrink, replace); the price
  table against a hand-computed record; the wrapper install and
  uninstall round-trip on a settings file with and without an existing
  status line; the wrapper script on a fake inner command.
- Frontend: the strip at each stage, the tooltips, the "not shared"
  wording per harness, the ANSI renderer on bold, dim, 256-colour and
  truecolor input, and the terminal look's bottom line with and without
  a recorded status line.

## What is left to the builder

- **Spike Kiro's ACP route first**, in a throwaway script, before
  building its collector: confirm the method name, the handshake and
  the card's text on the installed Kiro, and write the parser against
  that text. If the route does not work on the installed version, build
  everything else and leave Kiro at "limits not shared", saying so in
  the pull request.
- **Antigravity's per-session cost.** Its transcript on this machine
  could not be checked. If the transcript or `agy -p` records token
  counts per conversation, add them as tokens only (its quota is not
  priced in dollars); otherwise leave the cost cell out for Antigravity.
- The pixel thresholds of the shrink stages and the exact widths of the
  bars.
- How the incremental transcript totals are stored and evicted (a map
  keyed by path, dropped when the session leaves the board, is enough).
- The Windows wrapper's handling of the inner command (Git Bash runs
  the status line, so an inner `sh` command runs the same way).
- The shape of the CLI subcommand's output (`status` prints one line,
  like `hooks status` does).
- Order and placement of the two install buttons on first run.

## Version

A `feat`, so the minor version is bumped in the pull request, as
`CLAUDE.md` says.
