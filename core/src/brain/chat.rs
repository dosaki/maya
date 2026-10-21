//! One conversation for voice and Chat: the last turns, kept across restarts
//! in `<claude_dir>/maya/brain/chat.json`.

use crate::config;
pub use crate::window::Via;
use std::path::PathBuf;

/// Turns kept in the log and on disk.
pub const KEEP: usize = 500;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatTurn {
    /// "user" or "maya".
    pub who: String,
    pub text: String,
    pub at: u64,
    pub via: Via,
}

/// The Chat tab's status dot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Dot {
    Idle,
    Working,
    Waiting,
    Unread,
}

pub struct ChatLog {
    turns: Vec<ChatTurn>,
    seen_at: u64,
    total: u64,
    path: Option<PathBuf>,
}

impl ChatLog {
    /// Reads the log at `path`; a missing or unreadable file gives an empty log.
    pub fn load(path: PathBuf) -> ChatLog {
        let mut turns: Vec<ChatTurn> = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        if turns.len() > KEEP {
            let extra = turns.len() - KEEP;
            turns.drain(..extra);
        }
        // What was there before this run has been read already.
        let seen_at = turns.last().map_or(0, |t| t.at);
        ChatLog { total: turns.len() as u64, turns, seen_at, path: Some(path) }
    }

    /// A log that is never saved.
    pub fn in_memory() -> ChatLog {
        ChatLog { turns: Vec::new(), seen_at: 0, total: 0, path: None }
    }

    /// Adds a turn, saves, and returns how many turns have been pushed in all.
    pub fn push(&mut self, turn: ChatTurn) -> u64 {
        self.turns.push(turn);
        if self.turns.len() > KEEP {
            let extra = self.turns.len() - KEEP;
            self.turns.drain(..extra);
        }
        self.total += 1;
        self.save();
        self.total
    }

    pub fn last(&self, n: usize) -> Vec<ChatTurn> {
        self.turns[self.turns.len().saturating_sub(n)..].to_vec()
    }

    /// The user has looked at the log at `now_ms`.
    pub fn seen(&mut self, now_ms: u64) {
        self.seen_at = now_ms;
    }

    pub fn dot(&self, working: bool) -> Dot {
        if working {
            return Dot::Working;
        }
        match self.turns.last() {
            Some(t) if t.who == "maya" && t.at > self.seen_at => Dot::Unread,
            Some(t) if t.who == "maya" && t.text.trim_end().ends_with('?') => Dot::Waiting,
            _ => Dot::Idle,
        }
    }

    pub fn count(&self) -> u64 {
        self.total
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(bytes) = serde_json::to_vec(&self.turns) {
            if let Err(e) = config::write_private_atomic(path, &bytes) {
                crate::log::line("chat", format!("could not save the chat log: {e}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(who: &str, text: &str, at: u64) -> ChatTurn {
        ChatTurn { who: who.into(), text: text.into(), at, via: Via::Chat }
    }

    fn tmp() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("brain").join("chat.json");
        (dir, p)
    }

    #[test]
    fn the_log_keeps_the_last_500_turns_and_survives_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("chat.json");
        let mut log = ChatLog::load(p.clone());
        for i in 0..510 {
            log.push(turn("user", &i.to_string(), i));
        }
        let back = ChatLog::load(p);
        assert_eq!(back.last(1000).len(), KEEP);
        assert_eq!(back.last(1)[0].text, "509");
    }

    #[test]
    fn the_dot_is_working_then_unread_then_waiting_then_idle() {
        let (_dir, p) = tmp();
        let mut log = ChatLog::load(p);
        assert_eq!(log.dot(true), Dot::Working);
        log.push(turn("maya", "Done, it's #52.", 10));
        assert_eq!(log.dot(true), Dot::Working, "working beats unread");
        assert_eq!(log.dot(false), Dot::Unread);
        log.seen(11);
        assert_eq!(log.dot(false), Dot::Idle);
        log.push(turn("maya", "Which repo?", 12));
        log.seen(13);
        assert_eq!(log.dot(false), Dot::Waiting);
        log.push(turn("user", "maya", 14));
        assert_eq!(log.dot(false), Dot::Idle);
    }

    #[test]
    fn a_corrupt_log_loads_empty_and_the_next_push_writes_a_valid_file() {
        let (_dir, p) = tmp();
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"[{\"who\":\"us").unwrap();
        let mut log = ChatLog::load(p.clone());
        assert!(log.last(10).is_empty());
        log.push(turn("user", "hi", 1));
        let back = ChatLog::load(p);
        assert_eq!(back.last(10), vec![turn("user", "hi", 1)]);
    }
}
