//! GitLab's issues: one call per rule and project.

use super::{merge, status_of, ItemRepo, WorkItem};
use crate::config::{Assignee, ForgeItems, GitProvider, ItemRule};
use crate::forge::{encode, get_json_as, send_json_as, Forge, Http, Me, Need};
use serde_json::Value;

/// The calls for a rule: open issues, one per project, or one across GitLab when it names none.
pub fn list_urls(api_base: &str, rule: &ItemRule) -> Vec<String> {
    let query = "state=opened&order_by=updated_at&sort=desc&per_page=50";
    let labels = if rule.labels.is_empty() { String::new() } else { format!("&labels={}", rule.labels.iter().map(|l| encode(l)).collect::<Vec<_>>().join(",")) };
    if rule.repos.is_empty() {
        let who = match &rule.assignee {
            Assignee::Me => "&scope=assigned_to_me".to_string(),
            Assignee::User { login } => format!("&scope=all&assignee_username={}", encode(login)),
            Assignee::Anyone => String::new(),
        };
        return vec![format!("{api_base}/issues?{query}{who}{labels}")];
    }
    let who = match &rule.assignee {
        Assignee::Me => "&scope=assigned_to_me".to_string(),
        Assignee::User { login } => format!("&assignee_username={}", encode(login)),
        Assignee::Anyone => String::new(),
    };
    rule.repos.iter().map(|r| format!("{api_base}/projects/{}/issues?{query}{who}{labels}", encode(r))).collect()
}

