# Settings Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild every desktop and phone Settings screen in the approved
card-and-row design, using one shared set of builders.

**Architecture:** A new `src/settings-ui.ts` returns plain DOM parts (a
screen head, groups, cards, rows, switches, two-way choices, states and
collapsible sources). `renderSettings` in `src/settings.ts`, `renderDevices`
in `src/devices.ts` and `renderNetwork` in `src/mobile/network.ts` are
rebuilt from those parts one screen at a time. Each control keeps its
`name`, value and handler. Repaints keep today's model: every desktop
section is replaced except Devices, which `morph` patches. One new CSS block
in `src/styles.css`, scoped to `.settings.pane`, replaces the old Settings
layout rules, and `src/mobile/mobile.css` only resizes the parts for touch.

**Tech Stack:** Vanilla TypeScript DOM, CSS, vitest with jsdom (with
`styles.css` and `mobile.css` loaded), and Playwright WebKit for the visual
checks.

**Spec:** `docs/superpowers/specs/2026-11-12-settings-redesign-design.md`,
and the approved mockups beside it in `2026-11-12-settings-redesign/`
(`desktop.html`, `phone.html`). The mockups' copy and structure are the
spec, and their CSS is the starting point for the new block.

## Global Constraints

- No setting is added, removed or changes meaning. Every field keeps its
  `name`, `value`, `data-*` attributes and handler calls.
- Repaints stay as they are. `patchPanel` replaces every desktop section
  except Devices, and Devices is patched with `morph`. On the phone, only
  the Devices page goes through `morph`. The Devices markup must be the
  same elements in the same order for the same model.
- State the user changes without a save (which Work Items source is open,
  whether a custom tool's details is open) lives in the model.
- New CSS is scoped to `.settings.pane` (desktop `#settings`, phone
  `#settings`). The Voice popover (`#voice-panel.settings.popover`) must
  look as it did.
- New tokens on `:root`: `--rule: #221e16; --field: #0f0e0b; --soft:
  #1c1913; --danger: #e5786b;`. The light block
  (`:root[data-theme="light"]`) gets light values for each, with
  `--danger: #c0262c`.
- Spacing steps 4/8/12/16/20/24/32/36/56. Radii: card 12, box inside a card
  10, field and button 8, switch and badge a full pill.
- Class names use the existing `settings__` prefix. Where a test already
  finds an element by an old class (`settings__section`, `settings__nav`,
  `settings__error`, `settings__code`, `settings__code-expiry`,
  `settings__addresses`, `settings__devices`, `settings__assistant`,
  `settings__status--network`, `settings__version`, `settings__update-status`,
  `settings__rule`, `settings__git`, `settings__custom`, `settings__keys`,
  `settings__progress`, `settings__git-test`), keep that class on the
  element that plays the same part, beside the new ones.
- A test that checks old markup is changed to check the same behaviour in
  the new markup. It is never dropped.
- Phone tap targets are at least 40px, and rows at least 56px.
- All user-facing copy (ledes, notes, help lines) is exactly as the spec
  writes it.
- Before the first code change, run the `lore` skill on "Settings
  rendering, morph and repaint, settings tests" and read what earlier
  sessions learned.

## Review Focus

1. **A repaint while a source is open or a field is focused.** A status push
   (voice, network, config-changed) repaints Settings. The open Work Items
   source must stay open, its custom tool's details must stay open, and the
   focused field keeps focus and caret (`restoreFocus`). This is tested in
   Task 6.
2. **The light theme.** Every new part must be readable on
   `data-theme="light"`: hairlines, sunken fields, the switch off and on,
   danger text, the pairing card. This is checked in Task 13's screenshots.
3. **Long values.** A long list command, device name, rule summary or Git
   test message must wrap or truncate with an ellipsis, never push a
   control out of its card. Truncation is tested in Task 6; the rest is
   checked in Task 13's screenshots.
4. **Narrow windows.** At 1100, 800 and 500px wide, the shell, rows and
   Release Notes art follow the spec's breakpoints (900 and 560 for the
   shell, 1100 for the art). This is checked in Task 13's screenshots.
5. **Keyboard use.** A source head is a `button` with `aria-expanded` that
   opens and folds it with Enter and Space. A switch inside the head does
   not fold the card. Every control shows the gold focus ring. Tested in
   Tasks 1 and 6.

---

### Task 1: The builders and their CSS

**Files:**
- Create: `src/settings-ui.ts`
- Create: `src/settings-ui.test.ts`
- Modify: `src/styles.css` (add the tokens to `:root` at line 4 and to the
  light block at line 596; add the new block after the existing Settings
  rules, around line 104)

