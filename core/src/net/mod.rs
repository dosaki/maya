//! Main and assistant Mayas over the local network.
pub mod protocol;
pub mod merge;
pub mod routing;
pub mod server;
pub mod client;
pub mod promote;
pub mod identity;
pub mod secure;
pub mod live;
pub mod group;

use crate::config::{Config, NetworkConfig, NetworkRole, PairedAssistant};
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
    /// This machine's IPv4 addresses, for an assistant to type as the main's host.
    pub addresses: Vec<String>,
    /// The last code took three wrong tries; the main must regenerate one.
    pub code_used_up: bool,
    /// Protocol 1 pairings were dropped on update (`NetworkConfig::pair_again`).
    pub pair_again: bool,
    /// Every device of the group: this one first, then the others.
    #[serde(default)]
    pub devices: Vec<group::DeviceRow>,
}

pub const CODE_USED_UP: &str = "Code used up after 3 wrong tries — Regenerate.";

/// An address another machine on the LAN could reach: not loopback,
/// link-local (169.254/16) or unspecified.
fn usable_v4(ip: std::net::Ipv4Addr) -> bool {
    !(ip.is_loopback() || ip.is_link_local() || ip.is_unspecified())
}

/// This machine's usable IPv4 addresses, sorted; empty when they cannot be listed.
pub fn local_addresses() -> Vec<String> {
    let mut v: Vec<String> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i.addr {
            if_addrs::IfAddr::V4(a) if usable_v4(a.ip) => Some(a.ip.to_string()),
            _ => None,
        })
        .collect();
    v.sort();
    v.dedup();
    v
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
    /// Connected and able to become the main.
    pub can_lead: bool,
    /// Connected and able to follow a new main; false for an older Maya.
    pub can_follow: bool,
    /// The main lets it see and control every session (its paired entry).
    pub mirror: bool,
    /// Connected and asked to mirror (`CAP_MIRROR`): the main shows its switch.
    pub wants_mirror: bool,
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
    /// What the main said about mirroring at the last connect; `None` before.
    pub mirror: Option<MirrorAccess>,
    /// The main interprets voice commands for this Maya (`CAP_VOICE` at the last welcome).
    pub voice: bool,
    /// The main shows its Chat to this Maya (`CAP_CHAT` at the last welcome).
    pub chat: bool,
    /// The word the main listens for, from its last board; `None` before
    /// one, or from an older main.
    pub wake_word: Option<String>,
}

/// Whether the main shares its board with this Maya.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MirrorAccess {
    Allowed,
    /// The main's switch for this Maya is off.
    Refused,
    /// The main runs an older Maya that cannot mirror.
    Unsupported,
}

/// The running side of the network role: the main's server or the
/// assistant's client, and an old main's redirect listener.
#[derive(Default)]
pub struct NetworkState {
    pub server: Option<server::ServerHandle>,
    pub client: Option<client::ClientHandle>,
    /// After a handover, on the port this Maya listened on as main.
    pub redirect: Option<server::ServerHandle>,
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
    /// The old main's redirect listener; started once `StopMain` frees its port.
    StartRedirect,
    StopRedirect,
    RestartRedirect,
}

/// The redirect an old main keeps: its port and the main it points to.
/// Only an assistant that handed over has one.
pub fn redirect_of(c: &Config) -> Option<(u16, server::MoveTarget)> {
    let n = &c.network;
    (n.role == NetworkRole::Assistant && n.redirect_port != 0 && !n.main_key.is_empty()).then(|| (n.redirect_port, server::MoveTarget { host: n.main_host.clone(), port: c.main_port(), key: n.main_key.clone() }))
}

