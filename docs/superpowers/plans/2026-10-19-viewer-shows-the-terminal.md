# The viewer shows what the terminal shows — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The session viewer shows, in both looks, what the agent's terminal shows: each tool call as `Name(arg)` with its output (a 4-line preview that expands), edits as a coloured line diff with `+a −b` counts, failed calls in red, a `✻ Working… (12m 4s)` line while the session works, and a status line (model · folder · branch · PR · ctx %).

**Architecture:** `core/src/transcript.rs`'s `Turn` gains optional tool details (`output`, `output_lines`, `is_error`, `diff`, `added`, `removed`) that travel on the existing `WireTurn` as `#[serde(default)]` fields, so older Mayas and phones ignore them. A new `core/src/diff.rs` holds the line type and a hand-written line diff (LCS, size-capped) plus a reader for the patch Claude Code records. The Claude Code, Codex and OpenCode readers pair each call with its result; a history reply to another machine goes through `transcript::for_remote` (40 lines). On the page, a new `src/toolturn.ts` draws one DOM per tool call that the stylesheet shows as a terminal call or a chat row; `src/modal.ts` adds the working line and the status line, whose branch comes from a new `Card.branch` filled by the PR-badge poll.

**Tech Stack:** Rust (Tauri 2, serde, serde_json), vanilla TypeScript (vitest, jsdom).

**Spec:** `docs/superpowers/specs/2026-10-19-resume-sessions-and-terminal-theme-design.md`, section "Addendum (2026-10-19): the viewer shows what the terminal shows" (the rest of that spec is already implemented on this branch).

## Global Constraints

- Branch `feat/resume-sessions-and-terminal-theme` (already checked out). Never commit to or push `main`; never push without asking first.
- Commit messages: Conventional Commits, plain `type: description` (no work-item codes), ending with a `Co-Authored-By:` trailer naming the model that implements the task, e.g. `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- No version bump: 0.25.0 is already set on this branch (`chore(release): 0.25.0`); these `feat`s ship in it.
- No new crates and no new npm packages. The line diff is written by hand: an LCS over lines after trimming the common head and tail, falling back to "all removed, then all added" above `MAX_DIFF_CELLS = 1_000_000` old×new line pairs.
- Unix-only tests are gated with `#[cfg(unix)]`; paths are compared as `Path`/`PathBuf` (by component), never as strings (Windows CI). (No test in this plan touches the file system by path string; keep it that way.)
- Checks, from the repo root: `cargo test --workspace`, `pnpm test`, `pnpm exec tsc --noEmit`. `pnpm ear:build` must have run once (Tauri bundles the sidecar).
- Constants: `TOOL_OUTPUT_LINES = 200` (output lines, and diff lines, a turn keeps), `REMOTE_OUTPUT_LINES = 40` (in a history reply to another machine), `PREVIEW_LINES = 4` (output lines shown before "… +N lines"). `MAX_TOOL_TURNS = 200` is unchanged.
- Exact strings: a tool turn's text is `Name(arg)` (e.g. `Bash(cargo test)`, replacing `Bash: cargo test`); terminal look: the existing `⏺` marker, `⎿`, `… +N lines` (U+2026), an edit as `Update(<path>)` with `⎿ Updated <path> (+a −b)` (U+2212 minus); chat look: `▸ Bash · cmd`, `▸ Update · path (+a −b)`, a red ` ✕` when the call failed; both looks: `✻ Working… (12m 4s)` (also `4s`, `1h 2m`); status line `model · folder · branch · PR · ctx %` (PR as `PR #12` / `MR !12`, ctx as `ctx 42%`), under the composer in the terminal look, and `model · branch` in the chat look's header beside ctx %.
- Wire: `Turn` gains `output`, `outputLines`, `isError`, `diff` (`[{op: "add"|"del"|"same", line?, text}]`), `added`, `removed`; `Card` gains `branch`. All `#[serde(default)]` and left out when empty: an older Maya or phone ignores them, an older assistant sends none.
- Text from a transcript reaches the page only through `textContent` (never `innerHTML`).
- The terminal look's colours, font and panel chrome, and the Model/Effort pickers, do not change; the chat look changes only where the spec says (tool rows, the working line, the header's model · branch). The agent's permission mode and token counts are not shown (Maya does not know them).

## Review Focus

1. A tool's result lands after its call was already drawn (same `text`, now with `output`): the open panel repaints with the result instead of keeping the bare call (Task 4 `sees a tool result that landed after its call, and a changed diff`).
2. A working session repaints about once a second: an expanded "… +N lines" and an opened chat row stay open across repaints, and a newly opened panel starts collapsed (Task 4 `counts lines the cap left out, and keeps an expanded result open across repaints`, `gives the chat look a compact row that opens the output or diff`).
3. Tool output or a diff line holding markup (`<img src=x onerror=…>`, `<script>`) shows as literal text, never as elements (Task 4 `renders output and diff text as text, never markup`).
4. A huge edit (a reformatted 5,000-line file) does not stall the once-a-second history read: above `MAX_DIFF_CELLS` the diff is all removed then all added, kept to 200 lines with the true counts (Task 1 `a_huge_edit_falls_back_to_all_removed_then_all_added`, `a_diff_keeps_200_lines_and_counts_them_all`).
5. The 1 MB tail begins mid-session or mid-call: a result whose call fell before the window, a call still running, a result holding an image block, Windows line ends in the output — no panic; the orphan result is ignored, the running call shows bare, the image is skipped, each line counts once (Task 2 `a_cut_off_tail_a_running_call_and_odd_results_do_no_harm`).

---

## File Structure

