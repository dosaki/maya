//! What a main does with a session command bound for an assistant: the
//! helpers the desktop app and the phone share. Sending itself is
//! `server::send_command_with`; this is what wraps the arguments.

use super::protocol::Attachment;
use super::NetworkStatus;
use crate::agents::{self, AgentInfo};
use crate::model::Harness;
use base64::Engine;
use serde::Serialize;
use std::time::Duration;

/// How long a main waits for a command's result from an assistant.
pub const ROUTE_TIMEOUT: Duration = Duration::from_secs(30);

/// The command must fit in one frame: one file, and all of them together.
const MAX_BYTES: u64 = 20 * 1024 * 1024;

/// What the Machine pickers show: each paired assistant's display name,
/// host and platform, and whether it is connected now.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MachineInfo {
    pub name: String,
    pub hostname: String,
    pub platform: String,
    pub connected: bool,
}

/// The agents a machine can start, and whether its Maya takes a session
/// name (an older one ignores names).
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentsReply {
    pub agents: Vec<AgentInfo>,
    pub names: bool,
}

/// Reads and base64-encodes local files for a remote reply's attachments;
/// refuses a missing file, one over 20 MB, or files over 20 MB together.
/// `name` on each `Attachment` is the path exactly as given, since the
/// assistant matches on it.
pub fn remote_attachments(paths: &[String]) -> Result<Vec<Attachment>, String> {
    let mut total = 0u64;
    for p in paths {
        let meta = std::fs::metadata(p).map_err(|_| format!("Attachment not found: {p}"))?;
        if meta.len() > MAX_BYTES {
            return Err("The file is too large (over 20 MB).".into());
        }
        total += meta.len();
    }
    if total > MAX_BYTES {
        return Err("Attachments total more than 20 MB.".into());
    }
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        let bytes = std::fs::read(p).map_err(|_| format!("Attachment not found: {p}"))?;
        out.push(Attachment { name: p.clone(), bytes: base64::engine::general_purpose::STANDARD.encode(bytes) });
    }
    Ok(out)
}

/// A remote that reports no agents runs an older Maya: it starts only
/// Claude Code and ignores a name. Locally, `local_list` is asked.
pub fn agents_reply(local: bool, remote: Option<Vec<AgentInfo>>, local_list: impl FnOnce() -> Vec<AgentInfo>) -> AgentsReply {
    if local {
        return AgentsReply { agents: local_list(), names: true };
    }
    match remote {
        Some(agents) => AgentsReply { agents, names: true },
        None => AgentsReply { agents: vec![agents::claude()], names: false },
    }
}

/// An older Maya ignores the agent and starts Claude Code: refuse instead.
pub fn check_remote_agent(agent: Harness, remote: &Option<Vec<AgentInfo>>, machine: &str) -> Result<(), String> {
    match remote {
        None if agent != Harness::ClaudeCode => Err(format!("{machine} runs an older Maya that can only start Claude Code.")),
        Some(list) if !list.iter().any(|a| a.harness == agent) => Err(format!("That agent is not installed on {machine}.")),
        _ => Ok(()),
    }
}

