//! GitLab's issues: one call per rule and project.

use super::{merge, status_of, ItemRepo, WorkItem};
use crate::config::{Assignee, ForgeItems, GitProvider, ItemRule};
use crate::forge::{encode, get_json_as, Forge, Http, Need};
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GitConnection, WorkItemsConfig};
    use crate::forge::fake::FakeHttp;
    use crate::forge::ForgeSite;

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
