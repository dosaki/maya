# Eye Reply Modal Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Terminal and Reply buttons to each card, and a modal showing the session's recent conversation with a composer that posts a message into the session's inbox socket.

**Architecture:** Registry gains the socket path; transcript gains a structured turn reader; a new `inbox.rs` writes one stream-json line to the session's Unix socket after checking the peer pid; two Tauri commands expose history and send. The frontend adds card buttons and a `modal.ts` renderer wired in `main.ts`.

**Tech Stack:** as the first plan (Rust, Tauri 2, libc, Vitest).

**Spec:** `docs/superpowers/specs/2026-09-28-eye-reply-modal-design.md`

## Global Constraints

- Message line format, exactly: `{"type":"user","message":{"role":"user","content":"<text>"}}` followed by `\n`. No auth line.
- Refuse to write when the socket's peer pid is not the session pid.
- History: last 30 turns, from the last 1 MB of the transcript.
- Tool summary fields in priority order: `command`, `file_path`, `path`, `prompt`, `description`, `questions[0].question`; trimmed to 120 chars with `…`.
- Text over 100 000 characters is refused before connecting.
- All commands are `#[tauri::command(async)]`.
- Commit per task, conventional messages, `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.

## Review Focus

1. Session exits while the modal is open, then the user presses Send: `send_reply` returns "Session is no longer running." and the UI shows it; no panic. Test in Task 4.
2. Transcript line for a user turn whose content is an array of only `tool_result` blocks: not a turn. Test in Task 2.
3. A tool_use with none of the summary fields (e.g. `ListAgents` with `{}`): summary is just the tool name. Test in Task 2.
4. Newlines and quotes in the reply text: JSON-escaped, arrive intact. Test in Task 3.
5. Clicking a card's Terminal button must not also open the modal (event bubbling). Test in Task 5.

---

### Task 1: Socket path in registry and `hasInbox` on the card

**Files:**
- Modify: `src-tauri/src/registry.rs`, `src-tauri/src/model.rs`, `src-tauri/src/state.rs`, `src/types.ts`

**Interfaces:**
- Produces: `RegistrySession.messaging_socket_path: Option<String>` (serde `messagingSocketPath`), `Card.has_inbox: bool` (JSON `hasInbox`), TS `Card.hasInbox: boolean`.

- [ ] **Step 1: Failing tests**

In `registry.rs` tests, extend `parses_real_registry_file` with:
```rust
        assert_eq!(s.messaging_socket_path.as_deref(), Some("/tmp/cc-socks/49643.sock"));
```
In `state.rs` tests add:
```rust
    #[test]
    fn card_reports_whether_the_session_has_an_inbox() {
        let mut r = reg("idle", 0);
        assert!(!run(&r, &[], &TranscriptTail::default(), 1).has_inbox);
        r.messaging_socket_path = Some("/tmp/cc-socks/7.sock".into());
        assert!(run(&r, &[], &TranscriptTail::default(), 1).has_inbox);
    }
```
Run `cargo test` → compile errors on the missing fields.

- [ ] **Step 2: Implement**

`registry.rs` struct: add
```rust
    #[serde(rename = "messagingSocketPath", default)]
    pub messaging_socket_path: Option<String>,
```
`model.rs` `Card`: add `pub has_inbox: bool,` after `awaiting`. Update the model test literal with `has_inbox: false`.
`state.rs` `derive`: `has_inbox: r.messaging_socket_path.is_some(),`; test helper `reg()` gets `messaging_socket_path: None,`.
`store.rs` test registry JSON needs nothing (field defaults to None).
`src/types.ts` `Card`: add `hasInbox: boolean;`. `src/board.test.ts` `card()` helper: add `hasInbox: true,`.

- [ ] **Step 3: Verify and commit**

`cargo test` all green; `pnpm test` green.
```bash
git add -A src src-tauri/src && git commit -m "feat: expose session inbox availability on cards

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: Transcript turns

**Files:**
- Create: `src-tauri/fixtures/transcript/turns.jsonl`
- Modify: `src-tauri/src/transcript.rs`

**Interfaces:**
- Produces: `transcript::Turn { kind: TurnKind, text: String }`, `TurnKind::{User, Assistant, Tool}` (serde lowercase), `transcript::parse_turns(text: &str, max_turns: usize) -> Vec<Turn>`, `transcript::read_turns(path: &Path, max_turns: usize) -> Vec<Turn>` (last 1 MB), `transcript::tool_summary(name: &str, input: &Value) -> String`.

