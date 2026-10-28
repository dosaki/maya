# Markdown in Chat, spoken as plain words: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Chat renders Maya's replies as markdown with working links, the voice reads only their prose, and Chat opens at the latest message.

**Architecture:** One pure function, `maya_core::speech::from_markdown`, turns markdown into the words to say, using `pulldown-cmark`. The Tauri app applies it where a line reaches the computer's voice and to the line the phone speaks; Chat history keeps the raw markdown. The Chat pane reuses the session view's sanitised `renderMarkdown` and its link routing.

**Tech Stack:** Rust (`pulldown-cmark` 0.13), TypeScript + vitest (`marked`, `DOMPurify` already present).

**Spec:** `docs/superpowers/specs/2026-10-28-chat-markdown-design.md`

## Global Constraints

- Only her (`maya`) turns render as markdown; the user's turns stay plain text.
- Links never navigate the window: clicks go to `invoke("open_url", { url })`.
- Chat history (`listener::remember`) stores her raw markdown; only speech is converted.
- Tables, code blocks, images and raw HTML are never spoken. A link is spoken as its label; a bare URL as "a link".
- Speech adds no fixed line for what it skips: she writes the lead-in.
- Version: a `feat` → bump minor from `main` (0.36.0 → 0.37.0 unless `main` moved) with `sh scripts/set-version.sh`, committed alone as `chore(release): <version>`.

## Review Focus

1. Maya's fixed plain lines ("Focusing api-fix.", "Telling api_fix_v2: open the PR.", "Resuming 2 sessions, 1 here and 1 on Gnowee.") must be spoken exactly as before → Task 1 test `plain_lines_pass_through_unchanged`.
2. Her reply ending in a table, followed by an action result joined by `one_line`, must not glue the result into the table → Task 2 test `her_markdown_and_what_ran_stay_apart`.
3. A reply that ends in a question followed by a table must still be heard as a question only if the spoken words end in "?" → Task 2 uses the spoken words for `asks_question`.
4. A reply that is only a table speaks nothing (and does not error) → Task 1 test `only_a_table_says_nothing`.
5. A `javascript:` or relative link in her text is not clickable → Task 4 test "her links open in the browser and unsafe ones are inert".

---

### Task 1: `speech::from_markdown` in core

**Files:**
- Modify: `Cargo.toml` (workspace deps), `core/Cargo.toml`, `core/src/lib.rs` (add `pub mod speech;` in alphabetical order)
- Create: `core/src/speech.rs` (module doc + function + tests)

**Interfaces:**
- Produces: `pub fn from_markdown(text: &str) -> String` in `maya_core::speech`.

- [ ] **Step 1: Add the dependency.** Workspace: `pulldown-cmark = { version = "0.13", default-features = false }`; core: `pulldown-cmark.workspace = true`.

- [ ] **Step 2: Write the failing tests** in `core/src/speech.rs`:

```rust
#[test] fn plain_lines_pass_through_unchanged() {
    for l in ["Sure. Focusing api-fix.", "Telling api_fix_v2: open the PR.", "Resuming 2 sessions, 1 here and 1 on Gnowee.", "Which one?"] {
        assert_eq!(from_markdown(l), l);
    }
}
#[test] fn a_link_is_read_as_its_label_and_a_bare_url_as_a_link() {
    assert_eq!(from_markdown("See [the PR](https://github.com/o/r/pull/1)."), "See the PR.");
    assert_eq!(from_markdown("It is at https://example.com/x now."), "It is at a link now.");
    assert_eq!(from_markdown("<https://example.com>"), "a link.");
}
#[test] fn emphasis_headings_and_inline_code_are_plain_words() {
    assert_eq!(from_markdown("## Done\n**All green**, and `main` is merged."), "Done. All green, and main is merged.");
}
#[test] fn list_items_become_sentences() {
    assert_eq!(from_markdown("Two left:\n- api-fix\n- docs, waiting?"), "Two left: api-fix. docs, waiting?");
}
#[test] fn tables_code_blocks_and_images_are_not_read() {
    let t = "The failing tests are in a table in the chat.\n\n| test | file |\n|---|---|\n| a | x.rs |\n\n```sh\ncargo test\n```\n![shot](https://x/y.png)";
    assert_eq!(from_markdown(t), "The failing tests are in a table in the chat.");
}
#[test] fn only_a_table_says_nothing() {
    assert_eq!(from_markdown("| a |\n|---|\n| 1 |"), "");
}
#[test] fn paragraphs_join_into_one_line() {
    assert_eq!(from_markdown("Sure.\n\nFocusing api-fix."), "Sure. Focusing api-fix.");
}
```

- [ ] **Step 3: Run** `cargo test -p maya-core speech` → FAIL (no `from_markdown`).

