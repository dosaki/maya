use crate::launch::shell_single_quote;
use crate::state::truncate;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// How much of a transcript is read for its title: the head holds the first
/// prompt, the tail the latest title records.
const HEAD_BYTES: u64 = 65_536;
const TAIL_BYTES: u64 = 262_144;

/// A past session in a project folder that its agent can resume.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
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

use crate::model::Harness;
use crate::transcript::{Turn, TurnKind};

/// Where each agent keeps its sessions on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentDirs {
    pub claude: PathBuf,
    pub codex: PathBuf,
    pub agy: PathBuf,
    pub grok: PathBuf,
    pub kiro: PathBuf,
}

fn store_for(claude_dir: &Path, dir: &str) -> PathBuf {
    claude_dir.join("projects").join(crate::registry::project_dir_name(dir))
}

fn mtime_ms(path: &Path) -> Option<u64> {
    Some(std::fs::metadata(path).ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as u64)
}

/// The first `bytes` of `path` as text.
fn head(path: &Path, bytes: u64) -> String {
    std::fs::File::open(path)
        .ok()
        .map(|f| {
            use std::io::Read;
            let mut buf = Vec::new();
            let _ = f.take(bytes).read_to_end(&mut buf);
            String::from_utf8_lossy(&buf).into_owned()
        })
        .unwrap_or_default()
}

fn head_and_tail(path: &Path) -> String {
    let tail = crate::transcript::tail_text(path, TAIL_BYTES).unwrap_or_default();
    format!("{}\n{tail}", head(path, HEAD_BYTES))
}

/// The first line of the first user turn, as a title.
fn first_prompt(turns: &[Turn]) -> Option<String> {
    turns.iter().find(|t| t.kind == TurnKind::User).and_then(|t| t.text.lines().map(str::trim).find(|l| !l.is_empty()).map(|l| truncate(l, 120)))
}

/// Claude Code's transcripts for `dir`. Only top-level ones count:
/// subfolders hold subagent runs.
fn claude_sessions(claude_dir: &Path, dir: &str) -> Vec<ResumableSession> {
    let Ok(entries) = std::fs::read_dir(store_for(claude_dir, dir)) else { return vec![] };
    entries
        .flatten()
        .filter(|e| e.path().is_file() && e.path().extension().map_or(false, |x| x == "jsonl"))
        .filter_map(|e| {
            let path = e.path();
            let id = path.file_stem()?.to_str()?.to_string();
            let last_active_ms = mtime_ms(&path)?;
            let title = title_from(&head_and_tail(&path)).unwrap_or_else(|| id.clone());
            Some(ResumableSession { id, title, last_active_ms, running: false })
        })
        .collect()
}

/// Codex's rollouts whose recorded working directory is `dir`, named from
/// `session_index.jsonl`, else by their first prompt.
fn codex_sessions(codex_dir: &Path, dir: &str) -> Vec<ResumableSession> {
    let index = std::fs::read_to_string(codex_dir.join("session_index.jsonl")).unwrap_or_default();
    crate::codex::rollouts(codex_dir)
        .into_iter()
        .filter_map(|path| {
            let (id, cwd) = crate::codex::rollout_meta(&path)?;
            if cwd != dir {
                return None;
            }
            let last_active_ms = mtime_ms(&path)?;
            let title = crate::codex::thread_name(&index, &id).or_else(|| first_prompt(&crate::codex::parse_turns(&head(&path, HEAD_BYTES), usize::MAX))).unwrap_or_else(|| id.clone());
            Some(ResumableSession { id, title, last_active_ms, running: false })
        })
        .collect()
}

/// Antigravity's conversations with `dir` as workspace, from `history.jsonl`,
/// last active at their latest entry.
fn antigravity_sessions(agy_dir: &Path, dir: &str) -> Vec<ResumableSession> {
    let history = std::fs::read_to_string(agy_dir.join("history.jsonl")).unwrap_or_default();
    let mut seen: Vec<(String, u64)> = Vec::new();
    for v in history.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
        let (Some(id), Some(ws)) = (v["conversationId"].as_str(), v["workspace"].as_str()) else { continue };
        if ws != dir {
            continue;
        }
        let ts = v["timestamp"].as_u64().unwrap_or(0);
        match seen.iter_mut().find(|(i, _)| i == id) {
            Some(e) => e.1 = e.1.max(ts),
            None => seen.push((id.to_string(), ts)),
        }
    }
    seen.into_iter()
        .map(|(id, ts)| ResumableSession { title: crate::antigravity::conversation_name(&history, &id).unwrap_or_else(|| id.clone()), id, last_active_ms: ts, running: false })
        .collect()
}

