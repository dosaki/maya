# Deployments Tab Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Deployments tab that lists GitHub Actions runs waiting for the user's approval (with Approve and Reject) and the deployment runs queued behind them ("Waiting on …").

**Architecture:** A new `core/src/deployments.rs` holds the model, the pure rules (parsing, blocker matching, ordering) and the per-check orchestration. The GitHub calls live in `core/src/forge/github.rs` on the existing `Http` layer, with a per-use 403 message. `src-tauri/src/lib.rs` polls every 120 s on its own thread (main or standalone only), keeps a `DeploymentState`, and exposes commands. A new `src/deployments.ts` paints the pane.

**Tech Stack:** Rust (Tauri 2, `ureq`, `serde_json`), vanilla TypeScript, vitest.

**Spec:** `docs/superpowers/specs/2026-10-28-deployments-tab-design.md`

## Global Constraints

- GitHub only. A GitLab connection is ignored by everything here.
- Poll every **120 s**, always. No faster check.
- Active repos: pushed within **7 days** (`7 * 86_400_000` ms), at most **5 pages** of 100.
- The "needs approval" answer per repo is kept **1 hour** (`3_600_000` ms), in memory only.
- Runs calls: `status=waiting&per_page=50`, `status=pending&per_page=20`, `status=in_progress&per_page=50`, `status=queued&per_page=50`. No date filter on runs.
- Review comment, verbatim: `"Approved in Maya."` / `"Rejected in Maya."`.
- Copy, verbatim: `"No deployments are waiting on you."`, `"Deployments are checked on the main."`, `"That deployment is no longer waiting."`, `"Waiting on another deployment"`.
- Every error passes `forge::scrub`; no token in errors, logs or events.
- Only a main or a standalone Maya (`NetworkRole::Main | Off`) polls.
- Version: this is a `feat`; bump `0.35.1` → `0.36.0` (re-check `main` first).

## Review Focus

- A run waiting on two environments, only one of which the user can approve: Approve sends only that environment's id (Task 3 test `approve_sends_only_the_given_environments`, Task 2 test `approvable_keeps_only_what_the_user_can_approve`).
- A run whose `head_commit` or `triggering_actor` is null (some `workflow_dispatch` and re-run events): the row still parses, with an empty title or the `actor` login (Task 2 test `a_run_without_head_commit_or_triggering_actor_still_parses`).
- A run approved or rejected on GitHub between the poll and the click: GitHub answers with an error; the row shows it and the repo is re-checked so the row goes (Task 5 step: re-check on any outcome).
- A repo listing whose `pushed_at` is null (an empty repo) or a page shorter than 100: the listing stops there (Task 3 test `active_repos_stops_at_a_null_pushed_at_and_a_short_page`).
- A fine-grained token without Actions or Environments access: every repo refuses, and the tab shows the permissions it needs, not "No deployments are waiting on you." (Task 4 test `every_repo_refusing_is_an_error_naming_the_permissions`).

---

### Task 1: Per-use token message in the forge layer

**Files:**
- Modify: `core/src/forge/mod.rs` (`scopes`, `check`, `answer`, `get_json`, `post_graphql`; around lines 248–290)

**Interfaces:**
- Produces:
  - `pub(crate) enum Need { Reviews, Deployments }` (`Clone, Copy`)
  - `pub(crate) fn get_json_as(http: &dyn Http, forge: &Forge, url: &str, need: Need) -> Result<serde_json::Value, String>`
  - `pub(crate) fn post_json_as(http: &dyn Http, forge: &Forge, url: &str, body: &serde_json::Value, need: Need) -> Result<String, String>`
  - `pub fn is_refusal(e: &str) -> bool` (true for the "the token was refused" message)
  - `get_json` and `post_graphql` keep their signatures and use `Need::Reviews`.

- [ ] **Step 1: Write the failing test** in `forge/mod.rs` tests:

```rust
#[test]
fn a_refusal_names_the_permissions_of_its_use() {
    let f = Forge { site: ForgeSite::of(&GitConnection::default_for(GitProvider::Github)), token: "t".into() };
    let http = fake::FakeHttp::new(vec![("/x", 403, "{}")]);
    let reviews = get_json(&http, &f, "https://api.github.com/x").unwrap_err();
    let deploys = get_json_as(&http, &f, "https://api.github.com/x", Need::Deployments).unwrap_err();
    assert_eq!(reviews, "the token was refused (403). It needs the repo scope (fine-grained: Pull requests read).");
    assert_eq!(deploys, "the token was refused (403). It needs the repo scope (fine-grained: Actions read, Deployments read and write, Environments read, Metadata read).");
    assert!(is_refusal(&deploys) && !is_refusal("404: Not Found"));
    let http = fake::FakeHttp::new(vec![("/y", 200, "ok")]);
    assert_eq!(post_json_as(&http, &f, "https://api.github.com/y", &serde_json::json!({"a": 1}), Need::Deployments).unwrap(), "ok");
    assert_eq!(http.seen.lock().unwrap()[0].2.as_deref(), Some(r#"{"a":1}"#));
}
```

