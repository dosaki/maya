//! Judges whether a finished message asks the user for something, with a
//! one-shot call to Maya's agent, and remembers each verdict on disk.

use crate::interpreter::{json_in, output_within, RunError};
use crate::model::Harness;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// What the judge made of a message: whether it asks, and the sentence that does.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub asks: bool,
    pub line: Option<String>,
}

pub const SYSTEM_PROMPT: &str = "You read the last message an AI coding agent wrote to its user when it finished a turn. Decide whether the agent has stopped because it needs something from the user before it can go on: an answer, a decision, an approval, or an action only the user can take. The ask can be anywhere in the message and in any wording. An offer that waits on the user's go-ahead (\"I can fix that if you want\", \"say go and I'll start\") counts.\nIt is not an ask when:\n- the message only reports what was done, found or decided;\n- the agent says it is still working, or watching or waiting on something other than the user, such as a build, a review, a merge or a timer; this holds even when the message also lists things for the user to check or do;\n- the agent lists what the user owns or could do later, such as a test plan, open items or reminders, without stopping for it;\n- a decision is pending elsewhere and the message only mentions it;\n- the agent answers a question and says what comes next without asking for anything.\nWhen in doubt, it is not an ask.\nReply with JSON only, giving your reason first in one short sentence: {\"why\": \"<reason>\", \"asks\": true, \"line\": \"<the sentence that asks, copied exactly from the message>\"} or {\"why\": \"<reason>\", \"asks\": false, \"line\": null}.";

/// How much of the end of a message the judge reads.
const MAX_CHARS: usize = 8_000;
const TIMEOUT: Duration = Duration::from_secs(30);

/// `<session id>:<FNV-1a 64 of the prompt and the text, 16 hex digits>`.
/// The prompt is in the hash so a message is judged again when the prompt
/// changes, instead of keeping a verdict the old prompt gave.
pub fn key(session_id: &str, text: &str) -> String {
    key_for(SYSTEM_PROMPT, session_id, text)
}

fn key_for(prompt: &str, session_id: &str, text: &str) -> String {
    let hash = prompt.bytes().chain([0]).chain(text.bytes()).fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3));
    format!("{session_id}:{hash:016x}")
}

/// Verdicts by `key`, kept in a file so a message is judged once.
pub struct AskCache {
    path: PathBuf,
    verdicts: HashMap<String, Verdict>,
    dirty: bool,
}

impl AskCache {
    /// A missing, unreadable or invalid file is an empty cache.
    pub fn load(path: PathBuf) -> AskCache {
        let verdicts = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        AskCache { path, verdicts, dirty: false }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self, key: &str) -> Option<&Verdict> {
        self.verdicts.get(key)
    }

    pub fn insert(&mut self, key: String, v: Verdict) {
        if self.verdicts.insert(key, v.clone()) != Some(v) {
            self.dirty = true;
        }
    }

    /// Drops the verdicts whose key is not in `live`.
    pub fn retain(&mut self, live: &HashSet<String>) {
        let before = self.verdicts.len();
        self.verdicts.retain(|k, _| live.contains(k));
        self.dirty |= self.verdicts.len() != before;
    }

    /// Writes the file when something changed since the last load or save.
    pub fn save(&mut self) -> std::io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string(&self.verdicts).map_err(std::io::Error::other)?;
        crate::config::write_private_atomic(&self.path, text.as_bytes()).map_err(std::io::Error::other)?;
        self.dirty = false;
        Ok(())
    }
}

fn straight(s: &str) -> String {
    s.replace('’', "'")
}

/// The verdict inside the agent's one-shot output. An ask's line must come
/// from `text`; failing that it is the wording check's, else the last line.
pub fn parse_verdict(agent: Harness, stdout: &str, text: &str) -> Result<Verdict, String> {
    let reply = crate::launch::final_text(agent, stdout)?;
    let v = json_in(&reply).ok_or_else(|| format!("no JSON verdict in: {}", reply.chars().take(120).collect::<String>()))?;
    let asks = v["asks"].as_bool().ok_or("verdict has no boolean asks")?;
    if !asks {
        return Ok(Verdict { asks, line: None });
    }
    let whole = straight(text);
    let given = v["line"].as_str().map(str::trim).filter(|l| !l.is_empty() && whole.contains(&straight(l)));
    let line = given
        .map(str::to_string)
        .or_else(|| crate::state::asking_line(text))
        .or_else(|| text.lines().map(str::trim).rfind(|l| !l.is_empty()).map(str::to_string));
    Ok(Verdict { asks, line })
}