- [ ] **Step 1: Fixture** `src-tauri/fixtures/transcript/turns.jsonl`:

```
{"type":"user","message":{"role":"user","content":"build me a board"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Sure."},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"pnpm test","description":"Run tests"}}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t2","name":"ListAgents","input":{}}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t2","content":"none"}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Tests pass."},{"type":"text","text":"Anything else?"}]}}
{"type":"user","isSidechain":true,"message":{"role":"user","content":"subagent prompt, hidden"}}
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"yes, ship it"}]}}
```

- [ ] **Step 2: Failing tests** in `transcript.rs`:

```rust
    #[test]
    fn parses_turns_in_order_skipping_tool_results_and_sidechains() {
        let turns = parse_turns(&fixture("turns.jsonl"), 30);
        let got: Vec<(TurnKind, &str)> = turns.iter().map(|t| (t.kind, t.text.as_str())).collect();
        assert_eq!(got, vec![
            (TurnKind::User, "build me a board"),
            (TurnKind::Assistant, "Sure."),
            (TurnKind::Tool, "Bash: pnpm test"),
            (TurnKind::Tool, "ListAgents"),
            (TurnKind::Assistant, "Tests pass.\n\nAnything else?"),
            (TurnKind::User, "yes, ship it"),
        ]);
    }

    #[test]
    fn caps_to_the_last_n_turns() {
        let turns = parse_turns(&fixture("turns.jsonl"), 2);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[1].text, "yes, ship it");
    }

    #[test]
    fn tool_summary_prefers_command_then_paths_then_question() {
        use serde_json::json;
        assert_eq!(tool_summary("Bash", &json!({"command": "ls", "description": "d"})), "Bash: ls");
        assert_eq!(tool_summary("Edit", &json!({"file_path": "/a.rs"})), "Edit: /a.rs");
        assert_eq!(tool_summary("Agent", &json!({"prompt": "x".repeat(300)})).chars().count(), "Agent: ".len() + 120);
        assert_eq!(tool_summary("AskUserQuestion", &json!({"questions": [{"question": "Which?"}]})), "AskUserQuestion: Which?");
        assert_eq!(tool_summary("ListAgents", &json!({})), "ListAgents");
    }

    #[test]
    fn read_turns_missing_file_is_empty() {
        assert!(read_turns(Path::new("/nonexistent/x.jsonl"), 30).is_empty());
    }
```
`cargo test transcript` → compile errors.

- [ ] **Step 3: Implement** (above tests, after `TranscriptTail`):

```rust
pub const TURNS_TAIL_BYTES: u64 = 1_048_576;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TurnKind { User, Assistant, Tool }

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Turn { pub kind: TurnKind, pub text: String }

pub fn tool_summary(name: &str, input: &Value) -> String {
    let arg = ["command", "file_path", "path", "prompt", "description"]
        .iter()
        .find_map(|k| input.get(*k).and_then(|v| v.as_str()))
        .or_else(|| input["questions"][0]["question"].as_str());
    match arg {
        Some(a) if !a.trim().is_empty() => format!("{name}: {}", crate::state::truncate(a.trim(), 120)),
        _ => name.to_string(),
    }
}

fn text_blocks(content: &Value) -> Vec<String> {
    match content {
        Value::String(s) => vec![s.trim().to_string()].into_iter().filter(|s| !s.is_empty()).collect(),
        Value::Array(blocks) => blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect(),
        _ => vec![],
    }
}

pub fn parse_turns(text: &str, max_turns: usize) -> Vec<Turn> {
    let mut turns = Vec::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if v["isSidechain"].as_bool() == Some(true) { continue; }
        let content = &v["message"]["content"];
        match v["type"].as_str() {
            Some("user") => {
                let t = text_blocks(content);
                if !t.is_empty() { turns.push(Turn { kind: TurnKind::User, text: t.join("\n\n") }); }
            }
            Some("assistant") => {
                let t = text_blocks(content);
                if !t.is_empty() { turns.push(Turn { kind: TurnKind::Assistant, text: t.join("\n\n") }); }
                if let Some(blocks) = content.as_array() {
                    for b in blocks.iter().filter(|b| b["type"] == "tool_use") {
                        let name = b["name"].as_str().unwrap_or("tool");
                        turns.push(Turn { kind: TurnKind::Tool, text: tool_summary(name, &b["input"]) });
                    }
                }
            }
            _ => {}
        }
    }
    let skip = turns.len().saturating_sub(max_turns);
    turns.split_off(skip)
}

pub fn read_turns(path: &Path, max_turns: usize) -> Vec<Turn> {
    let Some(text) = tail_text(path, TURNS_TAIL_BYTES) else { return vec![] };
    parse_turns(&text, max_turns)
}
```
Refactor `read_tail` to share a `tail_text(path, max_bytes) -> Option<String>` helper that does the seek, read and first-partial-line drop; `read_tail` becomes `tail_text(path, max_bytes).map(|t| parse_tail(&t)).unwrap_or_default()`. Existing transcript tests must stay green.

