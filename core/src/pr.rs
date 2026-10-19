use crate::model::PullRequest;
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use crate::forge::{Forge, Http};

/// How long a lookup result (a PR or none) is trusted before the forge is asked again.
pub const TTL_MS: u64 = 60_000;

/// The checked-out branch; None when detached or not a repo.
pub fn branch_of(dir: &Path) -> Option<String> {
    // `symbolic-ref` also names a branch with no commits yet, and fails when detached.
    let out = crate::command("git").args(["-C", dir.to_str()?, "symbolic-ref", "--short", "-q", "HEAD"]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    let b = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !b.is_empty()).then_some(b)
}

/// The PR or MR of `dir`'s branch, from the forge its `origin` belongs to.
/// Ok(None) when there is no repo, origin, branch or switched-on forge.
pub fn lookup(dir: &Path, forges: &[Forge], http: &dyn Http) -> Result<Option<PullRequest>, String> {
    let Some(origin) = crate::forge::origin_of(dir) else { return Ok(None) };
    let Some((forge, repo)) = crate::forge::route(&origin, forges) else { return Ok(None) };
    let Some(branch) = branch_of(dir) else { return Ok(None) };
    crate::forge::current_pr(http, forge, &repo, &branch)
}

struct Entry {
    pr: Option<PullRequest>,
    checked_at: u64,
}

/// Lookup results by session directory.
#[derive(Default)]
pub struct PrCache {
    entries: HashMap<String, Entry>,
    /// The branch checked out in each directory, from the same poll.
    branches: HashMap<String, String>,
}

impl PrCache {
    pub fn get(&self, dir: &str) -> Option<PullRequest> {
        self.entries.get(dir).and_then(|e| e.pr.clone())
    }

    pub fn set(&mut self, dir: &str, pr: Option<PullRequest>, now_ms: u64) {
        self.entries.insert(dir.to_string(), Entry { pr, checked_at: now_ms });
    }

    pub fn branch(&self, dir: &str) -> Option<String> {
        self.branches.get(dir).cloned()
    }

    /// None: detached, or not a repo.
    pub fn set_branch(&mut self, dir: &str, branch: Option<String>) {
        match branch {
            Some(b) => {
                self.branches.insert(dir.to_string(), b);
            }
            None => {
                self.branches.remove(dir);
            }
        }
    }

    /// The directories among `dirs` never looked up, or looked up over `TTL_MS` ago.
    pub fn due(&self, dirs: &[String], now_ms: u64) -> Vec<String> {
        dirs.iter()
            .filter(|d| self.entries.get(*d).map_or(true, |e| now_ms.saturating_sub(e.checked_at) >= TTL_MS))
            .cloned()
            .collect()
    }

    /// Forgets everything, so every directory is looked up again.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.branches.clear();
    }

    /// Forgets directories not in `live`.
    pub fn retain(&mut self, live: &[String]) {
        self.entries.retain(|d, _| live.contains(d));
        self.branches.retain(|d, _| live.contains(d));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PullRequest;

    #[test]
    fn cache_serves_known_results_and_reports_what_is_due() {
        let mut c = PrCache::default();
        let dirs = vec!["/a".to_string(), "/b".to_string()];
        assert_eq!(c.due(&dirs, 1000), dirs);
        c.set("/a", Some(PullRequest { number: 1, url: "u".into(), state: "open".into(), forge: Default::default() }), 1000);
        c.set("/b", None, 1000);
        assert_eq!(c.get("/a").map(|p| p.number), Some(1));
        assert_eq!(c.get("/b"), None);
        assert_eq!(c.get("/unknown"), None);
        assert!(c.due(&dirs, 1000 + TTL_MS - 1).is_empty());
        assert_eq!(c.due(&dirs, 1000 + TTL_MS), dirs);
        // Directories no longer live are dropped.
        c.retain(&["/a".to_string()]);
        assert_eq!(c.due(&dirs, 1000), vec!["/b".to_string()]);
    }

    use crate::config::{GitConnection, GitProvider};
    use crate::forge::{fake::FakeHttp, Forge, ForgeSite};

    fn github() -> Forge {
        Forge { site: ForgeSite::of(&GitConnection::default_for(GitProvider::Github)), token: "t".into() }
    }

    #[cfg(unix)]
    fn repo_on_branch(origin: Option<&str>, branch: &str) -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| assert!(std::process::Command::new("git").args(["-C", t.path().to_str().unwrap()]).args(args).status().unwrap().success(), "{args:?}");
        git(&["init", "-q", "-b", branch]);
        if let Some(o) = origin {
            git(&["remote", "add", "origin", o]);
        }
        t
    }

    #[cfg(unix)]
    #[test]
    fn lookup_asks_the_routed_forge_for_the_current_branch() {
        let t = repo_on_branch(Some("git@github.com:Org/r.git"), "feat/x");
        let http = FakeHttp::new(vec![("/pulls", 200, r#"[{"number":3,"html_url":"https://github.com/Org/r/pull/3","state":"open","draft":false}]"#)]);
        assert_eq!(lookup(t.path(), &[github()], &http).unwrap().map(|p| p.number), Some(3));
        assert!(http.seen.lock().unwrap()[0].0.contains("head=Org%3Afeat%2Fx"));
    }

    #[cfg(unix)]
    #[test]
    fn lookup_is_none_without_a_repo_a_known_forge_or_an_origin() {
        let http = FakeHttp::new(vec![]);
        let plain = tempfile::tempdir().unwrap();
        assert_eq!(lookup(plain.path(), &[github()], &http), Ok(None));
        let elsewhere = repo_on_branch(Some("git@gitlab.com:grp/r.git"), "main");
        assert_eq!(lookup(elsewhere.path(), &[github()], &http), Ok(None), "GitLab is off");
        let no_origin = repo_on_branch(None, "main");
        assert_eq!(lookup(no_origin.path(), &[github()], &http), Ok(None));
        assert!(http.seen.lock().unwrap().is_empty(), "no call was made");
    }

    #[test]
    fn clear_forgets_every_result() {
        let mut c = PrCache::default();
        c.set("/a", None, 1000);
        c.clear();
        assert_eq!(c.due(&["/a".to_string()], 1000), vec!["/a".to_string()]);
    }

    #[test]
    fn branches_are_kept_by_directory_and_forgotten_with_it() {
        let mut c = PrCache::default();
        c.set_branch("/a", Some("feat/x".into()));
        assert_eq!(c.branch("/a").as_deref(), Some("feat/x"));
        c.set_branch("/a", None);
        assert_eq!(c.branch("/a"), None, "detached or not a repo");
        c.set_branch("/a", Some("main".into()));
        c.set_branch("/b", Some("dev".into()));
        c.retain(&["/b".to_string()]);
        assert_eq!(c.branch("/a"), None);
        assert_eq!(c.branch("/b").as_deref(), Some("dev"));
        c.clear();
        assert_eq!(c.branch("/b"), None);
    }
}
