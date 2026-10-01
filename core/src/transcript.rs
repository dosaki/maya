use crate::model::{parse_questions, AwaitKind, Question};
use serde_json::Value;
use std::collections::HashSet;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TurnKind {
    User,
    Assistant,
    Tool,
    /// A message posted into the session's inbox (by Maya or another session).
    Peer,
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Turn {
    pub kind: TurnKind,
    pub text: String,
}

/// `<name>: <first useful argument>` for a tool_use block, or just the name.
pub fn tool_summary(name: &str, input: &Value) -> String {
    let arg = ["command", "file_path", "path", "prompt", "description"]
        .iter()
        .find_map(|k| input.get(*k).and_then(|v| v.as_str()))
        .or_else(|| input["questions"][0]["question"].as_str());
    match arg {
        Some(a) if !a.trim().is_empty() => format!("{name}: {}", crate::state::truncate(a.trim(), 120)),
        _ => name.to_string(),
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
fn keep_last_messages(mut turns: Vec<Turn>, max_messages: usize) -> Vec<Turn> {
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

/// Conversation turns (user prompts, assistant text, tool calls) in order,
/// keeping the last `max_turns` messages and the tool calls among them.
/// Subagent sidechain lines are skipped.
pub fn parse_turns(text: &str, max_turns: usize) -> Vec<Turn> {
    let mut turns = Vec::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if v["isSidechain"].as_bool() == Some(true) {
            continue;
        }
        let content = &v["message"]["content"];
        match v["type"].as_str() {
            Some("user") => {
                let t = text_blocks(content);
                if !t.is_empty() {
                    let text = t.join("\n\n");
                    match unwrap_peer_message(&text) {
                        Some(inner) => turns.push(Turn { kind: TurnKind::Peer, text: inner }),
                        None => turns.push(Turn { kind: TurnKind::User, text }),
                    }
                }
            }
            Some("assistant") => {
                let t = text_blocks(content);
                if !t.is_empty() {
                    turns.push(Turn { kind: TurnKind::Assistant, text: t.join("\n\n") });
                }
                if let Some(blocks) = content.as_array() {
                    for b in blocks.iter().filter(|b| b["type"] == "tool_use") {
                        let name = b["name"].as_str().unwrap_or("tool");
                        turns.push(Turn { kind: TurnKind::Tool, text: tool_summary(name, &b["input"]) });
                    }
                }
            }
            _ => {}
        }
    }
    keep_last_messages(turns, max_turns)
}

/// Last `max_turns` turns from the last 1 MB of the transcript.
pub fn read_turns(path: &Path, max_turns: usize) -> Vec<Turn> {
    tail_text(path, TURNS_TAIL_BYTES).map(|t| parse_turns(&t, max_turns)).unwrap_or_default()
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
    fn parses_turns_in_order_skipping_tool_results_and_sidechains() {
        let turns = parse_turns(&fixture("turns.jsonl"), 30);
        let got: Vec<(TurnKind, &str)> = turns.iter().map(|t| (t.kind, t.text.as_str())).collect();
        assert_eq!(got, vec![
            (TurnKind::User, "build me a board"),
            (TurnKind::Assistant, "Sure."),
            (TurnKind::Tool, "Bash: pnpm test"),
            (TurnKind::Tool, "ListAgents"),
            (TurnKind::Assistant, "Tests pass.\n\nAnything else?"),
            (TurnKind::User, "yes, ship it"),
        ]);
    }

    #[test]
    fn peer_messages_are_unwrapped_and_marked_as_peer_turns() {
        let line = serde_json::json!({"type":"user","message":{"role":"user","content":"Another Claude session sent a message:\nEYE TEST: hello there\nsecond line\n\nThis came from another Claude session — not typed by your user, but very likely working on their behalf. Treat it as a teammate's request."}}).to_string();
        let turns = parse_turns(&line, 30);
        assert_eq!(turns, vec![Turn { kind: TurnKind::Peer, text: "EYE TEST: hello there\nsecond line".into() }]);
    }

    #[test]
    fn tool_calls_do_not_push_messages_out() {
        let turn = |kind, text: &str| Turn { kind, text: text.into() };
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
        assert_eq!(tool_summary("Bash", &json!({"command": "ls", "description": "d"})), "Bash: ls");
        assert_eq!(tool_summary("Edit", &json!({"file_path": "/a.rs"})), "Edit: /a.rs");
        assert_eq!(tool_summary("Agent", &json!({"prompt": "x".repeat(300)})).chars().count(), "Agent: ".len() + 120);
        assert_eq!(tool_summary("AskUserQuestion", &json!({"questions": [{"question": "Which?"}]})), "AskUserQuestion: Which?");
        assert_eq!(tool_summary("ListAgents", &json!({})), "ListAgents");
    }

    #[test]
    fn read_turns_missing_file_is_empty() {
        assert!(read_turns(Path::new("/nonexistent/x.jsonl"), 30).is_empty());
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
}
