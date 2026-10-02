//! A session's console on Windows: Windows has no ttys, so Maya reaches a
//! session through the console its process is attached to. Keys are written
//! into that console's input, as if typed, and the window hosting it (a
//! Windows Terminal window, or a classic console) is what focus raises.
//!
//! A process has at most one console, so every attach happens under one
//! lock and is undone before it is released.

use std::sync::Mutex;
use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HWND, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows_sys::Win32::System::Console::{AttachConsole, FreeConsole, GetConsoleProcessList, GetConsoleWindow, WriteConsoleInputW, INPUT_RECORD, KEY_EVENT, KEY_EVENT_RECORD, KEY_EVENT_RECORD_0, ENHANCED_KEY, SHIFT_PRESSED};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{MapVirtualKeyW, MAPVK_VK_TO_VSC, VK_DOWN, VK_PACKET, VK_RETURN, VK_TAB};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetAncestor, GA_ROOTOWNER};

static ATTACH: Mutex<()> = Mutex::new(());

/// The terminal key Maya uses for `pid`'s console, in place of a tty path.
pub fn console_key(pid: i32) -> String {
    format!("console:{pid}")
}

/// The pid in a `console:<pid>` key.
pub fn console_pid(key: &str) -> Option<u32> {
    key.strip_prefix("console:")?.parse().ok().filter(|&p| p > 0)
}

/// One key press Maya types.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Key {
    /// A UTF-16 unit of text.
    Unit(u16),
    Down,
    /// Shift+Tab.
    BackTab,
    Enter,
}

/// `text` as key presses: the escape sequences Maya's answers use (Down
/// `ESC [ B`, Shift+Tab `ESC [ Z`) become those keys, the rest is typed.
pub fn keys_for(text: &str) -> Vec<Key> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if let Some(r) = rest.strip_prefix("\x1b[B") {
            out.push(Key::Down);
            rest = r;
        } else if let Some(r) = rest.strip_prefix("\x1b[Z") {
            out.push(Key::BackTab);
            rest = r;
        } else {
            let mut buf = [0u16; 2];
            out.extend(c.encode_utf16(&mut buf).iter().map(|&u| Key::Unit(u)));
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

fn record(vk: u16, unit: u16, down: bool, state: u32) -> INPUT_RECORD {
    // SAFETY: MapVirtualKeyW only reads its arguments; INPUT_RECORD is plain data.
    let scan = if vk == VK_PACKET { 0 } else { unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) as u16 } };
    let mut r: INPUT_RECORD = unsafe { std::mem::zeroed() };
    r.EventType = KEY_EVENT as u16;
    r.Event.KeyEvent = KEY_EVENT_RECORD { bKeyDown: down as i32, wRepeatCount: 1, wVirtualKeyCode: vk, wVirtualScanCode: scan, uChar: KEY_EVENT_RECORD_0 { UnicodeChar: unit }, dwControlKeyState: state };
    r
}

/// A press and a release for every key.
pub fn records(keys: &[Key]) -> Vec<INPUT_RECORD> {
    keys.iter()
        .flat_map(|k| {
            let (vk, unit, state) = match *k {
                Key::Unit(u) => (VK_PACKET, u, 0),
                Key::Down => (VK_DOWN, 0, ENHANCED_KEY),
                Key::BackTab => (VK_TAB, '\t' as u16, SHIFT_PRESSED),
                Key::Enter => (VK_RETURN, '\r' as u16, 0),
            };
            [record(vk, unit, true, state), record(vk, unit, false, state)]
        })
        .collect()
}

/// Runs `f` while attached to `pid`'s console.
fn attached<T>(pid: u32, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let _guard = ATTACH.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: plain Win32 calls; the attach is undone before the lock is released.
    unsafe {
        FreeConsole();
        if AttachConsole(pid) == 0 {
            return Err(format!("Could not reach the console of process {pid}: {}.", std::io::Error::last_os_error()));
        }
    }
    let out = f();
    // SAFETY: detaches from the console attached above.
    unsafe { FreeConsole() };
    out
}

/// Types `text`, then Enter, into `pid`'s console.
pub fn type_line(pid: u32, text: &str) -> Result<(), String> {
    let mut keys = keys_for(text);
    keys.push(Key::Enter);
    let recs = records(&keys);
    attached(pid, || {
        let name: Vec<u16> = "CONIN$\0".encode_utf16().collect();
        // SAFETY: a NUL-terminated name; the handle is closed below.
        let input = unsafe { CreateFileW(name.as_ptr(), GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut()) };
        if input == INVALID_HANDLE_VALUE {
            return Err(format!("Could not open the console's input: {}.", std::io::Error::last_os_error()));
        }
        let mut done = 0usize;
        let mut result = Ok(());
        while done < recs.len() {
            let mut n = 0u32;
            // SAFETY: a valid input handle and a slice of records of the given length.
            if unsafe { WriteConsoleInputW(input, recs[done..].as_ptr(), (recs.len() - done) as u32, &mut n) } == 0 || n == 0 {
                result = Err(format!("Could not type into the console: {}.", std::io::Error::last_os_error()));
                break;
            }
            done += n as usize;
        }
        // SAFETY: closes the handle opened above.
        unsafe { CloseHandle(input) };
        result
    })
}