- [ ] **Step 4: Implement `from_markdown`.** Parse with `Options::ENABLE_TABLES | ENABLE_STRIKETHROUGH | ENABLE_TASKLISTS`. Keep a skip depth: inside `Table`, `CodeBlock`, `Image`, `HtmlBlock` nothing is emitted; `Html`/`InlineHtml` events are dropped. `Text` and inline `Code` append their text, with each whitespace-separated word starting `http://` or `https://` replaced by "a link"; a `Link` whose text equals its URL (autolink) reads "a link". `SoftBreak`/`HardBreak` append a space. At the end of a `Paragraph`, `Heading` or `Item`: trim trailing spaces; if anything was written and it does not end in one of `.!?:;…`, append `.`; then a space. Finally collapse runs of whitespace and trim.

- [ ] **Step 5: Run** `cargo test -p maya-core speech` → PASS.

- [ ] **Step 6: Commit** `feat(speech): read markdown as plain words`.

### Task 2: Speak the words, keep the markdown

**Files:**
- Modify: `src-tauri/src/listener.rs` (`reply_aloud_as` ~160-187, `reply_then_idle` ~407)
- Modify: `src-tauri/src/net_app.rs` (`brain_reply_data` ~304)
- Modify: `src-tauri/src/brain_route.rs` (`one_line` ~56 and its test ~515)

**Interfaces:**
- Consumes: `maya_core::speech::from_markdown(&str) -> String`.

- [ ] **Step 1: Write the failing test** in `brain_route.rs` tests, replacing `her_words_and_what_ran_make_one_line`:

```rust
#[test] fn her_words_and_what_ran_make_one_line() {
    assert_eq!(one_line(&["Sure.", "", " Focusing api-fix. "]), "Sure.\n\nFocusing api-fix.");
    assert_eq!(one_line(&["  ", ""]), "");
    assert_eq!(maya_core::speech::from_markdown(&one_line(&["Sure.", "Focusing api-fix."])), "Sure. Focusing api-fix.");
}
#[test] fn her_markdown_and_what_ran_stay_apart() {
    let line = one_line(&["Failures below.\n\n| t |\n|---|\n| a |", "Focusing api-fix."]);
    assert_eq!(maya_core::speech::from_markdown(&line), "Failures below. Focusing api-fix.");
}
```

- [ ] **Step 2: Run** `cargo test -p maya brain_route` (use the src-tauri package name from `src-tauri/Cargo.toml`) → FAIL.

- [ ] **Step 3: `one_line` joins parts with `"\n\n"`** (doc comment: separate paragraphs, so markdown ends cleanly; speech still says them as one utterance).

- [ ] **Step 4: In `reply_aloud_as`,** compute `let words = maya_core::speech::from_markdown(text);` after the blank check. `remember` keeps `text`; `v.status.said` and the `Utterance` get `words`. If `words` is empty, skip speaking but still remember. **In `reply_then_idle`,** pass `&from_markdown(text)` to `asks_question`. **In `brain_reply_data`,** push `from_markdown(say)` instead of `say.trim()`.

- [ ] **Step 5: Run** `cargo test --workspace` → PASS (fix any test that pinned the old `" "` join, keeping its intent).

- [ ] **Step 6: Commit** `feat(voice): speak her markdown as plain words`.

### Task 3: Her prompt

**Files:** Modify `core/src/brain/prompt.rs` (line 14-15 and test `she_keeps_her_reply_short_enough_to_speak`).

- [ ] **Step 1: Update the test** to assert the prompt contains `"your reply is spoken aloud"`, `"Your reply is markdown"`, `"[label](url)"`, and `"say in a few words what it shows"`, and no longer contains `"Leave out full paths"`.

- [ ] **Step 2: Run** `cargo test -p maya-core brain::prompt` → FAIL.

- [ ] **Step 3: Replace lines 14-15 with:**

```
Reply in short plain sentences, because your reply is spoken aloud. Keep the spoken sentences under 30 words when you can.\n\
Your reply is markdown: Chat shows it rendered, and the voice reads only its sentences and lists. Tables, code blocks and images are not read; a link is read as its label. Write a link as [label](url). Keep full paths, commit hashes, long ids and URLs out of your sentences. When such details matter, put them in a table or code block and say in a few words what it shows, for example \"The failing tests are in a table in the chat.\"\n\
```

- [ ] **Step 4: Run** `cargo test -p maya-core brain::prompt` → PASS.

- [ ] **Step 5: Commit** `feat(brain): write for the ear and put details on screen`.

### Task 4: Chat renders her markdown

**Files:**
- Modify: `src/chat.ts` (`renderChat`, `ChatHandlers`, `initChat`), `src/styles.css` (~309-316, ~451)
- Test: `src/chat.test.ts`

**Interfaces:**
- Produces: `ChatHandlers.onOpenLink(url: string): void`.

