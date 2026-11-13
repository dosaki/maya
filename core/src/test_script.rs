//! Executable scripts for tests, written so that running one cannot fail
//! with "Text file busy".

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// Writes `contents` to `path` and makes it executable. A child shell
/// writes it, so this process never holds a write handle to it: a process
/// another test starts meanwhile would inherit that handle until it execs,
/// and running the script then fails on Linux with "Text file busy".
pub fn write(path: &Path, contents: &str) {
    let mut sh = Command::new("sh").arg("-c").arg("cat > \"$1\" && chmod 755 \"$1\"").arg("sh").arg(path).stdin(Stdio::piped()).spawn().unwrap();
    sh.stdin.take().unwrap().write_all(contents.as_bytes()).unwrap();
    assert!(sh.wait().unwrap().success(), "could not write {}", path.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_script_that_runs_at_once() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("fake");
        write(&p, "#!/bin/sh\necho \"hello $1\"\n");
        let out = std::process::Command::new(&p).arg("you").output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "hello you\n");
    }
}
