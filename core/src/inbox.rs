#[cfg(unix)]
use std::io::Write;
#[cfg(all(unix, not(target_os = "android")))]
use std::os::unix::io::AsRawFd;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::Path;
#[cfg(unix)]
use std::time::Duration;

/// Well under Claude Code's ~1M character cap for same-machine messages.
pub const MAX_CHARS: usize = 100_000;

/// One stream-json user message, newline-terminated, as documented for the
/// session inbox socket.
pub fn message_line(text: &str) -> String {
    let v = serde_json::json!({"type": "user", "message": {"role": "user", "content": text}});
    format!("{v}\n")
}

/// Pid of the process on the other end of a Unix socket (macOS LOCAL_PEERPID).
#[cfg(target_os = "macos")]
pub fn peer_pid(stream: &UnixStream) -> Option<i32> {
    let mut pid: libc::pid_t = 0;
    let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    let r = unsafe {
        libc::getsockopt(stream.as_raw_fd(), libc::SOL_LOCAL, libc::LOCAL_PEERPID, &mut pid as *mut _ as *mut libc::c_void, &mut len)
    };
    if r == 0 {
        Some(pid)
    } else {
        None
    }
}

/// Pid of the process on the other end of a Unix socket (Linux SO_PEERCRED).
#[cfg(target_os = "linux")]
pub fn peer_pid(stream: &UnixStream) -> Option<i32> {
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let r = unsafe {
        libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, &mut cred as *mut _ as *mut libc::c_void, &mut len)
    };
    if r == 0 {
        Some(cred.pid)
    } else {
        None
    }
}

/// Elsewhere the owner cannot be checked, so `send` refuses.
#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
pub fn peer_pid(_stream: &UnixStream) -> Option<i32> {
    None
}

/// Why `text` cannot be sent at all, whatever the transport.
fn check_text(text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("Message is empty.".into());
    }
    if text.chars().count() > MAX_CHARS {
        return Err(format!("Message is too long (over {MAX_CHARS} characters)."));
    }
    Ok(())
}

/// The line that must open every connection to a session's inbox on
/// native Windows (optional elsewhere), as documented for the inbox socket.
pub fn auth_line(token: &str) -> String {
    let v = serde_json::json!({"type": "auth", "token": token});
    format!("{v}\n")
}

/// The inbox token Maya's hook recorded for the session on `pipe_path`
/// (Claude Code hands it to hooks as `CLAUDE_CODE_MESSAGING_TOKEN`).
#[cfg(windows)]
fn session_token(maya_dir: &Path, pipe_path: &Path) -> Result<String, String> {
    let missing = "Maya has not seen this session's inbox token yet: it arrives with the session's next hook event (install the hook in Settings), or use the terminal.";
    let path = crate::hook_install::token_path(maya_dir, &pipe_path.to_string_lossy()).ok_or(missing)?;
    let token = std::fs::read_to_string(&path).map_err(|_| missing)?;
    let token = token.trim();
    if token.is_empty() {
        return Err(missing.into());
    }
    Ok(token.to_string())
}

/// Pid of the process that created the named pipe `file` is connected to.
#[cfg(windows)]
pub fn pipe_server_pid(file: &std::fs::File) -> Option<i32> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
    let mut pid = 0u32;
    // SAFETY: a valid pipe handle and a u32 to write to.
    let ok = unsafe { GetNamedPipeServerProcessId(file.as_raw_handle() as _, &mut pid) };
    (ok != 0).then_some(pid as i32)
}

/// Posts `text` into the session inbox on the named pipe `pipe_path`: the
/// auth line with the session's token, then the message. Refused unless
/// the pipe was created by `expected_pid`.
#[cfg(windows)]
pub fn send(pipe_path: &Path, expected_pid: i32, text: &str) -> Result<(), String> {
    check_text(text)?;
    let token = session_token(&crate::claude_dir().join("maya"), pipe_path)?;
    send_with_token(pipe_path, expected_pid, &token, text)
}

