# Choose the agent that powers Maya — implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The user picks the agent that powers Maya (Claude Code, Codex, Antigravity or Grok Build) once on first start and in Settings; it interprets voice commands, picks folders, and is the default for new, resumed and review sessions. Resume works for every agent, session controls follow a per-agent capability table, and the Review button runs a configurable prompt.

**Architecture:** One config field `agent` (plus `agentModel` and `reviewPrompt`) read by the Rust core. `core/src/launch.rs` gains the per-agent one-shot command, its output parser and the capability table; `interpreter.rs`, the classifier, `resume.rs`, `reviews.rs` and `actions.rs` take the agent instead of assuming `claude`. The frontend mirrors the capability table, adds a Maya section to Settings, a first-start modal, and an Agent field to Resume.

**Tech Stack:** Rust (Cargo workspace: `core/`, `src-tauri/`, `cli/`), Tauri 2, vanilla TypeScript with vitest, serde_json.

**Spec:** `docs/superpowers/specs/2026-10-03-maya-agent-design.md`

## Global Constraints

- Branch `feat/maya-agent`; never commit to or push `main`; ask before each push (user rule).
- Commit messages are conventional (`feat:`, `fix:`, `docs:`, `chore:`), ending with the attribution lines from the session reminder.
- Harness ids are kebab-case in JSON: `claude-code`, `codex`, `antigravity`, `grok` (`core/src/model.rs:55-62`, `src/types.ts:20`).
- Shell lines quote every user-derived value with `launch::shell_single_quote`; model ids pass `launch::plain_model_id`; session ids pass `resume::plain_session_id` (`[A-Za-z0-9_-]`, at most 128 bytes).
- A one-shot run has no tools, nothing persisted, one turn, 25 s timeout, Maya's data folder as its working directory, cleared environment plus `launch::clean_env`.
- Copy: "Let Maya choose", "(chosen by Maya)", "I can't find the <binary> command.", "<Agent> has no <control>."
- Maya's data folder stays `~/.claude/maya`.
- Tests: `cargo test --workspace` from the repo root and `pnpm test`; the frontend must also pass `pnpm build` (runs `tsc`).
- Release: this is a `feat`, so the last task bumps the minor version from `main`'s (0.9.0 → 0.10.0 unless `main` moved) with `sh scripts/set-version.sh`, committed alone as `chore(release): 0.10.0`.

## Review Focus

1. A config with `agent` set to an agent that is no longer installed: the interpreter and classifier must fail with "I can't find the <binary> command." / "Could not find the <binary> command.", not panic or silently run Claude. Pinned in Task 3 (`find_binary`) and Task 4.
2. A one-shot reply that is not JSON, or JSON with no `say`: Maya must say "Sorry, I didn't catch that." and not act. Pinned in Task 2 (`final_text` errors) and Task 3 (`parse_reply` on garbage).
3. A resume id typed by hand or relayed over the network that is not in the folder's listing, or contains `/`, `..` or a quote: nothing opens. Pinned in Task 5.
4. A review prompt template that mentions a placeholder but is otherwise a slash command, and one that mentions none: the rendered prompt must name the PR exactly once. Pinned in Task 6.
5. An upgrade with `interpreterModel: "sonnet"` and no `agent`, followed by picking Codex on first start: the Claude alias must not become Codex's model. Pinned in Task 1 (`migrate`) and Task 9 (the save clears `agentModel` for a non-Claude agent).

---

### Task 1: Config fields `agent`, `agentModel`, `reviewPrompt`, with migration

**Files:**
- Modify: `core/src/config.rs:50-150, 195-205, 330-345`
- Modify: `docs/superpowers/specs/2026-10-03-maya-agent-design.md` (three findings amended, see Step 6)

**Interfaces:**
- Produces: `Config.agent: Option<Harness>`, `Config.agent_model: String`, `Config.review_prompt: String`, `Config.interpreter_model: Option<String>` (legacy, read only), `Config::brain(&self) -> Harness`, `Config::brain_model(&self) -> Option<&str>`, `Config::migrate(self) -> Config`.

- [ ] **Step 1: Write the failing tests** in the `tests` module of `core/src/config.rs`

```rust
    #[test]
    fn agent_defaults_to_none_and_the_brain_to_claude_code() {
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes":30}"#).unwrap();
        assert_eq!(c.agent, None);
        assert_eq!(c.brain(), crate::model::Harness::ClaudeCode);
        assert_eq!(c.brain_model(), None);
        assert_eq!(c.review_prompt, "");
        let text = serde_json::to_string(&c).unwrap();
        assert!(!text.contains("\"agent\""), "an unset agent is not written: {text}");
        assert!(!text.contains("interpreterModel"), "{text}");
    }

    #[test]
    fn an_old_interpreter_model_becomes_the_claude_agent_model_only() {
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes":30,"interpreterModel":"sonnet"}"#).unwrap();
        let c = c.migrate();
        assert_eq!(c.agent_model, "sonnet");
        assert_eq!(c.interpreter_model, None);
        assert_eq!(c.brain_model(), Some("sonnet"));
        let codex: Config = serde_json::from_str(r#"{"completedTimeoutMinutes":30,"agent":"codex","interpreterModel":"sonnet"}"#).unwrap();
        assert_eq!(codex.migrate().agent_model, "", "a Claude alias is not carried to another agent");
        let kept: Config = serde_json::from_str(r#"{"completedTimeoutMinutes":30,"agentModel":"haiku","interpreterModel":"sonnet"}"#).unwrap();
        assert_eq!(kept.migrate().agent_model, "haiku", "a model already chosen wins");
    }

    #[test]
    fn agent_model_and_review_prompt_round_trip() {
        let c = Config { agent: Some(crate::model::Harness::Grok), agent_model: "grok-4.7".into(), review_prompt: "/should-i-approve".into(), ..Default::default() };
        let text = serde_json::to_string(&c).unwrap();
        assert!(text.contains("\"agent\":\"grok\""), "{text}");
        assert!(text.contains("\"agentModel\":\"grok-4.7\""), "{text}");
        assert!(text.contains("\"reviewPrompt\":\"/should-i-approve\""), "{text}");
        let back: Config = serde_json::from_str(&text).unwrap();
        assert_eq!(back.brain(), crate::model::Harness::Grok);
        assert_eq!(back.brain_model(), Some("grok-4.7"));
        assert_eq!(Config { agent_model: "   ".into(), ..Default::default() }.brain_model(), None, "blank means the agent's default");
    }
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core config::tests`
Expected: compile errors, "no field `agent`".

- [ ] **Step 3: Add the fields, the helpers and the migration**

In `core/src/config.rs`, add `use crate::model::Harness;` at the top. Replace the `interpreter_model` field (`:83-85`) with:

```rust
    /// The agent that powers Maya: it interprets voice commands, picks
    /// folders for "Let Maya choose", and is the default for new, resumed
    /// and review sessions. None until the first-start modal or Settings
    /// sets it; `brain()` then reads Claude Code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<Harness>,
    /// That agent's model id; "" means the agent's own default.
    #[serde(default)]
    pub agent_model: String,
    /// The prompt a review session starts with; "" means the built-in one
    /// (`reviews::DEFAULT_REVIEW_PROMPT`).
    #[serde(default)]
    pub review_prompt: String,
    /// The voice interpreter's model before 0.10, kept only to be read once
    /// by `migrate`; never written again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interpreter_model: Option<String>,
```

Delete `default_interpreter_model` (`:123-125`). In `Default::default()` replace `interpreter_model: "haiku".into(),` with:

```rust
            agent: None,
            agent_model: String::new(),
            review_prompt: String::new(),
            interpreter_model: None,
```

In `impl Config` (next to `completed_timeout_ms`) add:

```rust
    /// The agent that powers Maya; Claude Code until one is chosen.
    pub fn brain(&self) -> Harness {
        self.agent.unwrap_or(Harness::ClaudeCode)
    }

    /// Maya's agent's model, or None for the agent's own default.
    pub fn brain_model(&self) -> Option<&str> {
        Some(self.agent_model.trim()).filter(|m| !m.is_empty())
    }

    /// Carries a pre-0.10 `interpreterModel` into `agentModel`, only when
    /// Maya's agent is (still) Claude Code, whose aliases it named.
    pub fn migrate(mut self) -> Config {
        if let Some(m) = self.interpreter_model.take() {
            if self.agent_model.trim().is_empty() && self.brain() == Harness::ClaudeCode {
                self.agent_model = m;
            }
        }
        self
    }
```

Change `load` (`:197`) to `...unwrap_or_default().migrate().for_this_platform()`.

Update the existing test `voice_settings_default_off_with_haiku` (`:334-342`): rename it `voice_settings_default_off`, delete its two `interpreter_model` assertions (`assert_eq!(c.interpreter_model, "haiku")` and the `"sonnet"` one), keep the rest.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p maya-core config::`
Expected: PASS. Then `cargo build --workspace` to find other users of `interpreter_model`: `src-tauri/src/listener.rs:310` still reads it; leave that compile error for Task 3, or if you need a green build now, replace `store.config.interpreter_model.clone()` there with `store.config.agent_model.clone()` as a stopgap (Task 3 rewrites that block anyway).

- [ ] **Step 5: Frontend config type and test fixtures**

In `src/settings.ts` `ConfigJson` (`:158-172`) replace `interpreterModel: string;` with:

```ts
  agent?: Harness | null;
  agentModel?: string;
  reviewPrompt?: string;
