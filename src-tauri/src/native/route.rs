//! Where card actions go: a tty of a terminal Maya hosts goes to the host,
//! any other to the platform's terminal. New sessions open where Settings says.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use maya_core::answer;
use maya_core::config::{self, NewSessionTerminal};
use maya_core::terminal::Terminal;

use super::host::{NativeHost, TermId};

/// The platform's terminal with Maya's own terminals in front of it.
pub struct WithNative {
    pub inner: &'static dyn Terminal,
    pub host: &'static NativeHost,
    /// Where new sessions open, read on each use so a change in Settings counts at once.
    pub choice: fn() -> NewSessionTerminal,
}

/// The terminal every card action goes through.
pub static TERMINAL: LazyLock<WithNative> = LazyLock::new(|| WithNative { inner: platform(), host: &super::HOST, choice: configured_choice });

#[cfg(not(target_os = "linux"))]
fn platform() -> &'static dyn Terminal {
    &crate::term::TERMINAL
}
#[cfg(target_os = "linux")]
fn platform() -> &'static dyn Terminal {
    &*crate::term::TERMINAL
}

/// The choice saved in Settings; Terminal when there is none or it cannot be read.
pub(crate) fn configured_choice() -> NewSessionTerminal {
    config::load(&maya_core::claude_dir().join("maya/config.json")).new_session_terminal
}

thread_local! {
    /// The terminal this thread's last Native open started, for `opening`.
    static OPENED: Cell<Option<TermId>> = const { Cell::new(None) };
}

/// Runs `f`, and hands back with its result the Native terminal it started,
/// if it started one: the app opens its viewer at once. An action opens on
/// the thread that runs it, so another thread's opens never count.
pub fn opening<T>(f: impl FnOnce() -> T) -> (T, Option<TermId>) {
    OPENED.set(None);
    let out = f();
    (out, OPENED.take())
}

/// What `focus` says for a Native tty: its terminal is in Maya, not a window.
const NO_WINDOW: &str = "Open the session from its card.";

