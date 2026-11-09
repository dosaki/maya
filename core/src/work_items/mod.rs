//! Work items: the issues of a forge (or a custom tool), in one shape.

pub mod github;
pub mod gitlab;

use crate::config::GitProvider;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ItemRepo {
    pub forge: GitProvider,
    pub path: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkItem {
    /// `<source>:<id>`: unique across sources.
    pub key: String,
    /// "github" or "gitlab" (a custom tool's id later).
    pub source: String,
    /// As shown: `acme/api#412`.
    pub id: String,
    pub number: Option<u64>,
    pub title: String,
    pub url: String,
    pub description: String,
    pub status: Option<String>,
    /// The labels that are not the status.
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
    pub author: String,
    pub updated_at: String,
    pub repo: Option<ItemRepo>,
    /// The repo is tracker-only: Start asks Maya to pick the code's repository.
    pub tracker: bool,
}

/// The status (as spelled in `status_labels`: the first listed one the labels
/// carry, ignoring case) and the labels that are not a status label.
pub fn status_of(labels: &[String], status_labels: &[String]) -> (Option<String>, Vec<String>) {
    let is_status = |l: &String, s: &String| l.eq_ignore_ascii_case(s);
    let status = status_labels.iter().find(|s| labels.iter().any(|l| is_status(l, s))).cloned();
    let rest = labels.iter().filter(|l| !status_labels.iter().any(|s| is_status(l, s))).cloned().collect();
    (status, rest)
}

/// The lists as one, one item per key (the first seen), newest `updated_at` first.
pub fn merge(lists: Vec<Vec<WorkItem>>) -> Vec<WorkItem> {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<WorkItem> = lists.into_iter().flatten().filter(|i| seen.insert(i.key.clone())).collect();
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    out
}

#[cfg(test)]
pub(crate) fn test_item() -> WorkItem {
    WorkItem {
        key: String::new(),
        source: String::new(),
        id: String::new(),
        number: None,
        title: String::new(),
        url: String::new(),
        description: String::new(),
        status: None,
        labels: vec![],
        assignees: vec![],
        author: String::new(),
        updated_at: String::new(),
        repo: None,
        tracker: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_is_the_first_listed_label_it_carries() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(status_of(&s(&["bug", "In Progress", "todo"]), &s(&["todo", "in progress"])), (Some("todo".into()), s(&["bug"])));
        assert_eq!(status_of(&s(&["bug"]), &s(&["todo"])), (None, s(&["bug"])));
        assert_eq!(status_of(&s(&["status::doing"]), &s(&[])), (None, s(&["status::doing"])), "only listed labels are statuses");
    }

    #[test]
    fn merge_keeps_one_of_each_newest_first() {
        let item = |key: &str, at: &str| WorkItem { key: key.into(), updated_at: at.into(), ..test_item() };
        let out = merge(vec![vec![item("github:a#1", "2026-11-01"), item("github:a#2", "2026-11-03")], vec![item("github:a#1", "2026-11-01")]]);
        assert_eq!(out.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(), vec!["github:a#2", "github:a#1"]);
    }
}