**Interfaces:**
- Produces (all in `src/settings-ui.ts`):
  - `screenHead(title: string, lede: string): HTMLElement`: a
    `div.settings__head` holding `h2.settings__title` and
    `p.settings__lede`.
  - `group(opts: { title?: string; note?: string | Node; foot?: string | Node }, ...children: Node[]): HTMLElement`:
    a `div.settings__group` holding `h3.settings__group-title`,
    `p.settings__group-note`, the children, then `p.settings__group-foot`.
    Each optional part is present only when given.
  - `card(...rows: Node[]): HTMLElement`: `div.settings__card`.
  - `row(opts: { label: string | Node; help?: string | Node | null; control?: Node | Node[]; tag?: "label" | "div"; stack?: boolean }): HTMLElement`:
    a `label` (default when `control` is one labelable element) or `div`,
    with class `settings__row` (plus `settings__row--stack` when `stack`).
    It holds `span.settings__row-text` (containing `span.settings__label`
    and `span.settings__help`), then `div.settings__control` holding the
    control(s). When the control has a `name`, the help gets
    `id="settings-help-<name>"` and the control gets `aria-describedby`
    pointing at it.
  - `cardHead(opts: { label: string | Node; help?: string; control?: Node; order?: number }): HTMLElement`:
    a `row` with `settings__row--head`. With `order`, it starts with
    `span.settings__order` holding the number.
  - `footRow(left: string | Node, right?: Node): HTMLElement`:
    `div.settings__row.settings__row--foot`.
  - `toggle(name: string | null, checked: boolean, onChange: (on: boolean) => void, ariaLabel?: string): HTMLInputElement`:
    `input[type=checkbox].settings__toggle`.
  - `choice<T extends string>(name: string, options: readonly (readonly [T, string])[], value: T, onChange: (v: T) => void, disabled?: boolean): HTMLElement`:
    `div.settings__seg[role=radiogroup]` holding one `label` per option,
    each with `input[type=radio][name][value]` and a `span` of text.
  - `state(text: string, tone: "on" | "main" | "off"): HTMLElement`:
    `span.settings__state.settings__state--<tone>`.
  - `btn(text: string, opts: { action?: string; kind?: "plain" | "quiet" | "danger" | "gold"; disabled?: boolean }, onClick?: () => void): HTMLButtonElement`:
    `button[type=button].settings__btn` (plus `settings__btn--<kind>`
    unless plain), with `data-action` when given.
  - `addButton(text: string, action: string, onClick: () => void): HTMLButtonElement`:
    `button.settings__add[data-action]`.
  - `notice(text: string, tone: "error" | "info"): HTMLElement`:
    `div.settings__notice` (plus `settings__error` when `tone` is
    `"error"`).
  - `source(opts: { key: string; name: string; tag?: string; summary: string; monoSummary?: boolean; stateText: string; stateOn: boolean; open: boolean; onToggle: (open: boolean) => void; control: HTMLInputElement; body: Node[] }): HTMLElement`:
    `div.settings__card.settings__source[data-source=<key>]`, with
    `data-open` when open, and `settings__source--off` when `!stateOn`.
    The head is `div.settings__source-head` holding a
    `button.settings__source-toggle[aria-expanded]` (a chevron span,
    `span.settings__source-name`, the optional `span.settings__source-tag`,
    and `span.settings__source-sum`, with `mono` when `monoSummary`), then
    `span.settings__source-state` and the `control`. The body is
    `div.settings__source-body`. A click on the toggle flips `data-open`
    and `aria-expanded` in place, then calls `onToggle(open)`.

- [ ] **Step 1: Write the failing tests** in `src/settings-ui.test.ts`:
  - `row` with a select named `agent` and help "x": the root is a `LABEL`
    with class `settings__row`. `.settings__label` reads the label,
    `.settings__help#settings-help-agent` reads "x", and the select's
    `aria-describedby` is `settings-help-agent`.
  - `row` with two buttons as control: the root is a `DIV`.
  - `row({ stack: true, ... })` has `settings__row--stack`.
  - `group({ title: "Agent", note: "n", foot: "f" }, card())`: children in
    the order title, note, card, foot. `group({}, card())` has only the
    card.
  - `toggle("listen", true, fn)`: a checked checkbox with class
    `settings__toggle` and `name="listen"`. Changing it calls `fn(false)`.
  - `choice("gitClone-github", [["ssh","SSH"],["https","HTTPS"]], "https", fn)`:
    two radios named `gitClone-github`, the `https` one checked. Clicking
    `ssh` calls `fn("ssh")`.
  - `state("Installed","on")` has classes `settings__state` and
    `settings__state--on`.
  - `notice("boom","error")` has classes `settings__notice` and
    `settings__error`.
  - `source({ open: false, ... })`: no `data-open`, the toggle has
    `aria-expanded="false"`, and `.settings__source-sum` holds the summary.
    Clicking the toggle sets `data-open`, sets `aria-expanded="true"`, and
    calls `onToggle(true)`. Clicking `control` neither flips it nor calls
    `onToggle`. Pressing Enter on the focused toggle opens it (it is a
    `button`).
  - `cardHead({ label: "ElevenLabs", order: 1 })`: `.settings__order`
    reads "1".

- [ ] **Step 2: Run** `pnpm vitest run src/settings-ui.test.ts`.
  Expected: FAIL (module not found).

- [ ] **Step 3: Implement** `src/settings-ui.ts` with the signatures above.

- [ ] **Step 4: Add the CSS** to `src/styles.css`: the four tokens (dark on
  `:root`, light in the light block) and one block scoped to
  `.settings.pane` for every class above. Take the rules from the
  mockup's `<style>` (`desktop.html`: `.card`, `.row*`, `.switch`, `.seg`,
  `.btn*`, `.add`, `.state*`, `.src*`, `.order`, fields), renamed to the
  `settings__` classes. Fields inside `.settings.pane .settings__card` get
  the mockup's field look (36px tall, `--field` background, radius 8, gold
  focus ring `box-shadow: 0 0 0 3px rgba(217,171,60,.16)`). Respect
  `prefers-reduced-motion` for the switch and chevron transitions.

