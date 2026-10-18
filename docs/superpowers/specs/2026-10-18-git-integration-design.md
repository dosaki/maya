# Git integration: GitHub and GitLab — design

Date: 2026-10-18
Status: implemented

## Purpose

Today Maya does everything Git-related through the `gh` CLI, with the
token `gh` finds: the PR badge on a session card, the Pull requests tab
and its review notifications, and the clone a "Review" starts with. Users
of GitLab get none of it, and users of GitHub must install and log in to
`gh`.

This change adds **Settings → Git**: GitHub and GitLab, each switched on
or off on its own, each with a token that is either pasted (and kept in
the OS key store) or read from a named environment variable. GitLab has a
base URL, `https://gitlab.com` when left blank. Maya then talks to the
GitHub and GitLab APIs directly and never runs `gh`.

Success:

- With only GitHub on and a pasted token, everything that works today
  still works, with `gh` uninstalled.
- With GitLab on (gitlab.com or a self-hosted URL), a session in a
  GitLab checkout shows its MR on its card, the Pull requests tab lists
  the MRs waiting on the user next to the GitHub PRs, "Review" clones and
  starts a review session, and an MR of the user's that gets approved or
  has changes requested is announced like a PR.
- With both off, Maya makes no Git API calls and the Pull requests tab
  says the integration is off.
- A user upgrading with `gh` logged in notices nothing: their GitHub
  token is imported once.

## Decisions

- **Both providers at once.** GitHub and GitLab are independent; "None"
  is both switched off. There is no single provider picker.
- **One instance of each.** github.com, and one GitLab instance at a
  configurable base URL. Config stores a list so more instances (another
  GitLab, GitHub Enterprise) can come later without a migration. Not in
  this change.
- **No `gh`.** All calls go to the provider APIs over `ureq`, already a
  dependency of `core`. Cloning uses plain `git clone` with the user's
  own git credentials.
- **Token sources:** a pasted token, stored in the OS key store
  (`src-tauri/src/keystore.rs`, as the ElevenLabs key is), or the name of
  an environment variable. The token never enters `config.json` and never
  reaches the webview.
- **Upgrade:** a config without the `git` field gets GitHub switched on.
  If `gh auth token` succeeds once at that first start, its output is
  stored as GitHub's pasted token; otherwise GitHub reads `GH_TOKEN`.
  GitLab starts off. After that, `gh` is never run.
- **Each Maya has its own Git settings.** An assistant machine looks up
  PRs for its own sessions with its own settings. The phone has none: it
  mirrors its main's board.

## Settings → Git

A new settings group, `git`, titled "Git", placed before "Pull requests"
(which keeps the clones folder and the review prompt).

```
GitHub    [on/off]
  Token:  (•) Paste a token  [••••••••]  [Clear]   ( ) From env var  [GH_TOKEN    ]
  [Test]  Connected as @login
GitLab    [on/off]
  URL:    [                          ]  placeholder https://gitlab.com
  Token:  (•) Paste a token  [        ]            ( ) From env var  [GITLAB_TOKEN]
  [Test]
```

- The fields under a provider are disabled while it is off.
- **Paste a token:** a password field. When a token is stored it shows
  "Stored" and a Clear button; the field stays empty. Saving a non-empty
  field calls `store_git_token(provider, token)`; Clear calls
  `clear_git_token(provider)`. The page learns only `hasToken: bool`
  through `get_git_status`.
- **From env var:** a name field, defaulting to `GH_TOKEN` (GitHub) and
  `GITLAB_TOKEN` (GitLab). A blank name reads as the default. The name
  goes into a shell line (the login-shell probe), so `set_config` refuses
  anything but letters, digits and `_`, not starting with a digit.
- **GitLab URL:** blank means `https://gitlab.com`. On save it is trimmed
  of trailing slashes and of a pasted `/api/v4`, and must parse as an `http` or `https` URL; a path
  is allowed (`https://example.com/gitlab`). An invalid URL is refused by
  `set_config` with a message naming the field.
- **Test** calls `test_git_connection(provider)`, which resolves the
  token and makes one `whoami` call. It shows "Connected as @login" or
  the error text.

## Config