| File | Responsibility |
|---|---|
| `core/src/diff.rs` (new) | `DiffOp`, `DiffLine`, `line_diff` (the hand-written LCS), `from_patch` (Claude Code's `structuredPatch`), `numbered_additions`, `counts`, `MAX_DIFF_CELLS`. |
| `core/src/lib.rs` | `pub mod diff;` |
| `core/src/transcript.rs` | `Turn`'s tool fields, `Turn::new`/`set_output`/`set_diff`, `WireTurn`, `TOOL_OUTPUT_LINES`, `REMOTE_OUTPUT_LINES`, `cap_lines`, `for_remote`; Claude Code: `Name(arg)`, results paired by `tool_use_id`, edit diffs. |
| `core/src/codex.rs`, `core/src/opencode.rs` | Their tool calls as `name(arg)` with outputs. |
| `core/src/grok.rs`, `core/src/kiro.rs`, `core/src/antigravity.rs` | Only `Turn { … }` → `Turn::new(…)`; they render as before. |
| `core/src/actions.rs` | `remote_history` (a history reply for another machine). |
| `cli/src/executor.rs`, `src-tauri/src/net_app.rs` | Answer `CommandKind::History` with `remote_history`. |
| `core/src/pr.rs`, `core/src/store.rs`, `core/src/model.rs` | `PrCache` branches, `Store::set_branch`, `Card.branch`; a Claude Code card's `model` from its transcript. |
| `src-tauri/src/lib.rs` | The PR poll also records each directory's branch. |
| `src/types.ts` | `DiffLine`, `Turn`'s new fields, `Card.branch`. |
| `src/toolturn.ts` (new) | One tool call for the viewer, drawn for both looks: row, call, output preview and expand, diff; the open/expanded state. |
| `src/modal.ts` | Uses `renderToolTurn`; `sameTurns` compares the new fields; working line; status lines; `patchModal` carries them. |
| `src/styles.css` | Tool rows, results, diff colours, working line, status lines, in both looks. |
| `docs/DEVELOPING.md` | The layout names `diff.rs` and `toolturn.ts`. |

---

### Task 1: Tool turns carry output, diffs and counts

**Files:**
- Create: `core/src/diff.rs`
- Modify: `core/src/lib.rs` (add `pub mod diff;` after `pub mod context;`)
- Modify: `core/src/transcript.rs:99-128` (Turn, WireTurn), plus new constants and functions, tests
- Modify: `core/src/opencode.rs`, `core/src/grok.rs`, `core/src/codex.rs`, `core/src/kiro.rs`, `core/src/antigravity.rs`, `core/src/actions.rs` (every `Turn { kind, text }` literal becomes `Turn::new(kind, text)`)
- Modify: `docs/DEVELOPING.md`

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `crate::diff`: `pub const MAX_DIFF_CELLS: usize = 1_000_000;` `pub enum DiffOp { Add, Del, Same }` (serde lowercase); `pub struct DiffLine { pub op: DiffOp, pub line: Option<u32>, pub text: String }` with `DiffLine::new(op: DiffOp, line: Option<u32>, text: &str) -> DiffLine`; `pub fn line_diff(old: &str, new: &str) -> Vec<DiffLine>` (no line numbers); `pub fn from_patch(hunks: &serde_json::Value) -> Option<Vec<DiffLine>>`; `pub fn numbered_additions(text: &str) -> Vec<DiffLine>`; `pub fn counts(diff: &[DiffLine]) -> (u32, u32)` (added, removed).
  - `crate::transcript`: `pub const TOOL_OUTPUT_LINES: usize = 200; pub const REMOTE_OUTPUT_LINES: usize = 40;` `Turn { kind, text, output: Option<String>, output_lines: u32, is_error: bool, diff: Option<Vec<DiffLine>>, added: u32, removed: u32 }` (all `pub`); `Turn::new(kind: TurnKind, text: String) -> Turn`; `Turn::set_output(&mut self, text: &str)`; `Turn::set_diff(&mut self, diff: Vec<DiffLine>)`; `pub fn cap_lines(text: &str, max: usize) -> (String, u32)`; `pub fn for_remote(turns: Vec<Turn>) -> Vec<Turn>`.
  - JSON (to the page and other Mayas): `{"kind","text","notice"?,"output"?,"outputLines"?,"isError"?,"diff"?,"added"?,"removed"?}`, each optional field left out when empty.

Decided here (the spec leaves them open): the page needs to say "… +N lines" truthfully after a cap, so a turn also carries `output_lines`, the whole result's line count; a diff needs no such field, because `added + removed` minus the kept `+`/`−` lines gives what was left out. `REMOTE_OUTPUT_LINES` cuts diffs as well as output.

- [ ] **Step 1: Write the failing tests**

Add `pub mod diff;` to `core/src/lib.rs` after `pub mod context;`. Create `core/src/diff.rs` with only its tests for now (Step 3 adds the code above them):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn ops(d: &[DiffLine]) -> Vec<(DiffOp, Option<u32>, &str)> {
        d.iter().map(|l| (l.op, l.line, l.text.as_str())).collect()
    }

    #[test]
    fn an_edit_keeps_common_lines_and_marks_the_rest() {
        let d = line_diff("a\nb\nc", "a\nB\nc\nd");
        assert_eq!(ops(&d), vec![(DiffOp::Same, None, "a"), (DiffOp::Del, None, "b"), (DiffOp::Add, None, "B"), (DiffOp::Same, None, "c"), (DiffOp::Add, None, "d")]);
        assert_eq!(counts(&d), (2, 1));
    }

    #[test]
    fn identical_empty_and_one_sided_texts() {
        assert_eq!(ops(&line_diff("x\ny", "x\ny")), vec![(DiffOp::Same, None, "x"), (DiffOp::Same, None, "y")]);
        assert_eq!(ops(&line_diff("", "a\nb")), vec![(DiffOp::Add, None, "a"), (DiffOp::Add, None, "b")]);
        assert_eq!(ops(&line_diff("a", "")), vec![(DiffOp::Del, None, "a")]);
        assert!(line_diff("", "").is_empty());
    }

    #[test]
    fn a_moved_line_is_matched_by_the_longest_common_run() {
        // b and c stay; a moves from the top to the bottom.
        let d = line_diff("a\nb\nc", "b\nc\na");
        assert_eq!(ops(&d), vec![(DiffOp::Del, None, "a"), (DiffOp::Same, None, "b"), (DiffOp::Same, None, "c"), (DiffOp::Add, None, "a")]);
        assert_eq!(counts(&d), (1, 1));
    }

    #[test]
    fn a_huge_edit_falls_back_to_all_removed_then_all_added() {
        let old = (0..5000).map(|i| format!("o{i}")).collect::<Vec<_>>().join("\n");
        let new = (0..5000).map(|i| format!("n{i}")).collect::<Vec<_>>().join("\n");
        let d = line_diff(&old, &new);
        assert_eq!(d.len(), 10_000);
        assert!(d[..5000].iter().all(|l| l.op == DiffOp::Del));
        assert!(d[5000..].iter().all(|l| l.op == DiffOp::Add));
        assert_eq!(counts(&d), (5000, 5000));
    }

    #[test]
    fn a_recorded_patch_numbers_its_lines() {
        let patch = serde_json::json!([
            {"oldStart": 3, "oldLines": 3, "newStart": 3, "newLines": 4, "lines": ["-fn mian() {", "+fn main() {", "     run();", "+    done();", " }", "\\ No newline at end of file"]},
            {"oldStart": 40, "oldLines": 1, "newStart": 41, "newLines": 0, "lines": ["-gone"]},
        ]);
        assert_eq!(ops(&from_patch(&patch).unwrap()), vec![
            (DiffOp::Del, Some(3), "fn mian() {"),
            (DiffOp::Add, Some(3), "fn main() {"),
            (DiffOp::Same, Some(4), "    run();"),
            (DiffOp::Add, Some(5), "    done();"),
            (DiffOp::Same, Some(6), "}"),
            (DiffOp::Del, Some(40), "gone"),
        ]);
        assert_eq!(from_patch(&serde_json::json!([])), None, "a new file's patch is empty");
        assert_eq!(from_patch(&serde_json::Value::Null), None);
    }

    #[test]
    fn a_new_file_is_every_line_added_from_one() {
        assert_eq!(ops(&numbered_additions("# Notes\n\nfirst\n")), vec![(DiffOp::Add, Some(1), "# Notes"), (DiffOp::Add, Some(2), ""), (DiffOp::Add, Some(3), "first")]);
    }
}
```

In `core/src/transcript.rs`'s `mod tests`, add `use crate::diff::{DiffLine, DiffOp};` under the existing `use` lines, and append:

```rust
    fn numbered(n: usize) -> String {
        (1..=n).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn a_tool_turn_carries_its_output_diff_and_counts_on_the_wire() {
        let mut edit = Turn::new(TurnKind::Tool, "Edit(/x/a.rs)".into());
        edit.set_diff(vec![DiffLine::new(DiffOp::Del, Some(3), "b"), DiffLine::new(DiffOp::Add, Some(3), "c")]);
        let mut bash = Turn::new(TurnKind::Tool, "Bash(false)".into());
        bash.set_output("boom\n");
        bash.is_error = true;
        let json = serde_json::to_value(vec![edit.clone(), bash.clone()]).unwrap();
        assert_eq!(json, serde_json::json!([
            {"kind": "tool", "text": "Edit(/x/a.rs)", "diff": [{"op": "del", "line": 3, "text": "b"}, {"op": "add", "line": 3, "text": "c"}], "added": 1, "removed": 1},
            {"kind": "tool", "text": "Bash(false)", "output": "boom", "outputLines": 1, "isError": true},
        ]));
        let back: Vec<Turn> = serde_json::from_value(json).unwrap();
        assert_eq!(back, vec![edit, bash]);
    }

    #[test]
    fn an_older_peers_turns_read_without_tool_details_and_ours_read_on_an_older_peer() {
        let old: Vec<Turn> = serde_json::from_value(serde_json::json!([{"kind": "tool", "text": "Bash: ls"}])).unwrap();
        assert_eq!(old, vec![Turn::new(TurnKind::Tool, "Bash: ls".into())]);
        // 0.24's Turn: kind and text only, unknown fields ignored.
        #[derive(serde::Deserialize, Debug, PartialEq)]
        struct OldTurn { kind: String, text: String }
        let mut t = Turn::new(TurnKind::Tool, "Bash(ls)".into());
        t.set_output("a.rs");
        let seen: Vec<OldTurn> = serde_json::from_value(serde_json::to_value(vec![t]).unwrap()).unwrap();
        assert_eq!(seen, vec![OldTurn { kind: "tool".into(), text: "Bash(ls)".into() }]);
    }

    #[test]
    fn output_keeps_its_first_200_lines_and_the_whole_count() {
        let mut t = Turn::new(TurnKind::Tool, "Bash(seq 250)".into());
        t.set_output(&format!("{}\n\n", numbered(250)));
        assert_eq!(t.output.as_deref(), Some(numbered(TOOL_OUTPUT_LINES).as_str()));
        assert_eq!(t.output_lines, 250);
        t.set_output("  \n\n");
        assert_eq!((t.output.as_deref(), t.output_lines), (None, 0), "a blank result is none");
        t.set_output("a\r\nb\r\n");
        assert_eq!((t.output.as_deref(), t.output_lines), (Some("a\nb"), 2), "Windows line ends count once");
    }

    #[test]
    fn a_diff_keeps_200_lines_and_counts_them_all() {
        let mut t = Turn::new(TurnKind::Tool, "Write(/x/big.txt)".into());
        t.set_diff(crate::diff::numbered_additions(&numbered(300)));
        assert_eq!(t.diff.as_ref().unwrap().len(), TOOL_OUTPUT_LINES);
        assert_eq!((t.added, t.removed), (300, 0));
    }

    #[test]
    fn a_history_for_another_machine_carries_40_lines_and_the_whole_counts() {
        let mut out = Turn::new(TurnKind::Tool, "Bash(seq 250)".into());
        out.set_output(&numbered(250));
        let mut edit = Turn::new(TurnKind::Tool, "Write(/x/big.txt)".into());
        edit.set_diff(crate::diff::numbered_additions(&numbered(100)));
        let plain = Turn::new(TurnKind::User, "hi".into());
        let sent = for_remote(vec![out, edit, plain.clone()]);
        assert_eq!(sent[0].output.as_deref(), Some(numbered(REMOTE_OUTPUT_LINES).as_str()));
        assert_eq!(sent[0].output_lines, 250, "the page still says how many lines there were");
        assert_eq!(sent[1].diff.as_ref().unwrap().len(), REMOTE_OUTPUT_LINES);
        assert_eq!((sent[1].added, sent[1].removed), (100, 0));
        assert_eq!(sent[2], plain);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p maya-core diff::`
Expected: FAIL to compile — `cannot find function line_diff` (and `DiffLine`, `Turn::new`, `set_output`, … in `transcript`).

- [ ] **Step 3: Write the implementation**

Put this at the top of `core/src/diff.rs`, above the tests:

```rust
//! Line diffs for the session viewer: an edit's old and new text, or the
//! patch Claude Code recorded, as added, removed and unchanged lines.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Above this many old×new line pairs the diff stops matching lines and
/// shows every old line removed, then every new one added: the table would
/// cost too much for a history read that runs every second.
pub const MAX_DIFF_CELLS: usize = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffOp {
    Add,
    Del,
    Same,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLine {
    pub op: DiffOp,
    /// Its number in the new file (the old one for a removed line), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    pub text: String,
}

impl DiffLine {
    pub fn new(op: DiffOp, line: Option<u32>, text: &str) -> DiffLine {
        DiffLine { op, line, text: text.to_string() }
    }
}

/// (added, removed) lines.
pub fn counts(diff: &[DiffLine]) -> (u32, u32) {
    diff.iter().fold((0, 0), |(a, r), l| match l.op {
        DiffOp::Add => (a + 1, r),
        DiffOp::Del => (a, r + 1),
        DiffOp::Same => (a, r),
    })
}

/// `old` against `new` line by line: common lines kept, the rest removed
/// then added. No line numbers: an edit's strings do not say where in the
/// file they sit.
pub fn line_diff(old: &str, new: &str) -> Vec<DiffLine> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    // Most edits change a few lines in the middle: match the ends first.
    let head = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let tail = a[head..].iter().rev().zip(b[head..].iter().rev()).take_while(|(x, y)| x == y).count();
    let same = |l: &&str| DiffLine::new(DiffOp::Same, None, l);
    let mut out: Vec<DiffLine> = a[..head].iter().map(same).collect();
    out.extend(middle(&a[head..a.len() - tail], &b[head..b.len() - tail]));
    out.extend(a[a.len() - tail..].iter().map(same));
    out
}

/// The changed middle: a longest-common-subsequence walk, or every line
/// removed then added when either side is empty or the table is too big.
fn middle(a: &[&str], b: &[&str]) -> Vec<DiffLine> {
    let (n, m) = (a.len(), b.len());
    if n == 0 || m == 0 || n.saturating_mul(m) > MAX_DIFF_CELLS {
        return a.iter().map(|l| DiffLine::new(DiffOp::Del, None, l)).chain(b.iter().map(|l| DiffLine::new(DiffOp::Add, None, l))).collect();
    }
    // lcs[i * (m + 1) + j]: the longest common subsequence of a[i..] and b[j..].
    let at = |i: usize, j: usize| i * (m + 1) + j;
    let mut lcs = vec![0u32; (n + 1) * (m + 1)];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[at(i, j)] = if a[i] == b[j] { lcs[at(i + 1, j + 1)] + 1 } else { lcs[at(i + 1, j)].max(lcs[at(i, j + 1)]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::with_capacity(n + m);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push(DiffLine::new(DiffOp::Same, None, a[i]));
            i += 1;
            j += 1;
        } else if lcs[at(i + 1, j)] >= lcs[at(i, j + 1)] {
            out.push(DiffLine::new(DiffOp::Del, None, a[i]));
            i += 1;
        } else {
            out.push(DiffLine::new(DiffOp::Add, None, b[j]));
            j += 1;
        }
    }
    out.extend(a[i..].iter().map(|l| DiffLine::new(DiffOp::Del, None, l)));
    out.extend(b[j..].iter().map(|l| DiffLine::new(DiffOp::Add, None, l)));
    out
}

/// Claude Code's `structuredPatch` (hunks of `oldStart`, `newStart` and
/// `lines` led by ' ', '-' or '+') as numbered lines: a removed line carries
/// its old number, the others their new one. None unless it is a non-empty
/// list of hunks.
pub fn from_patch(hunks: &Value) -> Option<Vec<DiffLine>> {
    let hunks = hunks.as_array().filter(|h| !h.is_empty())?;
    let mut out = Vec::new();
    for h in hunks {
        let mut old = h["oldStart"].as_u64().unwrap_or(1) as u32;
        let mut new = h["newStart"].as_u64().unwrap_or(1) as u32;
        for l in h["lines"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            // Anything else ("\ No newline at end of file") is not a line of the file.
            let op = match l.chars().next() {
                Some('+') => DiffOp::Add,
                Some('-') => DiffOp::Del,
                Some(' ') => DiffOp::Same,
                _ => continue,
            };
            let text = &l[1..];
            match op {
                DiffOp::Del => {
                    out.push(DiffLine::new(op, Some(old), text));
                    old += 1;
                }
                DiffOp::Add => {
                    out.push(DiffLine::new(op, Some(new), text));
                    new += 1;
                }
                DiffOp::Same => {
                    out.push(DiffLine::new(op, Some(new), text));
                    old += 1;
                    new += 1;
                }
            }
        }
    }
    Some(out)
}

/// Every line of `text` added, numbered from 1: a new file.
pub fn numbered_additions(text: &str) -> Vec<DiffLine> {
    text.lines().enumerate().map(|(i, l)| DiffLine::new(DiffOp::Add, Some(i as u32 + 1), l)).collect()
}
```

In `core/src/transcript.rs`, add `use crate::diff::DiffLine;` after `use crate::model::{parse_questions, AwaitKind, Question};`, and replace the block from `#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]` above `pub struct Turn` through the end of `impl From<WireTurn> for Turn { … }` (lines 99–128) with:

```rust
/// Most lines of a tool's output, or of an edit's diff, a turn keeps.
pub const TOOL_OUTPUT_LINES: usize = 200;
/// Most of those lines a history reply to another machine carries.
pub const REMOTE_OUTPUT_LINES: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(into = "WireTurn", from = "WireTurn")]
pub struct Turn {
    pub kind: TurnKind,
    pub text: String,
    /// A tool call's result text, at most `TOOL_OUTPUT_LINES` lines.
    pub output: Option<String>,
    /// Lines in the whole result before any cap; 0 without output.
    pub output_lines: u32,
    /// The tool call failed.
    pub is_error: bool,
    /// An edit's lines, at most `TOOL_OUTPUT_LINES`.
    pub diff: Option<Vec<DiffLine>>,
    /// Lines the whole edit adds and removes.
    pub added: u32,
    pub removed: u32,
}

impl Turn {
    /// A turn with no tool details.
    pub fn new(kind: TurnKind, text: String) -> Turn {
        Turn { kind, text, output: None, output_lines: 0, is_error: false, diff: None, added: 0, removed: 0 }
    }

    /// The result text: its first `TOOL_OUTPUT_LINES` lines and the whole
    /// count. A blank result is none.
    pub fn set_output(&mut self, text: &str) {
        let (kept, total) = cap_lines(text, TOOL_OUTPUT_LINES);
        self.output_lines = if kept.is_empty() { 0 } else { total };
        self.output = (!kept.is_empty()).then_some(kept);
    }

    /// The edit's diff: every line counted, the first `TOOL_OUTPUT_LINES` kept.
    pub fn set_diff(&mut self, mut diff: Vec<DiffLine>) {
        (self.added, self.removed) = crate::diff::counts(&diff);
        diff.truncate(TOOL_OUTPUT_LINES);
        self.diff = Some(diff);
    }
}

/// The first `max` lines of `text` (trailing blank space dropped) and how
/// many lines it had in all.
pub fn cap_lines(text: &str, max: usize) -> (String, u32) {
    let lines: Vec<&str> = text.trim_end().lines().collect();
    (lines[..lines.len().min(max)].join("\n"), lines.len() as u32)
}

/// Turns as a history reply to another machine carries them: output and
/// diffs cut to `REMOTE_OUTPUT_LINES` lines, the whole output's line count
/// and the edit's counts kept so the page can say what was left out.
pub fn for_remote(turns: Vec<Turn>) -> Vec<Turn> {
    turns
        .into_iter()
        .map(|mut t| {
            if let Some(out) = t.output.take() {
                let (kept, total) = cap_lines(&out, REMOTE_OUTPUT_LINES);
                t.output_lines = t.output_lines.max(total);
                t.output = Some(kept);
            }
            if let Some(d) = t.diff.as_mut() {
                d.truncate(REMOTE_OUTPUT_LINES);
            }
            t
        })
        .collect()
}

/// A turn as it travels to the page and to other Mayas. A notice goes as a
/// tool line marked `notice`, so a Maya from before notices (0.14) still
/// reads the history and shows it as a plain line. The tool details are left
/// out when empty; an older Maya ignores them and an older one's turns
/// arrive without them.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireTurn {
    kind: TurnKind,
    text: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    notice: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    output_lines: u32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    is_error: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    diff: Option<Vec<DiffLine>>,
    #[serde(default, skip_serializing_if = "is_zero")]
    added: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    removed: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl From<Turn> for WireTurn {
    fn from(t: Turn) -> Self {
        let notice = t.kind == TurnKind::Notice;
        WireTurn { kind: if notice { TurnKind::Tool } else { t.kind }, text: t.text, notice, output: t.output, output_lines: t.output_lines, is_error: t.is_error, diff: t.diff, added: t.added, removed: t.removed }
    }
}

impl From<WireTurn> for Turn {
    fn from(w: WireTurn) -> Self {
        Turn { kind: if w.notice { TurnKind::Notice } else { w.kind }, text: w.text, output: w.output, output_lines: w.output_lines, is_error: w.is_error, diff: w.diff, added: w.added, removed: w.removed }
    }
}
```

Then turn every other `Turn { kind…, text… }` literal into `Turn::new(…)` (the `text` argument is a `String`, so `"x".into()` still infers):

```bash
perl -pi -e 's/(?<![A-Za-z])Turn \{ kind: ([^,{}]+), text: (.+?) \}/Turn::new($1, $2)/g; s/(?<![A-Za-z])Turn \{ kind, text: (.+?) \}/Turn::new(kind, $1)/g; s/(?<![A-Za-z])Turn \{ kind: ([^,{}]+), text \}/Turn::new($1, text)/g' core/src/transcript.rs core/src/opencode.rs core/src/grok.rs core/src/codex.rs core/src/kiro.rs core/src/antigravity.rs core/src/actions.rs
```

The lookbehind leaves `WireTurn {`, `OldTurn {` and `VoiceTurn {` alone. If the build then reports `missing fields output, output_lines …` anywhere, change that literal to `Turn::new(kind, text)` by hand.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p maya-core diff::` then `cargo test -p maya-core transcript::`
Expected: PASS (the existing `a_notice_reaches_an_older_maya_as_a_plain_line_it_can_read` still passes: empty tool fields are left out of the JSON).

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Name the new module in the layout**

In `docs/DEVELOPING.md`, replace

```
  `interpreter.rs`, `log.rs` (the Debug tab and `maya.log`),
```

with

```
  `interpreter.rs`, `log.rs` (the Debug tab and `maya.log`),
  `transcript.rs` and `diff.rs` (a session's turns for the viewer: tool output, edit diffs),
```

- [ ] **Step 6: Commit**

```bash
git add core/src/diff.rs core/src/lib.rs core/src/transcript.rs core/src/opencode.rs core/src/grok.rs core/src/codex.rs core/src/kiro.rs core/src/antigravity.rs core/src/actions.rs docs/DEVELOPING.md
git commit -m "feat: tool turns carry their output, edit diffs and counts" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Claude Code's tool calls with their results and diffs

**Files:**
- Create: `core/fixtures/transcript/tools.jsonl`
- Modify: `core/src/transcript.rs` (`tool_summary`, `parse_turns`, new `edit_diff`, `attach_results`, `attach_result`, `result_text`; tests)

**Interfaces:**
- Consumes (Task 1): `Turn::new`, `Turn::set_output`, `Turn::set_diff`, `crate::diff::{line_diff, from_patch, numbered_additions, DiffLine, DiffOp}`, `TOOL_OUTPUT_LINES`.
- Produces: `pub fn tool_summary(name: &str, input: &Value) -> String` now returns `Name(arg)`; `parse_turns(text, max_turns)` returns tool turns with `output`/`output_lines`/`is_error` from the matching `tool_result` and, for Edit/MultiEdit/Write, `diff`/`added`/`removed`.

Decided here: the diff first comes from the call's input (Edit/MultiEdit: old against new strings, no line numbers; Write: every line added, numbered from 1). When the result arrives and its line's `toolUseResult` carries a `structuredPatch` (or `type: "create"` for a new file), that replaces it, with line numbers. An edit shows its diff, not Claude Code's "has been updated" note; a failed edit shows its error and no diff. `toolUseResult` is used only when its line holds one `tool_result`.

- [ ] **Step 1: Write the fixture and the failing tests**

Create `core/fixtures/transcript/tools.jsonl`:

```
{"type":"user","message":{"role":"user","content":"run the tests and fix the typo"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"b1","name":"Bash","input":{"command":"cargo test","description":"Run tests"}}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"b1","content":"running 3 tests\ntest a ... ok\ntest b ... ok\ntest c ... ok\n\ntest result: ok. 3 passed","is_error":false}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"b2","name":"Bash","input":{"command":"ls /nope"}}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"b2","content":[{"type":"text","text":"Exit code 1\nls: /nope: No such file or directory"}],"is_error":true}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"e1","name":"Edit","input":{"file_path":"/x/src/a.rs","old_string":"fn mian() {\n    run();\n}","new_string":"fn main() {\n    run();\n    done();\n}"}}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"e1","content":"The file /x/src/a.rs has been updated."}]},"toolUseResult":{"filePath":"/x/src/a.rs","structuredPatch":[{"oldStart":3,"oldLines":3,"newStart":3,"newLines":4,"lines":["-fn mian() {","+fn main() {","     run();","+    done();"," }"]}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"w1","name":"Write","input":{"file_path":"/x/NOTES.md","content":"# Notes\n\nfirst\n"}}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"w1","content":"File created successfully at: /x/NOTES.md"}]},"toolUseResult":{"type":"create","filePath":"/x/NOTES.md","content":"# Notes\n\nfirst\n","structuredPatch":[]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Fixed the typo; tests pass."}]}}
```

In `core/src/transcript.rs`'s `mod tests`, replace the body of `tool_summary_prefers_command_then_paths_then_question` with:

```rust
        use serde_json::json;
        assert_eq!(tool_summary("Bash", &json!({"command": "ls", "description": "d"})), "Bash(ls)");
        assert_eq!(tool_summary("Edit", &json!({"file_path": "/a.rs"})), "Edit(/a.rs)");
        assert_eq!(tool_summary("Agent", &json!({"prompt": "x".repeat(300)})).chars().count(), "Agent()".len() + 120);
        assert_eq!(tool_summary("AskUserQuestion", &json!({"questions": [{"question": "Which?"}]})), "AskUserQuestion(Which?)");
        assert_eq!(tool_summary("ListAgents", &json!({})), "ListAgents");
```

Replace the test `parses_turns_in_order_skipping_tool_results_and_sidechains` with:

```rust
    #[test]
    fn parses_turns_in_order_with_results_on_their_calls_and_no_sidechains() {
        let turns = parse_turns(&fixture("turns.jsonl"), 30);
        let got: Vec<(TurnKind, &str, Option<&str>)> = turns.iter().map(|t| (t.kind, t.text.as_str(), t.output.as_deref())).collect();
        assert_eq!(got, vec![
            (TurnKind::User, "build me a board", None),
            (TurnKind::Tool, "Bash(pnpm test)", Some("ok")),
            (TurnKind::Tool, "ListAgents", Some("none")),
            (TurnKind::Assistant, "Tests pass.\n\nAnything else?", None),
            (TurnKind::User, "yes, ship it", None),
        ]);
    }
```

Append:

```rust
    #[test]
    fn tool_calls_carry_their_results_errors_and_edit_diffs() {
        let turns = parse_turns(&fixture("tools.jsonl"), 30);
        let kinds: Vec<(TurnKind, &str)> = turns.iter().map(|t| (t.kind, t.text.as_str())).collect();
        assert_eq!(kinds, vec![
            (TurnKind::User, "run the tests and fix the typo"),
            (TurnKind::Tool, "Bash(cargo test)"),
            (TurnKind::Tool, "Bash(ls /nope)"),
            (TurnKind::Tool, "Edit(/x/src/a.rs)"),
            (TurnKind::Tool, "Write(/x/NOTES.md)"),
            (TurnKind::Assistant, "Fixed the typo; tests pass."),
        ]);
        let bash = &turns[1];
        assert_eq!(bash.output.as_deref(), Some("running 3 tests\ntest a ... ok\ntest b ... ok\ntest c ... ok\n\ntest result: ok. 3 passed"));
        assert_eq!((bash.output_lines, bash.is_error), (6, false));
        let failed = &turns[2];
        assert_eq!(failed.output.as_deref(), Some("Exit code 1\nls: /nope: No such file or directory"));
        assert!(failed.is_error);
        let edit = &turns[3];
        assert_eq!(edit.output, None, "an edit shows its diff, not Claude Code's note");
        assert_eq!(edit.diff, Some(vec![
            DiffLine::new(DiffOp::Del, Some(3), "fn mian() {"),
            DiffLine::new(DiffOp::Add, Some(3), "fn main() {"),
            DiffLine::new(DiffOp::Same, Some(4), "    run();"),
            DiffLine::new(DiffOp::Add, Some(5), "    done();"),
            DiffLine::new(DiffOp::Same, Some(6), "}"),
        ]));
        assert_eq!((edit.added, edit.removed), (2, 1));
        let write = &turns[4];
        assert_eq!(write.diff, Some(vec![DiffLine::new(DiffOp::Add, Some(1), "# Notes"), DiffLine::new(DiffOp::Add, Some(2), ""), DiffLine::new(DiffOp::Add, Some(3), "first")]));
        assert_eq!((write.added, write.removed), (3, 0));
    }

    #[test]
    fn an_edit_before_its_result_diffs_its_input_and_a_failed_one_shows_the_error() {
        let edit = serde_json::json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"e1","name":"Edit","input":{"file_path":"/x/a.rs","old_string":"a\nb","new_string":"a\nc"}}]}}).to_string();
        let t = &parse_turns(&edit, 30)[0];
        assert_eq!(t.diff, Some(vec![DiffLine::new(DiffOp::Same, None, "a"), DiffLine::new(DiffOp::Del, None, "b"), DiffLine::new(DiffOp::Add, None, "c")]));
        assert_eq!((t.added, t.removed), (1, 1));
        let failed = serde_json::json!({"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"e1","content":"<tool_use_error>String to replace not found in file.</tool_use_error>","is_error":true}]}}).to_string();
        let t = &parse_turns(&format!("{edit}\n{failed}"), 30)[0];
        assert!(t.is_error);
        assert_eq!(t.diff, None);
        assert_eq!((t.added, t.removed), (0, 0));
        assert_eq!(t.output.as_deref(), Some("<tool_use_error>String to replace not found in file.</tool_use_error>"));
    }

    #[test]
    fn a_multi_edit_joins_the_diffs_of_its_edits() {
        let line = serde_json::json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"m1","name":"MultiEdit","input":{"file_path":"/x/a.rs","edits":[{"old_string":"a","new_string":"b"},{"old_string":"c\nd","new_string":"c"}]}}]}}).to_string();
        let t = &parse_turns(&line, 30)[0];
        assert_eq!(t.text, "MultiEdit(/x/a.rs)");
        assert_eq!(t.diff.as_ref().unwrap().len(), 4);
        assert_eq!((t.added, t.removed), (1, 2));
    }

    #[test]
    fn a_long_result_keeps_its_first_200_lines_and_the_whole_count() {
        let out = (1..=250).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let call = serde_json::json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"b1","name":"Bash","input":{"command":"seq 250"}}]}});
        let result = serde_json::json!({"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"b1","content":out}]}});
        let t = &parse_turns(&format!("{call}\n{result}"), 30)[0];
        assert_eq!(t.output.as_deref().unwrap().lines().count(), TOOL_OUTPUT_LINES);
        assert!(t.output.as_deref().unwrap().ends_with("line 200"));
        assert_eq!(t.output_lines, 250);
    }

    #[test]
    fn a_cut_off_tail_a_running_call_and_odd_results_do_no_harm() {
        let text = [
            // The result of a call made before the tail began.
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"gone","content":"old output"}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"r1","name":"Read","input":{"file_path":"/x/shot.png"}}]}}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"r1","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"iVBORw0"}},{"type":"text","text":"line one\r\nline two\r\n"}]}]}}"#,
            // Still running: no result yet.
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"b9","name":"Bash","input":{"command":"sleep 60"}}]}}"#,
        ]
        .join("\n");
        let turns = parse_turns(&text, 30);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].text, "Read(/x/shot.png)");
        assert_eq!((turns[0].output.as_deref(), turns[0].output_lines), (Some("line one\nline two"), 2));
        assert_eq!(turns[1], Turn::new(TurnKind::Tool, "Bash(sleep 60)".into()));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p maya-core transcript::`
Expected: FAIL — `tool_summary` returns `Bash: ls`, tool turns have no `output` or `diff`.

- [ ] **Step 3: Write the implementation**

In `core/src/transcript.rs`, change `use std::collections::HashSet;` to `use std::collections::{HashMap, HashSet};`, and replace `tool_summary` with:

```rust
/// `Name(first useful argument)` for a tool_use block, as the agent's
/// terminal shows the call, or just the name.
pub fn tool_summary(name: &str, input: &Value) -> String {
    let arg = ["command", "file_path", "path", "prompt", "description"]
        .iter()
        .find_map(|k| input.get(*k).and_then(|v| v.as_str()))
        .or_else(|| input["questions"][0]["question"].as_str());
    match arg {
        Some(a) if !a.trim().is_empty() => format!("{name}({})", crate::state::truncate(a.trim(), 120)),
        _ => name.to_string(),
    }
}

/// The diff an edit tool's input describes: Edit and MultiEdit as old
/// against new text, Write as every line added. None for other tools.
fn edit_diff(name: &str, input: &Value) -> Option<Vec<crate::diff::DiffLine>> {
    match name {
        "Edit" => Some(crate::diff::line_diff(input["old_string"].as_str()?, input["new_string"].as_str()?)),
        "MultiEdit" => Some(
            input["edits"]
                .as_array()?
                .iter()
                .flat_map(|e| crate::diff::line_diff(e["old_string"].as_str().unwrap_or(""), e["new_string"].as_str().unwrap_or("")))
                .collect(),
        ),
        "Write" => Some(crate::diff::numbered_additions(input["content"].as_str()?)),
        _ => None,
    }
}

/// A `tool_result` block's text: a string, or the text blocks of a list
/// (images and other blocks are skipped).
fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// Puts a `tool_result` block's outcome on its call. An edit keeps its diff,
/// numbered from the patch Claude Code recorded (`recorded`, the line's
/// `toolUseResult`) when there is one; a failed call, edit or not, shows its
/// error as output and no diff.
fn attach_result(t: &mut Turn, block: &Value, recorded: Option<&Value>) {
    t.is_error = block["is_error"].as_bool() == Some(true);
    if t.diff.is_some() && !t.is_error {
        let patched = recorded.and_then(|r| crate::diff::from_patch(&r["structuredPatch"]));
        let created = recorded.filter(|r| r["type"].as_str() == Some("create")).and_then(|r| r["content"].as_str()).map(crate::diff::numbered_additions);
        if let Some(d) = patched.or(created) {
            t.set_diff(d);
        }
        return;
    }
    t.diff = None;
    (t.added, t.removed) = (0, 0);
    t.set_output(&result_text(&block["content"]));
}

/// Puts each `tool_result` block of a user line on the call it answers. The
/// line's `toolUseResult` belongs to its result only when it holds one.
fn attach_results(items: &mut [Item], calls: &HashMap<String, usize>, content: &Value, recorded: &Value) {
    let Some(blocks) = content.as_array() else { return };
    let results: Vec<&Value> = blocks.iter().filter(|b| b["type"] == "tool_result").collect();
    let recorded = (results.len() == 1 && recorded.is_object()).then_some(recorded);
    for b in results {
        // A result whose call fell before the tail began has nothing to go on.
        let Some(&i) = b["tool_use_id"].as_str().and_then(|id| calls.get(id)) else { continue };
        if let Some(Item::Turn(t)) = items.get_mut(i) {
            attach_result(t, b, recorded);
        }
    }
}
```

Replace `parse_turns` with:

```rust
/// Conversation turns (user prompts, assistant text, tool calls with their
/// results) in order, keeping the last `max_turns` messages and the tool
/// calls among them. Subagent sidechain lines, harness-injected user lines
/// (skill bodies, notifications, subagent hand-backs) and progress narration
/// are skipped; a compaction or an interrupt is a notice.
pub fn parse_turns(text: &str, max_turns: usize) -> Vec<Turn> {
    let mut items = Vec::new();
    // Where each tool call's turn sits, by its tool_use id, for its result.
    let mut calls: HashMap<String, usize> = HashMap::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if v["isSidechain"].as_bool() == Some(true) {
            continue;
        }
        let content = &v["message"]["content"];
        match v["type"].as_str() {
            Some("user") => {
                attach_results(&mut items, &calls, content, &v["toolUseResult"]);
                if v["isCompactSummary"].as_bool() == Some(true) {
                    items.push(notice("Earlier conversation compacted"));
                    continue;
                }
                let t = text_blocks(content);
                if !t.is_empty() {
                    let text = t.join("\n\n");
                    let meta = v["isMeta"].as_bool() == Some(true);
                    match unwrap_peer_message(&text) {
                        // A subagent's final report comes back framed as a peer message.
                        Some(inner) if inner.starts_with("<agent-message") => {}
                        Some(inner) => items.push(Item::Turn(Turn::new(TurnKind::Peer, inner))),
                        None if text.starts_with(INTERRUPTED) => items.push(notice("Interrupted")),
                        // Other meta lines are skill bodies, reminders and caveats.
                        None if meta => {}
                        None if is_harness_tag(&text) => {
                            if is_user_initiated(&text) {
                                items.push(Item::Break);
                            }
                        }
                        None => {
                            let text = strip_paste_tags(&text);
                            if !text.is_empty() {
                                items.push(Item::Turn(Turn::new(TurnKind::User, text)));
                            }
                        }
                    }
                }
            }
            Some("assistant") => {
                let t = text_blocks(content);
                if !t.is_empty() {
                    items.push(Item::Turn(Turn::new(TurnKind::Assistant, t.join("\n\n"))));
                }
                if let Some(blocks) = content.as_array() {
                    for b in blocks.iter().filter(|b| b["type"] == "tool_use") {
                        let name = b["name"].as_str().unwrap_or("tool");
                        let mut turn = Turn::new(TurnKind::Tool, tool_summary(name, &b["input"]));
                        if let Some(d) = edit_diff(name, &b["input"]) {
                            turn.set_diff(d);
                        }
                        if let Some(id) = b["id"].as_str() {
                            calls.insert(id.to_string(), items.len());
                        }
                        items.push(Item::Turn(turn));
                    }
                }
            }
            _ => {}
        }
    }
    keep_last_messages(drop_narration(items), max_turns)
}
```

(`items` only grows inside the loop, so the indexes in `calls` stay valid until `drop_narration` runs after it.)

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p maya-core transcript::`
Expected: PASS.

Run: `cargo test -p maya-core`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/transcript.rs core/fixtures/transcript/tools.jsonl
git commit -m "feat: read Claude Code's tool results and edit diffs for the viewer" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Codex and OpenCode outputs, and the 40-line cap across machines

**Files:**
- Modify: `core/src/transcript.rs` (`keep_last_messages` becomes `pub(crate)`)
- Modify: `core/src/codex.rs` (`parse_turns`, new `call_line`, `shell_line`, `patch_file`, `call_output`; test)
- Modify: `core/src/opencode.rs` (`tool_line`, new `tool_turn`, `messages`; test)
- Modify: `core/src/actions.rs` (new `remote_history`; test)
- Modify: `cli/src/executor.rs:74`, `src-tauri/src/net_app.rs:433` (answer `History` with `remote_history`)

**Interfaces:**
- Consumes (Task 1): `Turn::new`, `Turn::set_output`, `transcript::for_remote`, `transcript::REMOTE_OUTPUT_LINES`; `transcript::keep_last_messages(turns: Vec<Turn>, max_messages: usize) -> Vec<Turn>` (made `pub(crate)` here).
- Produces: `pub fn actions::remote_history(l: &Local, session_id: &str) -> Result<Vec<transcript::Turn>, String>`; Codex and OpenCode tool turns as `name(arg)` with `output`/`is_error`.

Decided here: Codex rollouts record `function_call`/`custom_tool_call` (with `call_id`) and `function_call_output`/`custom_tool_call_output`; an output is either the older JSON `{"output": …, "metadata": {"exit_code": n}}` or the newer text led by `Exit code: n`, a non-zero code marking it failed. No rollout on this Mac has a tool call, so the test lines follow that format. OpenCode 2.0's server (its OpenAPI, read live) puts a tool's result in `state.content` (text items) and a failure in `state.status: "error"` with `state.error.message`; OpenCode 1's `state.output` is read too. Codex now keeps the last `max_turns` messages with the tool calls among them, as Claude Code does, so tool calls never push messages out. Kiro, Grok and Antigravity are unchanged.

- [ ] **Step 1: Write the failing tests**

Append to `core/src/codex.rs`'s `mod tests`:

```rust
    #[test]
    fn tool_calls_carry_their_output_and_a_failed_exit_code() {
        let text = concat!(
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"list files"}]}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"bash\",\"-lc\",\"ls\"],\"workdir\":\"/x\"}","call_id":"call_1"}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"function_call_output","call_id":"call_1","output":"{\"output\":\"a.rs\\nb.rs\\n\",\"metadata\":{\"exit_code\":0,\"duration_seconds\":0.1}}"}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"cat nope\"}","call_id":"call_2"}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"function_call_output","call_id":"call_2","output":"Exit code: 1\nWall time: 0.1 seconds\nOutput:\ncat: nope: No such file or directory\n"}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"custom_tool_call","name":"apply_patch","input":"*** Begin Patch\n*** Update File: src/a.rs\n@@\n-x\n+y\n*** End Patch","call_id":"call_3"}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"call_3","output":"Success. Updated the following files:\nM src/a.rs\n"}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"function_call_output","call_id":"call_from_before_the_tail","output":"lost"}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Two files."}]}}"#, "\n",
        );
        let turns = parse_turns(text, 30);
        let got: Vec<(TurnKind, &str, Option<&str>, bool)> = turns.iter().map(|t| (t.kind, t.text.as_str(), t.output.as_deref(), t.is_error)).collect();
        assert_eq!(got, vec![
            (TurnKind::User, "list files", None, false),
            (TurnKind::Tool, "shell(ls)", Some("a.rs\nb.rs"), false),
            (TurnKind::Tool, "exec_command(cat nope)", Some("Exit code: 1\nWall time: 0.1 seconds\nOutput:\ncat: nope: No such file or directory"), true),
            (TurnKind::Tool, "apply_patch(src/a.rs)", Some("Success. Updated the following files:\nM src/a.rs"), false),
            (TurnKind::Assistant, "Two files.", None, false),
        ]);
    }

    #[test]
    fn tool_calls_do_not_push_codex_messages_out() {
        let mut text = String::from(r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"go"}]}}"#);
        for i in 0..40 {
            text.push_str(&format!("\n{{\"type\":\"response_item\",\"payload\":{{\"type\":\"function_call\",\"name\":\"shell\",\"arguments\":\"{{\\\"cmd\\\":\\\"echo {i}\\\"}}\",\"call_id\":\"c{i}\"}}}}"));
        }
        let turns = parse_turns(&text, 2);
        assert_eq!(turns.len(), 41, "the one message and every call after it");
        assert_eq!(turns[0].text, "go");
    }
