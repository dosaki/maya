# Card Design Pass Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Calmer session cards (design A3), a Compact / Roomy / Spacious card size setting (default Spacious), and Pull Requests and Deployments as one centred column of rows.

**Architecture:** The size is a config field (`cardSize`) mirrored onto `<html data-cards="…">` like `data-theme`. CSS custom properties per level feed the card and row spacing. `renderCard` gets a new structure: head, a sub line, an ask block, the snippet, and a footer of facts and icon buttons. The PR and deployment renderers keep their `.pr` classes but lay out as rows inside one panel.

**Tech Stack:** Rust (`maya-core` config), TypeScript UI with vitest/jsdom, plain CSS.

**Spec:** `docs/superpowers/specs/2026-11-13-card-design-pass-design.md`

**Mockups (reference, git-ignored):**
- `.superpowers/brainstorm/19451-1794571529/content/cards-a-pass3.html` (the card)
- `cards-room-compare.html` (the three sizes)
- `cards-and-lists-v2.html` (list design 1)

## Global Constraints

- A click anywhere on a card still opens its chat (`cardActionFor` falls through to `open`). There is no Reply button.
- Every other rule for what a card shows stays as today:
  - Close (`canClose`), Terminal (`!card.machine && (!isLinux() || card.terminal)`), Stop (`canStop`, disabled for `OPEN_DELAY_MS` and while stopping);
  - Compact (`capabilitiesOf(h).compact && percent >= COMPACT_AT`), PR (`card.pr`);
  - answers enabled after `OPEN_DELAY_MS`, the multi-select note, the several-questions warning, and a stale card dimmed.
- Card size values: `compact`, `roomy`, `spacious`. Labels: "Compact", "Roomy", "Spacious". Default `spacious`. The Settings label is "Cards", in General → Appearance, under Theme.
- Colours come from existing tokens only (`--danger` for the 75%+ ring, `--muted`, `--gold`…), so the light theme keeps working.
- Repaint with `morph`; never `replaceChildren` a node the pointer may be over (CLAUDE.md lore).
- Settings UI is vertical: each row on its own line.
- The phone keeps its own card padding (`src/mobile/mobile.css` `.card { padding: 12px 14px }`) and its 40px tap targets. It gets the new structure and Spacious gaps, and has no first-paint script, as with the theme.
- Release: `sh scripts/set-version.sh 0.57.0` (check main is still 0.56.0), committed with RELEASE_NOTES.md as `chore(release): 0.57.0`.

## Review Focus

1. A card that knows only its project (no branch, model, cost, PR or context): the sub line has no stray `·` and the footer still lines its buttons up on the right. Pinned in Task 2.
2. A long branch (`feat/a-very-long-branch-name-that-overflows`) is shortened with an ellipsis, and its `title` holds the whole branch. Pinned in Task 2.
3. Stop as an icon is still disabled inside `OPEN_DELAY_MS` and is still labelled for screen readers ("Stop"). Pinned in Task 2.
4. A saved size that is not one of the three (an older or hand-edited config) reads as Spacious, and so does a page with no stored size. Pinned in Task 1.
5. Keyboard: a card is still focusable (`tabIndex 0`) and still opens with Enter, as today. Check the existing key handling still finds the card once Reply is gone. Pinned in Task 2.

---

### Task 1: The card size setting

**Files:**
- Modify: `core/src/config.rs`:
  - add `CardSize` beside `ColorTheme` (same derives, `#[serde(rename_all = "lowercase")]`, `#[default] Spacious`, plus `Compact` and `Roomy`);
  - add the field `pub card_size: CardSize` with `#[serde(default)]` after `color_theme`;
  - add `card_size: CardSize::Spacious` to `Config::default`'s literal;
  - add the test.
- Modify: `src/types.ts` (`export type CardSize = "compact" | "roomy" | "spacious"`)
- Modify: `src/theme.ts`:
  - `CARD_SIZES` (`[["compact","Compact"],["roomy","Roomy"],["spacious","Spacious"]] as const`);
  - `CARD_SIZE_KEY = "maya.cardSize"`;
  - `asCardSize(value: unknown): CardSize`, which gives Spacious for anything else;
  - `applyCardSize(size: CardSize): void`, which sets `document.documentElement.dataset.cards` and stores the key, wrapped in try/catch like `applyColorTheme`.
