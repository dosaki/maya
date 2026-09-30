//! Main and assistant Mayas over the local network.
pub mod protocol;
pub mod merge;
pub mod server;
pub mod client;

use crate::config::{Config, NetworkRole, PairedAssistant};
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
    /// Why the main's server is not running (the port is taken…); `None` once it starts.
    pub main_error: Option<String>,
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
    /// Its IP address as the main last saw it; empty if never recorded.
    pub address: String,
    pub connected: bool,
    pub last_seen: Option<u64>,
    /// Something the user should know: a different Maya version, or a board
    /// this Maya could not read.
    pub note: Option<String>,
}

#[derive(Serialize, Clone, Default, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AssistantLink {
    pub connected: bool,
    pub main_name: Option<String>,
    /// The last failure; while `retrying`, the page shows it under "Reconnecting…".
    pub error: Option<String>,
    /// The client is waiting to try again (not removed, not unpaired).
    pub retrying: bool,
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
        // A new port or a new name: assistants reconnect within seconds and see both.
        (true, true) if before.listen_port() != after.listen_port() || before.network.name != after.network.name => out.push(NetChange::RestartMain),
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

/// The paired list as the main shows it while its server is down: every
/// assistant disconnected, named as they would be on cards.
pub fn paired_offline(paired: &[PairedAssistant]) -> Vec<AssistantStatus> {
    let entries: Vec<(String, String)> = paired.iter().map(|p| (p.name.clone(), p.address.clone())).collect();
    paired
        .iter()
        .zip(merge::display_names(&entries))
        .map(|(p, name)| AssistantStatus { id: p.id.clone(), name, hostname: p.hostname.clone(), platform: p.platform.clone(), address: p.address.clone(), connected: false, last_seen: p.last_seen, note: None })
        .collect()
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
        assert_eq!(network_change(&main, &with(Main, |c| c.network.name = "desk".into())), vec![RestartMain], "renaming the main restarts it so assistants see the name");
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
    fn a_main_that_cannot_listen_says_why_and_keeps_its_paired_list() {
        let p = |id: &str, name: &str, address: &str| PairedAssistant { id: id.into(), name: name.into(), hostname: "h".into(), platform: "macos".into(), token: "t".into(), address: address.into(), last_seen: Some(42) };
        let status = NetworkStatus { role: NetworkRole::Main, assistants: paired_offline(&[p("a1", "laptop", "10.0.0.5"), p("b2", "desk", "10.0.0.6")]), main_error: Some("Could not listen on port 4127: Address already in use".into()), ..Default::default() };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["mainError"], "Could not listen on port 4127: Address already in use");
        assert_eq!(json["assistants"][0]["name"], "laptop");
        assert_eq!(json["assistants"][0]["address"], "10.0.0.5");
        assert_eq!(json["assistants"][0]["lastSeen"], 42, "when it was last seen survives a restart");
        let twins = paired_offline(&[p("a1", "laptop", "10.0.0.5"), p("b2", "laptop", "10.0.0.6")]);
        assert_eq!(twins.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["laptop (10.0.0.5)", "laptop (10.0.0.6)"]);
        assert_eq!(json["assistants"][1]["connected"], false);
        assert_eq!(serde_json::to_value(NetworkStatus::default()).unwrap()["mainError"], serde_json::Value::Null);
    }

    #[test]
    fn hostname_is_not_empty() {
        assert!(!local_hostname().is_empty());
        assert!(!local_hostname().ends_with(".local"));
    }
}
