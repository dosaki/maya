# OpenAI-compatible Voice and Chatterbox Server Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Maya speaks each line with the first voice that works, in the fixed order ElevenLabs → an OpenAI-compatible speech server → the built-in voice, and the repo ships a small Chatterbox server (native with `uv`, or Docker) plus a guide.

**Architecture:** `maya_core` gains the server's HTTP client (`voice_server.rs`, over `ureq`) and turns the speech queue's single ElevenLabs option into a chain of remote voices with in-memory "down" marks (`notify.rs`). The app (`src-tauri`) builds the chain from the config, caches and plays audio, and stores the server key. Settings replaces the provider dropdown with one switchable section per remote voice. `chatterbox/` holds a one-file FastAPI server, its Dockerfile and compose file.

**Tech Stack:** Rust (serde, ureq 3, std mpsc), TypeScript + vitest (Settings DOM), Python 3.11 + FastAPI + uvicorn + `chatterbox-tts` (PEP 723 script run by `uv`), Docker/Podman compose.

**Spec:** `docs/superpowers/specs/2026-10-23-openai-compatible-voice-design.md`

## Global Constraints

- Chain order is fixed: ElevenLabs → voice server → built-in. The built-in voice is always last and always there.
- Nobody's voice changes on upgrade: ElevenLabs is on only when the old config has `voiceProvider: "elevenlabs"`; the server starts off.
- Voice settings are per device, in `Config`; never synced or carried by promotion.
- Server requests: `POST <base>/v1/audio/speech` with `{"input", "voice", "response_format": "wav"}` plus `"model"` only when the model field is not blank; `GET <base>/v1/audio/voices`; optional `Authorization: Bearer <key>`.
- Connection timeout **3 s** (server and ElevenLabs); whole-request timeout **60 s** for the server.
- A failed remote voice is marked down for **5 minutes** or until its settings change; a cached line still plays while it is down; "Try the voice" ignores down marks.
- Secrets never on a command line or in a log. The server key lives in the OS key store (`keystore.rs`).
- The ElevenLabs cache file names stay exactly as today (`<voice>-<hash>.mp3`), so existing caches keep working.
- Desktop only; the phone is untouched.
- Unix-only tests are gated with `#[cfg(unix)]`; paths are compared by component.
- Version bump to **0.31.0** (main is 0.30.0) with `sh scripts/set-version.sh 0.31.0`, its own commit `chore(release): 0.31.0`; check `Cargo.lock` changed.
- Server pins: `chatterbox-tts==0.1.7`, `setuptools<80`, `fastapi==0.161.1`, `uvicorn==0.54.0`, Python `>=3.11,<3.12`; default host `127.0.0.1`, port `8004`, device `cpu`.

## Review Focus

1. **Upgrade with ElevenLabs set up but switched to built-in** (`voiceProvider: "builtin"`, voice id still stored): ElevenLabs must stay off. Pinned in Task 1.
2. **A server address typed loosely** (`localhost:8004`, `http://mac.local:8004/v1/`, surrounding spaces): normalised when it has a scheme; without one the server is left out of the chain and Settings says why, rather than every line failing. Pinned in Tasks 2 and 4.
3. **A server answering 200 with something that is not audio** (an empty body or a JSON error): no cache file is written, so a bad answer is not replayed forever. Pinned in Task 2.
4. **A cache miss while a voice is down** is not a new failure: the mark is not renewed and the report does not list it. Only a network attempt marks a voice down. Pinned in Task 3.
5. **"Try the voice" while ElevenLabs is marked down** tries ElevenLabs anyway and, on success, clears the mark so the next announcement uses it. Pinned in Task 3.

---

### Task 1: Config fields for the chain

**Files:**
- Modify: `core/src/config.rs` (fields next to `voice_provider` at `:107-115`, `Default` at `:384-386`, tests near `:708-716`)

**Interfaces:**
- Produces:
  - `Config.elevenlabs_enabled: Option<bool>` — JSON `elevenlabsEnabled`, `#[serde(default, skip_serializing_if = "Option::is_none")]`.
  - `Config.voice_server: VoiceServer` — JSON `voiceServer`, `#[serde(default)]`.
  - `pub struct VoiceServer { pub enabled: bool, pub url: String, pub voice: String, pub model: String }` — `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]`, `#[serde(rename_all = "camelCase", default)]`.
  - `impl Config { pub fn elevenlabs_on(&self) -> bool }` — `elevenlabs_enabled` when set, else `voice_provider == VoiceProvider::Elevenlabs`.
  - `voice_provider` stays as it is (an older Maya still reads it); the new UI stops writing it.

- [ ] **Step 1: Write the failing tests** in `config.rs`'s test module:

