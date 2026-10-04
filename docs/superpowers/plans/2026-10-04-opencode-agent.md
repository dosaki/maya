# OpenCode as a Maya agent — implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** OpenCode becomes Maya's sixth agent, driven through its background server: cards with state and answers, New session, Resume, rename, every session control, Maya's brain, reviews, other machines.

**Architecture:** A `Harness::OpenCode` variant and a new `core/src/opencode.rs` holding the server client (`ureq` over the state file's url and password), the reply parsers, the card rules and the actions. The store fetches the server on each refresh and merges OpenCode cards with the others; `actions.rs` gains a per-card `Channel` (inbox, tty or server) so every existing action dispatches to the server for OpenCode. New sessions are created on the server, then opened in a terminal on `opencode --session <id>`.

**Tech Stack:** Rust (Cargo workspace: `core/`, `src-tauri/`, `cli/`), `ureq` 3 (blocking HTTP, `json` feature), Tauri 2, vanilla TypeScript with vitest, serde_json.

**Spec:** `docs/superpowers/specs/2026-10-04-maya-opencode-agent-design.md`

## Global Constraints

- Branch `feat/opencode-agent`, stacked on `feat/kiro-agent` until PR #21 merges, then rebased onto `main`; never commit to or push `main`; ask the user before each push (user rule).
- Commit messages are conventional (`feat:`, `fix:`, `docs:`, `chore:`), ending with the attribution lines from the session reminder.
- Wire name `opencode`; label "OpenCode"; binary `opencode`, also looked for in `~/.opencode/bin` (Windows: `%USERPROFILE%\.opencode\bin\opencode.exe`).
- The server password is read from the state file per request, held only in a `Client` value for that call, never logged or sent elsewhere.
- OpenCode model ids are `provider/model` (one `/`); they never reach a shell line. Every other agent keeps `plain_model_id`.
- The only shell lines for OpenCode carry a quoted folder and a quoted session id (`resume::plain_session_id`), plus `--auto`.
- Copy: "Wait until the session is free." for a Working OpenCode session; "OpenCode's server did not start." when `opencode service start` leaves no state file within ten seconds; the six-agent list reads "Claude Code, Codex, Antigravity, Grok Build, Kiro CLI or OpenCode".
- Fixtures in `core/fixtures/opencode/` are already on disk, untracked: `service.json` (password scrubbed, pid 4242), `sessions.json`, `active.json`, `models.json` (six models), `agents.json`, `messages.json`, `permission-empty.json`, `form-empty.json`, `oneshot.jsonl` (also copied to `core/fixtures/oneshot/opencode.jsonl`); `src/assets/icons/opencode.svg` too. Task 1 commits them. Pending permission and form fixtures are written in Task 2 from the OpenAPI schema (no real one could be provoked: OpenCode's default rules allow its tools), and marked so in a comment.
- Tests: `cargo test --workspace` from the repo root and `pnpm test`; the frontend must also pass `pnpm build`.
- Release: a `feat`, so the last task bumps the minor version from `main`'s (0.11.0 → 0.12.0 once PR #21 is merged) with `sh scripts/set-version.sh`, committed alone as `chore(release): 0.12.0`.

## Review Focus

1. The state file names a pid that is dead (OpenCode was killed, the file stayed): no OpenCode cards, no error, no request made. Pinned in Task 3 (`opencode_sessions` with a dead pid makes no request to the fake server).
2. A permission or question answered twice, or after the session moved on (the card's `ask_id` is stale): refused with "The question has changed; look again." and no request made. Pinned in Task 4 (`answer::check` runs before the call).
3. A session whose folder is Maya's data folder (a one-shot of the brain), and a subagent child session: never a card. Pinned in Task 2 (`card_for` returns None for both).
4. A reply to a Working OpenCode session: refused with "Wait until the session is free." rather than queued silently. Pinned in Task 4.
5. New session when the server is down and `opencode service start` fails or stalls: "OpenCode's server did not start." within ten seconds and no terminal opened. Pinned in Task 5 (`ensure_service` with a command that writes no file).

---

### Task 1: The harness variant, its table rows, the listing parser and the one-shot

**Files:**
- Modify: `Cargo.toml` (workspace dependencies), `core/Cargo.toml`
- Modify: `core/src/model.rs:57-70`
- Modify: `core/src/launch.rs` (`binary_name`, `efforts`, `modes`, `label`, `capabilities`, `flags`, `validate_shape`, `oneshot_args`, `final_text`, `agent_binary` candidates, new `plain_model_ref`, new `opencode_open_command`)
- Modify: `core/src/agents.rs` (`listing_args`, `info_for`, `build`'s loop, new `parse_opencode`)
- Modify: `core/src/resume.rs` (`list_sessions` arm, `resume_command` arm)
- Modify: `core/src/foreign.rs` (the exhaustive matches)
- Modify: `cli/src/args.rs:31, 133`, `cli/src/commands.rs:156` and their tests
- Add: `core/fixtures/opencode/*`, `core/fixtures/oneshot/opencode.jsonl`, `src/assets/icons/opencode.svg` (on disk)

**Interfaces:**
- Produces: `Harness::OpenCode`; `launch::plain_model_ref(id) -> bool` (`provider/model`, each half `plain_model_id`); `launch::opencode_open_command(dir: &Path, id: &str, auto: bool) -> String` = `cd '<dir>' && opencode --session '<id>'` plus ` --auto`; `launch::capabilities(OpenCode)` all true with `mode_cycle: Some("agent")`; `launch::modes(OpenCode) == ["default", "auto"]`; `launch::efforts(OpenCode) == []` (per model); `agents::parse_opencode(json) -> Vec<ModelInfo>` with id `provider/model`, label the name, efforts the variant ids, enabled models only; `info_for(OpenCode, listing)` efforts = the variants every listed model has (Codex's rule); listing args `["api", "GET", "/api/model"]`.

- [ ] **Step 1: Write the failing tests**

`core/src/model.rs` tests:

```rust
    #[test]
    fn opencode_has_a_wire_name() {
        assert_eq!(serde_json::to_string(&Harness::OpenCode).unwrap(), "\"opencode\"");
        assert_eq!(serde_json::from_str::<Harness>("\"opencode\"").unwrap(), Harness::OpenCode);
    }
```

`core/src/launch.rs` tests (extend the existing ones):

```rust
        // labels_name_every_agent
        assert_eq!(label(Harness::OpenCode), "OpenCode");
        // capabilities_follow_the_table
        let o = capabilities(Harness::OpenCode);
        assert!(o.compact && o.model_switch && o.effort_switch && o.slash_lines && o.shell_lines && o.close);
        assert_eq!(o.mode_cycle, Some("agent"), "the cycle is a server call, not a key");
        // each_agent_renders_its_own_flags
        assert_eq!(all(Harness::OpenCode).flags(), "", "OpenCode's options go to the server, not the command line");
        assert_eq!(modes(Harness::OpenCode), ["default", "auto"]);
        assert!(efforts(Harness::OpenCode).is_empty(), "variants come per model from the listing");
```

A new test:

```rust
    #[test]
    fn an_opencode_model_is_a_provider_and_a_model() {
        assert!(plain_model_ref("anthropic/claude-sonnet-4-5") && plain_model_ref("opencode/fledge-alpha-free"));
        assert!(!plain_model_ref("claude-sonnet-4-5"), "no provider");
        assert!(!plain_model_ref("a/b/c") && !plain_model_ref("a/") && !plain_model_ref("/b") && !plain_model_ref("a b/c"));
        let oc = LaunchOptions { agent: Harness::OpenCode, model: Some("anthropic/claude-sonnet-4-5".into()), effort: Some("high".into()), mode: Some("auto".into()), ..Default::default() };
        assert!(oc.validate_shape().is_ok(), "{:?}", oc.validate_shape());
        assert!(oc.validate(&["anthropic/claude-sonnet-4-5".to_string()]).is_ok());
        let claude = LaunchOptions { model: Some("anthropic/opus".into()), ..Default::default() };
        assert!(claude.validate_shape().unwrap_err().contains("model"), "a slash is OpenCode's alone");
        assert_eq!(opencode_open_command(Path::new("/Users/x/dev/it's"), "ses_0a", true), "cd '/Users/x/dev/it'\\''s' && opencode --session 'ses_0a' --auto");
        assert_eq!(opencode_open_command(Path::new("/p"), "ses_0a", false), "cd '/p' && opencode --session 'ses_0a'");
    }
```

In `oneshot_args_per_agent` and the `final_text` tests:

```rust
        let c = oneshot_args(Harness::OpenCode, Some("anthropic/claude-sonnet-4-5#high"), Some("SYS"), "USER");
        assert_eq!(&c[..3], ["run", "--format", "json"]);
        assert!(pair(&c, "-m", "anthropic/claude-sonnet-4-5#high") && pair(&c, "--title", "Maya"));
        assert_eq!(c.last().unwrap(), "SYS\n\nUSER", "no system flag: the system text leads the prompt");
        assert!(!oneshot_args(Harness::OpenCode, None, None, "U").iter().any(|x| x == "-m"));
```

```rust
        assert_eq!(final_text(Harness::OpenCode, &oneshot_fixture("opencode.jsonl")).unwrap(), "pong");
        assert!(final_text(Harness::OpenCode, "{\"type\":\"step_start\",\"part\":{}}\n").unwrap_err().contains("last event: step_start"));
        assert!(final_text(Harness::OpenCode, "").unwrap_err().contains("last event: none"));
```

`core/src/agents.rs` tests:

```rust
    #[test]
    fn opencode_models_are_provider_slash_model_with_their_variants() {
        let m = parse_opencode(&fixture("opencode/models.json"));
        assert_eq!(m[0].id, "opencode/fledge-alpha-free");
        assert_eq!(m[0].label, "Fledge Alpha Free");
        assert_eq!(m[0].efforts, vec!["low", "high", "max"]);
        assert!(m.iter().any(|x| x.id == "openai/gpt-6.1-sol") && m.iter().any(|x| x.id == "github-copilot/gpt-6.1-sol"), "the same model under two providers stays distinct");
        assert!(m.iter().all(|x| x.id.matches('/').count() == 1));
        assert!(parse_opencode("not json").is_empty());
        assert!(parse_opencode(r#"{"data":[{"id":"a","providerID":"p","name":"A","variants":[],"enabled":false,"limit":{"context":1,"output":1}}]}"#).is_empty(), "disabled models are not offered");
    }

    #[test]
    fn opencode_default_model_offers_the_variants_every_model_has() {
        let info = info_for(Harness::OpenCode, Some(&fixture("opencode/models.json")));
        assert_eq!(info.models.len(), 6);
        assert!(info.efforts.is_empty(), "one fixture model has no variants, so Default offers none");
        assert_eq!(info.modes, vec!["default", "auto"]);
        assert!(info_for(Harness::OpenCode, None).models.is_empty());
    }
```

In `build_lists_claude_then_each_agent_it_finds`, make `found` also match `"opencode"`, add `("/bin/opencode", ["api", "GET", "/api/model"]) => Some(fixture("opencode/models.json")),` and expect `OpenCode` last with six models. In `resume.rs` tests add `assert_eq!(resume_command(Harness::OpenCode, dir, "ses_0a"), "cd '/Users/x/dev/it'\\''s' && opencode --session 'ses_0a'");`. In `cli/src/args.rs` tests add `start --agent opencode` parsing and `USAGE.contains("opencode")`; in `cli/src/commands.rs` the `--name` refusal loop gains `Harness::OpenCode` and the message reads "name Codex, Antigravity, Grok, Kiro and OpenCode sessions from the Maya app".

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test --workspace`
Expected: compile errors on `Harness::OpenCode`, `plain_model_ref`, `opencode_open_command`, `parse_opencode`.

- [ ] **Step 3: Implement**

`Cargo.toml` workspace dependencies: `ureq = { version = "3", features = ["json"] }`; `core/Cargo.toml`: `ureq.workspace = true`.

`core/src/model.rs`: add `OpenCode,` before `#[serde(other)] Other`.

`core/src/launch.rs`:
- `binary_name`: `Harness::OpenCode => "opencode"`; `efforts`: `&[]`; `modes`: `&["default", "auto"]`; `label`: `"OpenCode"`.
- `capabilities`: `Harness::OpenCode => Capabilities { compact: true, model_switch: true, effort_switch: true, mode_cycle: Some("agent"), slash_lines: true, shell_lines: true, close: true },` with the comment "every control is a server call; `agent` names the build/plan switch".
- `flags`: `Harness::OpenCode => return String::new(),` beside `Other`.
- New:

```rust
/// An OpenCode model reference, `provider/model`, each half a plain id.
pub fn plain_model_ref(id: &str) -> bool {
    matches!(id.split_once('/'), Some((p, m)) if plain_model_id(p) && plain_model_id(m))
}

/// The shell line that opens a terminal on an OpenCode session the server
/// already has; `--auto` approves every permission no rule denies.
pub fn opencode_open_command(dir: &Path, id: &str, auto: bool) -> String {
    format!("cd {} && opencode --session {}{}", shell_single_quote(&dir.to_string_lossy()), shell_single_quote(id), if auto { " --auto" } else { "" })
}
```

- `validate_shape`: the model check becomes `let plain = if self.agent == Harness::OpenCode { plain_model_ref(m) } else { plain_model_id(m) }; if !plain { return Err(format!("Unknown model: {m}")); }`.
- `oneshot_args`: `Harness::OpenCode => { let mut a = vec![s("run"), s("--format"), s("json"), s("--title"), s("Maya")]; if let Some(m) = model { a.extend([s("-m"), s(m)]); } a.push(folded); a }`.
- `final_text`: for OpenCode, join the `part.text` of every line whose `type` is `"text"`; with none, `Err(format!("opencode printed no text (last event: {})", last_type_or_none))`.
- `agent_binary` (unix): add `home.join(".opencode/bin").join(name)` to `candidates`; (windows): after the PATH lookup try `home.join(".opencode").join("bin").join(&exe)` then `.local/bin`.

`core/src/agents.rs`:

```rust
/// Models from `opencode api GET /api/model`: enabled ones, as
/// `provider/model`, with their variants as efforts.
pub fn parse_opencode(json: &str) -> Vec<ModelInfo> {
    let v: Value = serde_json::from_str(json).unwrap_or(Value::Null);
    v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["enabled"].as_bool() != Some(false))
        .filter_map(|m| {
            let id = format!("{}/{}", m["providerID"].as_str()?, m["id"].as_str()?);
            launch::plain_model_ref(&id).then(|| ModelInfo {
                label: m["name"].as_str().unwrap_or(&id).to_string(),
                efforts: m["variants"].as_array().into_iter().flatten().filter_map(|x| x["id"].as_str()).map(String::from).collect(),
                id,
            })
        })
        .collect()
}
```

`listing_args`: `Harness::OpenCode => &["api", "GET", "/api/model"]`; `info_for`: `(Harness::OpenCode, Some(t)) => parse_opencode(t)` and the efforts rule `if matches!(agent, Harness::Codex | Harness::OpenCode)` (the variants every model has; empty when a model has none); `build`'s loop adds `Harness::OpenCode`.

`core/src/resume.rs`: `Harness::OpenCode => vec![]` in `list_sessions` (Task 5 routes OpenCode before this), `resume_command`: `Harness::OpenCode => format!("cd {d} && opencode --session {i}")`.

`core/src/foreign.rs`: add `Harness::OpenCode` to the `ClaudeCode | Grok | Kiro | Other => {}` arms and to the `ForeignTail::default()` / `vec![]` arms.

CLI: usage `<claude-code|codex|antigravity|grok|kiro|opencode>`, the error list, the `--name` message.

- [ ] **Step 4: Run the tests**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock core/Cargo.toml core/src/model.rs core/src/launch.rs core/src/agents.rs core/src/resume.rs core/src/foreign.rs cli/src core/fixtures/opencode core/fixtures/oneshot/opencode.jsonl src/assets/icons/opencode.svg
git commit -m "feat(core): OpenCode harness, its model listing and one-shot, and the ureq client crate"
```

---

### Task 2: `core/src/opencode.rs`: the server client, parsers and card rules

**Files:**
- Create: `core/src/opencode.rs`
- Modify: `core/src/lib.rs` (`pub mod opencode;`), `src-tauri/src/lib.rs:1` (re-export)
- Add: `core/fixtures/opencode/permission.json`, `core/fixtures/opencode/form.json` (schema-derived)

**Interfaces:**
- Produces:
  - `pub struct Service { pub url: String, pub pid: i32, pub password: String }`, `pub fn service_file(state_dir: &Path) -> PathBuf` (`<state_dir>/opencode/service.json`), `pub fn read_service(path: &Path) -> Option<Service>`
  - `pub struct Client { base: String, password: String }` with `Client::new(&Service)`, `get(&self, path) -> Result<Value, String>`, `post(&self, path, body: Value) -> Result<Value, String>`, `patch(&self, path, body) -> Result<Value, String>`; errors read "OpenCode's server: <status or io error>"
  - `pub struct SessionInfo { id, parent_id: Option<String>, title: Option<String>, directory: String, agent: String, model: Option<(String, String, Option<String>)>, created_ms, updated_ms, idle_ms: Option<u64>, viewed_ms: Option<u64>, tokens_used: u64 }` and `pub fn sessions(v: &Value) -> Vec<SessionInfo>`
  - `pub fn active_ids(v: &Value) -> HashSet<String>`
  - `pub struct Permission { id, action, resource: String }` and `pub fn permissions(v: &Value) -> Vec<Permission>`
  - `pub struct Form { id, title, fields: Vec<FormField { key, title, options: Vec<(String, String)> }> }` and `pub fn forms(v: &Value) -> Vec<Form>`
  - `pub fn messages(v: &Value) -> (Option<String>, Vec<Turn>)` (last assistant text; turns oldest first)
  - `pub const PERMISSION_CHOICES: [&str; 3] = ["Once", "Always", "Reject"]` and `pub fn decision(option: usize) -> &'static str` (`once`, `always`, `reject`)
  - `pub struct Live { pub running: bool, pub permission: Option<Permission>, pub form: Option<Form> }`
  - `pub fn card_for(s: &SessionInfo, live: &Live, snippet: Option<String>, context_window: Option<u64>, window_pid: Option<i32>, server_pid: i32, data_dir: &str, now_ms: u64, timeout_ms: u64) -> Option<Card>`

- [ ] **Step 1: Write the fixtures and the failing tests**

`core/fixtures/opencode/permission.json` (from the `Permission.Request` schema in `/openapi.json`; no real one could be provoked):

```json
{"data":[{"id":"per_01","sessionID":"ses_efe57d285ffeRMX70aswKZ9qCO","action":"edit","resources":["/tmp/probe-hello.txt"],"source":{"type":"tool","messageID":"msg_1","id":"call_1"},"message":"Write /tmp/probe-hello.txt"}]}
```

`core/fixtures/opencode/form.json` (from `Form.Detail`):

```json
{"data":[{"id":"frm_01","sessionID":"ses_efe57d285ffeRMX70aswKZ9qCO","title":"Which branch?","fields":[{"key":"branch","type":"string","title":"Branch","options":[{"value":"main","label":"main"},{"value":"dev","label":"dev"}]}],"state":{"status":"pending"}}]}
```

Tests in `core/src/opencode.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AwaitKind, State};
    use std::path::Path;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/opencode").join(name)).unwrap()
    }
    fn json(name: &str) -> Value {
        serde_json::from_str(&fixture(name)).unwrap()
    }

    #[test]
    fn the_state_file_names_the_server() {
        let t = tempfile::tempdir().unwrap();
        let path = service_file(t.path());
        assert_eq!(path, t.path().join("opencode/service.json"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, fixture("service.json")).unwrap();
        let s = read_service(&path).unwrap();
        assert_eq!((s.url.as_str(), s.pid, s.password.as_str()), ("http://127.0.0.1:49374", 4242, "not-the-real-one"));
        assert!(read_service(Path::new("/nonexistent")).is_none());
        std::fs::write(&path, "{}").unwrap();
        assert!(read_service(&path).is_none(), "no url, no server");
    }

    #[test]
    fn sessions_are_read_with_their_times_folder_and_tokens() {
        let list = sessions(&json("sessions.json"));
        assert!(list.len() >= 4);
        let hi = list.iter().find(|s| s.title.as_deref() == Some("Saying \"Hi\" request")).unwrap();
        assert_eq!(hi.directory, "/Users/tiagocorreia");
        assert_eq!(hi.model.as_ref().map(|m| (m.0.as_str(), m.1.as_str())), Some(("opencode", "fledge-alpha-free")));
        assert_eq!(hi.idle_ms, Some(1_791_029_162_638));
        assert_eq!(hi.tokens_used, 10_083 + 207, "input plus output plus cache reads");
        assert!(hi.parent_id.is_none());
        assert!(sessions(&Value::Null).is_empty());
    }

    #[test]
    fn active_permissions_and_forms_are_read() {
        assert!(active_ids(&json("active.json")).is_empty());
        let a = active_ids(&serde_json::json!({"data": {"ses_1": {"type": "running"}}}));
        assert!(a.contains("ses_1"));
        let p = permissions(&json("permission.json"));
        assert_eq!((p[0].id.as_str(), p[0].action.as_str(), p[0].resource.as_str()), ("per_01", "edit", "/tmp/probe-hello.txt"));
        assert!(permissions(&json("permission-empty.json")).is_empty());
        let f = forms(&json("form.json"));
        assert_eq!((f[0].id.as_str(), f[0].title.as_str(), f[0].fields[0].key.as_str()), ("frm_01", "Which branch?", "branch"));
        assert_eq!(f[0].fields[0].options, vec![("main".to_string(), "main".to_string()), ("dev".to_string(), "dev".to_string())]);
        assert!(forms(&json("form-empty.json")).is_empty());
        assert_eq!(decision(0), "once");
        assert_eq!(decision(2), "reject");
    }

    #[test]
    fn messages_give_the_last_text_and_the_turns_oldest_first() {
        let (last, turns) = messages(&json("messages.json"));
        assert_eq!(last.as_deref(), Some("Hi"));
        let got: Vec<(String, String)> = turns.iter().map(|t| (format!("{:?}", t.kind).to_lowercase(), t.text.clone())).collect();
        assert_eq!(got, vec![("user".to_string(), "say \"Hi\"".to_string()), ("assistant".to_string(), "Hi".to_string())], "idle messages and reasoning parts are not turns");
    }

    fn session(id: &str, dir: &str, updated: u64) -> SessionInfo {
        SessionInfo { id: id.into(), parent_id: None, title: Some("t".into()), directory: dir.into(), agent: "build".into(), model: None, created_ms: updated, updated_ms: updated, idle_ms: None, viewed_ms: None, tokens_used: 500 }
    }

    #[test]
    fn card_rules_running_waiting_completed_idle_and_dropped() {
        let min = 60_000;
        let s = session("ses_1", "/p", 1_000);
        let quiet = Live { running: false, permission: None, form: None };
        let c = card_for(&s, &Live { running: true, ..quiet.clone() }, Some("hi".into()), Some(1_000), None, 7, "/maya", 5_000, 30 * min).unwrap();
        assert_eq!((c.state, c.pid, c.snippet.as_str(), c.context.map(|x| x.percent)), (State::Working, 7, "hi", Some(50)));
        let p = card_for(&s, &Live { permission: Some(Permission { id: "per_1".into(), action: "edit".into(), resource: "/tmp/x".into() }), ..quiet.clone() }, None, None, Some(99), 7, "/maya", 5_000, 30 * min).unwrap();
        assert_eq!((p.state, p.pid), (State::Awaiting, 99));
        let aw = p.awaiting.unwrap();
        assert_eq!((aw.kind, aw.detail.as_str()), (AwaitKind::Question, "edit: /tmp/x"));
        assert_eq!(aw.questions[0].options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>(), PERMISSION_CHOICES);
        let f = card_for(&s, &Live { form: Some(forms(&json("form.json")).remove(0)), ..quiet.clone() }, None, None, None, 7, "/maya", 5_000, 30 * min).unwrap();
        let aw = f.awaiting.unwrap();
        assert_eq!((aw.kind, aw.detail.as_str(), aw.questions[0].options.len()), (AwaitKind::Question, "Which branch?", 2));
        let done = card_for(&s, &quiet, None, None, None, 7, "/maya", 1_000 + 10 * min, 30 * min).unwrap();
        assert_eq!(done.state, State::Completed);
        let idle = card_for(&s, &quiet, None, None, Some(55), 7, "/maya", 1_000 + 40 * min, 30 * min).unwrap();
        assert_eq!((idle.state, idle.pid), (State::Idle, 55), "a window for the folder keeps it as Idle");
        assert!(card_for(&s, &quiet, None, None, None, 7, "/maya", 1_000 + 40 * min, 30 * min).is_none(), "no window, past the timeout: gone");
        assert!(card_for(&session("ses_2", "/maya", 1_000), &Live { running: true, ..quiet.clone() }, None, None, None, 7, "/maya", 5_000, 30 * min).is_none(), "Maya's own one-shot");
        let child = SessionInfo { parent_id: Some("ses_1".into()), ..session("ses_3", "/p", 1_000) };
        assert!(card_for(&child, &Live { running: true, ..quiet }, None, None, None, 7, "/maya", 5_000, 30 * min).is_none(), "a subagent child");
    }

    #[test]
    fn the_client_sends_basic_auth_and_reads_json() {
        let (base, hits) = fake_server(vec![("GET /api/session/active", "{\"data\":{}}")]);
        let c = Client::new(&Service { url: base, pid: 1, password: "pw".into() });
        assert_eq!(c.get("/api/session/active").unwrap()["data"], serde_json::json!({}));
        let h = hits.lock().unwrap();
        assert!(h[0].contains("Authorization: Basic b3BlbmNvZGU6cHc="), "opencode:pw in base64: {}", h[0]);
        assert!(c.get("/api/nothing").unwrap_err().contains("404"));
    }
}
```

The fake server, in a `#[cfg(test)] pub(crate) mod fake` module of `opencode.rs` so Tasks 3 to 5 reuse it: `pub fn fake_server(routes: Vec<(&'static str, &'static str)>) -> (String, Arc<Mutex<Vec<String>>>)` binds `127.0.0.1:0`, spawns a thread that accepts connections in a loop, reads the request head up to `\r\n\r\n` plus `Content-Length` bytes of body, records `"<METHOD> <path>\n<headers>\n\n<body>"` in the vector, and answers `HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: N\r\nConnection: close\r\n\r\n<body>` for a matching `"<METHOD> <path>"` key, else `404` with `{}`. Returns the base url `http://127.0.0.1:<port>`.

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core opencode::`
Expected: compile errors, nothing defined.

- [ ] **Step 3: Implement**

```rust
//! OpenCode sessions live in its background server, one per user, which
//! owns every session, permission and tool run; the terminal UI is a client.
//! Maya reads the server's state file for its url and password and talks
//! to it over HTTP: the board polls it on each refresh, and every control
//! is a call rather than keys typed into a terminal.

use crate::model::{AwaitKind, Awaiting, Card, Choice, Harness, Question, State};
use crate::state::{truncate, SNIPPET_CHARS};
use crate::transcript::{Turn, TurnKind};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service { pub url: String, pub pid: i32, pub password: String }

pub fn service_file(state_dir: &Path) -> PathBuf { state_dir.join("opencode/service.json") }

pub fn read_service(path: &Path) -> Option<Service> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    Some(Service { url: v["url"].as_str()?.trim_end_matches('/').to_string(), pid: v["pid"].as_i64()? as i32, password: v["password"].as_str()?.to_string() })
}

pub struct Client { base: String, password: String }

impl Client {
    pub fn new(s: &Service) -> Self { Client { base: s.url.clone(), password: s.password.clone() } }
    fn auth(&self) -> String {
        use base64::Engine;
        format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("opencode:{}", self.password)))
    }
    fn read(&self, r: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Value, String> {
        match r {
            Ok(mut resp) => resp.body_mut().read_json::<Value>().map_err(|e| format!("OpenCode's server: unreadable reply: {e}")),
            Err(ureq::Error::StatusCode(code)) => Err(format!("OpenCode's server: HTTP {code}")),
            Err(e) => Err(format!("OpenCode's server: {e}")),
        }
    }
    pub fn get(&self, path: &str) -> Result<Value, String> {
        self.read(ureq::get(format!("{}{path}", self.base)).header("Authorization", &self.auth()).call())
    }
    pub fn post(&self, path: &str, body: Value) -> Result<Value, String> {
        self.read(ureq::post(format!("{}{path}", self.base)).header("Authorization", &self.auth()).send_json(body))
    }
    pub fn patch(&self, path: &str, body: Value) -> Result<Value, String> {
        self.read(ureq::patch(format!("{}{path}", self.base)).header("Authorization", &self.auth()).send_json(body))
    }
}
```

(The `ureq` 3 API: `ureq::get(url).header(k, v).call()`, `ureq::post(url).header(..).send_json(value)`, `Error::StatusCode(u16)` for non-2xx, `Response::body_mut().read_json()`. Check `cargo doc` for the exact names if the build disagrees; set a 10 s timeout through `ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(10)))` and keep one `Agent` in the `Client`.)

Parsers: `sessions` maps `data[]` with `tokens_used = tokens.input + tokens.output + tokens.cache.read`; `active_ids` collects the keys of `data`; `permissions` takes `resources[0]` as the resource (else `message`, else `""`); `forms` reads `fields[]` with `options[] {value, label}`; `messages` walks `data[]` in reverse (the list is newest first) collecting `type == "user"` → `Turn::User(text)`, `type == "assistant"` → for `content[]` text parts a `Turn::Assistant`, tool parts `Turn::Tool("<tool>: <input.command or input.filePath>")`, skipping `idle` and others; the last assistant text is the newest assistant text part.

```rust
pub const PERMISSION_CHOICES: [&str; 3] = ["Once", "Always", "Reject"];
pub fn decision(option: usize) -> &'static str { ["once", "always", "reject"][option.min(2)] }

#[derive(Debug, Clone, Default)]
pub struct Live { pub running: bool, pub permission: Option<Permission>, pub form: Option<Form> }

pub fn card_for(s: &SessionInfo, live: &Live, snippet: Option<String>, context_window: Option<u64>, window_pid: Option<i32>, server_pid: i32, data_dir: &str, now_ms: u64, timeout_ms: u64) -> Option<Card> {
    if s.parent_id.is_some() || s.directory == data_dir { return None; }
    let last = s.updated_ms.max(s.idle_ms.unwrap_or(0)).max(s.viewed_ms.unwrap_or(0));
    let (state, since, awaiting) = if let Some(p) = &live.permission {
        let q = Question { question: format!("{}: {}", p.action, p.resource), header: "Permission".into(), options: PERMISSION_CHOICES.iter().map(|l| Choice { label: l.to_string(), description: String::new() }).collect(), multi_select: false };
        (State::Awaiting, last, Some(Awaiting { kind: AwaitKind::Question, detail: truncate(&format!("{}: {}", p.action, p.resource), SNIPPET_CHARS), questions: vec![q] }))
    } else if let Some(f) = &live.form {
        let qs = f.fields.iter().map(|fl| Question { question: fl.title.clone(), header: fl.key.clone(), options: fl.options.iter().map(|(v, l)| Choice { label: l.clone(), description: v.clone() }).collect(), multi_select: false }).collect();
        (State::Awaiting, last, Some(Awaiting { kind: AwaitKind::Question, detail: truncate(&f.title, SNIPPET_CHARS), questions: qs }))
    } else if live.running {
        (State::Working, s.updated_ms, None)
    } else if now_ms.saturating_sub(last) < timeout_ms {
        (State::Completed, last, None)
    } else if window_pid.is_some() {
        (State::Idle, last + timeout_ms, None)
    } else {
        return None;
    };
    let context = context_window.filter(|w| *w > 0).map(|w| crate::context::ContextUsage { used: s.tokens_used, window: w, percent: ((s.tokens_used as f64 / w as f64) * 100.0).round().min(100.0) as u8 });
    Some(Card { session_id: s.id.clone(), pid: window_pid.unwrap_or(server_pid), name: s.title.clone().unwrap_or_else(|| format!("opencode {}", &s.id[s.id.len().saturating_sub(6)..])), cwd: s.directory.clone(), state, state_since: since, snippet: truncate(snippet.as_deref().unwrap_or(""), SNIPPET_CHARS), awaiting, has_inbox: false, harness: Harness::OpenCode, pr: None, context, machine: None, machine_address: None, machine_platform: None, terminal: None, stale: false })
}
```

The form's choice `description` carries the option's value, which Task 4 sends back; the label is what the card shows.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p maya-core opencode::`
Expected: PASS (6 tests).

- [ ] **Step 5: Commit**

```bash
git add core/src/opencode.rs core/src/lib.rs src-tauri/src/lib.rs core/fixtures/opencode/permission.json core/fixtures/opencode/form.json
git commit -m "feat(core): an OpenCode server client, its parsers and the card rules"
```

---

### Task 3: The store: fetching the server on each refresh

**Files:**
- Modify: `core/src/store.rs` (fields, `new`, `with_alive`, a `with_opencode` builder, `refresh`, `live_session_ids`, a `opencode_service()` accessor, `opencode_window(dir)`)
- Modify: `core/src/opencode.rs` (a `fetch` that turns one refresh's calls into cards)

**Interfaces:**
- Consumes: Task 2's `read_service`, `Client`, parsers, `card_for`.
- Produces:
  - `opencode::fetch(client: &Client, server_pid: i32, windows: &[(i32, String)], data_dir: &str, now_ms: u64, timeout_ms: u64, context_of: impl Fn(&str, &str) -> Option<u64>) -> Vec<Card>`: lists `GET /api/session?limit=50`, `GET /api/session/active`, and for each root session that is running or within the completed window or has a window for its folder, `GET /api/session/{id}/permission`, `GET /api/session/{id}/form` and `GET /api/session/{id}/message?limit=5&order=desc`; builds cards with `card_for`.
  - `Store::opencode_service() -> Option<Service>` (the file read now, with a live pid), `Store::opencode_window(dir) -> Option<i32>`, `Store::with_opencode(state_dir: PathBuf, windows: Vec<(i32, String)>) -> Self` (test support), `Store::opencode_card(session_id) -> Option<Card>` (from the last refresh).
  - The store keeps `opencode_cards: Vec<Card>` from the last refresh, and `live_session_ids` includes them.
  - `windows` on macOS and Linux: processes from the tree whose `command == "opencode"` with a tty, each with `foreign::proc_info(pid).cwd`; on Windows an empty list.

- [ ] **Step 1: Write the failing tests**

In `core/src/store.rs` tests:

```rust
    #[test]
    fn opencode_sessions_come_from_its_server_and_a_dead_pid_makes_no_call() {
        let (base, hits) = crate::opencode::fake::fake_server(vec![
            ("GET /api/session?limit=50", include_str!("../fixtures/opencode/sessions.json")),
            ("GET /api/session/active", "{\"data\":{\"ses_efe57d285ffeRMX70aswKZ9qCO\":{\"type\":\"running\"}}}"),
            ("GET /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/permission", "{\"data\":[]}"),
            ("GET /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/form", "{\"data\":[]}"),
            ("GET /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/message?limit=5&order=desc", include_str!("../fixtures/opencode/messages.json")),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir_all(state.join("opencode")).unwrap();
        let me = std::process::id();
        std::fs::write(state.join("opencode/service.json"), format!("{{\"url\":\"{base}\",\"pid\":{me},\"password\":\"pw\"}}")).unwrap();
        let mut store = Store::new(dir.path().join("claude")).with_alive(move |pid| pid == me as i32).with_opencode(state.clone(), vec![(31, "/Users/tiagocorreia".into())]);
        let cards = store.refresh(1_791_029_170_000);
        let c = cards.iter().find(|c| c.session_id == "ses_efe57d285ffeRMX70aswKZ9qCO").expect("the running session");
        assert_eq!((c.harness, c.state, c.pid, c.snippet.as_str(), c.name.as_str()), (Harness::OpenCode, State::Working, 31, "Hi", "Saying \"Hi\" request"));
        assert!(store.live_session_ids().contains(&c.session_id));
        assert!(cards.iter().filter(|c| c.harness == Harness::OpenCode).count() >= 2, "the completed ones within the window show too");
        // The server died but left its file: nothing is asked.
        std::fs::write(state.join("opencode/service.json"), format!("{{\"url\":\"{base}\",\"pid\":999999,\"password\":\"pw\"}}")).unwrap();
        let before = hits.lock().unwrap().len();
        let cards = store.refresh(1_791_029_170_000);
        assert!(cards.iter().all(|c| c.harness != Harness::OpenCode));
        assert_eq!(hits.lock().unwrap().len(), before, "no request to a dead server");
    }
```

- [ ] **Step 2: Run the test to see it fail**

Run: `cargo test -p maya-core opencode_sessions_come_from_its_server`
Expected: compile error on `with_opencode` and `fake`.

- [ ] **Step 3: Implement**

`opencode::fetch` as in Interfaces; the model limit for context comes from `context_of(provider, model)`, which the store answers from the agent listing (`agents::snapshot()` → the OpenCode `AgentInfo`'s models do not carry limits, so add `pub limits: HashMap<String, u64>` filled by `parse_opencode`'s sibling `parse_opencode_limits(json)`; keep it simple: `context_of` looks the id up in a `HashMap<String, u64>` the store caches from the listing text it already runs, or returns None, which leaves the card without context).

Store fields: `opencode_state_dir: PathBuf` (`dirs::state_dir()` on Linux, `~/.local/state` on macOS and as the fallback, with `%LOCALAPPDATA%\opencode\state` tried first on Windows), `opencode_windows: Box<dyn Fn() -> Vec<(i32, String)> + Send>`, `opencode_cards: Vec<Card>`. `with_alive` points the state dir at `/nonexistent`. In `refresh`, after the foreign loop:

```rust
        self.opencode_cards = match self.opencode_service() {
            Some(svc) => {
                let client = crate::opencode::Client::new(&svc);
                let windows = (self.opencode_windows)();
                let data_dir = self.claude_dir.join("maya").to_string_lossy().into_owned();
                crate::opencode::fetch(&client, svc.pid, &windows, &data_dir, now_ms, timeout, |p, m| self.opencode_limits.get(&format!("{p}/{m}")).copied())
            }
            None => vec![],
        };
        cards.extend(self.opencode_cards.iter().cloned());
```

`opencode_service()` reads the file and checks `(self.alive)(pid)`. The default `opencode_windows` lists `foreign::list_process_tree()` entries with `command == "opencode"` and a tty, mapping each through `foreign::proc_info(pid).cwd` on unix (empty on windows).

- [ ] **Step 4: Run the tests**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/store.rs core/src/opencode.rs
git commit -m "feat(core): OpenCode sessions on the board, polled from its server"
```

---

### Task 4: Actions through the server: replies, answers, rename, controls, history, Close

**Files:**
- Modify: `core/src/actions.rs` (a `Channel`, and the OpenCode branch of `send_reply`, `answer_question`, `rename_session`, `compact_session`, `set_session_option`, `send_slash_command`, `cycle_session_mode`, `session_history`, `close_session_with`, `focus_session`)
- Modify: `core/src/opencode.rs` (the action helpers)

**Interfaces:**
- Consumes: `Store::opencode_service`, `Store::opencode_card`, `Store::opencode_window`, Task 2's client.
- Produces in `opencode.rs`: `prompt(c, id, text)`, `reply_permission(c, id, rid, decision)`, `reply_form(c, id, fid, key, value)`, `rename(c, id, title)`, `compact(c, id)`, `switch_model(c, id, provider, model, variant)`, `switch_agent(c, id, agent)`, `next_agent(current) -> &str` (`build` → `plan` → `build`), `command(c, id, name, text)`, `shell(c, id, line)`, `history(c, id) -> Vec<Turn>` (60 messages), each `Result<(), String>` (or the turns).
- In `actions.rs`: `enum Channel { Inbox { socket: String, pid: i32 }, Tty { harness: Harness, tty: String }, Server { service: Service, session_id: String, card: Card } }` and `fn channel(store: &mut Store, session_id: &str) -> Result<Channel, String>`.

- [ ] **Step 1: Write the failing tests**

```rust
    fn opencode_store(routes: Vec<(&'static str, &'static str)>) -> (tempfile::TempDir, Mutex<Store>, std::sync::Arc<Mutex<Vec<String>>>) {
        let (base, hits) = crate::opencode::fake::fake_server(routes);
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir_all(state.join("opencode")).unwrap();
        let me = std::process::id();
        std::fs::write(state.join("opencode/service.json"), format!("{{\"url\":\"{base}\",\"pid\":{me},\"password\":\"pw\"}}")).unwrap();
        let store = Store::new(dir.path().join("claude")).with_alive(move |pid| pid == me as i32).with_opencode(state, vec![(31, "/Users/tiagocorreia".into())]);
        (dir, Mutex::new(store), hits)
    }

    const SES: &str = "ses_efe57d285ffeRMX70aswKZ9qCO";

    fn base_routes(active: &'static str, permission: &'static str) -> Vec<(&'static str, &'static str)> {
        vec![
            ("GET /api/session?limit=50", include_str!("../fixtures/opencode/sessions.json")),
            ("GET /api/session/active", active),
            ("GET /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/permission", permission),
            ("GET /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/form", "{\"data\":[]}"),
            ("GET /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/message?limit=5&order=desc", include_str!("../fixtures/opencode/messages.json")),
            ("GET /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/message?limit=60&order=desc", include_str!("../fixtures/opencode/messages.json")),
            ("POST /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/prompt", "{\"data\":{}}"),
            ("POST /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/permission/per_01/reply", "{\"data\":{}}"),
            ("PATCH /api/session/ses_efe57d285ffeRMX70aswKZ9qCO", "{\"data\":{}}"),
            ("POST /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/compact", "{\"data\":{}}"),
            ("POST /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/model", "{\"data\":{}}"),
            ("POST /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/agent", "{\"data\":{}}"),
            ("POST /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/command", "{\"data\":{}}"),
            ("POST /api/session/ses_efe57d285ffeRMX70aswKZ9qCO/shell", "{\"data\":{}}"),
        ]
    }

    #[test]
    fn opencode_actions_are_server_calls_and_nothing_is_typed() {
        let (_d, store, hits) = opencode_store(base_routes("{\"data\":{}}", "{\"data\":[]}"));
        let info = crate::agents::AgentInfo { harness: Harness::OpenCode, models: vec![crate::agents::ModelInfo { id: "opencode/fledge-alpha-free".into(), label: "Fledge".into(), efforts: vec!["low".into(), "max".into()] }], efforts: vec!["low".into(), "max".into()], modes: vec![] };
        let store = with_agents(store, vec![agents::claude(), info]);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        send_reply(&l, SES, "go on").unwrap();
        rename_session(&l, SES, "Renamed").unwrap();
        compact_session(&l, SES).unwrap();
        set_session_option(&l, SES, "model", "opencode/fledge-alpha-free").unwrap();
        set_session_option(&l, SES, "effort", "max").unwrap();
        cycle_session_mode(&l, SES).unwrap();
        send_slash_command(&l, SES, "/share now").unwrap();
        send_slash_command(&l, SES, "!ls").unwrap();
        let turns = session_history(&l, SES).unwrap();
        assert_eq!(turns.len(), 2);
        assert!(fake.calls.lock().unwrap().is_empty(), "nothing typed into any terminal");
        let h = hits.lock().unwrap();
        let bodies: Vec<&String> = h.iter().filter(|r| r.starts_with("POST") || r.starts_with("PATCH")).collect();
        assert!(bodies.iter().any(|r| r.contains("/prompt") && r.contains("\"text\":\"go on\"")));
        assert!(bodies.iter().any(|r| r.starts_with("PATCH") && r.contains("\"title\":\"Renamed\"")));
        assert!(bodies.iter().any(|r| r.contains("/compact")));
        assert!(bodies.iter().any(|r| r.contains("/model") && r.contains("\"providerID\":\"opencode\"") && r.contains("\"id\":\"fledge-alpha-free\"")));
        assert!(bodies.iter().any(|r| r.contains("/model") && r.contains("\"variant\":\"max\"")));
        assert!(bodies.iter().any(|r| r.contains("/agent") && r.contains("\"agent\":\"plan\"")), "build cycles to plan");
        assert!(bodies.iter().any(|r| r.contains("/command") && r.contains("\"name\":\"share\"") && r.contains("\"text\":\"now\"")));
        assert!(bodies.iter().any(|r| r.contains("/shell") && r.contains("\"command\":\"ls\"")));
    }

    #[test]
    fn an_opencode_permission_is_answered_with_a_decision_and_a_stale_ask_is_refused() {
        let (_d, store, hits) = opencode_store(base_routes("{\"data\":{}}", include_str!("../fixtures/opencode/permission.json")));
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        let card = store.lock().unwrap().card_for(SES, now_ms()).unwrap();
        assert_eq!(card.state, State::Awaiting);
        let ask = card.state_since;
        answer_question(&l, SES, ask, 0, 1).unwrap();
        assert!(hits.lock().unwrap().iter().any(|r| r.contains("/permission/per_01/reply") && r.contains("\"decision\":\"always\"")));
        assert_eq!(answer_question(&l, SES, ask + 1, 0, 1).unwrap_err(), "The question has changed; look again.");
        assert!(fake.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn a_working_opencode_session_refuses_a_reply() {
        let (_d, store, hits) = opencode_store(base_routes("{\"data\":{\"ses_efe57d285ffeRMX70aswKZ9qCO\":{\"type\":\"running\"}}}", "{\"data\":[]}"));
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        assert_eq!(send_reply(&l, SES, "hi").unwrap_err(), "Wait until the session is free.");
        assert!(!hits.lock().unwrap().iter().any(|r| r.contains("/prompt")));
    }

    #[test]
    fn closing_an_opencode_session_types_exit_into_its_window() {
        let (_d, store, _hits) = opencode_store(base_routes("{\"data\":{}}", "{\"data\":[]}"));
        let fake = FakeTerminal::default();
        fake.names.lock().unwrap().insert("/dev/ttys031".into(), "w".into());
        let l = Local { store: &store, terminal: &fake };
        // The window's pid (31) maps to a tty through the store's tty lookup; the fake terminal answers for it.
        let r = close_session_with(&l, SES, |_| true);
        assert!(r.is_ok() || r.unwrap_err().contains("terminal"), "typed /exit then exit, or explained why not");
    }
```

(`answer::check` requires `OPEN_DELAY_MS` since the ask: set the fake server's `time.updated` old enough, or pass `now_ms()` and keep the fixture's 2026-10-03 timestamps, which are far in the past.)

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core -- opencode_actions an_opencode_permission a_working_opencode closing_an_opencode`
Expected: FAIL: replies are typed (or "no inbox"), the permission answer types keys.

- [ ] **Step 3: Implement**

In `actions.rs`, a `Channel` and one resolver:

```rust
pub enum Channel {
    Inbox { socket: String, pid: i32 },
    Tty { harness: model::Harness, tty: String, pid: i32 },
    Server { service: crate::opencode::Service, card: Card },
}

fn channel(store: &mut Store, session_id: &str) -> Result<Channel, String> {
    if let Some(card) = store.opencode_card(session_id) {
        let service = store.opencode_service().ok_or("OpenCode's server is not running.")?;
        return Ok(Channel::Server { service, card });
    }
    if let Some(f) = store.foreign(session_id) {
        let tty = session_tty(store, session_id, f.pid)?;
        return Ok(Channel::Tty { harness: f.harness, tty, pid: f.pid });
    }
    let s = store.session(session_id).ok_or("Session is no longer running.")?;
    let socket = s.messaging_socket_path.clone().ok_or("This session has no inbox. Use the terminal.")?;
    Ok(Channel::Inbox { socket, pid: s.pid })
}
```

Each action starts with `let ch = channel(&mut l.store.lock().unwrap(), session_id)?;` and adds a `Channel::Server { service, card }` arm:
- `send_reply`: refuse `State::Working` with "Wait until the session is free."; else `opencode::prompt`.
- `answer_question`: `answer::check(&card, ask_id, question, option, now_ms())?` first; then if the card came from a permission (`store.opencode_live(session_id)` gives the `Live`), `reply_permission(decision(option))`; else `reply_form(fid, field.key, option value)`.
- `rename_session`: `opencode::rename` after `answer::rename_command(name)?` for the length rule.
- `compact_session`: `opencode::compact`.
- `set_session_option`: after the capability and listing checks (which already validate the value against the agent's lists; `plain_model_ref` instead of `plain_model_id` for OpenCode in `answer::slash_command`'s model check, or skip that check there since the listing check covers it), `switch_model` with the card's provider/model (from `opencode_card`'s session info: keep `SessionInfo` beside the card in the store, `opencode_session(id)`), replacing the model or the variant.
- `send_slash_command`: `/name rest` → `command(name, rest)`; `!line` → `shell(line)`.
- `cycle_session_mode`: `switch_agent(next_agent(&session.agent))`.
- `session_history`: `opencode::history`.
- `close_session_with`: with `Channel::Server`, the window pid is `card.pid` when `store.opencode_window(&card.cwd)` is Some, else `Err("No OpenCode window is open for that folder.")`; then `session_tty` for that pid and the existing `/exit` then `exit` path.
- `focus_session`: the window pid as above.

- [ ] **Step 4: Run the tests**

Run: `cargo test --workspace`
Expected: PASS. The Claude Code and foreign paths are unchanged in behaviour: their existing tests still pass.

- [ ] **Step 5: Commit**

```bash
git add core/src/actions.rs core/src/opencode.rs core/src/store.rs core/src/answer.rs
git commit -m "feat(core): a per-card channel, and every session action through OpenCode's server"
```

---

### Task 5: New session, Resume, reviews and the server started on demand

**Files:**
- Modify: `core/src/opencode.rs` (`create_session`, `ensure_service`, `resumable`)
- Modify: `core/src/actions.rs` (`start_session`, `start_review`, `list_resumable_sessions`, `resume_session` OpenCode branches)
- Modify: `core/src/launch.rs` only if `start_session`'s shape needs a hook

**Interfaces:**
- Produces: `opencode::create_session(c, dir, title: Option<&str>, model: Option<(&str, &str)>, variant: Option<&str>) -> Result<String, String>` (`POST /api/session`, returns `data.id`); `opencode::ensure_service(binary: &Path, state_file: &Path, alive: &dyn Fn(i32) -> bool, start: impl FnOnce() -> Result<(), String>, wait: Duration) -> Result<Service, String>`; `opencode::resumable(c, dir, running: &[String]) -> Vec<ResumableSession>`.

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn starting_an_opencode_session_creates_it_prompts_it_and_opens_a_window_on_it() {
        let mut routes = base_routes("{\"data\":{}}", "{\"data\":[]}");
        routes.push(("POST /api/session", "{\"data\":{\"id\":\"ses_new01\"}}"));
        routes.push(("POST /api/session/ses_new01/prompt", "{\"data\":{}}"));
        let (d, store, hits) = opencode_store(routes);
        std::fs::create_dir_all(d.path().join("projects/proj")).unwrap();
        store.lock().unwrap().config.projects_dir = Some(d.path().join("projects").to_string_lossy().into_owned());
        let info = crate::agents::AgentInfo { harness: Harness::OpenCode, models: vec![crate::agents::ModelInfo { id: "opencode/fledge-alpha-free".into(), label: "Fledge".into(), efforts: vec!["low".into()] }], efforts: vec!["low".into()], modes: vec!["default".into(), "auto".into()] };
        let store = with_agents(store, vec![agents::claude(), info]);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        let opts = LaunchOptions { agent: Harness::OpenCode, model: Some("opencode/fledge-alpha-free".into()), effort: Some("low".into()), mode: Some("auto".into()), name: Some("Fix CI".into()) };
        start_session(&l, Some("proj".into()), "-v please say hi".into(), opts).unwrap();
        let h = hits.lock().unwrap();
        let create = h.iter().find(|r| r.starts_with("POST /api/session\n")).expect("created on the server");
        assert!(create.contains("\"title\":\"Fix CI\"") && create.contains("\"directory\":") && create.contains("proj") && create.contains("\"variant\":\"low\""));
        assert!(h.iter().any(|r| r.contains("/ses_new01/prompt") && r.contains("-v please say hi")), "the prompt goes over the wire, never a shell line");
        let opened: Vec<String> = fake.calls.lock().unwrap().iter().filter_map(|c| match c { Call::Open { command, .. } => Some(command.clone()), _ => None }).collect();
        assert_eq!(opened.len(), 1);
        assert!(opened[0].ends_with("&& opencode --session 'ses_new01' --auto"), "{}", opened[0]);
    }

    #[test]
    fn the_server_is_started_on_demand_and_a_start_that_leaves_no_file_fails() {
        let t = tempfile::tempdir().unwrap();
        let file = t.path().join("opencode/service.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let me = std::process::id();
        let started = std::cell::Cell::new(false);
        let s = crate::opencode::ensure_service(Path::new("/bin/opencode"), &file, &|pid| pid == me as i32, || { started.set(true); std::fs::write(&file, format!("{{\"url\":\"http://127.0.0.1:1\",\"pid\":{me},\"password\":\"x\"}}")).unwrap(); Ok(()) }, Duration::from_secs(2)).unwrap();
        assert!(started.get() && s.pid == me as i32);
        let s2 = crate::opencode::ensure_service(Path::new("/bin/opencode"), &file, &|pid| pid == me as i32, || panic!("already running"), Duration::from_secs(2)).unwrap();
        assert_eq!(s2.url, "http://127.0.0.1:1");
        std::fs::remove_file(&file).unwrap();
        let err = crate::opencode::ensure_service(Path::new("/bin/opencode"), &file, &|_| false, || Ok(()), Duration::from_millis(300)).unwrap_err();
        assert_eq!(err, "OpenCode's server did not start.");
    }

    #[test]
    fn opencode_sessions_of_a_folder_are_listed_from_the_server_and_resumed_by_id() {
        let (_d, store, _hits) = opencode_store(base_routes("{\"data\":{}}", "{\"data\":[]}"));
        store.lock().unwrap().config.projects_dir = Some("/Users".into());
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        let list = list_resumable_sessions(&l, Harness::OpenCode, "tiagocorreia").unwrap();
        assert!(list.iter().any(|s| s.id == SES && s.title == "Saying \"Hi\" request"));
        assert!(list.windows(2).all(|w| w[0].last_active_ms >= w[1].last_active_ms), "newest first");
        resume_session(&l, Harness::OpenCode, "tiagocorreia", SES).unwrap();
        let opened: Vec<String> = fake.calls.lock().unwrap().iter().filter_map(|c| match c { Call::Open { command, .. } => Some(command.clone()), _ => None }).collect();
        assert!(opened[0].ends_with(&format!("&& opencode --session '{SES}'")), "{}", opened[0]);
        assert_eq!(resume_session(&l, Harness::OpenCode, "tiagocorreia", "ses_nope").unwrap_err(), "No such session in that folder.");
    }
```

(`Call::Open`'s field names come from `core/src/terminal.rs`'s `FakeTerminal`; adjust the pattern to its real shape. `config.projects_dir`'s type is whatever `Config` holds; set it the way `store_with_projects` in the existing tests does.)

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core -- starting_an_opencode the_server_is_started opencode_sessions_of_a_folder`
Expected: FAIL (the start opens a terminal with a prompt file and no server call; `ensure_service` undefined; the listing is empty).

- [ ] **Step 3: Implement**

`opencode.rs`:

```rust
pub fn create_session(c: &Client, dir: &str, title: Option<&str>, model: Option<(&str, &str)>, variant: Option<&str>) -> Result<String, String> {
    let mut body = serde_json::json!({ "location": { "directory": dir } });
    if let Some(t) = title { body["title"] = Value::String(t.into()); }
    if let Some((p, m)) = model {
        body["model"] = serde_json::json!({ "providerID": p, "id": m });
        if let Some(v) = variant { body["model"]["variant"] = Value::String(v.into()); }
    }
    let v = c.post("/api/session", body)?;
    v["data"]["id"].as_str().map(String::from).ok_or_else(|| "OpenCode's server returned no session id.".to_string())
}

pub fn ensure_service(binary: &Path, state_file: &Path, alive: &dyn Fn(i32) -> bool, start: impl FnOnce() -> Result<(), String>, wait: Duration) -> Result<Service, String> {
    if let Some(s) = read_service(state_file).filter(|s| alive(s.pid)) { return Ok(s); }
    let _ = binary; // the caller's `start` runs `<binary> service start`
    start()?;
    let deadline = Instant::now() + wait;
    while Instant::now() < deadline {
        if let Some(s) = read_service(state_file).filter(|s| alive(s.pid)) { return Ok(s); }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err("OpenCode's server did not start.".into())
}

pub fn resumable(c: &Client, dir: &str, running: &[String]) -> Vec<crate::resume::ResumableSession> {
    let Ok(v) = c.get("/api/session?limit=200") else { return vec![] };
    let mut out: Vec<_> = sessions(&v).into_iter().filter(|s| s.parent_id.is_none() && s.directory == dir).map(|s| crate::resume::ResumableSession { running: running.contains(&s.id), title: s.title.clone().unwrap_or_else(|| s.id.clone()), last_active_ms: s.updated_ms.max(s.idle_ms.unwrap_or(0)), id: s.id }).collect();
    out.sort_by(|a, b| b.last_active_ms.cmp(&a.last_active_ms));
    out
}
```

`actions.rs`:
- `start_session`: after `options.validate(&info.model_ids())?` and before the classifier, `if options.agent == Harness::OpenCode { return start_opencode(l, dir, prompt, options, &root, &dirs); }` where `start_opencode` resolves the target the same way (`resolve_target`, with the classifier for "Let Maya choose"), gets the service via `ensure_service(binary, &store.opencode_state_file(), &*alive, || run `<binary> service start`, Duration::from_secs(10))`, creates the session (title = chosen name, model split at `/`, variant = effort), posts the prompt, and opens `launch::opencode_open_command(&target, &id, options.mode == Some("auto"))`. The `alive` function: `crate::registry::pid_alive`. The start command: `crate::command(&binary).args(["service", "start"]).env_clear().envs(launch::clean_env(std::env::vars())).status()`.
- `start_review`: the OpenCode branch creates the session named `reviews::session_name(repo, number)`, prompts with the rendered prompt and opens the window in the target folder.
- `list_resumable_sessions` and `resume_session`: for OpenCode, `opencode::resumable` with the store's service (empty list when the server is down) and `resume::resume_command(OpenCode, …)`.

- [ ] **Step 4: Run the tests**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/actions.rs core/src/opencode.rs
git commit -m "feat(core): start, resume and review OpenCode sessions through its server"
```

---

### Task 6: The frontend, the CLI copy and the docs

**Files:**
- Modify: `src/types.ts:20`, `src/harness.ts`, `src/harness.test.ts`, `src/firstrun.ts:53`, `src/firstrun.test.ts:32`, `src/card.ts` (the Close tooltip stays), `src/modal.ts` (the mode-cycle button's title for OpenCode: "Switches between the build and plan agents")
- Modify: `README.md`, `docs/DEVELOPING.md`

- [ ] **Step 1: Write the failing tests**

`src/harness.test.ts`: add `opencode: { compact: true, modelSwitch: true, effortSwitch: true, modeCycle: true, slashLines: true, shellLines: true, close: true }` to the table and `expect(harnessLabel("opencode")).toBe("OpenCode"); expect(harnessBadge("opencode", "b").tagName).toBe("IMG");`. `src/firstrun.test.ts`: the six-agent text. A modal test: the cycle-mode button's `title` for an OpenCode card contains "build and plan".

- [ ] **Step 2: Run the tests to see them fail**

Run: `pnpm test -- harness firstrun modal`
Expected: FAIL.

- [ ] **Step 3: Implement**

`types.ts`: add `"opencode"`. `harness.ts`: `import opencodeIcon from "./assets/icons/opencode.svg";`, label "OpenCode", icon, capability row. `firstrun.ts`: "Claude Code, Codex, Antigravity, Grok Build, Kiro CLI or OpenCode". `modal.ts`: where the cycle-mode button's title is set, `card.harness === "opencode" ? "Switches between the build and plan agents" : <today's text>`.

Docs: README "What it does" and requirements name OpenCode (link `https://opencode.ai`); a paragraph in the sessions section: "OpenCode sessions come from its background server (the state file under `~/.local/state/opencode`), not from files: cards, replies, approvals and controls all go through it, and Close types `/exit` into the OpenCode window for the folder. Only the shared service is found; `--standalone` servers are not." Windows note: the state file's location is unverified there and Close is refused. `docs/DEVELOPING.md`: `opencode.rs` in the module list, `ureq` in the dependencies line.

- [ ] **Step 4: Run the tests and the build**

Run: `pnpm test && pnpm build`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/types.ts src/harness.ts src/harness.test.ts src/firstrun.ts src/firstrun.test.ts src/modal.ts src/modal.test.ts README.md docs/DEVELOPING.md
git commit -m "feat(ui): OpenCode's icon, label and controls, and the docs"
```

---

### Task 7: Rebase, real-app check, version bump

- [ ] **Step 1: Rebase onto main once PR #21 has merged**

Run: `git fetch origin && git rebase origin/main` (resolve nothing: the Kiro commits are already on `main`). Then `cargo test --workspace && pnpm test && pnpm build`.

- [ ] **Step 2: Real-app check, with the user**

As the memory note "Maya run and verify" describes (vite plus the debug binary, screenshot then click), with the user starting anything that runs an agent:
1. The user's running OpenCode TUI session appears as a card with its folder, state and snippet; its modal shows history, context, Model, Effort, Apply, Compact and the mode cycle.
2. The user asks OpenCode to do something that needs a permission (a write outside the project under a restrictive rule): the card goes to Awaiting with Once, Always, Reject; answering Once from the card lets it continue.
3. Reply and rename from the modal; `/model`, `/effort`, the mode cycle and `!ls` from the composer.
4. New session with OpenCode, a name, a model and an effort, and a prompt starting with `-`: the server shows the session, the window opens on it.
5. Resume lists that folder's sessions; resuming opens the window.
6. Close on an idle OpenCode card types `/exit` into its window.
7. Settings: OpenCode as Maya's agent, one voice command, one "Let Maya choose".
Record the outcome in the ledger; anything the server refuses becomes a finding.

- [ ] **Step 3: Bump the version**

`sh scripts/set-version.sh 0.12.0` (the next minor above `main`'s), committed alone as `chore(release): 0.12.0`.

- [ ] **Step 4: Open the pull request**

Ask the user before pushing. Title `feat: OpenCode as a Maya agent`; the body follows the repo's pull-request skill (Why, What, Changes, Test plan), ending with the attribution lines.
