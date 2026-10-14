//! Handing the main role to an assistant: the list the new main gets and
//! how each side's config changes. The list carries public keys only, and
//! travels over the encrypted channel, so nothing secret moves.

use super::protocol::new_id;
use crate::config::{Config, NetworkRole, PairedAssistant};

/// The entry the old main makes for itself in the list it hands over: it
/// connects to the new main with its own key. Its address is filled in by
/// the new main at its first connection. `mirror` is true for a phone: the
/// user pressed Make main on it.
pub fn own_entry(main_name: &str, mirror: bool, key: &str) -> PairedAssistant {
    PairedAssistant { id: new_id(), name: main_name.into(), hostname: super::local_hostname(), platform: std::env::consts::OS.into(), key: key.into(), address: String::new(), last_seen: None, mirror }
}

/// Every paired assistant but the one promoted, then the old main's own entry.
pub fn handover_list(paired: &[PairedAssistant], promoted: &str, me: &PairedAssistant) -> Vec<PairedAssistant> {
    paired.iter().filter(|p| p.id != promoted).cloned().chain(std::iter::once(me.clone())).collect()
}

/// The promoted assistant's config: a main with the list, on its own port
/// and name, with its link to the old main gone.
pub fn as_main(c: &mut Config, list: Vec<PairedAssistant>) {
    let n = &mut c.network;
    n.role = NetworkRole::Main;
    n.assistants = list;
    n.main_host.clear();
    n.main_port = 0;
    n.main_key.clear();
}

/// The old main's config: an assistant of `host:port`, pinning the new
/// main's key. Listening for "Maya" goes off, as pairing does.
pub fn as_assistant_of(c: &mut Config, host: &str, port: u16, main_key: &str) {
    let n = &mut c.network;
    n.role = NetworkRole::Assistant;
    n.main_host = host.into();
    n.main_port = port;
    n.main_key = main_key.into();
    n.assistants.clear();
    c.listen = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, name: &str) -> PairedAssistant {
        PairedAssistant { id: id.into(), name: name.into(), hostname: "h".into(), platform: "macos".into(), key: format!("k-{id}"), address: "10.0.0.9".into(), last_seen: Some(5), mirror: false }
    }

    #[test]
    fn only_a_phone_s_own_entry_may_mirror() {
        assert!(own_entry("Pixel", true, "a2V5").mirror);
        assert!(!own_entry("desk", false, "a2V5").mirror);
    }

    #[test]
    fn the_list_leaves_the_new_main_out_and_adds_the_old_one() {
        let me = own_entry("desk", false, "a2V5");
        let list = handover_list(&[entry("g", "laptop"), entry("b", "mini")], "g", &me);
        assert_eq!(list.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["b", me.id.as_str()]);
        assert_eq!(list[0], entry("b", "mini"), "the others go as they are, key included");
        assert_eq!(me.name, "desk");
        assert!(me.address.is_empty() && me.last_seen.is_none());
        assert!(!me.id.is_empty());
        assert_eq!(me.key, "a2V5", "the old main connects with its own key");
    }

    #[test]
    fn the_new_main_keeps_its_port_and_name_and_forgets_its_link() {
        let mut c = Config::default();
        c.network.role = NetworkRole::Assistant;
        c.network.port = 5000;
        c.network.name = "laptop".into();
        c.network.main_host = "10.0.0.2".into();
        c.network.main_port = 4127;
        c.network.main_key = "k-old".into();
        as_main(&mut c, vec![entry("b", "mini")]);
        assert_eq!(c.network.role, NetworkRole::Main);
        assert_eq!((c.network.port, c.network.name.as_str()), (5000, "laptop"));
        assert_eq!(c.network.assistants, vec![entry("b", "mini")]);
        assert!(c.network.main_host.is_empty() && c.network.main_port == 0 && c.network.main_key.is_empty());
    }

    #[test]
    fn the_old_main_becomes_an_assistant_pinning_the_new_key_and_stops_listening() {
        let mut c = Config::default();
        c.network.role = NetworkRole::Main;
        c.network.assistants = vec![entry("g", "laptop")];
        c.listen = true;
        as_assistant_of(&mut c, "10.0.0.9", 5000, "a2V5");
        assert_eq!(c.network.role, NetworkRole::Assistant);
        assert_eq!((c.network.main_host.as_str(), c.network.main_port), ("10.0.0.9", 5000));
        assert_eq!(c.network.main_key, "a2V5");
        assert!(c.network.assistants.is_empty());
        assert!(!c.listen, "the main listens for its assistants");
    }
}
