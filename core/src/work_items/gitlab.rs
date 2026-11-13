//! GitLab's issues: one call per rule and project.

use super::{merge, status_of, Issue, ItemRepo, WorkItem};
use crate::config::{Assignee, ForgeItems, GitProvider, ItemRule};
use crate::forge::{encode, get_json_as, send_json_as, Forge, Http, Me, Need};
use serde_json::Value;

/// The calls for a rule: open issues, one per project, or one across GitLab
/// (archived projects left out) when it names none.
pub fn list_urls(api_base: &str, rule: &ItemRule) -> Vec<String> {
    let query = "state=opened&order_by=updated_at&sort=desc&per_page=50";
    let labels = if rule.labels.is_empty() { String::new() } else { format!("&labels={}", rule.labels.iter().map(|l| encode(l)).collect::<Vec<_>>().join(",")) };
    if rule.repos.is_empty() {
        let who = match &rule.assignee {
            Assignee::Me => "&scope=assigned_to_me".to_string(),
            Assignee::User { login } => format!("&scope=all&assignee_username={}", encode(login)),
            Assignee::Anyone => String::new(),
        };
        return vec![format!("{api_base}/issues?{query}&non_archived=true{who}{labels}")];
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
    let (status, labels) = status_of(&label_names(v), &cfg.status_labels);
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
/// A project a rule names is left out when it is archived.
pub fn list(http: &dyn Http, forge: &Forge, cfg: &ForgeItems) -> Result<Vec<WorkItem>, String> {
    let mut lists = Vec::new();
    let mut archived: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    for rule in &cfg.rules {
        let mut live = Vec::new();
        for repo in &rule.repos {
            let is = match archived.get(repo) {
                Some(is) => *is,
                None => {
                    let is = is_archived(http, forge, repo)?;
                    archived.insert(repo.clone(), is);
                    is
                }
            };
            if !is {
                live.push(repo.clone());
            }
        }
        if !rule.repos.is_empty() && live.is_empty() {
            continue; // every project it names is archived: not a search across GitLab
        }
        let rule = ItemRule { repos: live, ..rule.clone() };
        for url in list_urls(&forge.site.api_base(), &rule) {
            let reply = get_json_as(http, forge, &url, Need::Issues)?;
            lists.push(reply.as_array().map(|a| a.iter().filter_map(|i| parse_issue(i, cfg)).collect()).unwrap_or_default());
        }
    }
    Ok(merge(lists))
}

/// Whether GitLab says the project is archived.
fn is_archived(http: &dyn Http, forge: &Forge, repo: &str) -> Result<bool, String> {
    let v = get_json_as(http, forge, &format!("{}/projects/{}", forge.site.api_base(), encode(repo)), Need::Issues)?;
    Ok(v["archived"].as_bool() == Some(true))
}

/// The open issue `number` of `repo`; None when it is closed.
pub fn get(http: &dyn Http, forge: &Forge, repo: &str, number: u64, cfg: &ForgeItems) -> Result<Option<Issue>, String> {
    let v = get_json_as(http, forge, &issue_url(forge, repo, number), Need::Issues)?;
    if v["state"].as_str() == Some("closed") {
        return Ok(None);
    }
    let item = parse_issue(&v, cfg).ok_or("GitLab's reply was not an issue.")?;
    Ok(Some(Issue { item, assignee_ids: assignee_ids(&v), labels: label_names(&v) }))
}

fn issue_url(forge: &Forge, repo: &str, number: u64) -> String {
    format!("{}/projects/{}/issues/{number}", forge.site.api_base(), encode(repo))
}

/// Every label name, as the issue spells it.
fn label_names(v: &Value) -> Vec<String> {
    v["labels"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
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
        let others = statuses_to_remove(&cfg.status_labels, status, all_labels);
        if !others.is_empty() {
            body["remove_labels"] = serde_json::json!(others.join(","));
        }
    }
    let url = issue_url(forge, repo, number);
    let reply = send_json_as(http, forge, "PUT", &url, &body, Need::IssueWrite)?;
    let kept = assignees_of_reply(&reply)?;
    // GitLab answers 200 even when it leaves the labels alone.
    if let Some(status) = &cfg.start_status {
        if !labels_of_reply(&reply)?.iter().any(|l| l.eq_ignore_ascii_case(status)) {
            return Err("The issue was assigned to you, but its status was not set.".into());
        }
    }
    if kept.contains(&me.id) {
        return Ok(());
    }
    if kept.len() != 1 {
        return Err("The issue was not assigned to you.".into());
    }
    // A single-assignee tier kept the other one: `me` replaces them.
    let reply = send_json_as(http, forge, "PUT", &url, &serde_json::json!({ "assignee_ids": [me.id] }), Need::IssueWrite)?;
    if assignees_of_reply(&reply)?.contains(&me.id) {
        Ok(())
    } else {
        Err("The issue was not assigned to you.".into())
    }
}

/// Every status label but `start`, in the order of `status_labels`, spelled as
/// the issue spells it where it carries one.
fn statuses_to_remove(status_labels: &[String], start: &str, all_labels: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in status_labels.iter().filter(|s| !s.eq_ignore_ascii_case(start)) {
        let spelled = all_labels.iter().find(|l| l.eq_ignore_ascii_case(s)).unwrap_or(s);
        if !out.iter().any(|o| o.eq_ignore_ascii_case(spelled)) {
            out.push(spelled.clone());
        }
    }
    out
}

fn assignees_of_reply(reply: &str) -> Result<Vec<u64>, String> {
    Ok(assignee_ids(&reply_json(reply)?))
}

fn labels_of_reply(reply: &str) -> Result<Vec<String>, String> {
    Ok(label_names(&reply_json(reply)?))
}

fn reply_json(reply: &str) -> Result<Value, String> {
    serde_json::from_str(reply).map_err(|e| format!("GitLab's reply was not JSON: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GitConnection, WorkItemsConfig};
    use crate::forge::fake::FakeHttp;
    use crate::forge::{ForgeSite, Reply};
    use std::sync::Mutex;

    fn gitlab_forge() -> Forge {
        let c = GitConnection { enabled: true, base_url: Some("https://gitlab.acme.io".into()), ..GitConnection::default_for(GitProvider::Gitlab) };
        Forge { site: ForgeSite::of(&c), token: "glpat".into() }
    }

    /// Answers each request with the next reply, recording (url, body, method).
    struct Seq {
        replies: Mutex<Vec<(u16, String)>>,
        seen: Mutex<Vec<(String, String, String)>>,
    }

    impl Seq {
        fn new(replies: &[(u16, &str)]) -> Self {
            Seq { replies: Mutex::new(replies.iter().rev().map(|(s, b)| (*s, b.to_string())).collect()), seen: Mutex::new(vec![]) }
        }
    }

    impl Http for Seq {
        fn get(&self, _: &str, _: &str) -> Result<Reply, String> {
            Err("no get".into())
        }
        fn post_json(&self, _: &str, _: &str, _: &str) -> Result<Reply, String> {
            Err("no post".into())
        }
        fn send_json(&self, method: &str, url: &str, _: &str, body: &str) -> Result<Reply, String> {
            self.seen.lock().unwrap().push((url.into(), body.into(), method.into()));
            self.replies.lock().unwrap().pop().map(Reply::from).ok_or_else(|| "no reply".to_string())
        }
    }

    fn doing_cfg() -> ForgeItems {
        ForgeItems { status_labels: vec!["status::todo".into(), "status::doing".into()], start_status: Some("status::doing".into()), ..WorkItemsConfig::default().gitlab }
    }

    #[test]
    fn take_replaces_the_assignee_when_the_tier_allows_only_one() {
        let http = Seq::new(&[(200, r#"{"assignees":[{"id":5}],"labels":["status::doing","backend"]}"#), (200, r#"{"assignees":[{"id":9}]}"#)]);
        take(&http, &gitlab_forge(), "grp/r", 17, &Me { login: "tiago".into(), id: 9 }, &[5], &["status::todo".to_string(), "backend".to_string()], &doing_cfg()).unwrap();
        let seen = http.seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "the first PUT kept only the old assignee, so a second one replaces it");
        let first: Value = serde_json::from_str(&seen[0].1).unwrap();
        assert_eq!(first, serde_json::json!({"assignee_ids": [5, 9], "add_labels": "status::doing", "remove_labels": "status::todo"}));
        assert_eq!(seen[1].1, r#"{"assignee_ids":[9]}"#);
        assert!(seen[0].0.ends_with("/projects/grp%2Fr/issues/17") && seen[0].2 == "PUT");
    }

    #[test]
    fn take_errors_when_the_second_put_still_leaves_me_out() {
        let http = Seq::new(&[(200, r#"{"assignees":[{"id":5}],"labels":["status::doing"]}"#), (200, r#"{"assignees":[{"id":5}]}"#)]);
        let e = take(&http, &gitlab_forge(), "grp/r", 17, &Me { login: "t".into(), id: 9 }, &[5], &[], &doing_cfg()).unwrap_err();
        assert_eq!(e, "The issue was not assigned to you.");
    }

    #[test]
    fn take_errors_when_gitlab_leaves_the_status_unset() {
        let http = Seq::new(&[(200, r#"{"assignees":[{"id":9}],"labels":["status::todo"]}"#)]);
        let e = take(&http, &gitlab_forge(), "grp/r", 17, &Me { login: "t".into(), id: 9 }, &[], &["status::todo".to_string()], &doing_cfg()).unwrap_err();
        assert_eq!(e, "The issue was assigned to you, but its status was not set.");
        assert_eq!(http.seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn take_does_not_guess_from_an_unreadable_reply_or_a_crowded_one() {
        let me = Me { login: "t".into(), id: 9 };
        let http = Seq::new(&[(200, "<html>")]);
        let e = take(&http, &gitlab_forge(), "grp/r", 1, &me, &[5], &[], &doing_cfg()).unwrap_err();
        assert!(e.starts_with("GitLab's reply was not JSON"), "{e}");
        assert_eq!(http.seen.lock().unwrap().len(), 1);
        let http = Seq::new(&[(200, r#"{"assignees":[{"id":5},{"id":6}],"labels":["status::doing"]}"#)]);
        let e = take(&http, &gitlab_forge(), "grp/r", 1, &me, &[5, 6], &[], &doing_cfg()).unwrap_err();
        assert_eq!(e, "The issue was not assigned to you.");
        assert_eq!(http.seen.lock().unwrap().len(), 1, "a second PUT would drop two assignees");
    }

    #[test]
    fn every_other_status_label_goes_not_just_the_one_the_issue_shows() {
        let http = Seq::new(&[(200, r#"{"assignees":[{"id":9}],"labels":["Status::Doing"]}"#)]);
        let cfg = ForgeItems { status_labels: vec!["status::todo".into(), "status::review".into(), "status::doing".into()], start_status: Some("status::doing".into()), ..WorkItemsConfig::default().gitlab };
        take(&http, &gitlab_forge(), "grp/r", 1, &Me { login: "t".into(), id: 9 }, &[], &["Status::Todo".to_string()], &cfg).unwrap();
        let first: Value = serde_json::from_str(&http.seen.lock().unwrap()[0].1).unwrap();
        assert_eq!(first["remove_labels"], "Status::Todo,status::review", "the issue's spelling where it has one, else the configured one");
    }

    #[test]
    fn take_makes_one_call_when_me_is_among_the_assignees_and_no_label_is_removed() {
        let http = FakeHttp::new(vec![("/issues/3", 200, r#"{"assignees":[{"id":5},{"id":9}],"labels":["doing"]}"#)]);
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
        let issue = get(&open, &gitlab_forge(), "grp/r", 4, &WorkItemsConfig::default().gitlab).unwrap().unwrap();
        assert_eq!((issue.item.id.as_str(), issue.assignee_ids), ("grp/r#4", vec![5]));
    }

    #[test]
    fn get_returns_the_issues_own_labels_status_ones_included() {
        let body = r#"{"iid":4,"state":"opened","title":"t","web_url":"u","references":{"full":"grp/r#4"},"labels":["bug","Todo","In Progress"],"assignees":[],"author":{"username":"a"}}"#;
        let http = FakeHttp::new(vec![("/projects/grp%2Fr/issues/4", 200, body)]);
        let cfg = ForgeItems { status_labels: vec!["todo".into(), "in progress".into()], ..WorkItemsConfig::default().gitlab };
        let issue = get(&http, &gitlab_forge(), "grp/r", 4, &cfg).unwrap().unwrap();
        assert_eq!(issue.labels, vec!["bug", "Todo", "In Progress"], "as the issue spells them");
        assert_eq!((issue.item.status.as_deref(), issue.item.labels.clone()), (Some("todo"), vec!["bug".to_string()]));
    }

    #[test]
    fn a_rule_becomes_one_call_per_project_with_encoded_paths() {
        let api = "https://gitlab.acme.io/api/v4";
        let me = ItemRule { assignee: Assignee::Me, labels: vec![], repos: vec![], statuses: vec![] };
        assert_eq!(list_urls(api, &me), vec![format!("{api}/issues?state=opened&order_by=updated_at&sort=desc&per_page=50&non_archived=true&scope=assigned_to_me")]);
        let anyone = ItemRule { assignee: Assignee::Anyone, labels: vec!["status::todo".into(), "a b".into()], repos: vec!["grp/sub/r".into(), "grp/x".into()], statuses: vec![] };
        assert_eq!(list_urls(api, &anyone), vec![
            format!("{api}/projects/grp%2Fsub%2Fr/issues?state=opened&order_by=updated_at&sort=desc&per_page=50&labels=status%3A%3Atodo,a%20b"),
            format!("{api}/projects/grp%2Fx/issues?state=opened&order_by=updated_at&sort=desc&per_page=50&labels=status%3A%3Atodo,a%20b"),
        ]);
        let maria = ItemRule { assignee: Assignee::User { login: "maria".into() }, labels: vec![], repos: vec![], statuses: vec![] };
        assert_eq!(list_urls(api, &maria), vec![format!("{api}/issues?state=opened&order_by=updated_at&sort=desc&per_page=50&non_archived=true&scope=all&assignee_username=maria")]);
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
        let cfg = ForgeItems { rules: vec![ItemRule { assignee: Assignee::Anyone, labels: vec![], repos: vec!["grp/a".into(), "grp/b".into()], statuses: vec![] }], ..WorkItemsConfig::default().gitlab };
        let http = FakeHttp::new(vec![("grp%2Fa", 200, "[]"), ("grp%2Fb", 404, r#"{"message":"404 Project Not Found"}"#)]);
        assert_eq!(list(&http, &f, &cfg).unwrap_err(), "404: 404 Project Not Found");
    }

    #[test]
    fn an_archived_project_a_rule_names_is_left_out_and_asked_about_once() {
        let named = |repos: &[&str]| ItemRule { assignee: Assignee::Anyone, labels: vec![], repos: repos.iter().map(|s| s.to_string()).collect(), statuses: vec![] };
        let cfg = ForgeItems { rules: vec![named(&["grp/old", "grp/new"]), named(&["grp/old"])], ..WorkItemsConfig::default().gitlab };
        let issue = r#"[{"iid":1,"title":"t","web_url":"u","references":{"full":"grp/new#1"},"labels":[],"assignees":[],"author":{"username":"a"}}]"#;
        let http = FakeHttp::new(vec![
            ("/projects/grp%2Fold/issues", 200, r#"[{"iid":2,"title":"t","web_url":"u","references":{"full":"grp/old#2"}}]"#),
            ("/projects/grp%2Fnew/issues", 200, issue),
            ("/projects/grp%2Fold", 200, r#"{"archived":true}"#),
            ("/projects/grp%2Fnew", 200, r#"{"archived":false}"#),
        ]);
        let items = list(&http, &gitlab_forge(), &cfg).unwrap();
        assert_eq!(items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), vec!["grp/new#1"]);
        let seen: Vec<String> = http.seen.lock().unwrap().iter().map(|s| s.0.clone()).collect();
        assert!(!seen.iter().any(|u| u.contains("grp%2Fold/issues")), "{seen:?}");
        assert_eq!(seen.iter().filter(|u| u.ends_with("/projects/grp%2Fold")).count(), 1, "{seen:?}");
    }

    #[test]
    fn a_rule_whose_projects_are_all_archived_asks_for_nothing_more() {
        let cfg = ForgeItems { rules: vec![ItemRule { assignee: Assignee::Me, labels: vec![], repos: vec!["grp/old".into()], statuses: vec![] }], ..WorkItemsConfig::default().gitlab };
        let http = FakeHttp::new(vec![("/projects/grp%2Fold", 200, r#"{"archived":true}"#), ("/issues", 200, r#"[{"iid":9,"title":"t","web_url":"u","references":{"full":"x/y#9"}}]"#)]);
        assert!(list(&http, &gitlab_forge(), &cfg).unwrap().is_empty());
        assert_eq!(http.seen.lock().unwrap().len(), 1, "not a search across GitLab");
    }
}
