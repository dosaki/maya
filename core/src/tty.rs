//! The terminal device a process runs on, read from `ps`.

use std::process::Command;

/// Turns `ps -o tty= -p <pid>` output into `/dev/ttysNNN`.
pub fn tty_from_ps(output: &str) -> Option<String> {
    let t = output.trim();
    if t.is_empty() || t == "??" || t == "-" {
        return None;
    }
    Some(format!("/dev/{t}"))
}

/// `/dev/ttysNNN` of the terminal hosting `pid`.
pub fn tty_for_pid(pid: i32) -> Result<String, String> {
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
        assert_eq!(tty_from_ps(""), None);
    }

    #[test]
    fn tty_for_pid_of_dead_process_is_an_error() {
        assert!(tty_for_pid(2_000_000_000).is_err());
    }
}
