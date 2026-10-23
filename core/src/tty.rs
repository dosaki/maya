//! The terminal device a process runs on: on Linux from `/proc/<pid>/fd/0`,
//! else (and as the fallback) from `ps`; and the terminal tab's above any
//! shell wrapper.

#[cfg(unix)]
use std::process::Command;

/// Turns `ps -o tty= -p <pid>` output into `/dev/ttysNNN` (macOS) or
/// `/dev/pts/N` (Linux). `??` (macOS), `?` (Linux) and `-` mean none.
pub fn tty_from_ps(output: &str) -> Option<String> {
    let t = output.trim();
    if t.is_empty() || t == "??" || t == "?" || t == "-" {
        return None;
    }
    Some(format!("/dev/{t}"))
}

/// The tty of the terminal tab hosting `tty`. A shell wrapper such as Kiro
/// CLI's (`zsh (kiro-cli-term)`) runs the shell on a pseudo-terminal of its
/// own inside the tab's, so an agent's tty is not one its terminal knows:
/// each step goes from a process on the tty to a parent on another one, up
/// to six wrappers deep. A tmux pane stays as it is: its server has no tty.
pub fn outer_tty(procs: &[crate::foreign::Proc], tty: &str) -> String {
    let mut cur = tty.to_string();
    for _ in 0..6 {
        let parent_tty = procs
            .iter()
            .filter(|p| p.tty.as_deref() == Some(cur.as_str()))
            .find_map(|p| procs.iter().find(|q| q.pid == p.ppid)?.tty.clone().filter(|t| *t != cur));
        match parent_tty {
            Some(t) => cur = t,
            None => break,
        }
    }
    cur
}

/// The target of `/proc/<pid>/fd/0` when it is a terminal device: a
/// pseudo-terminal (`/dev/pts/N`) or a console (`/dev/ttyN`). A pipe, a
/// socket or `/dev/null` is not.
#[cfg(any(target_os = "linux", test))]
pub fn tty_from_fd_link(target: &str) -> Option<String> {
    (target.starts_with("/dev/pts/") || target.starts_with("/dev/tty")).then(|| target.to_string())
}

/// The tty of `pid`'s stdin, read from `/proc`, if it is a terminal.
#[cfg(target_os = "linux")]
fn tty_from_proc(pid: i32) -> Option<String> {
    let link = std::fs::read_link(format!("/proc/{pid}/fd/0")).ok()?;
    tty_from_fd_link(&link.to_string_lossy())
}

/// Windows has no ttys: a session is reached through its process's console,
/// keyed `console:<pid>`.
#[cfg(windows)]
pub fn tty_for_pid(pid: i32) -> Result<String, String> {
    if crate::registry::pid_alive(pid) {
        Ok(crate::win_console::console_key(pid))
    } else {
        Err(format!("no console for process {pid}"))
    }
}

/// The terminal device hosting `pid`: its own, or its terminal tab's when a
/// shell wrapper sits between them (`outer_tty`).
#[cfg(unix)]
pub fn tty_for_pid(pid: i32) -> Result<String, String> {
    own_tty(pid).map(|t| outer_tty(&crate::foreign::list_process_tree(), &t))
}

/// The terminal device `pid` itself runs on.
#[cfg(unix)]
fn own_tty(pid: i32) -> Result<String, String> {
    #[cfg(target_os = "linux")]
    if let Some(t) = tty_from_proc(pid) {
        return Ok(t);
    }
    let ps = Command::new("ps")
        .args(["-o", "tty=", "-p", &pid.to_string()])
        .output()
        .map_err(|e| format!("could not run ps: {e}"))?;
    let stdout = String::from_utf8_lossy(&ps.stdout);
    tty_from_ps(&stdout).ok_or_else(|| format!("no tty for process {pid}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tty_from_ps_output() {
        assert_eq!(tty_from_ps("ttys021 \n"), Some("/dev/ttys021".to_string()));
        assert_eq!(tty_from_ps("  ttys007\n"), Some("/dev/ttys007".to_string()));
        assert_eq!(tty_from_ps("??\n"), None);
        // Linux's `ps` shows a process with no terminal as `?`.
        assert_eq!(tty_from_ps("?\n"), None);
        assert_eq!(tty_from_ps("pts/3\n"), Some("/dev/pts/3".to_string()));
        assert_eq!(tty_from_ps(""), None);
    }

    #[test]
    fn a_terminal_on_stdin_is_the_tty_and_anything_else_is_not() {
        assert_eq!(tty_from_fd_link("/dev/pts/3"), Some("/dev/pts/3".to_string()));
        assert_eq!(tty_from_fd_link("/dev/tty1"), Some("/dev/tty1".to_string()));
        assert_eq!(tty_from_fd_link("/dev/null"), None);
        assert_eq!(tty_from_fd_link("pipe:[123]"), None);
        assert_eq!(tty_from_fd_link("socket:[9]"), None);
    }

    /// `ps -axo pid=,ppid=,tty=,comm=` with Kiro CLI's shell wrapper in a
    /// Terminal tab on ttys001, running a resumed Claude Code on ttys019,
    /// beside an unwrapped tab on ttys003 and a tmux pane on ttys030.
    const WRAPPED: &str = "\
13266 1 ?? /System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal
27613 13266 ttys001 login
27614 27613 ttys001 zsh (kiro-cli-term)
27631 27614 ttys019 /bin/zsh
30083 27631 ttys019 claude
83315 13266 ttys003 login
83316 83315 ttys003 -zsh
88321 83316 ttys003 claude
500 1 ?? tmux
501 500 ttys030 -zsh
502 501 ttys030 claude
";

    #[test]
    fn a_wrapped_shells_tty_is_the_one_its_terminal_tab_has() {
        let procs = crate::foreign::parse_ps_tree(WRAPPED);
        assert_eq!(outer_tty(&procs, "/dev/ttys019"), "/dev/ttys001");
    }

    #[test]
    fn an_unwrapped_tab_a_tmux_pane_and_an_unknown_tty_stay_as_they_are() {
        let procs = crate::foreign::parse_ps_tree(WRAPPED);
        assert_eq!(outer_tty(&procs, "/dev/ttys003"), "/dev/ttys003");
        // The tmux server has no tty: the pane's is the one tmux knows.
        assert_eq!(outer_tty(&procs, "/dev/ttys030"), "/dev/ttys030");
        assert_eq!(outer_tty(&procs, "/dev/ttys099"), "/dev/ttys099");
        assert_eq!(outer_tty(&[], "/dev/ttys019"), "/dev/ttys019");
    }

    #[test]
    fn a_wrapper_inside_a_wrapper_leads_to_the_outermost_tty() {
        let procs = crate::foreign::parse_ps_tree(&format!("{WRAPPED}31000 30083 ttys019 zsh (kiro-cli-term)\n31001 31000 ttys040 /bin/zsh\n31002 31001 ttys040 codex\n"));
        assert_eq!(outer_tty(&procs, "/dev/ttys040"), "/dev/ttys001");
    }

    #[test]
    fn tty_for_pid_of_dead_process_is_an_error() {
        assert!(tty_for_pid(2_000_000_000).is_err());
    }
}
