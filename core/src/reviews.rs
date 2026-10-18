use crate::launch::{shell_single_quote, LaunchOptions};
use crate::model::Harness;
use crate::config::GitProvider;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Stdio;

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
  repository { nameWithOwner }
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
    })
}

/// `gh api graphql` output of `QUERY` as one list of review PRs.
pub fn parse_graphql(json: &str) -> Result<Vec<ReviewPr>, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("gh output was not JSON: {e}"))?;
    let error = v["errors"].as_array().and_then(|e| e.first()).and_then(|e| e["message"].as_str()).map(|m| format!("GitHub said: {m}"));
    let data = &v["data"];
    // An org the token cannot reach adds an error but keeps the rest.
    let Some(login) = data["viewer"]["login"].as_str() else {
        return Err(error.unwrap_or_else(|| "gh output had no viewer".into()));
    };
    if let Some(e) = &error {
        crate::log::line("app", format!("pull requests: partial results: {e}"));
    }
    let list = |key: &str, reason: Reason| -> Vec<ReviewPr> {
        data[key]["nodes"].as_array().map(|a| a.iter().filter_map(|p| parse_node(p, reason, login)).collect()).unwrap_or_default()
    };
    // A pending request after a review means the author asked again: the
    // old verdict is not this round's, so the PR is to review.
    let requested: Vec<ReviewPr> = list("requested", Reason::Review).into_iter().map(|p| ReviewPr { my_review: None, ..p }).collect();
    let review = merge(requested, list("reviewed", Reason::Review));
    Ok(merge(review, list("assigned", Reason::Assigned)))
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

/// Open PRs where the user is a requested reviewer, has reviewed, or is an assignee.
pub fn fetch() -> Result<Vec<ReviewPr>, String> {
    let out = crate::gh()
        .args(["api", "graphql", "-f", &format!("query={QUERY}")])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run gh: {e}"))?;
    // gh exits non-zero on any GraphQL error, partial results included.
    match parse_graphql(&String::from_utf8_lossy(&out.stdout)) {
        Ok(prs) => Ok(prs),
        Err(_) if !out.status.success() => Err(format!("gh failed: {}", String::from_utf8_lossy(&out.stderr).trim())),
        Err(e) => Err(e),
    }
}

/// Remembers the review state of each assigned PR so a turn to approved or
/// changes requested is announced once. The first poll only primes it:
/// what was already decided when Maya starts is not news.
#[derive(Default)]
pub struct PrNotifier {
    last: std::collections::HashMap<String, Option<ReviewMark>>,
    primed: bool,
}

