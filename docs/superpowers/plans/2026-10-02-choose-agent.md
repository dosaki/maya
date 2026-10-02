# Choose the agent and name a new session — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The New session modal starts Claude Code, Codex, Antigravity or Grok Build with that agent's own options and an optional name, and Maya can rename a running session of any of them.

**Architecture:** `model::Harness` doubles as the agent choice. `launch.rs` learns each agent's binary, option lists, flags and prompt form. A new `agents.rs` lists installed agents and asks each for its models (cached, refreshed in the background). A new `pending_names.rs` keeps names for agents with no name flag, shows them on the card, and asks for `/rename` once the session is free; the app and CLI tick type it. The network board carries each machine's agents. The modal is data-driven from that list.

**Tech Stack:** Rust (maya-core, Tauri 2 app, maya CLI), vanilla TypeScript + vitest.

**Spec:** `docs/superpowers/specs/2026-10-02-maya-choose-agent-design.md`

## Global Constraints

- Agents: Claude Code (`claude`), Codex (`codex`), Antigravity (`agy`), Grok Build (`grok`); the `Harness` serde names are `claude-code`, `codex`, `antigravity`, `grok`.
- Nothing the user typed reaches a shell line unquoted: effort and mode from fixed lists; a model from the agent's own list and matching `[A-Za-z0-9._-]+`; the name only through `shell_single_quote` (Claude) or typed as `/rename <name>` (others).
- Names follow `answer::rename_command`: one non-blank line, at most `answer::MAX_NAME_CHARS` (60) characters.
- Other agents get `/rename` only when free: card state Completed or Idle. Claude Code keeps `answer::check_free`.
- Pending names: dropped when the agent's name equals it, when its session ends, or 10 minutes after launch if unmatched. In memory only.
- Model listings: cached 10 minutes, 20-second timeout per listing, never waited on by the network board.
- An older Maya on another machine (board without `agents`) offers Claude Code only and no Name field.
- Lock order in the app: `network`, then the server's mutex, then `store`. Never type into a terminal while holding `store`.
- Release: `feat` → `sh scripts/set-version.sh 0.8.0` (unless `main` moved), its own `chore(release): 0.8.0` commit.
- Never push or open the PR without asking the user first.

## Review Focus

1. **A prompt that starts with `-`** (e.g. `--help me`) must reach every agent as the prompt, not as an option. Pinned in Task 1 (`each_agent_takes_a_dash_prompt_as_the_prompt`) and checked live in Task 9.
2. **A name with shell metacharacters** (`it's "$(rm -rf ~)"`) must start the session with that literal name. Pinned in Task 1 (`a_claude_name_is_single_quoted`) and Task 5 (`a_pending_name_is_typed_as_one_rename_line`).
3. **Two sessions of one agent in one folder, started back to back with different names**, must each get their own name. Pinned in Task 4 (`two_names_in_one_folder_claim_different_sessions`).
4. **An installed agent whose listing hangs, fails or prints more than a pipe holds** (agy offline, grok logged out, Codex's large catalogue) must give "Default" only and never hang the modal. Pinned in Task 2 (`run_listing_reads_output_bigger_than_a_pipe`, `run_listing_gives_up_after_the_timeout`, `info_without_a_listing_offers_default_only`).
5. **A remembered model the agent no longer lists, and an older remote Maya**, must fall back to "Default" and to Claude-only. Pinned in Task 8 (`a_remembered_model_no_longer_listed_falls_back_to_default`, `an_older_remote_offers_claude_only_and_no_name`).

---

### Task 1: Per-agent launch lines

**Files:**
- Modify: `core/src/model.rs` (Harness: Default)
- Modify: `core/src/launch.rs`
- Modify: `src-tauri/src/lib.rs:905` (`options.validate()` → `options.validate_shape()`)
- Modify: `core/src/actions.rs:230,239,246` (call sites only, behaviour unchanged here)
- Test: `core/src/launch.rs` tests module

**Interfaces:**
- Produces:
  - `impl Default for Harness` → `ClaudeCode`
  - `LaunchOptions { agent: Harness, model, effort, mode: Option<String>, name: Option<String> }` (all `#[serde(default)]`)
  - `launch::binary_name(Harness) -> &'static str`
  - `launch::efforts(Harness) -> &'static [&'static str]`, `launch::modes(Harness) -> &'static [&'static str]`
  - `launch::plain_model_id(&str) -> bool`
  - `LaunchOptions::validate_shape(&self) -> Result<(), String>`; `LaunchOptions::validate(&self, models: &[String]) -> Result<(), String>`
  - `LaunchOptions::flags(&self) -> String`; `LaunchOptions::chosen_name(&self) -> Option<&str>`
  - `launch::session_command(target: &Path, prompt_file: &Path, opts: &LaunchOptions, session_id: Option<&str>) -> String`
  - `launch::agent_binary(name: &str) -> Option<PathBuf>` (`claude_binary()` becomes `agent_binary("claude")`)
  - `launch::new_session_uuid() -> String`

- [ ] **Step 1: Write the failing tests** (add to `launch.rs` tests; fix the existing literals as noted)

```rust
    use crate::model::Harness;

    fn opts(agent: Harness) -> LaunchOptions {
        LaunchOptions { agent, ..Default::default() }
    }

    #[test]
    fn the_agent_defaults_to_claude_code_when_a_caller_leaves_it_out() {
        let o: LaunchOptions = serde_json::from_str(r#"{"model":"opus"}"#).unwrap();
        assert_eq!(o.agent, Harness::ClaudeCode);
        assert_eq!(o.name, None);
    }

    #[test]
    fn each_agent_renders_its_own_flags() {
        let all = |agent| LaunchOptions { agent, model: Some("m-1".into()), effort: Some("high".into()), mode: Some(modes(agent)[0].into()), ..Default::default() };
        assert_eq!(all(Harness::ClaudeCode).flags(), " --model m-1 --effort high --permission-mode manual");
        assert_eq!(all(Harness::Codex).flags(), " -m m-1 -c model_reasoning_effort=high -s read-only");
        assert_eq!(all(Harness::Antigravity).flags(), " --model m-1 --effort high --mode accept-edits");
        // Grok lists no effort values, so none is passed.
        assert_eq!(all(Harness::Grok).flags(), " -m m-1 --permission-mode default");
    }

    #[test]
    fn effort_and_mode_must_be_the_agents_own() {
        let o = |agent, effort: &str, mode: &str| LaunchOptions { agent, effort: Some(effort.into()), mode: Some(mode.into()), ..Default::default() };
        assert!(o(Harness::Codex, "ultra", "workspace-write").validate_shape().is_ok());
        assert!(o(Harness::Codex, "high", "plan").validate_shape().unwrap_err().contains("mode"));
        assert!(o(Harness::Antigravity, "xhigh", "plan").validate_shape().unwrap_err().contains("effort"));
        assert!(o(Harness::Grok, "high", "plan").validate_shape().unwrap_err().contains("effort"));
    }

    #[test]
    fn a_model_must_be_plain_and_in_the_agents_list() {
        let m = |model: &str| LaunchOptions { agent: Harness::Codex, model: Some(model.into()), ..Default::default() };
        let listed = vec!["gpt-6.1-sol".to_string()];
        assert!(m("gpt-6.1-sol").validate(&listed).is_ok());
        assert!(m("gpt-9").validate(&listed).unwrap_err().contains("model"));
        assert!(m("gpt; rm -rf /").validate_shape().unwrap_err().contains("model"));
        assert!(plain_model_id("claude-opus-4-6-thinking") && plain_model_id("grok-4.7"));
        assert!(!plain_model_id("") && !plain_model_id("a b") && !plain_model_id("$(x)"));
    }

    #[test]
    fn a_name_follows_the_rename_rules() {
        let n = |name: &str| LaunchOptions { name: Some(name.into()), ..Default::default() };
        assert!(n("Fix the CI").validate_shape().is_ok());
        assert!(n("   ").validate_shape().is_ok(), "blank means no name");
        assert_eq!(n("   ").chosen_name(), None);
        assert!(n("two\nlines").validate_shape().unwrap_err().contains("one line"));
        assert!(n(&"x".repeat(61)).validate_shape().unwrap_err().contains("too long"));
    }

    #[test]
    fn each_agent_runs_its_binary_with_the_prompt() {
        let line = |o: &LaunchOptions, id| session_command(Path::new("/r/a"), Path::new("/p/1.txt"), o, id);
        assert!(line(&opts(Harness::ClaudeCode), None).ends_with("&& claude -- \"$p\""));
        assert!(line(&opts(Harness::Codex), None).ends_with("&& codex -- \"$p\""));
        assert!(line(&opts(Harness::Antigravity), None).ends_with("&& agy --prompt-interactive=\"$p\""));
        assert!(line(&opts(Harness::Grok), Some("0b9c-id")).ends_with("&& grok --session-id '0b9c-id' -- \"$p\""));
    }

    #[test]
    fn each_agent_takes_a_dash_prompt_as_the_prompt() {
        // The prompt is never in the line itself: it is read from the file into
        // $p, and given after `--`, or as `--flag="$p"` for Go's flag parser.
        for agent in [Harness::ClaudeCode, Harness::Codex, Harness::Antigravity, Harness::Grok] {
            let s = session_command(Path::new("/r"), Path::new("/p/1.txt"), &opts(agent), None);
            assert!(s.ends_with(" -- \"$p\"") || s.ends_with("--prompt-interactive=\"$p\""), "{s}");
        }
    }

    #[test]
    fn a_claude_name_is_single_quoted() {
        let o = LaunchOptions { name: Some("it's \"$(rm -rf ~)\"".into()), ..Default::default() };
        let s = session_command(Path::new("/r"), Path::new("/p/1.txt"), &o, None);
        assert!(s.ends_with(r#"&& claude -n 'it'\''s "$(rm -rf ~)"' -- "$p""#), "{s}");
        // The other agents take no name flag: Maya renames them later.
        let codex = LaunchOptions { agent: Harness::Codex, name: Some("x".into()), ..Default::default() };
        assert!(!session_command(Path::new("/r"), Path::new("/p/1.txt"), &codex, None).contains(" -n "));
    }

    #[test]
    fn session_uuids_are_version_4() {
        let id = new_session_uuid();
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.iter().map(|p| p.len()).collect::<Vec<_>>(), vec![8, 4, 4, 4, 12]);
        assert!(parts[2].starts_with('4'));
        assert!("89ab".contains(&parts[3][..1]));
        assert_ne!(id, new_session_uuid());
    }
```

Existing tests to update in the same step:
- `launch_options_accept_only_known_values`: replace each `.validate()` with `.validate(&MODELS.iter().map(|s| s.to_string()).collect::<Vec<_>>())`, and add `..Default::default()` to every struct literal that lists all of `model`, `effort`, `mode`.
- `launch_options_render_as_claude_flags`: add `..Default::default()` to the `all` literal.
- `launch_command_carries_the_flags_before_the_prompt` and `shell_quoting_and_applescript_escaping`: unchanged (they go through `applescript_launch`, which passes `None`).

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core launch::`
Expected: compile errors (`agent`, `name`, `modes`, `plain_model_id`, `session_command` arity, `new_session_uuid` not found).

- [ ] **Step 3: Implement**

In `core/src/model.rs`, after the `Harness` enum:

```rust
/// A new session runs Claude Code unless the caller picks another agent.
impl Default for Harness {
    fn default() -> Self {
        Harness::ClaudeCode
    }
}
```

In `core/src/launch.rs` (replace `LaunchOptions`, `validate`, `flags`, `session_command`, `applescript_launch`; generalise `claude_binary`):

```rust
use crate::model::Harness;

/// The command each agent runs as.
pub fn binary_name(agent: Harness) -> &'static str {
    match agent {
        Harness::ClaudeCode => "claude",
        Harness::Codex => "codex",
        Harness::Antigravity => "agy",
        Harness::Grok => "grok",
    }
}

