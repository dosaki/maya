# Work Items tab name and Coral source Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename the "Issues" tab to "Work Items", and add Coral Boards as a built-in Work Items source whose work items and defects list and Start like GitHub/GitLab issues.

**Architecture:** A new `core/src/work_items/coral.rs` talks to Coral's REST API through the existing `forge::Http` (which learns to send a Coral token as `x-api-token`). `WorkItemsConfig` gains a `coral` section; the desktop poll and Start in `src-tauri/src/work_items_app.rs` get a Coral branch beside the forges'; Settings → Work Items gets a Coral source card.

**Tech Stack:** Rust (maya-core, Tauri 2), TypeScript (vanilla DOM, vitest), ureq 3.

**Spec:** `docs/superpowers/specs/2026-11-12-coral-work-items-design.md`

## Global Constraints

- Version 0.55.0 via `sh scripts/set-version.sh 0.55.0`, committed alone with the release notes as `chore(release): 0.55.0` (see `CLAUDE.md`).
- The internal tab id `issues`, the `issues-count` element, session names `issue <id>` and branch prefix `issue-` do not change.
- The Coral token never reaches `config.json`, a log line or a message: scrub it with `forge::scrub` in every error.
- No real Coral address anywhere in code, tests or docs: tests use `https://coral.test`.
- Projects, phases and subtasks never show: only `itemType=work_item` and `itemType=defect` are asked for.
- Coral source id is `"coral"`, label `"Coral"`; item key `coral:<item UUID>`; item id the `displayCode`.
- Repaint with `morph`/`keepEqualChildren` from `src/morph.ts`, never `replaceChildren`, for anything that repaints while the pointer may be on it (CLAUDE.md lore).
- Settings UI uses the shared builders in `src/settings-ui.ts` (`row`, `group`, `footRow`, …) and the vertical layout.
- Run `npm test` (vitest), `npx tsc --noEmit`, and `cargo test -p maya-core` plus `cargo test -p maya` (src-tauri) before each commit that touches them.

## Review Focus

1. **A Coral address typed with a trailing `/` or a path** (`https://coral.test/`): every URL must still be `https://coral.test/api/v1/...`. Test in Task 2 (`base_is_trimmed`).
2. **An item with no assignee, no description, or a tag without a scope**: it still lists. Test in Task 3 (`parse_item_tolerates_missing_fields`).
3. **A product with a GitLab repository, or a repository URL with `.git` or a trailing slash**: it routes to the right forge path. Test in Task 4 (`product_repos_route_to_known_forges`).
4. **More than 500 open items across rules**: listing stops at 500, no endless cursor loop on a repeated `nextCursor`. Test in Task 3 (`list_stops_at_the_cap_and_on_a_repeated_cursor`).
5. **The token pasted with spaces or a newline**: it is trimmed before use and storage. Test in Task 2 (`resolve_trims_and_refuses_blank`).

---

### Task 1: The tab is "Work Items"

**Files:**
- Modify: `index.html:20` (tab button text)
- Modify: `src/workitems.ts:198-199` (off text), comments naming "the Issues pane/tab"
- Modify: `src/settings.ts:398,471,1291,1403,1427` (help lines naming the tab; "Issues to pick up" → "Work items to pick up")
- Modify: `src/tabs.ts:21`, `src/types.ts:352,380,422` (comments)
- Modify: `README.md` (the Work Items lines)
- Modify: `core/src/config.rs:224,453`, `src-tauri/src/work_items_app.rs:1`, `src-tauri/src/lib.rs:161` (comments)
- Test: `src/workitems.test.ts`, `src/settings.test.ts`

**Interfaces:** none new.

