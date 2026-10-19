use crate::launch::{shell_single_quote, LaunchOptions};
use crate::model::Harness;
use crate::config::GitProvider;
use serde::Serialize;
use std::path::{Path, PathBuf};
use crate::forge::{Forge, ForgeError, ForgeSite, Http};

/// Why a PR is on the list.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Reason {
    Review,
    Assigned,
}

/// Where a review stands: a standing verdict, else a comment.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ReviewMark {
    Approved,
    Commented,
    ChangesRequested,
}

/// A pull request waiting on the user.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewPr {
    pub number: u64,
    /// GitHub or GitLab.
    pub forge: GitProvider,
    /// The repo's full path: `owner/name`, or `group/sub/name` on GitLab.
    pub repo: String,
    pub title: String,
    pub author: String,
    pub url: String,
    pub is_draft: bool,
    /// ISO 8601, as GitHub gives it; sorts lexically.
    pub updated_at: String,
    pub reasons: Vec<Reason>,
    /// The user's own review; None when they have not reviewed it, or wrote it.
    pub my_review: Option<ReviewMark>,
    /// What the other reviewers (people, not bots, nor the author) decided:
    /// changes requested by anyone wins, then an approval, then a comment.
    pub pr_review: Option<ReviewMark>,
    /// Who gave `pr_review`.
    pub pr_reviewers: Vec<String>,
    /// The repo's SSH and HTTPS clone addresses, as the forge's API gave them.
    #[serde(default)]
    pub ssh_url: Option<String>,
    #[serde(default)]
    pub https_url: Option<String>,
}

impl ReviewPr {
    /// `Org/bedrock#451` or `grp/r!12`: how the lists and the voice name it.
    pub fn reference(&self) -> String {
        format!("{}{}{}", self.repo, self.forge.sigil(), self.number)
    }

    /// The repo's last path segment.
    pub fn repo_name(&self) -> &str {
        self.repo.rsplit('/').next().unwrap_or(&self.repo)
    }
}

/// Three searches in one call: PRs asking for the user's review, open PRs
/// they reviewed (GitHub drops the request once they do), and PRs assigned
/// to them; each with the reviews needed for `ReviewMark`.
pub const QUERY: &str = r#"query {
  viewer { login }
  requested: search(query: "is:pr is:open review-requested:@me sort:updated-desc", type: ISSUE, first: 50) { nodes { ...pr } }
  reviewed: search(query: "is:pr is:open reviewed-by:@me -author:@me sort:updated-desc", type: ISSUE, first: 50) { nodes { ...pr } }
  assigned: search(query: "is:pr is:open assignee:@me sort:updated-desc", type: ISSUE, first: 50) { nodes { ...pr } }
}
fragment pr on PullRequest {
  number title url isDraft updatedAt
  repository { nameWithOwner sshUrl url }
  author { login }
  latestOpinionatedReviews(first: 100) { nodes { state author { __typename login } } }
  latestReviews(first: 100) { nodes { state author { __typename login } } }
}"#;

/// The state of `login`'s review in a `{nodes: [{state, author}]}` list.
fn review_state<'a>(reviews: &'a serde_json::Value, login: &str) -> Option<&'a str> {
    reviews["nodes"].as_array()?.iter().find(|r| r["author"]["login"].as_str().is_some_and(|l| l.eq_ignore_ascii_case(login)))?["state"].as_str()
}

fn my_review(p: &serde_json::Value, login: &str) -> Option<ReviewMark> {
    if p["author"]["login"].as_str().is_some_and(|a| a.eq_ignore_ascii_case(login)) {
        return None;
    }
    match review_state(&p["latestOpinionatedReviews"], login) {
        Some("APPROVED") => return Some(ReviewMark::Approved),
        Some("CHANGES_REQUESTED") => return Some(ReviewMark::ChangesRequested),
        _ => {}
    }
    (review_state(&p["latestReviews"], login) == Some("COMMENTED")).then_some(ReviewMark::Commented)
}

