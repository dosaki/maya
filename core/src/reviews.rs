use crate::launch::{shell_single_quote, LaunchOptions};
use crate::model::Harness;
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
    let out = crate::command("gh")
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
    fn review_session_name_is_predictable() {
        assert_eq!(session_name("Org/bedrock", 451), "review bedrock #451");
    }

    fn pr() -> ReviewPr {
        ReviewPr { number: 451, repo: "Org/bedrock".into(), title: "docs: notes".into(), author: "jane".into(), url: "https://github.com/Org/bedrock/pull/451".into(), is_draft: false, updated_at: "".into(), reasons: vec![Reason::Review] }
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
