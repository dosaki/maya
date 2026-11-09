# Work items: GitHub and GitLab issues, and custom tools

## Intent

Maya lists the work items the user cares about (GitHub issues, GitLab
issues, and items from any other project management tool through a script)
and starts an agent session that implements one with a single button,
**Start**, in the right repository. Today the user finds an issue in a
browser, finds or makes a checkout, starts a session and pastes the issue
in. Maya already does the same for pull requests (the Review button); this
does it for the work itself.

## Goals

- A new **Issues** tab with two areas: **To pick** (left) and **Being
  worked on** (right), each a single column.
- Per source, the user decides what shows: GitHub and GitLab by rules
  (assignee, labels, repositories); a custom tool by its own script.
- **Start** assigns the item to the user, optionally sets its status, and
  starts a session in a fresh worktree of the item's repository with an
  editable prompt.
- When the repository is not plain (a custom item without one, or an issue
  in a tracker-only repository), Maya's brain picks one and the user
  confirms or changes it before anything starts.
- The phone shows the Issues tab and can Start; the voice can list and
  start items.

## Non-goals

- GitHub Projects boards, GitLab groups or epics, milestones: the "where"
  of a rule is a list of repositories only.
- Closed issues: rules match open issues only.
- Creating, editing, commenting on or closing items from Maya. The only
  writes are the assignment and the status on Start.
- A notification when a new item appears.
- Removing a worktree, a branch or a clone (Maya never removes them; as
  today).

## Anti-goals

- Start must never run anything in the user's own checkouts: work happens
  only in Maya's clones folder.
- A failed write-back (read-only token, a failing on-start command) must
  not stop the session from starting.
- A token must never appear in an error, a log line or the custom
  command's environment: Maya adds none of its stored tokens to it. (The
  command still gets the user's own environment as their shell sets it;
  Maya does not strip variables from it. *My call.*)
- The custom command runs only as the user typed it in Settings: never a
  command that came from a network peer, the phone, or an item's data.

## Approaches

1. **Native GitHub and GitLab, custom tools by a script (chosen).** A
   work-items module in the Rust core beside the pull request code: GitHub
   and GitLab over their APIs with the tokens Settings → Git holds, each
   rule one search; the custom script's JSON read into the same item
   shape; Start built on Review's clone and worktree steps. No extra tools
   to install, and errors can say exactly what is wrong (a token without
   write access).
2. **Everything is a script, Maya ships the GitHub and GitLab ones** (on
   `gh` and `glab`). One code path, but the user must install and sign in
   to both CLIs apart from the tokens Maya has, the rules form becomes
   command-line arguments, errors become script failures, and Windows
   quoting is fragile. Not chosen: it is not "native".
3. **Maya's brain fetches items through its MCP tools.** Slow, costs tokens
   on every refresh, and not predictable. Dropped.

The user chose 1.

## Sources

Every source turns its items into one shape, `WorkItem`, so the tab, the
phone, the voice and Start treat them alike:

| Field | Meaning |
|---|---|
| `source` | which source: `github`, `gitlab`, or a custom tool's id |
| `id` | the item's code as shown: `acme/api#412`, `platform/billing#17`, `ENG-1204` |
| `title` | required |
| `url` | required; opens in the browser |
| `description` | Markdown; may be empty |
| `status` | optional: the status label or the tool's status |
| `labels` | the other labels (status labels are not repeated here) |
| `assignees`, `author` | logins or names |
| `updatedAt` | ISO 8601 |
| `repo` | `owner/name`, `github:owner/name`, `gitlab:group/name`, or a clone URL; may be missing. A bare `owner/name` is on GitHub when GitHub is switched on, else on GitLab (*my call*) |

### GitHub and GitLab

- They use the connections in Settings → Git (the same token, the same
  GitLab URL); a service switched off there shows in Work Items as off,
  with a link to Settings → Git. Work Items has its own on/off per service.
