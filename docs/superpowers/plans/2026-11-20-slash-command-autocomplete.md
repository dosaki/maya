# Slash command autocompletion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Typing `/` in a session's message box lists that session's commands (agent built-ins, user, plugin and project commands and skills), for local and remote sessions and every agent that takes `/commands`.

**Architecture:** A new `maya_core::commands` module lists commands from disk per agent. Each assistant sends its machine-wide lists to the main in a new `Up::Commands` message, only when they change; the main keeps them per machine. A session window shows a cached list (or the machine's shared list, or built-ins), then asks the session's machine for the full list (`CommandKind::ListCommands`) in the background and caches it. A small popup above the composer (`src/slash.ts`) does matching and keys.

**Tech Stack:** Rust (maya-core, Tauri 2 app, mobile, cli), vanilla TypeScript + vitest (jsdom).

**Spec:** `docs/superpowers/specs/2026-11-20-slash-command-autocomplete-design.md`

## Global Constraints

- Older Mayas keep working: every new serde field is `#[serde(default)]`; an older main ignores `Up::Commands` (it logs "ignored a message"); an older assistant never sends it, and the main answers `ListCommands` for it at once with an error instead of waiting `ROUTE_TIMEOUT`.
- An unknown harness or command source from a newer Maya is dropped or read as `other`, never fails the message (as `known_agents` in `core/src/net/protocol.rs` does).
- Descriptions are cut where they are made, in `CommandInfo::new`: first non-empty line only, at most 100 characters, cut at a word boundary with `…` appended.
- Project commands never go in `Up::Commands`; only `ListCommands` returns them.
- Repaint the popup's rows with `morph`/`keepEqualChildren` from `src/morph.ts`, never `replaceChildren` (CLAUDE.md: WKWebView drops `:hover` from a replaced node).
- A pick fills the box and never sends. Only agents with `caps.slashLines` get the popup, and only while the composer is shown (not the question menu).
- Run Rust tests with `cargo test --workspace` from the repo root, frontend tests with `pnpm test`.
- Version: `feat` → minor bump from `main`'s version, with `RELEASE_NOTES.md` (see CLAUDE.md).

## Review Focus

1. **Esc with the list open** closes the list only; the window stays open (the modal's document `keydown` handler in `openModal` dismisses on Escape, so the popup must `stopPropagation`). Test in Task 7.
2. **IME composition** (`ev.isComposing`): the popup ignores every key while composing, like `composerKey`. Test in Task 6.
3. **A `/` that is not at the start of the first word**: `hello /co`, `/compact now` with the caret after the space, or a second line starting with `/` must not open the list. Test in Task 6.
4. **Broken files on disk**: no front matter, front matter without a closing `---`, invalid `installed_plugins.json` or `settings.json`, a missing `installPath`, a symlink loop under `commands/`: the scan skips what it cannot read and never panics or hangs. Tests in Tasks 1 and 2.
5. **A late answer**: the background refresh for a window that was closed, or replaced by another session's window, must not change the new window's list (it may still write the cache). Test in Task 7.

---

### Task 1: Command types, front matter, built-ins

**Files:**
- Create: `core/src/commands/mod.rs`, `core/src/commands/builtins.rs`
- Modify: `core/src/lib.rs` (add `pub mod commands;`)
- Test: in-file `#[cfg(test)]` modules

**Interfaces:**
- Produces:
  ```rust
  #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
  #[serde(rename_all = "camelCase")]
  pub struct CommandInfo { pub name: String, #[serde(default)] pub description: String, pub source: CommandSource }
  #[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
  #[serde(rename_all = "kebab-case")]
  pub enum CommandSource { Builtin, User, Plugin, Project, #[serde(other)] Other }
  #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
  pub struct AgentCommands { pub harness: Harness, pub commands: Vec<CommandInfo> }
  impl CommandInfo { pub fn new(name: &str, description: &str, source: CommandSource) -> CommandInfo }
  pub fn cut_description(text: &str) -> String;
  pub struct FrontMatter { pub name: Option<String>, pub description: Option<String>, pub user_invocable: bool }
  pub fn front_matter(text: &str) -> FrontMatter;          // user_invocable defaults to true
  pub fn description_of(text: &str) -> String;            // front matter description, else first non-empty body line
  pub fn dedupe(list: Vec<CommandInfo>) -> Vec<CommandInfo>; // keeps the first of each name, order kept
  pub fn builtins(h: Harness) -> Vec<CommandInfo>;         // in builtins.rs, re-exported
  pub fn builtin_lists() -> Vec<AgentCommands>;            // every harness except Other
  ```

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn a_description_keeps_its_first_line_and_at_most_100_chars() {
    assert_eq!(cut_description("\n  Make a commit.\nMore."), "Make a commit.");
    let long = "word ".repeat(40);
    let cut = cut_description(&long);
    assert!(cut.chars().count() <= 100 && cut.ends_with('…') && !cut.contains("wor…"), "{cut}");
}
#[test] fn front_matter_reads_name_description_and_user_invocable() {
    let fm = front_matter("---\nname: commit\ndescription: \"Create a git commit\"\nuser-invocable: false\n---\nbody");
    assert_eq!((fm.name.as_deref(), fm.description.as_deref(), fm.user_invocable), (Some("commit"), Some("Create a git commit"), false));
}
#[test] fn a_folded_description_is_joined() {
    assert_eq!(front_matter("---\ndescription: >\n  Use when\n  starting\n---\n").description.as_deref(), Some("Use when starting"));
}
#[test] fn no_or_unclosed_front_matter_reads_as_none() {
    assert_eq!(front_matter("# Title\nbody").description, None);
    assert_eq!(front_matter("---\ndescription: x\nno end").description, None);
    assert_eq!(description_of("\n\nRun the thing\nmore"), "Run the thing");
}
#[test] fn dedupe_keeps_the_first_of_each_name() { /* [a(user), b, a(builtin)] -> [a(user), b] */ }
#[test] fn every_agent_but_other_has_builtins_including_compact() {
    for h in [Harness::ClaudeCode, Harness::Codex, Harness::OpenCode, Harness::Kiro, Harness::Grok, Harness::Antigravity] {
        assert!(builtins(h).iter().any(|c| c.name == "compact" && c.source == CommandSource::Builtin), "{h:?}");
    }
    assert!(builtins(Harness::Other).is_empty());
}
#[test] fn an_unknown_source_reads_as_other() {
    let c: CommandInfo = serde_json::from_str(r#"{"name":"x","source":"future"}"#).unwrap();
    assert_eq!((c.source, c.description.as_str()), (CommandSource::Other, ""));
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p maya-core commands::`
Expected: FAIL to compile (module missing).

- [ ] **Step 3: Implement**

No YAML crate: `front_matter` reads only when line 1 is exactly `---`, up to the next `---` line; `key: value` lines, values with surrounding `"`/`'` stripped; a value of `>`, `>-`, `|` or `|-` takes the following indented lines joined with spaces. `builtins` returns the names and descriptions in the Appendix, source `Builtin`, through `CommandInfo::new`.

- [ ] **Step 4: Run to see them pass**

Run: `cargo test -p maya-core commands::`
Expected: PASS.

- [ ] **Step 5: Commit**

`git commit -m "feat(core): command lists' types, front matter and built-ins"`

---

### Task 2: Claude Code's commands from disk

**Files:**
- Create: `core/src/commands/claude.rs`
- Test: in-file tests with `tempfile::tempdir()`

**Interfaces:**
- Consumes: Task 1's `CommandInfo`, `front_matter`, `description_of`, `dedupe`, `builtins`.
- Produces:
  ```rust
  /// Where the agents keep their files; tests point these at temp dirs.
  pub struct Homes { pub home: PathBuf, pub claude: PathBuf }   // in mod.rs
  impl Homes { pub fn current() -> Homes }                      // home = dirs::home_dir(), claude = home/.claude; actions.rs builds its own with the store's claude_dir()
  pub fn project_dirs(cwd: &Path) -> Vec<PathBuf>;              // in mod.rs: cwd, then parents up to and including the first holding `.git`; just [cwd] when none does
  pub fn claude_machine(h: &Homes) -> Vec<CommandInfo>;
  pub fn claude_session(h: &Homes, cwd: &Path) -> Vec<CommandInfo>;
  ```

Rules (from the agents' docs, checked 2026-11-20):
- Skill: `<root>/<dir>/SKILL.md`; name = front matter `name`, else `<dir>`; left out when `user-invocable: false`.
- Command file: `<root>/**/*.md`; name = path below `<root>` without `.md`, folders joined with `:` (`frontend/component.md` → `frontend:component`). Depth at most 4; symlinked folders are not followed.
- User: `<claude>/skills` (source `User`, skipping `synced/`), `<claude>/skills/synced/<id>/<name>/SKILL.md` as `anthropic-skills:<name>` (`User`), `<claude>/commands` (`User`).
- Project, for each of `project_dirs(cwd)`: `.claude/skills`, `.claude/commands` (`Project`).
- Plugins (`Plugin`): `<claude>/plugins/installed_plugins.json` is `{"version":2,"plugins":{"<id>":[{scope, projectPath?, installPath, ...}]}}`. An entry counts when `scope` is `user`, or (session only) `project`/`local` with `cwd` under `projectPath`. A plugin is enabled unless the merged `enabledPlugins` maps its id to `false`; merge `<claude>/settings.json`, then (session only) `<project>/.claude/settings.json`, then `<project>/.claude/settings.local.json`, later wins. Namespace = `name` in `<installPath>/.claude-plugin/plugin.json`, else the id before `@`. Commands: `<installPath>/commands/*.md`, or the manifest's `commands` when it is a string or array of paths (relative to `installPath`), which replaces the folder. Skills: `<installPath>/skills/<dir>/SKILL.md`, plus a root `SKILL.md`, plus manifest `skills` paths. Typed name `<namespace>:<name>`.
- Order before `dedupe`: user skills, user commands, project skills, project commands, plugins, built-ins.
- `claude_machine` = the same without anything project-only.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn user_skills_and_commands_are_listed_with_their_names() {
    // <claude>/skills/review/SKILL.md (description: Review it), <claude>/commands/frontend/component.md (no front matter, first line "Make a component")
    // claude_machine -> contains {review, "Review it", User} and {frontend:component, "Make a component", User}
}
#[test] fn a_skill_that_is_not_user_invocable_is_left_out() {}
#[test] fn an_enabled_user_plugin_lists_namespaced_commands_and_skills() {
    // installed_plugins.json with "commit-commands@tkc" scope user, installPath with .claude-plugin/plugin.json {"name":"commit-commands"},
    // commands/commit.md, skills/clean/SKILL.md -> "commit-commands:commit", "commit-commands:clean" (Plugin)
}
#[test] fn a_plugin_disabled_in_settings_is_left_out() {}
#[test] fn a_project_scoped_plugin_counts_only_inside_its_project() {}
#[test] fn project_commands_come_only_with_a_session_and_from_parents_up_to_the_repo() {
    // repo/.git, repo/.claude/commands/ship.md, cwd = repo/app; claude_session has {ship, Project}; claude_machine has no "ship"
}
#[test] fn a_user_skill_wins_over_a_builtin_of_the_same_name() {}
#[test] fn broken_files_are_skipped() {
    // installed_plugins.json = "{not json", settings.json = "", a plugin whose installPath is missing,
    // a commands/loop -> commands symlink: claude_session returns built-ins and readable files, no panic
}
```

- [ ] **Step 2: Run to see them fail** — `cargo test -p maya-core commands::claude` → FAIL (missing functions).
- [ ] **Step 3: Implement `claude_machine` and `claude_session`** in `core/src/commands/claude.rs` per the rules above.
- [ ] **Step 4: Run to see them pass** — `cargo test -p maya-core commands::` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(core): list Claude Code's skills, commands and plugin commands"`

