//! Main and assistant Mayas over the local network.
pub mod protocol;
pub mod merge;
pub mod server;
pub mod client;

use crate::config::{Config, NetworkRole};
use serde::Serialize;

/// What the Network section of Settings shows, sent as the `network` event.
#[derive(Serialize, Clone, Default, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NetworkStatus {
    pub role: NetworkRole,
    /// The open pairing code on a main.
    pub code: Option<PairingCode>,
    /// A main's paired assistants.
    pub assistants: Vec<AssistantStatus>,
    /// An assistant's link to its main.
    pub assistant: AssistantLink,
}

#[derive(Serialize, Clone, Default, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PairingCode {
    pub code: String,
    pub expires_at: u64,
}

#[derive(Serialize, Clone, Default, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AssistantStatus {
    pub id: String,
    pub name: String,
    pub hostname: String,
    pub platform: String,
    pub connected: bool,
    pub last_seen: Option<u64>,
}

#[derive(Serialize, Clone, Default, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AssistantLink {
    pub connected: bool,
    pub main_name: Option<String>,
    pub error: Option<String>,
}

/// The running side of the network role: the main's server or the assistant's client.
#[derive(Default)]
pub struct NetworkState {
    pub server: Option<server::ServerHandle>,
    pub client: Option<client::ClientHandle>,
    pub status: NetworkStatus,
}

/// What a settings change asks of the server and the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetChange {
    StartMain,
    StopMain,
    RestartMain,
    StartAssistant,
    StopAssistant,
    RestartAssistant,
}

/// The changes to make, stops first; empty when nothing network-related changed.
pub fn network_change(before: &Config, after: &Config) -> Vec<NetChange> {
    use NetworkRole::*;
    let (b, a) = (&before.network, &after.network);
    let mut out = vec![];
    match (b.role == Main, a.role == Main) {
        (true, false) => out.push(NetChange::StopMain),
        (false, true) => out.push(NetChange::StartMain),
        (true, true) if before.listen_port() != after.listen_port() => out.push(NetChange::RestartMain),
        _ => {}
    }
    let link = |c: &Config| (c.network.main_host.clone(), c.main_port(), c.network.name.clone(), c.network.assistant_id.clone(), c.network.token.clone());
    match (b.role == Assistant, a.role == Assistant) {
        (true, false) => out.insert(0, NetChange::StopAssistant),
        (false, true) => out.push(NetChange::StartAssistant),
        (true, true) if link(before) != link(after) => out.push(NetChange::RestartAssistant),
        _ => {}
    }
    out
}

/// This computer's name without `.local`, for the main's `welcome`.
pub fn local_hostname() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer is valid for its length and gethostname NUL-terminates within it.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
    if rc != 0 {
        return "this Mac".into();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let name = String::from_utf8_lossy(&buf[..end]).into_owned();
    name.strip_suffix(".local").map(str::to_string).unwrap_or(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(role: NetworkRole, f: impl FnOnce(&mut Config)) -> Config {
        let mut c = Config::default();
        c.network.role = role;
        f(&mut c);
        c
    }

    #[test]
    fn role_and_port_changes_start_stop_or_restart() {
        use NetChange::*;
        use NetworkRole::*;
        let off = with(Off, |_| {});
        let main = with(Main, |_| {});
        assert_eq!(network_change(&off, &off), vec![]);
        assert_eq!(network_change(&off, &main), vec![StartMain]);
        assert_eq!(network_change(&main, &off), vec![StopMain]);
        assert_eq!(network_change(&main, &main), vec![]);
        assert_eq!(network_change(&main, &with(Main, |c| c.network.port = 5000)), vec![RestartMain]);
        assert_eq!(network_change(&main, &with(Main, |c| c.network.port = protocol::DEFAULT_PORT)), vec![], "0 and the default port are the same port");
        assert_eq!(network_change(&main, &with(Main, |c| c.completed_timeout_minutes += 1)), vec![], "other settings leave the server alone");
        let asst = with(Assistant, |c| c.network.main_host = "10.0.0.2".into());
        assert_eq!(network_change(&main, &asst), vec![StopMain, StartAssistant]);
        assert_eq!(network_change(&asst, &main), vec![StopAssistant, StartMain]);
        assert_eq!(network_change(&asst, &with(Assistant, |c| c.network.main_host = "10.0.0.3".into())), vec![RestartAssistant]);
        assert_eq!(network_change(&asst, &asst.clone()), vec![]);
        assert_eq!(network_change(&asst, &off), vec![StopAssistant]);
    }

    #[test]
    fn hostname_is_not_empty() {
        assert!(!local_hostname().is_empty());
        assert!(!local_hostname().ends_with(".local"));
    }
}
