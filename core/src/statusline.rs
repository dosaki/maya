//! Claude Code's status line, wrapped. Claude Code pipes a JSON payload to
//! the status line command on every assistant message; it carries the
//! account's rate limits, the session's cost, context window and model,
//! none of which Maya sees otherwise. The wrapper keeps that payload per
//! session in `~/.claude/maya/usage-<session>.json`, then runs the user's
//! own command (kept in `maya/statusline-inner`) and prints its output, so
//! the terminal shows what it showed before.
use crate::hook_install::{read_settings, write_settings_with_backup};
use crate::usage::{window_label, AccountLimits, LimitWindow};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The wrapper (POSIX sh + jq). Reads stdin into a temp file, runs the
/// inner command from `$HOME/.claude/maya/statusline-inner` (if the file is
/// non-empty) with the payload on stdin capturing stdout, writes
/// `$HOME/.claude/maya/usage-<session_id>.json` atomically (tmp + mv) when
/// `.session_id` is 36 hex digits and dashes, then prints the captured
/// output. Exits 0 always. `recorded_from` is its reference: the two write
/// the same record.
pub const WRAPPER_SCRIPT: &str = r#"#!/bin/sh
# Installed by Maya (Manage All Your Agents) as Claude Code's status line.
# Keeps the payload's usage and limits in ~/.claude/maya/usage-<session>.json,
# then runs your own status line command, kept in statusline-inner, and
# prints what it printed. Always exits 0.
dir="$HOME/.claude/maya"
in="$dir/statusline-in.$$"
trap 'rm -f "$in"; exit 0' HUP INT TERM
cat > "$in" 2>/dev/null
nl='
'
out=""
if [ -s "$dir/statusline-inner" ]; then
  # The x keeps the trailing newlines that $(...) would strip.
  out=$(sh "$dir/statusline-inner" < "$in"; printf x)
  out=${out%x}