Note: the text-block filter in `text_blocks` also drops the block when assistant text is inside a user entry that starts with `<local-command` or similar; not needed now.

- [ ] **Step 4: Verify and commit**

`cargo test` green.
```bash
git add src-tauri/src/transcript.rs src-tauri/fixtures/transcript/turns.jsonl && git commit -m "feat: structured conversation turns from transcripts

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 3: Inbox client

**Files:**
- Create: `src-tauri/src/inbox.rs`
- Modify: `src-tauri/src/lib.rs` (`pub mod inbox;`)

**Interfaces:**
- Produces: `inbox::message_line(text: &str) -> String` (the JSON line with trailing `\n`), `inbox::MAX_CHARS: usize = 100_000`, `inbox::peer_pid(stream: &UnixStream) -> Option<i32>`, `inbox::send(socket_path: &Path, expected_pid: i32, text: &str) -> Result<(), String>`.

- [ ] **Step 1: Failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::net::UnixListener;

    fn listen(dir: &Path) -> (UnixListener, std::path::PathBuf) {
        let p = dir.join("s.sock");
        (UnixListener::bind(&p).unwrap(), p)
    }

    #[test]
    fn message_line_is_stream_json_user_message() {
        assert_eq!(message_line("hi"), "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n");
        let v: serde_json::Value = serde_json::from_str(message_line("a \"q\"\nb").trim()).unwrap();
        assert_eq!(v["message"]["content"], "a \"q\"\nb");
    }

    #[test]
    fn send_writes_exactly_one_line_to_the_socket() {
        let dir = tempfile::tempdir().unwrap();
        let (listener, path) = listen(dir.path());
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = String::new();
            s.read_to_string(&mut buf).unwrap();
            buf
        });
        send(&path, std::process::id() as i32, "line one\nline \"two\"").unwrap();
        assert_eq!(server.join().unwrap(), message_line("line one\nline \"two\""));
    }

    #[test]
    fn refuses_wrong_peer_pid_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let (listener, path) = listen(dir.path());
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = String::new();
            s.read_to_string(&mut buf).unwrap();
            buf
        });
        let err = send(&path, 1, "x").unwrap_err();
        assert!(err.contains("not owned by that session"), "{err}");
        assert_eq!(server.join().unwrap(), "");
    }

    #[test]
    fn refuses_missing_socket_empty_and_oversized_text() {
        let dir = tempfile::tempdir().unwrap();
        assert!(send(&dir.path().join("nope.sock"), 1, "x").is_err());
        assert!(send(&dir.path().join("nope.sock"), 1, "  ").unwrap_err().contains("empty"));
        assert!(send(&dir.path().join("nope.sock"), 1, &"x".repeat(MAX_CHARS + 1)).unwrap_err().contains("too long"));
    }
}
```
`cargo test inbox` → compile errors.

- [ ] **Step 2: Implement**