#[cfg(windows)]
pub fn send_with_token(pipe_path: &Path, expected_pid: i32, token: &str, text: &str) -> Result<(), String> {
    use std::io::Write;
    if !pipe_path.to_string_lossy().starts_with(r"\\.\pipe\") {
        return Err("Refusing to send: the session's inbox is not a named pipe.".into());
    }
    let mut pipe = std::fs::OpenOptions::new().read(true).write(true).open(pipe_path).map_err(|e| format!("Could not connect to the session inbox: {e}."))?;
    match pipe_server_pid(&pipe) {
        Some(p) if p == expected_pid => {}
        Some(_) => return Err("Refusing to send: the pipe is not owned by that session.".into()),
        None => return Err("Refusing to send: could not verify who owns the pipe.".into()),
    }
    // One write, so the server sees the auth line and the message together.
    let payload = auth_line(token) + &message_line(text);
    pipe.write_all(payload.as_bytes()).map_err(|e| format!("Could not write to the session inbox: {e}."))?;
    Ok(())
}

/// Posts `text` into the session inbox at `socket_path`, refusing unless the
/// listening process is `expected_pid`.
#[cfg(unix)]
pub fn send(socket_path: &Path, expected_pid: i32, text: &str) -> Result<(), String> {
    check_text(text)?;
    let mut stream = UnixStream::connect(socket_path).map_err(|e| format!("Could not connect to the session inbox: {e}."))?;
    stream.set_write_timeout(Some(Duration::from_secs(5))).map_err(|e| e.to_string())?;
    match peer_pid(&stream) {
        Some(p) if p == expected_pid => {}
        Some(_) => return Err("Refusing to send: the socket is not owned by that session.".into()),
        None => return Err("Refusing to send: could not verify who owns the socket.".into()),
    }
    stream.write_all(message_line(text).as_bytes()).map_err(|e| format!("Could not write to the session inbox: {e}."))?;
    stream.flush().map_err(|e| e.to_string())?;
    let _ = stream.shutdown(std::net::Shutdown::Write);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_line_is_stream_json_user_message() {
        assert_eq!(message_line("hi"), "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n");
        let v: serde_json::Value = serde_json::from_str(message_line("a \"q\"\nb").trim()).unwrap();
        assert_eq!(v["message"]["content"], "a \"q\"\nb");
    }

    #[test]
    fn auth_line_is_the_documented_json() {
        assert_eq!(auth_line("ab\"c"), "{\"type\":\"auth\",\"token\":\"ab\\\"c\"}\n");
    }

    #[test]
    fn refuses_empty_and_oversized_text() {
        let nope = Path::new("nope.sock");
        assert!(send(nope, 1, "  ").unwrap_err().contains("empty"));
        assert!(send(nope, 1, &"x".repeat(MAX_CHARS + 1)).unwrap_err().contains("too long"));
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{ReadFile, PIPE_ACCESS_DUPLEX};
    use windows_sys::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT};

    /// A pipe server in this process: returns its path and a thread that
    /// yields everything one client wrote.
    fn serve() -> (std::path::PathBuf, std::thread::JoinHandle<String>) {
        let name = format!(r"\\.\pipe\LOCAL\maya-test-{}-{}", std::process::id(), rand::random::<u64>());
        let wide: Vec<u16> = std::ffi::OsStr::new(&name).encode_wide().chain([0]).collect();
        // SAFETY: a NUL-terminated name; null security attributes.
        let h = unsafe { CreateNamedPipeW(wide.as_ptr(), PIPE_ACCESS_DUPLEX, PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT, 1, 4096, 4096, 0, std::ptr::null()) };
        assert_ne!(h, INVALID_HANDLE_VALUE);
        let h = h as usize;
        let server = std::thread::spawn(move || {
            let h = h as windows_sys::Win32::Foundation::HANDLE;
            let mut out = Vec::new();
            // SAFETY: a valid pipe handle, read into a stack buffer of the given length.
            unsafe {
                ConnectNamedPipe(h, std::ptr::null_mut());
                let mut buf = [0u8; 4096];
                loop {
                    let mut n = 0u32;
                    if ReadFile(h, buf.as_mut_ptr(), buf.len() as u32, &mut n, std::ptr::null_mut()) == 0 || n == 0 {
                        break;
                    }
                    out.extend_from_slice(&buf[..n as usize]);
                }
                CloseHandle(h);
            }
            String::from_utf8(out).unwrap()
        });
        (name.into(), server)
    }

    #[test]
    fn sends_the_auth_line_then_the_message() {
        let (path, server) = serve();
        send_with_token(&path, std::process::id() as i32, "tok", "line one\nline \"two\"").unwrap();
        assert_eq!(server.join().unwrap(), auth_line("tok") + &message_line("line one\nline \"two\""));
    }

    #[test]
    fn refuses_a_pipe_another_process_owns_without_writing() {
        let (path, server) = serve();
        let err = send_with_token(&path, 1, "tok", "x").unwrap_err();
        assert!(err.contains("not owned by that session"), "{err}");
        assert_eq!(server.join().unwrap(), "");
    }

    #[test]
    fn the_token_is_the_one_the_hook_recorded_for_that_pipe() {
        let dir = tempfile::tempdir().unwrap();
        let pipe = Path::new(r"\\.\pipe\LOCAL\cc-msg-feed");
        assert!(session_token(dir.path(), pipe).unwrap_err().contains("next hook event"));
        crate::hook_install::record_token(dir.path(), "Stop", Some(&pipe.to_string_lossy()), Some("tok")).unwrap();
        assert_eq!(session_token(dir.path(), pipe).unwrap(), "tok");
        assert!(session_token(dir.path(), Path::new(r"\\.\pipe\LOCAL\cc-msg-other")).is_err());
    }

    #[test]
    fn refuses_what_is_not_a_named_pipe() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("inbox");
        std::fs::write(&file, "").unwrap();
        assert!(send_with_token(&file, 1, "tok", "x").unwrap_err().contains("not a named pipe"));
    }
}

