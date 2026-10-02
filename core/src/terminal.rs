//! The platform seam: where a session's terminal lives and how keys reach it.
use std::collections::HashMap;
use std::path::Path;

pub trait Terminal: Send + Sync {
    /// Runs `command` (a shell line) in a visible terminal in `cwd`, under
    /// `label` where the terminal has names (tmux); returns that name.
    fn open(&self, command: &str, cwd: &Path, label: &str) -> Result<Option<String>, String>;
    /// Types `text` then Enter into the terminal hosting `tty`.
    fn type_line(&self, tty: &str, text: &str) -> Result<(), String>;
    /// Brings the terminal hosting `tty` forward; Err says what to do instead.
    fn focus(&self, tty: &str) -> Result<(), String>;
    /// The terminal's own name for the pane hosting `tty`, if it has one.
    fn name_for_tty(&self, tty: &str) -> Option<String>;
    /// `name_for_tty` for many ttys at once, keyed by tty; ttys without a
    /// name are left out. A terminal that lists all its panes in one call
    /// (tmux) overrides this to do so once.
    fn names_for_ttys(&self, ttys: &[String]) -> HashMap<String, String> {
        ttys.iter().filter_map(|t| Some((t.clone(), self.name_for_tty(t)?))).collect()
    }
    /// Keys that will still reach the terminal hosting `tty` once the
    /// process it was found through has exited, to try in order. A device
    /// path outlives its process, so by default that is `tty` itself; a
    /// Windows console is reached through a process, so it lists the
    /// console's other processes.
    fn reach_after_exit(&self, tty: &str) -> Vec<String> {
        vec![tty.to_string()]
    }
}

#[cfg(any(test, feature = "test-support"))]
pub use test_support::{Call, FakeTerminal};

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex;

    #[derive(Debug, Clone, PartialEq)]
    pub enum Call {
        Open { command: String, cwd: PathBuf, label: String },
        Type { tty: String, text: String },
        Focus { tty: String },
    }

    #[derive(Default)]
    pub struct FakeTerminal {
        pub calls: Mutex<Vec<Call>>,
        pub names: Mutex<HashMap<String, String>>,
        pub fail_type: Option<String>,
        /// What `reach_after_exit` answers for a tty; absent means the tty itself.
        pub peers: Mutex<HashMap<String, Vec<String>>>,
        /// Ttys that refuse typing, as a console whose process has exited does.
        pub dead: Mutex<std::collections::HashSet<String>>,
    }

    impl Terminal for FakeTerminal {
        fn open(&self, command: &str, cwd: &Path, label: &str) -> Result<Option<String>, String> {
            self.calls.lock().unwrap().push(Call::Open { command: command.into(), cwd: cwd.into(), label: label.into() });
            Ok(Some(label.into()))
        }
        fn type_line(&self, tty: &str, text: &str) -> Result<(), String> {
            if let Some(e) = &self.fail_type {
                return Err(e.clone());
            }
            if self.dead.lock().unwrap().contains(tty) {
                return Err(format!("{tty} is gone"));
            }
            self.calls.lock().unwrap().push(Call::Type { tty: tty.into(), text: text.into() });
            Ok(())
        }
        fn focus(&self, tty: &str) -> Result<(), String> {
            self.calls.lock().unwrap().push(Call::Focus { tty: tty.into() });
            Ok(())
        }
        fn name_for_tty(&self, tty: &str) -> Option<String> {
            self.names.lock().unwrap().get(tty).cloned()
        }
        fn reach_after_exit(&self, tty: &str) -> Vec<String> {
            self.peers.lock().unwrap().get(tty).cloned().unwrap_or_else(|| vec![tty.to_string()])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_terminal_records_calls_and_answers_names() {
        let t = FakeTerminal::default();
        t.names.lock().unwrap().insert("/dev/pts/3".into(), "maya-1a2b3c4d".into());
        assert_eq!(t.open("claude", Path::new("/p"), "maya-1a2b3c4d").unwrap(), Some("maya-1a2b3c4d".into()));
        t.type_line("/dev/pts/3", "/compact").unwrap();
        assert_eq!(t.name_for_tty("/dev/pts/3").as_deref(), Some("maya-1a2b3c4d"));
        assert_eq!(t.name_for_tty("/dev/pts/9"), None);
        let calls = t.calls.lock().unwrap();
        assert!(matches!(&calls[0], Call::Open { label, .. } if label == "maya-1a2b3c4d"));
        assert!(matches!(&calls[1], Call::Type { text, .. } if text == "/compact"));
    }

    #[test]
    fn names_for_ttys_asks_for_each_tty_by_default_and_leaves_out_unnamed_ones() {
        let t = FakeTerminal::default();
        t.names.lock().unwrap().insert("/dev/pts/3".into(), "maya-1a2b3c4d".into());
        let names = t.names_for_ttys(&["/dev/pts/3".into(), "/dev/pts/9".into()]);
        assert_eq!(names.len(), 1);
        assert_eq!(names.get("/dev/pts/3").map(String::as_str), Some("maya-1a2b3c4d"));
    }

    #[test]
    fn fake_terminal_can_refuse_typing() {
        let t = FakeTerminal { fail_type: Some("This session is not in tmux; only replies reach it.".into()), ..Default::default() };
        assert_eq!(t.type_line("/dev/pts/3", "x"), Err("This session is not in tmux; only replies reach it.".into()));
    }
}
