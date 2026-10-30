# Release Notes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every release carries user-facing notes from `RELEASE_NOTES.md`, shown in the GitHub release and in Settings → Release Notes on the desktop and the phone, whose Settings tab becomes an Android-style list of groups.

**Architecture:** A shell script (`scripts/release-notes.sh`) checks the file's version and assembles the release body; `release-version.sh` and `release.yml` call it. The pages import the file at build time (`?raw`) through `src/release-notes.ts`. The phone's `src/mobile/network.ts` renders either a list of groups or one group's page, with a history entry so Android's Back returns to the list.

**Tech Stack:** POSIX sh, GitHub Actions, TypeScript, Vite, Vitest (jsdom).

**Spec:** `docs/superpowers/specs/2026-10-30-release-notes-design.md`

## Global Constraints

- `RELEASE_NOTES.md` sits at the repository root; its first line is exactly `# <version>`, then `## New` / `## Fixed` bullet lists (an empty group left out), written for users.
- The release body order: `## Release Notes`, the file without its first line, the install text (verbatim from today's `release.yml`, `Built from <sha> on main. ...`), then GitHub's generated list (`--generate-notes`).
- Desktop: `["releaseNotes", "Release Notes"]` is the last entry of `SETTINGS_GROUPS`, after Debug.
- Phone tab label: "Settings" (was "Network"). Groups, in order: Network, Notifications, Voice, Session viewer, Release Notes.
- Phone group contents: Network = This phone, then Join a main (or Key once joined); Notifications = Notifications then Battery; Voice = Voice; Session viewer = Session viewer; Release Notes = the notes.
- The version shown in the app is the file's first line (builder's call, settled here).
- No behaviour of any phone field, switch or button changes; only where it sits.
- The phone does not remember the open group; leaving the tab lands back on the list.
- This pull request bumps `0.41.0` → `0.42.0`, its own commit `chore(release): 0.42.0` carrying the rewritten notes.
- Commits follow Conventional Commits and end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

- A notes file saved with Windows line endings (`# 0.42.0\r`) or trailing spaces on the first line: the check passes and the app shows `Maya 0.42.0`, not a stray `\r`. (Task 1, Task 2)
- A first line written `# v0.42.0` or with no heading: the check fails, naming what it found and what it expected. (Task 1)
- Pressing ← then Android's Back: the first returns to the list, the second leaves the app; Back is never needed twice to get out of a group. (Task 3)
- A status event or a save finishing while a group page is open: the repaint keeps that page and the typed values. (Task 3)
- A save failing on the Voice, Notifications or Session viewer page: its error shows on that page. (Task 3)

---

### Task 1: The notes file, its check and the release body