---

### Task 3: Other agents, machine and session lists, shared cache

**Files:**
- Create: `core/src/commands/others.rs`
- Modify: `core/src/commands/mod.rs`

**Interfaces:**
- Consumes: Tasks 1–2.
- Produces:
  ```rust
  pub fn machine_wide(h: Harness, homes: &Homes) -> Vec<CommandInfo>;
  pub fn for_session(h: Harness, cwd: &Path, homes: &Homes) -> Vec<CommandInfo>;
  /// This machine's machine-wide lists for Claude Code and every agent `agents::cached()` lists; rebuilt at most every 60 s.
  pub fn shared() -> Vec<AgentCommands>;
  ```

Folders (built-ins always come last; `dedupe` after):
- **Codex:** built-ins only (skills are typed `$name`, custom prompts are deprecated).
- **OpenCode:** `<home>/.config/opencode/commands/**/*.md` and `.../command/**/*.md` (`User`); project `.opencode/commands` and `.opencode/command` in `cwd` (`Project`). Name = path without `.md`, folders joined with `/`.
- **Kiro:** skills `<home>/.kiro/skills/<dir>/SKILL.md` and prompts `<home>/.kiro/prompts/*.md` (`User`); project `.kiro/skills`, `.kiro/prompts` in `cwd` (`Project`). Prompt name = file stem.
- **Grok:** `<home>/.grok/skills`, `<home>/.grok/commands/*.md` (`User`); project `.grok/skills`, `.grok/commands` for each of `project_dirs(cwd)` (`Project`). Front matter `user-invocable: false` hides.
- **Antigravity:** `<home>/.gemini/config/skills` (`User`); project `.agents/skills` for each of `project_dirs(cwd)` (`Project`).
- **Other:** empty.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn opencode_commands_join_folders_with_a_slash() { /* .config/opencode/commands/team/review.md -> "team/review" */ }
#[test] fn kiro_prompts_and_skills_are_listed() {}
#[test] fn codex_has_builtins_only() { /* even with ~/.codex/prompts/x.md present */ }
#[test] fn for_session_adds_project_commands_machine_wide_does_not() { /* one per agent with project folders */ }
#[test] fn other_has_nothing() { assert!(for_session(Harness::Other, Path::new("/"), &homes).is_empty()); }
```

- [ ] **Step 2: Run to see them fail** — `cargo test -p maya-core commands::` → FAIL.
- [ ] **Step 3: Implement** `machine_wide`, `for_session` (dispatch per harness), and `shared()` with a `static Mutex<Option<(Instant, Vec<AgentCommands>)>>`, 60 s max age, harnesses = Claude Code plus `agents::cached()`'s, without `Other`.
- [ ] **Step 4: Run to see them pass** — `cargo test -p maya-core commands::` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(core): command lists for Codex, OpenCode, Kiro, Grok and Antigravity"`

