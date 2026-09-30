<p align="center">
  <img src="src/assets/maya.png" width="200" alt="Maya">
</p>

# Maya

**Manage All Your Agents.** Maya is a macOS app that puts every coding-agent
session running on your Mac on one board, tells you when one needs you, and
lets you answer without hunting for the right terminal. Say her name and she
does it by voice.

## What Maya does

**One board for all your sessions.** Every running Claude Code session is a
card in one of four columns: Idle, Working, Awaiting Decision, Completed.
Codex, Antigravity and Grok Build sessions appear on the same board with
their own logos. Each card shows the session's name, project, what it said
last, how full its context window is, and the pull request it is working
on, if any.

**Act from the card.** Jump to the session's Terminal tab, read the whole
conversation as rendered markdown, reply (with files and images attached by
drag-and-drop or paste), or answer the question it is asking with one click.
Rename a session, change its model, effort or permission mode, and compact
its context when the meter passes 75%.

**Start and resume.** The "+" in the Idle column starts a new session in one
of your project folders, or lets Claude pick the folder from your prompt.
The resume button lists the earlier sessions of a folder so you can pick one
up where it stopped.

**Know when you are needed.** When a session starts waiting on a decision,
or finishes, Maya tells you: a system notification, or her voice saying
"hexgrid needs a decision". She stays quiet under a Focus mode. An
ElevenLabs voice is optional.

**Review pull requests.** The Pull Requests tab lists the pull requests
waiting for your review. One click opens the pull request; another starts a
review session in the right checkout, cloning the repository first if you
do not have one, and asks Claude whether it should be approved.

**Talk to her.** With voice on, "Maya, what's waiting on me?" gets a spoken
summary, "Maya, tell hexgrid to go ahead" sends the reply after a read-back
and your "yes", and "Maya, review collector 14" starts a review. When she
asks you something back, just answer. See [Voice](#voice).

## Install

macOS only, Apple Silicon and Intel. One command:

    curl -fsSL https://raw.githubusercontent.com/dosaki/maya/main/install.sh | sh

It downloads the latest release, puts `Maya.app` in `/Applications` (or
`~/Applications` when that is not writable) and clears the quarantine flag,
since the builds are not notarised. Then `open -a Maya`. Run the same
command again to update; set `MAYA_INSTALL_DIR` to install somewhere else.

Prefer a manual install? Take the `.dmg` for your architecture from the
[latest release](https://github.com/dosaki/maya/releases/latest), drag Maya
to Applications, and the first time right-click it and choose Open.

You need [Claude Code](https://claude.com/claude-code) installed and signed
in. The Pull Requests tab needs the [GitHub CLI](https://cli.github.com/)
(`gh auth login`). Voice needs macOS 14 or later.

### First run

1. Open Settings (the tab on the right) and press **Install hook**. The hook
   is what makes "Awaiting Decision" and "Completed" exact; without it a
   permission prompt shows as Working. It never blocks Claude Code and a
   backup of your `settings.json` is taken before every change.
2. Set your **projects directory** (for example `~/dev`) so the "+" can list
   your folders, and a **clones directory** for pull-request reviews.
3. Optionally turn on **Listen for "Maya"** and pick a speech recogniser
   (see Voice).

## Voice

Maya listens for her name and acts on what you say. Everything she can do
from a card she can do by voice: report what is waiting, reply to a
session, answer its question, bring its terminal forward, compact it,
resume or start a session, open or review a pull request.

- **Off by default.** Turn on "Listen for 'Maya'" in Settings. A microphone
  appears in the top bar; clicking it opens the voice panel with the
  conversation and a Yes / No for the current read-back. macOS asks for
  microphone and speech-recognition access the first time.
- **The wake word is "Maya".** Say it with the command, or alone: she says
  "Yes?" and waits. When she asks you something back, answer without the
  wake word; she listens for eight seconds and remembers the last few
  exchanges, so "the second one" works.
- **Sending is read back first.** Anything that sends text, answers a
  question, starts or resumes a session or starts a review is read back and
  needs a "yes". Reports, focus and compact happen at once.
- **Two speech recognisers.** *System* uses Apple's on-device recognition,
  which needs Dictation turned on (System Settings › Keyboard › Dictation).
  On a work Mac where that switch is locked by a management profile,
  choose *Built-in (Whisper)* under Settings › Voice assistant › Speech
  recognition and download a model; the 60 MB one is recommended.
- **Privacy.** Audio never leaves the Mac; both recognisers run on the
  device. Only the text of your command, the last few exchanges, and a
  summary of the board and pull-request list go to Claude, through a
  one-shot `claude -p` call with no tools. The one-off model download is
  the Built-in recogniser's only network access.

The Debug tab shows what she heard, what she made of it, what she asked
Claude, what came back, what ran and what she said. The same lines go to
`~/.claude/maya/maya.log`, which starts afresh on every launch. Paste them
into an issue if something goes wrong.

## How it works

Maya does not run an agent of her own; she reads what the agents already
write and drives them through their own interfaces.

- **Sessions** come from the registry Claude Code keeps in
  `~/.claude/sessions/` and the tail of each session transcript. Codex,
  Antigravity and Grok Build sessions are found through their running
  processes and their own session files.
- **States** come from the hook events in `~/.claude/maya/events.jsonl`
  when the hook is installed, with a rule-based reading of the transcript
  as the fallback (a turn that ends in a question is "Awaiting Decision").
- **Replies** go through Claude Code's session inbox socket, or are typed
  into the session's Terminal tab for the other agents. Terminal actions
  use AppleScript and AppKit, which is why Maya is macOS only for now.
- **Pull requests** come from `gh`, refreshed every two minutes.
- **Voice** is a small listener sidecar (Apple's recogniser or whisper.cpp)
  streaming text to Maya; a wake-word and confirmation state machine
  decides what to do; the command, board and pull requests go to Claude
  (Haiku by default) for one JSON action, which Maya validates against the
  board before doing anything.

Settings and data live in `~/.claude/maya/`: `config.json`, `events.jsonl`,
`hook.sh`, `maya.log`, downloaded speech models under `models/`.

## Developing

Building from source, cutting a release and code signing are covered in
[docs/DEVELOPING.md](docs/DEVELOPING.md).