```rust
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

pub const MAX_CHARS: usize = 100_000;

/// One stream-json user message, newline-terminated.
pub fn message_line(text: &str) -> String {
    let v = serde_json::json!({"type": "user", "message": {"role": "user", "content": text}});
    format!("{}\n", v)
}

/// Pid of the process on the other end of a Unix socket (macOS LOCAL_PEERPID).
pub fn peer_pid(stream: &UnixStream) -> Option<i32> {
    const SOL_LOCAL: libc::c_int = 0;
    const LOCAL_PEERPID: libc::c_int = 0x002;
    let mut pid: libc::pid_t = 0;
    let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    let r = unsafe {
        libc::getsockopt(stream.as_raw_fd(), SOL_LOCAL, LOCAL_PEERPID, &mut pid as *mut _ as *mut libc::c_void, &mut len)
    };
    if r == 0 { Some(pid) } else { None }
}

pub fn send(socket_path: &Path, expected_pid: i32, text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("Message is empty.".into());
    }
    if text.chars().count() > MAX_CHARS {
        return Err(format!("Message is too long (over {MAX_CHARS} characters)."));
    }
    let stream = UnixStream::connect(socket_path).map_err(|e| format!("Could not connect to the session inbox: {e}."))?;
    stream.set_write_timeout(Some(Duration::from_secs(5))).map_err(|e| e.to_string())?;
    match peer_pid(&stream) {
        Some(p) if p == expected_pid => {}
        Some(_) => return Err("Refusing to send: the socket is not owned by that session.".into()),
        None => return Err("Refusing to send: could not verify who owns the socket.".into()),
    }
    let mut stream = stream;
    stream.write_all(message_line(text).as_bytes()).map_err(|e| format!("Could not write to the session inbox: {e}."))?;
    stream.flush().map_err(|e| e.to_string())?;
    let _ = stream.shutdown(std::net::Shutdown::Write);
    Ok(())
}
```
Add `pub mod inbox;` to `lib.rs`.

- [ ] **Step 3: Verify and commit**

`cargo test inbox` → 4 pass; full `cargo test` green.
```bash
git add src-tauri/src/inbox.rs src-tauri/src/lib.rs && git commit -m "feat: inbox client that posts a stream-json line to a session socket

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 4: Commands `session_history` and `send_reply`

**Files:**
- Modify: `src-tauri/src/store.rs`, `src-tauri/src/lib.rs`

**Interfaces:**
- Produces: `Store::session(&self, session_id) -> Option<RegistrySession>` (live registry lookup), `Store::transcript_path_for(&self, s: &RegistrySession) -> PathBuf`; commands `session_history(session_id: String) -> Result<Vec<Turn>, String>`, `send_reply(session_id: String, text: String) -> Result<(), String>`.

- [ ] **Step 1: Failing store test**

```rust
    #[test]
    fn session_lookup_and_transcript_path_prefer_hook_supplied_path() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().to_path_buf();
        std::fs::create_dir_all(claude.join("sessions")).unwrap();
        std::fs::create_dir_all(claude.join("eye")).unwrap();
        std::fs::write(claude.join("sessions/7.json"), r#"{"pid":7,"sessionId":"s7","cwd":"/Users/x/dev/eye","name":"eye-7","status":"idle"}"#).unwrap();
        std::fs::write(claude.join("eye/events.jsonl"), "{\"session_id\":\"s7\",\"hook_event_name\":\"Stop\",\"transcript_path\":\"/hooked/s7.jsonl\",\"received_at\":1}\n").unwrap();
        let mut store = Store::new(claude.clone()).with_alive(|_| true);
        store.refresh(2);
        let s = store.session("s7").expect("live session");
        assert_eq!(store.transcript_path_for(&s), PathBuf::from("/hooked/s7.jsonl"));
        assert!(store.session("nope").is_none());
    }
```
`cargo test store` → compile error.

- [ ] **Step 2: Implement**

In `store.rs`:
```rust
    pub fn session(&self, session_id: &str) -> Option<RegistrySession> {
        self.registry().into_iter().find(|s| s.session_id == session_id)
    }

    pub fn transcript_path_for(&self, s: &RegistrySession) -> PathBuf {
        match self.events.transcript_path_for(&s.session_id) {
            Some(p) => PathBuf::from(p),
            None => registry::transcript_path(&self.claude_dir, s),
        }
    }
```
and use `transcript_path_for` inside `refresh` instead of the inline match.

In `lib.rs`:
```rust
#[tauri::command(async)]
fn session_history(state: TauriState<AppState>, session_id: String) -> Result<Vec<transcript::Turn>, String> {
    let (path,) = {
        let store = state.store.lock().unwrap();
        let s = store.session(&session_id).ok_or("Session is no longer running.")?;
        (store.transcript_path_for(&s),)
    };
    Ok(transcript::read_turns(&path, 30))
}