---

### Task 4: The device group carries command lists

**Files:**
- Modify: `core/src/net/protocol.rs`, `core/src/net/client.rs`, `core/src/net/server.rs`, `core/src/net/merge.rs`, `core/src/net/routing.rs`

**Interfaces:**
- Consumes: `AgentCommands`, `CommandInfo` (Task 1).
- Produces:
  - `Up::Commands { #[serde(deserialize_with = "known_commands")] agents: Vec<AgentCommands> }` — entries with an unknown harness or `Other` dropped.
  - `CommandKind::ListCommands { session: String }`; `routing::target_of` → `Target::Session`.
  - `Request::ListCommands` (the machine is `Up::Request.machine`).
  - `Executor::commands(&self) -> Option<Vec<AgentCommands>>`, default `None`.
  - `RemoteBoard.commands: Option<Vec<AgentCommands>>`, set from the peer's last `Up::Commands`.
  - `merge::commands_of(remotes: &[RemoteBoard], machine: &str) -> Option<Vec<AgentCommands>>`.
  - `routing::NO_COMMANDS: &str = "That machine's Maya is too old to list its commands."`

- [ ] **Step 1: Write the failing tests**

```rust
// protocol.rs
#[test] fn commands_round_trip_and_drop_unknown_agents() {
    let text = r#"{"type":"commands","agents":[{"harness":"future","commands":[]},{"harness":"claude-code","commands":[{"name":"compact","source":"builtin"}]}]}"#;
    let Up::Commands { agents } = decode_up(text).unwrap() else { panic!() };
    assert_eq!(agents.len(), 1);
}
#[test] fn list_commands_targets_its_session() { /* target_of(ListCommands{session:"s1"}) == Target::Session("s1") */ }
// client.rs (use the existing fake executor and main used by the Up::Terminals tests near line 1349)
#[test] fn commands_go_up_once_and_again_only_when_they_change() {
    // exec.commands() returns L1: exactly one Up::Commands within 10 s; after two board refreshes, still one;
    // switch to L2: a second Up::Commands with L2.
}
// server.rs (pattern of the test at line 2412)
#[test] fn a_main_keeps_an_assistants_commands_on_its_board() { /* send Up::Commands; commands_of(boards, "desk") == Some(list) */ }
// merge.rs
#[test] fn commands_of_is_none_for_an_older_maya() {}
```