/// The PR's review state from people other than the user and the author.
fn pr_review(p: &serde_json::Value, login: &str) -> (Option<ReviewMark>, Vec<String>) {
    let author = p["author"]["login"].as_str().unwrap_or("");
    let by = |key: &str, state: &str| -> Vec<String> {
        p[key]["nodes"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter(|r| r["state"] == state && r["author"]["__typename"] != "Bot")
                    .filter_map(|r| r["author"]["login"].as_str())
                    .filter(|l| !l.eq_ignore_ascii_case(login) && !l.eq_ignore_ascii_case(author) && !l.ends_with("[bot]"))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    for (mark, key, state) in [
        (ReviewMark::ChangesRequested, "latestOpinionatedReviews", "CHANGES_REQUESTED"),
        (ReviewMark::Approved, "latestOpinionatedReviews", "APPROVED"),
        (ReviewMark::Commented, "latestReviews", "COMMENTED"),
    ] {
        let who = by(key, state);
        if !who.is_empty() {
            return (Some(mark), who);
        }
    }
    (None, vec![])
}

fn parse_node(p: &serde_json::Value, reason: Reason, login: &str) -> Option<ReviewPr> {
    let (pr_review, pr_reviewers) = pr_review(p, login);
    Some(ReviewPr {
        number: p["number"].as_u64()?,
        forge: GitProvider::Github,
        repo: p["repository"]["nameWithOwner"].as_str()?.to_string(),
        title: p["title"].as_str().unwrap_or("").to_string(),
        author: p["author"]["login"].as_str().unwrap_or("").to_string(),
        url: p["url"].as_str()?.to_string(),
        is_draft: p["isDraft"].as_bool().unwrap_or(false),
        updated_at: p["updatedAt"].as_str().unwrap_or("").to_string(),
        reasons: vec![reason],
        my_review: my_review(p, login),
        pr_review,
        pr_reviewers,
        ssh_url: p["repository"]["sshUrl"].as_str().map(String::from),
        // `url` is the repo's web address; the clone address adds `.git`.
        https_url: p["repository"]["url"].as_str().map(|u| format!("{}.git", u.trim_end_matches('/'))),
    })
}

/// GitHub's reply to `QUERY` as one list of review PRs, and the error
/// GitHub reported beside them, if any. Not logged here: the caller holds
/// the token to scrub it with.
pub fn parse_graphql(json: &str) -> Result<(Vec<ReviewPr>, Option<String>), String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("GitHub's reply was not JSON: {e}"))?;
    let error = v["errors"].as_array().and_then(|e| e.first()).and_then(|e| e["message"].as_str()).map(|m| format!("GitHub said: {m}"));
    let data = &v["data"];
    // An org the token cannot reach adds an error but keeps the rest.
    let Some(login) = data["viewer"]["login"].as_str() else {
        return Err(error.unwrap_or_else(|| "GitHub's reply had no viewer".into()));
    };
    let list = |key: &str, reason: Reason| -> Vec<ReviewPr> {
        data[key]["nodes"].as_array().map(|a| a.iter().filter_map(|p| parse_node(p, reason, login)).collect()).unwrap_or_default()
    };
    // A pending request after a review means the author asked again: the
    // old verdict is not this round's, so the PR is to review.
    let requested: Vec<ReviewPr> = list("requested", Reason::Review).into_iter().map(|p| ReviewPr { my_review: None, ..p }).collect();
    let review = merge(requested, list("reviewed", Reason::Review));
    Ok((merge(review, list("assigned", Reason::Assigned)), error))
}

/// One list from two, de-duplicated by URL with reasons combined, newest first.
pub fn merge(a: Vec<ReviewPr>, b: Vec<ReviewPr>) -> Vec<ReviewPr> {
    let mut out: Vec<ReviewPr> = Vec::new();
    for pr in a.into_iter().chain(b) {
        match out.iter_mut().find(|x| x.url == pr.url) {
            Some(existing) => {
                for r in pr.reasons {
                    if !existing.reasons.contains(&r) {
                        existing.reasons.push(r);
                    }
                }
            }
            None => out.push(pr),
        }
    }
    out.sort_by(|x, y| y.updated_at.cmp(&x.updated_at));
    out
}

/// One forge's answer to "what is waiting on me".
pub struct Fetched {
    pub forge: GitProvider,
    pub result: Result<Vec<ReviewPr>, String>,
}

/// Asks every forge at once.
pub fn fetch(forges: &[Forge], http: &dyn Http) -> Vec<Fetched> {
    std::thread::scope(|s| {
        let handles: Vec<_> = forges.iter().map(|f| (f.site.provider, s.spawn(move || crate::forge::review_prs(http, f)))).collect();
        handles.into_iter().map(|(forge, h)| Fetched { forge, result: h.join().unwrap_or_else(|_| Err("the check stopped unexpectedly".into())) }).collect()
    })
}

/// The new list and the error lines: a forge that answered replaces its
/// PRs, one that failed keeps its last ones and adds a line, and a forge
/// that was not asked (off, or no token) drops out. `errors` holds the
/// lines for forges that were not asked.
pub fn combine(previous: &[ReviewPr], fetched: Vec<Fetched>, mut errors: Vec<ForgeError>) -> (Vec<ReviewPr>, Vec<ForgeError>) {
    let mut out = Vec::new();
    for f in fetched {
        match f.result {
            Ok(prs) => out = merge(out, prs),
            Err(e) => {
                out = merge(out, previous.iter().filter(|p| p.forge == f.forge).cloned().collect());
                errors.push(ForgeError { forge: f.forge, message: format!("{} check failed: {e}", f.forge.label()) });
            }
        }
    }
    (out, errors)
}

/// Remembers the review state of each assigned PR so a turn to approved or
/// changes requested is announced once. A forge's first answer only primes
/// it: what was already decided when Maya starts, or when a forge is first
/// switched on or given a token, is not news.
#[derive(Default)]
pub struct PrNotifier {
    last: std::collections::HashMap<String, Option<ReviewMark>>,
    /// Forges that have answered once: what they held then is not news.
    primed: std::collections::HashSet<GitProvider>,
}

impl PrNotifier {
    /// The assigned PRs that became approved or changes requested since the last call.
    /// `answered` lists the forges whose fetch succeeded this round.
    pub fn take_changes(&mut self, prs: &[ReviewPr], answered: &[GitProvider]) -> Vec<ReviewPr> {
        let mut out = Vec::new();
        for p in prs.iter().filter(|p| p.reasons.contains(&Reason::Assigned)) {
            let before = self.last.insert(p.url.clone(), p.pr_review).flatten();
            if self.primed.contains(&p.forge) && matches!(p.pr_review, Some(ReviewMark::Approved | ReviewMark::ChangesRequested)) && p.pr_review != before {
                out.push(p.clone());
            }
        }
        self.primed.extend(answered.iter().copied());
        out
    }
}

