use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, PartialEq)]
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
    pub received_at: u64,
}

pub struct EventLog {
    path: PathBuf,
    offset: u64,
    by_session: HashMap<String, Vec<HookEvent>>,
    raw_by_session: HashMap<String, Vec<String>>,
}

impl EventLog {
    pub fn new(path: PathBuf) -> Self {
        Self { path, offset: 0, by_session: HashMap::new(), raw_by_session: HashMap::new() }
    }

    pub fn events_for(&self, session_id: &str) -> &[HookEvent] {
        self.by_session.get(session_id).map(|v| v.as_slice()).unwrap_or(&[])
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
            self.raw_by_session.clear();
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
                self.raw_by_session.entry(ev.session_id.clone()).or_default().push(raw.to_string());
                self.by_session.entry(ev.session_id.clone()).or_default().push(ev);
                count += 1;
            }
        }
        self.offset += complete_len as u64;
        Ok(count)
    }

    /// Rewrites the file keeping only events of `keep` sessions, then re-reads.
    pub fn compact(&mut self, keep: &HashSet<String>) -> std::io::Result<()> {
        self.read_new()?;
        let mut kept: Vec<(u64, &String)> = Vec::new();
        for (sid, raws) in &self.raw_by_session {
            if !keep.contains(sid) {
                continue;
            }
            for (i, raw) in raws.iter().enumerate() {
                let ts = self.by_session[sid][i].received_at;
                kept.push((ts, raw));
            }
        }
        kept.sort_by_key(|(ts, _)| *ts);
        let mut text = String::new();
        for (_, raw) in kept {
            text.push_str(raw);
            text.push('\n');
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.path, text)?;
        self.offset = 0;
        self.by_session.clear();
        self.raw_by_session.clear();
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
        assert_eq!(log.events_for("a")[1].hook_event_name, "UserPromptSubmit");
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
    fn compact_keeps_only_live_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        std::fs::write(&path, line("a", "Stop", "", 1) + &line("b", "Stop", "", 2) + &line("a", "UserPromptSubmit", "", 3)).unwrap();
        let mut log = EventLog::new(path.clone());
        log.read_new().unwrap();
        let keep: HashSet<String> = ["a".to_string()].into_iter().collect();
        log.compact(&keep).unwrap();
        assert_eq!(log.events_for("a").len(), 2);
        assert!(log.events_for("b").is_empty());
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(!text.contains("\"session_id\":\"b\""));
    }
}
