use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HookEvent {
    pub session_id: String,
    pub hook_event_name: String,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_input: Option<serde_json::Value>,
    #[serde(default)]
    pub notification_type: Option<String>,
    #[serde(default)]
    pub transcript_path: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub received_at: u64,
}

pub struct EventLog {
    path: PathBuf,
    offset: u64,
    by_session: HashMap<String, Vec<HookEvent>>,
}

/// True for main-agent events that start a new turn; everything before them
/// is irrelevant to the current state and can be dropped.
fn starts_turn(e: &HookEvent) -> bool {
    e.agent_id.is_none() && matches!(e.hook_event_name.as_str(), "Stop" | "UserPromptSubmit")
}

impl EventLog {
    pub fn new(path: PathBuf) -> Self {
        Self { path, offset: 0, by_session: HashMap::new() }
    }

    pub fn events_for(&self, session_id: &str) -> &[HookEvent] {
        self.by_session.get(session_id).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Current size of the log file on disk, 0 when missing.
    pub fn file_len(&self) -> u64 {
        std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0)
    }

    pub fn transcript_path_for(&self, session_id: &str) -> Option<&str> {
        self.events_for(session_id).iter().rev().find_map(|e| e.transcript_path.as_deref())
    }

    /// Reads lines appended since the last call. A trailing line without `\n`
    /// is left unread. If the file shrank, everything is re-read from zero.
    pub fn read_new(&mut self) -> std::io::Result<usize> {
        let mut file = match std::fs::File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e),
        };
        let len = file.metadata()?.len();
        if len < self.offset {
            self.offset = 0;
            self.by_session.clear();
        }
        file.seek(SeekFrom::Start(self.offset))?;
        let mut buf = String::new();
        file.read_to_string(&mut buf)?;

        let complete_len = match buf.rfind('\n') {
            Some(i) => i + 1,
            None => return Ok(0),
        };
        let mut count = 0;
        for raw in buf[..complete_len].lines() {
            if raw.trim().is_empty() {
                continue;
            }
            if let Ok(ev) = serde_json::from_str::<HookEvent>(raw) {
                let events = self.by_session.entry(ev.session_id.clone()).or_default();
                if starts_turn(&ev) {
                    events.clear();
                }
                events.push(ev);
                count += 1;
            }
        }
        self.offset += complete_len as u64;
        Ok(count)
    }

    /// Rewrites the file keeping only events of `keep` sessions, then re-reads.
    pub fn compact(&mut self, keep: &HashSet<String>) -> std::io::Result<()> {
        self.compact_with(keep, |_| {})
    }

    /// `between` runs just before the rewrite; tests use it to simulate a hook
    /// appending while compaction is in progress.
    pub fn compact_with(&mut self, keep: &HashSet<String>, between: impl FnOnce(&std::path::Path)) -> std::io::Result<()> {
        self.read_new()?;
        let mut kept: Vec<&HookEvent> = self
            .by_session
            .iter()
            .filter(|(sid, _)| keep.contains(*sid))
            .flat_map(|(_, events)| events.iter())
            .collect();
        kept.sort_by_key(|e| e.received_at);
        let mut text = String::new();
        for e in kept {
            if let Ok(line) = serde_json::to_string(e) {
                text.push_str(&line);
                text.push('\n');
            }
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        between(&self.path);
        // Carry over anything a hook appended after our last read (complete or
        // partial lines alike) so the rewrite never drops a live event.
        if let Ok(mut file) = std::fs::File::open(&self.path) {
            if file.seek(SeekFrom::Start(self.offset)).is_ok() {
                let mut late = Vec::new();
                if file.read_to_end(&mut late).is_ok() {
                    text.push_str(&String::from_utf8_lossy(&late));
                }
            }
        }
        std::fs::write(&self.path, text)?;
        self.offset = 0;
        self.by_session.clear();
        self.read_new()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::io::Write;

    fn line(session: &str, event: &str, extra: &str, ts: u64) -> String {
        format!(
            "{{\"session_id\":\"{session}\",\"hook_event_name\":\"{event}\",\"transcript_path\":\"/t/{session}.jsonl\",\"cwd\":\"/x\"{extra},\"received_at\":{ts}}}\n"
        )
    }

    #[test]
    fn missing_file_reads_zero_events() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = EventLog::new(dir.path().join("events.jsonl"));
        assert_eq!(log.read_new().unwrap(), 0);
        assert!(log.events_for("nope").is_empty());
    }

    #[test]
    fn reads_incrementally_and_groups_by_session() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(line("a", "UserPromptSubmit", "", 1).as_bytes()).unwrap();
        f.write_all(line("b", "Stop", "", 2).as_bytes()).unwrap();
        f.flush().unwrap();

        let mut log = EventLog::new(path.clone());
        assert_eq!(log.read_new().unwrap(), 2);
        assert_eq!(log.events_for("a").len(), 1);
        assert_eq!(log.events_for("b")[0].hook_event_name, "Stop");

        f.write_all(line("a", "PreToolUse", ",\"tool_name\":\"AskUserQuestion\",\"tool_input\":{\"questions\":[{\"question\":\"Which?\"}]}", 3).as_bytes()).unwrap();
        f.flush().unwrap();
        assert_eq!(log.read_new().unwrap(), 1);
        let a = log.events_for("a");
        assert_eq!(a.len(), 2);
        assert_eq!(a[1].tool_name.as_deref(), Some("AskUserQuestion"));
        assert_eq!(a[1].tool_input.as_ref().unwrap()["questions"][0]["question"], "Which?");
        assert_eq!(a[1].received_at, 3);
        assert_eq!(log.transcript_path_for("a"), Some("/t/a.jsonl"));
    }

    #[test]
    fn partial_last_line_is_kept_for_next_read_and_bad_lines_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(line("a", "Stop", "", 1).as_bytes()).unwrap();
        f.write_all(b"this is not json\n").unwrap();
        f.write_all(b"{\"session_id\":\"a\",\"hook_event_na").unwrap();
        f.flush().unwrap();

        let mut log = EventLog::new(path.clone());
        assert_eq!(log.read_new().unwrap(), 1);

        f.write_all(b"me\":\"UserPromptSubmit\",\"received_at\":2}\n").unwrap();
        f.flush().unwrap();
        assert_eq!(log.read_new().unwrap(), 1);
        // UserPromptSubmit starts a new turn, so the earlier Stop is trimmed.
        let a: Vec<&str> = log.events_for("a").iter().map(|e| e.hook_event_name.as_str()).collect();
        assert_eq!(a, vec!["UserPromptSubmit"]);
    }

    #[test]
    fn truncated_file_resets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        std::fs::write(&path, line("a", "Stop", "", 1) + &line("a", "Stop", "", 2)).unwrap();
        let mut log = EventLog::new(path.clone());
        assert_eq!(log.read_new().unwrap(), 2);
        std::fs::write(&path, line("b", "Stop", "", 3)).unwrap();
        log.read_new().unwrap();
        assert!(log.events_for("a").is_empty());
        assert_eq!(log.events_for("b").len(), 1);
    }

    #[test]
    fn parses_agent_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        std::fs::write(&path, line("a", "PostToolUse", ",\"agent_id\":\"ag1\"", 1)).unwrap();
        let mut log = EventLog::new(path);
        log.read_new().unwrap();
        assert_eq!(log.events_for("a")[0].agent_id.as_deref(), Some("ag1"));
    }

    #[test]
    fn stop_and_prompt_trim_earlier_events_of_that_session() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        std::fs::write(&path, line("a", "PostToolUse", "", 1) + &line("a", "PostToolUse", "", 2) + &line("a", "Stop", "", 3) + &line("a", "PostToolUse", "", 4) + &line("b", "PostToolUse", "", 5)).unwrap();
        let mut log = EventLog::new(path);
        log.read_new().unwrap();
        let a: Vec<&str> = log.events_for("a").iter().map(|e| e.hook_event_name.as_str()).collect();
        assert_eq!(a, vec!["Stop", "PostToolUse"]);
        assert_eq!(log.events_for("b").len(), 1);
    }

    #[test]
    fn compact_keeps_lines_appended_while_rewriting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        std::fs::write(&path, line("a", "Stop", "", 1) + &line("b", "Stop", "", 2)).unwrap();
        let mut log = EventLog::new(path.clone());
        log.read_new().unwrap();
        let keep: HashSet<String> = ["a".to_string()].into_iter().collect();
        let late = line("a", "PermissionRequest", "", 3);
        log.compact_with(&keep, |p| {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().append(true).open(p).unwrap();
            f.write_all(late.as_bytes()).unwrap();
        })
        .unwrap();
        let a: Vec<&str> = log.events_for("a").iter().map(|e| e.hook_event_name.as_str()).collect();
        assert_eq!(a, vec!["Stop", "PermissionRequest"]);
        assert!(log.events_for("b").is_empty());
    }

    #[test]
    fn file_len_reports_size_and_zero_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let log = EventLog::new(path.clone());
        assert_eq!(log.file_len(), 0);
        std::fs::write(&path, "12345").unwrap();
        assert_eq!(log.file_len(), 5);
    }

    #[test]
    fn compact_keeps_only_live_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        std::fs::write(&path, line("a", "Stop", "", 1) + &line("b", "Stop", "", 2) + &line("a", "UserPromptSubmit", "", 3)).unwrap();
        let mut log = EventLog::new(path.clone());
        log.read_new().unwrap();
        let keep: HashSet<String> = ["a".to_string()].into_iter().collect();
        log.compact(&keep).unwrap();
        // a's Stop was trimmed by its later UserPromptSubmit; b is gone entirely.
        assert_eq!(log.events_for("a").len(), 1);
        assert!(log.events_for("b").is_empty());
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 1);
        assert!(!text.contains("\"session_id\":\"b\""));
    }
}