- Each service holds a list of **rules**. A rule has:
  - **Assignee**: me, a named login, or anyone.
  - **Labels**: zero or more; an issue must carry all of them.
  - **Where**: any repository, or a list of repositories.
  An issue shows when it matches any rule, once.
- A rule with Assignee "anyone" must name its repositories: with no
  assignee and no repository, GitHub's search (and GitLab's
  `scope=all`) would cover every public repository. Settings refuses that
  rule and says why. (*My call.*)
- Each rule becomes one search: on GitHub a search with `is:issue
  is:open` plus `assignee:`, `label:` and `repo:` terms; on GitLab the
  issues API with `state=opened`, `assignee_username`, `labels` and the
  project. A rule over N repositories may need N GitLab calls. Pull
  requests never show.
- **Status labels**: per service, an ordered list of labels that count as
  statuses (e.g. `todo`, `in progress`, `in review`). An issue's status is
  the first of those it carries. Only listed labels are statuses: a GitLab
  scoped label such as `status::todo` counts only when it is in the list.
- **Already assigned to someone else**: Start first asks "This issue is
  already assigned to \<names>. Are you sure you want to take over?" On
  Yes, Maya adds the user to the assignees where the service allows more
  than one (GitHub; GitLab tiers with multiple assignees), else replaces
  the assignee, then starts. On No, nothing happens. An issue assigned
  only to the user, or to nobody, asks nothing. For a custom item this
  uses its `assignees` field; the on-start command does the rest.
- **On Start**: assign it to the user (always), and optionally set the
  status to one status label, which removes the other status labels. This
  replaces the "Add labels / Remove labels" fields of the older Settings
  mockup.
- **Tracker-only repositories**: per service, a list of repositories whose
  issues are about code elsewhere. Their issues go through the repo pick.
- Writing needs more than reading: GitHub fine-grained tokens need
  *Issues: read and write* (classic `repo` already has it); GitLab needs
  `api` (`read_api` cannot assign). The section says so.

### Custom tools

- **Add New** in Settings → Work Items adds one, with: a **name**, an
  on/off switch, a **list command**, and an optional **on-start command**.
  The same tool can be added twice with different arguments.
- The list command runs on every refresh, through the user's shell, with a
  time limit of 30 s. It prints:

  ```json
  { "items": [ {
    "id": "ENG-1204",
    "title": "Export reports as CSV",
    "url": "https://linear.app/acme/issue/ENG-1204",
    "description": "…",
    "status": "Todo",
    "labels": ["frontend"],
    "assignees": ["tiago"],
    "author": "maria",
    "updatedAt": "2026-11-06T10:00:00Z",
    "repo": "acme/web"
  } ] }
  ```

  `id`, `title` and `url` are required; an item missing one is skipped;
  unknown fields are ignored. A non-zero exit, a timeout or bad JSON makes
  that tool's error (its exit status and the first lines of its stderr)
  show at the top of the tab; the other sources still show.
- Filtering is the script's job: Maya passes it nothing.
- The on-start command, when set, runs on Start with the item's JSON on
  stdin, to assign and move the item in that tool. Its failure is a notice,
  as for GitHub and GitLab.
- Settings shows the format and a short example script by the command box.

## The Issues tab

The approved mockups are in `docs/superpowers/specs/2026-11-09-work-items/`:
`issues-tab.html` (the tab) and `repo-pick-and-settings.html` (the repo
pick dialog and Settings → Work Items; its first screen is an older tab
layout, replaced by `issues-tab.html`, and its label add/remove fields
are replaced by the status labels in Sources).

- A tab **Issues**, after Pull Requests, with a count of To pick.
- Above both areas: source, repository and status filters, a search, and
  when it last checked. They apply to both areas.
- Errors (a source failing, a missing token, a token without write access
  after a Start) show above the areas, as on Pull Requests.
- **To pick** (left): every item without a session, one column. Each shows
  its source, id, status (a gold tag), other labels (grey), title,
  assignee and age, and a **Start** button. An item with no repo says "No
  repo: Maya picks".
