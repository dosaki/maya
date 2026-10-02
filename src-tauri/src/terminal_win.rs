//! Windows Terminal: new sessions open in a window of their own running Git
//! Bash (the shell lines Maya builds are POSIX, and Claude Code needs Git
//! Bash on Windows anyway); keys and focus go through the session's console
//! (see `maya_core::win_console`).

use maya_core::terminal::Terminal;
use maya_core::win_console;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Writes `command` to a script under `~/.claude/maya/launch` that deletes
/// itself, runs the command, then leaves an interactive shell open as
/// Terminal.app does. A file sidesteps the quoting of two command lines
/// (wt.exe's, then bash's) and wt.exe reading `;` as its own separator.
pub fn launch_script(dir: &Path, command: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let path = dir.join(format!("{stamp}.sh"));
    std::fs::write(&path, script_text(command)).map_err(|e| format!("cannot write the launch script: {e}"))?;
    Ok(path)
}

pub fn script_text(command: &str) -> String {
    format!("rm -f -- \"$0\"\n{command}\nexec bash -li\n")
}

/// `wt.exe` when Windows Terminal is installed (it is an app-execution alias
/// under WindowsApps, found on PATH).
fn windows_terminal() -> Option<PathBuf> {
    std::env::var("PATH").ok().and_then(|path| maya_core::launch::find_on_path(&path, "wt.exe"))
}

/// The wt.exe arguments that open `bash script` in a new window titled `title`.
pub fn wt_args(bash: &Path, script: &Path, cwd: &Path, title: &str) -> Vec<String> {
    let mut args = vec!["-w".to_string(), "new".to_string(), "new-tab".to_string()];
    if !title.is_empty() {
        args.extend(["--title".to_string(), title.replace(';', ",")]);
    }
    if !cwd.as_os_str().is_empty() {
        args.extend(["-d".to_string(), cwd.display().to_string()]);
    }
    args.extend([bash.display().to_string(), "-l".to_string(), script.display().to_string()]);
    args
}

/// Runs the POSIX shell line `cmd` in a new terminal window in `cwd`.
pub fn open_terminal_in(cmd: &str, cwd: &Path, title: &str) -> Result<(), String> {
    let bash = maya_core::launch::git_bash().ok_or("Git for Windows is not installed; Maya needs its bash to start sessions (https://git-scm.com/download/win).")?;
    let script = launch_script(&maya_core::claude_dir().join("maya").join("launch"), cmd)?;
    let result = match windows_terminal() {
        Some(wt) => Command::new(wt).args(wt_args(&bash, &script, cwd, title)).creation_flags(CREATE_NO_WINDOW).spawn().map(drop),
        // Without Windows Terminal, a console window of its own, in `cwd`
        // when there is one (a review's script changes folder itself).
        None => {
            let mut cmd = Command::new(&bash);
            cmd.args(["-l".as_ref(), script.as_os_str()]).creation_flags(CREATE_NEW_CONSOLE);
            if !cwd.as_os_str().is_empty() {
                cmd.current_dir(cwd);
            }
            cmd.spawn().map(drop)
        }
    };
    result.map_err(|e| {
        let _ = std::fs::remove_file(&script);
        format!("could not open a terminal: {e}")
    })
}

/// Runs `cmd` in a new terminal window, as the review flow does on macOS.
pub fn open_terminal_with(cmd: &str) -> Result<(), String> {
    open_terminal_in(cmd, Path::new(""), "Maya")
}

/// Brings the window showing `pid`'s console to the front.
pub fn focus_pid(pid: i32) -> Result<(), String> {
    let window = win_console::console_window(pid as u32)?;
    raise(window)
}

/// Raises `window` over the others. Windows only lets the foreground
/// process hand the foreground on, which Maya is when the user clicks; when
/// it is not (a voice command), attaching to the foreground thread's input
/// lets the call through.
fn raise(window: windows_sys::Win32::Foundation::HWND) -> Result<(), String> {
    use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows_sys::Win32::UI::WindowsAndMessaging::{BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE};
    // SAFETY: plain Win32 calls on window handles; the input attach is undone.
    unsafe {
        if IsIconic(window) != 0 {
            ShowWindow(window, SW_RESTORE);
        }
        if SetForegroundWindow(window) != 0 {
            return Ok(());
        }
        let fg_thread = GetWindowThreadProcessId(GetForegroundWindow(), std::ptr::null_mut());
        let me = GetCurrentThreadId();
        let attached = fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, 1) != 0;
        BringWindowToTop(window);
        let ok = SetForegroundWindow(window) != 0;
        if attached {
            AttachThreadInput(me, fg_thread, 0);
        }
        if ok {
            Ok(())
        } else {
            Err("Windows would not bring the terminal forward; it is flashing in the taskbar.".into())
        }
    }
}

/// Windows Terminal, or a console window where it is not installed.
pub struct WindowsTerminal;

/// The app's one terminal, borrowed by every local action.
pub static TERMINAL: WindowsTerminal = WindowsTerminal;

fn pid_of(tty: &str) -> Result<u32, String> {
    win_console::console_pid(tty).ok_or_else(|| format!("{tty} is not a console Maya can reach."))
}

impl Terminal for WindowsTerminal {
    fn open(&self, command: &str, cwd: &Path, label: &str) -> Result<Option<String>, String> {
        open_terminal_in(command, cwd, label).map(|_| None)
    }
    fn type_line(&self, tty: &str, text: &str) -> Result<(), String> {
        win_console::type_line(pid_of(tty)?, text)
    }
    fn focus(&self, tty: &str) -> Result<(), String> {
        focus_pid(pid_of(tty)? as i32)
    }
    fn name_for_tty(&self, _tty: &str) -> Option<String> {
        None
    }
    fn reach_after_exit(&self, tty: &str) -> Vec<String> {
        let others = pid_of(tty).and_then(win_console::other_console_pids).unwrap_or_default();
        others.into_iter().map(|p| win_console::console_key(p as i32)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launch_script_removes_itself_runs_the_line_and_keeps_a_shell() {
        let dir = tempfile::tempdir().unwrap();
        let p = launch_script(dir.path(), "cd 'E:\\dev\\x' && claude -- \"$p\"").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "rm -f -- \"$0\"\ncd 'E:\\dev\\x' && claude -- \"$p\"\nexec bash -li\n");
        assert_eq!(p.extension().unwrap(), "sh");
    }

    #[test]
    fn wt_opens_a_new_window_with_title_and_folder() {
        let a = wt_args(Path::new("C:\\Git\\bin\\bash.exe"), Path::new("C:\\u\\1.sh"), Path::new("E:\\dev\\x"), "x; y");
        assert_eq!(a, ["-w", "new", "new-tab", "--title", "x, y", "-d", "E:\\dev\\x", "C:\\Git\\bin\\bash.exe", "-l", "C:\\u\\1.sh"]);
        let a = wt_args(Path::new("b"), Path::new("s"), Path::new(""), "");
        assert_eq!(a, ["-w", "new", "new-tab", "b", "-l", "s"]);
    }

    #[test]
    fn keys_are_consoles_not_ttys() {
        assert_eq!(pid_of("console:7"), Ok(7));
        assert!(pid_of("/dev/ttys001").is_err());
        assert!(TERMINAL.focus("console:2000000000").is_err());
    }
}
