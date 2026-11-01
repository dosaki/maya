//! The group of trusted devices: the rules every Maya applies to its copy.
//! Pure functions over `NetworkConfig` and `GroupCopy`; no I/O.

use super::protocol;
use crate::config::{NetworkConfig, NetworkRole, PairedAssistant};
use serde::{Deserialize, Serialize};

/// The group as one Maya holds it: its version, the live members and the
/// tombstones of the removed ones. Public keys and labels only.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct GroupCopy {
    pub version: u64,
    pub members: Vec<PairedAssistant>,
    pub removed: Vec<PairedAssistant>,
}

/// One device in the list Settings shows.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRow {
    pub id: String,
    pub key: String,
    pub name: String,
    pub hostname: String,
    pub platform: String,
    pub address: String,
    pub last_seen: Option<u64>,
    pub is_main: bool,
    pub is_self: bool,
}

/// The group as `n` holds it.
pub fn of_config(n: &NetworkConfig) -> GroupCopy {
    GroupCopy { version: n.group_version, members: n.assistants.clone(), removed: n.removed.clone() }
}

/// Adds (or pairs again) a member: the version moves up by one and the
/// entry takes it as its generation. A tombstone with the same key goes; a
/// live entry with the same key is replaced, keeping its `id`.
pub fn add(g: &mut GroupCopy, mut entry: PairedAssistant) -> PairedAssistant {
    g.version += 1;
    entry.generation = g.version;
    g.removed.retain(|r| r.key != entry.key);
    match g.members.iter_mut().find(|m| m.key == entry.key) {
        Some(m) => {
            entry.id = m.id.clone();
            *m = entry.clone();
        }
        None => g.members.push(entry.clone()),
    }
    entry
}

/// Removes the live member with `key`, leaving a tombstone one version
/// newer than the group was. `None` when there is no such member.
pub fn remove(g: &mut GroupCopy, key: &str) -> Option<PairedAssistant> {
    let at = g.members.iter().position(|m| m.key == key)?;
    let mut gone = g.members.remove(at);
    g.version += 1;
    gone.generation = g.version;
    gone.mirror = false;
    g.removed.retain(|r| r.key != key);
    g.removed.push(gone.clone());
    Some(gone)
}

/// Takes the removals of another copy: a tombstone removes a live member of
/// an older or equal generation. Members the other copy lists are never
/// added. Returns whether anything changed.
pub fn merge_copy(g: &mut GroupCopy, theirs: &GroupCopy) -> bool {
    let mut changed = false;
    for t in &theirs.removed {
        if let Some(at) = g.members.iter().position(|m| m.key == t.key && m.generation <= t.generation) {
            g.members.remove(at);
            g.removed.retain(|r| r.key != t.key);
            g.removed.push(t.clone());
            changed = true;
        } else if let Some(r) = g.removed.iter_mut().find(|r| r.key == t.key) {
            if t.generation > r.generation {
                r.generation = t.generation;
                changed = true;
            }
        }
    }
    if changed {
        g.version = g.version.max(theirs.version) + 1;
    }
    changed
}

/// Takes `copy` as this Maya's group, minus its own entry.
pub fn adopt(n: &mut NetworkConfig, my_key: &str, copy: &GroupCopy) {
    n.assistants = copy.members.iter().filter(|m| m.key != my_key).cloned().collect();
    n.removed = copy.removed.clone();
    n.group_version = copy.version;
}

/// Whether `key` may be followed as the main. With the group the sender
/// gave (`seen`), it must be a live member of that; without one, of the
/// group held here (an empty group is an older main that sends none).
pub fn may_follow(n: &NetworkConfig, seen: Option<&GroupCopy>, key: &str) -> bool {
    match seen {
        Some(s) => s.members.iter().any(|m| m.key == key) && !s.removed.iter().any(|r| r.key == key),
        None => {
            (n.assistants.iter().any(|m| m.key == key) && !n.removed.iter().any(|r| r.key == key))
                || n.assistants.is_empty()
        }
    }
}

/// Phones follow a main; they never are one.
pub fn is_phone(p: &PairedAssistant) -> bool {
    p.platform == "android"
}

/// Makes this Maya an assistant of the live member `key` at `host:port`.
pub fn join(n: &mut NetworkConfig, my_key: &str, key: &str, host: &str, port: u16) -> Result<(), String> {
    if key == my_key {
        return Err("This is this device.".into());
    }
    let member = n.assistants.iter().find(|m| m.key == key).ok_or("That device is not in your group.")?;
    if is_phone(member) {
        return Err("A phone cannot be the main Maya.".into());
    }
    let host = host.trim();
    if host.is_empty() {
        return Err("Type the main Maya's host name or address.".into());
    }
    n.role = NetworkRole::Assistant;
    n.main_host = host.to_string();
    n.main_port = if port == 0 { protocol::DEFAULT_PORT } else { port };
    n.main_key = key.to_string();
    n.pair_again = false;
    n.end_redirect();
    Ok(())
}