/// The efforts an agent takes, from its `--help`. Each Codex model takes a
/// subset of these; Grok lists none.
pub fn efforts(agent: Harness) -> &'static [&'static str] {
    match agent {
        Harness::ClaudeCode => EFFORTS,
        Harness::Codex => &["low", "medium", "high", "xhigh", "max", "ultra"],
        Harness::Antigravity => &["low", "medium", "high", "max"],
        Harness::Grok => &[],
    }
}

/// The modes an agent takes: Claude's and Grok's permission modes, Codex's
/// sandbox policies, Antigravity's execution modes.
pub fn modes(agent: Harness) -> &'static [&'static str] {
    match agent {
        Harness::ClaudeCode => MODES,
        Harness::Codex => &["read-only", "workspace-write", "danger-full-access"],
        Harness::Antigravity => &["accept-edits", "plan"],
        Harness::Grok => &["default", "acceptEdits", "auto", "dontAsk", "bypassPermissions", "plan"],
    }
}

/// A model id is put in a shell line only when it is plain: letters,
/// digits, `.`, `_` and `-`.
pub fn plain_model_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 100 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
}

/// Per-session choices for a new session. None or "" means "use the defaults".
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct LaunchOptions {
    pub agent: Harness,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub mode: Option<String>,
    /// The session's name; only Claude Code takes it as a flag.
    pub name: Option<String>,
}

impl LaunchOptions {
    pub fn chosen_name(&self) -> Option<&str> {
        chosen(&self.name)
    }

    /// Checks everything that can be checked without the agent's model list.
    pub fn validate_shape(&self) -> Result<(), String> {
        if let Some(m) = chosen(&self.model) {
            if !plain_model_id(m) {
                return Err(format!("Unknown model: {m}"));
            }
        }
        if let Some(e) = chosen(&self.effort) {
            check_choice("effort", e, efforts(self.agent))?;
        }
        if let Some(m) = chosen(&self.mode) {
            check_choice("mode", m, modes(self.agent))?;
        }
        if let Some(n) = self.chosen_name() {
            crate::answer::rename_command(n)?;
        }
        Ok(())
    }

    /// `validate_shape`, and a chosen model must be in `models`, the agent's own list.
    pub fn validate(&self, models: &[String]) -> Result<(), String> {
        self.validate_shape()?;
        match chosen(&self.model) {
            Some(m) if !models.iter().any(|x| x == m) => Err(format!("Unknown model: {m}")),
            _ => Ok(()),
        }
    }

    /// The agent's flags for the chosen options, each preceded by a space.
    /// Only valid values are rendered; call `validate` first.
    pub fn flags(&self) -> String {
        let (model, effort, mode) = (chosen(&self.model), chosen(&self.effort), chosen(&self.mode));
        let (m_flag, e_flag, mode_flag) = match self.agent {
            Harness::ClaudeCode => ("--model", Some("--effort"), "--permission-mode"),
            Harness::Codex => ("-m", Some("-c model_reasoning_effort="), "-s"),
            Harness::Antigravity => ("--model", Some("--effort"), "--mode"),
            Harness::Grok => ("-m", None, "--permission-mode"),
        };
        let mut out = String::new();
        if let Some(m) = model {
            out.push_str(&format!(" {m_flag} {m}"));
        }
        match (e_flag, effort) {
            (Some(f), Some(e)) if f.ends_with('=') => out.push_str(&format!(" {f}{e}")),
            (Some(f), Some(e)) => out.push_str(&format!(" {f} {e}")),
            _ => {}
        }
        if let Some(m) = mode {
            out.push_str(&format!(" {mode_flag} {m}"));
        }
        out
    }
}
```

Delete the old `validate` (its body moved into `validate_shape`), keep `MODELS`, `EFFORTS`, `MODES`, `chosen`, `check_choice`.

```rust
/// The shell line that starts a session: it reads the prompt file into a
/// variable, deletes the file, and runs the agent with the prompt as one
/// argument that is never read as an option: after `--`, or for
/// Antigravity, whose Go flag parser has no `--`, as `--prompt-interactive="$p"`.
/// `session_id` is Grok's `--session-id`, so Maya knows the new session's id.
pub fn session_command(target: &Path, prompt_file: &Path, opts: &LaunchOptions, session_id: Option<&str>) -> String {
    let file = shell_single_quote(&prompt_file.to_string_lossy());
    let mut args = opts.flags();
    if opts.agent == Harness::ClaudeCode {
        if let Some(n) = opts.chosen_name() {
            args.push_str(&format!(" -n {}", shell_single_quote(n)));
        }
    }
    if let Some(id) = session_id {
        args.push_str(&format!(" --session-id {}", shell_single_quote(id)));
    }
    let prompt = if opts.agent == Harness::Antigravity { " --prompt-interactive=\"$p\"" } else { " -- \"$p\"" };
    format!(
        "cd {} && p=\"$(cat {file})\" && rm -f {file} && {}{args}{prompt}",
        shell_single_quote(&target.to_string_lossy()),
        binary_name(opts.agent)
    )
}

pub fn applescript_launch(target: &Path, prompt_file: &Path, opts: &LaunchOptions) -> String {
    applescript_run(&session_command(target, prompt_file, opts, None))
}

