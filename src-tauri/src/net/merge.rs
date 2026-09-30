//! Local and remote cards as one board, and where a session id lives.

use crate::model::Card;
use std::collections::HashSet;

pub const STALE_MS: u64 = 30_000;
pub const EXPIRE_MS: u64 = 5 * 60_000;

/// The last snapshot from one assistant.
#[derive(Clone, Debug)]
pub struct RemoteBoard {
    pub machine: String,
    pub hostname: String,
    pub platform: String,
    pub cards: Vec<Card>,
    pub dirs: Vec<String>,
    pub received_at: u64,
    pub connected: bool,
}

/// Local cards first, then each machine's cards with `machine` set and
/// `stale` when the snapshot is old or the link is down; machines quiet for
/// `EXPIRE_MS` are left out.
///
/// A session id shows once: a disconnected board's card gives way to a
/// connected board listing the same id (a machine paired again, whose old
/// snapshot lingers), and otherwise the first board listing it wins.
pub fn merged(local: Vec<Card>, remotes: &[RemoteBoard], now_ms: u64) -> Vec<Card> {
    let mut out = local;
    let live = |b: &RemoteBoard| now_ms.saturating_sub(b.received_at) <= EXPIRE_MS;
    let connected: HashSet<&str> = remotes.iter().filter(|b| b.connected && live(b)).flat_map(|b| b.cards.iter().map(|c| c.session_id.as_str())).collect();
    let mut seen: HashSet<String> = HashSet::new();
    for b in remotes {
        let age = now_ms.saturating_sub(b.received_at);
        if !live(b) {
            continue;
        }
        let stale = !b.connected || age > STALE_MS;
        for c in &b.cards {
            if seen.contains(&c.session_id) || (!b.connected && connected.contains(c.session_id.as_str())) {
                continue;
            }
            seen.insert(c.session_id.clone());
            let mut c = c.clone();
            c.machine = Some(b.machine.clone());
            c.stale = stale;
            out.push(c);
        }
    }
    out
}

/// Names as shown on cards, from `(name, hostname, id)`: a name shared by two
/// machines gets its hostname, and one still shared (the same machine paired
/// twice) also gets the first four characters of its id.
pub fn display_names(entries: &[(String, String, String)]) -> Vec<String> {
    entries
        .iter()
        .map(|(name, host, id)| {
            let same_name = entries.iter().filter(|(n, _, _)| n == name).count() > 1;
            let same_host = entries.iter().filter(|(n, h, _)| n == name && h == host).count() > 1;
            match (same_name, same_host) {
                (false, _) => name.clone(),
                (true, false) => format!("{name} ({host})"),
                (true, true) => format!("{name} ({host}, {})", id.chars().take(4).collect::<String>()),
            }
        })
        .collect()
}

/// The machine a session id runs on; a connected board wins over a
/// disconnected one listing the same id.
pub fn machine_of(remotes: &[RemoteBoard], session_id: &str) -> Option<String> {
    let listing = || remotes.iter().filter(|b| b.cards.iter().any(|c| c.session_id == session_id));
    listing().find(|b| b.connected).or_else(|| listing().next()).map(|b| b.machine.clone())
}

