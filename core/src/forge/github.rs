//! GitHub: the PR of a branch (REST) and the PRs waiting on the user (the
//! GraphQL search `reviews::QUERY`, parsed by `reviews::parse_graphql`).

use super::{encode, get_json, post_graphql, scrub, Forge, Http};
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
}
