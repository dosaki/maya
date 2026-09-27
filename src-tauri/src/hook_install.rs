use serde_json::{json, Map, Value};
use std::path::Path;

pub const HOOK_MARKER: &str = ".claude/eye/hook.sh";

pub const HOOK_SCRIPT: &str = r#"#!/bin/sh
# Installed by Eye (Claude session board). Appends each hook payload to
# ~/.claude/eye/events.jsonl with a received_at timestamp. Never blocks,
# never prints, always exits 0.
dir="$HOME/.claude/eye"
mkdir -p "$dir" 2>/dev/null
t=$(date +%s000)
# Keep only the fields Eye reads; tool_response and the rest are dropped so
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

fn group_is_ours(group: &Value) -> bool {
    group["hooks"]
        .as_array()
        .map(|hs| hs.iter().any(|h| h["command"].as_str().map_or(false, |c| c.contains(HOOK_MARKER))))
        .unwrap_or(false)
}

pub fn is_installed(settings: &Value) -> bool {
    settings["hooks"]
        .as_object()
        .map(|hooks| hooks.values().any(|groups| groups.as_array().map_or(false, |g| g.iter().any(group_is_ours))))
        .unwrap_or(false)
}

/// Removes our entries, then adds one group per event. Idempotent.
pub fn install(settings: Value, command: &str) -> Value {
    let settings = remove(settings);
    let mut root = settings.as_object().cloned().unwrap_or_default();
    let mut hooks = root.get("hooks").and_then(|h| h.as_object()).cloned().unwrap_or_default();
    for (event, matcher) in HOOK_EVENTS {
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
        let backup = claude_dir.join(format!("settings.json.eye-backup-{stamp}"));
        std::fs::copy(&path, &backup).map_err(|e| format!("cannot back up settings.json: {e}"))?;
    }
    let text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(&path, text + "\n").map_err(|e| format!("cannot write settings.json: {e}"))
}

pub fn install_to(claude_dir: &Path) -> Result<(), String> {
    let settings = read_settings(claude_dir)?;
    let eye_dir = claude_dir.join("eye");
    std::fs::create_dir_all(&eye_dir).map_err(|e| format!("cannot create {}: {e}", eye_dir.display()))?;
    let script = eye_dir.join("hook.sh");
    std::fs::write(&script, HOOK_SCRIPT).map_err(|e| format!("cannot write hook.sh: {e}"))?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    let command = "\"$HOME/.claude/eye/hook.sh\"";
    write_settings_with_backup(claude_dir, &install(settings, command))
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

    #[test]
    fn install_adds_every_event_with_marker_command() {
        let out = install(json!({"model": "opus"}), "\"$HOME/.claude/eye/hook.sh\"");
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
        let once = install(existing, "\"$HOME/.claude/eye/hook.sh\"");
        let twice = install(once.clone(), "\"$HOME/.claude/eye/hook.sh\"");
        assert_eq!(once, twice);
        let stop = twice["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2);
        assert_eq!(stop[0]["hooks"][0]["command"], "say done");
    }

    #[test]
    fn remove_strips_only_marker_entries() {
        let existing = json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}]}]}});
        let installed = install(existing, "\"$HOME/.claude/eye/hook.sh\"");
        let removed = remove(installed);
        assert!(!is_installed(&removed));
        assert_eq!(removed["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert!(removed["hooks"].get("PermissionRequest").is_none(), "empty arrays are dropped");
    }

    #[test]
    fn install_and_remove_preserve_existing_key_order() {
        let existing: Value = serde_json::from_str(r#"{"zeta":1,"hooks":{"Stop":[{"hooks":[{"type":"command","command":"say done"}]}]},"alpha":2}"#).unwrap();
        let installed = install(existing, "x/.claude/eye/hook.sh");
        let out = serde_json::to_string(&installed).unwrap();
        assert!(out.starts_with(r#"{"zeta":1,"hooks":"#), "{out}");
        assert!(out.ends_with(r#""alpha":2}"#), "{out}");
        let out = serde_json::to_string(&remove(installed)).unwrap();
        assert_eq!(out, r#"{"zeta":1,"hooks":{"Stop":[{"hooks":[{"type":"command","command":"say done"}]}]},"alpha":2}"#);
    }

    #[test]
    fn is_installed_false_without_hooks() {
        assert!(!is_installed(&json!({})));
        assert!(!is_installed(&json!({"hooks": {}})));
    }

    #[test]
    fn install_to_writes_script_backup_and_settings() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.json"), "{\"model\":\"opus\"}").unwrap();
        install_to(dir.path()).unwrap();

        let script = dir.path().join("eye/hook.sh");
        assert!(script.exists());
        let mode = std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&script).unwrap().permissions());
        assert!(mode & 0o100 != 0, "script must be executable");

        let backups: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("settings.json.eye-backup-")).collect();
        assert_eq!(backups.len(), 1);

        assert!(status(dir.path()).unwrap());
        let s: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("settings.json")).unwrap()).unwrap();
        assert_eq!(s["model"], "opus");

        remove_from(dir.path()).unwrap();
        assert!(!status(dir.path()).unwrap());
    }

    #[test]
    fn install_to_creates_settings_when_missing_and_refuses_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        install_to(dir.path()).unwrap();
        assert!(status(dir.path()).unwrap());

        let bad = tempfile::tempdir().unwrap();
        std::fs::write(bad.path().join("settings.json"), "{ not json").unwrap();
        let err = install_to(bad.path()).unwrap_err();
        assert!(err.contains("settings.json"));
        assert_eq!(std::fs::read_to_string(bad.path().join("settings.json")).unwrap(), "{ not json");
    }

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
        let log = std::fs::read_to_string(dir.path().join(".claude/eye/events.jsonl")).unwrap();
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
        let log = std::fs::read_to_string(dir.path().join(".claude/eye/events.jsonl")).unwrap();
        let v: serde_json::Value = serde_json::from_str(log.trim()).unwrap();
        assert_eq!(v["tool_input"]["command"].as_str().unwrap().len(), 400);
        assert!(log.len() < 1000, "line should be small: {}", log.len());
    }

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
        let log = std::fs::read_to_string(dir.path().join(".claude/eye/events.jsonl")).unwrap();
        let v: serde_json::Value = serde_json::from_str(log.trim()).unwrap();
        assert_eq!(v["session_id"], "s1");
        assert!(v["received_at"].as_u64().unwrap() > 1_700_000_000_000);
    }
}
