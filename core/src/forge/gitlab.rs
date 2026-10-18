//! GitLab: the MR of a branch (REST) and the MRs waiting on the user (one
//! GraphQL query). A reviewer stays on an MR after reviewing, so the
//! review-requested list also holds the ones the user already reviewed.

use super::{encode, get_json, post_graphql, Forge, Http};
use crate::config::GitProvider;
use crate::model::PullRequest;
use crate::reviews::{merge, Reason, ReviewMark, ReviewPr};
use serde_json::Value;

pub const QUERY: &str = r#"query {
  currentUser {
    username
    requested: reviewRequestedMergeRequests(state: opened, first: 50, sort: UPDATED_DESC) { nodes { ...mr } }
    assigned: assignedMergeRequests(state: opened, first: 50, sort: UPDATED_DESC) { nodes { ...mr } }
  }
}
fragment mr on MergeRequest {
  iid title webUrl draft updatedAt
  project { fullPath }
  author { username }
  reviewers { nodes { username bot mergeRequestInteraction { reviewState } } }
}"#;

fn mark(state: &str) -> Option<ReviewMark> {
    match state {
        "APPROVED" => Some(ReviewMark::Approved),
        "REQUESTED_CHANGES" => Some(ReviewMark::ChangesRequested),
        "REVIEWED" => Some(ReviewMark::Commented),
        _ => None,
    }
}

/// (username, bot, mark) for each reviewer.
fn reviewers(m: &Value) -> Vec<(String, bool, Option<ReviewMark>)> {
    m["reviewers"]["nodes"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| Some((r["username"].as_str()?.to_string(), r["bot"].as_bool().unwrap_or(false), mark(r["mergeRequestInteraction"]["reviewState"].as_str().unwrap_or("")))))
                .collect()
        })
        .unwrap_or_default()
}

fn parse_node(m: &Value, reason: Reason, me: &str) -> Option<ReviewPr> {
    let author = m["author"]["username"].as_str().unwrap_or("").to_string();
    let rs = reviewers(m);
    let my_review = if author.eq_ignore_ascii_case(me) { None } else { rs.iter().find(|(u, _, _)| u.eq_ignore_ascii_case(me)).and_then(|(_, _, k)| *k) };
    let others: Vec<&(String, bool, Option<ReviewMark>)> = rs.iter().filter(|(u, bot, _)| !bot && !u.eq_ignore_ascii_case(me) && !u.eq_ignore_ascii_case(&author)).collect();
    let (pr_review, pr_reviewers) = [ReviewMark::ChangesRequested, ReviewMark::Approved, ReviewMark::Commented]
        .into_iter()
        .map(|want| (want, others.iter().filter(|(_, _, k)| *k == Some(want)).map(|(u, _, _)| u.clone()).collect::<Vec<_>>()))
        .find(|(_, who)| !who.is_empty())
        .map_or((None, vec![]), |(k, who)| (Some(k), who));
    Some(ReviewPr {
        number: m["iid"].as_str()?.parse().ok()?,
        forge: GitProvider::Gitlab,
        repo: m["project"]["fullPath"].as_str()?.to_string(),
        title: m["title"].as_str().unwrap_or("").to_string(),
        author,
        url: m["webUrl"].as_str()?.to_string(),
        is_draft: m["draft"].as_bool().unwrap_or(false),
        updated_at: m["updatedAt"].as_str().unwrap_or("").to_string(),
        reasons: vec![reason],
        my_review,
        pr_review,
        pr_reviewers,
    })
}

/// GitLab's reply to `QUERY` as one list of review MRs.
pub fn parse_graphql(json: &str) -> Result<Vec<ReviewPr>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("GitLab's reply was not JSON: {e}"))?;
    let user = &v["data"]["currentUser"];
    let Some(me) = user["username"].as_str() else {
        if let Some(m) = v["errors"].as_array().and_then(|e| e.first()).and_then(|e| e["message"].as_str()) {
            return Err(format!("GitLab said: {m}"));
        }
        // An anonymous query answers with a null user rather than a 401.
        return Err("the token was refused. It needs the read_api scope.".into());
    };
    let list = |key: &str, reason: Reason| -> Vec<ReviewPr> { user[key]["nodes"].as_array().map(|a| a.iter().filter_map(|m| parse_node(m, reason, me)).collect()).unwrap_or_default() };
    Ok(merge(list("requested", Reason::Review), list("assigned", Reason::Assigned)))
}

