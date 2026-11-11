# Work Items: Custom Tools Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the user add any project management tool to the Issues tab as a script: a list command whose JSON becomes work items, and an optional on-start command that Start runs with the item on stdin.

**Architecture:** A new `core/src/work_items/custom.rs` runs a tool's command through the user's shell with a time limit and reads its `{"items": [...]}` into the existing `WorkItem` shape. `work_items::fetch` asks the custom tools beside GitHub and GitLab, so the poll, the started list, the repo pick and Start treat their items like issues. Start works the same, except that the write-back is the on-start command, and the worktree folder is named after the item's id when it has no number.

**Tech Stack:** Rust (maya-core, Tauri app), vanilla TypeScript with vitest.

**Spec:** `docs/superpowers/specs/2026-11-09-work-items-design.md`, sections "Custom tools", "Sources" (the `repo` field), "The Issues tab", "Start". Part 1 (GitHub and GitLab) is `docs/superpowers/plans/2026-11-09-work-items-core.md`, shipped in 0.51.0. The phone and the voice are not in this plan.

## Global Constraints

- The list command runs on every refresh (every 120 s, and when Settings → Work Items or Settings → Git change), through the user's shell, with a time limit of **30 s**. Maya passes it nothing.
- Output: `{ "items": [ { "id", "title", "url", "description", "status", "labels", "assignees", "author", "updatedAt", "repo" } ] }`. `id`, `title` and `url` are required; an item missing one is skipped; unknown fields are ignored.
- A non-zero exit, a timeout or bad JSON makes that tool's error (its exit status and the first lines of its stderr) show at the top of the tab; the other sources still show, and the tool keeps its last items, as a failing GitHub does.
- `repo` is `owner/name`, `github:owner/name`, `gitlab:group/name`, or a clone URL; may be missing. A bare `owner/name` is on GitHub when GitHub is switched on in Settings → Git, else on GitLab. No repo, or one Maya cannot place, goes through the repo pick.
- The on-start command, when set, runs on Start with the item's JSON on stdin. Its failure is a notice; it never stops the session from starting.
- A token must never appear in the command's environment: Maya adds no variable to it. (It gets the user's own environment as their shell sets it.)
- A command runs only as the user typed it in Settings: never a command from a network peer, the phone, or an item's data. Item data reaches the on-start command only on stdin, never in its command line.
- A started custom item that the list no longer returns shows its last known copy until its session is removed.
- Settings shows the format and a short example script by the command box ("Show the output format and an example script").
- Copy is plain English, as in the rest of Maya. The version bump is `feat`: minor, from `main`'s version (0.52.2 → **0.53.0** unless `main` moved).

**Builder's calls made here (confirm before Task 1):**
1. **"Your name there".** The spec asks the takeover question for a custom item from its `assignees`, but Maya cannot know the user's name in another tool. Each tool gets an optional **Your name there** field. When it is set, Start asks about taking over an item assigned to anyone else. When it is blank, Start asks nothing for that tool.
2. **Interactive login shell.** The command runs as `$SHELL -lic '<command>'` (falling back to `launch::LOGIN_SHELL`), so variables set in `~/.zshrc` (an API key) are there, as `forge::env` already does for tokens. Whatever the rc files print before the JSON is skipped. On Windows it runs as `cmd.exe /C <command>`.
3. **No save is refused.** **Add New** saves a tool with a blank name and blank commands. A tool whose list command is blank is not run and not an error. A blank name shows as "Custom tool".
4. **Limits.** At most 4 MB of output, 500 items and 20,000 characters of description per item are read. More output than that is an error, and the rest is cut.

## Review Focus

