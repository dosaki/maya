use crate::diff::DiffLine;
use crate::model::{parse_questions, AwaitKind, Question};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub const TAIL_BYTES: u64 = 262_144;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenQuestion {
    pub kind: AwaitKind,
    pub detail: String,
    pub questions: Vec<Question>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TranscriptTail {
    pub last_assistant_text: Option<String>,
    pub open_question: Option<OpenQuestion>,
    /// Context in use after the last real assistant message: input plus
    /// cache-read plus cache-creation tokens. None before the first turn.
    pub context_tokens: Option<u64>,
    /// Model id of that message, e.g. `claude-opus-5[1m]`.
    pub model: Option<String>,
}

/// The last `max_bytes` of `path` as text, with the first (possibly partial)
/// line dropped when the read did not start at offset zero. None when the
/// file cannot be read.
pub(crate) fn tail_text(path: &Path, max_bytes: u64) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let meta = file.metadata().ok()?;
    let start = meta.len().saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if start > 0 {
        let i = text.find('\n')?;
        Some(text[i + 1..].to_string())
    } else {
        Some(text)
    }
}

/// Reads at most `max_bytes` from the end of `path` and parses it.
pub fn read_tail(path: &Path, max_bytes: u64) -> TranscriptTail {
    tail_text(path, max_bytes).map(|t| parse_tail(&t)).unwrap_or_default()
}

pub const TURNS_TAIL_BYTES: u64 = 1_048_576;
/// A transcript line longer than this (an image result, a huge paste) is
/// skipped whole when reading the history, unless emptying its long
/// `"data"` strings brings it within this.
pub const TURNS_LINE_MAX_BYTES: usize = 262_144;
/// The history reads at most this many bytes from the end of the file.
pub const TURNS_SCAN_MAX_BYTES: u64 = 8 * 1_048_576;
const TURNS_CHUNK_BYTES: u64 = 1_048_576;
/// A line longer than this is skipped without being held in memory.
pub const TURNS_LINE_CEILING_BYTES: usize = 16 * 1_048_576;

/// A `"data"` string longer than this is emptied before a long line's
/// length is checked: base64 image payloads sit under `source.data`.
const DATA_STRIP_MIN: usize = 1024;

/// `line` with every `"data":"…"` string over `DATA_STRIP_MIN` bytes
/// emptied, found by scanning bytes, not by parsing. Inside a JSON string
/// a quote is escaped, so `"data":"` can only be a key and its value.
fn strip_long_data(line: &[u8]) -> Vec<u8> {
    const KEY: &[u8] = b"\"data\":\"";
    let mut out = Vec::with_capacity(line.len().min(4096));
    let mut i = 0;
    while let Some(at) = line[i..].windows(KEY.len()).position(|w| w == KEY) {
        let start = i + at + KEY.len();
        let mut end = start;
        while end < line.len() && line[end] != b'"' {
            end += if line[end] == b'\\' { 2 } else { 1 };
        }
        let end = end.min(line.len());
        out.extend_from_slice(&line[i..start]);
        if end - start <= DATA_STRIP_MIN {
            out.extend_from_slice(&line[start..end]);
        }
        i = end;
    }
    out.extend_from_slice(&line[i..]);
    out
}

/// The newest whole lines of `path` that total `keep_bytes`, oldest first and
/// joined with `\n`. A line longer than `line_max` has its long `"data"`
/// strings (image payloads) emptied and is kept when that brings it within
/// `line_max`; otherwise it is skipped and costs nothing. Reads at most
/// `scan_max` bytes from the end; the partial line at the start of the
/// scanned region is dropped. None when the file cannot be read.
pub(crate) fn tail_lines(path: &Path, keep_bytes: u64, line_max: usize, scan_max: u64) -> Option<String> {
    tail_lines_chunked(path, keep_bytes, line_max, scan_max, TURNS_CHUNK_BYTES, TURNS_LINE_CEILING_BYTES)
}