/// The pickers' machine list from the network status: the display names
/// the cards use (`name (address)` for assistants sharing a name).
pub fn machines_of(status: &NetworkStatus) -> Vec<MachineInfo> {
    status.assistants.iter().map(|a| MachineInfo { name: a.name.clone(), hostname: a.hostname.clone(), platform: a.platform.clone(), connected: a.connected }).collect()
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::AgentInfo;
    use crate::model::Harness;
    use crate::net::{AssistantStatus, NetworkStatus};
    use base64::Engine;

    #[test]
    fn remote_attachments_encodes_a_small_file() {
        let dir = std::env::temp_dir().join(format!("maya-route-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.txt");
        std::fs::write(&path, b"hello there").unwrap();
        let path_str = path.to_string_lossy().into_owned();
        let out = remote_attachments(&[path_str.clone()]).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, path_str, "the name carries the main's full local path exactly");
        assert_eq!(base64::engine::general_purpose::STANDARD.decode(&out[0].bytes).unwrap(), b"hello there");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn remote_attachments_refuses_a_missing_file() {
        let err = remote_attachments(&["/no/such/file-for-maya-tests.txt".to_string()]).unwrap_err();
        assert!(err.contains("Attachment not found"), "{err}");
    }

    #[test]
    fn remote_attachments_refuses_a_file_over_20_mb() {
        let dir = std::env::temp_dir().join(format!("maya-route-test-big-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.bin");
        std::fs::File::create(&path).unwrap().set_len(20 * 1024 * 1024 + 1).unwrap();
        let err = remote_attachments(&[path.to_string_lossy().into_owned()]).unwrap_err();
        assert_eq!(err, "The file is too large (over 20 MB).");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn remote_attachments_refuses_files_over_20_mb_together() {
        let dir = std::env::temp_dir().join(format!("maya-route-test-total-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths: Vec<String> = (0..2)
            .map(|i| {
                let path = dir.join(format!("part{i}.bin"));
                std::fs::File::create(&path).unwrap().set_len(11 * 1024 * 1024).unwrap();
                path.to_string_lossy().into_owned()
            })
            .collect();
        assert!(remote_attachments(&paths[..1]).is_ok(), "one alone fits");
        assert_eq!(remote_attachments(&paths).unwrap_err(), "Attachments total more than 20 MB.");
        std::fs::remove_dir_all(&dir).ok();
    }

    fn agent(h: Harness) -> AgentInfo {
        AgentInfo { harness: h, models: vec![], efforts: vec![], modes: vec![] }
    }

    #[test]
    fn an_older_remote_starts_claude_code_only_and_ignores_names() {
        let reply = agents_reply(false, None, Vec::new);
        assert_eq!(reply.agents.len(), 1);
        assert_eq!(reply.agents[0].harness, Harness::ClaudeCode);
        assert!(!reply.names);
        let reply = agents_reply(false, Some(vec![agent(Harness::Codex)]), Vec::new);
        assert_eq!(reply.agents[0].harness, Harness::Codex);
        assert!(reply.names);
        let reply = agents_reply(true, None, || vec![agent(Harness::Kiro)]);
        assert_eq!(reply.agents[0].harness, Harness::Kiro);
        assert!(reply.names);
    }

    #[test]
    fn check_remote_agent_refuses_what_the_machine_cannot_start() {
        assert_eq!(check_remote_agent(Harness::Codex, &None, "laptop").unwrap_err(), "laptop runs an older Maya that can only start Claude Code.");
        assert!(check_remote_agent(Harness::ClaudeCode, &None, "laptop").is_ok());
        let list = Some(vec![agent(Harness::ClaudeCode)]);
        assert_eq!(check_remote_agent(Harness::Codex, &list, "laptop").unwrap_err(), "That agent is not installed on laptop.");
        assert!(check_remote_agent(Harness::ClaudeCode, &list, "laptop").is_ok());
    }

    #[test]
    fn machines_of_keeps_the_status_display_names() {
        let status = NetworkStatus {
            assistants: vec![
                AssistantStatus { id: "a".into(), name: "laptop (10.0.0.2)".into(), hostname: "laptop".into(), platform: "macos".into(), address: "10.0.0.2".into(), connected: true, last_seen: None, note: None },
                AssistantStatus { id: "b".into(), name: "laptop (10.0.0.3)".into(), hostname: "laptop".into(), platform: "linux".into(), address: "10.0.0.3".into(), connected: false, last_seen: Some(1), note: None },
            ],
            ..Default::default()
        };
        let m = machines_of(&status);
        assert_eq!(m.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["laptop (10.0.0.2)", "laptop (10.0.0.3)"]);
        assert_eq!((m[0].connected, m[1].connected), (true, false));
        assert_eq!(m[1].platform, "linux");
    }
}
