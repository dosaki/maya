use crate::launch::shell_single_quote;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Why a PR is on the list.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Reason {
    Review,
    Assigned,
}

/// A pull request waiting on the user.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewPr {
    pub number: u64,
    /// `owner/name`.
    pub repo: String,
    pub title: String,
    pub author: String,
    pub url: String,
    pub is_draft: bool,
    /// ISO 8601, as GitHub gives it; sorts lexically.
    pub updated_at: String,
    pub reasons: Vec<Reason>,
}

pub const SEARCH_FIELDS: &str = "number,title,repository,url,author,updatedAt,isDraft";

/// `gh search prs --json` output as review PRs, all tagged with `reason`.
pub fn parse_search(json: &str, reason: Reason) -> Result<Vec<ReviewPr>, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("gh output was not JSON: {e}"))?;
    let arr = v.as_array().ok_or("gh output was not a list")?;
    Ok(arr
        .iter()
        .filter_map(|p| {
            Some(ReviewPr {
                number: p["number"].as_u64()?,
                repo: p["repository"]["nameWithOwner"].as_str()?.to_string(),
                title: p["title"].as_str().unwrap_or("").to_string(),
                author: p["author"]["login"].as_str().unwrap_or("").to_string(),
                url: p["url"].as_str()?.to_string(),
                is_draft: p["isDraft"].as_bool().unwrap_or(false),
                updated_at: p["updatedAt"].as_str().unwrap_or("").to_string(),
                reasons: vec![reason],
            })
        })
        .collect())
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

fn search(flag: &str, reason: Reason) -> Result<Vec<ReviewPr>, String> {
    let out = Command::new("gh")
        .args(["search", "prs", flag, "--state=open", "--limit", "50", "--json", SEARCH_FIELDS])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run gh: {e}"))?;
    if !out.status.success() {
        return Err(format!("gh failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    parse_search(&String::from_utf8_lossy(&out.stdout), reason)
}

/// Open PRs where the user is a requested reviewer or an assignee.
pub fn fetch() -> Result<Vec<ReviewPr>, String> {
    Ok(merge(search("--review-requested=@me", Reason::Review)?, search("--assignee=@me", Reason::Assigned)?))
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
    let out = Command::new("git").args(["-C", dir.to_str()?, "config", "--get", "remote.origin.url"]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
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

/// The shell line the review terminal runs.
pub fn shell_command(target: &ReviewTarget, name_with_owner: &str, number: u64) -> String {
    let dir = shell_single_quote(&target.dir.to_string_lossy());
    let start = format!(
        "cd {dir} && claude --name {} -- {}",
        shell_single_quote(&session_name(name_with_owner, number)),
        shell_single_quote(&format!("/should-i-approve PR #{number}"))
    );
    if target.clone {
        let parent = shell_single_quote(&target.dir.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default());
        format!("mkdir -p {parent} && gh repo clone {} {dir} && {start}", shell_single_quote(name_with_owner))
    } else {
        start
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const ONE: &str = r#"[{"number":451,"title":"docs: notes","repository":{"nameWithOwner":"Org/bedrock"},"url":"https://github.com/Org/bedrock/pull/451","author":{"login":"jane"},"updatedAt":"2026-09-28T10:00:00Z","isDraft":false}]"#;
    const TWO: &str = r#"[{"number":3,"title":"feat: x","repository":{"nameWithOwner":"Org/plugins"},"url":"https://github.com/Org/plugins/pull/3","author":{"login":"bob"},"updatedAt":"2026-09-29T09:00:00Z","isDraft":true},{"number":451,"title":"docs: notes","repository":{"nameWithOwner":"Org/bedrock"},"url":"https://github.com/Org/bedrock/pull/451","author":{"login":"jane"},"updatedAt":"2026-09-28T10:00:00Z","isDraft":false}]"#;

    #[test]
    fn parses_gh_search_output_and_tags_the_reason() {
        let prs = parse_search(ONE, Reason::Review).unwrap();
        assert_eq!(prs.len(), 1);
        let p = &prs[0];
        assert_eq!((p.number, p.repo.as_str(), p.title.as_str(), p.author.as_str(), p.is_draft), (451, "Org/bedrock", "docs: notes", "jane", false));
        assert_eq!(p.url, "https://github.com/Org/bedrock/pull/451");
        assert_eq!(p.updated_at, "2026-09-28T10:00:00Z");
        assert_eq!(p.reasons, vec![Reason::Review]);
        assert!(parse_search("not json", Reason::Review).is_err());
    }

    #[test]
    fn merge_dedupes_by_url_combines_reasons_and_sorts_newest_first() {
        let review = parse_search(ONE, Reason::Review).unwrap();
        let assigned = parse_search(TWO, Reason::Assigned).unwrap();
        let all = merge(review, assigned);
        assert_eq!(all.iter().map(|p| p.number).collect::<Vec<_>>(), vec![3, 451]);
        assert_eq!(all[1].reasons, vec![Reason::Review, Reason::Assigned]);
        assert_eq!(all[0].reasons, vec![Reason::Assigned]);
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
    fn shell_command_clones_when_needed_then_starts_the_review() {
        let plain = shell_command(&ReviewTarget { dir: "/Users/x/dev/bedrock".into(), clone: false }, "Org/bedrock", 451);
        assert_eq!(plain, "cd '/Users/x/dev/bedrock' && claude --name 'review bedrock #451' -- '/should-i-approve PR #451'");
        let cloned = shell_command(&ReviewTarget { dir: "/Users/x/dev/reviews/bedrock-451".into(), clone: true }, "Org/bedrock", 451);
        assert_eq!(cloned, "mkdir -p '/Users/x/dev/reviews' && gh repo clone 'Org/bedrock' '/Users/x/dev/reviews/bedrock-451' && cd '/Users/x/dev/reviews/bedrock-451' && claude --name 'review bedrock #451' -- '/should-i-approve PR #451'");
    }

    #[test]
    fn review_session_name_is_predictable() {
        assert_eq!(session_name("Org/bedrock", 451), "review bedrock #451");
    }
}