fi
line=$out
while [ "${line%"$nl"}" != "$line" ]; do line=${line%"$nl"}; done
t=$(date +%s000)
# One jq call: the session id on the first line, the record on the second,
# or nothing when the id is not 36 hex digits and dashes.
rec=$(jq -rc --arg line "$line" --arg t "$t" '
  def str: if type == "string" then . else null end;
  def num: if type == "number" then . else null end;
  select(.session_id | type == "string" and length == 36 and (explode | all(
    . == 45 or (. >= 48 and . <= 57) or (. >= 65 and . <= 70) or (. >= 97 and . <= 102))))
  | .session_id, {
      at: ($t | tonumber),
      sessionId: .session_id,
      model: (.model.id? // null | str),
      version: (.version | str),
      contextUsedPercent: (.context_window.used_percentage? // null | num
        | if . == null then null else round | if . > 100 then 100 elif . < 0 then 0 else . end end),
      contextWindowSize: (.context_window.context_window_size? // null | num
        | if . == null or . < 0 then null else floor end),
      rateLimits: .rate_limits,
      totalCostUsd: (.cost.total_cost_usd? // null | num),
      line: $line
    }' < "$in" 2>/dev/null)
if [ -n "$rec" ]; then
  id=${rec%%"$nl"*}
  case $id in
    *[!0-9a-fA-F-]*) ;;
    *)
      if [ ${#id} -eq 36 ]; then
        printf '%s\n' "${rec#*"$nl"}" > "$dir/usage-$id.json.$$" &&
          mv -f "$dir/usage-$id.json.$$" "$dir/usage-$id.json"
      fi
      ;;
  esac
fi 2>/dev/null
rm -f "$in"
printf '%s' "$out"
exit 0
"#;

/// In the unix command, `"$HOME/.claude/maya/maya-statusline"`.
pub const WRAPPER_MARKER: &str = ".claude/maya/maya-statusline";
/// The tail of the Windows command,
/// `"$HOME/.claude/maya/maya-hook.exe" statusline`.
pub const WRAPPER_EXE_MARKER: &str = ".claude/maya/maya-hook.exe\" statusline";
/// The wrapper script's file name in `~/.claude/maya`.
pub const SCRIPT_FILE: &str = "maya-statusline";
/// The user's own status line command, kept while the wrapper runs it.
pub const INNER_FILE: &str = "statusline-inner";
const USER_FILE: &str = "settings.json";
const LOCAL_FILE: &str = "settings.local.json";

#[cfg(unix)]
fn wrapper_command() -> String {
    format!("\"$HOME/{WRAPPER_MARKER}\"")
}

#[cfg(windows)]
fn wrapper_command() -> String {
    format!("\"$HOME/{WRAPPER_EXE_MARKER}")
}

fn is_wrapper(command: &str) -> bool {
    command.contains(WRAPPER_MARKER) || command.contains(WRAPPER_EXE_MARKER)
}

/// Where the managed settings file lives on this platform.
#[cfg(target_os = "macos")]
pub fn managed_settings_path() -> PathBuf {
    PathBuf::from("/Library/Application Support/ClaudeCode/managed-settings.json")
}

#[cfg(windows)]
pub fn managed_settings_path() -> PathBuf {
    PathBuf::from(r"C:\ProgramData\ClaudeCode\managed-settings.json")
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn managed_settings_path() -> PathBuf {
    PathBuf::from("/etc/claude-code/managed-settings.json")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub installed: bool,
    /// A file that overrides the user's status line, naming why install is refused.
    pub blocked_by: Option<String>,
}

fn has_status_line(settings: &Value) -> bool {
    settings.get("statusLine").is_some_and(|v| !v.is_null())
}

fn command_of(settings: &Value) -> Option<&str> {
    settings.get("statusLine")?.get("command")?.as_str()
}

/// The managed file, when it sets a status line: it wins over every user
/// file, so wrapping the user's would change nothing. A managed file Maya
/// cannot read or parse blocks nothing; Claude Code would not apply it either.
fn blocked_by(managed: &Path) -> Option<String> {
    let text = std::fs::read_to_string(managed).ok()?;
    let settings: Value = serde_json::from_str(&text).ok()?;
    has_status_line(&settings).then(|| managed.display().to_string())
}

/// The user-level file whose `statusLine` Claude Code uses, and its
/// contents: `settings.local.json` when it holds one, else `settings.json`.
fn effective(claude_dir: &Path) -> Result<(&'static str, Value), String> {
    let local = read_settings(claude_dir, LOCAL_FILE)?;
    if has_status_line(&local) {
        return Ok((LOCAL_FILE, local));
    }
    Ok((USER_FILE, read_settings(claude_dir, USER_FILE)?))
}

pub fn status(claude_dir: &Path) -> Result<Status, String> {
    status_in(claude_dir, &managed_settings_path())
}

pub(crate) fn status_in(claude_dir: &Path, managed: &Path) -> Result<Status, String> {
    if let Some(file) = blocked_by(managed) {
        return Ok(Status { installed: false, blocked_by: Some(file) });
    }
    let (_, settings) = effective(claude_dir)?;
    Ok(Status { installed: command_of(&settings).is_some_and(is_wrapper), blocked_by: None })
}

/// Installs into `settings.local.json` when it holds a `statusLine`, else
/// `settings.json`; refuses with `Err` naming the managed file when it
/// holds one. Saves the previous `command` to `maya/statusline-inner`
/// (empty file when there was none; an existing inner file is kept when
/// the current command is already the wrapper). Carries every other key of
/// the entry over. Writes the script (0755) on unix. Backs the settings
/// file up like `hook_install` does.
pub fn install_to(claude_dir: &Path) -> Result<Status, String> {
    install_in(claude_dir, &managed_settings_path())
}

pub(crate) fn install_in(claude_dir: &Path, managed: &Path) -> Result<Status, String> {
    if let Some(file) = blocked_by(managed) {
        return Err(format!("{file} sets the status line, which overrides yours, so Maya cannot record usage from it"));
    }
    let (file, mut settings) = effective(claude_dir)?;
    let previous = command_of(&settings).map(str::to_string);
    let Some(root) = settings.as_object_mut() else {
        return Err(format!("{file} is not a JSON object, refusing to modify it"));
    };
    let maya_dir = claude_dir.join("maya");
    std::fs::create_dir_all(&maya_dir).map_err(|e| format!("cannot create {}: {e}", maya_dir.display()))?;
    #[cfg(windows)]
    if !maya_dir.join(crate::hook_install::HOOK_EXE).is_file() {
        return Err("Maya's Claude hook is not installed; install it first, as the status line runs it".into());
    }
    let inner = maya_dir.join(INNER_FILE);
    match previous.as_deref() {
        Some(c) if is_wrapper(c) => {
            if !inner.exists() {
                std::fs::write(&inner, "").map_err(|e| format!("cannot write {INNER_FILE}: {e}"))?;
            }
        }
        c => std::fs::write(&inner, c.unwrap_or("")).map_err(|e| format!("cannot write {INNER_FILE}: {e}"))?,
    }
    #[cfg(unix)]
    write_script(&maya_dir)?;
    let mut entry = root.get("statusLine").and_then(Value::as_object).cloned().unwrap_or_default();
    entry.insert("type".into(), "command".into());
    entry.insert("command".into(), wrapper_command().into());
    root.insert("statusLine".into(), Value::Object(entry));
    write_settings_with_backup(claude_dir, file, &settings)?;
    status_in(claude_dir, managed)
}

/// Renamed into place, executable first: Claude Code may run the status
/// line at any moment and must never find half a script.
#[cfg(unix)]
fn write_script(maya_dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let fresh = maya_dir.join(format!("{SCRIPT_FILE}.new"));
    std::fs::write(&fresh, WRAPPER_SCRIPT).map_err(|e| format!("cannot write {SCRIPT_FILE}: {e}"))?;
    std::fs::set_permissions(&fresh, std::fs::Permissions::from_mode(0o755)).map_err(|e| format!("cannot make {SCRIPT_FILE} executable: {e}"))?;
    std::fs::rename(&fresh, maya_dir.join(SCRIPT_FILE)).map_err(|e| format!("cannot replace {SCRIPT_FILE}: {e}"))
}

/// Restores the inner command (or removes `statusLine` when the inner file
/// is empty), deletes the inner file, the script and every
/// `maya/usage-*.json`.
pub fn remove_from(claude_dir: &Path) -> Result<Status, String> {
    remove_in(claude_dir, &managed_settings_path())
}

pub(crate) fn remove_in(claude_dir: &Path, managed: &Path) -> Result<Status, String> {
    let maya_dir = claude_dir.join("maya");
    let inner = std::fs::read_to_string(maya_dir.join(INNER_FILE)).unwrap_or_default();
    for file in [LOCAL_FILE, USER_FILE] {
        let mut settings = read_settings(claude_dir, file)?;
        if !command_of(&settings).is_some_and(is_wrapper) {
            continue;
        }
        let Some(root) = settings.as_object_mut() else { continue };
        if inner.trim().is_empty() {
            root.shift_remove("statusLine");
        } else if let Some(entry) = root.get_mut("statusLine").and_then(Value::as_object_mut) {
            entry.insert("command".into(), inner.clone().into());
        }
        write_settings_with_backup(claude_dir, file, &settings)?;
    }
    let ours = |name: &str| {
        name == INNER_FILE
            || name == SCRIPT_FILE
            || name.starts_with("statusline-in.")
            || (name.starts_with("usage-") && name.contains(".json"))
    };
    if let Ok(entries) = std::fs::read_dir(&maya_dir) {
        for entry in entries.flatten() {
            if entry.file_name().to_str().is_some_and(ours) {
                std::fs::remove_file(entry.path()).map_err(|e| format!("cannot remove {}: {e}", entry.path().display()))?;
            }
        }
    }
    status_in(claude_dir, managed)
}

/// What the wrapper writes, and what Maya reads back.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recorded {
    pub at: u64,
    pub session_id: String,
    pub model: Option<String>,
    pub version: Option<String>,
    pub context_used_percent: Option<u8>,
    pub context_window_size: Option<u64>,
    pub rate_limits: Option<Value>,
    pub total_cost_usd: Option<f64>,
    /// What the user's own status line printed, without trailing newlines.
    pub line: String,
}

/// A Claude Code session id: 36 hex digits and dashes, so it is safe in a
/// file name.
fn is_session_id(id: &str) -> bool {
    id.len() == 36 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

/// A percentage rounded and held to 0..=100.
fn percent(used: f64) -> u8 {
    used.round().clamp(0.0, 100.0) as u8
}

/// Builds a `Recorded` from a payload and the captured line (the Windows
/// binary's path and the tests' reference for the script). `None` when the
/// payload has no usable `session_id`.
pub fn recorded_from(payload: &Value, line: &str, now_ms: u64) -> Option<Recorded> {
    let session_id = payload.get("session_id")?.as_str().filter(|id| is_session_id(id))?;
    let text = |p: &str| payload.pointer(p).and_then(Value::as_str).map(str::to_string);
    let number = |p: &str| payload.pointer(p).and_then(Value::as_f64);
    Some(Recorded {
        at: now_ms,
        session_id: session_id.to_string(),
        model: text("/model/id"),
        version: text("/version"),
        context_used_percent: number("/context_window/used_percentage").map(percent),
        context_window_size: number("/context_window/context_window_size").filter(|n| *n >= 0.0).map(|n| n.floor() as u64),
        rate_limits: payload.get("rate_limits").filter(|v| !v.is_null()).cloned(),
        total_cost_usd: number("/cost/total_cost_usd"),
        line: line.to_string(),
    })
}

fn recorded_path(maya_dir: &Path, session_id: &str) -> Option<PathBuf> {
    is_session_id(session_id).then(|| maya_dir.join(format!("usage-{session_id}.json")))
}

/// Writes `maya/usage-<id>.json` atomically.
pub fn write_recorded(maya_dir: &Path, r: &Recorded) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    let path = recorded_path(maya_dir, &r.session_id).ok_or_else(|| Error::new(ErrorKind::InvalidInput, "not a session id"))?;
    std::fs::create_dir_all(maya_dir)?;
    let text = serde_json::to_string(r).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;
    let tmp = maya_dir.join(format!("usage-{}.json.{}", r.session_id, std::process::id()));
    std::fs::write(&tmp, text + "\n")?;
    std::fs::rename(&tmp, &path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

pub fn read_recorded(maya_dir: &Path, session_id: &str) -> Option<Recorded> {
    let text = std::fs::read_to_string(recorded_path(maya_dir, session_id)?).ok()?;
    serde_json::from_str(&text).ok()
}

/// The newest recording in the folder by `at`.
pub fn newest_recorded(maya_dir: &Path) -> Option<Recorded> {
    std::fs::read_dir(maya_dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let id = name.to_str()?.strip_prefix("usage-")?.strip_suffix(".json")?.to_string();
            read_recorded(maya_dir, &id)
        })
        .max_by_key(|r| r.at)
}

/// The label of a `rate_limits` key: `five_hour` → "5h", `seven_day` →
/// "7d", any other key with its underscores as spaces.
fn label_of(key: &str) -> String {
    match key {
        "five_hour" => window_label(5 * 60),
        "seven_day" => window_label(7 * 24 * 60),
        other => other.replace('_', " "),
    }
}

/// Seconds since the epoch: a number past 2100-01-01 in seconds is taken
/// as millis, a string as an ISO-8601 UTC time.
fn resets_secs(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => {
            let n = n.as_f64().filter(|n| *n >= 0.0)? as u64;
            Some(if n > 4_102_444_800 { n / 1000 } else { n })
        }
        Value::String(s) => crate::codex::ms_of(s).map(|ms| ms / 1000),
        _ => None,
    }
}

/// `rate_limits` → windows: `five_hour` → "5h", `seven_day` → "7d", any
/// other key → the key with `_` as space; `used_percentage` rounded and
/// capped at 100; `resets_at` in seconds (a number > 4_102_444_800 is
/// millis and is divided by 1000; an ISO string is parsed with
/// `codex::ms_of`). `None` when `rate_limits` is absent; `Some` with no
/// windows when it is an empty object. A window without a numeric
/// `used_percentage` is left out.
pub fn limits_of(r: &Recorded, account: Option<String>) -> Option<AccountLimits> {
    let map = r.rate_limits.as_ref()?.as_object()?;
    let windows = map
        .iter()
        .filter_map(|(key, w)| {
            let used = w.get("used_percentage")?.as_f64()?;
            Some(LimitWindow { label: label_of(key), percent: percent(used), resets_at: w.get("resets_at").and_then(resets_secs) })
        })
        .collect();
    Some(AccountLimits { account, windows, read_at: r.at })
}

/// A file's modification time and length: what says it changed.
type Stamp = (Option<std::time::SystemTime>, u64);

fn stamp_of(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok(), meta.len()))
}

/// One usage file as `RecordedCache` keeps it.
struct Entry {
    stamp: Stamp,
    recorded: Option<Recorded>,
    /// The `at` of the recording that last changed this session's numbers:
    /// a status line re-rendered on its refresh interval writes a fresh `at`
    /// with what it learned at its last message, which is not a new reading.
    learned_at: u64,
}

/// The numbers a recording learned from Claude Code's last message.
fn same_numbers(a: &Recorded, b: &Recorded) -> bool {
    a.rate_limits == b.rate_limits
        && a.total_cost_usd == b.total_cost_usd
        && a.context_used_percent == b.context_used_percent
        && a.context_window_size == b.context_window_size
}

/// Every `usage-<id>.json` in the folder, each parsed again only when its
/// mtime or size changes; a file that does not parse is kept as `None`
/// until it changes.
#[derive(Default)]
pub struct RecordedCache {
    files: std::collections::HashMap<String, Entry>,
    /// Number of files parsed (for tests and diagnostics).
    pub reads: usize,
}

impl RecordedCache {
    /// Lists `maya_dir` once and stats each usage file, parsing only the
    /// ones whose stamp changed and forgetting the ones that are gone.
    pub fn scan(&mut self, maya_dir: &Path) {
        let mut seen = std::collections::HashMap::with_capacity(self.files.len());
        for entry in std::fs::read_dir(maya_dir).into_iter().flatten().flatten() {
            let name = entry.file_name();
            let Some(id) = name.to_str().and_then(|n| n.strip_prefix("usage-")?.strip_suffix(".json")).filter(|id| is_session_id(id)) else { continue };
            let Ok(meta) = entry.metadata() else { continue };
            let stamp = (meta.modified().ok(), meta.len());
            let known = self.files.remove(id);
            let kept = match known {
                Some(e) if e.stamp == stamp => e,
                known => {
                    self.reads += 1;
                    let recorded = read_recorded(maya_dir, id);
                    let learned_at = match (&recorded, known) {
                        (Some(new), Some(Entry { recorded: Some(old), learned_at, .. })) if same_numbers(new, &old) => learned_at,
                        (Some(new), _) => new.at,
                        (None, _) => 0,
                    };
                    Entry { stamp, recorded, learned_at }
                }
            };
            seen.insert(id.to_string(), kept);
        }
        self.files = seen;
    }

    /// The recording of `session_id` as of the last `scan`.
    pub fn get(&self, session_id: &str) -> Option<Recorded> {
        self.files.get(session_id)?.recorded.clone()
    }

    /// When `session_id`'s numbers last changed, as of the last `scan`.
    pub fn learned_at(&self, session_id: &str) -> Option<u64> {
        let e = self.files.get(session_id)?;
        e.recorded.as_ref().map(|_| e.learned_at)
    }

    /// The account's limits: those of the recording that learned its
    /// `rate_limits` last, whichever session wrote it, read at that time.
    /// `Some` with no windows when recordings exist and none carries
    /// `rate_limits` (an API key or a gateway); `None` without recordings.
    pub fn limits(&self, account: Option<String>) -> Option<AccountLimits> {
        let recorded = || self.files.values().filter_map(|e| Some((e.recorded.as_ref()?, e.learned_at)));
        let newest = recorded().filter_map(|(r, learned_at)| Some((limits_of(r, None)?, learned_at))).max_by_key(|(_, learned_at)| *learned_at);
        match newest {
            Some((limits, read_at)) => Some(AccountLimits { account, read_at, ..limits }),
            None => {
                let read_at = recorded().map(|(_, learned_at)| learned_at).max()?;
                Some(AccountLimits { account, windows: vec![], read_at })
            }
        }
    }
}

/// The one field of `.claude.json` Maya reads; serde skips the rest.
#[derive(Deserialize)]
struct ClaudeJson {
    #[serde(rename = "oauthAccount")]
    oauth_account: Option<OauthAccount>,
}

#[derive(Deserialize)]
struct OauthAccount {
    #[serde(rename = "emailAddress")]
    email_address: Option<String>,
}

/// `oauthAccount.emailAddress` from `<claude_dir's parent>/.claude.json`, re-read on change.
pub fn account_email(claude_dir: &Path) -> Option<String> {
    static SEEN: std::sync::Mutex<Option<(PathBuf, Stamp, Option<String>)>> = std::sync::Mutex::new(None);
    let path = claude_dir.parent()?.join(".claude.json");
    let stamp = stamp_of(&path)?;
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((p, s, email)) = seen.as_ref() {
        if *p == path && *s == stamp {
            return email.clone();
        }
    }
    let email = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<ClaudeJson>(&text).ok())
        .and_then(|j| j.oauth_account?.email_address)
        .filter(|e| !e.is_empty());
    *seen = Some((path, stamp, email.clone()));
    email
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    const SID: &str = "4f1c2a7e-0b1d-4c3e-9a8f-1234567890ab";

    fn fixture() -> Value {
        serde_json::from_str(include_str!("../fixtures/statusline/payload.json")).unwrap()
    }

    /// A managed settings path that does not exist, so the machine's own
    /// managed file never leaks into a test.
    fn no_managed(dir: &Path) -> PathBuf {
        dir.join("no-managed-settings.json")
    }

    /// A temp `~/.claude`; on Windows with the hook binary the wrapper
    /// command runs, which install requires.
    fn claude_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        if cfg!(windows) {
            std::fs::create_dir_all(dir.path().join("maya")).unwrap();
            std::fs::write(dir.path().join("maya").join("maya-hook.exe"), "").unwrap();
        }
        dir
    }

    fn read_json(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn wrapper_marker() -> &'static str {
        if cfg!(windows) { WRAPPER_EXE_MARKER } else { WRAPPER_MARKER }
    }

    fn windows(l: &AccountLimits) -> Vec<(String, u8, Option<u64>)> {
        l.windows.iter().map(|w| (w.label.clone(), w.percent, w.resets_at)).collect()
    }

    fn with_limits(rate_limits: Value) -> Recorded {
        let mut p = fixture();
        p["rate_limits"] = rate_limits;
        recorded_from(&p, "", 7).unwrap()
    }

    #[test]
    fn recorded_from_keeps_the_fields_maya_reads() {
        let line = "\u{1b}[1mOpus\u{1b}[0m";
        let r = recorded_from(&fixture(), line, 1_793_000_000_000).unwrap();
        assert_eq!(r.at, 1_793_000_000_000);
        assert_eq!(r.session_id, SID);
        assert_eq!(r.model.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(r.version.as_deref(), Some("2.1.99"));
        assert_eq!(r.context_used_percent, Some(12));
        assert_eq!(r.context_window_size, Some(1_000_000));
        assert_eq!(r.total_cost_usd, Some(4.1234));
        assert_eq!(r.rate_limits.as_ref().unwrap()["five_hour"]["used_percentage"], 78.4);
        assert_eq!(r.line, line);

        let mut p = fixture();
        p.as_object_mut().unwrap().remove("session_id");
        assert_eq!(recorded_from(&p, line, 1), None);
        p["session_id"] = json!("../x");
        assert_eq!(recorded_from(&p, line, 1), None);
    }

    #[test]
    fn limits_of_maps_the_two_windows_and_accepts_millis_or_iso_resets() {
        let r = recorded_from(&fixture(), "", 9).unwrap();
        let l = limits_of(&r, Some("me@example.com".into())).unwrap();
        assert_eq!(l.account.as_deref(), Some("me@example.com"));
        assert_eq!(l.read_at, 9);
        assert_eq!(
            windows(&l),
            [("5h".to_string(), 78, Some(1_793_275_200)), ("7d".to_string(), 41, Some(1_793_700_000))]
        );

        let millis = with_limits(json!({"five_hour": {"used_percentage": 1, "resets_at": 1_793_275_200_000u64}}));
        assert_eq!(limits_of(&millis, None).unwrap().windows[0].resets_at, Some(1_793_275_200));
        let iso = with_limits(json!({"five_hour": {"used_percentage": 1, "resets_at": "2026-10-29T12:00:00Z"}}));
        assert_eq!(limits_of(&iso, None).unwrap().windows[0].resets_at, Some(1_793_275_200));

        let mut none = fixture();
        none.as_object_mut().unwrap().remove("rate_limits");
        assert_eq!(limits_of(&recorded_from(&none, "", 1).unwrap(), None), None);
        let empty = limits_of(&with_limits(json!({})), None).unwrap();
        assert!(empty.windows.is_empty());

        let over = with_limits(json!({"opus_weekly": {"used_percentage": 140.0}}));
        let over = limits_of(&over, None).unwrap();
        assert_eq!(windows(&over), [("opus weekly".to_string(), 100, None)]);
    }

    #[test]
    fn write_and_read_recorded_round_trip_and_newest_wins() {
        let dir = tempfile::tempdir().unwrap();
        let maya = dir.path().join("maya");
        let mut older = recorded_from(&fixture(), "old", 1).unwrap();
        older.session_id = "00000000-0000-0000-0000-000000000001".into();
        let newer = recorded_from(&fixture(), "new", 2).unwrap();
        write_recorded(&maya, &newer).unwrap();
        write_recorded(&maya, &older).unwrap();
        assert_eq!(read_recorded(&maya, &older.session_id), Some(older.clone()));
        assert_eq!(read_recorded(&maya, SID), Some(newer.clone()));
        assert_eq!(read_recorded(&maya, "../settings"), None);
        assert_eq!(newest_recorded(&maya), Some(newer));
        assert_eq!(newest_recorded(&dir.path().join("missing")), None);
    }

    #[test]
    fn the_recorded_cache_parses_only_changed_files_and_keeps_the_newest_reading() {
        let dir = tempfile::tempdir().unwrap();
        let maya = dir.path().join("maya");
        let other = "00000000-0000-0000-0000-000000000001";
        let broken = "00000000-0000-0000-0000-000000000002";
        let mut cache = RecordedCache::default();
        let read_at = |c: &RecordedCache| c.limits(None).map(|l| l.read_at);
        cache.scan(&maya);
        assert_eq!((cache.get(SID), read_at(&cache), cache.reads), (None, None, 0));

        let first = recorded_from(&fixture(), "one", 1).unwrap();
        let mut older = recorded_from(&fixture(), "older", 3).unwrap();
        older.session_id = other.into();
        write_recorded(&maya, &first).unwrap();
        write_recorded(&maya, &older).unwrap();
        std::fs::write(maya.join("events.jsonl"), "{}\n").unwrap();
        std::fs::write(maya.join(format!("usage-{other}.json.123")), "half").unwrap();
        cache.scan(&maya);
        assert_eq!((cache.get(SID), read_at(&cache), cache.reads), (Some(first), Some(3), 2));
        cache.scan(&maya);
        assert_eq!(cache.reads, 2, "unchanged files are not read again");

        let mut second = recorded_from(&fixture(), "second", 5).unwrap();
        second.total_cost_usd = Some(9.0);
        write_recorded(&maya, &second).unwrap();
        std::fs::write(maya.join(format!("usage-{broken}.json")), "not json").unwrap();
        cache.scan(&maya);
        assert_eq!((cache.get(SID), read_at(&cache), cache.reads), (Some(second), Some(5), 4));
        assert_eq!(cache.get(broken), None);
        cache.scan(&maya);
        assert_eq!(cache.reads, 4, "a file that does not parse is not parsed again until it changes");

        std::fs::remove_file(maya.join(format!("usage-{SID}.json"))).unwrap();
        cache.scan(&maya);
        assert_eq!((cache.get(SID), read_at(&cache), cache.reads), (None, Some(3), 4));
    }

    fn five_hour(sid: &str, line: &str, at: u64, used: u64) -> Recorded {
        let mut r = with_limits(json!({"five_hour": {"used_percentage": used}}));
        r.session_id = sid.into();
        r.line = line.into();
        r.at = at;
        r
    }

    #[test]
    fn an_idle_re_render_does_not_make_its_old_limits_the_newest() {
        let dir = tempfile::tempdir().unwrap();
        let maya = dir.path().join("maya");
        let idle = "00000000-0000-0000-0000-000000000001";
        let mut cache = RecordedCache::default();
        let shown = |c: &RecordedCache| c.limits(None).map(|l| (l.windows[0].percent, l.read_at));
        write_recorded(&maya, &five_hour(idle, "i", 1_000, 10)).unwrap();
        cache.scan(&maya);
        write_recorded(&maya, &five_hour(SID, "t", 2_000, 20)).unwrap();
        cache.scan(&maya);
        assert_eq!(shown(&cache), Some((20, 2_000)));

        // The idle session re-renders on its refresh interval: a later `at`, the same numbers.
        write_recorded(&maya, &five_hour(idle, "idle", 3_000, 10)).unwrap();
        cache.scan(&maya);
        assert_eq!(cache.learned_at(idle), Some(1_000), "unchanged numbers keep when they were learned");
        assert_eq!(shown(&cache), Some((20, 2_000)), "the talking session's reading still wins");

        // Its next message brings new numbers.
        write_recorded(&maya, &five_hour(idle, "idle again", 4_000, 30)).unwrap();
        cache.scan(&maya);
        assert_eq!(cache.learned_at(idle), Some(4_000));
        assert_eq!(shown(&cache), Some((30, 4_000)));
    }

    #[test]
    fn recordings_without_rate_limits_give_no_limits_and_never_hide_a_reading() {
        let dir = tempfile::tempdir().unwrap();
        let maya = dir.path().join("maya");
        let mut cache = RecordedCache::default();
        let mut api_key = recorded_from(&fixture(), "key", 5_000).unwrap();
        api_key.rate_limits = None;
        write_recorded(&maya, &api_key).unwrap();
        cache.scan(&maya);
        let limits = cache.limits(Some("me@example.com".into())).unwrap();
        assert_eq!(limits, AccountLimits { account: Some("me@example.com".into()), windows: vec![], read_at: 5_000 });

        // A newer session before its first reply has no rate_limits yet; an older reading still wins.
        let other = "00000000-0000-0000-0000-000000000001";
        write_recorded(&maya, &five_hour(other, "o", 1_000, 12)).unwrap();
        cache.scan(&maya);
        let limits = cache.limits(None).unwrap();
        assert_eq!((limits.windows[0].percent, limits.read_at), (12, 1_000));
    }

    #[test]
    fn account_email_reads_the_signed_in_address_beside_the_claude_folder() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join(".claude");
        let file = dir.path().join(".claude.json");
        assert_eq!(account_email(&claude), None);
        std::fs::write(&file, r#"{"oauthAccount":{"emailAddress":"me@example.com","accountUuid":"x"},"projects":{}}"#).unwrap();
        assert_eq!(account_email(&claude).as_deref(), Some("me@example.com"));
        std::fs::write(&file, r#"{"oauthAccount":{"emailAddress":"you@example.com"}}"#).unwrap();
        assert_eq!(account_email(&claude).as_deref(), Some("you@example.com"));
        std::fs::write(&file, r#"{"oauthAccount":{}}"#).unwrap();
        assert_eq!(account_email(&claude), None);
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(account_email(&claude), None);
    }

    #[test]
    fn install_wraps_the_existing_command_and_keeps_its_other_keys() {
        let dir = claude_dir();
        let managed = no_managed(dir.path());
        let settings = dir.path().join("settings.json");
        std::fs::write(
            &settings,
            r#"{"statusLine":{"type":"command","command":"sh ~/sl.sh","refreshInterval":60,"padding":1},"model":"opus"}"#,
        )
        .unwrap();
        let st = install_in(dir.path(), &managed).unwrap();
        assert_eq!(st, Status { installed: true, blocked_by: None });
        let v = read_json(&settings);
        assert!(v["statusLine"]["command"].as_str().unwrap().contains(wrapper_marker()), "{v}");
        assert_eq!(v["statusLine"]["type"], "command");
        assert_eq!(v["statusLine"]["refreshInterval"], 60);
        assert_eq!(v["statusLine"]["padding"], 1);
        assert_eq!(v["model"], "opus");
        let inner = dir.path().join("maya").join(INNER_FILE);
        assert_eq!(std::fs::read_to_string(&inner).unwrap(), "sh ~/sl.sh");
        assert!(status_in(dir.path(), &managed).unwrap().installed);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let script = dir.path().join("maya").join(SCRIPT_FILE);
            assert_eq!(std::fs::read_to_string(&script).unwrap(), WRAPPER_SCRIPT);
            assert_eq!(std::fs::metadata(&script).unwrap().permissions().mode() & 0o777, 0o755);
        }

        install_in(dir.path(), &managed).unwrap();
        assert_eq!(std::fs::read_to_string(&inner).unwrap(), "sh ~/sl.sh");
        let backups = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("settings.json.maya-backup-"))
            .count();
        assert!(backups >= 1);

        let st = remove_in(dir.path(), &managed).unwrap();
        assert_eq!(st, Status { installed: false, blocked_by: None });
        let v = read_json(&settings);
        assert_eq!(
            v,
            json!({"statusLine":{"type":"command","command":"sh ~/sl.sh","refreshInterval":60,"padding":1},"model":"opus"})
        );
    }

    #[test]
    fn install_without_a_status_line_writes_an_empty_inner_and_remove_removes_the_entry() {
        let dir = claude_dir();
        let managed = no_managed(dir.path());
        let settings = dir.path().join("settings.json");
        std::fs::write(&settings, r#"{"model":"opus"}"#).unwrap();
        install_in(dir.path(), &managed).unwrap();
        let maya = dir.path().join("maya");
        assert_eq!(std::fs::read_to_string(maya.join(INNER_FILE)).unwrap(), "");
        assert_eq!(read_json(&settings)["statusLine"]["type"], "command");
        std::fs::write(maya.join("usage-x.json"), "{}").unwrap();
        std::fs::write(maya.join("events.jsonl"), "").unwrap();

        remove_in(dir.path(), &managed).unwrap();
        let v = read_json(&settings);
        assert_eq!(v, json!({"model": "opus"}));
        assert!(!maya.join(INNER_FILE).exists());
        assert!(!maya.join(SCRIPT_FILE).exists());
        assert!(!maya.join("usage-x.json").exists());
        assert!(maya.join("events.jsonl").exists(), "only the status line's files go");
    }

    #[test]
    fn install_prefers_settings_local_when_it_holds_the_status_line() {
        let dir = claude_dir();
        let managed = no_managed(dir.path());
        let local = dir.path().join("settings.local.json");
        let user = dir.path().join("settings.json");
        std::fs::write(&local, r#"{"statusLine":{"type":"command","command":"echo local"}}"#).unwrap();
        std::fs::write(&user, r#"{"model":"opus"}"#).unwrap();
        assert!(install_in(dir.path(), &managed).unwrap().installed);
        assert!(read_json(&local)["statusLine"]["command"].as_str().unwrap().contains(wrapper_marker()));
        assert_eq!(read_json(&user), json!({"model": "opus"}));
        assert_eq!(std::fs::read_to_string(dir.path().join("maya").join(INNER_FILE)).unwrap(), "echo local");

        remove_in(dir.path(), &managed).unwrap();
        assert_eq!(read_json(&local), json!({"statusLine":{"type":"command","command":"echo local"}}));
        assert_eq!(read_json(&user), json!({"model": "opus"}));
    }

    #[test]
    fn status_is_blocked_by_a_managed_status_line() {
        let dir = tempfile::tempdir().unwrap();
        let managed = dir.path().join("managed-settings.json");
        std::fs::write(&managed, r#"{"statusLine":{"type":"command","command":"corp-status"}}"#).unwrap();
        let shown = managed.display().to_string();
        assert_eq!(status_in(dir.path(), &managed).unwrap(), Status { installed: false, blocked_by: Some(shown.clone()) });
        let err = install_in(dir.path(), &managed).unwrap_err();
        assert!(err.contains(&shown), "{err}");
        assert!(!dir.path().join("settings.json").exists());

        std::fs::write(&managed, r#"{"permissions":{}}"#).unwrap();
        assert_eq!(status_in(dir.path(), &managed).unwrap(), Status { installed: false, blocked_by: None });
    }

    #[cfg(windows)]
    #[test]
    fn install_on_windows_needs_the_hook_binary_it_runs() {
        let dir = tempfile::tempdir().unwrap();
        let err = install_in(dir.path(), &no_managed(dir.path())).unwrap_err();
        assert!(err.contains("hook"), "{err}");
        assert!(!dir.path().join("settings.json").exists());
    }

    #[test]
    fn managed_settings_path_is_this_platforms() {
        let p = managed_settings_path();
        assert!(p.ends_with("managed-settings.json"), "{}", p.display());
    }

    #[cfg(unix)]
    fn has_jq() -> bool {
        let found = std::process::Command::new("sh")
            .args(["-c", "command -v jq"])
            .stdout(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !found {
            eprintln!("skipping: jq is not on PATH, and the status line script needs it");
        }
        found
    }

    /// Writes the script and `inner` under a temp `HOME`, runs it with
    /// `payload` on stdin, and returns (home, stdout, exit success).
    #[cfg(unix)]
    fn run_script(inner: &str, payload: &Value) -> (tempfile::TempDir, String, bool) {
        use std::io::Write;
        let home = tempfile::tempdir().unwrap();
        let maya = home.path().join(".claude").join("maya");
        std::fs::create_dir_all(&maya).unwrap();
        let script = maya.join(SCRIPT_FILE);
        std::fs::write(&script, WRAPPER_SCRIPT).unwrap();
        std::fs::write(maya.join(INNER_FILE), inner).unwrap();
        let mut child = std::process::Command::new("sh")
            .arg(&script)
            .env("HOME", home.path())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(payload.to_string().as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        (home, String::from_utf8(out.stdout).unwrap(), out.status.success())
    }

    #[cfg(unix)]
    fn usage_files(maya: &Path) -> Vec<String> {
        std::fs::read_dir(maya)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("usage-"))
            .collect()
    }

    #[cfg(unix)]
    #[test]
    fn the_script_records_the_payload_and_prints_the_inner_output() {
        if !has_jq() {
            return;
        }
        let (home, stdout, ok) = run_script(r#"printf 'LINE %s' "$(jq -r .model.id)""#, &fixture());
        assert!(ok);
        assert_eq!(stdout, "LINE claude-opus-5-5");
        let maya = home.path().join(".claude").join("maya");
        let r = read_recorded(&maya, SID).unwrap();
        assert_eq!(r.line, "LINE claude-opus-5-5");
        assert_eq!(r.total_cost_usd, Some(4.1234));
        assert!(r.at > 1_700_000_000_000);
        assert_eq!(r, recorded_from(&fixture(), "LINE claude-opus-5-5", r.at).unwrap(), "the script and recorded_from agree");
        assert_eq!(usage_files(&maya), [format!("usage-{SID}.json")], "no temp files left behind");
    }

    #[cfg(unix)]
    #[test]
    fn the_script_prints_the_inner_output_even_for_a_bad_session_id() {
        if !has_jq() {
            return;
        }
        let mut p = fixture();
        p["session_id"] = json!("nope");
        let (home, stdout, ok) = run_script(r#"printf 'LINE %s' "$(jq -r .model.id)""#, &p);
        assert!(ok);
        assert_eq!(stdout, "LINE claude-opus-5-5");
        assert!(usage_files(&home.path().join(".claude").join("maya")).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn the_script_prints_nothing_and_exits_zero_without_an_inner_command() {
        if !has_jq() {
            return;
        }
        let (home, stdout, ok) = run_script("", &fixture());
        assert!(ok);
        assert_eq!(stdout, "");
        let r = read_recorded(&home.path().join(".claude").join("maya"), SID).unwrap();
        assert_eq!(r.line, "");
        assert_eq!(r.context_used_percent, Some(12));
    }

    #[cfg(unix)]
    #[test]
    fn the_script_prints_multi_line_output_unchanged_and_records_it_without_trailing_newlines() {
        if !has_jq() {
            return;
        }
        let (home, stdout, ok) = run_script(r#"printf 'one\ntwo\n\n'; exit 3"#, &fixture());
        assert!(ok, "exits 0 whatever the inner command does");
        assert_eq!(stdout, "one\ntwo\n\n");
        let r = read_recorded(&home.path().join(".claude").join("maya"), SID).unwrap();
        assert_eq!(r.line, "one\ntwo");
    }
}