/// A random version-4 UUID, for Grok's `--session-id`.
pub fn new_session_uuid() -> String {
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::rng().fill_bytes(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}
```

Generalise the binary lookup. Unix: rename `claude_binary` to `agent_binary(name: &str)`, replacing `"claude"` with `name` in the PATH lookup, the three candidates (`home.join(".local/bin").join(name)`, `PathBuf::from("/opt/homebrew/bin").join(name)`, `PathBuf::from("/usr/local/bin").join(name)`) and the login-shell line (`format!("command -v {name}")`; `name` comes only from `binary_name`). Windows: rename to `agent_binary(name: &str)`, looking for `format!("{name}.exe")` on PATH and in `~/.local/bin`. Then, on both:

```rust
/// The `claude` binary, for the folder classifier.
pub fn claude_binary() -> Option<PathBuf> {
    agent_binary("claude")
}
```

Call sites: `core/src/actions.rs:230` `options.validate()?` → `options.validate_shape()?` (Task 5 replaces it), `:246` `launch::session_command(&target, &file, &options)` → `launch::session_command(&target, &file, &options, None)`; `src-tauri/src/lib.rs:905` `options.validate()?` → `options.validate_shape()?`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p maya-core launch:: && cargo build --workspace`
Expected: PASS; the workspace builds.

- [ ] **Step 5: Commit**

```bash
git add core/src/model.rs core/src/launch.rs core/src/actions.rs src-tauri/src/lib.rs
git commit -m "feat(core): launch lines for Codex, Antigravity and Grok Build"
```

---

### Task 2: Which agents are installed, and their models

**Files:**
- Create: `core/src/agents.rs`
- Create: `core/fixtures/codex/models.json`, `core/fixtures/antigravity/models.txt`, `core/fixtures/grok/models.txt` (copy from `/private/tmp/claude-501/-Users-tiagocorreia-dev-maya/69d0d15d-0130-48b2-8460-0f3c4616352e/scratchpad/fixtures/codex-models.json`, `agy-models.txt`, `grok-models.txt`: real output saved on 2026-10-02; the Codex one trimmed to four models, one hidden)
- Modify: `core/src/lib.rs` (`pub mod agents;`)

**Interfaces:**
- Consumes: `launch::{binary_name, efforts, modes, plain_model_id, agent_binary, clean_env, MODELS}`
- Produces:
  - `agents::ModelInfo { id: String, label: String, efforts: Vec<String> }` (serde camelCase)
  - `agents::AgentInfo { harness: Harness, models: Vec<ModelInfo>, efforts: Vec<String>, modes: Vec<String> }` with `fn model_ids(&self) -> Vec<String>`
  - `agents::claude() -> AgentInfo`
  - `agents::parse_codex(&str)`, `parse_agy(&str)`, `parse_grok(&str) -> Vec<ModelInfo>`
  - `agents::info_for(Harness, Option<&str>) -> AgentInfo`
  - `agents::build(find, run) -> Vec<AgentInfo>`
  - `agents::run_listing(&Path, &[&str], Duration) -> Option<String>`
  - `agents::snapshot() -> Vec<AgentInfo>` (never waits), `agents::current() -> Vec<AgentInfo>` (waits for the first listing)

- [ ] **Step 1: Copy the fixtures and write the failing tests** (in `agents.rs`'s tests module)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn fixture(rel: &str) -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(rel)).unwrap()
    }

    #[test]
    fn codex_models_are_the_listed_ones_with_their_efforts() {
        let m = parse_codex(&fixture("codex/models.json"));
        assert_eq!(m.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["gpt-6.1-sol", "gpt-6-luna", "gpt-5.5"], "gpt-reserve is hidden");
        assert_eq!(m[0].label, "GPT-6.1-Sol");
        assert_eq!(m[2].efforts, vec!["low", "medium", "high", "xhigh"]);
        assert!(parse_codex("not json").is_empty());
    }

    #[test]
    fn antigravity_models_are_the_tab_separated_lines() {
        let m = parse_agy(&fixture("antigravity/models.txt"));
        assert_eq!(m[0], ModelInfo { id: "gemini-3.8-flash-high".into(), label: "Gemini 3.8 Flash (High)".into(), efforts: vec![] });
        assert!(m.iter().all(|m| !m.id.starts_with("Fetching")));
        assert_eq!(m.len(), 14);
    }

    #[test]
    fn grok_models_are_the_lines_under_available_models() {
        assert_eq!(parse_grok(&fixture("grok/models.txt")).iter().map(|m| m.id.clone()).collect::<Vec<_>>(), vec!["grok-4.7"]);
        let two = "Default model: a\n\nAvailable models:\n  * grok-4.7 (default)\n    grok-5-mini\n";
        assert_eq!(parse_grok(two).iter().map(|m| m.id.clone()).collect::<Vec<_>>(), vec!["grok-4.7", "grok-5-mini"]);
        assert!(parse_grok("You are not authenticated.").is_empty());
    }

    #[test]
    fn codex_default_model_offers_the_efforts_every_model_takes() {
        let info = info_for(Harness::Codex, Some(&fixture("codex/models.json")));
        assert_eq!(info.efforts, vec!["low", "medium", "high", "xhigh"]);
        assert_eq!(info.modes, vec!["read-only", "workspace-write", "danger-full-access"]);
    }

    #[test]
    fn info_without_a_listing_offers_default_only() {
        for agent in [Harness::Codex, Harness::Antigravity, Harness::Grok] {
            let info = info_for(agent, None);
            assert!(info.models.is_empty(), "{agent:?}");
            assert_eq!(info.modes, launch::modes(agent).iter().map(|s| s.to_string()).collect::<Vec<_>>());
        }
    }

    #[test]
    fn build_lists_claude_then_each_agent_it_finds() {
        let found = |name: &str| (name == "codex" || name == "grok").then(|| PathBuf::from(format!("/bin/{name}")));
        let run = |bin: &Path, args: &[&str]| -> Option<String> {
            match (bin.to_str().unwrap(), args) {
                ("/bin/codex", ["debug", "models"]) => Some(fixture("codex/models.json")),
                ("/bin/grok", ["models"]) => None,
                other => panic!("unexpected {other:?}"),
            }
        };
        let list = build(found, run);
        assert_eq!(list.iter().map(|a| a.harness).collect::<Vec<_>>(), vec![Harness::ClaudeCode, Harness::Codex, Harness::Grok]);
        assert_eq!(list[0], claude());
        assert_eq!(list[1].models.len(), 3);
        assert!(list[2].models.is_empty(), "a failed listing still lists the agent");
    }

    #[test]
    fn claude_offers_maya_s_fixed_lists() {
        let c = claude();
        assert_eq!(c.model_ids(), vec!["fable", "opus", "sonnet", "haiku"]);
        assert_eq!(c.models[1].label, "Opus");
        assert_eq!(c.efforts.len(), 5);
        assert_eq!(c.modes.len(), 6);
    }

    #[cfg(unix)]
    fn script(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join("lister");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[cfg(unix)]
    #[test]
    fn run_listing_reads_output_bigger_than_a_pipe() {
        let t = tempfile::tempdir().unwrap();
        let bin = script(t.path(), "head -c 300000 /dev/zero | tr '\\0' x");
        let out = run_listing(&bin, &[], Duration::from_secs(10)).unwrap();
        assert_eq!(out.len(), 300_000);
    }

    #[cfg(unix)]
    #[test]
    fn run_listing_gives_up_after_the_timeout_or_a_failure() {
        let t = tempfile::tempdir().unwrap();
        let slow = script(t.path(), "sleep 5; echo late");
        let start = std::time::Instant::now();
        assert_eq!(run_listing(&slow, &[], Duration::from_secs(1)), None);
        assert!(start.elapsed() < Duration::from_secs(4));
        let failing = script(t.path(), "echo partial; exit 3");
        assert_eq!(run_listing(&failing, &[], Duration::from_secs(5)), None);
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core agents::`
Expected: compile error, module `agents` not found.

- [ ] **Step 3: Implement `core/src/agents.rs`** and add `pub mod agents;` to `core/src/lib.rs`

```rust
//! The agents this machine can start, and the models each offers, from the
//! agents' own listings (`codex debug models`, `agy models`, `grok models`).
//! A listing can take seconds (`agy models` asks a server), so the list is
//! cached for ten minutes and refreshed in the background.

use crate::launch;
use crate::model::Harness;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{mpsc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
    /// The efforts this model takes when they differ by model (Codex);
    /// empty means the agent's own list.
    #[serde(default)]
    pub efforts: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub harness: Harness,
    pub models: Vec<ModelInfo>,
    pub efforts: Vec<String>,
    pub modes: Vec<String>,
}

impl AgentInfo {
    pub fn model_ids(&self) -> Vec<String> {
        self.models.iter().map(|m| m.id.clone()).collect()
    }
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// Claude Code, with Maya's fixed lists: its model aliases need no listing.
pub fn claude() -> AgentInfo {
    let label = |id: &str| id[..1].to_uppercase() + &id[1..];
    AgentInfo {
        harness: Harness::ClaudeCode,
        models: launch::MODELS.iter().map(|id| ModelInfo { id: id.to_string(), label: label(id), efforts: vec![] }).collect(),
        efforts: strings(launch::EFFORTS),
        modes: strings(launch::MODES),
    }
}

/// Models from `codex debug models`: those in Codex's own picker
/// (`visibility: "list"`), with the efforts each takes.
pub fn parse_codex(json: &str) -> Vec<ModelInfo> {
    let v: Value = serde_json::from_str(json).unwrap_or(Value::Null);
    let known = launch::efforts(Harness::Codex);
    v["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["visibility"].as_str() == Some("list"))
        .filter_map(|m| {
            let id = m["slug"].as_str().filter(|id| launch::plain_model_id(id))?;
            let efforts = m["supported_reasoning_levels"].as_array().into_iter().flatten().filter_map(|l| l["effort"].as_str()).filter(|e| known.contains(e)).map(String::from).collect();
            Some(ModelInfo { id: id.to_string(), label: m["display_name"].as_str().unwrap_or(id).to_string(), efforts })
        })
        .collect()
}

/// Models from `agy models`: `id<TAB>label` lines after "Fetching available models...".
pub fn parse_agy(text: &str) -> Vec<ModelInfo> {
    text.lines()
        .filter_map(|l| {
            let (id, label) = l.split_once('\t')?;
            let id = id.trim();
            launch::plain_model_id(id).then(|| ModelInfo { id: id.to_string(), label: label.trim().to_string(), efforts: vec![] })
        })
        .collect()
}

/// Models from `grok models`: the indented lines under "Available models:",
/// the default one marked `* … (default)`.
pub fn parse_grok(text: &str) -> Vec<ModelInfo> {
    text.lines()
        .skip_while(|l| l.trim() != "Available models:")
        .skip(1)
        .map(str::trim)
        .take_while(|l| !l.is_empty())
        .filter_map(|l| {
            let id = l.trim_start_matches('*').trim().trim_end_matches("(default)").trim();
            launch::plain_model_id(id).then(|| ModelInfo { id: id.to_string(), label: id.to_string(), efforts: vec![] })
        })
        .collect()
}

/// The arguments that make an agent print its models.
fn listing_args(agent: Harness) -> &'static [&'static str] {
    match agent {
        Harness::Codex => &["debug", "models"],
        Harness::Antigravity | Harness::Grok => &["models"],
        Harness::ClaudeCode => &[],
    }
}

/// An agent's choices from its listing; with none (not run, failed, timed
/// out) it offers no models, so only "Default".
pub fn info_for(agent: Harness, listing: Option<&str>) -> AgentInfo {
    if agent == Harness::ClaudeCode {
        return claude();
    }
    let models = match (agent, listing) {
        (Harness::Codex, Some(t)) => parse_codex(t),
        (Harness::Antigravity, Some(t)) => parse_agy(t),
        (Harness::Grok, Some(t)) => parse_grok(t),
        _ => vec![],
    };
    // With "Default" chosen the model may be any of them: offer what all take.
    let efforts = if agent == Harness::Codex {
        launch::efforts(agent).iter().filter(|e| models.iter().all(|m| m.efforts.iter().any(|x| x == *e))).map(|e| e.to_string()).collect()
    } else {
        strings(launch::efforts(agent))
    };
    AgentInfo { harness: agent, models, efforts, modes: strings(launch::modes(agent)) }
}

/// Claude Code, then each other agent `find` locates, with the models `run` lists for it.
pub fn build(find: impl Fn(&str) -> Option<PathBuf>, run: impl Fn(&Path, &[&str]) -> Option<String>) -> Vec<AgentInfo> {
    let mut out = vec![claude()];
    for agent in [Harness::Codex, Harness::Antigravity, Harness::Grok] {
        let Some(bin) = find(launch::binary_name(agent)) else { continue };
        let listing = run(&bin, listing_args(agent));
        out.push(info_for(agent, listing.as_deref()));
    }
    out
}

pub const LISTING_TIMEOUT: Duration = Duration::from_secs(20);

/// What `binary args` printed, if it exited successfully within `timeout`.
/// Stdout is read on a thread of its own: Codex's catalogue is far bigger
/// than a pipe holds, and a child blocked on a full pipe never exits.
pub fn run_listing(binary: &Path, args: &[&str], timeout: Duration) -> Option<String> {
    let mut child = crate::command(binary)
        .args(args)
        .env_clear()
        .envs(launch::clean_env(std::env::vars()))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut stdout, &mut s);
        let _ = tx.send(s);
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    // A helper the agent left running may hold stdout open: do not wait on it for long.
    let out = rx.recv_timeout(Duration::from_secs(2)).ok()?;
    status.success().then_some(out)
}

const MAX_AGE: Duration = Duration::from_secs(600);

struct Cache {
    list: Option<Vec<AgentInfo>>,
    at: Option<Instant>,
    fetching: bool,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache { list: None, at: None, fetching: false });
static READY: Condvar = Condvar::new();

/// Starts a listing on a thread of its own when the last is missing or stale.
fn refresh_if_stale(c: &mut Cache) {
    if c.fetching || c.at.is_some_and(|at| at.elapsed() < MAX_AGE) {
        return;
    }
    c.fetching = true;
    std::thread::spawn(|| {
        let list = build(launch::agent_binary, |bin, args| run_listing(bin, args, LISTING_TIMEOUT));
        let mut c = CACHE.lock().unwrap();
        c.list = Some(list);
        c.at = Some(Instant::now());
        c.fetching = false;
        READY.notify_all();
    });
}

/// The agents as last listed, starting a new listing when that one is over
/// ten minutes old. Never waits: before the first listing it is Claude Code alone.
pub fn snapshot() -> Vec<AgentInfo> {
    let mut c = CACHE.lock().unwrap();
    refresh_if_stale(&mut c);
    c.list.clone().unwrap_or_else(|| vec![claude()])
}

/// The agents, waiting for the first listing when there is none yet.
pub fn current() -> Vec<AgentInfo> {
    let mut c = CACHE.lock().unwrap();
    refresh_if_stale(&mut c);
    while c.list.is_none() {
        c = READY.wait(c).unwrap();
    }
    c.list.clone().unwrap_or_default()
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p maya-core agents::`
Expected: PASS (the two `run_listing` tests are Unix-only).

- [ ] **Step 5: Commit**

```bash
git add core/src/agents.rs core/src/lib.rs core/fixtures/codex/models.json core/fixtures/antigravity/models.txt core/fixtures/grok/models.txt
git commit -m "feat(core): list installed agents and the models each offers"
```

---

### Task 3: Card names follow renames

Codex and Antigravity names are read once, when a process is first matched, and Grok's registry entry is kept by pid, so a `/rename` never reaches the card until Maya restarts.

**Files:**
- Modify: `core/src/store.rs` (`refresh_foreign`, a new `NameFiles` cache)
- Test: `core/src/store.rs` tests module (or `core/src/store/tests` wherever the existing store tests live: `grep -n "mod tests" core/src/store.rs`)

**Interfaces:**
- Consumes: `codex::thread_name(&str, &str)`, `antigravity::conversation_name(&str, &str)`, `foreign::grok_sessions`
- Produces: names on foreign cards that change when the agent's files change.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn a_codex_rename_reaches_the_card_on_the_next_refresh() {
        let t = tempfile::tempdir().unwrap();
        let codex = t.path().join("codex");
        std::fs::create_dir_all(&codex).unwrap();
        let rollout = codex.join("rollout.jsonl");
        std::fs::write(&rollout, "").unwrap();
        std::fs::write(codex.join("session_index.jsonl"), "{\"id\":\"c1\",\"thread_name\":\"Say hi\"}\n").unwrap();
        let s = ForeignSession { harness: Harness::Codex, pid: 42, tty: Some("ttys001".into()), session_id: "c1".into(), cwd: "/x".into(), name: "Say hi".into(), transcript_path: rollout };
        let mut store = Store::new(t.path().join("claude")).with_alive(|_| true).with_foreign(codex.clone(), t.path().join("agy"), vec![s]);
        assert_eq!(store.refresh(1).iter().find(|c| c.session_id == "c1").unwrap().name, "Say hi");
        std::fs::write(codex.join("session_index.jsonl"), "{\"id\":\"c1\",\"thread_name\":\"Say hi\"}\n{\"id\":\"c1\",\"thread_name\":\"Fix CI\"}\n").unwrap();
        assert_eq!(store.refresh(2).iter().find(|c| c.session_id == "c1").unwrap().name, "Fix CI");
    }

    #[test]
    fn an_antigravity_rename_reaches_the_card_on_the_next_refresh() {
        let t = tempfile::tempdir().unwrap();
        let agy = t.path().join("agy");
        std::fs::create_dir_all(&agy).unwrap();
        let s = ForeignSession { harness: Harness::Antigravity, pid: 43, tty: Some("ttys002".into()), session_id: "a1".into(), cwd: "/x".into(), name: "agy-43".into(), transcript_path: agy.join("t.jsonl") };
        let mut store = Store::new(t.path().join("claude")).with_alive(|_| true).with_foreign(t.path().join("codex"), agy.clone(), vec![s]);
        std::fs::write(agy.join("history.jsonl"), "{\"display\":\"/rename Fix CI\",\"timestamp\":3,\"workspace\":\"/x\",\"conversationId\":\"a1\",\"type\":\"slash_command\"}\n").unwrap();
        assert_eq!(store.refresh(1).iter().find(|c| c.session_id == "a1").unwrap().name, "Fix CI");
    }
```

(If `ForeignSession`/`Harness` are not yet imported in the store tests, add `use crate::foreign::ForeignSession; use crate::model::Harness;`.)

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core store::`
Expected: FAIL, left `"Say hi"` / `"agy-43"`.

- [ ] **Step 3: Implement**

In `core/src/store.rs` add a cache that re-reads a file only when its length or modification time changes (`history.jsonl` grows with every prompt):

`Store` must stay `Send` (it lives in a `Mutex` shared across threads), so the cached text is an `Arc<str>`:

```rust
/// Text of the files names come from, read again only when they change.
#[derive(Default)]
struct NameFiles(std::collections::HashMap<PathBuf, (Option<std::time::SystemTime>, u64, std::sync::Arc<str>)>);

impl NameFiles {
    fn read(&mut self, path: &Path) -> std::sync::Arc<str> {
        let meta = std::fs::metadata(path).ok();
        let stamp = (meta.as_ref().and_then(|m| m.modified().ok()), meta.as_ref().map_or(0, |m| m.len()));
        if let Some((m, len, text)) = self.0.get(path) {
            if (*m, *len) == stamp {
                return text.clone();
            }
        }
        let text: std::sync::Arc<str> = std::fs::read_to_string(path).unwrap_or_default().into();
        self.0.insert(path.to_path_buf(), (stamp.0, stamp.1, text.clone()));
        text
    }
}
```

Add a field `names: NameFiles` to `Store` (`names: NameFiles::default()` in `new`). At the end of `refresh_foreign`, replace the Grok loop and add the name pass:

```rust
        for s in grok {
            // Grok's title changes with `/rename`: keep the newest.
            self.foreign.entry(s.pid).and_modify(|e| e.name = s.name.clone()).or_insert(s);
        }
        self.refresh_names();
    }

    /// Re-reads Codex and Antigravity names, which change with `/rename`.
    fn refresh_names(&mut self) {
        let has = |h: Harness| self.foreign.values().any(|s| s.harness == h);
        if has(Harness::Codex) {
            let index = self.names.read(&self.codex_dir.join("session_index.jsonl"));
            for s in self.foreign.values_mut().filter(|s| s.harness == Harness::Codex) {
                if let Some(n) = crate::codex::thread_name(&index, &s.session_id) {
                    s.name = n;
                }
            }
        }
        if has(Harness::Antigravity) {
            let history = self.names.read(&self.agy_dir.join("history.jsonl"));
            for s in self.foreign.values_mut().filter(|s| s.harness == Harness::Antigravity) {
                if let Some(n) = crate::antigravity::conversation_name(&history, &s.session_id) {
                    s.name = n;
                }
            }
        }
    }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p maya-core`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/store.rs
git commit -m "fix(core): show other agents' new names after a rename"
```

---

### Task 4: Pending names

**Files:**
- Create: `core/src/pending_names.rs`
- Modify: `core/src/lib.rs` (`pub mod pending_names;`)
- Modify: `core/src/store.rs` (field, apply in `refresh`, accessors)

**Interfaces:**
- Consumes: `model::{Card, Harness, State}`
- Produces:
  - `pending_names::PendingName::new(harness: Harness, cwd: &str, name: &str, launched_ms: u64, known: Vec<String>, session_id: Option<String>) -> PendingName`
  - `pending_names::PendingNames` with `add(PendingName)`, `forget(&str)`, `apply(&mut self, cards: &mut [Card], now_ms: u64) -> Vec<(String, String)>`
  - `pending_names::is_free(&Card) -> bool`
  - `pending_names::MATCH_WINDOW_MS: u64 = 600_000`
  - `Store::add_pending_name(PendingName)`, `Store::forget_pending_name(&str)`, `Store::take_due_renames() -> Vec<(String, String)>`

- [ ] **Step 1: Write the failing tests** (in `pending_names.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Card, Harness, State};

    fn card(id: &str, harness: Harness, cwd: &str, state: State, snippet: &str) -> Card {
        Card { session_id: id.into(), pid: 1, name: format!("auto-{id}"), cwd: cwd.into(), state, state_since: 0, snippet: snippet.into(), awaiting: None, has_inbox: false, harness, pr: None, context: None, machine: None, machine_address: None, machine_platform: None, terminal: None, stale: false }
    }

    fn pending(name: &str, known: &[&str]) -> PendingName {
        PendingName::new(Harness::Codex, "/dev/a", name, 1_000, known.iter().map(|s| s.to_string()).collect(), None)
    }

    #[test]
    fn a_new_session_shows_the_name_at_once_and_is_renamed_once_free() {
        let mut p = PendingNames::default();
        p.add(pending("Fix CI", &["old"]));
        let mut cards = vec![card("old", Harness::Codex, "/dev/a", State::Idle, "x"), card("new", Harness::Codex, "/dev/a/", State::Working, "")];
        assert!(p.apply(&mut cards, 2_000).is_empty(), "working: not yet");
        assert_eq!(cards[1].name, "Fix CI");
        assert_eq!(cards[0].name, "auto-old", "a session running before the launch is never it");
        cards[1].state = State::Completed;
        assert_eq!(p.apply(&mut cards, 3_000), vec![("new".to_string(), "Fix CI".to_string())]);
        assert!(p.apply(&mut cards, 4_000).is_empty(), "only once");
        assert_eq!(cards[1].name, "Fix CI", "still shown until the agent's own name is it");
        cards[1].name = "Fix CI".into();
        p.apply(&mut cards, 5_000);
        assert!(p.is_empty());
    }

    #[test]
    fn a_session_that_has_not_started_its_turn_is_not_renamed() {
        // Idle with no reply yet: the first prompt may still be on its way in.
        let mut p = PendingNames::default();
        p.add(pending("Fix CI", &[]));
        let mut cards = vec![card("new", Harness::Codex, "/dev/a", State::Idle, "")];
        assert!(p.apply(&mut cards, 2_000).is_empty());
        cards[0].snippet = "Hi!".into();
        assert_eq!(p.apply(&mut cards, 3_000).len(), 1);
    }

    #[test]
    fn two_names_in_one_folder_claim_different_sessions() {
        let mut p = PendingNames::default();
        p.add(pending("First", &[]));
        p.add(pending("Second", &[]));
        let mut cards = vec![card("s1", Harness::Codex, "/dev/a", State::Working, ""), card("s2", Harness::Codex, "/dev/a", State::Working, "")];
        p.apply(&mut cards, 2_000);
        assert_eq!((cards[0].name.as_str(), cards[1].name.as_str()), ("First", "Second"));
    }

    #[test]
    fn only_the_same_agent_and_folder_match() {
        let mut p = PendingNames::default();
        p.add(pending("Fix CI", &[]));
        let mut cards = vec![card("g", Harness::Grok, "/dev/a", State::Working, ""), card("b", Harness::Codex, "/dev/b", State::Working, "")];
        p.apply(&mut cards, 2_000);
        assert!(cards.iter().all(|c| c.name.starts_with("auto-")));
    }

    #[test]
    fn a_grok_name_waits_for_its_own_session_id() {
        let mut p = PendingNames::default();
        p.add(PendingName::new(Harness::Grok, "/dev/a", "Fix CI", 1_000, vec![], Some("g-1".into())));
        let mut cards = vec![card("other", Harness::Grok, "/dev/a", State::Working, "")];
        p.apply(&mut cards, 2_000);
        assert_eq!(cards[0].name, "auto-other");
        cards.push(card("g-1", Harness::Grok, "/dev/a", State::Working, ""));
        p.apply(&mut cards, 3_000);
        assert_eq!(cards[1].name, "Fix CI");
    }

    #[test]
    fn names_are_dropped_when_the_session_ends_or_never_comes() {
        let mut p = PendingNames::default();
        p.add(pending("Ends", &[]));
        let mut cards = vec![card("s1", Harness::Codex, "/dev/a", State::Working, "")];
        p.apply(&mut cards, 2_000);
        p.apply(&mut [], 3_000);
        assert!(p.is_empty(), "its session ended");
        p.add(pending("Never", &[]));
        p.apply(&mut [], 1_000 + MATCH_WINDOW_MS - 1);
        assert!(!p.is_empty());
        p.apply(&mut [], 1_000 + MATCH_WINDOW_MS);
        assert!(p.is_empty(), "unmatched for ten minutes");
    }

    #[test]
    fn forget_drops_the_name_of_one_session() {
        let mut p = PendingNames::default();
        p.add(pending("Fix CI", &[]));
        let mut cards = vec![card("s1", Harness::Codex, "/dev/a", State::Working, "")];
        p.apply(&mut cards, 2_000);
        p.forget("s1");
        assert!(p.is_empty());
    }

    #[test]
    fn free_means_completed_or_idle() {
        assert!(is_free(&card("a", Harness::Codex, "/", State::Completed, "")));
        assert!(is_free(&card("a", Harness::Codex, "/", State::Idle, "")));
        assert!(!is_free(&card("a", Harness::Codex, "/", State::Working, "")));
        assert!(!is_free(&card("a", Harness::Codex, "/", State::Awaiting, "")));
    }
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p maya-core pending_names::`
Expected: compile error, module not found.

- [ ] **Step 3: Implement `core/src/pending_names.rs`**

```rust
//! Names chosen in the New session modal for agents that take none when
//! they start (Codex, Antigravity, Grok Build). The name shows on the new
//! session's card at once; `/rename` is typed once the session is free.
//! Typing earlier is unsafe: keys sent to a TUI that is not at its input
//! box act as shortcuts.

use crate::model::{Card, Harness, State};
use std::collections::HashSet;

/// How long a name waits for its session to appear.
pub const MATCH_WINDOW_MS: u64 = 10 * 60 * 1000;

#[derive(Debug, Clone)]
pub struct PendingName {
    harness: Harness,
    cwd: String,
    name: String,
    launched_ms: u64,
    /// Sessions running at launch: none of them is the new one.
    known: HashSet<String>,
    /// The session, once matched; set from the start for Grok's `--session-id`.
    session_id: Option<String>,
    /// Its card has been on the board.
    seen: bool,
    seen_working: bool,
    renamed: bool,
}

impl PendingName {
    pub fn new(harness: Harness, cwd: &str, name: &str, launched_ms: u64, known: Vec<String>, session_id: Option<String>) -> Self {
        Self { harness, cwd: cwd.to_string(), name: name.to_string(), launched_ms, known: known.into_iter().collect(), session_id, seen: false, seen_working: false, renamed: false }
    }
}

/// Ready for typed input: not running a turn and not asking anything.
pub fn is_free(card: &Card) -> bool {
    matches!(card.state, State::Completed | State::Idle)
}

fn same_dir(a: &str, b: &str) -> bool {
    a.trim_end_matches(['/', '\\']) == b.trim_end_matches(['/', '\\'])
}

#[derive(Debug, Default)]
pub struct PendingNames(Vec<PendingName>);

impl PendingNames {
    pub fn add(&mut self, p: PendingName) {
        self.0.push(p);
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Drops the name waiting for `session_id`: the user renamed it themselves.
    pub fn forget(&mut self, session_id: &str) {
        self.0.retain(|p| p.session_id.as_deref() != Some(session_id));
    }

    /// Matches names to new sessions, shows them on `cards`, and returns
    /// `(session id, name)` for each session to `/rename` now. A name is
    /// dropped once the agent's own name is it, once its session has ended,
    /// or when no session came within `MATCH_WINDOW_MS`.
    pub fn apply(&mut self, cards: &mut [Card], now_ms: u64) -> Vec<(String, String)> {
        let mut claimed: HashSet<String> = self.0.iter().filter_map(|p| p.session_id.clone()).collect();
        for p in self.0.iter_mut().filter(|p| p.session_id.is_none()) {
            let found = cards.iter().find(|c| c.harness == p.harness && same_dir(&c.cwd, &p.cwd) && !p.known.contains(&c.session_id) && !claimed.contains(&c.session_id));
            if let Some(c) = found {
                claimed.insert(c.session_id.clone());
                p.session_id = Some(c.session_id.clone());
            }
        }
        let mut due = Vec::new();
        self.0.retain_mut(|p| {
            let waiting = now_ms.saturating_sub(p.launched_ms) < MATCH_WINDOW_MS;
            let Some(id) = p.session_id.clone() else { return waiting };
            let Some(card) = cards.iter_mut().find(|c| c.session_id == id) else {
                // Gone after it was seen: ended. Never seen (Grok's id): still coming.
                return !p.seen && waiting;
            };
            p.seen = true;
            if card.name == p.name {
                return false;
            }
            card.name = p.name.clone();
            p.seen_working |= card.state == State::Working;
            // A turn must have run: before that the first prompt may still be arriving.
            if !p.renamed && is_free(card) && (p.seen_working || !card.snippet.is_empty()) {
                p.renamed = true;
                due.push((id, p.name.clone()));
            }
            true
        });
        due
    }
}
```

Add `pub mod pending_names;` to `core/src/lib.rs`.

In `core/src/store.rs`: fields `pending: crate::pending_names::PendingNames` and `due_renames: Vec<(String, String)>` (both `Default` in `new`). At the end of `refresh`, before `cards` is returned:

```rust
        let due = self.pending.apply(&mut cards, now_ms);
        self.due_renames.extend(due);
        cards
```

(`cards` must be `let mut cards`; it already is.) And the accessors:

```rust
    pub fn add_pending_name(&mut self, p: crate::pending_names::PendingName) {
        self.pending.add(p);
    }

    pub fn forget_pending_name(&mut self, session_id: &str) {
        self.pending.forget(session_id);
        self.due_renames.retain(|(id, _)| id != session_id);
    }

    /// Sessions to `/rename` now, each returned once.
    pub fn take_due_renames(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.due_renames)
    }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p maya-core`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/pending_names.rs core/src/lib.rs core/src/store.rs
git commit -m "feat(core): pending names for agents that take no name flag"
```

---

### Task 5: Start any agent, and rename any session

**Files:**
- Modify: `core/src/store.rs` (agents source)
- Modify: `core/src/actions.rs` (`start_session`, `rename_session`, new `run_due_renames`)
- Test: `core/src/actions.rs` tests module

**Interfaces:**
- Consumes: Tasks 1, 2, 4.
- Produces:
  - `Store::agents_source(&self) -> Arc<dyn Fn() -> Vec<AgentInfo> + Send + Sync>`; test-support `Store::with_agents(self, Vec<AgentInfo>) -> Self` (`with_alive` sets `vec![agents::claude()]`)
  - `actions::start_session(l, dir, prompt, options)` (same signature) starting `options.agent`
  - `actions::rename_session(l, session_id, name)` for every agent
  - `actions::run_due_renames(l: &Local)`

- [ ] **Step 1: Write the failing tests** (in `actions.rs` tests, next to `start_session_opens_the_session_command_under_a_maya_label`; reuse that test's setup helpers for the store, projects dir and `FakeTerminal`; read it first)

```rust
    fn codex_info() -> crate::agents::AgentInfo {
        crate::agents::info_for(Harness::Codex, Some(r#"{"models":[{"slug":"gpt-6.1-sol","display_name":"GPT-6.1-Sol","visibility":"list","supported_reasoning_levels":[{"effort":"low"}]}]}"#))
    }

    #[test]
    fn start_session_runs_the_chosen_agent_and_keeps_its_name_for_later() {
        // Same setup as start_session_opens_the_session_command_under_a_maya_label,
        // with the store built `.with_agents(vec![agents::claude(), codex_info()])`.
        let opts = LaunchOptions { agent: Harness::Codex, model: Some("gpt-6.1-sol".into()), name: Some("Fix CI".into()), ..Default::default() };
        start_session(&l, Some("proj".into()), "hello".into(), opts).unwrap();
        let Call::Open { command, .. } = fake.calls.lock().unwrap()[0].clone() else { panic!() };
        assert!(command.ends_with("&& codex -m gpt-6.1-sol -- \"$p\""), "{command}");
        // A Codex session in that folder, new since the launch, takes the name.
        let mut cards = vec![test_card("c-new", Harness::Codex, &proj_path)];
        store.lock().unwrap().pending_apply_for_test(&mut cards);
        assert_eq!(cards[0].name, "Fix CI");
    }

    #[test]
    fn start_session_refuses_an_agent_that_is_not_installed_or_a_model_it_does_not_list() {
        let opts = LaunchOptions { agent: Harness::Antigravity, ..Default::default() };
        assert!(start_session(&l, Some("proj".into()), "hello".into(), opts).unwrap_err().contains("not installed"));
        let opts = LaunchOptions { agent: Harness::Codex, model: Some("gpt-9".into()), ..Default::default() };
        assert!(start_session(&l, Some("proj".into()), "hello".into(), opts).unwrap_err().contains("model"));
        assert!(fake.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn a_grok_start_passes_a_session_id() {
        // Store `.with_agents(vec![agents::claude(), agents::info_for(Harness::Grok, None)])`.
        start_session(&l, Some("proj".into()), "hello".into(), LaunchOptions { agent: Harness::Grok, name: Some("x".into()), ..Default::default() }).unwrap();
        let Call::Open { command, .. } = fake.calls.lock().unwrap()[0].clone() else { panic!() };
        assert!(command.contains("grok --session-id '") && command.ends_with("' -- \"$p\""), "{command}");
    }

    #[test]
    fn rename_reaches_other_agents_only_when_they_are_free() {
        // A store `.with_foreign(..)` holding a Codex session "c1" on tty "ttys009"
        // whose rollout tail makes it Working (a `task_started` with no
        // `task_complete`; copy a line from core/fixtures/codex).
        assert!(rename_session(&l, "c1", "Fix CI").unwrap_err().contains("free"));
        // Then rewrite the rollout so the turn has completed.
        rename_session(&l, "c1", "Fix CI").unwrap();
        assert_eq!(fake.calls.lock().unwrap().last().unwrap(), &Call::Type { tty: "ttys009".into(), text: "/rename Fix CI".into() });
    }

    #[test]
    fn a_pending_name_is_typed_as_one_rename_line() {
        // A free Codex session "c1" on "ttys009" (completed rollout) and a pending
        // name for it whose session already finished a turn.
        store.lock().unwrap().add_pending_name(PendingName::new(Harness::Codex, "/x", "it's \"$(rm -rf ~)\"", now_ms(), vec![], Some("c1".into())));
        store.lock().unwrap().refresh(now_ms());
        run_due_renames(&l);
        run_due_renames(&l);
        let typed: Vec<Call> = fake.calls.lock().unwrap().clone();
        assert_eq!(typed, vec![Call::Type { tty: "ttys009".into(), text: "/rename it's \"$(rm -rf ~)\"".into() }], "typed once, literally");
    }
```

`pending_apply_for_test` is a `#[cfg(any(test, feature = "test-support"))]` helper on `Store` that calls `self.pending.apply(cards, now_ms())` (add it in this task). `test_card` builds a `Card` like Task 4's `card()`; put it in the actions tests. Fill the `// same setup` comments by copying the setup lines of the existing test they name: the code to copy exists in the file, so read it before writing these tests.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p maya-core actions::`
Expected: FAIL / compile errors (`with_agents`, `run_due_renames`, `pending_apply_for_test`).

- [ ] **Step 3: Implement**

`core/src/store.rs`: a field `agents: std::sync::Arc<dyn Fn() -> Vec<crate::agents::AgentInfo> + Send + Sync>`, `Arc::new(crate::agents::current)` in `new`; in `with_alive` add `self.agents = std::sync::Arc::new(|| vec![crate::agents::claude()]);`; then:

```rust
    /// Where the installed agents come from; called without the store's lock
    /// held, since the first listing can take seconds.
    pub fn agents_source(&self) -> std::sync::Arc<dyn Fn() -> Vec<crate::agents::AgentInfo> + Send + Sync> {
        self.agents.clone()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_agents(mut self, list: Vec<crate::agents::AgentInfo>) -> Self {
        self.agents = std::sync::Arc::new(move || list.clone());
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn pending_apply_for_test(&mut self, cards: &mut [Card]) {
        self.pending.apply(cards, now_ms());
    }
```

`core/src/actions.rs` `start_session`:

```rust
pub fn start_session(l: &Local, dir: Option<String>, prompt: String, options: launch::LaunchOptions) -> Result<StartResult, String> {
    if prompt.trim().is_empty() {
        return Err("Type a prompt first.".into());
    }
    options.validate_shape()?;
    let (root, maya_dir, agents) = {
        let store = l.store.lock().unwrap();
        (projects_root(&store)?, store.claude_dir().join("maya"), store.agents_source())
    };
    let label_of = |h: model::Harness| match h {
        model::Harness::ClaudeCode => "Claude Code",
        model::Harness::Codex => "Codex",
        model::Harness::Antigravity => "Antigravity",
        model::Harness::Grok => "Grok Build",
    };
    let info = agents().into_iter().find(|a| a.harness == options.agent).ok_or_else(|| format!("{} is not installed on this machine.", label_of(options.agent)))?;
    options.validate(&info.model_ids())?;
    let dirs = launch::list_project_dirs(&root);
    let picked = match dir {
        Some(_) => None,
        None => {
            let binary = launch::claude_binary().ok_or("Could not find the claude command.")?;
            launch::classify(&binary, &root, &prompt, &dirs, launch::CLASSIFIER_TIMEOUT)
        }
    };
    let (target, how) = launch::resolve_target(&root, &dirs, dir.as_deref(), picked.as_deref())?;
    let file = launch::write_prompt_file(&maya_dir, &prompt)?;
    let grok_id = (options.agent == model::Harness::Grok).then(launch::new_session_uuid);
    // Sessions already running cannot be the new one.
    let known = l.store.lock().unwrap().live_session_ids();
    let terminal = l.terminal.open(&launch::session_command(&target, &file, &options, grok_id.as_deref()), &target, &tmux_label())?;
    if let (Some(name), false) = (options.chosen_name(), options.agent == model::Harness::ClaudeCode) {
        let target = target.to_string_lossy();
        l.store.lock().unwrap().add_pending_name(crate::pending_names::PendingName::new(options.agent, &target, name, now_ms(), known, grok_id));
    }
    Ok(StartResult { dir: target.to_string_lossy().into_owned(), how: how.to_string(), terminal })
}
```

`rename_session`:

```rust
/// Types `/rename <name>` into the session's terminal. Claude Code takes it
/// whenever it is not asking a question; another agent only when free, since
/// keys typed into its TUI elsewhere act as shortcuts. The new name comes
/// back through the agent's own files on the next refresh.
pub fn rename_session(l: &Local, session_id: &str, name: &str) -> Result<(), String> {
    let line = answer::rename_command(name)?;
    let tty = {
        let mut store = l.store.lock().unwrap();
        let card = store.card_for(session_id, now_ms()).ok_or("Session is no longer running.")?;
        if card.harness == model::Harness::ClaudeCode {
            answer::check_free(&card)?;
        } else if !crate::pending_names::is_free(&card) {
            return Err("Wait until the session is free to rename it.".into());
        }
        // The user's own name wins over one still waiting from the start.
        store.forget_pending_name(session_id);
        session_tty(&store, session_id, card.pid)?
    };
    l.terminal.type_line(&tty, &line)
}
```

`run_due_renames`:

```rust
/// Types `/rename` into the sessions whose pending name is due. Looked up
/// under the store's lock, typed after it is released.
pub fn run_due_renames(l: &Local) {
    let due: Vec<(String, String)> = {
        let mut store = l.store.lock().unwrap();
        store
            .take_due_renames()
            .into_iter()
            .filter_map(|(id, name)| {
                let pid = store.foreign(&id)?.pid;
                Some((session_tty(&store, &id, pid).ok()?, answer::rename_command(&name).ok()?))
            })
            .collect()
    };
    for (tty, line) in due {
        if let Err(e) = l.terminal.type_line(&tty, &line) {
            crate::log::line("rename", format!("could not rename the session on {tty}: {e}"));
        }
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p maya-core && cargo build --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/store.rs core/src/actions.rs
git commit -m "feat(core): start any agent, and rename Codex, Antigravity and Grok sessions"
```

---

### Task 6: Agents on the network board

**Files:**
- Modify: `core/src/net/protocol.rs` (`Up::Board`)
- Modify: `core/src/net/merge.rs` (`RemoteBoard.agents`, `agents_of`)
- Modify: `core/src/net/server.rs` (store `agents` from the board, test helpers)
- Modify: `core/src/net/client.rs` (`Executor::agents`, send it)
- Modify: `cli/src/executor.rs`, `src-tauri/src/net_app.rs` (`fn agents`)

**Interfaces:**
- Consumes: `agents::{AgentInfo, snapshot}`
- Produces:
  - `Up::Board { cards, dirs, #[serde(default, skip_serializing_if = "Option::is_none")] agents: Option<Vec<AgentInfo>> }`
  - `RemoteBoard { …, agents: Option<Vec<AgentInfo>> }`
  - `merge::agents_of(remotes: &[RemoteBoard], machine: &str) -> Option<Vec<AgentInfo>>` (None: an older Maya, or no such machine)
  - `client::Executor::agents(&self) -> Option<Vec<AgentInfo>>` with default `None`

- [ ] **Step 1: Write the failing tests**

In `core/src/net/protocol.rs` tests (or merge.rs tests if protocol has none):

```rust
    #[test]
    fn a_board_from_an_older_maya_has_no_agents() {
        let old = r#"{"type":"board","cards":[],"dirs":["proj"]}"#;
        let Up::Board { agents, .. } = serde_json::from_str::<Up>(old).unwrap() else { panic!() };
        assert_eq!(agents, None);
        let new = encode(&Up::Board { cards: vec![], dirs: vec![], agents: Some(vec![crate::agents::claude()]) });
        assert!(new.contains("\"agents\":[{\"harness\":\"claude-code\""), "{new}");
    }
```

In `core/src/net/merge.rs` tests:

```rust
    #[test]
    fn agents_of_names_a_machines_agents_or_none_for_an_older_maya() {
        let mut a = board("a", "h", &[], 0, true);
        a.agents = Some(vec![crate::agents::claude()]);
        let b = board("b", "h", &[], 0, true);
        let boards = vec![a, b];
        assert_eq!(agents_of(&boards, "a").unwrap()[0].harness, crate::model::Harness::ClaudeCode);
        assert_eq!(agents_of(&boards, "b"), None);
        assert_eq!(agents_of(&boards, "zzz"), None);
    }
```

In `core/src/net/server.rs`'s end-to-end test that sends `Up::Board { cards: vec![card("r1", "remote")], dirs: vec!["proj".into()] }` (line ~1207): send `agents: Some(vec![crate::agents::claude()])` and, after the `wait_until`, assert `handle.boards().iter().find(|b| b.machine == "laptop").unwrap().agents.as_ref().unwrap().len() == 1`.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p maya-core net::`
Expected: compile errors.

- [ ] **Step 3: Implement**

- `protocol.rs`: `Board { cards: Vec<Card>, dirs: Vec<String>, #[serde(default, skip_serializing_if = "Option::is_none")] agents: Option<Vec<crate::agents::AgentInfo>> }`.
- `merge.rs`: add `pub agents: Option<Vec<crate::agents::AgentInfo>>,` to `RemoteBoard` and to the test helper `board(..)` (`agents: None`), and:

```rust
/// The agents `machine` can start, as it last reported them; None for an
/// older Maya, which starts Claude Code only.
pub fn agents_of(remotes: &[RemoteBoard], machine: &str) -> Option<Vec<crate::agents::AgentInfo>> {
    remotes.iter().find(|b| b.machine == machine).and_then(|b| b.agents.clone())
}
```

- `server.rs`: in `handle`, `Up::Board { cards, dirs, agents }` and store `agents` where `dirs` is stored on the `RemoteBoard` (follow `dirs` through the arm); add `agents: None` (or the field) to every `Up::Board`/`RemoteBoard` literal the compiler points at.
- `client.rs`: in `trait Executor` add

```rust
    /// The agents this machine can start, for the main's New session modal.
    fn agents(&self) -> Option<Vec<crate::agents::AgentInfo>> {
        None
    }
```

and at line ~401: `let (cards, dirs) = exec.board(); let frame = encode(&Up::Board { cards, dirs, agents: exec.agents() });`.
- `cli/src/executor.rs` and `src-tauri/src/net_app.rs`: implement `fn agents(&self) -> Option<Vec<AgentInfo>> { Some(maya_core::agents::snapshot()) }` (`snapshot` never waits; the 10-minute cache keeps the frame unchanged between listings, so the "board changed" comparison still holds).

- [ ] **Step 4: Run the tests**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/net cli/src/executor.rs src-tauri/src/net_app.rs
git commit -m "feat(net): each machine reports the agents it can start"
```

---

### Task 7: The app and CLI: list agents, start them, type due renames

**Files:**
- Modify: `src-tauri/src/lib.rs` (`list_agents` command, remote start guard, `run_due_renames` in `refresh_and_emit`, warm the cache at startup)
- Modify: `cli/src/run_cmd.rs` (`run_due_renames` in the watcher)
- Modify: `cli/src/args.rs` (`--agent`, `--name`), `cli/src/main.rs` or wherever `USAGE` is defined
- Test: `src-tauri/src/lib.rs` tests (a pure helper), `cli/src/args.rs` tests

**Interfaces:**
- Consumes: Tasks 1–6.
- Produces:
  - Tauri command `list_agents(machine: Option<String>) -> AgentsReply { agents: Vec<AgentInfo>, names: bool }` (serde camelCase)
  - pure `agents_reply(local: bool, remote: Option<Vec<AgentInfo>>, local_list: impl FnOnce() -> Vec<AgentInfo>) -> AgentsReply`
  - pure `check_remote_agent(agent: Harness, remote: &Option<Vec<AgentInfo>>, machine: &str) -> Result<(), String>`
  - CLI: `maya start … --agent <claude-code|codex|antigravity|grok> --name <name>`

- [ ] **Step 1: Write the failing tests**

`src-tauri/src/lib.rs` tests:

```rust
    #[test]
    fn an_older_remote_offers_claude_and_no_name() {
        let r = agents_reply(false, None, || panic!("not local"));
        assert_eq!(r.agents, vec![maya_core::agents::claude()]);
        assert!(!r.names);
        let r = agents_reply(false, Some(vec![maya_core::agents::claude(), maya_core::agents::info_for(Harness::Grok, None)]), || panic!("not local"));
        assert_eq!(r.agents.len(), 2);
        assert!(r.names);
        assert!(agents_reply(true, None, || vec![maya_core::agents::claude()]).names);
    }

    #[test]
    fn a_remote_start_needs_the_agent_on_that_machine() {
        assert!(check_remote_agent(Harness::ClaudeCode, &None, "laptop").is_ok());
        assert!(check_remote_agent(Harness::Codex, &None, "laptop").unwrap_err().contains("laptop"));
        let grok_only = Some(vec![maya_core::agents::claude(), maya_core::agents::info_for(Harness::Grok, None)]);
        assert!(check_remote_agent(Harness::Grok, &grok_only, "laptop").is_ok());
        assert!(check_remote_agent(Harness::Codex, &grok_only, "laptop").unwrap_err().contains("not installed"));
    }
```

`cli/src/args.rs` tests:

```rust
    #[test]
    fn start_takes_an_agent_and_a_name() {
        let Cmd::Start { options, .. } = parse(&["start".into(), "--agent".into(), "codex".into(), "--name".into(), "Fix CI".into(), "--prompt".into(), "go".into()]).unwrap() else { panic!() };
        assert_eq!(options.agent, maya_core::model::Harness::Codex);
        assert_eq!(options.name.as_deref(), Some("Fix CI"));
        assert!(parse(&a("start --agent gpt")).unwrap_err().contains("agent"));
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p maya -p maya-cli` (package names: check `src-tauri/Cargo.toml` and `cli/Cargo.toml` `name =`)
Expected: compile errors.

- [ ] **Step 3: Implement**

`src-tauri/src/lib.rs`:

```rust
use maya_core::agents::{self, AgentInfo};

#[derive(serde::Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AgentsReply {
    agents: Vec<AgentInfo>,
    /// The machine runs a Maya that names sessions; an older one ignores names.
    names: bool,
}

fn agents_reply(local: bool, remote: Option<Vec<AgentInfo>>, local_list: impl FnOnce() -> Vec<AgentInfo>) -> AgentsReply {
    if local {
        return AgentsReply { agents: local_list(), names: true };
    }
    match remote {
        Some(agents) => AgentsReply { agents, names: true },
        None => AgentsReply { agents: vec![agents::claude()], names: false },
    }
}

/// The agents the chosen machine can start, with their models. Locally the
/// first call waits for the listing (seconds); a remote's comes with its board.
#[tauri::command(async)]
fn list_agents(state: TauriState<AppState>, machine: Option<String>) -> AgentsReply {
    let local = is_local(&machine);
    let remote = if local { None } else { merge::agents_of(&remote_boards(&state), machine.as_deref().unwrap_or_default()) };
    agents_reply(local, remote, agents::current)
}

/// An older Maya ignores the agent and starts Claude Code: refuse instead.
fn check_remote_agent(agent: Harness, remote: &Option<Vec<AgentInfo>>, machine: &str) -> Result<(), String> {
    match remote {
        None if agent != Harness::ClaudeCode => Err(format!("{machine} runs an older Maya that can only start Claude Code.")),
        Some(list) if !list.iter().any(|a| a.harness == agent) => Err(format!("That agent is not installed on {machine}.")),
        _ => Ok(()),
    }
}
```

In `start_session`'s remote branch, after `options.validate_shape()?;` and `let machine = machine.unwrap();`: `check_remote_agent(options.agent, &merge::agents_of(&remote_boards(&state), &machine), &machine)?;`. Register `list_agents` in the `invoke_handler` list next to `start_session`. In `refresh_and_emit`, right after the block that builds `cards` releases the store lock (after line ~465): `actions::run_due_renames(&local(&app.state::<AppState>()));`. In the `setup` closure, warm the listing: `let _ = maya_core::agents::snapshot();`. (`Harness` is `maya_core::model::Harness`; import it if the file does not.)

`cli/src/run_cmd.rs`: in the watcher closure (line ~200), after `s.lock().unwrap().refresh(now_ms());` add `actions::run_due_renames(&actions::Local { store: &s, terminal: &*term });`, with `let term = exec.terminal.clone();` captured next to `s` (the closure moves it). Import `maya_core::actions` if needed.

`cli/src/args.rs` `parse_start`:

```rust
            "--agent" => {
                let v = take_value(rest, &mut i, "--agent")?;
                options.agent = serde_json::from_value(serde_json::Value::String(v.clone())).map_err(|_| format!("{USAGE}\nunknown agent: {v} (claude-code, codex, antigravity or grok)"))?;
            }
            "--name" => options.name = Some(take_value(rest, &mut i, "--name")?),
```

Add `[--agent <claude-code|codex|antigravity|grok>] [--name <name>]` to the `start` line of `USAGE` (and to README's CLI section if it lists `start`'s flags: `grep -n "maya start" README.md docs/*.md`).

- [ ] **Step 4: Run the tests and build**

Run: `cargo test --workspace && cargo clippy --workspace -- -D warnings`
Expected: PASS, no warnings.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs cli/src README.md
git commit -m "feat: list agents per machine, rename due sessions on each tick, maya start --agent --name"
```

---

### Task 8: The modal: Agent, Name, and per-agent options

**Files:**
- Modify: `src/newsession.ts`
- Modify: `src/newsession.test.ts`, `src/newsession-flow.test.ts`
- Modify: `src/styles.css` only if the new fields need it (they reuse `newsession__field`)

**Interfaces:**
- Consumes: `list_agents` (`{ agents: AgentInfo[]; names: boolean }`), `start_session` with `options: { agent, name, model, effort, mode }`.
- Produces (exports): `ModelInfo`, `AgentInfo`, `CLAUDE_AGENT`, `optionFields(agent: AgentInfo, model: string)`, `StartChoices`.

- [ ] **Step 1: Write the failing tests**

`src/newsession.test.ts` (extend `base` with `agents: [CLAUDE_AGENT], agent: "claude-code", names: true, name: ""`, and `handlers()` with `onAgent: vi.fn(), onModel: vi.fn()`):

```ts
import { CLAUDE_AGENT, optionFields, renderNewSession, type AgentInfo } from "./newsession";

const CODEX: AgentInfo = {
  harness: "codex",
  models: [
    { id: "gpt-6.1-sol", label: "GPT-6.1-Sol", efforts: ["low", "medium", "high", "xhigh", "max", "ultra"] },
    { id: "gpt-5.5", label: "GPT-5.5", efforts: ["low", "medium", "high", "xhigh"] },
  ],
  efforts: ["low", "medium", "high", "xhigh"],
  modes: ["read-only", "workspace-write", "danger-full-access"],
};
const GROK: AgentInfo = { harness: "grok", models: [{ id: "grok-4.7", label: "grok-4.7", efforts: [] }], efforts: [], modes: ["default", "plan"] };

describe("agents", () => {
  it("hides the Agent field when Claude Code is the only agent", () => {
    expect(renderNewSession(base, handlers()).querySelector("select[name=agent]")).toBeNull();
  });

  it("lists the installed agents and reports a change", () => {
    const h = handlers();
    const el = renderNewSession({ ...base, agents: [CLAUDE_AGENT, CODEX, GROK] }, h);
    const sel = el.querySelector<HTMLSelectElement>("select[name=agent]")!;
    expect([...sel.options].map((o) => o.textContent)).toEqual(["Claude Code", "Codex", "Grok Build"]);
    sel.value = "codex";
    sel.dispatchEvent(new Event("change"));
    expect(h.onAgent).toHaveBeenCalledWith("codex");
  });

  it("shows only the fields the agent has", () => {
    const el = renderNewSession({ ...base, agents: [CLAUDE_AGENT, GROK], agent: "grok" }, handlers());
    expect(el.querySelector("select[name=effort]")).toBeNull();
    expect([...el.querySelector<HTMLSelectElement>("select[name=mode]")!.options].map((o) => o.value)).toEqual(["", "default", "plan"]);
  });

  it("offers the chosen Codex model's efforts, and the common ones for Default", () => {
    expect(optionFields(CODEX, "gpt-6.1-sol").find((f) => f.name === "effort")!.choices.map(([v]) => v)).toContain("ultra");
    expect(optionFields(CODEX, "").find((f) => f.name === "effort")!.choices.map(([v]) => v)).toEqual(["low", "medium", "high", "xhigh"]);
  });

  it("a_remembered_model_no_longer_listed_falls_back_to_default", () => {
    const el = renderNewSession({ ...base, agents: [CLAUDE_AGENT, CODEX], agent: "codex", options: { model: "gpt-4", effort: "", mode: "" } }, handlers());
    expect(el.querySelector<HTMLSelectElement>("select[name=model]")!.value).toBe("");
  });

  it("starts with the agent and the name", () => {
    const h = handlers();
    const el = renderNewSession({ ...base, agents: [CLAUDE_AGENT, CODEX], agent: "codex", dir: "a", prompt: "go" }, h);
    el.querySelector<HTMLInputElement>("input[name=name]")!.value = "  Fix CI ";
    el.querySelector<HTMLButtonElement>("button[data-action=start]")!.click();
    expect(h.onStart).toHaveBeenCalledWith("", "a", "go", { agent: "codex", name: "Fix CI", model: "", effort: "", mode: "" });
  });

  it("an_older_remote_offers_claude_only_and_no_name", () => {
    const el = renderNewSession({ ...base, machines: twoMachines, machine: "laptop", names: false }, handlers());
    expect(el.querySelector("input[name=name]")).toBeNull();
    expect(el.querySelector("select[name=agent]")).toBeNull();
  });
});
```

Update the existing `onStart` expectations in `newsession.test.ts` from `defaults` (`{ model, effort, mode }`) to `{ agent: "claude-code", name: "", ...defaults }`.

`src/newsession-flow.test.ts`: in each `invoke.mockImplementation`, add `if (cmd === "list_agents") return Promise.resolve({ agents: [CLAUDE_AGENT_FROM_RUST], names: true });` where the constant is the Claude entry as Rust sends it (`{ harness: "claude-code", models: [{ id: "fable", label: "Fable", efforts: [] }, …], efforts: [...], modes: [...] }`; import `CLAUDE_AGENT` from `./newsession` and use it). Change the `start_session` expectations to `options: { agent: "claude-code", name: "", model: "opus", effort: "", mode: "plan" }` (and the Default one likewise). Add:

```ts
  it("remembers the agent and each agent's options", async () => {
    const codex = { harness: "codex", models: [{ id: "gpt-5.5", label: "GPT-5.5", efforts: ["low"] }], efforts: ["low"], modes: ["read-only"] };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "list_machines") return Promise.resolve([]);
      if (cmd === "list_project_dirs") return Promise.resolve(["a"]);
      if (cmd === "list_agents") return Promise.resolve({ agents: [CLAUDE_AGENT, codex], names: true });
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await openNewSession();
    await flush();
    const agent = document.querySelector<HTMLSelectElement>("select[name=agent]")!;
    agent.value = "codex";
    agent.dispatchEvent(new Event("change"));
    document.querySelector<HTMLSelectElement>("select[name=model]")!.value = "gpt-5.5";
    closeNewSession();
    await openNewSession();
    await flush();
    expect(document.querySelector<HTMLSelectElement>("select[name=agent]")!.value).toBe("codex");
    expect(document.querySelector<HTMLSelectElement>("select[name=model]")!.value).toBe("gpt-5.5");
    // Back to Claude Code with Default options for the other tests.
    const back = document.querySelector<HTMLSelectElement>("select[name=agent]")!;
    back.value = "claude-code";
    back.dispatchEvent(new Event("change"));
    for (const sel of document.querySelectorAll<HTMLSelectElement>("select[name=model], select[name=effort], select[name=mode]")) sel.value = "";
  });

  it("a list_agents failure leaves Claude Code alone", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "list_machines") return Promise.resolve([]);
      if (cmd === "list_project_dirs") return Promise.resolve(["a"]);
      return Promise.reject(new Error("no"));
    });
    await openNewSession();
    await flush();
    expect(document.querySelector("select[name=agent]")).toBeNull();
    expect(document.querySelector("select[name=model]")).not.toBeNull();
  });
```

- [ ] **Step 2: Run them to see them fail**

Run: `pnpm test -- newsession`
Expected: FAIL (no `CLAUDE_AGENT`, `optionFields`, Agent/Name fields).

- [ ] **Step 3: Implement in `src/newsession.ts`**

Types and helpers (keep `MODEL_CHOICES`, `EFFORT_CHOICES`, `MODE_CHOICES`, `renderChoice`: `modal.ts` uses them):

```ts
import type { Harness } from "./types";
import { harnessLabel } from "./harness";

export interface ModelInfo { id: string; label: string; efforts: string[] }
/** One agent a machine can start, as `list_agents` reports it. */
export interface AgentInfo { harness: Harness; models: ModelInfo[]; efforts: string[]; modes: string[] }
/** What Start sends: the agent, the name ("" for none) and its options. */
export type StartChoices = SessionOptions & { agent: Harness; name: string };

/** Claude Code as Maya always knows it, before or without a listing. */
export const CLAUDE_AGENT: AgentInfo = {
  harness: "claude-code",
  models: MODEL_CHOICES.map(([id, label]) => ({ id, label, efforts: [] })),
  efforts: EFFORT_CHOICES.map(([v]) => v),
  modes: MODE_CHOICES.map(([v]) => v),
};

type OptionField = { name: keyof SessionOptions; label: string; choices: [string, string][] };

/** The option fields `agent` has, for the chosen `model`; fields without choices are left out. */
export function optionFields(agent: AgentInfo, model: string): OptionField[] {
  const chosen = agent.models.find((m) => m.id === model);
  const efforts = chosen && chosen.efforts.length > 0 ? chosen.efforts : agent.efforts;
  const fields: OptionField[] = [
    { name: "model", label: "Model", choices: agent.models.map((m) => [m.id, m.label]) },
    { name: "effort", label: "Effort", choices: efforts.map((v) => [v, v]) },
    { name: "mode", label: "Mode", choices: agent.modes.map((v) => [v, v]) },
  ];
  return fields.filter((f) => f.choices.length > 0);
}
```

Remove the now-unused `OPTION_FIELDS` export if nothing else imports it (`grep -rn OPTION_FIELDS src`).

Model and handlers:

```ts
export interface NewSessionModel {
  // …existing fields…
  /** The chosen machine's agents, Claude Code first. */
  agents: AgentInfo[];
  agent: Harness;
  /** The machine's Maya takes names (an older one does not). */
  names: boolean;
  name: string;
}

export interface NewSessionHandlers {
  onStart(machine: string, dir: string | null, prompt: string, choices: StartChoices): void;
  onClose(): void;
  onOpenSettings(): void;
  onMachine(machine: string): void;
  onAgent(agent: Harness): void;
  /** The model changed: Codex's efforts depend on it. */
  onModel(model: string): void;
}
```

In `renderNewSession`, after the directory field is built and before the option row:

```ts
  const agent = m.agents.find((a) => a.harness === m.agent) ?? m.agents[0] ?? CLAUDE_AGENT;
  let agentLabel: HTMLLabelElement | null = null;
  if (m.agents.length > 1) {
    agentLabel = el("label", "newsession__field");
    agentLabel.append(el("span", "newsession__label", "Agent"));
    const agentSelect = el("select", "newsession__select");
    agentSelect.name = "agent";
    for (const a of m.agents) {
      const o = document.createElement("option");
      o.value = a.harness;
      o.textContent = harnessLabel(a.harness);
      agentSelect.append(o);
    }
    agentSelect.value = agent.harness;
    agentSelect.addEventListener("change", () => h.onAgent(agentSelect.value as Harness));
    agentLabel.append(agentSelect);
  }
```

Replace the option-row loop:

```ts
  const optionRow = el("div", "newsession__options");
  const optionSelects: [keyof SessionOptions, HTMLSelectElement][] = [];
  for (const f of optionFields(agent, m.options.model ?? "")) {
    const field = renderChoice(f.name, f.label, f.choices, m.options[f.name] ?? "");
    const sel = field.querySelector("select")!;
    if (f.name === "model") sel.addEventListener("change", () => h.onModel(sel.value));
    optionSelects.push([f.name, sel]);
    optionRow.append(field);
  }
```

The Name field, before the prompt:

```ts
  let nameInput: HTMLInputElement | null = null;
  let nameLabel: HTMLLabelElement | null = null;
  if (m.names) {
    nameLabel = el("label", "newsession__field");
    nameLabel.append(el("span", "newsession__label", "Name"));
    nameInput = el("input", "newsession__select");
    nameInput.name = "name";
    nameInput.type = "text";
    nameInput.maxLength = 60;
    nameInput.placeholder = "Optional; the agent names it otherwise";
    nameInput.value = m.name;
    nameLabel.append(nameInput);
  }
```

`readOptions` and `tryStart`:

```ts
  const readChoices = (): StartChoices => {
    const o: StartChoices = { agent: agent.harness, name: nameInput?.value.trim() ?? "", model: "", effort: "", mode: "" };
    for (const [name, sel] of optionSelects) o[name] = sel.value;
    return o;
  };
  // in tryStart:
    h.onStart(m.machine, select.value || null, prompt, readChoices());
```

Append order: `form.append(...[agentLabel, dirLabel, optionRow, nameLabel, promptLabel, actions].filter((n): n is HTMLElement => n !== null));`.

Module state and flow:

```ts
let draftName = "";
let lastAgent: Harness = "claude-code";
/** The last options per agent; unlike the prompt they are kept after a start. */
const lastOptions: Partial<Record<Harness, SessionOptions>> = {};
const EMPTY: SessionOptions = { model: "", effort: "", mode: "" };
const optionsFor = (a: Harness): SessionOptions => ({ ...EMPTY, ...lastOptions[a] });
```

`readOptionsFrom(host)` reads `select[name=model|effort|mode]` when present (a missing field reads `""`) and is stored with `lastOptions[current.model.agent] = …`. In `paint()`, before re-rendering: keep `m.name` from `input[name=name]` (unless `m.done`), and `m.options = lastOptions[m.agent] = readOptionsFrom(host)`. Handlers passed in `paint()`:

```ts
      onAgent: (agent) => {
        if (!current) return;
        lastOptions[current.model.agent] = readOptionsFrom(host);
        current.model.agent = lastAgent = agent;
        current.model.options = optionsFor(agent);
        repaint(); // render without reading the selects back: they belong to the old agent
      },
      onModel: () => paint(),
```

(Split `paint()` into `capture()` (reads the DOM into the model) and `repaint()` (renders); `paint()` = `capture(); repaint();`. `onAgent` captures first, then switches, then `repaint()`.)

Loading agents, called from `openNewSession` and `chooseMachine` next to `loadDirs`:

```ts
async function loadAgents(machine: string): Promise<void> {
  const me = current;
  if (!me) return;
  let reply: { agents: AgentInfo[]; names: boolean };
  try {
    reply = await invoke<{ agents: AgentInfo[]; names: boolean }>("list_agents", { machine });
  } catch {
    reply = { agents: [CLAUDE_AGENT], names: machine === "" };
  }
  if (current !== me || me.model.machine !== machine) return;
  capture();
  me.model.agents = reply.agents.length > 0 ? reply.agents : [CLAUDE_AGENT];
  me.model.names = reply.names;
  const wanted = me.model.agents.some((a) => a.harness === lastAgent) ? lastAgent : "claude-code";
  me.model.agent = wanted;
  me.model.options = optionsFor(wanted);
  repaint();
}
```

`openNewSession` initial model: `agents: [CLAUDE_AGENT], agent: "claude-code", names: true, name: draftName, options: optionsFor("claude-code")`, then `void loadAgents("")` alongside `await loadDirs("")`. `chooseMachine` resets `agents: [CLAUDE_AGENT]`, `agent: "claude-code"`, `names: machine === ""`, then `void loadAgents(machine)`. `start()` sends `invoke("start_session", { dir, prompt, options: choices, machine })` with `choices: StartChoices`; on success clear `draftName` with `draft`. `closeNewSession` keeps `draftName` from the input like `draft`.

- [ ] **Step 4: Run the tests and the type check**

Run: `pnpm test && pnpm exec tsc --noEmit`
Expected: PASS, no type errors.

- [ ] **Step 5: Commit**

```bash
git add src/newsession.ts src/newsession.test.ts src/newsession-flow.test.ts src/styles.css
git commit -m "feat: pick the agent and name a new session in the New session modal"
```

---

### Task 9: Check it in the real app, bump, PR

**Files:**
- Modify: `src-tauri/tauri.conf.json`, `package.json`, `Cargo.toml`, `Cargo.lock` (via the script)

- [ ] **Step 1: Full suite**

Run: `cargo test --workspace && pnpm test && cargo clippy --workspace -- -D warnings`
Expected: all PASS.

- [ ] **Step 2: Real app** (memory `maya-run-and-verify`: vite + debug binary, screenshot then click). **Ask the user first**: this starts real sessions on their Claude, Codex, Antigravity and Grok accounts. With a yes, in the `maya` folder:
  1. Open New session; check the Agent list shows the four agents and each one's options (Codex's efforts change with the model; Grok has no Effort).
  2. Start each agent with prompt `-- say hi in one word` (it starts with `-`, Review Focus 1) and name `Maya probe <agent>`. Each must start, take the whole prompt as its prompt, and its card must show the name at once.
  3. Once each is Completed, check the agent's own name: `tail -1 ~/.codex/session_index.jsonl`, the Grok `summary.json` (`generated_title`, `title_is_manual`), `grep 'Maya probe' ~/.gemini/antigravity-cli/history.jsonl`, and the Claude card.
  4. Rename one of each from the session modal; check the card follows within one refresh.
  5. Close the probe sessions.

  If Antigravity rejects `--prompt-interactive="$p"`, stop and report: the fallback is `-i "$p"` with a leading-`-` prompt refused in `validate_shape`.

- [ ] **Step 3: Version bump**

```bash
git fetch origin && git show origin/main:package.json | grep '"version"'
sh scripts/set-version.sh 0.8.0   # one minor above main's version
git add src-tauri/tauri.conf.json package.json Cargo.toml Cargo.lock
git commit -m "chore(release): 0.8.0"
```

- [ ] **Step 4: Mark the spec implemented**

Set `Status: implemented` in `docs/superpowers/specs/2026-10-02-maya-choose-agent-design.md`; commit `docs: the choose-agent design is implemented`. (Note in the spec: models arrive with the agent list, so the Agent field appears once the listing is in, instead of a "Loading…" Model field.)

- [ ] **Step 5: Ask the user, then push and open the PR** with the conventional-pull-request skill. Never push before the user says yes.

---

### Task 10: What the live check found

Added after Task 9's live check on 2026-10-02 (report: the probe sessions for Claude Code and Grok Build worked end to end; Codex and Antigravity did not get their name; the session modal could not rename them).

**Files:**
- Modify: `src/modal.ts:418` (the rename gate), `src/modal.test.ts`
- Modify: `src-tauri/src/terminal_app.rs` (how a line reaches a Codex TUI), `core/src/actions.rs` (`run_due_renames` logging), and whatever the Antigravity investigation names
- Test: the files above

**Interfaces:** none new. `Terminal::type_line(tty, text)` keeps its signature.

- [ ] **Step 1: The session modal offers rename for every agent.** `renderTitle(m.card.name, h, claude)` enables the rename click only for Claude Code. Enable it for every harness; the backend refuses a non-free session with "Wait until the session is free to rename it.", which the modal already shows as an error. Write the failing test in `src/modal.test.ts` first (a Codex card's title is clickable and `onRename` is called), then make it pass. Commit: `fix: offer rename for every agent's session`.

- [ ] **Step 2: Codex — find why the typed line is not submitted.** Use superpowers:systematic-debugging. Facts from the live check: Maya typed `/rename Maya probe Codex` into the Codex TUI (tty `ttys012`, still open) through `terminal_app.rs`'s AppleScript `do script "<text>" in t`; the text appeared in Codex's input and the trailing newline became a new line inside the input instead of submitting. Codex 0.160.0 (updated 2026-10-02) enables the kitty keyboard protocol on start (`CSI > 7 u` was seen in a pty probe), under which a terminal reports Enter as `ESC [ 13 u`, so a bare `\n`/`\r` from `do script` may read as a newline, not Enter. Form the hypothesis, test it on the open Codex session (e.g. `do script` the text without a trailing newline is impossible; try sending the Enter as a separate key: `osascript -e 'tell application "System Events" to key code 36'` with Terminal frontmost, or writing `\x1b[13u` to the tty), and pick the smallest fix that keeps Claude Code, Antigravity and Grok working (Claude Code replies and renames must keep submitting). Pin it with a unit test on the AppleScript/line builder if the fix changes it. Commit: `fix: submit the typed line to a Codex session`.

- [ ] **Step 3: Antigravity — find why the rename never fired.** Facts: the agy probe (tty `ttys013`, still open) started with the prompt and its card showed the pending name, but no `/rename` reached it: nothing in `~/.gemini/antigravity-cli/history.jsonl`, nothing in `~/.claude/maya/maya.log`. Candidates, in order: (1) the pending name never became due — `pending_names::apply` needs the card free AND (seen Working or a non-empty snippet); check what `foreign::derive` gives an agy card after one short turn (`core/src/antigravity.rs::parse_tail`: `last_agent_text`, `working`); (2) `run_due_renames` dropped it silently — `session_tty` or `rename_command` returned Err inside the `filter_map` (Task 5's deferred finding); (3) the card's `cwd` did not match the launch folder. Add the log line on every drop in `run_due_renames` first (it is cheap and was already deferred), then reproduce with the debug app and a fresh agy probe in the `maya` folder (the user approved live probes), read the log, and fix the real cause with a test. Commit: `fix: rename an Antigravity session once its first turn ends` (or the title the cause deserves).

- [ ] **Step 4: Verify in the real app** — one fresh Codex probe and one fresh Antigravity probe with a name; both must show the name in the agent's own files; rename each from the session modal. Then `cargo test --workspace && pnpm test`.
