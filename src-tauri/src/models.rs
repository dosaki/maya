//! Whisper models for the built-in recogniser: the table, where they live,
//! and downloading them once, verified, into `<claude_dir>/maya/models/`.

use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{LazyLock, Mutex};

pub struct WhisperModel {
    pub id: &'static str,
    pub file: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    pub label: &'static str,
}

pub const MODELS: &[WhisperModel] = &[
    WhisperModel { id: "tiny.en", file: "ggml-tiny.en.bin", bytes: 77_704_715, sha256: "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f", label: "Tiny (78 MB, fastest)" },
    WhisperModel { id: "base.en-q5_1", file: "ggml-base.en-q5_1.bin", bytes: 59_721_011, sha256: "4baf70dd0d7c4247ba2b81fafd9c01005ac77c2f9ef064e00dcf195d0e2fdd2f", label: "Base, quantised (60 MB, recommended)" },
    WhisperModel { id: "base.en", file: "ggml-base.en.bin", bytes: 147_964_211, sha256: "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002", label: "Base (148 MB)" },
    WhisperModel { id: "small.en-q5_1", file: "ggml-small.en-q5_1.bin", bytes: 190_098_681, sha256: "bfdff4894dcb76bbf647d56263ea2a96645423f1669176f4844a1bf8e478ad30", label: "Small, quantised (190 MB, most accurate)" },
];

pub use maya_core::config::DEFAULT_MODEL;
const BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

pub fn model(id: &str) -> Option<&'static WhisperModel> {
    MODELS.iter().find(|m| m.id == id)
}

pub fn models_dir(claude_dir: &Path) -> PathBuf {
    claude_dir.join("maya").join("models")
}

pub fn model_path(claude_dir: &Path, id: &str) -> Option<PathBuf> {
    model(id).map(|m| models_dir(claude_dir).join(m.file))
}

/// Present with the exact byte count. The checksum is checked at download
/// time; re-hashing 200 MB on every listener start would be too slow.
pub fn is_downloaded(claude_dir: &Path, id: &str) -> bool {
    match (model(id), model_path(claude_dir, id)) {
        (Some(m), Some(p)) => std::fs::metadata(&p).map(|md| md.len() == m.bytes).unwrap_or(false),
        _ => false,
    }
}

