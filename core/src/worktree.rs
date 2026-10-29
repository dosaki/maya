//! A new session's own worktree: a branch and a folder made from the
//! checkout the session would otherwise run in. Maya only adds them; it
//! never removes a worktree, a branch or a folder.

use std::path::{Path, PathBuf};
use std::process::Stdio;

/// The branch name for a session: the trimmed name, else the prompt's
/// first six words, reduced to lower-case letters and digits joined by
/// single dashes and cut to 40 characters; `session` when nothing is left.
pub fn branch_slug(name: Option<&str>, prompt: &str) -> String {
    let name = name.map(str::trim).filter(|n| !n.is_empty());
    let source = match name {
        Some(n) => n.to_string(),
        None => prompt.split_whitespace().take(6).collect::<Vec<_>>().join(" "),
    };
    let mut slug = String::new();
    for c in source.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    // Everything left is ASCII, so cutting at a byte is cutting at a character.
    let slug = slug[..slug.len().min(40)].trim_matches('-');
    if slug.is_empty() { "session".into() } else { slug.into() }
}

/// What `git -C dir args` prints on success, trimmed; None when it fails.
fn git_out(dir: &Path, args: &[&str]) -> Option<String> {
    let out = crate::command("git").arg("-C").arg(dir).args(args).stdin(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Adds a worktree for a new session on a new branch cut from the
/// checkout's current commit, under `clones_root`, and returns its
/// folder. Only the top folder of a repository qualifies.
pub fn add_for_session(checkout: &Path, clones_root: &Path, name: Option<&str>, prompt: &str) -> Result<PathBuf, String> {
    let folder = checkout.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let not_a_repo = || format!("{folder} is not a git repository, so it cannot start in a worktree.");
    // A subfolder reports its repository's top, which then differs from it.
    let top = git_out(checkout, &["rev-parse", "--show-toplevel"]).ok_or_else(not_a_repo)?;
    match (std::fs::canonicalize(&top), std::fs::canonicalize(checkout)) {
        (Ok(a), Ok(b)) if a == b => {}
        _ => return Err(not_a_repo()),
    }
    let slug = branch_slug(name, prompt);
    let candidate = (1..=100)
        .map(|n| if n == 1 { slug.clone() } else { format!("{slug}-{n}") })
        .find(|c| {
            let taken = git_out(checkout, &["show-ref", "--verify", "--quiet", &format!("refs/heads/{c}")]).is_some();
            !taken && !clones_root.join(format!("{folder}-{c}")).exists()
        })
        .ok_or_else(|| format!("No free branch name starting with {slug} in {folder}."))?;
    let dir = clones_root.join(format!("{folder}-{candidate}"));
    std::fs::create_dir_all(clones_root).map_err(|e| format!("Could not create {}: {e}", clones_root.display()))?;
    let out = crate::command("git")
        .arg("-C")
        .arg(checkout)
        .args(["worktree", "add", "-b", &candidate])
        .arg(&dir)
        .arg("HEAD")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Could not run git: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) -> String {
        let o = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }

    /// A repository `root/name` with one commit.
    fn repo(root: &Path, name: &str) -> PathBuf {
        let d = root.join(name);
        std::fs::create_dir_all(&d).unwrap();
        git(&d, &["init", "-q"]);
        git(&d, &["config", "user.email", "t@example.com"]);
        git(&d, &["config", "user.name", "T"]);
        std::fs::write(d.join("a.txt"), "a").unwrap();
        git(&d, &["add", "."]);
        git(&d, &["commit", "-q", "-m", "first"]);
        d
    }

    #[test]
    fn a_slug_comes_from_the_name_else_the_prompt() {
        assert_eq!(branch_slug(Some("Fix CI: v1.2/..lock"), "ignored"), "fix-ci-v1-2-lock");
        assert_eq!(branch_slug(Some("  "), "Add the login page and its tests please now"), "add-the-login-page-and-its");
        assert_eq!(branch_slug(None, "修复登录 🚀"), "session");
        assert!(branch_slug(None, &"word ".repeat(30)).len() <= 40);
        assert!(!branch_slug(None, &"abcdefghij ".repeat(6)).ends_with('-'));
    }

    #[test]
    fn a_worktree_branches_off_the_checkouts_current_branch() {
        let t = tempfile::tempdir().unwrap();
        let clones = t.path().join("clones");
        let proj = repo(t.path(), "proj");
        git(&proj, &["switch", "-q", "-c", "feature"]);
        let wt = add_for_session(&proj, &clones, Some("Fix CI"), "x").unwrap();
        assert_eq!(wt, clones.join("proj-fix-ci"));
        assert_eq!(git(&wt, &["rev-parse", "--abbrev-ref", "HEAD"]), "fix-ci");
        assert_eq!(git(&wt, &["rev-parse", "HEAD"]), git(&proj, &["rev-parse", "feature"]));
        assert_eq!(git(&proj, &["rev-parse", "--abbrev-ref", "HEAD"]), "feature");
    }

    #[test]
    fn a_taken_branch_or_folder_gets_the_next_number() {
        let t = tempfile::tempdir().unwrap();
        let clones = t.path().join("clones");
        let proj = repo(t.path(), "proj");
        add_for_session(&proj, &clones, Some("fix"), "x").unwrap();
        std::fs::create_dir_all(clones.join("proj-fix-2")).unwrap();
        assert_eq!(add_for_session(&proj, &clones, Some("fix"), "x").unwrap(), clones.join("proj-fix-3"));
    }

    #[test]
    fn only_the_top_of_a_repository_can_start_in_a_worktree() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        let clones = root.join("clones");
        let proj = repo(root, "proj");
        let plain = root.join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        assert_eq!(add_for_session(&plain, &clones, None, "x").unwrap_err(), "plain is not a git repository, so it cannot start in a worktree.");
        let pkg = proj.join("packages/api");
        std::fs::create_dir_all(&pkg).unwrap();
        assert_eq!(add_for_session(&pkg, &clones, None, "x").unwrap_err(), "api is not a git repository, so it cannot start in a worktree.");
    }

    #[test]
    fn a_repository_without_commits_returns_gits_message() {
        let t = tempfile::tempdir().unwrap();
        let clones = t.path().join("clones");
        let empty = t.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        git(&empty, &["init", "-q"]);
        let e = add_for_session(&empty, &clones, None, "x").unwrap_err();
        assert!(!e.is_empty() && !e.contains("not a git repository, so"), "{e}");
    }
}
