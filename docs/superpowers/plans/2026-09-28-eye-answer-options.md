# Eye Answer Options Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render AskUserQuestion options as buttons on cards and in the modal, and answer them by injecting keys into the session's Terminal tab.

**Architecture:** `model::Awaiting` carries the parsed questions; `state.rs`/`transcript.rs` fill them from hook events and transcript tool_use blocks; new `answer.rs` holds the pure key builder, AppleScript builder and precondition check; `focus.rs` shares `tty_for_pid`; a Tauri command `answer_question` ties them together. Frontend: `options.ts` renders option buttons (shared by card and modal), `progress.ts` tracks the next question per session, `actions.ts` routes option clicks.

**Tech Stack:** unchanged (Rust, Tauri 2, Vitest).

**Spec:** `docs/superpowers/specs/2026-09-28-eye-answer-options-design.md`

## Global Constraints

- Key sequence for option index `i` (0-based): `"\x1b[B"` × i, then the Enter that `do script` appends. Never more Downs than `options.len() - 1`.
- Only send when `check` passes; `check` requires the question to have been open ≥ 1000 ms.
- After the last question of a multi-question ask: wait 400 ms, then type `""`.
- AppleScript for typing must not `activate` Terminal.
- Commit per task, conventional messages, `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.

## Review Focus

1. Hook event whose `tool_input.questions` is missing or malformed (older Claude Code, projection dropped it): awaiting is still a question with no options and no buttons; nothing panics. Test in Task 1.
2. The state changes between render and click (question answered in the terminal meanwhile): `check` refuses with "not waiting for a question"; no keys are sent. Test in Task 2.
3. `option_index` equal to `options.len()` (would land on "Type something"): refused. Test in Task 2.
4. Two-question ask: clicking an option for question 0 must not send the submit Enter; the click for question 1 must. Test in Task 3 (pure helper `needs_submit`).
5. Option label containing quotes or backslashes never reaches AppleScript (labels are display only; the script carries only escape sequences). Assert in Task 2's AppleScript test that `applescript_type` escapes `"` and `\` in `text`.

---

### Task 1: Questions on `Awaiting`

**Files:**
- Modify: `src-tauri/src/model.rs`, `src-tauri/src/state.rs`, `src-tauri/src/transcript.rs`, `src/types.ts`

**Interfaces:**
- `model::Choice { label: String, description: String }`, `model::Question { question, header, options: Vec<Choice>, multi_select: bool }` (serde camelCase), `model::Awaiting.questions: Vec<Question>`.
- `model::parse_questions(input: &serde_json::Value) -> Vec<Question>` (reads `input["questions"]`; tolerant: missing → empty, missing description → "", missing multiSelect → false).
- `transcript::OpenQuestion.questions: Vec<Question>`.

- [ ] **Step 1: Failing tests**

`model.rs` tests:
```rust
    #[test]
    fn parses_questions_from_tool_input_tolerantly() {
        let v = serde_json::json!({"questions": [
            {"question": "Size?", "header": "Size", "options": [{"label": "S", "description": "small"}, {"label": "L"}]},
            {"question": "Toppings?", "header": "Top", "multiSelect": true, "options": [{"label": "A", "description": "a"}]}
        ]});
        let q = parse_questions(&v);
        assert_eq!(q.len(), 2);
        assert_eq!(q[0].options[1], Choice { label: "L".into(), description: "".into() });
        assert!(!q[0].multi_select);
        assert!(q[1].multi_select);
        assert!(parse_questions(&serde_json::json!({})).is_empty());
        assert!(parse_questions(&serde_json::json!({"questions": "nope"})).is_empty());
        let json = serde_json::to_value(&q[1]).unwrap();
        assert_eq!(json["multiSelect"], true);
    }
```
`state.rs` tests: in `ask_user_question_is_awaiting_with_question_text`, build the event input with `options` and assert `aw.questions[0].options[0].label == "Tauri"`; add:
```rust
    #[test]
    fn question_without_options_is_still_awaiting_with_no_buttons() {
        let evs = [tool_ev("PreToolUse", "AskUserQuestion", serde_json::json!({}), 10)];
        let aw = run(&reg("busy", 0), &evs, &TranscriptTail::default(), 20).awaiting.unwrap();
        assert_eq!(aw.kind, AwaitKind::Question);
        assert!(aw.questions.is_empty());
    }
```
`transcript.rs` tests: in `detects_open_question_and_last_text`, assert `q.questions[0].header == "Completed"` (fixture already has header) and add an option to the fixture line's first question: change `"options":[]` to `"options":[{"label":"Finished","description":"Claude finished its turn"}]` and assert `q.questions[0].options[0].label == "Finished"`.