- [ ] **Step 1: Write the failing tests** in `src/chat.test.ts` (use the file's existing handler helper, adding `onOpenLink: vi.fn()`):

```ts
it("renders her turns as markdown and keeps the user's as text", () => {
  const el = renderChat({ turns: [{ who: "user", text: "**hi**", at: 1, via: "chat" }, { who: "maya", text: "**Done.** See [the PR](https://github.com/o/r/pull/1).", at: 2, via: "maya" }], status: IDLE_STATUS, draft: "" }, h);
  const [user, maya] = el.querySelectorAll(".chat__turn");
  expect(user.textContent).toBe("**hi**");
  expect(maya.querySelector(".md strong")!.textContent).toBe("Done.");
});
it("her links open in the browser and unsafe ones are inert", () => {
  const el = renderChat({ turns: [{ who: "maya", text: "[pr](https://github.com/o/r/pull/1) [bad](javascript:alert(1)) [rel](/x)", at: 2, via: "maya" }], status: IDLE_STATUS, draft: "" }, h);
  const links = el.querySelectorAll("a");
  const ev = new MouseEvent("click", { bubbles: true, cancelable: true });
  links[0].dispatchEvent(ev);
  expect(ev.defaultPrevented).toBe(true);
  expect(h.onOpenLink).toHaveBeenCalledWith("https://github.com/o/r/pull/1");
  expect(links[1].hasAttribute("href")).toBe(false);
  expect(links[2].hasAttribute("href")).toBe(false);
});
```

Update the `chat-flow.test.ts` assertion on `.chat__turn` text if it breaks (her text is now inside `.md`; `textContent` should still match).

- [ ] **Step 2: Run** `pnpm test src/chat` → FAIL.

- [ ] **Step 3: Implement.** In `renderChat`, a `maya` turn appends `renderMarkdown(t.text)` (from `./markdown`); a `user` turn keeps `textContent`. One click listener on the log, as in `src/modal.ts:595-602`: an `a[href]` click calls `preventDefault()` and `h.onOpenLink(href)`. Update the doc comment ("Her text is set as text, never as markup") to say her text is sanitised markdown. In `initChat`, wire `onOpenLink: (url) => void invoke("open_url", { url }).catch((e) => showToast(String(e)))`. CSS: widen the `.turn__text .md` rules at ~309-316 and the table rule to also match `.chat__turn .md` (for example `:is(.turn__text, .chat__turn) .md`), and add `.chat__turn--maya { white-space: normal; }`. Check `src/mobile/mobile.css` loads these rules on the phone; if not, add the same there.

- [ ] **Step 4: Run** `pnpm test` → PASS.

- [ ] **Step 5: Commit** `feat(chat): render her replies as markdown with working links`.

### Task 5: Chat opens at the latest message

**Files:** Modify `src/chat.ts` (`maya:tab` listener ~212). Test: `src/chat-flow.test.ts`.

- [ ] **Step 1: Write the failing test** in `chat-flow.test.ts`:

```ts
it("opens at the latest message", async () => {
  const tabs = makeTabs();
  initChat();
  await flush();
  on<ChatTurn[]>("chat-reset")({ payload: [{ who: "maya", text: "one", at: 1, via: "maya" }, { who: "maya", text: "two", at: 2, via: "maya" }] });
  const log = document.querySelector<HTMLElement>(".chat__log")!;
  Object.defineProperty(log, "scrollHeight", { configurable: true, value: 900 });
  tabs.show("chat");
  await flush();
  expect(log.scrollTop).toBe(900);
});
```

(jsdom clamps nothing, so `scrollTop` keeps what is set.)

- [ ] **Step 2: Run** `pnpm test src/chat-flow` → FAIL.

- [ ] **Step 3: Implement.** When `maya:tab` opens `chat`, set the pane's `.chat__log` `scrollTop` to its `scrollHeight`. If the pane is not yet laid out at that moment in the real app, do it in a `requestAnimationFrame` as well; the test awaits `flush()`.

- [ ] **Step 4: Run** `pnpm test` → PASS.

- [ ] **Step 5: Commit** `fix(chat): open at the latest message`.

### Task 6: README and version

**Files:** Modify `README.md` (Chat paragraphs near "Maya's brain", ~319-350, and the phone Chat paragraph ~214); version files via the script.

- [ ] **Step 1:** Add two sentences where README describes her chat: her replies show as markdown with links that open in the browser; the voice reads only her sentences and lists, and she tells you when a table or code in the chat has the details.
- [ ] **Step 2:** Commit `docs: markdown in Chat`.
- [ ] **Step 3:** `git fetch origin main`, read `main`'s version, run `sh scripts/set-version.sh <minor bump>`; `cargo test --workspace` and `pnpm test` pass; commit alone as `chore(release): <version>`.
