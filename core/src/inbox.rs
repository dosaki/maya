use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
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
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn peer_pid(_stream: &UnixStream) -> Option<i32> {
    None
}

/// Posts `text` into the session inbox at `socket_path`, refusing unless the
/// listening process is `expected_pid`.
pub fn send(socket_path: &Path, expected_pid: i32, text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("Message is empty.".into());
    }
    if text.chars().count() > MAX_CHARS {
        return Err(format!("Message is too long (over {MAX_CHARS} characters)."));
    }
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
    use std::io::Read;
    use std::os::unix::net::UnixListener;

    fn listen(dir: &Path) -> (UnixListener, std::path::PathBuf) {
        let p = dir.join("s.sock");
        (UnixListener::bind(&p).unwrap(), p)
    }

    #[test]
    fn message_line_is_stream_json_user_message() {
        assert_eq!(message_line("hi"), "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n");
        let v: serde_json::Value = serde_json::from_str(message_line("a \"q\"\nb").trim()).unwrap();
        assert_eq!(v["message"]["content"], "a \"q\"\nb");
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
    fn refuses_missing_socket_empty_and_oversized_text() {
        let dir = tempfile::tempdir().unwrap();
        assert!(send(&dir.path().join("nope.sock"), 1, "x").is_err());
        assert!(send(&dir.path().join("nope.sock"), 1, "  ").unwrap_err().contains("empty"));
        assert!(send(&dir.path().join("nope.sock"), 1, &"x".repeat(MAX_CHARS + 1)).unwrap_err().contains("too long"));
    }
}
