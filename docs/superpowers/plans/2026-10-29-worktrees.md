# Worktrees Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An optional "New worktree" for new sessions, and PR reviews that run in worktrees of one clone per repository under the clones directory.

**Architecture:** A new `core/src/worktree.rs` makes a new session's worktree with `git` in the core, before the agent starts. Reviews describe their git work as a list of `git` argument lists (`reviews::git_steps`): the terminal's shell line runs them for every agent but OpenCode, and the core runs them for OpenCode. A new protocol cap tells the main which paired machines understand the option.

**Tech Stack:** Rust (maya-core, Tauri app, CLI), vanilla TypeScript (vitest), `git` on PATH.

**Spec:** `docs/superpowers/specs/2026-10-29-worktrees-design.md`

## Global Constraints

- Spawn `git` from the core with `crate::command("git")` (it hides the console window on Windows), never `std::process::Command::new` outside tests.
- Every path, URL and ref in a shell line is quoted with `launch::shell_single_quote`.
- Maya deletes no worktree, branch or folder, ever.
- The worktree option off changes nothing: same folder, same command, same result as today.
- Worktree folders: `<clones>/<folder>-<slug>` for a session, `<clones>/<repo>-<number>` (`<repo>-mr<number>` on GitLab) for a review; the clone is `<clones>/<repo>`.
- User-facing copy, verbatim:
  - checkbox: `New worktree`
  - warning: `Once this session is closed, Maya cannot resume it from the Resume list. Resume it from its folder with the agent.`
  - `<folder> is not a git repository, so it cannot start in a worktree.`
  - `<folder> is not a clone of <repo>; move it or change the clones directory.`
  - `A review of <repo> <sigil><number> is already running.`
  - `<machine> runs an older Maya that cannot start a session in a worktree.`
  - prompt sentence: `The pull request is checked out in this folder.`
  - settings label: `Clones directory (for reviews and worktrees)`
- Version: `feat` → bump `main`'s 0.39.2 to 0.40.0 with `sh scripts/set-version.sh 0.40.0`, committed alone as `chore(release): 0.40.0`.

## Review Focus

1. A prompt or name with no ASCII letters or digits ("修复登录", "🚀") must still give a valid branch (`session`), not an empty one. Pinned in Task 1.
2. A project folder inside a repo but not its top (a monorepo package) must refuse with the "not a git repository" message, not make a worktree of the parent. Pinned in Task 1.
3. A repo with no commits yet (unborn `HEAD`) must return git's message as the error, not panic. Pinned in Task 1.
4. A clones directory whose path has spaces or a `'` must still give a shell line that runs. Pinned in Task 4.
5. A review worktree the user deleted by hand must be added again (prune first), not fail as "already registered". Pinned in Task 4.

---

### Task 1: `core/src/worktree.rs`, a new session's worktree

**Files:**
- Create: `core/src/worktree.rs`
- Modify: `core/src/lib.rs` (add `pub mod worktree;` in alphabetical place)

**Interfaces:**
- Produces:
  - `pub fn branch_slug(name: Option<&str>, prompt: &str) -> String`
  - `pub fn add_for_session(checkout: &Path, clones_root: &Path, name: Option<&str>, prompt: &str) -> Result<PathBuf, String>` returns the new worktree's folder.

**Slug rules:** the trimmed name when it is non-empty, else the prompt's first 6 whitespace-separated words; lower-cased; every run of characters that are not ASCII alphanumerics becomes one `-`; `-` trimmed from both ends; cut to at most 40 characters and trimmed of `-` again; `session` when nothing is left.