`core/src/config.rs` gains:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub git: Option<Vec<GitConnection>>,

pub struct GitConnection {
    pub provider: GitProvider,          // "github" | "gitlab"
    pub enabled: bool,
    pub base_url: Option<String>,       // gitlab only; None = https://gitlab.com
    pub token: TokenSource,
}

pub enum TokenSource {
    Stored,                              // the key store item for the provider
    Env { name: String },
}
```

- `Config::default()` has GitHub on with `Env { name: "GH_TOKEN" }` and
  GitLab off with `Env { name: "GITLAB_TOKEN" }`, base URL None.
- `git_connections()` returns one GitHub and one GitLab connection, in
  that order, with the default for any missing, so the page always has
  one of each.
- A config with no `git` field (None) needs the one-time `gh` import
  (see Upgrade). The marker is the missing field, not a separate flag:
  once saved, the field exists.

## Tokens

- **Stored:** two new key store items, `GITHUB_TOKEN` (service
  `maya-github`) and `GITLAB_TOKEN` (service `maya-gitlab`), account
  `token`.
- **Env:** on Unix the value is read from the login shell (`$SHELL -lic`),
  because a GUI app on macOS does not inherit `~/.zshrc`. This
  generalises today's `gh_token_probe`/`shell_gh_tokens` into
  `shell_env(name)`, cached per name for the life of the process. On
  Windows, the process environment. The process's own environment wins
  when the variable is set there and non-empty, as today.
- **Resolution** (`forge::resolve_token`) returns `Ok(token)`, or
  `Err("No GitHub token: paste one or set GH_TOKEN in Settings → Git")`
  for an empty or missing value. A forge with no token makes no call.
- **Scopes** named in 401/403 messages: GitHub classic `repo`
  (fine-grained: Pull requests read, Metadata read); GitLab `read_api`.

## The forge module

`core/src/forge/` replaces every use of `gh`.

### `forge/mod.rs`

- `Forge { provider, base_url, token }`: one enabled, resolved
  connection. `api_base()` is `https://api.github.com` for GitHub and
  `<base_url>/api/v4` for GitLab; `graphql_url()` is
  `https://api.github.com/graphql` or `<base_url>/api/graphql`.
- `trait Http` with `get(url, token)` and `post_json(url, token, body)`
  returning `(status, body)`. The real one is `ureq` with a 15 s timeout
  and the header `Authorization: Bearer <token>` (both APIs accept it).
  Tests use a canned implementation.
- **Error text** goes through `scrub(text, token)`, which replaces the
  token with `***`. No token is logged.
- **Routing:** `route(remote, forges) -> Option<(&Forge, String)>` turns
  a remote URL into a forge and a repo path:
  - `git@HOST:PATH(.git)`, `ssh://git@HOST(:port)/PATH(.git)`,
    `https://HOST(/prefix)/PATH(.git)`; trailing slashes and `.git`
    stripped.
  - GitHub matches host `github.com`; the path is `owner/name`.
  - GitLab matches the host of its base URL. For an `https` remote, a
    base URL path prefix (`/gitlab`) is stripped from the remote path
    first. The path keeps nested groups: `grp/sub/repo`.
  - Matching is case-insensitive on host and path. No match, or a
    provider that is off, returns None.
  This replaces `reviews::remote_matches`.

### `forge/github.rs`

- `current_pr(http, forge, repo, branch) -> Result<Option<PullRequest>>`:
  `GET /repos/{repo}/pulls?head={owner}:{branch}&state=all&per_page=10`.
  Picks the first open one, else the most recently updated. State is
  `draft` when `draft` is true, else `open`, or `merged` when
  `merged_at` is set, else `closed`. PRs from forks are not found (as a
  limitation, noted).
- `review_prs(http, forge) -> Result<Vec<ReviewPr>>`: POSTs today's
  `reviews::QUERY` to the GraphQL URL; `reviews::parse_graphql` is kept
  as is. A body with `errors` and no data is an error; partial data is
  used, as today.
- `whoami(http, forge) -> Result<String>`: `GET /user` → `login`.

### `forge/gitlab.rs`

