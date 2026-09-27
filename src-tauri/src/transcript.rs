use crate::model::AwaitKind;
use serde_json::Value;
use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub const TAIL_BYTES: u64 = 262_144;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenQuestion {
    pub kind: AwaitKind,
    pub detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TranscriptTail {
    pub last_assistant_text: Option<String>,
    pub open_question: Option<OpenQuestion>,
}

/// Reads at most `max_bytes` from the end of `path` and parses it.
pub fn read_tail(path: &Path, max_bytes: u64) -> TranscriptTail {
    let Ok(mut file) = std::fs::File::open(path) else { return TranscriptTail::default() };
    let Ok(meta) = file.metadata() else { return TranscriptTail::default() };
    let start = meta.len().saturating_sub(max_bytes);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return TranscriptTail::default();
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return TranscriptTail::default();
    }
    let text = String::from_utf8_lossy(&bytes);
    if start > 0 {
        // Drop the first, possibly partial, line.
        match text.find('\n') {
            Some(i) => parse_tail(&text[i + 1..]),
            None => TranscriptTail::default(),
        }
    } else {
        parse_tail(&text)
    }
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

    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let kind = v["type"].as_str().unwrap_or("");
        let Some(content) = v["message"]["content"].as_array() else { continue };
        match kind {
            "assistant" => {
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
                                    open.push((id, OpenQuestion { kind: AwaitKind::Question, detail: q }));
                                }
                                Some("ExitPlanMode") => {
                                    open.push((id, OpenQuestion { kind: AwaitKind::Plan, detail: "Plan approval".to_string() }));
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
    TranscriptTail { last_assistant_text: last_text, open_question }
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