#[tauri::command(async)]
fn send_reply(state: TauriState<AppState>, session_id: String, text: String) -> Result<(), String> {
    let (socket, pid) = {
        let store = state.store.lock().unwrap();
        let s = store.session(&session_id).ok_or("Session is no longer running.")?;
        let socket = s.messaging_socket_path.clone().ok_or("This session has no inbox. Use the terminal.")?;
        (socket, s.pid)
    };
    inbox::send(std::path::Path::new(&socket), pid, &text)
}
```
Register both in `generate_handler!`. The mutex is released before any socket or file I/O.

- [ ] **Step 3: Verify and commit**

`cargo test` green; `pnpm tauri build --debug --no-bundle` succeeds.
```bash
git add src-tauri/src && git commit -m "feat: session_history and send_reply commands

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 5: Card buttons and the modal

**Files:**
- Create: `src/modal.ts`, `src/modal.test.ts`
- Modify: `src/card.ts`, `src/board.test.ts`, `src/main.ts`, `src/types.ts`, `src/styles.css`, `index.html`

**Interfaces:**
- `types.ts`: `Turn { kind: "user" | "assistant" | "tool"; text: string }`.
- `card.ts`: buttons `button.card__btn[data-action=terminal]` and `[data-action=reply]` inside `.card__actions`.
- `modal.ts`:
  - `ModalModel { card: Card; turns: Turn[]; status: { ok: boolean; text: string } | null; draft: string }`
  - `ModalHandlers { onSend(text: string): void; onTerminal(): void; onClose(): void }`
  - `renderModal(model: ModalModel, h: ModalHandlers): HTMLElement` (pure; root `.modal` with `.modal__backdrop`, `.modal__panel`)
  - `openModal(card: Card): Promise<void>` and `closeModal(): void` and `refreshModal(cards: Card[]): void` (stateful, uses `#modal-host`).

- [ ] **Step 1: Failing tests**

Append to `src/board.test.ts`:
```ts
describe("card actions", () => {
  it("renders Terminal and Reply buttons", () => {
    const el = renderCard(card({}), NOW);
    expect(el.querySelector("button[data-action=terminal]")?.textContent).toBe("Terminal");
    expect(el.querySelector("button[data-action=reply]")?.textContent).toBe("Reply");
  });
});
```
Create `src/modal.test.ts`:
```ts
import { describe, expect, it, vi } from "vitest";
import { renderModal } from "./modal";
import type { Card, Turn } from "./types";

const base: Card = { sessionId: "s", pid: 1, name: "eye-1", cwd: "/x/dev/eye", state: "idle", stateSince: 0, snippet: "", awaiting: null, hasInbox: true };
const turns: Turn[] = [{ kind: "user", text: "hi" }, { kind: "assistant", text: "hello" }, { kind: "tool", text: "Bash: ls" }];
const handlers = () => ({ onSend: vi.fn(), onTerminal: vi.fn(), onClose: vi.fn() });

describe("renderModal", () => {
  it("shows header, turns with kind classes and a composer", () => {
    const h = handlers();
    const el = renderModal({ card: base, turns, status: null, draft: "" }, h);
    expect(el.querySelector(".modal__title")?.textContent).toBe("eye-1");
    expect(el.querySelector(".modal__project")?.textContent).toBe("eye");
    expect(el.querySelector(".modal__state")?.textContent).toBe("Idle");
    const kinds = [...el.querySelectorAll(".turn")].map((t) => t.className);
    expect(kinds).toEqual(["turn turn--user", "turn turn--assistant", "turn turn--tool"]);
    expect(el.querySelector("textarea")).not.toBeNull();
    expect(el.querySelector(".modal__banner")).toBeNull();
  });

  it("sends trimmed text on button click and on Cmd+Enter, never blank", () => {
    const h = handlers();
    const el = renderModal({ card: base, turns: [], status: null, draft: "" }, h);
    const ta = el.querySelector<HTMLTextAreaElement>("textarea")!;
    ta.value = "   ";
    el.querySelector<HTMLButtonElement>("button[data-action=send]")!.click();
    expect(h.onSend).not.toHaveBeenCalled();
    ta.value = "  reply please  ";
    ta.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", metaKey: true, bubbles: true }));
    expect(h.onSend).toHaveBeenCalledWith("reply please");
  });

  it("shows the awaiting banner with a terminal button, and a status line", () => {
    const h = handlers();
    const el = renderModal({ card: { ...base, state: "awaiting", awaiting: { kind: "permission", detail: "Bash: rm" } }, turns: [], status: { ok: false, text: "boom" }, draft: "" }, h);
    expect(el.querySelector(".modal__banner")?.textContent).toContain("waiting for a decision");
    el.querySelector<HTMLButtonElement>(".modal__banner button")!.click();
    expect(h.onTerminal).toHaveBeenCalled();
    expect(el.querySelector(".modal__status")?.textContent).toBe("boom");
    expect(el.querySelector(".modal__status")?.className).toContain("modal__status--error");
  });

  it("replaces the composer when the session has no inbox", () => {
    const el = renderModal({ card: { ...base, hasInbox: false }, turns: [], status: null, draft: "" }, handlers());
    expect(el.querySelector("textarea")).toBeNull();
    expect(el.querySelector(".modal__noinbox")?.textContent).toContain("no inbox");
  });

  it("closes on backdrop click and on the close button", () => {
    const h = handlers();
    const el = renderModal({ card: base, turns: [], status: null, draft: "" }, h);
    el.querySelector<HTMLElement>(".modal__backdrop")!.click();
    el.querySelector<HTMLButtonElement>("button[data-action=close]")!.click();
    expect(h.onClose).toHaveBeenCalledTimes(2);
  });
});
```
`pnpm test` → fails to resolve `./modal`, card test fails.

