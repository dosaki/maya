//! The CLI's `Executor`: runs the main's commands with core's local session
//! actions over the tmux terminal, and describes this machine's board with
//! each card's tmux session name. Used while pairing (where it never runs a
//! command) and while `maya run` keeps a connection to the main open.

use maya_core::actions;
use maya_core::launch;
use maya_core::model::Card;
use maya_core::net::client::{self, Executor};
use maya_core::net::protocol::CommandKind;
use maya_core::now_ms;
use maya_core::store::Store;
use maya_core::terminal::Terminal;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub struct CliExecutor {
    pub store: Arc<Mutex<Store>>,
    pub terminal: Arc<dyn Terminal>,
    /// Set by the watcher when the board may have changed.
    pub board_due: Arc<AtomicBool>,
    /// The tty `ps` reported for each pid the registry gave none for.
    ttys: Mutex<HashMap<i32, String>>,
}

impl CliExecutor {
    pub fn new(store: Arc<Mutex<Store>>, terminal: Arc<dyn Terminal>) -> Self {
        Self { store, terminal, board_due: Arc::new(AtomicBool::new(false)), ttys: Mutex::new(HashMap::new()) }
    }

    /// The session's tty: the registry's, else the one `ps` reports (cached).
    fn tty_of(&self, pid: i32, registry_tty: Option<&str>) -> Option<String> {
        if let Some(t) = registry_tty {
            return Some(t.to_string());
        }
        let mut cache = self.ttys.lock().unwrap();
        if let Some(t) = cache.get(&pid) {
            return Some(t.clone());
        }
        let t = maya_core::tty::tty_for_pid(pid).ok()?;
        cache.insert(pid, t.clone());
        Some(t)
    }
}

fn data<T: serde::Serialize>(v: Result<T, String>) -> Result<Option<Value>, String> {
    v.and_then(|v| serde_json::to_value(v).map(Some).map_err(|e| e.to_string()))
}

impl Executor for CliExecutor {
    fn execute(&self, kind: CommandKind) -> Result<Option<Value>, String> {
        let l = actions::Local { store: &self.store, terminal: &*self.terminal };
        let done = |r: Result<(), String>| r.map(|_| None);
        match kind {
            CommandKind::Reply { session, text, attachments } => {
                let (maya_dir, exists) = {
                    let mut s = self.store.lock().unwrap();
                    (s.claude_dir().join("maya"), s.card_for(&session, now_ms()).is_some())
                };
                done(client::reply_with_attachments(&maya_dir, exists, &text, attachments, now_ms(), |text| actions::send_reply(&l, &session, &text)))
            }
            CommandKind::Answer { session, ask_id, question, option } => done(actions::answer_question(&l, &session, ask_id, question, option)),
            CommandKind::Compact { session } => done(actions::compact_session(&l, &session)),
            CommandKind::Rename { session, name } => done(actions::rename_session(&l, &session, &name)),
            CommandKind::SetOption { session, setting, value } => done(actions::set_session_option(&l, &session, &setting, &value)),
            CommandKind::CycleMode { session } => done(actions::cycle_session_mode(&l, &session)),
            CommandKind::Start { dir, prompt, options } => data(actions::start_session(&l, dir, prompt, options)),
            CommandKind::Resume { dir, session } => done(actions::resume_session(&l, &dir, &session)),
            CommandKind::ListResumable { dir } => data(actions::list_resumable_sessions(&l, &dir)),
            CommandKind::History { session } => data(actions::session_history(&l, &session)),
        }
    }

    fn board(&self) -> (Vec<Card>, Vec<String>) {
        let (mut cards, root, ttys) = {
            let mut s = self.store.lock().unwrap();
            let cards = s.refresh(now_ms());
            let ttys: Vec<Option<String>> = cards.iter().map(|c| s.session(&c.session_id).and_then(|r| r.tty).or_else(|| s.foreign(&c.session_id).and_then(|f| f.tty))).collect();
            (cards, s.config.projects_dir_path(), ttys)
        };
        // A pid that is gone may come back as another process: forget its tty.
        self.ttys.lock().unwrap().retain(|pid, _| cards.iter().any(|c| c.pid == *pid));
        for (c, tty) in cards.iter_mut().zip(ttys) {
            c.terminal = self.tty_of(c.pid, tty.as_deref()).and_then(|t| self.terminal.name_for_tty(&t));
        }
        let dirs = root.filter(|r| r.is_dir()).map(|r| launch::list_project_dirs(&r)).unwrap_or_default();
        (cards, dirs)
    }

    fn board_requested(&self) -> bool {
        self.board_due.swap(false, Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maya_core::terminal::{Call, FakeTerminal};

    #[test]
    fn the_board_names_each_cards_tmux_session_by_its_tty() {
        let (dir, store) = maya_core::actions::test_support::store_with_session("s1", 4242);
        let fake = FakeTerminal::default();
        fake.names.lock().unwrap().insert("/dev/pts/3".into(), "maya-1a2b3c4d".into());
        let ex = CliExecutor::new(Arc::new(store), Arc::new(fake));
        let (cards, _) = ex.board();
        assert_eq!(cards[0].terminal.as_deref(), Some("maya-1a2b3c4d"));
        drop(dir);
    }

    #[test]
    fn a_compact_command_is_typed_through_the_terminal() {
        let (dir, store) = maya_core::actions::test_support::store_with_session("s1", 4242);
        let fake = Arc::new(FakeTerminal::default());
        let ex = CliExecutor::new(Arc::new(store), fake.clone());
        assert_eq!(ex.execute(CommandKind::Compact { session: "s1".into() }), Ok(None));
        assert!(matches!(&fake.calls.lock().unwrap()[0], Call::Type { text, .. } if text == "/compact"));
        drop(dir);
    }
}
