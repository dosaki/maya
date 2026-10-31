# The Phone's Session Window as a Pane Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** On the phone, show what `#modal-host` holds (session window, new-session and resume dialogs) as a pane under the top bar, like Chat, so the reply box brings up the keyboard.

**Architecture:** CSS in `src/mobile/mobile.css` turns `#modal-host` into a flex child of `#app`'s column and un-pins `.modal` inside it; while the host holds something, the board and the rows above it are hidden. A tab switch on the phone closes whatever the host holds, popping the card's history entry. The pinned layer's keyboard workaround from #70 (`trackVisibleArea`) is removed.

**Tech Stack:** TypeScript, Vite, Vitest (jsdom), Tauri 2 Android web view.

**Spec:** `docs/superpowers/specs/2026-10-31-mobile-session-pane-design.md`

## Global Constraints

- No change on the desktop: `src/styles.css` and the desktop's `src/main.ts` stay as they are.
- `keepFocusedAboveKeyboard` stays: Chat relies on it.
- The top bar and its tabs stay on screen while a session is open.
- Tapping a tab closes an open session as × does (history entry popped), and closes an open new-session or resume dialog.
- Every commit message ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Release: patch bump from `main`'s version (0.44.0 → 0.44.1 unless `main` moved), with a `# 0.44.1` / `## Fixed` section at the top of `RELEASE_NOTES.md`, in one `chore(release): 0.44.1` commit.

## Review Focus

1. **An empty host hides nothing.** After ×, Back or a tab switch, `#modal-host` must be truly empty (no whitespace or leftover node), or the board stays hidden. Pinned by the CSS test's selector (`:empty`) and Task 1's "empties the host" assertion.
2. **Back after a tab switch.** Switching tabs pops the card's history entry once; Back must then not reopen or close anything else. Pinned by Task 1's `dismissModal` calling the dismiss callback exactly once, and not at all when nothing is open.
3. **Notification tap from another tab.** `openFromTap` calls `tabs.show("sessions")` and then opens the card; the tab switch must close only what was open before, never the card being opened. Pinned by the call order in Task 2 (close runs inside `show`, before `open`) — check on the phone.
4. **The new-session prompt with the keyboard up.** The compact panel's form must still scroll under its header. The existing `.modal__panel--compact .newsession` rule stays; check on the phone.
5. **Desktop untouched.** Only `mobile.css` and `src/mobile/*` change for layout; the desktop page never loads `mobile.css`.

---

### Task 1: `dismissModal` — close the session window as × does

**Files:**
- Modify: `src/modal.ts:930-934` (the private `dismiss`)
- Test: `src/modal-open.test.ts`

**Interfaces:**
- Produces: `export function dismissModal(): void` in `src/modal.ts` — closes the open session window and calls the `setOnDismiss` callback; does nothing when none is open.

- [ ] **Step 1: Write the failing test** in `src/modal-open.test.ts`, new `describe("dismissModal")`:
  - `it("closes the open window and tells the board once")`: `document.body.innerHTML = '<div id="modal-host"></div>'`; `invoke` resolves `session_history` to `[]`; `const onDismiss = vi.fn(); setOnDismiss(onDismiss)`; `await openModal(kiro)`; `dismissModal()`; expect `document.getElementById("modal-host")!.childNodes.length` toBe `0`; expect `onDismiss` toHaveBeenCalledTimes `1`; `dismissModal()` again; still called `1` time. `setOnDismiss(null)` at the end.

- [ ] **Step 2: Run it to see it fail**

Run: `npx vitest run src/modal-open.test.ts`
Expected: FAIL — `dismissModal` is not exported.

- [ ] **Step 3: Rename `dismiss` to an exported `dismissModal`** in `src/modal.ts`, with a one-line doc comment, and update its in-file callers (`onClose: dismiss`, the Escape `keyHandler`).

- [ ] **Step 4: Run the tests**

Run: `npx vitest run src/modal-open.test.ts src/modal.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/modal.ts src/modal-open.test.ts
git commit -m "refactor(viewer): let the page close the session window as × does"
```

### Task 2: The phone shows the host as a pane under the top bar

