# Markdown in Chat, spoken as plain words

Status: design, agreed with the user on 2026-10-28. Items marked
**(my call)** were decided by Claude while writing this up and were not
discussed; the user can overturn any of them.

## Intent

Chat with Maya's brain is meant to be heard. Her spoken words must be
short: no full paths, commit hashes or long ids read aloud. But the screen
can carry more than the voice: working links, and detail the user may want
to read. When a detail matters, Maya says so in a few words and the chat
shows it.

Today one plain text does both jobs. Chat shows it as text (nothing is
clickable), and the voice reads it exactly as written.

## Goals

- **Chat renders her messages as markdown**: bold, lists, code, tables and
  links. The user's own messages stay plain text.
- **Links open in the browser** and never navigate the window.
- **Speech reads the rendered words, not the markup.** A link is read as
  its label, and bold and headings as plain words. Tables, code blocks and
  images are not read.
- **She introduces what speech skips.** Her prompt tells her to keep the
  spoken sentences short and to put details that matter on screen (paths,
  hashes, lists of failures) in a table, list or code block, with a short
  spoken lead-in such as "The failing tests are in a table in the chat."
  She writes the lead-in. Speech adds no fixed line of its own.
- **Chat opens at the latest message.** Today it opens at the top: the
  pane is redrawn while its tab is hidden, the hidden log has no height,
  so scrolling to the end does nothing, and opening the tab never scrolls
  it again.

## Non-goals

- Markdown in the user's messages, or in the voice panel.
- A separate "spoken" and "written" text from the model. One reply serves
  both. Speech is derived from it.

## Approaches

1. **Parse the markdown in Rust with `pulldown-cmark`.** Chosen. Speech
   runs in Rust, also when no window is open (her own news, phone
   requests). A real parser handles tables without leading pipes, nested
   lists, reference links and inline code, and reads markdown the same way
   `marked` does in the window.
2. **Strip the markup by hand in Rust.** No new dependency, but each edge
   case it misses is read aloud as noise.
3. **Convert in the window with `marked`.** Ruled out: Maya speaks when no
   window is open.

## Decided

- Chat reuses `renderMarkdown` from `src/markdown.ts`, the sanitised
  renderer the session view uses: no scripts, images or event handlers,
  and only `http(s)` links. Link clicks go through the same handler that
  the session view uses (no navigation).
- Chat history keeps her raw markdown. Only the text sent to the voice is
  converted.
- Opening the Chat tab scrolls its log to the latest message.

## My calls

- **(my call)** The speech text is made by one function in `core`,
  `speech::from_markdown`, with unit tests. It is applied where every
  line reaches the computer's voice (`listener::reply_aloud_as`) and to
  the line the phone speaks (`net_app::brain_reply_data`). Maya's fixed
  lines are plain text and pass through it unchanged; tests pin that.
- **(my call)** A bare URL in her prose is read as "a link".
- **(my call)** List items become separate sentences. Inline code is read
  as its text. A heading is read as a sentence. If nothing is left to say
  (a reply that is only a table), nothing is spoken.
- **(my call)** The phone uses the same `renderChat`, so it gets markdown
  too. If the phone has no way to open a link outside the app, the builder
  says so and does not add one.

## Left to the builder

- The exact prompt wording, which replaces the brevity line added earlier
  on this branch, and its tests.
- How the Chat tab's open event reaches the scroll (the `maya:tab` event
  already fires on each switch).