```rust
#[test]
fn elevenlabs_is_on_only_where_it_was_the_chosen_voice() {
    let old = |json: &str| serde_json::from_str::<Config>(json).unwrap();
    assert!(old(r#"{"completedTimeoutMinutes": 5, "voiceProvider": "elevenlabs", "elevenlabsVoiceId": "abc"}"#).elevenlabs_on());
    assert!(!old(r#"{"completedTimeoutMinutes": 5, "voiceProvider": "builtin", "elevenlabsVoiceId": "abc"}"#).elevenlabs_on(), "set up but not chosen stays off");
    assert!(!Config::default().elevenlabs_on());
    assert!(!old(r#"{"completedTimeoutMinutes": 5, "voiceProvider": "elevenlabs", "elevenlabsEnabled": false}"#).elevenlabs_on());
    assert!(old(r#"{"completedTimeoutMinutes": 5, "elevenlabsEnabled": true}"#).elevenlabs_on());
}

#[test]
fn voice_server_defaults_off_and_round_trips() {
    assert_eq!(Config::default().voice_server, VoiceServer::default());
    assert!(!Config::default().voice_server.enabled);
    let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes": 5, "voiceServer": {"enabled": true, "url": "http://localhost:8004", "voice": "tiago.wav"}}"#).unwrap();
    assert_eq!(c.voice_server, VoiceServer { enabled: true, url: "http://localhost:8004".into(), voice: "tiago.wav".into(), model: String::new() });
    let text = serde_json::to_string(&c).unwrap();
    assert!(text.contains("\"voiceServer\":{\"enabled\":true"));
    assert!(!text.contains("elevenlabsEnabled"), "unset stays unwritten");
}
```

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p maya-core -- config::tests::elevenlabs_is_on_only config::tests::voice_server_defaults`
Expected: compile error, no field `voice_server` / no method `elevenlabs_on`.

- [ ] **Step 3: Add the fields, `VoiceServer`, the defaults and `elevenlabs_on`** as in Interfaces, with doc comments in the file's style. Update the `VoiceProvider` doc comment to say it is only read for the migration to `elevenlabs_on`.

- [ ] **Step 4: Run the config tests**

Run: `cargo test -p maya-core config::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/config.rs
git commit -m "feat: config for a voice server and an ElevenLabs switch"
```

---

### Task 2: The voice server client in core

**Files:**
- Create: `core/src/voice_server.rs`
- Modify: `core/src/lib.rs` (add `pub mod voice_server;`)

**Interfaces:**
- Produces (all `pub` in `maya_core::voice_server`):
  - `const CONNECT_TIMEOUT: Duration = 3 s`, `const REQUEST_TIMEOUT: Duration = 60 s`.
  - `fn base_url(raw: &str) -> Option<String>` — trims spaces; requires an `http://` or `https://` prefix (else `None`); strips trailing `/` then a trailing `/v1`, then trailing `/` again.
  - `fn speech_url(base: &str) -> String` → `<base>/v1/audio/speech`; `fn voices_url(base: &str) -> String` → `<base>/v1/audio/voices`.
  - `fn request_body(text: &str, voice: &str, model: &str) -> String` — JSON with `input`, `voice`, `response_format: "wav"`, and `model` only when `model.trim()` is not empty.
  - `fn parse_voices(json: &str) -> Result<Vec<String>, String>` — reads `voices` as an array of strings, or of objects with a string `name` (other servers); missing `voices` → empty; not JSON → `Err`.
  - `fn list_voices(base: &str, key: Option<&str>) -> Result<Vec<String>, String>` — `GET voices_url`; a 404 means the server has no list → `Ok(vec![])`; other non-2xx → `Err`.
  - `fn synthesize(base: &str, key: Option<&str>, voice: &str, model: &str, text: &str, path: &Path) -> Result<(), String>` — `POST speech_url` with `request_body`; writes `<path>.part` then renames to `path`; returns `Err` and leaves no file when the status is not 2xx, the body is empty, or `Content-Type` starts with `application/json` or `text/`. Error text names the server: `could not reach the voice server at <base>: <e>` / `the voice server answered <status>: <first 200 chars of the body>`.
  - Both HTTP calls use one `ureq::Agent` built like `core/src/forge/mod.rs:211-213` (platform-verifier TLS) with `timeout_connect(Some(CONNECT_TIMEOUT))`, `timeout_global(Some(REQUEST_TIMEOUT))`, `http_status_as_error(false)`. The key goes only in the `Authorization: Bearer` header.

- [ ] **Step 1: Write the failing pure tests** in `voice_server.rs`:

```rust
#[test]
fn base_url_is_normalised_and_needs_a_scheme() {
    assert_eq!(base_url(" http://localhost:8004/ ").as_deref(), Some("http://localhost:8004"));
    assert_eq!(base_url("http://mac.local:8004/v1/").as_deref(), Some("http://mac.local:8004"));
    assert_eq!(base_url("https://api.openai.com/v1").as_deref(), Some("https://api.openai.com"));
    assert_eq!(base_url("localhost:8004"), None);
    assert_eq!(base_url("   "), None);
    assert_eq!(speech_url("http://h:1"), "http://h:1/v1/audio/speech");
    assert_eq!(voices_url("http://h:1"), "http://h:1/v1/audio/voices");
}

#[test]
fn request_asks_for_wav_and_sends_the_model_only_when_given() {
    let v: serde_json::Value = serde_json::from_str(&request_body("hi \"x\"", "tiago.wav", " ")).unwrap();
    assert_eq!(v["input"], "hi \"x\"");
    assert_eq!(v["voice"], "tiago.wav");
    assert_eq!(v["response_format"], "wav");
    assert!(v.get("model").is_none());
    let v: serde_json::Value = serde_json::from_str(&request_body("hi", "alloy", "tts-1")).unwrap();
    assert_eq!(v["model"], "tts-1");
}

#[test]
fn voices_parse_from_names_or_objects() {
    assert_eq!(parse_voices(r#"{"status":"ok","voices":["default","tiago.wav"]}"#).unwrap(), vec!["default", "tiago.wav"]);
    assert_eq!(parse_voices(r#"{"voices":[{"name":"amy"},{"id":"x"}]}"#).unwrap(), vec!["amy"]);
    assert!(parse_voices("{}").unwrap().is_empty());
    assert!(parse_voices("nope").is_err());
}
```

