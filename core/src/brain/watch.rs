//! Watches: a session or a pull request the brain asked to hear about. Each is one-shot.

use std::path::PathBuf;

use serde_json::Value;

use super::notes::state_note;
use crate::model::{Card, PullRequest, State};

/// Added to a watch that fired because its session finished or needs a
/// decision: Maya's built-in announcement has said so already.
pub const ANNOUNCED: &str = "Maya has already announced this; do not repeat it; only act if the watch asked for an action.";

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

/// Splits "repo#n" or "repo!n" into the sigil, the repo path (lowercase) and the number.
fn split_ref(reference: &str) -> Option<(char, String, u64)> {
    let r = reference.trim();
    let at = r.rfind(['#', '!'])?;
    let number = r[at + 1..].trim().parse().ok()?;
    Some((r[at..].chars().next()?, r[..at].trim().to_lowercase(), number))
}

/// A card carries only a PR number and url, not its repo, so a reference is
/// matched against the url path: the repo part must equal the segment(s)
/// right before `/pull/<n>` (`#`) or `/-/merge_requests/<n>` (`!`).
fn pr_matches(pr: &PullRequest, reference: &str) -> bool {
    let Some((sigil, repo, number)) = split_ref(reference) else { return false };
    let path = pr.url.split("://").last().unwrap_or("");
    let segs: Vec<String> = path.split('/').skip(1).filter(|s| !s.is_empty()).map(str::to_lowercase).collect();
    let (marker, back) = if sigil == '#' { ("pull", 0) } else { ("merge_requests", 1) };
    let Some(i) = segs.iter().position(|s| s == marker) else { return false };
    if segs.get(i + 1).and_then(|n| n.parse::<u64>().ok()) != Some(number) || pr.number != number {
        return false;
    }
    if back == 1 && (i == 0 || segs[i - 1] != "-") {
        return false;
    }
    let repo_segs = &segs[..i - back];
    let want: Vec<&str> = repo.split('/').filter(|s| !s.is_empty()).collect();
    want.is_empty() || (want.len() <= repo_segs.len() && repo_segs[repo_segs.len() - want.len()..].iter().map(String::as_str).eq(want.iter().copied()))
}

fn card_matches_pr(card: &Card, reference: &str) -> bool {
    card.pr.as_ref().map_or(false, |p| pr_matches(p, reference))
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
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
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
                    Some(a) => state_note(b, a).map(|n| match a.state {
                        State::Completed | State::Awaiting => format!("{n}. {ANNOUNCED}"),
                        _ => n,
                    }),
                    None => Some(format!("{} ended", b.name)),
                })
            } else if let Some(reference) = &w.pr {
                before.iter().find(|b| card_matches_pr(b, reference)).and_then(|b| {
                    let old = b.pr.as_ref()?;
                    match after.iter().find(|a| card_matches_pr(a, reference)) {
                        Some(a) => {
                            let new = a.pr.as_ref()?;
                            (old.state != new.state).then(|| format!("{reference} moved to {}", new.state))
                        }
                        None if !after.iter().any(|a| a.session_id == b.session_id) => {
                            Some(format!("{} ended; its PR {reference} was {}", b.name, old.state))
                        }
                        None => None,
                    }
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
    use crate::model::PullRequest;
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
        assert_eq!(fired, vec![format!("Watch {id} (tell the user): api-fix moved to completed. {ANNOUNCED}")]);
        assert!(w.fired(&[card("1", "api-fix", State::Completed)], &[card("1", "api-fix", State::Working)]).is_empty());
    }

    #[test]
    fn a_watch_fired_by_a_finish_or_a_decision_says_maya_announced_it_already() {
        let d = dir();
        let mut w = Watches::load(d.path().join("w.json"));
        let cards = [card("1", "api-fix", State::Working)];
        let id = w.add(&json!({"kind":"watch","session":"api-fix","note":"n"}), &cards).unwrap();
        assert_eq!(
            w.fired(&cards, &[card("1", "api-fix", State::Awaiting)]),
            vec![format!("Watch {id} (n): api-fix moved to awaiting. Maya has already announced this; do not repeat it; only act if the watch asked for an action.")]
        );
        let id = w.add(&json!({"kind":"watch","session":"api-fix","note":"n"}), &cards).unwrap();
        assert_eq!(w.fired(&cards, &[card("1", "api-fix", State::Idle)]), vec![format!("Watch {id} (n): api-fix moved to idle")], "nothing announced an idle session");
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

    fn pr_card(id: &str, url: &str, n: u64, state: &str) -> Card {
        let mut c = card(id, "api-fix", State::Working);
        c.pr = Some(PullRequest { number: n, url: url.into(), state: state.into(), forge: Default::default() });
        c
    }

    #[test]
    fn a_pr_reference_matches_the_url_path_exactly() {
        let gh = |u: &str, n| PullRequest { number: n, url: u.into(), state: "open".into(), forge: Default::default() };
        assert!(!pr_matches(&gh("https://github.com/o/rapid-api/pull/12", 12), "api#12"));
        assert!(pr_matches(&gh("https://github.com/o/api/pull/12", 12), "api#12"));
        assert!(!pr_matches(&gh("https://github.com/org/bedrock-fork/pull/3", 3), "org/bedrock#3"));
        assert!(pr_matches(&gh("https://github.com/Org/bedrock/pull/3", 3), "org/bedrock#3"));
        assert!(pr_matches(&gh("https://gitlab.com/g/proj/-/merge_requests/4", 4), "proj!4"));
        assert!(!pr_matches(&gh("https://github.com/g/proj/pull/4", 4), "proj!4"));
        assert!(!pr_matches(&gh("https://gitlab.com/g/proj/-/merge_requests/4", 4), "proj#4"));
    }

    #[test]
    fn a_pr_watch_fires_when_its_session_ends_or_the_pr_moves_to_another_card() {
        let d = dir();
        let mut w = Watches::load(d.path().join("w.json"));
        let url = "https://github.com/o/r/pull/7";
        let id = w.add(&json!({"kind":"watch","pr":"o/r#7","note":"n"}), &[]).unwrap();
        assert_eq!(w.fired(&[pr_card("1", url, 7, "open")], &[]), vec![format!("Watch {id} (n): api-fix ended; its PR o/r#7 was open")]);
        let id = w.add(&json!({"kind":"watch","pr":"o/r#7","note":"n"}), &[]).unwrap();
        assert_eq!(w.fired(&[pr_card("1", url, 7, "open")], &[pr_card("2", url, 7, "merged")]), vec![format!("Watch {id} (n): o/r#7 moved to merged")]);
    }

    #[test]
    fn saving_creates_the_parent_folder_and_remove_forgets_a_watch() {
        let d = dir();
        let path = d.path().join("brain").join("watches.json");
        let mut w = Watches::load(path.clone());
        let id = w.add(&json!({"kind":"watch","pr":"o/r#1","note":""}), &[]).unwrap();
        assert_eq!(Watches::load(path.clone()).list().len(), 1);
        assert!(!w.remove(id + 1));
        assert!(w.remove(id));
        assert!(w.list().is_empty());
        assert!(Watches::load(path).list().is_empty());
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