**`add_for_session`:**
1. `git -C <checkout> rev-parse --show-toplevel`; compare its output with `checkout` after `std::fs::canonicalize` of both (macOS `/tmp` is `/private/tmp`). Failure or mismatch → `Err("<folder> is not a git repository, so it cannot start in a worktree.")`, `<folder>` being `checkout`'s file name.
2. Candidates `slug`, `slug-2`, `slug-3` … up to `slug-100`: the first where `git -C <checkout> show-ref --verify --quiet refs/heads/<candidate>` fails **and** `<clones_root>/<folder>-<candidate>` does not exist.
3. `std::fs::create_dir_all(clones_root)`, then `git -C <checkout> worktree add -b <candidate> <clones_root>/<folder>-<candidate> HEAD`. Non-zero exit → `Err` with git's stderr, trimmed.

- [ ] **Step 1: Write the failing tests** in `worktree.rs`'s `mod tests`. A helper `repo(root, name) -> PathBuf` runs `git init -q`, sets `user.email`/`user.name` locally, writes a file and commits it.

```rust
#[test]
fn a_slug_comes_from_the_name_else_the_prompt() {
    assert_eq!(branch_slug(Some("Fix CI: v1.2/..lock"), "ignored"), "fix-ci-v1-2-lock");
    assert_eq!(branch_slug(Some("  "), "Add the login page and its tests please now"), "add-the-login-page-and-its");
    assert_eq!(branch_slug(None, "修复登录 🚀"), "session");
    assert!(branch_slug(None, &"word ".repeat(30)).len() <= 40);
    assert!(!branch_slug(None, &"abcdefghij ".repeat(6)).ends_with('-'));
}

#[test]
fn a_worktree_branches_off_the_checkouts_current_branch() {
    // repo "proj" on branch "feature" (git switch -c feature after the first commit)
    let wt = add_for_session(&proj, &clones, Some("Fix CI"), "x").unwrap();
    assert_eq!(wt, clones.join("proj-fix-ci"));
    // `git -C wt rev-parse --abbrev-ref HEAD` == "fix-ci"
    // `git -C wt rev-parse HEAD` == `git -C proj rev-parse feature`
    // `git -C proj rev-parse --abbrev-ref HEAD` is still "feature"
}

#[test]
fn a_taken_branch_or_folder_gets_the_next_number() {
    add_for_session(&proj, &clones, Some("fix"), "x").unwrap();
    std::fs::create_dir_all(clones.join("proj-fix-2")).unwrap();
    assert_eq!(add_for_session(&proj, &clones, Some("fix"), "x").unwrap(), clones.join("proj-fix-3"));
}

#[test]
fn only_the_top_of_a_repository_can_start_in_a_worktree() {
    let plain = root.join("plain"); std::fs::create_dir_all(&plain).unwrap();
    assert_eq!(add_for_session(&plain, &clones, None, "x").unwrap_err(), "plain is not a git repository, so it cannot start in a worktree.");
    let pkg = proj.join("packages/api"); std::fs::create_dir_all(&pkg).unwrap();
    assert_eq!(add_for_session(&pkg, &clones, None, "x").unwrap_err(), "api is not a git repository, so it cannot start in a worktree.");
}

#[test]
fn a_repository_without_commits_returns_gits_message() {
    // `git init -q empty`, no commit
    let e = add_for_session(&empty, &clones, None, "x").unwrap_err();
    assert!(!e.is_empty() && !e.contains("not a git repository, so"), "{e}");
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p maya-core worktree::`
Expected: FAIL to compile (`branch_slug` not found).

- [ ] **Step 3: Implement `branch_slug` and `add_for_session`** as specified above.

- [ ] **Step 4: Run them to see them pass**

Run: `cargo test -p maya-core worktree::`
Expected: 5 passed.

- [ ] **Step 5: Commit**

```bash
git add core/src/worktree.rs core/src/lib.rs
git commit -m "feat(core): make a worktree for a new session"
```

---

### Task 2: the option on `LaunchOptions`, `start_session` and the CLI

**Files:**
- Modify: `core/src/launch.rs:146-157` (`LaunchOptions`)
- Modify: `core/src/actions.rs:658-691` (`start_session`)
- Modify: `cli/src/args.rs:32` (usage), `cli/src/args.rs:125-150` (`parse_start`)