- [ ] **Step 2: Run it and see it fail**

Run: `cargo test -p maya-core forge::tests::a_refusal_names_the_permissions_of_its_use`
Expected: FAIL to compile (`Need`, `get_json_as` not found).

- [ ] **Step 3: Implement.** `scopes(p, need)`: GitHub + `Deployments` gives `"repo scope (fine-grained: Actions read, Deployments read and write, Environments read, Metadata read)"`; GitHub + `Reviews` and GitLab keep today's text. Thread `need` through `check` and `answer`. Before you settle the permission names, check them against GitHub's docs for the endpoints in Task 3 (context7 or the REST docs). If GitHub names them differently, use GitHub's names in the message and the test.

- [ ] **Step 4: Run the forge tests**

Run: `cargo test -p maya-core forge::`
Expected: PASS, including the existing `a_refused_token_names_the_scope_and_other_errors_the_message`.

- [ ] **Step 5: Commit**

```bash
git add core/src/forge/mod.rs
git commit -m "feat(forge): name the token permissions each use needs"
```

---

### Task 2: The deployments model and its rules

**Files:**
- Create: `core/src/deployments.rs`
- Modify: `core/src/lib.rs` (add `pub mod deployments;`, alphabetical after `context`)

**Interfaces:**
- Produces (all in `maya_core::deployments`, all `Serialize` ones `#[serde(rename_all = "camelCase")]`):
  - `pub struct Environment { pub id: u64, pub name: String }`
  - `pub struct Blocker { pub run_id: u64, pub sha: String, pub title: String, pub url: String }`
  - `#[serde(tag = "kind", rename_all = "camelCase")] pub enum RunWait { Approval { environments: Vec<Environment> }, Blocked { by: Option<Blocker> } }`
  - `pub struct DeploymentRun { pub repo: String, pub run_id: u64, pub run_number: u64, pub workflow: String, pub branch: String, pub sha: String, pub title: String, pub actor: String, pub url: String, pub since: String, pub wait: RunWait }`
  - `pub struct Run { pub id: u64, pub number: u64, pub workflow_id: u64, pub name: String, pub branch: String, pub sha: String, pub title: String, pub actor: String, pub url: String, pub created_at: String, pub status: String }` (not serialized)
  - `pub fn parse_runs(v: &serde_json::Value) -> Vec<Run>` (reads `workflow_runs`)
  - `pub fn needs_approval(v: &serde_json::Value) -> bool` (reads `environments[].protection_rules[].type == "required_reviewers"`)
  - `pub fn approvable(v: &serde_json::Value) -> Vec<Environment>` (pending_deployments array; `current_user_can_approve == true`; `environment.id`, `environment.name`)
  - `pub fn find_blocker(pending: &Run, others: &[Run]) -> Option<Blocker>`
  - `pub fn row(repo: &str, run: &Run, wait: RunWait) -> DeploymentRun`
  - `pub fn order(runs: Vec<DeploymentRun>) -> Vec<DeploymentRun>`
  - `pub fn replace_repo(runs: &[DeploymentRun], repo: &str, fresh: Vec<DeploymentRun>) -> Vec<DeploymentRun>` (drops `repo`'s rows, adds `fresh`, returns `order(..)`)
  - `pub fn approval_count(runs: &[DeploymentRun]) -> usize`
  - `pub const ACTIVE_WINDOW_MS: u64 = 7 * 86_400_000; pub const ANSWER_TTL_MS: u64 = 3_600_000; pub const REPO_PAGES: u32 = 5;`

- [ ] **Step 1: Write the failing tests** (module `tests` in `deployments.rs`; build JSON with `serde_json::json!`, and a `fn run(id, number, workflow_id, status, created_at) -> Run` helper):

```rust
#[test]
fn parse_runs_reads_the_fields_a_row_needs() {
    let v = json!({"workflow_runs": [{"id": 7, "run_number": 41, "workflow_id": 3, "name": "Deploy", "head_branch": "main",
        "head_sha": "a1b2c3d4e5", "head_commit": {"message": "fix login redirect\n\nbody"}, "triggering_actor": {"login": "tiago"},
        "actor": {"login": "bot"}, "html_url": "https://github.com/O/r/actions/runs/7", "created_at": "2026-10-28T08:00:00Z", "status": "waiting"}]});
    let r = &parse_runs(&v)[0];
    assert_eq!((r.id, r.number, r.workflow_id, r.title.as_str(), r.actor.as_str(), r.status.as_str()), (7, 41, 3, "fix login redirect", "tiago", "waiting"));
}

#[test]
fn a_run_without_head_commit_or_triggering_actor_still_parses() {
    let v = json!({"workflow_runs": [{"id": 7, "run_number": 1, "workflow_id": 3, "name": "Deploy", "head_branch": "main", "head_sha": "s",
        "head_commit": null, "triggering_actor": null, "actor": {"login": "bot"}, "html_url": "u", "created_at": "c", "status": "pending"}]});
    let r = &parse_runs(&v)[0];
    assert_eq!((r.title.as_str(), r.actor.as_str()), ("", "bot"));
}

#[test]
fn needs_approval_only_for_required_reviewers() {
    assert!(needs_approval(&json!({"environments": [{"protection_rules": [{"type": "branch_policy"}]}, {"protection_rules": [{"type": "required_reviewers"}]}]})));
    assert!(!needs_approval(&json!({"environments": [{"protection_rules": [{"type": "branch_policy"}]}]})));
    assert!(!needs_approval(&json!({"total_count": 0, "environments": []})));
}

#[test]
fn approvable_keeps_only_what_the_user_can_approve() {
    let v = json!([{"environment": {"id": 1, "name": "production"}, "current_user_can_approve": true},
                   {"environment": {"id": 2, "name": "eu"}, "current_user_can_approve": false}]);
    assert_eq!(approvable(&v), vec![Environment { id: 1, name: "production".into() }]);
}

#[test]
fn the_blocker_is_the_newest_earlier_live_run_of_the_same_workflow() {
    let pending = run(9, 12, 3, "pending", "t");
    let others = vec![run(5, 10, 3, "waiting", "t"), run(6, 11, 3, "in_progress", "t"), run(8, 11, 4, "waiting", "t"),
                      run(4, 9, 3, "completed", "t"), run(10, 13, 3, "queued", "t")];
    assert_eq!(find_blocker(&pending, &others).map(|b| b.run_id), Some(6));
    assert_eq!(find_blocker(&pending, &[run(8, 11, 4, "waiting", "t")]), None);
}

#[test]
fn order_groups_by_repo_oldest_first_with_blocked_rows_under_their_blocker() {
    // Repo B's oldest row is older than A's, so B comes first; in A the blocked row follows its blocker.
    let a1 = approval("O/a", 1, "2026-10-28T09:00:00Z");
    let a2 = approval("O/a", 2, "2026-10-28T08:30:00Z");
    let a3 = blocked("O/a", 3, "2026-10-28T08:00:00Z", Some(1));
    let b1 = approval("O/b", 4, "2026-10-28T07:00:00Z");
    let ids: Vec<u64> = order(vec![a1, a2, a3, b1]).iter().map(|r| r.run_id).collect();
    assert_eq!(ids, vec![4, 2, 1, 3]);
}

#[test]
fn replace_repo_swaps_one_repos_rows_and_the_count_counts_approvals() {
    let runs = order(vec![approval("O/a", 1, "t1"), blocked("O/a", 2, "t2", Some(1)), approval("O/b", 3, "t0")]);
    assert_eq!(approval_count(&runs), 2);
    let after = replace_repo(&runs, "O/a", vec![]);
    assert_eq!(after.iter().map(|r| r.run_id).collect::<Vec<_>>(), vec![3]);
}
```

(`approval(repo, id, since)` and `blocked(repo, id, since, by_run_id)` are test helpers that build a `DeploymentRun`.)

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p maya-core deployments::`
Expected: FAIL to compile.

- [ ] **Step 3: Implement.** Notes:
  - `parse_runs`: `title` = first line of `head_commit.message`, else `""`. `actor` = `triggering_actor.login`, else `actor.login`, else `""`. Skip entries without `id`.
  - `find_blocker`: among `others` with the same `workflow_id`, a lower `number`, and status `waiting`, `in_progress` or `queued`, take the highest `number`.
  - `order`: sort rows by `since` ascending (ISO strings sort lexically). Then, inside each repo, move each blocked row whose `by.run_id` matches another row of that repo to just after that row. Order the repos by their oldest `since`, keeping each repo's rows together.

- [ ] **Step 4: Run them and see them pass**

Run: `cargo test -p maya-core deployments::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/deployments.rs core/src/lib.rs
git commit -m "feat(deployments): the model, blocker matching and ordering"
```

---

### Task 3: The GitHub calls

**Files:**
- Modify: `core/src/forge/github.rs`

**Interfaces:**
- Consumes: `Need`, `get_json_as`, `post_json_as` (Task 1); `Run`, `Environment`, `parse_runs`, `needs_approval`, `approvable`, `ACTIVE_WINDOW_MS`, `REPO_PAGES` (Task 2); `crate::codex::ms_of` for ISO → ms.
- Produces (all use `Need::Deployments`):
  - `pub fn active_repos(http: &dyn Http, forge: &Forge, now_ms: u64) -> Result<Vec<String>, String>`
  - `pub fn repo_needs_approval(http: &dyn Http, forge: &Forge, repo: &str) -> Result<bool, String>` (a `404` answer is `Ok(false)`)
  - `pub fn runs(http: &dyn Http, forge: &Forge, repo: &str, status: &str, per_page: u32) -> Result<Vec<Run>, String>`
  - `pub fn pending_deployments(http: &dyn Http, forge: &Forge, repo: &str, run_id: u64) -> Result<Vec<Environment>, String>`
  - `pub fn review_deployment(http: &dyn Http, forge: &Forge, repo: &str, run_id: u64, environment_ids: &[u64], approve: bool) -> Result<(), String>`

URLs (base `forge.site.api_base()`):
- `/user/repos?sort=pushed&direction=desc&per_page=100&affiliation=owner,collaborator,organization_member&page={n}`
- `/repos/{repo}/environments?per_page=100`
- `/repos/{repo}/actions/runs?status={status}&per_page={per_page}`
- `/repos/{repo}/actions/runs/{run_id}/pending_deployments` (GET, and POST with `{"environment_ids": [...], "state": "approved"|"rejected", "comment": "Approved in Maya."|"Rejected in Maya."}`)

- [ ] **Step 1: Write the failing tests** in `github.rs` tests (`now` = `ms_of("2026-10-28T12:00:00Z")`):

```rust
#[test]
fn active_repos_stops_at_the_first_repo_older_than_a_week() {
    let page = r#"[{"full_name":"O/new","pushed_at":"2026-10-27T00:00:00Z"},{"full_name":"O/old","pushed_at":"2026-10-20T00:00:00Z"},{"full_name":"O/x","pushed_at":"2026-10-28T00:00:00Z"}]"#;
    let http = FakeHttp::new(vec![("/user/repos", 200, page)]);
    assert_eq!(active_repos(&http, &forge(), now()).unwrap(), vec!["O/new"]);
    assert_eq!(http.seen.lock().unwrap().len(), 1);
}

#[test]
fn active_repos_stops_at_a_null_pushed_at_and_a_short_page() {
    let http = FakeHttp::new(vec![("/user/repos", 200, r#"[{"full_name":"O/a","pushed_at":"2026-10-28T00:00:00Z"},{"full_name":"O/e","pushed_at":null}]"#)]);
    assert_eq!(active_repos(&http, &forge(), now()).unwrap(), vec!["O/a"]);
    let http = FakeHttp::new(vec![("/user/repos", 200, r#"[{"full_name":"O/a","pushed_at":"2026-10-28T00:00:00Z"}]"#)]);
    assert_eq!(active_repos(&http, &forge(), now()).unwrap(), vec!["O/a"]);
    assert_eq!(http.seen.lock().unwrap().len(), 1, "a page under 100 is the last");
}

#[test]
fn active_repos_reads_at_most_five_pages() {
    let full: String = format!("[{}]", (0..100).map(|i| format!(r#"{{"full_name":"O/r{i}","pushed_at":"2026-10-28T00:00:00Z"}}"#)).collect::<Vec<_>>().join(","));
    let http = FakeHttp { replies: vec![("/user/repos", 200, full)], seen: Default::default() };
    assert_eq!(active_repos(&http, &forge(), now()).unwrap().len(), 500);
    let seen = http.seen.lock().unwrap();
    assert_eq!(seen.len(), 5);
    assert!(seen[4].0.ends_with("&page=5"));
}

#[test]
fn a_repo_without_environments_access_needs_no_approval() {
    let http = FakeHttp::new(vec![("/environments", 404, r#"{"message":"Not Found"}"#)]);
    assert_eq!(repo_needs_approval(&http, &forge(), "O/r"), Ok(false));
    let http = FakeHttp::new(vec![("/environments", 403, "{}")]);
    assert!(crate::forge::is_refusal(&repo_needs_approval(&http, &forge(), "O/r").unwrap_err()));
}

#[test]
fn approve_sends_only_the_given_environments() {
    let http = FakeHttp::new(vec![("/pending_deployments", 200, "[]")]);
    review_deployment(&http, &forge(), "O/r", 7, &[1], true).unwrap();
    review_deployment(&http, &forge(), "O/r", 7, &[1, 2], false).unwrap();
    let seen = http.seen.lock().unwrap();
    assert_eq!(seen[0].0, "https://api.github.com/repos/O/r/actions/runs/7/pending_deployments");
    let body: serde_json::Value = serde_json::from_str(seen[0].2.as_deref().unwrap()).unwrap();
    assert_eq!(body, serde_json::json!({"environment_ids": [1], "state": "approved", "comment": "Approved in Maya."}));
    let body: serde_json::Value = serde_json::from_str(seen[1].2.as_deref().unwrap()).unwrap();
    assert_eq!(body, serde_json::json!({"environment_ids": [1, 2], "state": "rejected", "comment": "Rejected in Maya."}));
}

#[test]
fn runs_and_pending_deployments_ask_the_right_urls() {
    let http = FakeHttp::new(vec![("/pending_deployments", 200, r#"[{"environment":{"id":1,"name":"production"},"current_user_can_approve":true}]"#),
                                  ("/actions/runs?", 200, r#"{"workflow_runs":[]}"#)]);
    assert_eq!(runs(&http, &forge(), "O/r", "waiting", 50).unwrap(), vec![]);
    assert_eq!(pending_deployments(&http, &forge(), "O/r", 7).unwrap().len(), 1);
    let seen = http.seen.lock().unwrap();
    assert_eq!(seen[0].0, "https://api.github.com/repos/O/r/actions/runs?status=waiting&per_page=50");
}
```

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p maya-core forge::github::`
Expected: FAIL to compile.

- [ ] **Step 3: Implement** the five functions. `repo_needs_approval` maps an error starting with `"404"` to `Ok(false)`. `active_repos` stops at a missing or null `pushed_at`, at the first one older than `now_ms - ACTIVE_WINDOW_MS`, at a page shorter than 100, or after `REPO_PAGES` pages.

- [ ] **Step 4: Run them and see them pass**

Run: `cargo test -p maya-core forge::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/forge/github.rs
git commit -m "feat(forge): GitHub calls for waiting runs and their approvals"
```

---

### Task 4: One check, with the hourly cache and the refusal rule

**Files:**
- Modify: `core/src/deployments.rs`

**Interfaces:**
- Consumes: Task 3's functions; `crate::forge::{Forge, Http, is_refusal}`; `crate::log::line`.
- Produces:
  - `#[derive(Default, Clone)] pub struct ApprovalRepos` with `pub fn fresh(&self, repo: &str, now_ms: u64) -> Option<bool>` (an answer under `ANSWER_TTL_MS` old) and `pub fn set(&mut self, repo: &str, needs: bool, now_ms: u64)`
  - `pub fn check_repo(http: &dyn Http, forge: &Forge, repo: &str) -> Result<Vec<DeploymentRun>, String>`
  - `pub fn fetch(http: &dyn Http, forge: &Forge, cache: &mut ApprovalRepos, now_ms: u64) -> Result<Vec<DeploymentRun>, String>` (ordered)

`check_repo`:
1. `waiting = runs(repo, "waiting", 50)`. For each, `pending_deployments`. When it is not empty, add an `Approval` row.
2. `pending = runs(repo, "pending", 20)`. If it's empty, stop.
3. For each pending run, call `find_blocker` against `waiting`. Only when one has no blocker, fetch `in_progress` (once per check) and try again. If it still has none, fetch `queued` (once) and try again. Add a `Blocked` row.

`fetch`:
1. `active_repos`. An error here is the check's error.
2. For each repo, use `cache.fresh` or else `repo_needs_approval`. An `Ok` is cached. An error is logged and the repo skipped.
3. For each repo that needs approval, call `check_repo`. An error is logged and the repo skipped.
4. "Attempted" counts the repos where this check made at least one call after the repo list. "Refused" counts those whose failure `is_refusal`. When `attempted > 0 && refused == attempted`, return `Err` with the first refusal's message.
5. Do the per-repo work (steps 2 and 3) on up to 8 threads with `std::thread::scope`, then update the cache from the results.

Log lines go through `crate::log::line("app", format!("deployments: {repo}: {e}"))`.

- [ ] **Step 1: Write the failing tests** (FakeHttp needles are matched in order, so list more specific ones first):

```rust
#[test]
fn a_waiting_run_the_user_can_approve_is_a_row_and_one_waiting_on_others_is_not() { /* waiting runs 7 and 8; 7's pending_deployments can approve, 8's cannot → rows == [7] with environments ["production"] */ }

#[test]
fn a_pending_run_finds_its_blocker_and_only_asks_for_in_progress_when_needed() { /* pending 9 (workflow 3), waiting 7 (workflow 3) → Blocked{by: 7}; seen URLs contain no "status=in_progress" */ }

#[test]
fn a_pending_run_with_no_live_run_in_its_workflow_waits_on_another_deployment() { /* pending 9 wf 3, in_progress/queued empty → Blocked{by: None}; both extra URLs were asked once */ }

#[test]
fn repos_without_required_reviewers_are_not_checked_and_the_answer_lasts_an_hour() {
    // fetch at t: environments asked, no runs asked; fetch at t+59min: environments not asked again; at t+61min: asked again.
}

#[test]
fn a_failing_repo_is_skipped_and_the_others_show() { /* O/a environments 500, O/b normal with one approval row → Ok([O/b row]) */ }

#[test]
fn every_repo_refusing_is_an_error_naming_the_permissions() {
    // Both repos' environments answer 403 → Err(e) with is_refusal(&e) and e.contains("Actions read").
}
```

Write each body out with the same `FakeHttp` routes as Task 3. The comments above say what each one asserts.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p maya-core deployments::`
Expected: FAIL to compile.

- [ ] **Step 3: Implement** `ApprovalRepos`, `check_repo` and `fetch` as described above.

- [ ] **Step 4: Run all core tests**

Run: `cargo test -p maya-core`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/deployments.rs
git commit -m "feat(deployments): one check across active repos, cached hourly"
```

---

### Task 5: Poll, state and commands in the app

**Files:**
- Modify: `src-tauri/src/lib.rs`: the `use maya_core::{…}` line (add `deployments`), `AppState` (around 136), a new `DeploymentState` next to `ReviewState` (around 254), `poll_deployments` after `poll_reviews` (around 296), the commands after `list_review_prs` (around 416), the `invoke_handler` list (around 1423), the `.manage(AppState { … })` initialiser (around 1397), and the thread next to the reviews thread (around 1519)
- Modify: `src-tauri/src/git.rs`: `settings_changed` also runs `crate::poll_deployments(&handle)`

**Interfaces:**
- Consumes: `deployments::{fetch, check_repo, replace_repo, ApprovalRepos, DeploymentRun, RunWait}`, `forge::github::review_deployment`, `git::forges`, `git::http()`, `open_in_browser`, `now_ms`.
- Produces:
  - `#[derive(Default, Clone, serde::Serialize)] #[serde(rename_all = "camelCase")] pub struct DeploymentState { pub runs: Vec<deployments::DeploymentRun>, pub error: Option<String>, pub off: bool, pub assistant: bool, pub fetched_at: Option<u64> }`
  - `AppState` fields: `pub deployments: Mutex<DeploymentState>`, `pub deployment_repos: Mutex<deployments::ApprovalRepos>`
  - `pub(crate) fn poll_deployments(app: &AppHandle)`, which emits `"deployments"` with a `DeploymentState`
  - Commands: `list_deployments() -> DeploymentState`; `deployment_approve(repo: String, run_id: u64) -> Result<(), String>`; `deployment_reject(repo: String, run_id: u64) -> Result<(), String>`; `open_deployment(repo: String, run_id: u64, blocker: bool) -> Result<(), String>`

- [ ] **Step 1: Implement `poll_deployments`**, following `poll_reviews`:
  1. Read the role from `store.config.network.role`. On `Assistant`, store `DeploymentState { assistant: true, fetched_at: Some(now), ..Default }`, emit it, and return.
  2. Find the GitHub connection in `config.git_connections()`. If it's disabled, store `off: true`, emit, and return.
  3. Call `git::forges`. If GitHub is missing (no token), set `error` to that `ForgeError`'s message, keep the runs, emit, and return.
  4. Clone the cache, then `deployments::fetch(git::http(), &forge, &mut cache, now_ms())` with no lock held.
  5. If `git_generation` changed, log `"deployments: the Git settings changed during the check; its answers are dropped"` and return.
  6. Store the cache back. `Ok` replaces `runs` and clears `error`. `Err(e)` keeps `runs`, sets `error` to `"GitHub check failed: {e}"`, and logs it. Set `fetched_at`, then emit.

- [ ] **Step 2: Implement the commands.**
  - `deployment_approve` and `deployment_reject` both call one `fn review_deployment_cmd(app: &AppHandle, repo: &str, run_id: u64, approve: bool) -> Result<(), String>`:
    1. Find the row in `DeploymentState` with an `Approval` wait. If there's none, return `Err("That deployment is no longer waiting.")`.
    2. Resolve the GitHub forge, then call `forge::github::review_deployment` with that row's environment ids.
    3. **Whatever GitHub answered,** run `deployments::check_repo` for that repo. On `Ok`, call `replace_repo` into the state and emit `"deployments"`.
    4. Return GitHub's result.
  - Make both commands `#[tauri::command(async)]`.
  - `open_deployment` opens the row's `url`, or its `Blocked { by: Some(b) }` `b.url` when `blocker` is true. If neither exists, return `Err("That deployment is no longer waiting.")`.
  - Register all four commands in `invoke_handler`.

- [ ] **Step 3: Add the thread** after the reviews thread:

```rust
let deployments_handle = app.handle().clone();
std::thread::spawn(move || loop {
    poll_deployments(&deployments_handle);
    std::thread::sleep(Duration::from_secs(120));
});
```

Also add `crate::poll_deployments(&handle);` to the thread in `git::settings_changed`.

- [ ] **Step 4: Build and test**

Run: `cargo build -p maya && cargo test --workspace`
Expected: builds; all tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs src-tauri/src/git.rs
git commit -m "feat: poll waiting deployments and approve or reject them"
```

---

### Task 6: The Deployments tab

**Files:**
- Modify: `src/types.ts`: add the types below after `ReviewState`
- Modify: `src/tabs.ts`: add `"deployments"` to `Tab`, `PANES` (`deployments: "deployments"`) and `asTab`, plus `setDeploymentCount(n: number)` (writes `#deployments-count`, empty at 0)
- Modify: `src/tabs.test.ts`: add the button and pane to `dom()`
- Modify: `index.html`: after the Pull Requests button, `<button class="tab" type="button" role="tab" data-tab="deployments">Deployments <span class="tab__count" id="deployments-count"></span></button>`; after `#reviews`, `<div id="deployments" hidden></div>`
- Create: `src/deployments.ts`, `src/deployments.test.ts`
- Modify: `src/main.ts`: state, listener, first fetch, clicks, the 10 s repaint
- Modify: `src/styles.css`: `#deployments` like `#reviews`, a `.deploy--blocked` indent, and `.deploy__error`, which reuses `.reviews__error`'s colours (including the light-theme rule at line 480)

**Interfaces:**
- Consumes: the `deployments` event and the four commands from Task 5.
- Produces (TS):

```ts
export interface DeployEnvironment { id: number; name: string }
export interface DeployBlocker { runId: number; sha: string; title: string; url: string }
export type RunWait = { kind: "approval"; environments: DeployEnvironment[] } | { kind: "blocked"; by: DeployBlocker | null };
export interface DeploymentRun { repo: string; runId: number; runNumber: number; workflow: string; branch: string; sha: string; title: string; actor: string; url: string; since: string; wait: RunWait }
export interface DeploymentState { runs: DeploymentRun[]; error: string | null; off: boolean; assistant: boolean; fetchedAt: number | null }
```

  - `src/deployments.ts`:
    - `export interface RowUi { busy: Set<number>; errors: Map<number, string> }`
    - `export function renderDeployments(state: DeploymentState, nowMs: number, ui: RowUi): HTMLElement`
    - `export type DeploymentAction = { kind: "approve" | "reject" | "open-run" | "open-blocker"; repo: string; runId: number } | { kind: "git-settings" }`
    - `export function deploymentActionFor(target: Element): DeploymentAction | null`
    - `export function approvalCount(runs: DeploymentRun[]): number`

Row markup (reuse the `.pr` card classes):
- `article.pr.deploy` with `data-repo` and `data-run-id`.
- Head: the repo's short name, then a `button[data-action=open-run]` with the workflow name, then the branch. Below it, the title.
- Approval rows: one `.pr__chip` per environment name. Meta line: `"{actor} · waiting {formatAge} · {repo}"`.
- Blocked rows: the extra class `deploy--blocked`, no chips, and a `button[data-action=open-blocker]` reading `` `Waiting on ${by.sha.slice(0, 7)} ${by.title}` ``. When `by` is null, a plain span reading `"Waiting on another deployment"`.
- Actions on approval rows: `button.card__btn[data-action=reject]` "Reject" and `button.card__btn.card__btn--primary[data-action=approve]` "Approve". Both are `disabled` while `ui.busy.has(runId)`. `ui.errors.get(runId)` shows in a `.deploy__error` line.
- Pane states, in this order:
  1. `assistant` gives the line "Deployments are checked on the main."
  2. `off` gives "GitHub is off. Turn it on in Settings → Git. " plus an "Open Settings" `button[data-action=git-settings]`.
  3. `error` shows above the list in a `.reviews__error` line.
  4. With no runs: "Checking…" while `fetchedAt === null`, otherwise "No deployments are waiting on you."

- [ ] **Step 1: Write the failing tests** in `src/deployments.test.ts`:

```ts
it("paints an approval row with its environments and both buttons", () => {
  const root = renderDeployments(state([approval(7, ["production"])]), NOW, ui());
  expect(root.querySelector(".pr__chip")?.textContent).toBe("production");
  expect([...root.querySelectorAll("button[data-action=reject], button[data-action=approve]")].map((b) => b.textContent)).toEqual(["Reject", "Approve"]);
});
it("paints a blocked row with what it waits on and no buttons", () => {
  const root = renderDeployments(state([approval(7, ["production"]), blocked(9, { runId: 7, sha: "a1b2c3d4e5", title: "fix login redirect", url: "u" })]), NOW, ui());
  const row = root.querySelector<HTMLElement>("[data-run-id='9']")!;
  expect(row.classList.contains("deploy--blocked")).toBe(true);
  expect(row.querySelector("[data-action=open-blocker]")?.textContent).toBe("Waiting on a1b2c3d fix login redirect");
  expect(row.querySelector("[data-action=approve]")).toBeNull();
});
it("says waiting on another deployment when the blocker is unknown", () => { /* blocked(9, null) → text "Waiting on another deployment" */ });
it("disables a busy row's buttons and shows its error", () => { /* ui busy {7}, errors {7: "403: nope"} → both disabled, ".deploy__error" text "403: nope" */ });
it("shows the empty, checking, off and assistant states", () => { /* the four copy strings from Global Constraints */ });
it("reads a click into an action", () => { /* click on approve inside row 7 of O/r → {kind:"approve", repo:"O/r", runId:7} */ });
it("counts only approval rows", () => { expect(approvalCount([approval(7, ["p"]), blocked(9, null)])).toBe(1); });
```

In `src/tabs.test.ts`, add a test that clicking `[data-tab=deployments]` shows `#deployments` and that `localStorage` `"deployments"` is restored.

- [ ] **Step 2: Run them and see them fail**

Run: `pnpm test -- deployments tabs`
Expected: FAIL (module not found).

- [ ] **Step 3: Implement** the types, `tabs.ts`, `index.html`, `deployments.ts` and the CSS. In `main.ts`:
  - Keep `let deployments: DeploymentState` (initially `{ runs: [], error: null, off: false, assistant: false, fetchedAt: null }`) and a `RowUi`.
  - `paintDeployments()` replaces `#deployments`' children with `renderDeployments(...)` and calls `tabs?.setDeploymentCount(approvalCount(deployments.runs))`.
  - Wire it up like reviews: `listen("deployments")`, `invoke("list_deployments")`, and add `paintDeployments()` to the 10 s `setInterval`.
  - On a click in `#deployments`:
    - `approve` / `reject`: add to `busy`, delete its error, repaint, then `invoke("deployment_approve" | "deployment_reject", { repo, runId })`. On a throw, set `errors[runId] = String(e)`. Finally remove it from `busy` and repaint.
    - `open-run` / `open-blocker`: `invoke("open_deployment", { repo, runId, blocker })`, with a toast on error.
    - `git-settings`: the same as in `reviewAction`.

- [ ] **Step 4: Run the frontend tests and the type check**

Run: `pnpm test && pnpm exec tsc --noEmit`
Expected: PASS, no type errors.

- [ ] **Step 5: See it work.** Run `pnpm tauri dev`, open Deployments, and check that it fills within 2 minutes (or shows "No deployments are waiting on you."). If a real run waits on you, approve it and confirm the row goes. Note in the commit or PR what you could and could not try for real.

- [ ] **Step 6: Commit**

```bash
git add index.html src/types.ts src/tabs.ts src/tabs.test.ts src/deployments.ts src/deployments.test.ts src/main.ts src/styles.css
git commit -m "feat: a Deployments tab to approve or reject waiting runs"
```

---

### Task 7: Docs and the release bump

**Files:**
- Modify: `docs/DEVELOPING.md` (the "Git integration" section)
- Modify: `README.md` (next to the line 46 "Pull requests:" bullet and the line 538 bullet)
- Modify (by script): `src-tauri/tauri.conf.json`, `package.json`, `Cargo.toml`, `Cargo.lock`

- [ ] **Step 1: Document it.** In `DEVELOPING.md`'s Git integration section, add a paragraph covering:
  - the tab and the 2-minute check;
  - the 7-day repo window;
  - the required-reviewers filter with its 1-hour answer;
  - that only the main or a standalone Maya checks;
  - the token permissions (classic `repo`; fine-grained Actions read, Deployments read and write, Environments read, Metadata read).

  In `README.md`, add one bullet in each of the two places, in the same voice as their neighbours. Also say that there is no Deployments tab on the phone, next to line 233.

- [ ] **Step 2: Commit the docs**

```bash
git add docs/DEVELOPING.md README.md
git commit -m "docs: describe the Deployments tab and its token permissions"
```

- [ ] **Step 3: Bump the version against `main`.** Run `git fetch origin && git show origin/main:package.json | grep '"version"'`. If `main` is still at `0.35.1`, run `sh scripts/set-version.sh 0.36.0`. Otherwise bump the minor of `main`'s version.

- [ ] **Step 4: Verify and commit on its own**

Run: `cargo test --workspace && pnpm test`
Expected: PASS.

```bash
git add src-tauri/tauri.conf.json package.json Cargo.toml Cargo.lock
git commit -m "chore(release): 0.36.0"
```
