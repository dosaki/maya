//! GitHub: the PR of a branch (REST) and the PRs waiting on the user (the
//! GraphQL search `reviews::QUERY`, parsed by `reviews::parse_graphql`).

use super::{encode, get_json, get_json_as, post_graphql, post_json_as, scrub, Forge, Http, Need};
use crate::deployments::{approvable, needs_approval, parse_runs, Environment, Run, ACTIVE_WINDOW_MS, REPO_PAGES};
use crate::config::GitProvider;
use crate::model::PullRequest;
use crate::reviews::ReviewPr;

/// The PR to show from a `GET /pulls` list: the first open one, else the
/// first (the list is newest-updated first).
pub fn parse_pulls(v: &serde_json::Value) -> Option<PullRequest> {
    let list = v.as_array()?;
    let p = list.iter().find(|p| p["state"] == "open").or_else(|| list.first())?;
    let state = if p["state"] == "open" {
        if p["draft"].as_bool() == Some(true) {
            "draft"
        } else {
            "open"
        }
    } else if p["merged_at"].is_string() {
        "merged"
    } else {
        "closed"
    };
    Some(PullRequest { number: p["number"].as_u64()?, url: p["html_url"].as_str()?.to_string(), state: state.into(), forge: GitProvider::Github })
}

/// The PR whose head is `branch` on the repo's owner. PRs from forks are not found.
pub fn current_pr(http: &dyn Http, forge: &Forge, repo: &str, branch: &str) -> Result<Option<PullRequest>, String> {
    let owner = repo.split('/').next().unwrap_or(repo);
    let url = format!("{}/repos/{repo}/pulls?head={}&state=all&sort=updated&direction=desc&per_page=10", forge.site.api_base(), encode(&format!("{owner}:{branch}")));
    Ok(parse_pulls(&get_json(http, forge, &url)?))
}

/// Open PRs asking for the user's review, reviewed by them, or assigned to them.
/// GitHub's error text may quote what it was sent, so it is scrubbed here,
/// where the token is known.
pub fn review_prs(http: &dyn Http, forge: &Forge) -> Result<Vec<ReviewPr>, String> {
    let (prs, partial) = crate::reviews::parse_graphql(&post_graphql(http, forge, crate::reviews::QUERY)?).map_err(|e| scrub(&e, &forge.token))?;
    if let Some(e) = partial {
        crate::log::line("app", format!("pull requests: partial results: {}", scrub(&e, &forge.token)));
    }
    Ok(prs)
}

/// The token's user.
pub fn whoami(http: &dyn Http, forge: &Forge) -> Result<String, String> {
    let v = get_json(http, forge, &format!("{}/user", forge.site.api_base()))?;
    v["login"].as_str().map(str::to_string).ok_or_else(|| "GitHub did not say who the token belongs to.".into())
}

/// The repos pushed to within the last week, newest first: the pages stop at
/// the first older (or never pushed) repo, a short page, or `REPO_PAGES`.
pub fn active_repos(http: &dyn Http, forge: &Forge, now_ms: u64) -> Result<Vec<String>, String> {
    let cutoff = now_ms.saturating_sub(ACTIVE_WINDOW_MS);
    let mut repos = Vec::new();
    for page in 1..=REPO_PAGES {
        let url = format!("{}/user/repos?sort=pushed&direction=desc&per_page=100&affiliation=owner,collaborator,organization_member&page={page}", forge.site.api_base());
        let v = get_json_as(http, forge, &url, Need::Deployments)?;
        let list = v.as_array().map(Vec::as_slice).unwrap_or_default();
        for r in list {
            let pushed = r["pushed_at"].as_str().and_then(crate::codex::ms_of);
            let (Some(name), Some(pushed)) = (r["full_name"].as_str(), pushed) else { return Ok(repos) };
            if pushed < cutoff {
                return Ok(repos);
            }
            repos.push(name.to_string());
        }
        if list.len() < 100 {
            break;
        }
    }
    Ok(repos)
}

/// Whether the repo has an environment with required reviewers. A 404 (no
/// access to environments) means no.
pub fn repo_needs_approval(http: &dyn Http, forge: &Forge, repo: &str) -> Result<bool, String> {
    match get_json_as(http, forge, &format!("{}/repos/{repo}/environments?per_page=100", forge.site.api_base()), Need::Deployments) {
        Ok(v) => Ok(needs_approval(&v)),
        Err(e) if e.starts_with("404") => Ok(false),
        Err(e) => Err(e),
    }
}

/// The repo's runs in `status` (`waiting`, `queued`, ...), newest first.
pub fn runs(http: &dyn Http, forge: &Forge, repo: &str, status: &str, per_page: u32) -> Result<Vec<Run>, String> {
    let url = format!("{}/repos/{repo}/actions/runs?status={status}&per_page={per_page}", forge.site.api_base());
    Ok(parse_runs(&get_json_as(http, forge, &url, Need::Deployments)?))
}