impl PrNotifier {
    /// The assigned PRs that became approved or changes requested since the last call.
    pub fn take_changes(&mut self, prs: &[ReviewPr]) -> Vec<ReviewPr> {
        let mut out = Vec::new();
        for p in prs.iter().filter(|p| p.reasons.contains(&Reason::Assigned)) {
            let before = self.last.insert(p.url.clone(), p.pr_review).flatten();
            if self.primed && matches!(p.pr_review, Some(ReviewMark::Approved | ReviewMark::ChangesRequested)) && p.pr_review != before {
                out.push(p.clone());
            }
        }
        self.primed = true;
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

/// True when a git remote URL (SSH or HTTPS) points at `owner/name` on GitHub.
pub fn remote_matches(remote: &str, name_with_owner: &str) -> bool {
    let r = remote.trim().trim_end_matches('/');
    let r = r.strip_suffix(".git").unwrap_or(r);
    let path = if let Some(p) = r.strip_prefix("git@github.com:") {
        p
    } else if let Some(p) = r.strip_prefix("https://github.com/") {
        p
    } else if let Some(p) = r.strip_prefix("ssh://git@github.com/") {
        p
    } else {
        return false;
    };
    path.eq_ignore_ascii_case(name_with_owner)
}

fn origin_of(dir: &Path) -> Option<String> {
    let out = crate::command("git").args(["-C", dir.to_str()?, "config", "--get", "remote.origin.url"]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}

/// A checkout of `name_with_owner` under `projects_root`: the folder named
/// after the repo when it is one, else the first match by folder name.
pub fn find_checkout(projects_root: &Path, name_with_owner: &str) -> Option<PathBuf> {
    let Ok(entries) = std::fs::read_dir(projects_root) else { return None };
    let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir() && p.join(".git").exists()).collect();
    dirs.sort();
    let repo_name = name_with_owner.rsplit('/').next().unwrap_or(name_with_owner);
    let named = projects_root.join(repo_name);
    if dirs.contains(&named) && origin_of(&named).map_or(false, |o| remote_matches(&o, name_with_owner)) {
        return Some(named);
    }
    dirs.into_iter().find(|d| origin_of(d).map_or(false, |o| remote_matches(&o, name_with_owner)))
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

/// The free project checkout if there is one; else `<clones>/<repo>-<number>`,
/// cloned unless it already exists.
pub fn resolve_target(projects_root: &Path, clones_root: &Path, name_with_owner: &str, number: u64, live_cwds: &[String]) -> ReviewTarget {
    if let Some(dir) = find_checkout(projects_root, name_with_owner) {
        if !is_busy(&dir, live_cwds) {
            return ReviewTarget { dir, clone: false };
        }
    }
    let repo_name = name_with_owner.rsplit('/').next().unwrap_or(name_with_owner);
    let dir = clones_root.join(format!("{repo_name}-{number}"));
    let clone = !dir.is_dir();
    ReviewTarget { dir, clone }
}

/// The name a review session is started with, so the board can tie it back
/// to its PR: `review <repo> #<number>`.
pub fn session_name(name_with_owner: &str, number: u64) -> String {
    let repo_name = name_with_owner.rsplit('/').next().unwrap_or(name_with_owner);
    format!("review {repo_name} #{number}")
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

/// The shell line the review terminal runs: a clone first when needed, then
/// the agent in the folder on the prompt file, as a new session named
/// `review <repo> #<number>` (`-n` for Claude Code; the others get the name
/// typed later, as `actions::start_session` does).
pub fn shell_command(agent: Harness, target: &ReviewTarget, name_with_owner: &str, number: u64, prompt_file: &Path, grok_id: Option<&str>) -> String {
    let opts = LaunchOptions { agent, name: Some(session_name(name_with_owner, number)), ..Default::default() };
    let start = crate::launch::session_command(&target.dir, prompt_file, &opts, grok_id);
    if target.clone {
        let dir = shell_single_quote(&target.dir.to_string_lossy());
        let parent = shell_single_quote(&target.dir.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default());
        format!("mkdir -p {parent} && gh repo clone {} {dir} && {start}", shell_single_quote(name_with_owner))
    } else {
        start
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GitProvider;
    use std::path::Path;

    fn node(number: u64, repo: &str, author: &str, updated: &str, opinionated: &str, latest: &str) -> String {
        format!(
            r#"{{"number":{number},"title":"t{number}","url":"https://github.com/{repo}/pull/{number}","isDraft":false,"updatedAt":"{updated}","repository":{{"nameWithOwner":"{repo}"}},"author":{{"login":"{author}"}},"latestOpinionatedReviews":{{"nodes":[{opinionated}]}},"latestReviews":{{"nodes":[{latest}]}}}}"#
        )
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
        let prs = parse_graphql(&json).unwrap();
        assert_eq!(prs.iter().map(|p| p.number).collect::<Vec<_>>(), vec![3, 451]);
        let p = &prs[1];
        assert_eq!((p.number, p.repo.as_str(), p.title.as_str(), p.author.as_str(), p.is_draft), (451, "Org/bedrock", "t451", "jane", false));
        assert_eq!(p.url, "https://github.com/Org/bedrock/pull/451");
        assert_eq!(p.reasons, vec![Reason::Review]);
        assert_eq!(p.my_review, None, "not reviewed yet");
        assert_eq!(prs[0].reasons, vec![Reason::Review, Reason::Assigned]);
        assert!(parse_graphql("not json").is_err());
        assert!(parse_graphql(r#"{"errors":[{"message":"bad query"}]}"#).unwrap_err().contains("bad query"));
    }

    #[test]
    fn a_pr_the_user_reviewed_stays_under_review_requested() {
        let done = node(9, "Org/x", "jane", "2026-09-28T10:00:00Z", &review("me", "APPROVED"), &review("me", "APPROVED"));
        let prs = parse_graphql(&response(&[], &[done], &[])).unwrap();
        assert_eq!(prs[0].reasons, vec![Reason::Review]);
        assert_eq!(prs[0].my_review, Some(ReviewMark::Approved), "logins match case-insensitively");
    }

    #[test]
    fn a_pending_request_means_to_review_whatever_the_user_said_before() {
        let again = node(8, "Org/x", "jane", "2026-09-28T10:00:00Z", &review("me", "CHANGES_REQUESTED"), &review("me", "CHANGES_REQUESTED"));
        let prs = parse_graphql(&response(&[again.clone()], &[again], &[])).unwrap();
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].my_review, None, "the author asked again: the old verdict is not this round's review");
    }

    #[test]
    fn partial_results_are_kept_when_github_also_reports_an_error() {
        let ok = node(1, "o/r", "jane", "", "", "");
        let json = response(&[ok], &[], &[]).replacen("{\"data\"", "{\"errors\":[{\"message\":\"SAML enforcement\"}],\"data\"", 1);
        assert_eq!(parse_graphql(&json).unwrap().len(), 1);
    }

    #[test]
    fn the_searches_keep_the_most_recently_updated() {
        assert_eq!(QUERY.matches("sort:updated-desc").count(), 3);
    }

    #[test]
    fn the_users_review_prefers_a_verdict_over_a_comment_and_ignores_others() {
        let v = |opinionated: &str, latest: &str| {
            let n = node(1, "o/r", "jane", "", opinionated, latest);
            parse_graphql(&response(&[], &[n], &[])).unwrap()[0].my_review
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
        let prs = parse_graphql(&response(&[], &[], &[own])).unwrap();
        assert_eq!(prs[0].reasons, vec![Reason::Assigned]);
        assert_eq!(prs[0].my_review, None, "replying on your own PR is not reviewing it");
    }

    #[test]
    fn remote_matching_accepts_ssh_and_https_forms_case_insensitively() {
        assert!(remote_matches("git@github.com:Org/bedrock.git", "org/bedrock"));
        assert!(remote_matches("https://github.com/Org/bedrock", "Org/bedrock"));
        assert!(remote_matches("https://github.com/Org/bedrock.git/", "Org/bedrock"));
        assert!(!remote_matches("git@github.com:Org/bedrock-fork.git", "Org/bedrock"));
        assert!(!remote_matches("git@gitlab.com:Org/bedrock.git", "Org/bedrock"));
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
        assert_eq!(find_checkout(t.path(), "Org/bouncer"), Some(main));
        std::fs::remove_dir_all(t.path().join("bouncer")).unwrap();
        assert_eq!(find_checkout(t.path(), "Org/bouncer"), Some(alt));
        assert_eq!(find_checkout(t.path(), "Org/missing"), None);
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
        let free = resolve_target(&projects, &clones, "Org/bedrock", 451, &[]);
        assert_eq!(free, ReviewTarget { dir: main.clone(), clone: false });
        let busy = resolve_target(&projects, &clones, "Org/bedrock", 451, &[main.to_string_lossy().into_owned()]);
        assert_eq!(busy, ReviewTarget { dir: clones.join("bedrock-451"), clone: true });
        std::fs::create_dir_all(clones.join("bedrock-451")).unwrap();
        let again = resolve_target(&projects, &clones, "Org/bedrock", 451, &[main.to_string_lossy().into_owned()]);
        assert_eq!(again, ReviewTarget { dir: clones.join("bedrock-451"), clone: false }, "an existing clone is reused");
        let none = resolve_target(&projects, &clones, "Org/nowhere", 7, &[]);
        assert_eq!(none, ReviewTarget { dir: clones.join("nowhere-7"), clone: true });
    }

    #[test]
    fn review_session_name_is_predictable() {
        assert_eq!(session_name("Org/bedrock", 451), "review bedrock #451");
    }

    /// The PR's review state as seen by viewer `Me` on a PR by `author`.
    fn pr_state(author: &str, opinionated: &[String], latest: &[String]) -> (Option<ReviewMark>, Vec<String>) {
        let n = node(1, "o/r", author, "", &opinionated.join(","), &latest.join(","));
        let p = parse_graphql(&response(&[], &[], &[n])).unwrap().remove(0);
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
        assert_eq!(nums(n.take_changes(&[assigned(1, Some(Approved)), assigned(2, None)])), Vec::<u64>::new(), "the first poll only primes");
        assert_eq!(nums(n.take_changes(&[assigned(1, Some(Approved)), assigned(2, None)])), Vec::<u64>::new(), "no change, no news");
        assert_eq!(nums(n.take_changes(&[assigned(1, Some(Approved)), assigned(2, Some(Commented))])), Vec::<u64>::new(), "a comment is not announced");
        assert_eq!(nums(n.take_changes(&[assigned(1, Some(ChangesRequested)), assigned(2, Some(Approved))])), vec![1, 2]);
        assert_eq!(nums(n.take_changes(&[assigned(1, Some(ChangesRequested)), assigned(2, Some(Approved))])), Vec::<u64>::new(), "once each");
        assert_eq!(nums(n.take_changes(&[assigned(2, Some(Approved))])), Vec::<u64>::new());
        assert_eq!(nums(n.take_changes(&[assigned(1, Some(ChangesRequested)), assigned(2, Some(Approved))])), Vec::<u64>::new(), "a PR that left the list and came back unchanged is not news");
        assert_eq!(nums(n.take_changes(&[assigned(3, Some(Approved))])), vec![3], "newly assigned and already approved is news");
        let requested_only = ReviewPr { reasons: vec![Reason::Review], ..assigned(4, Some(Approved)) };
        assert_eq!(nums(n.take_changes(&[requested_only])), Vec::<u64>::new(), "only assigned PRs are announced");
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
        let prs = parse_graphql(&response(&[node(1, "o/r", "jane", "", "", "")], &[], &[])).unwrap();
        assert_eq!(prs[0].forge, GitProvider::Github);
    }

    fn pr() -> ReviewPr {
        ReviewPr { number: 451, forge: GitProvider::Github, repo: "Org/bedrock".into(), title: "docs: notes".into(), author: "jane".into(), url: "https://github.com/Org/bedrock/pull/451".into(), is_draft: false, updated_at: "".into(), reasons: vec![Reason::Review], my_review: None, pr_review: None, pr_reviewers: vec![] }
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
        let s = shell_command(crate::model::Harness::ClaudeCode, &free, "Org/bedrock", 451, file, None);
        assert_eq!(s, "cd '/Users/x/dev/bedrock' && p=\"$(cat '/Users/x/.claude/maya/prompts/1.txt')\" && rm -f '/Users/x/.claude/maya/prompts/1.txt' && claude -n 'review bedrock #451' -- \"$p\"");
        let clone = ReviewTarget { dir: PathBuf::from("/Users/x/dev/reviews/bedrock-451"), clone: true };
        let s = shell_command(crate::model::Harness::Codex, &clone, "Org/bedrock", 451, file, None);
        assert!(s.starts_with("mkdir -p '/Users/x/dev/reviews' && gh repo clone 'Org/bedrock' '/Users/x/dev/reviews/bedrock-451' && "), "{s}");
        assert!(s.ends_with("&& codex -- \"$p\""), "no -n for Codex: the name waits as a pending name: {s}");
        let s = shell_command(crate::model::Harness::Grok, &free, "Org/bedrock", 451, file, Some("1111-2222"));
        assert!(s.ends_with("&& grok --session-id '1111-2222' -- \"$p\""), "{s}");
        let s = shell_command(crate::model::Harness::Antigravity, &free, "Org/bedrock", 451, file, None);
        assert!(s.ends_with("&& agy --prompt-interactive=\"$p\""), "{s}");
    }
}
