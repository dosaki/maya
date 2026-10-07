//! Codex lifecycle hooks, kept separate from Claude's settings and event log.
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const MARKER: &str = "maya-codex-hook";
const EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PermissionRequest",
    "PostToolUse",
    "Stop",
    "Interrupt",
    "SessionEnd",
];

pub fn dir() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".codex"))
}

fn read(dir: &Path) -> Result<Value, String> {
    match std::fs::read_to_string(dir.join("hooks.json")) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|e| format!("hooks.json is not valid JSON, refusing to modify it: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(e.to_string()),
    }
}

pub fn remove(mut value: Value) -> Value {
    // `get_mut`, not `value["hooks"]`: indexing inserts a null `hooks`,
    // which `install` then refuses as not an object.
    if let Some(hooks) = value.get_mut("hooks").and_then(Value::as_object_mut) {
        for groups in hooks.values_mut() {
            if let Some(groups) = groups.as_array_mut() {
                for group in groups.iter_mut() {
                    if let Some(handlers) = group["hooks"].as_array_mut() {
                        handlers
                            .retain(|h| !h["command"].as_str().is_some_and(|c| c.contains(MARKER)));
                    }
                }
                groups.retain(|g| !g["hooks"].as_array().is_some_and(Vec::is_empty));
            }
        }
        hooks.retain(|_, g| !g.as_array().is_some_and(Vec::is_empty));
    }
    value
}

pub fn install(value: Value, command: &str) -> Result<Value, String> {
    let mut value = remove(value);
    let root = value
        .as_object_mut()
        .ok_or("hooks.json must contain an object")?;
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("hooks must be an object")?;
    for event in EVENTS {
        let groups = hooks
            .entry(*event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or_else(|| format!("{event} hooks must be an array"))?;
        groups.push(json!({"hooks": [{"type": "command", "command": command, "timeout": 1}]}));
    }
    Ok(value)
}

fn write(dir: &Path, value: &Value) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join("hooks.json");
    if path.exists() {
        std::fs::copy(
            &path,
            dir.join(format!("hooks.json.maya-backup-{}", crate::now_ms())),
        )
        .map_err(|e| e.to_string())?;
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(value).map_err(|e| e.to_string())? + "\n",
    )
    .map_err(|e| e.to_string())
}

pub fn status(dir: &Path) -> Result<bool, String> {
    let value = read(dir)?;
    Ok(value["hooks"].as_object().is_some_and(|h| {
        h.values().any(|g| {
            g.as_array().is_some_and(|g| {
                g.iter().any(|g| {
                    g["hooks"].as_array().is_some_and(|h| {
                        h.iter()
                            .any(|h| h["command"].as_str().is_some_and(|c| c.contains(MARKER)))
                    })
                })
            })
        })
    }))
}

pub fn remove_from(dir: &Path) -> Result<bool, String> {
    write(dir, &remove(read(dir)?))?;
    status(dir)
}

pub fn install_to(dir: &Path) -> Result<bool, String> {
    #[cfg(any(windows, target_os = "linux"))]
    {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let exe_dir = exe.parent().ok_or("cannot find Maya's own folder")?;
        let source = crate::hook_install::hook_source_in(exe_dir).ok_or_else(|| format!("Maya's hook helper ({}) is not next to the app", crate::hook_install::HOOK_EXE))?;
        install_binary_with(dir, &source)
    }
    #[cfg(target_os = "macos")]
    {
        let value = read(dir)?;
        let target_dir = dir.join("maya");
        std::fs::create_dir_all(&target_dir).map_err(|e| e.to_string())?;
        use std::os::unix::fs::PermissionsExt;
        let target = target_dir.join("maya-codex-hook.sh");
        let script = crate::hook_install::HOOK_SCRIPT.replace("events.jsonl", "codex-events.jsonl");
        std::fs::write(&target, script).map_err(|e| e.to_string())?;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
        let command = format!("'{}'", target.display().to_string().replace('\'', "'\\''"));
        write(dir, &install(value, &command)?)?;
        status(dir)
    }
    #[cfg(target_os = "android")]
    {
        let _ = dir;
        Err("Codex hooks are not installed from the phone".into())
    }
}

/// The installed copy of the hook binary, named so the command carries `MARKER`.
#[cfg(windows)]
const CODEX_HOOK_EXE: &str = "maya-codex-hook.exe";
#[cfg(target_os = "linux")]
const CODEX_HOOK_EXE: &str = "maya-codex-hook";

