//! Handing the main role to an assistant: the group the new main gets and
//! how each side's config changes. The group carries public keys only, and
//! travels over the encrypted channel, so nothing secret moves.

use super::group::GroupCopy;
use crate::config::{Config, NetworkRole, PairedAssistant};

/// The group the new main gets: every member but the one promoted, then
/// the old main's own entry (`me`). Tombstones and version go as they are.
pub fn handover(group: &GroupCopy, promoted: &str, me: &PairedAssistant) -> GroupCopy {
    GroupCopy {
        version: group.version,
        members: group.members.iter().filter(|p| p.id != promoted).cloned().chain(std::iter::once(me.clone())).collect(),
        removed: group.removed.clone(),
    }
}

/// The promoted assistant's config: a main with the group, on its own port
/// and name, with its link to the old main gone and no redirect to keep.
pub fn as_main(c: &mut Config, group: GroupCopy) {
    let n = &mut c.network;
    n.role = NetworkRole::Main;
    n.assistants = group.members;
    n.removed = group.removed;
    n.group_version = group.version;
    n.main_host.clear();
    n.main_port = 0;
    n.main_key.clear();
    n.end_redirect();
}

/// The old main's config: an assistant of `host:port`, pinning the new
/// main's key. It keeps the group, and remembers the port it listened on
/// as main, for the redirect. Listening for "Maya" goes off, as pairing does.
pub fn as_assistant_of(c: &mut Config, host: &str, port: u16, main_key: &str) {
    let listened = c.listen_port();
    let n = &mut c.network;
    n.role = NetworkRole::Assistant;
    n.main_host = host.into();
    n.main_port = port;
    n.main_key = main_key.into();
    n.redirect_port = listened;
    c.listen = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, name: &str) -> PairedAssistant {
        PairedAssistant { id: id.into(), name: name.into(), hostname: "h".into(), platform: "macos".into(), key: format!("k-{id}"), address: "10.0.0.9".into(), last_seen: Some(5), mirror: false, generation: 0 }
    }

    #[test]
    fn handover_keeps_tombstones_and_version_and_adds_me() {
        let me = crate::net::group::me(&Default::default(), "a2V5");
        let group = GroupCopy { version: 7, members: vec![entry("g", "laptop"), entry("b", "mini")], removed: vec![entry("x", "gone")] };
        let out = handover(&group, "g", &me);
        assert_eq!(out.members.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["b", "a2V5"]);
        assert_eq!(out.members[0], entry("b", "mini"), "the others go as they are, key included");
        assert_eq!((out.version, out.removed), (7, vec![entry("x", "gone")]));
        assert_eq!(out.members[1].key, "a2V5", "the old main connects with its own key");
        assert!(!out.members[1].mirror);
    }

    #[test]
    fn the_old_main_keeps_the_group_and_its_port_for_redirects() {
        let mut c = Config::default();
        c.network.role = NetworkRole::Main;
        c.network.port = 4200;
        c.network.assistants = vec![entry("a", "one"), entry("b", "two")];
        as_assistant_of(&mut c, "10.0.0.9", 5000, "a2V5");
        assert_eq!(c.network.assistants, vec![entry("a", "one"), entry("b", "two")]);
        assert_eq!(c.network.redirect_port, 4200);
    }

    #[test]
    fn the_new_main_takes_the_group_and_stops_redirecting() {
        let mut c = Config::default();
        c.network.redirect_port = 4200;
        let group = GroupCopy { version: 9, members: vec![entry("b", "mini")], removed: vec![entry("x", "gone")] };
        as_main(&mut c, group);
        assert_eq!(c.network.redirect_port, 0);
        assert_eq!((c.network.group_version, c.network.removed.clone()), (9, vec![entry("x", "gone")]));
        assert_eq!(c.network.assistants, vec![entry("b", "mini")]);
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
        as_main(&mut c, GroupCopy { members: vec![entry("b", "mini")], ..Default::default() });
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
        assert_eq!(c.network.assistants, vec![entry("g", "laptop")], "the group stays");
        assert!(!c.listen, "the main listens for its assistants");
    }
}