```

and import `type Harness` from `./types`. Leave `SettingsModel.interpreterModel`, `onInterpreter` and the "Voice interpreter" select in place for now (Task 8 replaces them); so `pnpm build` passes, change the init line `model.interpreterModel = config.interpreterModel;` to `model.interpreterModel = config.agentModel ?? "";` and `onInterpreter` to `saveConfig({ agentModel: m })`. In `src/settings-flow.test.ts:29` replace `interpreterModel: "haiku",` with `agentModel: "haiku",`.

Run: `pnpm test && pnpm build`
Expected: PASS.

- [ ] **Step 6: Amend the spec's findings to what the probes showed**

In `docs/superpowers/specs/2026-10-03-maya-agent-design.md`:
- In the Findings one-shot table, Codex's Output cell becomes: `--json` (the last `agent_message` item on stdout); Antigravity's One-shot cell becomes `agy -p=<prompt>` (the prompt attached to the flag; other flags before it).
- In Behaviour › Thinking, the sentence starting "The reply is the agent's final text" becomes: "The reply is the agent's final text: Claude's `result`, the text of Codex's last `agent_message` item, Antigravity's `response`, Grok's `text`. The first JSON object in it is the reply, as today."
- In Behaviour › Other machines, replace the review bullet with: "Reviews start on the main machine, as today; Maya's agent and the review prompt are its settings."
- In the Code section, the `src/modal.ts` line becomes: "`src/modal.ts`: controls gated on the agent's capabilities, mirrored in `src/harness.ts` from the Rust table."

- [ ] **Step 7: Commit**

```bash
git add core/src/config.rs src/settings.ts src/settings-flow.test.ts docs/superpowers/specs/2026-10-03-maya-agent-design.md
git commit -m "feat(config): Maya's agent, its model and the review prompt"
```

---

### Task 2: Capability table, one-shot arguments and final text (`launch.rs`)

**Files:**
- Modify: `core/src/launch.rs` (after `modes`, `:46`; and the tests module)
- Create: `core/fixtures/oneshot/claude.json`, `codex.jsonl`, `agy.json`, `grok.json`

**Interfaces:**
- Consumes: `Harness`, `answer::SHIFT_TAB`.
- Produces: `launch::label(Harness) -> &'static str`; `launch::Capabilities { compact, model_switch, effort_switch, mode_cycle: Option<&'static str>, slash_lines, shell_lines }`; `launch::capabilities(Harness) -> Capabilities`; `launch::oneshot_args(agent, model: Option<&str>, system: Option<&str>, user: &str) -> Vec<String>`; `launch::final_text(agent, stdout: &str) -> Result<String, String>`; `launch::find_binary(agent) -> Result<PathBuf, String>`.

- [ ] **Step 1: Probe the unconfirmed capability cells (needs the user's go-ahead: it opens Terminal windows and starts each agent)**

The cells to settle: Codex mode cycle and `!` lines; Antigravity `/model <id>` and `!` lines; Grok `!` lines. For each agent, from a scratch folder:

```bash
osascript -e 'tell application "Terminal" to do script "cd /tmp && mkdir -p maya-probe && cd maya-probe && codex"'
sleep 10
osascript -e 'tell application "System Events" to keystroke "!echo probe-ok"' -e 'tell application "System Events" to key code 36'
sleep 4
screencapture -l "$(osascript -e 'tell application "Terminal" to id of front window')" "$SCRATCHPAD/probe-codex-shell.png"
```

Read the PNG with the Read tool. `probe-ok` printed as a command result means `shell_lines: true`; the text treated as a chat message means `false`. For the mode cycle press Shift+Tab (`key code 48 using shift down`), screenshot, and look for a mode label change in the TUI's footer. For Antigravity's `/model`, type `/model <an id from agy models>` and Enter; a model label change means `model_switch: true`. Quit each TUI with Ctrl+C twice or `/quit`. Record each answer in the table in Step 4 and in the spec's capability table (replace each "to probe" cell with what was seen). If the user is not available for the probe, keep the safe defaults below (`false` / `None`) and say so in the PR description.

- [ ] **Step 2: Save the fixtures**

`core/fixtures/oneshot/claude.json`:

```json
{"type":"result","subtype":"success","is_error":false,"result":"```json\n{\"say\":\"Telling hexgrid: go ahead.\",\"action\":{\"kind\":\"reply\",\"session\":\"hexgrid\",\"text\":\"go ahead\"},\"confirm\":true}\n```"}
```

`core/fixtures/oneshot/codex.jsonl` (from `codex exec --json`, 2026-10-03):

```
{"type":"thread.started","thread_id":"01a10098-aaf3-7ca3-810a-127e0dd43c8b"}
{"type":"turn.started"}
{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"{\"say\":\"hi\",\"action\":null,\"confirm\":false}"}}
{"type":"turn.completed","usage":{"input_tokens":14279,"cached_input_tokens":12288,"cache_write_input_tokens":0,"output_tokens":17,"reasoning_output_tokens":0}}
```

`core/fixtures/oneshot/agy.json` (from `agy --output-format json -p=…`):

```json
{"conversation_id":"8899f83b-c40f-43fb-8562-eec9caa4566b","status":"SUCCESS","response":"{\"say\":\"hi\",\"action\":null,\"confirm\":false}\n","duration_seconds":3.123258,"num_turns":1,"usage":{"input_tokens":12622,"output_tokens":172,"thinking_tokens":159,"cache_read_tokens":0,"total_tokens":12794}}
```

`core/fixtures/oneshot/grok.json` (from `grok -p … --output-format json`):

```json
{
  "text": "{\"say\":\"hi\",\"action\":null,\"confirm\":false}",
  "stopReason": "end_turn",
  "sessionId": "01a10098-cf9b-7591-9b65-2cd2bc83be08",
  "num_turns": 1
}
```

- [ ] **Step 3: Write the failing tests** in `core/src/launch.rs`'s tests module

```rust
    fn oneshot_fixture(name: &str) -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/oneshot").join(name)).unwrap()
    }

    #[test]
    fn labels_name_every_agent() {
        assert_eq!(label(Harness::ClaudeCode), "Claude Code");
        assert_eq!(label(Harness::Codex), "Codex");
        assert_eq!(label(Harness::Antigravity), "Antigravity");
        assert_eq!(label(Harness::Grok), "Grok Build");
    }

    #[test]
    fn capabilities_follow_the_table() {
        let c = capabilities(Harness::ClaudeCode);
        assert!(c.compact && c.model_switch && c.effort_switch && c.slash_lines && c.shell_lines);
        assert_eq!(c.mode_cycle, Some(crate::answer::SHIFT_TAB));
        let x = capabilities(Harness::Codex);
        assert!(x.compact && x.slash_lines);
        assert!(!x.model_switch && !x.effort_switch, "Codex's /model opens a picker");
        let g = capabilities(Harness::Grok);
        assert!(g.compact && g.model_switch && g.slash_lines);
        assert_eq!(g.mode_cycle, Some(crate::answer::SHIFT_TAB));
        assert_eq!(capabilities(Harness::Antigravity).mode_cycle, Some(crate::answer::SHIFT_TAB));
    }

    #[test]
    fn oneshot_args_per_agent() {
        let pair = |a: &[String], f: &str, v: &str| a.windows(2).any(|w| w[0] == f && w[1] == v);
        let a = oneshot_args(Harness::ClaudeCode, Some("haiku"), Some("SYS"), "USER");
        assert_eq!(a[0], "-p");
        assert!(pair(&a, "--model", "haiku") && pair(&a, "--system-prompt", "SYS") && pair(&a, "--output-format", "json") && pair(&a, "--tools", "") && pair(&a, "--max-turns", "1"));
        assert_eq!(a.last().unwrap(), "USER");
        assert!(!oneshot_args(Harness::ClaudeCode, None, None, "U").iter().any(|x| x == "--model" || x == "--system-prompt"));

        let c = oneshot_args(Harness::Codex, Some("gpt-5.5"), Some("SYS"), "USER");
        assert_eq!(&c[..2], ["exec", "--json"]);
        assert!(c.contains(&"--ephemeral".to_string()) && c.contains(&"--skip-git-repo-check".to_string()) && pair(&c, "-s", "read-only") && pair(&c, "-m", "gpt-5.5"));
        assert_eq!(c[c.len() - 2], "--");
        assert_eq!(c.last().unwrap(), "SYS\n\nUSER", "no system flag: the system text leads the prompt");

        let g = oneshot_args(Harness::Antigravity, None, Some("SYS"), "USER");
        assert!(pair(&g, "--output-format", "json") && g.contains(&"--sandbox".to_string()) && g.contains(&"--disable-slash-commands".to_string()));
        assert_eq!(g.last().unwrap(), "-p=SYS\n\nUSER", "agy takes the prompt attached to -p");
        assert!(!g.iter().any(|x| x == "--model"));

        let k = oneshot_args(Harness::Grok, Some("grok-4.7"), Some("SYS"), "USER");
        assert_eq!(&k[..2], ["-p", "USER"]);
        assert!(pair(&k, "--output-format", "json") && pair(&k, "--tools", "") && pair(&k, "--max-turns", "1") && pair(&k, "--permission-mode", "plan") && pair(&k, "-m", "grok-4.7") && pair(&k, "--system-prompt-override", "SYS"));
    }

    #[test]
    fn final_text_reads_each_agents_envelope() {
        assert!(final_text(Harness::ClaudeCode, &oneshot_fixture("claude.json")).unwrap().contains("\"say\":\"Telling hexgrid: go ahead.\""));
        assert_eq!(final_text(Harness::Codex, &oneshot_fixture("codex.jsonl")).unwrap(), r#"{"say":"hi","action":null,"confirm":false}"#);
        assert_eq!(final_text(Harness::Antigravity, &oneshot_fixture("agy.json")).unwrap().trim(), r#"{"say":"hi","action":null,"confirm":false}"#);
        assert_eq!(final_text(Harness::Grok, &oneshot_fixture("grok.json")).unwrap(), r#"{"say":"hi","action":null,"confirm":false}"#);
    }

    #[test]
    fn final_text_reports_failures_and_garbage() {
        assert!(final_text(Harness::ClaudeCode, r#"{"type":"result","is_error":true,"result":"Not logged in"}"#).unwrap_err().contains("Not logged in"));
        assert!(final_text(Harness::ClaudeCode, "not json").unwrap_err().contains("not JSON"));
        assert!(final_text(Harness::Codex, "{\"type\":\"turn.started\"}\n").unwrap_err().contains("no agent message"));
        assert!(final_text(Harness::Codex, "{\"type\":\"error\",\"message\":\"quota\"}\n").unwrap_err().contains("quota"));
        assert!(final_text(Harness::Antigravity, r#"{"status":"ERROR","response":""}"#).unwrap_err().contains("agy failed"));
        assert!(final_text(Harness::Grok, r#"{"stopReason":"error"}"#).unwrap_err().contains("no text"));
    }

    #[test]
    fn find_binary_names_the_missing_command() {
        // A binary no machine has: the message names the command the user must install.
        let err = find_binary_named("maya-no-such-agent-xyz").unwrap_err();
        assert_eq!(err, "Could not find the maya-no-such-agent-xyz command.");
    }
```

- [ ] **Step 4: Run the tests to see them fail**

Run: `cargo test -p maya-core launch::tests`
Expected: compile errors ("cannot find function `label`" and so on).

- [ ] **Step 5: Implement**

In `core/src/launch.rs`, after `pub fn modes` (`:46`), add:

```rust
/// The agent's name as the UI shows it.
pub fn label(agent: Harness) -> &'static str {
    match agent {
        Harness::ClaudeCode => "Claude Code",
        Harness::Codex => "Codex",
        Harness::Antigravity => "Antigravity",
        Harness::Grok => "Grok Build",
    }
}

/// What Maya can type into a running session of this agent: from each
/// agent's docs and a probe of its TUI (the design's capability table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// `/compact` is a command.
    pub compact: bool,
    /// `/model <id>` switches the model; false where `/model` only opens a picker.
    pub model_switch: bool,
    /// `/effort <level>` switches the effort.
    pub effort_switch: bool,
    /// The keys that cycle the permission or execution mode; None where no key does.
    pub mode_cycle: Option<&'static str>,
    /// `/` lines are commands to the agent.
    pub slash_lines: bool,
    /// `!` lines run in the shell.
    pub shell_lines: bool,
}

pub fn capabilities(agent: Harness) -> Capabilities {
    let shift_tab = Some(crate::answer::SHIFT_TAB);
    match agent {
        Harness::ClaudeCode => Capabilities { compact: true, model_switch: true, effort_switch: true, mode_cycle: shift_tab, slash_lines: true, shell_lines: true },
        // `/model` and `/permissions` open pickers in Codex; a typed value does nothing.
        Harness::Codex => Capabilities { compact: true, model_switch: false, effort_switch: false, mode_cycle: None, slash_lines: true, shell_lines: false },
        Harness::Antigravity => Capabilities { compact: true, model_switch: false, effort_switch: false, mode_cycle: shift_tab, slash_lines: true, shell_lines: false },
        // Grok's `--help` lists no effort values, so none can be checked before typing.
        Harness::Grok => Capabilities { compact: true, model_switch: true, effort_switch: false, mode_cycle: shift_tab, slash_lines: true, shell_lines: false },
    }
}
```

Correct `Codex`/`Antigravity`/`Grok` cells to what Step 1 showed. Then, after `clean_env` (`:172`), add:

```rust
/// The arguments for one non-interactive turn of `agent`: `user`, answered
/// under `system`, with no tools, nothing persisted, one turn, and the
/// agent's JSON envelope on stdout (see `final_text`). Codex and
/// Antigravity have no system-prompt flag: the system text leads the
/// prompt for them. Antigravity reads the prompt only attached to `-p`.
pub fn oneshot_args(agent: Harness, model: Option<&str>, system: Option<&str>, user: &str) -> Vec<String> {
    let s = |v: &str| v.to_string();
    let folded = match system {
        Some(sys) => format!("{sys}\n\n{user}"),
        None => user.to_string(),
    };
    match agent {
        Harness::ClaudeCode => {
            let mut a = vec![s("-p"), s("--output-format"), s("json"), s("--strict-mcp-config"), s("--disable-slash-commands"), s("--no-session-persistence"), s("--max-turns"), s("1"), s("--tools"), s(""), s("--setting-sources"), s("user")];
            if let Some(sys) = system {
                a.extend([s("--system-prompt"), s(sys)]);
            }
            if let Some(m) = model {
                a.extend([s("--model"), s(m)]);
            }
            a.push(s(user));
            a
        }
        Harness::Codex => {
            let mut a = vec![s("exec"), s("--json"), s("--ephemeral"), s("--skip-git-repo-check"), s("-s"), s("read-only"), s("--color"), s("never")];
            if let Some(m) = model {
                a.extend([s("-m"), s(m)]);
            }
            a.extend([s("--"), folded]);
            a
        }
        Harness::Antigravity => {
            let mut a = vec![s("--output-format"), s("json"), s("--sandbox"), s("--disable-slash-commands")];
            if let Some(m) = model {
                a.extend([s("--model"), s(m)]);
            }
            a.push(format!("-p={folded}"));
            a
        }
        Harness::Grok => {
            let mut a = vec![s("-p"), s(user), s("--output-format"), s("json"), s("--tools"), s(""), s("--max-turns"), s("1"), s("--permission-mode"), s("plan")];
            if let Some(sys) = system {
                a.extend([s("--system-prompt-override"), s(sys)]);
            }
            if let Some(m) = model {
                a.extend([s("-m"), s(m)]);
            }
            a
        }
    }
}

/// The agent's final answer inside its one-shot output: Claude's `result`,
/// the text of Codex's last `agent_message` item, Antigravity's `response`,
/// Grok's `text`. Err names the failure the envelope reports.
pub fn final_text(agent: Harness, stdout: &str) -> Result<String, String> {
    use serde_json::Value;
    let clip = |t: &str| t.chars().take(120).collect::<String>();
    let json = |who: &str| serde_json::from_str::<Value>(stdout.trim()).map_err(|e| format!("{who} output was not JSON: {e}"));
    match agent {
        Harness::ClaudeCode => {
            let v = json("claude")?;
            let result = v["result"].as_str().unwrap_or("");
            if v["is_error"].as_bool() == Some(true) {
                return Err(format!("claude failed: {}", clip(result)));
            }
            Ok(result.to_string())
        }
        Harness::Codex => {
            let mut last = None;
            for line in stdout.lines() {
                let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
                match v["type"].as_str() {
                    Some("item.completed") if v["item"]["type"].as_str() == Some("agent_message") => last = v["item"]["text"].as_str().map(str::to_string),
                    Some("error") => return Err(format!("codex failed: {}", clip(v["message"].as_str().unwrap_or("")))),
                    Some("turn.failed") => return Err(format!("codex failed: {}", clip(&v["error"].to_string()))),
                    _ => {}
                }
            }
            last.ok_or_else(|| format!("no agent message in: {}", clip(stdout)))
        }
        Harness::Antigravity => {
            let v = json("agy")?;
            if v["status"].as_str() != Some("SUCCESS") {
                return Err(format!("agy failed: {}", clip(&v.to_string())));
            }
            Ok(v["response"].as_str().unwrap_or("").to_string())
        }
        Harness::Grok => {
            let v = json("grok")?;
            v["text"].as_str().map(str::to_string).ok_or_else(|| format!("no text in: {}", clip(stdout)))
        }
    }
}

/// The agent's binary, or the message that names the command to install.
pub fn find_binary(agent: Harness) -> Result<PathBuf, String> {
    find_binary_named(binary_name(agent))
}

fn find_binary_named(name: &str) -> Result<PathBuf, String> {
    agent_binary(name).ok_or_else(|| format!("Could not find the {name} command."))
}
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p maya-core launch::tests`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add core/src/launch.rs core/fixtures/oneshot docs/superpowers/specs/2026-10-03-maya-agent-design.md
git commit -m "feat(core): per-agent one-shot command, reply envelope and capability table"
```

---

### Task 3: The interpreter and the folder classifier run on Maya's agent

**Files:**
- Modify: `core/src/interpreter.rs:1-3, 100-122, 359-421` and its tests
- Modify: `core/src/launch.rs:331-366` (`claude_binary`, `classify`) and tests `:764-782`
- Modify: `core/src/actions.rs:360-366` (`start_session`'s classifier call), `:344-349` (`label_of`)
- Modify: `src-tauri/src/listener.rs:306-340` (interpret), `:211-215` (voice "start")

**Interfaces:**
- Consumes: `launch::oneshot_args`, `launch::final_text`, `launch::find_binary`, `Config::brain`, `Config::brain_model`.
- Produces: `interpreter::parse_reply(agent: Harness, stdout: &str) -> Result<Reply, String>`; `interpreter::run(agent: Harness, binary: &Path, model: Option<&str>, command, cards, prs, history, cwd, timeout) -> Result<Reply, RunError>`; `interpreter::output_within(program: &str, cmd: Command, timeout) -> Result<String, RunError>`; `launch::classify(agent: Harness, binary: &Path, model: Option<&str>, root, user_prompt, dirs, timeout) -> Option<String>`. `launch::claude_binary` is removed.

- [ ] **Step 1: Write the failing tests**

In `core/src/interpreter.rs` tests, replace `parses_the_json_inside_claude_print_output` (`:517`) and the `claude_args` test (`:647-652`) with:

```rust
    fn fixture(name: &str) -> String {
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/oneshot").join(name)).unwrap()
    }

    #[test]
    fn parses_the_reply_inside_each_agents_output() {
        let r = parse_reply(crate::model::Harness::ClaudeCode, &fixture("claude.json")).unwrap();
        assert_eq!(r.say, "Telling hexgrid: go ahead.");
        assert_eq!(r.action.unwrap()["kind"], "reply");
        assert!(r.confirm);
        for (agent, file) in [(crate::model::Harness::Codex, "codex.jsonl"), (crate::model::Harness::Antigravity, "agy.json"), (crate::model::Harness::Grok, "grok.json")] {
            let r = parse_reply(agent, &fixture(file)).unwrap();
            assert_eq!((r.say.as_str(), r.action.is_none(), r.confirm), ("hi", true, false), "{file}");
        }
    }

    #[test]
    fn garbage_and_failures_are_errors_not_replies() {
        assert!(parse_reply(crate::model::Harness::Grok, "hello").unwrap_err().contains("not JSON"));
        assert!(parse_reply(crate::model::Harness::Grok, r#"{"text":"no json here"}"#).unwrap_err().contains("no JSON reply"));
        assert!(parse_reply(crate::model::Harness::ClaudeCode, r#"{"type":"result","is_error":true,"result":"Not logged in"}"#).unwrap_err().contains("Not logged in"));
    }
```

In `core/src/launch.rs` tests, change the two classify tests (`:767-782`) so the fake binary prints Claude's envelope:

```rust
    #[cfg(unix)]
    #[test]
    fn classify_uses_the_reply_and_ignores_none() {
        let t = tempfile::tempdir().unwrap();
        let bin = fake_binary(t.path(), r#"echo '{"type":"result","result":"b"}'"#);
        assert_eq!(classify(Harness::ClaudeCode, &bin, None, t.path(), "p", &dirs(), Duration::from_secs(5)).as_deref(), Some("b"));
        let bin = fake_binary(t.path(), r#"echo '{"type":"result","result":"NONE"}'"#);
        assert_eq!(classify(Harness::ClaudeCode, &bin, None, t.path(), "p", &dirs(), Duration::from_secs(5)), None);
        // Another agent's envelope is read the same way.
        let bin = fake_binary(t.path(), r#"echo '{"text":"sonarqube","stopReason":"end_turn"}'"#);
        assert_eq!(classify(Harness::Grok, &bin, Some("grok-4.7"), t.path(), "p", &dirs(), Duration::from_secs(5)).as_deref(), Some("sonarqube"));
    }

    #[cfg(unix)]
    #[test]
    fn classify_times_out_and_falls_back() {
        let t = tempfile::tempdir().unwrap();
        let bin = fake_binary(t.path(), "sleep 5; echo b");
        let start = Instant::now();
        assert_eq!(classify(Harness::ClaudeCode, &bin, None, t.path(), "p", &dirs(), Duration::from_secs(1)), None);
        assert!(start.elapsed() < Duration::from_secs(3));
    }
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core interpreter:: launch::tests::classify`
Expected: compile errors.

- [ ] **Step 3: Implement in `core/src/interpreter.rs`**

Change the module doc (`:1`) to `//! Turns a spoken command into one action with a one-shot call to Maya's agent.` Delete `claude_args` (`:359-369`). Replace `parse_reply` (`:106-122`) with:

```rust
/// The reply inside the agent's one-shot output.
pub fn parse_reply(agent: crate::model::Harness, stdout: &str) -> Result<Reply, String> {
    let text = crate::launch::final_text(agent, stdout)?;
    let r = json_in(&text).ok_or_else(|| format!("no JSON reply in: {}", text.chars().take(120).collect::<String>()))?;
    Ok(Reply {
        say: r["say"].as_str().unwrap_or("").trim().to_string(),
        action: match &r["action"] {
            Value::Null => None,
            a => Some(a.clone()),
        },
        confirm: r["confirm"].as_bool().unwrap_or(false),
    })
}
```

Change `output_within` to take the program name for its messages: `pub fn output_within(program: &str, mut cmd: Command, timeout: Duration)`, with `format!("could not run {program}: {e}")` and `format!("could not read {program}'s output")`. Its three test calls (`:662-667`) gain `"sh"` as the first argument. Replace `run` (`:404-421`) with:

```rust
/// Runs the interpreter on `agent` at `binary`. `cwd` is a neutral directory
/// (Maya's data dir), so no project's instructions or settings load.
pub fn run(agent: crate::model::Harness, binary: &Path, model: Option<&str>, command: &str, cards: &[Card], prs: &[ReviewPr], history: &[(String, String)], cwd: &Path, timeout: Duration) -> Result<Reply, RunError> {
    std::fs::create_dir_all(cwd).map_err(|e| RunError::Failed(format!("could not create {}: {e}", cwd.display())))?;
    let summary = board_summary(cards);
    let pr_lines = pr_summary(prs);
    let prompt = user_prompt(command, &summary, &pr_lines, history);
    let program = crate::launch::binary_name(agent);
    crate::log::line("interpreter", format!("asking {program} {}: {command}\nboard:\n{summary}\npull requests:\n{}\nrecent exchanges: {}", model.unwrap_or("(default model)"), if pr_lines.is_empty() { "none" } else { pr_lines.as_str() }, history.len()));
    let mut cmd = crate::command(binary);
    cmd.args(crate::launch::oneshot_args(agent, model, Some(&system_prompt()), &prompt)).current_dir(cwd).env_clear().envs(crate::launch::clean_env(std::env::vars()));
    let started = std::time::Instant::now();
    let out = output_within(program, cmd, timeout).inspect_err(|e| crate::log::line("interpreter", format!("failed after {:.1}s: {e}", started.elapsed().as_secs_f32())))?;
    let ms = started.elapsed().as_millis();
    let reply = parse_reply(agent, &out).map_err(RunError::Failed);
    match &reply {
        Ok(r) => crate::log::line("interpreter", format!("reply in {ms} ms: say={:?} action={} confirm={}", r.say, r.action.as_ref().map_or("null".to_string(), |a| a.to_string()), r.confirm)),
        Err(e) => crate::log::line("interpreter", format!("unusable reply in {ms} ms: {e}\nraw: {}", crate::log::clip(&out, 1500))),
    }
    reply
}
```

- [ ] **Step 4: Implement the classifier in `core/src/launch.rs`**

Delete `claude_binary` (`:331-334`). Replace `classify` (`:337-366`) with:

```rust
/// Runs the headless folder picker on `agent`; None on NONE, no match,
/// timeout or any error.
pub fn classify(agent: Harness, binary: &Path, model: Option<&str>, root: &Path, user_prompt: &str, dirs: &[String], timeout: Duration) -> Option<String> {
    let mut child = crate::command(binary)
        .args(oneshot_args(agent, model, None, &classifier_prompt(user_prompt, dirs)))
        .current_dir(root)
        .env_clear()
        .envs(clean_env(std::env::vars()))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = final_text(agent, &String::from_utf8_lossy(&out.stdout)).ok()?;
    pick_dir(&text, dirs)
}
```

- [ ] **Step 5: Callers: `actions.rs` and `listener.rs`**

In `core/src/actions.rs` `start_session`, delete the local `label_of` closure (`:344-349`) and use `launch::label(options.agent)` in the "is not installed" message. Replace the classifier arm (`:362-365`) with:

```rust
        None => {
            let (brain, model) = {
                let store = l.store.lock().unwrap();
                (store.config.brain(), store.config.brain_model().map(String::from))
            };
            let binary = launch::find_binary(brain)?;
            launch::classify(brain, &binary, model.as_deref(), &root, &prompt, &dirs, launch::CLASSIFIER_TIMEOUT)
        }
```

Fix the doc comment above `start_session`: "With no `dir`, Maya's agent picks the folder from the prompt."

In `src-tauri/src/listener.rs` `interpret` (`:306-340`): the tuple becomes `(dirs_local, brain, model, maya_dir)` with `store.config.brain()` and `store.config.brain_model().map(String::from)`; replace the binary block and the call:

```rust
    let binary = match launch::find_binary(brain) {
        Ok(b) => b,
        Err(_) => {
            reply_then_idle(app, generation, &format!("I can't find the {} command.", launch::binary_name(brain)), inbox);
            return;
        }
    };
    let reply = match interpreter::run(brain, &binary, model.as_deref(), cmd, &cards, &prs, &history, &maya_dir, interpreter::TIMEOUT) {
```

In the voice `"start"` arm (`:211-215`), start with Maya's agent:

```rust
        "start" => {
            let dir = action["dir"].as_str().unwrap_or("").to_string();
            let prompt = action["prompt"].as_str().unwrap_or("").to_string();
            let brain = state.store.lock().unwrap().config.brain();
            start_session(app.clone(), state.clone(), Some(dir), prompt, launch::LaunchOptions { agent: brain, ..Default::default() }, machine)?;
            Ok("Started.".into())
        }
```

(`state` is the `TauriState<AppState>` already in scope there; if the lock order comments in that function forbid taking `store` here, read `brain` at the top of the function where `store` is first locked and pass it down.)

- [ ] **Step 6: Build and test**

Run: `cargo build --workspace && cargo test --workspace`
Expected: PASS. If `cli/` referenced `claude_binary`, replace it with `launch::find_binary(Harness::ClaudeCode)` (grep showed no such use).

- [ ] **Step 7: Commit**

```bash
git add core/src/interpreter.rs core/src/launch.rs core/src/actions.rs src-tauri/src/listener.rs
git commit -m "feat: the voice interpreter and folder classifier run on Maya's agent"
```

---

### Task 4: Session controls follow the capability table

**Files:**
- Modify: `core/src/answer.rs:12-22` (`slash_command`) and its test `:156-162`
- Modify: `core/src/actions.rs:118-121, 243-271` (`compact_session`, `set_session_option`, `send_slash_command`, `cycle_session_mode`, `type_into_session`)
- Modify: `src/harness.ts`, `src/card.ts:76`, `src/modal.ts:415-437, 95-135, 470-520, 658-700, 818-830`
- Test: `core/src/actions.rs` tests, `core/src/answer.rs` tests, `src/harness.test.ts` (new), `src/modal.test.ts`, `src/card.test.ts`

**Interfaces:**
- Consumes: `launch::capabilities`, `launch::label`, `agents::claude`, `agents::info_for`, `Store::agents_source`.
- Produces: `answer::slash_command(setting, value, models: &[String], efforts: &[String]) -> Result<String, String>`; TS `CAPABILITIES: Record<Harness, Capabilities>` and `capabilitiesOf(h: Harness)` in `src/harness.ts`; `ModalModel.agent?: AgentInfo`.

- [ ] **Step 1: Write the failing Rust tests**

`core/src/answer.rs`, replace `slash_commands_only_for_known_settings`:

```rust
    #[test]
    fn slash_commands_only_for_listed_values() {
        let models = vec!["opus".to_string(), "grok-4.7".to_string()];
        let efforts = vec!["xhigh".to_string()];
        assert_eq!(slash_command("model", "opus", &models, &efforts), Ok("/model opus".to_string()));
        assert_eq!(slash_command("model", "grok-4.7", &models, &efforts), Ok("/model grok-4.7".to_string()));
        assert_eq!(slash_command("effort", "xhigh", &models, &efforts), Ok("/effort xhigh".to_string()));
        assert!(slash_command("model", "gpt", &models, &efforts).unwrap_err().contains("model"));
        assert!(slash_command("effort", "turbo", &models, &efforts).unwrap_err().contains("effort"));
        assert!(slash_command("model", "opus; rm -rf", &models, &efforts).unwrap_err().contains("model"));
        assert!(slash_command("mode", "plan", &models, &efforts).unwrap_err().contains("setting"));
    }
```

`core/src/actions.rs` tests (next to `store_with_codex`):

```rust
    #[test]
    fn controls_follow_the_agents_capabilities() {
        let (_t, store, _path) = store_with_codex(&format!("{TURN_STARTED}\n{TURN_COMPLETE}\n"));
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        // Codex: /compact is typed (with Codex's second Enter), a /model value is refused, Shift+Tab has no meaning.
        compact_session(&l, "c1").unwrap();
        assert!(fake.calls.lock().unwrap().iter().any(|c| matches!(c, Call::Type { tty, text } if tty == "ttys009" && text == "/compact")));
        assert_eq!(set_session_option(&l, "c1", "model", "gpt-5.5").unwrap_err(), "Codex has no /model.");
        assert_eq!(cycle_session_mode(&l, "c1").unwrap_err(), "Codex has no mode cycle.");
        send_slash_command(&l, "c1", "/status").unwrap();
        assert!(fake.calls.lock().unwrap().iter().any(|c| matches!(c, Call::Type { text, .. } if text == "/status")));
        assert_eq!(send_slash_command(&l, "c1", "!ls").unwrap_err(), "Codex has no ! shell lines.");
    }

    #[test]
    fn a_model_switch_is_checked_against_the_agents_own_list() {
        let (_t, store, _path) = store_with_codex(&format!("{TURN_STARTED}\n{TURN_COMPLETE}\n"));
        let grok = ForeignSession { harness: Harness::Grok, pid: 78, tty: Some("ttys010".into()), session_id: "g1".into(), cwd: "/x".into(), name: "Grok".into(), transcript_path: PathBuf::from("/nonexistent") };
        let store = Mutex::new(store.into_inner().unwrap().with_processes(vec![grok]));
        let info = crate::agents::AgentInfo { harness: Harness::Grok, models: vec![crate::agents::ModelInfo { id: "grok-4.7".into(), label: "grok-4.7".into(), efforts: vec![] }], efforts: vec![], modes: vec![] };
        let store = with_agents(store, vec![agents::claude(), info]);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        store.lock().unwrap().refresh(now_ms());
        assert!(set_session_option(&l, "g1", "model", "grok-9").unwrap_err().contains("Unknown model"));
        set_session_option(&l, "g1", "model", "grok-4.7").unwrap();
        assert!(fake.calls.lock().unwrap().iter().any(|c| matches!(c, Call::Type { tty, text } if tty == "ttys010" && text == "/model grok-4.7")));
    }
```

(`with_agents` already exists in `actions::test_support`; `with_processes` is on `Store`. A Grok card's state is read from the folder holding its `transcript_path`: copy `core/fixtures/grok/events.jsonl` and `chat_history.jsonl` into a temp folder whose last event is `turn_ended`, and set `transcript_path` to that folder's `events.jsonl`, so the card is Idle or Completed and `is_free` lets the typing through.)

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core actions::tests::controls_follow answer::tests::slash_commands`
Expected: compile errors (wrong arity of `slash_command`).

- [ ] **Step 3: Implement in Rust**

`core/src/answer.rs`:

```rust
/// The slash command that changes `setting` to `value` in a running session.
/// Only the model and effort have such a command; the value must be in
/// the agent's own list and plain enough to type.
pub fn slash_command(setting: &str, value: &str, models: &[String], efforts: &[String]) -> Result<String, String> {
    let allowed: Vec<&str> = match setting {
        "model" => models.iter().map(String::as_str).collect(),
        "effort" => efforts.iter().map(String::as_str).collect(),
        _ => return Err(format!("Unknown setting: {setting}")),
    };
    if !crate::launch::plain_model_id(value) {
        return Err(format!("Unknown {setting}: {value}"));
    }
    check_choice(setting, value, &allowed)?;
    Ok(format!("/{setting} {value}"))
}
```

`core/src/actions.rs`, replace `type_into_session` (`:259-271`) and the four callers:

```rust
/// Types `text` into a free session's terminal when the agent has `control`
/// (`allowed` reads that off its capabilities). Claude Code queues what is
/// typed while it works; the other agents must be idle or completed, since
/// keys typed into their TUIs elsewhere act as shortcuts.
fn type_into_session(l: &Local, session_id: &str, text: &str, control: &str, allowed: impl Fn(&launch::Capabilities) -> bool) -> Result<(), String> {
    let (harness, tty) = {
        let mut store = l.store.lock().unwrap();
        let card = store.card_for(session_id, now_ms()).ok_or("Session is no longer running.")?;
        if !allowed(&launch::capabilities(card.harness)) {
            return Err(format!("{} has no {control}.", launch::label(card.harness)));
        }
        if card.harness == model::Harness::ClaudeCode {
            answer::check_free(&card)?;
        } else if !crate::pending_names::is_free(&card) {
            return Err("Wait until the session is free.".into());
        }
        (card.harness, session_tty(&store, session_id, card.pid)?)
    };
    type_line_for(l, harness, &tty, text)
}

/// Types `/compact` into the session's terminal.
pub fn compact_session(l: &Local, session_id: &str) -> Result<(), String> {
    type_into_session(l, session_id, answer::COMPACT, "/compact", |c| c.compact)
}

/// Types `/model x` or `/effort y` into the session's terminal, after
/// checking the value against the agent's own listing.
pub fn set_session_option(l: &Local, session_id: &str, setting: &str, value: &str) -> Result<(), String> {
    let (harness, agents) = {
        let mut store = l.store.lock().unwrap();
        let card = store.card_for(session_id, now_ms()).ok_or("Session is no longer running.")?;
        (card.harness, store.agents_source())
    };
    let info = if harness == model::Harness::ClaudeCode {
        crate::agents::claude()
    } else {
        agents().into_iter().find(|a| a.harness == harness).unwrap_or_else(|| crate::agents::info_for(harness, None))
    };
    let line = answer::slash_command(setting, value, &info.model_ids(), &info.efforts)?;
    let is_model = setting == "model";
    type_into_session(l, session_id, &line, &format!("/{setting}"), move |c| if is_model { c.model_switch } else { c.effort_switch })
}

/// Types a `/command` or `!` shell line from the composer into the
/// session's terminal, after `answer::check_terminal_command`.
pub fn send_slash_command(l: &Local, session_id: &str, text: &str) -> Result<(), String> {
    let line = answer::check_terminal_command(text)?;
    let shell = line.starts_with('!');
    type_into_session(l, session_id, &line, if shell { "! shell lines" } else { "/ commands" }, move |c| if shell { c.shell_lines } else { c.slash_lines })
}

/// Sends the agent's mode-cycle keys (Shift+Tab) to the session's terminal.
pub fn cycle_session_mode(l: &Local, session_id: &str) -> Result<(), String> {
    let harness = l.store.lock().unwrap().card_for(session_id, now_ms()).ok_or("Session is no longer running.")?.harness;
    let keys = launch::capabilities(harness).mode_cycle.ok_or_else(|| format!("{} has no mode cycle.", launch::label(harness)))?;
    type_into_session(l, session_id, keys, "mode cycle", |c| c.mode_cycle.is_some())
}
```

Check the order of `type_into_session`'s checks against the test: the "has no" refusal must come before the free check so an Idle Codex card still says "Codex has no /model.".

- [ ] **Step 4: Run the Rust tests**

Run: `cargo test --workspace`
Expected: PASS (fix any remaining `slash_command` callers the compiler finds).

- [ ] **Step 5: Write the failing frontend tests**

`src/harness.test.ts` (new):

```ts
import { describe, expect, it } from "vitest";
import { CAPABILITIES, capabilitiesOf } from "./harness";

describe("capabilities", () => {
  it("mirror the Rust table", () => {
    expect(CAPABILITIES["claude-code"]).toEqual({ compact: true, modelSwitch: true, effortSwitch: true, modeCycle: true, slashLines: true, shellLines: true });
    expect(CAPABILITIES.codex.modelSwitch).toBe(false);
    expect(CAPABILITIES.codex.compact).toBe(true);
    expect(CAPABILITIES.grok.modelSwitch).toBe(true);
    expect(capabilitiesOf("antigravity").modeCycle).toBe(true);
  });
});
```

`src/modal.test.ts`: add, using the file's existing `base` card and `handlers()` factory:

```ts
  it("shows the controls the agent has", () => {
    const h = handlers();
    const codex = renderModal({ card: { ...base, harness: "codex", hasInbox: false, context: { used: 80_000, window: 100_000, percent: 80 } }, turns: [], status: null, draft: "" }, h);
    expect(codex.querySelector("button[data-action=compact]")).not.toBeNull();
    expect(codex.querySelector("select[name=model]")).toBeNull();
    expect(codex.querySelector("button[data-action=cycle-mode]")).toBeNull();
    const grok = renderModal({ card: { ...base, harness: "grok", hasInbox: false }, turns: [], status: null, draft: "", agent: { harness: "grok", models: [{ id: "grok-4.7", label: "grok-4.7", efforts: [] }], efforts: [], modes: [] } }, h);
    expect([...grok.querySelectorAll<HTMLOptionElement>("select[name=model] option")].map((o) => o.value)).toEqual(["", "grok-4.7"]);
    expect(grok.querySelector("select[name=effort]")).toBeNull();
    expect(grok.querySelector("button[data-action=cycle-mode]")).not.toBeNull();
  });
```

`src/card.test.ts`: add a test that a Codex card at 80 % context shows the compact button and an Antigravity one too (both `compact: true`), mirroring the existing Claude compact-button test in that file.

- [ ] **Step 6: Implement the frontend**

`src/harness.ts`, append:

```ts
/** What Maya can type into a running session of each agent: the Rust table in `core/src/launch.rs`. */
export interface Capabilities {
  compact: boolean;
  modelSwitch: boolean;
  effortSwitch: boolean;
  modeCycle: boolean;
  slashLines: boolean;
  shellLines: boolean;
}

export const CAPABILITIES: Record<Harness, Capabilities> = {
  "claude-code": { compact: true, modelSwitch: true, effortSwitch: true, modeCycle: true, slashLines: true, shellLines: true },
  codex: { compact: true, modelSwitch: false, effortSwitch: false, modeCycle: false, slashLines: true, shellLines: false },
  antigravity: { compact: true, modelSwitch: false, effortSwitch: false, modeCycle: true, slashLines: true, shellLines: false },
  grok: { compact: true, modelSwitch: true, effortSwitch: false, modeCycle: true, slashLines: true, shellLines: false },
};

export function capabilitiesOf(h: Harness): Capabilities {
  return CAPABILITIES[h] ?? CAPABILITIES["claude-code"];
}
```

Keep this in step with the probe results written into `launch::capabilities`.

`src/card.ts:76`: `if (capabilitiesOf(card.harness).compact && card.context && card.context.percent >= COMPACT_AT) actions.append(compactButton());` (import `capabilitiesOf`).

`src/modal.ts`:
- `ModalModel` gains `/** The card's agent with its model list, for the Model and Effort pickers; absent until listed. */ agent?: AgentInfo;` (import `type AgentInfo, CLAUDE_AGENT` from `./newsession`, and `capabilitiesOf` from `./harness`).
- `renderTweaks(h, caps, info)`:

```ts
function renderTweaks(h: ModalHandlers, caps: Capabilities, info: AgentInfo): HTMLElement | null {
  const row = el("div", "modal__tweaks");
  const selects: ["model" | "effort", HTMLSelectElement][] = [];
  if (caps.modelSwitch && info.models.length > 0) {
    const model = renderChoice("model", "", info.models.map((m) => [m.id, m.label]), "", "Model");
    selects.push(["model", model.querySelector("select")!]);
    row.append(model);
  }
  if (caps.effortSwitch && info.efforts.length > 0) {
    const effort = renderChoice("effort", "", info.efforts.map((v) => [v, v]), "", "Effort");
    selects.push(["effort", effort.querySelector("select")!]);
    row.append(effort);
  }
  if (selects.length > 0) {
    const apply = el("button", "card__btn", "Apply");
    apply.type = "button";
    apply.dataset.action = "apply";
    const sync = () => { apply.disabled = selects.every(([, sel]) => sel.value === ""); };
    for (const [, sel] of selects) sel.addEventListener("change", sync);
    apply.addEventListener("click", () => {
      if (apply.disabled) return;
      for (const [name, sel] of selects) {
        if (sel.value) h.onSetOption(name, sel.value);
        sel.value = "";
      }
      sync();
    });
    sync();
    row.append(apply);
  }
  if (caps.modeCycle) {
    const cycle = el("button", "card__btn", "Cycle mode");
    cycle.type = "button";
    cycle.dataset.action = "cycle-mode";
    cycle.title = "Sends Shift+Tab to the terminal: the next mode. Check the terminal to see which.";
    cycle.addEventListener("click", () => h.onCycleMode());
    row.append(cycle);
  }
  return row.childElementCount > 0 ? row : null;
}
```

- In `renderModal`: `const caps = capabilitiesOf(m.card.harness);` replaces the `claude` flag for the compact button (`if (caps.compact && m.card.context.percent >= COMPACT_AT)`) and the tweaks: `const tweaks = renderTweaks(h, caps, m.agent ?? (m.card.harness === "claude-code" ? CLAUDE_AGENT : { harness: m.card.harness, models: [], efforts: [], modes: [] })); if (tweaks) panel.append(tweaks);`. Keep `claude` for the composer's presence (`m.card.hasInbox || !claude`) and its placeholder. In `trySend`: `const line = ta.value.trim(); if (isTerminalCommand(line) && (line.startsWith("/") ? caps.slashLines : caps.shellLines)) { h.onCommand(line); return; }`.
- In `openModal`, after `paint(...)`, list the agent's models once for a non-Claude card: 

```ts
  if (card.harness !== "claude-code") {
    const me = current;
    void invoke<{ agents: AgentInfo[] }>("list_agents", { machine: card.machine ?? "" })
      .then((r) => {
        if (current !== me) return;
        me.model.agent = r.agents.find((a) => a.harness === card.harness);
        paint();
      })
      .catch(() => undefined);
  }
```

Drop the now-unused `MODEL_CHOICES`/`EFFORT_CHOICES` imports if nothing else uses them.

- [ ] **Step 7: Run the frontend tests and build**

Run: `pnpm test && pnpm build`
Expected: PASS. Fix any existing modal test that asserted the tweaks row for a Codex card or the "only available" copy.

- [ ] **Step 8: Commit**

```bash
git add core/src/answer.rs core/src/actions.rs src/harness.ts src/harness.test.ts src/card.ts src/card.test.ts src/modal.ts src/modal.test.ts
git commit -m "feat: /compact, model and effort changes, the mode cycle and / lines for every agent that has them"
```

---

### Task 5: Resume for every agent

**Files:**
- Modify: `core/src/resume.rs` (whole listing section `:52-99`), `core/src/codex.rs:137-170` (`rollouts`, `rollout_meta`), `core/src/store.rs` (`agent_dirs`, `with_agent_dirs`), `core/src/actions.rs:305-312, 387-400`, `core/src/net/protocol.rs:88-90`, `src-tauri/src/net_app.rs:182-183`, `cli/src/executor.rs:72-73`, `src-tauri/src/lib.rs:875-891`, `src-tauri/src/listener.rs:204-209`
- Test: `core/src/resume.rs` tests, `core/src/actions.rs` tests, `core/src/net/protocol.rs` tests

**Interfaces:**
- Consumes: `codex::thread_name`, `codex::parse_turns`, `codex::ms_of`, `antigravity::conversation_name`, `grok::encode_cwd`, `grok::title`, `transcript::{Turn, TurnKind}`, `state::truncate`.
- Produces: `resume::AgentDirs { claude, codex, agy, grok: PathBuf }`; `resume::list_sessions(agent, dirs: &AgentDirs, dir: &str, running_ids: &[String]) -> Vec<ResumableSession>`; `resume::plain_session_id(&str) -> bool`; `resume::resume_command(agent, dir: &Path, id: &str) -> String`; `codex::rollouts(codex_dir) -> Vec<PathBuf>`; `codex::rollout_meta(path) -> Option<(String, String)>` (id, cwd); `Store::agent_dirs(&self) -> AgentDirs`; `actions::list_resumable_sessions(l, agent, dir)`; `actions::resume_session(l, agent, dir, session_id)`; `CommandKind::Resume { dir, session, agent }` and `CommandKind::ListResumable { dir, agent }` with `#[serde(default)] agent: Harness`; Tauri commands `list_resumable_sessions(dir, machine, agent: Option<Harness>)` and `resume_session(dir, session_id, machine, agent: Option<Harness>)`.

- [ ] **Step 1: Write the failing tests** in `core/src/resume.rs`

```rust
    use crate::model::Harness;

    fn dirs_in(t: &Path) -> AgentDirs {
        AgentDirs { claude: t.join("claude"), codex: t.join("codex"), agy: t.join("agy"), grok: t.join("grok") }
    }

    #[test]
    fn codex_sessions_of_a_folder_come_from_its_rollouts_and_index() {
        let t = tempfile::tempdir().unwrap();
        let d = dirs_in(t.path());
        let day = d.codex.join("sessions/2026/10/03");
        std::fs::create_dir_all(&day).unwrap();
        let meta = |id: &str, cwd: &str| format!("{{\"timestamp\":\"2026-10-03T05:15:41.197Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"{cwd}\",\"originator\":\"codex-tui\"}}}}\n{{\"timestamp\":\"2026-10-03T05:15:42.000Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"Say hi please\"}}]}}}}\n");
        std::fs::write(day.join("rollout-2026-10-03T06-15-39-aaaa.jsonl"), meta("aaaa", "/p/maya")).unwrap();
        std::fs::write(day.join("rollout-2026-10-03T06-16-39-bbbb.jsonl"), meta("bbbb", "/p/other")).unwrap();
        std::fs::write(day.join("rollout-2026-10-03T06-17-39-cccc.jsonl"), meta("cccc", "/p/maya")).unwrap();
        std::fs::write(d.codex.join("session_index.jsonl"), "{\"id\":\"aaaa\",\"thread_name\":\"Testing Codex\",\"updated_at\":\"2026-10-03T06:20:00Z\"}\n").unwrap();
        let list = list_sessions(Harness::Codex, &d, "/p/maya", &["cccc".to_string()]);
        let ids: Vec<&str> = list.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"aaaa") && ids.contains(&"cccc") && !ids.contains(&"bbbb"), "{ids:?}");
        let a = list.iter().find(|s| s.id == "aaaa").unwrap();
        assert_eq!(a.title, "Testing Codex");
        assert!(!a.running);
        let c = list.iter().find(|s| s.id == "cccc").unwrap();
        assert_eq!(c.title, "Say hi please", "no index name: the first prompt");
        assert!(c.running);
    }

    #[test]
    fn antigravity_sessions_of_a_folder_come_from_history() {
        let t = tempfile::tempdir().unwrap();
        let d = dirs_in(t.path());
        std::fs::create_dir_all(&d.agy).unwrap();
        std::fs::write(
            d.agy.join("history.jsonl"),
            concat!(
                "{\"display\":\"Say hello\",\"timestamp\":1790671434791,\"workspace\":\"/p/maya\",\"conversationId\":\"c-1\"}\n",
                "{\"display\":\"/rename Testing AGY\",\"timestamp\":1790671500000,\"workspace\":\"/p/maya\",\"conversationId\":\"c-1\",\"type\":\"slash_command\"}\n",
                "{\"display\":\"Other work\",\"timestamp\":1790671600000,\"workspace\":\"/p/other\",\"conversationId\":\"c-2\"}\n",
                "{\"display\":\"/context\",\"timestamp\":1790671700000,\"workspace\":\"/p/maya\",\"type\":\"slash_command\"}\n",
            ),
        )
        .unwrap();
        let list = list_sessions(Harness::Antigravity, &d, "/p/maya", &[]);
        assert_eq!(list.len(), 1, "{list:?}");
        assert_eq!((list[0].id.as_str(), list[0].title.as_str(), list[0].last_active_ms), ("c-1", "Testing AGY", 1790671500000));
    }

    #[test]
    fn grok_sessions_of_a_folder_come_from_summaries_newest_first() {
        let t = tempfile::tempdir().unwrap();
        let d = dirs_in(t.path());
        let folder = d.grok.join("sessions").join(crate::grok::encode_cwd("/p/maya"));
        for (id, title, at) in [("g-old", "First probe", "2026-10-01T10:00:00.000000Z"), ("g-new", "Maya probe Grok", "2026-10-02T20:24:44.754218Z")] {
            std::fs::create_dir_all(folder.join(id)).unwrap();
            std::fs::write(folder.join(id).join("summary.json"), format!("{{\"info\":{{\"id\":\"{id}\",\"cwd\":\"/p/maya\"}},\"session_summary\":\"{title}\",\"updated_at\":\"{at}\"}}")).unwrap();
        }
        std::fs::create_dir_all(folder.join("no-summary")).unwrap();
        let list = list_sessions(Harness::Grok, &d, "/p/maya", &[]);
        assert_eq!(list.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["g-new", "g-old"]);
        assert_eq!(list[0].title, "Maya probe Grok");
        assert!(list[0].last_active_ms > list[1].last_active_ms);
        assert!(list_sessions(Harness::Grok, &d, "/p/elsewhere", &[]).is_empty());
    }

    #[test]
    fn resume_commands_per_agent_quote_folder_and_id() {
        let dir = Path::new("/Users/x/dev/it's");
        assert_eq!(resume_command(Harness::ClaudeCode, dir, "abc-123"), "cd '/Users/x/dev/it'\\''s' && claude --resume 'abc-123'");
        assert_eq!(resume_command(Harness::Codex, dir, "abc-123"), "cd '/Users/x/dev/it'\\''s' && codex resume 'abc-123'");
        assert_eq!(resume_command(Harness::Antigravity, dir, "abc-123"), "cd '/Users/x/dev/it'\\''s' && agy --conversation 'abc-123'");
        assert_eq!(resume_command(Harness::Grok, dir, "abc-123"), "cd '/Users/x/dev/it'\\''s' && grok -r 'abc-123'");
    }

    #[test]
    fn plain_session_ids_only() {
        assert!(plain_session_id("01a0fc3b-e7fe-7963-94c8-509ae7a10285"));
        assert!(plain_session_id("abc_DEF-9"));
        assert!(!plain_session_id(""));
        assert!(!plain_session_id("../x"));
        assert!(!plain_session_id("a/b"));
        assert!(!plain_session_id("it's"));
        assert!(!plain_session_id(&"x".repeat(129)));
    }
```

Delete the old `resume_command_changes_directory_and_resumes_by_id` test (`:150`). Keep the existing Claude listing tests; change their `list_sessions(claude_dir, dir, running)` calls to `list_sessions(Harness::ClaudeCode, &AgentDirs { claude: claude_dir.to_path_buf(), codex: PathBuf::new(), agy: PathBuf::new(), grok: PathBuf::new() }, dir, running)`.

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core resume::`
Expected: compile errors.

- [ ] **Step 3: Implement `core/src/codex.rs` helpers**

Rename `rollout_files` to `pub fn rollouts(codex_dir: &Path) -> Vec<PathBuf>` (update its two callers in the file) and add, after `first_line`:

```rust
/// The thread id and working directory a rollout's `session_meta` records.
pub fn rollout_meta(path: &Path) -> Option<(String, String)> {
    let first = first_line(path)?;
    let v: Value = serde_json::from_str(first.trim()).ok()?;
    if v["type"].as_str() != Some("session_meta") {
        return None;
    }
    let id = v["payload"]["id"].as_str().or_else(|| v["payload"]["session_id"].as_str())?.to_string();
    Some((id, v["payload"]["cwd"].as_str()?.to_string()))
}
```

Rewrite `rollout_for_id` to use it: `rollouts(codex_dir).into_iter().find(|p| …ends_with(&suffix)).and_then(|p| rollout_meta(&p).map(|(_, cwd)| (p, cwd)))`.

- [ ] **Step 4: Implement `core/src/resume.rs`**

Replace everything from `fn store_for` (`:52`) to the end of `resume_command` (`:99`) with:

```rust
use crate::model::Harness;
use crate::transcript::{Turn, TurnKind};

/// Where each agent keeps its sessions on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentDirs {
    pub claude: PathBuf,
    pub codex: PathBuf,
    pub agy: PathBuf,
    pub grok: PathBuf,
}

fn store_for(claude_dir: &Path, dir: &str) -> PathBuf {
    claude_dir.join("projects").join(crate::registry::project_dir_name(dir))
}

fn mtime_ms(path: &Path) -> Option<u64> {
    Some(std::fs::metadata(path).ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as u64)
}

/// The first `bytes` of `path` as text.
fn head(path: &Path, bytes: u64) -> String {
    std::fs::File::open(path)
        .ok()
        .map(|f| {
            use std::io::Read;
            let mut buf = Vec::new();
            let _ = f.take(bytes).read_to_end(&mut buf);
            String::from_utf8_lossy(&buf).into_owned()
        })
        .unwrap_or_default()
}

fn head_and_tail(path: &Path) -> String {
    let tail = crate::transcript::tail_text(path, TAIL_BYTES).unwrap_or_default();
    format!("{}\n{tail}", head(path, HEAD_BYTES))
}

/// The first line of the first user turn, as a title.
fn first_prompt(turns: &[Turn]) -> Option<String> {
    turns.iter().find(|t| t.kind == TurnKind::User).and_then(|t| t.text.lines().map(str::trim).find(|l| !l.is_empty()).map(|l| truncate(l, 120)))
}

/// Claude Code's transcripts for `dir`. Only top-level ones count:
/// subfolders hold subagent runs.
fn claude_sessions(claude_dir: &Path, dir: &str) -> Vec<ResumableSession> {
    let Ok(entries) = std::fs::read_dir(store_for(claude_dir, dir)) else { return vec![] };
    entries
        .flatten()
        .filter(|e| e.path().is_file() && e.path().extension().map_or(false, |x| x == "jsonl"))
        .filter_map(|e| {
            let path = e.path();
            let id = path.file_stem()?.to_str()?.to_string();
            let last_active_ms = mtime_ms(&path)?;
            let title = title_from(&head_and_tail(&path)).unwrap_or_else(|| id.clone());
            Some(ResumableSession { id, title, last_active_ms, running: false })
        })
        .collect()
}

/// Codex's rollouts whose recorded working directory is `dir`, named from
/// `session_index.jsonl`, else by their first prompt.
fn codex_sessions(codex_dir: &Path, dir: &str) -> Vec<ResumableSession> {
    let index = std::fs::read_to_string(codex_dir.join("session_index.jsonl")).unwrap_or_default();
    crate::codex::rollouts(codex_dir)
        .into_iter()
        .filter_map(|path| {
            let (id, cwd) = crate::codex::rollout_meta(&path)?;
            if cwd != dir {
                return None;
            }
            let last_active_ms = mtime_ms(&path)?;
            let title = crate::codex::thread_name(&index, &id).or_else(|| first_prompt(&crate::codex::parse_turns(&head(&path, HEAD_BYTES), usize::MAX))).unwrap_or_else(|| id.clone());
            Some(ResumableSession { id, title, last_active_ms, running: false })
        })
        .collect()
}

/// Antigravity's conversations with `dir` as workspace, from `history.jsonl`,
/// last active at their latest entry.
fn antigravity_sessions(agy_dir: &Path, dir: &str) -> Vec<ResumableSession> {
    let history = std::fs::read_to_string(agy_dir.join("history.jsonl")).unwrap_or_default();
    let mut seen: Vec<(String, u64)> = Vec::new();
    for v in history.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
        let (Some(id), Some(ws)) = (v["conversationId"].as_str(), v["workspace"].as_str()) else { continue };
        if ws != dir {
            continue;
        }
        let ts = v["timestamp"].as_u64().unwrap_or(0);
        match seen.iter_mut().find(|(i, _)| i == id) {
            Some(e) => e.1 = e.1.max(ts),
            None => seen.push((id.to_string(), ts)),
        }
    }
    seen.into_iter()
        .map(|(id, ts)| ResumableSession { title: crate::antigravity::conversation_name(&history, &id).unwrap_or_else(|| id.clone()), id, last_active_ms: ts, running: false })
        .collect()
}

/// Grok's sessions under `sessions/<encoded dir>/`, named from `summary.json`.
fn grok_sessions(grok_dir: &Path, dir: &str) -> Vec<ResumableSession> {
    let folder = grok_dir.join("sessions").join(crate::grok::encode_cwd(dir));
    let Ok(entries) = std::fs::read_dir(folder) else { return vec![] };
    entries
        .flatten()
        .filter_map(|e| {
            let id = e.file_name().to_str()?.to_string();
            let summary_path = e.path().join("summary.json");
            let summary = std::fs::read_to_string(&summary_path).ok()?;
            let v: Value = serde_json::from_str(&summary).ok()?;
            let last_active_ms = v["updated_at"].as_str().and_then(crate::codex::ms_of).or_else(|| mtime_ms(&summary_path))?;
            Some(ResumableSession { title: crate::grok::title(&summary).unwrap_or_else(|| id.clone()), id, last_active_ms, running: false })
        })
        .collect()
}

/// The agent's sessions recorded for `dir`, newest first, running ones marked.
pub fn list_sessions(agent: Harness, dirs: &AgentDirs, dir: &str, running_ids: &[String]) -> Vec<ResumableSession> {
    let mut out = match agent {
        Harness::ClaudeCode => claude_sessions(&dirs.claude, dir),
        Harness::Codex => codex_sessions(&dirs.codex, dir),
        Harness::Antigravity => antigravity_sessions(&dirs.agy, dir),
        Harness::Grok => grok_sessions(&dirs.grok, dir),
    };
    for s in &mut out {
        s.running = running_ids.contains(&s.id);
    }
    out.sort_by(|a, b| b.last_active_ms.cmp(&a.last_active_ms));
    out
}

/// A session id safe to put in a shell line and a path: letters, digits,
/// `-` and `_`, at most 128 bytes.
pub fn plain_session_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The shell line that resumes `id` inside `dir` with `agent`.
pub fn resume_command(agent: Harness, dir: &Path, id: &str) -> String {
    let d = shell_single_quote(&dir.to_string_lossy());
    let i = shell_single_quote(id);
    match agent {
        Harness::ClaudeCode => format!("cd {d} && claude --resume {i}"),
        Harness::Codex => format!("cd {d} && codex resume {i}"),
        Harness::Antigravity => format!("cd {d} && agy --conversation {i}"),
        Harness::Grok => format!("cd {d} && grok -r {i}"),
    }
}
```

Remove the old `transcript_exists` (its caller goes in Step 5). Keep `title_from`, `first_user_text`, `HEAD_BYTES`, `TAIL_BYTES`, `ResumableSession`.

- [ ] **Step 5: Store, actions, protocol, dispatchers**

`core/src/store.rs`, in `impl Store`:

```rust
    /// Where each agent keeps its sessions.
    pub fn agent_dirs(&self) -> crate::resume::AgentDirs {
        crate::resume::AgentDirs { claude: self.claude_dir.clone(), codex: self.codex_dir.clone(), agy: self.agy_dir.clone(), grok: self.grok_dir.clone() }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_agent_dirs(mut self, codex: PathBuf, agy: PathBuf, grok: PathBuf) -> Self {
        self.codex_dir = codex;
        self.agy_dir = agy;
        self.grok_dir = grok;
        self
    }
```

`core/src/actions.rs`:

```rust
/// The agent's past sessions of a project folder, newest first, running ones marked.
pub fn list_resumable_sessions(l: &Local, agent: model::Harness, dir: &str) -> Result<Vec<resume::ResumableSession>, String> {
    let (path, dirs, running) = {
        let store = l.store.lock().unwrap();
        (project_path(&store, dir)?, store.agent_dirs(), store.live_session_ids())
    };
    Ok(resume::list_sessions(agent, &dirs, &path.to_string_lossy(), &running))
}

/// Opens a terminal in the folder resuming `session_id` with `agent`. The id
/// must be one the agent recorded for that folder, so nothing typed or
/// relayed reaches the shell line unchecked.
pub fn resume_session(l: &Local, agent: model::Harness, dir: &str, session_id: &str) -> Result<(), String> {
    if !resume::plain_session_id(session_id) {
        return Err("No such session in that folder.".into());
    }
    let (path, dirs, running) = {
        let store = l.store.lock().unwrap();
        (project_path(&store, dir)?, store.agent_dirs(), store.live_session_ids())
    };
    if running.iter().any(|id| id == session_id) {
        return Err("That session is already running.".into());
    }
    let known = resume::list_sessions(agent, &dirs, &path.to_string_lossy(), &running);
    if !known.iter().any(|s| s.id == session_id) {
        return Err("No such session in that folder.".into());
    }
    l.terminal.open(&resume::resume_command(agent, &path, session_id), &path, &tmux_label()).map(|_| ())
}
```

Add an actions test:

```rust
    #[test]
    fn resume_refuses_ids_the_agent_did_not_record_and_resumes_known_ones() {
        let (dir, store) = store_with_projects(&["proj"]);
        let grok_dir = dir.path().join("grok");
        let proj = dir.path().join("projects").join("proj");
        let folder = grok_dir.join("sessions").join(crate::grok::encode_cwd(&proj.to_string_lossy())).join("g-1");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("summary.json"), r#"{"session_summary":"Probe","updated_at":"2026-10-02T20:24:44.754218Z"}"#).unwrap();
        let store = Mutex::new(store.into_inner().unwrap().with_agent_dirs(dir.path().join("codex"), dir.path().join("agy"), grok_dir));
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        assert_eq!(resume_session(&l, Harness::Grok, "proj", "../g-1").unwrap_err(), "No such session in that folder.");
        assert_eq!(resume_session(&l, Harness::Grok, "proj", "g-9").unwrap_err(), "No such session in that folder.");
        assert_eq!(resume_session(&l, Harness::Codex, "proj", "g-1").unwrap_err(), "No such session in that folder.", "another agent's id");
        resume_session(&l, Harness::Grok, "proj", "g-1").unwrap();
        let calls = fake.calls.lock().unwrap();
        assert!(matches!(&calls[0], Call::Open { command, .. } if command.ends_with("&& grok -r 'g-1'")), "{calls:?}");
    }
```

(`store_with_projects` builds the projects root with `make_projects(dir.path(), dirs)`; read that helper in `actions::test_support` and build `proj` the same way instead of assuming `projects/proj`. Any existing resume test in `actions.rs` that calls `resume_session(&l, dir, id)` gets `Harness::ClaudeCode` as its new second argument.)

`core/src/net/protocol.rs:89-90`:

```rust
    Resume { dir: String, session: String, #[serde(default)] agent: crate::model::Harness },
    ListResumable { dir: String, #[serde(default)] agent: crate::model::Harness },
```

and a test in its module:

```rust
    #[test]
    fn resume_without_an_agent_means_claude_code() {
        let k: CommandKind = serde_json::from_str(r#"{"kind":"resume","dir":"maya","session":"abc"}"#).unwrap();
        assert_eq!(k, CommandKind::Resume { dir: "maya".into(), session: "abc".into(), agent: crate::model::Harness::ClaudeCode });
        let k: CommandKind = serde_json::from_str(r#"{"kind":"list_resumable","dir":"maya","agent":"grok"}"#).unwrap();
        assert_eq!(k, CommandKind::ListResumable { dir: "maya".into(), agent: crate::model::Harness::Grok });
    }
```

`src-tauri/src/net_app.rs:182-183` and `cli/src/executor.rs:72-73`:

```rust
        CommandKind::Resume { dir, session, agent } => done(actions::resume_session(&l, agent, &dir, &session)),
        CommandKind::ListResumable { dir, agent } => to_data(actions::list_resumable_sessions(&l, agent, &dir)?),
```

(`data(...)` instead of `to_data(...)?` in the CLI, as its neighbours do.)

`src-tauri/src/lib.rs:875-891`:

```rust
/// Past sessions of a project folder for `agent`, newest first, with running ones marked.
#[tauri::command(async)]
fn list_resumable_sessions(app: AppHandle, state: TauriState<AppState>, dir: String, machine: Option<String>, agent: Option<Harness>) -> Result<Vec<resume::ResumableSession>, String> {
    let agent = agent.unwrap_or_default();
    if !is_local(&machine) {
        let machine = machine.unwrap();
        check_remote_agent(agent, &merge::agents_of(&remote_boards(&state), &machine), &machine)?;
        return route_data(&app, &machine, CommandKind::ListResumable { dir, agent });
    }
    actions::list_resumable_sessions(&local(&state), agent, &dir)
}

/// Opens a terminal in the folder resuming the session with `agent`.
#[tauri::command(async)]
fn resume_session(app: AppHandle, state: TauriState<AppState>, dir: String, session_id: String, machine: Option<String>, agent: Option<Harness>) -> Result<(), String> {
    let agent = agent.unwrap_or_default();
    if !is_local(&machine) {
        let machine = machine.unwrap();
        check_remote_agent(agent, &merge::agents_of(&remote_boards(&state), &machine), &machine)?;
        return route_done(&app, &machine, CommandKind::Resume { dir, session: session_id, agent });
    }
    actions::resume_session(&local(&state), agent, &dir, &session_id)
}
```

`src-tauri/src/listener.rs:204-209` (voice resume), with `brain` read as in Task 3:

```rust
        "resume" => {
            let dir = action["dir"].as_str().unwrap_or("").to_string();
            let brain = state.store.lock().unwrap().config.brain();
            let sessions = list_resumable_sessions(app.clone(), state.clone(), dir.clone(), machine.clone(), Some(brain))?;
            let latest = sessions.into_iter().find(|s| !s.running).ok_or("Nothing to resume there.")?;
            resume_session(app.clone(), state.clone(), dir, latest.id, machine, Some(brain))?;
            Ok("Resuming.".into())
        }
```

- [ ] **Step 6: Build and test**

Run: `cargo build --workspace && cargo test --workspace`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add core/src/resume.rs core/src/codex.rs core/src/store.rs core/src/actions.rs core/src/net/protocol.rs src-tauri/src/net_app.rs src-tauri/src/lib.rs src-tauri/src/listener.rs cli/src/executor.rs
git commit -m "feat: resume Codex, Antigravity and Grok Build sessions, not only Claude Code"
```

---

### Task 6: Reviews start with Maya's agent and a configurable prompt

**Files:**
- Modify: `core/src/reviews.rs:1-4, 150-176` and tests
- Modify: `core/src/actions.rs` (new `start_review`)
- Modify: `src-tauri/src/lib.rs:364-377` (`review_pr`)
- Test: `core/src/reviews.rs`, `core/src/actions.rs`

**Interfaces:**
- Consumes: `launch::session_command`, `launch::LaunchOptions`, `launch::write_prompt_file`, `launch::new_session_uuid`, `pending_names::PendingName::new`, `Config::brain`, `Config.review_prompt`.
- Produces: `reviews::DEFAULT_REVIEW_PROMPT: &str`; `reviews::render_prompt(template: &str, pr: &ReviewPr) -> String`; `reviews::shell_command(agent, target: &ReviewTarget, name_with_owner: &str, number: u64, prompt_file: &Path, grok_id: Option<&str>) -> String`; `actions::start_review(l, pr: &ReviewPr) -> Result<String, String>` (the folder).

- [ ] **Step 1: Write the failing tests** in `core/src/reviews.rs`

```rust
    fn pr() -> ReviewPr {
        ReviewPr { number: 451, repo: "Org/bedrock".into(), title: "docs: notes".into(), author: "jane".into(), url: "https://github.com/Org/bedrock/pull/451".into(), is_draft: false, updated_at: "".into(), reasons: vec![Reason::Review] }
    }

    #[test]
    fn the_review_prompt_fills_placeholders_or_appends_the_pr() {
        let d = render_prompt("", &pr());
        assert!(d.starts_with("Review pull request #451 of Org/bedrock (https://github.com/Org/bedrock/pull/451)."), "{d}");
        assert!(d.contains("(Recommended)") && d.contains("Request changes"), "{d}");
        assert_eq!(render_prompt("  \n", &pr()), d, "blank means the default");
        assert_eq!(render_prompt("/should-i-approve PR #{number}", &pr()), "/should-i-approve PR #451");
        assert_eq!(render_prompt("Look at {url} ({repo})", &pr()), "Look at https://github.com/Org/bedrock/pull/451 (Org/bedrock)");
        assert_eq!(render_prompt("/should-i-approve", &pr()), "/should-i-approve PR #451 (https://github.com/Org/bedrock/pull/451)");
    }

    #[test]
    fn the_review_shell_line_runs_the_agent_on_the_prompt_file_with_the_review_name() {
        let file = Path::new("/Users/x/.claude/maya/prompts/1.txt");
        let free = ReviewTarget { dir: PathBuf::from("/Users/x/dev/bedrock"), clone: false };
        let s = shell_command(crate::model::Harness::ClaudeCode, &free, "Org/bedrock", 451, file, None);
        assert_eq!(s, "cd '/Users/x/dev/bedrock' && p=\"$(cat '/Users/x/.claude/maya/prompts/1.txt')\" && rm -f '/Users/x/.claude/maya/prompts/1.txt' && claude -n 'review bedrock #451' -- \"$p\"");
        let clone = ReviewTarget { dir: PathBuf::from("/Users/x/dev/reviews/bedrock-451"), clone: true };
        let s = shell_command(crate::model::Harness::Codex, &clone, "Org/bedrock", 451, file, None);
        assert!(s.starts_with("mkdir -p '/Users/x/dev/reviews' && gh repo clone 'Org/bedrock' '/Users/x/dev/reviews/bedrock-451' && cd '/Users/x/dev/reviews/bedrock-451' && "), "{s}");
        assert!(s.ends_with("&& codex -- \"$p\""), "no -n for Codex: the name waits as a pending name: {s}");
        let s = shell_command(crate::model::Harness::Grok, &free, "Org/bedrock", 451, file, Some("1111-2222"));
        assert!(s.ends_with("&& grok --session-id '1111-2222' -- \"$p\""), "{s}");
        let s = shell_command(crate::model::Harness::Antigravity, &free, "Org/bedrock", 451, file, None);
        assert!(s.ends_with("&& agy --prompt-interactive=\"$p\""), "{s}");
    }
```

Delete the old `shell_command` tests that asserted `/should-i-approve`. In `core/src/actions.rs` tests:

```rust
    #[test]
    fn start_review_runs_maya_agent_and_queues_the_review_name() {
        let (dir, store) = store_with_projects(&["elsewhere"]);
        {
            let mut s = store.lock().unwrap();
            s.config.agent = Some(Harness::Codex);
            s.config.clones_dir = Some(dir.path().join("clones").to_string_lossy().into_owned());
            s.config.review_prompt = "/should-i-approve".into();
        }
        let store = with_agents(store, vec![agents::claude(), crate::agents::info_for(Harness::Codex, None)]);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        let pr = crate::reviews::ReviewPr { number: 451, repo: "Org/bedrock".into(), title: "docs".into(), author: "jane".into(), url: "https://github.com/Org/bedrock/pull/451".into(), is_draft: false, updated_at: String::new(), reasons: vec![] };
        let folder = start_review(&l, &pr).unwrap();
        assert!(folder.ends_with("clones/bedrock-451"), "{folder}");
        let calls = fake.calls.lock().unwrap();
        let Call::Open { command, .. } = &calls[0] else { panic!("{calls:?}") };
        assert!(command.contains("gh repo clone 'Org/bedrock'") && command.ends_with("&& codex -- \"$p\""), "{command}");
        let file = command.split("cat '").nth(1).unwrap().split('\'').next().unwrap();
        assert_eq!(std::fs::read_to_string(file).unwrap(), "/should-i-approve PR #451 (https://github.com/Org/bedrock/pull/451)");
        drop(calls);
        assert!(store.lock().unwrap().has_pending_name_for_test("review bedrock #451"), "the name waits for the Codex session");
    }

    #[test]
    fn start_review_refuses_an_agent_that_is_not_installed() {
        let (_dir, store) = store_with_projects(&["elsewhere"]);
        store.lock().unwrap().config.agent = Some(Harness::Grok);
        let store = with_agents(store, vec![agents::claude()]);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        let pr = crate::reviews::ReviewPr { number: 1, repo: "o/r".into(), title: String::new(), author: String::new(), url: "https://github.com/o/r/pull/1".into(), is_draft: false, updated_at: String::new(), reasons: vec![] };
        assert_eq!(start_review(&l, &pr).unwrap_err(), "Grok Build is not installed on this machine.");
        assert!(fake.calls.lock().unwrap().is_empty());
    }
```

Add to `core/src/pending_names.rs`, in `impl PendingNames`: `pub fn names(&self) -> Vec<String> { self.entries.iter().map(|p| p.name.clone()).collect() }`, and to `Store` under `#[cfg(any(test, feature = "test-support"))]`: `pub fn has_pending_name_for_test(&self, name: &str) -> bool { self.pending.names().iter().any(|n| n == name) }`.

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core reviews:: actions::tests::start_review`
Expected: compile errors.

- [ ] **Step 3: Implement `core/src/reviews.rs`**

Imports: `use crate::launch::{shell_single_quote, LaunchOptions}; use crate::model::Harness;`. Replace `shell_command` (`:164-176`) with:

```rust
/// The prompt a review session starts with: the template with `{number}`,
/// `{repo}` and `{url}` filled in. A blank template means the built-in one;
/// a template that names none of the three gets ` PR #<number> (<url>)`
/// appended, so a bare slash command still says which pull request.
pub const DEFAULT_REVIEW_PROMPT: &str = "Review pull request #{number} of {repo} ({url}). Make use of code review skills, and at the end give at most 3 options: Approve; Ask <questions here>; Request changes <changes here>. Mark the recommended option \"(Recommended)\". Show Ask and Request changes only when they are needed.";

pub fn render_prompt(template: &str, pr: &ReviewPr) -> String {
    let t = template.trim();
    let t = if t.is_empty() { DEFAULT_REVIEW_PROMPT } else { t };
    let number = pr.number.to_string();
    if ["{number}", "{repo}", "{url}"].iter().any(|p| t.contains(p)) {
        t.replace("{number}", &number).replace("{repo}", &pr.repo).replace("{url}", &pr.url)
    } else {
        format!("{t} PR #{number} ({})", pr.url)
    }
}

/// The shell line the review terminal runs: a clone first when needed, then
/// the agent in the folder on the prompt file, as a new session named
/// `review <repo> #<number>` (`-n` for Claude Code; the others get the name
/// typed later, as `actions::start_session` does).
pub fn shell_command(agent: Harness, target: &ReviewTarget, name_with_owner: &str, number: u64, prompt_file: &Path, grok_id: Option<&str>) -> String {
    let opts = LaunchOptions { agent, name: Some(session_name(name_with_owner, number)), ..Default::default() };
    let start = crate::launch::session_command(&target.dir, prompt_file, &opts, grok_id);
    if target.clone {
        let dir = shell_single_quote(&target.dir.to_string_lossy());
        let parent = shell_single_quote(&target.dir.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default());
        format!("mkdir -p {parent} && gh repo clone {} {dir} && {start}", shell_single_quote(name_with_owner))
    } else {
        start
    }
}
```

- [ ] **Step 4: Implement `actions::start_review`** (after `resume_session`)

```rust
/// Opens a terminal reviewing `pr` with Maya's agent: in the project
/// checkout when it is free, else in a clone under the clones directory.
/// Returns the folder. For an agent that takes no name on its command line
/// the review name waits in the store, as `start_session`'s does.
pub fn start_review(l: &Local, pr: &crate::reviews::ReviewPr) -> Result<String, String> {
    let (agent, projects, clones, live, maya_dir, template, agents) = {
        let store = l.store.lock().unwrap();
        (store.config.brain(), store.config.projects_dir_path(), store.config.clones_dir_path(), store.live_cwds(), store.claude_dir().join("maya"), store.config.review_prompt.clone(), store.agents_source())
    };
    let projects = projects.ok_or("Set a projects directory in Settings first.")?;
    if agent != model::Harness::ClaudeCode && !agents().iter().any(|a| a.harness == agent) {
        return Err(format!("{} is not installed on this machine.", launch::label(agent)));
    }
    let target = crate::reviews::resolve_target(&projects, &clones, &pr.repo, pr.number, &live);
    let file = launch::write_prompt_file(&maya_dir, &crate::reviews::render_prompt(&template, pr))?;
    let grok_id = (agent == model::Harness::Grok).then(launch::new_session_uuid);
    let known = {
        let mut store = l.store.lock().unwrap();
        store.refresh(now_ms());
        store.live_session_ids()
    };
    // A clone's folder does not exist yet: the terminal opens in its parent.
    let cwd = if target.clone { target.dir.parent().map(Path::to_path_buf).unwrap_or_else(|| target.dir.clone()) } else { target.dir.clone() };
    l.terminal.open(&crate::reviews::shell_command(agent, &target, &pr.repo, pr.number, &file, grok_id.as_deref()), &cwd, &tmux_label())?;
    if agent != model::Harness::ClaudeCode {
        let name = crate::reviews::session_name(&pr.repo, pr.number);
        l.store.lock().unwrap().add_pending_name(crate::pending_names::PendingName::new(agent, &target.dir.to_string_lossy(), &name, now_ms(), known, grok_id));
    }
    Ok(target.dir.to_string_lossy().into_owned())
}
```

`src-tauri/src/lib.rs` `review_pr` (`:364-377`) becomes:

```rust
/// Opens a terminal that reviews a listed PR with Maya's agent and the
/// review prompt from Settings, in the project checkout when it is free,
/// else in a clone under the clones dir.
#[tauri::command(async)]
fn review_pr(state: TauriState<AppState>, repo: String, number: u64) -> Result<String, String> {
    let pr = review_pr_for(&state, &repo, number)?;
    actions::start_review(&local(&state), &pr)
}
```

Remove the `term::open_terminal_with` import from `lib.rs` if it is now unused.

- [ ] **Step 5: Build and test**

Run: `cargo build --workspace && cargo test --workspace`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add core/src/reviews.rs core/src/actions.rs core/src/store.rs core/src/pending_names.rs src-tauri/src/lib.rs
git commit -m "feat: reviews start with Maya's agent on a configurable prompt, no skill required"
```

---

### Task 7: Agents listing reports only installed agents

**Files:**
- Modify: `core/src/agents.rs:138-146` (`build`), its tests
- Test: `core/src/agents.rs`

**Interfaces:**
- Produces: `agents::build` lists Claude Code only when `find("claude")` is Some. `agents::snapshot()` before the first listing and the `Done` fallback still give `vec![claude()]`.

- [ ] **Step 1: Write the failing test** in `core/src/agents.rs`

```rust
    #[test]
    fn build_lists_only_the_agents_found() {
        let none = build(|_| None, |_, _| None);
        assert!(none.is_empty(), "no agent installed, none listed: {none:?}");
        let only_grok = build(|name| (name == "grok").then(|| PathBuf::from("/bin/grok")), |_, _| Some("Available models:\n  grok-4.7 (default)\n".into()));
        assert_eq!(only_grok.iter().map(|a| a.harness).collect::<Vec<_>>(), vec![Harness::Grok]);
        let both = build(|name| matches!(name, "claude" | "codex").then(|| PathBuf::from(format!("/bin/{name}"))), |_, _| None);
        assert_eq!(both.iter().map(|a| a.harness).collect::<Vec<_>>(), vec![Harness::ClaudeCode, Harness::Codex], "Claude Code stays first");
    }
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p maya-core agents::tests::build_lists_only`
Expected: FAIL ("no agent installed, none listed").

- [ ] **Step 3: Implement**

```rust
/// Each installed agent, Claude Code first, with the models `run` lists for it.
pub fn build(find: impl Fn(&str) -> Option<PathBuf>, run: impl Fn(&Path, &[&str]) -> Option<String>) -> Vec<AgentInfo> {
    let mut out = vec![];
    if find("claude").is_some() {
        out.push(claude());
    }
    for agent in [Harness::Codex, Harness::Antigravity, Harness::Grok] {
        let Some(bin) = find(launch::binary_name(agent)) else { continue };
        let listing = run(&bin, listing_args(agent));
        out.push(info_for(agent, listing.as_deref()));
    }
    out
}
```

Fix any existing `build` test that assumed Claude is always listed (give its `find` a `claude` hit). Update the module doc's first line to "The agents installed on this machine, and the models each offers".

- [ ] **Step 4: Run the tests**

Run: `cargo test --workspace`
Expected: PASS. In `src/newsession.ts`, `loadAgents` already falls back to `[CLAUDE_AGENT]` for an empty list, so the modal still works with nothing installed.

- [ ] **Step 5: Commit**

```bash
git add core/src/agents.rs
git commit -m "fix(agents): list Claude Code only when it is installed"
```

---

### Task 8: Settings: the Maya section and the review prompt

**Files:**
- Modify: `src/settings.ts` (model `:48-95`, handlers `:122-156`, `ConfigJson`, render `:267-280, 536-549`, `saveConfig` `:867-890`, handlers `:915-940`, init `:1056-1093`)
- Modify: `src/settings.test.ts`, `src/settings-flow.test.ts`
- Create: `src/reviewprompt.ts` (the default prompt text, shared by Settings)

**Interfaces:**
- Consumes: `list_agents` (`{ agents: AgentInfo[]; names: boolean }`), `get_config`/`set_config` with `agent`, `agentModel`, `reviewPrompt`; `harnessLabel`, `CLAUDE_AGENT`.
- Produces: `SettingsModel.agent: Harness | null`, `.agentModel: string`, `.agents: AgentInfo[]`, `.reviewPrompt: string`; handlers `onAgent(h: Harness)`, `onAgentModel(id: string)`, `onReviewPrompt(text: string)`; `DEFAULT_REVIEW_PROMPT` in `src/reviewprompt.ts`. `interpreterModel` and `onInterpreter` are removed.

- [ ] **Step 1: Write the failing tests**

`src/settings.test.ts`: in `voiceBase` replace `interpreterModel: "haiku",` with `agent: null, agentModel: "", agents: [], reviewPrompt: "",`; in `handlers()` replace `onInterpreter: vi.fn(),` with `onAgent: vi.fn(), onAgentModel: vi.fn(), onReviewPrompt: vi.fn(),`. Add:

```ts
  const agents = [
    { harness: "claude-code" as const, models: [{ id: "opus", label: "Opus", efforts: [] }], efforts: [], modes: [] },
    { harness: "grok" as const, models: [{ id: "grok-4.7", label: "grok-4.7", efforts: [] }], efforts: [], modes: [] },
  ];

  it("puts Maya's agent first, with that agent's models under it", () => {
    const h = handlers();
    const el = renderSettings({ ...voiceBase, agents, agent: "grok", agentModel: "grok-4.7" }, h);
    expect(el.querySelector(".settings__section .settings__heading")?.textContent).toBe("Maya");
    const agent = el.querySelector<HTMLSelectElement>("select[name=agent]")!;
    expect([...agent.options].map((o) => o.textContent)).toEqual(["Claude Code", "Grok Build"]);
    expect(agent.value).toBe("grok");
    const model = el.querySelector<HTMLSelectElement>("select[name=agentModel]")!;
    expect([...model.options].map((o) => o.value)).toEqual(["", "grok-4.7"]);
    expect(model.value).toBe("grok-4.7");
    expect(el.querySelector("[data-for=agent]")?.textContent).toContain("voice commands");
    agent.value = "claude-code";
    agent.dispatchEvent(new Event("change"));
    expect(h.onAgent).toHaveBeenCalledWith("claude-code");
    model.value = "";
    model.dispatchEvent(new Event("change"));
    expect(h.onAgentModel).toHaveBeenCalledWith("");
  });

  it("falls back to Default for a model the agent no longer lists, and to Claude Code before the listing", () => {
    const el = renderSettings({ ...voiceBase, agents, agent: "grok", agentModel: "grok-9" }, handlers());
    expect(el.querySelector<HTMLSelectElement>("select[name=agentModel]")!.value).toBe("");
    const early = renderSettings({ ...voiceBase, agents: [], agent: null }, handlers());
    expect(el.querySelector("select[name=interpreterModel]")).toBeNull();
    expect([...early.querySelector<HTMLSelectElement>("select[name=agent]")!.options].map((o) => o.value)).toEqual(["claude-code"]);
  });

  it("offers the review prompt with the built-in one as placeholder", () => {
    const h = handlers();
    const el = renderSettings({ ...voiceBase, reviewPrompt: "" }, h);
    const ta = el.querySelector<HTMLTextAreaElement>("textarea[name=reviewPrompt]")!;
    expect(ta.placeholder).toContain("Review pull request #{number}");
    expect(ta.value).toBe("");
    ta.value = "/should-i-approve PR #{number}";
    ta.dispatchEvent(new Event("change"));
    expect(h.onReviewPrompt).toHaveBeenCalledWith("/should-i-approve PR #{number}");
  });
```

`src/settings-flow.test.ts`: the config object loses `agentModel: "haiku"` from Task 1 and gains `agent: "claude-code", agentModel: "", reviewPrompt: ""`; the mock gains `if (cmd === "list_agents") return Promise.resolve({ agents: [], names: true });`. Add a flow test:

```ts
  it("changing the agent clears its model and saves both", async () => {
    // same config/mocks as above, plus:
    // list_agents → { agents: [claude, grok as in settings.test.ts], names: true }
    await initSettings();
    await flush();
    await flush();
    const agent = document.querySelector<HTMLSelectElement>("select[name=agent]")!;
    agent.value = "grok";
    agent.dispatchEvent(new Event("change"));
    await flush();
    await flush();
    expect(invoke).toHaveBeenCalledWith("set_config", { config: expect.objectContaining({ agent: "grok", agentModel: "" }) });
  });
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `pnpm test -- settings`
Expected: FAIL (type errors on `agents`, no `select[name=agent]`).

- [ ] **Step 3: Implement**

Create `src/reviewprompt.ts`:

```ts
/** The prompt a review session starts with when Settings leaves it blank; mirrors `reviews::DEFAULT_REVIEW_PROMPT`. */
export const DEFAULT_REVIEW_PROMPT =
  'Review pull request #{number} of {repo} ({url}). Make use of code review skills, and at the end give at most 3 options: Approve; Ask <questions here>; Request changes <changes here>. Mark the recommended option "(Recommended)". Show Ask and Request changes only when they are needed.';
```

`src/settings.ts`:
- Imports: `import { CLAUDE_AGENT, type AgentInfo } from "./newsession"; import { harnessLabel } from "./harness"; import { DEFAULT_REVIEW_PROMPT } from "./reviewprompt";` and `type Harness` from `./types`.
- `SettingsModel`: replace `interpreterModel: string;` with

```ts
  /** The agent that powers Maya; null until chosen (the first-start modal asks). */
  agent: Harness | null;
  /** That agent's model id; "" is the agent's default. */
  agentModel: string;
  /** The agents installed on this machine, with their models; empty until listed. */
  agents: AgentInfo[];
  /** The review prompt; "" means the built-in one. */
  reviewPrompt: string;
```

- `SettingsHandlers`: replace `onInterpreter(model: string): void;` with `onAgent(agent: Harness): void; onAgentModel(id: string): void; onReviewPrompt(text: string): void;`.
- `ConfigJson`: `agent?: Harness | null; agentModel?: string; reviewPrompt?: string;` (from Task 1).
- `renderSettings`: create `const maya = section("Maya");` and `root.append(maya, sessions, notifications, assistant, network);`. Right after, before the hook status:

```ts
  const agents = model.agents.length > 0 ? model.agents : [CLAUDE_AGENT];
  const chosenAgent = agents.find((a) => a.harness === model.agent) ?? agents[0];
  const agentLabel = document.createElement("label");
  agentLabel.textContent = "Agent";
  const agentSel = document.createElement("select");
  agentSel.name = "agent";
  for (const a of agents) {
    const o = document.createElement("option");
    o.value = a.harness;
    o.textContent = harnessLabel(a.harness);
    agentSel.append(o);
  }
  agentSel.value = chosenAgent.harness;
  agentSel.addEventListener("change", () => h.onAgent(agentSel.value as Harness));
  agentLabel.append(agentSel);
  maya.append(agentLabel);
  const agentHint = document.createElement("div");
  agentHint.className = "settings__hint";
  agentHint.dataset.for = "agent";
  agentHint.textContent = "Interprets your voice commands, picks folders for 'Let Maya choose', and is the default for new and resumed sessions and reviews.";
  maya.append(agentHint);

  const agentModelLabel = document.createElement("label");
  agentModelLabel.textContent = "Model";
  const agentModelSel = document.createElement("select");
  agentModelSel.name = "agentModel";
  const def = document.createElement("option");
  def.value = "";
  def.textContent = "Default";
  agentModelSel.append(def);
  for (const m of chosenAgent.models) {
    const o = document.createElement("option");
    o.value = m.id;
    o.textContent = m.label;
    agentModelSel.append(o);
  }
  agentModelSel.value = chosenAgent.models.some((m) => m.id === model.agentModel) ? model.agentModel : "";
  agentModelSel.addEventListener("change", () => h.onAgentModel(agentModelSel.value));
  agentModelLabel.append(agentModelSel);
  maya.append(agentModelLabel);
```

After the clones directory field in `sessions`:

```ts
  const reviewLabel = document.createElement("label");
  reviewLabel.textContent = "Review prompt (for the Review button)";
  const reviewInput = document.createElement("textarea");
  reviewInput.name = "reviewPrompt";
  reviewInput.rows = 4;
  reviewInput.placeholder = DEFAULT_REVIEW_PROMPT;
  reviewInput.value = model.reviewPrompt;
  reviewInput.addEventListener("change", () => h.onReviewPrompt(reviewInput.value.trim()));
  reviewLabel.append(reviewInput);
  sessions.append(reviewLabel);
  const reviewHint = document.createElement("div");
  reviewHint.className = "settings__hint";
  reviewHint.dataset.for = "reviewPrompt";
  reviewHint.textContent = "Blank uses the built-in prompt. {number}, {repo} and {url} are filled in; a prompt that uses none gets \" PR #<number> (<url>)\" appended.";
  sessions.append(reviewHint);
```

Delete the "Voice interpreter" block (`:536-549`).

- `saveConfig`'s copy-back: add `model.agent = c.agent ?? null; model.agentModel = c.agentModel ?? ""; model.reviewPrompt = c.reviewPrompt ?? "";` and remove the `interpreterModel` line.
- Handlers: remove `onInterpreter`; add

```ts
    onAgent: (agent) => void run(() => saveConfig({ agent, agentModel: "" })),
    onAgentModel: (id) => void run(() => saveConfig({ agentModel: id })),
    onReviewPrompt: (text) => void run(() => saveConfig({ reviewPrompt: text })),
```

- Init: `model.agent = config.agent ?? null; model.agentModel = config.agentModel ?? ""; model.reviewPrompt = config.reviewPrompt ?? "";` and, after the `await run(...)` block (not inside it, since the first listing can take seconds):

```ts
  void invoke<{ agents: AgentInfo[] }>("list_agents", { machine: "" })
    .then((r) => {
      model.agents = r.agents;
      if (!panel.hidden) paint();
    })
    .catch(() => undefined);
```

- Initial `model` literal in `initSettings`: replace `interpreterModel: "haiku"` with `agent: null, agentModel: "", agents: [], reviewPrompt: ""`.
- `src/styles.css`: `.settings textarea { font: inherit; width: 100%; max-width: 640px; resize: vertical; }` next to the existing input rule.

- [ ] **Step 4: Run the tests and build**

Run: `pnpm test && pnpm build`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/settings.ts src/settings.test.ts src/settings-flow.test.ts src/reviewprompt.ts src/styles.css
git commit -m "feat(settings): choose the agent that powers Maya, its model, and the review prompt"
```

---

### Task 9: The first-start modal

**Files:**
- Create: `src/firstrun.ts`, `src/firstrun.test.ts`
- Modify: `src/main.ts:150-155` (call `maybeShowFirstRun`), `src/styles.css`

**Interfaces:**
- Consumes: `get_config`, `set_config`, `list_agents`, `CLAUDE_AGENT`, `HARNESS_ICON`, `harnessLabel`.
- Produces: `needsFirstRun(config: { agent?: Harness | null }): boolean`; `renderFirstRun(m: FirstRunModel, h: FirstRunHandlers): HTMLElement`; `maybeShowFirstRun(): Promise<void>`.

- [ ] **Step 1: Write the failing tests** in `src/firstrun.test.ts`

```ts
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { maybeShowFirstRun, needsFirstRun, renderFirstRun } from "./firstrun";

const flush = () => new Promise((r) => setTimeout(r, 0));
const claude = { harness: "claude-code" as const, models: [], efforts: [], modes: [] };
const codex = { harness: "codex" as const, models: [], efforts: [], modes: [] };
const handlers = () => ({ onChoose: vi.fn(), onContinue: vi.fn() });

describe("renderFirstRun", () => {
  it("lists the installed agents with the first preselected, and continues", () => {
    const h = handlers();
    const el = renderFirstRun({ agents: [claude, codex], chosen: "claude-code", saving: false, error: null }, h);
    expect(el.querySelector("h2")?.textContent).toBe("Which agent should power Maya?");
    const radios = el.querySelectorAll<HTMLInputElement>("input[name=agent]");
    expect([...radios].map((r) => r.value)).toEqual(["claude-code", "codex"]);
    expect(radios[0].checked).toBe(true);
    radios[1].click();
    expect(h.onChoose).toHaveBeenCalledWith("codex");
    el.querySelector<HTMLButtonElement>("button[data-action=continue]")!.click();
    expect(h.onContinue).toHaveBeenCalled();
    expect(el.querySelector(".modal__backdrop")).not.toBeNull();
    expect(el.querySelector("button[data-action=close]")).toBeNull();
  });

  it("says when nothing is installed and falls back to Claude Code", () => {
    const el = renderFirstRun({ agents: [], chosen: "claude-code", saving: false, error: null }, handlers());
    expect(el.textContent).toContain("Claude Code, Codex, Antigravity or Grok Build");
    expect(el.querySelector("button[data-action=continue]")?.textContent).toBe("Continue with Claude Code");
    expect(renderFirstRun({ agents: null, chosen: "claude-code", saving: false, error: null }, handlers()).textContent).toContain("Looking for agents");
  });
});

describe("maybeShowFirstRun", () => {
  beforeEach(() => {
    document.body.innerHTML = '<div id="modal-host"></div>';
    invoke.mockReset();
  });

  it("does nothing when an agent is chosen", async () => {
    expect(needsFirstRun({ agent: "codex" })).toBe(false);
    expect(needsFirstRun({})).toBe(true);
    invoke.mockImplementation((cmd: string) => (cmd === "get_config" ? Promise.resolve({ agent: "codex" }) : Promise.reject(new Error(cmd))));
    await maybeShowFirstRun();
    expect(document.querySelector(".modal")).toBeNull();
  });

  it("asks once, saves the choice with a cleared model for a non-Claude agent, and closes", async () => {
    let config: Record<string, unknown> = { completedTimeoutMinutes: 30, agentModel: "sonnet" };
    invoke.mockImplementation((cmd: string, args?: { config?: Record<string, unknown> }) => {
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "list_agents") return Promise.resolve({ agents: [claude, codex], names: true });
      if (cmd === "set_config") {
        config = { ...args!.config };
        return Promise.resolve({ ...config });
      }
      return Promise.reject(new Error("unexpected " + cmd));
    });
    const shown = maybeShowFirstRun();
    await flush();
    expect(document.querySelector("h2")?.textContent).toBe("Which agent should power Maya?");
    await flush();
    document.querySelector<HTMLInputElement>("input[name=agent][value=codex]")!.click();
    document.querySelector<HTMLButtonElement>("button[data-action=continue]")!.click();
    await flush();
    await flush();
    await shown;
    expect(invoke).toHaveBeenCalledWith("set_config", { config: expect.objectContaining({ agent: "codex", agentModel: "" }) });
    expect(document.querySelector(".modal")).toBeNull();
  });
});
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `pnpm test -- firstrun`
Expected: FAIL, module not found.

- [ ] **Step 3: Implement `src/firstrun.ts`**

```ts
import { invoke } from "@tauri-apps/api/core";
import { HARNESS_ICON, harnessLabel } from "./harness";
import { CLAUDE_AGENT, type AgentInfo } from "./newsession";
import type { Harness } from "./types";

export interface FirstRunModel {
  /** The installed agents; null while they are being listed. */
  agents: AgentInfo[] | null;
  chosen: Harness;
  saving: boolean;
  error: string | null;
}

export interface FirstRunHandlers {
  onChoose(agent: Harness): void;
  onContinue(): void;
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const n = document.createElement(tag);
  n.className = className;
  if (text !== undefined) n.textContent = text;
  return n;
}

/** True when no agent has been chosen yet: a fresh install, or the first start after 0.10. */
export function needsFirstRun(config: { agent?: Harness | null }): boolean {
  return config.agent == null;
}

/** The first-start question. It has no close button: Continue is the only way out. */
export function renderFirstRun(m: FirstRunModel, h: FirstRunHandlers): HTMLElement {
  const root = el("div", "modal");
  root.append(el("div", "modal__backdrop"));
  const panel = el("section", "modal__panel modal__panel--compact firstrun");
  panel.setAttribute("role", "dialog");
  panel.setAttribute("aria-modal", "true");
  const head = el("header", "modal__head");
  const titles = el("div", "modal__titles");
  titles.append(el("h2", "modal__title", "Which agent should power Maya?"));
  head.append(titles);
  panel.append(head);
  const body = el("div", "modal__setup");
  if (m.agents === null) {
    body.append(el("p", "", "Looking for agents on this machine…"));
  } else if (m.agents.length === 0) {
    body.append(el("p", "", "None of the agents Maya can use is installed: Claude Code, Codex, Antigravity or Grok Build. Install one, then pick it in Settings."));
  } else {
    body.append(el("p", "", "It interprets your voice commands, picks folders for 'Let Maya choose', and is the default for new and resumed sessions and reviews. You can change it in Settings."));
    const list = el("div", "resume__list");
    for (const a of m.agents) {
      const row = el("label", "firstrun__row");
      const radio = document.createElement("input");
      radio.type = "radio";
      radio.name = "agent";
      radio.value = a.harness;
      radio.checked = a.harness === m.chosen;
      radio.addEventListener("change", () => h.onChoose(a.harness));
      row.append(radio);
      const icon = HARNESS_ICON[a.harness];
      if (icon) {
        const img = document.createElement("img");
        img.src = icon;
        img.alt = "";
        img.width = 20;
        img.height = 20;
        row.append(img);
      }
      row.append(el("span", "", harnessLabel(a.harness)));
      list.append(row);
    }
    body.append(list);
  }
  if (m.agents !== null) {
    const go = el("button", "card__btn card__btn--primary", m.agents.length === 0 ? "Continue with Claude Code" : "Continue");
    go.type = "button";
    go.dataset.action = "continue";
    go.disabled = m.saving;
    go.addEventListener("click", () => h.onContinue());
    body.append(go);
  }
  panel.append(body);
  if (m.error) panel.append(el("div", "modal__status modal__status--error", m.error));
  root.append(panel);
  return root;
}

interface ConfigLike {
  agent?: Harness | null;
  agentModel?: string;
  [key: string]: unknown;
}

/** On a start with no agent chosen, asks which one, saves it, and closes. */
export async function maybeShowFirstRun(): Promise<void> {
  let config: ConfigLike;
  try {
    config = await invoke<ConfigLike>("get_config");
  } catch {
    return;
  }
  if (!needsFirstRun(config)) return;
  const host = document.getElementById("modal-host");
  if (!host) return;
  const model: FirstRunModel = { agents: null, chosen: "claude-code", saving: false, error: null };
  await new Promise<void>((resolve) => {
    const paint = () => host.replaceChildren(renderFirstRun(model, handlers));
    const handlers: FirstRunHandlers = {
      onChoose: (agent) => {
        model.chosen = agent;
      },
      onContinue: () => {
        void (async () => {
          model.saving = true;
          model.error = null;
          paint();
          try {
            const fresh = await invoke<ConfigLike>("get_config");
            const agentModel = model.chosen === "claude-code" ? (fresh.agentModel ?? "") : "";
            await invoke("set_config", { config: { ...fresh, agent: model.chosen, agentModel } });
            host.replaceChildren();
            resolve();
          } catch (e) {
            model.saving = false;
            model.error = String(e);
            paint();
          }
        })();
      },
    };
    paint();
    void invoke<{ agents: AgentInfo[] }>("list_agents", { machine: "" })
      .then((r) => {
        model.agents = r.agents;
        model.chosen = r.agents[0]?.harness ?? "claude-code";
      })
      .catch(() => {
        model.agents = [CLAUDE_AGENT];
        model.chosen = "claude-code";
      })
      .then(paint);
  });
}
```

`src/main.ts` `start()`: after `void initDebug();` add `void maybeShowFirstRun();` with `import { maybeShowFirstRun } from "./firstrun";`.

`src/styles.css`: `.firstrun__row { display: flex; align-items: center; gap: 10px; padding: 8px 10px; border: 1px solid var(--border); border-radius: 8px; cursor: pointer; } .firstrun__row:has(input:checked) { border-color: var(--working); }`.

- [ ] **Step 4: Run the tests and build**

Run: `pnpm test && pnpm build`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/firstrun.ts src/firstrun.test.ts src/main.ts src/styles.css
git commit -m "feat: ask which agent powers Maya on the first start"
```

---

### Task 10: Resume's Agent field, New session's default, and the "Let Maya choose" copy

**Files:**
- Create: `src/brain.ts`, `src/brain.test.ts`
- Modify: `src/resume.ts`, `src/resume.test.ts`, `src/newsession.ts:227, 238, 347, 429-446, 469-480`, `src/newsession.test.ts`

**Interfaces:**
- Consumes: `get_config`, `list_agents`, `list_resumable_sessions(dir, machine, agent)`, `resume_session(dir, sessionId, machine, agent)`.
- Produces: `brainAgent(): Promise<Harness>`; `defaultAgent(agents: AgentInfo[], brain: Harness): Harness`; `ResumeModel.agents: AgentInfo[]`, `.agent: Harness`; `ResumeHandlers.onAgent(agent: Harness)`.

- [ ] **Step 1: Write the failing tests**

`src/brain.test.ts`:

```ts
import { describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { brainAgent, defaultAgent } from "./brain";

const info = (h: "claude-code" | "codex" | "grok") => ({ harness: h, models: [], efforts: [], modes: [] });

describe("brain", () => {
  it("reads Maya's agent from the config, Claude Code when unset or unreadable", async () => {
    invoke.mockResolvedValueOnce({ agent: "grok" });
    expect(await brainAgent()).toBe("grok");
    invoke.mockResolvedValueOnce({});
    expect(await brainAgent()).toBe("claude-code");
    invoke.mockRejectedValueOnce(new Error("no backend"));
    expect(await brainAgent()).toBe("claude-code");
  });

  it("defaults to the brain when the machine has it, else the first agent listed", () => {
    expect(defaultAgent([info("claude-code"), info("grok")], "grok")).toBe("grok");
    expect(defaultAgent([info("claude-code"), info("codex")], "grok")).toBe("claude-code");
    expect(defaultAgent([info("codex")], "grok")).toBe("codex");
    expect(defaultAgent([], "grok")).toBe("claude-code");
  });
});
```

`src/resume.test.ts`: `handlers()` gains `onAgent: vi.fn()`; `base` gains `agents: [{ harness: "claude-code", models: [], efforts: [], modes: [] }], agent: "claude-code"`. Add:

```ts
  it("offers an Agent field when the machine has more than one, defaulting to the given agent", () => {
    const h = handlers();
    const agents = [{ harness: "claude-code" as const, models: [], efforts: [], modes: [] }, { harness: "codex" as const, models: [], efforts: [], modes: [] }];
    expect(renderResume(base, h, NOW).querySelector("select[name=agent]")).toBeNull();
    const el = renderResume({ ...base, agents, agent: "codex" }, h, NOW);
    const select = el.querySelector<HTMLSelectElement>("select[name=agent]")!;
    expect([...select.options].map((o) => o.textContent)).toEqual(["Claude Code", "Codex"]);
    expect(select.value).toBe("codex");
    select.value = "claude-code";
    select.dispatchEvent(new Event("change"));
    expect(h.onAgent).toHaveBeenCalledWith("claude-code");
  });
```

and in the existing flow test(s) that check `invoke` calls for `list_resumable_sessions` and `resume_session`, expect the extra `agent: "claude-code"` argument (and mock `get_config` → `{ agent: null }` and `list_agents` → `{ agents: [], names: true }`).

`src/newsession.test.ts`: replace "Let Claude choose" with "Let Maya choose", "chosen by Claude" with "chosen by Maya", and any `lastAgent` expectation ("the last agent is offered again") with: after `get_config` resolves `{ agent: "codex" }` and `list_agents` lists Claude and Codex, the Agent select's value is `codex`.

- [ ] **Step 2: Run the tests to see them fail**

Run: `pnpm test -- brain resume newsession`
Expected: FAIL.

- [ ] **Step 3: Implement**

`src/brain.ts`:

```ts
import { invoke } from "@tauri-apps/api/core";
import type { AgentInfo } from "./newsession";
import type { Harness } from "./types";

/** The agent that powers Maya, from the config; Claude Code when unset or unreadable. */
export async function brainAgent(): Promise<Harness> {
  try {
    const c = await invoke<{ agent?: Harness | null }>("get_config");
    return c.agent ?? "claude-code";
  } catch {
    return "claude-code";
  }
}

/** The agent a modal starts on: Maya's when the machine has it, else the first listed. */
export function defaultAgent(agents: AgentInfo[], brain: Harness): Harness {
  if (agents.some((a) => a.harness === brain)) return brain;
  return agents[0]?.harness ?? "claude-code";
}
```

`src/resume.ts`:
- Imports: `CLAUDE_AGENT, type AgentInfo` from `./newsession`, `harnessLabel` from `./harness`, `brainAgent, defaultAgent` from `./brain`, `type Harness` from `./types`.
- `ResumeModel`: add `/** The chosen machine's agents, Claude Code first. */ agents: AgentInfo[]; agent: Harness;`. `ResumeHandlers`: add `onAgent(agent: Harness): void;`.
- In `renderResume`, after the Machine field and before `if (m.needsSetup)`:

```ts
  if (m.agents.length > 1) {
    const agentLabel = el("label", "newsession__field");
    agentLabel.append(el("span", "newsession__label", "Agent"));
    const agentSelect = el("select", "newsession__select");
    agentSelect.name = "agent";
    for (const a of m.agents) {
      const o = document.createElement("option");
      o.value = a.harness;
      o.textContent = harnessLabel(a.harness);
      agentSelect.append(o);
    }
    agentSelect.value = m.agents.some((a) => a.harness === m.agent) ? m.agent : m.agents[0].harness;
    agentSelect.addEventListener("change", () => h.onAgent(agentSelect.value as Harness));
    agentLabel.append(agentSelect);
    form.append(agentLabel);
  }
```

- `paint()` handlers: `onAgent: (agent) => void chooseAgent(agent),`.
- New functions:

```ts
/** Lists the machine's agents and settles on Maya's, then refreshes the sessions if a folder is chosen. */
async function loadAgents(machine: string): Promise<void> {
  const me = current;
  if (!me) return;
  let agents: AgentInfo[];
  try {
    const reply = await invoke<{ agents: AgentInfo[]; names: boolean }>("list_agents", { machine });
    // An older Maya over there resumes Claude Code only.
    agents = reply.names && reply.agents.length > 0 ? reply.agents : [CLAUDE_AGENT];
  } catch {
    agents = [CLAUDE_AGENT];
  }
  const brain = await brainAgent();
  if (current !== me || me.model.machine !== machine) return;
  me.model.agents = agents;
  me.model.agent = defaultAgent(agents, brain);
  paint();
  if (me.model.dir) await loadSessions(me.model.dir);
}

async function chooseAgent(agent: Harness): Promise<void> {
  if (!current) return;
  current.model.agent = agent;
  current.model.sessions = [];
  paint();
  if (current.model.dir) await loadSessions(current.model.dir);
}
```

- `chooseMachine`: reset `m.agents = [CLAUDE_AGENT]; m.agent = "claude-code";` and `void loadAgents(machine);` before `await loadDirs(machine)`.
- `loadSessions`: `invoke("list_resumable_sessions", { dir, machine, agent: me.model.agent })`, and drop a late answer when `me.model.agent` changed too. `resume()`: `invoke("resume_session", { dir, sessionId, machine, agent: me.model.agent })`.
- `openResume`: initial model gains `agents: [CLAUDE_AGENT], agent: "claude-code"`; after `paint()`, `void loadAgents("");`.

`src/newsession.ts`:
- Delete `let lastAgent: Harness = "claude-code";` (`:429`) and every assignment to it (grep `lastAgent`). In `loadAgents`, replace the `wanted` line with `const wanted = defaultAgent(me.model.agents, await brainAgent());` (re-check `current !== me || me.model.machine !== machine` after the await; import `brainAgent, defaultAgent` from `./brain`).
- Copy: `auto.textContent = "Let Maya choose";` (`:227`); hint `"Maya can't choose for you on a remote machine; pick a folder."` (`:238`); `startedText` `"(chosen by Maya)"` (`:347`); the comment at `:220` "Maya cannot choose on a remote machine".

- [ ] **Step 4: Run the tests and build**

Run: `pnpm test && pnpm build`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/brain.ts src/brain.test.ts src/resume.ts src/resume.test.ts src/newsession.ts src/newsession.test.ts
git commit -m "feat: an Agent field on Resume, and Maya's agent as the default for new and resumed sessions"
```

---

### Task 11: Docs, real-app check, version bump and pull request

**Files:**
- Modify: `README.md:23-30` (requirements), `docs/superpowers/specs/2026-10-03-maya-agent-design.md` (Status: implemented; the capability table as probed)
- Modify: `src-tauri/tauri.conf.json`, `package.json`, `Cargo.toml`, `Cargo.lock` via `scripts/set-version.sh`

- [ ] **Step 1: README**

In `README.md` Requirements, replace the Claude Code bullet with:

```markdown
- One of [Claude Code](https://claude.com/claude-code), [Codex](https://github.com/openai/codex), Antigravity or Grok Build, installed and signed in. Maya asks which one powers her on the first start; the others show on the board when they are installed.
```

In "What it does", change "Does what you tell it by voice" to "Does what you tell her by voice, through the agent you choose", and add "- Resumes old sessions of any of them, and reviews pull requests with the agent you chose, on a prompt you can edit in Settings". Leave the per-platform install snippets as they are (they install Claude Code as the common case).

- [ ] **Step 2: Full verification**

Run, from the repo root:

```bash
cargo test --workspace
pnpm test
pnpm build
```

Expected: all PASS. Then the real-app check with the user's go-ahead (see memory `maya-run-and-verify`: vite plus the debug binary, screenshot before each click): delete `agent` from `~/.claude/maya/config.json` (back it up first), start Maya, confirm the first-start modal lists the installed agents, pick Codex, Continue; in Settings confirm the Maya section shows Codex with its models; say "Maya, what's waiting on me?" and confirm she answers (the log tab shows `asking codex`); New session shows Codex preselected; Resume shows the Agent field and lists a Codex session in a folder that has one; the Review button on a listed PR opens Codex with the default prompt; switch back to Claude Code in Settings and restore the config backup. Record what was seen in the PR description.

- [ ] **Step 3: Spec status**

Set `Status: implemented` in the spec and make sure its capability table matches `launch::capabilities` and `CAPABILITIES`.

```bash
git add README.md docs/superpowers/specs/2026-10-03-maya-agent-design.md
git commit -m "docs: README and spec for choosing the agent that powers Maya"
```

- [ ] **Step 4: Version bump**

```bash
git fetch origin main
git show origin/main:package.json | grep '"version"'
sh scripts/set-version.sh 0.10.0   # the minor above main's version
git add src-tauri/tauri.conf.json package.json Cargo.toml Cargo.lock
git commit -m "chore(release): 0.10.0"
```

- [ ] **Step 5: Pull request (ask the user before pushing)**

Push `feat/maya-agent` and open the PR with `gh pr create`, titled `feat: choose the agent that powers Maya`, with a body that lists: the Maya section and first-start modal; interpreter and classifier on the chosen agent; resume for every agent; capability-gated controls (and which cells the probe confirmed or left at the safe default); the review prompt; the migration from `interpreterModel`; what the real-app check showed. End the body with the attribution lines from the session reminder.