#[cfg(all(test, unix))]
mod unix_tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::net::UnixListener;

    fn listen(dir: &Path) -> (UnixListener, std::path::PathBuf) {
        let p = dir.join("s.sock");
        (UnixListener::bind(&p).unwrap(), p)
    }

    #[test]
    fn peer_pid_is_the_listening_process() {
        let dir = tempfile::tempdir().unwrap();
        let (_listener, path) = listen(dir.path());
        let stream = UnixStream::connect(&path).unwrap();
        assert_eq!(peer_pid(&stream), Some(std::process::id() as i32));
    }

    #[test]
    fn send_writes_exactly_one_line_to_the_socket() {
        let dir = tempfile::tempdir().unwrap();
        let (listener, path) = listen(dir.path());
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = String::new();
            s.read_to_string(&mut buf).unwrap();
            buf
        });
        send(&path, std::process::id() as i32, "line one\nline \"two\"").unwrap();
        assert_eq!(server.join().unwrap(), message_line("line one\nline \"two\""));
    }

    #[test]
    fn refuses_wrong_peer_pid_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let (listener, path) = listen(dir.path());
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = String::new();
            s.read_to_string(&mut buf).unwrap();
            buf
        });
        let err = send(&path, 1, "x").unwrap_err();
        assert!(err.contains("not owned by that session"), "{err}");
        assert_eq!(server.join().unwrap(), "");
    }

    #[test]
    fn refuses_a_missing_socket() {
        let dir = tempfile::tempdir().unwrap();
        assert!(send(&dir.path().join("nope.sock"), 1, "x").is_err());
    }
}
