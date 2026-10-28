# Deployments tab: approve or reject waiting GitHub deployments (design)

Date: 2026-10-28
Status: approved
Builds on: the Pull Requests tab (`core/src/reviews.rs`, `core/src/forge/`,
`src/reviews.ts`)

## Purpose

GitHub Actions runs can stop at an environment that requires approval
(for example `production-eu-west-2`). Today the user finds out by watching
GitHub. They want Maya to show these runs and let them approve or reject
from Maya, the same way the Pull Requests tab shows the PRs that wait on
them.

The tab also shows deployment runs that cannot start yet because an
earlier commit's deployment holds the same concurrency group. Those rows
say "Waiting on XXX", so the user can see that one approval releases the
next run.

Success: when a run waits for an approval the user is allowed to give,
it shows in the Deployments tab within 2 minutes. Approve and Reject
work from the row, and the row changes without waiting for the next check.
A deployment run held back by another run sits under that run and says
what it waits on.

## Decisions (by the user)

- A new tab, **Deployments**, next to Pull Requests.
- **GitHub only.**
- Each waiting run has two buttons: **Approve** and **Reject**. Reject
  is GitHub's "reject" of the pending deployment, which fails the run at
  that step. It does not cancel the whole run.
- **Repositories:** every repo the token can see, as with pull requests.
  Maya lists them newest push first and stops at the first repo with no
  push in the last 7 days.
- **"Waiting on XXX"** is a deployment run that has not started because
  another commit's deployment run, in the same concurrency group, is
  still running or waits for an approval.
