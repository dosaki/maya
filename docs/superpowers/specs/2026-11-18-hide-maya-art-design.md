# Hide Maya's picture beside the release notes

A small change. Approved on 2026-11-18.

## Why

Some people do not like the full-body picture of Maya that stands beside
the release notes. They should be able to turn it off.

## Goals

- One switch, in Settings → General → Appearance, below Theme and Cards:
  **Show Maya beside release notes**. On by default, so nothing changes
  for anyone who leaves it alone.
- Off hides the picture in both desktop places it stands beside the notes:
  Settings → Release Notes (`.settings__rn-art`) and the What's new window
  after an update (`.whatsnew__art`).
- The change shows at once, as Theme and Cards do, before the save lands.
- With the picture gone, the notes take its space: no empty column in
  Settings → Release Notes, and no left margin kept for her beside the
  What's new panel.
- The choice survives restarts and updates.

## Non-goals

- The phone. Its What's new already shows her as a banner of its own
  (`mobile/mobile.css`), and its network screen (`src/mobile/network.ts`)
  is a separate question. Neither changes.
- Any other picture of Maya (the logo, the avatar).

## Anti-goals

- The picture showing for a moment at start for someone who turned it off.
- A gap or a shifted layout where the picture was.

## Approaches

- **A Settings switch** (picked). Easy to find, easy to turn back on, and
  it matches the other Appearance choices. Its cost is that people have
  to know to look for it.
- **A Hide button on the picture itself.** Right where the annoyance is,
  but once she is hidden the button is gone with her, so bringing her back
  needs a setting anyway.

## How (decided)

- The choice is saved in Maya's config, as the card size is
  (`core/src/config.rs`, `saveConfig` in `src/settings.ts`).
- One marker on the page hides the picture wherever it stands, as
  `data-cards` sets the card size (`src/theme.ts`, `applyCardSize`): CSS
  keyed on it hides both pictures and drops the room kept for them, so
  the What's new window follows the setting without wiring of its own.
- The marker is remembered in `localStorage` and put on the page by the
  early script in `index.html`, as the theme and card size are, so a
  start paints it before the saved settings arrive.

## Left to the builder

- The config key's name and its Rust and TypeScript shapes.
- The switch's help line, if it has one.