1. **A shell that prints before the JSON.** An rc file prints "Restored session" or a banner on stdout. The list must still parse (Task 2: `noise_before_the_json_is_skipped`).
2. **A command that hangs, or leaves a child behind.** `sleep 100 &` in the script must not keep the poll waiting past 30 s. Task 2 kills the process group (`a_timeout_kills_the_command_and_its_children`).
3. **An item whose data looks like shell.** An id of `$(touch /tmp/x)` must reach the on-start command only as stdin JSON, and nothing is run from it (Task 2: `item_data_reaches_the_command_only_on_stdin`).
4. **Two tools with the same item id.** The same Linear added twice must show both items under their own keys, not merge them (Task 4: `two_tools_with_the_same_ids_keep_their_own_items`).
5. **A tool removed while its item is being worked on.** The started item keeps showing, and Start of a stale copy runs no command (Task 5: `an_item_of_a_removed_tool_runs_no_command`).

---

### Task 1: The custom tools in the settings file

**Files:**
- Modify: `core/src/config.rs` (beside `ForgeItems` ~line 401, `WorkItemsConfig` ~428, `normalize_work_items` ~729; tests ~834)
- Modify: `src/types.ts` (beside `WorkItemsConfig` ~431)
- Modify: `src/settings.ts` (`workItemsConfig` ~331)
- Test: `core/src/config.rs` tests, `src/settings.test.ts`

**Interfaces:**
- Produces (Rust, `maya_core::config`):
  ```rust
  #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
  #[serde(rename_all = "camelCase")]
  pub struct CustomTool {
      /// `custom-<n>`: the items' `source` and the first part of their key.
      #[serde(default)] pub id: String,
      #[serde(default)] pub name: String,
      #[serde(default)] pub enabled: bool,
      #[serde(default)] pub list_command: String,
      /// Blank: Start runs nothing.
      #[serde(default)] pub start_command: String,
      /// The user's name in this tool; blank asks no takeover question.
      #[serde(default)] pub me: String,
  }
  // WorkItemsConfig gains: #[serde(default)] pub custom: Vec<CustomTool>,
  impl WorkItemsConfig { pub fn tool(&self, source: &str) -> Option<&CustomTool> }
  impl CustomTool { pub fn label(&self) -> &str } // the name, or "Custom tool" when blank
  ```
- Produces (TS): `interface CustomTool { id: string; name: string; enabled: boolean; listCommand: string; startCommand: string; me: string }`, and `WorkItemsConfig.custom: CustomTool[]`. `workItemsConfig(raw)` defaults `custom` to `[]`.

- [ ] **Step 1: Write the failing tests** in `core/src/config.rs`:
  - `custom_tools_default_to_none_and_round_trip_in_camel_case`: `Config::default().work_items.custom` is empty. A file from before 0.53 (no `custom`) loads as empty. A tool serialises with the keys `listCommand`, `startCommand`, `me`.
  - `normalize_gives_each_tool_a_unique_id_and_trims`: tools with ids `["", "custom-2", "custom-2", "Bad Id!"]` become `["custom-1", "custom-2", "custom-3", "custom-4"]`. A valid id is kept, a duplicate or invalid one is replaced by the lowest free `custom-<n>`, and `name`, both commands and `me` are trimmed. A valid id is `custom-` followed by digits.
  - `a_tool_is_found_by_its_id_and_labelled_by_its_name`: `tool("custom-1")` finds it, `tool("github")` is None, and `label()` of a blank name is `"Custom tool"`.

  And in `src/settings.test.ts`: `workItemsConfig(null).custom` equals `[]`, and `workItemsConfig({ custom: [t] }).custom` equals `[t]`.

- [ ] **Step 2: Run them to see them fail**
  Run: `cargo test -p maya-core config::tests::custom config::tests::normalize_gives config::tests::a_tool_is && pnpm test src/settings.test.ts`
  Expected: compile errors (`CustomTool` missing) and a failing vitest.

- [ ] **Step 3: Implement** `CustomTool`, `WorkItemsConfig.custom`, `tool`, `label`, and the custom part of `normalize_work_items` (ids, then trimming). Add the TS types and the default.

- [ ] **Step 4: Run the tests to see them pass.** Same command. Expected: PASS.