- [ ] **Step 2: Run to see them fail** — `cargo test -p maya-core net::` → FAIL.
- [ ] **Step 3: Implement.** Client `pump`: a local `last_commands: Option<Vec<AgentCommands>>` starting `None`; inside the board branch (next to the terminals check), send `Up::Commands` when `exec.commands()` is `Some` and differs. Not gated by a cap. Server: store on the peer under the same `conns`/`serial` check as `Up::Terminals`, then `sync_boards()`; fill `RemoteBoard.commands` where `terminal_choices` is filled (line ~429). Add the match arms that `CommandKind::ListCommands` and `Request::ListCommands` need for the workspace to compile (the app-side answers come in Task 5; until then return `Err("not yet")` only where a match must be exhaustive).
- [ ] **Step 4: Run to see them pass** — `cargo test -p maya-core net::` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(net): assistants share their command lists with the main"`

---

### Task 5: Answering in the apps (desktop, CLI, phone)

**Files:**
- Modify: `core/src/actions.rs`, `src-tauri/src/lib.rs`, `src-tauri/src/net_app.rs`, `cli/src/executor.rs`, `mobile/src/commands.rs`, `mobile/src/lib.rs`

**Interfaces:**
- Consumes: Tasks 3–4.
- Produces (Tauri commands, the same names and shapes on desktop and phone):
  - `list_commands({ machine: string }) -> AgentCommands[]`. Local (empty machine): `commands::shared()`. Remote: `merge::commands_of`, else `commands::builtin_lists()`. Phone: `hub.request(machine, Request::ListCommands)`, else `builtin_lists()`.
  - `session_commands({ sessionId: string }) -> CommandInfo[]`. Local: `actions::session_commands`. Remote: `Err(NO_COMMANDS)` at once when `commands_of(machine)` is `None`, else route `CommandKind::ListCommands`. Phone: `hub.send(&session_id, |session| CommandKind::ListCommands { session })`.
  - `pub fn session_commands(l: &Local, session_id: &str) -> Result<Vec<CommandInfo>, String>` in `actions.rs`: finds the card with `store.card_for(session_id, now_ms())` (error "Session is no longer running."), then `commands::for_session(card.harness, Path::new(&card.cwd), &Homes::current())`.
  - `net_app.rs`: `execute` answers `ListCommands` with `to_data(actions::session_commands(..)?)`; `Request::ListCommands` answers like `Request::ListAgents` (machine → `commands_of`/`builtin_lists`; none → `shared()`); the `Request::Command` route checks `NO_COMMANDS` for `ListCommands` before sending. `Executor::commands` returns `Some(commands::shared())` in `net_app.rs` and `cli/src/executor.rs`.

