//! Handing the main role to an assistant: the list the new main gets, the
//! seal around it, and how each side's config changes.

use super::protocol::{new_id, new_token, open, seal, UNSEALABLE};
use crate::config::{Config, NetworkRole, PairedAssistant};
use serde::{Deserialize, Serialize};

/// The handover's plaintext.
#[derive(Serialize, Deserialize)]
struct Handover {
    assistants: Vec<PairedAssistant>,
}

/// The entry the old main makes for itself in the list it hands over: it
/// connects to the new main with this id and token. Its address is filled
/// in by the new main at its first authentication.
pub fn own_entry(main_name: &str) -> PairedAssistant {
    PairedAssistant { id: new_id(), name: main_name.into(), hostname: super::local_hostname(), platform: std::env::consts::OS.into(), token: new_token(), address: String::new(), last_seen: None }
}

/// Every paired assistant but the one promoted, then the old main's own entry.
pub fn handover_list(paired: &[PairedAssistant], promoted: &str, me: &PairedAssistant) -> Vec<PairedAssistant> {
    paired.iter().filter(|p| p.id != promoted).cloned().chain(std::iter::once(me.clone())).collect()
}

pub fn seal_list(token: &str, list: &[PairedAssistant]) -> String {
    let plain = serde_json::to_vec(&Handover { assistants: list.to_vec() }).expect("a list serializes");
    seal(token, &plain)
}

pub fn open_list(token: &str, sealed: &str) -> Result<Vec<PairedAssistant>, String> {
    let plain = open(token, sealed)?;
    serde_json::from_slice::<Handover>(&plain).map(|h| h.assistants).map_err(|_| UNSEALABLE.to_string())
}

/// The promoted assistant's config: a main with the list, on its own port
/// and name, with its link to the old main gone.
pub fn as_main(c: &mut Config, list: Vec<PairedAssistant>) {
    let n = &mut c.network;
    n.role = NetworkRole::Main;
    n.assistants = list;
    n.main_host.clear();
    n.main_port = 0;
    n.assistant_id.clear();
    n.token.clear();
}

/// The old main's config: an assistant of `host:port` with the entry it
/// made for itself. Listening for "Maya" goes off, as pairing does: the
/// main listens for its assistants.
pub fn as_assistant_of(c: &mut Config, host: &str, port: u16, me: &PairedAssistant) {
    let n = &mut c.network;
    n.role = NetworkRole::Assistant;
    n.main_host = host.into();
    n.main_port = port;
    n.assistant_id = me.id.clone();
    n.token = me.token.clone();
    n.assistants.clear();
    c.listen = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, name: &str) -> PairedAssistant {
        PairedAssistant { id: id.into(), name: name.into(), hostname: "h".into(), platform: "macos".into(), token: format!("t-{id}"), address: "10.0.0.9".into(), last_seen: Some(5) }
    }

    #[test]
    fn the_list_leaves_the_new_main_out_and_adds_the_old_one() {
        let me = own_entry("desk");
        let list = handover_list(&[entry("g", "laptop"), entry("b", "mini")], "g", &me);
        assert_eq!(list.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["b", me.id.as_str()]);
        assert_eq!(list[0], entry("b", "mini"), "the others go as they are, token included");
        assert_eq!(me.name, "desk");
        assert!(me.address.is_empty() && me.last_seen.is_none());
        assert!(!me.id.is_empty() && !me.token.is_empty());
        assert_ne!(own_entry("desk").token, me.token, "a fresh token each time");
    }

    #[test]
    fn the_list_opens_with_the_new_mains_token_only() {
        let list = vec![entry("b", "mini")];
        let sealed = seal_list("t-g", &list);
        assert!(!sealed.contains("t-b"), "no token in the clear");
        assert_eq!(open_list("t-g", &sealed).unwrap(), list);
        assert_eq!(open_list("t-b", &sealed).unwrap_err(), UNSEALABLE);
    }

    #[test]
    fn the_new_main_keeps_its_port_and_name_and_forgets_its_link() {
        let mut c = Config::default();
        c.network.role = NetworkRole::Assistant;
        c.network.port = 5000;
        c.network.name = "laptop".into();
        c.network.main_host = "10.0.0.2".into();
        c.network.main_port = 4127;
        c.network.assistant_id = "g".into();
        c.network.token = "t-g".into();
        as_main(&mut c, vec![entry("b", "mini")]);
        assert_eq!(c.network.role, NetworkRole::Main);
        assert_eq!((c.network.port, c.network.name.as_str()), (5000, "laptop"));
        assert_eq!(c.network.assistants, vec![entry("b", "mini")]);
        assert!(c.network.main_host.is_empty() && c.network.main_port == 0 && c.network.assistant_id.is_empty() && c.network.token.is_empty());
    }

    #[test]
    fn the_old_main_becomes_an_assistant_with_its_own_entry_and_stops_listening() {
        let mut c = Config::default();
        c.network.role = NetworkRole::Main;
        c.network.assistants = vec![entry("g", "laptop")];
        c.listen = true;
        let me = own_entry("desk");
        as_assistant_of(&mut c, "10.0.0.9", 5000, &me);
        assert_eq!(c.network.role, NetworkRole::Assistant);
        assert_eq!((c.network.main_host.as_str(), c.network.main_port), ("10.0.0.9", 5000));
        assert_eq!((c.network.assistant_id.as_str(), c.network.token.as_str()), (me.id.as_str(), me.token.as_str()));
        assert!(c.network.assistants.is_empty());
        assert!(!c.listen, "the main listens for its assistants");
    }
}