- [ ] **Step 5: Commit** `feat(work-items): custom tools in the settings file`

---

### Task 2: Running a tool's command

**Files:**
- Create: `core/src/work_items/custom.rs` (register `pub mod custom;` in `core/src/work_items/mod.rs`)
- Test: the same file

**Interfaces:**
- Produces:
  ```rust
  pub const TIMEOUT: Duration = Duration::from_secs(30);
  pub const MAX_OUTPUT: usize = 4 * 1024 * 1024;
  /// The argv that runs `command` through the user's shell: `[$SHELL or LOGIN_SHELL, "-lic", command]`; Windows `["cmd.exe", "/C", command]`.
  pub fn shell_argv(command: &str, shell: Option<&str>) -> Vec<String>
  /// Runs `argv` with `stdin` (None: closed) and returns its stdout, or the error line.
  pub fn run(argv: &[String], stdin: Option<&str>, timeout: Duration) -> Result<String, String>
  ```
  `shell_argv` uses `shell` only when it is an absolute path. On Windows it passes the command with `raw_arg`, as `win_console.rs` does. The caller passes `std::env::var("SHELL").ok().as_deref()`.

  `run` is built with `crate::command`, then `launch::scrub_appimage_env`. On unix it runs in its own process group (`process_group(0)`), and a timeout kills the whole group (`libc::kill(-pid, SIGKILL)`). Stdout and stderr are read on threads of their own, as `collect::run_with_timeout` does. It sets no environment variable.

  Its error lines are exact copy:
  - `the command exited with status {code}: “{stderr}”`, where `{stderr}` is the first 3 non-blank stderr lines joined with ` / `, each cut to 200 characters. With no stderr it is `the command exited with status {code}`.
  - `the command was stopped by a signal`
  - `the command did not finish within 30 s`
  - `the command could not run: {io error}`
  - `the command printed more than 4 MB`

