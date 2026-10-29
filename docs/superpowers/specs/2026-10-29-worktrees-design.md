# Worktrees: new sessions on their own branch, reviews off one clone (design)

Date: 2026-10-29
Status: approved
Builds on: starting sessions (`core/src/actions.rs` `start_session`,
`core/src/launch.rs`, `src/newsession.ts`) and PR reviews
(`core/src/reviews.rs`, `actions::start_review`)

## Purpose

Two sessions in one repository share one folder and one branch, so they
step on each other. Other agent managers (Conductor, Vibe Kanban,
Superset, the Codex app) give each session its own git worktree. The
user wants that in Maya as an option for a new session.

PR reviews clone the whole repository again for every pull request
(`<clones>/<repo>-<number>`), and when the project checkout is free they
run in it instead, which switches the branch in the folder the user
works in. The user wants every review to run under the clones directory,
off **one clone per repository**, with a worktree per pull request.

Success:

- A new session started with the worktree option runs in its own folder
  on a new branch, and the checkout it came from keeps its branch and
  files.
- The second review of a repository clones nothing: it fetches the pull
  request into the existing clone and adds a worktree.
- No review runs in the projects directory.

## Decisions (by the user)

- **New sessions work on the branch directly by default.** The worktree
  is an option in the new-session form, off by default. Off, nothing
  changes from today.
- **The new branch starts from the branch the checkout is on now** (its
  `HEAD`), not from the repository's default branch.
- **Reviews always run from the clones directory.** The projects
  directory is never used for a review, free or busy.
- **One clone per repository** in the clones directory, reused by every
  review of that repository through worktrees.
- **Merging, opening a PR, or discarding stays with the agent.** Agents
  already offer this at the end of their work. Maya adds no Merge, PR or
  Discard button.
- **Maya deletes nothing.** Worktrees and branches stay until the user or
  the agent removes them.
- **Approach C:** Maya creates a new session's worktree itself, in the
  core, before the agent starts (no network, no credentials, errors in
  the form). A review's clone and fetch stay in the terminal's shell
  line, as today, so they run with the user's own git credentials; the
  `git worktree add` is chained after them in the same line.
- **OpenCode reviews are the exception:** OpenCode starts through its
  server, which needs the folder first, so for an OpenCode review Maya
  runs the clone, fetch and worktree in the core, and shows any error in
  Maya. The credentials risk of approach A is accepted for this one agent.
- **The Resume dialog does not list worktree sessions.** A worktree
  session open when Maya quits comes back after a restart, as any
  session does; one that was closed cannot be resumed from Maya. The
  form warns about this when the worktree option is on.

## Approaches considered

- **A. Maya runs all git commands in the core.** Errors show in Maya and
  every folder exists before the agent starts. Rejected: `git clone` and
  `git fetch` need the user's credentials (SSH agent, credential
  helpers), which Maya does not always have when opened from the Dock or
  run headless on a paired machine; and a first clone of a big repository
  would block the Review button with no progress.
- **B. All git commands in the terminal's shell line.** One mechanism,
  the user's credentials, visible progress. Rejected: an OpenCode new
  session starts through OpenCode's server, with no shell line, so it
  could not get a worktree; and a bad branch name would fail in a
  terminal that is already open, with no card on the board.
- **C. Split by network (chosen).** New-session worktrees in the core;
  review clone, fetch and worktree in the shell line.

## Goals

- An option to start a new session in a fresh worktree on a new branch,
  for every agent, from the app, the CLI and a paired machine.
- Reviews in worktrees of one clone per repository, under the clones
  directory.

## Non-goals

- Merge, PR, discard or clean-up buttons, and any automatic deletion of
  worktrees or branches.
- A worktree option for sessions started by voice or by Maya's assistant:
  they keep working in the folder directly.
- Carrying uncommitted changes from the checkout into the worktree. The
  worktree starts from the last commit on the current branch.
- Setup scripts (copying `.env`, installing dependencies) or port
  allocation for worktrees.
- Moving or converting the per-PR full clones that earlier versions left
  in the clones directory.
- Listing worktree sessions in the Resume dialog.
- Making a second first review of a repository wait for the first one's
  clone (see "Two first reviews at once").

## Anti-goals

- A review that switches the branch, or touches the files, of a folder
  under the projects directory.
- The worktree option changing anything when it is off.
- A worktree folder that shows in the "+" folder list as a new project.

## Design

### Folder layout

Everything lives in the clones directory (`clones_dir`, default
`~/dev/reviews`), flat, each folder named after its repository so the
card's project name and the board search still read the repository:

| Folder | What it is |
|---|---|
| `<clones>/<repo>` | the one clone of a repository, for reviews. No session runs in it. |
| `<clones>/<repo>-<number>` (`<repo>-mr<number>` on GitLab) | a review's worktree. The same names as today's per-PR clones. |
| `<clones>/<folder>-<branch-slug>` | a new session's worktree, where `<folder>` is the project folder it came from |

`<repo>` is the last part of the repository's path (`bedrock` for
`Org/bedrock`), as today.

### New session in a worktree

**Form.** The new-session form gets a checkbox, "New worktree", off by
default, after the options row. While it is checked, a warning shows
under it: "Once this session is closed, Maya cannot resume it from the
Resume list. Resume it from its folder with the agent." It is sent as a
new field of `LaunchOptions`, `worktree: bool` (serde default `false`),
so it travels unchanged through the Tauri command, the CLI and
`CommandKind::Start` to a paired machine. The CLI gets
`maya start --worktree`.

