use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct RegistrySession {
    pub pid: i32,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub cwd: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(rename = "startedAt", default)]
    pub started_at: u64,
    #[serde(rename = "statusUpdatedAt", default)]
    pub status_updated_at: u64,
    /// "cli" for terminal sessions; "sdk-cli" for headless `claude -p` runs
    /// (e.g. plugin helpers). Missing means an older registry entry: treat as cli.
    #[serde(default = "default_entrypoint")]
    pub entrypoint: String,
    /// Unix socket where other processes can post messages to this session.
    #[serde(rename = "messagingSocketPath", default)]
    pub messaging_socket_path: Option<String>,
    /// The session's terminal device. Claude Code does not write it; when
    /// present it is used instead of asking `ps`.
    #[serde(default)]
    pub tty: Option<String>,
}

fn default_entrypoint() -> String {
    "cli".to_string()
}

impl RegistrySession {
    /// True for sessions a person is driving from a terminal. Headless SDK
    /// runs can never wait on the user, so the board hides them.
    pub fn is_interactive_cli(&self) -> bool {
        self.entrypoint == "cli"
    }
}

pub fn parse(json: &str) -> Result<RegistrySession, serde_json::Error> {
    serde_json::from_str(json)
}

/// Longest project folder name before Claude Code shortens it.
const PROJECT_NAME_MAX: usize = 200;

/// The folder under `~/.claude/projects` Claude Code keeps `cwd`'s
/// transcripts in: every UTF-16 unit that is not an ASCII letter or digit
/// becomes `-` (`/Users/x/my.app` is `-Users-x-my-app`, `E:\dev\maya` is
/// `E--dev-maya`), and a name over 200 characters is cut and suffixed with
/// a base-36 hash of `cwd`, as Claude Code does.
pub fn project_dir_name(cwd: &str) -> String {
    let name: String = cwd.chars().flat_map(|c| std::iter::repeat_n(if c.is_ascii_alphanumeric() { c } else { '-' }, if c.is_ascii_alphanumeric() { 1 } else { c.len_utf16() })).collect();
    if name.len() <= PROJECT_NAME_MAX {
        return name;
    }
    let hash = cwd.encode_utf16().fold(0i32, |h, u| h.wrapping_shl(5).wrapping_sub(h).wrapping_add(u as i32));
    format!("{}-{}", &name[..PROJECT_NAME_MAX], base36((hash as i64).unsigned_abs()))
}

fn base36(mut n: u64) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    loop {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// `~/.claude/projects/<project_dir_name(cwd)>/<sessionId>.jsonl`
pub fn transcript_path(claude_dir: &Path, s: &RegistrySession) -> PathBuf {
    claude_dir.join("projects").join(project_dir_name(&s.cwd)).join(format!("{}.jsonl", s.session_id))
}

/// True if a process with this pid exists and has not exited.
#[cfg(windows)]
pub fn pid_alive(pid: i32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    if pid <= 0 {
        return false;
    }
    // SAFETY: OpenProcess returns null or a handle we close; the exit code is written to a valid u32.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid as u32);
        if h.is_null() {
            // Access denied still means the process exists.
            return std::io::Error::last_os_error().raw_os_error() == Some(5);
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code) != 0;
        CloseHandle(h);
        ok && code == STILL_ACTIVE as u32
    }
}

/// True if a process with this pid exists (EPERM also means it exists).
#[cfg(unix)]
pub fn pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    let r = unsafe { libc::kill(pid, 0) };
    if r == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Every parseable `*.json` in `dir` whose pid passes `alive` and which is a