/// `install_to` with Maya's hook binary taken from `source`: copied into
/// `dir/maya` (skipped when identical, since a running hook holds it open;
/// renamed into place on Linux, executable first) and run with `--codex`.
#[cfg(any(windows, target_os = "linux"))]
pub fn install_binary_with(dir: &Path, source: &Path) -> Result<bool, String> {
    let value = read(dir)?;
    let target_dir = dir.join("maya");
    std::fs::create_dir_all(&target_dir).map_err(|e| e.to_string())?;
    let target = target_dir.join(CODEX_HOOK_EXE);
    let bytes = std::fs::read(source).map_err(|e| format!("cannot read Maya's hook helper: {e}"))?;
    if std::fs::read(&target).ok().as_deref() != Some(bytes.as_slice()) {
        #[cfg(windows)]
        std::fs::write(&target, &bytes).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let fresh = target_dir.join(format!("{CODEX_HOOK_EXE}.new"));
            std::fs::write(&fresh, &bytes).map_err(|e| e.to_string())?;
            std::fs::set_permissions(&fresh, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
            std::fs::rename(&fresh, &target).map_err(|e| e.to_string())?;
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    #[cfg(windows)]
    let command = format!("\"{}\" --codex", target.display());
    #[cfg(unix)]
    let command = format!("{} --codex", crate::launch::shell_single_quote(&target.to_string_lossy()));
    write(dir, &install(value, &command)?)?;
    status(dir)
}

/// Only hooks newer than the rollout may override its state. Subagent
/// lifecycle events do not describe the parent turn.
pub fn apply(tail: &mut crate::foreign::ForeignTail, events: &[crate::events::HookEvent]) {
    for event in events {
        if event.agent_id.is_some() || event.received_at < tail.last_event_ms {
            continue;
        }
        match event.hook_event_name.as_str() {
            "UserPromptSubmit" | "PreToolUse" | "PostToolUse" => {
                tail.working = true;
                tail.awaiting = None;
            }
            "PermissionRequest" => {
                tail.working = true;
                tail.awaiting = Some(
                    event
                        .tool_input
                        .as_ref()
                        .and_then(|v| v["command"].as_str())
                        .map(|c| format!("Approve: {c}"))
                        .unwrap_or_else(|| "Waiting for your approval".into()),
                );
            }
            "Stop" | "Interrupt" | "SessionEnd" => {
                tail.working = false;
                tail.awaiting = None;
            }
            _ => continue,
        }
        tail.last_event_ms = event.received_at;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configuration_round_trip_backs_up_and_refuses_invalid_json() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let original = json!({"description":"keep","hooks":{"Stop":[{"hooks":[{"type":"command","command":"other"}]}]}});
        std::fs::write(dir.join("hooks.json"), original.to_string()).unwrap();
        write(dir, &install(original.clone(), "maya-codex-hook.exe --codex").unwrap()).unwrap();
        assert!(status(dir).unwrap());
        assert!(std::fs::read_dir(dir).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with("hooks.json.maya-backup-")));
        assert!(!remove_from(dir).unwrap());
        assert_eq!(read(dir).unwrap(), original);
        std::fs::write(dir.join("hooks.json"), "invalid").unwrap();
        assert!(remove_from(dir).is_err());
        assert_eq!(std::fs::read_to_string(dir.join("hooks.json")).unwrap(), "invalid");
    }

    #[test]
    fn install_is_idempotent_and_preserves_other_handlers() {
        let other = json!({"hooks":{"Stop":[{"hooks":[{"type":"command","command":"other"}]}]},"description":"keep"});
        let once = install(other.clone(), "maya-codex-hook.exe --codex").unwrap();
        assert_eq!(
            install(once.clone(), "maya-codex-hook.exe --codex").unwrap(),
            once
        );
        assert_eq!(remove(once), other);
        assert_eq!(remove(json!({})), json!({}));
        assert!(install(json!({}), "maya-codex-hook --codex").unwrap()["hooks"]["Stop"].is_array(), "a Codex without hooks.json yet");
    }
    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn the_binary_hook_is_copied_beside_the_config_and_run_with_codex() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("hook-source");
        std::fs::write(&source, b"first").unwrap();
        let dir = temp.path().join("codex");
        assert!(install_binary_with(&dir, &source).unwrap());
        let target = dir.join("maya").join(CODEX_HOOK_EXE);
        assert_eq!(std::fs::read(&target).unwrap(), b"first");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o755);
        }
        let commands: Vec<String> = read(&dir).unwrap()["hooks"]["Stop"].as_array().unwrap().iter().map(|g| g["hooks"][0]["command"].as_str().unwrap().to_string()).collect();
        assert_eq!(commands.len(), 1);
        assert!(commands[0].contains(CODEX_HOOK_EXE) && commands[0].ends_with(" --codex"), "{}", commands[0]);
        std::fs::write(&source, b"second").unwrap();
        assert!(install_binary_with(&dir, &source).unwrap());
        assert_eq!(std::fs::read(&target).unwrap(), b"second");
        assert_eq!(read(&dir).unwrap()["hooks"]["Stop"].as_array().unwrap().len(), 1, "a reinstall replaces, never doubles");
    }
    #[test]
    fn hooks_override_stale_rollouts_and_clear_approvals() {
        let mut tail = crate::foreign::ForeignTail::default();
        let event = |name: &str, ts| {
            serde_json::from_value(json!({"session_id":"s","hook_event_name":name,"received_at":ts,"tool_input":{"command":"cargo test"}})).unwrap()
        };
        apply(&mut tail, &[event("PermissionRequest", 20)]);
        assert_eq!(tail.awaiting.as_deref(), Some("Approve: cargo test"));
        apply(&mut tail, &[event("Stop", 10)]);
        assert!(tail.awaiting.is_some());
        apply(&mut tail, &[event("PostToolUse", 30)]);
        assert!(tail.working && tail.awaiting.is_none());
        apply(&mut tail, &[event("Stop", 40)]);
        assert!(!tail.working);
    }
}
