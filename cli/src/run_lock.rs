//! The lock one `maya run` holds for its lifetime, so a second run (or a
//! `maya pair` swapping the credentials under it) is refused.
//!
//! It is an `flock` on `~/.claude/maya/cli-run.lock`: the kernel drops it
//! when the process dies, so a run killed without cleanup leaves nothing
//! that blocks the next one. The file is opened close-on-exec (Rust's
//! default), so the tmux server a run starts does not inherit it.

use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

pub fn path(maya_dir: &Path) -> PathBuf {
    maya_dir.join("cli-run.lock")
}

/// Held while this value lives; dropping it closes the file and frees the lock.
pub struct RunLock {
    _file: File,
}

impl RunLock {
    /// Takes the lock at `path` (creating the file, mode 0600) without
    /// waiting: `Ok(None)` when another open of it holds the lock, in this
    /// process or another.
    pub fn try_take(path: &Path) -> Result<Option<RunLock>, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)
            .map_err(|e| format!("could not open {}: {e}", path.display()))?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(Some(RunLock { _file: file }));
        }
        let e = std::io::Error::last_os_error();
        if e.raw_os_error() == Some(libc::EWOULDBLOCK) {
            Ok(None)
        } else {
            Err(format!("could not lock {}: {e}", path.display()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_take_on_one_path_fails_until_the_first_is_released() {
        let d = tempfile::tempdir().unwrap();
        let p = path(&d.path().join("maya"));
        let first = RunLock::try_take(&p).unwrap().expect("the first take gets the lock");
        // A second open of the same file in this same process contends too:
        // flock belongs to the open file description, not the process.
        assert!(RunLock::try_take(&p).unwrap().is_none());
        drop(first);
        assert!(RunLock::try_take(&p).unwrap().is_some(), "free once the first is dropped");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
    }
}