Run `cargo test` → compile errors (missing fields/fn).

- [ ] **Step 2: Implement**

`model.rs`: add `Choice`, `Question` (both `Serialize, Deserialize, Clone, Debug, PartialEq, Eq`, camelCase), `parse_questions`, and `pub questions: Vec<Question>` on `Awaiting` (with `#[serde(default)]`). Update the model test literal (`questions: vec![]`).
`state.rs`: `awaiting_for_tool` and the Notification arm set `questions: parse_questions(input)` for AskUserQuestion, `vec![]` otherwise; transcript fallback copies `q.questions.clone()`; update every `Awaiting { .. }` literal in tests.
`transcript.rs`: `OpenQuestion` gains `questions: Vec<Question>` filled from `block["input"]`; the ExitPlanMode arm uses `vec![]`; test literal in `state.rs` fallback test gets `questions: vec![]`.
`src/types.ts`: `Choice { label; description }`, `Question { question; header; options: Choice[]; multiSelect: boolean }`, `Card.awaiting: { kind; detail; questions: Question[] } | null`. Update test fixtures in `board.test.ts`, `modal.test.ts`, `patch.test.ts`, `actions.test.ts` to include `questions: []` where `awaiting` is non-null.

- [ ] **Step 3: Verify and commit**

`cargo test` and `pnpm test` green.
```bash
git add -A src src-tauri/src src-tauri/fixtures && git commit -m "feat: carry question options in the awaiting state

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: Answer driver in Rust

**Files:**
- Create: `src-tauri/src/answer.rs`
- Modify: `src-tauri/src/focus.rs`, `src-tauri/src/store.rs`, `src-tauri/src/lib.rs`

**Interfaces:**
- `focus::tty_for_pid(pid: i32) -> Result<String, String>` (extracted from `focus_pid`).
- `answer::keys_for_option(option_index: usize) -> String`
- `answer::needs_submit(question_index: usize, question_count: usize) -> bool` (true iff count > 1 and index == count − 1)
- `answer::applescript_type(tty: &str, text: &str) -> String`
- `answer::check(card: &Card, question_index: usize, option_index: usize, now_ms: u64) -> Result<(), String>`
- `answer::type_into_tty(tty: &str, text: &str) -> Result<(), String>` (runs osascript)
- `Store::card_for(&mut self, session_id: &str, now_ms: u64) -> Option<Card>`
- Command `answer_question(session_id: String, question_index: usize, option_index: usize) -> Result<(), String>`

- [ ] **Step 1: Failing tests** (`answer.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AwaitKind, Awaiting, Card, Choice, Question, State};

    fn q(n: usize, multi: bool) -> Question {
        Question { question: "Q?".into(), header: "H".into(), multi_select: multi,
            options: (0..n).map(|i| Choice { label: format!("o{i}"), description: "".into() }).collect() }
    }
    fn card(questions: Vec<Question>, since: u64) -> Card {
        Card { session_id: "s".into(), pid: 1, name: "n".into(), cwd: "/x".into(), state: State::Awaiting, state_since: since,
            snippet: "".into(), awaiting: Some(Awaiting { kind: AwaitKind::Question, detail: "Q?".into(), questions }), has_inbox: true }
    }

    #[test]
    fn keys_are_down_arrows_only() {
        assert_eq!(keys_for_option(0), "");
        assert_eq!(keys_for_option(2), "\x1b[B\x1b[B");
    }

    #[test]
    fn submit_only_after_last_of_several() {
        assert!(!needs_submit(0, 1));
        assert!(!needs_submit(0, 2));
        assert!(needs_submit(1, 2));
    }

    #[test]
    fn applescript_types_without_activating_and_escapes_text() {
        let s = applescript_type("/dev/ttys021", "a\"b\\c");
        assert!(s.contains("if tty of t is \"/dev/ttys021\""));
        assert!(s.contains("do script \"a\\\"b\\\\c\" in t"), "{s}");
        assert!(!s.contains("activate"));
        assert!(s.contains("return \"not found\""));
    }

    #[test]
    fn check_accepts_an_open_question_after_one_second() {
        assert_eq!(check(&card(vec![q(3, false)], 1000), 0, 2, 2000), Ok(()));
    }

    #[test]
    fn check_refuses_every_bad_case() {
        let c = card(vec![q(3, false), q(2, true)], 1000);
        assert!(check(&c, 0, 1, 1500).unwrap_err().contains("second"));
        assert!(check(&c, 0, 3, 2000).unwrap_err().contains("no such option"));
        assert!(check(&c, 2, 0, 2000).unwrap_err().contains("no such option"));
        assert!(check(&c, 1, 0, 2000).unwrap_err().contains("Multi-select"));
        let mut working = c.clone();
        working.state = State::Working;
        working.awaiting = None;
        assert!(check(&working, 0, 0, 2000).unwrap_err().contains("not waiting"));
        let mut perm = c.clone();
        perm.awaiting = Some(Awaiting { kind: AwaitKind::Permission, detail: "Bash".into(), questions: vec![] });
        assert!(check(&perm, 0, 0, 2000).unwrap_err().contains("not waiting"));
    }
}
```
`focus.rs`: add test `tty_for_pid_of_dead_process_is_an_error` (`tty_for_pid(2_000_000_000).is_err()`).
`store.rs`: add test `card_for_returns_the_derived_card_or_none` using the fake claude dir from `refresh_builds_cards_from_a_fake_claude_dir` (expect `Some` with name `eye-7`, and `None` for `"nope"`).

Run `cargo test` → compile errors.

- [ ] **Step 2: Implement**

`answer.rs`:
```rust
use crate::model::{AwaitKind, Card, State};
use std::process::Command;