/// Grok's sessions under `sessions/<encoded dir>/`, named from `summary.json`.
fn grok_sessions(grok_dir: &Path, dir: &str) -> Vec<ResumableSession> {
    let folder = grok_dir.join("sessions").join(crate::grok::encode_cwd(dir));
    let Ok(entries) = std::fs::read_dir(folder) else { return vec![] };
    entries
        .flatten()
        .filter_map(|e| {
            let id = e.file_name().to_str()?.to_string();
            let summary_path = e.path().join("summary.json");
            let summary = std::fs::read_to_string(&summary_path).ok()?;
            let v: Value = serde_json::from_str(&summary).ok()?;
            let last_active_ms = v["updated_at"].as_str().and_then(crate::codex::ms_of).or_else(|| mtime_ms(&summary_path))?;
            Some(ResumableSession { title: crate::grok::title(&summary).unwrap_or_else(|| id.clone()), id, last_active_ms, running: false })
        })
        .collect()
}

/// The agent's sessions recorded for `dir`, newest first, running ones marked.
pub fn list_sessions(agent: Harness, dirs: &AgentDirs, dir: &str, running_ids: &[String]) -> Vec<ResumableSession> {
    let mut out = match agent {
        Harness::ClaudeCode => claude_sessions(&dirs.claude, dir),
        Harness::Codex => codex_sessions(&dirs.codex, dir),
        Harness::Antigravity => antigravity_sessions(&dirs.agy, dir),
        Harness::Grok => grok_sessions(&dirs.grok, dir),
        Harness::Kiro | Harness::Other => vec![],
    };
    for s in &mut out {
        s.running = running_ids.contains(&s.id);
    }
    out.sort_by(|a, b| b.last_active_ms.cmp(&a.last_active_ms));
    out
}

