# Work Items, part 1: GitHub and GitLab issues — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An Issues tab that lists GitHub and GitLab issues by per-service rules, in two areas (To pick, Being worked on), with a Start button that assigns the issue to the user, sets its status label, and starts Maya's agent on it in a fresh worktree of its repository.

**Architecture:** A `work_items` module in `maya-core` beside the pull request code: rules and settings in `config.rs`; listing, fetching one issue and writing back per forge over the existing `forge::Http`; the Start target, git steps and prompt modelled on `reviews.rs`; Maya's one-shot agent picks a repository when there is none. `src-tauri` polls every 120 s like `poll_reviews`, keeps a record of started items, and exposes list/check/start commands. The page gets `src/workitems.ts` (tab, areas, filters, start dialog) and a Settings → Work Items group.

**Tech Stack:** Rust (maya-core, Tauri 2), `ureq` through `forge::Http`, vanilla TypeScript with vitest.

**Spec:** `docs/superpowers/specs/2026-11-09-work-items-design.md` (mockups in `docs/superpowers/specs/2026-11-09-work-items/`).

**Out of this plan (each its own plan and pull request, after this one merges):** custom tools (Settings → Add New, list and on-start commands), and the phone and voice. This plan keeps `WorkItem.source` a string so custom tools fit later.

## Global Constraints

- Words on screen, verbatim: tab **Issues**; areas **To pick** (left) and **Being worked on** (right); button **Start**; **Open session**; Settings group **Work Items**; "No repo: Maya picks"; takeover question "This issue is already assigned to \<names>. Are you sure you want to take over?"
- Rules match open issues only; pull requests never show.
- A rule with Assignee "anyone" must name repositories; Settings refuses it with: "A rule for anyone's issues must name its repositories."
- Status: an issue's status is the first label of the service's status-label list that it carries (labels compared case-insensitively); only listed labels are statuses.
- Refresh every 120 s, and at once after Settings → Git or Settings → Work Items change; a poll that began before the change drops its answers (`git_generation`).
- Start never runs in the user's own checkouts: only under the clones folder (`Config::clones_dir_path`).
- Branch: `issue-<slug>` where the slug is `worktree::branch_slug(None, "<id> <title>")` (so at most 40 characters); folder `<clones>/<base>-issue-<number>`; session name `issue <id>`.
- A failed write-back never stops the session from starting: it becomes a notice line returned to the page.
- No token in an error, a log line or a returned string: every forge call goes through `forge::answer`/`scrub` as today.
- The version: bump to `0.51.0` from `main`'s `0.50.0` with `sh scripts/set-version.sh 0.51.0`, and add the `# 0.51.0` section to `RELEASE_NOTES.md`, in one `chore(release): 0.51.0` commit (CLAUDE.md).

## Review Focus

- A GitHub label or login with a space or a quote (`in progress`): the search term must be quoted (`label:"in progress"`), or the search silently matches something else. Pinned in Task 2.
- A GitLab project in nested groups (`grp/sub/r`): the project id must be percent-encoded in every URL, and the repo path read from `references.full`. Pinned in Task 3.
- An issue carrying two status labels when Start sets a third: every other status label goes, other labels stay. Pinned in Task 4.
- Start on an item whose earlier worktree was deleted by hand but whose branch remains: reuse the branch, do not fail on `-b`. Pinned in Task 6.
- Start pressed while a session already runs in that item's folder: refused with "A session is already working on \<id>." rather than a second session in the same worktree. Pinned in Task 6.

---

## File Structure

| File | Responsibility |
|---|---|
| `core/src/config.rs` (modify) | `WorkItemsConfig`, `ForgeItems`, `ItemRule`, `Assignee`; defaults; `validate_work_items` |
| `core/src/forge/mod.rs` (modify) | `Http::send_json` (PATCH/PUT); `Need::Issues`, `Need::IssueWrite`; `me()` |
| `core/src/work_items/mod.rs` (create) | `WorkItem`, `ItemRepo`, `status_of`, `merge`, `fetch`/`combine` |
| `core/src/work_items/github.rs` (create) | search query, parse, list, get one, take (assign + labels) |
| `core/src/work_items/gitlab.rs` (create) | list URLs, parse, list, get one, take |
| `core/src/work_items/start.rs` (create) | `WorkTarget`, git steps, branch, prompt, session name, already-running check |
| `core/src/work_items/pick.rs` (create) | candidate repositories, pick prompt, reply parsing |
| `core/src/work_items/started.rs` (create) | started-items record, file, matching to cards, pruning |
| `core/src/actions.rs` (modify) | shared `launch_in_clone` extracted from `start_review`; `start_work_item` |
| `core/src/launch.rs` (modify) | `oneshot_text` extracted from `classify` |
| `src-tauri/src/work_items_app.rs` (create) | `WorkItemsState`, `poll_work_items`, Tauri commands |
| `src-tauri/src/lib.rs`, `src-tauri/src/git.rs` (modify) | state, poll thread, command registration, re-poll on settings change |
| `src/types.ts` (modify) | `WorkItem`, `WorkItemsState`, `StartCheck`, config types |
| `src/workitems.ts` + test (create) | areas, filters, cards, view, click actions, start dialog |
| `src/tabs.ts`, `index.html`, `src/main.ts`, `src/styles.css` (modify) | the Issues tab |
| `src/settings.ts` + `src/settings.test.ts` (modify) | Settings → Work Items |
| `README.md`, `RELEASE_NOTES.md`, version files (modify) | docs and release |

---

### Task 1: Work items settings

**Files:**
- Modify: `core/src/config.rs`
- Test: `core/src/config.rs` (tests module)

