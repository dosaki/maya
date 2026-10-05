use crate::model::PullRequest;
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;

/// How long a lookup result (a PR or none) is trusted before `gh` is asked again.
pub const TTL_MS: u64 = 60_000;

/// The PR record in `gh pr view --json number,url,state,isDraft` output.
pub fn parse(stdout: &str) -> Option<PullRequest> {
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;
    let number = v["number"].as_u64()?;
    let url = v["url"].as_str()?.to_string();
    let state = if v["isDraft"].as_bool() == Some(true) { "draft".to_string() } else { v["state"].as_str()?.to_ascii_lowercase() };
    Some(PullRequest { number, url, state })
}

/// Asks `gh` for the PR of `dir`'s current branch. None when there is no
/// repo, remote, PR, or `gh` itself.
pub fn lookup(dir: &Path) -> Option<PullRequest> {
    let out = crate::gh()
        .args(["pr", "view", "--json", "number,url,state,isDraft"])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse(&String::from_utf8_lossy(&out.stdout))
}

struct Entry {
    pr: Option<PullRequest>,
    checked_at: u64,
}

/// Lookup results by session directory.
#[derive(Default)]
pub struct PrCache {
    entries: HashMap<String, Entry>,
}

impl PrCache {
    pub fn get(&self, dir: &str) -> Option<PullRequest> {
        self.entries.get(dir).and_then(|e| e.pr.clone())
    }

    pub fn set(&mut self, dir: &str, pr: Option<PullRequest>, now_ms: u64) {
        self.entries.insert(dir.to_string(), Entry { pr, checked_at: now_ms });
    }

    /// The directories among `dirs` never looked up, or looked up over `TTL_MS` ago.
    pub fn due(&self, dirs: &[String], now_ms: u64) -> Vec<String> {
        dirs.iter()
            .filter(|d| self.entries.get(*d).map_or(true, |e| now_ms.saturating_sub(e.checked_at) >= TTL_MS))
            .cloned()
            .collect()
    }

    /// Forgets directories not in `live`.
    pub fn retain(&mut self, live: &[String]) {
        self.entries.retain(|d, _| live.contains(d));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PullRequest;

    #[test]
    fn parses_gh_pr_view_output_with_lowercase_state_and_draft() {
        let out = r#"{"number":781,"state":"MERGED","url":"https://github.com/o/r/pull/781","isDraft":false}"#;
        assert_eq!(parse(out), Some(PullRequest { number: 781, url: "https://github.com/o/r/pull/781".into(), state: "merged".into() }));
        let draft = r#"{"number":9,"state":"OPEN","url":"https://github.com/o/r/pull/9","isDraft":true}"#;
        assert_eq!(parse(draft).unwrap().state, "draft");
        assert_eq!(parse("no pull requests found for branch"), None);
        assert_eq!(parse(r#"{"number":"x"}"#), None);
    }

    #[test]
    fn cache_serves_known_results_and_reports_what_is_due() {
        let mut c = PrCache::default();
        let dirs = vec!["/a".to_string(), "/b".to_string()];
        assert_eq!(c.due(&dirs, 1000), dirs);
        c.set("/a", Some(PullRequest { number: 1, url: "u".into(), state: "open".into() }), 1000);
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

    #[test]
    fn lookup_in_a_directory_without_a_repo_is_none() {
        let t = tempfile::tempdir().unwrap();
        assert_eq!(lookup(t.path()), None);
    }
}