/// An issue of a list reply; None for a malformed node.
pub fn parse_issue(v: &Value, cfg: &ForgeItems) -> Option<WorkItem> {
    let number = v["iid"].as_u64()?;
    let id = v["references"]["full"].as_str()?.to_string();
    let path = id.rsplit_once('#')?.0.to_string();
    let names = |key: &str, field: &str| -> Vec<String> { v[key].as_array().map(|a| a.iter().filter_map(|x| x[field].as_str().map(str::to_string)).collect()).unwrap_or_default() };
    let all_labels: Vec<String> = v["labels"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let (status, labels) = status_of(&all_labels, &cfg.status_labels);
    Some(WorkItem {
        key: format!("gitlab:{id}"),
        source: "gitlab".into(),
        number: Some(number),
        title: v["title"].as_str()?.to_string(),
        url: v["web_url"].as_str()?.to_string(),
        description: v["description"].as_str().unwrap_or_default().to_string(),
        status,
        labels,
        assignees: names("assignees", "username"),
        author: v["author"]["username"].as_str().unwrap_or_default().to_string(),
        updated_at: v["updated_at"].as_str().unwrap_or_default().to_string(),
        tracker: cfg.tracker_repos.iter().any(|r| r.eq_ignore_ascii_case(&path)),
        repo: Some(ItemRepo { forge: GitProvider::Gitlab, path }),
        id,
    })
}

/// The issues of every rule, merged; the first failure is the service's.
pub fn list(http: &dyn Http, forge: &Forge, cfg: &ForgeItems) -> Result<Vec<WorkItem>, String> {
    let mut lists = Vec::new();
    for rule in &cfg.rules {
        for url in list_urls(&forge.site.api_base(), rule) {
            let reply = get_json_as(http, forge, &url, Need::Issues)?;
            lists.push(reply.as_array().map(|a| a.iter().filter_map(|i| parse_issue(i, cfg)).collect()).unwrap_or_default());
        }
    }
    Ok(merge(lists))
}

/// The open issue `number` of `repo` and its assignees' ids; None when it is closed.
pub fn get(http: &dyn Http, forge: &Forge, repo: &str, number: u64, cfg: &ForgeItems) -> Result<Option<(WorkItem, Vec<u64>)>, String> {
    let v = get_json_as(http, forge, &issue_url(forge, repo, number), Need::Issues)?;
    if v["state"].as_str() == Some("closed") {
        return Ok(None);
    }
    let item = parse_issue(&v, cfg).ok_or("GitLab's reply was not an issue.")?;
    Ok(Some((item, assignee_ids(&v))))
}

fn issue_url(forge: &Forge, repo: &str, number: u64) -> String {
    format!("{}/projects/{}/issues/{number}", forge.site.api_base(), encode(repo))
}

fn assignee_ids(v: &Value) -> Vec<u64> {
    v["assignees"].as_array().map(|a| a.iter().filter_map(|x| x["id"].as_u64()).collect()).unwrap_or_default()
}

/// Assigns the issue to `me` beside `assignee_ids` and sets the start status
/// (if any). A tier with one assignee keeps only one of them, so when `me`
/// is missing from the reply a second call makes `me` the assignee.
#[allow(clippy::too_many_arguments)]
pub fn take(http: &dyn Http, forge: &Forge, repo: &str, number: u64, me: &Me, assignee_ids_now: &[u64], all_labels: &[String], cfg: &ForgeItems) -> Result<(), String> {
    let mut ids = assignee_ids_now.to_vec();
    if !ids.contains(&me.id) {
        ids.push(me.id);
    }
    let mut body = serde_json::json!({ "assignee_ids": ids });
    if let Some(status) = &cfg.start_status {
        body["add_labels"] = serde_json::json!(status);
        let others: Vec<&str> = all_labels.iter().filter(|l| cfg.status_labels.iter().any(|s| l.eq_ignore_ascii_case(s)) && !l.eq_ignore_ascii_case(status)).map(String::as_str).collect();
        if !others.is_empty() {
            body["remove_labels"] = serde_json::json!(others.join(","));
        }
    }
    let url = issue_url(forge, repo, number);
    let reply = send_json_as(http, forge, "PUT", &url, &body, Need::IssueWrite)?;
    let kept: Vec<u64> = serde_json::from_str::<Value>(&reply).map(|v| assignee_ids(&v)).unwrap_or_default();
    if !kept.contains(&me.id) {
        send_json_as(http, forge, "PUT", &url, &serde_json::json!({ "assignee_ids": [me.id] }), Need::IssueWrite)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GitConnection, WorkItemsConfig};
    use crate::forge::fake::FakeHttp;
    use crate::forge::ForgeSite;

    fn gitlab_forge() -> Forge {
        let c = GitConnection { enabled: true, base_url: Some("https://gitlab.acme.io".into()), ..GitConnection::default_for(GitProvider::Gitlab) };
        Forge { site: ForgeSite::of(&c), token: "glpat".into() }
    }

    #[test]
    fn take_replaces_the_assignee_when_the_tier_allows_only_one() {
        let http = FakeHttp::new(vec![("/issues/17", 200, r#"{"assignees":[{"id":5}]}"#)]);
        let cfg = ForgeItems { status_labels: vec!["status::todo".into(), "status::doing".into()], start_status: Some("status::doing".into()), ..WorkItemsConfig::default().gitlab };
        take(&http, &gitlab_forge(), "grp/r", 17, &Me { login: "tiago".into(), id: 9 }, &[5], &["status::todo".to_string(), "backend".to_string()], &cfg).unwrap();
        let seen = http.seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "the first PUT kept only the old assignee, so a second one replaces it");
        let first: Value = serde_json::from_str(seen[0].2.as_deref().unwrap()).unwrap();
        assert_eq!(first, serde_json::json!({"assignee_ids": [5, 9], "add_labels": "status::doing", "remove_labels": "status::todo"}));
        assert_eq!(seen[1].2.as_deref(), Some(r#"{"assignee_ids":[9]}"#));
        assert!(seen[0].0.ends_with("/projects/grp%2Fr/issues/17") && seen[0].3.as_deref() == Some("PUT"));
    }

    #[test]
    fn take_makes_one_call_when_me_is_among_the_assignees_and_no_label_is_removed() {
        let http = FakeHttp::new(vec![("/issues/3", 200, r#"{"assignees":[{"id":5},{"id":9}]}"#)]);
        let cfg = ForgeItems { status_labels: vec!["doing".into()], start_status: Some("doing".into()), ..WorkItemsConfig::default().gitlab };
        take(&http, &gitlab_forge(), "grp/r", 3, &Me { login: "tiago".into(), id: 9 }, &[5], &[], &cfg).unwrap();
        let seen = http.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        let body: Value = serde_json::from_str(seen[0].2.as_deref().unwrap()).unwrap();
        assert_eq!(body, serde_json::json!({"assignee_ids": [5, 9], "add_labels": "doing"}), "no remove_labels key");
    }

    #[test]
    fn a_read_only_gitlab_token_is_told_it_needs_api() {
        let http = FakeHttp::new(vec![("/issues/1", 403, "{}")]);
        let e = take(&http, &gitlab_forge(), "a/b", 1, &Me { login: "t".into(), id: 1 }, &[], &[], &WorkItemsConfig::default().gitlab).unwrap_err();
        assert_eq!(e, "the token was refused (403). It needs the api scope.");
    }

    #[test]
    fn get_says_none_for_a_closed_issue_and_lists_assignee_ids_for_an_open_one() {
        let body = |state: &str| format!(r#"{{"iid":4,"state":"{state}","title":"t","web_url":"u","references":{{"full":"grp/r#4"}},"labels":[],"assignees":[{{"id":5,"username":"m"}}],"author":{{"username":"a"}}}}"#);
        let closed = FakeHttp::new(vec![("/projects/grp%2Fr/issues/4", 200, &body("closed"))]);
        assert_eq!(get(&closed, &gitlab_forge(), "grp/r", 4, &WorkItemsConfig::default().gitlab).unwrap(), None);
        let open = FakeHttp::new(vec![("/projects/grp%2Fr/issues/4", 200, &body("opened"))]);
        let (item, ids) = get(&open, &gitlab_forge(), "grp/r", 4, &WorkItemsConfig::default().gitlab).unwrap().unwrap();
        assert_eq!((item.id.as_str(), ids), ("grp/r#4", vec![5]));
    }

    #[test]
    fn a_rule_becomes_one_call_per_project_with_encoded_paths() {
        let api = "https://gitlab.acme.io/api/v4";
        let me = ItemRule { assignee: Assignee::Me, labels: vec![], repos: vec![] };
        assert_eq!(list_urls(api, &me), vec![format!("{api}/issues?state=opened&order_by=updated_at&sort=desc&per_page=50&scope=assigned_to_me")]);
        let anyone = ItemRule { assignee: Assignee::Anyone, labels: vec!["status::todo".into(), "a b".into()], repos: vec!["grp/sub/r".into(), "grp/x".into()] };
        assert_eq!(list_urls(api, &anyone), vec![
            format!("{api}/projects/grp%2Fsub%2Fr/issues?state=opened&order_by=updated_at&sort=desc&per_page=50&labels=status%3A%3Atodo,a%20b"),
            format!("{api}/projects/grp%2Fx/issues?state=opened&order_by=updated_at&sort=desc&per_page=50&labels=status%3A%3Atodo,a%20b"),
        ]);
        let maria = ItemRule { assignee: Assignee::User { login: "maria".into() }, labels: vec![], repos: vec![] };
        assert_eq!(list_urls(api, &maria), vec![format!("{api}/issues?state=opened&order_by=updated_at&sort=desc&per_page=50&scope=all&assignee_username=maria")]);
    }

    #[test]
    fn an_issue_takes_its_repo_from_the_full_reference() {
        let v: Value = serde_json::from_str(r#"{"iid":17,"title":"VAT missing","description":null,"web_url":"https://gitlab.acme.io/grp/sub/billing/-/issues/17",
          "updated_at":"2026-11-07T09:00:00Z","labels":["backend","status::todo"],"assignees":[{"username":"tiago"}],"author":{"username":"ana"},
          "references":{"full":"grp/sub/billing#17"}}"#).unwrap();
        let cfg = ForgeItems { status_labels: vec!["status::todo".into(), "status::doing".into()], ..WorkItemsConfig::default().gitlab };
        let i = parse_issue(&v, &cfg).unwrap();
        assert_eq!((i.key.as_str(), i.id.as_str(), i.number), ("gitlab:grp/sub/billing#17", "grp/sub/billing#17", Some(17)));
        assert_eq!(i.repo, Some(ItemRepo { forge: GitProvider::Gitlab, path: "grp/sub/billing".into() }));
        assert_eq!((i.status.as_deref(), i.labels.clone(), i.description.as_str()), (Some("status::todo"), vec!["backend".to_string()], ""));
    }

    #[test]
    fn list_merges_the_projects_and_one_failure_fails_the_service() {
        let c = GitConnection { enabled: true, base_url: Some("https://gitlab.acme.io".into()), ..GitConnection::default_for(GitProvider::Gitlab) };
        let f = Forge { site: ForgeSite::of(&c), token: "glpat".into() };
        let cfg = ForgeItems { rules: vec![ItemRule { assignee: Assignee::Anyone, labels: vec![], repos: vec!["grp/a".into(), "grp/b".into()] }], ..WorkItemsConfig::default().gitlab };
        let http = FakeHttp::new(vec![("grp%2Fa", 200, "[]"), ("grp%2Fb", 404, r#"{"message":"404 Project Not Found"}"#)]);
        assert_eq!(list(&http, &f, &cfg).unwrap_err(), "404: 404 Project Not Found");
    }
}
