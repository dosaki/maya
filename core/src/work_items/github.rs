//! GitHub's issues: one search per rule.

use super::{labels_after_status, merge, status_of, Issue, ItemRepo, WorkItem};
use crate::config::{Assignee, ForgeItems, GitProvider, ItemRule};
use crate::forge::{encode, get_json_as, send_json_as, Forge, Http, Me, Need};
use serde_json::Value;

/// The search for a rule: open issues outside archived repos, by assignee, labels and repos.
pub fn search_query(rule: &ItemRule) -> String {
    let mut terms = vec!["is:issue".to_string(), "is:open".to_string(), "archived:false".to_string()];
    match &rule.assignee {
        Assignee::Me => terms.push("assignee:@me".into()),
        Assignee::Anyone => {}
        Assignee::User { login } => terms.push(format!("assignee:{login}")),
    }
    terms.extend(rule.labels.iter().map(|l| format!("label:\"{}\"", l.replace('"', ""))));
    terms.extend(rule.repos.iter().map(|r| format!("repo:{r}")));
    terms.join(" ")
}

/// The `field` of each object in `v[key]`: `names(v, "labels", "name")`.
fn names(v: &Value, key: &str, field: &str) -> Vec<String> {
    v[key].as_array().map(|a| a.iter().filter_map(|x| x[field].as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// An issue of a search reply; None for a pull request or a malformed node.
pub fn parse_issue(v: &Value, cfg: &ForgeItems) -> Option<WorkItem> {
    if v.get("pull_request").is_some() {
        return None;
    }
    let number = v["number"].as_u64()?;
    let path = v["repository_url"].as_str()?.split("/repos/").nth(1)?.to_string();
    let id = format!("{path}#{number}");
    let (status, labels) = status_of(&names(v, "labels", "name"), &cfg.status_labels);
    Some(WorkItem {
        key: format!("github:{id}"),
        source: "github".into(),
        number: Some(number),
        title: v["title"].as_str()?.to_string(),
        url: v["html_url"].as_str()?.to_string(),
        description: v["body"].as_str().unwrap_or_default().to_string(),
        status,
        labels,
        assignees: names(v, "assignees", "login"),
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

/// The open issue `number` of `repo`; None when it is closed.
pub fn get(http: &dyn Http, forge: &Forge, repo: &str, number: u64, cfg: &ForgeItems) -> Result<Option<Issue>, String> {
    let v = get_json_as(http, forge, &format!("{}/repos/{repo}/issues/{number}", forge.site.api_base()), Need::Issues)?;
    if v.get("pull_request").is_some() {
        return Err("That is a pull request, not an issue.".into());
    }
    if v["state"].as_str() == Some("closed") {
        return Ok(None);
    }
    let item = parse_issue(&v, cfg).ok_or("GitHub's reply was not an issue.")?;
    let assignee_ids = v["assignees"].as_array().map(|a| a.iter().filter_map(|x| x["id"].as_u64()).collect()).unwrap_or_default();
    let labels = names(&v, "labels", "name");
    Ok(Some(Issue { item, assignee_ids, labels }))
}

/// Assigns the issue to `me` beside `assignee_logins`, and sets the start
/// status (if any) in place of the other status labels, in one call.
#[allow(clippy::too_many_arguments)]
pub fn take(http: &dyn Http, forge: &Forge, repo: &str, number: u64, me: &Me, assignee_logins: &[String], all_labels: &[String], cfg: &ForgeItems) -> Result<(), String> {
    let mut assignees: Vec<&str> = assignee_logins.iter().map(String::as_str).collect();
    if !assignees.contains(&me.login.as_str()) {
        assignees.push(&me.login);
    }
    let mut body = serde_json::json!({ "assignees": assignees });
    if let Some(status) = &cfg.start_status {
        body["labels"] = serde_json::json!(labels_after_status(all_labels, &cfg.status_labels, status));
    }
    let url = format!("{}/repos/{repo}/issues/{number}", forge.site.api_base());
    let reply = send_json_as(http, forge, "PATCH", &url, &body, Need::IssueWrite)?;
    // GitHub answers 200 and silently drops the assignees and labels a user
    // without triage rights may not set, so the reply is what counts.
    let v: Value = serde_json::from_str(&reply).map_err(|e| format!("GitHub's reply was not JSON: {e}"))?;
    if !names(&v, "assignees", "login").iter().any(|l| l.eq_ignore_ascii_case(&me.login)) {
        return Err("The issue was not assigned to you.".into());
    }
    if let Some(status) = &cfg.start_status {
        if !names(&v, "labels", "name").iter().any(|l| l.eq_ignore_ascii_case(status)) {
            return Err("The issue was assigned to you, but its status was not set.".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GitConnection, WorkItemsConfig};
    use crate::forge::fake::FakeHttp;
    use crate::forge::ForgeSite;

    fn github_forge() -> Forge {
        Forge { site: ForgeSite::of(&GitConnection::default_for(GitProvider::Github)), token: "ghp_x".into() }
    }

    #[test]
    fn take_adds_me_beside_the_others_and_sets_the_status_in_one_call() {
        let f = github_forge();
        let http = FakeHttp::new(vec![("/issues/412", 200, r#"{"assignees":[{"login":"maria"},{"login":"Tiago"}],"labels":[{"name":"bug"},{"name":"In Progress"}]}"#)]);
        let cfg = ForgeItems { status_labels: vec!["todo".into(), "in progress".into()], start_status: Some("in progress".into()), ..WorkItemsConfig::default().github };
        take(&http, &f, "acme/api", 412, &Me { login: "tiago".into(), id: 1 }, &["maria".to_string()], &["bug".to_string(), "todo".to_string()], &cfg).unwrap();
        let seen = http.seen.lock().unwrap();
        assert_eq!((seen[0].0.as_str(), seen[0].3.as_deref()), ("https://api.github.com/repos/acme/api/issues/412", Some("PATCH")));
        let body: Value = serde_json::from_str(seen[0].2.as_deref().unwrap()).unwrap();
        assert_eq!(body, serde_json::json!({"assignees": ["maria", "tiago"], "labels": ["bug", "in progress"]}));
    }

    #[test]
    fn take_without_a_start_status_leaves_the_labels_alone() {
        let http = FakeHttp::new(vec![("/issues/5", 200, r#"{"assignees":[{"login":"tiago"}],"labels":[]}"#)]);
        take(&http, &github_forge(), "a/b", 5, &Me { login: "tiago".into(), id: 1 }, &["tiago".to_string()], &[], &WorkItemsConfig::default().github).unwrap();
        let body: Value = serde_json::from_str(http.seen.lock().unwrap()[0].2.as_deref().unwrap()).unwrap();
        assert_eq!(body, serde_json::json!({"assignees": ["tiago"]}), "no labels key, and me is not listed twice");
    }

    #[test]
    fn take_errors_when_the_reply_leaves_me_out() {
        // GitHub answers 200 and drops what the user may not set.
        let http = FakeHttp::new(vec![("/issues/5", 200, r#"{"assignees":[{"login":"maria"}],"labels":[]}"#)]);
        let e = take(&http, &github_forge(), "a/b", 5, &Me { login: "tiago".into(), id: 1 }, &["maria".to_string()], &[], &WorkItemsConfig::default().github).unwrap_err();
        assert_eq!(e, "The issue was not assigned to you.");
    }

    #[test]
    fn take_errors_when_the_reply_lacks_the_start_status() {
        let http = FakeHttp::new(vec![("/issues/5", 200, r#"{"assignees":[{"login":"tiago"}],"labels":[{"name":"todo"}]}"#)]);
        let cfg = ForgeItems { status_labels: vec!["todo".into(), "in progress".into()], start_status: Some("in progress".into()), ..WorkItemsConfig::default().github };
        let e = take(&http, &github_forge(), "a/b", 5, &Me { login: "tiago".into(), id: 1 }, &[], &["todo".to_string()], &cfg).unwrap_err();
        assert_eq!(e, "The issue was assigned to you, but its status was not set.");
    }

    #[test]
    fn take_errors_on_a_reply_that_is_not_json() {
        let http = FakeHttp::new(vec![("/issues/5", 200, "<html>")]);
        let e = take(&http, &github_forge(), "a/b", 5, &Me { login: "tiago".into(), id: 1 }, &[], &[], &WorkItemsConfig::default().github).unwrap_err();
        assert!(e.starts_with("GitHub's reply was not JSON: "), "{e}");
    }

    #[test]
    fn a_read_only_token_is_told_it_needs_write() {
        let http = FakeHttp::new(vec![("/issues/1", 403, "{}")]);
        let e = take(&http, &github_forge(), "a/b", 1, &Me { login: "t".into(), id: 1 }, &[], &[], &WorkItemsConfig::default().github).unwrap_err();
        assert_eq!(e, "the token was refused (403). It needs the repo scope (fine-grained: Issues read and write).");
    }

    #[test]
    fn get_says_none_for_a_closed_issue() {
        let http = FakeHttp::new(vec![("/issues/9", 200, r#"{"number":9,"state":"closed","title":"t","html_url":"u","repository_url":"https://api.github.com/repos/a/b","labels":[],"assignees":[],"user":{"login":"x"}}"#)]);
        assert_eq!(get(&http, &github_forge(), "a/b", 9, &WorkItemsConfig::default().github).unwrap(), None);
    }

    #[test]
    fn get_returns_an_open_issue_with_its_assignee_ids_and_refuses_a_pull_request() {
        let open = r#"{"number":9,"state":"open","title":"t","html_url":"u","repository_url":"https://api.github.com/repos/a/b","labels":[{"name":"bug"},{"name":"Todo"}],"assignees":[{"login":"m","id":4},{"login":"n","id":6}],"user":{"login":"x"}}"#;
        let http = FakeHttp::new(vec![("/issues/9", 200, open), ("/issues/10", 200, r#"{"number":10,"state":"open","pull_request":{}}"#)]);
        let issue = get(&http, &github_forge(), "a/b", 9, &WorkItemsConfig::default().github).unwrap().unwrap();
        assert_eq!((issue.item.id.as_str(), issue.assignee_ids), ("a/b#9", vec![4, 6]));
        assert_eq!(issue.labels, vec!["bug", "Todo"]);
        assert_eq!(get(&http, &github_forge(), "a/b", 10, &WorkItemsConfig::default().github).unwrap_err(), "That is a pull request, not an issue.");
    }

    #[test]
    fn a_rule_becomes_one_search_with_quoted_terms() {
        let r = |assignee, labels: &[&str], repos: &[&str]| ItemRule { assignee, labels: labels.iter().map(|s| s.to_string()).collect(), repos: repos.iter().map(|s| s.to_string()).collect() };
        assert_eq!(search_query(&r(Assignee::Me, &[], &[])), "is:issue is:open archived:false assignee:@me");
        assert_eq!(
            search_query(&r(Assignee::Anyone, &["in progress", "bug"], &["acme/api", "acme/web"])),
            r#"is:issue is:open archived:false label:"in progress" label:"bug" repo:acme/api repo:acme/web"#
        );
        assert_eq!(search_query(&r(Assignee::User { login: "maria".into() }, &[r#"say "hi""#], &[])), r#"is:issue is:open archived:false assignee:maria label:"say hi""#, "a quote inside is dropped");
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
        assert!(seen[1].0.starts_with("https://api.github.com/search/issues?q=is%3Aissue%20is%3Aopen%20archived%3Afalse%20repo%3Aacme%2Fapi&sort=updated"), "{}", seen[1].0);
        let refused = FakeHttp::new(vec![("/search/issues", 403, "{}")]);
        assert_eq!(list(&refused, &f, &cfg).unwrap_err(), "the token was refused (403). It needs the repo scope (fine-grained: Issues read).");
    }
}
