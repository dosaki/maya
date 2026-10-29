# Session Usage and Limits Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every session window shows its context, the account's limit windows and the session's cost, for all six agents, local and remote, with the terminal look's bottom line showing the real Claude Code status line.

**Architecture:** Each agent's module in `core/` reads tokens and cost the way it reads context today, a status line wrapper records Claude Code's payload per session, and spawn-based collectors (Antigravity, Kiro, Grok) run off the store lock on a "due" list the way PR lookups do. Everything lands on three new optional `Card` fields and travels to the main inside the card. The frontend draws a header strip from those fields, shrinking in four stages by the header's width.

**Tech Stack:** Rust (workspace crates `core`, `src-tauri`, `cli`, `hook`), serde_json, vanilla TypeScript with vitest, POSIX `sh` + `jq` for the wrapper.

**Spec:** `docs/superpowers/specs/2026-10-29-session-usage-and-limits-design.md`

## Global Constraints

- New `Card` fields are optional, `#[serde(default, skip_serializing_if = "Option::is_none")]`, so mixed Maya versions keep talking.
- `Card` keeps deriving `Eq`: money is stored as integer micro-units (`u64`, millionths of a dollar or credit), never `f64`.
- Nothing added to `Store::refresh` spawns a process or reads a whole file: spawns run from due lists on their own threads; transcript totals are read incrementally under a byte budget per refresh.
- Maya never reads a sign-in token or credential file. The account label comes from `.claude.json`'s `oauthAccount.emailAddress` only.
- The wrapper prints exactly what the inner status line command printed, and only that.
- Estimated dollars are labelled "est."; Claude Code's own `total_cost_usd` and Grok's and OpenCode's own figures are not.
- Shrink stage thresholds on the header's width: wide ≥ 1100 px, medium ≥ 850, narrow ≥ 650, chip below.
- Price values in `core/src/pricing.rs` are copied from the providers' pricing pages on the day they are written, with that date in a comment. Never from memory.
- Each task ends in a commit. Commit messages follow Conventional Commits (`feat(scope):`, `test(scope):`, `docs:`), with the attribution line `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.
- Run Rust tests with `cargo test -p maya-core` (the whole workspace needs the ear sidecar built); frontend tests with `pnpm test`.

## Review Focus

1. A transcript that is truncated or replaced mid-session (a `/clear`, or Claude Code rewriting the file) must restart the running totals, not add onto them. Test in Task 3.
2. A status line payload whose `session_id` is not a UUID (or is missing) must record nothing and still print the inner command's output. Test in Task 4.
3. A Codex rollout whose `rate_limits` is `null` (proxied or `exec`) must yield tokens with no limits, never a 0% window. Test in Task 8.
4. A header narrower than any threshold on a card with no limits and no cost (a plain Grok card at the chip stage) must still render context and nothing broken. Test in Task 14.
5. A recorded status line with an escape sequence that is not SGR (cursor moves, OSC titles) must render as plain text with the sequence removed, never as visible garbage or injected markup. Test in Task 15.

---

## File structure

New:
- `core/src/usage.rs` — the shared types (`SessionUsage`, `Cost`, `CostUnit`, `AccountLimits`, `LimitWindow`, `TokenCounts`), the incremental transcript totals, the process runner with a timeout.
- `core/src/pricing.rs` — the price table and `cost_micros`.
- `core/src/statusline.rs` — the wrapper script, install/remove/status, the recorded payload files.
- `core/fixtures/statusline/payload.json`, `core/fixtures/codex/token_count.jsonl`, `core/fixtures/antigravity/usage.json`, `core/fixtures/grok/usage.json`, `core/fixtures/kiro/usage-card.txt` (from the spike).
- `src/usage.ts`, `src/usage.test.ts` — the strip, stages, wording, tooltips.
- `src/ansi.ts`, `src/ansi.test.ts` — SGR rendering.

Modified:
- `core/src/model.rs`, `src/types.ts` — `Card` fields.
- `core/src/foreign.rs` — `ForeignTail` fields and `derive`.
- `core/src/codex.rs`, `core/src/kiro.rs`, `core/src/grok.rs`, `core/src/antigravity.rs`, `core/src/opencode.rs` — per-agent readers.
- `core/src/store.rs` — caches, due lists, setters, card stamping.
- `core/src/lib.rs` — module list.
- `hook/src/main.rs` — the `statusline` mode (Windows wrapper).
- `src-tauri/src/lib.rs` — commands and the usage poll thread.
- `cli/src/args.rs`, `cli/src/commands.rs`, `cli/src/lib.rs`, `cli/src/run_cmd.rs` — the `statusline` subcommand and the poll thread.
- `src/settings.ts`, `src/modal.ts`, `src/styles.css` — UI.
- `docs/DEVELOPING.md`, `README.md`.

---

### Task 1: Card fields and shared types

**Files:**
- Create: `core/src/usage.rs`
- Modify: `core/src/lib.rs`, `core/src/model.rs:113-160`, `src/types.ts:150-200`
- Modify (add `usage: None, limits: None, status_line: None` to every `Card` literal): `core/src/state.rs:308`, `core/src/foreign.rs:66`, `core/src/opencode.rs:555`, `core/src/model.rs:255`, `core/src/notify.rs:897,993`, `core/src/actions.rs:1040`, `core/src/answer.rs:298`, `core/src/pending_names.rs:219`, `core/src/launch.rs:1187`, `core/src/interpreter.rs:572`
- Test: `core/src/usage.rs` (inline `mod tests`), `core/src/model.rs` tests

**Interfaces:**
- Produces, in `core/src/usage.rs`:
  ```rust
  #[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
  #[serde(rename_all = "camelCase")]
  pub struct TokenCounts { pub input: u64, pub cached_input: u64, pub cache_write: u64, pub output: u64 }
  #[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
  #[serde(rename_all = "lowercase")]
  pub enum CostUnit { Usd, Credits }
  #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
  #[serde(rename_all = "camelCase")]
  pub struct Cost { pub micros: u64, pub unit: CostUnit, pub estimated: bool, #[serde(default)] pub models: Vec<String> }
  #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
  #[serde(rename_all = "camelCase")]
  pub struct SessionUsage { #[serde(flatten)] pub tokens: TokenCounts, pub cost: Option<Cost> }
  #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
  #[serde(rename_all = "camelCase")]
  pub struct LimitWindow { pub label: String, pub percent: u8, pub resets_at: Option<u64> }
  #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
  #[serde(rename_all = "camelCase")]
  pub struct AccountLimits { pub account: Option<String>, pub windows: Vec<LimitWindow>, pub read_at: u64 }
  /// "5h" for 300 minutes, "7d" for 10080, else "<n>h" (whole hours) or "<n>d" (whole days, when ≥ 48 h and divisible by 1440).
  pub fn window_label(minutes: u64) -> String;
  ```
- Produces on `Card` (`core/src/model.rs`): `pub usage: Option<usage::SessionUsage>`, `pub limits: Option<usage::AccountLimits>`, `pub status_line: Option<String>`, each `#[serde(default, skip_serializing_if = "Option::is_none")]`.
- Produces in `src/types.ts`: `TokenCounts`, `Cost`, `SessionUsage`, `LimitWindow`, `AccountLimits` interfaces mirroring the above (camelCase: `cachedInput`, `cacheWrite`, `resetsAt`, `readAt`), and on `Card`: `usage?: SessionUsage | null; limits?: AccountLimits | null; statusLine?: string | null;`.

- [ ] **Step 1: Write the failing tests** in `core/src/usage.rs`:
  - `window_labels_name_the_known_windows_and_fall_back_to_hours_or_days`: `window_label(300) == "5h"`, `window_label(10080) == "7d"`, `window_label(60) == "1h"`, `window_label(2880) == "2d"`, `window_label(90) == "1h"` (whole hours, rounded down).
  - `session_usage_round_trips_with_flattened_tokens`: serialize `SessionUsage { tokens: TokenCounts { input: 1, cached_input: 2, cache_write: 3, output: 4 }, cost: Some(Cost { micros: 4_120_000, unit: CostUnit::Usd, estimated: true, models: vec!["claude-opus-5-5".into()] }) }` and assert the JSON has top-level `input`, `cachedInput`, `cacheWrite`, `output`, and `cost.unit == "usd"`; deserialize back equal.
  In `core/src/model.rs` tests add `a_card_without_usage_fields_deserializes_with_none_and_serializes_none_of_them`: take the existing round-trip fixture JSON, assert `usage`, `limits`, `status_line` are `None`, and that `serde_json::to_value` has no `usage`, `limits` or `statusLine` keys.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p maya-core usage:: model::` — Expected: compile errors (module and fields missing).

- [ ] **Step 3: Implement** `core/src/usage.rs` with the types and `window_label`, add `pub mod usage;` to `core/src/lib.rs`, add the three fields to `Card`, and add `usage: None, limits: None, status_line: None` to every literal listed above. Mirror the types in `src/types.ts`.

- [ ] **Step 4: Run tests** — `cargo test -p maya-core` and `pnpm test` — Expected: all pass (`pnpm test` compiles the new TS types through existing tests).

- [ ] **Step 5: Commit** — `feat(core): usage, limits and status line fields on the card`

---

### Task 2: The price table

**Files:**
- Create: `core/src/pricing.rs`
- Modify: `core/src/lib.rs`
- Test: inline

**Interfaces:**
- Produces:
  ```rust
  /// Dollars per million tokens.
  pub struct Price { pub input: f64, pub cached_input: f64, pub cache_write: f64, pub output: f64, /// The rates past 200k input tokens on the request, when the model has them.
  pub long: Option<(f64, f64, f64, f64)> }
  /// The row for a model id: `[1m]` suffix dropped, then the longest table prefix that matches (`claude-opus-5-5-20260301` → the `claude-opus-5-5` row; `gpt-6.1-sol` → its row).
  pub fn price_for(model: &str) -> Option<&'static Price>;
  /// Micro-dollars for one request's counts; `long` picks the over-200k rates when the row has them. None when the model has no row.
  pub fn cost_micros(model: &str, t: &crate::usage::TokenCounts, long: bool) -> Option<u64>;
  /// True when a request's input + cached + cache-write tokens exceed 200_000.
  pub fn is_long(t: &crate::usage::TokenCounts) -> bool;
  ```
  The table covers every Claude model Claude Code can run (the current Fable, Opus, Sonnet and Haiku generations and the previous one) and every Codex model (`gpt-*` ids seen in `~/.codex/sessions` `turn_context` lines and the Codex docs). Values are copied from https://docs.anthropic.com/en/docs/about-claude/pricing and https://openai.com/api/pricing on the day; the file's header comment names that date.

- [ ] **Step 1: Write the failing tests**:
  - `a_dated_model_id_matches_its_family_row_and_1m_suffix_is_ignored`: `price_for("claude-opus-5-5-20260301").is_some()`, `price_for("claude-opus-5-5[1m]")` equals `price_for("claude-opus-5-5")` by pointer, `price_for("nonsense-9").is_none()`.
  - `cost_is_the_sum_of_the_four_rates_in_micros`: pick the `claude-opus-5-5` row `p`; for `TokenCounts { input: 1_000_000, cached_input: 2_000_000, cache_write: 500_000, output: 100_000 }` assert `cost_micros(...)` equals `((p.input + 2.0 * p.cached_input + 0.5 * p.cache_write + 0.1 * p.output) * 1e6).round() as u64`.
  - `long_context_uses_the_long_rates_only_when_the_row_has_them`: a row with `long: Some(..)` gives a larger number with `long = true`; a row with `long: None` gives the same number either way.
  - `is_long_counts_input_cached_and_cache_write`: 150k + 60k cached → true; 200_000 exactly → false.

- [ ] **Step 2: Run to verify they fail** — `cargo test -p maya-core pricing::` — Expected: module missing.

- [ ] **Step 3: Implement** `core/src/pricing.rs` (a `static TABLE: &[(&str, Price)]`, longest-prefix match) and `pub mod pricing;`.

- [ ] **Step 4: Run** — `cargo test -p maya-core pricing::` — Expected: PASS.

- [ ] **Step 5: Commit** — `feat(core): list-price table for Claude and Codex models`

---

### Task 3: Incremental transcript totals for Claude Code

**Files:**
- Modify: `core/src/usage.rs`
- Test: inline, using a tempdir

**Interfaces:**
- Produces, in `core/src/usage.rs`:
  ```rust
  #[derive(Clone, Debug, Default, PartialEq, Eq)]
  pub struct Totals { pub tokens: TokenCounts, pub cost_micros: u64, /// Models with a price row, in first-seen order.
  pub priced: Vec<String>, /// Models without one.
  pub unpriced: Vec<String> }
  #[derive(Default)]
  pub struct TranscriptTotals { /* path → (bytes consumed, carried partial line, Totals) */ }
  impl TranscriptTotals {
      /// Reads what was appended to `path` since the last call, at most `budget` bytes, and returns the totals so far. A file shorter than what was consumed starts over from zero.
      pub fn get(&mut self, path: &Path, budget: u64) -> Totals;
      pub fn retain(&mut self, live: &[PathBuf]);
  }
  /// Per refresh, across all transcripts.
  pub const TOTALS_BUDGET_BYTES: u64 = 8 * 1_048_576;
  /// One main-chain assistant record's counts: `message.usage` of a record whose `type` is `assistant`, `isSidechain` is not true, and `message.model` is non-empty and not `<synthetic>`.
  pub fn record_counts(v: &serde_json::Value) -> Option<(String, TokenCounts)>;
  /// `Totals` as the card's usage: estimated, with `priced` as `models`; `cost: None` when nothing was priced.
  pub fn session_usage_of(t: &Totals) -> SessionUsage;
  ```
  Each record's cost is `pricing::cost_micros(model, &counts, pricing::is_long(&counts))`, summed per record (so the over-200k rates apply per request, as the spec says).

- [ ] **Step 1: Write the failing tests**:
  - `record_counts_skips_synthetic_sidechain_and_user_records`: three JSON lines (a real assistant record with `input_tokens: 10, cache_read_input_tokens: 1000, cache_creation_input_tokens: 90, output_tokens: 5`; one with model `<synthetic>`; one with `isSidechain: true`); only the first yields `Some(("claude-opus-5-5", TokenCounts { input: 10, cached_input: 1000, cache_write: 90, output: 5 }))`.
  - `totals_grow_with_appended_lines_and_restart_when_the_file_shrinks`: write two assistant lines, `get` → tokens summed; append a third → `get` adds it (assert `consumed` advanced by checking the totals, not internals); truncate the file to one line → `get` equals that one line's counts.
  - `a_partial_last_line_is_carried_to_the_next_read`: write a line and half of a second without a trailing newline; `get` counts one; append the rest and the newline; `get` counts two.
  - `the_budget_caps_one_read_and_the_rest_comes_next_time`: a file of 3 lines; `get(path, <bytes of the first line + 1>)` counts one; a second `get` with a large budget counts three.
  - `cost_is_summed_per_record_with_long_rates_per_request`: two records of a model with long rates, one with 150k input and one with 250k; `cost_micros` equals the sum of the two per-record `pricing::cost_micros` calls with `long` false then true; `priced == ["<model>"]`.
  - `an_unpriced_model_is_listed_and_contributes_no_cost`: a record for `made-up-model` → `unpriced == ["made-up-model"]`, `cost_micros == 0`, `session_usage_of(..).cost.is_none()`.

- [ ] **Step 2: Run to verify they fail** — `cargo test -p maya-core usage::` — Expected: functions missing.

- [ ] **Step 3: Implement** in `core/src/usage.rs`. Read with `File::seek` to the consumed offset, `take(budget)`, split on `\n`, keep the tail after the last newline as the carry. Detect shrink by `metadata().len() < consumed`.

- [ ] **Step 4: Run** — `cargo test -p maya-core usage::` — Expected: PASS.

- [ ] **Step 5: Commit** — `feat(core): incremental token totals and estimated cost from Claude transcripts`

---

### Task 4: The status line wrapper and its install

**Files:**
- Create: `core/src/statusline.rs`, `core/fixtures/statusline/payload.json`
- Modify: `core/src/lib.rs`, `hook/src/main.rs`
- Test: inline (the script test is `#[cfg(unix)]` and runs `sh`)

**Interfaces:**
- Consumes: `usage::{AccountLimits, LimitWindow, window_label}`.
- Produces, in `core/src/statusline.rs`:
  ```rust
  /// The wrapper script (POSIX sh + jq). Reads stdin into a temp file, runs the inner command from `$HOME/.claude/maya/statusline-inner` (if the file is non-empty) with the payload on stdin capturing stdout, writes `$HOME/.claude/maya/usage-<session_id>.json` atomically (tmp + mv) when `.session_id` matches `^[0-9a-fA-F-]{36}$`, then prints the captured output. Exits 0 always.
  pub const WRAPPER_SCRIPT: &str;
  pub const WRAPPER_MARKER: &str = ".claude/maya/maya-statusline";        // unix
  pub const WRAPPER_EXE_MARKER: &str = ".claude/maya/maya-hook.exe\" statusline"; // windows command suffix
  pub const INNER_FILE: &str = "statusline-inner";
  /// Where the managed settings file lives on this platform.
  pub fn managed_settings_path() -> PathBuf;
  #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
  #[serde(rename_all = "camelCase")]
  pub struct Status { pub installed: bool, /// A file that overrides the user's status line, naming why install is refused.
  pub blocked_by: Option<String> }
  pub fn status(claude_dir: &Path) -> Result<Status, String>;
  /// Installs into `settings.local.json` when it holds a `statusLine`, else `settings.json`; refuses with `Err` naming the managed file when it holds one. Saves the previous `command` to `maya/statusline-inner` (empty file when there was none; an existing inner file is kept when the current command is already the wrapper). Carries every other key of the entry over. Writes the script (0755) on unix. Backs the settings file up like `hook_install` does.
  pub fn install_to(claude_dir: &Path) -> Result<Status, String>;
  /// Restores the inner command (or removes `statusLine` when the inner file is empty), deletes the inner file, the script and every `maya/usage-*.json`.
  pub fn remove_from(claude_dir: &Path) -> Result<Status, String>;
  /// What the wrapper writes, and what Maya reads back.
  #[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
  #[serde(rename_all = "camelCase")]
  pub struct Recorded { pub at: u64, pub session_id: String, pub model: Option<String>, pub version: Option<String>, pub context_used_percent: Option<u8>, pub context_window_size: Option<u64>, pub rate_limits: Option<serde_json::Value>, pub total_cost_usd: Option<f64>, pub line: String }
  /// Builds a `Recorded` from a payload and the captured line (the Windows binary's path and the tests' reference for the script).
  pub fn recorded_from(payload: &serde_json::Value, line: &str, now_ms: u64) -> Option<Recorded>;
  /// Writes `maya/usage-<id>.json` atomically.
  pub fn write_recorded(maya_dir: &Path, r: &Recorded) -> std::io::Result<()>;
  pub fn read_recorded(maya_dir: &Path, session_id: &str) -> Option<Recorded>;
  /// The newest recording in the folder by `at`.
  pub fn newest_recorded(maya_dir: &Path) -> Option<Recorded>;
  /// `rate_limits` → windows: `five_hour` → "5h", `seven_day` → "7d", any other key → the key with `_` as space; `used_percentage` rounded and capped at 100; `resets_at` in seconds (a number > 4_102_444_800 is millis and is divided by 1000; an ISO string is parsed with `codex::ms_of`). `None` when `rate_limits` is absent; `Some` with no windows when it is an empty object.
  pub fn limits_of(r: &Recorded, account: Option<String>) -> Option<AccountLimits>;
  ```
- Produces in `hook/src/main.rs`: when the first argument is `statusline`, read stdin, run the inner command (`sh -c <inner>` with the payload on stdin, stdout captured, 5 s cap) when `maya/statusline-inner` is non-empty, `write_recorded(recorded_from(..))`, print the captured output, exit 0.

- [ ] **Step 1: Save the fixture** `core/fixtures/statusline/payload.json`: a real-shaped payload with `session_id` `"4f1c2a7e-0b1d-4c3e-9a8f-1234567890ab"`, `model.display_name "Opus 5.5"`, `model.id "claude-opus-5-5"`, `version "2.1.99"`, `context_window {context_window_size: 1000000, used_percentage: 12, current_usage: {...}}`, `rate_limits {five_hour: {used_percentage: 78.4, resets_at: 1793275200}, seven_day: {used_percentage: 41, resets_at: 1793700000}}`, `cost {total_cost_usd: 4.1234}`.

- [ ] **Step 2: Write the failing tests**:
  - `recorded_from_keeps_the_fields_maya_reads`: from the fixture and line `"\u{1b}[1mOpus\u{1b}[0m"` → `session_id`, `model == Some("claude-opus-5-5")`, `context_used_percent == Some(12)`, `context_window_size == Some(1_000_000)`, `total_cost_usd == Some(4.1234)`, `line` as given; a payload without `session_id`, or with `"session_id": "../x"`, → `None`.
  - `limits_of_maps_the_two_windows_and_accepts_millis_or_iso_resets`: fixture → windows `[("5h", 78, Some(1793275200)), ("7d", 41, Some(1793700000))]`; `resets_at: 1793275200000` → `Some(1793275200)`; `resets_at: "2026-10-29T12:00:00Z"` → `Some(1793275200)`; no `rate_limits` → `None`; `rate_limits: {}` → `Some` with empty windows; `used_percentage: 140.0` → 100.
  - `write_and_read_recorded_round_trip_and_newest_wins`: write two records with `at` 1 and 2 → `read_recorded` each; `newest_recorded` is the one with `at == 2`.
  - `install_wraps_the_existing_command_and_keeps_its_other_keys`: a temp claude dir with `settings.json` `{"statusLine":{"type":"command","command":"sh ~/sl.sh","refreshInterval":60,"padding":1},"model":"opus"}` → after `install_to`: `statusLine.command` contains `WRAPPER_MARKER` (unix) or `WRAPPER_EXE_MARKER` (windows), `refreshInterval == 60`, `padding == 1`, `model == "opus"`, `maya/statusline-inner` reads `sh ~/sl.sh`, `status().installed`; a second `install_to` leaves the inner file unchanged.
  - `install_without_a_status_line_writes_an_empty_inner_and_remove_removes_the_entry`: no `statusLine` → inner file exists and is empty; `remove_from` → no `statusLine` key, inner file gone, script gone, a `maya/usage-x.json` written beforehand is gone.
  - `install_prefers_settings_local_when_it_holds_the_status_line`: `settings.local.json` with a `statusLine` and `settings.json` without → the wrapper lands in `settings.local.json`; `remove_from` restores it there.
  - `status_is_blocked_by_a_managed_status_line`: make `managed_settings_path` overridable for tests (a `pub(crate) fn status_in(claude_dir, managed: &Path)`); a managed file with a `statusLine` → `Status { installed: false, blocked_by: Some(<managed path>) }` and `install_to` returns `Err` containing that path.
  - `#[cfg(unix)] the_script_records_the_payload_and_prints_the_inner_output`: tempdir as `HOME`, write the script to `$HOME/.claude/maya/maya-statusline`, inner file `printf 'LINE %s' "$(jq -r .model.id)"`, run `sh <script>` with the fixture on stdin via `std::process::Command` with `env("HOME", ..)` → stdout is `LINE claude-opus-5-5`, and `read_recorded` gives `line == "LINE claude-opus-5-5"`, `total_cost_usd == Some(4.1234)`. Skip (return) when `jq` is not on PATH, printing why.
  - `#[cfg(unix)] the_script_prints_the_inner_output_even_for_a_bad_session_id`: same with `session_id` `"nope"` → stdout `LINE claude-opus-5-5`, no `usage-*.json` written.
  - `#[cfg(unix)] the_script_prints_nothing_and_exits_zero_without_an_inner_command`: empty inner file → empty stdout, exit 0, file recorded.

- [ ] **Step 3: Run to verify they fail** — `cargo test -p maya-core statusline::` — Expected: module missing.

- [ ] **Step 4: Implement** `core/src/statusline.rs` and `pub mod statusline;`. Managed paths: macOS `/Library/Application Support/ClaudeCode/managed-settings.json`, Linux `/etc/claude-code/managed-settings.json`, Windows `C:\ProgramData\ClaudeCode\managed-settings.json`. Reuse `hook_install`'s read/backup/write helpers by making them `pub(crate)` with a file-name parameter. In the script, `jq` builds the record in one call: `jq -c --arg line "$out" --arg t "$t" '{at:($t|tonumber), sessionId:.session_id, model:.model.id, version, contextUsedPercent:(.context_window.used_percentage|if .==null then null else round end), contextWindowSize:.context_window.context_window_size, rateLimits:.rate_limits, totalCostUsd:.cost.total_cost_usd, line:$line}'`.

- [ ] **Step 5: Implement the `statusline` mode** in `hook/src/main.rs`.

- [ ] **Step 6: Run** — `cargo test -p maya-core statusline::` and `cargo build -p maya-hook` — Expected: PASS, builds.

- [ ] **Step 7: Commit** — `feat(core): status line wrapper that records Claude Code's payload per session`

---

### Task 5: Settings button and Tauri commands

**Files:**
- Modify: `src-tauri/src/lib.rs:1370-1392,1640-1660`, `src/settings.ts:90-110,725-770,1870-1880,2120-2140`
- Test: `src/settings.test.ts`

**Interfaces:**
- Consumes: `statusline::{status, install_to, remove_from, Status}`.
- Produces Tauri commands `statusline_status`, `install_statusline`, `remove_statusline`, each `-> Result<statusline::Status, String>` on the store's `claude_dir`.
- Produces in `src/settings.ts`: model field `statusline: { installed: boolean; blockedBy?: string | null } | null`, handlers `onStatuslineInstall`, `onStatuslineRemove`. Copy, in order: while null "Checking status line…"; blocked: `Usage and limits cannot be recorded: the status line is set in <file>.` with the button disabled; installed: `Usage and limits are recorded from Claude Code's status line.` with button `Remove usage status line`; not installed: `Usage and limits are not recorded. Install the status line wrapper to see them in session windows.` with button `Install usage status line`. The block sits right after the Claude hook's button.

- [ ] **Step 1: Write the failing tests** in `src/settings.test.ts`, following the file's existing hook tests: `the_status_line_block_offers_install_remove_or_says_what_blocks_it` renders the Sessions section with each of the four model states and asserts the texts and button `data-action` values `install-statusline` / `remove-statusline`, and that the blocked state's button is disabled.

- [ ] **Step 2: Run to verify they fail** — `pnpm test -- settings` — Expected: FAIL.

- [ ] **Step 3: Implement** the three commands (register them in the `generate_handler!` list), the model field loaded alongside `hook_status` (catching errors to `{ installed: false }`), the block and the handlers calling `invoke("install_statusline")` / `invoke("remove_statusline")` and storing the returned status.

- [ ] **Step 4: Run** — `pnpm test` and `cargo check -p maya` — Expected: PASS.

- [ ] **Step 5: Commit** — `feat(settings): install the usage status line from Settings`

---

### Task 6: The CLI's `statusline` subcommand

**Files:**
- Modify: `cli/src/args.rs`, `cli/src/commands.rs`, `cli/src/lib.rs:30-45`
- Test: inline in `cli/src/args.rs` and `cli/src/commands.rs`

**Interfaces:**
- Produces: `Cmd::Statusline(StatuslineOp)` with `StatuslineOp::{Install, Remove, Status}`, parsed from `maya statusline install|remove|status` (usage string extended on the first line: `<pair|run|status|hooks|statusline|config|start|--version>`); `commands::statusline(claude_dir, op) -> Result<String, String>` printing `status line: installed` / `status line: not installed` / `status line: blocked by <file>`, `status line installed`, `status line removed`. Install and status warn `NO_JQ` like `hooks` does.

- [ ] **Step 1: Write the failing tests**: in `args.rs`, `statusline_parses_its_three_ops_and_rejects_others` (`["statusline","install"]` → `Cmd::Statusline(Install)`, `["statusline"]` → `Err` containing `statusline needs one of`); in `commands.rs`, `statusline_status_reports_installed_state` mirroring `hooks_status_reports_installed_state` with a temp dir (skip the install assertion when `jq` is missing, as the script is still written without it).

- [ ] **Step 2: Run to verify they fail** — `cargo test -p maya-cli statusline` — Expected: FAIL.

- [ ] **Step 3: Implement** the parser arm, the command and the dispatch in `cli/src/lib.rs`.

- [ ] **Step 4: Run** — `cargo test -p maya-cli` — Expected: PASS.

- [ ] **Step 5: Commit** — `feat(cli): maya statusline install|remove|status`

---

### Task 7: Claude Code cards carry limits, cost, context and the status line

**Files:**
- Modify: `core/src/store.rs` (struct, `new`, `refresh` Claude loop), `core/src/statusline.rs` (a small mtime cache)
- Test: `core/src/store.rs` tests (there are temp-dir based store tests to follow)

**Interfaces:**
- Consumes: `statusline::{read_recorded, newest_recorded, limits_of, Recorded}`, `usage::{TranscriptTotals, TOTALS_BUDGET_BYTES, session_usage_of, SessionUsage, Cost, CostUnit}`.
- Produces in `core/src/statusline.rs`:
  ```rust
  /// Recorded files by session, re-read only when their mtime or size changes; `newest()` is the latest `at` among them.
  #[derive(Default)] pub struct RecordedCache { .. }
  impl RecordedCache { pub fn get(&mut self, maya_dir: &Path, session_id: &str) -> Option<Recorded>; pub fn newest(&self) -> Option<Recorded>; pub fn retain(&mut self, ids: &[String]); }
  /// `oauthAccount.emailAddress` from `<claude_dir's parent>/.claude.json`, re-read on change.
  pub fn account_email(claude_dir: &Path) -> Option<String>;
  ```
- Produces in `Store`: fields `recorded: statusline::RecordedCache`, `totals: usage::TranscriptTotals`. In `refresh`'s Claude loop, per card:
  1. `rec = self.recorded.get(maya_dir, &s.session_id)`.
  2. `card.status_line = rec.line` when non-empty.
  3. Context: when `rec` has both `context_used_percent` and `context_window_size`, `card.context = ContextUsage { used: window * pct / 100, window, percent: pct }`; else as today.
  4. Usage: tokens from `self.totals.get(&path, budget_left)` (the budget shared across the loop, `TOTALS_BUDGET_BYTES` per refresh); cost = `rec.total_cost_usd` as `Cost { micros: (usd * 1e6).round() as u64, unit: Usd, estimated: false, models: [] }` when present, else `session_usage_of(&totals).cost`.
  5. After the loop, `limits = self.recorded.newest().and_then(|r| limits_of(&r, account_email(&self.claude_dir)))` applied to every Claude Code card.
  `self.recorded.retain(ids)` and `self.totals.retain(&paths)` beside `self.tails.retain`.

- [ ] **Step 1: Write the failing tests** in `core/src/store.rs` (temp claude dir, a registry entry, a transcript file, and `maya/usage-<id>.json` written with `statusline::write_recorded`):
  - `a_recorded_status_line_fills_limits_cost_context_and_the_line`: assert the Claude card has `status_line == Some(line)`, `limits.windows[0].label == "5h"`, `context == Some(ContextUsage { used: 120_000, window: 1_000_000, percent: 12 })`, `usage.cost == Some(Cost { micros: 4_123_400, unit: Usd, estimated: false, models: vec![] })`, and tokens from the transcript.
  - `without_a_recording_cost_is_estimated_from_the_transcript_and_limits_are_none`: two assistant records of a priced model → `usage.cost.unwrap().estimated`, `limits.is_none()`, `status_line.is_none()`.
  - `the_newest_recording_sets_every_claude_cards_limits`: two sessions, only one with a recording → both cards carry the same `limits`.

- [ ] **Step 2: Run to verify they fail** — `cargo test -p maya-core store::` — Expected: FAIL on the new assertions.

- [ ] **Step 3: Implement** the caches and the stamping.

- [ ] **Step 4: Run** — `cargo test -p maya-core` — Expected: PASS.

- [ ] **Step 5: Commit** — `feat(core): Claude Code cards carry limits, cost and the recorded status line`

---

### Task 8: Codex limits and tokens from the rollout

**Files:**
- Create: `core/fixtures/codex/token_count.jsonl` (two `token_count` lines copied from a real rollout: one with `rate_limits` filled, one with `"rate_limits": null`; plus a `turn_context` line with `payload.model`)
- Modify: `core/src/foreign.rs:24-36,66`, `core/src/codex.rs:42-100`, `core/src/store.rs` (foreign loop)
- Test: inline in `codex.rs` and `store.rs`

**Interfaces:**
- Produces on `ForeignTail`: `pub usage: Option<crate::usage::SessionUsage>`, `pub limits: Option<crate::usage::AccountLimits>`, `pub model: Option<String>`; `foreign::derive` copies `usage`, `limits` and `model` onto the card.
- Produces in `core/src/codex.rs`:
  ```rust
  /// `info.total_token_usage` → counts: input = input_tokens − cached_input_tokens, cached_input = cached_input_tokens, cache_write = cache_write_input_tokens, output = output_tokens (reasoning is inside output).
  pub fn token_counts(p: &Value) -> Option<crate::usage::TokenCounts>;
  /// `rate_limits` → windows from `primary` and `secondary` (skipping a null one), labelled by `window_label(window_minutes)`, `used_percent` rounded, `resets_at` in seconds; `read_at` is the line's timestamp. None when `rate_limits` is null or has no windows.
  pub fn rate_limits(p: &Value, at_ms: u64) -> Option<crate::usage::AccountLimits>;
  ```
  `parse_tail` sets `t.model` from the newest `turn_context`, `t.usage` from the newest `token_count` with counts (cost estimated by `pricing::cost_micros(model, counts, false)` when the model is known, `estimated: true`, `models: [model]`), and `t.limits` from the newest `token_count` whose `rate_limits` is non-null.
- Produces in `Store::refresh`: after the foreign loop, the newest `limits` (by `read_at`) among Codex tails is applied to every Codex card.

- [ ] **Step 1: Write the failing tests**:
  - `token_counts_split_cached_from_input`: fixture line → `TokenCounts { input: 2918, cached_input: 12160, cache_write: 0, output: 5 }` (adjust to the saved line's numbers).
  - `rate_limits_label_windows_by_minutes_and_skip_null`: filled line → `[("5h", 0, Some(..)), ("7d", 1, Some(..))]`; the null line → `None`.
  - `parse_tail_keeps_the_newest_usage_model_and_limits`: the fixture text → `model == Some("gpt-6.1-sol")`, `usage.is_some()`, `limits.is_some()` (from the filled line even though the null line is newer).
  - In `store.rs`: `codex_limits_are_shared_by_every_codex_card` with two foreign Codex sessions whose rollouts differ (one without limits).

- [ ] **Step 2: Run to verify they fail** — `cargo test -p maya-core codex:: store::codex` — Expected: FAIL.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run** — `cargo test -p maya-core` — Expected: PASS.

- [ ] **Step 5: Commit** — `feat(codex): limits and token cost from the rollout`

---

### Task 9: Kiro credits per session

**Files:**
- Modify: `core/src/kiro.rs:36-58,183`, `core/src/store.rs` (Kiro branch of the foreign loop)
- Test: inline, with `core/fixtures/kiro/session.json`

**Interfaces:**
- Produces on `kiro::Meta`: `pub credits_micros: u64` = round(sum of `session_state.conversation_metadata.user_turn_metadatas[].metering_usage[].value` where `unit == "credit"` × 1e6).
- Produces `pub fn apply_credits(tail: &mut ForeignTail, meta_json: &str)` setting `tail.usage = Some(SessionUsage { tokens: TokenCounts::default(), cost: Some(Cost { micros, unit: Credits, estimated: false, models: [] }) })` when `credits_micros > 0`.
- `Store` reads `<id>.json` beside the events file through `NameFiles` (mtime-cached text) and calls `apply_credits` after `apply_live`.

- [ ] **Step 1: Write the failing tests**: `session_meta_sums_the_credits_of_every_turn` against the fixture (compute the expected sum from the fixture's `metering_usage` values and hard-code it); `apply_credits_sets_a_credit_cost_without_tokens`.

- [ ] **Step 2: Run to verify they fail** — `cargo test -p maya-core kiro::` — Expected: FAIL.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run** — `cargo test -p maya-core` — Expected: PASS.

- [ ] **Step 5: Commit** — `feat(kiro): credits spent per session`

---

### Task 10: OpenCode tokens and cost

**Files:**
- Modify: `core/src/opencode.rs:140-180,520-570`
- Test: inline, with `core/fixtures/opencode/sessions.json`

**Interfaces:**
- Produces on `SessionInfo`: `pub tokens: crate::usage::TokenCounts` (input, cache.read → cached_input, cache.write → cache_write, output + reasoning → output) and `pub cost_micros: u64` from `cost`; the card gets `usage: Some(SessionUsage { tokens, cost: Some(Cost { micros, unit: Usd, estimated: false, models: [] }) })`. `tokens_used` stays as it is for the context meter.

- [ ] **Step 1: Write the failing test**: extend `sessions_are_read_with_their_times_folder_and_tokens` with `tokens == TokenCounts { input: 10_083, cached_input: 0, cache_write: 0, output: 207 }` and `cost_micros == 0` for the `ses_efe57d…` entry, and a card test asserting `usage.is_some()`.

- [ ] **Step 2: Run to verify it fails** — `cargo test -p maya-core opencode::` — Expected: FAIL.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run** — Expected: PASS.

- [ ] **Step 5: Commit** — `feat(opencode): tokens and cost per session`

---

### Task 11: Collectors off the lock: the runner, the due list, and Grok's cost

**Files:**
- Create: `core/fixtures/grok/usage.json` (the `grok usage <id>` answer from this machine, trimmed to `sessionId`, `updatedAt`, `session`)
- Modify: `core/src/usage.rs`, `core/src/grok.rs`, `core/src/store.rs`, `src-tauri/src/lib.rs` (beside `poll_pull_requests`), `cli/src/run_cmd.rs:220-235`
- Test: inline

**Interfaces:**
- Produces in `core/src/usage.rs`:
  ```rust
  /// Runs `program args` with stdin closed, returns stdout on exit 0 within `timeout`; kills it and returns None otherwise. Uses `crate::command`.
  pub fn run_with_timeout(program: &str, args: &[&str], timeout: std::time::Duration) -> Option<String>;
  /// What the collectors should run now.
  #[derive(Debug, Default, PartialEq, Eq)]
  pub struct Due { pub antigravity: bool, pub kiro: bool, /// Grok session ids whose events file changed since their last run.
  pub grok: Vec<String> }
  ```
- Produces in `core/src/grok.rs`: `pub fn parse_usage(json: &str) -> Option<crate::usage::SessionUsage>` (`session.inputTokens` → input, `cachedReadTokens` → cached_input, `cacheCreationTokens` → cache_write, `outputTokens + reasoningTokens` → output; `costUsdTicks / 10_000` → micros, `estimated: false`).
- Produces in `Store`:
  ```rust
  pub const GROK_USAGE_MIN_GAP_MS: u64 = 60_000;
  pub const AGY_LIMITS_GAP_MS: u64 = 300_000;
  pub const KIRO_LIMITS_GAP_MS: u64 = 600_000;
  pub const NUDGE_STALE_MS: u64 = 60_000;
  /// Marks the harness's limits due now when older than `NUDGE_STALE_MS` (a window opened).
  pub fn nudge_limits(&mut self, h: Harness, now_ms: u64);
  pub fn usage_due(&mut self, now_ms: u64) -> usage::Due;   // marks what it hands out as started
  pub fn set_limits(&mut self, h: Harness, limits: Option<AccountLimits>, now_ms: u64);
  pub fn set_grok_usage(&mut self, session_id: &str, usage: Option<SessionUsage>, now_ms: u64);
  ```
  `refresh` stamps `card.limits` for Antigravity and Kiro cards from the per-harness cache, and `card.usage` for Grok cards from the per-session cache. A Grok session is due when its `events.jsonl` mtime is newer than its last run and the last run is ≥ 60 s old (or never).
- Produces `pub(crate) fn poll_usage(app: &AppHandle)` in `src-tauri/src/lib.rs` and the same loop in `cli/src/run_cmd.rs`: take `usage_due`, run each collector outside the lock (`grok usage <id>` with 10 s), set the results, then `refresh_and_emit` (app) or set `board_due` (CLI). Runs every 5 s on its own thread. This task wires only Grok; Tasks 12 and 13 add the other two arms to the same function.
- Produces a Tauri command `nudge_limits(session_id)` that finds the card's harness and calls `Store::nudge_limits`; `src/modal.ts` invokes it when a window opens (`openModal`), ignoring errors.

- [ ] **Step 1: Write the failing tests**:
  - `run_with_timeout_returns_output_and_kills_a_hanger` (`#[cfg(unix)]`): `sh -c 'echo hi'` → `Some("hi\n")`; `sh -c 'sleep 5'` with 200 ms → `None` within 1 s.
  - `parse_usage_reads_grok_tokens_and_ticks`: fixture → `tokens.input == 19581`, `output == 488 + 474`, `cost.micros == 40_362`.
  - In `store.rs`: `grok_usage_is_due_once_per_minute_and_only_after_the_events_file_changed`; `nudged_limits_are_due_at_once_when_stale`; `set_limits_stamps_every_card_of_that_harness`.

- [ ] **Step 2: Run to verify they fail** — Expected: FAIL.

- [ ] **Step 3: Implement** the runner, the parser, the store pieces, `poll_usage` in the app and the CLI loop, the nudge command and its call from `openModal`.

- [ ] **Step 4: Run** — `cargo test -p maya-core -p maya-cli`, `cargo check -p maya`, `pnpm test` — Expected: PASS.

- [ ] **Step 5: Commit** — `feat(core): usage collectors off the store lock, with Grok's cost`

---

### Task 12: Antigravity limits from print mode

**Files:**
- Create: `core/fixtures/antigravity/usage.json` (the `agy -p "/usage" --output-format json` answer recorded in the design conversation, verbatim)
- Modify: `core/src/antigravity.rs`, `src-tauri/src/lib.rs` (`poll_usage`), `cli/src/run_cmd.rs`
- Test: inline

**Interfaces:**
- Produces: `pub fn parse_usage(json: &str, now_ms: u64) -> Option<AccountLimits>`: for each `command.data.groups[]` and each of its `buckets[]`, a window labelled `"{short} {label}"` where `short` is `"Gemini"` for a group name starting `Gemini`, `"Claude+GPT"` for one starting `Claude and GPT`, else the name; `label` is `"7d"` for `window == "weekly"`, `"1d"` for `"daily"`, `"5h"` for `"five_hour"`, else the window string; `percent = round((1 − remaining_fraction) × 100)` capped 0..100; `resets_at` from `reset_time` via `codex::ms_of / 1000`. `None` when `status != "SUCCESS"` or no groups.
- `poll_usage`'s Antigravity arm: `run_with_timeout("agy", &["-p", "/usage", "--output-format", "json"], 20 s)` → `set_limits(Harness::Antigravity, parse_usage(..))`.

- [ ] **Step 1: Write the failing test**: `parse_usage_gives_one_window_per_bucket_named_by_group` → two windows `("Gemini 7d", 0, Some(1793877569))` and `("Claude+GPT 7d", 0, Some(1793877569))` (the epoch seconds of `2026-11-05T10:19:29Z`; verify with `date -u -j -f %Y-%m-%dT%H:%M:%SZ 2026-11-05T10:19:29Z +%s`); `remaining_fraction: 0.25` → 75; a `status: "ERROR"` answer → `None`.

- [ ] **Step 2: Run to verify it fails** — Expected: FAIL.

- [ ] **Step 3: Implement** the parser and the poll arm (app and CLI).

- [ ] **Step 4: Run** — Expected: PASS.

- [ ] **Step 5: Commit** — `feat(antigravity): weekly limits per model group`

---

### Task 13: Kiro account limits over ACP (spike first)

**Files:**
- Create: `core/fixtures/kiro/usage-card.txt` (from the spike), `core/examples/kiro_usage_probe.rs` (the spike, kept as an example like `inbox_probe`)
- Modify: `core/src/kiro.rs`, `src-tauri/src/lib.rs` (`poll_usage`), `cli/src/run_cmd.rs`
- Test: inline

**Interfaces:**
- Produces:
  ```rust
  /// Spawns `kiro-cli acp`, sends `initialize` (as `brain::acp::initialize`), `session/new` with `cwd` = home and no MCP servers, then `_kiro.dev/commands/execute` `{"sessionId": <id>, "command": "usage"}`; collects every line for up to `timeout` or until the request's response; returns the text of the card (the concatenated `content.text` of `session/update` notifications, or the response's `result` text). Kills the process on return.
  pub fn acp_usage_text(timeout: std::time::Duration) -> Result<String, String>;
  /// The card's text → one window: label "month", percent = round(used / cap × 100), `resets_at` from a date in the text when present. Matches the lines by regular expressions on the fixture's wording.
  pub fn parse_usage_card(text: &str, now_ms: u64) -> Option<AccountLimits>;
  ```
- `poll_usage`'s Kiro arm: `acp_usage_text(15 s)` → `parse_usage_card` → `set_limits(Harness::Kiro, ..)`; on `Err` or `None`, `set_limits(Kiro, None)` and `log::line("usage", ..)` at most once an hour (a timestamp on the store).

- [ ] **Step 1: Spike.** Write `core/examples/kiro_usage_probe.rs` that does the ACP exchange above with raw `std::process` and prints every line received, then run `cargo run -p maya-core --example kiro_usage_probe` on this machine. If the card arrives, save its text to `core/fixtures/kiro/usage-card.txt` and continue. If after two attempts (varying the method name per the issue thread: `_kiro.dev/commands/execute`, then `_kiro.dev/command/execute`) nothing usable arrives, stop this task here: leave the example, write `core/fixtures/kiro/usage-card.txt` absent, implement only the `None` path (Kiro windows say "limits not shared"), and say so in the PR description.

- [ ] **Step 2: Write the failing tests**: `parse_usage_card_reads_used_cap_and_reset` against the fixture (expected values from the fixture); `an_unrelated_text_gives_none`.

- [ ] **Step 3: Run to verify they fail** — Expected: FAIL.

- [ ] **Step 4: Implement** `acp_usage_text` (reuse `brain::acp::initialize` and `brain::jsonrpc::RPC.request`), the parser, the poll arm.

- [ ] **Step 5: Run** — `cargo test -p maya-core kiro::` — Expected: PASS.

- [ ] **Step 6: Commit** — `feat(kiro): account limits from the usage card over ACP`

---

### Task 14: The header strip

**Files:**
- Create: `src/usage.ts`, `src/usage.test.ts`
- Modify: `src/modal.ts:500-545` (replace the `modal__context` span with the strip; keep the Compact button), `src/modal.ts:427-450` (extend the panel `ResizeObserver` to set the stage on the head), `src/styles.css` (strip styles for both looks), `src/modal.test.ts:760-790`
- Test: `src/usage.test.ts`, `src/modal.test.ts`

**Interfaces:**
- Consumes: `Card.context`, `Card.limits`, `Card.usage`, `formatTokens`, `contextBand`, `COMPACT_AT`, `capabilitiesOf`.
- Produces in `src/usage.ts`:
  ```ts
  export type Stage = "wide" | "medium" | "narrow" | "chip";
  export const STAGE_MIN_PX = { wide: 1100, medium: 850, narrow: 650 } as const;
  export function stageFor(headWidthPx: number): Stage;
  /** "$4.12" (two decimals, micros rounded), "12.4 credits" (one decimal), with " est." appended when estimated. */
  export function costText(cost: Cost): string;
  /** "↻ 1h 12m" / "↻ 45m" / "↻ Thu 09:00" when more than a day away; "" when unknown. */
  export function resetText(resetsAt: number | null | undefined, nowMs: number): string;
  /** The muted note when there are no windows: Grok → "limits not shared"; OpenCode → "no limits"; limits present but empty → "no limits"; Claude Code / Codex / Antigravity / Kiro with limits null → "limits not read yet". */
  export function limitsNote(harness: Harness, limits: AccountLimits | null | undefined): string | null;
  /** Tooltip of a window: "5-hour limit 78% · resets in 1h 12m · tco@example.com" (label words: 5h → "5-hour limit", 7d → "Weekly limit", else "<label> limit"). */
  export function windowTitle(w: LimitWindow, account: string | null | undefined, nowMs: number): string;
  /** Tooltip of the cost: tokens by kind, then "Estimated at list prices for <models>" or "Reported by the agent", or "main conversation only" for an estimated Claude Code cost. */
  export function costTitle(usage: SessionUsage, harness: Harness): string;
  /** The band of the hottest window and the context, for the chip's colour. */
  export function hottestBand(card: Card): "low" | "mid" | "high";
  /** The strip element: class `usage usage--<stage>`, children `.usage__meter[data-kind=ctx|limit]` each with `.usage__label`, `.usage__bar > .usage__fill`, `.usage__pct`, `.usage__reset`; `.usage__cost` with `.usage__tokens`; `.usage__note` for the note; at the chip stage a single `.usage__chip` with `data-band`. */
  export function usageStrip(card: Card, stage: Stage, nowMs?: number): HTMLElement;
  ```
  Per stage: wide shows everything; medium drops `.usage__reset` and `.usage__tokens`; narrow drops `.usage__bar`; chip renders only `.usage__chip` with text `<ctx%> · <hottest limit %> · <cost>` (parts present only when known) and the full strip's texts joined in its `title`.
- Produces in `src/modal.ts`: the strip appended to `.modal__titles` after the harness badge (and the PR button), with `data-stage` driven by a `ResizeObserver` on `.modal__head` that re-renders the strip on a stage change; the `modal__context` span is removed (the strip's ctx meter carries its tooltip); the Compact button rule is unchanged. CSS: `.usage` horizontal flex, gap 14px; bars 70px (44px at medium), 6px tall, fill colours by `data-band` (`low` → `var(--working)` blue for limits and gold for ctx, `high` → `var(--awaiting)`); `.modal--terminal .usage` in the terminal palette; `.modal__statusline--head` hidden at narrow and chip; `.modal__project` hidden at chip.

- [ ] **Step 1: Write the failing tests** in `src/usage.test.ts`:
  - `stages_follow_the_thresholds`: 1100 → wide, 1099 → medium, 850 → medium, 649 → chip.
  - `cost_text_rounds_dollars_and_credits_and_marks_estimates`: `{ micros: 4_123_400, unit: "usd", estimated: true, models: [] }` → `$4.12 est.`; `{ micros: 12_400_000, unit: "credits", estimated: false }` → `12.4 credits`.
  - `reset_text_counts_down_or_names_the_day`: 72 min → `↻ 1h 12m`; 45 min → `↻ 45m`; 3 days → starts with `↻ ` and contains a weekday; null → ``.
  - `limits_note_per_harness`: the five cases in the interface.
  - `the_strip_drops_detail_by_stage`: a Claude card with ctx 62, windows 5h 78 and 7d 41, cost est. → wide has 3 meters, 2 `.usage__reset`, `.usage__tokens`; medium has no `.usage__reset`/`.usage__tokens`; narrow has no `.usage__bar`; chip has one `.usage__chip` with text `62% · 78% · $4.12 est.` and `data-band="high"`.
  - `a_bare_card_at_the_chip_stage_shows_only_context` (Review Focus 4): Grok card with ctx 8, no limits, no usage → chip text `8%`, no throw.
  - `antigravity_and_kiro_windows_keep_their_labels`: windows `Gemini 7d` and `month` render those labels.
  In `src/modal.test.ts`: `the_header_carries_the_usage_strip_and_no_context_span` (`.modal__titles .usage` exists, `.modal__context` does not, the Compact button still appears at ctx ≥ 75 for Claude Code), and `the_strip_stage_follows_the_heads_width` by calling the exported `applyStage(root, widthPx)` helper (export it from `modal.ts` for the test) and asserting `data-stage`.

- [ ] **Step 2: Run to verify they fail** — `pnpm test -- usage modal` — Expected: FAIL.

- [ ] **Step 3: Implement** `src/usage.ts`, the modal changes, the CSS.

- [ ] **Step 4: Run** — `pnpm test` — Expected: PASS (update the two existing `statusLine` assertions in `modal.test.ts` only if Task 15 has not yet changed them; otherwise leave them to Task 15).

- [ ] **Step 5: Commit** — `feat(viewer): usage and limits strip in the session window's header`

---

### Task 15: ANSI rendering and the terminal look's bottom line

**Files:**
- Create: `src/ansi.ts`, `src/ansi.test.ts`
- Modify: `src/modal.ts:478-482` (`statusLine`), the two `modal__statusline--foot` appends, `src/modal.test.ts:765-790`, `src/styles.css`
- Test: `src/ansi.test.ts`, `src/modal.test.ts`

**Interfaces:**
- Produces in `src/ansi.ts`:
  ```ts
  /** Text with SGR colour rendered as <span> runs (classes `ansi-b`, `ansi-d`, inline `color` for 16/256/truecolor), every other escape sequence removed (CSI, OSC up to BEL or ST, lone ESC). Never inserts markup from the text itself. */
  export function renderAnsi(text: string): DocumentFragment;
  /** The same text with every escape sequence removed. */
  export function stripAnsi(text: string): string;
  ```
- Produces in `src/modal.ts`: `statusLine(card)` returns `card.statusLine` when it is a non-empty string; otherwise Maya's line as today with the limits and cost appended: `… · ctx 62% · 5h 78% · 7d 41% · $4.12 est.` (each window as `<label> <pct>%`, then `costText`). The foot element gets `renderAnsi(statusLine(card))` when the line is recorded, plain text otherwise, and class `modal__statusline--recorded` when recorded.

- [ ] **Step 1: Write the failing tests** in `src/ansi.test.ts`:
  - `bold_dim_and_the_sixteen_colours_become_spans`: `"\u001b[1mA\u001b[0m \u001b[2mB\u001b[0m \u001b[31mC\u001b[0m"` → three spans with `ansi-b`, `ansi-d`, and a `color` style; text `A B C`.
  - `256_and_truecolor_set_inline_colours`: `\u001b[38;5;208mX\u001b[0m` and `\u001b[38;2;255;0;0mY\u001b[0m` → styles containing `rgb(` for both.
  - `non_sgr_sequences_are_dropped` (Review Focus 5): `"a\u001b[2Kb\u001b]0;title\u0007c\u001b[Hd"` → text `abcd`, no span attributes from the text.
  - `markup_in_the_text_stays_text`: `"<b>x</b>"` → textContent `<b>x</b>`, no `b` element.
  In `src/modal.test.ts`: change the existing foot assertions to `the_terminal_foot_shows_the_recorded_line_when_there_is_one` (card with `statusLine: "\u001b[1mOpus\u001b[0m │ 62%"` → foot textContent `Opus │ 62%`, has `.ansi-b`, class `modal__statusline--recorded`) and `mayas_own_line_appends_limits_and_cost_without_a_recording` (`busy` card plus limits and usage → `claude-opus-5 · eye · feat/x · PR #12 · ctx 42% · 5h 78% · 7d 41% · $4.12 est.`; the head line is unchanged `claude-opus-5 · feat/x`).

- [ ] **Step 2: Run to verify they fail** — `pnpm test -- ansi modal` — Expected: FAIL.

- [ ] **Step 3: Implement** `src/ansi.ts` (a small state machine over the string; colours from a 16-entry table and the 6×6×6 cube + greys), the `statusLine` change and the foot rendering, and CSS for `.ansi-b { font-weight: 600 }`, `.ansi-d { opacity: .6 }`, `.modal__statusline--recorded { white-space: pre }`.

- [ ] **Step 4: Run** — `pnpm test` — Expected: PASS.

- [ ] **Step 5: Commit** — `feat(viewer): the terminal look shows the session's recorded status line`

---

### Task 16: Docs, version and the pull request

**Files:**
- Modify: `docs/DEVELOPING.md` (a "Usage and limits" section after "Maya's brain": the three card fields, where each agent's numbers come from, the wrapper and its files, the collectors' cadences, the price table and its date), `README.md` (one bullet under "What it does": "Shows each session's context, its account's usage limits and its cost so far, in the session window")
- Run: `sh scripts/set-version.sh <main's minor + 1>.0`

- [ ] **Step 1: Write the docs.**

- [ ] **Step 2: Bump the version**: `git fetch origin main`, read the version in `origin/main:package.json`, bump the minor (a `feat`), run `sh scripts/set-version.sh <version>`, commit it alone as `chore(release): <version>`.

- [ ] **Step 3: Full verification**: `cargo test -p maya-core -p maya-cli -p maya-hook`, `cargo check -p maya`, `pnpm test`, `pnpm build`. Expected: all pass.

- [ ] **Step 4: Commit the docs** — `docs: usage and limits in the session window`, then open the PR with the `conventional-pull-request` skill. The PR description names what each agent shows, the Settings and CLI install steps, and whether Kiro's limits made it (Task 13).

---

## Plan decisions the spec left open (for the reviewer)

- **Transcript totals run under a byte budget per refresh, not on a thread.** The spec asked for the first read of a long transcript to run off the lock. A budget of 8 MB per refresh keeps `refresh` bounded with no thread or channel; a 65 MB transcript catches up over about eight ticks and shows a growing figure meanwhile.
- **Project-level `statusLine` files are not checked.** The spec asked install to refuse when a managed or project file overrides the status line. The managed file is checked; project files vary per session folder and are not. A session under such a project records nothing and shows "limits not read yet".
- **No first-run step.** The spec offered the wrapper on first run after the hook, but first run does not offer the hook today, so neither is added there. Settings and the CLI are the two install paths.
- **Antigravity's per-session cost is left out.** No transcript was available to check; the window shows context and limits for Antigravity.