- `current_pr`: `GET /projects/{url-encoded repo}/merge_requests?source_branch={branch}&state=all&order_by=updated_at&per_page=10`.
  Same pick rule. State: `opened` → `open` (or `draft` when `draft` is
  true), `merged`, `closed`/`locked` → `closed`. `number` is the `iid`,
  `url` the `web_url`.
- `review_prs`: one GraphQL query:

  ```graphql
  query {
    currentUser {
      username
      requested: reviewRequestedMergeRequests(state: opened, first: 50, sort: UPDATED_DESC) { nodes { ...mr } }
      assigned: assignedMergeRequests(state: opened, first: 50, sort: UPDATED_DESC) { nodes { ...mr } }
    }
  }
  fragment mr on MergeRequest {
    iid title webUrl draft updatedAt
    project { fullPath }
    author { username }
    reviewers { nodes { username bot mergeRequestInteraction { reviewState } } }
  }
  ```

  Parsed into `ReviewPr` with the same rules as GitHub: `my_review` from
  the user's own reviewer entry, `pr_review`/`pr_reviewers` from the
  others (not bots, not the author), where changes requested by anyone
  wins, then an approval, then a comment. Mapping: `APPROVED` →
  Approved, `REQUESTED_CHANGES` → ChangesRequested, `REVIEWED` →
  Commented; `UNREVIEWED`, `REVIEW_STARTED`, `UNAPPROVED` → none. An MR
  in both lists gets both reasons, as `reviews::merge` does today. A
  reviewer stays on a GitLab MR after reviewing, so GitHub's extra
  "reviewed by me" search has no GitLab counterpart. The field names are
  checked against a live gitlab.com response during implementation; a
  fixture of that response backs the parser test.
- `whoami`: `GET /user` → `username`.

### Shared types

`ReviewPr` gains `forge: GitProvider`; `repo` holds the full path (may
have more than two segments on GitLab). `PullRequest` gains `forge` too,
so the card can say "MR !12" or "PR #12".

## Wiring

- **Badge:** `pr::lookup(dir, forges)` reads `remote.origin.url` and
  `git symbolic-ref --short -q HEAD` (a detached `HEAD` means no PR),
  routes the remote, and calls the forge's `current_pr`. An error is
  cached as "none" for `TTL_MS` (60 s), as a failed `gh` call is today,
  so it is logged at most once a minute per directory.
- **Pull requests tab:** `reviews::fetch(forges)` asks every enabled
  forge on its own thread and merges the lists, newest update first. The
  result is `{ prs, errors: [{ forge, message }] }`. `ReviewState.error:
  Option<String>` becomes `errors: Vec<ForgeError>`; PRs from a forge
  that failed this time are kept from its last success, so one failing
  provider does not empty the tab. With no forge enabled, `fetch` is not
  called and the state says `off: true`.
- **Notifications:** `PrNotifier` is keyed by URL and works unchanged
  for MRs. `pr_banner` titles an MR `repo !12`, and
  `pr_spoken_line` says "repo merge request 12".
- **Review clone:** `shell_command` runs
  `git clone '<web base>/<repo>.git' '<dir>'`, where the web base is
  `https://github.com` or the GitLab base URL. `find_checkout` and
  `resolve_target` match folders through `forge::route`, and take the
  forge as well as the repo.
- **PR identity:** everything that names one PR — the tab's `ReviewAction`
  (`open-pr`, `review`, `terminal`), the `review`/`open` voice actions,
  and their `revalidate` on the main — carries `forge` beside `repo` and
  `number`, and looks the PR up by all three. An action without `forge`
  (from an older page or phone) means GitHub.
- **Voice:** `interpreter::pr_summary` writes GitLab lines as
  `grp/repo!12 (GitLab) | title | by author`. `find_pr` accepts
  `repo#n`, `repo!n`, a bare number and title words as today; a number
  that matches a PR and an MR asks "Which one: …#14 or …!14?".
- **Settings changes:** `set_config` clears the PR cache and starts one
  review poll when the `git` field changed. So do `store_git_token` and
  `clear_git_token`.