- [ ] **Step 1: Write the failing tests**

```rust
// core/src/actions.rs (beside the remote_history test near line 2467)
#[test] fn session_commands_lists_the_sessions_project_commands() {
    // a running Claude card with cwd = temp repo holding .claude/commands/ship.md -> contains "ship" (Project)
}
#[test] fn session_commands_for_an_unknown_session_is_an_error() {}
```

- [ ] **Step 2: Run to see them fail** — `cargo test -p maya-core actions::session_commands` → FAIL.
- [ ] **Step 3: Implement** the functions above and register both Tauri commands in `src-tauri/src/lib.rs` and `mobile/src/lib.rs`.
- [ ] **Step 4: Run all Rust tests** — `cargo test --workspace` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: list a session's commands on its own machine"`

---

### Task 6: The popup

**Files:**
- Create: `src/slash.ts`, `src/slash.test.ts`
- Modify: `src/types.ts` (types), `src/styles.css` (`.slash`, and `.modal__composer { position: relative }`)

**Interfaces:**
- Produces:
  ```ts
  export interface CommandInfo { name: string; description: string; source: string }
  export interface AgentCommands { harness: Harness; commands: CommandInfo[] }   // in types.ts
  /** The word typed after "/" when the caret is in the box's first word and it starts with "/"; else null. */
  export function slashQuery(value: string, caret: number): string | null;
  /** Names starting with `typed` first, then names containing it, each in list order; all for "". */
  export function matchCommands(list: CommandInfo[], typed: string): CommandInfo[];
  export interface SlashMenu { element: HTMLElement; setCommands(list: CommandInfo[]): void; onKey(ev: KeyboardEvent): boolean; close(): void }
  export function attachSlashMenu(box: HTMLTextAreaElement): SlashMenu;
  ```
