# Eye: selectable answer options — design

Date: 2026-09-28
Status: approved design
Builds on: `2026-09-28-eye-session-board-design.md`, `2026-09-28-eye-reply-modal-design.md`

## Purpose

When a session asks the user a question with options (Claude Code's
AskUserQuestion tool), show those options as buttons on the card and in the
modal, and answer the question in the terminal when one is clicked.

## Findings from the spike (verified on a throwaway session)

- The terminal picker lists options as `1. Label`, `2. Label` … then
  `N+1. Type something.` and `N+2. Chat about this`. The cursor starts on
  option 1. `↓` moves the cursor, `Enter` selects.
- Keys can be injected into the Terminal tab without focusing it, using
  AppleScript `do script "<text>" in <tab>`, which types the text and then
  Enter. `ESC [ B` typed this way acts as `↓`. So option n (1-based) is
  `ESC[B` × (n−1) followed by the Enter that `do script` appends.
- A single-question ask is submitted by that Enter. A multi-question ask
  moves to the next question after each selection and finishes on a
  "Ready to submit your answers? 1. Submit answers 2. Cancel" screen where
  Enter submits.
- Keys sent before the picker is on screen land in the prompt box; stray
  arrow keys move the cursor on the next picker. So the driver sends only
  when the question is confirmed open, and exactly one action per question.
- `Chat about this` (the last entry) declines the question. Nothing in this
  design ever moves the cursor past the listed options.
- Not verified, therefore out of scope: `Type something` custom answers and
  multi-select questions.

## Behaviour

### Data

- `Awaiting` gains `questions: Vec<Question>`; empty unless the kind is
  `question`. `Question { question: String, header: String, options:
  Vec<Choice>, multi_select: bool }`, `Choice { label: String, description:
  String }`. Parsed from the hook event's `tool_input.questions` (PreToolUse
  or PermissionRequest for AskUserQuestion) and, in the transcript fallback,
  from the tool_use input.
- Card JSON: `awaiting.questions[]` with camelCase `multiSelect`.

### UI

- Card in Awaiting Decision with a question: under the question text, the
  options of the **current** question as buttons showing the label; the
  description is the button's tooltip. For a multi-question ask the card
  shows `Question 1 of 2` above the buttons.
- Modal banner: the same, plus each option's description under its label.
- Multi-select questions: options rendered disabled with the note
  "Multi-select: answer in the terminal".
- Clicking an option sends the answer. On success the next question's
  options appear (or, after the last, the buttons disappear and the card
  moves on when the hook reports the tool finished). On failure a toast (card)
  or the modal status line shows the error and the buttons stay.
- Progress ("next question index") is kept per session in the frontend and
  reset whenever the session's awaiting timestamp or question list changes.
- Buttons are enabled only once the question has been open for 1 second, so
  the picker is on screen before any key is sent.

### Driver (Rust)

- `answer::keys_for_option(option_index: usize) -> String` returns
  `"\x1b[B"` repeated `option_index` times (0-based index, no newline).
- `answer::applescript_type(tty: &str, text: &str) -> String` builds the
  script that finds the tab by tty and runs `do script text in t`, returning
  `"ok"` or `"not found"`, without activating Terminal.
- `answer::check(card: &Card, question_index, option_index, now_ms) ->
  Result<(), String>` refuses unless: state is Awaiting with kind question;
  `question_index` and `option_index` are in range; the question is not
  multi-select; `now_ms - state_since >= 1000`.
- Command `answer_question(session_id, question_index, option_index)`:
  derive the session's current card; run `check`; resolve the pid's tty;
  type the keys; if the ask has more than one question and this was the last
  one, wait 400 ms and type `""` (Enter) to submit. Errors are sentences.
- `focus.rs` exposes `tty_for_pid(pid) -> Result<String, String>` shared by
  focus and answer.

### Errors

- "This session is not waiting for a question." (state changed)
- "That question has no such option."
- "Give the terminal a second to show the question." (too early)
- "Multi-select questions must be answered in the terminal."
- "No Terminal tab found for this session." / osascript errors verbatim.

## Testing

- Rust: `keys_for_option` (0 → "", 2 → two Down sequences); `applescript_type`
  contains `do script`, the tty and no `activate`; `check` for each refusal
  and the happy path; question parsing from a hook event with two questions
  including `multiSelect`; transcript fallback carries options.
- Vitest: card renders option buttons for question 0 with `data-q`/`data-opt`,
  tooltip from description, "Question 1 of 2" label; disabled for
  multi-select; modal banner renders descriptions; click routing yields an
  `answer` action with both indices; progress helper advances and resets.
- Manual: on a live session that asks a question, click an option on the
  card and see the terminal record the answer; do the same through the modal
  for a two-question ask.

## Out of scope

`Type something` custom answers, multi-select questions, permission
Allow/Deny buttons, plan-approval options.