```

Append to `core/src/opencode.rs`'s `mod tests`:

```rust
    #[test]
    fn tool_parts_carry_their_output_or_their_error() {
        let v = serde_json::json!({"data": [
            {"type": "assistant", "content": [
                {"type": "tool", "tool": "bash", "state": {"status": "error", "input": {"command": "ls /nope"}, "error": {"type": "unknown", "message": "ls: /nope: No such file or directory"}}},
                {"type": "tool", "tool": "read", "state": {"status": "completed", "input": {"filePath": "/x/a.rs"}, "content": [{"type": "text", "text": "fn main() {}"}]}},
                {"type": "tool", "tool": "bash", "state": {"status": "completed", "input": {"command": "ls"}, "output": "a.rs\nb.rs\n"}},
                {"type": "tool", "tool": "bash", "state": {"status": "running", "input": {"command": "sleep 9"}}}
            ]},
            {"type": "user", "text": "list"}
        ]});
        let (_, turns) = messages(&v);
        let got: Vec<(&str, Option<&str>, bool)> = turns.iter().map(|t| (t.text.as_str(), t.output.as_deref(), t.is_error)).collect();
        assert_eq!(got, vec![
            ("list", None, false),
            ("bash(ls /nope)", Some("ls: /nope: No such file or directory"), true),
            ("read(/x/a.rs)", Some("fn main() {}"), false),
            ("bash(ls)", Some("a.rs\nb.rs"), false),
            ("bash(sleep 9)", None, false),
        ]);
    }