- [ ] **Step 5: Run** `pnpm vitest run src/settings-ui.test.ts`.
  Expected: PASS.

- [ ] **Step 6: Commit** `feat(settings): add the Settings building blocks`.

### Task 2: The desktop shell, screen heads and the error notice

**Files:**
- Modify: `src/settings.ts` (`SETTINGS_GROUPS` area, lines 389-420;
  `renderSettings` from line 638; the `model.error` line at the end of
  `renderSettings`)
- Modify: `src/styles.css` (lines 60-104: the shell and the old caps)
- Test: `src/settings.test.ts`

**Interfaces:**
- Consumes: `screenHead`, `notice` (Task 1).
- Produces:
  - `export const SETTINGS_NAV: readonly (readonly SettingsGroup[])[] = [["general","keyboard"],["sessions","git","reviews","workItems"],["notifications","voice"],["network","devices"],["debug","releaseNotes"]]`
  - `export const SETTINGS_LEDES: Record<SettingsGroup, string>`, with the
    spec's ledes word for word (General "The agent behind Maya, and how
    she looks.", …, Release Notes "What changed in each version of
    Maya.").
  - Each `section(title, key)` now starts with `screenHead(title,
    SETTINGS_LEDES[key])` in place of `h2.settings__heading`. The section
    keeps `settings__section settings__section--<key>` and `data-group`.
    It loses its card look: the cards are inside it now.
  - The nav renders one `div.settings__nav-group` per `SETTINGS_NAV`
    entry. Its buttons keep `settings__nav-item`, `data-group` and the
    pointerdown/click behaviour.

- [ ] **Step 1: Write the failing tests** in `src/settings.test.ts`:
  - Each section's `.settings__title` is the group's title, and its
    `.settings__lede` equals `SETTINGS_LEDES[key]`. Check all twelve.
  - The nav holds 5 `.settings__nav-group` elements, with 2, 4, 2, 2 and 2
    items in `SETTINGS_NAV` order.
  - With `model.error = "boom"`, a `.settings__notice.settings__error`
    reading "boom" is the first child of `.settings__content`, above every
    section. This replaces the test at `settings.test.ts:820`, which
    expected the error after the Network role label.

- [ ] **Step 2: Run** `pnpm vitest run src/settings.test.ts`. Expected: the
  new tests FAIL.

- [ ] **Step 3: Implement.** Add the constants, change `section()`, group
  the nav, and move the error to the top of the content as a notice.

- [ ] **Step 4: Replace the shell CSS** (lines 60-104 and 90): centre
  `.settings__body--sidebar` as a grid `208px minmax(0, 760px)` with a
  56px gap, `justify-content: center`, and 40px top padding. Remove
  `.settings.pane { max-width: 1400px }`, the body's `900px` and the
  content's `680px`. Make the nav sticky (`top: 24px`), with a `--rule`
  hairline and 14px between nav groups. Under 900px the nav becomes a
  wrapping row above the content; under 560px `.settings__row` stacks.
  Remove the old section card look (`.settings__section` border,
  background and padding) and the old `.settings__heading`.

- [ ] **Step 5: Run** `pnpm vitest run src/settings.test.ts src/settings-flow.test.ts`.
  Expected: PASS. Fix any test that checked `.settings__heading` text so it
  checks `.settings__title`.

- [ ] **Step 6: Commit** `feat(settings): centred shell, screen heads and grouped sidebar`.

### Task 3: General, Keyboard, Notifications and Debug

**Files:**
- Modify: `src/settings.ts` (General ~700-790, `renderKeyboard` 447-517,
  Notifications ~935-945 and ~1405-1430, Debug ~955-962)
- Test: `src/settings.test.ts`

**Interfaces:**
- Consumes: `group`, `card`, `row`, `toggle`, `btn` (Task 1).

- [ ] **Step 1: Write the failing tests:**
  - General has three `.settings__group` elements titled Agent,
    Appearance and Updates. The agent `select[name=agent]` sits in a
    `.settings__row` whose `.settings__help` is today's agent hint. The
    Updates row's `.settings__label` reads "Maya \<version>". With
    `updateStatus` set, its help reads that status (keep the
    `settings__update-status` class). With `updatesOn` false there is no
    Check for updates button.
  - Keyboard: each `[data-keys]` row is a `.settings__row` (keeping
    `settings__keys`). The group foot reads "An Enter press in neither list
    does what the box does by itself." The Reset to defaults button follows
    the group.
  - Notifications: `input[name=notify]` and `input[name=speak]` have class
    `settings__toggle`. The speak row's help reads `"needs a decision", "is
    finished"`. When `focusVisible` is false, today's Focus hint is the
    group foot.
  - Debug: `input[name=debug-mode]` (the existing name) is a
    `settings__toggle`, in a row labelled "Debug mode" with help "Shows the
    Debug tab and Maya's own card."
  - The existing tests that use `box.closest("label")?.textContent` keep
    passing, because those rows are `label`s.

- [ ] **Step 2: Run** `pnpm vitest run src/settings.test.ts`. Expected: the
  new tests FAIL.

- [ ] **Step 3: Implement** the four screens from the builders, matching
  the spec's "Each desktop screen" for each. Key chips get class
  `settings__kbd`, with its CSS from the mockup's `.kbd`.

