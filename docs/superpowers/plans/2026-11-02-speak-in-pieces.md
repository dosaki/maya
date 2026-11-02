# Speak Long Replies in Pieces Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Maya speaks a long reply as short clips (one per paragraph, long paragraphs split at a natural stop), making the next clip while the current one plays.

**Architecture:** `core/src/speech.rs` turns markdown into a list of pieces. An `Utterance` carries the pieces; the speech thread in `core/src/notify.rs` walks them, playing piece *n* while a helper thread fetches piece *n+1* into the voice cache. The remote speaker is split into *fetch* (cache or synthesise, return the file) and *play*. Only the speech thread reads or writes the down-marks: the helper thread just returns a `Result`.

**Tech Stack:** Rust (core crate `maya_core`, Tauri app `src-tauri`), `pulldown-cmark`, std threads.

**Spec:** `docs/superpowers/specs/2026-11-02-speak-in-pieces-design.md`

## Global Constraints

- The piece limit is 400 bytes (`str::len`; a non-ASCII reply gets slightly shorter pieces): `pub const PIECE_LIMIT: usize = 400;` in `core/src/speech.rs`.
- Split order for an over-long block: sentence ends (`.` `!` `?` `…` followed by a space), then `;` `:` `,`, then between words. Never inside a word.
- `pieces(t).join(" ") == from_markdown(t)` for every input: Chat, the voice panel and history keep showing the same words as today.
- The speech hook runs once per utterance: `Starting(<whole text>)` before the first piece, `Finished` after the last.
- Once a voice fails on a piece, the rest of that utterance never goes back to it.
- The phone (`mobile/`) does not change.
- Version bump: `main` is 0.48.0 and this is a `feat`, so 0.49.0 (`sh scripts/set-version.sh 0.49.0`, then check `Cargo.lock` changed), in its own `chore(release): 0.49.0` commit with the `RELEASE_NOTES.md` section.
- Unix-only tests are gated `#[cfg(unix)]`; compare paths by component (Windows CI).

## Review Focus

1. **A reply whose voice server dies mid-reply**: the rest is spoken by the next voice, every piece is heard once, none twice. (Task 2: `a_failure_mid_reply_moves_the_rest_to_the_next_voice`.)
2. **A prefetch that fails** (server error on piece *n+1* while *n* plays): it counts as that voice failing on piece *n+1*, marks it down, and piece *n+1* goes to the next voice. (Task 2: `a_failed_prefetch_counts_as_the_voice_failing_on_that_piece`.)
3. **A voice marked down before the reply starts**: every piece tries its cache only, and a miss moves that piece (and the rest) on silently, as for a single line today. (Task 2: `a_voice_marked_down_plays_cached_pieces_only`.)
4. **One sentence longer than the limit with no punctuation** (a pasted log line, a long URL already turned into "a link"): split between words, no piece over the limit unless a single word is. (Task 1: `a_sentence_with_no_stops_splits_between_words`.)
5. **A reply that is only a table or code block**: no pieces, nothing spoken, exactly as today. (Task 1: `a_reply_with_no_prose_has_no_pieces`.)

---

### Task 1: Markdown into pieces

**Files:**
- Modify: `core/src/speech.rs`

**Interfaces:**
- Produces: `pub const PIECE_LIMIT: usize = 400;` and `pub fn pieces(text: &str) -> Vec<String>` in `maya_core::speech`. `from_markdown` keeps its signature and output.

- [ ] **Step 1: Write the failing tests** in `speech.rs`'s test module:

