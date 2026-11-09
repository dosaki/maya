//! Where Start works on an item: a worktree of its repository's one clone,
//! on a branch named after the item, and the prompt its session starts with.

use super::WorkItem;
use crate::forge::ForgeSite;
use crate::reviews::{clone_of, is_worktree_of};
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// Where an item's session works: a worktree of the repository's one clone,
/// on the item's branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkTarget {
    /// The one clone of the repository, shared with reviews.
    pub clone_dir: PathBuf,
    /// Where the session works: `<clone folder name>-issue-<number>`.
    pub dir: PathBuf,
    pub branch: String,
    /// `clone_dir` does not exist yet.
    pub clone: bool,
    /// `dir` is a worktree of `clone_dir` already.
    pub worktree_exists: bool,
    /// `refs/heads/<branch>` exists in the clone.
    pub branch_exists: bool,
}

/// The most characters a branch name has, `issue-` included.
const BRANCH_MAX: usize = 40;

/// `issue-<number>-<title as a slug>`, at most 40 characters: only the
/// title is cut, so items of one repository never share a branch however
/// long its path. With no number the id's slug stands in for it; a title
/// with nothing to slug leaves `issue-<number>`.
pub fn branch_name(item: &WorkItem) -> String {
    use crate::worktree::branch_slug;
    let number = item.number.map_or_else(|| branch_slug(None, &item.id), |n| n.to_string());
    let head = format!("issue-{number}");
    let has_title = item.title.chars().any(|c| c.is_ascii_alphanumeric());
    let name = if has_title { format!("{head}-{}", branch_slug(None, &item.title)) } else { head };
    // Everything is ASCII, so cutting at a byte is cutting at a character.
    name[..name.len().min(BRANCH_MAX)].trim_end_matches('-').to_string()
}

