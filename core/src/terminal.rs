//! The platform seam: where a session's terminal lives and how keys reach it.
use std::path::{Path, PathBuf};

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
}

#[cfg(any(test, feature = "test-support"))]
pub use test_support::{Call, FakeTerminal};

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use super::*;
    use std::collections::HashMap;
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
    fn fake_terminal_can_refuse_typing() {
        let t = FakeTerminal { fail_type: Some("This session is not in tmux; only replies reach it.".into()), ..Default::default() };
        assert_eq!(t.type_line("/dev/pts/3", "x"), Err("This session is not in tmux; only replies reach it.".into()));
    }
}