- **Removed:** `lib.rs`'s `gh()`, `GH_TOKEN_VARS`, `has_gh_token`,
  `gh_token_probe`, `parse_gh_token_probe`, `shell_gh_tokens` (the last
  two generalised into `shell_env`). `DEVELOPING.md` drops `gh` from the
  requirements and gains a short "Git integration" note.

## Upgrade

At startup, when the loaded config had no `git` field:

1. Run `gh auth token` once (found the way `gh()` finds it today, with
   a 5 s timeout, output never logged).
2. If it prints a token, store it as `GITHUB_TOKEN` in the key store and
   set GitHub's source to `Stored`.
3. Otherwise keep `Env { name: "GH_TOKEN" }`. Today's fallback to
   `GITHUB_TOKEN` survives as: an Env source named `GH_TOKEN` that is
   empty also tries `GITHUB_TOKEN`.
4. Save the config, so this never runs again.

## Pull requests tab

- Each card shows a small GitHub or GitLab mark before the repo name.
  GitLab cards read `repo !12`, GitHub cards `repo #12`.
- The repo filter's value is `<forge>:<repo>`, so the same path on both
  providers stays two options. Labels are unchanged (short name unless
  two share it).
- One error line per failing forge: "GitHub check failed: …" or
  "GitLab check failed: …".
- With no forge on: "Git integration is off. Turn on GitHub or GitLab in
  Settings → Git." with a button that opens that group.
- `relatedSession` matches on the review session name as today; its
  clone folder name is the last path segment plus the number, as today.
- Session cards' PR badge reads "MR !12" for GitLab.

## Errors

| Situation | What the user sees |
|---|---|
| Provider on, no token | "No GitLab token: set GITLAB_TOKEN or paste a token in Settings → Git." in the tab; no calls |
| 401 / 403 | "GitLab check failed: the token was refused (401). It needs the read_api scope." |
| Timeout, DNS, TLS | "GitLab check failed: could not reach gitlab.acme.io: <reason>" |
| GraphQL `errors` with no data | the first error's `message` |
| Bad GitLab URL in Settings | `set_config` refuses: "GitLab URL must be an http(s) address." |
| Key store unavailable | "Could not store the token: <reason>" on save; the source stays as it was |

Every message passes through `scrub` before it is logged or shown.

## Testing

TDD throughout. No test touches the network.

Rust (`core`):

- `route`: GitHub HTTPS, SSH and `ssh://` remotes; GitLab nested groups;
  a self-hosted base URL with a path prefix; a port in `ssh://`; host
  mismatch; a switched-off provider; case differences.
- GitHub `current_pr` and GitLab `current_pr` parsing: open preferred,
  draft, merged, closed, none.
- GitLab GraphQL parsing into `ReviewPr`, from a fixture: reasons from
  both lists, `my_review`, `pr_review` precedence, bots and the author
  ignored.
- `reviews::fetch` merging two forges, one failing: the other's PRs and
  one error.
- Token resolution: stored, env, empty, the `GITHUB_TOKEN` fallback.
- `scrub` removes the token from an error.
- Config: default, an old file with no `git` field (needs import), round
  trip, URL validation.
- `shell_command` for both forges.
- The interpreter: `pr_summary` for a GitLab MR, `find_pr` by `repo!n`,
  the PR/MR number clash, and `revalidate` refusing an action whose
  forge changed.

Frontend (vitest):

- The Git group: the toggles disable their fields, the radio choice
  shows the right field, a blank GitLab URL saves as None, the token
  field is never filled from state, Clear and Test call their commands.
- Cards: the forge mark, `!n` for GitLab, the repo filter keeping two
  same-path repos apart.
- The tab: one error line per forge, the "integration is off" state.

By hand (owed after merge, as for earlier features): a real github.com
token, a gitlab.com token, a self-hosted GitLab if one is available, and
env var mode on the Mac, laptop and Windows.

## Release

A `feat`: main is at 0.23.0, so this ships as 0.24.0, set with
`sh scripts/set-version.sh 0.24.0` in its own `chore(release): 0.24.0`
commit on `feat/git-integration`.

## Not in this change

- More than one GitLab instance, and GitHub Enterprise.
- PRs opened from forks on the session card.
- Writing to the forge (approving, commenting) from Maya.