/// The MR to show from a `GET /merge_requests` list: the first open one, else the first.
pub fn parse_mrs(v: &Value) -> Option<PullRequest> {
    let list = v.as_array()?;
    let m = list.iter().find(|m| m["state"] == "opened").or_else(|| list.first())?;
    let state = match m["state"].as_str()? {
        "opened" if m["draft"].as_bool() == Some(true) => "draft",
        "opened" => "open",
        "merged" => "merged",
        _ => "closed",
    };
    Some(PullRequest { number: m["iid"].as_u64()?, url: m["web_url"].as_str()?.to_string(), state: state.into(), forge: GitProvider::Gitlab })
}

pub fn current_pr(http: &dyn Http, forge: &Forge, repo: &str, branch: &str) -> Result<Option<PullRequest>, String> {
    let url = format!("{}/projects/{}/merge_requests?source_branch={}&state=all&order_by=updated_at&per_page=10", forge.site.api_base(), encode(repo), encode(branch));
    Ok(parse_mrs(&get_json(http, forge, &url)?))
}

pub fn review_prs(http: &dyn Http, forge: &Forge) -> Result<Vec<ReviewPr>, String> {
    parse_graphql(&post_graphql(http, forge, QUERY)?)
}

pub fn whoami(http: &dyn Http, forge: &Forge) -> Result<String, String> {
    let v = get_json(http, forge, &format!("{}/user", forge.site.api_base()))?;
    v["username"].as_str().map(str::to_string).ok_or_else(|| "GitLab did not say who the token belongs to.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GitConnection, GitProvider};
    use crate::forge::{fake::FakeHttp, ForgeSite};
    use crate::reviews::{Reason, ReviewMark};

    fn forge() -> Forge {
        let c = GitConnection { enabled: true, base_url: Some("https://gitlab.acme.io".into()), ..GitConnection::default_for(GitProvider::Gitlab) };
        Forge { site: ForgeSite::of(&c), token: "glpat".into() }
    }

    fn reviewer(name: &str, state: &str, bot: bool) -> String {
        format!(r#"{{"username":"{name}","bot":{bot},"mergeRequestInteraction":{{"reviewState":"{state}"}}}}"#)
    }

    fn mr(iid: u64, author: &str, updated: &str, reviewers: &[String]) -> String {
        format!(
            r#"{{"iid":"{iid}","title":"t{iid}","webUrl":"https://gitlab.acme.io/grp/sub/r/-/merge_requests/{iid}","draft":false,"updatedAt":"{updated}","project":{{"fullPath":"grp/sub/r"}},"author":{{"username":"{author}"}},"reviewers":{{"nodes":[{}]}}}}"#,
            reviewers.join(",")
        )
    }

    fn response(requested: &[String], assigned: &[String]) -> String {
        format!(r#"{{"data":{{"currentUser":{{"username":"Me","requested":{{"nodes":[{}]}},"assigned":{{"nodes":[{}]}}}}}}}}"#, requested.join(","), assigned.join(","))
    }

    #[test]
    fn requested_and_assigned_mrs_become_one_list_newest_first() {
        let a = mr(3, "jane", "2026-10-01T10:00:00Z", &[reviewer("me", "UNREVIEWED", false)]);
        let b = mr(7, "bob", "2026-10-02T10:00:00Z", &[]);
        let prs = parse_graphql(&response(&[a.clone(), b.clone()], &[b])).unwrap();
        assert_eq!(prs.iter().map(|p| p.number).collect::<Vec<_>>(), vec![7, 3]);
        let p = &prs[1];
        assert_eq!((p.forge, p.repo.as_str(), p.title.as_str(), p.author.as_str(), p.is_draft), (GitProvider::Gitlab, "grp/sub/r", "t3", "jane", false));
        assert_eq!(p.url, "https://gitlab.acme.io/grp/sub/r/-/merge_requests/3");
        assert_eq!((p.reasons.clone(), p.my_review), (vec![Reason::Review], None));
        assert_eq!(prs[0].reasons, vec![Reason::Review, Reason::Assigned]);
    }

    #[test]
    fn the_users_review_comes_from_their_reviewer_state() {
        let mine = |state: &str| parse_graphql(&response(&[mr(1, "jane", "", &[reviewer("ME", state, false)])], &[])).unwrap()[0].my_review;
        assert_eq!(mine("APPROVED"), Some(ReviewMark::Approved), "usernames match case-insensitively");
        assert_eq!(mine("REQUESTED_CHANGES"), Some(ReviewMark::ChangesRequested));
        assert_eq!(mine("REVIEWED"), Some(ReviewMark::Commented));
        for none in ["UNREVIEWED", "REVIEW_STARTED", "UNAPPROVED"] {
            assert_eq!(mine(none), None, "{none}");
        }
    }

    #[test]
    fn the_mrs_review_state_comes_from_other_people_changes_first() {
        let state = |author: &str, rs: &[String]| {
            let p = parse_graphql(&response(&[], &[mr(1, author, "", rs)])).unwrap().remove(0);
            (p.pr_review, p.pr_reviewers)
        };
        let split = [reviewer("jane", "APPROVED", false), reviewer("bob", "REQUESTED_CHANGES", false)];
        assert_eq!(state("me", &split), (Some(ReviewMark::ChangesRequested), vec!["bob".into()]));
        assert_eq!(state("me", &[reviewer("jane", "APPROVED", false)]), (Some(ReviewMark::Approved), vec!["jane".into()]));
        assert_eq!(state("me", &[reviewer("jane", "REVIEWED", false)]), (Some(ReviewMark::Commented), vec!["jane".into()]));
        assert_eq!(state("me", &[reviewer("renovate", "APPROVED", true)]), (None, vec![]), "bots do not count");
        assert_eq!(state("jane", &[reviewer("me", "APPROVED", false)]), (None, vec![]), "the user's own review is not the MR's");
        assert_eq!(state("jane", &[reviewer("jane", "APPROVED", false)]), (None, vec![]), "nor the author's");
    }

    #[test]
    fn no_current_user_means_the_token_was_not_accepted() {
        assert_eq!(parse_graphql(r#"{"data":{"currentUser":null}}"#).unwrap_err(), "the token was refused. It needs the read_api scope.");
        assert!(parse_graphql(r#"{"errors":[{"message":"Field 'x' doesn't exist"}]}"#).unwrap_err().contains("Field 'x'"));
        assert!(parse_graphql("<html>").is_err());
    }

    const MRS: &str = r#"[
      {"iid":4,"web_url":"https://gitlab.acme.io/grp/sub/r/-/merge_requests/4","state":"merged","draft":false},
      {"iid":5,"web_url":"https://gitlab.acme.io/grp/sub/r/-/merge_requests/5","state":"opened","draft":true}
    ]"#;

    #[test]
    fn the_open_mr_wins_and_states_map_to_the_cards() {
        let v: serde_json::Value = serde_json::from_str(MRS).unwrap();
        let pr = parse_mrs(&v).unwrap();
        assert_eq!((pr.number, pr.state.as_str(), pr.forge), (5, "draft", GitProvider::Gitlab));
        for (state, want) in [("merged", "merged"), ("closed", "closed"), ("locked", "closed"), ("opened", "open")] {
            let v = serde_json::json!([{ "iid": 1, "web_url": "u", "state": state, "draft": false }]);
            assert_eq!(parse_mrs(&v).unwrap().state, want, "{state}");
        }
        assert_eq!(parse_mrs(&serde_json::json!([])), None);
    }

    #[test]
    fn current_pr_asks_the_project_by_its_encoded_path_and_branch() {
        let http = FakeHttp::new(vec![("/merge_requests", 200, MRS)]);
        assert_eq!(current_pr(&http, &forge(), "grp/sub/r", "feat/x#1+y").unwrap().unwrap().number, 5);
        assert_eq!(
            http.seen.lock().unwrap()[0].0,
            "https://gitlab.acme.io/api/v4/projects/grp%2Fsub%2Fr/merge_requests?source_branch=feat%2Fx%231%2By&state=all&order_by=updated_at&per_page=10"
        );
    }

    #[test]
    fn review_prs_posts_to_the_graphql_url_and_whoami_reads_the_username() {
        let http = FakeHttp::new(vec![("/api/graphql", 200, &response(&[], &[])), ("/api/v4/user", 200, r#"{"username":"tiago"}"#)]);
        assert_eq!(review_prs(&http, &forge()).unwrap(), vec![]);
        assert_eq!(http.seen.lock().unwrap()[0].0, "https://gitlab.acme.io/api/graphql");
        assert_eq!(whoami(&http, &forge()).unwrap(), "tiago");
    }
}