/// True when the clone at `clone_dir` has the local branch `branch`.
fn has_branch(clone_dir: &Path, branch: &str) -> bool {
    clone_dir.exists()
        && crate::command("git")
            .arg("-C")
            .arg(clone_dir)
            .args(["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
}

/// Where `repo`'s item `number` is worked on under `clones_root`: the clone
/// reviews use, and `<clone folder name>-issue-<number>` beside it. A folder
/// of that name that is not a worktree of the clone is refused.
pub fn resolve_target(clones_root: &Path, site: &ForgeSite, repo: &str, number: u64, branch: &str) -> Result<WorkTarget, String> {
    let (base, clone_dir) = clone_of(clones_root, site, repo)?;
    let dir = clones_root.join(format!("{base}-issue-{number}"));
    let worktree_exists = is_worktree_of(&dir, &clone_dir);
    if dir.exists() && !worktree_exists {
        return Err(format!("{} is not a worktree of {repo}; move it or change the clones directory.", dir.display()));
    }
    Ok(WorkTarget { clone: !clone_dir.exists(), branch_exists: has_branch(&clone_dir, branch), branch: branch.to_string(), worktree_exists, clone_dir, dir })
}

/// The `git` argument lists run before the agent, in order: the clone once,
/// a fetch, and a worktree on the item's branch (made off the default
/// branch the first time). An existing worktree is used as it is, so work
/// in it is never reset. The prune clears a worktree deleted by hand.
pub fn git_steps(target: &WorkTarget, clone_url: &str) -> Vec<Vec<String>> {
    let clone = target.clone_dir.to_string_lossy().into_owned();
    let dir = target.dir.to_string_lossy().into_owned();
    let step = |args: &[&str]| args.iter().map(|a| a.to_string()).collect::<Vec<_>>();
    let mut steps = vec![];
    if target.clone {
        steps.push(step(&["clone", "--", clone_url, &clone]));
    }
    steps.push(step(&["-C", &clone, "fetch", "origin"]));
    if !target.worktree_exists {
        steps.push(step(&["-C", &clone, "worktree", "prune"]));
        if target.branch_exists {
            steps.push(step(&["-C", &clone, "worktree", "add", &dir, &target.branch]));
        } else {
            steps.push(step(&["-C", &clone, "worktree", "add", "-b", &target.branch, &dir, "origin/HEAD"]));
        }
    }
    steps
}

/// The prompt an item's session starts with when the setting is blank.
pub const DEFAULT_WORK_PROMPT: &str = "Work on {id}: {title} ({url}). This folder is a fresh worktree of {repo} on its own branch. Implement or fix what the work item asks, test it, and open a pull request when it is done.\n\n{description}";

/// The template (the built-in one when blank) with `{id}`, `{title}`,
/// `{url}`, `{description}` and `{repo}` filled in. It is filled in one
/// pass, so a title or description that holds a placeholder is left as written.
pub fn render_prompt(template: &str, item: &WorkItem, repo: &str) -> String {
    let t = template.trim();
    let mut rest = if t.is_empty() { DEFAULT_WORK_PROMPT } else { t };
    let values = [("{id}", item.id.as_str()), ("{title}", &item.title), ("{url}", &item.url), ("{description}", &item.description), ("{repo}", repo)];
    let mut out = String::new();
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        match values.iter().find(|(k, _)| rest.starts_with(k)) {
            Some((k, v)) => {
                out.push_str(v);
                rest = &rest[k.len()..];
            }
            None => {
                out.push('{');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GitConnection, GitProvider};
    use crate::forge::ForgeSite;
    use crate::work_items::{test_item, WorkItem};

    #[test]
    fn the_branch_names_the_item_and_stays_short() {
        let i = WorkItem { id: "acme/api#412".into(), number: Some(412), title: "Login times out after 30 s on slow networks".into(), ..test_item() };
        let b = branch_name(&i);
        assert!(b.starts_with("issue-412-login"), "{b}");
        assert!(b.len() <= 40 && !b.ends_with('-'), "{b}");
        assert_eq!(branch_name(&WorkItem { title: "!!!".into(), ..i.clone() }), "issue-412", "a title with nothing to slug");
        assert_eq!(branch_name(&WorkItem { number: None, title: "Login".into(), ..i }), "issue-acme-api-412-login", "the id stands in for a missing number");
    }

    #[test]
    fn items_of_a_repository_with_a_long_path_get_their_own_branches() {
        let item = |id: &str, n: u64, title: &str| WorkItem { id: id.into(), number: Some(n), title: title.into(), ..test_item() };
        let a = branch_name(&item("TheKeyholdingCompany/coral-boards#123", 123, "Fix login when the session cookie expires overnight"));
        let b = branch_name(&item("TheKeyholdingCompany/coral-boards#456", 456, "Add export of the whole board to a spreadsheet file"));
        assert_ne!(a, b);
        for (name, n) in [(&a, "123"), (&b, "456")] {
            assert!(name.len() <= 40 && name.contains(n), "{name}");
        }
    }

    #[test]
    fn a_new_item_gets_a_new_branch_off_the_default_and_an_old_branch_is_reused() {
        let t = |branch_exists, worktree_exists, clone| WorkTarget { clone_dir: "/c/api".into(), dir: "/c/api-issue-412".into(), branch: "issue-x".into(), clone, worktree_exists, branch_exists };
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(git_steps(&t(false, false, true), "git@github.com:acme/api.git"), vec![
            s(&["clone", "--", "git@github.com:acme/api.git", "/c/api"]),
            s(&["-C", "/c/api", "fetch", "origin"]),
            s(&["-C", "/c/api", "worktree", "prune"]),
            s(&["-C", "/c/api", "worktree", "add", "-b", "issue-x", "/c/api-issue-412", "origin/HEAD"]),
        ]);
        assert_eq!(git_steps(&t(true, false, false), "u")[1..], [s(&["-C", "/c/api", "worktree", "prune"]), s(&["-C", "/c/api", "worktree", "add", "/c/api-issue-412", "issue-x"])]);
        assert_eq!(git_steps(&t(true, true, false), "u"), vec![s(&["-C", "/c/api", "fetch", "origin"])], "an existing worktree is used as it is");
    }

    #[test]
    fn the_prompt_fills_the_item_in() {
        let i = WorkItem { id: "acme/api#412".into(), title: "Login".into(), url: "https://x/412".into(), description: "Steps".into(), ..test_item() };
        let p = render_prompt("", &i, "acme/api");
        assert!(p.starts_with("Work on acme/api#412: Login (https://x/412). This folder is a fresh worktree of acme/api"), "{p}");
        assert!(p.ends_with("\n\nSteps"));
        assert_eq!(render_prompt("/fix {id} in {repo}", &i, "acme/api"), "/fix acme/api#412 in acme/api");
    }

    #[test]
    fn a_folder_that_is_not_the_clones_worktree_is_refused() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("api-issue-412")).unwrap();
        let site = ForgeSite::of(&GitConnection::default_for(GitProvider::Github));
        let e = resolve_target(root.path(), &site, "acme/api", 412, "issue-x").unwrap_err();
        assert!(e.ends_with("is not a worktree of acme/api; move it or change the clones directory."), "{e}");
    }

    fn git(dir: &Path, args: &[&str]) {
        let o = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    }

    #[test]
    fn the_target_shares_the_reviews_clone_and_sees_an_old_branch() {
        let root = tempfile::tempdir().unwrap();
        let site = ForgeSite::of(&GitConnection::default_for(GitProvider::Github));
        let fresh = resolve_target(root.path(), &site, "acme/api", 412, "issue-x").unwrap();
        assert_eq!((fresh.clone_dir.clone(), fresh.dir.clone()), (root.path().join("api"), root.path().join("api-issue-412")));
        assert!(fresh.clone && !fresh.worktree_exists && !fresh.branch_exists, "{fresh:?}");
        let clone = root.path().join("api");
        std::fs::create_dir_all(&clone).unwrap();
        for args in [&["init", "-q"][..], &["remote", "add", "origin", "git@github.com:acme/api.git"], &["config", "user.email", "t@example.com"], &["config", "user.name", "T"], &["config", "commit.gpgsign", "false"], &["config", "core.hooksPath", "/dev/null"], &["commit", "-q", "--allow-empty", "-m", "a"], &["branch", "issue-x"]] {
            git(&clone, args);
        }
        let again = resolve_target(root.path(), &site, "acme/api", 412, "issue-x").unwrap();
        assert!(!again.clone && again.branch_exists, "{again:?}");
        assert!(!resolve_target(root.path(), &site, "acme/api", 412, "issue-y").unwrap().branch_exists);
    }
}