- Modify: `index.html` (the first-paint `<script>` also sets `dataset.cards` from `maya.cardSize` when it is `compact` or `roomy`).
- Modify: `src/settings.ts`:
  - `cardSize?: CardSize` on the model and on `ConfigJson`;
  - an `onCardSize?(s: CardSize): void` handler;
  - the `select[name=cardSize]` row "Cards" in the Appearance card under Theme;
  - in `applyConfig`: `model.cardSize = asCardSize(c.cardSize); applyCardSize(model.cardSize)`;
  - the handler, which sets the model and applies at once, then `saveConfig({ cardSize })`, exactly like `onColorTheme`.
- Modify: `src/styles.css`, which adds the size variables.
- Test: `src/theme.test.ts`, `src/settings.test.ts`, `src/theme-css.test.ts` (or the CSS test that reads `styles.css`).

**Interfaces:**
- Produces: `CardSize`, `CARD_SIZES`, `asCardSize`, `applyCardSize`, and the `cardSize` config key.
- Produces, the CSS custom properties Task 2 and Task 3 use. Each is defined on `:root` (Spacious) and overridden under `:root[data-cards="roomy"]` and `:root[data-cards="compact"]`:

| Property | compact | roomy | spacious |
|---|---|---|---|
| `--card-pad-y` / `--card-pad-x` | 10px / 12px | 16px / 18px | 20px / 22px |
| `--card-gap` (between cards) | 8px | 14px | 18px |
| `--card-name` (name font size) | 14px | 15px | 15.5px |
| `--card-text` / `--card-leading` (question, snippet) | 13px / 1.4 | 14px / 1.5 | 14.5px / 1.55 |
| `--card-section` (space between sub line, ask, snippet, footer) | 8px | 12px | 16px |
| `--card-opt-pad-y` / `--card-opt-pad-x` | 5px / 8px | 8px / 12px | 10px / 14px |
| `--card-icon` (icon button box) | 26px | 28px | 30px |
| `--row-pad-y` / `--row-pad-x` (PR and deployment rows) | 8px / 12px | 11px / 14px | 14px / 16px |

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn the_card_size_defaults_to_spacious_and_round_trips() {
    assert_eq!(Config::default().card_size, CardSize::Spacious);
    let older: Config = serde_json::from_str(r#"{"completedTimeoutMinutes":30}"#).unwrap();
    assert_eq!(older.card_size, CardSize::Spacious);
    let set = Config { card_size: CardSize::Compact, ..Config::default() };
    let text = serde_json::to_string(&set).unwrap();
    assert!(text.contains(r#""cardSize":"compact""#), "{text}");
    assert_eq!(serde_json::from_str::<Config>(&text).unwrap().card_size, CardSize::Compact);
}
```

```ts
// theme.test.ts
it("reads an unknown or missing card size as Spacious", () => {
  expect(asCardSize("roomy")).toBe("roomy");
  expect(asCardSize("huge")).toBe("spacious");
  expect(asCardSize(undefined)).toBe("spacious");
});
it("puts the card size on the page and remembers it", () => {
  applyCardSize("compact");
  expect(document.documentElement.dataset.cards).toBe("compact");
  expect(localStorage.getItem(CARD_SIZE_KEY)).toBe("compact");
});

// settings.test.ts, next to the theme tests
it("offers Compact, Roomy and Spacious cards under Theme, and saves a change", () => {
  // select[name=cardSize] options text ["Compact","Roomy","Spacious"], value "spacious" with no cardSize in the model;
  // change to "roomy" → h.onCardSize called with "roomy"; the row's label is "Cards" and it follows the Theme row
});

// the CSS test
it("defines every card size property for each size", () => {
  // for each property in the table: present in the :root block (spacious values),
  // and in :root[data-cards="roomy"] and :root[data-cards="compact"] with the values above
});
```

- [ ] **Step 2: Run them and see them fail**

Run: `cd core && cargo test -q card_size` and `pnpm -s vitest run src/theme.test.ts src/settings.test.ts src/theme-css.test.ts`
Expected: compile errors or failures for the missing names.

- [ ] **Step 3: Implement** as the Files list says. A settings-flow test, if one covers the theme handler, gets the same for `onCardSize`.

- [ ] **Step 4: Run the tests and see them pass**

Run: `cd core && cargo test -q`, `cd src-tauri && cargo test -q`, `pnpm -s vitest run`, `npx tsc --noEmit`
Expected: all pass.

- [ ] **Step 5: Commit** `feat(settings): pick Compact, Roomy or Spacious cards`

### Task 2: The session card, design A3

**Files:**
- Create: `src/assets/icons/stop.svg` (a 16×16 rounded filled square, `fill="currentColor"`); register `stop` in `src/icons.ts` `FILES`.
- Modify: `src/answer.ts`: `stopButton(card, nowMs, className = "card__btn", look: "text" | "icon" = "text")`. With `"icon"` it is an `iconButton("stop", "Stop", …)` with the same `data-action`, title, disabled rule and `card__btn--stop`.
- Modify: `src/options.ts`: `renderOptions` opts gain `numbered?: boolean`. When it is true, each `.options__btn` starts with `<span class="options__num">N</span>` (1-based), and the list stacks as rows (`.options--rows`).
- Modify: `src/card.ts` (`renderCard`).
- Modify: `src/styles.css` (card block, lines ~46–61, 357–361, 391–394, 416–419, 433–444, 472–474, 638–639).
- Test: `src/card.test.ts`, `src/board.test.ts`, `src/actions.test.ts`, `src/options.test.ts`, `src/icons.test.ts`.

**Interfaces:**
- Consumes: Task 1's CSS properties.
- Produces: the card's DOM, which the board's `morph` and `cardActionFor` read:

```
article.card.card--<state>[.card--stale]  (tabIndex 0, title cwd, data-session-id, data-pid; unchanged)
  header.card__head: span.card__name, span.card__age, [button.card__close data-action=close]
  div.card__sub:
    harness badge (.card__harness), span.card__project ("proj"),
    [span.card__remote (icon) + span.card__machine ("laptop")],
    [span.card__branch (text and title = branch)], [span.card__model]
    — separators are CSS (::before "·" on every part after the first text), never text nodes
  [div.card__ask (only when card.awaiting)]:
    [div.options__label "Question N of M · header"],
    div.card__question[data-kind] (awaiting.detail),
    the renderOptions(…, { numbered: true }) list, note and warning
  [p.card__snippet] (clamped to 2 lines)
  footer.card__foot:
    div.card__facts:
      [span.card__ctx[data-band] (ring + "62%"; title as contextMeter's title today)],
      [button.card__pr data-action=pr ("#97" or "!12" via prRef; title "<state> · <url>")],
      [span.card__cost (costText(card.usage.cost))]
    div.card__actions:
      [compact icon button], [terminal icon button], [stop icon button]
```

  - The question label moves out of `renderOptions` for the card. Keep `renderOptions` printing it for the modal: pass a flag, or let the card skip it. The executor rules on this.
  - `contextMeter` stays exported for any other caller. The card no longer uses it.

- [ ] **Step 1: Write the failing tests** (replace the tests that pin the old markup: the Reply-button tests in `board.test.ts` ~133–152 and 253, the `.card__meter` tests ~38–56, `card.test.ts` 41–43, and `actions.test.ts` 13, which should now click the card body)

```ts
it("has no Reply button: a click on the card opens it", () => {
  const el = renderCard(card({}), NOW);
  expect(el.querySelector("[data-action=reply]")).toBeNull();
  expect(cardActionFor(el.querySelector(".card__snippet") ?? el)).toEqual({ kind: "open", sessionId: el.dataset.sessionId });
});
it("shows the context as a ring with its percentage, red from 75%", () => {
  const ctx = (percent: number) => renderCard(card({ context: { used: percent * 2000, window: 200_000, percent } }), NOW).querySelector<HTMLElement>(".card__foot .card__ctx")!;
  expect(ctx(62).textContent).toBe("62%");
  expect(ctx(62).title).toBe("Context 62% · 124k of 200k"); // whatever formatTokens gives today; copy from the old meter test
  expect(ctx(62).dataset.band).toBe("mid");
  expect(ctx(90).dataset.band).toBe("high");
  expect(renderCard(card({}), NOW).querySelector(".card__ctx")).toBeNull();
  expect(renderCard(card({}), NOW).querySelector(".card__meter")).toBeNull();
});
it("names the project, machine, branch and model under the name, each only when known", () => {
  const el = renderCard(card({ machine: "laptop", branch: "feat/a-very-long-branch-name-that-overflows", model: "Opus 5.5" }), NOW);
  const sub = el.querySelector(".card__sub")!;
  expect(sub.querySelector(".card__project")!.textContent).toBe("proj");
  expect(sub.querySelector(".card__machine")!.textContent).toBe("laptop");
  expect(sub.querySelector<HTMLElement>(".card__branch")!.title).toBe("feat/a-very-long-branch-name-that-overflows");
  expect(sub.querySelector(".card__model")!.textContent).toBe("Opus 5.5");
  const bare = renderCard(card({}), NOW).querySelector(".card__sub")!;
  expect(bare.querySelector(".card__branch, .card__model, .card__machine")).toBeNull();
  expect(bare.textContent).not.toContain("·");
});
it("puts the PR and the cost in the footer", () => {
  const el = renderCard(card({ pr: { number: 97, url: "u", state: "open", forge: "github" }, usage: /* a usage with cost usd 1_240_000 micros */ }), NOW);
  expect(el.querySelector(".card__foot [data-action=pr]")!.textContent).toBe("#97");
  expect(el.querySelector(".card__foot .card__cost")!.textContent).toBe("$1.24");
});
it("offers Compact as Maya's compact icon from 75%", () => {
  // claude-code card at 80%: .card__actions button[data-action=compact] has svg.icon-compact; at 60% none
});
it("shows Stop as an icon, labelled, and disabled during the open delay", () => {
  // working card, stateSince NOW: button[data-action=stop] has svg.icon-stop, aria-label "Stop", disabled;
  // at NOW + OPEN_DELAY_MS it is enabled
});
it("shows a question as plain text over numbered answers, then the last message", () => {
  // awaiting card with two questions and a snippet:
  // .card__ask .options__label "Question 1 of 2 · <header>"; .card__question text = detail, data-kind kept;
  // .options__num texts ["1","2","3"]; .card__snippet present after .card__ask
});
it("keeps a card focusable", () => { expect(renderCard(card({}), NOW).tabIndex).toBe(0); });
```

Keep every existing test of the rules in Global Constraints (close, terminal on Linux/remote, stop, stale) passing. Only update their selectors, for example `.card__actions [data-action=stop]` still works.

- [ ] **Step 2: Run them and see them fail**

Run: `pnpm -s vitest run src/card.test.ts src/board.test.ts src/actions.test.ts src/options.test.ts src/icons.test.ts`
Expected: the new tests fail.

- [ ] **Step 3: Implement** the DOM above. Then the CSS:
  - card padding `var(--card-pad-y) var(--card-pad-x)`, with the footer's bottom padding taking the place of the card's;
  - `.column__cards` gap `var(--card-gap)`;
  - the ring, as a conic-gradient span using `--gold` (`--danger` for band high) with an inner disc in `--panel`;
  - icon buttons that are borderless and muted, gaining the text colour on card hover; Stop in the existing red;
  - `.card__question` with no tinted box: the kinds keep only the code font for `permission`;
  - numbered rows (`.options--rows`) as stacked bordered rows with `--card-opt-pad-*` and `.options__num` in `--awaiting`;
  - the snippet clamped to 2 lines;
  - the branch in code font with an ellipsis.

  Remove the CSS for the meter and the tinted ask box when nothing else uses it.

- [ ] **Step 4: Run all tests, then look at it**

Run: `pnpm -s vitest run` and `npx tsc --noEmit`
Expected: all pass.

Then render the board in Playwright WebKit with the real `styles.css`, the way the brainstorm did (`scratchpad/look/entry.ts`), at all three sizes. Compare with `cards-a-pass3.html` and `cards-room-compare.html`.

- [ ] **Step 5: Commit** `feat(board): calmer session cards with a footer of facts and icons`

### Task 3: Pull Requests and Deployments as rows

**Files:**
- Modify: `src/reviews.ts` (`renderReviewCard`), `src/deployments.ts` (`renderRun`, `renderDeployments`), `src/styles.css` (lines ~540–567)
- Test: `src/reviews.test.ts`, `src/deployments.test.ts`

**Interfaces:**
- Consumes: Task 1's `--row-pad-*`.
- Produces: each `article.pr` (and `article.pr.deploy`) becomes `div.pr__main` (head line + meta) followed by `div.card__actions`.
  - The head line is: dot, forge, `span.pr__repo`, then `span.pr__title`, all on one line.
  - Chips go before the buttons in `.card__actions`.
  - `.reviews__list` becomes one panel (`--panel` background, border, radius 8px) with no gaps and hairlines (`--rule`) between rows. It is centred with `max-width: 860px; margin: 0 auto`.
  - Every `data-*` attribute and `data-action` the click handlers read stays the same.
  - A blocked run keeps `.deploy--blocked`, which is now a left indent inside the panel.

- [ ] **Step 1: Write the failing tests**

```ts
it("lays a PR out as one row: text on the left, chips and buttons on the right", () => {
  const el = renderReviewCard(/* a draft PR, related card on this machine */);
  expect(el.querySelector(":scope > .pr__main .pr__title")!.textContent).toBe(/* title */);
  const right = el.querySelector(":scope > .card__actions")!;
  expect([...right.children].map((c) => (c as HTMLElement).dataset.action ?? c.className)).toEqual(["pr__chip pr__chip--draft", "terminal", "open-pr", "review"]);
});
it("lays a waiting deployment out as one row with its environments before Reject and Approve", () => {
  // .card__actions children: env chips, then reject, approve
});
it("keeps a blocked run's link to the run it waits on", () => {
  // .deploy--blocked has button[data-action=open-blocker]; deploymentActionFor on it still returns open-blocker
});
```

Existing tests of the actions (`reviewActionFor`-style and `deploymentActionFor`) must still pass unchanged.

- [ ] **Step 2: Run them and see them fail**

Run: `pnpm -s vitest run src/reviews.test.ts src/deployments.test.ts`
Expected: the new tests fail.

- [ ] **Step 3: Implement** the layout and the CSS:
  - each row is `display:flex; align-items:center; gap:12px; padding: var(--row-pad-y) var(--row-pad-x)`;
  - `.pr__main` has `flex:1; min-width:0`, and the title is shortened with an ellipsis;
  - under `@media (max-width: 600px)`, the row wraps and `.card__actions` takes the full width under the text.

- [ ] **Step 4: Run all tests, then look at it**

Run: `pnpm -s vitest run` and `npx tsc --noEmit`
Expected: all pass.

Then render both tabs in Playwright WebKit at 1400px and 390px wide, and compare with list design 1 in `cards-and-lists-v2.html`.

- [ ] **Step 5: Commit** `feat(reviews): Pull Requests and Deployments as one column of rows`

### Task 4: Docs and release

- [ ] **Step 1:** Update the README wherever it describes a card's Reply button or the context bar, or the PR grid; `grep -n "Reply\|context bar\|meter" README.md docs/*.md`. Commit as `docs: the new session card and rows`, if anything changed.
- [ ] **Step 2:** Check that main's version is still 0.56.0 (`git show origin/main:package.json`). Then run `sh scripts/set-version.sh 0.57.0`, and add at the top of RELEASE_NOTES.md:

```
# 0.57.0

## New

- Session cards are calmer: the context, PR and cost sit in a thin footer beside plain icon buttons, the card shows its branch and model, and answers to a question are numbered rows. Click a card to open its chat; the Reply button is gone.
- Settings → General → Cards picks Compact, Roomy or Spacious cards. Spacious is the default.
- Pull Requests and Deployments show as one column of rows instead of a grid of cards.
```

- [ ] **Step 3:** Commit both as `chore(release): 0.57.0`.