**Interfaces:**
- Consumes: `worktree::add_for_session` (Task 1).
- Produces: `LaunchOptions.worktree: bool`, "start in a new worktree"; `#[serde(default)]` already covers the struct, so an older sender's JSON decodes to `false`.

- [ ] **Step 1: Write the failing tests**
  - `launch.rs`: `a_launch_without_worktree_decodes_as_off`: `serde_json::from_str::<LaunchOptions>(r#"{"agent":"claude-code"}"#).unwrap().worktree == false`, and `{"agent":"claude-code","worktree":true}` gives `true`.
  - `actions.rs`, next to the `start_session` tests (they use `store_with_projects` and `FakeTerminal`): `start_session_in_a_worktree_runs_the_agent_there`. Make `proj` a git repo with one commit (as Task 1's helper), set `config.clones_dir` to `<tmp>/clones`, start with `LaunchOptions { worktree: true, name: Some("Fix CI".into()), ..Default::default() }`. Assert `r.dir` ends with `clones/proj-fix-ci`, and the opened command starts with `cd '<…>/clones/proj-fix-ci'`.
  - `actions.rs`: `start_session_in_a_worktree_refuses_a_folder_that_is_not_a_repository`: plain `proj`, `worktree: true` → `Err("proj is not a git repository, so it cannot start in a worktree.")`, and `fake.calls` is empty.
  - `actions.rs`: `start_session_without_the_option_is_unchanged`: the existing `start_session` tests still pass untouched (no new assertion needed; run them).
  - `cli/src/args.rs`: `parse(&["start", "--dir", "proj", "--worktree"])` gives `options.worktree == true`; without the flag, `false`.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p maya-core worktree && cargo test -p maya-cli start`
Expected: FAIL to compile (`no field worktree`).

- [ ] **Step 3: Implement**
  - `LaunchOptions`: `pub worktree: bool`, doc `/// Start in a new worktree on a new branch, under the clones directory.`
  - `start_session`: after `launch::resolve_target`, when `options.worktree`: refuse `how == "fallback"` with the "not a git repository" message for the projects root's file name; else replace the target with `crate::worktree::add_for_session(&target, &store.config.clones_dir_path(), options.chosen_name(), &prompt)?` (read `clones_dir_path()` in the first store lock with `root` and `maya_dir`). `how` is unchanged.
  - CLI: `"--worktree" => options.worktree = true,` (no value), and `[--worktree]` at the end of the `maya start` usage line.

- [ ] **Step 4: Run them to see them pass**

Run: `cargo test -p maya-core && cargo test -p maya-cli`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add core/src/launch.rs core/src/actions.rs cli/src/args.rs
git commit -m "feat: start a new session in a worktree from the core and the CLI"
```

---

### Task 3: paired machines say whether they take the option

**Files:**
- Modify: `core/src/net/protocol.rs:23-33` (new cap)
- Modify: `core/src/net/merge.rs:11-27` (`RemoteBoard`), and add `worktrees_of` next to `agents_of` (`merge.rs:124`)
- Modify: `core/src/net/server.rs:298-310` (`sync_boards`), `server.rs:1102` and every other `RemoteBoard { … }` literal (tests at `server.rs:2232`, `merge.rs:138`)
- Modify: `core/src/net/routing.rs:34-80` (`AgentsReply`, `agents_reply`, new check)
- Modify: `src-tauri/src/net_app.rs:177-179` (caps), `net_app.rs:454-477` (ListAgents, Start routing)
- Modify: `cli/src/notify.rs:50-52` (caps)
- Modify: `src-tauri/src/lib.rs:1336-1354` (`list_agents`, `start_session`), and its test at `lib.rs:1959`
- Modify: `mobile/src/commands.rs:157` (new argument, `false`)

**Interfaces:**
- Consumes: `LaunchOptions.worktree` (Task 2).
- Produces:
  - `pub const CAP_WORKTREE: &str = "worktree";` doc `/// A Maya with this cap starts a session in a new worktree when asked (`LaunchOptions::worktree`).`
  - `RemoteBoard.worktrees: bool`: connected and announced `CAP_WORKTREE`.
  - `pub fn worktrees_of(remotes: &[RemoteBoard], machine: &str) -> bool`
  - `AgentsReply { agents, names, worktrees: bool }`, serialised as `worktrees`.
  - `pub fn agents_reply(local: bool, remote: Option<Vec<AgentInfo>>, remote_worktrees: bool, local_list: impl FnOnce() -> Vec<AgentInfo>) -> AgentsReply`: `worktrees` is `true` locally, `remote_worktrees` for a remote.
  - `pub fn check_remote_worktree(options: &LaunchOptions, worktrees: bool, machine: &str) -> Result<(), String>`: `Err("<machine> runs an older Maya that cannot start a session in a worktree.")` when `options.worktree && !worktrees`.

- [ ] **Step 1: Write the failing tests**
  - `routing.rs`: extend the `agents_reply` tests (`routing.rs:199-206`): local → `worktrees == true`; remote with `remote_worktrees` false → false, true → true. New `check_remote_worktree_refuses_an_older_maya`: `worktree: true, worktrees: false` → the exact error for machine `"mini"`; `worktree: false, worktrees: false` → `Ok`; `worktree: true, worktrees: true` → `Ok`.
  - `server.rs`: following the board test at `server.rs:2017`, `a_board_says_whether_its_maya_takes_worktrees`: connect one assistant with caps `&[CAP_WORKTREE]` and one with `&[]` (helper `connect_as` / `pair_client_caps`), each sends a board; the main's boards have `worktrees` true and false.
  - `lib.rs:1959` test: update the calls to the new signature, and assert `agents_reply(false, None, true, …).worktrees == true`.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p maya-core net:: && cargo test -p maya older_remote`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**
  - `sync_boards`: `worktrees: self.peers.get(&id).is_some_and(|p| p.caps.iter().any(|c| c == CAP_WORKTREE))` in the relabelled `RemoteBoard`; literals elsewhere get `worktrees: false`.
  - Caps: the app's `caps()` returns `[CAP_FOLLOW, CAP_LEAD, CAP_WORKTREE]`; the CLI's `[CAP_FOLLOW, CAP_WORKTREE]`. The phone (`mobile/src/hub.rs`) runs no sessions and gets none.
  - `lib.rs list_agents`: pass `merge::worktrees_of(&boards, machine)` for a remote. `lib.rs start_session` remote branch: call `check_remote_worktree` after `check_remote_agent`.
  - `net_app.rs` `Request::ListAgents`: same as `list_agents`. `Target::Machine` with `Some(m)`: when `kind` is `CommandKind::Start { options, .. }`, call `check_remote_worktree(options, merge::worktrees_of(&crate::remote_boards(&state), m), m)?` next to `check_remote_agent`.

- [ ] **Step 4: Run them to see them pass**

Run: `cargo test --workspace`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add core/src/net src-tauri/src cli/src/notify.rs mobile/src/commands.rs
git commit -m "feat(net): tell the main which machines start sessions in worktrees"
```

---

### Task 4: reviews in worktrees of one clone (`reviews.rs`)

**Files:**
- Modify: `core/src/reviews.rs:279-370` (`find_checkout`, `ReviewTarget`, `resolve_target`, `DEFAULT_REVIEW_PROMPT`, `shell_command`) and its tests (`reviews.rs:560-590`, `reviews.rs:698-723`)

**Interfaces:**
- Produces:

```rust
pub struct ReviewTarget {
    /// The one clone of the repository: `<clones>/<repo>`.
    pub clone_dir: PathBuf,
    /// Where the review runs: `<clones>/<repo>-<number>` (`-mr<number>` on GitLab).
    pub dir: PathBuf,
    /// The forge's ref: `pull/<n>/head` on GitHub, `merge-requests/<n>/head` on GitLab.
    pub pr_ref: String,
    /// Maya's ref it is fetched into: `refs/maya/review/<n>` (`refs/maya/review/mr<n>` on GitLab).
    pub local_ref: String,
    /// `clone_dir` does not exist yet.
    pub clone: bool,
    /// `dir` is a worktree already (its `.git` is a file).
    pub worktree_exists: bool,
    /// `dir` is a full clone an earlier Maya made (its `.git` is a folder): review in it with no git steps.
    pub legacy: bool,
}
pub fn resolve_target(clones_root: &Path, site: &ForgeSite, repo: &str, number: u64) -> Result<ReviewTarget, String>
/// The `git` argument lists a review runs before its agent, in order.
pub fn git_steps(target: &ReviewTarget, clone_url: &str) -> Vec<Vec<String>>
/// Runs `git_steps` with `crate::command("git")`; the first failure returns git's stderr, trimmed.
pub fn run_git_steps(steps: &[Vec<String>]) -> Result<(), String>
pub fn shell_command(agent: Harness, target: &ReviewTarget, clone_url: &str, session: &str, prompt_file: &Path, grok_id: Option<&str>) -> String  // signature unchanged
```

`resolve_target`: `Err("<clone_dir> is not a clone of <repo>; move it or change the clones directory.")` (path shown with `display()`) when `clone_dir` exists and `!is_checkout_of(&clone_dir, site, repo)`. It no longer reads the projects directory; delete `find_checkout` and its test when nothing else uses it (grep first). `is_busy` stays.

`git_steps`, empty when `legacy`, else in order:
1. when `clone`: `clone <url> <clone_dir>`
2. `-C <clone_dir> fetch origin`
3. `-C <clone_dir> fetch origin +<pr_ref>:<local_ref>`
4. when `worktree_exists`: `-C <dir> checkout --detach <local_ref>`; else `-C <clone_dir> worktree prune`, then `-C <clone_dir> worktree add --detach <dir> <local_ref>`

`shell_command`: legacy → the agent's start line alone (as today's free checkout). Else `mkdir -p <clones root> && git … && git … && <start>`, rendering each argument bare when it is only `[A-Za-z0-9_-]`, else through `shell_single_quote`.