fn short_title(pr: &ReviewPr) -> String {
    format!("{} {}{}", pr.repo_name(), pr.forge.sigil(), pr.number)
}

/// A PR review banner: `repo #n`, `Approved by jane`, the PR's title.
pub fn pr_banner(pr: &ReviewPr) -> (String, String, String) {
    let what = match pr.pr_review {
        Some(ReviewMark::Approved) => "Approved",
        Some(ReviewMark::ChangesRequested) => "Changes requested",
        Some(ReviewMark::Commented) => "Commented",
        None => "Reviewed",
    };
    let subtitle = if pr.pr_reviewers.is_empty() { what.to_string() } else { format!("{what} by {}", pr.pr_reviewers.join(", ")) };
    (short_title(pr), subtitle, pr.title.trim().to_string())
}

/// What Maya says when an assigned PR is approved or needs changes.
pub fn pr_spoken_line(pr: &ReviewPr) -> Option<String> {
    let name = match pr.forge {
        GitProvider::Github => crate::notify::spoken_name(&format!("{} {}", pr.repo_name(), pr.number)),
        GitProvider::Gitlab => crate::notify::spoken_name(&format!("{} merge request {}", pr.repo_name(), pr.number)),
    };
    match pr.pr_review {
        Some(ReviewMark::Approved) => Some(format!("{name} was approved")),
        Some(ReviewMark::ChangesRequested) => Some(format!("{name} needs changes")),
        _ => None,
    }
}

fn is_checkout_of(dir: &Path, site: &ForgeSite, repo: &str) -> bool {
    crate::forge::origin_of(dir).and_then(|o| crate::forge::route(&o, std::slice::from_ref(site)).map(|(_, p)| p)).is_some_and(|p| p.eq_ignore_ascii_case(repo))
}

/// A checkout of `repo` on `site` under `projects_root`: the folder named
/// after the repo when it is one, else the first match by folder name.
pub fn find_checkout(projects_root: &Path, site: &ForgeSite, repo: &str) -> Option<PathBuf> {
    let Ok(entries) = std::fs::read_dir(projects_root) else { return None };
    let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir() && p.join(".git").exists()).collect();
    dirs.sort();
    let named = projects_root.join(repo.rsplit('/').next().unwrap_or(repo));
    if dirs.contains(&named) && is_checkout_of(&named, site, repo) {
        return Some(named);
    }
    dirs.into_iter().find(|d| is_checkout_of(d, site, repo))
}

/// True when any live session's working directory is `dir` or inside it.
pub fn is_busy(dir: &Path, live_cwds: &[String]) -> bool {
    let d = dir.to_string_lossy();
    let prefix = format!("{}/", d.trim_end_matches('/'));
    live_cwds.iter().any(|c| c == d.as_ref() || c.starts_with(&prefix))
}

/// Where a review runs and whether it must be cloned first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewTarget {
    pub dir: PathBuf,
    pub clone: bool,
}

/// The free project checkout if there is one; else `<clones>/<repo>-<number>`
/// (`<repo>-mr<number>` on GitLab, so `!12` never reuses GitHub's `#12`),
/// cloned unless it already exists.
pub fn resolve_target(projects_root: &Path, clones_root: &Path, site: &ForgeSite, repo: &str, number: u64, live_cwds: &[String]) -> ReviewTarget {
    if let Some(dir) = find_checkout(projects_root, site, repo) {
        if !is_busy(&dir, live_cwds) {
            return ReviewTarget { dir, clone: false };
        }
    }
    let repo_name = repo.rsplit('/').next().unwrap_or(repo);
    let marker = if site.provider == GitProvider::Gitlab { "mr" } else { "" };
    let dir = clones_root.join(format!("{repo_name}-{marker}{number}"));
    let clone = !dir.is_dir();
    ReviewTarget { dir, clone }
}

/// The name a review session is started with, so the board can tie it back
/// to its PR: `review <repo> #<number>`, or `review <repo> !<number>` on GitLab.
pub fn session_name(forge: GitProvider, name_with_owner: &str, number: u64) -> String {
    let repo_name = name_with_owner.rsplit('/').next().unwrap_or(name_with_owner);
    format!("review {repo_name} {}{number}", forge.sigil())
}

/// The prompt a review session starts with: the template with `{number}`,
/// `{repo}` and `{url}` filled in. A blank template means the built-in one;
/// a template that names none of the three gets ` PR #<number> (<url>)`
/// appended, so a bare slash command still says which pull request.
pub const DEFAULT_REVIEW_PROMPT: &str = "Review pull request #{number} of {repo} ({url}). Make use of code review skills, and at the end give at most 3 options: Approve; Ask <questions here>; Request changes <changes here>. Mark the recommended option \"(Recommended)\". Show Ask and Request changes only when they are needed.";

pub fn render_prompt(template: &str, pr: &ReviewPr) -> String {
    let t = template.trim();
    let t = if t.is_empty() { DEFAULT_REVIEW_PROMPT } else { t };
    let number = pr.number.to_string();
    if ["{number}", "{repo}", "{url}"].iter().any(|p| t.contains(p)) {
        t.replace("{number}", &number).replace("{repo}", &pr.repo).replace("{url}", &pr.url)
    } else {
        format!("{t} PR #{number} ({})", pr.url)
    }
}