/// The processes attached to `pid`'s console other than `pid` and Maya,
/// which attaches to read the list: what still reaches the console once
/// `pid` has exited.
pub fn other_console_pids(pid: u32) -> Result<Vec<u32>, String> {
    let all = attached(pid, || {
        let mut list = vec![0u32; 64];
        loop {
            // SAFETY: the buffer holds `list.len()` pids.
            let n = unsafe { GetConsoleProcessList(list.as_mut_ptr(), list.len() as u32) } as usize;
            if n == 0 {
                return Err(format!("Could not list the console's processes: {}.", std::io::Error::last_os_error()));
            }
            if n <= list.len() {
                list.truncate(n);
                return Ok(list);
            }
            list.resize(n, 0);
        }
    })?;
    Ok(others(&all, pid, std::process::id()))
}

fn others(all: &[u32], pid: u32, me: u32) -> Vec<u32> {
    all.iter().copied().filter(|&p| p != pid && p != me).collect()
}

/// The top-level window showing `pid`'s console: under Windows Terminal the
/// console window is a hidden pseudo window owned by the terminal's window.
pub fn console_window(pid: u32) -> Result<HWND, String> {
    attached(pid, || {
        // SAFETY: no arguments; GetAncestor accepts any window handle.
        let w = unsafe { GetConsoleWindow() };
        if w.is_null() {
            return Err(format!("Process {pid} has no console window."));
        }
        let root = unsafe { GetAncestor(w, GA_ROOTOWNER) };
        Ok(if root.is_null() { w } else { root })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_keys_round_trip() {
        assert_eq!(console_key(42), "console:42");
        assert_eq!(console_pid("console:42"), Some(42));
        assert_eq!(console_pid("console:0"), None);
        assert_eq!(console_pid("/dev/ttys001"), None);
    }

    #[test]
    fn the_console_is_reached_through_its_other_processes_but_not_the_session_or_maya() {
        assert_eq!(others(&[30, 7, 12, 99], 7, 30), [12, 99]);
        assert_eq!(others(&[7, 30], 7, 30), Vec::<u32>::new());
    }

    #[test]
    fn answers_become_arrow_and_back_tab_presses() {
        assert_eq!(keys_for("\x1b[B\x1b[Bok"), [Key::Down, Key::Down, Key::Unit('o' as u16), Key::Unit('k' as u16)]);
        assert_eq!(keys_for("\x1b[Z"), [Key::BackTab]);
        // Text outside the BMP is two units; a lone ESC is typed as is.
        assert_eq!(keys_for("\u{1f600}\x1b"), [Key::Unit(0xd83d), Key::Unit(0xde00), Key::Unit(0x1b)]);
    }

    #[test]
    fn every_key_is_pressed_then_released() {
        let recs = records(&[Key::Unit('é' as u16), Key::Down, Key::BackTab, Key::Enter]);
        assert_eq!(recs.len(), 8);
        let k: Vec<(i32, u16, u16, u32)> = recs.iter().map(|r| unsafe { (r.Event.KeyEvent.bKeyDown, r.Event.KeyEvent.wVirtualKeyCode, r.Event.KeyEvent.uChar.UnicodeChar, r.Event.KeyEvent.dwControlKeyState) }).collect();
        assert_eq!(k[0], (1, VK_PACKET, 'é' as u16, 0));
        assert_eq!(k[1], (0, VK_PACKET, 'é' as u16, 0));
        assert_eq!(k[2], (1, VK_DOWN, 0, ENHANCED_KEY));
        assert_eq!(k[4], (1, VK_TAB, '\t' as u16, SHIFT_PRESSED));
        assert_eq!(k[6], (1, VK_RETURN, '\r' as u16, 0));
    }

    /// A console program with no window that reads one line from its
    /// console (not the pipes the test harness gives it) into `out`.
    fn reader(out: &std::path::Path) -> std::process::Child {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let script = format!("set /p x=<CON & call echo %x%> \"{}\"", out.display());
        std::process::Command::new("cmd.exe").arg("/c").raw_arg(script).creation_flags(CREATE_NO_WINDOW).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).spawn().unwrap()
    }

    #[test]
    fn types_a_line_into_another_process_console() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("line.txt");
        let mut child = reader(&out);
        // Retry until cmd is up and reading.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut typed = false;
        while std::time::Instant::now() < deadline {
            if !typed && type_line(child.id(), "hello maya").is_ok() {
                typed = true;
            }
            if let Ok(Some(_)) = child.try_wait() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        let _ = child.kill();
        assert!(typed, "could not attach to the reader's console");
        assert_eq!(std::fs::read_to_string(&out).unwrap().trim(), "hello maya");
    }

    #[test]
    fn a_dead_pid_has_no_console() {
        assert!(type_line(2_000_000_000, "x").unwrap_err().contains("Could not reach the console"));
        assert!(console_window(2_000_000_000).is_err());
    }
}