/// `tail_lines`, reading `chunk` bytes at a time. A line is held whole
/// until its start is read, so it can be stripped, unless it grows past
/// `ceiling`: then it is skipped without holding the rest of it.
fn tail_lines_chunked(path: &Path, keep_bytes: u64, line_max: usize, scan_max: u64, chunk: u64, ceiling: usize) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let floor = len.saturating_sub(scan_max);
    let mut pos = len;
    // The front of the line whose start is still unread (newest side).
    let mut carry: Vec<u8> = Vec::new();
    // The line being skipped as past the ceiling; its start is still unread.
    let mut skipping = false;
    let mut kept: Vec<Vec<u8>> = Vec::new();
    let mut kept_bytes = 0u64;
    let take = |line: &[u8], kept: &mut Vec<Vec<u8>>, kept_bytes: &mut u64| {
        if line.is_empty() {
            return;
        }
        let line = if line.len() > line_max { strip_long_data(line) } else { line.to_vec() };
        if line.len() <= line_max {
            *kept_bytes += line.len() as u64 + 1;
            kept.push(line);
        }
    };
    while pos > floor && kept_bytes < keep_bytes {
        let start = pos.saturating_sub(chunk).max(floor);
        let mut buf = vec![0u8; (pos - start) as usize];
        file.seek(SeekFrom::Start(start)).ok()?;
        file.read_exact(&mut buf).ok()?;
        pos = start;
        if !skipping {
            buf.extend_from_slice(&carry);
        }
        carry.clear();
        let segs: Vec<&[u8]> = buf.split(|b| *b == b'\n').collect();
        let n = segs.len();
        if skipping && n == 1 {
            continue;
        }
        // The last segment starts a line that began after the newline; when
        // that line is being skipped, drop it.
        let end = if skipping { n - 1 } else { n };
        skipping = false;
        for line in segs[1..end].iter().rev() {
            take(line, &mut kept, &mut kept_bytes);
        }
        let first = segs[0];
        if pos == 0 {
            take(first, &mut kept, &mut kept_bytes);
        } else if first.len() > ceiling {
            skipping = true;
        } else {
            carry.extend_from_slice(first);
        }
    }
    kept.reverse();
    let mut out = Vec::new();
    for (i, line) in kept.iter().enumerate() {
        if i > 0 {
            out.push(b'\n');
        }
        out.extend_from_slice(line);
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TurnKind {
    User,
    Assistant,
    Tool,
    /// A message posted into the session's inbox (by Maya or another session).
    Peer,
    /// Something Claude Code noted in the conversation: a compaction or an interrupt.
    Notice,
}

const PEER_PREFIX: &str = "Another Claude session sent a message:";
const PEER_SUFFIX_MARK: &str = "\n\nThis came from another Claude session";

/// Strips Claude Code's peer-message framing, returning the inner message.
fn unwrap_peer_message(text: &str) -> Option<String> {
    let rest = text.strip_prefix(PEER_PREFIX)?.trim_start_matches('\n');
    let body = match rest.find(PEER_SUFFIX_MARK) {
        Some(i) => &rest[..i],
        None => rest,
    };
    Some(body.trim().to_string())
}

/// Claude Code's line when the user pressed Esc, sometimes with " for tool use".
const INTERRUPTED: &str = "[Request interrupted by user";

/// The user's text without Claude Code's `<pasted_content …>` wrappers.
fn strip_paste_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("<pasted_content") {
        out.push_str(&rest[..i]);
        match rest[i..].find('>') {
            Some(j) => rest = &rest[i + j + 1..],
            None => {
                rest = &rest[i..];
                break;
            }
        }
    }
    out.push_str(rest);
    out.replace("</pasted_content>", "").trim().to_string()
}

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

fn text_blocks(content: &Value) -> Vec<String> {
    let texts: Vec<String> = match content {
        Value::String(s) => vec![s.clone()],
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str().map(|t| t.to_string()))
            .collect(),
        _ => vec![],
    };
    texts.into_iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect()
}

/// Most tool lines kept in all, however many there are between messages.
const MAX_TOOL_TURNS: usize = 200;

/// The last `max_messages` user, assistant and peer turns with the tool
/// lines among them: tool calls do not count, or a busy session's last
/// message would scroll out after a few dozen commands.
pub(crate) fn keep_last_messages(mut turns: Vec<Turn>, max_messages: usize) -> Vec<Turn> {
    // From the oldest kept message (or the start, when there are fewer),
    // then the oldest tool lines past the cap go: tools never cost a message.
    let mut messages = 0;
    let mut start = 0;
    for (i, t) in turns.iter().enumerate().rev() {
        if t.kind != TurnKind::Tool {
            messages += 1;
            if messages == max_messages {
                start = i;
                break;
            }
        }
    }
    let mut kept = turns.split_off(start);
    let mut excess = kept.iter().filter(|t| t.kind == TurnKind::Tool).count().saturating_sub(MAX_TOOL_TURNS);
    kept.retain(|t| {
        let drop = excess > 0 && t.kind == TurnKind::Tool;
        if drop {
            excess -= 1;
        }
        !drop
    });
    kept
}