`DEFAULT_REVIEW_PROMPT`: insert `The pull request is checked out in this folder.` as the second sentence, right after `Review pull request #{number} of {repo} ({url}).`

- [ ] **Step 1: Write the failing tests** (replace `resolve_target`'s and the shell line's existing tests)
  - `a_review_targets_the_one_clone_and_a_worktree_per_pr`: empty clones root → `clone == true`, `dir == clones/bedrock-451`, `clone_dir == clones/bedrock`, `pr_ref == "pull/451/head"`, `local_ref == "refs/maya/review/451"`; GitLab `grp/r` 12 → `dir == clones/r-mr12`, `pr_ref == "merge-requests/12/head"`, `local_ref == "refs/maya/review/mr12"`.
  - `a_review_reuses_the_clone_and_spots_old_full_clones`: `repo_with_origin(clones, "bedrock", "git@github.com:Org/bedrock.git")` → `clone == false`; a `bedrock-451` made with `repo_with_origin` → `legacy == true`.
  - `a_clone_folder_of_another_repository_is_refused`: `repo_with_origin(clones, "bedrock", "git@github.com:Other/bedrock.git")` → the exact error.
  - `the_review_steps_clone_once_fetch_and_add_or_update_the_worktree`: compare `git_steps` with the expected lists for (fresh), (clone exists, no worktree: no `clone`, has `worktree prune` then `worktree add`), (worktree exists: ends with `checkout --detach`), (legacy: empty).
  - `the_review_shell_line_quotes_paths_with_spaces_and_quotes`: clones root `/Users/x/my reviews/it's` → the line contains `git clone 'git@github.com:Org/bedrock.git' '/Users/x/my reviews/it'\''s/bedrock'` (whatever `shell_single_quote` gives for that path) and ends with the agent's `-- "$p"`; and it runs under `sh -n -c <line>` with exit 0.
  - `the_review_steps_check_out_the_pull_request` (real git): a bare origin with commit A on `main` and commit B at `refs/pull/451/head` (`git push origin <B>:refs/pull/451/head`); `run_git_steps(&git_steps(&t, origin_path))` → `dir` exists and `git -C dir rev-parse HEAD == B`. Then `remove_dir_all(dir)`, resolve again, run again → succeeds (prune) and `HEAD == B`. Then push commit C to the same ref, resolve again (`worktree_exists`), run → `HEAD == C`.
  - `the_review_prompt_fills_placeholders_or_appends_the_pr`: update its default-template expectation to include the new sentence.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p maya-core reviews::`
Expected: FAIL to compile (`ReviewTarget` has no `clone_dir`).

- [ ] **Step 3: Implement** the `Produces` items above.

- [ ] **Step 4: Run them to see them pass**

Run: `cargo test -p maya-core reviews::`
Expected: all pass. (`actions.rs` does not compile until Task 5; run the module only.)

- [ ] **Step 5: Commit** together with Task 5 (the crate must compile at each commit). Go straight to Task 5.

---

### Task 5: `start_review` uses the new target

**Files:**
- Modify: `core/src/actions.rs:860-908` (`start_review`) and its tests (`actions.rs:1514-1600`)

**Interfaces:**
- Consumes: `reviews::{resolve_target, git_steps, run_git_steps, shell_command, is_busy}` (Task 4).

**Behaviour:**
- No projects directory needed: drop that read and its error.
- `let target = resolve_target(&clones, &site, &pr.repo, pr.number)?;`
- `is_busy(&target.dir, &live)` → `Err("A review of <repo name> <sigil><number> is already running.")` (`pr.repo_name()`, `pr.forge.sigil()`), before anything opens.
- `create_dir_all(&clones)` always; the terminal opens in `clones`.
- OpenCode: `run_git_steps(&git_steps(&target, &url))?`, then `start_opencode` in `target.dir` as today. Remove the "need the repository checked out under the projects directory" error.
- Others: `shell_command` as today; the pending name keeps `target.dir`.

- [ ] **Step 1: Write the failing tests** (rewrite the review tests that assumed a projects checkout)
  - `start_review_runs_maya_agent_and_queues_the_review_name`: keep its assertions; the command now also contains `fetch origin '+pull/451/head:refs/maya/review/451'` and `worktree add --detach`.
  - `a_second_review_of_a_repository_clones_nothing`: `repo_with_origin`-style `clones/bedrock` with origin `git@github.com:Org/bedrock.git` → the command has no `git clone`, has `fetch origin`.
  - `a_review_never_runs_in_the_projects_directory`: a free project checkout `bedrock` (origin `Org/bedrock`) under projects → the returned folder is `clones/bedrock-451`.
  - `a_review_already_running_is_refused`: a running session (`write_session`, see `store_with_projects_and_running`) with cwd `clones/bedrock-451` → the exact error and `fake.calls` empty.
  - `an_opencode_review_runs_the_git_steps_itself`: brain OpenCode (`with_agents` including `info_for(Harness::OpenCode, None)`), `pr.ssh_url = Some(<missing path>)` → `Err` holding git's clone error, and no terminal opened.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p maya-core review`
