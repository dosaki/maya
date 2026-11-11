//! Running a custom tool's command: the user's own script that lists work items.

use std::io::{Read, Write};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long a command may run.
pub const TIMEOUT: Duration = Duration::from_secs(30);
/// The most stdout a command may print.
pub const MAX_OUTPUT: usize = 4 * 1024 * 1024;
/// How much stderr is kept for the error line.
const MAX_STDERR: usize = 64 * 1024;

/// The argv that runs `command` through the user's shell: `[$SHELL or LOGIN_SHELL, "-lic", command]`; Windows `["cmd.exe", "/C", command]`.
pub fn shell_argv(command: &str, shell: Option<&str>) -> Vec<String> {
    if cfg!(windows) {
        return vec!["cmd.exe".into(), "/C".into(), command.into()];
    }
    let shell = shell.filter(|s| std::path::Path::new(s).is_absolute()).unwrap_or(crate::launch::LOGIN_SHELL);
    vec![shell.into(), "-lic".into(), command.into()]
}

/// Reads `r` to its end, keeping at most `cap` bytes. True when there was more.
fn read_capped(mut r: impl Read, cap: usize, flag: Option<&AtomicBool>) -> (Vec<u8>, bool) {
    let mut out = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        match r.read(&mut buf) {
            Ok(0) | Err(_) => return (out, false),
            Ok(n) => {
                let room = cap - out.len();
                if n > room {
                    out.extend_from_slice(&buf[..room]);
                    if let Some(f) = flag {
                        f.store(true, Ordering::SeqCst);
                    }
                    return (out, true);
                }
                out.extend_from_slice(&buf[..n]);
            }
        }
    }
}

/// Kills the command and everything it started.
fn kill_all(child: &mut std::process::Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Runs `argv` with `stdin` (None: closed) and returns its stdout, or the error line.
pub fn run(argv: &[String], stdin: Option<&str>, timeout: Duration) -> Result<String, String> {
    let (program, args) = argv.split_first().ok_or("the command could not run: no command")?;
    let mut cmd = crate::command(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // cmd.exe wants the command as typed, not quoted the way Rust would.
        match args.split_last() {
            Some((last, rest)) => cmd.args(rest).raw_arg(last),
            None => &mut cmd,
        };
    }
    #[cfg(not(windows))]
    cmd.args(args);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    crate::launch::scrub_appimage_env(&mut cmd);
    cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    let deadline = Instant::now() + timeout;
    let mut child = cmd.spawn().map_err(|e| format!("the command could not run: {e}"))?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let text = text.as_bytes().to_vec();
        // Its own thread: a command that never reads must not hold us up.
        std::thread::spawn(move || {
            let _ = pipe.write_all(&text);
        });
    }
    let too_much = Arc::new(AtomicBool::new(false));
    let (out_tx, out_rx) = mpsc::channel();
    let (err_tx, err_rx) = mpsc::channel();
    if let Some(stdout) = child.stdout.take() {
        let flag = too_much.clone();
        std::thread::spawn(move || {
            let _ = out_tx.send(read_capped(stdout, MAX_OUTPUT, Some(&flag)));
        });
    }
    if let Some(stderr) = child.stderr.take() {
        std::thread::spawn(move || {
            let _ = err_tx.send(read_capped(stderr, MAX_STDERR, None).0);
        });
    }
    let status = loop {
        if too_much.load(Ordering::SeqCst) {
            kill_all(&mut child);
            return Err("the command printed more than 4 MB".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                kill_all(&mut child);
                return Err(format!("the command did not finish within {} s", TIMEOUT.as_secs()));
            }
            Err(e) => {
                kill_all(&mut child);
                return Err(format!("the command could not run: {e}"));
            }
        }
    };
    // A grandchild may still hold the pipes open: wait for them only until the deadline.
    let wait = || deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(100));
    let (stdout, overflow) = out_rx.recv_timeout(wait()).unwrap_or_default();
    if overflow {
        return Err("the command printed more than 4 MB".into());
    }
    let stderr = err_rx.recv_timeout(Duration::from_millis(100)).unwrap_or_default();
    match status.code() {
        Some(0) => Ok(String::from_utf8_lossy(&stdout).into_owned()),
        Some(code) => {
            let lines: Vec<String> = String::from_utf8_lossy(&stderr).lines().filter(|l| !l.trim().is_empty()).take(3).map(|l| l.chars().take(200).collect()).collect();
            Err(if lines.is_empty() { format!("the command exited with status {code}") } else { format!("the command exited with status {code}: “{}”", lines.join(" / ")) })
        }
        None => Err("the command was stopped by a signal".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn sh(script: &str) -> Vec<String> {
        vec!["sh".into(), "-c".into(), script.into()]
    }

    #[test]
    fn the_shell_runs_the_command_as_typed() {
        if cfg!(windows) {
            return;
        }
        assert_eq!(shell_argv("linear-items --mine", Some("/bin/zsh")), ["/bin/zsh", "-lic", "linear-items --mine"]);
        assert_eq!(shell_argv("x", Some("zsh")), [crate::launch::LOGIN_SHELL, "-lic", "x"]);
        assert_eq!(shell_argv("x", None), [crate::launch::LOGIN_SHELL, "-lic", "x"]);
    }

    #[cfg(unix)]
    #[test]
    fn stdout_is_the_answer() {
        assert_eq!(run(&sh("echo '{\"items\":[]}'"), None, TIMEOUT), Ok("{\"items\":[]}\n".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn a_failure_says_its_status_and_first_stderr_lines() {
        assert_eq!(run(&sh("echo one >&2; echo two >&2; echo three >&2; echo four >&2; exit 3"), None, TIMEOUT), Err("the command exited with status 3: “one / two / three”".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn a_timeout_kills_the_command_and_its_children() {
        let start = Instant::now();
        assert_eq!(run(&sh("sleep 30 & sleep 30"), None, Duration::from_millis(300)), Err("the command did not finish within 30 s".to_string()));
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn too_much_output_is_an_error() {
        assert_eq!(run(&sh("head -c 5000000 /dev/zero"), None, TIMEOUT), Err("the command printed more than 4 MB".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn item_data_reaches_the_command_only_on_stdin() {
        let dir = std::env::temp_dir().join(format!("maya-custom-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = format!("cd '{}' && cat > got.json", dir.display());
        let json = r#"{"id":"$(touch pwned)"}"#;
        assert!(run(&sh(&script), Some(json), TIMEOUT).is_ok());
        assert_eq!(std::fs::read_to_string(dir.join("got.json")).unwrap(), json);
        assert!(!dir.join("pwned").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