/// The environments of a run that the user may approve.
pub fn pending_deployments(http: &dyn Http, forge: &Forge, repo: &str, run_id: u64) -> Result<Vec<Environment>, String> {
    Ok(approvable(&get_json_as(http, forge, &format!("{}/repos/{repo}/actions/runs/{run_id}/pending_deployments", forge.site.api_base()), Need::Deployments)?))
}

/// Approves or rejects a run's deployment to the given environments.
pub fn review_deployment(http: &dyn Http, forge: &Forge, repo: &str, run_id: u64, environment_ids: &[u64], approve: bool) -> Result<(), String> {
    let (state, comment) = if approve { ("approved", "Approved in Maya.") } else { ("rejected", "Rejected in Maya.") };
    let body = serde_json::json!({"environment_ids": environment_ids, "state": state, "comment": comment});
    post_json_as(http, forge, &format!("{}/repos/{repo}/actions/runs/{run_id}/pending_deployments", forge.site.api_base()), &body, Need::Deployments).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GitConnection, GitProvider};
    use crate::forge::{fake::FakeHttp, ForgeSite};

    fn forge() -> Forge {
        Forge { site: ForgeSite::of(&GitConnection::default_for(GitProvider::Github)), token: "ghp_x".into() }
    }

    const PULLS: &str = r#"[
      {"number":9,"html_url":"https://github.com/Org/r/pull/9","state":"closed","draft":false,"merged_at":"2026-10-01T00:00:00Z"},
      {"number":12,"html_url":"https://github.com/Org/r/pull/12","state":"open","draft":true,"merged_at":null}
    ]"#;

    #[test]
    fn the_open_pr_wins_over_a_newer_closed_one_and_draft_is_a_state() {
        let v: serde_json::Value = serde_json::from_str(PULLS).unwrap();
        let pr = parse_pulls(&v).unwrap();
        assert_eq!((pr.number, pr.state.as_str(), pr.url.as_str(), pr.forge), (12, "draft", "https://github.com/Org/r/pull/12", GitProvider::Github));
        let merged: serde_json::Value = serde_json::from_str(r#"[{"number":9,"html_url":"u","state":"closed","draft":false,"merged_at":"x"}]"#).unwrap();
        assert_eq!(parse_pulls(&merged).unwrap().state, "merged");
        let closed: serde_json::Value = serde_json::from_str(r#"[{"number":9,"html_url":"u","state":"closed","draft":false,"merged_at":null}]"#).unwrap();
        assert_eq!(parse_pulls(&closed).unwrap().state, "closed");
        assert_eq!(parse_pulls(&serde_json::json!([])), None);
    }

    #[test]
    fn current_pr_asks_for_the_branch_on_the_owner_encoded() {
        let http = FakeHttp::new(vec![("/pulls", 200, PULLS)]);
        let pr = current_pr(&http, &forge(), "Org/r", "feat/x#1+y").unwrap().unwrap();
        assert_eq!(pr.number, 12);
        let url = &http.seen.lock().unwrap()[0].0;
        assert_eq!(url, "https://api.github.com/repos/Org/r/pulls?head=Org%3Afeat%2Fx%231%2By&state=all&sort=updated&direction=desc&per_page=10");
    }

    #[test]
    fn graphql_errors_quoting_the_token_are_scrubbed_in_the_error_and_the_log() {
        let http = FakeHttp::new(vec![("/graphql", 200, r#"{"errors":[{"message":"no access for ghp_x"}]}"#)]);
        assert_eq!(review_prs(&http, &forge()).unwrap_err(), "GitHub said: no access for ***");
        let partial = r#"{"errors":[{"message":"SAML blocks ghp_x (partial-test)"}],"data":{"viewer":{"login":"Me"},"requested":{"nodes":[]},"reviewed":{"nodes":[]},"assigned":{"nodes":[]}}}"#;
        let http = FakeHttp::new(vec![("/graphql", 200, partial)]);
        assert_eq!(review_prs(&http, &forge()).unwrap(), vec![]);
        let lines = crate::log::lines();
        assert!(!lines.iter().any(|l| l.text.contains("ghp_x (partial-test)")), "{lines:?}");
        assert!(lines.iter().any(|l| l.text == "pull requests: partial results: GitHub said: SAML blocks *** (partial-test)"), "{lines:?}");
    }

    #[test]
    fn review_prs_posts_the_query_and_whoami_reads_the_login() {
        let body = r#"{"data":{"viewer":{"login":"Me"},"requested":{"nodes":[]},"reviewed":{"nodes":[]},"assigned":{"nodes":[]}}}"#;
        let http = FakeHttp::new(vec![("/graphql", 200, body), ("/user", 200, r#"{"login":"tiago"}"#)]);
        assert_eq!(review_prs(&http, &forge()).unwrap(), vec![]);
        assert!(http.seen.lock().unwrap()[0].2.as_deref().unwrap().contains("review-requested:@me"));
        assert_eq!(whoami(&http, &forge()).unwrap(), "tiago");
    }

    fn now() -> u64 {
        crate::codex::ms_of("2026-10-28T12:00:00Z").unwrap()
    }

    #[test]
    fn active_repos_stops_at_the_first_repo_older_than_a_week() {
        let page = r#"[{"full_name":"O/new","pushed_at":"2026-10-27T00:00:00Z"},{"full_name":"O/old","pushed_at":"2026-10-20T00:00:00Z"},{"full_name":"O/x","pushed_at":"2026-10-28T00:00:00Z"}]"#;
        let http = FakeHttp::new(vec![("/user/repos", 200, page)]);
        assert_eq!(active_repos(&http, &forge(), now()).unwrap(), vec!["O/new"]);
        assert_eq!(http.seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn active_repos_stops_at_a_null_pushed_at_and_a_short_page() {
        let http = FakeHttp::new(vec![("/user/repos", 200, r#"[{"full_name":"O/a","pushed_at":"2026-10-28T00:00:00Z"},{"full_name":"O/e","pushed_at":null}]"#)]);
        assert_eq!(active_repos(&http, &forge(), now()).unwrap(), vec!["O/a"]);
        let http = FakeHttp::new(vec![("/user/repos", 200, r#"[{"full_name":"O/a","pushed_at":"2026-10-28T00:00:00Z"}]"#)]);
        assert_eq!(active_repos(&http, &forge(), now()).unwrap(), vec!["O/a"]);
        assert_eq!(http.seen.lock().unwrap().len(), 1, "a page under 100 is the last");
    }

    #[test]
    fn active_repos_reads_at_most_five_pages() {
        let full: String = format!("[{}]", (0..100).map(|i| format!(r#"{{"full_name":"O/r{i}","pushed_at":"2026-10-28T00:00:00Z"}}"#)).collect::<Vec<_>>().join(","));
        let http = FakeHttp { replies: vec![("/user/repos", 200, full)], seen: Default::default() };
        assert_eq!(active_repos(&http, &forge(), now()).unwrap().len(), 500);
        let seen = http.seen.lock().unwrap();
        assert_eq!(seen.len(), 5);
        assert!(seen[4].0.ends_with("&page=5"));
    }

    #[test]
    fn a_repo_without_environments_access_needs_no_approval() {
        let http = FakeHttp::new(vec![("/environments", 404, r#"{"message":"Not Found"}"#)]);
        assert_eq!(repo_needs_approval(&http, &forge(), "O/r"), Ok(false));
        let http = FakeHttp::new(vec![("/environments", 403, "{}")]);
        assert!(crate::forge::is_refusal(&repo_needs_approval(&http, &forge(), "O/r").unwrap_err()));
    }

    #[test]
    fn approve_sends_only_the_given_environments() {
        let http = FakeHttp::new(vec![("/pending_deployments", 200, "[]")]);
        review_deployment(&http, &forge(), "O/r", 7, &[1], true).unwrap();
        review_deployment(&http, &forge(), "O/r", 7, &[1, 2], false).unwrap();
        let seen = http.seen.lock().unwrap();
        assert_eq!(seen[0].0, "https://api.github.com/repos/O/r/actions/runs/7/pending_deployments");
        let body: serde_json::Value = serde_json::from_str(seen[0].2.as_deref().unwrap()).unwrap();
        assert_eq!(body, serde_json::json!({"environment_ids": [1], "state": "approved", "comment": "Approved in Maya."}));
        let body: serde_json::Value = serde_json::from_str(seen[1].2.as_deref().unwrap()).unwrap();
        assert_eq!(body, serde_json::json!({"environment_ids": [1, 2], "state": "rejected", "comment": "Rejected in Maya."}));
    }

    #[test]
    fn runs_and_pending_deployments_ask_the_right_urls() {
        let http = FakeHttp::new(vec![
            ("/pending_deployments", 200, r#"[{"environment":{"id":1,"name":"production"},"current_user_can_approve":true}]"#),
            ("/actions/runs?", 200, r#"{"workflow_runs":[]}"#),
        ]);
        assert_eq!(runs(&http, &forge(), "O/r", "waiting", 50).unwrap(), vec![]);
        assert_eq!(pending_deployments(&http, &forge(), "O/r", 7).unwrap().len(), 1);
        let seen = http.seen.lock().unwrap();
        assert_eq!(seen[0].0, "https://api.github.com/repos/O/r/actions/runs?status=waiting&per_page=50");
    }
}