/// This Maya's own entry in the group.
pub fn me(n: &NetworkConfig, key: &str) -> PairedAssistant {
    let hostname = super::local_hostname();
    let name = if n.name.trim().is_empty() { hostname.clone() } else { n.name.clone() };
    PairedAssistant {
        id: key.to_string(),
        name,
        hostname,
        platform: std::env::consts::OS.to_string(),
        key: key.to_string(),
        address: String::new(),
        last_seen: None,
        mirror: false,
        generation: 0,
    }
}

/// The devices to show: this one first, then each live member in order.
/// Tombstones never appear.
pub fn device_rows(n: &NetworkConfig, me: &PairedAssistant) -> Vec<DeviceRow> {
    let row = |p: &PairedAssistant, is_main: bool, is_self: bool| DeviceRow {
        id: p.id.clone(),
        key: p.key.clone(),
        name: p.name.clone(),
        hostname: p.hostname.clone(),
        platform: p.platform.clone(),
        address: p.address.clone(),
        last_seen: p.last_seen,
        is_main,
        is_self,
    };
    let mut rows = vec![row(me, n.role == NetworkRole::Main, true)];
    for p in &n.assistants {
        rows.push(row(p, n.role == NetworkRole::Assistant && p.key == n.main_key, false));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, name: &str) -> PairedAssistant {
        PairedAssistant {
            id: format!("id-{key}-{name}"),
            name: name.into(),
            hostname: format!("{name}.lan"),
            platform: "macos".into(),
            key: key.into(),
            address: String::new(),
            last_seen: None,
            mirror: true,
            generation: 0,
        }
    }

    fn net(members: Vec<PairedAssistant>) -> NetworkConfig {
        NetworkConfig { assistants: members, ..Default::default() }
    }

    #[test]
    fn add_stamps_generation_and_keeps_the_id_of_a_known_key() {
        let mut g = GroupCopy::default();
        let first = add(&mut g, entry("A", "one"));
        assert_eq!(first.generation, 1);
        let again = add(&mut g, entry("A", "two"));
        assert_eq!(g.members.len(), 1);
        assert_eq!(again.id, first.id);
        assert_eq!(g.members[0].name, "two");
        assert_eq!(g.members[0].generation, 2);
        assert_eq!(g.version, 2);
    }

    #[test]
    fn remove_makes_a_tombstone() {
        let mut g = GroupCopy::default();
        add(&mut g, entry("A", "one"));
        assert!(remove(&mut g, "A").is_some());
        assert!(g.members.is_empty());
        assert_eq!(g.removed[0].generation, 2);
        assert!(!g.removed[0].mirror);
        assert_eq!(g.version, 2);
        assert_eq!(remove(&mut g, "nope"), None);
    }

    #[test]
    fn a_stale_tombstone_does_not_remove_a_device_paired_again() {
        let mut g = GroupCopy::default();
        add(&mut g, entry("A", "one"));
        remove(&mut g, "A");
        add(&mut g, entry("A", "one"));
        assert_eq!(g.members[0].generation, 3);
        assert!(g.removed.is_empty());
        let mut stale = entry("A", "one");
        stale.generation = 2;
        let theirs = GroupCopy { version: 2, members: vec![], removed: vec![stale] };
        assert!(!merge_copy(&mut g, &theirs));
        assert_eq!(g.members.len(), 1);
    }

    #[test]
    fn a_newer_tombstone_removes_and_raises_the_version_above_both() {
        let mut g = GroupCopy { version: 1, members: vec![], removed: vec![] };
        let mut a = entry("A", "one");
        a.generation = 1;
        g.members.push(a);
        let mut t = entry("A", "one");
        t.generation = 4;
        let theirs = GroupCopy { version: 7, members: vec![], removed: vec![t] };
        assert!(merge_copy(&mut g, &theirs));
        assert!(g.members.is_empty());
        assert_eq!(g.removed[0].generation, 4);
        assert_eq!(g.version, 8);
    }

    #[test]
    fn a_tie_goes_to_removed() {
        let mut g = GroupCopy::default();
        let mut a = entry("A", "one");
        a.generation = 3;
        g.members.push(a);
        let mut t = entry("A", "one");
        t.generation = 3;
        let theirs = GroupCopy { version: 3, members: vec![], removed: vec![t] };
        assert!(merge_copy(&mut g, &theirs));
        assert!(g.members.is_empty());
    }

    #[test]
    fn members_from_a_copy_are_never_added() {
        let mut g = GroupCopy::default();
        let theirs = GroupCopy { version: 5, members: vec![entry("B", "two")], removed: vec![] };
        assert!(!merge_copy(&mut g, &theirs));
        assert!(g.members.is_empty());
    }

    #[test]
    fn a_higher_generation_for_a_known_tombstone_counts_as_a_change() {
        let mut g = GroupCopy::default();
        add(&mut g, entry("A", "one"));
        remove(&mut g, "A");
        let mut t = entry("A", "one");
        t.generation = 9;
        let theirs = GroupCopy { version: 9, members: vec![], removed: vec![t] };
        assert!(merge_copy(&mut g, &theirs));
        assert_eq!(g.removed[0].generation, 9);
        assert_eq!(g.version, 10);
    }

    #[test]
    fn adopt_drops_my_own_entry() {
        let mut n = NetworkConfig::default();
        let copy = GroupCopy { version: 4, members: vec![entry("me", "mine"), entry("A", "one")], removed: vec![entry("Z", "gone")] };
        adopt(&mut n, "me", &copy);
        assert_eq!(n.assistants.len(), 1);
        assert_eq!(n.assistants[0].key, "A");
        assert_eq!(n.removed.len(), 1);
        assert_eq!(n.group_version, 4);
    }

    #[test]
    fn may_follow_cases() {
        let n = net(vec![entry("A", "one")]);
        let live = GroupCopy { version: 1, members: vec![entry("B", "two")], removed: vec![] };
        assert!(may_follow(&n, Some(&live), "B"));
        let gone = GroupCopy { version: 1, members: vec![entry("B", "two")], removed: vec![entry("B", "two")] };
        assert!(!may_follow(&n, Some(&gone), "B"));
        assert!(may_follow(&NetworkConfig::default(), None, "B"));
        assert!(!may_follow(&n, None, "B"));
        assert!(may_follow(&n, None, "A"));
    }

    #[test]
    fn join_rejects() {
        let mut phone = entry("P", "phone");
        phone.platform = "android".into();
        let mut n = net(vec![entry("A", "one"), phone]);
        assert_eq!(join(&mut n, "me", "me", "h", 1), Err("This is this device.".into()));
        assert_eq!(join(&mut n, "me", "X", "h", 1), Err("That device is not in your group.".into()));
        assert_eq!(join(&mut n, "me", "P", "h", 1), Err("A phone cannot be the main Maya.".into()));
        assert_eq!(join(&mut n, "me", "A", "  ", 1), Err("Type the main Maya's host name or address.".into()));
        assert_eq!(n.role, NetworkRole::Off);

        n.redirect_port = 9000;
        n.pair_again = true;
        assert_eq!(join(&mut n, "me", "A", " host.lan ", 0), Ok(()));
        assert_eq!(n.role, NetworkRole::Assistant);
        assert_eq!(n.main_host, "host.lan");
        assert_eq!(n.main_port, 4127);
        assert_eq!(n.main_key, "A");
        assert!(!n.pair_again);
        assert_eq!(n.redirect_port, 0);
    }

    #[test]
    fn device_rows_marks_self_and_main_and_hides_tombstones() {
        let mut n = net(vec![entry("A", "one"), entry("B", "two")]);
        n.removed = vec![entry("Z", "gone")];
        n.role = NetworkRole::Assistant;
        n.main_key = "B".into();
        let mine = entry("me", "mine");
        let rows = device_rows(&n, &mine);
        assert_eq!(rows.len(), 3);
        assert!(rows[0].is_self && !rows[0].is_main);
        assert_eq!(rows[1].key, "A");
        assert!(!rows[1].is_main);
        assert!(rows[2].is_main && !rows[2].is_self);
        n.role = NetworkRole::Main;
        let rows = device_rows(&n, &mine);
        assert!(rows[0].is_main);
        assert!(!rows[2].is_main);
    }

    #[test]
    fn me_uses_the_hostname_when_the_name_is_blank() {
        let e = me(&NetworkConfig::default(), "k");
        assert_eq!(e.id, "k");
        assert_eq!(e.name, e.hostname);
        let e = me(&NetworkConfig { name: "Desk".into(), ..Default::default() }, "k");
        assert_eq!(e.name, "Desk");
    }
}