- **Being worked on** (right): every item that has a Maya session on any of
  the user's machines, whatever state that session is in, one column. Each
  shows the session's state (its colour on the left edge), its latest
  line, and **Open session** instead of Start.
- An item moves right as soon as Start starts its session; it stays there
  while the session is on the board, Completed included, even when no rule
  matches it any more (Start's own status change often takes it out of a
  `todo` rule). It leaves when the item is closed, or when the session is
  removed from the board, in which case an open item goes back to To pick
  if a rule still lists it.
- So Maya remembers the items it started. On each refresh it fetches a
  started GitHub or GitLab issue by its own number when no rule returned
  it, to show its current status and to see whether it was closed. A
  custom tool cannot be asked for one item, so a started custom item that
  the list no longer returns shows its last known copy until its session
  is removed. (*My call.*)
- An item whose status says "in progress" without a Maya session stays in
  To pick, its tag showing the status.
- A click on the id opens the item in the browser.

## Start

0. When the item is assigned to someone other than the user, ask about
   taking it over (see Sources); stop on No. When the repo pick dialog
   also shows, the takeover question shows in that same dialog.
1. Find the repository: the item's repo, unless it has none or comes from a
   tracker-only repository. Then Maya's brain picks one from the
   repositories she knows (the switched-on forges' repositories in the
   rules, recent sessions' repositories, the clones folder), from the
   item's title and description, and a dialog shows her pick and why, a
   repository list to change it, and the branch; the user confirms with
   **Start session** or cancels. With a plain repo there is no dialog.
2. Write back: assign the user and set the status (GitHub and GitLab), or
   run the on-start command (custom). A failure becomes a notice; it does
   not stop step 3.
3. In the clones folder (Settings, as reviews use): clone the repository
   once, fetch, and add a worktree on a new branch off the default branch,
   named `issue-<id>-<title slug>` (the id reduced to letters, digits and
   dashes; at most 40 characters as `worktree::branch_slug` does). A
   worktree already there for this item is reused.
4. Start Maya's agent there on the work prompt, as a session named
   `issue <id>`, so the tab can link the item to its session.

The **work prompt** is editable in Settings → Work Items, like the review
prompt, with `{id}`, `{title}`, `{url}`, `{description}` and `{repo}`. The
built-in one says: implement this work item on this branch, its title,
link and description, and open a pull request when it is done.

## Phone

- The phone gets an Issues tab with the same two areas (stacked as the
  phone's width needs) and Start; the repo pick dialog shows on the phone
  too.
- The phone does not show pull requests today, so this needs new messages:
  the main sends the work items to the phone (as it sends sessions), and
  the phone asks the main to Start an item (the main runs the write-back,
  the clone and the session). The phone never sends a command line.

## Voice

- "Maya, start acme/api#412", "Maya, start ENG-1204" or by title: a new
  brain action `start_item`, validated against the current list, like
  `review`.
- "Maya, what issues do I have?": she reads the To pick list.
- When the repo needs picking, she says her pick and the usual 5-second
  window lets the user stop or correct it.
- When the item is assigned to someone else, she says so and asks whether
  to take it over; she goes on only on a spoken yes, never on silence.
  (*My call.*)
- The phone shows the takeover question as a dialog, like the desktop.

## Refresh

Every 120 s, with the pull requests, and when Settings → Work Items or
Settings → Git change (a check that started before the change drops its
answers, as Pull Requests does). Which machines poll follows what Pull
Requests does today; the phone's Start runs on the main machine.

## What's left to the builder

- The settings file's field names and the Settings → Work Items layout in
  detail, within the mockup.
- How the brain is asked for the repo pick (a one-shot prompt is fine).
- Exact GitHub and GitLab queries, paging (50 per rule is enough at
  first), and how rules over many repositories are batched.
- How an item is matched to its session beyond the `issue <id>` name (the
  worktree folder, as reviews match on theirs).