- [ ] **Step 1: Update the tests that pin the old words.** In `src/workitems.test.ts` and `src/settings.test.ts`, change expectations for the off text to `"Work items are off. …"` and the Settings help to `"Where the Work Items tab finds your work, and what Start does with an item."`. Add `it("the tab button says Work Items")` reading `index.html` (as other tests read files, or via `fs.readFileSync`) and asserting the `data-tab="issues"` button's text starts with `"Work Items"`.
- [ ] **Step 2: Run `npm test`.** Expected: those tests FAIL.
- [ ] **Step 3: Change the text.** Every place a person reads "Issues" naming the tab becomes "Work Items"; "issues" meaning the pane's contents in general becomes "work items" (forge-specific lines such as "GitHub Issues: read and write" keep "Issues", it is GitHub's permission name). Comments that name the tab change too. Off texts:
  - `"Work items are off. Turn on GitHub or GitLab in Settings → Git, or add a custom tool in Settings → Work Items. "`
  - `"Work items are off. Turn on GitHub, GitLab or a custom tool in Settings → Work Items. "`
- [ ] **Step 4: Run `npm test` and `npx tsc --noEmit`.** Expected: PASS. `grep -rn "Issues tab\|>Issues <" src index.html README.md` returns nothing.
- [ ] **Step 5: Commit** `feat(work-items): call the Issues tab Work Items`.

### Task 2: Coral settings, token, and the client's calls

**Files:**
- Modify: `core/src/config.rs` (new `CoralItems`, `WorkItemsConfig.coral`, validation)
- Modify: `core/src/forge/mod.rs` (`auth_header`, used by `Ureq`)
- Create: `core/src/work_items/coral.rs` (client core), register in `core/src/work_items/mod.rs`
- Modify: `src/types.ts`, `src/settings.ts:348` (`workItemsConfig` default for `coral`)
- Test: in-file `#[cfg(test)]` modules; `src/settings.test.ts`

**Interfaces:**
- Produces (config.rs):
  ```rust
  #[serde(rename_all = "camelCase")]
  pub struct CoralItems {
      pub enabled: bool,              // default false
      pub base_url: String,           // default ""
      pub token: TokenSource,         // default Stored
      pub rules: Vec<ItemRule>,       // default [ItemRule { assignee: Me, labels: [], repos: [] }]
      pub start_status: Option<String>, // a Coral status id
  }
  impl CoralItems { pub fn env_name(&self) -> Option<String> } // blank → "CORAL_TOKEN"
  // WorkItemsConfig: #[serde(default)] pub coral: CoralItems
  ```
  In a Coral rule, `labels` holds tag names and `repos` holds product codes; `Assignee::User { login }` holds an email.