- Behaviour: opens on `input`/`click`/`keyup` when `slashQuery` is non-null and something matches; first match highlighted; `ArrowDown`/`ArrowUp` move (wrapping); `Tab` fills; `Enter` fills unless `typed` is exactly some command's name, when `onKey` returns false so the composer sends; `Escape` closes and calls `ev.stopPropagation()`; returns false for everything while `ev.isComposing` or closed. Filling replaces the first word with `/<name> ` and keeps the rest of the text, caret after the space, then dispatches `input`. Rows: `mousedown` → `preventDefault()` (keeps focus), `click` fills. Each row shows `/name`, the description, and `· plugin` / `· project` for those sources. `role="listbox"`/`"option"`, `aria-selected`; the box gets `aria-expanded`, `aria-controls`, `aria-activedescendant`. Shows all matches in a scrolling list at most 8 rows tall; the highlighted row is scrolled into view. Rows are repainted with `morph` from `src/morph.ts`.

- [ ] **Step 1: Write the failing tests**

```ts
it("reads the typed word only in the first word", () => {
  expect(slashQuery("/co", 3)).toBe("co");
  expect(slashQuery("/", 1)).toBe("");
  expect(slashQuery("/compact now", 12)).toBeNull();
  expect(slashQuery("hello /co", 9)).toBeNull();
  expect(slashQuery("hi\n/co", 6)).toBeNull();
});
it("matches the start of a name first, then anywhere", () => {
  expect(matchCommands([cmd("code-review"), cmd("review"), cmd("rename")], "re").map((c) => c.name)).toEqual(["review", "rename", "code-review"]);
});
it("Tab fills the highlighted command and keeps the rest", () => { /* "/co" -> Tab -> "/compact " , caret 9 */ });
it("Enter fills a partial name but sends a whole one", () => { /* "/co": onKey(Enter) true, value "/compact "; "/compact": onKey(Enter) false */ });
it("Escape closes the list and stops the event", () => {});
it("ignores keys while composing", () => {});
it("a click fills without moving focus out of the box", () => {});
it("a new list keeps the open list's rows and highlight", () => { /* setCommands with the same + one more: first row node is the same object */ });
```