- **XXX** is the blocking run's commit: its short SHA and the first line
  of its message (for example "Waiting on `a1b2c3d` fix login
  redirect"), linked to that run on GitHub. A blocked row sits under its
  blocker when the blocker is on the list.
- **Blocked rows show whoever approves them.** Maya cannot know the
  approvers of a run that has not reached its environment yet, so every
  blocked run in the checked repos is listed.
- **Blocked rows have no buttons.** They link to the run. When the run
  reaches its approval step, it becomes a normal row on the next check.
- **Checks every 2 minutes, always**, like Pull Requests. No faster
  check while something waits.

## Goals

- See every run from active repos that waits for an approval the user
  can give (`current_user_can_approve`).
- Approve or reject from Maya.
- See which deployment runs are queued behind another run, and which run
  that is.

## Non-goals

- GitLab.
- Cancelling a whole run.
- Runs in repos with no push in 7 days. A deployment started by hand or
  on a schedule in a quiet repo is not seen. The user accepted this gap.
- Approving for someone else, or showing runs that wait only on other
  approvers. A blocker may still be named in "Waiting on" text.
- Repos with no environment that requires reviewers (see "Which repos
  are checked").

## Anti-goals

- An approval or rejection that Maya says worked but GitHub did not
  accept. The row reports GitHub's answer, not a guess.
- A token in any error, log line or event. All calls go through the
  forge layer's `scrub`.
- Running out the user's GitHub rate limit (5,000 calls an hour, shared
  with the Pull Requests tab and the user's other tools). This poll
  should stay under about 2,000 calls an hour.

## Findings

### In Maya

- `core/src/forge/mod.rs` has the HTTP layer (`Http` trait, `get_json`,
  `post_graphql`, `check`, `scrub`, `encode`) and a `FakeHttp` for
  tests. `Http` has `get` and `post_json`, which is enough for every
  call below.
- `forge::scopes` names the scope in a 401/403 message:
  `"repo scope (fine-grained: Pull requests read)"`. It is shared with
  the Pull Requests tab.
- `src-tauri/src/lib.rs:265` `poll_reviews` is the model for the poll:
  it reads `git_generation`, resolves the forges (`git::forges`),
  fetches, drops the answer if the Git settings changed during the check,
  stores a `ReviewState` snapshot in `AppState` and emits `reviews`. It
  runs on its own thread every 120 s (`lib.rs:1519`), on every role.
- `src/tabs.ts` defines `Tab`, `PANES`, `asTab` and `setCount`
  (`reviews-count`). `index.html:19` has the Pull Requests tab button.
- `src/main.ts:291` repaints the age labels every 10 s.

### In the user's repos (checked 2026-10-28)

- `coral-boards` (`deploy.yml`) and `lilypad` (`deploy.yml`) put
  `concurrency` on the **whole workflow**, with a fixed group
  (`deploy-services`, `deploy`) and `cancel-in-progress: false`. A
  second push to `main` while a deploy runs or waits gives a run whose
  status is `pending`. Its blocker is the earlier run of the same
  workflow. This is the case the "Waiting on" rows are for.
- `captain-hook` (`deployment.yml`) has no `concurrency`. One run goes
  testing → `staging-eu-west-2` → `production-eu-west-2` through
  `needs`. Only production has `required_reviewers`. Its runs give
  approval rows and never blocked rows.
- `coral-boards`'s `plan.yml` runs on pull requests in the `services`
  environment. If `services` requires reviewers, those PR runs also wait
  for approval and are listed. That is correct: they do wait on the user.
- On 2026-10-28 the user's token saw **108 repos** pushed to in the last
  7 days (owner, collaborator and organisation member). Most of them have
  no environment that requires reviewers (for example `aws-ci-identity`).

## Which repos are checked

108 repos at 2 calls each every 2 minutes is about 6,500 calls an hour,
which is over the limit. Only repos with an environment that requires
reviewers can have a run waiting for approval, so Maya checks only those:

1. Every 2 minutes, list the repos pushed in the last 7 days (call 1).
2. For each of them, know whether it has an environment with a
   `required_reviewers` rule (call 2). Keep that answer for **1 hour**
   per repo, so a repo is asked about once an hour, not every check. A
   repo that newly appears in the 7-day list is asked about on that
   check.
3. Check runs (calls 3 to 5) only in the repos where the answer is yes.

Cost with 108 active repos of which 20 have such an environment, and 3
waiting runs: 2 (repo list) + about 4 (the hourly environment checks,
spread out) + 40 (runs) + 3 (pending deployments) = about 50 calls per
check, about 1,500 an hour.

## GitHub calls

Base: `https://api.github.com`. Each call uses the forge's token.

1. **Active repos:**
   `GET /user/repos?sort=pushed&direction=desc&per_page=100&affiliation=owner,collaborator,organization_member`.
   Read pages until a repo's `pushed_at` is older than 7 days, and stop
   there. Stop after 5 pages at most.
2. **Does the repo need approvals?**
   `GET /repos/{repo}/environments?per_page=100`. Yes when any
   environment has a `protection_rules` entry with
   `type: "required_reviewers"`. A 404 (no access to environments) means
   no.
3. **Waiting runs:** `GET /repos/{repo}/actions/runs?status=waiting&per_page=50`.
   No date limit: an approval can wait up to 30 days.
4. **Blocked runs:** `GET /repos/{repo}/actions/runs?status=pending&per_page=20`.
   Then, only when there are pending runs and their blocker is not among
   the waiting runs, `GET /repos/{repo}/actions/runs?status=in_progress&per_page=50`
   and, if still not found, `?status=queued`.
5. **Pending deployments, one call per waiting run:**
   `GET /repos/{repo}/actions/runs/{id}/pending_deployments`. This gives
   each environment's `id`, `name` and `current_user_can_approve`.
6. **Approve or reject:**
   `POST /repos/{repo}/actions/runs/{id}/pending_deployments` with
   `{"environment_ids": [...], "state": "approved" | "rejected",
   "comment": "..."}`. All three fields are required.

## The model

```rust
/// A run that waits for the user's approval, or that waits behind one.
pub struct DeploymentRun {
    pub repo: String,             // owner/name
    pub run_id: u64,
    pub run_number: u64,
    pub workflow: String,         // the run's `name`
    pub branch: String,
    pub sha: String,              // head_sha
    pub title: String,            // first line of head_commit.message
    pub actor: String,            // triggering_actor.login
    pub url: String,              // html_url
    pub since: String,            // ISO 8601; when the run was created
    pub state: RunWait,
}

pub enum RunWait {
    /// Waits for an approval the user can give.
    Approval { environments: Vec<Environment> },   // id + name, only those the user can approve
    /// Held by an earlier run in its concurrency group.
    Blocked { by: Option<Blocker> },               // None: "Waiting on another deployment"
}

pub struct Blocker { pub run_id: u64, pub sha: String, pub title: String, pub url: String }
```

`DeploymentState` (in `AppState`) is shaped like `ReviewState`:
`runs`, `error` (a `ForgeError` or none), `off` (GitHub is not
switched on) and `fetched_at`. The 1-hour environment answers are kept
next to it, in memory only.

## Rules

- **Approval rows:** a `waiting` run whose pending deployments include at
  least one environment with `current_user_can_approve: true`. A run
  that waits only on other people is not listed.
- **Blocked rows:** every `pending` run in a checked repo. Its blocker is
  the most recent earlier run (lower `run_number`) of the same
  `workflow_id` in the same repo whose status is `waiting`,
  `in_progress` or `queued`. This matches the user's workflows, where
  the concurrency group is fixed per workflow.
  - If no such run exists: `Blocked { by: None }`, shown as "Waiting on
    another deployment". This happens when a group spans several
    workflows.
  - GitHub keeps at most one pending run per concurrency group, so
    there are no chains of blocked runs.
- **Order:** grouped by repo. Repos with the oldest waiting row come
  first. Within a repo, a blocked row comes right after its blocker.
  Otherwise rows are oldest first.
- **Tab count:** the number of approval rows. Blocked rows are not
  counted.

## Approve and Reject

- Two Tauri commands: `deployment_approve(repo, run_id)` and
  `deployment_reject(repo, run_id)`. Each looks up the run in
  `DeploymentState` to get its environment ids. A run that is no longer
  on the list returns "That deployment is no longer waiting."
- They send call 6 with the environments the user can approve. The
  comment is "Approved in Maya." or "Rejected in Maya."
- After GitHub answers, Maya checks that one repo again (calls 3 to 5),
  replaces that repo's rows in `DeploymentState`, and emits
  `deployments`. If GitHub refuses, the row shows the scrubbed error
  next to its buttons, and the buttons are enabled again.
- While a call is running, both buttons on that row are disabled.

## The poll

- A new thread in `lib.rs`, next to the reviews thread:
  `poll_deployments` every 120 s. It follows `poll_reviews`: it checks
  `git_generation`, takes the GitHub forge from `git::forges` (GitLab is
  ignored), fetches, stores and emits `deployments`.
- **Only a main or a standalone Maya polls.** An assistant does not, so
  the same token is not spent once per machine. An assistant's tab says
  "Deployments are checked on the main."
- If GitHub is off: `off: true`, and the tab says to switch GitHub on in
  Settings → Git.
- If the whole check fails (no network, rate limit, or the repo list
  call refuses): log it, keep the last list, and set `error`. The tab
  shows the error above the list, as Pull Requests does.
- If a call for one repo fails (for example, an org that blocks the
  token through SSO, or a 404): log it and skip that repo. The other
  repos still show.
- **If every checked repo refuses with 401 or 403**, that is an error for
  the whole tab, not an empty list. The error names the permissions the
  token needs (see Token).
- A command `deployments()` returns the current snapshot, for the tab's
  first paint.

## The tab

- `index.html`: a tab button "Deployments" with a
  `<span class="tab__count" id="deployments-count">` after Pull Requests,
  and a pane `id="deployments"`.
- `src/tabs.ts`: add `"deployments"` to `Tab`, `PANES` and `asTab`. The
  count follows `setCount`.
- `src/deployments.ts` (new), shaped like `src/reviews.ts`: it listens
  for `deployments`, paints the rows, and calls the two commands.
- **Approval row:** repo · workflow · branch, the commit title,
  environment names (for example `production-eu-west-2`), who triggered
  it, and how long it has waited (repainted by the 10 s timer in
  `main.ts`). Then **Reject** and **Approve**. The workflow name links
  to the run.
- **Blocked row:** the same facts (no environment), indented under its
  blocker when the blocker is shown, then "Waiting on `a1b2c3d` title"
  linked to the blocker's run. No buttons.
- **Empty state:** "No deployments are waiting on you."

## Token

- A classic token with `repo` covers every call. A fine-grained token
  needs **Actions: read**, **Deployments: read and write**,
  **Environments: read** and **Metadata: read**. Check these against
  GitHub's docs while building.
- The 403 message for the Deployments tab names these permissions. The
  Pull Requests tab keeps its own message: `forge::scopes` gets a
  variant per use, rather than one longer message for both.

## Tests

- `core` (Rust, `FakeHttp`, canned replies):
  - The repo list stops at the first repo older than 7 days, and after
    5 pages.
  - The environment answer: a repo with `required_reviewers` is checked,
    a repo without one is not, the answer is reused within the hour and
    asked again after it, and a 404 counts as no.
  - A `waiting` run where the user can approve gives an approval row
    with only the approvable environments. A run that waits only on other
    people gives no row.
  - Blocker matching: a waiting blocker, an in-progress blocker, a
    queued blocker, and no blocker ("another deployment"). The
    in-progress and queued calls are made only when needed.
  - Approve and Reject send the right body and the right environment ids.
  - A failing repo is skipped and the others still show. Every repo
    refusing with 403 gives a tab error. Errors are scrubbed of the
    token.
  - Ordering, and the count.
- Frontend (vitest): `asTab` accepts `deployments`; the rows, the
  waiting text and the empty state paint; buttons disable during a call
  and show an error on refusal; the assistant message shows on an
  assistant.

## Release

- A `feat`: bump the minor version with `scripts/set-version.sh`,
  committed on its own as `chore(release): <version>`.
- Add a short section on the Deployments tab and its token permissions
  to `docs/DEVELOPING.md` (Git integration) and to the README.

## Calls Claude made (check these)

- Only repos with an environment that requires reviewers are checked,
  and that answer is kept for 1 hour. Without this, the 108 active repos
  cost about 6,500 calls an hour.
- Only the main (or a standalone Maya) polls; assistants do not.
- Every 401/403 means a tab error; one repo's failure is skipped.
- The blocker is the newest earlier run of the same workflow that waits,
  runs or is queued. This fits `coral-boards` and `lilypad`.
- Desktop only: the phone does not get the tab.
- No banner and no voice when a new deployment starts to wait.
- No confirmation step on Approve or Reject: one click sends it.
- When one run waits on several environments that the user can approve,
  one Approve approves all of them, and the row lists them all.
- The comment sent to GitHub: "Approved in Maya." / "Rejected in Maya."

## Left to the builder

- Where the code lives: a new `core/src/deployments.rs` with the GitHub
  calls in `core/src/forge/github.rs`, as with reviews, unless a better
  split shows.
- Styling. Reuse the Pull Requests tab's row styles.
