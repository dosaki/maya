use std::process::Command;

/// Turns `ps -o tty= -p <pid>` output into `/dev/ttysNNN`.
pub fn tty_from_ps(output: &str) -> Option<String> {
    let t = output.trim();
    if t.is_empty() || t == "??" || t == "-" {
        return None;
    }
    Some(format!("/dev/{t}"))
}

pub fn applescript_for(tty: &str) -> String {
    format!(
        r#"tell application "Terminal"
  repeat with w in windows
    repeat with t in tabs of w
      if tty of t is "{tty}" then
        set selected tab of w to t
        set index of w to 1
        activate
        return "ok"
      end if
    end repeat
  end repeat
end tell
return "not found""#
    )
}

/// Brings the Terminal tab hosting `pid` to the front.
pub fn focus_pid(pid: i32) -> Result<(), String> {
    let ps = Command::new("ps")
        .args(["-o", "tty=", "-p", &pid.to_string()])
        .output()
        .map_err(|e| format!("could not run ps: {e}"))?;
    let stdout = String::from_utf8_lossy(&ps.stdout);
    let tty = tty_from_ps(&stdout).ok_or_else(|| format!("no tty for process {pid}"))?;

    let out = Command::new("osascript")
        .arg("-e")
        .arg(applescript_for(&tty))
        .output()
        .map_err(|e| format!("could not run osascript: {e}"))?;
    if !out.status.success() {
        return Err(format!("osascript failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let result = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if result == "ok" {
        Ok(())
    } else {
        Err(format!("no Terminal tab found for {tty}"))
    }
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
    fn applescript_targets_the_tty_and_activates() {
        let s = applescript_for("/dev/ttys021");
        assert!(s.contains("tell application \"Terminal\""));
        assert!(s.contains("if tty of t is \"/dev/ttys021\""));
        assert!(s.contains("set selected tab of w to t"));
        assert!(s.contains("activate"));
        assert!(s.contains("return \"not found\""));
    }

    #[test]
    fn focusing_a_dead_pid_is_an_error_not_a_panic() {
        let err = focus_pid(2_000_000_000).unwrap_err();
        assert!(err.to_lowercase().contains("tty") || err.to_lowercase().contains("process"), "{err}");
    }
}