pub const OPEN_DELAY_MS: u64 = 1000;
pub const SUBMIT_DELAY_MS: u64 = 400;

pub fn keys_for_option(option_index: usize) -> String {
    "\x1b[B".repeat(option_index)
}

pub fn needs_submit(question_index: usize, question_count: usize) -> bool {
    question_count > 1 && question_index + 1 == question_count
}

fn applescript_string(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

pub fn applescript_type(tty: &str, text: &str) -> String {
    format!(
        r#"tell application "Terminal"
  repeat with w in windows
    repeat with t in tabs of w
      if tty of t is "{tty}" then
        do script "{}" in t
        return "ok"
      end if
    end repeat
  end repeat
end tell
return "not found""#,
        applescript_string(text)
    )
}

pub fn check(card: &Card, question_index: usize, option_index: usize, now_ms: u64) -> Result<(), String> {
    let aw = match (&card.state, &card.awaiting) {
        (State::Awaiting, Some(aw)) if aw.kind == AwaitKind::Question => aw,
        _ => return Err("This session is not waiting for a question.".into()),
    };
    let Some(q) = aw.questions.get(question_index) else { return Err("That question has no such option.".into()) };
    if option_index >= q.options.len() {
        return Err("That question has no such option.".into());
    }
    if q.multi_select {
        return Err("Multi-select questions must be answered in the terminal.".into());
    }
    if now_ms.saturating_sub(card.state_since) < OPEN_DELAY_MS {
        return Err("Give the terminal a second to show the question.".into());
    }
    Ok(())
}

pub fn type_into_tty(tty: &str, text: &str) -> Result<(), String> {
    let out = Command::new("osascript").arg("-e").arg(applescript_type(tty, text)).output().map_err(|e| format!("could not run osascript: {e}"))?;
    if !out.status.success() {
        return Err(format!("osascript failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    if String::from_utf8_lossy(&out.stdout).trim() == "ok" { Ok(()) } else { Err(format!("No Terminal tab found for {tty}.")) }
}
```
`focus.rs`: extract the `ps` part of `focus_pid` into `pub fn tty_for_pid(pid: i32) -> Result<String, String>` and call it from `focus_pid`.
`store.rs`:
```rust
    pub fn card_for(&mut self, session_id: &str, now_ms: u64) -> Option<Card> {
        self.refresh(now_ms).into_iter().find(|c| c.session_id == session_id)
    }
```
`lib.rs`:
```rust
#[tauri::command(async)]
fn answer_question(state: TauriState<AppState>, session_id: String, question_index: usize, option_index: usize) -> Result<(), String> {
    let (card, pid) = {
        let mut store = state.store.lock().unwrap();
        let card = store.card_for(&session_id, now_ms()).ok_or("Session is no longer running.")?;
        let pid = card.pid;
        (card, pid)
    };
    answer::check(&card, question_index, option_index, now_ms())?;
    let count = card.awaiting.as_ref().map(|a| a.questions.len()).unwrap_or(0);
    let tty = focus::tty_for_pid(pid)?;
    answer::type_into_tty(&tty, &answer::keys_for_option(option_index))?;
    if answer::needs_submit(question_index, count) {
        std::thread::sleep(Duration::from_millis(answer::SUBMIT_DELAY_MS));
        answer::type_into_tty(&tty, "")?;
    }
    Ok(())
}
```
Register `answer_question`; add `pub mod answer;`.

- [ ] **Step 3: Verify and commit**

`cargo test` green (about 82). `pnpm tauri build --debug --no-bundle` ok.
```bash
git add src-tauri/src && git commit -m "feat: answer a session's question by typing into its Terminal tab

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 3: Option buttons, progress and wiring

**Files:**
- Create: `src/options.ts`, `src/options.test.ts`, `src/progress.ts`, `src/progress.test.ts`
- Modify: `src/card.ts`, `src/modal.ts`, `src/actions.ts`, `src/actions.test.ts`, `src/main.ts`, `src/styles.css`, `src/board.test.ts`, `src/modal.test.ts`

**Interfaces:**
- `options.ts`: `renderOptions(card: Card, next: number, opts: { descriptions: boolean; enabled: boolean }): HTMLElement | null` → `.options` containing `.options__label` ("Question 1 of 2 · <header>") when more than one question, and buttons `button.options__btn[data-action=answer][data-q][data-opt]` with `title` = description; when `descriptions` is true each button also contains `.options__desc`. Returns null when the card is not awaiting a question or `next >= questions.length`. Multi-select: buttons `disabled` and a `.options__note` "Multi-select: answer in the terminal". `enabled: false` disables all buttons (used during the first second).
- `progress.ts`: `makeProgress()` with `next(card): number` (0 unless advanced for the same `sessionId` + `stateSince`), `advance(card): void`, `reset(sessionId): void`.
- `actions.ts`: `CardAction` gains `{ kind: "answer"; sessionId; questionIndex; optionIndex }`.

- [ ] **Step 1: Failing tests**

`options.test.ts`:
```ts
import { describe, expect, it } from "vitest";
import { renderOptions } from "./options";
import type { Card } from "./types";

const two: Card = { sessionId: "s", pid: 1, name: "n", cwd: "/x", state: "awaiting", stateSince: 0, snippet: "", hasInbox: true,
  awaiting: { kind: "question", detail: "Size?", questions: [
    { question: "Size?", header: "Size", multiSelect: false, options: [{ label: "S", description: "small" }, { label: "L", description: "large" }] },
    { question: "Top?", header: "Top", multiSelect: true, options: [{ label: "A", description: "" }] },
  ] } };

describe("renderOptions", () => {
  it("renders the current question's options with indices and tooltips", () => {
    const el = renderOptions(two, 0, { descriptions: false, enabled: true })!;
    expect(el.querySelector(".options__label")?.textContent).toBe("Question 1 of 2 · Size");
    const btns = [...el.querySelectorAll<HTMLButtonElement>("button[data-action=answer]")];
    expect(btns.map((b) => b.textContent)).toEqual(["S", "L"]);
    expect(btns.map((b) => [b.dataset.q, b.dataset.opt])).toEqual([["0", "0"], ["0", "1"]]);
    expect(btns[1].title).toBe("large");
    expect(btns.every((b) => !b.disabled)).toBe(true);
    expect(el.querySelector(".options__desc")).toBeNull();
  });

  it("shows descriptions when asked and disables while not yet enabled", () => {
    const el = renderOptions(two, 0, { descriptions: true, enabled: false })!;
    expect([...el.querySelectorAll(".options__desc")].map((d) => d.textContent)).toEqual(["small", "large"]);
    expect([...el.querySelectorAll<HTMLButtonElement>("button")].every((b) => b.disabled)).toBe(true);
  });

  it("disables multi-select questions with a note", () => {
    const el = renderOptions(two, 1, { descriptions: false, enabled: true })!;
    expect(el.querySelector(".options__note")?.textContent).toContain("Multi-select");
    expect(el.querySelector<HTMLButtonElement>("button[data-action=answer]")!.disabled).toBe(true);
  });

  it("returns null when there is nothing to answer", () => {
    expect(renderOptions(two, 2, { descriptions: false, enabled: true })).toBeNull();
    expect(renderOptions({ ...two, state: "working", awaiting: null }, 0, { descriptions: false, enabled: true })).toBeNull();
    expect(renderOptions({ ...two, awaiting: { kind: "permission", detail: "Bash", questions: [] } }, 0, { descriptions: false, enabled: true })).toBeNull();
  });
});
```
`progress.test.ts`:
```ts
import { describe, expect, it } from "vitest";
import { makeProgress } from "./progress";
import type { Card } from "./types";

const c = (stateSince: number): Card => ({ sessionId: "s", pid: 1, name: "n", cwd: "/x", state: "awaiting", stateSince, snippet: "", hasInbox: true,
  awaiting: { kind: "question", detail: "q", questions: [] } });

describe("makeProgress", () => {
  it("advances per session and resets when the question set changes", () => {
    const p = makeProgress();
    expect(p.next(c(1))).toBe(0);
    p.advance(c(1));
    expect(p.next(c(1))).toBe(1);
    expect(p.next(c(2))).toBe(0);
    p.reset("s");
    expect(p.next(c(1))).toBe(0);
  });
});
```
`actions.test.ts`: add
```ts
  it("routes option buttons to an answer action with indices", () => {
    const el = document.createElement("article");
    el.className = "card";
    el.dataset.sessionId = "s1";
    el.innerHTML = '<button data-action="answer" data-q="1" data-opt="2">x</button>';
    document.body.replaceChildren(el);
    expect(cardActionFor(el.querySelector("button")!)).toEqual({ kind: "answer", sessionId: "s1", questionIndex: 1, optionIndex: 2 });
  });
```
`board.test.ts`: in `renderCard` describe add
```ts
  it("renders option buttons for an open question", () => {
    const el = renderCard(card({ state: "awaiting", stateSince: NOW - 5000, awaiting: { kind: "question", detail: "Q?", questions: [{ question: "Q?", header: "H", multiSelect: false, options: [{ label: "A", description: "" }] }] } }), NOW);
    expect(el.querySelector("button[data-action=answer]")?.textContent).toBe("A");
  });
```
`modal.test.ts`: in the awaiting banner test, give the card a question with options and assert `.modal__banner .options__desc` exists.

`pnpm test` → failures.

- [ ] **Step 2: Implement**

`options.ts`:
```ts
import type { Card } from "./types";

export function renderOptions(card: Card, next: number, opts: { descriptions: boolean; enabled: boolean }): HTMLElement | null {
  const aw = card.awaiting;
  if (card.state !== "awaiting" || !aw || aw.kind !== "question") return null;
  const q = aw.questions[next];
  if (!q) return null;
  const root = document.createElement("div");
  root.className = "options";
  if (aw.questions.length > 1) {
    const label = document.createElement("div");
    label.className = "options__label";
    label.textContent = `Question ${next + 1} of ${aw.questions.length} · ${q.header}`;
    root.append(label);
  }
  const list = document.createElement("div");
  list.className = "options__list";
  q.options.forEach((o, i) => {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "options__btn";
    btn.dataset.action = "answer";
    btn.dataset.q = String(next);
    btn.dataset.opt = String(i);
    btn.title = o.description;
    btn.disabled = !opts.enabled || q.multiSelect;
    if (opts.descriptions && o.description) {
      const label = document.createElement("span");
      label.className = "options__name";
      label.textContent = o.label;
      const desc = document.createElement("span");
      desc.className = "options__desc";
      desc.textContent = o.description;
      btn.append(label, desc);
    } else {
      btn.textContent = o.label;
    }
    list.append(btn);
  });
  root.append(list);
  if (q.multiSelect) {
    const note = document.createElement("div");
    note.className = "options__note";
    note.textContent = "Multi-select: answer in the terminal";
    root.append(note);
  }
  return root;
}
```
`progress.ts`:
```ts
import type { Card } from "./types";

export function makeProgress() {
  const state = new Map<string, { since: number; next: number }>();
  const entry = (card: Card) => {
    const e = state.get(card.sessionId);
    return e && e.since === card.stateSince ? e : { since: card.stateSince, next: 0 };
  };
  return {
    next: (card: Card) => entry(card).next,
    advance: (card: Card) => {
      const e = entry(card);
      state.set(card.sessionId, { since: e.since, next: e.next + 1 });
    },
    reset: (sessionId: string) => {
      state.delete(sessionId);
    },
  };
}
export type Progress = ReturnType<typeof makeProgress>;
```
`card.ts`: `renderCard(card, nowMs, next = 0)`; after the awaiting block, `const opts = renderOptions(card, next, { descriptions: false, enabled: nowMs - card.stateSince >= 1000 }); if (opts) root.append(opts);`. `board.ts`: `renderBoard(cards, nowMs, nextFor?: (c: Card) => number)` passing `nextFor?.(c) ?? 0` to `renderCard`.
`modal.ts`: `ModalModel` gains `next: number`; in the banner, if the awaiting is a question with options, append `renderOptions(m.card, m.next, { descriptions: true, enabled: Date.now() - m.card.stateSince >= 1000 })` and change the banner text to "This session is asking a question." when options render; `ModalHandlers` gains `onAnswer(q: number, opt: number): void`; the banner delegates clicks on `button[data-action=answer]` to `onAnswer`. Stateful part: a module-level `progress` (exported `setProgress(p)` called from main so both share one instance), `onAnswer` → `invoke("answer_question", { sessionId, questionIndex, optionIndex })` then `progress.advance(card)` and `paint({focusInput:false})` on success, `setStatus(false, err)` on failure. `patchModal` also swaps `.modal__banner` already, so options refresh.
`actions.ts`: when `action === "answer"`, read `data-q`/`data-opt` as numbers and return the answer action.
`main.ts`: `const progress = makeProgress(); setProgress(progress);` pass `(c) => progress.next(c)` to `renderBoard`; in `act`, for `answer`: `invoke("answer_question", {...})` then `progress.advance(card); paint();` or toast the error. Reset progress for sessions whose state is no longer awaiting in the `sessions` listener.
`styles.css`:
```css
.options { margin-top: 8px; }
.options__label { font-size: 11px; color: var(--muted); margin-bottom: 4px; }
.options__list { display: flex; flex-wrap: wrap; gap: 6px; }
.options__btn { background: var(--panel); border: 1px solid var(--awaiting); color: var(--text); border-radius: 6px; padding: 4px 10px; font-size: 12px; cursor: pointer; text-align: left; display: flex; flex-direction: column; gap: 2px; }
.options__btn:hover:not(:disabled) { background: color-mix(in srgb, var(--awaiting) 18%, var(--panel)); }
.options__btn:disabled { opacity: .5; cursor: default; }
.options__name { font-weight: 600; }
.options__desc { font-size: 11.5px; color: var(--muted); }
.options__note { font-size: 11.5px; color: var(--muted); margin-top: 4px; }
.modal__banner .options { flex-basis: 100%; }
.modal__banner { flex-wrap: wrap; }
```

- [ ] **Step 3: Verify**

`pnpm test` green; `pnpm tauri build --debug --no-bundle` ok.

- [ ] **Step 4: Manual acceptance**

Run the app. In any idle session, ask Claude to "use AskUserQuestion to ask me which of Red, Green, Blue I like". When the card moves to Awaiting Decision, its buttons appear after a second; click Green; the terminal records "→ Green" and the card moves to Working. Repeat with a two-question ask through the modal: after the first click the second question's options appear; after the second, the terminal shows the submit and the answer.

- [ ] **Step 5: Commit**

```bash
git add src && git commit -m "feat: selectable answer options on cards and in the modal

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```