- [ ] **Step 2: Implement**

`src/types.ts`: add `export interface Turn { kind: "user" | "assistant" | "tool"; text: string }` and `export const STATE_LABEL: Record<CardState, string> = { awaiting: "Awaiting Decision", working: "Working", completed: "Completed", idle: "Idle" };`.

`src/card.ts`: after the snippet, append
```ts
  const actions = el("div", "card__actions");
  const terminal = el("button", "card__btn", "Terminal");
  terminal.type = "button";
  terminal.dataset.action = "terminal";
  const reply = el("button", "card__btn card__btn--primary", "Reply");
  reply.type = "button";
  reply.dataset.action = "reply";
  actions.append(terminal, reply);
  root.append(actions);
```

`src/modal.ts`:
```ts
import { invoke } from "@tauri-apps/api/core";
import { projectName } from "./format";
import { STATE_LABEL, type Card, type Turn } from "./types";

export interface ModalModel { card: Card; turns: Turn[]; status: { ok: boolean; text: string } | null; draft: string }
export interface ModalHandlers { onSend(text: string): void; onTerminal(): void; onClose(): void }

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const n = document.createElement(tag);
  n.className = className;
  if (text !== undefined) n.textContent = text;
  return n;
}

export function renderModal(m: ModalModel, h: ModalHandlers): HTMLElement {
  const root = el("div", "modal");
  const backdrop = el("div", "modal__backdrop");
  backdrop.addEventListener("click", () => h.onClose());
  const panel = el("section", "modal__panel");
  panel.setAttribute("role", "dialog");
  panel.setAttribute("aria-modal", "true");

  const head = el("header", "modal__head");
  const titles = el("div", "modal__titles");
  titles.append(el("h2", "modal__title", m.card.name), el("span", "modal__project", projectName(m.card.cwd)));
  const state = el("span", `modal__state modal__state--${m.card.state}`, STATE_LABEL[m.card.state]);
  const close = el("button", "modal__close", "×");
  close.type = "button";
  close.dataset.action = "close";
  close.setAttribute("aria-label", "Close");
  close.addEventListener("click", () => h.onClose());
  head.append(titles, state, close);
  panel.append(head);

  if (m.card.state === "awaiting") {
    const banner = el("div", "modal__banner");
    banner.append(el("span", "", "This session is waiting for a decision in its terminal. A reply will queue behind it."));
    const open = el("button", "card__btn", "Open terminal");
    open.type = "button";
    open.addEventListener("click", () => h.onTerminal());
    banner.append(open);
    panel.append(banner);
  }

  const history = el("div", "modal__history");
  if (m.turns.length === 0) history.append(el("div", "modal__empty", "No transcript found."));
  for (const t of m.turns) {
    const turn = el("div", `turn turn--${t.kind}`);
    if (t.kind !== "tool") turn.append(el("div", "turn__who", t.kind === "user" ? "You" : "Claude"));
    turn.append(el("div", "turn__text", t.text));
    history.append(turn);
  }
  panel.append(history);

  if (m.card.hasInbox) {
    const form = el("div", "modal__composer");
    const ta = el("textarea", "modal__input");
    ta.placeholder = "Message this session… (⌘↵ to send)";
    ta.value = m.draft;
    ta.rows = 3;
    const trySend = () => {
      const text = ta.value.trim();
      if (text) h.onSend(text);
    };
    ta.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter" && (ev.metaKey || ev.ctrlKey)) { ev.preventDefault(); trySend(); }
    });
    const send = el("button", "card__btn card__btn--primary", "Send");
    send.type = "button";
    send.dataset.action = "send";
    send.addEventListener("click", trySend);
    form.append(ta, send);
    panel.append(form);
  } else {
    panel.append(el("div", "modal__noinbox", "This session has no inbox. Use the terminal."));
  }
  if (m.status) panel.append(el("div", `modal__status modal__status--${m.status.ok ? "ok" : "error"}`, m.status.text));

  root.append(backdrop, panel);
  return root;
}

let current: { model: ModalModel; keyHandler: (e: KeyboardEvent) => void } | null = null;

function paint(): void {
  const host = document.getElementById("modal-host");
  if (!host || !current) return;
  const m = current.model;
  const ta = host.querySelector<HTMLTextAreaElement>("textarea");
  if (ta) m.draft = ta.value;
  host.replaceChildren(renderModal(m, {
    onSend: (text) => void send(text),
    onTerminal: () => void invoke("focus_session", { pid: m.card.pid }).catch((e) => setStatus(false, String(e))),
    onClose: closeModal,
  }));
  const hist = host.querySelector(".modal__history");
  if (hist) hist.scrollTop = hist.scrollHeight;
  host.querySelector<HTMLTextAreaElement>("textarea")?.focus();
}

function setStatus(ok: boolean, text: string): void {
  if (!current) return;
  current.model.status = { ok, text };
  paint();
}

async function send(text: string): Promise<void> {
  if (!current) return;
  const { card } = current.model;
  try {
    await invoke("send_reply", { sessionId: card.sessionId, text });
    current.model.draft = "";
    setStatus(true, "Delivered");
    await loadTurns();
  } catch (e) {
    setStatus(false, String(e));
  }
}

async function loadTurns(): Promise<void> {
  if (!current) return;
  const { card } = current.model;
  try {
    current.model.turns = await invoke<Turn[]>("session_history", { sessionId: card.sessionId });
  } catch (e) {
    current.model.turns = [];
    current.model.status = { ok: false, text: String(e) };
  }
  paint();
}

export async function openModal(card: Card): Promise<void> {
  closeModal();
  const keyHandler = (e: KeyboardEvent) => { if (e.key === "Escape") closeModal(); };
  current = { model: { card, turns: [], status: null, draft: "" }, keyHandler };
  document.addEventListener("keydown", keyHandler);
  paint();
  await loadTurns();
}

export function closeModal(): void {
  if (!current) return;
  document.removeEventListener("keydown", current.keyHandler);
  current = null;
  document.getElementById("modal-host")?.replaceChildren();
}

/** Called on every board refresh: keeps the open modal's card and history current. */
export function refreshModal(cards: Card[]): void {
  if (!current) return;
  const fresh = cards.find((c) => c.sessionId === current!.model.card.sessionId);
  if (!fresh) { setStatus(false, "Session is no longer running."); return; }
  current.model.card = fresh;
  void loadTurns();
}
```

