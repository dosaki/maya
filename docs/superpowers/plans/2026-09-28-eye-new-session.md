# Eye New Session Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reorder the columns and add a "+" launcher that starts a new Claude session in a chosen or Claude-picked project folder with a first prompt.

**Architecture:** `config.rs` gains `projects_dir`; new `launch.rs` holds pure helpers (folder listing, prompt template, reply matching, AppleScript, env cleaning, binary lookup) and the two effectful steps (classifier run with timeout, Terminal launch); `lib.rs` adds `list_project_dirs` and `start_session`. Frontend: `types.ts` column order, `board.ts` "+" button, `newsession.ts` modal, `settings.ts` field, `main.ts` wiring.

**Tech Stack:** unchanged.

**Spec:** `docs/superpowers/specs/2026-09-28-eye-new-session-design.md`

## Global Constraints

- Classifier args exactly: `-p --strict-mcp-config --disable-slash-commands --model haiku --output-format text --no-session-persistence --max-turns 1 <prompt>`; never `--bare`.
- Spawned processes get Eye's env minus `ANTHROPIC_API_KEY`, `CLAUDECODE`, and any variable starting with `CLAUDE_CODE_`.
- Classifier timeout 90 s; fallback is always the projects directory.
- The launch command is `cd '<dir>' && claude "$(cat '<file>')"` in a new Terminal window, then `activate`.
- Commit per task with the usual trailer.

## Review Focus

1. Reply like `` `sonarqube` `` or `sonarqube.` or `Sonarqube` matches; `NONE`, empty, or a sentence does not. Test in Task 2.
2. A folder name containing `'` (e.g. `it's-here`) launches without breaking the shell command. Test in Task 2 (`applescript_launch`).
3. The classifier hanging: after 90 s the process is killed and the fallback is used, not an error. Test in Task 2 with a fake slow binary (a shell script that sleeps) and a 1 s timeout.
4. `start_session` with `dir` naming a folder outside the list (`../..`): refused. Test in Task 3.
5. Start clicked twice quickly: the second click is ignored while busy. Test in Task 4.

---

### Task 1: Column order, projects directory setting

**Files:**
- Modify: `src/types.ts`, `src/board.test.ts`, `src-tauri/src/config.rs`, `src-tauri/src/lib.rs`, `src/settings.ts`, `src/settings.test.ts`

**Interfaces:**
- `COLUMNS` order: idle, working, awaiting, completed.
- `Config { completed_timeout_minutes, projects_dir: Option<String> }` (JSON `projectsDir`, `#[serde(default)]`), `config::expand_home(s: &str) -> PathBuf`, `Config::projects_dir_path() -> Option<PathBuf>`.
- `set_config` refuses a non-directory with "Projects directory does not exist: <path>."
- Settings model gains `projectsDir: string`; handler `onProjectsDir(path: string)`; input `input[name=projectsDir]`.

- [ ] **Step 1: Failing tests.** `board.test.ts`: column order and titles become `["idle","working","awaiting","completed"]` / `["Idle","Working","Awaiting Decision","Completed"]`, counts `["1","2","1","1"]`, and `cols[1]` still holds `a, e`. `config.rs`: `expand_home("~/dev")` ends with `/dev` and starts with the home dir; `projects_dir` round-trips as `projectsDir`; missing key loads as `None`. `settings.test.ts`: renders `input[name=projectsDir]` with the model value and reports `onProjectsDir("~/dev")` on change. Run → RED.
- [ ] **Step 2: Implement.** Reorder `COLUMNS`; config field + helpers; `set_config` validates with `expand_home` and `is_dir()`; settings renders the field (label "Projects directory", placeholder `~/dev`) and `initSettings` loads/saves it via `set_config` with both fields.
- [ ] **Step 3: Verify, commit** `feat: reorder columns and add projects directory setting`.

---

### Task 2: Launch helpers in Rust

**Files:**
- Create: `src-tauri/src/launch.rs`
- Modify: `src-tauri/src/lib.rs` (`pub mod launch;`)

**Interfaces:**
- `launch::list_project_dirs(root: &Path) -> Vec<String>`
- `launch::classifier_prompt(user_prompt: &str, dirs: &[String]) -> String`
- `launch::pick_dir(reply: &str, dirs: &[String]) -> Option<String>`
- `launch::clean_env(vars: impl Iterator<Item = (String, String)>) -> Vec<(String, String)>`
- `launch::claude_binary() -> Option<PathBuf>`
- `launch::classify(binary: &Path, root: &Path, user_prompt: &str, dirs: &[String], timeout: Duration) -> Option<String>` (None on NONE/no match/timeout/error)
- `launch::write_prompt_file(eye_dir: &Path, prompt: &str) -> Result<PathBuf, String>`
- `launch::shell_single_quote(s: &str) -> String`, `launch::applescript_launch(target: &Path, prompt_file: &Path) -> String`
- `launch::open_terminal(target: &Path, prompt_file: &Path) -> Result<(), String>`
- `launch::CLASSIFIER_TIMEOUT: Duration = 90 s`