/// Hex SHA-256 of the file at `path`, read in 1 MB chunks.
fn sha256_of(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(|e| format!("could not read {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Stops the download child `pid`.
#[cfg(unix)]
fn kill(pid: u32) {
    unsafe { libc::kill(pid as i32, libc::SIGTERM) };
}

/// Stops the download child `pid`.
#[cfg(windows)]
fn kill(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    // SAFETY: OpenProcess returns null or a handle we close.
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !h.is_null() {
            TerminateProcess(h, 1);
            CloseHandle(h);
        }
    }
}

/// Model ids currently downloading, mapped to the curl child's pid. Guards
/// `download_whisper_model` against a second concurrent download of the same
/// model, and lets `abort_all` kill every curl child still running when Maya
/// exits, so a stalled download does not keep writing its `.part` file
/// after the app is gone.
static IN_FLIGHT: LazyLock<Mutex<HashMap<String, u32>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Whether `id` already has a download running.
pub fn is_in_flight(id: &str) -> bool {
    IN_FLIGHT.lock().unwrap().contains_key(id)
}

/// Kills every curl child still downloading, for a clean app exit.
pub fn abort_all() {
    for pid in IN_FLIGHT.lock().unwrap().values() {
        kill(*pid);
    }
}

/// The curl arguments to fetch `url` into `part`, quietly (`-fsSL`) and with
/// timeouts: `--connect-timeout 30` gives up on a connection that never
/// opens, and `--speed-limit 1024 --speed-time 60` gives up once the
/// transfer stays under 1 KB/s for 60 s (a stalled Wi-Fi link, a captive
/// portal, a CDN hiccup) instead of hanging forever.
fn curl_args(part: &Path, url: &str) -> Vec<String> {
    vec![
        "-fsSL".to_string(),
        "--connect-timeout".to_string(),
        "30".to_string(),
        "--speed-limit".to_string(),
        "1024".to_string(),
        "--speed-time".to_string(),
        "60".to_string(),
        "-o".to_string(),
        part.display().to_string(),
        url.to_string(),
    ]
}

pub fn verify(path: &Path, m: &WhisperModel) -> Result<(), String> {
    let len = std::fs::metadata(path).map_err(|e| format!("could not read {}: {e}", path.display()))?.len();
    if len != m.bytes {
        return Err(format!("wrong size: {len} bytes, expected {}", m.bytes));
    }
    let actual = sha256_of(path)?;
    if actual != m.sha256 {
        return Err(format!("checksum mismatch: {actual}"));
    }
    Ok(())
}

/// Fetches `url` to `<dir>/<file>.part`, reporting (received, total) while
/// curl runs, verifies it and renames it into place. Anything that fails
/// removes the part file.
pub fn fetch_to(dir: &Path, m: &WhisperModel, url: &str, progress: &dyn Fn(u64, u64)) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let part = dir.join(format!("{}.part", m.file));
    let target = dir.join(m.file);
    let _ = std::fs::remove_file(&part);
    let result = (|| {
        let mut child = Command::new("curl")
            .args(curl_args(&part, url))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not run curl: {e}"))?;
        IN_FLIGHT.lock().unwrap().insert(m.id.to_string(), child.id());
        loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                if !status.success() {
                    let mut err = String::new();
                    if let Some(mut e) = child.stderr.take() {
                        use std::io::Read;
                        let _ = e.read_to_string(&mut err);
                    }
                    return Err(format!("download failed: {}", if err.trim().is_empty() { format!("curl exited with {status}") } else { err.trim().to_string() }));
                }
                break;
            }
            let got = std::fs::metadata(&part).map(|md| md.len()).unwrap_or(0);
            progress(got, m.bytes);
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        verify(&part, m)?;
        std::fs::rename(&part, &target).map_err(|e| format!("could not move the model into place: {e}"))?;
        progress(m.bytes, m.bytes);
        Ok(target.clone())
    })();
    IN_FLIGHT.lock().unwrap().remove(m.id);
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

pub fn download(claude_dir: &Path, id: &str, progress: &dyn Fn(u64, u64)) -> Result<PathBuf, String> {
    let m = model(id).ok_or_else(|| format!("unknown model {id}"))?;
    fetch_to(&models_dir(claude_dir), m, &format!("{BASE_URL}/{}", m.file), progress)
}

pub fn remove(claude_dir: &Path, id: &str) -> Result<(), String> {
    let p = model_path(claude_dir, id).ok_or_else(|| format!("unknown model {id}"))?;
    match std::fs::remove_file(&p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("could not remove {}: {e}", p.display())),
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
    pub bytes: u64,
    pub downloaded: bool,
}

pub fn list(claude_dir: &Path) -> Vec<ModelInfo> {
    MODELS.iter().map(|m| ModelInfo { id: m.id.into(), label: m.label.into(), bytes: m.bytes, downloaded: is_downloaded(claude_dir, m.id) }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn the_table_has_the_four_models_with_the_default_marked() {
        assert_eq!(MODELS.len(), 4);
        assert_eq!(model("base.en-q5_1").unwrap().file, "ggml-base.en-q5_1.bin");
        assert_eq!(model("base.en-q5_1").unwrap().bytes, 59_721_011);
        assert!(model("nope").is_none());
        assert_eq!(DEFAULT_MODEL, "base.en-q5_1");
    }

    #[test]
    fn paths_live_under_maya_models() {
        let p = model_path(Path::new("/Users/x/.claude"), "tiny.en").unwrap();
        assert_eq!(p, Path::new("/Users/x/.claude/maya/models/ggml-tiny.en.bin"));
        assert!(model_path(Path::new("/x"), "nope").is_none());
    }

    #[test]
    fn downloaded_means_present_with_the_exact_size() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path();
        assert!(!is_downloaded(claude, "tiny.en"));
        let p = model_path(claude, "tiny.en").unwrap();
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"short").unwrap();
        assert!(!is_downloaded(claude, "tiny.en"), "a truncated file is not a model");
    }

    #[test]
    fn verify_checks_size_then_sha256() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("m.bin");
        let m = WhisperModel { id: "t", file: "m.bin", bytes: 3, sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad", label: "t" };
        std::fs::write(&p, b"ab").unwrap();
        assert!(verify(&p, &m).unwrap_err().contains("size"));
        std::fs::write(&p, b"abc").unwrap();
        assert!(verify(&p, &m).is_ok(), "sha256 of abc");
        std::fs::write(&p, b"abd").unwrap();
        assert!(verify(&p, &m).unwrap_err().contains("checksum"));
    }

    #[test]
    fn a_failed_download_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path();
        // An unreachable URL: the fetch fails fast.
        let m = WhisperModel { id: "t", file: "never.bin", bytes: 1, sha256: "00", label: "t" };
        let err = fetch_to(&models_dir(claude), &m, "http://127.0.0.1:9/never.bin", &|_, _| {}).unwrap_err();
        assert!(!err.is_empty());
        assert!(!models_dir(claude).join("never.bin.part").exists());
        assert!(!models_dir(claude).join("never.bin").exists());
        assert!(!is_in_flight("t"), "a failed download clears the in-flight marker");
    }

    #[test]
    fn curl_args_give_up_on_a_stalled_transfer() {
        let part = Path::new("/tmp/whatever.part");
        let args = curl_args(part, "https://example.com/x.bin");
        assert!(args.contains(&"--connect-timeout".to_string()));
        assert!(args.contains(&"30".to_string()));
        assert!(args.contains(&"--speed-limit".to_string()));
        assert!(args.contains(&"1024".to_string()));
        assert!(args.contains(&"--speed-time".to_string()));
        assert!(args.contains(&"60".to_string()));
        assert!(args.contains(&"-fsSL".to_string()));
    }

    #[test]
    fn the_in_flight_set_tracks_by_model_id() {
        assert!(!is_in_flight("ghost-model"));
        IN_FLIGHT.lock().unwrap().insert("ghost-model".to_string(), 424_242);
        assert!(is_in_flight("ghost-model"));
        IN_FLIGHT.lock().unwrap().remove("ghost-model");
        assert!(!is_in_flight("ghost-model"));
    }

    #[test]
    fn list_reports_downloaded_state() {
        let dir = tempfile::tempdir().unwrap();
        let l = list(dir.path());
        assert_eq!(l.len(), 4);
        assert!(l.iter().all(|m| !m.downloaded));
        assert_eq!(l[1].id, "base.en-q5_1");
    }

    #[test]
    fn remove_deletes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = model_path(dir.path(), "tiny.en").unwrap();
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::File::create(&p).unwrap().write_all(b"x").unwrap();
        remove(dir.path(), "tiny.en").unwrap();
        assert!(!p.exists());
        assert!(remove(dir.path(), "tiny.en").is_ok(), "removing a missing model is fine");
    }
}