/// Whether a user line's text was put there by the harness rather than
/// typed: a task notification, a slash-command echo, the input or output of
/// a `!` shell line, a caveat. They all open with a hyphenated
/// `<lowercase-tag`, which plain HTML in a prompt does not.
fn is_harness_tag(text: &str) -> bool {
    let Some(rest) = text.strip_prefix('<') else { return false };
    let name_len = rest.bytes().take_while(|b| b.is_ascii_lowercase() || *b == b'-').count();
    rest[..name_len].contains('-') && rest[name_len..].starts_with(['>', ' ', '\n'])
}

/// Whether a hidden user line was still the user's doing: the echo of a
/// `/command` or the input of a `!` shell line. What the assistant said
/// before one is a finished answer, not narration.
fn is_user_initiated(text: &str) -> bool {
    text.starts_with("<command-name>") || text.starts_with("<bash-input>")
}

/// A turn to show, or an invisible break between assistant runs.
enum Item {
    Turn(Turn),
    Break,
}

fn notice(text: &str) -> Item {
    Item::Turn(Turn::new(TurnKind::Notice, text.into()))
}

/// Only the last text of each assistant run (the texts between two
/// user or peer turns, or breaks) is an answer; the ones before it
/// narrate progress.
fn drop_narration(items: Vec<Item>) -> Vec<Turn> {
    let mut kept = Vec::with_capacity(items.len());
    let mut last_text_in_run: Option<usize> = None;
    for item in items {
        let t = match item {
            Item::Turn(t) => t,
            Item::Break => {
                last_text_in_run = None;
                continue;
            }
        };
        match t.kind {
            TurnKind::Assistant => {
                if let Some(i) = last_text_in_run {
                    kept.remove(i);
                }
                last_text_in_run = Some(kept.len());
            }
            TurnKind::User | TurnKind::Peer | TurnKind::Notice => last_text_in_run = None,
            TurnKind::Tool => {}
        }
        kept.push(t);
    }
    kept
}

/// Conversation turns (user prompts, assistant text, tool calls with their
/// results) in order, keeping the last `max_turns` messages and the tool calls among them.
/// Subagent sidechain lines, harness-injected user lines (skill bodies,
/// notifications, subagent hand-backs) and progress narration are skipped;
/// a compaction or an interrupt is a notice.
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

/// Last `max_turns` turns from the newest 1 MB of the transcript's lines,
/// skipping lines over 256 KB once their image payloads are emptied.
pub fn read_turns(path: &Path, max_turns: usize) -> Vec<Turn> {
    tail_lines(path, TURNS_TAIL_BYTES, TURNS_LINE_MAX_BYTES, TURNS_SCAN_MAX_BYTES)
        .map(|t| parse_turns(&t, max_turns))
        .unwrap_or_default()
}

/// Caches parsed tails per path, re-reading only when size or mtime changes.
#[derive(Default)]
pub struct TailCache {
    entries: std::collections::HashMap<std::path::PathBuf, (u64, Option<std::time::SystemTime>, TranscriptTail)>,
    /// Number of real reads performed (for tests and diagnostics).
    pub reads: usize,
}

impl TailCache {
    pub fn get(&mut self, path: &Path) -> TranscriptTail {
        let Ok(meta) = std::fs::metadata(path) else {
            self.entries.remove(path);
            return TranscriptTail::default();
        };
        let key = (meta.len(), meta.modified().ok());
        if let Some((len, mtime, tail)) = self.entries.get(path) {
            if (*len, *mtime) == key {
                return tail.clone();
            }
        }
        self.reads += 1;
        let tail = read_tail(path, TAIL_BYTES);
        self.entries.insert(path.to_path_buf(), (key.0, key.1, tail.clone()));
        tail
    }