- [ ] **Step 1: Failing tests** covering: listing (temp dir with `b/`, `a/`, `.hidden/`, `file.txt` → `["a","b"]`); prompt template contains the user prompt between `==========` lines and `a, b`; `pick_dir` cases from Review Focus 1; `clean_env` drops the three kinds of variables and keeps `PATH`; `shell_single_quote("it's")` → `'it'\''s'`; `applescript_launch` contains `do script`, the quoted `cd`, `claude \"$(cat`, `activate`, and escapes `"`/`\`; `classify` against a fake binary: a temp script that prints `b` → `Some("b")`, a script that sleeps 5 s with a 1 s timeout → `None` and finishes in under 3 s, a script that prints `NONE` → `None`; `write_prompt_file` creates the file with the exact content under `eye_dir/prompts`.
- [ ] **Step 2: Implement.** `classify` spawns with `Command::new(binary).args([...]).arg(classifier_prompt(...)).current_dir(root).env_clear().envs(clean_env(std::env::vars())).stdout(piped).stderr(null)`, polls `try_wait` every 100 ms until the deadline, kills on timeout, reads stdout, returns `pick_dir`. `claude_binary` checks the three candidates then `zsh -lc 'command -v claude'`. `open_terminal` runs `osascript -e` with `applescript_launch`.
- [ ] **Step 3: Verify, commit** `feat: launch helpers for new sessions`.

---

### Task 3: Commands

**Files:**
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- `list_project_dirs() -> Result<Vec<String>, String>` (error "Set a projects directory in Settings first." when unset; "Projects directory does not exist: <path>." when missing)
- `start_session(dir: Option<String>, prompt: String) -> Result<StartResult, String>`; `StartResult { dir: String, how: String }` with `how` in `chosen | classifier | fallback`.

- [ ] **Step 1: Failing test.** In `launch.rs`, a pure `resolve_target(root: &Path, dirs: &[String], dir: Option<&str>, picked: Option<&str>) -> Result<(PathBuf, &'static str), String>` with tests: chosen valid → `(root/dir, "chosen")`; chosen not in list (`"../.."`) → Err containing "not in the projects directory"; none + picked `Some("b")` → `(root/b, "classifier")`; none + `None` → `(root, "fallback")`.
- [ ] **Step 2: Implement** `resolve_target` and the two commands (async); `start_session` reads config, lists dirs, resolves (running `classify` only when `dir` is None), writes the prompt file, opens Terminal, returns the result. Blank prompt → "Type a prompt first."
- [ ] **Step 3: Verify, build, commit** `feat: list_project_dirs and start_session commands`.

---

### Task 4: "+" button and new-session modal

**Files:**
- Create: `src/newsession.ts`, `src/newsession.test.ts`
- Modify: `src/board.ts`, `src/board.test.ts`, `src/main.ts`, `src/styles.css`, `index.html`

**Interfaces:**
- `board.ts`: Idle column header gets `button.column__add[data-action=new-session]` with text `+` and title "New session"; other columns have none.
- `newsession.ts`: `NewSessionModel { dirs: string[]; dir: string | null; prompt: string; status: { ok: boolean; text: string } | null; busy: boolean; needsSetup: boolean }`, `NewSessionHandlers { onStart(dir: string | null, prompt: string): void; onClose(): void; onOpenSettings(): void }`, `renderNewSession(m, h): HTMLElement` (root `.modal` reusing modal styles, `select[name=dir]`, `textarea[name=prompt]`, `button[data-action=start]`), `openNewSession(): Promise<void>`, `closeNewSession(): void`.
- `main.ts`: board click on `[data-action=new-session]` → `openNewSession()`; `#modal-host` is shared, so opening one modal closes the other.

- [ ] **Step 1: Failing tests.** `board.test.ts`: only the idle column has the add button. `newsession.test.ts`: dropdown has "Let Claude choose" first then dirs; Start disabled when prompt blank and enabled after typing; click calls `onStart(null, "fix ci")` (and `onStart("sonarqube", …)` when selected); Cmd+Enter starts; `busy: true` disables Start and shows the status; `needsSetup: true` renders the hint and an Open Settings button calling `onOpenSettings`; a second Start click while busy does not call `onStart` again (Review Focus 5).
- [ ] **Step 2: Implement** the pure renderer, the stateful open/close (loads dirs via `list_project_dirs`, maps the "Set a projects directory" error to `needsSetup`), `onStart` → `busy = true`, status "Choosing a repository…" when `dir` is null, `invoke("start_session")`, status per `how`, close after 1.5 s; errors → status red, `busy = false`. `main.ts` wiring; `onOpenSettings` toggles the settings panel by clicking `#settings-toggle`. Styles for `.column__add` and the form.
- [ ] **Step 3: Verify, build.** Manual: set `~/dev` in Settings; "+" → choose a folder and a prompt → Terminal window opens in that folder with the prompt; again with "Let Claude choose" and a prompt mentioning a specific project → status names the picked folder.
- [ ] **Step 4: Commit** `feat: start a new session from the board`.
