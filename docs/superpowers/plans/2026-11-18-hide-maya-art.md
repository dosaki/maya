# Hide Maya's Picture Beside the Release Notes: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Settings switch that hides Maya's full-body picture in Settings → Release Notes and in the What's new window, on desktop.

**Architecture:** One saved boolean in the config, as `notifyOnPrReview` is. One marker on the page, `data-art="off"` on `<html>`, set the way `applyCardSize` sets `data-cards`, remembered in `localStorage` for the first paint. CSS keyed on that marker hides both pictures and drops the room kept for them.

**Tech Stack:** Rust (serde) in `core/`, TypeScript and Vitest in `src/`, plain CSS.

**Spec:** `docs/superpowers/specs/2026-11-18-hide-maya-art-design.md`

## Global Constraints

- The row label: `Show Maya beside release notes`. It goes in Settings → General → Appearance, directly below the Cards row.
- On by default. A config file written before the field reads as on.
- Desktop only: do not change `src/mobile/`.
- The change shows at once, before the save lands.
- Config JSON key: `releaseNotesArt` (boolean). `localStorage` key: `maya.releaseNotesArt`, holding `"off"` only when hidden.
- This is a `feat`: bump 0.60.0 → 0.61.0 with `sh scripts/set-version.sh 0.61.0`, and add a `# 0.61.0` section to `RELEASE_NOTES.md`, in one `chore(release): 0.61.0` commit.

## Review Focus

- **A start with the picture turned off:** the What's new window opens at start, before the saved settings arrive. It must not show her for a moment. The boot script test in Task 2 covers this.
- **Turned back on:** turning the switch off and then on again restores both pictures and their layout. That means the marker is removed, not set to some other value. Task 2's `applyReleaseNotesArt(true)` test covers this.
- **An older config file:** a file without `releaseNotesArt` shows her. Task 1's test covers this.
- **No gap:** with her hidden, Settings → Release Notes is one column, and the What's new panel has no 410px left margin. Task 2's CSS test covers this.
- **The phone:** nothing on the phone sets the marker, so its What's new banner stays. The rule is to make no change in `src/mobile/`.

---

### Task 1: The saved setting

**Files:**
- Modify: `core/src/config.rs` (the `Config` struct next to `notify_on_pr_review` at ~line 130; `Default` at ~line 750; tests next to `notify_on_pr_review_defaults_on_and_reads_from_older_files` at ~line 1650)

**Interfaces:**
- Produces: `Config.release_notes_art: bool`, with `#[serde(default = "default_true")]`, serialised as `"releaseNotesArt"`.

- [ ] **Step 1: Write the failing test** `release_notes_art_defaults_on_and_reads_from_older_files`, built the same way as the `notify_on_pr_review` one:
  - `assert!(Config::default().release_notes_art);`
  - An older file without the key reads as `true`.
  - Serialising `Config { release_notes_art: false, ..Config::default() }` contains `"releaseNotesArt":false`.
- [ ] **Step 2: Run it.** `cargo test -p maya-core release_notes_art` should FAIL to compile, with no field `release_notes_art`.
- [ ] **Step 3: Add the field**, with a doc comment ("Show Maya's full-body picture beside the release notes on the desktop"), and `release_notes_art: true` in `Default`.
- [ ] **Step 4: Run it.** `cargo test -p maya-core release_notes_art` should PASS. Then `cargo test --workspace` should pass, because no struct literal elsewhere lists every `Config` field. If one does, add the field there.
- [ ] **Step 5: Commit** `feat(config): remember whether to show Maya beside release notes`.

### Task 2: The marker, the first paint, and the CSS

**Files:**
- Modify: `src/theme.ts` (after `applyCardSize`)
- Modify: `index.html:8` (the inline boot script)
- Modify: `src/styles.css` (next to the `.settings__rn` rules at ~line 153–164, and the `.whatsnew` rules at ~line 334–350)
- Test: `src/theme.test.ts`, `src/theme-css.test.ts`

**Interfaces:**
- Produces: `export const RELEASE_NOTES_ART_KEY = "maya.releaseNotesArt"` and `export function applyReleaseNotesArt(show: boolean): void` in `src/theme.ts`.

- [ ] **Step 1: Write the failing tests** in `src/theme.test.ts`. Clear `document.documentElement.dataset.art` and `localStorage` in the file's existing `beforeEach`.
  - `applyReleaseNotesArt(false)` sets `dataset.art` to `"off"`, and `localStorage.getItem(RELEASE_NOTES_ART_KEY)` is `"off"`.
  - After that, `applyReleaseNotesArt(true)` leaves `dataset.art` undefined and the key `null`.
  - In `describe("the first paint")`: with `"off"` saved, `boot()` sets `dataset.art` to `"off"`. With nothing saved, or with `"on"` saved, `dataset.art` stays undefined.
  - `"uses the same release notes art key as the theme module"`: the boot script contains `RELEASE_NOTES_ART_KEY`, the same way the card size test checks its key.