`index.html`: add `<div id="modal-host"></div>` before the toast div.

`src/main.ts`: replace the click and keydown handlers with
```ts
  const board = document.getElementById("board");
  board?.addEventListener("click", (ev) => {
    const target = ev.target as HTMLElement;
    const cardEl = target.closest<HTMLElement>(".card");
    if (!cardEl) return;
    const card = cards.find((c) => c.sessionId === cardEl.dataset.sessionId);
    if (!card) return;
    const action = target.closest<HTMLElement>("[data-action]")?.dataset.action;
    if (action === "terminal") { void focus(card.pid); return; }
    void openModal(card);
  });
  board?.addEventListener("keydown", (ev) => {
    if (ev.key !== "Enter") return;
    const cardEl = (ev.target as HTMLElement).closest<HTMLElement>(".card");
    const card = cards.find((c) => c.sessionId === cardEl?.dataset.sessionId);
    if (card) void openModal(card);
  });
```
and in the `sessions` listener, after `paint()`, call `refreshModal(cards)`. Import `openModal, refreshModal` from `./modal`.

`src/styles.css`: append
```css
.card__actions { display: flex; gap: 6px; margin-top: 10px; }
.card__btn { background: var(--bg); border: 1px solid var(--border); border-radius: 6px; padding: 4px 10px; color: var(--text); cursor: pointer; font-size: 12px; }
.card__btn:hover { border-color: var(--working); }
.card__btn--primary { background: var(--working); border-color: var(--working); color: #fff; }
.modal { position: fixed; inset: 0; z-index: 30; display: flex; align-items: center; justify-content: center; }
.modal__backdrop { position: absolute; inset: 0; background: rgba(0,0,0,.45); }
.modal__panel { position: relative; width: min(760px, 92vw); height: min(80vh, 720px); background: var(--panel); border: 1px solid var(--border); border-radius: 12px; box-shadow: 0 20px 60px rgba(0,0,0,.35); display: flex; flex-direction: column; overflow: hidden; }
.modal__head { display: flex; align-items: center; gap: 12px; padding: 12px 16px; border-bottom: 1px solid var(--border); }
.modal__titles { flex: 1; display: flex; align-items: baseline; gap: 8px; min-width: 0; }
.modal__title { margin: 0; font-size: 15px; }
.modal__project { color: var(--muted); font-size: 12px; }
.modal__state { font-size: 11px; text-transform: uppercase; letter-spacing: .04em; padding: 2px 8px; border-radius: 999px; border: 1px solid var(--idle); color: var(--muted); }
.modal__state--awaiting { border-color: var(--awaiting); color: var(--awaiting); }
.modal__state--working { border-color: var(--working); color: var(--working); }
.modal__state--completed { border-color: var(--completed); color: var(--completed); }
.modal__close { background: none; border: none; font-size: 20px; color: var(--muted); cursor: pointer; }
.modal__banner { display: flex; align-items: center; gap: 12px; padding: 10px 16px; background: color-mix(in srgb, var(--awaiting) 14%, transparent); border-bottom: 1px solid var(--border); font-size: 13px; }
.modal__banner span { flex: 1; }
.modal__history { flex: 1; overflow-y: auto; padding: 12px 16px; display: flex; flex-direction: column; gap: 10px; }
.modal__empty { color: var(--muted); font-style: italic; text-align: center; padding: 24px; }
.turn { max-width: 88%; padding: 8px 12px; border-radius: 10px; font-size: 13px; line-height: 1.45; white-space: pre-wrap; word-break: break-word; }
.turn--user { align-self: flex-end; background: color-mix(in srgb, var(--working) 18%, var(--panel)); }
.turn--assistant { align-self: flex-start; background: color-mix(in srgb, var(--panel) 70%, var(--bg)); border: 1px solid var(--border); }
.turn--tool { align-self: flex-start; color: var(--muted); font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 11.5px; padding: 2px 8px; }
.turn__who { font-size: 11px; color: var(--muted); margin-bottom: 2px; }
.modal__composer { display: flex; gap: 8px; padding: 12px 16px; border-top: 1px solid var(--border); }
.modal__input { flex: 1; resize: vertical; min-height: 60px; padding: 8px 10px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg); color: var(--text); font: inherit; }
.modal__noinbox { padding: 12px 16px; border-top: 1px solid var(--border); color: var(--muted); font-size: 13px; }
.modal__status { padding: 6px 16px 12px; font-size: 12.5px; }
.modal__status--ok { color: var(--completed); }
.modal__status--error { color: #dc2626; }
```

- [ ] **Step 3: Verify**

`pnpm test` → all green (card actions + 5 modal tests). `pnpm tauri build --debug --no-bundle` succeeds (tsc strict).

- [ ] **Step 4: Manual acceptance**

Run the debug app. On an idle session card: Terminal focuses the tab; Reply opens the modal with recent turns, newest at the bottom; Escape closes. Send "Reply with the single word pong" to a session that is idle: within a few seconds its card moves to Working, then Completed, and the modal history shows the message and the answer. Open the modal on an Awaiting Decision card if one exists: the banner shows. Check a `--bare` or hookless session shows the no-inbox note if any exists.

- [ ] **Step 5: Commit**

```bash
git add index.html src && git commit -m "feat: card actions and reply modal with conversation history

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```