- [ ] **Step 1: Write the failing tests** (all `#[cfg(unix)]` except the first, and they use `["sh", "-c", ...]` argv so the user's rc files never load):
  - `the_shell_runs_the_command_as_typed`: `shell_argv("linear-items --mine", Some("/bin/zsh")) == ["/bin/zsh", "-lic", "linear-items --mine"]`. `Some("zsh")` (not absolute) and `None` give `[LOGIN_SHELL, "-lic", ...]`.
  - `stdout_is_the_answer`: `run(sh("echo '{\"items\":[]}'"), None, TIMEOUT) == Ok("{\"items\":[]}\n")`.
  - `a_failure_says_its_status_and_first_stderr_lines`: `sh("echo one >&2; echo two >&2; echo three >&2; echo four >&2; exit 3")` gives `Err("the command exited with status 3: “one / two / three”")`.
  - `a_timeout_kills_the_command_and_its_children`: `sh("sleep 30 & sleep 30")` with a 300 ms timeout returns `Err("the command did not finish within 30 s")` within 2 s. The message names `TIMEOUT`, not the test's value: format it from a `const`.
  - `too_much_output_is_an_error`: `sh("head -c 5000000 /dev/zero")` gives `Err("the command printed more than 4 MB")`.
  - `item_data_reaches_the_command_only_on_stdin`: in a temp dir, `run(sh("cat > got.json"), Some(r#"{"id":"$(touch pwned)"}"#), TIMEOUT)` succeeds. `got.json` holds that exact text, and `pwned` does not exist.

- [ ] **Step 2: Run them to see them fail**
  Run: `cargo test -p maya-core work_items::custom`
  Expected: compile error (module missing).

- [ ] **Step 3: Implement `shell_argv` and `run`.**

- [ ] **Step 4: Run the tests to see them pass.** Expected: PASS, the timeout test under 2 s.

- [ ] **Step 5: Commit** `feat(work-items): run a custom tool's command with a time limit`

---

### Task 3: Reading a tool's items

**Files:**
- Modify: `core/src/work_items/custom.rs`
- Test: the same file

**Interfaces:**
- Consumes: `CustomTool` (Task 1), `run`/`shell_argv` (Task 2), `crate::forge::{route, ForgeSite}`, `crate::attachments::is_web_url`, `WorkItem`/`ItemRepo` (`work_items/mod.rs`).
- Produces:
  ```rust
  pub const MAX_ITEMS: usize = 500;
  pub const MAX_DESCRIPTION: usize = 20_000;
  /// The items of a list command's stdout; rc-file noise before the JSON is skipped.
  pub fn parse_list(stdout: &str, tool: &CustomTool, sites: &[ForgeSite]) -> Result<Vec<WorkItem>, String>
  /// Where `repo` (as the script spelled it) lives, among the switched-on sites; None when Maya cannot place it.
  pub fn repo_of(repo: &str, sites: &[ForgeSite]) -> Option<ItemRepo>
  /// The item as the on-start command reads it: the list format, `repo` as `github:<path>` / `gitlab:<path>`.
  pub fn item_json(item: &WorkItem) -> String
  /// Runs the tool's list command through `argv_of(command)`; a blank command is Ok(empty).
  pub fn list_with(tool: &CustomTool, sites: &[ForgeSite], argv_of: &dyn Fn(&str) -> Vec<String>) -> Result<Vec<WorkItem>, String>
  /// `list_with` through the user's shell (`shell_argv` with `$SHELL`).
  pub fn list(tool: &CustomTool, sites: &[ForgeSite]) -> Result<Vec<WorkItem>, String>
  /// Runs the tool's on-start command with `item_json(item)` on stdin; a blank one does nothing.
  pub fn on_start(tool: &CustomTool, item: &WorkItem) -> Result<(), String>
  ```
  The item mapping is `key = "{tool.id}:{id}"`, `source = tool.id`, `number = None`, `tracker = false`, and `author` defaults to `""`. `labels` and `assignees` keep only string entries. `description` is cut to `MAX_DESCRIPTION` characters. An item whose `url` is not a web address counts as missing it. Items past `MAX_ITEMS` are dropped. `id`, `title` and `url` are trimmed, and a blank one counts as missing.

  To find the JSON: first the whole trimmed stdout, then, from each line that starts with `{`, the first JSON value from there (`serde_json::Deserializer::from_str(..).into_iter::<Value>()`). The first value with an `items` array wins. Otherwise the error is the exact copy `the command did not print {"items": [...]}`.

- [ ] **Step 1: Write the failing tests:**
  - `an_item_needs_an_id_a_title_and_a_web_url`: from the spec's example JSON plus items missing `id`, with a blank `title`, and with `url: "file:///etc/passwd"`, only `ENG-1204` comes back. It has `key == "custom-1:ENG-1204"`, `source == "custom-1"`, `status == Some("Todo")`, `labels == ["frontend"]`, `assignees == ["tiago"]`, `updated_at == "2026-11-06T10:00:00Z"`, and `number == None`.
  - `noise_before_the_json_is_skipped`: `"Restored session\n{\"items\":[...]}\nSaving session...\n"` parses.
  - `bad_output_is_an_error`: `"not json"`, `"{}"` and `"{\"items\":3}"` each give `Err("the command did not print {\"items\": [...]}")`.
  - `unknown_fields_and_odd_types_are_ignored`: `labels: ["a", 3, null]` gives `["a"]`, an extra `"priority": 1` is fine, and `"assignees": "tiago"` (not a list) gives `[]`.
  - `the_repo_is_placed_by_its_spelling`, with a GitHub site and a GitLab site at `https://gitlab.acme.io`:
    - `"acme/web"` gives GitHub `acme/web`.
    - `"github:acme/web"` gives GitHub, and `"gitlab:grp/sub/r"` gives GitLab `grp/sub/r`.
    - `"git@gitlab.acme.io:grp/r.git"` and `"https://github.com/acme/web.git"` are placed by `route`.
    - `"https://bitbucket.org/x/y"` and `"gitlab:g/r"` with no GitLab site give None.
    - With only the GitLab site on, `"acme/web"` gives GitLab.
  - `the_start_command_reads_the_item_as_listed`: `item_json` of a parsed item has the same `id`, `title`, `url`, `labels` and `assignees`, with `repo: "github:acme/web"`. An item with no repo has no `repo` key.
  - `#[cfg(unix)] a_blank_command_lists_nothing_and_starts_nothing`: `list` with `list_command: ""` is `Ok(vec![])`, and `on_start` with `start_command: ""` is `Ok(())`. Neither runs anything.

- [ ] **Step 2: Run them to see them fail.** `cargo test -p maya-core work_items::custom`. Expected: compile errors.

- [ ] **Step 3: Implement** `parse_list`, `repo_of`, `item_json`, `list` and `on_start`. `list` and `on_start` call `run(&shell_argv(cmd, std::env::var("SHELL").ok().as_deref()), ..., TIMEOUT)`.

- [ ] **Step 4: Run the tests to see them pass.**

- [ ] **Step 5: Commit** `feat(work-items): read a custom tool's items`

---

### Task 4: The custom tools in the Issues poll

**Files:**
- Modify: `core/src/work_items/mod.rs` (`Fetched`, `fetch`, `combine`, a new `SourceError`)
- Modify: `src-tauri/src/work_items_app.rs` (`WorkItemsState`, `poll_work_items`, the notice push in `work_item_start`)
- Modify: `src/types.ts` (`WorkItemsState.errors`, `sourceLabels`)
- Test: `core/src/work_items/mod.rs` tests

**Interfaces:**
- Consumes: `custom::list` (Task 3), `WorkItemsConfig.custom`, `CustomTool::label` (Task 1).
- Produces:
  ```rust
  /// One line above the Issues tab's areas.
  #[derive(Serialize, Clone, Debug, PartialEq, Eq)] #[serde(rename_all = "camelCase")]
  pub struct SourceError { pub source: String, pub message: String }
  pub struct Fetched { pub source: String, pub label: String, pub result: Result<Vec<WorkItem>, String> }
  /// The app passes `&custom::list` as `list_tool`; tests pass `list_with` over `sh -c`.
  pub fn fetch(forges: &[Forge], cfg: &WorkItemsConfig, http: &dyn Http, sites: &[ForgeSite], list_tool: &(dyn Fn(&CustomTool, &[ForgeSite]) -> Result<Vec<WorkItem>, String> + Sync)) -> Vec<Fetched>
  pub fn combine(previous: &[WorkItem], fetched: Vec<Fetched>, errors: Vec<SourceError>) -> (Vec<WorkItem>, Vec<SourceError>)
  ```
  `fetch` asks each switched-on forge (as now, `source = source_of(..)`, `label = provider.label()`) and each enabled tool with a non-blank list command (`source = tool.id`, `label = tool.label()`), all at once on scoped threads. `combine`'s failure line is `"{label} issues check failed: {e}"` for `github`/`gitlab`, as now, and `"{label}: {e}"` for a tool (the mockup's "Linear (mine-only): the command exited with status 1: “LINEAR_API_KEY is not set”").

  `WorkItemsState` changes:
  - `errors: Vec<SourceError>`. The poll turns each `ForgeError` from `git::forges` into `SourceError { source: source_of(forge), message }`.
  - New `source_labels: BTreeMap<String, String>`, holding each configured tool's id and label.
  - `off` is true only when no forge was asked, none is missing a token, and no tool was asked.

  TS: `errors: { source: string; message: string }[]` and `sourceLabels: Record<string, string>`.