**Files:**
- Create: `RELEASE_NOTES.md`
- Create: `scripts/release-notes.sh`
- Create: `scripts/test-release-notes.sh`
- Modify: `scripts/release-version.sh` (after the mismatch check, before `echo "version=$version"`)
- Modify: `.github/workflows/ci.yml` (Version job: one more step after "Check the updater manifest script")
- Modify: `.github/workflows/release.yml` (Release job's `run:` and the header comment)

**Interfaces:**
- Produces: `RELEASE_NOTES.md` (used by Task 2's `?raw` import); `sh scripts/release-notes.sh check <version> [file]` exits 0 when the file's first line, with any trailing `\r` and spaces removed, is exactly `# <version>`, else prints to stderr and exits 1; `sh scripts/release-notes.sh body <sha> [file]` prints the release body. `file` defaults to `RELEASE_NOTES.md` at the repository root.

- [ ] **Step 1: Create `RELEASE_NOTES.md` for the version on `main` today**

Task 5 rewrites it for 0.42.0; until then the check must pass on 0.41.0, so it describes what 0.41.0 shipped (from #69):

```markdown
# 0.41.0

## New

- The session window shows how full the session's context is, your account's usage limits with when they reset, and what the session has cost so far.
```

- [ ] **Step 2: Write the failing test `scripts/test-release-notes.sh`**

Modelled on `scripts/test-updater-manifest.sh` (temp dir, `trap`, `FAIL:` messages, exit 1). Cases, each on a fixture file in the temp dir:

- `check 0.42.0` passes on a file whose first line is `# 0.42.0`.
- `check 0.42.0` passes on `# 0.42.0\r` followed by CRLF lines, and on `# 0.42.0  `.
- `check 0.42.0` fails on `# 0.41.0`; stderr contains both `0.41.0` and `0.42.0`.
- `check 0.42.0` fails on `# v0.42.0`, and on a file whose first line is `## New`.
- `check 0.42.0` fails on a path that does not exist; stderr contains `missing`.
- `body abc123` on a file `# 0.42.0`, blank line, `## New`, blank, `- A thing.` prints, in order: a first line `## Release Notes`; `## New`; `- A thing.`; a line starting `Built from abc123 on main. macOS builds are not notarised`; and does not contain `# 0.42.0`.

- [ ] **Step 3: Run it to see it fail**

Run: `sh scripts/test-release-notes.sh`
Expected: fails (`scripts/release-notes.sh` does not exist).

- [ ] **Step 4: Write `scripts/release-notes.sh`**

Header comment in the style of the other scripts (what it does, usage lines). `check`: missing file → `RELEASE_NOTES.md is missing: write the notes for <version>`; mismatch → `RELEASE_NOTES.md is for '<first line>', not <version>: rewrite it for <version>` (exit 1). `body`: `## Release Notes`, blank line, the file from line 2 with leading blank lines dropped and `\r` stripped, blank line, then the install text moved verbatim from `release.yml`'s `--notes` with `${GITHUB_SHA}` replaced by `<sha>`.

- [ ] **Step 5: Run the test to see it pass**

Run: `sh scripts/test-release-notes.sh`
Expected: exits 0, no `FAIL:`.

- [ ] **Step 6: Wire the check and the body in**

- `scripts/release-version.sh`: after the version-mismatch block, `sh scripts/release-notes.sh check "$version"` (it prints its own error; `set -e` stops the script). Add a line to the header comment: it also fails when `RELEASE_NOTES.md` is not for that version.
- `ci.yml` Version job: step `Check the release notes script`, `run: sh scripts/test-release-notes.sh`.
- `release.yml` Release step: `sh scripts/release-notes.sh body "$GITHUB_SHA" > notes.md`, and `gh release create` takes `--notes-file notes.md` instead of `--notes "..."`; keep `--generate-notes`, `--title`, `--target`, `--repo`. Header comment: the release leads with `RELEASE_NOTES.md`.

- [ ] **Step 7: Verify**

Run: `sh scripts/test-release-notes.sh && sh scripts/release-notes.sh check 0.41.0 && sh scripts/release-notes.sh body deadbeef | head -5`
Expected: exit 0; the body starts `## Release Notes`, then `## New`.
Run: `GH_TOKEN=$(gh auth token) sh scripts/release-version.sh`
Expected: `version=0.41.0`, `release=false` (0.41.0 is released), exit 0.

- [ ] **Step 8: Commit**

```bash
git add RELEASE_NOTES.md scripts/release-notes.sh scripts/test-release-notes.sh scripts/release-version.sh .github/workflows/ci.yml .github/workflows/release.yml
git commit -m "feat(release): lead each release with RELEASE_NOTES.md and check its version"
```

---

### Task 2: Release notes on the page, and the desktop's Settings group

**Files:**
- Create: `src/release-notes.ts`
- Create: `src/release-notes.test.ts`
- Modify: `src/settings.ts` (`SETTINGS_GROUPS` at ~344; fill the new section near `debug.append(debugLabel)` at ~904)
- Modify: `src/settings.test.ts` (the sidebar test at ~424 and any other list of all groups)

**Interfaces:**
- Consumes: `RELEASE_NOTES.md` (Task 1); `renderMarkdown(text: string): HTMLElement` from `src/markdown.ts`.
- Produces, in `src/release-notes.ts`:
  - `export interface ReleaseNotes { version: string; body: string }`
  - `export function parseReleaseNotes(text: string): ReleaseNotes`: version from a first line `# <v>` (trimmed, `\r` dropped), body = the rest with leading blank lines dropped; no `# ` first line → `version: ""`, body = whole text.
  - `export const RELEASE_NOTES: ReleaseNotes` (parsed from `import raw from "../RELEASE_NOTES.md?raw"`).
  - `export function renderReleaseNotes(notes: ReleaseNotes = RELEASE_NOTES): HTMLElement`: a `div.release-notes` holding `p.settings__hint` with `Maya <version>` (left out when version is empty), then `renderMarkdown(body)`.

- [ ] **Step 1: Write the failing tests `src/release-notes.test.ts`**

```ts
it("reads the version from the first line and keeps the rest as the body", () => {
  expect(parseReleaseNotes("# 0.42.0\n\n## New\n\n- A thing.\n")).toEqual({ version: "0.42.0", body: "## New\n\n- A thing.\n" });
});
it("ignores Windows line endings and trailing spaces on the version line", () => {
  expect(parseReleaseNotes("# 0.42.0 \r\n\r\n- A thing.\r\n").version).toBe("0.42.0");
});
it("has no version when the first line is not a heading", () => {
  expect(parseReleaseNotes("- A thing.")).toEqual({ version: "", body: "- A thing." });
});
it("renders Maya <version> then the notes as Markdown", () => {
  const el = renderReleaseNotes({ version: "0.42.0", body: "## New\n\n- A thing." });
  expect(el.querySelector(".settings__hint")!.textContent).toBe("Maya 0.42.0");
  expect(el.querySelector("h2")!.textContent).toBe("New");
  expect(el.querySelector("li")!.textContent).toBe("A thing.");
});
it("is built from RELEASE_NOTES.md", () => {
  expect(RELEASE_NOTES.version).toMatch(/^\d+\.\d+\.\d+$/);
});
```

In `src/settings.test.ts`, the sidebar test's three lists gain `"Release Notes"` / `"releaseNotes"` at the end and `aria-current` gets a tenth `null`; add:

```ts
it("shows the release notes, last in the sidebar", () => {
  const el = renderSettings({ ...voiceBase, settingsGroup: "releaseNotes" }, handlers());
  const s = el.querySelector<HTMLElement>(".settings__section[data-group=releaseNotes]")!;
  expect(s.hidden).toBe(false);
  expect(s.querySelector(".settings__heading")!.textContent).toBe("Release Notes");
  expect(s.querySelector(".release-notes .settings__hint")!.textContent).toBe(`Maya ${RELEASE_NOTES.version}`);
});
```

- [ ] **Step 2: Run them to see them fail**

Run: `pnpm vitest run src/release-notes.test.ts src/settings.test.ts`
Expected: FAIL (module missing; sidebar has nine groups).

- [ ] **Step 3: Implement `src/release-notes.ts` and the group**

Add `["releaseNotes", "Release Notes"]` after `["debug", "Debug"]`; destructure `releaseNotes` from `sections` and `releaseNotes.append(renderReleaseNotes())`. Add CSS only if the rendered `.md` headings look out of place in the card (`src/styles.css`, beside the other `.settings__` rules).

- [ ] **Step 4: Run the tests and the type check**

Run: `pnpm vitest run src/release-notes.test.ts src/settings.test.ts && pnpm exec tsc`
Expected: PASS; tsc prints nothing.

- [ ] **Step 5: Commit**

```bash
git add src/release-notes.ts src/release-notes.test.ts src/settings.ts src/settings.test.ts src/styles.css
git commit -m "feat(settings): Release Notes group in the desktop's Settings"
```

---

### Task 3: The phone's Settings, Android style

**Files:**
- Create: `src/mobile/settings-history.ts`
- Create: `src/mobile/settings-history.test.ts`
- Modify: `src/mobile/network.ts` (model, handlers, `renderNetwork`, `initNetwork`; header comment)
- Modify: `src/mobile/network.test.ts`
- Modify: `src/mobile/mobile.css` (beside the `.network` rules at ~63-72)
- Modify: `mobile/web/index.html:19` (`>Network<` → `>Settings<`)

**Interfaces:**
- Consumes: `RELEASE_NOTES`, `renderReleaseNotes()` (Task 2); the `maya:tab` window event `makeTabs` dispatches with `detail: Tab`.
- Produces:
  - In `src/mobile/settings-history.ts`, mirroring `card-history.ts`: `export interface GroupHistory { opened(group: string): void; closed(): void; holding(): boolean }` and `export function makeGroupHistory(h: History = window.history): GroupHistory`. `opened` pushes `{ settings: group }` (replaces it if already holding one); `closed` calls `h.back()` only while holding; `holding` is true while `h.state.settings` is a string.
  - In `src/mobile/network.ts`:
    - `export type PhoneGroup = "network" | "notifications" | "voice" | "viewer" | "releaseNotes";`
    - `export const PHONE_GROUPS: readonly (readonly [PhoneGroup, string])[]` = Network, Notifications, Voice, Session viewer, Release Notes, in that order.
    - `NetworkModel` gains `group?: PhoneGroup | null` (absent or null = the list).
    - `NetworkHandlers` gains `onOpenGroup(g: PhoneGroup): void` and `onBack(): void`.
    - `export function groupSummary(m: NetworkModel, g: PhoneGroup): string`.

Summary lines (copy fixed here):

| Group | Summary |
|---|---|
| network | joined and connected with a main name: `Connected to <mainName>`; joined otherwise: `Reconnecting…`; not joined: `Not joined` |
| notifications | `!notificationsAllowed`: `Off in Android settings`; else the ones on, joined by `, `, from `Awaiting decision`, `Completed`; none: `Off` |
| voice | `Speak on|off, wake word on|off` |
| viewer | `Chat` or `Terminal` (chat when absent) |
| releaseNotes | `Maya <RELEASE_NOTES.version>` |

Markup: the list is `div.settings__body.settings-list` of `button.settings-list__item[data-group=<g>]`, each holding `span.settings-list__name` and `span.settings-list__summary` (chevron by CSS). A page is `div.settings__body.network` starting with `div.settings-page__bar` (a `button[data-action=back][aria-label=Back]` showing `←`, and `h2.settings-page__title` with the group's name), then `p.settings__error` when `m.error` is set (moved out of the This phone card, so it shows on every page), then the group's cards as `renderNetwork` builds them today. The Release Notes page is one `section.settings__section` holding `renderReleaseNotes()`.

- [ ] **Step 1: Write the failing tests**

`src/mobile/settings-history.test.ts`, with a fake `History` (an object with `state`, `pushState`, `replaceState`, `back` as `vi.fn`s that update `state`): opening pushes `{ settings: "voice" }`; opening again while holding replaces instead of pushing; `closed()` calls `back` once while holding and not at all otherwise.

`src/mobile/network.test.ts`: `handlers()` gains `onOpenGroup: vi.fn(), onBack: vi.fn()`; every existing `renderNetwork` test sets the group whose cards it checks (`group: "network"`, `"notifications"`, `"voice"`, `"viewer"`), and the `initNetwork` tests open the group they use first (`pane.querySelector("[data-group=notifications]").click()`). New tests:

```ts
it("opens on the list of groups, each with its summary", () => {
  const el = renderNetwork(base, handlers());
  const items = [...el.querySelectorAll<HTMLElement>(".settings-list__item")];
  expect(items.map((i) => i.querySelector(".settings-list__name")!.textContent)).toEqual(["Network", "Notifications", "Voice", "Session viewer", "Release Notes"]);
  expect(items.map((i) => i.querySelector(".settings-list__summary")!.textContent)).toEqual(["Connected to laptop", "Awaiting decision", "Speak on, wake word off", "Chat", `Maya ${RELEASE_NOTES.version}`]);
  expect(el.querySelector("input, select")).toBeNull();
});
it("summarises the network and notifications in each state", () => {
  expect(groupSummary({ ...base, status: off }, "network")).toBe("Not joined");
  expect(groupSummary({ ...base, status: { ...joinedStatus, assistant: { connected: false, mainName: null, error: null } } }, "network")).toBe("Reconnecting…");
  expect(groupSummary({ ...base, notifyAwaiting: false }, "notifications")).toBe("Off");
  expect(groupSummary({ ...base, notifyCompleted: true }, "notifications")).toBe("Awaiting decision, Completed");
  expect(groupSummary({ ...base, notificationsAllowed: false }, "notifications")).toBe("Off in Android settings");
});
it("taps into a group and back out", () => {
  const h = { ...handlers(), onOpenGroup: vi.fn(), onBack: vi.fn() };
  renderNetwork(base, h).querySelector<HTMLElement>("[data-group=voice]")!.click();
  expect(h.onOpenGroup).toHaveBeenCalledWith("voice");
  const page = renderNetwork({ ...base, group: "voice" }, h);
  expect(page.querySelector(".settings-page__title")!.textContent).toBe("Voice");
  expect(page.querySelector("input[name=speak]")).not.toBeNull();
  expect(page.querySelector("input[name=notifyAwaiting]")).toBeNull();
  page.querySelector<HTMLElement>("button[data-action=back]")!.click();
  expect(h.onBack).toHaveBeenCalled();
});
it("puts Battery under Notifications and the notes under Release Notes", () => {
  expect(renderNetwork({ ...base, group: "notifications" }, handlers()).querySelector("button[data-action=battery]")).not.toBeNull();
  expect(renderNetwork({ ...base, group: "releaseNotes" }, handlers()).querySelector(".release-notes")).not.toBeNull();
});
it("shows an error on whichever page is open", () => {
  for (const group of ["network", "notifications", "voice", "viewer"] as const) {
    expect(renderNetwork({ ...base, group, error: "boom" }, handlers()).querySelector(".settings__error")!.textContent).toBe("boom");
  }
});
```

`initNetwork` tests (jsdom, `tauri.replies` as the existing ones set them up): tapping a group shows its page and pushes a history entry; `history.back()` (then awaiting the `popstate`) returns to the list; ← returns to the list and leaves `history.state` without `settings`; a `network` event (`tauri.listeners.network({ payload: joinedStatus })`) while on the Voice page keeps the Voice page; `window.dispatchEvent(new CustomEvent("maya:tab", { detail: "sessions" }))` while on a page returns the pane to the list.

- [ ] **Step 2: Run them to see them fail**

Run: `pnpm vitest run src/mobile/settings-history.test.ts src/mobile/network.test.ts`
Expected: FAIL (module missing; no list).

- [ ] **Step 3: Implement**

`makeGroupHistory` as specified. In `renderNetwork`, keep today's card-building code and choose which cards to append by `m.group`; the list when no group. In `initNetwork`: `onOpenGroup` sets `model.group`, calls `history.opened(g)` and paints; `onBack` clears it, calls `history.closed()` and paints; a `popstate` listener clears `model.group` and paints when a group is open and `history.holding()` is false; a `maya:tab` listener does what `onBack` does when `detail !== "settings"` and a group is open. Update the file's header comment ("The phone's Settings tab: ...").

- [ ] **Step 4: Style the list and the bar in `src/mobile/mobile.css`**

Rows full width, name above a muted summary, a `›` chevron on the right via `::after`, a divider between rows, at least 48px tall; the bar's ← button and title on one line. The page keeps today's `.settings__body.network` grid, so the bar and the page's error span every column (`grid-column: 1 / -1`). Use the theme's variables (`--text`, `--muted`, `--border`, `--panel`).

- [ ] **Step 5: Run the phone's tests, the whole suite and the type check**

Run: `pnpm vitest run && pnpm exec tsc`
Expected: all pass; tsc prints nothing.

- [ ] **Step 6: Build the phone's page**

Run: `pnpm exec vite build --config vite.mobile.config.ts`
Expected: builds into `dist-mobile/` with no error (proves `../RELEASE_NOTES.md?raw` resolves from the `mobile/web` root).

- [ ] **Step 7: Commit**

```bash
git add src/mobile/settings-history.ts src/mobile/settings-history.test.ts src/mobile/network.ts src/mobile/network.test.ts src/mobile/mobile.css mobile/web/index.html
git commit -m "feat(mobile): Settings tab as a list of groups, with Release Notes"
```

---

### Task 4: The rule and the docs

**Files:**
- Modify: `CLAUDE.md` ("Pull requests bump the version")
- Modify: `docs/DEVELOPING.md` ("Releasing", ~598-613)
- Modify: `README.md` (~98-101 updates; ~229, ~234, ~247 the phone's "Network tab")

- [ ] **Step 1: `CLAUDE.md`**

Add bullets to "Pull requests bump the version": with the bump, rewrite `RELEASE_NOTES.md` for the new version (`# <version>`, then `## New` and `## Fixed`, an empty group left out; bullets for users, in plain words, covering everything the version ships), committed in the same `chore(release): <version>` commit; redo it whenever the bump is redone; before every push, check it still matches the version and describes everything the branch ships, since later commits change that. Note that CI fails when its version differs from the version files.

- [ ] **Step 2: `docs/DEVELOPING.md`**

In "Releasing": the example commit becomes `git commit -am "chore(release): 0.2.0"` after rewriting `RELEASE_NOTES.md`; the release's notes lead with `RELEASE_NOTES.md` under "Release Notes" (assembled by `scripts/release-notes.sh`, tested by `scripts/test-release-notes.sh` in CI), followed by the generated list; the version check also fails when `RELEASE_NOTES.md` is for another version. Mention the app shows the same file in Settings → Release Notes, built in at build time.

- [ ] **Step 3: `README.md`**

After "Settings has Check for updates": Settings → Release Notes shows what the installed version brought. Phone: "the Network tab" → "Settings › Notifications" (Keep Maya awake, and turning notifications off) and "Settings › Voice" (Speak).

- [ ] **Step 4: Verify nothing still calls the phone's tab "Network tab"**

Run: `grep -n "Network tab" README.md docs/DEVELOPING.md`
Expected: no output.

- [ ] **Step 5: Commit**

```bash
git add CLAUDE.md docs/DEVELOPING.md README.md
git commit -m "docs: release notes rule, and the phone's Settings in the README"
```

---

### Task 5: Release 0.42.0

**Files:**
- Modify: `RELEASE_NOTES.md`, and the version files via `scripts/set-version.sh`

- [ ] **Step 1: Bump from `main`'s version**

Run: `git fetch origin && git show origin/main:package.json | grep '"version"'`
Expected: `0.41.0` (if `main` moved, bump its minor instead, and use that version below).
Run: `sh scripts/set-version.sh 0.42.0`

- [ ] **Step 2: Rewrite `RELEASE_NOTES.md` for 0.42.0**

```markdown
# 0.42.0

## New

- Settings has a Release Notes page, on the desktop and on the phone, showing what this version brought.
- The phone's Network tab is now Settings: a list of groups (Network, Notifications, Voice, Session viewer, Release Notes), each opening its own page.
```

Add anything else that shipped on `main` after 0.41.0, if `main` moved.

- [ ] **Step 3: Verify**

Run: `GH_TOKEN=$(gh auth token) sh scripts/release-version.sh && pnpm vitest run && pnpm exec tsc`
Expected: `version=0.42.0`, `release=true`; tests pass; tsc silent.

- [ ] **Step 4: Commit**

```bash
git add RELEASE_NOTES.md package.json src-tauri/tauri.conf.json mobile/tauri.conf.json Cargo.toml Cargo.lock
git commit -m "chore(release): 0.42.0"
```
