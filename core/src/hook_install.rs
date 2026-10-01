use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
pub const HOOK_MARKER: &str = ".claude/maya/hook.sh";
/// The app on Linux bundles Maya's own `maya-hook`, as on Windows, so it
/// needs no jq.
#[cfg(target_os = "linux")]
pub const HOOK_MARKER: &str = ".claude/maya/maya-hook";
/// Claude Code runs hook commands through Git Bash on Windows, which has
/// no jq, so the hook there is Maya's own `maya-hook.exe`.
#[cfg(windows)]
pub const HOOK_MARKER: &str = ".claude/maya/maya-hook.exe";
/// Where `install_script_to` puts the jq hook script: the hook on macOS,
/// and the headless CLI's on Linux, whose single binary has no `maya-hook`
/// beside it.
#[cfg(unix)]
pub const SCRIPT_MARKER: &str = ".claude/maya/hook.sh";
/// The hook binary's file name on Windows and Linux, next to Maya's own
/// executable and, once installed, in `~/.claude/maya`.
#[cfg(windows)]
pub const HOOK_EXE: &str = "maya-hook.exe";
#[cfg(not(windows))]
pub const HOOK_EXE: &str = "maya-hook";
/// The hooks counted as installed and replaced on install: on Linux the
/// app's binary and the CLI's script are both Maya's current hook.
#[cfg(target_os = "linux")]
const CURRENT_MARKERS: &[&str] = &[HOOK_MARKER, SCRIPT_MARKER];
#[cfg(not(target_os = "linux"))]
const CURRENT_MARKERS: &[&str] = &[HOOK_MARKER];
/// Longest string kept from a `tool_input` field, in characters.
const FIELD_MAX: usize = 400;
/// Markers of earlier releases; removed on install, never counted as installed.
pub const LEGACY_MARKERS: &[&str] = &[".claude/eye/hook.sh"];

pub const HOOK_SCRIPT: &str = r#"#!/bin/sh
# Installed by Maya (Manage All Your Agents). Appends each hook payload to
# ~/.claude/maya/events.jsonl with a received_at timestamp. Never blocks,
# never prints, always exits 0.
dir="$HOME/.claude/maya"
mkdir -p "$dir" 2>/dev/null
t=$(date +%s000)
# Keep only the fields Maya reads; tool_response and the rest are dropped so
# the log stays small and every append fits in one atomic write.
jq -c --arg t "$t" '{
  session_id, hook_event_name, tool_name, notification_type, transcript_path, agent_id,
  tool_input: ((.tool_input // {}) | {command, file_path, path, questions}
    | with_entries(select(.value != null))
    | with_entries(if (.value | type) == "string" then .value |= .[0:400] else . end)),
  received_at: ($t|tonumber)
} | with_entries(select(.value != null))' >> "$dir/events.jsonl" 2>/dev/null
exit 0
"#;

/// What `HOOK_SCRIPT` appends for one hook payload, without the newline:
/// the fields Maya reads, `tool_input` strings cut to 400 characters, and
/// `received_at`. `None` when the payload is not a JSON object.
pub fn record(payload: &str, received_at: u64) -> Option<String> {
    let v: Value = serde_json::from_str(payload).ok()?;
    let v = v.as_object()?;
    let mut out = Map::new();
    for key in ["session_id", "hook_event_name", "tool_name", "notification_type", "transcript_path", "agent_id"] {
        if let Some(x) = v.get(key).filter(|x| !x.is_null()) {
            out.insert(key.into(), x.clone());
        }
    }
    let mut input = Map::new();
    if let Some(ti) = v.get("tool_input").and_then(Value::as_object) {
        for key in ["command", "file_path", "path", "questions"] {
            match ti.get(key) {
                None | Some(Value::Null) => {}
                Some(Value::String(t)) => {
                    input.insert(key.into(), Value::String(t.chars().take(FIELD_MAX).collect()));
                }
                Some(x) => {
                    input.insert(key.into(), x.clone());
                }
            }
        }
    }
    out.insert("tool_input".into(), Value::Object(input));
    out.insert("received_at".into(), json!(received_at));
    serde_json::to_string(&Value::Object(out)).ok()
}

