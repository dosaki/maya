//! Work items: the issues of a forge (or a custom tool), in one shape.

pub mod github;
pub mod gitlab;
pub mod pick;
pub mod start;
pub mod started;

use crate::config::{GitProvider, WorkItemsConfig};
use crate::forge::{Forge, ForgeError, Http};
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

/// The labels without any status label, plus the new status.
pub fn labels_after_status(current: &[String], status_labels: &[String], new: &str) -> Vec<String> {
    let mut out: Vec<String> = current.iter().filter(|l| !status_labels.iter().any(|s| l.eq_ignore_ascii_case(s))).cloned().collect();
    out.push(new.to_string());
    out
}

/// The lists as one, one item per key (the first seen), newest `updated_at` first.
pub fn merge(lists: Vec<Vec<WorkItem>>) -> Vec<WorkItem> {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<WorkItem> = lists.into_iter().flatten().filter(|i| seen.insert(i.key.clone())).collect();
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    out
}

/// The `source` of a forge's items: "github" or "gitlab".
pub fn source_of(forge: GitProvider) -> &'static str {
    match forge {
        GitProvider::Github => "github",
        GitProvider::Gitlab => "gitlab",
    }
}

/// One service's answer to "which issues do the rules return".
pub struct Fetched {
    pub forge: GitProvider,
    pub result: Result<Vec<WorkItem>, String>,
}

/// Asks every forge whose Work Items are on, at once.
pub fn fetch(forges: &[Forge], cfg: &WorkItemsConfig, http: &dyn Http) -> Vec<Fetched> {
    std::thread::scope(|s| {
        let handles: Vec<_> = forges
            .iter()
            .filter(|f| cfg.for_forge(f.site.provider).enabled)
            .map(|f| {
                let items = cfg.for_forge(f.site.provider);
                let ask = move || match f.site.provider {
                    GitProvider::Github => github::list(http, f, items),
                    GitProvider::Gitlab => gitlab::list(http, f, items),
                };
                (f.site.provider, s.spawn(ask))
            })
            .collect();
        handles.into_iter().map(|(forge, h)| Fetched { forge, result: h.join().unwrap_or_else(|_| Err("the check stopped unexpectedly".into())) }).collect()
    })
}

/// The new list and the error lines: a service that answered replaces its
/// items, one that failed keeps its last ones and adds a line, and one that
/// was not asked drops out. `errors` holds the lines for services that were
/// not asked.
pub fn combine(previous: &[WorkItem], fetched: Vec<Fetched>, mut errors: Vec<ForgeError>) -> (Vec<WorkItem>, Vec<ForgeError>) {
    let mut lists = Vec::new();
    for f in fetched {
        match f.result {
            Ok(items) => lists.push(items),
            Err(e) => {
                lists.push(previous.iter().filter(|i| i.source == source_of(f.forge)).cloned().collect());
                errors.push(ForgeError { forge: f.forge, message: format!("{} issues check failed: {e}", f.forge.label()) });
            }
        }
    }
    (merge(lists), errors)
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
    fn a_new_status_replaces_every_other_status_and_keeps_the_rest() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(labels_after_status(&s(&["bug", "todo", "In Review"]), &s(&["todo", "in progress", "in review"]), "in progress"), s(&["bug", "in progress"]));
    }

    #[test]
    fn merge_keeps_one_of_each_newest_first() {
        let item = |key: &str, at: &str| WorkItem { key: key.into(), updated_at: at.into(), ..test_item() };
        let out = merge(vec![vec![item("github:a#1", "2026-11-01"), item("github:a#2", "2026-11-03")], vec![item("github:a#1", "2026-11-01")]]);
        assert_eq!(out.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(), vec!["github:a#2", "github:a#1"]);
    }

    #[test]
    fn a_failed_service_keeps_its_last_items_and_says_so() {
        let gh = WorkItem { key: "github:a/b#1".into(), source: "github".into(), ..test_item() };
        let gl = WorkItem { key: "gitlab:g/r#2".into(), source: "gitlab".into(), ..test_item() };
        let fetched = vec![Fetched { forge: GitProvider::Github, result: Err("could not reach github.com: timeout".into()) }, Fetched { forge: GitProvider::Gitlab, result: Ok(vec![]) }];
        let (items, errors) = combine(&[gh.clone(), gl], fetched, vec![]);
        assert_eq!(items, vec![gh]);
        assert_eq!(errors[0].message, "GitHub issues check failed: could not reach github.com: timeout");
    }
}