Expected: FAIL.

- [ ] **Step 3: Implement** the behaviour above.

- [ ] **Step 4: Run them to see them pass**

Run: `cargo test --workspace`
Expected: all pass.

- [ ] **Step 5: Commit Tasks 4 and 5**

```bash
git add core/src/reviews.rs core/src/actions.rs
git commit -m "feat(reviews): review in worktrees of one clone per repository"
```

---

### Task 6: the "New worktree" checkbox

**Files:**
- Modify: `src/newsession.ts` (`NewSessionModel` at :84, render at :255-325, `capture` at :365, `loadAgents` at :444, `chooseMachine` at :465, `openNewSession` at :531), and the `StartChoices` type
- Modify: `src/styles.css` (one rule for the warning, from existing tokens)
- Test: `src/newsession.test.ts`

**Interfaces:**
- Consumes: `list_agents` reply `{ agents, names, worktrees }` (Task 3); `start_session`'s `options.worktree` (Task 2).
- Produces: `NewSessionModel.worktrees: boolean` (the machine takes the option) and `NewSessionModel.worktree: boolean` (ticked); `StartChoices.worktree: boolean`.

**Behaviour:** when `m.worktrees`, a `label.newsession__check` holding `input[type=checkbox][name=worktree]` and the text `New worktree` sits after the options row. While it is ticked, `p.newsession__hint.newsession__warning` shows the warning copy under it (toggled by the change listener, no repaint). `readChoices` sends `worktree: checkbox?.checked ?? false`. `capture` keeps the tick across repaints. A reply without `worktrees` (an older main) counts as `false`; the local machine before its reply counts as `true`; changing machine sets `worktrees = machine === ""` until `loadAgents` answers, and leaves `worktree` as it was.