/// Appends `record(payload)` to `maya_dir/events.jsonl` in one write, so
/// concurrent hooks never interleave. Errors are the caller's to ignore.
pub fn append_record(maya_dir: &Path, payload: &str, received_at: u64) -> std::io::Result<()> {
    append_record_named(maya_dir, "events.jsonl", payload, received_at)
}

pub fn append_record_named(maya_dir: &Path, filename: &str, payload: &str, received_at: u64) -> std::io::Result<()> {
    use std::io::Write;
    let Some(mut line) = record(payload, received_at) else { return Ok(()) };
    line.push('\n');
    std::fs::create_dir_all(maya_dir)?;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(maya_dir.join(filename))?;
    f.write_all(line.as_bytes())
}

/// The file holding the inbox token of the session whose inbox is
/// `socket`, named after the pipe (`cc-msg-<hex>`); None for a name that
/// is not plain letters, digits and dashes.
pub fn token_path(maya_dir: &Path, socket: &str) -> Option<std::path::PathBuf> {
    let name = socket.rsplit(['\\', '/']).next()?;
    let plain = !name.is_empty() && name.len() <= 128 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    plain.then(|| maya_dir.join("inbox").join(format!("{name}.token")))
}

/// Keeps the token Claude Code gives the hook (`CLAUDE_CODE_MESSAGING_TOKEN`,
/// for `CLAUDE_CODE_MESSAGING_SOCKET`) where Maya's replies find it: on
/// Windows every inbox connection must open with it. Written only when it
/// changed; removed when the session ends.
pub fn record_token(maya_dir: &Path, event: &str, socket: Option<&str>, token: Option<&str>) -> std::io::Result<()> {
    let Some(path) = socket.and_then(|s| token_path(maya_dir, s)) else { return Ok(()) };
    if event == "SessionEnd" {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    let Some(token) = token.map(str::trim).filter(|t| !t.is_empty()) else { return Ok(()) };
    if std::fs::read_to_string(&path).is_ok_and(|t| t == token) {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::config::write_private(&path, token.as_bytes())
}

/// The hook event name in a payload, if any.
pub fn event_name(payload: &str) -> Option<String> {
    serde_json::from_str::<Value>(payload).ok()?.get("hook_event_name")?.as_str().map(str::to_string)
}

/// Events hooked only so the hook binary sees a session's inbox token at
/// once: on Windows and Linux, SessionStart. They are not appended to
/// `events.jsonl`, whose readers know nothing of them, and the jq script
/// is never hooked to them.
#[cfg(any(windows, target_os = "linux"))]
pub const TOKEN_ONLY_EVENTS: &[&str] = &["SessionStart"];
#[cfg(target_os = "macos")]
pub const TOKEN_ONLY_EVENTS: &[&str] = &[];

/// (event name, matcher)
pub const HOOK_EVENTS: &[(&str, Option<&str>)] = &[
    ("PermissionRequest", None),
    ("PreToolUse", Some("AskUserQuestion|ExitPlanMode")),
    ("PostToolUse", None),
    ("PostToolUseFailure", None),
    ("PermissionDenied", None),
    ("Stop", None),
    ("UserPromptSubmit", None),
    ("SessionEnd", None),
    ("Notification", Some("permission_prompt")),
];

fn group_matches(group: &Value, markers: &[&str]) -> bool {
    group["hooks"]
        .as_array()
        .map(|hs| hs.iter().any(|h| h["command"].as_str().map_or(false, |c| markers.iter().any(|m| c.contains(m)))))
        .unwrap_or(false)
}

/// Ours, current or legacy: what `remove` strips.
fn group_is_ours(group: &Value) -> bool {
    group_matches(group, CURRENT_MARKERS) || group_matches(group, LEGACY_MARKERS)
}

pub fn is_installed(settings: &Value) -> bool {
    settings["hooks"]
        .as_object()
        .map(|hooks| hooks.values().any(|groups| groups.as_array().map_or(false, |g| g.iter().any(|g| group_matches(g, CURRENT_MARKERS)))))
        .unwrap_or(false)
}

/// Removes our entries, then adds one group per event. Idempotent.
pub fn install(settings: Value, command: &str) -> Value {
    install_events(settings, command, TOKEN_ONLY_EVENTS)
}

/// `install` with `token_only` hooked as well as `HOOK_EVENTS`.
fn install_events(settings: Value, command: &str, token_only: &[&str]) -> Value {
    let settings = remove(settings);
    let mut root = settings.as_object().cloned().unwrap_or_default();
    let mut hooks = root.get("hooks").and_then(|h| h.as_object()).cloned().unwrap_or_default();
    let token_only = token_only.iter().map(|e| (*e, None));
    for (event, matcher) in HOOK_EVENTS.iter().copied().chain(token_only) {
        let mut group = Map::new();
        if let Some(m) = matcher {
            group.insert("matcher".into(), json!(m));
        }
        group.insert("hooks".into(), json!([{ "type": "command", "command": command }]));
        let groups = hooks.entry(event.to_string()).or_insert_with(|| json!([]));
        if let Some(arr) = groups.as_array_mut() {
            arr.push(Value::Object(group));
        }
    }
    root.insert("hooks".into(), Value::Object(hooks));
    Value::Object(root)
}

/// Strips every group whose command contains the marker; drops empty events.
pub fn remove(settings: Value) -> Value {
    let mut root = settings.as_object().cloned().unwrap_or_default();
    if let Some(hooks) = root.get("hooks").and_then(|h| h.as_object()).cloned() {
        let mut cleaned = Map::new();
        for (event, groups) in hooks {
            let kept: Vec<Value> = groups.as_array().cloned().unwrap_or_default().into_iter().filter(|g| !group_is_ours(g)).collect();
            if !kept.is_empty() {
                cleaned.insert(event, Value::Array(kept));
            }
        }
        root.insert("hooks".into(), Value::Object(cleaned));
    }
    Value::Object(root)
}

fn read_settings(claude_dir: &Path) -> Result<Value, String> {
    let path = claude_dir.join("settings.json");
    if !path.exists() {
        return Ok(json!({}));
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read settings.json: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("settings.json is not valid JSON, refusing to modify it: {e}"))
}

fn write_settings_with_backup(claude_dir: &Path, settings: &Value) -> Result<(), String> {
    let path = claude_dir.join("settings.json");
    if path.exists() {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let backup = claude_dir.join(format!("settings.json.maya-backup-{stamp}"));
        std::fs::copy(&path, &backup).map_err(|e| format!("cannot back up settings.json: {e}"))?;
    }
    let text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(&path, text + "\n").map_err(|e| format!("cannot write settings.json: {e}"))
}

#[cfg(target_os = "macos")]
pub fn install_to(claude_dir: &Path) -> Result<(), String> {
    install_script_to(claude_dir)
}

/// Writes the jq hook script to `claude_dir/maya/hook.sh` and points every
/// hook at it: `install_to` on macOS, and the headless CLI everywhere.
#[cfg(unix)]
pub fn install_script_to(claude_dir: &Path) -> Result<(), String> {
    let settings = read_settings(claude_dir)?;
    let maya_dir = claude_dir.join("maya");
    std::fs::create_dir_all(&maya_dir).map_err(|e| format!("cannot create {}: {e}", maya_dir.display()))?;
    let script = maya_dir.join("hook.sh");
    std::fs::write(&script, HOOK_SCRIPT).map_err(|e| format!("cannot write hook.sh: {e}"))?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    let command = format!("\"$HOME/{SCRIPT_MARKER}\"");
    write_settings_with_backup(claude_dir, &install_events(settings, &command, &[]))
}

/// The hook binary in `dir`, the folder of Maya's own executable: Tauri
/// strips the target triple from a bundled sidecar's name, a dev build
/// keeps it (`maya-hook-<triple>`).
pub fn hook_source_in(dir: &Path) -> Option<PathBuf> {
    let plain = dir.join(HOOK_EXE);
    if plain.is_file() {
        return Some(plain);
    }
    let prefix = format!("{HOOK_EXE}-");
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.file_name().and_then(|n| n.to_str()).map_or(false, |n| n.starts_with(&prefix)) && p.is_file())
}

/// Copies `maya-hook` from beside this executable into `claude_dir/maya`
/// and points every hook at it.
#[cfg(target_os = "linux")]
pub fn install_to(claude_dir: &Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find Maya's own folder: {e}"))?;
    let dir = exe.parent().ok_or("cannot find Maya's own folder")?;
    let source = hook_source_in(dir).ok_or("Maya's hook binary (maya-hook) is not next to the app")?;
    install_with(claude_dir, &source)
}

/// Copies `maya-hook.exe` from beside this executable into
/// `claude_dir/maya` and points every hook at it.
#[cfg(windows)]
pub fn install_to(claude_dir: &Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find Maya's own folder: {e}"))?;
    let source = exe.parent().map(|d| d.join(HOOK_EXE)).ok_or("cannot find Maya's own folder")?;
    install_with(claude_dir, &source)
}

/// `install_to` with the hook binary taken from `source`. The copy is
/// skipped when the installed one is already identical, since Windows
/// refuses to overwrite a binary a running hook holds open; on Linux the
/// new one is renamed into place for the same reason and made executable.
#[cfg(any(windows, target_os = "linux"))]
pub fn install_with(claude_dir: &Path, source: &Path) -> Result<(), String> {
    let settings = read_settings(claude_dir)?;
    let maya_dir = claude_dir.join("maya");
    std::fs::create_dir_all(&maya_dir).map_err(|e| format!("cannot create {}: {e}", maya_dir.display()))?;
    let bytes = std::fs::read(source).map_err(|e| format!("cannot read {}: {e}", source.display()))?;
    let target = maya_dir.join(HOOK_EXE);
    if std::fs::read(&target).ok().as_deref() != Some(&bytes[..]) {
        #[cfg(windows)]
        std::fs::write(&target, &bytes).map_err(|e| format!("cannot write {HOOK_EXE} (a hook may be running; try again): {e}"))?;
        #[cfg(unix)]
        {
            let fresh = maya_dir.join(format!("{HOOK_EXE}.new"));
            std::fs::write(&fresh, &bytes).map_err(|e| format!("cannot write {HOOK_EXE}: {e}"))?;
            std::fs::rename(&fresh, &target).map_err(|e| format!("cannot replace {HOOK_EXE}: {e}"))?;
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).map_err(|e| format!("cannot make {HOOK_EXE} executable: {e}"))?;
    }
    let command = format!("\"$HOME/{HOOK_MARKER}\"");
    write_settings_with_backup(claude_dir, &install(settings, &command))
}

pub fn remove_from(claude_dir: &Path) -> Result<(), String> {
    let settings = read_settings(claude_dir)?;
    write_settings_with_backup(claude_dir, &remove(settings))
}

pub fn status(claude_dir: &Path) -> Result<bool, String> {
    Ok(is_installed(&read_settings(claude_dir)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// This platform's hook command, as `install_to` writes it.
    fn cmd() -> String {
        format!("\"$HOME/{HOOK_MARKER}\"")
    }

    #[test]
    fn install_adds_every_event_with_marker_command() {
        let out = install(json!({"model": "opus"}), &cmd());
        assert_eq!(out["model"], "opus");
        let hooks = out["hooks"].as_object().unwrap();
        for (event, matcher) in HOOK_EVENTS {
            let groups = hooks[*event].as_array().unwrap_or_else(|| panic!("missing {event}"));
            let g = groups.iter().find(|g| g["hooks"][0]["command"].as_str().unwrap().contains(HOOK_MARKER)).unwrap();
            assert_eq!(g["hooks"][0]["type"], "command");
            match matcher {
                Some(m) => assert_eq!(g["matcher"], *m),
                None => assert!(g.get("matcher").is_none()),
            }
        }
        assert!(is_installed(&out));
    }

    #[test]
    fn install_is_idempotent_and_preserves_other_hooks() {
        let existing = json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}]}]}});
        let once = install(existing, &cmd());
        let twice = install(once.clone(), &cmd());
        assert_eq!(once, twice);
        let stop = twice["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2);
        assert_eq!(stop[0]["hooks"][0]["command"], "say done");
    }

    #[test]
    fn remove_strips_only_marker_entries() {
        let existing = json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}]}]}});
        let installed = install(existing, &cmd());
        let removed = remove(installed);
        assert!(!is_installed(&removed));
        assert_eq!(removed["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert!(removed["hooks"].get("PermissionRequest").is_none(), "empty arrays are dropped");
    }

    #[test]
    fn install_and_remove_preserve_existing_key_order() {
        let existing: Value = serde_json::from_str(r#"{"zeta":1,"hooks":{"Stop":[{"hooks":[{"type":"command","command":"say done"}]}]},"alpha":2}"#).unwrap();
        let installed = install(existing, &format!("x/{HOOK_MARKER}"));
        let out = serde_json::to_string(&installed).unwrap();
        assert!(out.starts_with(r#"{"zeta":1,"hooks":"#), "{out}");
        assert!(out.ends_with(r#""alpha":2}"#), "{out}");
        let out = serde_json::to_string(&remove(installed)).unwrap();
        assert_eq!(out, r#"{"zeta":1,"hooks":{"Stop":[{"hooks":[{"type":"command","command":"say done"}]}]},"alpha":2}"#);
    }

    #[test]
    fn install_replaces_legacy_eye_entries() {
        let legacy = json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "\"$HOME/.claude/eye/hook.sh\""}]}], "PostToolUse": [{"hooks": [{"type": "command", "command": "\"$HOME/.claude/eye/hook.sh\""}]}]}});
        let out = install(legacy, &cmd());
        let text = serde_json::to_string(&out).unwrap();
        assert!(!text.contains(".claude/eye/hook.sh"), "{text}");
        assert!(is_installed(&out));
        assert!(!is_installed(&json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "\"$HOME/.claude/eye/hook.sh\""}]}]}})), "a legacy-only install does not count as installed");
    }

    #[test]
    fn is_installed_false_without_hooks() {
        assert!(!is_installed(&json!({})));
        assert!(!is_installed(&json!({"hooks": {}})));
    }

    #[cfg(unix)]
    #[test]
    fn install_script_to_writes_script_backup_and_settings() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.json"), "{\"model\":\"opus\"}").unwrap();
        install_script_to(dir.path()).unwrap();

        let script = dir.path().join("maya/hook.sh");
        assert!(script.exists());
        let mode = std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&script).unwrap().permissions());
        assert!(mode & 0o100 != 0, "script must be executable");

        let backups: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("settings.json.maya-backup-")).collect();
        assert_eq!(backups.len(), 1);

        assert!(status(dir.path()).unwrap());
        let s: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("settings.json")).unwrap()).unwrap();
        assert_eq!(s["model"], "opus");

        remove_from(dir.path()).unwrap();
        assert!(!status(dir.path()).unwrap());
    }

    #[cfg(target_os = "macos")]
    fn install_here(dir: &Path) -> Result<(), String> {
        install_to(dir)
    }

    #[cfg(any(windows, target_os = "linux"))]
    fn install_here(dir: &Path) -> Result<(), String> {
        let exe = dir.join("source-hook");
        std::fs::write(&exe, b"MZ fake hook").unwrap();
        install_with(dir, &exe)
    }

    #[cfg(windows)]
    #[test]
    fn install_with_copies_the_hook_and_points_settings_at_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.json"), "{\"model\":\"opus\"}").unwrap();
        install_here(dir.path()).unwrap();
        assert_eq!(std::fs::read(dir.path().join("maya").join(HOOK_EXE)).unwrap(), b"MZ fake hook");
        let s: Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("settings.json")).unwrap()).unwrap();
        assert_eq!(s["model"], "opus");
        assert_eq!(s["hooks"]["Stop"][0]["hooks"][0]["command"], "\"$HOME/.claude/maya/maya-hook.exe\"");
        assert!(status(dir.path()).unwrap());
        // Installing again with the same binary is fine, even when it is in use.
        install_here(dir.path()).unwrap();
        remove_from(dir.path()).unwrap();
        assert!(!status(dir.path()).unwrap());
    }

    #[test]
    fn tokens_are_kept_per_pipe_and_dropped_when_the_session_ends() {
        let dir = tempfile::tempdir().unwrap();
        let pipe = r"\\.\pipe\LOCAL\cc-msg-0123abcd";
        let path = dir.path().join("inbox").join("cc-msg-0123abcd.token");
        assert_eq!(token_path(dir.path(), pipe), Some(path.clone()));
        record_token(dir.path(), "PostToolUse", Some(pipe), Some(" tok1 ")).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "tok1");
        record_token(dir.path(), "Stop", Some(pipe), Some("tok2")).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "tok2");
        // No token or no socket leaves what is there.
        record_token(dir.path(), "Stop", Some(pipe), None).unwrap();
        record_token(dir.path(), "Stop", None, Some("tok3")).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "tok2");
        record_token(dir.path(), "SessionEnd", Some(pipe), Some("tok2")).unwrap();
        assert!(!path.exists());
        record_token(dir.path(), "SessionEnd", Some(pipe), None).unwrap();
    }

    #[test]
    fn token_files_are_named_only_after_plain_pipe_names() {
        let d = Path::new("m");
        assert_eq!(token_path(d, "/tmp/cc-socks/49643.sock"), None, "a dot is not plain");
        assert_eq!(token_path(d, r"\\.\pipe\LOCAL\.."), None, "only the last component is used, and .. is not plain");
        assert_eq!(token_path(d, ""), None);
        assert_eq!(token_path(d, "cc-msg-ab"), Some(d.join("inbox").join("cc-msg-ab.token")));
    }

    #[cfg(not(windows))]
    #[test]
    fn hook_source_prefers_the_plain_name_then_a_triple_suffixed_one() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(hook_source_in(d.path()), None);
        std::fs::write(d.path().join("maya-hook-x86_64-unknown-linux-gnu"), b"x").unwrap();
        assert_eq!(hook_source_in(d.path()).unwrap().file_name().unwrap(), "maya-hook-x86_64-unknown-linux-gnu");
        std::fs::write(d.path().join("maya-hook"), b"x").unwrap();
        assert_eq!(hook_source_in(d.path()).unwrap().file_name().unwrap(), "maya-hook");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn install_with_copies_the_binary_executable_and_points_hooks_at_it() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("settings.json"), "{}").unwrap();
        let src = d.path().join("src-hook");
        std::fs::write(&src, b"#!/bin/sh\n").unwrap();
        install_with(d.path(), &src).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let installed = d.path().join("maya").join("maya-hook");
        assert_eq!(std::fs::metadata(&installed).unwrap().permissions().mode() & 0o777, 0o755);
        assert!(status(d.path()).unwrap());
        let s: Value = serde_json::from_str(&std::fs::read_to_string(d.path().join("settings.json")).unwrap()).unwrap();
        assert_eq!(s["hooks"]["Stop"][0]["hooks"][0]["command"], "\"$HOME/.claude/maya/maya-hook\"");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_cli_script_counts_as_installed_and_the_app_binary_replaces_it() {
        let d = tempfile::tempdir().unwrap();
        install_script_to(d.path()).unwrap();
        assert!(status(d.path()).unwrap());
        let text = std::fs::read_to_string(d.path().join("settings.json")).unwrap();
        assert!(!text.contains("SessionStart"), "the script is not hooked to token-only events: {text}");
        let src = d.path().join("src-hook");
        std::fs::write(&src, b"#!/bin/sh\n").unwrap();
        install_with(d.path(), &src).unwrap();
        let text = std::fs::read_to_string(d.path().join("settings.json")).unwrap();
        assert!(!text.contains("hook.sh"), "{text}");
        let s: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(s["hooks"]["Stop"].as_array().unwrap().len(), 1);
        remove_from(d.path()).unwrap();
        assert!(!status(d.path()).unwrap());
    }

    #[test]
    fn token_only_events_are_hooked_where_the_hook_is_a_binary() {
        let out = install(json!({}), &cmd());
        assert_eq!(out["hooks"].get("SessionStart").is_some(), cfg!(any(windows, target_os = "linux")));
        assert!(!is_installed(&remove(out.clone())));
        assert_eq!(event_name(r#"{"hook_event_name":"SessionStart"}"#).as_deref(), Some("SessionStart"));
        assert_eq!(event_name("nope"), None);
    }

    #[test]
    fn record_projects_only_needed_fields() {
        let payload = r#"{"session_id":"s1","hook_event_name":"PostToolUse","agent_id":"ag","transcript_path":"/t.jsonl","cwd":"/x","tool_name":"Bash","tool_input":{"command":"ls","description":"list","questions":null},"tool_response":{"stdout":"HUGE"},"permission_mode":"auto"}"#;
        let v: Value = serde_json::from_str(&record(payload, 1_790_000_000_000).unwrap()).unwrap();
        assert!(v.get("tool_response").is_none());
        assert!(v.get("permission_mode").is_none());
        assert!(v.get("cwd").is_none());
        assert_eq!(v["agent_id"], "ag");
        assert_eq!(v["transcript_path"], "/t.jsonl");
        assert_eq!(v["tool_input"], json!({"command": "ls"}));
        assert_eq!(v["received_at"], 1_790_000_000_000u64);
    }

    #[test]
    fn record_caps_long_strings_by_characters() {
        let payload = format!(r#"{{"session_id":"s1","tool_input":{{"command":"{}"}}}}"#, "é".repeat(20_000));
        let line = record(&payload, 1).unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["tool_input"]["command"].as_str().unwrap().chars().count(), 400);
        assert!(line.len() < 1000, "{}", line.len());
    }

    #[test]
    fn record_ignores_what_is_not_an_object() {
        assert_eq!(record("not json", 1), None);
        assert_eq!(record("[1]", 1), None);
        assert_eq!(record("{}", 7).as_deref(), Some(r#"{"tool_input":{},"received_at":7}"#));
    }

    #[test]
    fn append_record_adds_one_line_per_payload() {
        let dir = tempfile::tempdir().unwrap();
        let maya = dir.path().join("maya");
        append_record(&maya, r#"{"session_id":"a","hook_event_name":"Stop"}"#, 1).unwrap();
        append_record(&maya, "garbage", 2).unwrap();
        append_record(&maya, r#"{"session_id":"b"}"#, 3).unwrap();
        let log = std::fs::read_to_string(maya.join("events.jsonl")).unwrap();
        let ids: Vec<String> = log.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()["session_id"].as_str().unwrap().to_string()).collect();
        assert_eq!(ids, ["a", "b"]);
    }

    #[test]
    fn install_to_creates_settings_when_missing_and_refuses_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        install_here(dir.path()).unwrap();
        assert!(status(dir.path()).unwrap());

        let bad = tempfile::tempdir().unwrap();
        std::fs::write(bad.path().join("settings.json"), "{ not json").unwrap();
        let err = install_here(bad.path()).unwrap_err();
        assert!(err.contains("settings.json"));
        assert_eq!(std::fs::read_to_string(bad.path().join("settings.json")).unwrap(), "{ not json");
    }

    #[cfg(unix)]
    #[test]
    fn hook_script_projects_only_needed_fields() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("hook.sh");
        std::fs::write(&script, HOOK_SCRIPT).unwrap();
        let payload = br#"{"session_id":"s1","hook_event_name":"PostToolUse","agent_id":"ag","transcript_path":"/t.jsonl","cwd":"/x","tool_name":"Bash","tool_input":{"command":"ls","description":"list","questions":null},"tool_response":{"stdout":"HUGE"},"permission_mode":"auto"}"#;
        let out = std::process::Command::new("sh")
            .arg(&script)
            .env("HOME", dir.path())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child.stdin.take().unwrap().write_all(payload).unwrap();
                child.wait_with_output()
            })
            .unwrap();
        assert!(out.status.success());
        let log = std::fs::read_to_string(dir.path().join(".claude/maya/events.jsonl")).unwrap();
        let v: serde_json::Value = serde_json::from_str(log.trim()).unwrap();
        assert!(v.get("tool_response").is_none(), "{log}");
        assert!(v.get("permission_mode").is_none(), "{log}");
        assert_eq!(v["agent_id"], "ag");
        assert_eq!(v["transcript_path"], "/t.jsonl");
        assert_eq!(v["tool_input"]["command"], "ls");
        assert!(v["tool_input"].get("description").is_none(), "{log}");
        assert!(v["tool_input"].get("questions").is_none(), "null members dropped: {log}");
        assert!(v["received_at"].as_u64().unwrap() > 1_700_000_000_000);
    }

    #[cfg(unix)]
    #[test]
    fn hook_script_caps_long_command_text() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("hook.sh");
        std::fs::write(&script, HOOK_SCRIPT).unwrap();
        let payload = format!(r#"{{"session_id":"s1","hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{{"command":"{}"}}}}"#, "x".repeat(20_000));
        let out = std::process::Command::new("sh")
            .arg(&script)
            .env("HOME", dir.path())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child.stdin.take().unwrap().write_all(payload.as_bytes()).unwrap();
                child.wait_with_output()
            })
            .unwrap();
        assert!(out.status.success());
        let log = std::fs::read_to_string(dir.path().join(".claude/maya/events.jsonl")).unwrap();
        let v: serde_json::Value = serde_json::from_str(log.trim()).unwrap();
        assert_eq!(v["tool_input"]["command"].as_str().unwrap().len(), 400);
        assert!(log.len() < 1000, "line should be small: {}", log.len());
    }

    #[cfg(unix)]
    #[test]
    fn hook_script_appends_payload_with_received_at() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("hook.sh");
        std::fs::write(&script, HOOK_SCRIPT).unwrap();
        let out = std::process::Command::new("sh")
            .arg(&script)
            .env("HOME", dir.path())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child.stdin.take().unwrap().write_all(b"{\"session_id\":\"s1\",\"hook_event_name\":\"Stop\"}").unwrap();
                child.wait_with_output()
            })
            .unwrap();
        assert!(out.status.success());
        assert!(out.stdout.is_empty(), "hook must print nothing");
        let log = std::fs::read_to_string(dir.path().join(".claude/maya/events.jsonl")).unwrap();
        let v: serde_json::Value = serde_json::from_str(log.trim()).unwrap();
        assert_eq!(v["session_id"], "s1");
        assert!(v["received_at"].as_u64().unwrap() > 1_700_000_000_000);
    }
}