pub fn dirs_of(remotes: &[RemoteBoard], machine: &str) -> Vec<String> {
    remotes.iter().find(|b| b.machine == machine).map(|b| b.dirs.clone()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Card, Harness, State};

    fn card(id: &str, name: &str) -> Card {
        Card { session_id: id.into(), pid: 1, name: name.into(), cwd: "/x/p".into(), state: State::Idle, state_since: 0, snippet: "".into(), awaiting: None, has_inbox: true, harness: Harness::ClaudeCode, pr: None, context: None, machine: None, stale: false }
    }

    fn board(machine: &str, hostname: &str, ids: &[&str], received_at: u64, connected: bool) -> RemoteBoard {
        RemoteBoard { machine: machine.into(), hostname: hostname.into(), platform: "macos".into(), cards: ids.iter().map(|i| card(i, i)).collect(), dirs: vec!["proj".into()], received_at, connected }
    }

    #[test]
    fn remote_cards_follow_local_ones_and_carry_their_machine() {
        let m = merged(vec![card("l1", "local")], &[board("laptop", "h1", &["r1", "r2"], 1_000, true)], 1_500);
        assert_eq!(m.iter().map(|c| c.session_id.as_str()).collect::<Vec<_>>(), ["l1", "r1", "r2"]);
        assert_eq!(m[0].machine, None);
        assert_eq!(m[1].machine.as_deref(), Some("laptop"));
        assert!(!m[1].stale);
    }

    #[test]
    fn stale_after_thirty_seconds_or_disconnect_and_gone_after_five_minutes() {
        let fresh = merged(vec![], &[board("a", "h", &["r"], 1_000, true)], 1_000 + STALE_MS);
        assert!(!fresh[0].stale);
        let stale = merged(vec![], &[board("a", "h", &["r"], 1_000, true)], 1_000 + STALE_MS + 1);
        assert!(stale[0].stale);
        let dropped = merged(vec![], &[board("a", "h", &["r"], 1_000, false)], 1_500);
        assert!(dropped[0].stale, "a disconnect greys at once");
        assert!(merged(vec![], &[board("a", "h", &["r"], 1_000, false)], 1_000 + EXPIRE_MS + 1).is_empty());
    }

    #[test]
    fn a_session_id_shows_once_and_a_connected_board_wins() {
        // The old pairing's snapshot sorts first but is disconnected.
        let boards = [board("laptop (h, abcd)", "h", &["r1", "old"], 1_000, false), board("laptop (h, wxyz)", "h", &["r1", "r2"], 1_000, true)];
        let m = merged(vec![], &boards, 1_500);
        let ids: Vec<(&str, &str)> = m.iter().map(|c| (c.session_id.as_str(), c.machine.as_deref().unwrap())).collect();
        assert_eq!(ids, [("old", "laptop (h, abcd)"), ("r1", "laptop (h, wxyz)"), ("r2", "laptop (h, wxyz)")]);
        // Two connected boards listing the same id: the first wins.
        let both = [board("a", "h1", &["r1"], 1_000, true), board("b", "h2", &["r1"], 1_000, true)];
        assert_eq!(merged(vec![], &both, 1_500).iter().map(|c| c.machine.clone().unwrap()).collect::<Vec<_>>(), ["a"]);
    }

    #[test]
    fn routing_prefers_the_connected_board() {
        let boards = [board("laptop (h, abcd)", "h", &["r1"], 0, false), board("laptop (h, wxyz)", "h", &["r1"], 0, true)];
        assert_eq!(machine_of(&boards, "r1").as_deref(), Some("laptop (h, wxyz)"));
        let none_connected = [board("a", "h", &["r1"], 0, false), board("b", "h", &["r1"], 0, false)];
        assert_eq!(machine_of(&none_connected, "r1").as_deref(), Some("a"));
    }

    #[test]
    fn duplicate_names_get_the_hostname() {
        let e = |n: &str, h: &str, id: &str| (n.to_string(), h.to_string(), id.to_string());
        let names = display_names(&[e("laptop", "h1", "a1"), e("laptop", "h2", "b2"), e("desk", "h3", "c3")]);
        assert_eq!(names, ["laptop (h1)", "laptop (h2)", "desk"]);
        let twice = display_names(&[e("laptop", "h1", "abcdef"), e("laptop", "h1", "wxyz12"), e("laptop", "h2", "q")]);
        assert_eq!(twice, ["laptop (h1, abcd)", "laptop (h1, wxyz)", "laptop (h2)"], "the same machine paired twice stays distinct");
    }

    #[test]
    fn routes_a_session_id_to_its_machine() {
        let boards = [board("a", "h1", &["r1"], 0, true), board("b", "h2", &["r2"], 0, true)];
        assert_eq!(machine_of(&boards, "r2").as_deref(), Some("b"));
        assert_eq!(machine_of(&boards, "l1"), None);
        assert_eq!(dirs_of(&boards, "a"), vec!["proj".to_string()]);
        assert!(dirs_of(&boards, "zzz").is_empty());
    }
}