**Files:**
- Create: `src/mobile/overlays.ts`, `src/mobile/overlays.test.ts`
- Modify: `src/mobile/mobile.css:16-23` (the cards-and-dialogs comment and the `.modal` rule)
- Modify: `src/mobile/main.ts` (imports; `makeTabs` callback; drop `trackVisibleArea()`)
- Modify: `src/mobile/keyboard.ts` (drop `VisibleArea`, `trackVisibleArea` and their comment)
- Modify: `src/mobile/keyboard.test.ts` (drop the `trackVisibleArea` and "the phone's card layer" describes; keep the others)

**Interfaces:**
- Consumes: `dismissModal()` from Task 1; `closeNewSession()` (`src/newsession.ts`), `closeResume()` (`src/resume.ts`).
- Produces: `export function closeOverlays(): void` in `src/mobile/overlays.ts` — calls `dismissModal()`, `closeNewSession()`, `closeResume()`.

- [ ] **Step 1: Write the failing tests**

`src/mobile/overlays.test.ts`:
  - `it("closes the session window, the new-session form and the resume list")`: `vi.mock("../modal")`, `vi.mock("../newsession")`, `vi.mock("../resume")`; `closeOverlays()`; expect `dismissModal`, `closeNewSession`, `closeResume` each called once.
  - `describe("the phone's card layer")` reading `../mobile/mobile.css?raw` (as `keyboard.test.ts` does), asserting:
    - the CSS contains `#modal-host:not(:empty) {` and that rule contains `flex: 1;` and `min-height: 0;`
    - the `.modal {` rule contains `position: static;` and does not contain `--visible-`
    - the CSS contains a rule starting `#app:has(> #modal-host:not(:empty)) >` that names `#board`, `#strip-host`, `#voice-host` and `#remembered` and contains `display: none;`

- [ ] **Step 2: Run them to see them fail**

Run: `npx vitest run src/mobile/overlays.test.ts`
Expected: FAIL — no `overlays.ts`, and the CSS rules are missing.

- [ ] **Step 3: Implement**
  - `src/mobile/overlays.ts`: `closeOverlays()` as in Interfaces, with a header comment saying a tab switch on the phone leaves no card or dialog over the pane it shows.
  - `src/mobile/mobile.css`: replace the `.modal { top: var(--visible-top…) }` rule and rewrite its comment. Rules, one per line (the tests split on lines):
    - `#modal-host:not(:empty) { flex: 1; min-height: 0; display: flex; flex-direction: column; }`
    - `.modal { position: static; inset: auto; flex: 1; min-height: 0; display: flex; }`
    - `#app:has(> #modal-host:not(:empty)) > :is(#board, #strip-host, #voice-host, #remembered) { display: none; }`
    The comment says why: in a pinned layer the reply box did not bring up the keyboard while Chat's, in the page, did; so the layer is a pane under the top bar, and the page shrinking for the keyboard keeps its box clear, as Chat's.
  - `src/mobile/main.ts`: call `closeOverlays()` first thing in the `makeTabs` callback; remove `trackVisibleArea` from the import and the call.
  - `src/mobile/keyboard.ts` / `keyboard.test.ts`: remove `trackVisibleArea`, `VisibleArea`, their comment block and tests.

- [ ] **Step 4: Run all tests and the type check**

Run: `npx vitest run && npx tsc --noEmit`
Expected: all PASS, no type errors.

- [ ] **Step 5: Commit**

```bash
git add src/mobile
git commit -m "fix(mobile): open a session as a pane under the top bar, like Chat"
```

### Task 3: Release

**Files:**
- Modify: version files via `sh scripts/set-version.sh`, `RELEASE_NOTES.md`

- [ ] **Step 1:** `git fetch origin && git show origin/main:package.json | grep '"version"'`; bump the patch from it with `sh scripts/set-version.sh <x.y.z+1>`.
- [ ] **Step 2:** Add at the top of `RELEASE_NOTES.md`: `# <version>`, `## Fixed`, one bullet: "On the phone, a session opens under the top bar, like Chat, and tapping its reply box brings up the keyboard. The new-session and resume dialogs open the same way, and tapping a tab closes them."
- [ ] **Step 3:** `npx vitest run` — PASS.
- [ ] **Step 4:** `git add -A && git commit -m "chore(release): <version>"`.
- [ ] **Step 5 (on the phone):** build the Android app per `docs/DEVELOPING.md`, open a session, tap the reply box — the keyboard comes up and the box sits above it; switch tabs and press Back; open a session from a notification while on Chat.
