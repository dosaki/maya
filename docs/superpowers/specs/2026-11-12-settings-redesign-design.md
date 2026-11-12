# Settings redesign

## Intent

Settings is cramped. Fields, help lines and buttons sit right on top of each
other in one flat card per screen, capped at 680px and pinned to the left
of a window the user keeps maximised. The goal is screens that look
gorgeous and are comfortable to use: room around everything, related settings
grouped and edged, and every help line next to the setting it explains.

The user rejected the first proposal, which spread blocks side by side
across the wide window. They prefer a vertical layout in nearly every place.
The extra width goes into centring and breathing room, not into more columns.

The approved mockups are committed beside this file, and they are the
reference for the look:

- `2026-11-12-settings-redesign/desktop.html`: all twelve desktop screens
  (use the sidebar).
- `2026-11-12-settings-redesign/phone.html`: the Android app's Settings list
  and pages.

Open them from the repository so the art loads
(`../../../../src/assets/maya-full-body.png`). The mockups are hand-written
HTML, not Maya's code. Their copy and structure are the spec, and their CSS is
a starting point.

## Goals

- Every desktop Settings screen (General, Keyboard, Sessions, Git, Pull
  requests, Work Items, Notifications, Voice, Network, Devices, Debug,
  Release Notes) and every phone Settings page (the list, Network, Devices,
  Notifications, Voice, Session viewer, Release Notes) is rebuilt in the
  new design.
- One small set of builders draws both, so desktop and phone stay in step
  and the next setting someone adds gets the design without trying.
- In Work Items, each source (GitHub, GitLab, each custom tool) folds up.
  Folded up, it still shows whether it is on.
- Fix in passing: a Work Items rule's Labels and Repositories fields are
  70px wide today (`.settings input { width: 70px }` reaches them), so
  values are cut off.

## Non-goals

- No setting is added, removed or changes meaning. Every field keeps its
  `name`, every handler is called as it is today, and every saved value is
  saved as it is today.
- No change to the rest of the app: the board, Chat, the modals and the
  phone's other tabs keep their look. Only the Settings pane (desktop
  `#settings`, phone `#settings`) changes. The Voice popover (`#voice-panel`)
  has the class `settings` too; it must keep its look (see Constraints).
- No new font and no new palette. Maya's black and gold, her system font,
  and the light theme stay.

## Anti-goals

- Blocks side by side to fill a wide window. The user tried that and said no.
- A repaint that loses what the user is doing: half-typed text, focus, an
  open source in Work Items, an open details, `:hover` (see Constraints).