- [ ] **Step 1: Write the failing tests** in `work_items/mod.rs`:
  - Update `a_failed_service_keeps_its_last_items_and_says_so` to the new `Fetched`.
  - `a_failed_tool_keeps_its_last_items_and_names_itself`: a previous `custom-1` item stays when its `Fetched` fails with `"the command exited with status 1"`, and the line is `"Linear (mine-only): the command exited with status 1"`.
  - `two_tools_with_the_same_ids_keep_their_own_items`: `custom-1:ENG-1` and `custom-2:ENG-1` both survive `merge`.
  - `#[cfg(unix)] fetch_asks_the_enabled_tools_with_a_command`: tools `custom-1` (`echo '{"items":[{"id":"A-1","title":"t","url":"https://x.test/1"}]}'`), `custom-2` disabled, and `custom-3` enabled with a blank command give exactly one `Fetched`, for `custom-1`, holding `A-1`. There are no forges, so pass a `FakeHttp` with no routes. `list_tool` is `&|t, s| custom::list_with(t, s, &|c| vec!["sh".into(), "-c".into(), c.into()])`, so the user's rc files never load in a test.

- [ ] **Step 2: Run them to see them fail.** `cargo test -p maya-core work_items::tests`.

- [ ] **Step 3: Implement** the core changes, then `poll_work_items`: pass `forge::sites(&conns)` and fill `source_labels`. Log a failing tool as `issues: {label}: {e}`. Change `work_item_start`'s notice push to `SourceError { source: item.source.clone(), .. }`, and update `WorkItemsState`'s TS type.

