//! The CLI's own run status, written by `maya run` and read by `maya status`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
pub struct RunStatus {
    pub pid: u32,
    pub connected: bool,
    pub main_name: Option<String>,
    pub error: Option<String>,
    pub updated_ms: u64,
}

pub fn path(maya_dir: &Path) -> PathBuf {
    maya_dir.join("cli-status.json")
}

/// Writes via a temp file and rename, so a half-written file is never read.
/// Called by `maya run` (Task 7); unused outside tests until then.
#[allow(dead_code)]
pub fn write(maya_dir: &Path, s: &RunStatus) -> Result<(), String> {
    std::fs::create_dir_all(maya_dir).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    let tmp = maya_dir.join("cli-status.json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path(maya_dir)).map_err(|e| e.to_string())
}

/// `None` on any error: missing file, unreadable, or not valid JSON.
pub fn read(maya_dir: &Path) -> Option<RunStatus> {
    let text = std::fs::read_to_string(path(maya_dir)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Called by `maya run` (Task 7); unused outside tests until then.
#[allow(dead_code)]
pub fn remove(maya_dir: &Path) {
    let _ = std::fs::remove_file(path(maya_dir));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_file_round_trips_and_is_gone_after_remove() {
        let d = tempfile::tempdir().unwrap();
        let s = RunStatus { pid: 7, connected: true, main_name: Some("Yhi".into()), error: None, updated_ms: 5 };
        write(d.path(), &s).unwrap();
        assert_eq!(read(d.path()), Some(s));
        remove(d.path());
        assert_eq!(read(d.path()), None);
    }
}
