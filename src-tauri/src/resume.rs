use crate::launch::shell_single_quote;
use crate::state::truncate;
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// How much of a transcript is read for its title: the head holds the first
/// prompt, the tail the latest title records.
const HEAD_BYTES: u64 = 65_536;
const TAIL_BYTES: u64 = 262_144;

/// A past session in a project folder that `claude --resume` can pick up.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResumableSession {
    pub id: String,
    pub title: String,
    /// Epoch millis of the transcript's last write.
    pub last_active_ms: u64,
    /// True when a live session already has this id: it cannot be resumed twice.
    pub running: bool,
}

fn first_user_text(v: &Value) -> Option<String> {
    let content = &v["message"]["content"];
    let text = if let Some(s) = content.as_str() {
        s.to_string()
    } else {
        content.as_array()?.iter().find_map(|b| (b["type"] == "text").then(|| b["text"].as_str().map(str::to_string)).flatten())?
    };
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(truncate(line, 120))
}

/// The best title in transcript `text`: the latest custom title (from
/// /rename), else the latest AI title, else the first user prompt.
pub fn title_from(text: &str) -> Option<String> {
    let mut custom = None;
    let mut ai = None;
    let mut prompt = None;
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        match v["type"].as_str() {
            Some("custom-title") => custom = v["customTitle"].as_str().map(str::to_string),
            Some("ai-title") => ai = v["aiTitle"].as_str().map(str::to_string),
            Some("user") if prompt.is_none() => prompt = first_user_text(&v),
            _ => {}
        }
    }
    custom.or(ai).or(prompt).map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

fn store_for(claude_dir: &Path, dir: &str) -> PathBuf {
    claude_dir.join("projects").join(dir.replace('/', "-"))
}

fn head_and_tail(path: &Path) -> String {
    let head = std::fs::File::open(path)
        .ok()
        .map(|f| {
            use std::io::Read;
            let mut buf = Vec::new();
            let _ = f.take(HEAD_BYTES).read_to_end(&mut buf);
            String::from_utf8_lossy(&buf).into_owned()
        })
        .unwrap_or_default();
    let tail = crate::transcript::tail_text(path, TAIL_BYTES).unwrap_or_default();
    format!("{head}\n{tail}")
}

/// The sessions recorded for `dir`, newest first. Only top-level transcripts
/// count: subfolders hold subagent runs.
pub fn list_sessions(claude_dir: &Path, dir: &str, running_ids: &[String]) -> Vec<ResumableSession> {
    let Ok(entries) = std::fs::read_dir(store_for(claude_dir, dir)) else { return vec![] };
    let mut out: Vec<ResumableSession> = entries
        .flatten()
        .filter(|e| e.path().is_file() && e.path().extension().map_or(false, |x| x == "jsonl"))
        .filter_map(|e| {
            let path = e.path();
            let id = path.file_stem()?.to_str()?.to_string();
            let last_active_ms = e.metadata().ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as u64;
            let title = title_from(&head_and_tail(&path)).unwrap_or_else(|| id.clone());
            Some(ResumableSession { running: running_ids.contains(&id), id, title, last_active_ms })
        })
        .collect();
    out.sort_by(|a, b| b.last_active_ms.cmp(&a.last_active_ms));
    out
}

/// True when `id` names a transcript of `dir` (ids are file stems, so a
/// path-like id never matches).
pub fn transcript_exists(claude_dir: &Path, dir: &str, id: &str) -> bool {
    !id.contains('/') && !id.contains("..") && store_for(claude_dir, dir).join(format!("{id}.jsonl")).is_file()
}

/// The shell line that resumes `id` inside `dir`.
pub fn resume_command(dir: &Path, id: &str) -> String {
    format!("cd {} && claude --resume {}", shell_single_quote(&dir.to_string_lossy()), shell_single_quote(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_prefers_custom_then_ai_then_first_prompt_then_nothing() {
        let custom = "{\"type\":\"ai-title\",\"aiTitle\":\"AI one\"}\n{\"type\":\"custom-title\",\"customTitle\":\"first\"}\n{\"type\":\"custom-title\",\"customTitle\":\"renamed later\"}\n";
        assert_eq!(title_from(custom).as_deref(), Some("renamed later"));
        let ai = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"fix the CI\"}}\n{\"type\":\"ai-title\",\"aiTitle\":\"Fixing CI\"}\n";
        assert_eq!(title_from(ai).as_deref(), Some("Fixing CI"));
        let prompt = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"  Add a resume button please  \"}]}}\n";
        assert_eq!(title_from(prompt).as_deref(), Some("Add a resume button please"));
        assert_eq!(title_from("{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"tool_result\",\"content\":\"x\"}]}}\n"), None);
        assert_eq!(title_from(""), None);
        // Long prompts are trimmed to one line of at most 120 characters.
        let long = format!("{{\"type\":\"user\",\"message\":{{\"content\":\"{}\\nsecond line\"}}}}\n", "a".repeat(300));
        let t = title_from(&long).unwrap();
        assert!(t.chars().count() <= 120 && !t.contains('\n'));
    }

    fn write(dir: &std::path::Path, name: &str, body: &str, age_secs: u64) {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        let when = std::time::SystemTime::now() - std::time::Duration::from_secs(age_secs);
        std::fs::File::open(&p).unwrap().set_modified(when).unwrap();
    }

    #[test]
    fn lists_top_level_transcripts_newest_first_marking_running_ones() {
        let t = tempfile::tempdir().unwrap();
        let claude = t.path().to_path_buf();
        let store = claude.join("projects/-Users-x-dev-eye");
        std::fs::create_dir_all(store.join("subagent-dir")).unwrap();
        write(&store, "aaa.jsonl", "{\"type\":\"custom-title\",\"customTitle\":\"Old one\"}\n", 3600);
        write(&store, "bbb.jsonl", "{\"type\":\"ai-title\",\"aiTitle\":\"Fresh\"}\n", 60);
        write(&store, "ccc.jsonl", "", 600);
        write(&store.join("subagent-dir"), "ddd.jsonl", "{\"type\":\"custom-title\",\"customTitle\":\"nested\"}\n", 1);
        std::fs::write(store.join("notes.txt"), "x").unwrap();
        let list = list_sessions(&claude, "/Users/x/dev/eye", &["ccc".to_string()]);
        assert_eq!(list.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["bbb", "ccc", "aaa"]);
        assert_eq!(list[0].title, "Fresh");
        assert_eq!(list[1].title, "ccc", "untitled sessions fall back to their id");
        assert!(list[1].running);
        assert!(!list[0].running);
        assert!(list[0].last_active_ms > list[2].last_active_ms);
        assert!(list_sessions(&claude, "/Users/x/dev/nothing", &[]).is_empty());
    }

    #[test]
    fn resume_command_changes_directory_and_resumes_by_id() {
        assert_eq!(resume_command(std::path::Path::new("/Users/x/dev/it's"), "abc-123"), "cd '/Users/x/dev/it'\\''s' && claude --resume 'abc-123'");
    }

    #[test]
    fn transcript_exists_only_for_a_listed_id() {
        let t = tempfile::tempdir().unwrap();
        let store = t.path().join("projects/-Users-x-dev-eye");
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(store.join("aaa.jsonl"), "").unwrap();
        assert!(transcript_exists(t.path(), "/Users/x/dev/eye", "aaa"));
        assert!(!transcript_exists(t.path(), "/Users/x/dev/eye", "zzz"));
        assert!(!transcript_exists(t.path(), "/Users/x/dev/eye", "../../aaa"));
    }
}