impl Terminal for WithNative {
    fn open(&self, command: &str, cwd: &Path, label: &str) -> Result<Option<String>, String> {
        match (self.choice)() {
            NewSessionTerminal::Native => self.open_native(command, cwd, label),
            NewSessionTerminal::Terminal | NewSessionTerminal::Iterm => self.inner.open(command, cwd, label),
        }
    }
    /// A hosted terminal has no name for tmux to find it by.
    fn open_native(&self, command: &str, cwd: &Path, _label: &str) -> Result<Option<String>, String> {
        let term = self.host.spawn(command, cwd)?;
        OPENED.set(Some(term));
        Ok(None)
    }
    fn type_line(&self, tty: &str, text: &str) -> Result<(), String> {
        match self.host.term_for_tty(tty) {
            Some(term) => self.host.write(term, format!("{text}\r").as_bytes()),
            None => self.inner.type_line(tty, text),
        }
    }
    fn press_keys(&self, tty: &str, keys: &str) -> Result<(), String> {
        match self.host.term_for_tty(tty) {
            Some(term) => self.host.write(term, keys.as_bytes()),
            None => self.inner.press_keys(tty, keys),
        }
    }
    /// One write: the reply either went in whole or not at all.
    fn type_reply(&self, tty: &str, lines: &[String]) -> Result<(), (usize, String)> {
        match self.host.term_for_tty(tty) {
            Some(term) => self.host.write(term, answer::reply_keys(lines).as_bytes()).map_err(|why| (0, why)),
            None => self.inner.type_reply(tty, lines),
        }
    }
    fn focus(&self, tty: &str) -> Result<(), String> {
        match self.host.term_for_tty(tty) {
            Some(_) => Err(NO_WINDOW.into()),
            None => self.inner.focus(tty),
        }
    }
    fn name_for_tty(&self, tty: &str) -> Option<String> {
        match self.host.term_for_tty(tty) {
            Some(_) => None,
            None => self.inner.name_for_tty(tty),
        }
    }
    /// The inner terminal is asked once, for the ttys the host does not have.
    fn names_for_ttys(&self, ttys: &[String]) -> HashMap<String, String> {
        let theirs: Vec<String> = ttys.iter().filter(|t| self.host.term_for_tty(t).is_none()).cloned().collect();
        self.inner.names_for_ttys(&theirs)
    }
    /// A hosted terminal closes with its program: nothing reaches it after.
    fn reach_after_exit(&self, tty: &str) -> Vec<String> {
        match self.host.term_for_tty(tty) {
            Some(_) => vec![],
            None => self.inner.reach_after_exit(tty),
        }
    }
    fn closes_with_agent(&self, tty: &str) -> bool {
        self.host.term_for_tty(tty).is_some() || self.inner.closes_with_agent(tty)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::native::host::{Frame, TermId, BATCH};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// The platform's terminal, standing in: records what reaches it.
    #[derive(Default)]
    struct Stub {
        calls: Mutex<Vec<String>>,
    }

    impl Stub {
        fn new() -> &'static Stub {
            Box::leak(Box::default())
        }
        fn record(&self, call: String) {
            self.calls.lock().unwrap().push(call);
        }
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Terminal for Stub {
        fn open(&self, command: &str, _cwd: &Path, _label: &str) -> Result<Option<String>, String> {
            self.record(format!("open {command}"));
            Ok(Some("app".into()))
        }
        fn type_line(&self, tty: &str, text: &str) -> Result<(), String> {
            self.record(format!("type {tty} {text}"));
            Ok(())
        }
        fn press_keys(&self, tty: &str, keys: &str) -> Result<(), String> {
            self.record(format!("keys {tty} {keys}"));
            Ok(())
        }
        fn type_reply(&self, tty: &str, lines: &[String]) -> Result<(), (usize, String)> {
            self.record(format!("reply {tty} {}", lines.join("|")));
            Ok(())
        }
        fn focus(&self, tty: &str) -> Result<(), String> {
            self.record(format!("focus {tty}"));
            Ok(())
        }
        fn name_for_tty(&self, _tty: &str) -> Option<String> {
            None
        }
    }

    const OTHER: &str = "/dev/ttys999";

    /// A route over a fresh stub and a fresh host of its own, choosing `choice`.
    fn route(choice: fn() -> NewSessionTerminal) -> (WithNative, &'static Stub) {
        let inner = Stub::new();
        let host: &'static NativeHost = Box::leak(Box::new(NativeHost::new(BATCH)));
        (WithNative { inner, host, choice }, inner)
    }

    fn native() -> NewSessionTerminal {
        NewSessionTerminal::Native
    }

    fn app() -> NewSessionTerminal {
        NewSessionTerminal::Terminal
    }

    /// Polls `done` every 10 ms for up to 5 s.
    fn within_5s(mut done: impl FnMut() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        done()
    }

    /// Starts a terminal that copies every byte typed into it, untouched, to
    /// `file`, and returns it once it is ready, with its tty.
    fn recorder(host: &NativeHost, file: &Path) -> (TermId, String) {
        let term = host.spawn(&format!("stty raw -echo; printf ready; cat > '{}'", file.display()), Path::new("/")).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        host.attach(
            term,
            0,
            Box::new(move |f| {
                if let Frame::Output(b) = f {
                    s.lock().unwrap().extend(b);
                }
                true
            }),
        )
        .unwrap();
        let ready = within_5s(|| String::from_utf8_lossy(&seen.lock().unwrap()).contains("ready"));
        if !ready {
            host.kill_all();
            panic!("the terminal never got ready");
        }
        let tty = host.tty(term).unwrap();
        (term, tty)
    }

    /// Waits until `file` holds exactly `want`, and returns what it holds.
    fn holds(file: &Path, want: &str) -> String {
        within_5s(|| std::fs::read_to_string(file).is_ok_and(|s| s == want));
        std::fs::read_to_string(file).unwrap_or_default()
    }

    #[test]
    fn open_follows_the_setting() {
        let (r, inner) = route(native);
        let opened = r.open("sleep 30", Path::new("/"), "maya-1");
        let roots = r.host.root_pids().len();
        r.host.kill_all();
        assert_eq!(opened, Ok(None));
        assert_eq!(roots, 1, "the host started the session");
        assert!(inner.calls().is_empty(), "{:?}", inner.calls());

        let (r, inner) = route(app);
        assert_eq!(r.open("claude", Path::new("/"), "maya-1"), Ok(Some("app".into())));
        assert_eq!(inner.calls(), ["open claude"]);
        assert!(r.host.root_pids().is_empty());
    }

    #[test]
    fn opening_hands_back_the_native_terminal_it_started() {
        let (r, _) = route(native);
        let (opened, term) = opening(|| r.open("sleep 30", Path::new("/"), "maya-1"));
        let started = r.host.root_pids().first().and_then(|&root| r.host.term_for_root(root));
        r.host.kill_all();
        assert_eq!(opened, Ok(None));
        assert!(term.is_some());
        assert_eq!(term, started, "the terminal it started");

        let (r, _) = route(app);
        assert_eq!(opening(|| r.open("claude", Path::new("/"), "maya-1")).1, None, "a terminal app's window is not Maya's");
        assert_eq!(opening(|| ()).1, None, "nothing opened, nothing handed back");
    }

    #[test]
    fn open_native_always_uses_the_host() {
        let (r, inner) = route(app);
        let opened = r.open_native("sleep 30", Path::new("/"), "maya-1");
        let roots = r.host.root_pids().len();
        r.host.kill_all();
        assert_eq!(opened, Ok(None));
        assert_eq!(roots, 1);
        assert!(inner.calls().is_empty(), "{:?}", inner.calls());
    }

    #[test]
    fn keys_for_a_native_tty_go_to_the_host_and_others_to_the_inner_terminal() {
        let (r, inner) = route(app);
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("typed");
        let (_, tty) = recorder(r.host, &file);
        let typed = (r.type_line(&tty, "text"), r.press_keys(&tty, answer::ESC));
        let got = holds(&file, "text\r\x1b");
        r.host.kill_all();
        assert_eq!(typed, (Ok(()), Ok(())));
        assert_eq!(got, "text\r\x1b");
        assert!(inner.calls().is_empty(), "{:?}", inner.calls());

        r.type_line(OTHER, "text").unwrap();
        r.press_keys(OTHER, answer::ESC).unwrap();
        assert_eq!(inner.calls(), [format!("type {OTHER} text"), format!("keys {OTHER} \x1b")]);
    }

    #[test]
    fn a_reply_to_a_native_tty_is_written_in_one_go() {
        let (r, inner) = route(app);
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("typed");
        let (_, tty) = recorder(r.host, &file);
        let lines = ["a".to_string(), "b".to_string()];
        let want = answer::reply_keys(&lines);
        let replied = r.type_reply(&tty, &lines);
        let got = holds(&file, &want);
        r.host.kill_all();
        assert_eq!(replied, Ok(()));
        assert_eq!(got, want);
        assert!(inner.calls().is_empty(), "{:?}", inner.calls());

        r.type_reply(OTHER, &lines).unwrap();
        assert_eq!(inner.calls(), [format!("reply {OTHER} a|b")]);
    }

    #[test]
    fn a_native_tty_closes_with_its_agent_and_has_no_focus_window() {
        let (r, inner) = route(app);
        let term = r.host.spawn("sleep 30", Path::new("/")).unwrap();
        let tty = r.host.tty(term).unwrap();
        let native = (r.closes_with_agent(&tty), r.focus(&tty), r.reach_after_exit(&tty));
        r.host.kill_all();
        assert_eq!(native, (true, Err("Open the session from its card.".into()), vec![]));
        assert!(inner.calls().is_empty(), "{:?}", inner.calls());

        assert!(!r.closes_with_agent(OTHER));
        assert_eq!(r.focus(OTHER), Ok(()));
        assert_eq!(r.reach_after_exit(OTHER), [OTHER]);
        assert_eq!(inner.calls(), [format!("focus {OTHER}")]);
    }
}