/// Asks `agent` whether `text` asks the user for something. `cwd` is a neutral
/// directory, so no project's instructions or settings load.
pub fn judge(agent: Harness, binary: &Path, model: Option<&str>, text: &str, cwd: &Path) -> Result<Verdict, RunError> {
    let skip = text.chars().count().saturating_sub(MAX_CHARS);
    let tail: String = text.chars().skip(skip).collect();
    let user = format!("Message:\n<<<\n{tail}\n>>>");
    let program = crate::launch::binary_name(agent);
    let mut cmd = crate::command(binary);
    cmd.args(crate::launch::oneshot_args(agent, model, Some(SYSTEM_PROMPT), &user)).current_dir(cwd).env_clear().envs(crate::launch::clean_env(std::env::vars()));
    let started = std::time::Instant::now();
    let out = output_within(program, cmd, TIMEOUT).inspect_err(|e| crate::log::line("asks", format!("failed after {:.1}s: {e}", started.elapsed().as_secs_f32())))?;
    let ms = started.elapsed().as_millis();
    let verdict = parse_verdict(agent, &out, text).map_err(RunError::Failed);
    match &verdict {
        Ok(v) => crate::log::line("asks", format!("verdict in {ms} ms: asks={} line={:?}", v.asks, v.line)),
        Err(e) => crate::log::line("asks", format!("unusable verdict in {ms} ms: {e}\nraw: {}", crate::log::clip(&out, 500))),
    }
    verdict
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(result: &str) -> String {
        serde_json::json!({"type": "result", "is_error": false, "result": result}).to_string()
    }

    #[test]
    fn keys_are_stable_and_change_with_the_text_and_the_prompt() {
        assert_eq!(key("s", "abc"), key("s", "abc"));
        assert_ne!(key("s", "abc"), key("s", "abd"));
        assert_eq!(key("s", "").len(), "s:".len() + 16);
        assert_eq!(key_for("", "s", ""), "s:af63bd4c8601b7df", "the empty prompt and text hash as one zero byte");
        assert_ne!(key_for("p1", "s", "x"), key_for("p2", "s", "x"), "a new prompt judges every message again");
        assert_eq!(key("s", "x"), key_for(SYSTEM_PROMPT, "s", "x"));
    }

    #[test]
    fn the_cache_round_trips_through_its_file_and_forgets_dead_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("asks.json");
        assert_eq!(AskCache::load(path.clone()).get("x"), None, "a missing file is an empty cache");
        let yes = Verdict { asks: true, line: Some("Which one?".into()) };
        let no = Verdict { asks: false, line: None };
        let mut cache = AskCache::load(path.clone());
        cache.insert("a:1".into(), yes.clone());
        cache.insert("b:2".into(), no.clone());
        cache.save().unwrap();
        let mut again = AskCache::load(path.clone());
        assert_eq!((again.get("a:1"), again.get("b:2")), (Some(&yes), Some(&no)));
        again.retain(&["a:1".to_string()].into_iter().collect());
        again.save().unwrap();
        let last = AskCache::load(path.clone());
        assert_eq!((last.get("a:1"), last.get("b:2")), (Some(&yes), None));
        std::fs::remove_file(&path).unwrap();
        let mut idle = AskCache::load(path.clone());
        idle.save().unwrap();
        assert!(!path.exists(), "nothing changed, so nothing is written");
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(AskCache::load(path).get("a:1"), None, "an invalid file is an empty cache");
    }

    #[test]
    fn a_verdict_reads_from_the_agents_reply() {
        let text = "Done with the parser.\nWhich way should I build it?\nThanks.";
        let a = Harness::ClaudeCode;
        let asks = envelope(r#"{"asks":true,"line":"Which way should I build it?"}"#);
        assert_eq!(parse_verdict(a, &asks, text).unwrap(), Verdict { asks: true, line: Some("Which way should I build it?".into()) });
        let no = envelope(r#"{"asks":false,"line":"Which way should I build it?"}"#);
        assert_eq!(parse_verdict(a, &no, text).unwrap(), Verdict { asks: false, line: None }, "no ask has no line");
        let reasoned = envelope(r#"{"why":"it waits on a choice","asks":true,"line":"Which way should I build it?"}"#);
        assert_eq!(parse_verdict(a, &reasoned, text).unwrap().line.as_deref(), Some("Which way should I build it?"), "the reason before the verdict is passed over");
        let bare = envelope(r#"{"asks":true}"#);
        assert_eq!(parse_verdict(a, &bare, text).unwrap().line.as_deref(), Some("Which way should I build it?"), "the wording check finds it");
        let plain = "All set.\nI wrote it down";
        assert_eq!(parse_verdict(a, &bare, plain).unwrap().line.as_deref(), Some("I wrote it down"), "else the last line");
        let made_up = envelope(r#"{"asks":true,"line":"Shall we ship?"}"#);
        assert_eq!(parse_verdict(a, &made_up, text).unwrap().line.as_deref(), Some("Which way should I build it?"), "a line not in the text is replaced");
        let curly = envelope(r#"{"asks":true,"line":"What's next?"}"#);
        assert_eq!(parse_verdict(a, &curly, "Done. What’s next?").unwrap().line.as_deref(), Some("What's next?"), "curly and straight apostrophes match");
        assert!(parse_verdict(a, &envelope("no json here"), text).is_err());
        assert!(parse_verdict(a, &envelope("} {"), text).is_err(), "braces the wrong way round are no verdict, not a panic");
        assert!(parse_verdict(a, &envelope(r#"{"line":"x"}"#), text).is_err());
        assert!(parse_verdict(a, &envelope(r#"{"asks":"yes"}"#), text).is_err());
    }
}