- [ ] **Step 1: Write the failing tests**
  - `it("offers New worktree, off by default, and passes it to start")`: render with `worktrees: true`; the checkbox exists and is unchecked; Start → `onStart` choices have `worktree: false`; tick it, Start → `worktree: true`.
  - `it("warns that a worktree session cannot be resumed from the list while it is ticked")`: no `.newsession__warning` before ticking; after ticking, its text is the exact warning; after unticking, gone.
  - `it("hides New worktree for a machine that cannot start one")`: `worktrees: false` → no `input[name=worktree]`, and Start sends `worktree: false`.

- [ ] **Step 2: Run them to see them fail**

Run: `pnpm test -- newsession`
Expected: 3 FAIL.

- [ ] **Step 3: Implement** the behaviour above.

- [ ] **Step 4: Run them to see them pass**

Run: `pnpm test`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/newsession.ts src/newsession.test.ts src/styles.css
git commit -m "feat(newsession): a New worktree option with its resume warning"
```

---

### Task 7: copy, docs and the release

**Files:**
- Modify: `src/settings.ts:825` (label), `core/src/config.rs:124` (doc comment: `/// Where review clones and worktrees go: reviews' and new sessions'.`)
- Modify: `README.md`: the "What it does" list (`README.md:13-38`), and "First run" step 2 (`README.md:107-108`)
- Version files via the script

- [ ] **Step 1: Update the label and comment** to the exact copy in Global Constraints; update any settings test that asserts the old label (`grep -n "for PR reviews" src/*.test.ts`).

- [ ] **Step 2: README.** Add to "What it does", after "Starts new sessions and resumes old ones": `- Starts a new session in its own git worktree when you tick New worktree, on a new branch off the branch your checkout is on, so sessions in one repository do not step on each other`. Change the review line's end to say reviews run in a worktree of one clone per repository. In "First run" step 2: `a **clones directory** for pull-request reviews and worktrees`.

- [ ] **Step 3: Run everything**

Run: `cargo test --workspace && pnpm test`
Expected: all pass.

- [ ] **Step 4: Commit the docs, then the version on its own**

```bash
git add src/settings.ts src/*.test.ts core/src/config.rs README.md
git commit -m "docs: describe worktrees and the clones directory"
sh scripts/set-version.sh 0.40.0
git add -A src-tauri/tauri.conf.json package.json Cargo.toml Cargo.lock
git commit -m "chore(release): 0.40.0"
```

Before the bump, check `git show origin/main:package.json | grep version`: if `main` has moved past 0.39.2, bump the minor of that version instead.