/// The shell line the review terminal runs: a `git clone` first when needed, with the user's own git credentials, then
/// the agent in the folder on the prompt file, as a new session named
/// `review <repo> #<number>` (`-n` for Claude Code; the others get the name
/// typed later, as `actions::start_session` does).
pub fn shell_command(agent: Harness, target: &ReviewTarget, clone_url: &str, session: &str, prompt_file: &Path, grok_id: Option<&str>) -> String {
    let opts = LaunchOptions { agent, name: Some(session.to_string()), ..Default::default() };
    let start = crate::launch::session_command(&target.dir, prompt_file, &opts, grok_id);
    if target.clone {
        let dir = shell_single_quote(&target.dir.to_string_lossy());
        let parent = shell_single_quote(&target.dir.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default());
        format!("mkdir -p {parent} && git clone {} {dir} && {start}", shell_single_quote(clone_url))
    } else {
        start
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GitProvider;
    use std::path::Path;

    /// `parse_graphql`'s list, without the partial error.
    fn parse(json: &str) -> Result<Vec<ReviewPr>, String> {
        parse_graphql(json).map(|(prs, _)| prs)
    }

    fn node(number: u64, repo: &str, author: &str, updated: &str, opinionated: &str, latest: &str) -> String {
        format!(
            r#"{{"number":{number},"title":"t{number}","url":"https://github.com/{repo}/pull/{number}","isDraft":false,"updatedAt":"{updated}","repository":{{"nameWithOwner":"{repo}"}},"author":{{"login":"{author}"}},"latestOpinionatedReviews":{{"nodes":[{opinionated}]}},"latestReviews":{{"nodes":[{latest}]}}}}"#
        )
    }

    #[test]
    fn the_clone_addresses_come_from_the_repository() {
        let with = r#"{"number":1,"title":"t","url":"https://github.com/o/r/pull/1","isDraft":false,"updatedAt":"","repository":{"nameWithOwner":"o/r","sshUrl":"git@github.com:o/r.git","url":"https://github.com/o/r"},"author":{"login":"jane"}}"#;
        let p = parse_node(&serde_json::from_str(with).unwrap(), Reason::Review, "me").unwrap();
        assert_eq!((p.ssh_url.as_deref(), p.https_url.as_deref()), (Some("git@github.com:o/r.git"), Some("https://github.com/o/r.git")));
        let without = node(2, "o/r", "jane", "", "", "");
        let p = parse_node(&serde_json::from_str(&without).unwrap(), Reason::Review, "me").unwrap();
        assert_eq!((p.ssh_url, p.https_url), (None, None));
    }

    #[test]
    fn the_query_asks_for_the_clone_addresses() {
        assert!(QUERY.contains("repository { nameWithOwner sshUrl url }"));
    }

    fn review(login: &str, state: &str) -> String {
        format!(r#"{{"state":"{state}","author":{{"__typename":"User","login":"{login}"}}}}"#)
    }

    fn bot(login: &str, state: &str) -> String {
        format!(r#"{{"state":"{state}","author":{{"__typename":"Bot","login":"{login}"}}}}"#)
    }

    fn response(requested: &[String], reviewed: &[String], assigned: &[String]) -> String {
        format!(
            r#"{{"data":{{"viewer":{{"login":"Me"}},"requested":{{"nodes":[{}]}},"reviewed":{{"nodes":[{}]}},"assigned":{{"nodes":[{}]}}}}}}"#,
            requested.join(","),
            reviewed.join(","),
            assigned.join(",")
        )
    }

    #[test]
    fn parses_the_graphql_searches_into_one_list_with_reasons_newest_first() {
        let pending = node(451, "Org/bedrock", "jane", "2026-09-28T10:00:00Z", "", "");
        let both = node(3, "Org/plugins", "bob", "2026-09-29T09:00:00Z", "", "");
        let json = response(&[pending.clone(), both.clone()], &[], &[both]);
        let prs = parse(&json).unwrap();
        assert_eq!(prs.iter().map(|p| p.number).collect::<Vec<_>>(), vec![3, 451]);
        let p = &prs[1];
        assert_eq!((p.number, p.repo.as_str(), p.title.as_str(), p.author.as_str(), p.is_draft), (451, "Org/bedrock", "t451", "jane", false));
        assert_eq!(p.url, "https://github.com/Org/bedrock/pull/451");
        assert_eq!(p.reasons, vec![Reason::Review]);
        assert_eq!(p.my_review, None, "not reviewed yet");
        assert_eq!(prs[0].reasons, vec![Reason::Review, Reason::Assigned]);
        assert!(parse("not json").is_err());
        assert!(parse(r#"{"errors":[{"message":"bad query"}]}"#).unwrap_err().contains("bad query"));
    }

    #[test]
    fn a_pr_the_user_reviewed_stays_under_review_requested() {
        let done = node(9, "Org/x", "jane", "2026-09-28T10:00:00Z", &review("me", "APPROVED"), &review("me", "APPROVED"));
        let prs = parse(&response(&[], &[done], &[])).unwrap();
        assert_eq!(prs[0].reasons, vec![Reason::Review]);
        assert_eq!(prs[0].my_review, Some(ReviewMark::Approved), "logins match case-insensitively");
    }

    #[test]
    fn a_pending_request_means_to_review_whatever_the_user_said_before() {
        let again = node(8, "Org/x", "jane", "2026-09-28T10:00:00Z", &review("me", "CHANGES_REQUESTED"), &review("me", "CHANGES_REQUESTED"));
        let prs = parse(&response(&[again.clone()], &[again], &[])).unwrap();
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].my_review, None, "the author asked again: the old verdict is not this round's review");
    }

    #[test]
    fn partial_results_are_kept_when_github_also_reports_an_error() {
        let ok = node(1, "o/r", "jane", "", "", "");
        let json = response(&[ok], &[], &[]).replacen("{\"data\"", "{\"errors\":[{\"message\":\"SAML enforcement\"}],\"data\"", 1);
        let (prs, partial) = parse_graphql(&json).unwrap();
        assert_eq!(prs.len(), 1);
        assert_eq!(partial.as_deref(), Some("GitHub said: SAML enforcement"), "for the caller to log scrubbed");
    }

    #[test]
    fn the_searches_keep_the_most_recently_updated() {
        assert_eq!(QUERY.matches("sort:updated-desc").count(), 3);
    }

    #[test]
    fn the_users_review_prefers_a_verdict_over_a_comment_and_ignores_others() {
        let v = |opinionated: &str, latest: &str| {
            let n = node(1, "o/r", "jane", "", opinionated, latest);
            parse(&response(&[], &[n], &[])).unwrap()[0].my_review
        };
        assert_eq!(v(&review("Me", "APPROVED"), &review("Me", "COMMENTED")), Some(ReviewMark::Approved));
        assert_eq!(v(&review("Me", "CHANGES_REQUESTED"), &review("Me", "CHANGES_REQUESTED")), Some(ReviewMark::ChangesRequested));
        assert_eq!(v("", &review("Me", "COMMENTED")), Some(ReviewMark::Commented));
        assert_eq!(v(&review("Me", "DISMISSED"), &review("Me", "DISMISSED")), None, "a dismissed verdict is no review");
        assert_eq!(v(&review("Me", "DISMISSED"), &review("Me", "COMMENTED")), Some(ReviewMark::Commented));
        let others = format!("{},{}", review("copilot-pull-request-reviewer", "COMMENTED"), review("bob", "APPROVED"));
        assert_eq!(v(&review("bob", "APPROVED"), &others), None, "someone else's review is not the user's");
    }

    #[test]
    fn the_users_own_pr_has_no_review_state() {
        let own = node(5, "o/r", "me", "", "", &review("Me", "COMMENTED"));
        let prs = parse(&response(&[], &[], &[own])).unwrap();
        assert_eq!(prs[0].reasons, vec![Reason::Assigned]);
        assert_eq!(prs[0].my_review, None, "replying on your own PR is not reviewing it");
    }

    use crate::forge::{ForgeError, ForgeSite};

    fn github_site() -> ForgeSite {
        ForgeSite::of(&crate::config::GitConnection::default_for(GitProvider::Github))
    }

    #[test]
    fn find_checkout_matches_a_gitlab_checkout_through_its_site() {
        let t = tempfile::tempdir().unwrap();
        let gl = ForgeSite::of(&crate::config::GitConnection { base_url: Some("https://gitlab.acme.io".into()), enabled: true, ..crate::config::GitConnection::default_for(GitProvider::Gitlab) });
        let r = repo_with_origin(t.path(), "r", "git@gitlab.acme.io:grp/sub/r.git");
        assert_eq!(find_checkout(t.path(), &gl, "grp/sub/r"), Some(r));
        assert_eq!(find_checkout(t.path(), &github_site(), "grp/sub/r"), None, "the same path on GitHub is another repo");
    }

    #[test]
    fn combine_keeps_a_failing_forges_last_prs_and_says_why() {
        let gh_old = ReviewPr { number: 1, url: "gh1".into(), updated_at: "2026-10-01".into(), ..pr() };
        let gl_old = ReviewPr { number: 2, url: "gl2".into(), forge: GitProvider::Gitlab, updated_at: "2026-10-02".into(), ..pr() };
        let gh_new = ReviewPr { number: 3, url: "gh3".into(), updated_at: "2026-10-03".into(), ..pr() };
        let fetched = vec![Fetched { forge: GitProvider::Github, result: Ok(vec![gh_new]) }, Fetched { forge: GitProvider::Gitlab, result: Err("401".into()) }];
        let (prs, errors) = combine(&[gh_old, gl_old], fetched, vec![]);
        assert_eq!(prs.iter().map(|p| p.number).collect::<Vec<_>>(), vec![3, 2], "GitHub's list is replaced, GitLab's kept");
        assert_eq!(errors, vec![ForgeError { forge: GitProvider::Gitlab, message: "GitLab check failed: 401".into() }]);
    }

    #[test]
    fn combine_drops_a_forge_that_was_not_asked_and_passes_its_error_on() {
        let gl_old = ReviewPr { forge: GitProvider::Gitlab, ..pr() };
        let missing = vec![ForgeError { forge: GitProvider::Gitlab, message: "No GitLab token: paste one in Settings → Git.".into() }];
        let (prs, errors) = combine(&[gl_old], vec![], missing.clone());
        assert!(prs.is_empty());
        assert_eq!(errors, missing);
    }

    #[test]
    fn fetch_asks_every_forge() {
        let gh = crate::forge::Forge { site: github_site(), token: "t".into() };
        let body = r#"{"data":{"viewer":{"login":"Me"},"requested":{"nodes":[]},"reviewed":{"nodes":[]},"assigned":{"nodes":[]}}}"#;
        let http = crate::forge::fake::FakeHttp::new(vec![("/graphql", 200, body)]);
        let out = fetch(&[gh], &http);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].forge, out[0].result.clone()), (GitProvider::Github, Ok(vec![])));
    }

    fn repo_with_origin(root: &Path, name: &str, origin: &str) -> std::path::PathBuf {
        let d = root.join(name);
        std::fs::create_dir_all(&d).unwrap();
        let ok = std::process::Command::new("git").args(["-C", d.to_str().unwrap(), "init", "-q"]).status().unwrap().success();
        assert!(ok);
        let ok = std::process::Command::new("git").args(["-C", d.to_str().unwrap(), "remote", "add", "origin", origin]).status().unwrap().success();
        assert!(ok);
        d
    }

    #[test]
    fn find_checkout_prefers_the_folder_named_after_the_repo() {
        let t = tempfile::tempdir().unwrap();
        let alt = repo_with_origin(t.path(), "bouncer-bnc-0011", "git@github.com:Org/bouncer.git");
        let main = repo_with_origin(t.path(), "bouncer", "https://github.com/Org/bouncer");
        repo_with_origin(t.path(), "other", "git@github.com:Org/other.git");
        std::fs::create_dir_all(t.path().join("plain-folder")).unwrap();
        assert_eq!(find_checkout(t.path(), &github_site(), "Org/bouncer"), Some(main));
        std::fs::remove_dir_all(t.path().join("bouncer")).unwrap();
        assert_eq!(find_checkout(t.path(), &github_site(), "Org/bouncer"), Some(alt));
        assert_eq!(find_checkout(t.path(), &github_site(), "Org/missing"), None);
    }

    #[test]
    fn a_checkout_is_busy_when_a_live_session_works_inside_it() {
        let live = vec!["/Users/x/dev/bouncer/packages/api".to_string(), "/Users/x/dev/other".to_string()];
        assert!(is_busy(Path::new("/Users/x/dev/bouncer"), &live));
        assert!(is_busy(Path::new("/Users/x/dev/other"), &live));
        assert!(!is_busy(Path::new("/Users/x/dev/bounce"), &live), "a prefix of the name is not inside it");
        assert!(!is_busy(Path::new("/Users/x/dev/free"), &live));
    }

    #[test]
    fn resolve_uses_a_free_checkout_else_a_clone_dir_reused_when_present() {
        let t = tempfile::tempdir().unwrap();
        let projects = t.path().join("dev");
        let clones = t.path().join("reviews");
        let main = repo_with_origin(&projects, "bedrock", "git@github.com:Org/bedrock.git");
        let free = resolve_target(&projects, &clones, &github_site(), "Org/bedrock", 451, &[]);
        assert_eq!(free, ReviewTarget { dir: main.clone(), clone: false });
        let busy = resolve_target(&projects, &clones, &github_site(), "Org/bedrock", 451, &[main.to_string_lossy().into_owned()]);
        assert_eq!(busy, ReviewTarget { dir: clones.join("bedrock-451"), clone: true });
        std::fs::create_dir_all(clones.join("bedrock-451")).unwrap();
        let again = resolve_target(&projects, &clones, &github_site(), "Org/bedrock", 451, &[main.to_string_lossy().into_owned()]);
        assert_eq!(again, ReviewTarget { dir: clones.join("bedrock-451"), clone: false }, "an existing clone is reused");
        let none = resolve_target(&projects, &clones, &github_site(), "Org/nowhere", 7, &[]);
        assert_eq!(none, ReviewTarget { dir: clones.join("nowhere-7"), clone: true });
    }

    #[test]
    fn a_gitlab_clone_folder_is_named_for_the_mr_and_never_reuses_githubs() {
        let t = tempfile::tempdir().unwrap();
        let (projects, clones) = (t.path().join("dev"), t.path().join("reviews"));
        std::fs::create_dir_all(clones.join("r-12")).unwrap();
        let gl = ForgeSite::of(&crate::config::GitConnection { base_url: Some("https://gitlab.acme.io".into()), enabled: true, ..crate::config::GitConnection::default_for(GitProvider::Gitlab) });
        let mr = resolve_target(&projects, &clones, &gl, "grp/r", 12, &[]);
        assert_eq!(mr, ReviewTarget { dir: clones.join("r-mr12"), clone: true }, "GitHub's r-12 is another repo's");
        let gh = resolve_target(&projects, &clones, &github_site(), "Org/r", 12, &[]);
        assert_eq!(gh, ReviewTarget { dir: clones.join("r-12"), clone: false });
    }

    #[test]
    fn review_session_name_is_predictable() {
        assert_eq!(session_name(GitProvider::Github, "Org/bedrock", 451), "review bedrock #451");
        assert_eq!(session_name(GitProvider::Gitlab, "grp/sub/bedrock", 12), "review bedrock !12");
    }

    /// The PR's review state as seen by viewer `Me` on a PR by `author`.
    fn pr_state(author: &str, opinionated: &[String], latest: &[String]) -> (Option<ReviewMark>, Vec<String>) {
        let n = node(1, "o/r", author, "", &opinionated.join(","), &latest.join(","));
        let p = parse(&response(&[], &[], &[n])).unwrap().remove(0);
        (p.pr_review, p.pr_reviewers)
    }

    #[test]
    fn the_prs_review_state_comes_from_other_people_changes_first() {
        let none: Vec<String> = vec![];
        assert_eq!(pr_state("me", &[review("jane", "APPROVED")], &[review("jane", "APPROVED")]), (Some(ReviewMark::Approved), vec!["jane".into()]));
        let split = [review("jane", "APPROVED"), review("bob", "CHANGES_REQUESTED"), review("al", "CHANGES_REQUESTED")];
        assert_eq!(pr_state("me", &split, &split), (Some(ReviewMark::ChangesRequested), vec!["bob".into(), "al".into()]), "changes requested wins");
        assert_eq!(pr_state("me", &[], &[review("jane", "COMMENTED")]), (Some(ReviewMark::Commented), vec!["jane".into()]));
        assert_eq!(pr_state("me", &[review("jane", "DISMISSED")], &[review("jane", "DISMISSED")]), (None, vec![]), "a dismissed verdict counts for nothing");
        assert_eq!(pr_state("me", &none, &none), (None, vec![]));
    }

    #[test]
    fn the_prs_review_state_leaves_out_bots_the_user_and_the_author() {
        let bots = [bot("copilot-pull-request-reviewer", "COMMENTED"), bot("dependabot", "APPROVED")];
        assert_eq!(pr_state("me", &bots, &bots), (None, vec![]), "Copilot comments on every PR");
        let mine = [review("Me", "APPROVED")];
        assert_eq!(pr_state("jane", &mine, &mine), (None, vec![]), "the user's own review is my_review, not the PR's");
        assert_eq!(pr_state("jane", &[], &[review("jane", "COMMENTED")]), (None, vec![]), "the author replying to threads is not a review");
    }

    fn assigned(number: u64, mark: Option<ReviewMark>) -> ReviewPr {
        ReviewPr { number, url: format!("https://github.com/o/r/pull/{number}"), reasons: vec![Reason::Assigned], pr_review: mark, ..pr() }
    }

    #[test]
    fn the_notifier_announces_an_assigned_pr_turning_approved_or_changes_requested_once() {
        use ReviewMark::*;
        let mut n = PrNotifier::default();
        let nums = |v: Vec<ReviewPr>| v.iter().map(|p| p.number).collect::<Vec<_>>();
        assert_eq!(nums(n.take_changes(&[assigned(1, Some(Approved)), assigned(2, None)], &[GitProvider::Github])), Vec::<u64>::new(), "the first poll only primes");
        assert_eq!(nums(n.take_changes(&[assigned(1, Some(Approved)), assigned(2, None)], &[GitProvider::Github])), Vec::<u64>::new(), "no change, no news");
        assert_eq!(nums(n.take_changes(&[assigned(1, Some(Approved)), assigned(2, Some(Commented))], &[GitProvider::Github])), Vec::<u64>::new(), "a comment is not announced");
        assert_eq!(nums(n.take_changes(&[assigned(1, Some(ChangesRequested)), assigned(2, Some(Approved))], &[GitProvider::Github])), vec![1, 2]);
        assert_eq!(nums(n.take_changes(&[assigned(1, Some(ChangesRequested)), assigned(2, Some(Approved))], &[GitProvider::Github])), Vec::<u64>::new(), "once each");
        assert_eq!(nums(n.take_changes(&[assigned(2, Some(Approved))], &[GitProvider::Github])), Vec::<u64>::new());
        assert_eq!(nums(n.take_changes(&[assigned(1, Some(ChangesRequested)), assigned(2, Some(Approved))], &[GitProvider::Github])), Vec::<u64>::new(), "a PR that left the list and came back unchanged is not news");
        assert_eq!(nums(n.take_changes(&[assigned(3, Some(Approved))], &[GitProvider::Github])), vec![3], "newly assigned and already approved is news");
        let requested_only = ReviewPr { reasons: vec![Reason::Review], ..assigned(4, Some(Approved)) };
        assert_eq!(nums(n.take_changes(&[requested_only], &[GitProvider::Github])), Vec::<u64>::new(), "only assigned PRs are announced");
    }

    #[test]
    fn a_forge_is_primed_by_its_own_first_answer() {
        use ReviewMark::*;
        let mr = |n, r| ReviewPr { forge: GitProvider::Gitlab, url: format!("https://gitlab.com/g/p/-/merge_requests/{n}"), ..assigned(n, r) };
        let nums = |v: Vec<ReviewPr>| v.iter().map(|p| p.number).collect::<Vec<_>>();
        let gh = [GitProvider::Github];
        let mut n = PrNotifier::default();
        n.take_changes(&[], &gh);
        assert_eq!(nums(n.take_changes(&[mr(7, Some(Approved))], &[GitProvider::Github, GitProvider::Gitlab])), Vec::<u64>::new(), "GitLab's first answer only primes it");
        assert_eq!(nums(n.take_changes(&[mr(7, Some(ChangesRequested))], &[GitProvider::Github, GitProvider::Gitlab])), vec![7]);
    }

    #[test]
    fn a_forge_that_failed_is_not_primed() {
        use ReviewMark::*;
        let mut n = PrNotifier::default();
        n.take_changes(&[], &[]);
        n.take_changes(&[assigned(1, Some(Approved))], &[]);
        let out = n.take_changes(&[assigned(1, Some(ChangesRequested))], &[GitProvider::Github]);
        assert!(out.is_empty(), "the first answer primes, even after failed rounds");
    }

    #[test]
    fn a_pr_review_banner_and_line_name_the_pr_and_the_reviewers() {
        let p = ReviewPr { pr_review: Some(ReviewMark::Approved), pr_reviewers: vec!["jane".into()], ..pr() };
        assert_eq!(pr_banner(&p), ("bedrock #451".into(), "Approved by jane".into(), "docs: notes".into()));
        assert_eq!(pr_spoken_line(&p).as_deref(), Some("bedrock 451 was approved"));
        let p = ReviewPr { pr_review: Some(ReviewMark::ChangesRequested), pr_reviewers: vec!["bob".into(), "al".into()], ..pr() };
        assert_eq!(pr_banner(&p).1, "Changes requested by bob, al");
        assert_eq!(pr_spoken_line(&p).as_deref(), Some("bedrock 451 needs changes"));
        assert_eq!(pr_spoken_line(&ReviewPr { pr_review: Some(ReviewMark::Commented), ..pr() }), None);
    }

    #[test]
    fn a_merge_request_is_named_with_a_bang_and_spoken_plainly() {
        let mr = ReviewPr { forge: GitProvider::Gitlab, repo: "grp/sub/bedrock".into(), number: 12, pr_review: Some(ReviewMark::Approved), pr_reviewers: vec!["jane".into()], ..pr() };
        assert_eq!(mr.reference(), "grp/sub/bedrock!12");
        assert_eq!(pr().reference(), "Org/bedrock#451");
        assert_eq!(pr_banner(&mr).0, "bedrock !12");
        assert_eq!(pr_spoken_line(&mr).as_deref(), Some("bedrock merge request 12 was approved"));
    }

    #[test]
    fn graphql_prs_are_github_prs() {
        let prs = parse(&response(&[node(1, "o/r", "jane", "", "", "")], &[], &[])).unwrap();
        assert_eq!(prs[0].forge, GitProvider::Github);
    }

    fn pr() -> ReviewPr {
        ReviewPr { number: 451, forge: GitProvider::Github, repo: "Org/bedrock".into(), title: "docs: notes".into(), author: "jane".into(), url: "https://github.com/Org/bedrock/pull/451".into(), is_draft: false, updated_at: "".into(), reasons: vec![Reason::Review], my_review: None, pr_review: None, pr_reviewers: vec![], ssh_url: None, https_url: None }
    }

    #[test]
    fn the_review_prompt_fills_placeholders_or_appends_the_pr() {
        let d = render_prompt("", &pr());
        assert!(d.starts_with("Review pull request #451 of Org/bedrock (https://github.com/Org/bedrock/pull/451)."), "{d}");
        assert!(d.contains("(Recommended)") && d.contains("Request changes"), "{d}");
        assert_eq!(render_prompt("  \n", &pr()), d, "blank means the default");
        assert_eq!(render_prompt("/should-i-approve PR #{number}", &pr()), "/should-i-approve PR #451");
        assert_eq!(render_prompt("Look at {url} ({repo})", &pr()), "Look at https://github.com/Org/bedrock/pull/451 (Org/bedrock)");
        assert_eq!(render_prompt("/should-i-approve", &pr()), "/should-i-approve PR #451 (https://github.com/Org/bedrock/pull/451)");
    }

    #[test]
    fn the_review_shell_line_runs_the_agent_on_the_prompt_file_with_the_review_name() {
        let file = Path::new("/Users/x/.claude/maya/prompts/1.txt");
        let free = ReviewTarget { dir: PathBuf::from("/Users/x/dev/bedrock"), clone: false };
        let s = shell_command(crate::model::Harness::ClaudeCode, &free, "https://github.com/Org/bedrock.git", "review bedrock #451", file, None);
        assert_eq!(s, "cd '/Users/x/dev/bedrock' && p=\"$(cat '/Users/x/.claude/maya/prompts/1.txt')\" && rm -f '/Users/x/.claude/maya/prompts/1.txt' && claude -n 'review bedrock #451' -- \"$p\"");
        let clone = ReviewTarget { dir: PathBuf::from("/Users/x/dev/reviews/bedrock-451"), clone: true };
        let s = shell_command(crate::model::Harness::Codex, &clone, "https://github.com/Org/bedrock.git", "review bedrock #451", file, None);
        assert!(s.starts_with("mkdir -p '/Users/x/dev/reviews' && git clone 'https://github.com/Org/bedrock.git' '/Users/x/dev/reviews/bedrock-451' && "), "{s}");
        assert!(s.ends_with("&& codex -- \"$p\""), "no -n for Codex: the name waits as a pending name: {s}");
        let s = shell_command(crate::model::Harness::Grok, &free, "https://github.com/Org/bedrock.git", "review bedrock #451", file, Some("1111-2222"));
        assert!(s.ends_with("&& grok --session-id '1111-2222' -- \"$p\""), "{s}");
        let s = shell_command(crate::model::Harness::Antigravity, &free, "https://github.com/Org/bedrock.git", "review bedrock #451", file, None);
        assert!(s.ends_with("&& agy --prompt-interactive=\"$p\""), "{s}");
    }
}