- [ ] **Step 2: Write the failing CSS test** in `src/theme-css.test.ts`, using the file's `block(selector)`:
  - `block(':root[data-art="off"] .settings.pane .settings__rn')` contains `grid-template-columns: minmax(0, 1fr)`.
  - `block(':root[data-art="off"] .settings.pane .settings__rn-art, :root[data-art="off"] .whatsnew__art')` contains `display: none`.
  - `block(':root[data-art="off"] .whatsnew .whatsnew__panel')` contains `margin-left: 0`.
- [ ] **Step 3: Run them.** `npx vitest run src/theme.test.ts src/theme-css.test.ts` should FAIL.
- [ ] **Step 4: Implement.**
  - `applyReleaseNotesArt`: on `false`, set `dataset.art = "off"` and store `"off"`. On `true`, `delete dataset.art` and remove the key. Wrap the storage calls in `try`, as `applyCardSize` does.
  - Boot script: add `if(localStorage.getItem("maya.releaseNotesArt")==="off")document.documentElement.dataset.art="off";` inside the existing `try`.
  - CSS: add the three rules exactly as the selectors in Step 2 name them, with each one on its own line, so `block` finds it.
- [ ] **Step 5: Run them.** `npx vitest run src/theme.test.ts src/theme-css.test.ts` should PASS.
- [ ] **Step 6: Commit** `feat(theme): a marker that hides Maya beside release notes`.

### Task 3: The switch in Settings

**Files:**
- Modify: `src/settings.ts`. Add `releaseNotesArt?: boolean` to the model (~line 100) and to `ConfigJson` (~line 445). Add `onReleaseNotesArt?(show: boolean): void` to the handlers (~line 234). Add the row in the Appearance group (~line 1271). Load it (~line 2502). Add the handler (~line 2660).
- Test: `src/settings.test.ts`, next to the Cards tests (~line 1722)

**Interfaces:**
- Consumes: `applyReleaseNotesArt(show: boolean)` from Task 2. Config key `releaseNotesArt` from Task 1.

- [ ] **Step 1: Write the failing tests.**
  - `"offers to hide Maya beside release notes under Cards, and reports the choice"`:
    - The General section has `input[name=releaseNotesArt]`, checked by default.
    - Its row's `.settings__label` is `Show Maya beside release notes`.
    - The Cards row's `nextElementSibling` is that row.
    - Clicking the input calls `h.onReleaseNotesArt` with `false`.
  - `"shows the saved choice"`: with `releaseNotesArt: false` in the model, the box is unchecked.
- [ ] **Step 2: Run them.** `npx vitest run src/settings.test.ts` should FAIL.
- [ ] **Step 3: Implement.**
  - The row uses `toggle("releaseNotesArt", model.releaseNotesArt !== false, (on) => h.onReleaseNotesArt?.(on))`.
  - On load: `model.releaseNotesArt = c.releaseNotesArt !== false; applyReleaseNotesArt(model.releaseNotesArt);`
  - The handler sets the model, calls `applyReleaseNotesArt(show)` at once, then `void run(() => saveConfig({ releaseNotesArt: show }))`. That's the same order as `onCardSize`.
- [ ] **Step 4: Run all of them.** `npx vitest run` and `npx tsc --noEmit` should both pass.
- [ ] **Step 5: Commit** `feat(settings): a switch to hide Maya beside release notes`.

### Task 4: Release

**Files:** the version files through `scripts/set-version.sh`, and `RELEASE_NOTES.md`.

- [ ] **Step 1:** Check `main`'s version (`git show origin/main:package.json | grep version`). Bump the minor from it. It is 0.61.0 when `main` is 0.60.0.
- [ ] **Step 2:** `sh scripts/set-version.sh 0.61.0`
- [ ] **Step 3:** Add at the top of `RELEASE_NOTES.md`:

  ```
  # 0.61.0

  ## New

  - You can hide Maya's picture beside the release notes: turn off Settings → General → Show Maya beside release notes. It goes from Settings → Release Notes and from the What's new window after an update.
  ```
- [ ] **Step 4:** `cargo test --workspace` and `npx vitest run` should both pass.
- [ ] **Step 5: Commit** `chore(release): 0.61.0`.
- [ ] **Step 6: Check in the app** with `pnpm tauri dev`, after quitting the installed Maya. Turn the switch off: the picture goes from Settings → Release Notes at once, and the notes fill the width. Restart: it is still gone, with no flash. Turn it back on: it comes back.
