//! Work items: the issues of a forge (or a custom tool), in one shape.

pub mod custom;
pub mod github;
pub mod gitlab;
pub mod pick;
pub mod start;
pub mod started;

use crate::config::{CustomTool, GitProvider, WorkItemsConfig};
use crate::forge::{Forge, ForgeSite, Http};
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

/// One issue as read on its own for Start: the item, its assignees' ids
/// and every label name as the issue spells it, status labels included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub item: WorkItem,
    pub assignee_ids: Vec<u64>,
    pub labels: Vec<String>,
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

/// One line above the Work Items tab's areas.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceError {
    pub source: String,
    pub message: String,
}

/// One source's answer to "which issues do the rules return": a forge or a custom tool.
pub struct Fetched {
    pub source: String,
    pub label: String,
    pub result: Result<Vec<WorkItem>, String>,
}

/// Asks every forge whose Work Items are on and every enabled tool with a
/// list command, at once. The app passes `&custom::list` as `list_tool`.
pub fn fetch(forges: &[Forge], cfg: &WorkItemsConfig, http: &dyn Http, sites: &[ForgeSite], list_tool: &(dyn Fn(&CustomTool, &[ForgeSite]) -> Result<Vec<WorkItem>, String> + Sync)) -> Vec<Fetched> {
    std::thread::scope(|s| {
        let mut handles: Vec<(String, String, _)> = Vec::new();
        for f in forges.iter().filter(|f| cfg.for_forge(f.site.provider).enabled) {
            let items = cfg.for_forge(f.site.provider);
            let ask = move || match f.site.provider {
                GitProvider::Github => github::list(http, f, items),
                GitProvider::Gitlab => gitlab::list(http, f, items),
            };
            handles.push((source_of(f.site.provider).to_string(), f.site.provider.label().to_string(), s.spawn(ask)));
        }
        for t in cfg.custom.iter().filter(|t| t.enabled && !t.list_command.trim().is_empty()) {
            handles.push((t.id.clone(), t.label().to_string(), s.spawn(move || list_tool(t, sites))));
        }
        handles
            .into_iter()
            .map(|(source, label, h)| Fetched { source, label, result: h.join().unwrap_or_else(|_| Err("the check stopped unexpectedly".into())) })
            .collect()
    })
}

/// The new list and the error lines: a source that answered replaces its
/// items, one that failed keeps its last ones and adds a line, and one that
/// was not asked drops out. `errors` holds the lines for services that were
/// not asked.
pub fn combine(previous: &[WorkItem], fetched: Vec<Fetched>, mut errors: Vec<SourceError>) -> (Vec<WorkItem>, Vec<SourceError>) {
    let mut lists = Vec::new();
    for f in fetched {
        match f.result {
            Ok(items) => lists.push(items),
            Err(e) => {
                lists.push(previous.iter().filter(|i| i.source == f.source).cloned().collect());
                let message = if f.source == "github" || f.source == "gitlab" { format!("{} issues check failed: {e}", f.label) } else { format!("{}: {e}", f.label) };
                errors.push(SourceError { source: f.source, message });
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
        let fetched = vec![
            Fetched { source: "github".into(), label: "GitHub".into(), result: Err("could not reach github.com: timeout".into()) },
            Fetched { source: "gitlab".into(), label: "GitLab".into(), result: Ok(vec![]) },
        ];
        let (items, errors) = combine(&[gh.clone(), gl], fetched, vec![]);
        assert_eq!(items, vec![gh]);
        assert_eq!(errors[0].message, "GitHub issues check failed: could not reach github.com: timeout");
    }

    #[test]
    fn a_failed_tool_keeps_its_last_items_and_names_itself() {
        let old = WorkItem { key: "custom-1:ENG-1".into(), source: "custom-1".into(), ..test_item() };
        let fetched = vec![Fetched { source: "custom-1".into(), label: "Linear (mine-only)".into(), result: Err("the command exited with status 1".into()) }];
        let (items, errors) = combine(&[old.clone()], fetched, vec![]);
        assert_eq!(items, vec![old]);
        assert_eq!(errors, vec![SourceError { source: "custom-1".into(), message: "Linear (mine-only): the command exited with status 1".into() }]);
    }

    #[test]
    fn two_tools_with_the_same_ids_keep_their_own_items() {
        let item = |key: &str, source: &str| WorkItem { key: key.into(), source: source.into(), ..test_item() };
        let out = merge(vec![vec![item("custom-1:ENG-1", "custom-1")], vec![item("custom-2:ENG-1", "custom-2")]]);
        assert_eq!(out.len(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn fetch_asks_the_enabled_tools_with_a_command() {
        use crate::forge::fake::FakeHttp;
        let tool = |id: &str, enabled: bool, cmd: &str| CustomTool { id: id.into(), name: id.into(), enabled, list_command: cmd.into(), ..Default::default() };
        let cfg = WorkItemsConfig {
            custom: vec![
                tool("custom-1", true, r#"echo '{"items":[{"id":"A-1","title":"t","url":"https://x.test/1"}]}'"#),
                tool("custom-2", false, "echo nope"),
                tool("custom-3", true, "  "),
            ],
            ..Default::default()
        };
        let http = FakeHttp::new(vec![]);
        let fetched = fetch(&[], &cfg, &http, &[], &|t, s| custom::list_with(t, s, &|c| vec!["sh".into(), "-c".into(), c.into()]));
        assert_eq!(fetched.len(), 1);
        assert_eq!(fetched[0].source, "custom-1");
        let items = fetched[0].result.as_ref().unwrap();
        assert_eq!(items.len(), 1);
        assert!(items[0].id.contains("A-1") || items[0].key.contains("A-1"), "{:?}", items[0]);
    }
}