```rust
#[test]
fn short_replies_are_one_piece() {
    assert_eq!(pieces("Sure.\n\nFocusing api-fix."), ["Sure. Focusing api-fix."]);
    assert_eq!(pieces("Two left:\n- api-fix\n- docs, waiting?"), ["Two left: api-fix. docs, waiting?"]);
}

#[test]
fn paragraphs_become_pieces_when_together_they_pass_the_limit() {
    let a = "a".repeat(10) + &" word".repeat(50); // ~260 chars
    let b = "b".repeat(10) + &" word".repeat(50);
    let p = pieces(&format!("{a}\n\n{b}"));
    assert_eq!(p.len(), 2);
    assert!(p[0].starts_with("aaaa") && p[0].ends_with('.'), "an added stop ends a piece that is not last: {:?}", p[0]);
    assert!(p[1].starts_with("bbbb") && !p[1].ends_with('.'), "the last piece keeps from_markdown's ending");
}

#[test]
fn a_long_paragraph_splits_at_sentence_ends() {
    let s = "This sentence is about forty characters. ".repeat(15); // ~615 chars, one paragraph
    let p = pieces(&s);
    assert!(p.len() >= 2);
    assert!(p.iter().all(|x| x.len() <= PIECE_LIMIT), "{p:?}");
    assert!(p.iter().all(|x| x.ends_with('.')), "each piece ends at a sentence end: {p:?}");
}

#[test]
fn a_long_sentence_splits_at_a_clause_before_a_word() {
    let s = format!("{}, {}", "word ".repeat(60).trim(), "more ".repeat(30).trim()); // one sentence, ~450 chars
    let p = pieces(&s);
    assert!(p[0].ends_with(','), "{p:?}");
}

#[test]
fn a_sentence_with_no_stops_splits_between_words() {
    let s = "word ".repeat(200);
    let p = pieces(&s);
    assert!(p.iter().all(|x| x.len() <= PIECE_LIMIT && !x.starts_with(' ') && !x.ends_with(' ')), "{p:?}");
    assert_eq!(p.join(" "), from_markdown(&s));
    let long_word = "x".repeat(PIECE_LIMIT + 5);
    assert_eq!(pieces(&long_word), [long_word.clone()], "a single word is never cut");
}

#[test]
fn a_reply_with_no_prose_has_no_pieces() {
    assert!(pieces("| a |\n|---|\n| 1 |").is_empty());
    assert!(pieces("").is_empty());
}

#[test]
fn pieces_join_back_into_what_from_markdown_says() {
    let long = "This sentence is about forty characters. ".repeat(15);
    for t in ["## Done\n**All green**, and `main` is merged.", "Two left:\n- api-fix\n- docs, waiting?", "See [the PR](https://github.com/o/r/pull/1).", "Sure.\n\nFocusing api-fix.", long.as_str(), format!("{long}\n\n{long}").as_str()] {
        assert_eq!(pieces(t).join(" "), from_markdown(t), "{t}");
    }
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p maya-core speech::`
Expected: FAIL, `cannot find function pieces` / `PIECE_LIMIT`.

- [ ] **Step 3: Implement `pieces`.** Factor `from_markdown`'s event loop into a private `fn blocks(text: &str) -> Vec<String>`: one entry per paragraph, heading or list item, whitespace-collapsed, ending with the stop `end_sentence` would add (none on the last block when it was only added, as `from_markdown` does today). `from_markdown` becomes `blocks(text).join(" ")`. `pieces` packs consecutive blocks into one piece while the joined length stays ≤ `PIECE_LIMIT`; a block over the limit is split by a private `fn split_long(block: &str, limit: usize) -> Vec<String>` (greedy: take the longest prefix ≤ limit ending at the latest sentence end, else the latest `;` `:` `,`, else the latest space; a single word longer than the limit is its own piece), and its parts are packed like blocks.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p maya-core speech::`
Expected: PASS, including every existing `from_markdown` test.

- [ ] **Step 5: Commit**

```bash
git add core/src/speech.rs
git commit -m "feat: split her words into pieces short enough to speak"
```

---

### Task 2: Speak an utterance piece by piece, fetching ahead

**Files:**
- Modify: `core/src/notify.rs` (Utterance ~195, RemoteSpeaker ~310, `speak_chain` ~382, `speak_line` ~415, `deliver` ~427, tests ~700–1080)
- Modify: `src-tauri/src/voice.rs` (`speak_remote` ~198 becomes `fetch_remote`)
- Modify: `src-tauri/src/lib.rs` (~1937, the `set_remote_speaker` call)

