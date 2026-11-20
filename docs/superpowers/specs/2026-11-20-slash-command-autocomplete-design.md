# Slash command autocompletion

A project. Approved on 2026-11-20.

## Why

A session's message box already sends a `/command` line to the session's
terminal (`src/modal.ts`, `send_slash_command`), but you have to know the
command and type all of it. Most of the commands worth running are not the
agent's built-ins. They are the skills and plugin commands you installed,
and nobody remembers all of those. While you type, the box should offer
the commands this session actually has, with what each one does.

## Goals

- Typing `/` at the start of the message box opens a list of the
  session's commands, each with its description. Typing narrows it.
- The list holds:
  - the agent's own commands (for Claude Code: `/compact`, `/model`, …);
  - your own commands and skills;
  - the commands and skills of your enabled plugins;
  - the project's commands and skills, for the session's folder.
- It works for a session on another machine in the device group: the
  list is that machine's, not the one showing the window.
- It works for every agent that takes `/commands` (`caps.slashLines` in
  `src/harness.ts`), each with its own commands.
- It works on the desktop and on the phone.
- Something shows at once when the window opens, then the full list
  replaces it within a moment.

## Non-goals

- Completing a command's arguments (`/model opus`, file paths).
- Completing `!shell` lines.
- Running a command on pick. A pick fills the box; you still send it.
- Agents that do not take `/commands` (`other`).

## Anti-goals

- Board updates growing large or frequent because of command lists.
- The message box getting slower to type in, or the list stealing Enter
  when you meant to send.
- A list that offers commands the session does not have (another
  machine's skills, another project's commands), shown as if they were
  real.
- Breaking a device group whose machines run older Mayas.

## Approaches

- **A. Ask the session's machine when the window opens.** Exact for the
  session's project and plugins, but a remote session waits one round
  trip before anything shows.
- **B. Each machine shares its commands on its board,** the way it
  shares its agents and models today (`AgentInfo` in `core/src/agents.rs`,
  `Up::Board.agents` in `core/src/net/protocol.rs`). Instant, but project
  commands vary by folder, so they would bloat every board.
- **C. Built-ins only, kept in the app.** Cheap, but leaves out skills and
  plugins, which is most of the point.

**Picked: B, then A in the background, with a cache.** Each machine
shares its agents' machine-wide commands with the group (in a message of
their own, sent only on change; see below) so there is something to show
at once; on opening a session window Maya asks that session's machine for
the full list (adding the project's) and replaces what it showed; the
last full list is cached so the next opening starts from it.

## Design

### A command

Each command has a name (without the `/`), a one-line description (may be
empty), and where it came from: built-in, user, plugin (with the
plugin's name), or project. The source shows quietly beside the
description (for example `· plugin`), so two commands with similar names
can be told apart.

### Shared with the device group (machine-wide)

Each machine shares, per agent, its machine-wide commands: the agent's
built-ins, your own commands and skills, and your enabled plugins'
commands and skills. Project commands are left out (decided: they depend
on the folder).

They do not go in the board or in `AgentInfo` (decided). The board is
resent whole every 10 s and on every card change (`BOARD_REFRESH`,
`BOARD_GAP` in `core/src/net/client.rs`), and a machine with many plugins
has around a hundred commands with paragraph-long descriptions. Instead
the commands travel in their own `Up` message, sent once on connecting
and again only when the list changes, the way `Up::Terminals` is sent
today. Each description is cut to one short line by the machine sending
it (decided), so the message stays small. The main keeps the latest list
per machine and agent beside the other per-machine state in
`core/src/net/merge.rs`. An older main ignores the new message; an older
assistant never sends it.

The machine rebuilds its list on the same schedule `agents::current`
refreshes the rest of its agent info, so a new skill reaches the group on
the next refresh, not instantly. That is acceptable: the background
refresh below picks it up for any window that opens.

### The background refresh (per session)

A new request, `list_commands { session }`, routed like the history
request (`CommandKind::History` and `session_history` in
`src-tauri/src/lib.rs`, and the phone's `mobile/src/commands.rs`). The
session's machine answers with the full list for that session: the
board's machine-wide list plus the project's commands and skills for the
session's folder. Maya sends it once each time a session window opens.

A machine on an older Maya does not know the request. Its error is
swallowed and the window keeps whatever it already showed.

### The cache

Maya keeps the last full list per machine, agent and project folder on
the device showing the window, and it survives restarts (decided). Where
it lives, and how stale entries are dropped, is the builder's call.

### What a window shows, in order

1. The cached list for this machine, agent and folder, if there is one.
2. Otherwise the shared list for this session's machine and agent.
3. Otherwise (the machine's Maya is too old to send commands) the
   agent's built-ins, from the Maya showing the window.

When the background refresh answers, its list replaces what is shown,
including while the list is open, and is written to the cache.

### Across agents

- **Claude Code:** built-ins (a list Maya keeps, like its model list in
  `agents::claude`); `~/.claude/commands`; `~/.claude/skills`; the
  commands and skills of plugins enabled in `~/.claude/settings.json`
  (`enabledPlugins`), found through `~/.claude/plugins/installed_plugins.json`;
  the project's `.claude/commands` and `.claude/skills`. Descriptions come
  from each file's front matter (`description:`); a command with none
  shows without a description. Plugin commands carry the name Claude Code
  accepts for them, namespaced (`plugin:command`) where Claude Code
  requires it. Only skills the user can invoke are listed.
- **Other agents** (Codex, OpenCode, Kiro, Grok, Antigravity): their
  built-ins, plus their own custom command folders where they have them
  (for example Codex prompts, OpenCode's command folders). The exact
  folders, and the name each agent wants typed for a custom command, are
  to be checked against each agent's documentation while planning. An
  agent with no custom commands gets built-ins only.

### In the box

```
┌──────────────────────────────────────────────┐
│ /compact      Clear history, keep a summary  │
│ /commit       Create a git commit · plugin   │
│ /code-review  Review the current diff        │
└──────────────────────────────────────────────┘
> /co▌
```

- Opens when the box's text starts with `/` and the caret is still in
  the first word; closes once a space follows the command, or the `/`
  is gone.
- Matches by the start of the name first, then anywhere in the name
  (so `/review` finds `code-review`).
- The first match is highlighted when the list opens; arrow keys move
  the highlight (decided).
- Tab fills the highlighted command, `/<name> `, into the box with the
  caret after it.
- Enter fills it the same way, except when what you typed is already a
  whole command's name: then Enter sends as today, so typing `/compact`
  and pressing Enter runs it in one press (decided).
- Esc closes the list and keeps the text. While the list is closed,
  Enter sends exactly as today (`composerKey` in `src/keys.ts`).
- A click (desktop) or tap (phone) fills it in the same way.
- Sits above the box, and stays within the window on a phone with the
  keyboard up.
- Only shown for agents with `caps.slashLines`, and only while the
  composer is shown (not while a question's menu stands in for it).
- The composer is patched in place, not rebuilt (`src/morph.ts`), so the
  list must survive a repaint with its highlight intact.

## Left to the builder

- How long a description may be before it is cut, and how it is cut.
- Where the cache lives (Rust side on disk or page storage) and when it
  forgets an entry.
- The exact Claude Code built-in list and its descriptions.
- How many matches to show at once, and how the list scrolls.
- The list's look, within the existing terminal-look styles.