- [ ] **Step 2: Write the failing HTTP tests** with a one-shot fake server on `std::net::TcpListener` bound to `127.0.0.1:0`. A helper `fn serve_once(status: &str, content_type: &str, body: &[u8]) -> (String /*base*/, std::thread::JoinHandle<String /*raw request*/>)` that accepts one connection, reads the request head and body (by `Content-Length`), writes the canned response, and returns the raw request text.
  - `synthesize_writes_the_audio_and_sends_the_key_as_a_bearer_header`: 200, `audio/wav`, body `b"RIFF....WAVE"`; assert the file at `path` equals the body, no `.part` file is left, and the request contains `POST /v1/audio/speech`, `authorization: Bearer k1` (compare case-insensitively) and `"response_format":"wav"`.
  - `synthesize_leaves_no_file_for_an_error_or_a_non_audio_answer`: three runs: 500 `text/plain` `boom` → `Err` containing `500` and `boom`; 200 `application/json` `{"error":"x"}` → `Err`; 200 `audio/wav` empty body → `Err`. After each, `!path.exists()`.
  - `list_voices_treats_a_404_as_no_list`: 404 → `Ok(vec![])`; 200 with `{"voices":["a.wav"]}` → `Ok(vec!["a.wav"])`.
  - `an_unreachable_server_fails_fast`: bind a listener, take its port, drop it; `synthesize` against that port returns `Err` containing `could not reach the voice server` in under 5 s.

- [ ] **Step 3: Run them and see them fail**

Run: `cargo test -p maya-core voice_server::`
Expected: compile errors (module/functions missing).

- [ ] **Step 4: Implement `voice_server.rs`** per Interfaces, with a module doc comment in the style of `src-tauri/src/voice.rs:1-5`.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p maya-core voice_server::`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add core/src/voice_server.rs core/src/lib.rs
git commit -m "feat: an OpenAI-compatible speech client"
```

---

### Task 3: A chain of voices in the speech queue

**Files:**
- Modify: `core/src/notify.rs` (`Utterance` `:106-123`, speaker `:216-227`, `speak_line` `:258-270`, `deliver` `:272-289`, `speak_and_wait` `:312-320`, tests `:554`, `:731-745`)
- Modify: `core/examples/linux_probe.rs:39`
- Modify (callers only, to keep the build green): `src-tauri/src/lib.rs:320-327`, `:559-571`, `:1180-1186`, `:1446`; `src-tauri/src/listener.rs:180-182`; `src-tauri/src/voice.rs:181-188`

**Interfaces:**
- Consumes: nothing new.
- Produces (in `maya_core::notify`):
  - `#[derive(Clone, PartialEq, Eq, Hash)] pub enum RemoteVoice { Elevenlabs { maya_dir: PathBuf, key: String, voice_id: String }, Server { maya_dir: PathBuf, base: String, voice: String, model: String, key: Option<String> } }` with `pub fn label(&self) -> &'static str` (`"ElevenLabs"` / `"the voice server"`) and `fn fingerprint(&self) -> u64` (std `DefaultHasher` over the whole value; marks live in memory, so a per-process hash is enough). `Debug` is written by hand and hides `key`: the error log must never show it.
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Reach { CacheOnly, Network }`.
  - `pub type RemoteSpeaker = fn(&RemoteVoice, &str, u8, Reach) -> Result<(), String>;` and `pub fn set_remote_speaker(speak: RemoteSpeaker)` replacing `ElevenSpeaker` / `set_eleven_speaker`. With `Reach::CacheOnly` the speaker plays only a cached line and otherwise returns `Err`.
  - `Utterance { text, voices: Vec<RemoteVoice>, trial: bool, done: Option<mpsc::Sender<SpeechReport>>, volume }`; `Utterance::new(text: String, voices: Vec<RemoteVoice>)` (trial false, volume 100). `trial` replaces `fallback`.
  - `#[derive(Clone, Debug, PartialEq, Eq)] pub struct SpeechReport { pub spoken_by: &'static str, pub failures: Vec<String> }` where each failure is `"<Label> failed: <error>."` with the label's first letter upper-cased, and `spoken_by` is a label or `"the built-in voice"`. `pub fn message(&self) -> String` joins the failures and `"Spoke with <spoken_by>."` with single spaces.
  - `pub const DOWN_FOR: Duration = Duration::from_secs(300);` and `#[derive(Default)] pub(crate) struct DownMarks(HashMap<u64, Instant>)` with `is_down(&self, fp: u64, now: Instant) -> bool` (marked less than `DOWN_FOR` ago), `mark(&mut self, fp: u64, now: Instant)`, `clear(&mut self, fp: u64)`.
  - `fn speak_chain(u: &Utterance, marks: &mut DownMarks, now: Instant, remote: &dyn Fn(&RemoteVoice, &str, u8, Reach) -> Result<(), String>, builtin: &dyn Fn(&str, u8)) -> SpeechReport` — for each voice in order: if `!u.trial && marks.is_down(fp, now)`, try `Reach::CacheOnly`; success returns, failure moves on silently (no mark, no failure entry). Otherwise try `Reach::Network`; success clears the mark and returns; failure marks it and pushes a failure entry. When no remote voice spoke, call `builtin` and report `"the built-in voice"`.
  - `speak_line` uses a `static DOWN: Mutex<DownMarks>`, `Instant::now()` and the installed speaker (no speaker installed → remote voices are skipped). `deliver` logs each failure with `crate::log::line("speech", …)` and sends the report to `done`.
  - `pub fn speak_and_wait(u: Utterance) -> Result<SpeechReport, String>` (`Err` only when the queue stopped).