- [ ] **Step 4: Run the tests to see them pass**, then `cargo build -p maya` and `pnpm test src/workitems.test.ts` (its fixture's `errors` entries change to `{ source, message }`).

- [ ] **Step 5: Commit** `feat(work-items): list the custom tools' items on the Issues tab`

---

### Task 5: Start on a custom item

**Files:**
- Modify: `core/src/work_items/start.rs` (`resolve_target`, a new `item_tag`)
- Modify: `core/src/actions.rs` (`work_item_plan` ~911)
- Modify: `src-tauri/src/work_items_app.rs` (`work_item_check`, `work_item_start`)
- Test: `core/src/work_items/start.rs`, `core/src/actions.rs`, `src-tauri/src/work_items_app.rs` tests

**Interfaces:**
- Consumes: `custom::on_start` (Task 3), `WorkItemsConfig::tool` and `CustomTool.me` (Task 1).
- Produces:
  ```rust
  /// The folder's and branch's tag for an item: its number, else its id as a slug (`worktree::branch_slug(None, id)`).
  pub fn item_tag(item: &WorkItem) -> String                          // start.rs
  pub fn resolve_target(clones_root: &Path, site: &ForgeSite, repo: &str, tag: &str, branch: &str) -> Result<WorkTarget, String>  // `number: u64` becomes `tag: &str`; dir `<base>-issue-<tag>`
  /// The takeover names for a custom item: its assignees other than `tool.me`; none when `me` is blank.
  fn custom_takeover(tool: &CustomTool, assignees: &[String]) -> Vec<String>   // work_items_app.rs
  ```
  `branch_name` uses `item_tag` for its head, so its behaviour stays the same. `work_item_plan` uses `item_tag(item)` and loses the "has no number" refusal.

  A custom item's `work_item_check` returns `takeover = custom_takeover(tool, &item.assignees)`. `work_item_start` for an item whose `source` is a configured tool runs these steps in this order:
  1. `takeover_refusal(&take_over, &item.assignees, &tool.me)`, only when `me` is set.
  2. `actions::work_item_preflight`.
  3. `custom::on_start(tool, &item)`, only when the tool is enabled. Its failure adds the notice `Started, but the on-start command for {id} failed: {e}` and logs `issues: on-start for {id}: {e}`.
  4. `actions::start_work_item`.

  An item whose tool is no longer configured starts with no command run. It is never looked up by name, only by `source`.

- [ ] **Step 1: Write the failing tests:**
  - `start.rs`:
    - `an_item_without_a_number_is_tagged_by_its_id`: `item_tag` of number 412 is `"412"`, and of id `ENG-1204` with no number it is `"eng-1204"`.
    - `branch_name` of the latter starts with `issue-eng-1204-`.
    - `resolve_target(.., "eng-1204", ..)` gives the dir `<clones>/<base>-issue-eng-1204`.
  - `actions.rs`: `start_work_item_opens_a_custom_item_by_its_id`, the custom twin of `start_work_item_opens_a_worktree_on_the_items_branch_and_names_the_session`, with `number: None, id: "ENG-1204"`. The folder ends in `-issue-eng-1204` and the session is named `issue ENG-1204`.
  - `work_items_app.rs`:
    - `a_custom_takeover_asks_only_when_my_name_is_set`: with `me` blank, `["maria"]` gives `[]`. With `me: "Tiago"`, `["tiago", "maria"]` gives `["maria"]`.
    - `an_item_of_a_removed_tool_runs_no_command`: test the lookup that `work_item_start` uses, `config.work_items.tool(&item.source)`, so that an item whose source is `custom-9` with no `custom-9` tool gets None, and a tool is never matched by name.

- [ ] **Step 2: Run them to see them fail.** `cargo test -p maya-core work_items::start actions:: && cargo test -p maya work_items_app`.

- [ ] **Step 3: Implement.** Update the callers of `resolve_target`, including the existing tests that pass a number, which now pass `"412"`.

- [ ] **Step 4: Run the whole Rust suite.** `cargo test --workspace`. Expected: PASS.

- [ ] **Step 5: Commit** `feat(work-items): start a custom tool's item`

---

### Task 6: Custom tools in Settings → Work Items

**Files:**
- Modify: `src/settings.ts` (after the GitHub/GitLab boxes ~1275, a handler beside `onWorkItems` ~224 and ~2126)
- Modify: `src/styles.css` only if the existing `.settings__git` box styles do not fit
- Test: `src/settings.test.ts`

**Interfaces:**
- Consumes: `CustomTool`, `WorkItemsConfig.custom` (Task 1).
- Produces: handler `onCustomTools?(tools: CustomTool[]): void`, wired as `saveWorkItems((w) => { w.custom = tools; })`.

  One `.settings__git.settings__custom` box per tool with `data-tool=<id>`, holding:
  - an on/off check (`name=ct-<i>-enabled`), and a name input (`ct-<i>-name`, placeholder "Linear (mine-only)");
  - a ✕ remove button (`data-action=ct-remove`);
  - "List command" (`ct-<i>-list`), "On-start command (optional)" (`ct-<i>-start`), and "Your name there (optional)" (`ct-<i>-me`) with the hint "Start asks before taking over an item assigned to someone else. Blank: it never asks.";
  - a `<details>` whose summary is "Show the output format and an example script", holding the hints and the example below.

  Then a "+ Add New" button (`data-action=ct-add`) that appends `{ id: "", name: "", enabled: true, listCommand: "", startCommand: "", me: "" }`. Every change sends the whole list through `onCustomTools`. Inputs save on `change`, as the work prompt does, never per keystroke.

  The `<details>` text is exact copy:
  - Hint: "The list command runs every 2 minutes through your shell and must finish within 30 seconds. It prints JSON like the example; id, title and url are required. repo is owner/name, github:owner/name, gitlab:group/name or a clone URL; without one, Maya picks the repository when you start."
  - Hint: "The on-start command runs when you press Start, with the item's JSON on its input, to assign or move it in your tool."
  - `<pre>` example:
    ```
    {"items": [{"id": "ENG-1204", "title": "Export reports as CSV",
      "url": "https://linear.app/acme/issue/ENG-1204", "status": "Todo",
      "labels": ["frontend"], "assignees": ["tiago"], "repo": "acme/web"}]}
    ```
  - `<pre>` example script:
    ```
    #!/bin/sh
    # My open tickets as Maya's work items (needs curl and jq).
    curl -s -H "Authorization: Bearer $TRACKER_TOKEN" https://tracker.example.com/api/my-tickets |
      jq '{items: [.[] | {id: .key, title: .summary, url: .link, status: .state, repo: .repository}]}'
    ```

- [ ] **Step 1: Write the failing tests** in `src/settings.test.ts`:
  - `"shows each custom tool with its commands and an example"`: given two tools, there are two `.settings__custom` boxes, the inputs hold their values, and the `<details>` summary text matches.
  - `"adds, edits and removes custom tools"`:
    - clicking `ct-add` calls `onCustomTools` with the old list plus the blank tool;
    - changing `ct-0-list` to `" linear-items "` calls it with `listCommand: "linear-items"` (trimmed in the page too);
    - clicking ✕ on the first calls it with the second alone.

- [ ] **Step 2: Run them to see them fail.** `pnpm test src/settings.test.ts`.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run them to see them pass**, then `pnpm test` (the whole page suite).

- [ ] **Step 5: Commit** `feat(work-items): add custom tools in Settings → Work Items`

---

### Task 7: Custom items on the Issues tab, docs and release

**Files:**
- Modify: `src/workitems.ts` (source label ~84–92 and ~213, off copy ~191–195)
- Modify: `README.md` (the Issues line ~33), `RELEASE_NOTES.md`, version files via `scripts/set-version.sh`
- Test: `src/workitems.test.ts`

**Interfaces:**
- Consumes: `WorkItemsState.sourceLabels`, `errors` (Task 4).
- Produces: a custom item's card shows `state.sourceLabels[item.source] ?? "Custom tool"` in `item__src`. The Source filter lists it by the same label.

  The off message, exact copy:
  - with `gitOff`: "Issues are off. Turn on GitHub or GitLab in Settings → Git, or add a custom tool in Settings → Work Items."
  - otherwise: "Issues are off. Turn on GitHub, GitLab or a custom tool in Settings → Work Items."

  The off link opens Settings → Work Items when `gitOff` is false, as now.

- [ ] **Step 1: Write the failing tests** in `src/workitems.test.ts`:
  - `"shows a custom tool's item by the tool's name"`: an item with `source: "custom-1"` and `sourceLabels: { "custom-1": "Linear" }` renders "Linear", and the Source filter offers "Linear".
  - Update the two "Issues are off" tests to the new copy.
  - `"shows a tool's error above the areas"`: `errors: [{ source: "custom-1", message: "Linear: the command exited with status 1" }]` renders that line.

- [ ] **Step 2: Run them to see them fail.** `pnpm test src/workitems.test.ts`.

- [ ] **Step 3: Implement**, then the docs:
  - README: extend the Issues bullet with "…and items from any other tool through a script of your own (Settings → Work Items → Add New)".
  - Bump: `sh scripts/set-version.sh 0.53.0` (from `main`'s version).
  - Add a `# 0.53.0` / `## New` section at the top of `RELEASE_NOTES.md`, in plain words: the Issues tab takes items from any project management tool through a script that prints them as JSON; Start can run a second script to assign or move the item there; Maya picks the repository when the item names none.
  - Commit the version and notes on their own as `chore(release): 0.53.0`.

- [ ] **Step 4: Run everything.** `pnpm test && cargo test --workspace`. Expected: PASS.

- [ ] **Step 5: Commit** `feat(work-items): show custom tools' items by name` (before the release commit).

---

## Manual checks before the PR

- [ ] Add a tool whose list command is a real script (e.g. `printf` of the spec's example). Its item shows with the tool's name, and Start opens a worktree `<repo>-issue-eng-1204` on `issue-eng-1204-…`.
- [ ] Set an on-start command `cat > /tmp/maya-start.json`, then Start an item: the file holds the item's JSON.
- [ ] Make the list command `exit 1` with a stderr line: the error shows above the areas, and GitHub's issues still show.
- [ ] With a `~/.zshrc` that exports `TRACKER_TOKEN`, a script that uses it works from Maya launched from the Dock.
