//! Watches: a session or a pull request the brain asked to hear about. Each is one-shot.

use std::path::PathBuf;

use serde_json::Value;

use super::notes::state_note;
use crate::model::Card;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Watch {
    pub id: u64,
    /// A session id.
    pub session: Option<String>,
    /// "repo#n" (or "repo!n" on GitLab).
    pub pr: Option<String>,
    pub note: String,
}

pub struct Watches {
    list: Vec<Watch>,
    next_id: u64,
    path: PathBuf,
}

/// Splits "repo#n" or "repo!n" into the repo part (lowercase) and the number.
fn split_ref(reference: &str) -> Option<(String, u64)> {
    let r = reference.trim();
    let at = r.rfind(['#', '!'])?;
    let number = r[at + 1..].trim().parse().ok()?;
    Some((r[..at].trim().to_lowercase(), number))
}

/// A card carries only a PR number and url, not its repo, so a reference
/// matches on the number and on the repo text appearing in the url.
fn card_matches_pr(card: &Card, reference: &str) -> bool {
    let Some((repo, number)) = split_ref(reference) else { return false };
    card.pr.as_ref().map_or(false, |p| p.number == number && (repo.is_empty() || p.url.to_lowercase().contains(&repo)))
}

impl Watches {
    /// Reads the file; a missing or corrupt one loads as empty.
    pub fn load(path: PathBuf) -> Watches {
        let list: Vec<Watch> = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let next_id = list.iter().map(|w| w.id).max().unwrap_or(0) + 1;
        Watches { list, next_id, path }
    }

    pub fn list(&self) -> &[Watch] {
        &self.list
    }

    fn save(&self) {
        let result = serde_json::to_vec_pretty(&self.list).map_err(|e| e.to_string()).and_then(|b| crate::config::write_private_atomic(&self.path, &b));
        if let Err(e) = result {
            crate::log::line("watch", format!("could not save the watches: {e}"));
        }
    }

    /// Adds a watch from a `{"kind":"watch", "session"|"pr", "note"}` action.
    pub fn add(&mut self, action: &Value, cards: &[Card]) -> Result<u64, String> {
        let note = action.get("note").and_then(Value::as_str).unwrap_or("").trim().to_string();
        let text = |k: &str| action.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
        let (session, pr) = match (text("session"), text("pr")) {
            (Some(name), _) => (Some(crate::interpreter::find_session(cards, name)?.session_id.clone()), None),
            (None, Some(reference)) => {
                if split_ref(reference).is_none() {
                    return Err(format!("{reference} is not a pull request reference like repo#12"));
                }
                (None, Some(reference.to_string()))
            }
            (None, None) => return Err("A watch needs a session or a pull request".into()),
        };
        let id = self.next_id;
        self.next_id += 1;
        self.list.push(Watch { id, session, pr, note });
        self.save();
        Ok(id)
    }

    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.list.len();
        self.list.retain(|w| w.id != id);
        let removed = self.list.len() != before;
        if removed {
            self.save();
        }
        removed
    }

    /// Removes and describes each watch whose target changed between the boards.
    pub fn fired(&mut self, before: &[Card], after: &[Card]) -> Vec<String> {
        let mut out = Vec::new();
        let mut done = Vec::new();
        for w in &self.list {
            let what = if let Some(sid) = &w.session {
                before.iter().find(|c| &c.session_id == sid).and_then(|b| match after.iter().find(|a| &a.session_id == sid) {
                    Some(a) => state_note(b, a),
                    None => Some(format!("{} ended", b.name)),
                })
            } else if let Some(reference) = &w.pr {
                after.iter().filter(|a| card_matches_pr(a, reference)).find_map(|a| {
                    let b = before.iter().find(|b| b.session_id == a.session_id)?;
                    let (old, new) = (b.pr.as_ref()?, a.pr.as_ref()?);
                    (old.state != new.state).then(|| format!("{reference} moved to {}", new.state))
                })
            } else {
                None
            };
            if let Some(what) = what {
                out.push(format!("Watch {} ({}): {what}", w.id, w.note));
                done.push(w.id);
            }
        }
        if !done.is_empty() {
            self.list.retain(|w| !done.contains(&w.id));
            self.save();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::super::notes::tests::card;
    use super::*;
    use crate::model::{PullRequest, State};
    use serde_json::json;

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn a_session_watch_fires_once_on_its_next_change_and_survives_a_reload() {
        let d = dir();
        let path = d.path().join("watches.json");
        let mut w = Watches::load(path.clone());
        let id = w.add(&json!({"kind":"watch","session":"api-fix","note":"tell the user"}), &[card("1", "api-fix", State::Working)]).unwrap();
        let w2 = Watches::load(path);
        assert_eq!(w2.list().len(), 1);
        let fired = w.fired(&[card("1", "api-fix", State::Working)], &[card("1", "api-fix", State::Completed)]);
        assert_eq!(fired, vec![format!("Watch {id} (tell the user): api-fix moved to completed")]);
        assert!(w.fired(&[card("1", "api-fix", State::Completed)], &[card("1", "api-fix", State::Working)]).is_empty());
    }

    #[test]
    fn a_watch_on_an_unknown_session_is_refused() {
        let d = dir();
        assert!(Watches::load(d.path().join("w.json")).add(&json!({"kind":"watch","session":"nope","note":""}), &[]).is_err());
    }

    #[test]
    fn a_session_watch_fires_when_the_card_disappears() {
        let d = dir();
        let mut w = Watches::load(d.path().join("w.json"));
        let cards = [card("1", "api-fix", State::Working)];
        let id = w.add(&json!({"kind":"watch","session":"API-fix","note":"n"}), &cards).unwrap();
        assert_eq!(w.fired(&cards, &[]), vec![format!("Watch {id} (n): api-fix ended")]);
        assert!(w.list().is_empty());
    }

    #[test]
    fn a_pr_watch_fires_when_the_pr_state_changes() {
        let d = dir();
        let mut w = Watches::load(d.path().join("w.json"));
        let id = w.add(&json!({"kind":"watch","pr":"Org/bedrock#451","note":"merge news"}), &[]).unwrap();
        let with = |state: &str| {
            let mut c = card("1", "api-fix", State::Working);
            c.pr = Some(PullRequest { number: 451, url: "https://github.com/Org/bedrock/pull/451".into(), state: state.into(), forge: Default::default() });
            c
        };
        assert!(w.fired(&[with("open")], &[with("open")]).is_empty());
        assert_eq!(w.fired(&[with("open")], &[with("merged")]), vec![format!("Watch {id} (merge news): Org/bedrock#451 moved to merged")]);
    }

    #[test]
    fn a_corrupt_file_loads_empty_and_ids_do_not_repeat_after_a_reload() {
        let d = dir();
        let path = d.path().join("w.json");
        std::fs::write(&path, "{nope").unwrap();
        assert!(Watches::load(path.clone()).list().is_empty());
        let mut w = Watches::load(path.clone());
        let a = w.add(&json!({"kind":"watch","pr":"o/r#1","note":""}), &[]).unwrap();
        let b = Watches::load(path).add(&json!({"kind":"watch","pr":"o/r#2","note":""}), &[]).unwrap();
        assert!(b > a);
    }
}
