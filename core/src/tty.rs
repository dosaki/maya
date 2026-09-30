//! The terminal device a process runs on: on Linux from `/proc/<pid>/fd/0`,
//! else (and as the fallback) from `ps`.

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

/// The terminal device hosting `pid`.
#[cfg(unix)]
pub fn tty_for_pid(pid: i32) -> Result<String, String> {
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

    #[test]
    fn tty_for_pid_of_dead_process_is_an_error() {
        assert!(tty_for_pid(2_000_000_000).is_err());
    }
}