**Interfaces:**
- Consumes: nothing from Task 1 (pieces arrive as `Vec<String>`).
- Produces:
  - `Utterance` gains `pub pieces: Vec<String>`. `Utterance::new(text, voices)` sets `pieces: vec![text.clone()]`. New `Utterance::in_pieces(pieces: Vec<String>, voices: Vec<RemoteVoice>) -> Utterance` with `text = pieces.join(" ")`.
  - `pub struct RemoteSpeaker { pub fetch: fn(&RemoteVoice, &str, Reach) -> Result<PathBuf, String>, pub play: fn(&Path, u8) -> Result<(), String> }` replaces the `RemoteSpeaker` type alias; `set_remote_speaker(RemoteSpeaker)` keeps its name. `fetch` with `Reach::CacheOnly` returns `Err("not cached")` for a miss; with `Reach::Network` it synthesises a miss into the cache.
  - In `src-tauri/src/voice.rs`: `pub fn fetch_remote(v: &RemoteVoice, text: &str, reach: Reach) -> Result<PathBuf, String>` (today's `speak_remote` without the `play`); `speak_remote` is removed. `lib.rs` installs `RemoteSpeaker { fetch: voice::fetch_remote, play: voice::play }`.

- [ ] **Step 1: Write the failing tests.** Replace the test double `Voices` so it scripts answers per `(label, piece text)` and logs calls in order as strings: `"fetch ElevenLabs Network p1"`, `"ahead the voice server Network p2"`, `"play p1"`, `"builtin p3"`. `speak_chain` is driven through a private trait (Step 3) that the double implements; its `fetch_ahead` runs the fetch at once, logs `ahead …`, and returns `Ahead::Ready`. Keep every existing chain test passing against the new double (one-piece utterances behave exactly as before), and add:

```rust
fn reply(n: usize) -> Utterance {
    Utterance::in_pieces((1..=n).map(|i| format!("p{i}")).collect(), vec![eleven(), server()])
}

#[test]
fn the_next_piece_is_fetched_before_the_current_one_plays() {
    let v = Voices::default();
    v.speak(&reply(3), &mut DownMarks::default(), Instant::now());
    assert_eq!(v.take_log(), ["fetch ElevenLabs Network p1", "ahead ElevenLabs Network p2", "play p1", "ahead ElevenLabs Network p3", "play p2", "play p3"]);
}

#[test]
fn a_failure_mid_reply_moves_the_rest_to_the_next_voice() {
    let v = Voices::default();
    v.fail(ELEVEN, "p2", "502");
    let mut marks = DownMarks::default();
    let report = v.speak(&reply(3), &mut marks, Instant::now());
    let log = v.take_log();
    assert_eq!(log.iter().filter(|l| l.starts_with("play")).collect::<Vec<_>>(), ["play p1", "play p2", "play p3"], "each piece is heard once");
    assert!(!log.iter().skip_while(|l| !l.contains("p2")).any(|l| l.contains(ELEVEN) && l.contains("p3")), "ElevenLabs is not asked for p3: {log:?}");
    assert_eq!(report, SpeechReport { spoken_by: SERVER, failures: vec!["ElevenLabs failed: 502.".into()] });
    assert!(marks.is_down(eleven().fingerprint(), Instant::now()));
}

#[test]
fn a_failed_prefetch_counts_as_the_voice_failing_on_that_piece() {
    // p2 fails when fetched ahead: p2 is fetched from the server at its turn, then p3 ahead from the server.
    let v = Voices::default();
    v.fail(ELEVEN, "p2", "timeout");
    v.speak(&reply(3), &mut DownMarks::default(), Instant::now());
    let log = v.take_log();
    let at = |s: &str| log.iter().position(|l| l == s).unwrap_or_else(|| panic!("{s} missing from {log:?}"));
    assert!(at("ahead ElevenLabs Network p2") < at("play p1"));
    assert!(at("play p1") < at("fetch the voice server Network p2"));
    assert!(at("fetch the voice server Network p2") < at("ahead the voice server Network p3"));
}

#[test]
fn with_every_remote_voice_failing_the_builtin_speaks_the_rest() {
    let v = Voices::default();
    v.fail(ELEVEN, "p2", "a");
    v.fail(SERVER, "p2", "b");
    let report = v.speak(&reply(3), &mut DownMarks::default(), Instant::now());
    assert_eq!(*v.builtin.borrow(), ["p2", "p3"]);
    assert_eq!(report.spoken_by, "the built-in voice");
    assert_eq!(report.failures.len(), 2);
}

#[test]
fn a_voice_marked_down_plays_cached_pieces_only() {
    let v = Voices::default();
    let mut marks = DownMarks::default();
    let t0 = Instant::now();
    marks.mark(eleven().fingerprint(), t0);
    v.fail_cache(ELEVEN, "p2");
    let report = v.speak(&reply(3), &mut marks, t0 + Duration::from_secs(1));
    let log = v.take_log();
    assert!(log.contains(&"fetch ElevenLabs CacheOnly p1".to_string()), "{log:?}");
    assert!(!log.iter().any(|l| l.contains("ElevenLabs Network")), "a down voice never goes to the network: {log:?}");
    assert!(!log.iter().any(|l| l.contains(ELEVEN) && l.contains("p3")), "after a miss the rest stays with the next voice: {log:?}");
    assert!(report.failures.is_empty(), "a cache miss while down is not a failure");
}

#[test]
fn a_reply_is_one_utterance_between_the_hook_calls() {
    let log = std::sync::Mutex::new(Vec::<String>::new());
    let hook = |p: SpeechPhase| log.lock().unwrap().push(format!("{p:?}"));
    deliver(Utterance::in_pieces(vec!["One.".into(), "Two.".into()], vec![]), Some(&hook), &|u: &Utterance| {
        log.lock().unwrap().push(format!("speak {}", u.pieces.len()));
        SpeechReport { spoken_by: "the built-in voice", failures: vec![] }
    });
    assert_eq!(*log.lock().unwrap(), ["Starting(\"One. Two.\")", "speak 2", "Finished"]);
}

#[test]
fn fetching_ahead_runs_on_another_thread() {
    let (started_tx, started_rx) = mpsc::channel();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let ahead = Ahead::spawn(move || {
        started_tx.send(()).unwrap();
        go_rx.recv().unwrap();
        Ok(PathBuf::from("/m/voice/x.wav"))
    });
    started_rx.recv_timeout(Duration::from_secs(5)).expect("the fetch started without being waited for");
    go_tx.send(()).unwrap();
    assert_eq!(ahead.wait(), Ok(PathBuf::from("/m/voice/x.wav")));
}
```

The double's helpers: `fail(label, piece, err)` fails both `fetch` and `ahead` for that pair over the network; `fail_cache(label, piece)` makes a `CacheOnly` fetch of it miss; anything unscripted succeeds. `take_log()` drains the log.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p maya-core notify::`
Expected: FAIL to compile (`in_pieces`, `Ahead`, `take_log` missing).

- [ ] **Step 3: Implement.**
  - `pub(crate) enum Ahead { Ready(Result<PathBuf, String>), Running(std::thread::JoinHandle<Result<PathBuf, String>>) }` with `fn spawn(f: impl FnOnce() -> Result<PathBuf, String> + Send + 'static) -> Ahead` and `fn wait(self) -> Result<PathBuf, String>` (a panicked thread is `Err("the fetch stopped")`).
  - A private trait the chain runs on, implemented for the installed `RemoteSpeaker` + `say_builtin` in production and by the test double:
    ```rust
    trait Speak {
        fn fetch(&self, v: &RemoteVoice, text: &str, reach: Reach) -> Result<PathBuf, String>;
        fn fetch_ahead(&self, v: &RemoteVoice, text: &str, reach: Reach) -> Ahead;
        fn play(&self, path: &Path, volume: u8) -> Result<(), String>;
        fn builtin(&self, text: &str, volume: u8);
    }
    ```
    Production `fetch_ahead` clones the voice and text into `Ahead::spawn`.
  - `speak_chain(u, marks, now, s: &dyn Speak) -> SpeechReport` keeps a position in `u.voices`. For each piece: try voices from the position, as today (down and not a trial → `CacheOnly`, a miss moves on silently; otherwise `Network`, an error marks it down, records the failure and moves on). The fetch for a piece uses the `Ahead` started for it when that was started for the same voice, else fetches now. Once a piece has its file, start `fetch_ahead` of the next piece with that voice and the same `Reach`, then `play`. A `play` error is that voice failing on that piece. Moving on advances the position for the rest of the utterance; past the last remote voice, `builtin` speaks this piece and every later one. A success clears the mark, as today. `spoken_by` is the voice of the last piece. An `Ahead` left unused is dropped (its thread finishes into the cache).
  - `speak_line` builds the production `Speak`; with no `RemoteSpeaker` installed, the built-in voice speaks each piece in turn. `deliver` logs `saying:` with `u.text` once.
  - `src-tauri/src/voice.rs`: `fetch_remote` returns the cache path after `synthesize` (or the cached file); its doc comment says it no longer plays. Update the module doc if it mentions `speak_remote`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p maya-core notify:: && cargo test -p maya --lib voice::`
Expected: PASS. Then `cargo clippy --workspace --all-targets -- -D warnings` passes.

- [ ] **Step 5: Commit**

```bash
git add core/src/notify.rs src-tauri/src/voice.rs src-tauri/src/lib.rs
git commit -m "feat: speak a long line piece by piece, making the next while one plays"
```

---

### Task 3: Replies are spoken in pieces, and the release

**Files:**
- Modify: `src-tauri/src/listener.rs` (`reply_aloud_as` ~159)
- Modify: `RELEASE_NOTES.md`, and the version files via `scripts/set-version.sh`

**Interfaces:**
- Consumes: `maya_core::speech::pieces` (Task 1), `notify::Utterance::in_pieces` (Task 2).

- [ ] **Step 1: Write the failing test.** `reply_aloud_as` needs an `AppHandle`, so pull its text step into `fn spoken_pieces(text: &str) -> (String, Vec<String>)` in `listener.rs` (the words for the panel, and the pieces) and test that:

```rust
#[test]
fn a_long_reply_is_spoken_in_pieces_and_shown_whole() {
    let para = "This sentence is about forty characters. ".repeat(8);
    let (words, pieces) = spoken_pieces(&format!("{para}\n\n{para}"));
    assert!(pieces.len() >= 2);
    assert_eq!(words, pieces.join(" "));
    assert_eq!(words, maya_core::speech::from_markdown(&format!("{para}\n\n{para}")));
}
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p maya --lib listener::tests::a_long_reply`
Expected: FAIL, `spoken_pieces` not found.

- [ ] **Step 3: Implement.** `reply_aloud_as` uses `spoken_pieces` for `words` (status, the `words.is_empty()` check) and queues `notify::Utterance { volume, ..notify::Utterance::in_pieces(pieces, voices) }`. Announcements and the voice test keep `Utterance::new`.

- [ ] **Step 4: Run the whole suite**

Run: `cargo test --workspace && npx vitest run`
Expected: all pass (a known-flaky CLI or Windows test may need one rerun; see memory).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/listener.rs
git commit -m "feat: speak long replies in pieces"
```

- [ ] **Step 6: Bump and release notes.** `sh scripts/set-version.sh 0.49.0`; check `git diff --stat` shows `Cargo.lock` changed. Add at the top of `RELEASE_NOTES.md`:

```markdown
# 0.49.0

## New

- Maya speaks a long reply one paragraph at a time, and splits a very long paragraph at the end of a sentence. A local voice server no longer garbles the end of a long reply.
- While one part plays, Maya already makes the next, so the pauses between parts stay short.
- If the voice fails partway through a reply, Maya speaks the rest with the next voice in your list.
```

```bash
git add RELEASE_NOTES.md package.json src-tauri/tauri.conf.json mobile/tauri.conf.json Cargo.toml Cargo.lock
git commit -m "chore(release): 0.49.0"
```

(Stage exactly the files `set-version.sh` changed; check with `git status` first.)

- [ ] **Step 7: Hand check owed.** On this Mac with the local voice server, ask Maya something whose answer is several paragraphs; confirm each piece plays, the gaps are short, and she does not hear herself between pieces. This needs the user; note it in the PR as unticked.