- [ ] **Step 2: Run to see them fail** — `pnpm test src/slash.test.ts` → FAIL.
- [ ] **Step 3: Implement** `src/slash.ts` and its CSS: the list sits at `bottom: 100%` of `.modal__composer`, full width, `max-height` of 8 rows and at most `40vh` (so it fits above the phone's keyboard), in the existing terminal-look tokens.
- [ ] **Step 4: Run to see them pass** — `pnpm test src/slash.test.ts` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: a slash command list above the message box"`

---

### Task 7: The cache and the session window

**Files:**
- Create: `src/slash-cache.ts`, `src/slash-cache.test.ts`
- Modify: `src/modal.ts`, `src/modal-open.test.ts` (or a new `src/modal-slash.test.ts`)

**Interfaces:**
- Consumes: Task 5's Tauri commands, Task 6's `attachSlashMenu`.
- Produces:
  ```ts
  export function commandsKey(card: Pick<Card, "machine" | "harness" | "cwd">): string;  // machine ?? "", harness, cwd joined by "\u0000"
  export function cachedCommands(key: string): CommandInfo[] | null;
  export function rememberCommands(key: string, list: CommandInfo[]): void;  // localStorage "maya.slashCommands.v1", newest 40 keys kept, every access in try/catch
  ```
- `modal.ts`: the model gains `commands?: CommandInfo[]`. The composer, when `caps.slashLines`, calls `attachSlashMenu(ta)`, appends its `element` to `form`, seeds it with `m.commands ?? []`, and keeps it in a module-level `slash` (cleared by `closeModal`). The `keydown` listener becomes `if (slash?.onKey(ev)) return; composerKey(ev, ta, trySend);`. `openModal` calls `loadCommands(card)`: (1) `cachedCommands(key)`; (2) if none, `list_commands({ machine: card.machine ?? "" })` → this harness's list, used only if step 3 has not answered yet; (3) `session_commands({ sessionId })` → set and `rememberCommands`; errors ignored. Each set is skipped when `current !== me`; setting writes `me.model.commands` and calls `slash?.setCommands`.

- [ ] **Step 1: Write the failing tests**

```ts
// slash-cache.test.ts
it("remembers per machine, agent and folder", () => {});
it("keeps the newest 40 lists", () => {});
it("reads nothing, and throws nothing, when storage throws", () => {});
// modal slash tests (mock invoke as the other modal tests do)
it("shows the cached list, then the session's own list", async () => {});
it("falls back to the machine's shared list when nothing is cached", async () => {});
it("a list that lands after the window closed changes nothing on screen", async () => {});
it("Escape with the list open keeps the window open", () => {});
it("the list survives a repaint of the window", () => { /* patchModal keeps .slash and its highlight */ });
it("no list for an agent without slash lines, nor while the question menu shows", () => {});
```

- [ ] **Step 2: Run to see them fail** — `pnpm test src/slash-cache.test.ts src/modal` → FAIL.
- [ ] **Step 3: Implement** `src/slash-cache.ts` and the `modal.ts` changes above. Update the composer placeholder only if it now promises something untrue (it does not need to mention the list).
- [ ] **Step 4: Run all frontend tests** — `pnpm test` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: session windows suggest their commands, cached per project"`

---

### Task 8: Version and release notes

**Files:** version files via the script, `RELEASE_NOTES.md`

- [ ] **Step 1:** `git fetch origin main` and read `main`'s version; bump its minor with `sh scripts/set-version.sh <x.y+1.0>`.
- [ ] **Step 2:** Add at the top of `RELEASE_NOTES.md`:

```markdown
# <version>

## New

- Type `/` in a session's message box to see the commands that session has: the agent's own, your skills and commands, your plugins', and the project's, each with what it does. Tab or Enter fills one in. It works for sessions on your other machines, and on the phone.
```

- [ ] **Step 3:** `cargo test --workspace && pnpm test` → PASS.
- [ ] **Step 4:** `git commit -m "chore(release): <version>"` with both changes.

---

## Appendix: built-in commands

Descriptions are short; leave one empty where none is given.

**Claude Code:** add-dir (Add a working folder) · agents (Manage subagents) · bug (Report a bug) · clear (Start a new conversation) · code-review (Review the current diff) · compact (Clear history, keep a summary) · config (Open settings) · context (Show context use) · copy (Copy the last reply) · cost (Show usage) · diff (Show changes) · doctor (Check the installation) · effort (Set reasoning effort) · exit (Quit) · export (Export the conversation) · fast (Toggle fast mode) · feedback (Send feedback) · fork (Fork the conversation) · goal (Set a goal) · help (Show help) · hooks (Manage hooks) · init (Create CLAUDE.md) · loop (Run a prompt on an interval) · mcp (Manage MCP servers) · memory (Edit memory) · model (Choose the model) · output-style (Set the output style) · permissions (Manage permissions) · plan (Plan mode) · plugin (Manage plugins) · release-notes (Show release notes) · reload-plugins (Reload plugins) · reload-skills (Reload skills) · rename (Rename the session) · resume (Resume a session) · rewind (Rewind the conversation) · schedule (Schedule a cloud agent) · security-review (Review pending changes for security) · simplify (Clean up changed code) · skills (List skills) · status (Show status) · statusline (Set up the status line) · stop (Stop) · tasks (List background tasks) · theme (Change the theme) · usage (Show usage and limits) · workflows (Manage workflows).

**Codex:** model (Choose model and effort) · permissions · skills (Pick a skill) · review (Review changes) · rename · new (New session) · resume · fork · init (Create AGENTS.md) · compact (Summarise the conversation) · diff · mention (Mention a file) · status · usage · mcp · copy · export · logout · quit · clear · agents · stop.

**OpenCode:** agents (Switch agent) · clear (Clear session) · compact (Compact session) · copy · diff · editor (Open editor) · exit · export · fork · help · models (Switch model) · new (New session) · redo · rename · sessions (Resume a session) · skills · stats (Usage statistics) · themes · undo · variants (Switch model variant).

**Kiro:** help · quit · clear · compact · chat · context · model · effort · agent · plan · tools · permissions · prompts · mcp · hooks · usage · editor · copy · rewind · todos.

**Grok:** new (New conversation) · resume · compact · context · fork · rewind · copy · export · quit · rename · model · effort · plan · memory · hooks · plugins · skills · loop · goal · usage · settings · doctor.

**Antigravity:** clear (New conversation) · resume · fork · rewind · rename · exit · model · effort · config · permissions · plan · goal · agents · mcp · plugin · skills · hooks · tasks · diff · context · copy · usage · compact · help.