    /// Drops entries for paths no longer in use.
    pub fn retain(&mut self, live: &[std::path::PathBuf]) {
        self.entries.retain(|p, _| live.contains(p));
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

pub fn parse_tail(text: &str) -> TranscriptTail {
    let mut last_text: Option<String> = None;
    let mut open: Vec<(String, OpenQuestion)> = Vec::new();
    let mut answered: HashSet<String> = HashSet::new();
    let mut context_tokens: Option<u64> = None;
    let mut model: Option<String> = None;

    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let kind = v["type"].as_str().unwrap_or("");
        let Some(content) = v["message"]["content"].as_array() else { continue };
        match kind {
            "assistant" => {
                // Synthetic records (interruptions, errors) carry no real usage.
                let m = v["message"]["model"].as_str().unwrap_or("");
                let u = &v["message"]["usage"];
                let total = ["input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens"].iter().map(|k| u[k].as_u64().unwrap_or(0)).sum::<u64>();
                if !m.is_empty() && !m.starts_with('<') && total > 0 {
                    context_tokens = Some(total);
                    model = Some(m.to_string());
                }
                for block in content {
                    match block["type"].as_str() {
                        Some("text") => {
                            if let Some(t) = block["text"].as_str() {
                                if !t.trim().is_empty() {
                                    last_text = Some(t.trim().to_string());
                                }
                            }
                        }
                        Some("tool_use") => {
                            let id = block["id"].as_str().unwrap_or("").to_string();
                            match block["name"].as_str() {
                                Some("AskUserQuestion") => {
                                    let q = block["input"]["questions"][0]["question"].as_str().unwrap_or("Question").to_string();
                                    open.push((id, OpenQuestion { kind: AwaitKind::Question, detail: q, questions: parse_questions(&block["input"]) }));
                                }
                                Some("ExitPlanMode") => {
                                    open.push((id, OpenQuestion { kind: AwaitKind::Plan, detail: "Plan approval".to_string(), questions: vec![] }));
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
            }
            "user" => {
                for block in content {
                    if block["type"].as_str() == Some("tool_result") {
                        if let Some(id) = block["tool_use_id"].as_str() {
                            answered.insert(id.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let open_question = open.into_iter().rev().find(|(id, _)| !answered.contains(id)).map(|(_, q)| q);
    TranscriptTail { last_assistant_text: last_text, open_question, context_tokens, model }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{DiffLine, DiffOp};
    use crate::model::AwaitKind;
    use std::path::Path;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/transcript").join(name)).unwrap()
    }

    #[test]
    fn detects_open_question_and_last_text() {
        let t = parse_tail(&fixture("question-open.jsonl"));
        assert_eq!(t.last_assistant_text.as_deref(), Some("I have enough context. One question first."));
        let q = t.open_question.unwrap();
        assert_eq!(q.kind, AwaitKind::Question);
        assert_eq!(q.detail, "What should Completed mean?");
        assert_eq!(q.questions[0].header, "Completed");
        assert_eq!(q.questions[0].options[0].label, "Finished");
    }

    #[test]
    fn reads_context_tokens_and_model_from_the_last_real_assistant_message() {
        let text = concat!(
            r#"{"type":"assistant","message":{"model":"claude-opus-5","usage":{"input_tokens":10,"cache_read_input_tokens":1000,"cache_creation_input_tokens":90,"output_tokens":5},"content":[{"type":"text","text":"a"}]}}"#, "\n",
            r#"{"type":"assistant","message":{"model":"claude-opus-5","usage":{"input_tokens":20,"cache_read_input_tokens":2000,"cache_creation_input_tokens":180,"output_tokens":5},"content":[{"type":"text","text":"b"}]}}"#, "\n",
            r#"{"type":"assistant","message":{"model":"<synthetic>","usage":{"input_tokens":0,"output_tokens":0},"content":[{"type":"text","text":"Request interrupted"}]}}"#, "\n",
        );
        let t = parse_tail(text);
        assert_eq!(t.context_tokens, Some(2200));
        assert_eq!(t.model.as_deref(), Some("claude-opus-5"));
        assert_eq!(parse_tail("").context_tokens, None);
    }

    #[test]
    fn answered_question_is_not_open() {
        let t = parse_tail(&fixture("question-answered.jsonl"));
        assert!(t.open_question.is_none());
        assert_eq!(t.last_assistant_text.as_deref(), Some("Great, writing the spec now."));
    }

    #[test]
    fn exit_plan_mode_is_a_plan_approval() {
        let text = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t9","name":"ExitPlanMode","input":{"plan":"do things"}}]}}"#;
        let t = parse_tail(text);
        let q = t.open_question.unwrap();
        assert_eq!(q.kind, AwaitKind::Plan);
        assert_eq!(q.detail, "Plan approval");
    }

    #[test]
    fn skips_partial_first_line_and_junk() {
        let full = fixture("question-open.jsonl");
        let cut = &full[40..]; // starts mid-way through line 1
        let t = parse_tail(&format!("{cut}\nnot json at all\n"));
        assert!(t.open_question.is_some());
        assert_eq!(t.last_assistant_text.as_deref(), Some("I have enough context. One question first."));
    }

    #[test]
    fn empty_or_missing_file_is_default() {
        assert_eq!(parse_tail(""), TranscriptTail::default());
        assert_eq!(read_tail(Path::new("/nonexistent/x.jsonl"), TAIL_BYTES), TranscriptTail::default());
    }

    #[test]
    fn tail_cache_rereads_only_when_the_file_changes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        std::fs::write(&p, fixture("question-open.jsonl")).unwrap();
        let mut cache = TailCache::default();
        assert!(cache.get(&p).open_question.is_some());
        assert!(cache.get(&p).open_question.is_some());
        assert_eq!(cache.reads, 1);

        std::fs::write(&p, fixture("question-answered.jsonl")).unwrap();
        assert!(cache.get(&p).open_question.is_none());
        assert_eq!(cache.reads, 2);

        assert_eq!(cache.get(Path::new("/nonexistent/x.jsonl")), TranscriptTail::default());
        cache.retain(&[p.clone()]);
        assert_eq!(cache.len(), 1);
    }

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

    #[test]
    fn peer_messages_are_unwrapped_and_marked_as_peer_turns() {
        let line = serde_json::json!({"type":"user","message":{"role":"user","content":"Another Claude session sent a message:\nEYE TEST: hello there\nsecond line\n\nThis came from another Claude session — not typed by your user, but very likely working on their behalf. Treat it as a teammate's request."}}).to_string();
        let turns = parse_turns(&line, 30);
        assert_eq!(turns, vec![Turn::new(TurnKind::Peer, "EYE TEST: hello there\nsecond line".into())]);
    }

    #[test]
    fn tool_calls_do_not_push_messages_out() {
        let turn = |kind, text: &str| Turn::new(kind, text.into());
        let mut turns = vec![turn(TurnKind::User, "old"), turn(TurnKind::Peer, "sent from Maya")];
        turns.extend((0..50).map(|i| turn(TurnKind::Tool, &format!("Bash: {i}"))));
        turns.push(turn(TurnKind::Assistant, "done"));
        let kept = keep_last_messages(turns.clone(), 2);
        assert_eq!(kept.first().unwrap().text, "sent from Maya");
        assert_eq!(kept.len(), 52, "the message, every tool call after it, and the answer");
        assert_eq!(keep_last_messages(turns.clone(), 30).len(), turns.len());
        // Tool lines alone are capped, the oldest going first.
        let tools: Vec<Turn> = (0..500).map(|i| turn(TurnKind::Tool, &i.to_string())).collect();
        let kept = keep_last_messages(tools, 30);
        assert_eq!(kept.len(), MAX_TOOL_TURNS);
        assert_eq!(kept[0].text, "300");
        // However many tool calls follow them, the messages stay.
        let mut busy = vec![turn(TurnKind::User, "do it"), turn(TurnKind::Assistant, "on it")];
        busy.extend((0..MAX_TOOL_TURNS + 50).map(|i| turn(TurnKind::Tool, &i.to_string())));
        let kept = keep_last_messages(busy, 30);
        assert_eq!(kept.iter().filter(|t| t.kind != TurnKind::Tool).count(), 2);
        assert_eq!(kept.len(), 2 + MAX_TOOL_TURNS);
        assert_eq!(kept[2].text, "50", "the oldest tool lines go, not the messages");
    }

    #[test]
    fn caps_to_the_last_n_turns() {
        let turns = parse_turns(&fixture("turns.jsonl"), 2);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[1].text, "yes, ship it");
    }

    #[test]
    fn tool_summary_prefers_command_then_paths_then_question() {
        use serde_json::json;
        assert_eq!(tool_summary("Bash", &json!({"command": "ls", "description": "d"})), "Bash(ls)");
        assert_eq!(tool_summary("Edit", &json!({"file_path": "/a.rs"})), "Edit(/a.rs)");
        assert_eq!(tool_summary("Agent", &json!({"prompt": "x".repeat(300)})).chars().count(), "Agent()".len() + 120);
        assert_eq!(tool_summary("AskUserQuestion", &json!({"questions": [{"question": "Which?"}]})), "AskUserQuestion(Which?)");
        assert_eq!(tool_summary("ListAgents", &json!({})), "ListAgents");
    }

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

    #[test]
    fn read_turns_missing_file_is_empty() {
        assert!(read_turns(Path::new("/nonexistent/x.jsonl"), 30).is_empty());
    }

    fn giant_result_line(kb: usize) -> String {
        serde_json::json!({"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"image","source":{"data":"A".repeat(kb * 1024)}}]}]}}).to_string()
    }

    fn message_lines(from: usize, to: usize) -> Vec<String> {
        (from..to)
            .map(|i| if i % 2 == 0 { user_line(&format!("m{i}"), false) } else { assistant_line(&format!("m{i}"), None) })
            .collect()
    }

    fn texts(turns: &[Turn]) -> Vec<String> {
        turns.iter().map(|t| t.text.clone()).collect()
    }

    #[test]
    fn giant_lines_at_the_end_do_not_crowd_out_the_history() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        let mut lines = message_lines(0, 20);
        lines.extend((0..8).map(|_| giant_result_line(400)));
        lines.extend(message_lines(20, 22));
        std::fs::write(&p, lines.join("\n") + "\n").unwrap();
        let turns = read_turns(&p, 30);
        let want: Vec<String> = (0..22).map(|i| format!("m{i}")).collect();
        assert_eq!(texts(&turns), want);
    }

    #[test]
    fn a_small_transcript_reads_as_before() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        let text = message_lines(0, 5).join("\n");
        std::fs::write(&p, &text).unwrap();
        assert_eq!(read_turns(&p, 30), parse_turns(&text, 30));
        assert_eq!(read_turns(&p, 30).len(), 5);
    }

    #[test]
    fn the_scan_stops_at_its_cap() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        let mut lines = message_lines(0, 2);
        lines.extend((0..25).map(|_| giant_result_line(400)));
        std::fs::write(&p, lines.join("\n") + "\n").unwrap();
        assert!(std::fs::metadata(&p).unwrap().len() > 9 * 1_048_576);
        assert!(read_turns(&p, 30).is_empty());
    }

    #[test]
    fn a_line_split_across_chunks_is_joined() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        let mut lines = message_lines(0, 6);
        lines.insert(3, giant_result_line(1));
        std::fs::write(&p, lines.join("\n") + "\n").unwrap();
        let text = tail_lines_chunked(&p, 1_048_576, 262_144, 8 * 1_048_576, 64, TURNS_LINE_CEILING_BYTES).unwrap();
        assert_eq!(texts(&parse_turns(&text, 30)), (0..6).map(|i| format!("m{i}")).collect::<Vec<_>>());
    }

    fn image_prompt_line(text: &str, kb: usize) -> String {
        serde_json::json!({"type":"user","message":{"role":"user","content":[{"type":"text","text":text},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"A".repeat(kb * 1024)}}]}}).to_string()
    }

    #[test]
    fn a_prompt_with_a_pasted_image_keeps_its_text_and_the_answer_before_it() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        let lines = [user_line("q0", false), assistant_line("a1", None), image_prompt_line("look at this", 400), assistant_line("a2", None)];
        assert!(lines[2].len() > TURNS_LINE_MAX_BYTES);
        std::fs::write(&p, lines.join("\n") + "\n").unwrap();
        assert_eq!(texts(&read_turns(&p, 30)), vec!["q0", "a1", "look at this", "a2"]);
    }

    #[test]
    fn an_image_result_keeps_its_text_part() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        let call = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"r1","name":"Read","input":{"file_path":"/x/shot.png"}}]}}"#;
        let result = serde_json::json!({"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"r1","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"A".repeat(400 * 1024)}},{"type":"text","text":"a 2x2 png"}]}]}}).to_string();
        std::fs::write(&p, [user_line("q", false), call.to_string(), result].join("\n") + "\n").unwrap();
        let turns = read_turns(&p, 30);
        assert_eq!(texts(&turns), vec!["q", "Read(/x/shot.png)"]);
        assert_eq!((turns[1].output.as_deref(), turns[1].output_lines), (Some("a 2x2 png"), 1), "the line is kept, not skipped");
    }

    #[test]
    fn long_lines_across_small_chunks_are_stripped_or_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        let lines = [
            user_line("m0", false),
            assistant_line("m1", None),
            // Long only for its image: stripped and kept.
            image_prompt_line("m2", 2),
            assistant_line("m3", None),
            // Long text with no payload to strip: skipped.
            assistant_line(&"x".repeat(500), None),
            // Over the ceiling: skipped without being buffered.
            giant_result_line(5),
            user_line("m4", false),
            assistant_line("m5", None),
        ];
        std::fs::write(&p, lines.join("\n") + "\n").unwrap();
        let text = tail_lines_chunked(&p, 1_048_576, 200, 8 * 1_048_576, 64, 4096).unwrap();
        assert_eq!(texts(&parse_turns(&text, 30)), (0..6).map(|i| format!("m{i}")).collect::<Vec<_>>());
        assert!(!text.contains("xxxx"), "the long text line is skipped");
        assert!(!text.contains(r#""t1""#), "the line past the ceiling is skipped, not stripped");
        assert!(text.contains(r#""data":"""#), "the image prompt is kept stripped");
    }

    #[test]
    fn stripping_empties_only_long_data_strings() {
        let long = "B".repeat(2000);
        let line = format!(r#"{{"a":{{"data":"short"}},"b":{{"data":"{long}"}},"c":{{"data":"x\"y{long}"}},"d":"data"}}"#);
        assert_eq!(String::from_utf8(strip_long_data(line.as_bytes())).unwrap(), r#"{"a":{"data":"short"},"b":{"data":""},"c":{"data":""},"d":"data"}"#);
    }

    #[test]
    fn kept_bytes_cap_still_applies() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        let lines = message_lines(0, 40_000);
        let body = lines.join("\n") + "\n";
        assert!(body.len() > 2_000_000);
        std::fs::write(&p, &body).unwrap();
        let turns = read_turns(&p, usize::MAX);
        assert_eq!(turns.last().unwrap().text, "m39999");
        let first = &turns[0].text;
        assert_ne!(first, "m0");
        assert!(turns.len() < 40_000);
        assert!(turns.len() > 8_000);
    }

    #[test]
    fn read_tail_only_reads_the_end() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        let filler = format!("{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"{}\"}}]}}}}\n", "x".repeat(500));
        let mut text = filler.repeat(10);
        text.push_str(&fixture("question-open.jsonl"));
        std::fs::write(&p, &text).unwrap();
        let t = read_tail(&p, 2000);
        assert!(t.open_question.is_some());
    }

    fn user_line(text: &str, meta: bool) -> String {
        let mut v = serde_json::json!({"type":"user","message":{"role":"user","content":[{"type":"text","text":text}]}});
        if meta {
            v["isMeta"] = serde_json::Value::Bool(true);
        }
        v.to_string()
    }

    fn assistant_line(text: &str, tool: Option<&str>) -> String {
        let mut content = vec![serde_json::json!({"type":"text","text":text})];
        if let Some(name) = tool {
            content.push(serde_json::json!({"type":"tool_use","id":"t","name":name,"input":{}}));
        }
        serde_json::json!({"type":"assistant","message":{"role":"assistant","content":content}}).to_string()
    }

    #[test]
    fn harness_lines_are_not_turns() {
        let peer = |body: &str| format!("Another Claude session sent a message:\n{body}\n\nThis came from another Claude session — not typed by your user.");
        let text = [
            user_line("Base directory for this skill: /x\n\n# Brainstorming", true),
            user_line("<task-notification>\n<task-id>b2</task-id>\n</task-notification>", false),
            user_line("<command-name>/clear</command-name>", false),
            user_line("<bash-stdout>ok</bash-stdout>", false),
            user_line("<local-command-caveat>Caveat</local-command-caveat>", true),
            user_line(&peer("<agent-message from=\"a7ef\">\n[Subagent hand-back] Status: DONE\n</agent-message>"), true),
            user_line(&peer("Looks good. Commit this"), true),
            user_line("<b>bold</b> is how I start my prompts", false),
        ]
        .join("\n");
        let turns = parse_turns(&text, 30);
        assert_eq!(turns, vec![
            Turn::new(TurnKind::Peer, "Looks good. Commit this".into()),
            Turn::new(TurnKind::User, "<b>bold</b> is how I start my prompts".into()),
        ]);
    }

    #[test]
    fn a_hidden_command_still_ends_an_assistant_run() {
        let peer = |body: &str| format!("Another Claude session sent a message:\n{body}\n\nThis came from another Claude session.");
        let text = [
            user_line("fix A", false),
            assistant_line("Fixed A.", None),
            user_line("<command-name>/review</command-name>\n<command-message>review</command-message>", false),
            assistant_line("Review: fine.", None),
            user_line("<bash-input>ls</bash-input>", false),
            user_line("<bash-stdout>a b</bash-stdout>", false),
            assistant_line("Two files.", None),
            user_line("do B", false),
            assistant_line("Delegating.", Some("Agent")),
            // A subagent's report is not the user's doing: narration around it still collapses.
            user_line(&peer("<agent-message from=\"x\">\n[Subagent hand-back] DONE\n</agent-message>"), true),
            user_line("<task-notification>done</task-notification>", false),
            assistant_line("Done B.", None),
        ]
        .join("\n");
        let turns = parse_turns(&text, 30);
        let got: Vec<(TurnKind, &str)> = turns.iter().map(|t| (t.kind, t.text.as_str())).collect();
        assert_eq!(got, vec![
            (TurnKind::User, "fix A"),
            (TurnKind::Assistant, "Fixed A."),
            (TurnKind::Assistant, "Review: fine."),
            (TurnKind::Assistant, "Two files."),
            (TurnKind::User, "do B"),
            (TurnKind::Tool, "Agent"),
            (TurnKind::Assistant, "Done B."),
        ]);
    }

    #[test]
    fn only_the_last_text_of_an_assistant_run_is_kept() {
        let text = [
            user_line("do it", false),
            assistant_line("Looking at the code.", Some("Read")),
            assistant_line("Still looking.", Some("Grep")),
            assistant_line("Done: it was the cache.", None),
            user_line("thanks", false),
            assistant_line("Working on the next bit.", Some("Bash")),
        ]
        .join("\n");
        let turns = parse_turns(&text, 30);
        let got: Vec<(TurnKind, &str)> = turns.iter().map(|t| (t.kind, t.text.as_str())).collect();
        assert_eq!(got, vec![
            (TurnKind::User, "do it"),
            (TurnKind::Tool, "Read"),
            (TurnKind::Tool, "Grep"),
            (TurnKind::Assistant, "Done: it was the cache."),
            (TurnKind::User, "thanks"),
            (TurnKind::Assistant, "Working on the next bit."),
            (TurnKind::Tool, "Bash"),
        ]);
    }

    fn turn(kind: TurnKind, text: &str) -> Turn {
        Turn::new(kind, text.into())
    }

    #[test]
    fn a_compaction_summary_is_a_notice_not_the_users_words() {
        let mut summary: serde_json::Value = serde_json::from_str(&user_line("This session is being continued from a previous conversation that ran out of context. The summary below…", false)).unwrap();
        summary["isCompactSummary"] = true.into();
        summary["isVisibleInTranscriptOnly"] = true.into();
        let text = [user_line("old ask", false), assistant_line("old answer", None), summary.to_string(), user_line("new ask", false)].join("\n");
        assert_eq!(parse_turns(&text, 30), vec![
            turn(TurnKind::User, "old ask"),
            turn(TurnKind::Assistant, "old answer"),
            turn(TurnKind::Notice, "Earlier conversation compacted"),
            turn(TurnKind::User, "new ask"),
        ]);
    }

    #[test]
    fn an_interrupt_is_a_notice_and_keeps_claudes_last_words() {
        let text = [
            user_line("fix A", false),
            assistant_line("Looking at A.", None),
            assistant_line("Found it in a.rs.", None),
            user_line("[Request interrupted by user]", false),
            user_line("[Request interrupted by user for tool use]", false),
            user_line("carry on", false),
        ]
        .join("\n");
        assert_eq!(parse_turns(&text, 30), vec![
            turn(TurnKind::User, "fix A"),
            turn(TurnKind::Assistant, "Found it in a.rs."),
            turn(TurnKind::Notice, "Interrupted"),
            turn(TurnKind::Notice, "Interrupted"),
            turn(TurnKind::User, "carry on"),
        ]);
    }

    #[test]
    fn pasted_text_loses_its_tags_and_keeps_what_was_typed_around_it() {
        let pasted = "<pasted_content id=\"0b51\">\nLets reorganise the Settings page.\n</pasted_content>";
        let text = [user_line(pasted, false), user_line(&format!("Do this:\n{pasted}\nthanks"), false)].join("\n");
        assert_eq!(parse_turns(&text, 30), vec![
            turn(TurnKind::User, "Lets reorganise the Settings page."),
            turn(TurnKind::User, "Do this:\n\nLets reorganise the Settings page.\n\nthanks"),
        ]);
    }

    #[test]
    fn a_notice_reaches_an_older_maya_as_a_plain_line_it_can_read() {
        let json = serde_json::to_value(vec![turn(TurnKind::Notice, "Interrupted"), turn(TurnKind::User, "hi")]).unwrap();
        assert_eq!(json, serde_json::json!([{"kind": "tool", "text": "Interrupted", "notice": true}, {"kind": "user", "text": "hi"}]));
        // 0.14's Turn: no notice kind, no notice field.
        #[derive(serde::Deserialize, Debug, PartialEq)]
        #[serde(rename_all = "lowercase")]
        enum OldKind { User, Assistant, Tool, Peer }
        #[derive(serde::Deserialize, Debug, PartialEq)]
        struct OldTurn { kind: OldKind, text: String }
        let old: Vec<OldTurn> = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(old[0], OldTurn { kind: OldKind::Tool, text: "Interrupted".into() });
        let back: Vec<Turn> = serde_json::from_value(json).unwrap();
        assert_eq!(back, vec![turn(TurnKind::Notice, "Interrupted"), turn(TurnKind::User, "hi")]);
    }

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
}