**Interfaces:**
- Produces:
  - `pub enum Assignee { Me, Anyone, User { login: String } }` — serde `tag = "kind"`, camelCase (`{"kind":"me"}`, `{"kind":"user","login":"maria"}`)
  - `pub struct ItemRule { pub assignee: Assignee, #[serde(default)] pub labels: Vec<String>, #[serde(default)] pub repos: Vec<String> }` — empty `repos` = any repository
  - `pub struct ForgeItems { pub enabled: bool, pub rules: Vec<ItemRule>, #[serde(default)] pub status_labels: Vec<String>, #[serde(default)] pub start_status: Option<String>, #[serde(default)] pub tracker_repos: Vec<String> }`
  - `pub struct WorkItemsConfig { pub github: ForgeItems, pub gitlab: ForgeItems, #[serde(default)] pub prompt: String }` with `Default`: both services `enabled: true`, one rule `{ assignee: Me, labels: [], repos: [] }`, no status labels, `prompt: ""`
  - `Config.work_items: WorkItemsConfig` with `#[serde(default)]`
  - `impl WorkItemsConfig { pub fn for_forge(&self, p: GitProvider) -> &ForgeItems }`
  - `pub fn validate_work_items(w: &WorkItemsConfig) -> Result<(), String>`
  - `impl Config { pub fn normalize_work_items(&mut self) }` — trims every string, drops empty labels/repos/status labels, strips `https://<host>/` and a trailing `.git` from a pasted repo URL, sets `start_status` to None when it is not in `status_labels`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn work_items_default_to_my_issues_anywhere_on_both_services() {
    let w = Config::default().work_items;
    for f in [&w.github, &w.gitlab] {
        assert!(f.enabled);
        assert_eq!(f.rules, vec![ItemRule { assignee: Assignee::Me, labels: vec![], repos: vec![] }]);
        assert!(f.status_labels.is_empty() && f.start_status.is_none() && f.tracker_repos.is_empty());
    }
    let older: Config = serde_json::from_str(r#"{"completedTimeoutMinutes":5}"#).unwrap();
    assert_eq!(older.work_items, WorkItemsConfig::default(), "a file from before 0.51 reads as the defaults");
}

#[test]
fn work_items_round_trip_in_camel_case() {
    let mut c = Config::default();
    c.work_items.github.rules.push(ItemRule { assignee: Assignee::User { login: "maria".into() }, labels: vec!["in progress".into()], repos: vec!["acme/api".into()] });
    c.work_items.github.status_labels = vec!["todo".into(), "in progress".into()];
    c.work_items.github.start_status = Some("in progress".into());
    let text = serde_json::to_string(&c).unwrap();
    assert!(text.contains(r#""workItems":{"github":{"enabled":true,"rules":[{"assignee":{"kind":"me"},"#), "{text}");
    assert!(text.contains(r#"{"assignee":{"kind":"user","login":"maria"},"labels":["in progress"],"repos":["acme/api"]}"#), "{text}");
    assert!(text.contains(r#""statusLabels":["todo","in progress"],"startStatus":"in progress""#), "{text}");
    assert_eq!(serde_json::from_str::<Config>(&text).unwrap(), c);
}

#[test]
fn a_rule_for_anyone_must_name_its_repositories() {
    let mut w = WorkItemsConfig::default();
    w.gitlab.rules.push(ItemRule { assignee: Assignee::Anyone, labels: vec!["urgent".into()], repos: vec![] });
    assert_eq!(validate_work_items(&w).unwrap_err(), "A rule for anyone's issues must name its repositories.");
    w.gitlab.rules[1].repos = vec!["grp/r".into()];
    assert!(validate_work_items(&w).is_ok());
    w.github.rules.push(ItemRule { assignee: Assignee::User { login: " ".into() }, labels: vec![], repos: vec![] });
    assert_eq!(validate_work_items(&w).unwrap_err(), "A rule for someone else's issues needs their login.");
}

#[test]
fn normalize_trims_and_drops_what_is_not_a_status() {
    let mut c = Config::default();
    c.work_items.github.rules[0].labels = vec![" bug ".into(), "".into()];
    c.work_items.github.rules[0].repos = vec!["https://github.com/acme/api.git".into(), " acme/web ".into()];
    c.work_items.github.status_labels = vec![" todo ".into(), "".into()];
    c.work_items.github.start_status = Some("doing".into());
    c.normalize_work_items();
    let g = &c.work_items.github;
    assert_eq!(g.rules[0].labels, vec!["bug"]);
    assert_eq!(g.rules[0].repos, vec!["acme/api", "acme/web"]);
    assert_eq!(g.status_labels, vec!["todo"]);
    assert_eq!(g.start_status, None, "not one of the status labels");
}
```

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p maya-core config::tests::work_items -- --nocapture` and `cargo test -p maya-core config::tests::a_rule_for_anyone`
Expected: FAIL to compile (`ItemRule` not found).

- [ ] **Step 3: Add the types, defaults, `for_forge`, `validate_work_items` and `normalize_work_items` in `core/src/config.rs`**

Put the types after `GitConnection`. `Config::default()` sets `work_items: WorkItemsConfig::default()`.

- [ ] **Step 4: Run the tests and see them pass**

Run: `cargo test -p maya-core config::`
Expected: PASS, the existing config tests included.

- [ ] **Step 5: Validate and normalize on save**

In `src-tauri/src/lib.rs::set_config`, after `config::validate_git(...)?;` add `config::validate_work_items(&config.work_items)?;`, and after `config.normalize_git();` add `config.normalize_work_items();`. Run `cargo build -p maya` — Expected: builds.

- [ ] **Step 6: Commit**

```bash
git add core/src/config.rs src-tauri/src/lib.rs
git commit -m "feat(work-items): rules and status labels in the settings"
```

---

### Task 2: The work item, and GitHub's list

**Files:**
- Create: `core/src/work_items/mod.rs`, `core/src/work_items/github.rs`
- Modify: `core/src/lib.rs` (`pub mod work_items;`), `core/src/forge/mod.rs`
- Test: in each file's tests module

**Interfaces:**
- Consumes: `ForgeItems`, `ItemRule`, `Assignee` (Task 1); `forge::{Forge, Http, get_json_as, encode, scrub}`.
- Produces:
  - in `forge/mod.rs`: `Need::Issues` and `Need::IssueWrite`; `scopes` gives (GitHub, Issues) `"repo scope (fine-grained: Issues read)"`, (GitHub, IssueWrite) `"repo scope (fine-grained: Issues read and write)"`, (GitLab, Issues) `"read_api scope"`, (GitLab, IssueWrite) `"api scope"`.
  - in `work_items/mod.rs`:
    ```rust
    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
    #[serde(rename_all = "camelCase")]
    pub struct ItemRepo { pub forge: GitProvider, pub path: String }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
    #[serde(rename_all = "camelCase")]
    pub struct WorkItem {
        /// `<source>:<id>`: unique across sources.
        pub key: String,
        /// "github" or "gitlab" (a custom tool's id later).
        pub source: String,
        /// As shown: `acme/api#412`.
        pub id: String,
        pub number: Option<u64>,
        pub title: String,
        pub url: String,
        pub description: String,
        pub status: Option<String>,
        /// The labels that are not the status.
        pub labels: Vec<String>,
        pub assignees: Vec<String>,
        pub author: String,
        pub updated_at: String,
        pub repo: Option<ItemRepo>,
        /// The repo is tracker-only: Start asks Maya to pick the code's repository.
        pub tracker: bool,
    }
    pub fn status_of(labels: &[String], status_labels: &[String]) -> (Option<String>, Vec<String>)
    pub fn merge(lists: Vec<Vec<WorkItem>>) -> Vec<WorkItem>  // de-duplicated by key, newest `updated_at` first
    ```
    `status_of` returns the status as spelled in `status_labels`, and every label that is not a status label.
  - in `work_items/github.rs`:
    ```rust
    pub fn search_query(rule: &ItemRule) -> String
    pub fn parse_issue(v: &Value, cfg: &ForgeItems) -> Option<WorkItem>  // None for a pull request
    pub fn parse_search(v: &Value, cfg: &ForgeItems) -> Vec<WorkItem>
    pub fn list(http: &dyn Http, forge: &Forge, cfg: &ForgeItems) -> Result<Vec<WorkItem>, String>
    ```
    `list` runs one `GET {api}/search/issues?q=<encoded query>&sort=updated&order=desc&per_page=50` per rule (`Need::Issues`), and merges.

- [ ] **Step 1: Write the failing tests**

```rust
// work_items/mod.rs
#[test]
fn the_status_is_the_first_listed_label_it_carries() {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert_eq!(status_of(&s(&["bug", "In Progress", "todo"]), &s(&["todo", "in progress"])), (Some("todo".into()), s(&["bug"])));
    assert_eq!(status_of(&s(&["bug"]), &s(&["todo"])), (None, s(&["bug"])));
    assert_eq!(status_of(&s(&["status::doing"]), &s(&[])), (None, s(&["status::doing"])), "only listed labels are statuses");
}

#[test]
fn merge_keeps_one_of_each_newest_first() {
    let item = |key: &str, at: &str| WorkItem { key: key.into(), updated_at: at.into(), ..test_item() };
    let out = merge(vec![vec![item("github:a#1", "2026-11-01"), item("github:a#2", "2026-11-03")], vec![item("github:a#1", "2026-11-01")]]);
    assert_eq!(out.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(), vec!["github:a#2", "github:a#1"]);
}

// work_items/github.rs
#[test]
fn a_rule_becomes_one_search_with_quoted_terms() {
    let r = |assignee, labels: &[&str], repos: &[&str]| ItemRule { assignee, labels: labels.iter().map(|s| s.to_string()).collect(), repos: repos.iter().map(|s| s.to_string()).collect() };
    assert_eq!(search_query(&r(Assignee::Me, &[], &[])), "is:issue is:open assignee:@me");
    assert_eq!(
        search_query(&r(Assignee::Anyone, &["in progress", "bug"], &["acme/api", "acme/web"])),
        r#"is:issue is:open label:"in progress" label:"bug" repo:acme/api repo:acme/web"#
    );
    assert_eq!(search_query(&r(Assignee::User { login: "maria".into() }, &[r#"say "hi""#], &[])), r#"is:issue is:open assignee:maria label:"say hi""#, "a quote inside is dropped");
}

#[test]
fn the_search_reply_becomes_items_and_skips_pull_requests() {
    let v: Value = serde_json::from_str(r#"{"items":[
      {"number":412,"title":"Login times out","html_url":"https://github.com/acme/api/issues/412","body":"Steps…","updated_at":"2026-11-08T10:00:00Z",
       "repository_url":"https://api.github.com/repos/acme/api","user":{"login":"jane"},
       "labels":[{"name":"bug"},{"name":"todo"}],"assignees":[{"login":"tiago"}]},
      {"number":7,"title":"A PR","html_url":"https://github.com/acme/api/pull/7","pull_request":{},"repository_url":"https://api.github.com/repos/acme/api","labels":[],"assignees":[],"user":{"login":"x"}}
    ]}"#).unwrap();
    let cfg = ForgeItems { status_labels: vec!["todo".into()], tracker_repos: vec!["acme/api".into()], ..WorkItemsConfig::default().github };
    let items = parse_search(&v, &cfg);
    assert_eq!(items.len(), 1);
    let i = &items[0];
    assert_eq!((i.key.as_str(), i.source.as_str(), i.id.as_str(), i.number), ("github:acme/api#412", "github", "acme/api#412", Some(412)));
    assert_eq!((i.status.as_deref(), i.labels.clone(), i.assignees.clone(), i.author.as_str()), (Some("todo"), vec!["bug".to_string()], vec!["tiago".to_string()], "jane"));
    assert_eq!(i.repo, Some(ItemRepo { forge: GitProvider::Github, path: "acme/api".into() }));
    assert!(i.tracker, "acme/api is tracker-only");
    assert_eq!(i.description, "Steps…");
}

#[test]
fn list_asks_once_per_rule_and_names_the_issues_permission_on_refusal() {
    let f = Forge { site: ForgeSite::of(&GitConnection::default_for(GitProvider::Github)), token: "ghp_x".into() };
    let mut cfg = WorkItemsConfig::default().github;
    cfg.rules.push(ItemRule { assignee: Assignee::Anyone, labels: vec![], repos: vec!["acme/api".into()] });
    let http = FakeHttp::new(vec![("/search/issues", 200, r#"{"items":[]}"#)]);
    assert!(list(&http, &f, &cfg).unwrap().is_empty());
    let seen = http.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert!(seen[1].0.starts_with("https://api.github.com/search/issues?q=is%3Aissue%20is%3Aopen%20repo%3Aacme%2Fapi&sort=updated"), "{}", seen[1].0);
    let refused = FakeHttp::new(vec![("/search/issues", 403, "{}")]);
    assert_eq!(list(&refused, &f, &cfg).unwrap_err(), "the token was refused (403). It needs the repo scope (fine-grained: Issues read).");
}
```

(`test_item()` is a `#[cfg(test)] pub(crate) fn` in `work_items/mod.rs` returning a `WorkItem` with every field empty or None; later tasks' tests reuse it.)

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p maya-core work_items::`
Expected: FAIL to compile (module missing).

- [ ] **Step 3: Implement**

`search_query`: terms in the order is/assignee/labels/repos; a label is wrapped in `"…"` with any `"` removed from it; a repo is written bare. `parse_issue` reads the repo path from `repository_url` after `/repos/`, skips a node with a `pull_request` key, sets `tracker` when the path is in `cfg.tracker_repos` (case-insensitive). Map `forge::check`'s errors through `Need::Issues`.

- [ ] **Step 4: Run them and see them pass**

Run: `cargo test -p maya-core work_items:: forge::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/lib.rs core/src/forge/mod.rs core/src/work_items/
git commit -m "feat(work-items): list GitHub issues by rule"
```

---

### Task 3: GitLab's list

**Files:**
- Create: `core/src/work_items/gitlab.rs`
- Test: same file

**Interfaces:**
- Consumes: `WorkItem`, `ItemRepo`, `status_of`, `merge` (Task 2); `Need::Issues`.
- Produces:
  ```rust
  pub fn list_urls(api_base: &str, rule: &ItemRule) -> Vec<String>
  pub fn parse_issue(v: &Value, cfg: &ForgeItems) -> Option<WorkItem>
  pub fn list(http: &dyn Http, forge: &Forge, cfg: &ForgeItems) -> Result<Vec<WorkItem>, String>
  ```
  `list_urls`: no repos → one `{api}/issues?…`; with repos → one `{api}/projects/<encoded path>/issues?…` per repo. Every URL has `state=opened&order_by=updated_at&sort=desc&per_page=50`, then: Me → `scope=assigned_to_me` (on both endpoints); User → `scope=all&assignee_username=<encoded login>` on `/issues`, `assignee_username=<encoded login>` on a project; Anyone → nothing (it always has projects); labels → `labels=<each encoded, joined by ,>`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_rule_becomes_one_call_per_project_with_encoded_paths() {
    let api = "https://gitlab.acme.io/api/v4";
    let me = ItemRule { assignee: Assignee::Me, labels: vec![], repos: vec![] };
    assert_eq!(list_urls(api, &me), vec![format!("{api}/issues?state=opened&order_by=updated_at&sort=desc&per_page=50&scope=assigned_to_me")]);
    let anyone = ItemRule { assignee: Assignee::Anyone, labels: vec!["status::todo".into(), "a b".into()], repos: vec!["grp/sub/r".into(), "grp/x".into()] };
    assert_eq!(list_urls(api, &anyone), vec![
        format!("{api}/projects/grp%2Fsub%2Fr/issues?state=opened&order_by=updated_at&sort=desc&per_page=50&labels=status%3A%3Atodo,a%20b"),
        format!("{api}/projects/grp%2Fx/issues?state=opened&order_by=updated_at&sort=desc&per_page=50&labels=status%3A%3Atodo,a%20b"),
    ]);
    let maria = ItemRule { assignee: Assignee::User { login: "maria".into() }, labels: vec![], repos: vec![] };
    assert_eq!(list_urls(api, &maria), vec![format!("{api}/issues?state=opened&order_by=updated_at&sort=desc&per_page=50&scope=all&assignee_username=maria")]);
}

#[test]
fn an_issue_takes_its_repo_from_the_full_reference() {
    let v: Value = serde_json::from_str(r#"{"iid":17,"title":"VAT missing","description":null,"web_url":"https://gitlab.acme.io/grp/sub/billing/-/issues/17",
      "updated_at":"2026-11-07T09:00:00Z","labels":["backend","status::todo"],"assignees":[{"username":"tiago"}],"author":{"username":"ana"},
      "references":{"full":"grp/sub/billing#17"}}"#).unwrap();
    let cfg = ForgeItems { status_labels: vec!["status::todo".into(), "status::doing".into()], ..WorkItemsConfig::default().gitlab };
    let i = parse_issue(&v, &cfg).unwrap();
    assert_eq!((i.key.as_str(), i.id.as_str(), i.number), ("gitlab:grp/sub/billing#17", "grp/sub/billing#17", Some(17)));
    assert_eq!(i.repo, Some(ItemRepo { forge: GitProvider::Gitlab, path: "grp/sub/billing".into() }));
    assert_eq!((i.status.as_deref(), i.labels.clone(), i.description.as_str()), (Some("status::todo"), vec!["backend".to_string()], ""));
}

#[test]
fn list_merges_the_projects_and_one_failure_fails_the_service() {
    let c = GitConnection { enabled: true, base_url: Some("https://gitlab.acme.io".into()), ..GitConnection::default_for(GitProvider::Gitlab) };
    let f = Forge { site: ForgeSite::of(&c), token: "glpat".into() };
    let cfg = ForgeItems { rules: vec![ItemRule { assignee: Assignee::Anyone, labels: vec![], repos: vec!["grp/a".into(), "grp/b".into()] }], ..WorkItemsConfig::default().gitlab };
    let http = FakeHttp::new(vec![("grp%2Fa", 200, "[]"), ("grp%2Fb", 404, r#"{"message":"404 Project Not Found"}"#)]);
    assert_eq!(list(&http, &f, &cfg).unwrap_err(), "404: 404 Project Not Found");
}
```

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p maya-core work_items::gitlab`
Expected: FAIL to compile.

- [ ] **Step 3: Implement** `list_urls`, `parse_issue` (repo path = `references.full` up to its last `#`; `description` null → ""), and `list` (every URL in turn; the first error is the service's error).

- [ ] **Step 4: Run them and see them pass**

Run: `cargo test -p maya-core work_items::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/work_items/
git commit -m "feat(work-items): list GitLab issues by rule"
```

---

### Task 4: One issue, and taking it

**Files:**
- Modify: `core/src/forge/mod.rs`, `core/src/work_items/{mod,github,gitlab}.rs`
- Test: same files

**Interfaces:**
- Consumes: Tasks 2–3.
- Produces:
  - `Http::send_json(&self, method: &str, url: &str, token: &str, body: &str) -> Result<(u16, String), String>` (`"PATCH"` or `"PUT"`), implemented on `Ureq` with the same headers as `post_json`; `FakeHttp` records it like `post_json` and also the method (`seen` becomes `(url, token, body, method)`; update existing test reads).
  - `pub(crate) fn send_json_as(http, forge, method, url, body: &Value, need: Need) -> Result<String, String>` in `forge/mod.rs`.
  - `pub struct Me { pub login: String, pub id: u64 }` and `pub fn me(http: &dyn Http, forge: &Forge) -> Result<Me, String>` (`GET {api}/user`; GitHub `login`/`id`, GitLab `username`/`id`).
  - `work_items::labels_after_status(current: &[String], status_labels: &[String], new: &str) -> Vec<String>` — the current labels without any status label, plus `new`.
  - `pub fn get(http, forge, repo: &str, number: u64, cfg: &ForgeItems) -> Result<Option<(WorkItem, Vec<u64>)>, String>` in each of `github.rs` and `gitlab.rs`: the issue and its assignees' ids; `Ok(None)` when it is closed. (GitHub `GET {api}/repos/{repo}/issues/{n}`, `state == "closed"`; GitLab `GET {api}/projects/{enc}/issues/{n}`, `state == "closed"`.)
  - `take`, which assigns the user (keeping the others where the service allows) and, when `cfg.start_status` is Some, sets that status. `all_labels` is every label the issue carries now, its status included (`item.labels` plus `item.status` from a fresh `get`):
    - GitHub (assignees go by login): `pub fn take(http, forge, repo: &str, number: u64, me: &Me, assignee_logins: &[String], all_labels: &[String], cfg: &ForgeItems) -> Result<(), String>` — one `PATCH {api}/repos/{repo}/issues/{n}` with `{"assignees": <assignee_logins + me.login, deduped>}` plus, with a start status, `"labels": labels_after_status(all_labels, &cfg.status_labels, status)`.
    - GitLab (assignees go by id): `pub fn take(http, forge, repo: &str, number: u64, me: &Me, assignee_ids: &[u64], all_labels: &[String], cfg: &ForgeItems) -> Result<(), String>` — `PUT {api}/projects/{enc}/issues/{n}` with `{"assignee_ids": <assignee_ids + me.id, deduped>}` plus, with a start status, `"add_labels": <status>, "remove_labels": <the other status labels in all_labels, joined by ,>` (the key left out when there are none). When the reply's `assignees` lacks `me.id` (a single-assignee tier), a second `PUT` with `{"assignee_ids": [me.id]}`.
    - Both use `Need::IssueWrite`, so a read-only token's refusal says what it needs.

- [ ] **Step 1: Write the failing tests**

```rust
// work_items/mod.rs
#[test]
fn a_new_status_replaces_every_other_status_and_keeps_the_rest() {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert_eq!(labels_after_status(&s(&["bug", "todo", "In Review"]), &s(&["todo", "in progress", "in review"]), "in progress"), s(&["bug", "in progress"]));
}

// work_items/github.rs
#[test]
fn take_adds_me_beside_the_others_and_sets_the_status_in_one_call() {
    let f = github_forge();
    let http = FakeHttp::new(vec![("/issues/412", 200, "{}")]);
    let cfg = ForgeItems { status_labels: vec!["todo".into(), "in progress".into()], start_status: Some("in progress".into()), ..WorkItemsConfig::default().github };
    take(&http, &f, "acme/api", 412, &Me { login: "tiago".into(), id: 1 }, &["maria".to_string()], &["bug".to_string(), "todo".to_string()], &cfg).unwrap();
    let seen = http.seen.lock().unwrap();
    assert_eq!((seen[0].0.as_str(), seen[0].3.as_deref()), ("https://api.github.com/repos/acme/api/issues/412", Some("PATCH")));
    let body: Value = serde_json::from_str(seen[0].2.as_deref().unwrap()).unwrap();
    assert_eq!(body, serde_json::json!({"assignees": ["maria", "tiago"], "labels": ["bug", "in progress"]}));
}

#[test]
fn a_read_only_token_is_told_it_needs_write() {
    let http = FakeHttp::new(vec![("/issues/1", 403, "{}")]);
    let e = take(&http, &github_forge(), "a/b", 1, &Me { login: "t".into(), id: 1 }, &[], &[], &WorkItemsConfig::default().github).unwrap_err();
    assert_eq!(e, "the token was refused (403). It needs the repo scope (fine-grained: Issues read and write).");
}

#[test]
fn get_says_none_for_a_closed_issue() {
    let http = FakeHttp::new(vec![("/issues/9", 200, r#"{"number":9,"state":"closed","title":"t","html_url":"u","repository_url":"https://api.github.com/repos/a/b","labels":[],"assignees":[],"user":{"login":"x"}}"#)]);
    assert_eq!(get(&http, &github_forge(), "a/b", 9, &WorkItemsConfig::default().github).unwrap(), None);
}

// work_items/gitlab.rs
#[test]
fn take_replaces_the_assignee_when_the_tier_allows_only_one() {
    let http = FakeHttp::new(vec![("/issues/17", 200, r#"{"assignees":[{"id":5}]}"#)]);
    let cfg = ForgeItems { status_labels: vec!["status::todo".into(), "status::doing".into()], start_status: Some("status::doing".into()), ..WorkItemsConfig::default().gitlab };
    take(&http, &gitlab_forge(), "grp/r", 17, &Me { login: "tiago".into(), id: 9 }, &[5], &["status::todo".to_string(), "backend".to_string()], &cfg).unwrap();
    let seen = http.seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "the first PUT kept only the old assignee, so a second one replaces it");
    let first: Value = serde_json::from_str(seen[0].2.as_deref().unwrap()).unwrap();
    assert_eq!(first, serde_json::json!({"assignee_ids": [5, 9], "add_labels": "status::doing", "remove_labels": "status::todo"}));
    assert_eq!(seen[1].2.as_deref(), Some(r#"{"assignee_ids":[9]}"#));
    assert!(seen[0].0.ends_with("/projects/grp%2Fr/issues/17") && seen[0].3.as_deref() == Some("PUT"));
}
```

(`github_forge()` and `gitlab_forge()` are test helpers in each tests module, as `forge()` is in `forge/github.rs`.)

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p maya-core work_items:: forge::`
Expected: FAIL to compile.

- [ ] **Step 3: Implement** `send_json` (trait, `Ureq`, `FakeHttp`), `send_json_as`, `Me`/`me`, `labels_after_status`, `get` and `take` in both services, as the Interfaces say. Fix the existing `forge` tests that index `seen` tuples.

- [ ] **Step 4: Run them and see them pass**

Run: `cargo test -p maya-core`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/forge/ core/src/work_items/
git commit -m "feat(work-items): fetch one issue, assign it and set its status"
```

---

### Task 5: Started items and the poll

**Files:**
- Create: `core/src/work_items/started.rs`, `src-tauri/src/work_items_app.rs`
- Modify: `core/src/work_items/mod.rs`, `src-tauri/src/lib.rs`, `src-tauri/src/git.rs`
- Test: `core/src/work_items/{mod,started}.rs`

**Interfaces:**
- Consumes: Tasks 1–4; `git::forges`, `git::http`, `AppState.git_generation`; `model::Card`.
- Produces:
  - in `work_items/mod.rs`:
    ```rust
    pub struct Fetched { pub forge: GitProvider, pub result: Result<Vec<WorkItem>, String> }
    pub fn fetch(forges: &[Forge], cfg: &WorkItemsConfig, http: &dyn Http) -> Vec<Fetched>  // every switched-on service at once, as reviews::fetch
    pub fn combine(previous: &[WorkItem], fetched: Vec<Fetched>, errors: Vec<ForgeError>) -> (Vec<WorkItem>, Vec<ForgeError>)  // as reviews::combine: a failed service keeps its last items and adds "<Label> issues check failed: <e>"
    ```
    `fetch` skips a forge whose `ForgeItems.enabled` is false.
  - in `started.rs`:
    ```rust
    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
    #[serde(rename_all = "camelCase")]
    pub struct Started { pub key: String, pub item: WorkItem, pub dir: String, pub started_at: u64 }
    pub fn path(maya_dir: &Path) -> PathBuf            // <maya_dir>/work-items.json
    pub fn load(path: &Path) -> Vec<Started>            // missing or bad file: empty
    pub fn save(path: &Path, started: &[Started]) -> Result<(), String>
    pub fn session_name(id: &str) -> String             // "issue <id>"
    pub fn has_session(s: &Started, cards: &[Card]) -> bool  // a card named session_name(id), or whose cwd is s.dir or under it
    pub const GRACE_MS: u64 = 10 * 60 * 1000;
    pub fn prune(started: Vec<Started>, closed: &[String], cards: &[Card], now_ms: u64) -> Vec<Started>
    ```
    `prune` drops an entry whose key is in `closed`, and one with no session that started more than `GRACE_MS` ago.
  - in `src-tauri/src/work_items_app.rs`:
    ```rust
    #[derive(Serialize, Clone, Default)]
    #[serde(rename_all = "camelCase")]
    pub struct WorkItemsState {
        pub items: Vec<WorkItem>,       // the rules' items, then started items no rule returned (their latest copy)
        pub started: Vec<Started>,
        pub errors: Vec<ForgeError>,
        pub off: bool,                  // no switched-on Git connection with Work Items on
        pub fetched_at: Option<u64>,
    }
    pub(crate) fn poll_work_items(app: &AppHandle)
    #[tauri::command] fn list_work_items(state) -> WorkItemsState
    ```
    `AppState` gains `work_items: Mutex<WorkItemsState>` (lock rule: never held across a network call or with another lock). The poll: read config and connections; `git::forges`; `fetch`; for each started entry of a switched-on forge that no rule returned, `get` it (closed → its key goes in `closed`; error → keep the stored copy); `prune` with the board's cards; save the started file when it changed; drop the answers when `git_generation` moved; store; `emit("work-items", &state)`.
  - In `src-tauri/src/lib.rs`: extract `pub(crate) fn board_cards(app: &AppHandle) -> Vec<Card>` from the first lines of `refresh_and_emit` (local cards merged with remote boards), and use it there; spawn a thread `loop { poll_work_items(&h); sleep(120 s) }` next to the review poll; register `list_work_items`.
  - In `git::settings_changed`: also spawn `crate::work_items_app::poll_work_items(&handle)`. In `set_config`: when `before.work_items != config.work_items`, bump `git_generation` and spawn the poll.

- [ ] **Step 1: Write the failing tests**

```rust
// work_items/mod.rs
#[test]
fn a_failed_service_keeps_its_last_items_and_says_so() {
    let gh = WorkItem { key: "github:a/b#1".into(), source: "github".into(), ..test_item() };
    let gl = WorkItem { key: "gitlab:g/r#2".into(), source: "gitlab".into(), ..test_item() };
    let fetched = vec![Fetched { forge: GitProvider::Github, result: Err("could not reach github.com: timeout".into()) }, Fetched { forge: GitProvider::Gitlab, result: Ok(vec![]) }];
    let (items, errors) = combine(&[gh.clone(), gl], fetched, vec![]);
    assert_eq!(items, vec![gh]);
    assert_eq!(errors[0].message, "GitHub issues check failed: could not reach github.com: timeout");
}

// work_items/started.rs
#[test]
fn a_started_item_matches_its_session_by_name_or_folder() {
    let s = Started { key: "github:a/b#1".into(), item: WorkItem { id: "a/b#1".into(), ..test_item() }, dir: "/c/b-issue-1".into(), started_at: 0 };
    let card = |name: &str, cwd: &str| Card { name: name.into(), cwd: cwd.into(), ..Card::test_default() };
    assert!(has_session(&s, &[card("issue a/b#1", "/elsewhere")]));
    assert!(has_session(&s, &[card("renamed", "/c/b-issue-1/sub")]));
    assert!(!has_session(&s, &[card("renamed", "/c/b-issue-10")]), "a sibling folder is not the item's");
}

#[test]
fn prune_drops_closed_items_and_sessionless_ones_after_the_grace() {
    let s = |key: &str, at: u64| Started { key: key.into(), item: test_item(), dir: format!("/c/{key}"), started_at: at };
    let now = GRACE_MS * 3;
    let kept = prune(vec![s("closed", now), s("young", now - 1000), s("old", 0)], &["closed".into()], &[], now);
    assert_eq!(kept.iter().map(|s| s.key.as_str()).collect::<Vec<_>>(), vec!["young"]);
}

#[test]
fn the_started_file_round_trips_and_a_bad_one_reads_empty() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(dir.path());
    assert!(load(&p).is_empty());
    let one = vec![Started { key: "k".into(), item: test_item(), dir: "/d".into(), started_at: 5 }];
    save(&p, &one).unwrap();
    assert_eq!(load(&p), one);
    std::fs::write(&p, "not json").unwrap();
    assert!(load(&p).is_empty());
}
```

(If `Card` has no test constructor, add `#[cfg(test)] pub fn test_default() -> Card` in `core/src/model.rs` with empty fields.)

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p maya-core work_items::`
Expected: FAIL to compile.

- [ ] **Step 3: Implement** `fetch`, `combine`, `started.rs`, then `work_items_app.rs`, `board_cards`, the poll thread, the command registration and the re-poll triggers.

- [ ] **Step 4: Run the tests and build the app**

Run: `cargo test -p maya-core work_items:: && cargo build -p maya`
Expected: PASS, and the app builds.

- [ ] **Step 5: Commit**

```bash
git add core/src/ src-tauri/src/
git commit -m "feat(work-items): poll the issues and remember started ones"
```

---

### Task 6: Where Start works, and the shared launcher

**Files:**
- Create: `core/src/work_items/start.rs`
- Modify: `core/src/reviews.rs` (make `is_checkout_of`, `is_worktree_of` `pub(crate)`), `core/src/actions.rs`
- Test: `core/src/work_items/start.rs`, `core/src/actions.rs`

**Interfaces:**
- Consumes: `WorkItem`, `ItemRepo`, `started::session_name`; `reviews::{is_checkout_of, is_worktree_of, is_busy, run_git_steps}`; `worktree::branch_slug`; `forge::ForgeSite`.
- Produces:
  ```rust
  pub struct WorkTarget {
      pub clone_dir: PathBuf, pub dir: PathBuf, pub branch: String,
      pub clone: bool,            // the clone does not exist yet
      pub worktree_exists: bool,  // dir is already a worktree of the clone
      pub branch_exists: bool,    // refs/heads/<branch> exists in the clone
  }
  pub fn branch_name(item: &WorkItem) -> String   // "issue-" + worktree::branch_slug(None, &format!("{} {}", item.id, item.title)), cut so the whole is at most 40 characters
  pub fn resolve_target(clones_root: &Path, site: &ForgeSite, repo: &str, number: u64, branch: &str) -> Result<WorkTarget, String>
  pub fn git_steps(target: &WorkTarget, clone_url: &str) -> Vec<Vec<String>>
  pub const DEFAULT_WORK_PROMPT: &str = "Work on {id}: {title} ({url}). This folder is a fresh worktree of {repo} on its own branch. Implement or fix what the work item asks, test it, and open a pull request when it is done.\n\n{description}";
  pub fn render_prompt(template: &str, item: &WorkItem, repo: &str) -> String  // blank template = DEFAULT_WORK_PROMPT; fills {id} {title} {url} {description} {repo}
  ```
  `resolve_target` picks the clone base as `reviews::resolve_target` does (`<repo name>`, or the full path with `/` → `-` when that folder is another repository's), and `dir = <clones>/<base>-issue-<number>`; refuses a `dir` that exists and is not a worktree of the clone with "\<dir> is not a worktree of \<repo>; move it or change the clones directory."
  `git_steps`: clone when `clone`; `fetch origin`; when not `worktree_exists`: `worktree prune`, then `worktree add <dir> <branch>` when `branch_exists`, else `worktree add -b <branch> <dir> origin/HEAD`.
  - In `actions.rs`, extract from `start_review` everything after the target is resolved into
    ```rust
    fn launch_in_clone(l: &Local, agent: Harness, clones: &Path, dir: &Path, steps: Vec<Vec<String>>, session: &str, prompt: &str) -> Result<String, String>
    ```
    (the OpenCode branch runs the steps itself; the others build the shell line from `steps` + `launch::session_command`, open the terminal and queue the pending name.) `reviews::shell_command` becomes `pub fn shell_command(agent, dir: &Path, clone_dir: &Path, steps: &[Vec<String>], session, prompt_file, grok_id) -> String`; `start_review` calls `launch_in_clone` and keeps its behaviour (its existing tests must still pass unchanged apart from the call shape).
  - `pub fn start_work_item(l: &Local, item: &WorkItem, repo: &ItemRepo) -> Result<String, String>` in `actions.rs`: agent checks as `start_review`; site for `repo.forge` (else "Turn on \<Label> in Settings → Git first."); `resolve_target`; when the folder is busy (`reviews::is_busy` on it and its canonical path) → `Err(format!("A session is already working on {}.", item.id))`; create the clones folder; `launch_in_clone` with `render_prompt(&config.work_items.prompt, item, &repo.path)` and `started::session_name(&item.id)`. Returns the folder.

- [ ] **Step 1: Write the failing tests**

```rust
// work_items/start.rs
#[test]
fn the_branch_names_the_item_and_stays_short() {
    let i = WorkItem { id: "acme/api#412".into(), title: "Login times out after 30 s on slow networks".into(), ..test_item() };
    let b = branch_name(&i);
    assert!(b.starts_with("issue-acme-api-412-login"), "{b}");
    assert!(b.len() <= 40, "{b}");
}

#[test]
fn a_new_item_gets_a_new_branch_off_the_default_and_an_old_branch_is_reused() {
    let t = |branch_exists, worktree_exists, clone| WorkTarget { clone_dir: "/c/api".into(), dir: "/c/api-issue-412".into(), branch: "issue-x".into(), clone, worktree_exists, branch_exists };
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert_eq!(git_steps(&t(false, false, true), "git@github.com:acme/api.git"), vec![
        s(&["clone", "--", "git@github.com:acme/api.git", "/c/api"]),
        s(&["-C", "/c/api", "fetch", "origin"]),
        s(&["-C", "/c/api", "worktree", "prune"]),
        s(&["-C", "/c/api", "worktree", "add", "-b", "issue-x", "/c/api-issue-412", "origin/HEAD"]),
    ]);
    assert_eq!(git_steps(&t(true, false, false), "u")[2..], [s(&["-C", "/c/api", "worktree", "prune"]), s(&["-C", "/c/api", "worktree", "add", "/c/api-issue-412", "issue-x"])]);
    assert_eq!(git_steps(&t(true, true, false), "u"), vec![s(&["-C", "/c/api", "fetch", "origin"])], "an existing worktree is used as it is");
}

#[test]
fn the_prompt_fills_the_item_in() {
    let i = WorkItem { id: "acme/api#412".into(), title: "Login".into(), url: "https://x/412".into(), description: "Steps".into(), ..test_item() };
    let p = render_prompt("", &i, "acme/api");
    assert!(p.starts_with("Work on acme/api#412: Login (https://x/412). This folder is a fresh worktree of acme/api"), "{p}");
    assert!(p.ends_with("\n\nSteps"));
    assert_eq!(render_prompt("/fix {id} in {repo}", &i, "acme/api"), "/fix acme/api#412 in acme/api");
}

#[test]
fn a_folder_that_is_not_the_clones_worktree_is_refused() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("api-issue-412")).unwrap();
    let site = ForgeSite::of(&GitConnection::default_for(GitProvider::Github));
    let e = resolve_target(root.path(), &site, "acme/api", 412, "issue-x").unwrap_err();
    assert!(e.ends_with("is not a worktree of acme/api; move it or change the clones directory."), "{e}");
}

// actions.rs
#[test]
fn start_work_item_refuses_a_folder_a_session_already_works_in() {
    // Build a Local with a live card whose cwd is <clones>/api-issue-412, as start_review's busy test does.
    // assert_eq!(err, "A session is already working on acme/api#412.");
}
```

(The last test follows `start_review`'s existing busy test in `actions.rs` for its `Local` set-up; copy that set-up.)

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p maya-core work_items::start actions::`
Expected: FAIL to compile.

- [ ] **Step 3: Implement** `start.rs`, the `reviews.rs` visibility and `shell_command` change, `launch_in_clone`, and `start_work_item`.

- [ ] **Step 4: Run all core tests**

Run: `cargo test -p maya-core`
Expected: PASS, every `reviews` and `actions` test included.

- [ ] **Step 5: Commit**

```bash
git add core/src/
git commit -m "feat(work-items): start an item in a fresh worktree of its repository"
```

---

### Task 7: Maya picks the repository

**Files:**
- Create: `core/src/work_items/pick.rs`
- Modify: `core/src/launch.rs`
- Test: both

**Interfaces:**
- Consumes: `WorkItem`, `ItemRepo`, `WorkItemsConfig`; `launch::{oneshot_args, final_text, find_binary, CLASSIFIER_TIMEOUT}`; `forge::{origin_of, route, ForgeSite}`.
- Produces:
  - `pub fn oneshot_text(agent: Harness, binary: &Path, model: Option<&str>, prompt: &str, cwd: &Path, timeout: Duration) -> Option<String>` in `launch.rs`, extracted from `classify` (which then calls it and `pick_dir`).
  - in `pick.rs`:
    ```rust
    pub fn candidates(cfg: &WorkItemsConfig, items: &[WorkItem], clones: &[ItemRepo]) -> Vec<ItemRepo>
    pub fn pick_prompt(item: &WorkItem, candidates: &[ItemRepo]) -> String
    pub fn parse_pick(reply: &str, candidates: &[ItemRepo]) -> Option<(ItemRepo, String)>
    pub fn clone_repos(clones_root: &Path, sites: &[ForgeSite]) -> Vec<ItemRepo>  // each subfolder's origin, routed; worktrees named *-issue-* or *-<n> included is fine (de-duplicated)
    ```
    `candidates`: every rule's named repos and every listed item's repo, plus `clones`, minus every tracker repo, de-duplicated, sorted by `<forge>:<path>`. Candidates are written in the prompt as `github:acme/api`.
    `pick_prompt` asks: "Which repository holds the code for this work item?", gives the item's id, title and description (description cut to 2000 characters), lists the candidates one per line, and says: "Reply with two lines: the repository exactly as listed, then one short sentence saying why. Reply NONE if none fits."
    `parse_pick`: the first non-empty line, trimmed of quotes/backticks and a trailing period, matched case-insensitively against the listed spellings; the second non-empty line is the reason (empty if missing). NONE or no match → None.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn candidates_leave_out_the_trackers_and_repeat_nothing() {
    let mut cfg = WorkItemsConfig::default();
    cfg.github.tracker_repos = vec!["acme/issues".into()];
    cfg.github.rules.push(ItemRule { assignee: Assignee::Anyone, labels: vec![], repos: vec!["acme/api".into(), "acme/issues".into()] });
    let item = |f, p: &str| WorkItem { repo: Some(ItemRepo { forge: f, path: p.into() }), ..test_item() };
    let c = candidates(&cfg, &[item(GitProvider::Github, "acme/web"), item(GitProvider::Github, "acme/api")], &[ItemRepo { forge: GitProvider::Gitlab, path: "grp/r".into() }]);
    assert_eq!(c.iter().map(|r| format!("{:?}:{}", r.forge, r.path)).collect::<Vec<_>>(), vec!["Github:acme/api", "Github:acme/web", "Gitlab:grp/r"]);
}

#[test]
fn the_pick_is_a_listed_repository_and_a_reason() {
    let c = vec![ItemRepo { forge: GitProvider::Github, path: "acme/api".into() }, ItemRepo { forge: GitProvider::Gitlab, path: "grp/r".into() }];
    assert_eq!(parse_pick("`github:acme/api`\nThe search endpoint lives there.", &c), Some((c[0].clone(), "The search endpoint lives there.".into())));
    assert_eq!(parse_pick("GITLAB:grp/r.", &c), Some((c[1].clone(), String::new())));
    assert_eq!(parse_pick("NONE", &c), None);
    assert_eq!(parse_pick("github:acme/other\nwhy", &c), None);
}

#[test]
fn the_prompt_lists_the_candidates_as_they_must_be_answered() {
    let c = vec![ItemRepo { forge: GitProvider::Github, path: "acme/api".into() }];
    let p = pick_prompt(&WorkItem { id: "ENG-1".into(), title: "Rate-limit search".into(), description: "x".repeat(5000), ..test_item() }, &c);
    assert!(p.contains("\ngithub:acme/api\n") && p.contains("Rate-limit search"));
    assert!(p.len() < 3000, "the description is cut");
}
```

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p maya-core work_items::pick launch::`
Expected: FAIL to compile.

- [ ] **Step 3: Implement** `oneshot_text` (and `classify` on top of it), and `pick.rs`.

- [ ] **Step 4: Run them and see them pass**

Run: `cargo test -p maya-core`
Expected: PASS, the existing `classify_*` tests included.

- [ ] **Step 5: Commit**

```bash
git add core/src/
git commit -m "feat(work-items): Maya picks the repository for an item without one"
```

---

### Task 8: The check and start commands

**Files:**
- Modify: `src-tauri/src/work_items_app.rs`, `src-tauri/src/lib.rs`
- Test: `src-tauri/src/work_items_app.rs` (pure helpers)

**Interfaces:**
- Consumes: everything above.
- Produces:
  ```rust
  #[derive(Serialize)] #[serde(rename_all = "camelCase")]
  pub struct RepoPick { pub picked: Option<ItemRepo>, pub why: String, pub options: Vec<ItemRepo> }
  #[derive(Serialize)] #[serde(rename_all = "camelCase")]
  pub struct StartCheck { pub takeover: Vec<String>, pub pick: Option<RepoPick>, pub branch: String }
  #[derive(Serialize)] #[serde(rename_all = "camelCase")]
  pub struct WorkStarted { pub dir: String, pub native_term: Option<u64>, pub notice: Option<String> }

  fn others(assignees: &[String], me: &str) -> Vec<String>   // assignees that are not me, case-insensitive
  #[tauri::command(async)] fn work_item_check(app: AppHandle, key: String) -> Result<StartCheck, String>
  #[tauri::command(async)] fn work_item_start(app: AppHandle, key: String, repo: Option<ItemRepo>, take_over: bool) -> Result<WorkStarted, String>
  #[tauri::command(async)] fn open_work_item(state: TauriState<AppState>, key: String) -> Result<(), String>  // opens item.url from the list
  ```
  - `work_item_check`: finds the item by key in `WorkItemsState.items` (else "That issue is no longer on the list."); `me()` for its forge (cache the login per forge in `AppState` until `git_generation` moves); `takeover = others(item.assignees, me.login)`; when `item.repo` is None or `item.tracker`, runs the pick (`candidates` with `clone_repos`, `oneshot_text` on Maya's brain in Maya's data folder, `parse_pick`) and returns `RepoPick` (picked None and why "" when Maya had no answer; `options` always the candidates); `branch = start::branch_name(item)`.
  - `work_item_start`: refuses `repo` None when the item needs a pick ("Choose a repository first."), and an item with others assigned when `!take_over` ("This issue is assigned to someone else."); re-reads the issue with `get` (closed → "That issue was closed."); `take` with `Need::IssueWrite` — an error becomes `notice = Some(format!("Started, but {} was not updated: {e}", item.id))`; then `native::route::opening(|| actions::start_work_item(&local, &item, &repo))`; on success records `Started { key, item, dir, started_at: now }` in the started file and in `WorkItemsState.started`, emits `work-items`, and returns.
  - Register the three commands in `lib.rs`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn others_are_the_assignees_that_are_not_me() {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert_eq!(others(&s(&["Tiago", "maria"]), "tiago"), s(&["maria"]));
    assert!(others(&s(&["tiago"]), "TIAGO").is_empty());
    assert!(others(&[], "tiago").is_empty());
}
```

- [ ] **Step 2: Run it and see it fail**

Run: `cargo test -p maya work_items_app`
Expected: FAIL to compile.

- [ ] **Step 3: Implement** the structs, `others` and the three commands as the Interfaces say.

- [ ] **Step 4: Run the tests and build**

Run: `cargo test -p maya work_items_app && cargo build -p maya`
Expected: PASS, and it builds.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/
git commit -m "feat(work-items): check and start commands with takeover and repo pick"
```

---

### Task 9: The Issues tab

**Files:**
- Create: `src/workitems.ts`, `src/workitems.test.ts`
- Modify: `src/types.ts`, `src/tabs.ts`, `index.html`, `src/main.ts`, `src/styles.css`

**Interfaces:**
- Consumes: `list_work_items`, the `work-items` event, `open_work_item` (Task 5/8); `Card`, `STATE_LABEL`, `formatAge`, `iconElement`.
- Produces:
  - in `src/types.ts`: `ItemRepo`, `WorkItem`, `Started`, `WorkItemsState`, `StartCheck`, `RepoPick`, `WorkStarted` mirroring the Rust (camelCase).
  - in `src/workitems.ts`:
    ```ts
    export interface WorkFilters { source: string | null; repo: string | null; status: string | null; search: string }
    export function sessionFor(item: WorkItem, started: Started[], cards: Card[]): Card | null
    export function areas(state: WorkItemsState, cards: Card[]): { toPick: WorkItem[]; working: { item: WorkItem; card: Card }[] }
    export function applyWorkFilters(items: WorkItem[], f: WorkFilters): WorkItem[]
    export function renderItemCard(item: WorkItem, nowMs: number, card: Card | null): HTMLElement
    export function makeWorkItemsView(host: HTMLElement): { update(state: WorkItemsState, cards: Card[], nowMs: number): void }
    export type WorkItemAction = { kind: "open-item" | "start"; key: string } | { kind: "session"; key: string; pid: number } | { kind: "settings" };
    export function workItemActionFor(target: Element): WorkItemAction | null
    ```
    `sessionFor`: a card named `issue <id>`, or whose `cwd` is the started entry's `dir` or under it; the most recently changed (`stateSince`) wins.
    `areas`: `working` = items with a session (in `items` order); `toPick` = the rest. Search matches id, title and labels, case-insensitively. The filters are built once (as `makeReviewsView` does, so an open dropdown survives repaints) and apply to both areas.
    `renderItemCard` (`article.item`, `data-key`): head with the forge icon, the id as a link button (`data-action="open-item"`), the status as `span.item__status`, other labels as `span.pr__chip`; the title; then for a To pick item the meta ("Unassigned" or the assignees, " · ", age) or "No repo: Maya picks" when `repo` is null or `tracker`, and a `button.card__btn.card__btn--primary` "Start" (`data-action="start"`); for a working item the card's state (`STATE_LABEL`), its `snippet` in `.item__last`, a left border in the state's colour (`item--<state>`), and "Open session" (`data-action="session"`, `data-pid`).
  - `src/tabs.ts`: `Tab` gains `"issues"`, `PANES.issues = "issues"`.
  - `index.html`: after the Pull Requests tab, `<button class="tab" type="button" role="tab" data-tab="issues">Issues <span class="tab__count" id="issues-count"></span></button>`, and `<div id="issues" hidden></div>` after `#reviews`.
  - `src/main.ts`: `workItems` state, `paintWorkItems()` on the event, on card changes and on the 10 s timer; the count is `toPick.length`; clicks go through `workItemActionFor` (session → `focus(pid)`, open-item → `open_work_item`, start → Task 10's `startWorkItem`, settings → open Settings → Work Items).
  - Markup: `.issues__bar` (filters), `.issues__error` lines, `.issues__cols` holding `section.issues__area.issues__area--pick` then `section.issues__area.issues__area--working`, each with a head `h3` ("To pick" / "Being worked on") and `span.issues__count`, then its single column of cards.
  - `src/styles.css`: `.issues__cols` (two equal columns, To pick first), `.issues__area` and its head, `.item`, `.item__status` (gold tag), `.item__last`, `.item--working|awaiting|completed|idle` borders, as in `docs/superpowers/specs/2026-11-09-work-items/issues-tab.html`; under 760 px the columns stack.

- [ ] **Step 1: Write the failing tests** (in `src/workitems.test.ts`, with fixtures `item(overrides)` and `card(overrides)`)

```ts
it("puts an item with a session on the right and the rest on the left", () => {
  const a = item({ key: "github:a/b#1", id: "a/b#1" });
  const b = item({ key: "github:a/b#2", id: "a/b#2" });
  const s = card({ name: "issue a/b#1", state: "working" });
  const { toPick, working } = areas({ items: [a, b], started: [], errors: [], off: false, fetchedAt: 1 }, [s]);
  expect(toPick.map((i) => i.key)).toEqual(["github:a/b#2"]);
  expect(working).toEqual([{ item: a, card: s }]);
});

it("links a session by its folder when it was renamed", () => {
  const a = item({ key: "k", id: "a/b#1" });
  const started = [{ key: "k", item: a, dir: "/c/b-issue-1", startedAt: 0 }];
  expect(sessionFor(a, started, [card({ name: "x", cwd: "/c/b-issue-1" })])).not.toBeNull();
  expect(sessionFor(a, started, [card({ name: "x", cwd: "/c/b-issue-10" })])).toBeNull();
});

it("filters by source, repository, status and words", () => {
  const items = [item({ key: "1", source: "github", title: "Login times out", status: "todo", repo: { forge: "github", path: "acme/api" } }), item({ key: "2", source: "gitlab", title: "VAT", labels: ["backend"] })];
  expect(applyWorkFilters(items, { source: "gitlab", repo: null, status: null, search: "" }).map((i) => i.key)).toEqual(["2"]);
  expect(applyWorkFilters(items, { source: null, repo: "github:acme/api", status: "todo", search: "LOGIN" }).map((i) => i.key)).toEqual(["1"]);
  expect(applyWorkFilters(items, { source: null, repo: null, status: null, search: "backend" }).map((i) => i.key)).toEqual(["2"]);
});

it("shows Start on an item to pick and says when Maya must pick the repository", () => {
  const el = renderItemCard(item({ repo: null }), Date.now(), null);
  expect(el.querySelector("[data-action=start]")?.textContent).toBe("Start");
  expect(el.textContent).toContain("No repo: Maya picks");
});

it("shows Open session and the session's state on a working item", () => {
  const el = renderItemCard(item({}), Date.now(), card({ state: "awaiting", snippet: "Should the retry email them?", pid: 42 }));
  expect(el.querySelector("[data-action=session]")?.getAttribute("data-pid")).toBe("42");
  expect(el.classList.contains("item--awaiting")).toBe(true);
  expect(el.textContent).toContain("Should the retry email them?");
  expect(el.querySelector("[data-action=start]")).toBeNull();
});

it("keeps the filter dropdowns across repaints", () => {
  const host = document.createElement("div");
  const view = makeWorkItemsView(host);
  view.update({ items: [item({})], started: [], errors: [], off: false, fetchedAt: 1 }, [], Date.now());
  const sel = host.querySelector("select");
  view.update({ items: [item({}), item({ key: "2" })], started: [], errors: [], off: false, fetchedAt: 2 }, [], Date.now());
  expect(host.querySelector("select")).toBe(sel);
  expect(host.querySelector(".issues__area--pick .issues__count")?.textContent).toBe("2");
});
```

- [ ] **Step 2: Run them and see them fail**

Run: `pnpm test -- src/workitems.test.ts`
Expected: FAIL (module missing).

- [ ] **Step 3: Implement** the types, `workitems.ts`, the tab, the markup, the wiring and the styles. Repaint the areas with `morph` from `src/morph.ts`, not `replaceChildren` (CLAUDE.md lore: WKWebView drops `:hover` on a replaced node).

- [ ] **Step 4: Run all page tests and the type check**

Run: `pnpm test && pnpm exec tsc --noEmit`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/ index.html
git commit -m "feat(work-items): the Issues tab with To pick and Being worked on"
```

---

### Task 10: The Start dialog

**Files:**
- Modify: `src/workitems.ts`, `src/workitems.test.ts`, `src/main.ts`, `src/styles.css`

**Interfaces:**
- Consumes: `work_item_check`, `work_item_start` (Task 8); `#modal-host` and `.modal__panel--compact` (as `firstrun.ts` uses them); `showToast`, `openStarted` in `main.ts`.
- Produces:
  ```ts
  export function needsDialog(check: StartCheck): boolean   // takeover non-empty or pick present
  export function renderStartDialog(item: WorkItem, check: StartCheck, h: { onStart(repo: ItemRepo | null): void; onCancel(): void }): HTMLElement
  export async function startWorkItem(key: string, deps: { invoke: typeof invoke; host: HTMLElement; toast(s: string): void; opened(nativeTerm: number | null, dir: string): void }): Promise<void>
  ```
  `renderStartDialog`: title "Start \<id>: \<title>"; when `takeover` is non-empty, the line "This issue is already assigned to \<names joined by ', '>. Are you sure you want to take over?"; when `pick` is present, "Maya picked \<repo> — \<why>" (or "Maya could not tell which repository; choose one." when `picked` is null), a `select[name=repo]` of the options (labelled `<path> (GitHub|GitLab)`), preselected on the pick; the branch; buttons Cancel and the primary "Start session" ("Yes, take over and start" when there is a takeover), disabled while no repository is chosen.
  `startWorkItem`: `work_item_check`; with no dialog needed, `work_item_start` with `repo: null, takeOver: false`; otherwise mounts the dialog in `host` and on start calls `work_item_start` with the chosen repo and `takeOver: check.takeover.length > 0`; then toasts "Working on \<id> in \<folder>" and the notice when there is one, and calls `opened`. Errors toast. Escape and Cancel close the dialog and start nothing.

- [ ] **Step 1: Write the failing tests**

```ts
it("asks before taking over someone else's issue", () => {
  const el = renderStartDialog(item({ id: "a/b#1", title: "T" }), { takeover: ["maria", "joe"], pick: null, branch: "issue-a-b-1-t" }, { onStart: vi.fn(), onCancel: vi.fn() });
  expect(el.textContent).toContain("This issue is already assigned to maria, joe. Are you sure you want to take over?");
  expect(el.querySelector<HTMLButtonElement>(".card__btn--primary")?.textContent).toBe("Yes, take over and start");
});

it("shows Maya's pick preselected and the options to change it", () => {
  const api = { forge: "github", path: "acme/api" } as const;
  const web = { forge: "github", path: "acme/web" } as const;
  const on = { onStart: vi.fn(), onCancel: vi.fn() };
  const el = renderStartDialog(item({}), { takeover: [], pick: { picked: api, why: "The endpoint lives there.", options: [api, web] }, branch: "b" }, on);
  expect(el.querySelector<HTMLSelectElement>("select[name=repo]")?.value).toBe("github:acme/api");
  el.querySelector<HTMLButtonElement>(".card__btn--primary")!.click();
  expect(on.onStart).toHaveBeenCalledWith(api);
});

it("starts at once when nothing needs asking", async () => {
  const invoke = vi.fn().mockResolvedValueOnce({ takeover: [], pick: null, branch: "b" }).mockResolvedValueOnce({ dir: "/c/api-issue-1", nativeTerm: null, notice: null });
  const toast = vi.fn();
  await startWorkItem("k", { invoke, host: document.createElement("div"), toast, opened: vi.fn() });
  expect(invoke).toHaveBeenNthCalledWith(2, "work_item_start", { key: "k", repo: null, takeOver: false });
  expect(toast).toHaveBeenCalledWith(expect.stringContaining("api-issue-1"));
});

it("cancel starts nothing", async () => {
  const host = document.createElement("div");
  const invoke = vi.fn().mockResolvedValueOnce({ takeover: ["maria"], pick: null, branch: "b" });
  const done = startWorkItem("k", { invoke, host, toast: vi.fn(), opened: vi.fn() });
  await vi.waitFor(() => expect(host.querySelector("button")).not.toBeNull());
  [...host.querySelectorAll("button")].find((b) => b.textContent === "Cancel")!.click();
  await done;
  expect(invoke).toHaveBeenCalledTimes(1);
});
```

- [ ] **Step 2: Run them and see them fail**

Run: `pnpm test -- src/workitems.test.ts`
Expected: FAIL.

- [ ] **Step 3: Implement** the dialog and `startWorkItem`, and wire `main.ts`'s start action to it with `#modal-host`.

- [ ] **Step 4: Run all page tests and the type check**

Run: `pnpm test && pnpm exec tsc --noEmit`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/
git commit -m "feat(work-items): ask before a takeover and confirm Maya's repository pick"
```

---

### Task 11: Settings → Work Items

**Files:**
- Modify: `src/settings.ts`, `src/settings.test.ts`, `src/types.ts`, `src/styles.css`

**Interfaces:**
- Consumes: `ConfigJson` (gains `workItems`), the existing `set_config` save path and `OPEN_SETTINGS_GROUP`.
- Produces:
  - `SETTINGS_GROUPS` gains `["workItems", "Work Items"]` after `["reviews", "Pull requests"]`.
  - The group has one box per service ("GitHub", "GitLab"):
    - the on/off checkbox `name="wi-<forge>-enabled"`; when that service's Git connection is off, the box says "\<Label> is off in Settings → Git." with a link button that opens the Git group, and the rest of the box is disabled;
    - its rules, each a `.settings__rule` with: an Assignee `select` (Me / Someone / Anyone) and a login input shown for Someone; a Labels text input (comma-separated); a Repositories text input (comma-separated; blank = any repository); a remove button "✕"; then "+ Add rule";
    - "Status labels" (comma-separated, in order), "On Start, set status to" (`select`: "Leave as it is" + each status label), "Tracker-only repositories" (comma-separated);
    - the hint: GitHub "Start assigns the issue and sets its status: the token needs Issues: read and write (fine-grained) or repo (classic)." / GitLab "Start assigns the issue and sets its status: the token needs the api scope."
  - Below both: "Work prompt" textarea (`name="workPrompt"`), with the hint "Blank uses the built-in prompt. {id}, {title}, {url}, {description} and {repo} are filled in."
  - Comma lists are split, trimmed and emptied of blanks on save; the backend's `validate_work_items` error shows as the save error.

- [ ] **Step 1: Write the failing tests** (follow the existing `settings.test.ts` pattern for rendering a model and reading what `set_config` was called with)

```ts
it("lists Work Items after Pull requests", () => {
  expect(SETTINGS_GROUPS.map(([k]) => k).indexOf("workItems")).toBe(SETTINGS_GROUPS.map(([k]) => k).indexOf("reviews") + 1);
});

it("saves a rule typed as comma lists", async () => {
  // render with the default config; set GitHub's first rule to Anyone, labels "urgent, bug", repositories "acme/api";
  // set status labels "todo, in progress" and On Start "in progress"; save.
  // expect the saved config.workItems.github to equal:
  // { enabled: true, rules: [{ assignee: { kind: "anyone" }, labels: ["urgent", "bug"], repos: ["acme/api"] }],
  //   statusLabels: ["todo", "in progress"], startStatus: "in progress", trackerRepos: [] }
});

it("disables a service that is off in Settings → Git and links there", () => {
  // render with git: GitLab enabled false; expect the GitLab box's inputs disabled and its text to contain "GitLab is off in Settings → Git."
});

it("offers only the status labels for On Start", () => {
  // render with statusLabels ["todo", "doing"]; expect the On Start select's options to be ["Leave as it is", "todo", "doing"].
});
```

(Write these as full tests with the helpers `settings.test.ts` already has; the comments give the inputs and the exact expected values.)

- [ ] **Step 2: Run them and see them fail**

Run: `pnpm test -- src/settings.test.ts`
Expected: FAIL.

- [ ] **Step 3: Implement** the group in `settings.ts` and its styles, following the Git group's code.

- [ ] **Step 4: Run all page tests and the type check**

Run: `pnpm test && pnpm exec tsc --noEmit`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/
git commit -m "feat(work-items): Settings → Work Items"
```

---

### Task 12: Docs, version and release notes

**Files:**
- Modify: `README.md`, `RELEASE_NOTES.md`, the version files (through the script)

- [ ] **Step 1: README**

In "What it does", after the pull requests bullet, add: "Lists your GitHub and GitLab issues by the rules you set in Settings → Work Items (assignee, labels, repositories), and starts one with Start: Maya assigns it to you, sets its status label, and starts her agent on it in a fresh worktree of its repository". In Install → Requirements, extend the pull requests line: "Issues: the same tokens; Start needs write access (GitHub Issues: read and write, or repo; GitLab api)."

- [ ] **Step 2: Version and release notes**

Run: `git fetch origin && git show origin/main:package.json | grep '"version"'` — Expected: `"version": "0.50.0"` (if `main` moved, bump from its version instead: a `feat` bumps the minor).
Run: `sh scripts/set-version.sh 0.51.0`
Add at the top of `RELEASE_NOTES.md`:

```markdown
# 0.51.0

## New

- An Issues tab lists your GitHub and GitLab issues. Choose what shows in Settings → Work Items: rules by assignee, labels and repositories, and which labels are statuses.
- Start on an issue assigns it to you, sets its status, and starts Maya's agent on it in a fresh worktree of its repository. If someone else has the issue, Maya asks first. If the issue does not say which repository holds the code, Maya picks one and you confirm it.
- Issues being worked on move to their own column, with their session's state and a button to open it.
```

- [ ] **Step 3: Check everything**

Run: `cargo test --workspace && pnpm test && pnpm exec tsc --noEmit`
Expected: all PASS.

- [ ] **Step 4: Commit**

```bash
git add README.md RELEASE_NOTES.md package.json Cargo.toml Cargo.lock src-tauri/tauri.conf.json mobile/tauri.conf.json
git commit -m "chore(release): 0.51.0"
```

(`git status` after the script shows exactly which version files it touched; add those.)
