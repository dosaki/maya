use maya_core::terminal::Terminal;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const NOT_IN_TMUX: &str = "This session is not in tmux; only replies reach it.";

pub struct Tmux { pub binary: PathBuf }

impl Default for Tmux {
    fn default() -> Self { Self { binary: PathBuf::from("tmux") } }
}

fn sq(s: &str) -> String { maya_core::launch::shell_single_quote(s) }

pub fn new_session_args(label: &str, cwd: &Path, command: &str) -> Vec<String> {
    let inner = format!("{command}; exec \"${{SHELL:-sh}}\"");
    ["new-session", "-d", "-s", label, "-c", &cwd.to_string_lossy()].iter().map(|s| s.to_string()).chain([format!("sh -c {}", sq(&inner))]).collect()
}

pub fn send_keys_args(target: &str, text: &str) -> Vec<Vec<String>> {
    let base = |rest: &[&str]| -> Vec<String> { ["send-keys", "-t", target].iter().chain(rest).map(|s| s.to_string()).collect() };
    let mut out = Vec::new();
    let mut literal = String::new();
    let mut keys: Vec<&str> = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("\x1b[B") { keys.push("Down"); rest = r; }
        else if let Some(r) = rest.strip_prefix("\x1b[Z") { keys.push("BTab"); rest = r; }
        else { let c = rest.chars().next().unwrap(); literal.push(c); rest = &rest[c.len_utf8()..]; }
    }
    if !literal.is_empty() { out.push(base(&["-l", "--", &literal])); }
    if !keys.is_empty() { out.push(base(&keys)); }
    out.push(base(&["Enter"]));
    out
}

/// `tmux list-panes -a -F '#{pane_tty}\t#{session_name}:#{window_index}.#{pane_index}\t#{session_name}'`
pub const LIST_PANES_FORMAT: &str = "#{pane_tty}\t#{session_name}:#{window_index}.#{pane_index}\t#{session_name}";

pub fn pane_for_tty(out: &str, tty: &str) -> Option<(String, String)> {
    let want = tty.trim_start_matches("/dev/");
    out.lines().filter_map(|l| { let mut f = l.split('\t'); Some((f.next()?, f.next()?, f.next()?)) })
        .find(|(t, _, _)| t.trim_start_matches("/dev/") == want)
        .map(|(_, target, session)| (target.to_string(), session.to_string()))
}

impl Tmux {
    fn run(&self, args: &[String]) -> Result<String, String> {
        let out = Command::new(&self.binary).args(args).stdin(Stdio::null()).output().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound { format!("tmux is not installed on {}", maya_core::net::local_hostname()) } else { format!("could not run tmux: {e}") }
        })?;
        if !out.status.success() { return Err(format!("tmux failed: {}", String::from_utf8_lossy(&out.stderr).trim())); }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
    fn pane(&self, tty: &str) -> Option<(String, String)> {
        let out = self.run(&["list-panes".into(), "-a".into(), "-F".into(), LIST_PANES_FORMAT.into()]).ok()?;
        pane_for_tty(&out, tty)
    }
}

impl Terminal for Tmux {
    fn open(&self, command: &str, cwd: &Path, label: &str) -> Result<Option<String>, String> {
        self.run(&new_session_args(label, cwd, command))?;
        Ok(Some(label.to_string()))
    }
    fn type_line(&self, tty: &str, text: &str) -> Result<(), String> {
        let (target, _) = self.pane(tty).ok_or(NOT_IN_TMUX)?;
        for args in send_keys_args(&target, text) { self.run(&args)?; }
        Ok(())
    }
    fn focus(&self, tty: &str) -> Result<(), String> {
        Err(match self.pane(tty) { Some((_, s)) => format!("Attach with: tmux attach -t {s}"), None => "Attach with: tmux attach -t <session>".into() })
    }
    fn name_for_tty(&self, tty: &str) -> Option<String> {
        self.pane(tty).map(|p| p.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn v(a: &[&str]) -> Vec<String> { a.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn new_session_keeps_the_pane_open_after_claude_exits_and_quotes_the_cwd() {
        let args = new_session_args("maya-1a2b3c4d", Path::new("/p/it's here"), "cd '/p' && claude -- \"$p\"");
        assert_eq!(args[..6].to_vec(), v(&["new-session", "-d", "-s", "maya-1a2b3c4d", "-c", "/p/it's here"]));
        assert_eq!(args[6], "sh -c 'cd '\\''/p'\\'' && claude -- \"$p\"; exec \"${SHELL:-sh}\"'");
    }

    #[test]
    fn send_keys_types_literal_text_then_enter_and_maps_escape_sequences() {
        let calls = send_keys_args("maya-1:0.0", "/compact");
        assert_eq!(calls, vec![v(&["send-keys", "-t", "maya-1:0.0", "-l", "--", "/compact"]), v(&["send-keys", "-t", "maya-1:0.0", "Enter"])]);
        let calls = send_keys_args("t", "\x1b[B\x1b[B");
        assert_eq!(calls, vec![v(&["send-keys", "-t", "t", "Down", "Down"]), v(&["send-keys", "-t", "t", "Enter"])]);
        let calls = send_keys_args("t", "\x1b[Z");
        assert_eq!(calls[0], v(&["send-keys", "-t", "t", "BTab"]));
        // An empty line is just Enter.
        assert_eq!(send_keys_args("t", ""), vec![v(&["send-keys", "-t", "t", "Enter"])]);
    }

    #[test]
    fn pane_lookup_matches_the_tty_exactly() {
        let out = "/dev/pts/3\tmaya-1a2b3c4d:0.0\tmaya-1a2b3c4d\n/dev/pts/31\twork:1.0\twork\n";
        assert_eq!(pane_for_tty(out, "/dev/pts/3"), Some(("maya-1a2b3c4d:0.0".into(), "maya-1a2b3c4d".into())));
        assert_eq!(pane_for_tty(out, "/dev/pts/9"), None);
        // macOS ttys come without the /dev prefix from `ps`; both spellings match.
        assert_eq!(pane_for_tty("/dev/ttys004\ta:0.0\ta\n", "ttys004").map(|p| p.1), Some("a".into()));
    }

    #[test]
    fn a_missing_tmux_binary_names_the_machine() {
        let t = Tmux { binary: PathBuf::from("/nonexistent/tmux") };
        let err = t.open("true", Path::new("/"), "maya-00000000").unwrap_err();
        assert_eq!(err, format!("tmux is not installed on {}", maya_core::net::local_hostname()));
        assert_eq!(t.type_line("/dev/pts/3", "x"), Err(NOT_IN_TMUX.into()));
        assert_eq!(t.name_for_tty("/dev/pts/3"), None);
        assert_eq!(t.focus("/dev/pts/3"), Err("Attach with: tmux attach -t <session>".into()));
    }
}