/// terminal (`cli`) session, sorted by name.
pub fn list(dir: &Path, alive: &dyn Fn(i32) -> bool) -> Vec<RegistrySession> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let Ok(session) = parse(&text) else { continue };
        if session.is_interactive_cli() && alive(session.pid) {
            out.push(session);
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_dir_names_match_claude_code() {
        assert_eq!(project_dir_name("/Users/x/dev/eye"), "-Users-x-dev-eye");
        assert_eq!(project_dir_name("E:\\dev\\maya"), "E--dev-maya");
        assert_eq!(project_dir_name("C:\\WINDOWS\\system32"), "C--WINDOWS-system32");
        assert_eq!(project_dir_name("/Users/x/my.app_v2"), "-Users-x-my-app-v2");
        // One UTF-16 unit is one dash: two for a character outside the BMP.
        assert_eq!(project_dir_name("/caf\u{e9}/\u{1f600}"), "-caf----");
    }

    #[test]
    fn long_project_dir_names_are_cut_and_hashed() {
        // "feo44x" is what Claude Code's `Math.abs(hash).toString(36)` gives for this path.
        let name = project_dir_name(&format!("/{}", "a".repeat(250)));
        assert_eq!(name, format!("-{}-feo44x", "a".repeat(199)));
        assert_eq!(base36(0), "0");
        assert_eq!(base36(36), "10");
        assert_eq!(base36(2_147_483_648), "zik0zk", "the abs of i32::MIN");
    }
    use std::path::Path;

    fn fixtures() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/registry")
    }

    #[test]
    fn parses_real_registry_file() {
        let s = parse(&std::fs::read_to_string(fixtures().join("49643.json")).unwrap()).unwrap();
        assert_eq!(s.pid, 49643);
        assert_eq!(s.session_id, "af62296a-4054-44ff-8c47-65ac4f472599");
        assert_eq!(s.cwd, "/Users/tiagocorreia/dev/eye");
        assert_eq!(s.name, "eye-3b");
        assert_eq!(s.status, "busy");
        assert_eq!(s.status_updated_at, 1790587602132);
        assert_eq!(s.started_at, 1790587340040);
        assert_eq!(s.messaging_socket_path.as_deref(), Some("/tmp/cc-socks/49643.sock"));
    }

    #[test]
    fn derives_transcript_path_from_cwd() {
        let s = parse(&std::fs::read_to_string(fixtures().join("49643.json")).unwrap()).unwrap();
        let p = transcript_path(Path::new("/Users/tiagocorreia/.claude"), &s);
        assert_eq!(
            p,
            Path::new("/Users/tiagocorreia/.claude/projects/-Users-tiagocorreia-dev-eye/af62296a-4054-44ff-8c47-65ac4f472599.jsonl")
        );
    }

    #[test]
    fn list_skips_bad_files_and_dead_pids() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::copy(fixtures().join("49643.json"), dir.path().join("49643.json")).unwrap();
        std::fs::copy(fixtures().join("bad.json"), dir.path().join("bad.json")).unwrap();
        let mut dead = std::fs::read_to_string(fixtures().join("49643.json")).unwrap();
        dead = dead.replace("\"pid\":49643", "\"pid\":1").replace("eye-3b", "dead-1");
        std::fs::write(dir.path().join("1.json"), dead).unwrap();

        let all = list(dir.path(), &|_| true);
        assert_eq!(all.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["dead-1", "eye-3b"]);

        let live = list(dir.path(), &|pid| pid == 49643);
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].name, "eye-3b");
    }

    #[test]
    fn list_hides_headless_sdk_runs_but_keeps_entries_without_entrypoint() {
        let dir = tempfile::tempdir().unwrap();
        let base = std::fs::read_to_string(fixtures().join("49643.json")).unwrap();
        std::fs::write(dir.path().join("49643.json"), &base).unwrap();
        let headless = base.replace("\"entrypoint\":\"cli\"", "\"entrypoint\":\"sdk-cli\"").replace("eye-3b", "t-c4").replace("\"pid\":49643", "\"pid\":2");
        std::fs::write(dir.path().join("2.json"), headless).unwrap();
        let legacy = base.replace("\"entrypoint\":\"cli\",", "").replace("eye-3b", "legacy-1").replace("\"pid\":49643", "\"pid\":3");
        assert!(!legacy.contains("entrypoint"));
        std::fs::write(dir.path().join("3.json"), legacy).unwrap();

        let names: Vec<String> = list(dir.path(), &|_| true).into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["eye-3b", "legacy-1"]);
    }

    #[test]
    fn own_pid_is_alive_and_huge_pid_is_not() {
        assert!(pid_alive(std::process::id() as i32));
        assert!(!pid_alive(2_000_000_000));
    }
}