/// A role switched by hand ends the redirect; a handover (`network_promote`)
/// does not come through here.
pub fn settle_role_change(before: &NetworkConfig, after: &mut NetworkConfig) {
    if before.role != after.role {
        after.end_redirect();
    }
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
    let link = |c: &Config| (c.network.main_host.clone(), c.main_port(), c.network.name.clone(), c.network.main_key.clone());
    match (b.role == Assistant, a.role == Assistant) {
        (true, false) => out.insert(0, NetChange::StopAssistant),
        (false, true) => out.push(NetChange::StartAssistant),
        (true, true) if link(before) != link(after) => out.push(NetChange::RestartAssistant),
        _ => {}
    }
    // Stopped before anything starts, which may take its port; started
    // after `StopMain` frees it.
    match (redirect_of(before), redirect_of(after)) {
        (Some(_), None) => out.insert(0, NetChange::StopRedirect),
        (None, Some(_)) => out.push(NetChange::StartRedirect),
        (Some(b), Some(a)) if b != a => out.push(NetChange::RestartRedirect),
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
        .map(|(p, name)| AssistantStatus { id: p.id.clone(), name, hostname: p.hostname.clone(), platform: p.platform.clone(), address: p.address.clone(), connected: false, last_seen: p.last_seen, note: None, can_lead: false, can_follow: false, mirror: p.mirror, wants_mirror: false })
        .collect()
}

/// This computer's name without `.local`, for the main's `welcome`.
#[cfg(windows)]
pub fn local_hostname() -> String {
    std::env::var("COMPUTERNAME").ok().filter(|n| !n.is_empty()).unwrap_or_else(|| "this PC".into())
}

/// This computer's name without `.local`, for the main's `welcome`.
#[cfg(unix)]
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

    #[test]
    fn usable_addresses_skip_loopback_and_link_local() {
        let ip = |s: &str| s.parse::<std::net::Ipv4Addr>().unwrap();
        assert!(usable_v4(ip("192.168.1.20")));
        assert!(usable_v4(ip("10.0.0.5")));
        assert!(!usable_v4(ip("127.0.0.1")));
        assert!(!usable_v4(ip("169.254.10.1")));
        assert!(!usable_v4(ip("0.0.0.0")));
    }

    #[test]
    fn local_addresses_are_usable_v4_strings() {
        for a in local_addresses() {
            assert!(usable_v4(a.parse().unwrap()), "{a}");
        }
    }

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

    /// The old main after it handed over to the main at 10.0.0.9:5001.
    fn redirecting(c: &mut Config) {
        c.network.main_host = "10.0.0.9".into();
        c.network.main_port = 5001;
        c.network.main_key = "a2V5".into();
        c.network.redirect_port = 4200;
    }

    #[test]
    fn network_change_starts_and_stops_the_redirect() {
        use NetChange::*;
        use NetworkRole::*;
        let main = with(Main, |c| c.network.port = 4200);
        let old = with(Assistant, redirecting);
        assert_eq!(network_change(&main, &old), vec![StopMain, StartAssistant, StartRedirect], "the redirect starts once the main's port is free");
        let later = with(Assistant, |c| {
            redirecting(c);
            c.network.main_host = "10.0.0.10".into();
            c.network.main_key = "b2V5".into();
        });
        assert_eq!(network_change(&old, &later), vec![RestartAssistant, RestartRedirect], "a later promotion points the redirect at the newest main");
        assert_eq!(network_change(&old, &with(Assistant, |c| {
            redirecting(c);
            c.network.redirect_port = 4300;
        })), vec![RestartRedirect]);
        let ended = with(Assistant, |c| {
            redirecting(c);
            c.network.redirect_port = 0;
        });
        assert_eq!(network_change(&old, &ended), vec![StopRedirect]);
        assert_eq!(network_change(&old, &old.clone()), vec![]);
        assert_eq!(network_change(&main, &with(Assistant, |c| {
            redirecting(c);
            c.network.main_key.clear();
        })), vec![StopMain, StartAssistant], "no main to point to, no redirect");
        assert_eq!(network_change(&old, &with(Main, |c| c.network.port = 4200)), vec![StopRedirect, StopAssistant, StartMain], "the redirect frees the port before the main takes it");
    }

    #[test]
    fn a_hand_join_ends_the_redirect() {
        let old = with(NetworkRole::Assistant, |c| {
            redirecting(c);
            c.network.assistants.push(PairedAssistant { id: "u".into(), name: "mini".into(), hostname: "h".into(), platform: "macos".into(), key: "dQ==".into(), address: String::new(), last_seen: None, mirror: false, generation: 1 });
        });
        let mut joined = old.clone();
        group::join(&mut joined.network, "bWU=", "dQ==", "10.0.0.11", 4127).unwrap();
        assert_eq!(joined.network.redirect_port, 0);
        assert_eq!(network_change(&old, &joined), vec![NetChange::StopRedirect, NetChange::RestartAssistant]);
    }

    #[test]
    fn switching_role_by_hand_ends_the_redirect() {
        let old = with(NetworkRole::Assistant, redirecting);
        let mut off = old.network.clone();
        off.role = NetworkRole::Off;
        settle_role_change(&old.network, &mut off);
        assert_eq!(off.redirect_port, 0);
        let mut same = old.network.clone();
        same.name = "desk".into();
        settle_role_change(&old.network, &mut same);
        assert_eq!(same.redirect_port, 4200, "the same role keeps it");
    }

    #[test]
    fn a_main_that_cannot_listen_says_why_and_keeps_its_paired_list() {
        let p = |id: &str, name: &str, address: &str| PairedAssistant { id: id.into(), name: name.into(), hostname: "h".into(), platform: "macos".into(), key: "k".into(), address: address.into(), last_seen: Some(42), mirror: false, generation: 0 };
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
