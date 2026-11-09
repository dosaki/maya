//! GitHub's issues: one search per rule.

use super::{merge, status_of, ItemRepo, WorkItem};
use crate::config::{Assignee, ForgeItems, GitProvider, ItemRule};
use crate::forge::{encode, get_json_as, Forge, Http, Need};
use serde_json::Value;

/// The search for a rule: open issues, by assignee, labels and repos.
pub fn search_query(rule: &ItemRule) -> String {
    let mut terms = vec!["is:issue".to_string(), "is:open".to_string()];
    match &rule.assignee {
        Assignee::Me => terms.push("assignee:@me".into()),
        Assignee::Anyone => {}
        Assignee::User { login } => terms.push(format!("assignee:{login}")),
    }
    terms.extend(rule.labels.iter().map(|l| format!("label:\"{}\"", l.replace('"', ""))));
    terms.extend(rule.repos.iter().map(|r| format!("repo:{r}")));
    terms.join(" ")
}

/// An issue of a search reply; None for a pull request or a malformed node.
pub fn parse_issue(v: &Value, cfg: &ForgeItems) -> Option<WorkItem> {
    if v.get("pull_request").is_some() {
        return None;
    }
    let number = v["number"].as_u64()?;
    let path = v["repository_url"].as_str()?.split("/repos/").nth(1)?.to_string();
    let id = format!("{path}#{number}");
    let names = |key: &str, field: &str| -> Vec<String> { v[key].as_array().map(|a| a.iter().filter_map(|x| x[field].as_str().map(str::to_string)).collect()).unwrap_or_default() };
    let (status, labels) = status_of(&names("labels", "name"), &cfg.status_labels);
    Some(WorkItem {
        key: format!("github:{id}"),
        source: "github".into(),
        number: Some(number),
        title: v["title"].as_str()?.to_string(),
        url: v["html_url"].as_str()?.to_string(),
        description: v["body"].as_str().unwrap_or_default().to_string(),
        status,
        labels,
        assignees: names("assignees", "login"),
        author: v["user"]["login"].as_str().unwrap_or_default().to_string(),
        updated_at: v["updated_at"].as_str().unwrap_or_default().to_string(),
        tracker: cfg.tracker_repos.iter().any(|r| r.eq_ignore_ascii_case(&path)),
        repo: Some(ItemRepo { forge: GitProvider::Github, path }),
        id,
    })
}

pub fn parse_search(v: &Value, cfg: &ForgeItems) -> Vec<WorkItem> {
    v["items"].as_array().map(|a| a.iter().filter_map(|i| parse_issue(i, cfg)).collect()).unwrap_or_default()
}

/// The issues of every rule, merged.
pub fn list(http: &dyn Http, forge: &Forge, cfg: &ForgeItems) -> Result<Vec<WorkItem>, String> {
    let mut lists = Vec::new();
    for rule in &cfg.rules {
        let url = format!("{}/search/issues?q={}&sort=updated&order=desc&per_page=50", forge.site.api_base(), encode(&search_query(rule)));
        lists.push(parse_search(&get_json_as(http, forge, &url, Need::Issues)?, cfg));
    }
    Ok(merge(lists))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GitConnection, WorkItemsConfig};
    use crate::forge::fake::FakeHttp;
    use crate::forge::ForgeSite;

    #[test]
    fn a_rule_becomes_one_search_with_quoted_terms() {
        let r = |assignee, labels: &[&str], repos: &[&str]| ItemRule { assignee, labels: labels.iter().map(|s| s.to_string()).collect(), repos: repos.iter().map(|s| s.to_string()).collect() };
        assert_eq!(search_query(&r(Assignee::Me, &[], &[])), "is:issue is:open assignee:@me");
        assert_eq!(
            search_query(&r(Assignee::Anyone, &["in progress", "bug"], &["acme/api", "acme/web"])),
            r#"is:issue is:open label:"in progress" label:"bug" repo:acme/api repo:acme/web"#
        );
        assert_eq!(search_query(&r(Assignee::User { login: "maria".into() }, &[r#"say "hi""#], &[])), r#"is:issue is:open assignee:maria label:"say hi""#, "a quote inside is dropped");
    }

    #[test]
    fn the_search_reply_becomes_items_and_skips_pull_requests() {
        let v: Value = serde_json::from_str(
            r#"{"items":[
      {"number":412,"title":"Login times out","html_url":"https://github.com/acme/api/issues/412","body":"Steps…","updated_at":"2026-11-08T10:00:00Z",
       "repository_url":"https://api.github.com/repos/acme/api","user":{"login":"jane"},
       "labels":[{"name":"bug"},{"name":"todo"}],"assignees":[{"login":"tiago"}]},
      {"number":7,"title":"A PR","html_url":"https://github.com/acme/api/pull/7","pull_request":{},"repository_url":"https://api.github.com/repos/acme/api","labels":[],"assignees":[],"user":{"login":"x"}}
    ]}"#,
        )
        .unwrap();
        let cfg = ForgeItems { status_labels: vec!["todo".into()], tracker_repos: vec!["acme/api".into()], ..WorkItemsConfig::default().github };
        let items = parse_search(&v, &cfg);
        assert_eq!(items.len(), 1);
        let i = &items[0];
        assert_eq!((i.key.as_str(), i.source.as_str(), i.id.as_str(), i.number), ("github:acme/api#412", "github", "acme/api#412", Some(412)));
        assert_eq!((i.status.as_deref(), i.labels.clone(), i.assignees.clone(), i.author.as_str()), (Some("todo"), vec!["bug".to_string()], vec!["tiago".to_string()], "jane"));
        assert_eq!(i.repo, Some(ItemRepo { forge: GitProvider::Github, path: "acme/api".into() }));
        assert!(i.tracker, "acme/api is tracker-only");
        assert_eq!(i.description, "Steps…");
    }

    #[test]
    fn list_asks_once_per_rule_and_names_the_issues_permission_on_refusal() {
        let f = Forge { site: ForgeSite::of(&GitConnection::default_for(GitProvider::Github)), token: "ghp_x".into() };
        let mut cfg = WorkItemsConfig::default().github;
        cfg.rules.push(ItemRule { assignee: Assignee::Anyone, labels: vec![], repos: vec!["acme/api".into()] });
        let http = FakeHttp::new(vec![("/search/issues", 200, r#"{"items":[]}"#)]);
        assert!(list(&http, &f, &cfg).unwrap().is_empty());
        let seen = http.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(seen[1].0.starts_with("https://api.github.com/search/issues?q=is%3Aissue%20is%3Aopen%20repo%3Aacme%2Fapi&sort=updated"), "{}", seen[1].0);
        let refused = FakeHttp::new(vec![("/search/issues", 403, "{}")]);
        assert_eq!(list(&refused, &f, &cfg).unwrap_err(), "the token was refused (403). It needs the repo scope (fine-grained: Issues read).");
    }
}
