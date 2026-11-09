//! The issues the user started a session for, kept across restarts.

use super::WorkItem;
use crate::model::Card;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Started {
    pub key: String,
    /// Its latest copy.
    pub item: WorkItem,
    /// The folder its session works in.
    pub dir: String,
    /// Epoch millis.
    pub started_at: u64,
}

/// How long a started item waits for its session to show on the board.
pub const GRACE_MS: u64 = 10 * 60 * 1000;

/// `<maya_dir>/work-items.json`.
pub fn path(maya_dir: &Path) -> PathBuf {
    maya_dir.join("work-items.json")
}

/// The started items; none when the file is missing or does not parse.
pub fn load(path: &Path) -> Vec<Started> {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(path: &Path, started: &[Started]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(started).map_err(|e| e.to_string())?;
    crate::config::write_private_atomic(path, text.as_bytes())
}

/// The name a started item's session is given.
pub fn session_name(id: &str) -> String {
    format!("issue {id}")
}

/// A card is the item's session when it carries the item's name, or works
/// in the item's folder or below it.
pub fn has_session(s: &Started, cards: &[Card]) -> bool {
    let name = session_name(&s.item.id);
    let dir = Path::new(&s.dir);
    cards.iter().any(|c| c.name == name || (!s.dir.is_empty() && Path::new(&c.cwd).starts_with(dir)))
}

/// Drops the items that closed, and those whose session has not shown up
/// (or has gone) once `GRACE_MS` has passed since they started.
pub fn prune(started: Vec<Started>, closed: &[String], cards: &[Card], now_ms: u64) -> Vec<Started> {
    started
        .into_iter()
        .filter(|s| !closed.contains(&s.key))
        .filter(|s| has_session(s, cards) || now_ms.saturating_sub(s.started_at) <= GRACE_MS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Card;
    use crate::work_items::{test_item, WorkItem};

    #[test]
    fn a_started_item_matches_its_session_by_name_or_folder() {
        let s = Started { key: "github:a/b#1".into(), item: WorkItem { id: "a/b#1".into(), ..test_item() }, dir: "/c/b-issue-1".into(), started_at: 0 };
        let card = |name: &str, cwd: &str| Card { name: name.into(), cwd: cwd.into(), ..Card::test_default() };
        assert!(has_session(&s, &[card("issue a/b#1", "/elsewhere")]));
        assert!(has_session(&s, &[card("renamed", "/c/b-issue-1/sub")]));
        assert!(!has_session(&s, &[card("renamed", "/c/b-issue-10")]), "a sibling folder is not the item's");
    }

    #[test]
    fn prune_drops_closed_items_and_sessionless_ones_after_the_grace() {
        let s = |key: &str, at: u64| Started { key: key.into(), item: test_item(), dir: format!("/c/{key}"), started_at: at };
        let now = GRACE_MS * 3;
        let kept = prune(vec![s("closed", now), s("young", now - 1000), s("old", 0)], &["closed".into()], &[], now);
        assert_eq!(kept.iter().map(|s| s.key.as_str()).collect::<Vec<_>>(), vec!["young"]);
    }

    #[test]
    fn the_started_file_round_trips_and_a_bad_one_reads_empty() {
        let dir = tempfile::tempdir().unwrap();
        let p = path(dir.path());
        assert!(load(&p).is_empty());
        let one = vec![Started { key: "k".into(), item: test_item(), dir: "/d".into(), started_at: 5 }];
        save(&p, &one).unwrap();
        assert_eq!(load(&p), one);
        std::fs::write(&p, "not json").unwrap();
        assert!(load(&p).is_empty());
    }
}