- Produces (forge/mod.rs): `pub fn auth_header(token: &str) -> (&'static str, String)` → `("x-api-token", token)` when the token starts with `cbp_` or `cba_` (Coral's token prefixes), else `("Authorization", "Bearer <token>")`. `Ureq::get/post_json/send_json` use it.
- Produces (coral.rs):
  ```rust
  pub struct Coral { pub base: String, pub token: String } // base without trailing '/'
  pub fn resolve(c: &CoralItems, stored: impl Fn() -> Option<String>, env: impl Fn(&str) -> Option<String>) -> Result<Coral, String>
  impl Coral { pub fn api(&self, path: &str) -> String /* "{base}/api/v1{path}" */; pub fn web(&self, code: &str) -> String /* "{base}/items/{code}" */ }
  pub(crate) fn get_json(http: &dyn Http, c: &Coral, url: &str) -> Result<Value, String>
  pub(crate) fn send_json(http: &dyn Http, c: &Coral, method: &str, url: &str, body: &Value) -> Result<(), String>
  pub struct CoralMe { pub id: String, pub name: String, pub email: String }
  pub fn me(http: &dyn Http, c: &Coral) -> Result<CoralMe, String> // GET /users/me
  ```
  Errors (all scrubbed of the token): 401/403 → `"Coral refused the token: paste a new one in Settings → Work Items."`; transport error → `"could not reach <host>: <e>"`; other non-2xx → `"<status>: <message or error field, else first 200 chars>"`; 404 → `"404: …"` (callers test `starts_with("404")`).

- [ ] **Step 1: Write failing tests.**
  - config.rs: `coral_is_off_by_default_and_an_old_file_reads` (a `WorkItemsConfig` JSON without `coral` deserialises with `coral.enabled == false`, one Me rule); `validate_refuses_a_coral_rule_for_anyone_without_products` (error `"A Coral rule for anyone's work items must name its products."`); `validate_refuses_a_bad_coral_address` (enabled with `"ftp://x"` or `""` → `"The Coral address must start with https://."`; `"http://localhost:3000"` and `"http://127.0.0.1:3000"` pass; validation runs only when `enabled`); `coral_env_name_defaults_to_coral_token`.
  - forge/mod.rs: `coral_tokens_go_in_x_api_token` (`auth_header("cbp_ab")` → `("x-api-token","cbp_ab")`; `auth_header("ghp_x")` → `("Authorization","Bearer ghp_x")`).
  - coral.rs: `base_is_trimmed` (`resolve` with `"https://coral.test/ "` → `api("/users/me") == "https://coral.test/api/v1/users/me"`); `resolve_trims_and_refuses_blank` (stored `" cbp_x\n"` → token `"cbp_x"`; stored None → `Err("No Coral token: paste one in Settings → Work Items.")`; env source with nothing set → `Err("No Coral token: set CORAL_TOKEN or paste a token in Settings → Work Items.")`); `a_refused_token_says_so_without_the_token` (FakeHttp 401 body containing the token → message equals the refusal line and does not contain `cbp_`); `me_reads_id_name_and_email`.
  - settings.test.ts: `workItemsConfig(null).coral` equals `{ enabled: false, baseUrl: "", token: { kind: "stored" }, rules: [{ assignee: { kind: "me" }, labels: [], repos: [] }], startStatus: null }` (match the existing `TokenSource` JSON shape used by `GitConnectionJson`).
- [ ] **Step 2: Run `cargo test -p maya-core` and `npm test`.** Expected: FAIL (types missing).
- [ ] **Step 3: Implement** the interfaces above. Validation lives in `validate_work_items`; trim `base_url` and strip trailing `/` when saving (in `validate_work_items`'s caller path or a `normalize` the save already uses for forges, whichever the code has).
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Commit** `feat(work-items): Coral settings and client`.

### Task 3: Listing Coral work items

**Files:**
- Modify: `core/src/work_items/coral.rs`
- Modify: `core/src/work_items/mod.rs` (`fetch` asks Coral; `combine` message)
- Test: in-file tests

**Interfaces:**
- Consumes: Task 2's `Coral`, `CoralMe`, `get_json`, `me`.
- Produces:
  ```rust
  pub const SOURCE: &str = "coral";
  pub const MAX_ITEMS: usize = 500;
  pub fn parse_item(v: &Value, c: &Coral) -> Option<WorkItem>
  pub fn list(http: &dyn Http, c: &Coral, cfg: &CoralItems, me: &CoralMe) -> Result<(Vec<WorkItem>, Vec<String>), String>
      // Ok((items, warnings)): warnings are per-rule lines ("Coral has no tag <name>", "Coral has no product <code>", "Coral has no user <email>"); Err only when nothing could be asked (users/me or every call failing on transport/refusal)
  pub fn get(http: &dyn Http, c: &Coral, code: &str) -> Result<Option<CoralItem>, String> // None when 404 or status category "done"
  pub struct CoralItem { pub item: WorkItem, pub uuid: String, pub item_type: String, pub assignments: Vec<Assignment>, pub status_id: Option<String> }
  pub struct Assignment { pub user_id: String, pub role: String }
  pub fn product_of(code: &str) -> &str // part before the last '-'; whole code when none
  // mod.rs: fetch(forges, cfg, http, sites, coral: Option<(&Coral, &CoralMe)>, list_tool) — Coral asked when Some and cfg.coral.enabled; its Fetched has source "coral", label "Coral".
  ```
  `parse_item` mapping (spec "Listing"): key `coral:<id>`, id `displayCode`, number None, url `c.web(displayCode)`, status `status.name`, labels = tag `"<scope>: <name>"` when scope is non-empty else `name`, assignees = names (email when name empty) of `assignments` with role `assignee`, author `createdByUser.name`, `repo: None`, `tracker: true`, description `""` when null.
  `list` per rule: resolve assignee (`Me` → `me.id`; `User{login}` → id via `GET /users` matching email case-insensitively; `Anyone` → none), resolve each tag name to ids via `GET /tags` (a "scope: name" form matches scope and name; a bare name matches every tag with that name; when a bare name matches several, ask once per matching tag id and merge), then for each of `itemType=work_item` and `itemType=defect` page with `pageSize=100&statusCategory=todo,active` plus `assignees`, `productCode` (CSV), `tagId` (repeated) and `tagCombinator=and`, following `nextCursor` (`cursor=`) until none, a repeat, or `MAX_ITEMS` in all. A 404 on a call with `productCode` becomes the "no product" warning (name the codes) and skips that rule. Resolve tags and users once per `list` call. The builder checks `/api/v1/tags` and `/api/v1/users` replies and query params against `/Users/tiagocorreia/dev/coral-boards/packages/web/src/routes/api/v1/` before writing the parsers.
  `combine`: a Coral failure reads `"Coral work items check failed: <e>"`; warnings go into the error lines with source `"coral"` and the items still show.

- [ ] **Step 1: Write failing tests** (FakeHttp routes by URL substring):
  - `parse_item_maps_a_work_item` (full reply → every field above, key `coral:6f1c…`, url `https://coral.test/items/BED-0021`, labels `["team: api", "urgent"]`, assignees only role `assignee`).
  - `parse_item_tolerates_missing_fields` (null description, no assignments, no tags, no `createdByUser` → still Some, empty strings/vecs).
  - `list_asks_for_work_items_and_defects_only_and_open_ones` (seen URLs contain `itemType=work_item` and `itemType=defect`, each with `statusCategory=todo,active` and `assignees=<me id>`; none contain `phase` or `subtask`).
  - `a_rule_names_products_and_tags_by_id` (rule labels `["team: api"]`, repos `["BED","OPS"]` → URL has `productCode=BED,OPS`, `tagId=<id>`, `tagCombinator=and`).
  - `an_unknown_tag_or_product_skips_only_that_rule` (one rule with unknown tag, one fine → items from the fine rule, warning `"Coral has no tag nope"`; a 404 for `productCode=ZZZ` → warning `"Coral has no product ZZZ"`).
  - `list_stops_at_the_cap_and_on_a_repeated_cursor` (a reply whose `nextCursor` repeats ends the loop; 600 items across pages → 500).
  - `get_is_none_for_a_done_or_missing_item`; `product_of_is_the_code_before_its_number` (`"BED-0021"`→`"BED"`, `"MY-APP-0003"`→`"MY-APP"`).
  - mod.rs: `fetch_asks_coral_when_it_is_on` and `a_failed_coral_check_keeps_its_items_and_says_so` (message `"Coral work items check failed: …"`).
- [ ] **Step 2: Run `cargo test -p maya-core work_items`.** Expected: FAIL.
- [ ] **Step 3: Implement** the interfaces.
- [ ] **Step 4: Run `cargo test -p maya-core`.** Expected: PASS.
- [ ] **Step 5: Commit** `feat(work-items): list Coral work items and defects`.

### Task 4: Coral's Start: repositories, statuses, take

**Files:**
- Modify: `core/src/work_items/coral.rs`
- Test: in-file tests

**Interfaces:**
- Consumes: Task 2–3 (`Coral`, `CoralMe`, `CoralItem`, `get_json`, `send_json`, `product_of`).
- Produces:
  ```rust
  pub struct CoralStatus { pub id: String, pub name: String, pub category: String, pub sub: bool /* isAlsoSubStatus */, pub position: i64 }
  pub fn statuses(http: &dyn Http, c: &Coral) -> Result<Vec<CoralStatus>, String> // GET /statuses, sorted by position
  pub fn product_repos(http: &dyn Http, c: &Coral, product: &str, sites: &[ForgeSite]) -> Result<Vec<ItemRepo>, String>
      // GET /products/{code}/repositories; each `url` through forge::route(url, sites); unknown hosts dropped; deduped ignoring case
  pub fn status_for(item_type: &str, start: &str, all: &[CoralStatus]) -> Option<String>
      // the start id, unless item_type is "defect" and that status has sub=false: then the first (by position) status with the same category and sub=true; None when there is none
  pub fn take(http: &dyn Http, c: &Coral, it: &CoralItem, me: &CoralMe, start_status: Option<&str>) -> Result<(), String>
  ```
  `take`: if `me.id` is not among `it.assignments` with role `assignee`, `PUT /work-items/{code}/assignments` with every existing assignment (all roles) plus `{userId: me.id, role: "assignee"}`. Then, when `start_status` is Some: read `statuses`, pick `status_for(&it.item_type, start, …)`; None → `Err("Coral allows no status like <name> on a defect; the status was left as it is.")`; Some(id) equal to `it.status_id` → nothing; else `PATCH /work-items/{code}` `{"statusId": id}`. Assignment always comes first. A Coral 400 surfaces as `"400: <Coral's message>"`.

- [ ] **Step 1: Write failing tests:**
  - `product_repos_route_to_known_forges` (urls `https://github.com/acme/api.git`, `https://gitlab.acme.test/g/r/`, `https://bitbucket.org/x/y` with sites github + gitlab at `https://gitlab.acme.test` → `[Github acme/api, Gitlab g/r]`).
  - `take_adds_me_beside_the_others_then_sets_the_status` (seen order: PUT assignments whose body keeps a reviewer and an existing assignee and adds me, then PATCH with the status id).
  - `take_skips_the_assignment_when_i_am_assigned_and_the_status_when_it_is_set`.
  - `a_defect_gets_the_first_allowed_status_of_the_same_kind` (`status_for("defect", "s-active", [s-active sub=false pos 1, s-doing sub=true pos 2, s-test sub=true pos 3 (category active)])` → `"s-doing"`; for `"work_item"` → `"s-active"`; no allowed → None).
  - `a_refused_status_change_shows_corals_message` (PATCH 400 `{"error":"Bugs need a severity"}` → `Err` containing `"Bugs need a severity"`).
- [ ] **Step 2: Run `cargo test -p maya-core coral`.** Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run `cargo test -p maya-core`.** Expected: PASS.
- [ ] **Step 5: Commit** `feat(work-items): Start assigns and sets the status of a Coral item`.

### Task 5: The desktop app polls and starts Coral items

**Files:**
- Modify: `src-tauri/src/keystore.rs` (`CORAL_TOKEN: Item { service: "maya-coral", account: "token", label: "Maya Coral token", linux_account: "coral-token" }`)
- Modify: `src-tauri/src/git.rs` (`coral()` resolver; commands)
- Modify: `src-tauri/src/work_items_app.rs` (poll, started re-read, `StartCheck.repo`, Start's Coral branch, pick options)
- Modify: `src-tauri/src/lib.rs` (`AppState.coral_me`, register commands)
- Modify: `core/src/work_items/pick.rs` (`candidates` takes `first: &[ItemRepo]`)
- Modify: `src/types.ts` (`StartCheck.repo`), `src/workitems.ts` (use `check.repo`; `SOURCE_LABEL.coral = "Coral"`; card shows "Coral" text badge like custom tools)
- Test: in-file Rust tests; `src/workitems.test.ts`

**Interfaces:**
- Consumes: Tasks 2–4.
- Produces (Tauri commands, all `#[tauri::command(async)]` where they call the network):
  - `store_coral_token(token: String) -> Result<(), String>`, `clear_coral_token() -> Result<(), String>` (both call `git::settings_changed`)
  - `get_coral_status() -> CoralStatusJson { has_token: bool }` (camelCase)
  - `test_coral_connection() -> Result<String, String>` → `"Connected as <name>"`
  - `coral_statuses() -> Result<Vec<CoralStatus>, String>` (`CoralStatus` derives `Serialize`, camelCase)
- Produces (Rust): `pub fn git::coral(cfg: &CoralItems) -> Result<Coral, String>` (reads `keystore::CORAL_TOKEN`, `forge::env::read`); `StartCheck { takeover, pick, branch, repo: Option<ItemRepo> }`; `pick::candidates(cfg, items, clones, first: &[ItemRepo])` puts `first` at the front (existing dedup keeps the first spelling).
- Produces (TS): `StartCheck.repo: ItemRepo | null`; `needsDialog(check)` unchanged; `startItem` sends `repo: chosen?.repo ?? check.repo`.

Behaviour (spec "Listing" and "Start"):
- Poll: when `cfg.coral.enabled`, resolve Coral (a resolve error becomes a `SourceError { source: "coral" }` line, like a missing forge token) and `me` through `AppState.coral_me: Mutex<Option<(u64, CoralMe)>>` kept per `git_generation`; pass to `work_items::fetch`. `off` is false when Coral is on. `source_labels` gets `"coral" → "Coral"`.
- Started re-read: a started item with `source == "coral"` that no rule returned is read with `coral::get(…, &s.item.id)`; Some → its `item` replaces the copy (key unchanged); None → closed.
- `work_item_check` for a Coral item: fresh `coral::get` (None → `Err("That work item was closed.")`), `show_fresh`, takeover = assignee names other than `me.name` (ignoring case; also not `me.email`), `product_repos` for `product_of(id)`: exactly one → `repo: Some(it)`, `pick: None`; else `pick: Some(pick_repo(…, first = product repos))`.
- `work_item_start` for a Coral item: fresh read, `takeover_refusal` against `me.name`, preflight, `coral::take(…, cfg.coral.start_status.as_deref())` with a failure as a notice `"Started, but <id> was not updated: <e>"`, then as today. `repo_to_start` accepts the passed repo (Coral items `needs_pick`).

- [ ] **Step 1: Write failing tests:**
  - keystore/git: `each_provider_has_its_key_store_item` extended: `CORAL_TOKEN.service == "maya-coral"`, distinct from GitHub's and GitLab's.
  - work_items_app.rs: `coral_takeover_names_the_other_assignees` (assignees `["Ana", "Me Person"]`, me name `"me person"` → `["Ana"]`); `one_product_repo_needs_no_dialog` (a pure helper `fn coral_start_repo(repos: Vec<ItemRepo>) -> Option<ItemRepo>`: one → Some, zero or two → None).
  - pick.rs: `the_first_repos_come_first` (`candidates(…, first=[gitlab g/r])[0]` is `g/r`, and it is not repeated).
  - workitems.test.ts: `it("starts in the repository the check found when no dialog is needed")` (invoke stub: check returns `{ takeover: [], pick: null, branch: "issue-bed-0021-x", repo: { forge: "github", path: "acme/api" } }`; the `work_item_start` call's `repo` equals that repo); `it("names Coral items' source Coral")` (renderItemCard with `sourceLabel` "Coral" shows the text).
- [ ] **Step 2: Run `cargo test -p maya` (src-tauri), `cargo test -p maya-core`, `npm test`.** Expected: FAIL.
- [ ] **Step 3: Implement.** Register the five commands in `lib.rs`'s `invoke_handler`.
- [ ] **Step 4: Run all three test commands and `npx tsc --noEmit`.** Expected: PASS.
- [ ] **Step 5: Commit** `feat(work-items): the app lists and starts Coral items`.

### Task 6: The Coral card in Settings → Work Items

**Files:**
- Modify: `src/settings.ts` (Coral source in the Sources group; handlers; `initSettings` wiring), `src/types.ts` (`CoralItems`, `CoralStatus`)
- Modify: `src/styles.css` only if the shared classes do not cover it
- Test: `src/settings.test.ts`

**Interfaces:**
- Consumes: Task 5's commands; Task 2's `CoralItems` JSON.
- Produces: `SettingsHandlers` gains `onCoralToken(token: string): void`, `onCoralClearToken(): void`, `onCoralTest(): Promise<string>`, `onCoralStatuses(): Promise<CoralStatus[]>`; `SettingsModel` gains `coralHasToken: boolean`, `coralStatuses: CoralStatus[] | null`. The card is a fold in the Sources group (`data-source="coral"`, open state kept with `storedSourcesOpen()` like the others), on/off switch visible when folded, summary line from `forgeSummary`-style text (`coralSummary(items: CoralItems): string`, exported).

Card rows (spec "The Settings card"), using `row`/`footRow`:
- **Address** text input (`placeholder="https://coral.example.com"`), saved on change.
- **Token**: the same paste/"From variable" control Settings → Git uses (reuse its builder; variable placeholder `CORAL_TOKEN`), and a **Test** button whose result shows in the row's help (`"Connected as <name>"` or the error).
- **Work items to pick up**: the existing rules editor with labels "Tags" and "Products" in place of "Labels" and "Repositories", and the "someone" field's placeholder "their email". Refactor the forge rules editor into a function taking those words if it is inline today.
- **On Start, set status to**: a `<select>` of `coralStatuses` by name plus "Leave as it is" (value `""`); while `coralStatuses` is null it holds only the saved id's option (text "Saved status") and "Leave as it is", with help `"Test the connection to list Coral's statuses."`. Help: `"A defect gets the first allowed status of the same kind when this one is not allowed for defects."`
- Foot line: `"The token acts as you in Coral: Start assigns the item to you and sets its status."`
- Statuses load (`onCoralStatuses`) when the card opens with a token, and after a successful Test.

- [ ] **Step 1: Write failing tests** in `src/settings.test.ts` (render with a model and stub handlers, as the existing Work Items tests do):
  - `it("shows a Coral source, off by default, with its switch visible when folded")`.
  - `it("saves the Coral address, rules and start status")` (change inputs → saved config has `coral.baseUrl`, a rule with `labels: ["team: api"]`, `repos: ["BED"]`, `startStatus: "s-doing"`).
  - `it("calls the rule fields Tags and Products for Coral")`.
  - `it("lists Coral's statuses once they load, and keeps the saved one before")`.
  - `it("shows the Test result")`.
  - `coralSummary` cases: off → `"Off"`; on with one Me rule → same wording style as `forgeSummary`.
- [ ] **Step 2: Run `npm test`.** Expected: FAIL.
- [ ] **Step 3: Implement**; wire `initSettings` to the Task 5 commands (`get_coral_status` on load and after token changes).
- [ ] **Step 4: Run `npm test` and `npx tsc --noEmit`.** Expected: PASS. Then a WebKit screenshot check of Settings → Work Items with the Coral card open (Playwright WebKit with the real `src/styles.css`, as in earlier tasks), compared by eye with the GitHub card.
- [ ] **Step 5: Commit** `feat(work-items): a Coral card in Settings`.

### Task 7: README, release notes and version

**Files:**
- Modify: `README.md`, `RELEASE_NOTES.md`; version files via `scripts/set-version.sh`

- [ ] **Step 1:** README's Work Items lines: how to connect Coral (Settings → Work Items → Coral: its address, and a personal token made in Coral; work items and defects show; Start assigns you and sets the status).
- [ ] **Step 2:** Commit `docs(work-items): connecting Coral`.
- [ ] **Step 3:** `git fetch origin && git show origin/main:package.json | grep '"version"'` — bump from that (expected 0.54.0 → 0.55.0): `sh scripts/set-version.sh 0.55.0`.
- [ ] **Step 4:** Add at the top of `RELEASE_NOTES.md`:
  ```
  # 0.55.0

  ## New

  - The Issues tab is now called Work Items, the same as its settings.
  - Coral work items and defects show in Work Items beside your GitHub and GitLab issues. Connect Coral in Settings → Work Items with its address and a personal token. Start assigns the item to you, sets its status and opens a session in the product's repository.
  ```
- [ ] **Step 5:** Run `npm test`, `cargo test -p maya-core`, `cargo test -p maya`. Expected: PASS. Commit both as `chore(release): 0.55.0`.