```

Append to `core/src/actions.rs`'s `mod tests` (next to `a_peer_message_in_the_history_stays_a_peer_turn`):

```rust
    #[test]
    fn a_history_for_another_machine_cuts_tool_output_to_40_lines() {
        let (_d, store) = store_with_session("s1", 4242);
        let s = store.lock().unwrap().session("s1").unwrap();
        let path = store.lock().unwrap().transcript_path_for(&s);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let out = (1..=100).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let call = serde_json::json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"b1","name":"Bash","input":{"command":"seq 100"}}]}});
        let result = serde_json::json!({"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"b1","content":out}]}});
        std::fs::write(&path, format!("{call}\n{result}\n")).unwrap();
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        let here = session_history(&l, "s1").unwrap();
        assert_eq!(here[0].output.as_deref().unwrap().lines().count(), 100);
        let there = remote_history(&l, "s1").unwrap();
        assert_eq!(there[0].output.as_deref().unwrap().lines().count(), transcript::REMOTE_OUTPUT_LINES);
        assert_eq!(there[0].output_lines, 100, "the page still says how many lines there were");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p maya-core codex::` then `cargo test -p maya-core opencode::` then `cargo test -p maya-core actions::tests::a_history_for_another_machine`
Expected: FAIL — Codex has no tool turns, OpenCode's text is `bash: ls /nope` with no output, and `remote_history` does not exist.

- [ ] **Step 3: Write the implementation**

In `core/src/transcript.rs`, change `fn keep_last_messages(` to `pub(crate) fn keep_last_messages(`.

In `core/src/codex.rs`, replace `parse_turns` with:

```rust
/// User and assistant messages and tool calls with their output, newest
/// last, skipping developer prompts and the environment preamble Codex
/// injects as a user message. Tool calls do not count against `max_turns`.
pub fn parse_turns(text: &str, max_turns: usize) -> Vec<Turn> {
    let mut out: Vec<Turn> = Vec::new();
    // Where each call's turn sits, by its call_id, for its output.
    let mut calls: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if v["type"].as_str() != Some("response_item") {
            continue;
        }
        let p = &v["payload"];
        match p["type"].as_str() {
            Some("message") => {
                let kind = match p["role"].as_str() {
                    Some("user") => TurnKind::User,
                    Some("assistant") => TurnKind::Assistant,
                    _ => continue,
                };
                let text: String = p["content"].as_array().map(|c| c.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default();
                let text = text.trim();
                if text.is_empty() || text.starts_with("<environment_context>") {
                    continue;
                }
                out.push(Turn::new(kind, text.to_string()));
            }
            Some("function_call" | "custom_tool_call") => {
                if let Some(id) = p["call_id"].as_str() {
                    calls.insert(id.to_string(), out.len());
                }
                out.push(Turn::new(TurnKind::Tool, call_line(p)));
            }
            Some("function_call_output" | "custom_tool_call_output") => {
                let Some(&i) = p["call_id"].as_str().and_then(|id| calls.get(id)) else { continue };
                let (text, failed) = call_output(p);
                out[i].is_error = failed;
                out[i].set_output(&text);
            }
            _ => {}
        }
    }
    crate::transcript::keep_last_messages(out, max_turns)
}

/// `name(arg)` for a Codex tool call: a shell call's command line, a
/// patch's file, else the input's first line; just the name without one.
fn call_line(p: &Value) -> String {
    let name = p["name"].as_str().unwrap_or("tool");
    let args: Value = p["arguments"].as_str().and_then(|a| serde_json::from_str(a).ok()).unwrap_or(Value::Null);
    let input = p["input"].as_str().unwrap_or("");
    let arg = shell_line(&args["command"])
        .or_else(|| args["cmd"].as_str().map(str::to_string))
        .or_else(|| args["path"].as_str().map(str::to_string))
        .or_else(|| patch_file(input))
        .or_else(|| input.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string));
    match arg.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
        Some(a) => format!("{name}({})", crate::state::truncate(a, 120)),
        None => name.to_string(),
    }
}

/// The command a shell call runs: `["bash", "-lc", "ls"]` is the line `ls`;
/// other argument lists are joined with spaces.
fn shell_line(command: &Value) -> Option<String> {
    match command {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => {
            let parts: Vec<&str> = parts.iter().filter_map(Value::as_str).collect();
            match parts.as_slice() {
                [shell, flag, line] if shell.ends_with("sh") && flag.starts_with('-') && flag.ends_with('c') => Some(line.to_string()),
                _ => Some(parts.join(" ")).filter(|s| !s.is_empty()),
            }
        }
        _ => None,
    }
}

/// The file an `apply_patch` input changes, from its first `*** … File:` line.
fn patch_file(input: &str) -> Option<String> {
    input
        .lines()
        .find_map(|l| ["*** Update File: ", "*** Add File: ", "*** Delete File: "].iter().find_map(|p| l.strip_prefix(*p)))
        .map(|f| f.trim().to_string())
}

/// A tool call's output text and whether it failed. Older rollouts wrap it
/// as `{"output": …, "metadata": {"exit_code": n}}`; newer ones lead with
/// `Exit code: n`.
fn call_output(p: &Value) -> (String, bool) {
    let raw = p["output"].as_str().unwrap_or("");
    if let Ok(v) = serde_json::from_str::<Value>(raw) {
        if let Some(out) = v["output"].as_str() {
            return (out.to_string(), v["metadata"]["exit_code"].as_i64().is_some_and(|c| c != 0));
        }
    }
    let code = raw.lines().next().and_then(|l| l.strip_prefix("Exit code: ")).and_then(|c| c.trim().parse::<i64>().ok());
    (raw.to_string(), code.is_some_and(|c| c != 0))
}
```

In `core/src/opencode.rs`, replace `tool_line` with these two functions, and in `messages` replace `Some("tool") => turns.push(Turn::new(TurnKind::Tool, tool_line(part))),` with `Some("tool") => turns.push(tool_turn(part)),`:

```rust
/// `name(arg)` for a tool part, as the terminal shows the call, or just the name.
fn tool_line(part: &Value) -> String {
    let name = part["tool"].as_str().unwrap_or("tool");
    let input = &part["state"]["input"];
    match ["command", "filePath", "path", "pattern"].iter().find_map(|k| input[k].as_str()).map(str::trim).filter(|a| !a.is_empty()) {
        Some(a) => format!("{name}({})", truncate(a, 120)),
        None => name.to_string(),
    }
}

/// A tool part as a turn with its result: `state.output` (OpenCode 1) or
/// the text items of `state.content` (2.0). A failed part leads with its
/// error message; a running one has no result yet.
fn tool_turn(part: &Value) -> Turn {
    let mut t = Turn::new(TurnKind::Tool, tool_line(part));
    let state = &part["state"];
    let content = state["content"].as_array().into_iter().flatten().filter(|c| c["type"] == "text").filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n");
    let text = state["output"].as_str().map(str::to_string).unwrap_or(content);
    t.is_error = state["status"].as_str() == Some("error");
    let error = state["error"]["message"].as_str().or_else(|| state["error"].as_str()).unwrap_or("");
    let whole = [error, text.as_str()].into_iter().filter(|s| !s.trim().is_empty()).collect::<Vec<_>>().join("\n");
    t.set_output(&whole);
    t
}
```

In `core/src/actions.rs`, after `session_history`, add:

```rust
/// A session's history as a reply to another machine: tool output and
/// diffs cut to `transcript::REMOTE_OUTPUT_LINES` lines.
pub fn remote_history(l: &Local, session_id: &str) -> Result<Vec<transcript::Turn>, String> {
    session_history(l, session_id).map(transcript::for_remote)
}
```

In `cli/src/executor.rs`, change `CommandKind::History { session } => data(actions::session_history(&l, &session)),` to:

```rust
            CommandKind::History { session } => data(actions::remote_history(&l, &session)),
```

In `src-tauri/src/net_app.rs`, change `CommandKind::History { session } => to_data(actions::session_history(&l, &session)?),` to:

```rust
        CommandKind::History { session } => to_data(actions::remote_history(&l, &session)?),
```

(The desktop's own `session_history` command keeps reading local sessions whole; a remote card's history comes from the assistant's executor, and a phone's from the main's, both now through `remote_history`.)

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p maya-core codex::` then `cargo test -p maya-core opencode::` then `cargo test -p maya-core actions::`
Expected: PASS (the existing `turns_come_from_user_and_assistant_messages_without_environment_noise` and `messages_give_the_last_text_and_the_turns_oldest_first` still pass).

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/transcript.rs core/src/codex.rs core/src/opencode.rs core/src/actions.rs cli/src/executor.rs src-tauri/src/net_app.rs
git commit -m "feat: Codex and OpenCode tool output, cut to 40 lines across machines" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Tool output and diffs in both looks

**Files:**
- Create: `src/toolturn.ts`, `src/toolturn.test.ts`
- Modify: `src/types.ts` (`DiffLine`, `Turn` fields)
- Modify: `src/modal.ts` (import; the turn loop at lines 548–561; `sameTurns`; `openModal`)
- Modify: `src/styles.css` (append)
- Modify: `src/modal-helpers.test.ts`, `src/modal.test.ts`
- Modify: `docs/DEVELOPING.md`

**Interfaces:**
- Consumes (Tasks 1–3): the turn JSON `{kind, text, notice?, output?, outputLines?, isError?, diff?: {op, line?, text}[], added?, removed?}`.
- Produces:
  - `src/types.ts`: `export interface DiffLine { op: "add" | "del" | "same"; line?: number | null; text: string }`; `Turn` gains `output?: string | null; outputLines?: number; isError?: boolean; diff?: DiffLine[] | null; added?: number; removed?: number`.
  - `src/toolturn.ts`: `PREVIEW_LINES = 4`; `toolUi: { opened: Set<string>; expanded: Set<string> }`; `resetToolUi(): void`; `toolKey(t: Turn): string`; `toolHead(text: string): { name: string; arg: string }`; `rowText(t: Turn): string`; `headText(t: Turn): string`; `renderToolTurn(t: Turn): HTMLElement`.
  - `src/modal.ts`: `sameTurns` also compares the tool fields.

Decided here: one DOM per tool call serves both looks, so the header toggle (which only flips `modal--terminal`) switches them at once: a `.turn__row` button (the chat row), the existing `.turn__text` (the terminal's `Bash(cmd)`, or `Update(path)` for an edit), and a `.turn__result` (output under `⎿` with the 4-line preview, or the diff). The stylesheet shows the row in the chat look and the call and result in the terminal look; an opened chat row shows the same result block. Which rows are open and which results are expanded is kept in `toolUi` by `toolKey` (the call's text, the start of its output and its counts), not by index, so a repaint keeps them and new turns arriving do not shift them; `openModal` clears it. Every edit, Write included, is titled `Update(path)` as the spec writes it. The existing `⏺` marker is kept (the addendum's `●` is read as that marker; the terminal look's styling does not change).

- [ ] **Step 1: Write the failing tests**

Create `src/toolturn.test.ts`:

```ts
import styles from "./styles.css?raw";
import { beforeEach, describe, expect, it } from "vitest";
import { PREVIEW_LINES, renderToolTurn, resetToolUi, rowText, toolHead } from "./toolturn";
import type { Turn } from "./types";

const lines = (n: number) => Array.from({ length: n }, (_, i) => `line ${i + 1}`).join("\n");
const bash: Turn = { kind: "tool", text: "Bash(cargo test)", output: lines(10), outputLines: 10 };
const edit: Turn = {
  kind: "tool",
  text: "Edit(/x/src/a.rs)",
  diff: [
    { op: "del", line: 3, text: "fn mian() {" },
    { op: "add", line: 3, text: "fn main() {" },
    { op: "same", line: 4, text: "    run();" },
    { op: "add", line: 5, text: "    done();" },
  ],
  added: 2,
  removed: 1,
};

beforeEach(resetToolUi);

describe("tool turns", () => {
  it("reads the call's name and argument, in this Maya's form and an older one's", () => {
    expect(toolHead("Bash(cargo test)")).toEqual({ name: "Bash", arg: "cargo test" });
    expect(toolHead("Bash: cargo test")).toEqual({ name: "Bash", arg: "cargo test" });
    expect(toolHead("mcp__pr-watch__wait_for_pr(37)")).toEqual({ name: "mcp__pr-watch__wait_for_pr", arg: "37" });
    expect(toolHead("ListAgents")).toEqual({ name: "ListAgents", arg: "" });
  });

  it("shows the first 4 lines under ⎿ and expands the rest in place", () => {
    const t = renderToolTurn(bash);
    expect(t.querySelector(".turn__mark")?.textContent).toBe("⏺");
    expect(t.querySelector(".turn__text")?.textContent).toBe("Bash(cargo test)");
    expect(t.querySelector(".turn__elbow")?.textContent).toBe("⎿");
    expect(t.querySelector(".turn__out")?.textContent).toBe(lines(PREVIEW_LINES));
    const more = t.querySelector<HTMLButtonElement>(".turn__more")!;
    expect(more.textContent).toBe("… +6 lines");
    more.click();
    expect(t.querySelector(".turn__out")?.textContent).toBe(lines(10));
    expect(t.querySelector(".turn__more")).toBeNull();
  });

  it("counts lines the cap left out, and keeps an expanded result open across repaints", () => {
    const capped: Turn = { ...bash, output: lines(40), outputLines: 250 };
    const t = renderToolTurn(capped);
    expect(t.querySelector(".turn__more")?.textContent).toBe("… +246 lines");
    t.querySelector<HTMLButtonElement>(".turn__more")!.click();
    expect(t.querySelector(".turn__left")?.textContent).toBe("… +210 lines");
    const again = renderToolTurn({ ...capped });
    expect(again.querySelector(".turn__out")?.textContent).toBe(lines(40));
    expect(again.querySelector(".turn__more")).toBeNull();
    resetToolUi();
    expect(renderToolTurn({ ...capped }).querySelector(".turn__more")).not.toBeNull();
  });

  it("shows a short result whole, and no result without output", () => {
    const short = renderToolTurn({ kind: "tool", text: "Bash(ls)", output: "a.rs\nb.rs", outputLines: 2 });
    expect(short.querySelector(".turn__out")?.textContent).toBe("a.rs\nb.rs");
    expect(short.querySelector(".turn__more")).toBeNull();
    expect(renderToolTurn({ kind: "tool", text: "ListAgents" }).querySelector(".turn__result")).toBeNull();
  });

  it("draws an edit as Update(path) with its counts and coloured, numbered lines", () => {
    const t = renderToolTurn(edit);
    expect(t.querySelector(".turn__text")?.textContent).toBe("Update(/x/src/a.rs)");
    expect(t.querySelector(".turn__summary-text")?.textContent).toBe("Updated /x/src/a.rs (+2 −1)");
    const rows = [...t.querySelectorAll(".diff__line")].map((r) => [r.className, r.querySelector(".diff__no")?.textContent, r.querySelector(".diff__sign")?.textContent, r.querySelector(".diff__text")?.textContent]);
    expect(rows).toEqual([
      ["diff__line diff__line--del", "3", "-", "fn mian() {"],
      ["diff__line diff__line--add", "3", "+", "fn main() {"],
      ["diff__line diff__line--same", "4", " ", "    run();"],
      ["diff__line diff__line--add", "5", "+", "    done();"],
    ]);
    expect(styles).toContain(".diff__line--add {");
    expect(styles).toContain(".diff__line--del {");
  });

  it("leaves a diff line's number blank when unknown, and says how many lines a cap left out", () => {
    const t = renderToolTurn({ kind: "tool", text: "Write(/x/big.txt)", diff: [{ op: "add", text: "first" }], added: 300, removed: 0 });
    expect(t.querySelector(".diff__no")?.textContent).toBe("");
    expect(t.querySelector(".turn__diff .turn__left")?.textContent).toBe("… +299 lines");
  });

  it("marks a failed call red in both looks", () => {
    const t = renderToolTurn({ kind: "tool", text: "Bash(ls /nope)", output: "ls: /nope: No such file or directory", outputLines: 1, isError: true });
    expect(t.classList.contains("turn--failed")).toBe(true);
    expect(t.querySelector(".turn__row")?.textContent).toBe("▸ Bash · ls /nope ✕");
    expect(styles).toContain(".turn--failed .turn__out { color: #dc2626; }");
    expect(styles).toContain(".turn__fail { color: #dc2626; }");
  });

  it("gives the chat look a compact row that opens the output or diff", () => {
    expect(rowText(bash)).toBe("▸ Bash · cargo test");
    expect(rowText(edit)).toBe("▸ Update · /x/src/a.rs (+2 −1)");
    expect(rowText({ kind: "tool", text: "ListAgents" })).toBe("▸ ListAgents");
    const t = renderToolTurn(bash);
    const row = t.querySelector<HTMLButtonElement>(".turn__row")!;
    expect(row.textContent).toBe("▸ Bash · cargo test");
    expect(t.classList.contains("turn--open")).toBe(false);
    row.click();
    expect(t.classList.contains("turn--open")).toBe(true);
    expect(row.getAttribute("aria-expanded")).toBe("true");
    expect(renderToolTurn({ ...bash }).classList.contains("turn--open")).toBe(true);
    row.click();
    expect(renderToolTurn({ ...bash }).classList.contains("turn--open")).toBe(false);
    expect(renderToolTurn({ kind: "tool", text: "ListAgents" }).querySelector<HTMLButtonElement>(".turn__row")!.disabled).toBe(true);
    expect(styles).toContain(".turn--open > .turn__result { display: block; }");
    expect(styles).toContain(".modal--terminal .turn__row { display: none; }");
  });

  it("renders output and diff text as text, never markup", () => {
    const evil = '<img src=x onerror="alert(1)"><script>alert(2)</script>';
    const t = renderToolTurn({ kind: "tool", text: `Bash(${evil})`, output: evil, outputLines: 1 });
    expect(t.querySelector("img, script")).toBeNull();
    expect(t.querySelector(".turn__out")?.textContent).toBe(evil);
    expect(t.querySelector(".turn__text")?.textContent).toBe(`Bash(${evil})`);
    const d = renderToolTurn({ kind: "tool", text: "Edit(/x/a.html)", diff: [{ op: "add", line: 1, text: evil }], added: 1, removed: 0 });
    expect(d.querySelector("img, script")).toBeNull();
    expect(d.querySelector(".diff__text")?.textContent).toBe(evil);
  });
});
```

In `src/modal-helpers.test.ts`, add `import type { Turn } from "./types";` and, inside `describe("sameTurns", …)`, append:

```ts
  it("sees a tool result that landed after its call, and a changed diff", () => {
    const call: Turn = { kind: "tool", text: "Bash(ls)" };
    expect(sameTurns([call], [{ ...call, output: "a.rs", outputLines: 1 }])).toBe(false);
    expect(sameTurns([{ ...call, output: "x" }], [{ ...call, output: "x", isError: true }])).toBe(false);
    const d: Turn = { kind: "tool", text: "Edit(/a)", diff: [{ op: "add", line: 1, text: "a" }], added: 1, removed: 0 };
    expect(sameTurns([d], [{ ...d, diff: [{ op: "add", line: 1, text: "b" }] }])).toBe(false);
    expect(sameTurns([d], [{ ...d }])).toBe(true);
  });
```

Append to `src/modal.test.ts`:

```ts
describe("tool output in the viewer", () => {
  it("draws a tool call with its result in both looks, through the one renderer", () => {
    const t: Turn = { kind: "tool", text: "Bash(ls)", output: "a.rs\nb.rs", outputLines: 2 };
    for (const theme of ["terminal", undefined] as const) {
      const el = renderModal({ card: base, turns: [t], status: null, draft: "", theme }, handlers());
      expect(el.querySelector(".turn--tool .turn__row")?.textContent).toBe("▸ Bash · ls");
      expect(el.querySelector(".turn--tool .turn__text")?.textContent).toBe("Bash(ls)");
      expect(el.querySelector(".turn--tool .turn__out")?.textContent).toBe("a.rs\nb.rs");
    }
  });
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `pnpm test src/toolturn.test.ts src/modal-helpers.test.ts src/modal.test.ts`
Expected: FAIL — `./toolturn` cannot be resolved; `sameTurns` ignores `output`; the viewer has no `.turn__row`.

- [ ] **Step 3: Write the implementation**

In `src/types.ts`, replace the `Turn` interface with:

```ts
/** One line of an edit's diff; `line` is its number in the new file (the old one for a removal), when known. */
export interface DiffLine {
  op: "add" | "del" | "same";
  line?: number | null;
  text: string;
}

export interface Turn {
  kind: "user" | "assistant" | "tool" | "peer";
  /** A compaction or an interrupt, sent as a tool line so older Mayas can read it. */
  notice?: boolean;
  text: string;
  /** A tool call's result: at most 200 lines, 40 from another machine. */
  output?: string | null;
  /** Lines in the whole result before any cap; absent from an older Maya. */
  outputLines?: number;
  /** The tool call failed. */
  isError?: boolean;
  /** An edit's lines: at most 200, 40 from another machine. */
  diff?: DiffLine[] | null;
  /** Lines the whole edit adds and removes. */
  added?: number;
  removed?: number;
}
```

Create `src/toolturn.ts`:

```ts
import type { Turn } from "./types";

/** Output lines a tool call shows before "… +N lines". */
export const PREVIEW_LINES = 4;

/**
 * What the user opened, by `toolKey`: chat rows (`opened`) and results shown
 * whole (`expanded`). Kept outside the DOM so the once-a-second repaint of a
 * working session leaves them open; a newly opened panel starts with none.
 */
export const toolUi = { opened: new Set<string>(), expanded: new Set<string>() };

export function resetToolUi(): void {
  toolUi.opened.clear();
  toolUi.expanded.clear();
}

/** A tool turn's identity across repaints, where its index moves as turns arrive and old ones drop off. */
export function toolKey(t: Turn): string {
  return [t.text, (t.output ?? "").slice(0, 200), t.added ?? 0, t.removed ?? 0].join("\u0000");
}

/** The call's name and argument: `Bash(cargo test)`, or an older Maya's `Bash: cargo test`. */
export function toolHead(text: string): { name: string; arg: string } {
  const call = /^([\w.:-]+)\(([\s\S]*)\)$/.exec(text);
  if (call) return { name: call[1], arg: call[2] };
  const older = /^([\w.:-]+): ([\s\S]*)$/.exec(text);
  if (older) return { name: older[1], arg: older[2] };
  return { name: text, arg: "" };
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const n = document.createElement(tag);
  n.className = className;
  if (text !== undefined) n.textContent = text;
  return n;
}

/** "(+a −b)": the lines an edit adds and removes. */
function counts(t: Turn): string {
  return `(+${t.added ?? 0} −${t.removed ?? 0})`;
}

/** The chat look's row: "▸ Bash · cmd", "▸ Update · path (+a −b)". */
export function rowText(t: Turn): string {
  const { name, arg } = toolHead(t.text);
  const shown = t.diff ? "Update" : name;
  const head = arg ? `▸ ${shown} · ${arg}` : `▸ ${shown}`;
  return t.diff ? `${head} ${counts(t)}` : head;
}

/** The terminal look's call: as the agent wrote it, an edit as "Update(path)". */
export function headText(t: Turn): string {
  return t.diff ? `Update(${toolHead(t.text).arg})` : t.text;
}

/** Output under ⎿: the first PREVIEW_LINES lines and "… +N lines", which shows the rest in place. */
function renderOutput(t: Turn, key: string): HTMLElement {
  const result = el("div", "turn__result turn__result--out");
  const lines = (t.output ?? "").split("\n");
  const total = Math.max(t.outputLines ?? 0, lines.length);
  const pre = el("pre", "turn__out");
  result.append(el("span", "turn__elbow", "⎿"), pre);
  const showAll = () => {
    pre.textContent = lines.join("\n");
    // Lines a cap kept out of the turn cannot be shown, only counted.
    if (total > lines.length) result.append(el("span", "turn__left", `… +${total - lines.length} lines`));
  };
  const hidden = total - PREVIEW_LINES;
  if (hidden <= 0 || toolUi.expanded.has(key)) {
    showAll();
    return result;
  }
  pre.textContent = lines.slice(0, PREVIEW_LINES).join("\n");
  const more = el("button", "turn__more", `… +${hidden} lines`);
  more.type = "button";
  more.dataset.action = "more";
  more.addEventListener("click", (ev) => {
    ev.stopPropagation();
    toolUi.expanded.add(key);
    more.remove();
    showAll();
  });
  result.append(more);
  return result;
}

/** An edit: "⎿ Updated path (+a −b)", then its lines in red and green with their numbers when known. */
function renderDiff(t: Turn): HTMLElement {
  const result = el("div", "turn__result turn__result--diff");
  const summary = el("div", "turn__summary");
  summary.append(el("span", "turn__elbow", "⎿"), el("span", "turn__summary-text", `Updated ${toolHead(t.text).arg} ${counts(t)}`));
  const table = el("div", "turn__diff");
  const diff = t.diff ?? [];
  for (const d of diff) {
    const line = el("div", `diff__line diff__line--${d.op}`);
    line.append(el("span", "diff__no", d.line != null ? String(d.line) : ""), el("span", "diff__sign", d.op === "add" ? "+" : d.op === "del" ? "-" : " "), el("span", "diff__text", d.text));
    table.append(line);
  }
  const left = (t.added ?? 0) + (t.removed ?? 0) - diff.filter((d) => d.op !== "same").length;
  if (left > 0) table.append(el("span", "turn__left", `… +${left} lines`));
  result.append(summary, table);
  return result;
}

/**
 * A tool call for the viewer, drawn for both looks (the stylesheet shows
 * one): the chat look's row, the terminal look's call, and its result.
 * Every string from the transcript goes in as text.
 */
export function renderToolTurn(t: Turn): HTMLElement {
  const key = toolKey(t);
  const body = t.diff ? renderDiff(t) : t.output ? renderOutput(t, key) : null;
  const turn = el("div", "turn turn--tool");
  turn.classList.toggle("turn--failed", !!t.isError);
  const open = !!body && toolUi.opened.has(key);
  turn.classList.toggle("turn--open", open);
  const row = el("button", "turn__row", rowText(t));
  row.type = "button";
  row.dataset.action = "tool";
  row.disabled = !body;
  row.setAttribute("aria-expanded", String(open));
  if (t.isError) row.append(el("span", "turn__fail", " ✕"));
  row.addEventListener("click", () => {
    const now = !toolUi.opened.has(key);
    if (now) toolUi.opened.add(key);
    else toolUi.opened.delete(key);
    turn.classList.toggle("turn--open", now);
    row.setAttribute("aria-expanded", String(now));
  });
  turn.append(el("span", "turn__mark", "⏺"), row, el("div", "turn__text", headText(t)));
  if (body) turn.append(body);
  return turn;
}
```

In `src/modal.ts`:

1. Add `import { renderToolTurn, resetToolUi } from "./toolturn";` after the `./types` import.
2. In `renderModal`, make the first lines of the turn loop (`for (const t of m.turns) {`) read:

```ts
  for (const t of m.turns) {
    if (t.kind === "tool" && !t.notice) {
      history.append(renderToolTurn(t));
      continue;
    }
    const kind = t.notice ? "notice" : t.kind;
```

(the rest of the loop is unchanged; it now draws user, assistant, peer and notice turns).

3. Replace `sameTurns` with:

```ts
/** True when both lists hold the same turns in the same order, tool results and diffs included. */
export function sameTurns(a: Turn[], b: Turn[]): boolean {
  return a.length === b.length && a.every((t, i) => sameTurn(t, b[i]));
}

function sameTurn(a: Turn, b: Turn): boolean {
  return (
    a.kind === b.kind &&
    a.text === b.text &&
    !!a.notice === !!b.notice &&
    (a.output ?? null) === (b.output ?? null) &&
    (a.outputLines ?? 0) === (b.outputLines ?? 0) &&
    !!a.isError === !!b.isError &&
    (a.added ?? 0) === (b.added ?? 0) &&
    (a.removed ?? 0) === (b.removed ?? 0) &&
    JSON.stringify(a.diff ?? null) === JSON.stringify(b.diff ?? null)
  );
}
```

4. In `openModal`, right after `closeModal();`, add `resetToolUi();`.

Append to `src/styles.css`:

```css
/* Tool calls. Chat look: a compact row that opens the output or diff. */
.turn__row { all: unset; cursor: pointer; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 11.5px; color: var(--muted); white-space: pre-wrap; word-break: break-word; }
.turn__row:disabled { cursor: default; }
.turn__fail { color: #dc2626; }
.turn--tool > .turn__text { display: none; }
.turn__result { display: none; margin-top: 4px; }
.turn--open > .turn__result { display: block; }
.turn__elbow { display: none; }
.turn__out { margin: 0; white-space: pre-wrap; word-break: break-word; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 11.5px; }
.turn--failed .turn__out { color: #dc2626; }
.turn__more { all: unset; cursor: pointer; color: var(--muted); font-size: 11.5px; }
.turn__more:hover { color: var(--text); }
.turn__left { color: var(--muted); font-size: 11.5px; }
.turn__diff { margin-top: 2px; }
.diff__line { display: grid; grid-template-columns: 5ch 2ch minmax(0, 1fr); font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 11.5px; white-space: pre-wrap; word-break: break-word; }
.diff__no { color: var(--muted); text-align: right; padding-right: 1ch; }
.diff__line--add { color: #8fdca8; background: rgba(46, 160, 67, .16); }
.diff__line--del { color: #f4a3a3; background: rgba(248, 81, 73, .16); }
/* Terminal look: the call as the agent wrote it, its result under ⎿. */
.modal--terminal .turn__row { display: none; }
.modal--terminal .turn--tool > .turn__text { display: block; }
.modal--terminal .turn--tool > .turn__result { display: block; }
.modal--terminal .turn--tool > .turn__result--out { display: grid; grid-template-columns: 3ch minmax(0, 1fr); }
.modal--terminal .turn__result--out > :not(.turn__elbow) { grid-column: 2; }
.modal--terminal .turn__elbow { display: inline; }
.modal--terminal .turn__summary .turn__elbow { margin-right: 1ch; }
```

In `docs/DEVELOPING.md`, replace

```
- `src/` — `main.ts`, `board.ts`, `card.ts`, `modal.ts`, `reviews.ts`,
```

with

```
- `src/` — `main.ts`, `board.ts`, `card.ts`, `modal.ts` with `toolturn.ts` (a tool call's output and diff in the viewer), `reviews.ts`,
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `pnpm test src/toolturn.test.ts src/modal-helpers.test.ts src/modal.test.ts`
Expected: PASS (the existing "keeps a multi-line tool call whole in one .turn__text, in both looks" and the markers test still pass).

Run: `pnpm test && pnpm exec tsc --noEmit`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/toolturn.ts src/toolturn.test.ts src/types.ts src/modal.ts src/styles.css src/modal-helpers.test.ts src/modal.test.ts docs/DEVELOPING.md
git commit -m "feat: the viewer shows tool output and edit diffs in both looks" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: The working line and the status line

**Files:**
- Modify: `core/src/pr.rs` (`PrCache` branches; test)
- Modify: `core/src/store.rs` (`set_branch`; `refresh` fills `branch`, and `model` for Claude Code; test)
- Modify: `core/src/model.rs` (`Card.branch`; test)
- Modify: every `Card { … }` literal: `core/examples/linux_probe.rs`, `core/src/notify.rs`, `core/src/actions.rs`, `core/src/answer.rs`, `core/src/state.rs`, `core/src/foreign.rs`, `core/src/pending_names.rs`, `core/src/model.rs`, `core/src/interpreter.rs`, `core/src/net/server.rs`, `core/src/net/protocol.rs`, `core/src/net/merge.rs`, `core/src/opencode.rs`, `src-tauri/src/badge.rs`, `src-tauri/src/net_app.rs`, `src-tauri/src/lib.rs`, `src-tauri/src/listener.rs`, `mobile/src/alerts.rs`, `mobile/tests/round_trip.rs`
- Modify: `src-tauri/src/lib.rs` (`poll_pull_requests`)
- Modify: `src/types.ts` (`Card.branch`), `src/modal.ts`, `src/styles.css`, `src/modal.test.ts`

**Interfaces:**
- Consumes: `pr::branch_of(dir: &Path) -> Option<String>`; `TranscriptTail.model`; `projectName(cwd)` (`src/format.ts`); `prRef(pr)` (`src/types.ts`).
- Produces:
  - Rust: `Card.branch: Option<String>` (`#[serde(default, skip_serializing_if = "Option::is_none")]`, JSON `branch`); `PrCache::branch(&self, dir: &str) -> Option<String>`, `PrCache::set_branch(&mut self, dir: &str, branch: Option<String>)`; `Store::set_branch(&mut self, dir: &str, branch: Option<String>)`.
  - TS (`src/modal.ts`): `workingFor(ms: number): string`, `workingText(card: Card, nowMs: number): string | null`, `statusLine(card: Card): string`, `headStatus(card: Card): string`; `Card.branch?: string | null`.

Decided here: the branch is looked up by the PR-badge poll (every directory, every `pr::TTL_MS`), with or without a forge switched on, and travels on the card like the PR, so a remote card and the phone have it. A Claude Code card's `model` is now filled from its transcript (the status line's first item); the Model/Effort pickers are unaffected, since Claude Code's models carry no per-model efforts. The chat look's header already shows the folder, the PR and ctx %, so its part of the status line is `model · branch`, beside ctx %. Both status elements are always rendered (empty when nothing is known) so `patchModal` swaps them in place; the stylesheet shows one per look. The working line sits at the end of the history, so it scrolls with the turns, and its time moves on every board refresh without a repaint.

- [ ] **Step 1: Write the failing tests**

Append to `core/src/pr.rs`'s `mod tests`:

```rust
    #[test]
    fn branches_are_kept_by_directory_and_forgotten_with_it() {
        let mut c = PrCache::default();
        c.set_branch("/a", Some("feat/x".into()));
        assert_eq!(c.branch("/a").as_deref(), Some("feat/x"));
        c.set_branch("/a", None);
        assert_eq!(c.branch("/a"), None, "detached or not a repo");
        c.set_branch("/a", Some("main".into()));
        c.set_branch("/b", Some("dev".into()));
        c.retain(&["/b".to_string()]);
        assert_eq!(c.branch("/a"), None);
        assert_eq!(c.branch("/b").as_deref(), Some("dev"));
        c.clear();
        assert_eq!(c.branch("/b"), None);
    }
```

Append to `core/src/store.rs`'s `mod tests`:

```rust
    #[test]
    fn refresh_attaches_the_branch_and_a_claude_sessions_model() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().to_path_buf();
        std::fs::create_dir_all(claude.join("sessions")).unwrap();
        std::fs::create_dir_all(claude.join("projects/-Users-x-dev-eye")).unwrap();
        std::fs::write(claude.join("sessions/7.json"), r#"{"pid":7,"sessionId":"s7","cwd":"/Users/x/dev/eye","name":"eye-7","status":"idle"}"#).unwrap();
        std::fs::write(
            claude.join("projects/-Users-x-dev-eye/s7.jsonl"),
            r#"{"type":"assistant","message":{"model":"claude-opus-5[1m]","usage":{"input_tokens":10,"cache_read_input_tokens":0,"cache_creation_input_tokens":0},"content":[{"type":"text","text":"hi"}]}}"#,
        ).unwrap();
        let mut store = Store::new(claude).with_alive(|_| true);
        let card = store.refresh(1).remove(0);
        assert_eq!(card.model.as_deref(), Some("claude-opus-5[1m]"));
        assert_eq!(card.branch, None, "not looked up yet");
        store.set_branch("/Users/x/dev/eye", Some("feat/x".into()));
        assert_eq!(store.refresh(2)[0].branch.as_deref(), Some("feat/x"));
    }
```

Append to `core/src/model.rs`'s `mod tests`:

```rust
    #[test]
    fn a_card_carries_its_branch_and_an_older_cards_has_none() {
        let older = serde_json::json!({"sessionId": "s1", "pid": 1, "name": "n", "cwd": "/x", "state": "idle", "stateSince": 0, "snippet": "", "awaiting": null, "hasInbox": false, "harness": "claude-code", "pr": null, "context": null});
        let card: Card = serde_json::from_value(older).unwrap();
        assert_eq!(card.branch, None);
        let with = Card { branch: Some("feat/x".into()), ..card };
        assert_eq!(serde_json::to_value(&with).unwrap()["branch"], "feat/x");
        assert!(serde_json::to_value(Card { branch: None, ..with }).unwrap().get("branch").is_none(), "left out when unknown");
    }
```

Append to `src/modal.test.ts`, and add `statusLine, workingFor` to its import from `./modal` (`patchModal`, `styles` and `Card` are already imported):

```ts
describe("working and status lines", () => {
  const busy: Card = { ...base, state: "working", stateSince: 1_000, model: "claude-opus-5", branch: "feat/x", pr: { number: 12, url: "u", state: "open" }, context: { used: 84_000, window: 200_000, percent: 42 } };

  it("says how long the session has worked", () => {
    expect(workingFor(4_000)).toBe("4s");
    expect(workingFor(724_000)).toBe("12m 4s");
    expect(workingFor(3_725_000)).toBe("1h 2m");
    expect(workingFor(-5_000)).toBe("0s");
  });

  it("shows ✻ Working… while the session works, in both looks", () => {
    for (const theme of ["terminal", "chat"] as const) {
      const el = renderModal({ card: busy, turns, status: null, draft: "", theme }, handlers(), 1_000 + 724_000);
      expect(el.querySelector(".modal__history .modal__working")?.textContent).toBe("✻ Working… (12m 4s)");
    }
    expect(renderModal({ card: base, turns, status: null, draft: "" }, handlers()).querySelector(".modal__working")).toBeNull();
  });

  it("puts model · folder · branch · PR · ctx under the prompt, and model · branch in the header", () => {
    expect(statusLine(busy)).toBe("claude-opus-5 · eye · feat/x · PR #12 · ctx 42%");
    expect(statusLine({ ...busy, pr: { number: 3, url: "u", state: "open", forge: "gitlab" } })).toBe("claude-opus-5 · eye · feat/x · MR !3 · ctx 42%");
    expect(statusLine(base)).toBe("eye");
    const el = renderModal({ card: busy, turns, status: null, draft: "", theme: "terminal" }, handlers(), 2_000);
    expect(el.querySelector(".modal__composer + .modal__statusline--foot")?.textContent).toBe("claude-opus-5 · eye · feat/x · PR #12 · ctx 42%");
    expect(el.querySelector(".modal__titles .modal__statusline--head")?.textContent).toBe("claude-opus-5 · feat/x");
  });

  it("shows the foot in the terminal look and the header part in the chat look", () => {
    expect(styles).toContain(".modal__statusline--foot { display: none; }");
    expect(styles).toContain(".modal--terminal .modal__statusline--foot { display: block;");
    expect(styles).toContain(".modal--terminal .modal__statusline--head { display: none; }");
  });

  it("a repaint carries the status lines and the working line along", () => {
    const root = renderModal({ card: base, turns, status: null, draft: "" }, handlers());
    patchModal(root, renderModal({ card: busy, turns, status: null, draft: "" }, handlers(), 2_000));
    expect(root.querySelector(".modal__statusline--head")?.textContent).toBe("claude-opus-5 · feat/x");
    expect(root.querySelector(".modal__statusline--foot")?.textContent).toBe("claude-opus-5 · eye · feat/x · PR #12 · ctx 42%");
    expect(root.querySelector(".modal__working")?.textContent).toBe("✻ Working… (1s)");
  });
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p maya-core pr::` then `pnpm test src/modal.test.ts`
Expected: FAIL — `no method named set_branch`; `workingFor` and `statusLine` are not exported.

- [ ] **Step 3: Write the implementation**

In `core/src/model.rs`, add after `Card`'s `model` field:

```rust
    /// The branch checked out in `cwd`, from the same lookup as `pr`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
```

Give every `Card` literal the field. The one-line ones end in `stale: false, model: None`:

```bash
perl -pi -e 's/stale: false, model: None(?!, branch)/stale: false, model: None, branch: None/g' core/examples/linux_probe.rs core/src/notify.rs core/src/actions.rs core/src/answer.rs core/src/state.rs core/src/foreign.rs core/src/pending_names.rs core/src/model.rs core/src/interpreter.rs core/src/net/server.rs core/src/net/protocol.rs core/src/net/merge.rs src-tauri/src/badge.rs src-tauri/src/net_app.rs src-tauri/src/lib.rs src-tauri/src/listener.rs
```

and by hand: in `core/src/opencode.rs`'s `card_for`, after `model: s.model.as_ref().map(|(p, m, _)| format!("{p}/{m}")),` add `branch: None,`; in `mobile/src/alerts.rs` (the test card, `model: None,` after `stale: false,`) and `mobile/tests/round_trip.rs` (`fn card`, `model: None,` after `stale: false,`) add `branch: None,` on the line after `model: None,`. `cargo build --workspace --tests` names any literal still missing it.

In `core/src/pr.rs`, replace `PrCache` and its `impl` with:

```rust
/// Lookup results by session directory.
#[derive(Default)]
pub struct PrCache {
    entries: HashMap<String, Entry>,
    /// The branch checked out in each directory, from the same poll.
    branches: HashMap<String, String>,
}

impl PrCache {
    pub fn get(&self, dir: &str) -> Option<PullRequest> {
        self.entries.get(dir).and_then(|e| e.pr.clone())
    }

    pub fn set(&mut self, dir: &str, pr: Option<PullRequest>, now_ms: u64) {
        self.entries.insert(dir.to_string(), Entry { pr, checked_at: now_ms });
    }

    pub fn branch(&self, dir: &str) -> Option<String> {
        self.branches.get(dir).cloned()
    }

    /// None: detached, or not a repo.
    pub fn set_branch(&mut self, dir: &str, branch: Option<String>) {
        match branch {
            Some(b) => {
                self.branches.insert(dir.to_string(), b);
            }
            None => {
                self.branches.remove(dir);
            }
        }
    }

    /// The directories among `dirs` never looked up, or looked up over `TTL_MS` ago.
    pub fn due(&self, dirs: &[String], now_ms: u64) -> Vec<String> {
        dirs.iter()
            .filter(|d| self.entries.get(*d).map_or(true, |e| now_ms.saturating_sub(e.checked_at) >= TTL_MS))
            .cloned()
            .collect()
    }

    /// Forgets everything, so every directory is looked up again.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.branches.clear();
    }

    /// Forgets directories not in `live`.
    pub fn retain(&mut self, live: &[String]) {
        self.entries.retain(|d, _| live.contains(d));
        self.branches.retain(|d, _| live.contains(d));
    }
}
```

In `core/src/store.rs`, add after `set_pr`:

```rust
    /// Records the branch checked out in `dir`, looked up beside its PR.
    pub fn set_branch(&mut self, dir: &str, branch: Option<String>) {
        self.prs.set_branch(dir, branch);
    }
```

and in `refresh`: for Claude Code sessions, after `card.pr = self.prs.get(&s.cwd);` and the `card.context = …` line, add

```rust
            card.branch = self.prs.branch(&s.cwd);
            // Claude Code names its model in the transcript; the viewer's status line shows it.
            card.model = tail.model.clone();
```

for foreign sessions, after `card.pr = self.prs.get(&s.cwd);` add `card.branch = self.prs.branch(&s.cwd);`; for OpenCode, after `card.pr = self.prs.get(&card.cwd);` add `card.branch = self.prs.branch(&card.cwd);`.

In `src-tauri/src/lib.rs`'s `poll_pull_requests`, make the loop read:

```rust
    for dir in due {
        // The status line's branch comes from this poll too, forge or not.
        let branch = pr::branch_of(std::path::Path::new(&dir));
        let pr = pr::lookup(std::path::Path::new(&dir), &forges, git::http()).unwrap_or_else(|e| {
            log::line("app", format!("pull request for {dir}: {e}"));
            None
        });
        let state = app.state::<AppState>();
        let mut store = state.store.lock().unwrap();
        if state.git_generation.load(std::sync::atomic::Ordering::SeqCst) != generation {
            log::line("app", "pull requests: the Git settings changed during the lookup; its answers are dropped");
            return;
        }
        store.set_pr(&dir, pr, now_ms());
        store.set_branch(&dir, branch);
    }
```

In `src/types.ts`, add to `Card` after `model`:

```ts
  /** The branch checked out in `cwd`, from the same lookup as the PR; absent until looked up. */
  branch?: string | null;
```

In `src/modal.ts`:

1. Add `prRef` to the import from `./types`.
2. Add, above `renderModal`:

```ts
/** "4s", "12m 4s", "1h 2m": how long a session has worked. A clock running ahead reads 0s. */
export function workingFor(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${s % 60}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

/** "✻ Working… (12m 4s)" while the session works, from `stateSince`; null otherwise. */
export function workingText(card: Card, nowMs: number): string | null {
  return card.state === "working" ? `✻ Working… (${workingFor(nowMs - card.stateSince)})` : null;
}

/** The terminal look's status line: model · folder · branch · PR · ctx %, each when known. */
export function statusLine(card: Card): string {
  const pr = card.pr ? `${card.pr.forge === "gitlab" ? "MR" : "PR"} ${prRef(card.pr)}` : null;
  return [card.model, projectName(card.cwd), card.branch, pr, card.context ? `ctx ${card.context.percent}%` : null].filter(Boolean).join(" · ");
}

/** The chat look's part of it, beside the header's folder, PR and ctx %: model · branch. */
export function headStatus(card: Card): string {
  return [card.model, card.branch].filter(Boolean).join(" · ");
}
```

3. In `renderModal`, right after the `if (m.card.context) { … }` block in the header, add:

```ts
  titles.append(el("span", "modal__statusline modal__statusline--head", headStatus(m.card)));
```

right after the turn loop (before `history.addEventListener("click", …)`), add:

```ts
  const working = workingText(m.card, nowMs);
  if (working) history.append(el("div", "modal__working", working));
```

and right after `panel.append(form);`, add:

```ts
    panel.append(el("div", "modal__statusline modal__statusline--foot", statusLine(m.card)));
```

4. In `patchModal`, after `swap(".modal__state");`, add:

```ts
  swap(".modal__statusline--head");
  swap(".modal__statusline--foot");
```

5. Add, above `refreshModal`:

```ts
/** Moves the open panel's working line on to `nowMs` without a repaint. */
function tickWorking(nowMs: number): void {
  if (!current) return;
  const line = document.querySelector<HTMLElement>("#modal-host .modal__working");
  const text = workingText(current.model.card, nowMs);
  if (line && text) line.textContent = text;
}
```

and in `refreshModal`, after `current.model.card = fresh;`, add `tickWorking(Date.now());`.

Append to `src/styles.css`:

```css
/* While the session works, and the status line: the terminal look's under the prompt, the chat look's in the header. */
.modal__working { align-self: flex-start; font-size: 12px; color: var(--gold); }
.modal--terminal .modal__working { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
.modal__statusline--head { font-size: 12px; color: var(--muted); }
.modal__statusline--head:empty { display: none; }
.modal__statusline--foot { display: none; }
.modal--terminal .modal__statusline--foot { display: block; padding: 0 16px 8px 32px; background: #0a0a0a; color: #8a8478; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 11.5px; }
.modal--terminal .modal__statusline--head { display: none; }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p maya-core pr::` then `cargo test -p maya-core store::tests::refresh_attaches` then `cargo test -p maya-core model::` then `pnpm test src/modal.test.ts`
Expected: PASS.

Run: `cargo test --workspace && pnpm test && pnpm exec tsc --noEmit`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/pr.rs core/src/store.rs core/src/model.rs core/examples/linux_probe.rs core/src/notify.rs core/src/actions.rs core/src/answer.rs core/src/state.rs core/src/foreign.rs core/src/pending_names.rs core/src/interpreter.rs core/src/net/server.rs core/src/net/protocol.rs core/src/net/merge.rs core/src/opencode.rs src-tauri/src/badge.rs src-tauri/src/net_app.rs src-tauri/src/lib.rs src-tauri/src/listener.rs mobile/src/alerts.rs mobile/tests/round_trip.rs src/types.ts src/modal.ts src/styles.css src/modal.test.ts
git commit -m "feat: the viewer's working line and status line" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## By hand (owed, not automated)

- On the Mac, a long Claude Code session open in the viewer, side by side with its terminal, in both looks: Bash output and its "… +N lines", an Edit and a Write diff, a failed command, the working line ticking, the status line's branch and PR.
- The same session viewed from the main as a remote card (laptop) and from the phone: output cut to 40 lines, the counts still right.
- A Codex session with a shell call and an `apply_patch`, to confirm the rollout shapes this plan assumed.
