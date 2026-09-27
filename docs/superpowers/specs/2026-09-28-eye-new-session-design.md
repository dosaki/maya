# Eye: column order and new-session launcher — design

Date: 2026-09-28
Status: approved design
Builds on: the three earlier Eye specs in this folder.

## Purpose

Reorder the board to Idle, Working, Awaiting Decision, Completed, and let the
user start a new Claude Code session from the board: pick a project folder
(or let Claude pick one from the prompt) and type the first prompt.

## Findings from the spike

- A headless pick works and is fast when run outside a Claude session's
  environment: `claude -p --strict-mcp-config --disable-slash-commands
  --model haiku --output-format text --no-session-persistence --max-turns 1
  "<prompt>"` answered `sonarqube` in 8 s for a SonarQube CI prompt against
  the user's `~/dev` folder list. `--bare` must not be used: it skips the
  stored login.
- Inside a Claude session's shell the nested variables (`ANTHROPIC_API_KEY`,
  `CLAUDECODE`, `CLAUDE_CODE_*`) break auth; Eye clears them when spawning.
- `claude` lives at `~/.local/bin/claude` on this machine; GUI apps do not
  inherit the login shell PATH, so the binary is resolved explicitly.
- A new Terminal window with a command is `tell application "Terminal" to do
  script "<cmd>"` (no `in` clause), as used by the earlier spikes.
- A session in a folder never trusted before shows Claude Code's trust prompt
  and registers (appears on the board) only after it is accepted.
- Headless `-p` runs register with `entrypoint: sdk-cli`, which the board
  already hides.

## Behaviour

### Columns

Order: Idle, Working, Awaiting Decision, Completed.

### Settings

- New field **Projects directory** (text, e.g. `~/dev`). `~` expands to the
  home directory. Saved as `projectsDir` in `~/.claude/eye/config.json`.
- Saving a value that is not an existing directory is refused with
  "Projects directory does not exist: <path>".

### The "+" button

- In the Idle column header, right of the count, labelled `+` with tooltip
  "New session".
- Opens the new-session modal. If no projects directory is configured, the
  modal shows "Set a projects directory in Settings first." with an Open
  Settings button instead of the form.

### New-session modal

- **Directory** dropdown: first entry "Let Claude choose", then every
  non-hidden subfolder of the projects directory, sorted by name.
- **Prompt** textarea, required (whitespace-only is not accepted). Cmd+Enter
  starts.
- **Start** button; disabled while a start is in progress.
- Status line: "Choosing a repository…" while the classifier runs;
  "Started in <folder>" / "Started in <folder> (chosen by Claude)" /
  "Started in <projects dir> (no clear match, Claude will work it out)";
  errors in red. The modal closes 1.5 s after a successful start.
- Escape or the backdrop closes it; a draft prompt is kept until the app
  restarts.

### Start flow (Rust command `start_session(dir: Option<String>, prompt: String)`)

1. Resolve the projects directory from config; error if unset or missing.
2. Target folder:
   - `dir` given: must be one of the listed subfolders (else error).
   - `dir` absent: run the classifier (below). A matched name becomes the
     target; NONE, no match, timeout or any error falls back to the projects
     directory itself.
3. Write the prompt to `~/.claude/eye/prompts/<epoch-millis>.txt`.
4. Open a new Terminal window running
   `cd '<target>' && claude "$(cat '<prompt file>')"` and activate Terminal.
5. Return `{ dir: <target path>, how: "chosen" | "classifier" | "fallback" }`.

### Classifier

- Command: the `claude` binary resolved from, in order, `~/.local/bin/claude`,
  `/opt/homebrew/bin/claude`, `/usr/local/bin/claude`, then
  `zsh -lc 'command -v claude'`.
- Args: `-p --strict-mcp-config --disable-slash-commands --model haiku
  --output-format text --no-session-persistence --max-turns 1 <prompt>`.
- Working directory: the projects directory. Environment: Eye's own, minus
  `ANTHROPIC_API_KEY`, `CLAUDECODE` and every `CLAUDE_CODE_*` variable.
- Timeout: 90 s, after which the process is killed and the fallback applies.
- Prompt template:

```
The user submitted this prompt without choosing a directory:
==========
<prompt>
==========
These are the folders in the projects directory: <a, b, c>
Reply with exactly one folder name from that list that this prompt most
likely belongs to, or NONE if none clearly fits. Reply with the name only,
nothing else.
```

- Reply matching: trim, strip surrounding quotes/backticks and a trailing
  period, compare case-insensitively with the folder names; `NONE` or no
  match means no pick.

## Errors

- "Set a projects directory in Settings first."
- "Projects directory does not exist: <path>."
- "That folder is not in the projects directory."
- "Could not find the claude command." (classifier and launch both need it
  only via the shell for launch; the launch uses the login shell's PATH)
- Terminal/osascript errors verbatim.

## Testing

- Rust: `list_project_dirs` (hidden and files excluded, sorted);
  `classifier_prompt` contains the prompt and the list; `pick_dir` (exact,
  case-insensitive, quoted, `NONE`, garbage); `applescript_launch` escapes
  `'` in paths for the shell and `"`/`\` for AppleScript and activates
  Terminal; `expand_home`; `clean_env` removes the nested variables;
  `set_config` refusal for a missing directory.
- Vitest: COLUMNS order; `+` button in the Idle header only; new-session
  modal renders dropdown entries and disables Start on a blank prompt;
  Cmd+Enter starts; the no-projects-directory hint; settings field present
  and reported on change.
- Manual: start a session with a chosen folder and with "Let Claude choose";
  see the Terminal window open with the prompt and the card appear.

## Out of scope

Choosing the classifier model in Settings, opening in a tab instead of a
window, remembering the last chosen folder, starting sessions on other
machines.