- [ ] **Step 4: Run** `pnpm vitest run src/settings.test.ts src/settings-flow.test.ts`.
  Expected: PASS.

- [ ] **Step 5: Commit** `feat(settings): General, Keyboard, Notifications and Debug in the new design`.

### Task 4: Sessions and Pull requests

**Files:**
- Modify: `src/settings.ts` (Sessions ~790-905, Pull requests ~905-955)
- Test: `src/settings.test.ts`

**Interfaces:**
- Consumes: `group`, `card`, `row`, `state`, `btn`, `toggle` (Task 1).

- [ ] **Step 1: Write the failing tests:**
  - Sessions has groups titled "Session tracking" (note "Hooks let the
    agents tell Maya what they are doing, rather than her guessing from
    transcripts."), "Board" and "New sessions".
  - Session tracking has three rows, labelled "Claude Code hook", "Usage
    status line" and "Codex hook". Each has a `.settings__state`: tone `on`
    with "Installed" when installed, `off` with "Not installed" otherwise.
    Each keeps its existing button and `data-action` (the test at
    `settings.test.ts:172-177` still finds "Install Claude hook" or "Remove
    Claude hook" as the button's accessible text; keep those texts as the
    button's `aria-label` if the visible text becomes "Install" or
    "Remove"). Today's status sentences, and the status line's blocked-by
    text when set, are the rows' help lines.
  - Board: `input[name=timeout]` is followed by a `span` reading "minutes".
  - Pull requests: groups "Reviews" and "Your pull requests".
    `textarea[name=reviewPrompt]` is in a `.settings__row--stack` with
    `rows=7`. `input[name=notifyPrReview]` is a `settings__toggle`.
  - New sessions: `select[name=newSessionTerminal]` (where it shows today)
    is in a row labelled "Open in", and `input[name=projectsDir]` in one
    labelled "Projects directory".

- [ ] **Step 2: Run** `pnpm vitest run src/settings.test.ts`. Expected: the
  new tests FAIL.

- [ ] **Step 3: Implement** both screens.

- [ ] **Step 4: Run** `pnpm vitest run src/settings.test.ts src/settings-flow.test.ts`.
  Expected: PASS.

- [ ] **Step 5: Commit** `feat(settings): Sessions and Pull requests in the new design`.

### Task 5: Git

**Files:**
- Modify: `src/settings.ts` (the Git loop, ~965-1090)
- Test: `src/settings.test.ts`

**Interfaces:**
- Consumes: `card`, `cardHead`, `row`, `footRow`, `toggle`, `choice`,
  `state`, `btn` (Task 1).

- [ ] **Step 1: Write the failing tests:**
  - One `.settings__card.settings__git[data-provider]` per forge. Its head
    reads "GitHub" or "GitLab" and holds
    `input[name=gitEnabled-<p>].settings__toggle`. A card that is off has
    `settings__card--off`.
  - The token kind is a `.settings__seg` with radios
    `gitTokenKind-<p>` (`stored` and `env`). The clone protocol is a
    `.settings__seg` with radios `gitClone-<p>` (`ssh` and `https`). The
    handlers fire as today.
  - The Token row's help reads "Needs the repo scope." (GitHub) or "Needs
    read_api." (GitLab).
  - With a stored token, the token field's help reads "Saved in
    \<store>." and a quiet Clear (`data-action=git-clear`) is in the row.
  - The foot row holds the Test button (`data-action=git-test`) and, when
    `gitStatus[p].test` is set, a `.settings__git-test` with that text.
  - The screen's lede carries the scope line, so today's `gitHint`
    paragraph is gone.

- [ ] **Step 2: Run** `pnpm vitest run src/settings.test.ts`. Expected: the
  new tests FAIL.

- [ ] **Step 3: Implement** the Git screen.

- [ ] **Step 4: Run** `pnpm vitest run src/settings.test.ts src/settings-flow.test.ts`.
  Expected: PASS.

- [ ] **Step 5: Commit** `feat(settings): Git in the new design`.

### Task 6: Work Items: collapsible sources, rule boxes and the work prompt

**Files:**
- Modify: `src/settings.ts` (the model, ~93-197; the Work Items blocks,
  ~1090-1405; `initSettings`, to seed `sourcesOpen`)
- Modify: `src/styles.css` (rule boxes; remove `.settings__rule*` at
  457-464, the `.settings input { width: 70px }` reach into rules, and
  `.settings__custom pre` at 642)
- Test: `src/settings.test.ts`, `src/settings-flow.test.ts`

**Interfaces:**
- Consumes: `group`, `card`, `row`, `footRow`, `source`, `toggle`,
  `addButton`, `btn` (Task 1).
- Produces:
  - In `SettingsModel`: `sourcesOpen?: Record<string, boolean>` (keyed
    `github`, `gitlab` or a custom tool's `id`) and
    `customDetailsOpen?: Record<string, boolean>` (keyed by tool `id`).
  - `export const SOURCES_OPEN_KEY = "maya.settingsSources"`.
  - `export function storedSourcesOpen(): Record<string, boolean>`: reads
    the JSON from localStorage, returning `{}` when it is missing,
    unreadable or not an object.
  - `export function ruleSummary(rule: ItemRule): string`.
  - `export function forgeSummary(items: ForgeItems): string`.

- [ ] **Step 1: Write the failing tests:**
  - `ruleSummary`:
    - `{assignee:{kind:"me"},labels:[],repos:[]}` gives "Assigned to me".
    - `{kind:"user",login:"alice"}` gives "Assigned to alice", and
      `{kind:"user",login:""}` gives "Assigned to someone".
    - `{kind:"anyone"}` gives "Anyone's issues".
    - With labels `["a","b","c"]`, ", labelled a, b or c" is added.
    - With repos `["x/y","x/z"]`, ", in x/y or x/z" is added.
    - With both: "Assigned to me, labelled bug, in dosaki/maya".
  - `forgeSummary`: 1 rule and no start status gives "1 rule"; 2 rules
    with `startStatus` "in progress" gives "2 rules; Start sets status to
    in progress".
  - Rendering: the Sources group (note "Open one to change what the
    Issues tab picks up from it.") holds `[data-source=github]`,
    `[data-source=gitlab]` and one `[data-source=<id>]` per custom tool, in
    that order, then a `.settings__add` reading "+ Add a custom tool".
  - With no `sourcesOpen`, no source has `data-open`.
    `.settings__source-sum` reads `forgeSummary(...)` for a forge and the
    list command (with `mono`) for a custom tool. The custom tool's
    `.settings__source-tag` reads "Custom tool".
  - `.settings__source-state` reads "On" when that source's switch
    (`wi-github-enabled`, `ct-0-enabled`) is on, and "Off" when it is off.
    When the Git connection for that forge is off, it reads "Git off", and
    `.settings__source-sum` reads "GitHub is off in Settings → Git.".
  - Clicking `[data-source=github] .settings__source-toggle` sets
    `model.sourcesOpen.github = true`, writes `{"github":true}` to
    `localStorage["maya.settingsSources"]`, and calls no save handler.
    A second render with that model has `[data-source=github][data-open]`.
  - Each rule is a `.settings__rule` box whose `.settings__rule-head`
    reads `ruleSummary(rule)` and holds a quiet Remove
    (`data-action=wi-remove-rule`). Typing in `wi-labels` updates that
    head on `input`.
  - A custom tool's Name row holds `input[name=ct-0-name]`. "Remove this
    tool" (`data-action=ct-remove`) is in the body's foot, not the head.
  - Clicking "+ Add a custom tool" opens the new source:
    `sourcesOpen[<new id>]` is true.
  - The work prompt is a `.settings__row--stack` holding
    `textarea[name=workPrompt]` (keep the existing name).
  - A long list command does not widen the head: `.settings__source-sum`
    has `text-overflow: ellipsis` and `white-space: nowrap` in its computed
    style (vitest loads `styles.css`).
  - In `settings-flow.test.ts`: open GitHub, put focus in
    `input[name=wi-statusLabels]`, then dispatch `maya:config-changed`.
    After the repaint GitHub is still open, and the focused field is
    `wi-statusLabels` again (Review Focus 1). Do the same for a custom
    tool's details: open them and repaint, and they are still open.

- [ ] **Step 2: Run** `pnpm vitest run src/settings.test.ts src/settings-flow.test.ts`.
  Expected: the new tests FAIL.

- [ ] **Step 3: Implement.** Add the model fields, `SOURCES_OPEN_KEY` and
  `storedSourcesOpen` (seeded into the model in `initSettings`, next to
  `storedSettingsGroup()`), `ruleSummary` and `forgeSummary`. Rebuild each
  forge box and each custom tool box with `source()`. The forge body follows
  the spec: rules as a stack row, then Status labels, On Start and
  Tracker-only repositories, with the token hint as the foot. When the Git
  connection is off, keep today's disabling and the "Open Git settings"
  link (`data-action` as today) at the top of the body. A custom tool's
  `details` gets `open` from `customDetailsOpen[id]`, and its `toggle`
  event writes the model. `onToggle` writes `model.sourcesOpen` and
  localStorage (try/catch, as `SETTINGS_GROUP_KEY` does), with no repaint.

- [ ] **Step 4: Fix the rule field width in CSS.** Inside `.settings__rule`
  every input and select is `width: 100%`. The rule body is a grid
  `120px minmax(0, 1fr)`, and the old `.settings input { width: 70px }`
  no longer reaches it.

- [ ] **Step 5: Run** `pnpm vitest run src/settings.test.ts src/settings-flow.test.ts`.
  Expected: PASS. Update the Git-off test (~1519-1522) to the new place of
  the message, keeping its checks that the fields are disabled and the link
  opens the Git group.

- [ ] **Step 6: Commit** `feat(settings): Work Items sources fold up and show whether they are on`.

### Task 7: Voice

**Files:**
- Modify: `src/settings.ts` (Speaking ~1580-1725, Listening ~1430-1590)
- Test: `src/settings.test.ts`, `src/settings-flow.test.ts`

**Interfaces:**
- Consumes: `group`, `card`, `cardHead`, `row`, `footRow`, `toggle`, `btn`
  (Task 1).

- [ ] **Step 1: Write the failing tests:**
  - Speaking's note reads "Maya speaks with the first voice that works, in
    this order." The `[data-for=voice-order]` element stays, as that note.
  - There are three cards with heads ordered 1, 2 and 3, labelled
    "ElevenLabs", "Voice server" and `builtinVoiceName()`.
    `input[name=elevenEnabled]` and `input[name=serverEnabled]` are
    `settings__toggle`s in the heads. Card 2's head help reads "Any
    OpenAI-compatible speech server.", and card 3's reads "Built in; always
    there."
  - A card whose switch is off has `settings__card--off`. Its fields are
    still rendered as today (the ElevenLabs key row shows only when
    enabled, as today).
  - The volume card holds the `voiceVolume` range, and a foot row with
    `[data-action=try-voice]` and, when set, the voice report.
  - Listening: `input[name=listen]` is a toggle in a row labelled "Listen
    for the wake word", help "On-device speech recognition.". The wake word,
    recognizer and microphone rows keep their `name`s. Whisper model rows
    (when the recognizer is built in) keep their `data-*` actions and
    `settings__progress`.
  - The existing tests at `settings.test.ts:292-293` and `558` check the
    label text through `closest("label")`. Update them to the new labels
    ("ElevenLabs", "Voice server", "Listen for the wake word").

- [ ] **Step 2: Run** `pnpm vitest run src/settings.test.ts`. Expected: the
  new tests FAIL.

- [ ] **Step 3: Implement** Voice per the spec. The `checkbox`,
  `textField` and `keyField` helpers inside `renderSettings` (~1593-1630)
  become thin wrappers over `row` and `toggle`, or are removed if no
  caller is left.

- [ ] **Step 4: Run** `pnpm vitest run src/settings.test.ts src/settings-flow.test.ts`.
  Expected: PASS.

- [ ] **Step 5: Commit** `feat(settings): Voice in the new design`.

### Task 8: Network and the pairing card

**Files:**
- Modify: `src/settings.ts` (Network ~1725-1960; `tickPairingCode` 580-593;
  `tickAddresses` 607-622)
- Modify: `src/styles.css` (`.settings__code` at 132)
- Test: `src/settings.test.ts`, `src/settings-flow.test.ts`

**Interfaces:**
- Consumes: `group`, `card`, `row`, `footRow`, `btn`, `notice` (Task 1).
- Produces: the pairing card `div.settings__pair` holding
  `.settings__code` (the code, or "No code yet" with
  `settings__code--empty`), `.settings__code-expiry` and the
  `[data-action=regenerate-code]` button. `tickPairingCode` still finds
  these by class.

- [ ] **Step 1: Write the failing tests:**
  - The main role shows a card with the role select (row labelled "This
    Maya"), Port (whose help is the `.settings__addresses` line) and This
    machine's name.
  - With a live code, `.settings__pair .settings__code` reads the
    formatted code, the expiry reads as today, and the regenerate button
    reads "Regenerate". With no code: `.settings__code.settings__code--empty`
    reads "No code yet" and the button reads "Show pairing code".
  - The group foot reads "Paired devices are in Settings › Devices."
  - The New key row's help reads "Drops every pairing, so each device must
    pair again.", and its button has `settings__btn--danger`. The
    confirmation opens in that card, as today.
  - The assistant role: the host, port, name and pairing code fields keep
    their `name`s, Pair is `settings__btn--gold`, and the status lines
    (`settings__status--network`) sit in the card's foot.
  - The existing `tickPairingCode` and `tickAddresses` tests still pass
    unchanged. If one breaks, the markup is wrong, not the test.

- [ ] **Step 2: Run** `pnpm vitest run src/settings.test.ts src/settings-flow.test.ts`.
  Expected: the new tests FAIL.

- [ ] **Step 3: Implement** Network per the spec. `tickPairingCode`
  switches `settings__code--empty` and its text when a code expires.
  `tickAddresses` puts a missing addresses line in as the Port row's help.

- [ ] **Step 4: Add the pairing card CSS:** a `--gold-deep` edge, radius
  14, the radial glow from the mockup's `.code`, the code at 40px gold
  monospace, and the empty state at 20px muted.

- [ ] **Step 5: Run** `pnpm vitest run src/settings.test.ts src/settings-flow.test.ts`.
  Expected: PASS.

- [ ] **Step 6: Commit** `feat(settings): Network and the pairing card in the new design`.

### Task 9: Devices (desktop and phone)

**Files:**
- Modify: `src/devices.ts` (`selfRow`, `memberRow`, `assistantRows`,
  `joinForm`, `renderDevices`)
- Modify: `src/settings.ts` (the Devices section, ~1962-1985)
- Modify: `src/styles.css` (`.settings__assistant*` and `.settings__badge`,
  134-139)
- Test: `src/devices.test.ts`

**Interfaces:**
- Consumes: `state` and `btn` (Task 1). The buttons keep the local
  `button()` helper's `data-*` attributes, with `settings__btn` classes
  added; there are no per-button listeners.
- Produces: each device row is `div.settings__assistant.settings__device`
  (keeping `data-row`, `data-key` and `data-id`). It holds
  `span.settings__device-mark` (Mac, Windows, Linux or Phone, from the
  platform), `div.settings__device-text` (`div.settings__device-name`
  with an optional `span.settings__badge`, `div.settings__device-meta`,
  and on a phone row the mirror switch), then a `.settings__state`, then
  `div.settings__assistant-actions`. `renderDevices(..., opts?: { stateUnderName?: boolean })`
  puts the state inside `.settings__device-meta` when `stateUnderName` is
  true (the phone).

- [ ] **Step 1: Write the failing tests** in `src/devices.test.ts`:
  - The self row's name reads its name, with a `.settings__badge` reading
    "This device" ("This phone" when `thisDevice` is "this phone"). Its meta
    reads the platform and address.
  - The main's row has `.settings__state--main` reading "Main". A
    connected assistant has `.settings__state--on` reading "Connected".
    Others have `--off` with "Last seen 1h ago" or "Never connected".
  - Marks: `macos` gives "Mac", `windows` "Windows", `linux` "Linux",
    `android` "Phone".
  - The mirror switch (`input[data-action=mirror]`) has class
    `settings__toggle`, sits in `.settings__device-text`, and is labelled
    "Sees and controls all sessions".
  - With `stateUnderName`, `.settings__state` is inside
    `.settings__device-meta`.
  - The `after a repaint` tests (`devices.test.ts:122-168`) pass
    unchanged: patching a render onto a kept list leaves the same row
    nodes.
  - Update the test at lines 31-38 from badge "Main" to the state check
    above.

- [ ] **Step 2: Run** `pnpm vitest run src/devices.test.ts`. Expected: the
  new tests FAIL.

- [ ] **Step 3: Implement** the rows. Keep the same element order for the
  same model, so `morph` patches.

- [ ] **Step 4: Run** `pnpm vitest run src/devices.test.ts src/settings.test.ts src/mobile/network.test.ts`.
  Expected: PASS.

- [ ] **Step 5: Commit** `feat(settings): Devices rows with marks and states`.

### Task 10: Release Notes on the desktop

**Files:**
- Modify: `src/settings.ts` (`releaseNotes.append(renderReleaseNotes())`,
  ~963)
- Modify: `src/styles.css` (`.release-notes*` at 105-109: the Settings
  styling scoped to `.settings.pane`, leaving the What's new window's alone)
- Test: `src/settings.test.ts`

**Interfaces:**
- Consumes: `renderReleaseNotes` (`src/release-notes.ts`) and the art
  import that `src/whatsnew.ts` uses for `maya-full-body.png`.
- Produces: `div.settings__rn` holding `img.settings__rn-art[alt=""]` and
  the notes.

- [ ] **Step 1: Write the failing tests:** the Release Notes section holds
  `.settings__rn` with an `img.settings__rn-art` whose `alt` is "" and
  whose `src` is the same art as the What's new window, followed by
  `.release-notes`. The running version's heading carries a
  `.settings__running` pill reading "Running".

- [ ] **Step 2: Run** `pnpm vitest run src/settings.test.ts`. Expected: the
  new tests FAIL.

- [ ] **Step 3: Implement** it, and add the CSS from the mockup:
  `.settings__rn { display: grid; grid-template-columns: 290px minmax(0, 1fr); gap: 8px; }`,
  the art sticky at `top: 24px` with `height: min(76vh, 600px)`,
  `margin-left: -64px` and the gold drop shadow. Under a 1100px window,
  one column and no art. The notes are at most 68ch, with headings,
  spacing and hairlines as in the spec.

- [ ] **Step 4: Run** `pnpm vitest run src/settings.test.ts src/whatsnew.test.ts`.
  Expected: PASS.

- [ ] **Step 5: Commit** `feat(settings): Maya presents the release notes`.

### Task 11: The phone's Settings

**Files:**
- Modify: `src/mobile/network.ts` (`PHONE_GROUPS` 22-29; `section` and
  `button` 80-92; the `*Cards` functions 105-240; `renderNetwork` from
  272)
- Modify: `src/mobile/mobile.css` (Settings rules at 53 and 66-87)
- Test: `src/mobile/network.test.ts`, `src/mobile/settings-history.test.ts`

**Interfaces:**
- Consumes: every builder from Task 1, and `renderDevices(..., { stateUnderName: true })`
  from Task 9.
- Produces:
  - `export const PHONE_LIST: readonly (readonly PhoneGroup[])[] = [["network","devices"],["notifications","voice"],["viewer"],["releaseNotes"]]`
  - `export const PHONE_LEDES: Partial<Record<PhoneGroup, string>>`, with
    the spec's ledes (Network, Devices, Notifications, Voice). Release
    Notes' lede is `You're running Maya ${RUNNING_VERSION}.`

- [ ] **Step 1: Write the failing tests** in `src/mobile/network.test.ts`:
  - The list: a `.settings__title` reading "Settings", then 4
    `.settings__card`s holding 2, 2, 1 and 1 `.settings-list__item`s, in
    `PHONE_LIST` order. Names and summaries are unchanged (the test at
    lines 110-112 keeps its expectations). Network's summary has a
    `.settings__state--on` when connected.
  - A page: `.settings-page__back` reads "Settings", then
    `.settings__title` reads the group's name and `.settings__lede` reads
    `PHONE_LEDES[group]`. Update `.settings-page__title` checks (lines 131,
    168, 192, 292) to `.settings__title`.
  - Network, not joined: the Join a main group's note is `JOIN_HINT`. The
    `host`, `joinPort` and `code` inputs keep their names, and
    `joinPort` and `code` share one `.settings__row--split`. Join is
    `settings__btn--gold`.
  - Network, joined: "Connected to \<main>" is a `.settings__state--on`,
    and Save name and Leave are side by side in a `.settings__bar`, with
    Leave as `settings__btn--danger`.
  - Notifications: `notifyAwaiting` and `notifyCompleted` are toggles.
    "Notifications are off for Maya in Android settings." is a
    `.settings__notice.settings__error` after the card. Battery's help
    reads "Android may otherwise stop Maya with the screen off, and her
    notifications with her."
  - Voice: the toggles `speak` and `wakeWord`, with the help lines from the
    spec, and the foot "Voice needs a desktop main."
  - Session viewer: a `.settings__seg` with radios `name=viewerTheme`,
    values `chat` and `terminal`. Choosing `terminal` calls
    `onViewerTheme("terminal")`.
  - Release Notes: `img.settings__rn-banner[alt=""]` comes before a
    `.settings__rn-head` holding the title and lede.
  - The settings-history tests pass unchanged.

- [ ] **Step 2: Run** `pnpm vitest run src/mobile`. Expected: the new tests
  FAIL.

- [ ] **Step 3: Implement** the list and every page from the builders, per
  the spec's "The phone".

- [ ] **Step 4: Replace the phone CSS** (mobile.css 66-87) with the touch
  sizes from `phone.html`: titles 26px, rows at least 56px (list rows
  64px), fields 44px tall at 15px, switches 44×26, full-width bar buttons
  at least 46px. The banner follows `mobile.css:111` (200px,
  `object-fit: cover`, `object-position: 50% 6%`, the same mask), the head
  `margin-top: -72px`, right-aligned, with `mobile.css:113`'s text shadow.
  The back link sits over the banner (`position: relative; z-index: 1`).

- [ ] **Step 5: Run** `pnpm vitest run src/mobile src/devices.test.ts`.
  Expected: PASS.

- [ ] **Step 6: Commit** `feat(mobile): the phone's Settings in the new design`.

### Task 12: Remove the old Settings CSS, and check the popover

**Files:**
- Modify: `src/styles.css` (whatever is left of the old Settings rules:
  lines 72-104, 133-140, 270-271, 306-309, 457-464, 642 as of the start of
  this plan)
- Test: `src/settings.test.ts`

- [ ] **Step 1: Write the failing test:** render the Voice popover
  (a `div#voice-panel.settings.popover` holding `renderVoicePanel(...)`
  from `src/voice.ts:67`) in
  jsdom with `styles.css` loaded. Its buttons and inputs have the same
  computed `padding`, `border-radius` and `font-size` as they had before
  this branch. Record the values by running the test once on `main` first.

- [ ] **Step 2: Run** it on this branch. Expected: PASS if Tasks 1-11
  scoped their rules correctly. If it fails, fix the scoping.

- [ ] **Step 3: Delete** every old Settings rule that no element uses any
  more. Check each class with `grep -rn "<class>" src/` before deleting it.

- [ ] **Step 4: Run** `pnpm test`. Expected: PASS, the whole suite.

- [ ] **Step 5: Commit** `refactor(settings): drop the old Settings layout rules`.

### Task 13: Visual check in WebKit and in Maya

**Files:** none committed, except fixes the check turns up.

- [ ] **Step 1: Render each screen with the real CSS.** Write a throwaway
  vitest file outside `src/`, or one deleted after use, that renders
  `renderSettings` for each group with a realistic model (both forges on,
  two rules, a custom tool, a main with a live code and three devices) and
  writes each section's HTML into a page that links `src/styles.css`. Do
  the same for the phone's list and pages with `mobile.css`.

- [ ] **Step 2: Screenshot it** with Playwright WebKit at 1728×1000,
  1100×900, 800×900 and 500×900 (desktop), and 390×844 (phone), once in dark
  and once with `data-theme="light"`. Compare with `desktop.html` and
  `phone.html`. Check:
  - Review Focus 2 (light theme), 3 (long values) and 4 (narrow windows);
  - the Work Items rule fields show "good first issue" in full.
  Fix what differs and commit each fix with its test.

- [ ] **Step 3: Check in the app.** Run `pnpm tauri dev`. Open Settings,
  hover a Devices row, and let a network status push repaint: the hover
  stays. Open and fold a Work Items source with the mouse and with the
  keyboard. Open the Voice popover and compare it with `main`. Mention
  anything you could not check (for example, the phone build).

### Task 14: Version and release notes

**Files:**
- Modify: `RELEASE_NOTES.md`, and the version files via
  `scripts/set-version.sh`

- [ ] **Step 1:** `git fetch origin && git show origin/main:package.json | grep '"version"'`.
  Bump the minor version from that (0.54.0 if `main` is still 0.53.0) with
  `sh scripts/set-version.sh <version>`.

- [ ] **Step 2:** Add the top section of `RELEASE_NOTES.md`:

```markdown
# <version>

## New

- Settings has a new look, on the computer and on the phone. Each screen is centred, with its settings grouped in cards, the help for each setting next to it, a switch for every on and off, and room to breathe.
- In Settings → Work Items, each source (GitHub, GitLab and each custom tool) folds up, and shows whether it is on while folded. Maya remembers which ones you left open.
- Settings → Release Notes shows Maya beside the notes.

## Fixed

- A Work Items rule's Labels and Repositories fields are wide enough to read again.
```

- [ ] **Step 3:** Run `pnpm test` and `sh scripts/test-release-notes.sh`.
  Expected: PASS.

- [ ] **Step 4: Commit** `chore(release): <version>` with both changes.