/// A session id safe to put in a shell line and a path: letters, digits,
/// `-` and `_`, at most 128 bytes.
pub fn plain_session_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The shell line that resumes `id` inside `dir` with `agent`.
pub fn resume_command(agent: Harness, dir: &Path, id: &str) -> String {
    let d = shell_single_quote(&dir.to_string_lossy());
    let i = shell_single_quote(id);
    match agent {
        Harness::ClaudeCode => format!("cd {d} && claude --resume {i}"),
        Harness::Codex => format!("cd {d} && codex resume {i}"),
        Harness::Antigravity => format!("cd {d} && agy --conversation {i}"),
        Harness::Grok => format!("cd {d} && grok -r {i}"),
        Harness::Kiro => format!("cd {d} && kiro-cli chat --resume-id {i}"),
        Harness::Other => format!("cd {d} && echo 'Maya does not know that agent.'"),
    }
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
        std::fs::OpenOptions::new().write(true).open(&p).unwrap().set_modified(when).unwrap();
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
        let dirs = AgentDirs { claude: claude.to_path_buf(), codex: PathBuf::new(), agy: PathBuf::new(), grok: PathBuf::new(), kiro: PathBuf::new() };
        let list = list_sessions(Harness::ClaudeCode, &dirs, "/Users/x/dev/eye", &["ccc".to_string()]);
        assert_eq!(list.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["bbb", "ccc", "aaa"]);
        assert_eq!(list[0].title, "Fresh");
        assert_eq!(list[1].title, "ccc", "untitled sessions fall back to their id");
        assert!(list[1].running);
        assert!(!list[0].running);
        assert!(list[0].last_active_ms > list[2].last_active_ms);
        assert!(list_sessions(Harness::ClaudeCode, &dirs, "/Users/x/dev/nothing", &[]).is_empty());
    }

    use crate::model::Harness;

    fn dirs_in(t: &Path) -> AgentDirs {
        AgentDirs { claude: t.join("claude"), codex: t.join("codex"), agy: t.join("agy"), grok: t.join("grok"), kiro: t.join("kiro") }
    }

    #[test]
    fn codex_sessions_of_a_folder_come_from_its_rollouts_and_index() {
        let t = tempfile::tempdir().unwrap();
        let d = dirs_in(t.path());
        let day = d.codex.join("sessions/2026/10/03");
        std::fs::create_dir_all(&day).unwrap();
        let meta = |id: &str, cwd: &str| format!("{{\"timestamp\":\"2026-10-03T05:15:41.197Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"{cwd}\",\"originator\":\"codex-tui\"}}}}\n{{\"timestamp\":\"2026-10-03T05:15:42.000Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"Say hi please\"}}]}}}}\n");
        std::fs::write(day.join("rollout-2026-10-03T06-15-39-aaaa.jsonl"), meta("aaaa", "/p/maya")).unwrap();
        std::fs::write(day.join("rollout-2026-10-03T06-16-39-bbbb.jsonl"), meta("bbbb", "/p/other")).unwrap();
        std::fs::write(day.join("rollout-2026-10-03T06-17-39-cccc.jsonl"), meta("cccc", "/p/maya")).unwrap();
        std::fs::write(d.codex.join("session_index.jsonl"), "{\"id\":\"aaaa\",\"thread_name\":\"Testing Codex\",\"updated_at\":\"2026-10-03T06:20:00Z\"}\n").unwrap();
        let list = list_sessions(Harness::Codex, &d, "/p/maya", &["cccc".to_string()]);
        let ids: Vec<&str> = list.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"aaaa") && ids.contains(&"cccc") && !ids.contains(&"bbbb"), "{ids:?}");
        let a = list.iter().find(|s| s.id == "aaaa").unwrap();
        assert_eq!(a.title, "Testing Codex");
        assert!(!a.running);
        let c = list.iter().find(|s| s.id == "cccc").unwrap();
        assert_eq!(c.title, "Say hi please", "no index name: the first prompt");
        assert!(c.running);
    }

    #[test]
    fn antigravity_sessions_of_a_folder_come_from_history() {
        let t = tempfile::tempdir().unwrap();
        let d = dirs_in(t.path());
        std::fs::create_dir_all(&d.agy).unwrap();
        std::fs::write(
            d.agy.join("history.jsonl"),
            concat!(
                "{\"display\":\"Say hello\",\"timestamp\":1790671434791,\"workspace\":\"/p/maya\",\"conversationId\":\"c-1\"}\n",
                "{\"display\":\"/rename Testing AGY\",\"timestamp\":1790671500000,\"workspace\":\"/p/maya\",\"conversationId\":\"c-1\",\"type\":\"slash_command\"}\n",
                "{\"display\":\"Other work\",\"timestamp\":1790671600000,\"workspace\":\"/p/other\",\"conversationId\":\"c-2\"}\n",
                "{\"display\":\"/context\",\"timestamp\":1790671700000,\"workspace\":\"/p/maya\",\"type\":\"slash_command\"}\n",
            ),
        )
        .unwrap();
        let list = list_sessions(Harness::Antigravity, &d, "/p/maya", &[]);
        assert_eq!(list.len(), 1, "{list:?}");
        assert_eq!((list[0].id.as_str(), list[0].title.as_str(), list[0].last_active_ms), ("c-1", "Testing AGY", 1790671500000));
    }

    #[test]
    fn grok_sessions_of_a_folder_come_from_summaries_newest_first() {
        let t = tempfile::tempdir().unwrap();
        let d = dirs_in(t.path());
        let folder = d.grok.join("sessions").join(crate::grok::encode_cwd("/p/maya"));
        for (id, title, at) in [("g-old", "First probe", "2026-10-01T10:00:00.000000Z"), ("g-new", "Maya probe Grok", "2026-10-02T20:24:44.754218Z")] {
            std::fs::create_dir_all(folder.join(id)).unwrap();
            std::fs::write(folder.join(id).join("summary.json"), format!("{{\"info\":{{\"id\":\"{id}\",\"cwd\":\"/p/maya\"}},\"session_summary\":\"{title}\",\"updated_at\":\"{at}\"}}")).unwrap();
        }
        std::fs::create_dir_all(folder.join("no-summary")).unwrap();
        let list = list_sessions(Harness::Grok, &d, "/p/maya", &[]);
        assert_eq!(list.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["g-new", "g-old"]);
        assert_eq!(list[0].title, "Maya probe Grok");
        assert!(list[0].last_active_ms > list[1].last_active_ms);
        assert!(list_sessions(Harness::Grok, &d, "/p/elsewhere", &[]).is_empty());
    }

    #[test]
    fn resume_commands_per_agent_quote_folder_and_id() {
        let dir = Path::new("/Users/x/dev/it's");
        assert_eq!(resume_command(Harness::ClaudeCode, dir, "abc-123"), "cd '/Users/x/dev/it'\\''s' && claude --resume 'abc-123'");
        assert_eq!(resume_command(Harness::Codex, dir, "abc-123"), "cd '/Users/x/dev/it'\\''s' && codex resume 'abc-123'");
        assert_eq!(resume_command(Harness::Antigravity, dir, "abc-123"), "cd '/Users/x/dev/it'\\''s' && agy --conversation 'abc-123'");
        assert_eq!(resume_command(Harness::Grok, dir, "abc-123"), "cd '/Users/x/dev/it'\\''s' && grok -r 'abc-123'");
    }

    #[test]
    fn plain_session_ids_only() {
        assert!(plain_session_id("01a0fc3b-e7fe-7963-94c8-509ae7a10285"));
        assert!(plain_session_id("abc_DEF-9"));
        assert!(!plain_session_id(""));
        assert!(!plain_session_id("../x"));
        assert!(!plain_session_id("a/b"));
        assert!(!plain_session_id("it's"));
        assert!(!plain_session_id(&"x".repeat(129)));
    }
}