**Older paired machines.** A Maya that predates this field would drop
it and start on the shared branch. The protocol's `Hello` gets a new cap,
`CAP_WORKTREE`, and the form shows the checkbox only for this machine
and for paired machines that announce it. *(My call.)*

**In `start_session`**, after the target folder is resolved (chosen or
picked by the classifier) and before `open_session`, when
`options.worktree` is set:

1. The target must be the top of a git working tree
   (`git -C <dir> rev-parse --show-toplevel` is `<dir>`). Otherwise the
   start fails with "<folder> is not a git repository, so it cannot
   start in a worktree." The fallback target (the projects root itself,
   when nothing fits) fails the same way.
2. The branch name is a slug of the session name, or of the first words
   of the prompt when there is no name: lower case, words joined by `-`,
   only characters `git check-ref-format --branch` accepts, at most 40
   characters.
3. When that branch or the folder `<clones>/<folder>-<slug>` already
   exists, Maya adds `-2`, `-3`, and so on, until both are free.
4. `git -C <dir> worktree add -b <branch> <clones>/<folder>-<slug> HEAD`.
   A failure returns git's message as the start's error, so it shows in
   the form.
5. The session starts in the worktree folder through `open_session`, as
   any other start. Remembering and resuming the session work as they do
   for any folder, since the recorded folder is the worktree.

This works for OpenCode too, because the folder exists before the server
session is created.

On a paired machine, that machine's Maya creates the worktree in its
own projects and clones directories.

### Review in a worktree

`reviews::resolve_target` no longer looks in the projects directory. It
returns the clone folder `<clones>/<repo>`, the worktree folder
`<clones>/<repo>-<number>`, and the pull request's ref on the forge:
`pull/<number>/head` on GitHub, `merge-requests/<number>/head` on GitLab.

`reviews::shell_command` builds this line before the agent's own start
line (each step only when needed, all joined with `&&`, all paths
single-quoted as today):

1. `mkdir -p <clones>`
2. Clone, when `<clones>/<repo>` does not exist:
   `git clone <url> <clones>/<repo>`
3. Fetch every branch, so `origin/<base>` is current for agents that
   diff against it, and the pull request into a ref of Maya's own:
   `git -C <clones>/<repo> fetch origin && git -C <clones>/<repo> fetch origin +<pr-ref>:refs/maya/review/<number>`
   (`refs/maya/review/mr<number>` on GitLab). *(My call: fetching only
   the PR's ref would leave `origin/main` at the first clone, and a diff
   against it would show weeks of unrelated commits.)*
4. When the worktree folder does not exist:
   `git -C <clones>/<repo> worktree prune && git -C <clones>/<repo> worktree add --detach <worktree> refs/maya/review/<number>`.
   The prune clears a worktree the user deleted by hand, which git
   would still count as registered.
   When it exists: `git -C <worktree> checkout --detach refs/maya/review/<number>`,
   so a second review sees the pull request's latest commits.
5. The agent starts in the worktree, as today.

Steps 3 and 4 also run when the clone already exists, so the clone is
made once and then only fetched.

The terminal opens in the clones directory, which `start_review` makes
first (a terminal cannot start in a missing folder), as it does today
for a clone.

**A review already running.** When a live session's folder is the
review's worktree (`reviews::is_busy`), the review fails before a
terminal opens: "A review of <repo> #<number> is already running."
*(My call.)* When the worktree has changes an earlier agent left, git's
checkout in step 4 fails and the terminal shows git's message; Maya does
not force it, since Maya deletes nothing. *(My call.)*

**Two first reviews at once.** Whether the clone exists is decided when
the shell line is built. When a second review of a repository starts
while its first clone is still running, its `git clone` fails and the
terminal shows git's message; the user starts it again once the first
clone is done. *(My call.)*

**The review prompt.** The built-in prompt (`DEFAULT_REVIEW_PROMPT`)
gains "The pull request is checked out in this folder.", so agents do
not run `gh pr checkout`, which fails when the branch is checked out in
another worktree. A custom template is left as the user wrote it.
*(My call.)*

**A folder left by an earlier version.** When `<clones>/<repo>-<number>`
exists and is a full clone (its `.git` is a folder, not a file), Maya
reviews in it as today, with no clone, fetch or worktree. It is not
converted.

**A `<clones>/<repo>` that is not this repository's clone** (its
`origin` names another repository, checked with
`reviews::is_checkout_of`): the review fails with "<folder> is not a
clone of <repo>; move it or change the clones directory." before a
terminal opens.

**OpenCode.** An OpenCode review starts through OpenCode's server, which
needs the worktree to exist before the session is created, so the shell
line cannot make it. For OpenCode, `start_review` runs steps 1 to 4 in
the core, as `git` processes with no shell, then starts the session in
the worktree as today. A failing step returns git's message as the
review's error, so it shows in Maya. The first clone of a big
repository makes the Review button wait until it finishes; that is
accepted for this agent.

### Settings

The clones directory's label becomes "Clones directory (for reviews and
worktrees)", and its comment in `config.rs` says the same.

## What is left to the builder

- Removing `find_checkout` if nothing uses it once reviews stop looking
  in the projects directory (`is_busy` stays, for the running-review
  check).
- Sharing one list of git steps between the review shell line and the
  OpenCode path, so the two cannot drift.
- Tests: real `git` in temporary folders for the worktree start and the
  OpenCode review (`core/src/pr.rs` and `core/src/reviews.rs` tests
  already do this), and string tests for the review shell line.
