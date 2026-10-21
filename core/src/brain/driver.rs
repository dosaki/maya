//! What every resident brain looks like to Maya: a process that takes
//! messages and reports turns, whichever agent runs it.

use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::Sender;
use std::sync::Arc;

use crate::model::Harness;

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Her session has started and names its id: from the agent's own
    /// report, not from the spawn.
    Ready { session_id: String },
    /// A turn began, including one she started herself (a subagent woke her).
    TurnStarted,
    TurnEnded { text: String, context_tokens: Option<u64> },
    /// A turn ended in an error: whatever it wrote is not her reply.
    TurnFailed { context_tokens: Option<u64> },
    /// Background subagents.
    TaskStarted(String),
    TaskEnded(String),
    /// The process ended; the reason.
    Exited(String),
}

pub struct Launch {
    pub agent: Harness,
    pub binary: PathBuf,
    pub model: Option<String>,
    pub cwd: PathBuf,
    pub system_prompt: String,
    pub resume: Option<String>,
    /// Folders the agent may write to, for agents with a sandbox (not Claude).
    pub writable: Vec<PathBuf>,
}

pub trait Driver: Send {
    fn send(&mut self, text: &str) -> Result<(), String>;
    /// The running turn and every live background task.
    fn interrupt(&mut self) -> Result<(), String>;
    /// `Err("unsupported")` where the agent compacts itself.
    fn compact(&mut self) -> Result<(), String>;
    fn background_subagents(&self) -> bool;
    /// The session id Maya started it with, when the agent takes one; the
    /// real one comes with `Event::Ready`.
    fn assumed_session_id(&self) -> Option<String> {
        None
    }
    /// When her process last wrote a line (epoch ms; 0 before any), stamped
    /// by its reader for every line, events or not. Shared, so it is read
    /// without the driver's lock. None for a driver that keeps no clock.
    fn output_clock(&self) -> Option<Arc<AtomicU64>> {
        None
    }
    fn shutdown(&mut self);
}

/// Start the resident brain for `l.agent`. Events arrive on `events`;
/// `Exited` is the last one.
pub fn spawn(l: &Launch, events: Sender<Event>) -> Result<Box<dyn Driver>, String> {
    match l.agent {
        Harness::ClaudeCode => Ok(Box::new(super::claude::ClaudeDriver::spawn(l, events)?)),
        other => Err(format!("no resident brain for {}", crate::launch::label(other))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_agents_have_no_resident_brain_yet() {
        let l = Launch { agent: Harness::Codex, binary: "codex".into(), model: None, cwd: std::env::temp_dir(), system_prompt: String::new(), resume: None, writable: vec![] };
        let (tx, _rx) = std::sync::mpsc::channel();
        assert_eq!(spawn(&l, tx).err().as_deref(), Some("no resident brain for Codex"));
    }
}
