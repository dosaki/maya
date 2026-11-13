# Card design pass

## Why

Session cards look busy. Each card has a tinted question box, a row of up
to five bordered buttons in three colours, a context bar on the left edge
and an agent badge. Pull Requests and Deployments show as a grid of cards
that wraps sideways in a wide window. That goes against "Maya UI:
vertical, not side by side". The user wants both to look good. They chose
the designs below from mockups.

## Goals

- A calmer session card that still shows everything today's card shows,
  and also what the session window's status line knows: the branch, the
  model and the cost.
- A setting for how roomy the cards are: Compact, Roomy or Spacious, with
  Spacious as the default.
- Pull Requests and Deployments as one centred column of rows.

## Non-goals

- Work Items: not designed in this pass, so it stays as it is.
- New session actions or information beyond what the card or the session
  window's status line already has.
- The session window (modal), the Chat tab and Settings, apart from the
  new setting.

## Approaches

The mockups decided the look. For the cards, the user saw three options:

- **A, a quiet footer.** Picked.
- **B, buttons only on hover.** Not picked.
- **C, compact two-line cards.** Not picked.

For the lists, the user saw two options:

- **1, inbox rows.** Picked.
- **2, rows grouped by repository.** Not picked.

For the room, the user first saw two levels, which looked alike. Then
they saw three levels side by side, and chose to offer all three as a
setting.

## Design

### Session card

A click anywhere on the card still opens its chat.

**Head**
- The name, the age, and the × (Close) when the session can close, as
  today.

**Line under the name**
- The agent's mark (the current harness icon, small), then the project.
- On a remote session, the remote mark and the machine name.
- The branch, in code font. A long branch is shortened with "…", and
  hovering shows all of it.
- The model.
- Each part shows only when Maya knows it. `card.branch` and `card.model`
  can be absent.

**Ask (only while the card waits on the user)**
- "Question N of M · header" when there are several questions.
- The question as plain text, with no tinted box. A permission ask shows
  its command in code font.
- The answers as stacked, numbered rows (1, 2, 3), still disabled for
  the open delay as today.
- The multi-select note and the "answer every question here or in the
  terminal" warning stay.

**Last message**
- The snippet, clamped to two lines (today it is three), under the
  answers when there are any.

**Footer**
- A hairline above it.
- On the left:
  - the context as a small ring with its percentage, red from 75%
    (`COMPACT_AT`);
  - the PR or MR as a small pill (`#97` / `!12`), which still opens it;
  - the cost so far, when known (`card.usage.cost`, formatted as the
    session window does).
- On the right, as plain icon buttons with no border:
  - Compact, with Maya's compact icon, under the same rule as today
    (from 75%, and only for agents that can compact);
  - Terminal, under the same rule as today;
  - Stop, in red, under the same rule as today.
- The Reply button goes: a click on the card already opens the chat.
- The context meter on the left edge goes: the ring replaces it.
- A stale card stays dimmed.

### Card size setting

- Settings → General, beside Theme: **Cards**, with the options Compact,
  Roomy and Spacious. Spacious is the default.
- It is saved in the config like `colorTheme`, and remembered in
  `localStorage` for the first paint, as the theme is.
- It sets the padding, the gaps, the answer-row height and the text size
  of the session cards, and the padding of the Pull Requests and
  Deployments rows. The three levels match the mockup "Room side by side"
  (`.superpowers/brainstorm/…/cards-room-compare.html`):
  - **Compact:** card padding about 10×12px, 8px between cards, 14px
    names.
  - **Roomy:** card padding about 16×18px, 14px between cards, 15px
    names, and looser lines.
  - **Spacious:** card padding about 20×22px, 18px between cards, 15.5px
    names, taller answer rows (10px padding), and larger icon buttons.

### Pull Requests and Deployments

- One centred column (max width about 860px) in any window. The rows sit
  inside a single panel with hairlines between them, and are no longer
  separate cards in a grid.

**A PR row**
- The review mark dot, the forge icon, the repo's short name and number
  (bold), and the title on one line, shortened when it is too long.
- Under it, the author and "updated … ago".
- On the right: the draft chip, the related session's Terminal button
  (or "on <machine>"), Open, and Review (gold).

**A deployment row**
- The repo, the workflow link, the branch and the title.
- Under it, the actor and "waiting …".
- On the right: the environment chips, Reject, and Approve (gold).
- A blocked run sits indented under the run it waits on, with its "Waits
  on <sha> <title>" link.
- Errors and empty states stay as they are.

### Phone

- The phone shares `src/` and the stylesheet, so it gets the new cards
  and rows. It keeps its own card padding and tap sizes, as it keeps its
  own colour theme.
- At phone width, a row's buttons wrap below its text.

## Left to the builder

- The exact sizes for each level, within the values above.
- Whether rows keep the `.pr` class names or get their own.
- How the setting reaches the page, for example a `data-cards` attribute
  on `<html>`, as the theme uses `data-theme`.