- Caller changes in this task (mechanical, same behaviour): wrap today's ElevenLabs tuple as `RemoteVoice::Elevenlabs { maya_dir, key, voice_id }` in a one-element `Vec`; `try_voice` uses `trial: true` and returns `Ok(())` after `speak_and_wait(..)?`; rename `voice::speak` to `pub fn speak_remote(v: &RemoteVoice, text: &str, volume: u8, reach: Reach) -> Result<(), String>` handling the `Elevenlabs` variant (cache hit → play; `CacheOnly` miss → `Err("not cached")`; else synthesize then play) and returning `Err("not supported yet")` for `Server` until Task 4; install it with `set_remote_speaker`. `linux_probe` matches on `Ok(report)` and prints `report.message()`.

- [ ] **Step 1: Write the failing tests** in `notify.rs`'s test module, with a recording fake `remote` (pushes `(label, reach)` to a `RefCell<Vec<_>>` and answers from a per-label script) and a fake `builtin` that records the text:

```rust
fn eleven() -> RemoteVoice { RemoteVoice::Elevenlabs { maya_dir: "/m".into(), key: "k".into(), voice_id: "v".into() } }
fn server() -> RemoteVoice { RemoteVoice::Server { maya_dir: "/m".into(), base: "http://h:1".into(), voice: "default".into(), model: String::new(), key: None } }
```

  - `the_chain_goes_elevenlabs_then_server_then_builtin`: ElevenLabs `Err("402")`, server `Ok` → report `{ spoken_by: "the voice server", failures: ["ElevenLabs failed: 402."] }`; built-in not called; calls were `[("ElevenLabs", Network), ("the voice server", Network)]`.
  - `with_every_remote_voice_failing_the_builtin_speaks`: both `Err` → `spoken_by == "the built-in voice"`, two failures, built-in called with the text; `report.message() == "ElevenLabs failed: a. The voice server failed: b. Spoke with the built-in voice."`.
  - `a_failed_voice_is_skipped_for_five_minutes_but_its_cache_still_plays`: fail ElevenLabs once at `t0`; at `t0 + 60 s` it is called with `CacheOnly` only (answer `Err` → server speaks, failures list empty, still down at `t0 + 299 s`); at `t0 + DOWN_FOR` it is called with `Network` again.
  - `a_cache_miss_while_down_does_not_renew_the_mark`: fail at `t0`, cache miss at `t0 + 200 s`, then at `t0 + 300 s` the call is `Network`.
  - `changed_settings_are_not_down`: fail `eleven()` at `t0`; a `RemoteVoice::Elevenlabs` with another `voice_id` at `t0 + 1 s` is called with `Network`.
  - `a_trial_ignores_marks_and_a_success_clears_them`: fail at `t0`; trial at `t0 + 1 s` calls `Network` and succeeds; a non-trial at `t0 + 2 s` calls `Network` (mark cleared).
  - `a_remote_voice_never_shows_its_key`: `format!("{:?}", eleven())` does not contain `"k\""`/`key: "k"`; a `Server` with `key: Some("sk-secret")` does not contain `sk-secret`.
  - Update the existing `deliver` tests (`:731-745`) to the new `done` type and `Utterance::new(.., vec![])`; keep `Utterance::new("x".into(), vec![]).volume == 100`.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p maya-core notify::`
Expected: compile errors (`RemoteVoice`, `speak_chain` missing).

- [ ] **Step 3: Implement the core changes**, then the caller changes listed in Interfaces.

- [ ] **Step 4: Build and test the workspace**

Run: `cargo test -p maya-core notify:: && cargo build --workspace --examples && cargo test -p maya`
Expected: PASS; ElevenLabs behaves as before.

- [ ] **Step 5: Commit**

```bash
git add core/src/notify.rs core/examples/linux_probe.rs src-tauri/src/lib.rs src-tauri/src/listener.rs src-tauri/src/voice.rs
git commit -m "feat: speak through a chain of voices that remembers which are down"
```

---

### Task 4: The app builds the chain and speaks through the server

**Files:**
- Modify: `src-tauri/src/keystore.rs:20` (new item)
- Modify: `src-tauri/src/voice.rs` (cache paths, `speak_remote`, curl connect timeout, Linux player comment, tests)
- Modify: `src-tauri/src/lib.rs` (`eleven_wanted`/`eleven_settings` at `:442-460` and their callers at `:309-327`, `:524-571`, `:1180`; commands near `:1155-1186`; the handler list near `:1396`; tests near `:1526-1535`)
- Modify: `src-tauri/src/listener.rs:11`, `:174-182`

**Interfaces:**
- Consumes: `Config::elevenlabs_on`, `Config.voice_server` (Task 1); `voice_server::{base_url, synthesize, list_voices}` (Task 2); `RemoteVoice`, `Reach`, `SpeechReport`, `speak_and_wait`, `set_remote_speaker` (Task 3).
- Produces:
  - `keystore::VOICE_SERVER: Item = Item { service: "maya-voice-server", account: "api-key", label: "Maya voice server", linux_account: "voice-server" }`.
  - In `voice.rs`: `pub fn store_server_key(key: &str) -> Result<(), String>`, `pub fn load_server_key() -> Option<String>`; `pub fn cache_path(maya_dir: &Path, tag: &str, text: &str, ext: &str) -> PathBuf` (same filtering as today, so `cache_path(dir, voice_id, text, "mp3")` equals today's path); `pub fn server_cache_path(maya_dir: &Path, base: &str, model: &str, voice: &str, text: &str) -> PathBuf` = `cache_path(maya_dir, &format!("s{:016x}", hash(&format!("{base}\n{model}\n{voice}"))), text, "wav")`; `speak_remote` now handles `Server` with `voice_server::synthesize`. ElevenLabs curl calls gain `--connect-timeout 3`.
  - In `lib.rs`, replacing `eleven_wanted`/`eleven_settings`:
    - `#[derive(Clone, Debug, PartialEq, Eq)] pub(crate) enum WantedVoice { Elevenlabs { maya_dir: PathBuf, voice_id: String }, Server { maya_dir: PathBuf, base: String, voice: String, model: String } }`.
    - `pub(crate) fn voices_wanted(cfg: &Config, claude_dir: &Path) -> Vec<WantedVoice>` — read under the store lock, no key lookups: ElevenLabs when `cfg.elevenlabs_on()` and the voice id is not blank; then the server when `cfg.voice_server.enabled`, `base_url(&url)` is `Some`, and the voice is not blank. `maya_dir = claude_dir.join("maya")`.
    - `pub(crate) fn voice_chain(wanted: Vec<WantedVoice>) -> Vec<RemoteVoice>` — called outside the lock; ElevenLabs is dropped when no key is stored; the server key is `voice::load_server_key()` (optional).
  - Tauri commands: `set_voice_server_key(key: String) -> Result<(), String>`, `has_voice_server_key() -> bool`, `list_server_voices(state) -> Result<Vec<String>, String>` (reads the config's URL; `base_url` `None` → `Err("Not a server address: start it with http:// or https://.")`), and `try_voice(..) -> Result<String, String>` returning `report.message()` with `trial: true`. `voice::use_elevenlabs` is removed.

- [ ] **Step 1: Write the failing tests**
  - In `voice.rs`: keep `cache_path_is_stable_per_voice_and_text_and_a_safe_file_name` with the new `ext` argument `"mp3"`, and add `server_cache_is_wav_and_keyed_by_address_model_and_voice`: different `base`, `model` or `voice` give different paths; same inputs the same path; the name ends `.wav`, starts with `s`, and is alphanumeric plus `-` and `.`. Rename the `player_command` test message `"aplay cannot decode MP3"` to `"aplay is never used"`.
  - In `lib.rs`, replace `the_elevenlabs_key_is_wanted_only_for_the_elevenlabs_provider_with_a_voice` with `the_wanted_voices_follow_the_switches_in_chain_order`: default config → `[]`; `voiceProvider: elevenlabs` + voice `"v"` → `[Elevenlabs{voice_id:"v"}]`; ElevenLabs on with blank voice → `[]`; server enabled with `url: "http://localhost:8004/"`, voice `"default"` → `[Server{base:"http://localhost:8004", ..}]`; server enabled with `url: "localhost:8004"` → `[]`; server enabled with blank voice → `[]`; both on → ElevenLabs first, server second. Keep `voice_chain(vec![]) == vec![]`.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p maya -- voice:: tests::the_wanted_voices`
Expected: compile errors.

- [ ] **Step 3: Implement** per Interfaces; register the three new commands in the `invoke_handler` list next to the ElevenLabs ones; update the two module doc comments (`voice.rs` now covers both remote voices).

- [ ] **Step 4: Run the app's tests and clippy**

Run: `cargo test -p maya && cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS, no warnings.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/keystore.rs src-tauri/src/voice.rs src-tauri/src/lib.rs src-tauri/src/listener.rs
git commit -m "feat: speak through an OpenAI-compatible server after ElevenLabs"
```

---

### Task 5: Settings shows the chain

**Files:**
- Modify: `src/settings.ts` (types `:103-106`, `:169-175`, `:287-290`; handlers interface `:209-212`; Voice section `:1065-1139`; model defaults `:1472-1475`; `loadVoices` `:1506-1511`; `applyConfig` `:1552-1553`; handlers `:1655-1658`)
- Test: `src/settings.test.ts` (`voiceBase` `:4-14`, the inline models at `:175-227`, the ElevenLabs test at `:237`), `src/settings-flow.test.ts` (configs at `:24`, `:48`, `:73`, `:109`, `:154`, `:205`)

**Interfaces:**
- Consumes: commands `has_elevenlabs_key`, `list_elevenlabs_voices`, `set_elevenlabs_key`, `set_voice_server_key`, `has_voice_server_key`, `list_server_voices`, `try_voice` (→ string) from Task 4; config JSON `elevenlabsEnabled?: boolean | null`, `voiceServer?: { enabled: boolean; url: string; voice: string; model: string }`.
- Produces:
  - Model: remove `voiceProvider`; add `elevenEnabled: boolean`, `serverEnabled: boolean`, `serverUrl: string`, `serverVoice: string`, `serverModel: string`, `serverKeySet: boolean`, `serverVoices: string[]`, `serverVoicesError: string | null`, `voiceReport: string | null`. Remove the `VoiceProvider` type; keep `voiceProvider` in `ConfigJson` as optional (read for the migration only).
  - Handlers: remove `onVoiceProvider`; add `onElevenEnabled(on: boolean)`, `onServerEnabled(on: boolean)`, `onServerUrl(url: string)`, `onServerVoice(voice: string)`, `onServerModel(model: string)`, `onServerKey(key: string)`.
  - `applyConfig`: `elevenEnabled = c.elevenlabsEnabled ?? c.voiceProvider === "elevenlabs"`; server fields from `c.voiceServer` with empty defaults.
  - Saving: `onElevenEnabled` → `saveConfig({ elevenlabsEnabled: on })`; server fields through a `saveServer(patch)` that merges onto `fresh.voiceServer` the way `saveNetwork` merges `network` (`:1581-1587`). `onServerUrl`, `onServerEnabled` and `onServerKey` reload voices.
  - `loadVoices`: ElevenLabs as today but gated on `elevenEnabled`; the server when `serverEnabled && serverUrl`: `serverKeySet = has_voice_server_key`, then `list_server_voices`; a thrown error sets `serverVoices = []` and `serverVoicesError = String(e)` instead of `model.error`.
  - `onTryVoice`: `voiceReport = await invoke<string>("try_voice")`.
- DOM (exact names and copy):
  - The `select[name=voiceProvider]` is gone.
  - `[data-for=voice-order]`: `Maya speaks with the first voice that works: ElevenLabs, then the voice server, then ${builtinVoiceName()} (built in).`
  - `input[type=checkbox][name=elevenEnabled]` labelled `Use ElevenLabs`; when checked, today's `input[name=elevenKey]` and `select[name=elevenVoice]`.
  - `input[type=checkbox][name=serverEnabled]` labelled `Use a voice server (OpenAI-compatible)`; when checked: `input[name=serverUrl]` (label `Server address`, placeholder `http://localhost:8004`); the voice as `select[name=serverVoice]` when `serverVoices` is not empty, else `input[name=serverVoice]` (label `Voice`, placeholder `default`); `[data-for=server-voices]` showing `Could not list the server's voices: ${serverVoicesError}` when set; `input[name=serverModel]` (label `Model`, placeholder `optional`); `input[type=password][name=serverKey]` (label `API key`, placeholder `saved in ${secretStore()}; paste to replace` when `serverKeySet`, else `optional; paste a key`, cleared after change like `elevenKey`).
  - `[data-for=voice-report]` after the Try button, showing `voiceReport` when set.

- [ ] **Step 1: Write the failing tests** in `settings.test.ts`, replacing the ElevenLabs test at `:237`:
  - `shows the voice order and a switch per remote voice`: with both off, the order text is present, `select[name=voiceProvider]` is null, the two checkboxes are unchecked, and no `elevenKey`/`serverUrl` fields exist; ticking `elevenEnabled` calls `onElevenEnabled(true)`; ticking `serverEnabled` calls `onServerEnabled(true)`.
  - `the ElevenLabs section shows its key and voice when on`: `elevenEnabled: true`, voices `[{voiceId:"abc",name:"Rachel"}]` → key field present, voice select value `abc`, change calls `onElevenVoice`.
  - `the server section offers a list when the server has one and a text box when not`: `serverEnabled: true, serverVoices: ["default","tiago.wav"], serverVoice: "tiago.wav"` → `select[name=serverVoice]` with value `tiago.wav`; with `serverVoices: [], serverVoicesError: "404"` → `input[name=serverVoice]` and `[data-for=server-voices]` contains `Could not list the server's voices: 404`; changing `serverUrl` calls `onServerUrl("http://mac.local:8004")`; pasting into `serverKey` calls `onServerKey` and clears the field.
  - `the voice test's report is shown`: `voiceReport: "ElevenLabs failed: 402. Spoke with the voice server."` → `[data-for=voice-report]` has that text.
  - In `settings-flow.test.ts` add `an old ElevenLabs config opens with ElevenLabs switched on`: config `voiceProvider: "elevenlabs", elevenlabsVoiceId: "abc"` and no `elevenlabsEnabled` → the `elevenEnabled` checkbox is checked; with `voiceProvider: "builtin", elevenlabsVoiceId: "abc"` it is unchecked.
  - Replace `voiceProvider: "builtin" as const` in `voiceBase` and the inline models with the new fields (all off, empty, `null`).

- [ ] **Step 2: Run them and see them fail**

Run: `pnpm vitest run src/settings.test.ts src/settings-flow.test.ts`
Expected: FAIL on the new tests.

- [ ] **Step 3: Implement** per Interfaces and DOM.

- [ ] **Step 4: Run the UI tests and the type check**

Run: `pnpm vitest run && pnpm tsc --noEmit`
Expected: PASS, no type errors.

- [ ] **Step 5: Commit**

```bash
git add src/settings.ts src/settings.test.ts src/settings-flow.test.ts
git commit -m "feat: Settings sets up each voice in the chain"
```

---

### Task 6: The Chatterbox server, its container and a CI check

**Files:**
- Create: `chatterbox/server.py`, `chatterbox/Dockerfile`, `chatterbox/compose.yaml`, `chatterbox/voices/.gitkeep`
- Modify: `.gitignore` (ignore `chatterbox/voices/*` except `.gitkeep`)
- Modify: `.github/workflows/ci.yml` (a step in the `version` job, after `Check the updater manifest script`)

**Interfaces:**
- Consumes: the request and voice-list shapes from Task 2 (`request_body`, `parse_voices`).
- Produces: a server Maya talks to:
  - PEP 723 header: `requires-python = ">=3.11,<3.12"`, dependencies exactly the Global Constraints pins. Started with `uv run chatterbox/server.py [--host 127.0.0.1] [--port 8004] [--device cpu|mps|cuda] [--voices <dir>]`; `--voices` defaults to the `voices/` folder next to the script. The key comes only from the env var `CHATTERBOX_API_KEY` (never a flag, so it stays out of `ps`).
  - Loads `ChatterboxTurboTTS.from_pretrained(device=…)` once at start and keeps the built-in conditionals (`model.conds`) as voice `default`.
  - `GET /v1/audio/voices` → `{"status": "ok", "voices": ["default", <file names in the voices folder with .wav/.mp3/.flac extensions, sorted>]}`; the folder is re-read on every call.
  - `POST /v1/audio/speech` with `{"input", "voice"?, "model"?, "response_format"?, "speed"?}`: blank/absent voice or `default` → built-in voice; otherwise the voice must exactly equal a name from the current folder listing (never joined into a path from user input) or it is a 404 `{"error": "unknown voice: <name>"}`. `response_format` other than absent/`wav` → 400 `{"error": "only wav is supported"}`. A clip that is too short → 400 with Chatterbox's message. Prepared conditionals are cached per voice name and file mtime. All generation is behind one `threading.Lock`; the response is `audio/wav`.
  - With `CHATTERBOX_API_KEY` set, both endpoints need `Authorization: Bearer <key>` (checked with `hmac.compare_digest`) or answer 401.
  - On start it prints one line: the address, device, and whether a key is required.
  - `Dockerfile`: `python:3.11-slim`, `apt-get install -y --no-install-recommends libgomp1`, `pip install` the same pins (with a comment to keep them in step with `server.py`'s header), copies `server.py` to `/app`, `CMD ["python", "/app/server.py", "--host", "0.0.0.0"]`.
  - `compose.yaml`: service `chatterbox` built from `.`, ports `"127.0.0.1:8004:8004"` (a comment: use `"8004:8004"` to let other machines in, and set a key), volumes `./voices:/app/voices` and named volume `hf-cache` at `/root/.cache/huggingface`, `environment: CHATTERBOX_API_KEY: ${CHATTERBOX_API_KEY:-}`, `restart: unless-stopped`, and a commented NVIDIA block (`deploy.resources.reservations.devices` with `driver: nvidia`, plus `command` adding `--device cuda`).
  - CI step `Check the Chatterbox server compiles`: `python3 -m py_compile chatterbox/server.py`.

- [ ] **Step 1: Write `server.py`, the Dockerfile, the compose file and the CI step.**

- [ ] **Step 2: Check it compiles**

Run: `python3 -m py_compile chatterbox/server.py && echo ok`
Expected: `ok`.

- [ ] **Step 3: Run it natively and check both endpoints** (downloads models on first run; peak ~6.6 GB of memory)

Run: `uv run chatterbox/server.py` in the background, then `curl -s localhost:8004/v1/audio/voices` and `curl -s -o /tmp/maya-voice.wav -w '%{http_code} %{content_type}\n' -H 'Content-Type: application/json' -d '{"input":"Maya here. Is my voice ok?","voice":"default","response_format":"wav"}' localhost:8004/v1/audio/speech` and `afplay /tmp/maya-voice.wav`.
Expected: `{"status":"ok","voices":["default"]}`; `200 audio/wav`; the line plays. Then `{"voice":"../server.py"}` → 404, `{"response_format":"mp3"}` → 400, and with `CHATTERBOX_API_KEY=k` a request without the header → 401 and with `Authorization: Bearer k` → 200.

- [ ] **Step 4: Run it under compose**

Run: `docker compose -f chatterbox/compose.yaml up -d --build`, wait for the start line in `docker compose -f chatterbox/compose.yaml logs`, repeat Step 3's voices and speech requests, then `docker compose -f chatterbox/compose.yaml down`.
Expected: the same answers (slower; the VM needs ≥ 10 GB of memory).

- [ ] **Step 5: Commit**

```bash
git add chatterbox .gitignore .github/workflows/ci.yml
git commit -m "feat: a Chatterbox speech server for Maya, native or in Docker"
```

---

### Task 7: The guide and the docs

**Files:**
- Create: `docs/chatterbox.md`
- Modify: `README.md` (Voice section, `:237-262`: the chain, and the volume bullet's "for the built-in voice and ElevenLabs alike")
- Modify: `docs/DEVELOPING.md` (Layout: `core/src/` gains `voice_server.rs`; `src-tauri/src/` `voice.rs` becomes "(ElevenLabs and the voice server: cache and playback)"; a new `chatterbox/` bullet)

**Interfaces:**
- Consumes: the server's flags, env var, endpoints and defaults (Task 6); the Settings copy (Task 5); the spike numbers in the spec.

- [ ] **Step 1: Write `docs/chatterbox.md`** with these sections:
  - **What you get**: the chain (ElevenLabs → server → built-in), that any OpenAI-compatible server works, and that the phone keeps Android's voice.
  - **Run it natively (recommended on a Mac)**: install `uv`; from a checkout `uv run chatterbox/server.py`, or without one `curl -O https://raw.githubusercontent.com/dosaki/maya/main/chatterbox/server.py && uv run server.py`; first start downloads the model; ~6.6 GB of memory; CPU is the default because it beat the Apple GPU (table from the spec); `--device cuda` on NVIDIA.
  - **Start it at login (macOS)**: a `~/Library/LaunchAgents/com.dosaki.maya.chatterbox.plist` with `ProgramArguments` (`uv` path from `which uv`, `run`, the script path), `RunAtLoad` and `KeepAlive` true, log paths under `~/Library/Logs/`, then `launchctl bootstrap gui/$(id -u) <plist>`; a note that a key in `EnvironmentVariables` is stored in plain text in that file.
  - **Run it with Docker or Podman**: `docker compose -f chatterbox/compose.yaml up -d`; the memory note (give the VM ≥ 10 GB; `podman machine set --memory 12288`); on a Mac it is ~2.5× slower than native (numbers); the NVIDIA block.
  - **Add a voice**: drop a clip of 10 s or more (`.wav`, `.mp3` or `.flac`) in `chatterbox/voices/`; its file name with extension is the voice name; no restart needed.
  - **Point Maya at it**: Settings → Voice → `Use a voice server (OpenAI-compatible)`, `Server address` `http://localhost:8004`, pick the voice, `Try the voice`.
  - **Use it from another machine**: start with `--host 0.0.0.0` (or the compose port change) and `CHATTERBOX_API_KEY`; on the other machine use `http://<this Mac's address>:8004` and paste the key; when this Mac is unreachable, Maya falls back to the built-in voice and retries after 5 minutes.
  - **What to expect**: ~2 s before a short new line, instant for a cached one; no streaming; every clip carries Resemble's inaudible watermark.
  - **Troubleshooting**: `'NoneType' object is not callable` → `setuptools<80` (already pinned); `libgomp.so.1` in a custom image; a container killed while loading → VM memory.
- [ ] **Step 2: Update the README Voice section and DEVELOPING Layout**, linking `docs/chatterbox.md`.
- [ ] **Step 3: Check the links**

Run: `grep -n "chatterbox.md" README.md docs/DEVELOPING.md && test -f docs/chatterbox.md && echo ok`
Expected: the links and `ok`.

- [ ] **Step 4: Commit**

```bash
git add docs/chatterbox.md README.md docs/DEVELOPING.md
git commit -m "docs: set up a Chatterbox voice for Maya"
```

---

### Task 8: Release version

**Files:**
- Modify (by script): `src-tauri/tauri.conf.json`, `package.json`, `Cargo.toml`, `Cargo.lock`, and any other file the script keeps in step

- [ ] **Step 1: Check `main`'s version**

Run: `git fetch -q && git show origin/main:package.json | grep '"version"'`
Expected: `"version": "0.30.0"`. If it moved, bump from that version instead (a `feat` → next minor).

- [ ] **Step 2: Bump**

Run: `sh scripts/set-version.sh 0.31.0 && git status --short`
Expected: the version files changed, **including `Cargo.lock`**; if `Cargo.lock` did not change, run `cargo update -w` and check again.

- [ ] **Step 3: Full check**

Run: `cargo test --workspace && pnpm vitest run && pnpm tsc --noEmit`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git commit -am "chore(release): 0.31.0"
```