- A test made to pass by dropping what it checks. The existing tests find
  fields by `name` and by `data-*` attributes. Where a test checks old markup
  (a class name, a text node's exact place), change it to check the same
  behaviour in the new markup.

## Approaches

The user picked **A**.

- **A. Shared builders (picked).** `src/settings-ui.ts` holds a few small
  builders: a screen head, a group, a card, a row, a stacked row, a switch,
  a two-way choice, and a collapsible source. Each screen in
  `src/settings.ts` and each phone page in `src/mobile/network.ts` is
  rebuilt from them, one at a time. The old `.settings__*` layout rules in
  `styles.css` are replaced by one new block, and `mobile.css` only resizes
  for touch. This changes the most code (the ~1,400 lines that build
  Settings), but desktop and phone cannot drift, and new settings get the
  design by default.
- **B. CSS over the current markup**, with wrappers added where needed.
  This is the smallest change, but the design is about grouping that the
  flat markup lacks, so it would fill up with one-off fixes, and the phone
  would need its own copy.
- **C. HTML template strings per screen.** Rejected: Settings is built from
  DOM elements with their handlers attached and repaints with `morph`, so
  templates would fight how it already works.

## Constraints

- **Repaints stay as they are.** `patchPanel` (`src/settings.ts:2004`)
  replaces every desktop section on each paint except Devices, which it
  patches with `morph` (`src/morph.ts`) because its one delegated listener
  reads the DOM. The phone's `render` in `src/mobile/network.ts` does the
  same: only its Devices page goes through `morph`. The other sections'
  listeners close over the values they were rendered with, so they must be
  replaced, and the repaint tests expect that
  (`src/settings-flow.test.ts:1105`). Keep this model. Moving every screen
  to patch-in-place is a separate project. *(Builder's call, settled
  after the builder check.)* So:
  - Devices' markup must stay stable from one render to the next (the same
    elements in the same order for the same model), so `morph` patches it.
    Per the lore, WKWebView drops `:hover`, a tooltip and a selection from a
    replaced node.
  - State the user changes without a save (which Work Items source is open,
    whether a custom tool's details is open) lives in the model, so the
    replaced section is rebuilt with it. Opening or folding a source changes
    the DOM in place and records the change in the model. It does not wait
    for a repaint, so the head keeps its hover and focus.
- **Every group is built on every paint, and the sidebar only toggles
  `hidden`** (see `renderSettings`). Keep that: the pairing-code and address
  ticks find their elements in place (`tickAddresses`,
  `tickPairingCode`).
- **The light theme** (`:root[data-theme="light"]`, `src/styles.css:596`)
  must give the new tokens light values. The mockups show only dark.
- **The `.settings` class is shared** with the Voice popover
  (`#voice-panel.settings.popover` in `index.html`, filled by
  `src/voice.ts`), and `.settings button` and `.settings input` rules reach
  into it. Scope the new rules to the Settings pane, and check that the
  popover looks as it did.
- **Phone.** `mobile.css` loads after `styles.css` and already resizes
  Settings for touch (`src/mobile/mobile.css:53-87`). Tap targets stay at
  least 40px (the app's rule), and rows at least 56px.
- **Tests:** `src/settings.test.ts`, `src/devices.test.ts`,
  `src/mobile/network.test.ts`, `src/mobile/settings-history.test.ts`,
  `src/firstrun.test.ts` and any others that render Settings. vitest runs
  with `styles.css` and `mobile.css` loaded (`vitest.config.ts`).
- **Check in the app.** Per the lore, Playwright's WebKit with the real
  `src/styles.css` reproduces WebKit layout, but not the hover loss. Check
  hover in Maya itself.

## The design

### Tokens

These are added to `:root` beside the existing ones, with light-theme values
in the light block:

| Token | Dark | Use |
|---|---|---|
| `--rule` | `#221e16` | the hairline between rows inside a card |
| `--field` | `#0f0e0b` | a sunken field, a rule box, a code block |
| `--soft` | `#1c1913` | an open card's header, a quiet button, hover |
| `--danger` | `#e5786b` | Remove, New key, Leave |

Light values are the builder's choice, by the same logic: rule a shade off
`--panel`, field a shade off `--bg`, soft between them, and danger the
light theme's existing red (`#c0262c`).

Spacing steps: 4, 8, 12, 16, 20, 24, 32, 36, 56. Radii: 12 for a card, 10
for a box inside one (a rule, a code block, the dashed Add), 8 for fields
and buttons, and a full pill for switches and badges.

### The desktop shell

- The sidebar (208px) and one content column (760px at most) are centred
  together in the pane, 56px apart, with 40px of space above. A wide window
  gets equal margins on both sides. The pane's `max-width: 1400px` and the
  body's `900px` go.
- The sidebar is in five groups split by a hairline: General and Keyboard;
  Sessions, Git, Pull requests and Work Items; Notifications and Voice;
  Network and Devices; Debug and Release Notes. The current item keeps
  today's look (panel background, `--gold-deep` border, `--gold-bright`
  text). The sidebar stays in view as the content scrolls.
- Under 900px the sidebar becomes a wrapping row above the content (as
  today under 720px). Under 560px rows stack: the label above the control.

### The parts

| Part | What it is |
|---|---|
| Screen head | The screen's name (24px, 600) and one muted line saying what the screen is for, 32px above the first group |
| Group | An optional title (15px, 600, sentence case), an optional note under it (13px, muted), then its cards, and an optional foot note under them. 36px between groups |
| Card | `--panel`, a 1px `--border` edge, radius 12. Its rows are split by a `--rule` hairline. 16px between cards |
| Row | Label (500) with its help line under it (13px, muted, at most 52ch) on the left, and the control on the right (fields 300px wide). 16px × 20px padding, at least 60px tall |
| Stacked row | The label and help, then the control at the full width under them: text boxes, long commands, rule lists |
| Card head | A row on `--soft` at the card's top with a 15px label: an on/off for the whole card (GitHub, a voice provider). A card that is off dims its other rows |
| Foot row | A short row at a card's bottom: a status on the left, an action on the right ("Signed in as dosaki", Test) |
| Switch | Every on/off. It is still an `input type=checkbox`, keeping its `name`, drawn as a 38×22 pill that is gold when on |
| Two-way choice | Two or three radios drawn as one control. Each is still an `input type=radio` with its `name` and `value` |
| State | A dot and a word: green "Installed" or "Connected", gold "Main", grey otherwise |
| Buttons | A plain button, a quiet one (no edge, muted text) for Remove and Clear, a danger colour for removing things, and one gold button only where a screen has a single main action |
| Add | A dashed full-width button under a list: "+ Add rule", "+ Add a custom tool" |

Today's error line (`model.error`) shows at the top of the content column,
as a notice box in the danger colour, above the screen head. It is not
buried at the end. *(Builder's call; the mockups do not show it.)*

### Each desktop screen

The ledes are the user-facing copy. Keep them as written.

**General**: "The agent behind Maya, and how she looks."
- Agent: Agent (with today's help line), Model.
- Appearance: Theme.
- Updates: "Maya \<version>", with today's update status as its help line,
  and Check for updates. When updates are off, the row shows the version
  only.

**Keyboard**: "The keys for the Chat tab, a session's reply box and the
new-session prompt."
- One card, one row per action. The keys show as chips that look like
  keys, with × on each removable one, then a quiet "+ Add". The add's
  recording state ("Press keys… (Esc to cancel)") and the error stay as they
  are, in the row.
- Foot note: "An Enter press in neither list does what the box does by
  itself." Then Reset to defaults.

**Sessions**: "How Maya follows your sessions, and where new ones start."
- Session tracking, note "Hooks let the agents tell Maya what they are
  doing, rather than her guessing from transcripts." Rows: Claude Code
  hook, Usage status line and Codex hook. Each has a help line, a state
  (Installed or Not installed) and an action (Remove or Install). Today's
  status sentences become the help lines. Whatever the status line's
  "blocked by" case says today, it says in the help line.
- Board: "Completed decays to Idle after" [number] "minutes"; Session
  viewer.
- New sessions: Projects directory; Open in (the terminal picker, on the
  platforms that show it today).

**Git**: "The accounts Maya reads pull requests and merge requests with.
Reviews clone with your own git credentials."
- A card per forge. Its head holds the name and the on/off. Its rows: URL
  (GitLab only); Token, a two-way choice (Paste a token or From env var)
  with the scope it needs as help ("Needs the repo scope." or "Needs
  read_api."); the token field ("Saved in \<store>." as help when one is
  stored, and a quiet Clear) or Variable; Clone with, a two-way choice (SSH
  or HTTPS); and a foot row with the last Test's outcome and Test.

**Pull requests**: "Where reviews are checked out, what the Review button
asks the agent, and when Maya tells you about your own pull requests."
- Reviews: Clones directory ("For reviews and worktrees."); Review prompt
  as a stacked row, with today's help line and a 7-row text box.
- Your pull requests: the switch "Notify me when a PR assigned to me is
  approved or needs changes".

**Work Items**: "Where the Issues tab finds your work, and what Start does
with an item."
- **Sources**, note "Open one to change what the Issues tab picks up from
  it." One collapsible card per source: GitHub, GitLab, then each custom
  tool. The head is always visible and holds:
  - a chevron and the source's name (a custom tool also gets a "Custom
    tool" tag);
  - when folded up, a one-line summary. For a forge: "N rule(s)", plus
    "; Start sets status to \<status>" when that is set. For a custom tool:
    its list command, in monospace;
  - "On" (gold) or "Off", and the on/off switch. A source that is off greys
    its name and body.
  Clicking the head (not the switch) opens or folds it.
- An open forge: "Issues to pick up" as a stacked row ("An issue shows when
  it matches any one of these rules."). Each rule is a box: a header line
  that sums the rule up, and a quiet Remove. Under it, Assignee, Labels and
  Repositories, each label beside its full-width field. Then "+ Add rule".
  Then Status labels ("In order, comma-separated."), On Start, set status
  to, and Tracker-only repositories ("Comma-separated."), with the token
  hint as the card's foot.
- The rule's header line is written from its fields: "Assigned to me",
  "Assigned to anyone" or "Assigned to \<login>"; then ", labelled a, b or
  c" when it has labels; then ", in owner/x or owner/y" when it names
  repositories. The exact wording is the builder's.
- An open custom tool: Name ("How its items are labelled on the Issues
  tab."); List command (stacked, monospace, help "Runs every 2 minutes
  through your shell, and must finish within 30 seconds."); On-start command
  (optional) (stacked, monospace, today's help); Your name there (optional)
  with today's help; "Output format and an example script" as a details;
  then "Remove this tool" at the card's foot, away from the switch.
- "+ Add a custom tool" under the sources.
- Work prompt: one stacked row, help "What Start tells the agent. Blank
  uses the built-in prompt. {id}, {title}, {url}, {description} and {repo}
  are filled in."
- **A forge whose Git account is off** (in Settings › Git) keeps today's
  behaviour: its fields are disabled, except the on/off, and the open body
  starts with "GitHub is off in Settings → Git." and the "Open Git
  settings" link. Folded, the head shows that same sentence in place of the
  summary, and its state word reads "Git off" (muted) in place of "On",
  because the tab fetches nothing from it. *(Builder's call.)*
- **Which sources are open** *(builder's call; the user did not set a
  rule)*: all are folded on the first visit. Each source's open or folded
  state is then remembered per device, the same way the open group is
  (`maya.settingsGroup` in localStorage), keyed by source (`github`,
  `gitlab`, the custom tool's `id`). It lives in the model, so a repaint
  keeps it. A source just added with "+ Add a custom tool" opens.

**Notifications**: "How Maya tells you a session needs you."
- One card: "Notify me when a session awaits a decision"; "Speak instead
  of a sound" with help "\"needs a decision\", \"is finished\"". The Focus
  hint, when it shows today, becomes the card's foot note.

**Voice**: "How Maya speaks to you, and how she hears you."
- Speaking, note "Maya speaks with the first voice that works, in this
  order." Then one card per voice, numbered with a small gold-edged circle
  (1, 2, 3), because Maya does try them in this order:
  1. ElevenLabs: on/off in the head; API key ("Saved in \<store>." when
     set); Voice.
  2. Voice server: head help "Any OpenAI-compatible speech server.";
     on/off; Server address; Voice (the list, or a text field as today, with
     the list error); Model; API key.
  3. The built-in voice (`builtinVoiceName()`), head help "Built in; always
     there."
  Then a card with Volume (slider and percent) and a foot row with Try the
  voice and, when there is one, the last test's report.
- Listening: Listen for the wake word (help "On-device speech
  recognition."); Wake word, with today's help; Speech recognition;
  Microphone. The Whisper model list and download progress, when they
  show today, become rows in this card, each with its size and state and its
  Download or Remove, in today's wording. Today's listen error and note
  become the card's foot note in the danger colour or muted.

**Network**: "Join your other computers and your phone to this Maya."
- One card: "This Maya" (the role select); then by role, what shows today,
  as rows:
  - Main: Port (help: the machine's addresses line, which `tickAddresses`
    keeps updating); This machine's name.
  - Assistant: the main's host, port, this machine's name, the pairing
    code field, and Pair as the card's one gold action. The status, error,
    last-connected and "pair again" lines go in its foot.
- **Pairing code** (main): the one loud thing in Settings. A card with a
  `--gold-deep` edge and a faint gold glow in one corner, the code in 40px
  gold monospace, "Type this on the machine you are pairing." and the expiry
  beside it (still ticked in place by `tickPairingCode`), and Regenerate.
  The "used up" and "expired" states keep today's wording in that card.
  With no live code (before the first one, or after one expires), the card
  keeps today's behaviour. It shows a muted placeholder in the code's place
  ("No code yet") and the "Show pairing code" button, and a code appears only
  when the user asks for one. That keeps `tickPairingCode`, which finds the
  code's elements by class and turns Regenerate back into "Show pairing
  code", working as it does. *(Builder's call.)*
  Foot note: "Paired devices are in Settings › Devices."
- New key: a row with help "Drops every pairing, so each device must pair
  again." and a danger New key button. Its confirmation ("Drop every
  pairing and make a new key?") opens in that card.
- Forget main, Pair again and the other role-specific buttons stay, each in
  the row or foot where its context is.

**Devices**: "The computers and phones joined to this Maya."
- One card, one row per device: a small mark naming the platform (Mac,
  Windows, Linux, Phone), the name (with a "This device" badge on this one)
  and its platform and address under it, a state on the right (Main,
  Connected, Last seen …, Never connected), then its buttons (Make main,
  Join, and a quiet danger Remove). A phone's "Sees and controls all
  sessions" switch sits under its name. The Join form and the Make main
  confirmation open inside that device's row. Name clashes keep today's
  hostname hint.
- Off: the card holds today's line "Turn the network on in Settings ›
  Network to see your devices."

**Debug**: "For looking under the hood."
- One card: Debug mode, help "Shows the Debug tab and Maya's own card."

**Release Notes**: "What changed in each version of Maya."
- Two columns: Maya's full-body art (`src/assets/maya-full-body.png`) on
  the left, and the notes on the right. She is 290px wide, set 64px into
  the gap toward the sidebar, at most 600px tall (76vh), sticky as the notes
  scroll, with the same gold glow as the What's new window. Her raised hand
  presents the notes. Under a 1100px window she hides and the notes take
  the column.
- The notes are at most 68ch wide. Each version is an 18px heading, the
  running one with a gold "Running" pill. New and Fixed are 13px gold
  headings, list items have 8px between them at 1.6 line height, and a
  hairline separates versions. This restyles `renderReleaseNotes` output in
  Settings only. The What's new window keeps its own look.

### The phone

The phone keeps its list-then-page navigation, and its history entries
(`settings-history.ts`) behave as today.

- **The list**: the title "Settings" (26px), then cards of tappable rows,
  grouped like the desktop sidebar: Network and Devices; Notifications and
  Voice; Session viewer; Release Notes. Each row shows its name, its
  summary (`groupSummary`, as today; Network's "Connected to …" gets the
  green state dot) and a chevron. Rows are at least 64px tall.
- **A page**: a gold "‹ Settings" back link, the page's title (26px) and a
  lede, then the desktop's parts at phone sizes. Fields sit under their
  label at 44px tall and 15px text. Actions are full-width buttons at the
  bottom of their card. Switches are 44×26.
- Ledes: Network "This phone joins the Maya on your computer."; Devices
  "The computers and phones in your group."; Notifications "When the phone
  tells you a session needs you."; Voice "While Maya is on screen. She
  never listens with the app closed."; Release Notes "You're running Maya
  \<version>."
- **Network**, joined: This phone (Name; the state "Connected to \<main>"
  with host:port beside it; the reconnect error when there is one; Save
  name and a danger Leave side by side); Key (help "Drops every pairing, so
  each device must pair again." and a full-width danger New key, with its
  confirmation in place). Not joined: Join a main, with today's note "Open
  Settings → Network on the main, choose Show pairing code, and type the
  code it shows." (`JOIN_HINT`),
  then Host, Port and Pairing code (Port and Pairing code side by side),
  and Join as the gold button; then This phone with Name and Save name. The
  "pair again" line stays.
- **Notifications**: one card with Awaiting Decision and Completed as
  switches. When Android's switch is off, "Notifications are off for Maya
  in Android settings." is a notice box under the card. Then Battery, with
  help "Android may otherwise stop Maya with the screen off, and her
  notifications with her." and a full-width "Keep Maya awake".
- **Voice**: Speak ("Reads out sessions that need you.") and Wake word
  "\<word>" ("Experimental. The word is set on your computer.") as switches,
  and the foot note "Voice needs a desktop main."
- **Session viewer**: one row, "Show sessions as", with a two-way choice
  of Chat or Terminal in place of today's select. It keeps the `name`
  `viewerTheme` and its values.
- **Devices**: as on the desktop, but the state goes under the name, since
  the row has no room beside it. A phone shows no Make main or pairing,
  as today.
- **Release Notes**: Maya from the waist up as a full-width banner, exactly
  as the phone's What's new window shows her (`src/mobile/mobile.css:111`:
  200px tall, `object-fit: cover`, `object-position: 50% 6%`, and a mask
  that fades her out from 55% down). The title and lede sit over the fade,
  aligned right, with the same text shadow (`mobile.css:113`). The back
  link sits over the banner's top left.

## The builders

`src/settings-ui.ts`, used by `src/settings.ts`, `src/devices.ts` and
`src/mobile/network.ts`. It sketches the parts in the table above: a
screen head, a group (title, note, foot), a card, a row (label, help,
control), a stacked row, a card head with an on/off, a foot row, a switch,
a two-way choice, a state, an add button, and a collapsible source (head
with name, tag, summary, On/Off and switch, plus its body). Its exact API
is the builder's. It must:

- return plain elements with a stable structure, so `morph` can patch the
  Devices section that uses them;
- take the control as an element, so each screen keeps creating its own
  inputs with their `name`, value and handlers;
- give help lines `aria-describedby` from their control, and give a switch
  and a two-way choice an accessible name from their row's label;
- keep visible keyboard focus (gold ring) on every control, the
  collapsible head included (`aria-expanded`).

## What's left to the builder

- The light-theme token values.
- The exact API of `settings-ui.ts`, and its CSS class names (keep one
  prefix).
- The rule header wording, within the shape above.
- How the open/folded state of sources and of the custom tool's details is
  stored, within the rule above.
- The order of the work. One screen at a time with its tests is the
  sensible order; the builders come first.

## Release

A `feat`: bump the minor version (0.54.0 from main's 0.53.0, or from main's
version at merge time) and add the release notes section, as CLAUDE.md
says. The notes are for the people who use Maya, for example: "Settings
has a new look: centred, with settings grouped in cards, help next to each
setting, switches for every on and off, and room to breathe, on the
computer and the phone." "In Work Items, each source folds up and shows
whether it is on." Fixed: "A Work Items rule's Labels and Repositories
fields are wide enough to read again."
