//! Maya picks the repository for a work item that has none.

use super::{ItemRepo, WorkItem};
use crate::config::{GitProvider, WorkItemsConfig};
use crate::forge::ForgeSite;
use std::path::{Path, PathBuf};

fn word(forge: GitProvider) -> &'static str {
    super::source_of(forge)
}

fn spelled(r: &ItemRepo) -> String {
    format!("{}:{}", word(r.forge), r.path)
}

/// The repositories Maya may pick from: `first` (a Coral product's, say) in
/// its order, then every rule's named repos, every listed item's repo and
/// `clones` sorted by `<forge>:<path>`; minus the trackers, once each
/// (ignoring case).
pub fn candidates(cfg: &WorkItemsConfig, items: &[WorkItem], clones: &[ItemRepo], first: &[ItemRepo]) -> Vec<ItemRepo> {
    let mut out: Vec<ItemRepo> = Vec::new();
    for (forge, items_cfg) in [(GitProvider::Github, &cfg.github), (GitProvider::Gitlab, &cfg.gitlab)] {
        out.extend(items_cfg.rules.iter().flat_map(|r| r.repos.iter()).map(|p| ItemRepo { forge, path: p.clone() }));
    }
    out.extend(items.iter().filter_map(|i| i.repo.clone()));
    out.extend(clones.iter().cloned());
    let is_tracker = |r: &ItemRepo| {
        let trackers = match r.forge {
            GitProvider::Github => &cfg.github.tracker_repos,
            GitProvider::Gitlab => &cfg.gitlab.tracker_repos,
        };
        trackers.iter().any(|t| t.eq_ignore_ascii_case(&r.path))
    };
    out.sort_by_key(|r| spelled(r).to_lowercase());
    let mut all: Vec<ItemRepo> = first.iter().cloned().chain(out).filter(|r| !is_tracker(r)).collect();
    // Paths ignore case, as the forges do; the first spelling seen is kept.
    let mut seen = std::collections::HashSet::new();
    all.retain(|r| seen.insert(spelled(r).to_lowercase()));
    all
}

/// The question for the agent: the item, and the candidates one per line.
pub fn pick_prompt(item: &WorkItem, candidates: &[ItemRepo]) -> String {
    let description: String = item.description.chars().take(2000).collect();
    let list = candidates.iter().map(spelled).collect::<Vec<_>>().join("\n");
    format!(
        "Which repository holds the code for this work item?\n==========\n{}: {}\n{}\n==========\nThese are the repositories:\n{}\nReply with two lines: the repository exactly as listed, then one short sentence saying why. Reply NONE if none fits.",
        item.id,
        item.title,
        description.trim(),
        list
    )
}

/// The listed repository the reply's first line names (ignoring case, quotes,
/// backticks and a trailing period) and the reason on its second line.
pub fn parse_pick(reply: &str, candidates: &[ItemRepo]) -> Option<(ItemRepo, String)> {
    let mut lines = reply.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next()?.trim_matches(|c| c == '"' || c == '\'' || c == '`').trim_end_matches('.').trim();
    let repo = candidates.iter().find(|c| spelled(c).eq_ignore_ascii_case(first))?;
    Some((repo.clone(), lines.next().unwrap_or("").to_string()))
}

/// The repository of each of `folders`: its origin, routed to a
/// switched-on site. Folders without an origin or on no site are left out;
/// a repository in two folders counts once.
pub fn repos_of_folders(folders: &[PathBuf], sites: &[ForgeSite]) -> Vec<ItemRepo> {
    let mut out: Vec<ItemRepo> = folders
        .iter()
        .filter_map(|dir| {
            let remote = crate::forge::origin_of(dir)?;
            let (site, path) = crate::forge::route(&remote, sites)?;
            Some(ItemRepo { forge: site.provider, path })
        })
        .collect();
    out.sort_by_key(spelled);
    out.dedup();
    out
}

/// The repository of each subfolder of `clones_root`, as `repos_of_folders`
/// finds them.
pub fn clone_repos(clones_root: &Path, sites: &[ForgeSite]) -> Vec<ItemRepo> {
    let Ok(entries) = std::fs::read_dir(clones_root) else { return Vec::new() };
    let folders: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    repos_of_folders(&folders, sites)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Assignee, ItemRule};
    use crate::work_items::test_item;

    #[test]
    fn candidates_leave_out_the_trackers_and_repeat_nothing() {
        let mut cfg = WorkItemsConfig::default();
        cfg.github.tracker_repos = vec!["acme/issues".into()];
        cfg.github.rules.push(ItemRule { assignee: Assignee::Anyone, labels: vec![], repos: vec!["acme/api".into(), "acme/issues".into()], statuses: vec![] });
        let item = |f, p: &str| WorkItem { repo: Some(ItemRepo { forge: f, path: p.into() }), ..test_item() };
        let c = candidates(&cfg, &[item(GitProvider::Github, "acme/web"), item(GitProvider::Github, "acme/api")], &[ItemRepo { forge: GitProvider::Gitlab, path: "grp/r".into() }], &[]);
        assert_eq!(c.iter().map(|r| format!("{:?}:{}", r.forge, r.path)).collect::<Vec<_>>(), vec!["Github:acme/api", "Github:acme/web", "Gitlab:grp/r"]);
    }

    #[test]
    fn candidates_match_trackers_and_repeats_ignoring_case() {
        let mut cfg = WorkItemsConfig::default();
        cfg.github.tracker_repos = vec!["Acme/Issues".into()];
        cfg.github.rules.push(ItemRule { assignee: Assignee::Anyone, labels: vec![], repos: vec!["acme/issues".into(), "Acme/API".into()], statuses: vec![] });
        let c = candidates(&cfg, &[], &[ItemRepo { forge: GitProvider::Github, path: "acme/api".into() }], &[]);
        assert_eq!(c.len(), 1, "{c:?}");
        assert!(c[0].path.eq_ignore_ascii_case("acme/api"));
    }

    #[test]
    fn the_first_repos_come_first() {
        let mut cfg = WorkItemsConfig::default();
        cfg.github.rules.push(ItemRule { assignee: Assignee::Anyone, labels: vec![], repos: vec!["acme/api".into()], statuses: vec![] });
        let gl = ItemRepo { forge: GitProvider::Gitlab, path: "g/r".into() };
        let c = candidates(&cfg, &[], &[ItemRepo { forge: GitProvider::Gitlab, path: "G/R".into() }], &[gl.clone()]);
        assert_eq!(c[0], gl);
        assert_eq!(c.len(), 2, "{c:?}");
        assert_eq!(c[1].path, "acme/api");
    }

    #[test]
    fn the_pick_is_a_listed_repository_and_a_reason() {
        let c = vec![ItemRepo { forge: GitProvider::Github, path: "acme/api".into() }, ItemRepo { forge: GitProvider::Gitlab, path: "grp/r".into() }];
        assert_eq!(parse_pick("`github:acme/api`\nThe search endpoint lives there.", &c), Some((c[0].clone(), "The search endpoint lives there.".into())));
        assert_eq!(parse_pick("GITLAB:grp/r.", &c), Some((c[1].clone(), String::new())));
        assert_eq!(parse_pick("NONE", &c), None);
        assert_eq!(parse_pick("github:acme/other\nwhy", &c), None);
    }

    #[test]
    fn the_prompt_lists_the_candidates_as_they_must_be_answered() {
        let c = vec![ItemRepo { forge: GitProvider::Github, path: "acme/api".into() }];
        let p = pick_prompt(&WorkItem { id: "ENG-1".into(), title: "Rate-limit search".into(), description: "x".repeat(5000), ..test_item() }, &c);
        assert!(p.contains("\ngithub:acme/api\n") && p.contains("Rate-limit search"));
        assert!(p.len() < 3000, "the description is cut");
    }

    #[test]
    fn a_multibyte_description_is_cut_by_characters() {
        let p = pick_prompt(&WorkItem { description: "é".repeat(5000), ..test_item() }, &[]);
        assert!(p.contains(&"é".repeat(2000)) && !p.contains(&"é".repeat(2001)));
    }

    #[test]
    fn clone_repos_route_each_origin_and_skip_the_rest() {
        let root = tempfile::tempdir().unwrap();
        let git = |dir: &Path, args: &[&str]| {
            assert!(crate::command("git").arg("-C").arg(dir).args(args).output().unwrap().status.success());
        };
        for (name, origin) in [("api", Some("git@github.com:acme/api.git")), ("api-issue-7", Some("https://github.com/acme/api.git")), ("other", Some("git@example.org:x/y.git")), ("bare", None)] {
            let d = root.path().join(name);
            std::fs::create_dir(&d).unwrap();
            git(&d, &["init", "-q"]);
            if let Some(o) = origin {
                git(&d, &["remote", "add", "origin", o]);
            }
        }
        std::fs::write(root.path().join("file.txt"), "").unwrap();
        let sites = [ForgeSite { provider: GitProvider::Github, web_base: "https://github.com".into(), clone: Default::default() }];
        assert_eq!(clone_repos(root.path(), &sites), vec![ItemRepo { forge: GitProvider::Github, path: "acme/api".into() }]);
    }

    #[test]
    fn repos_of_folders_route_each_origin_and_skip_the_rest() {
        let root = tempfile::tempdir().unwrap();
        let git = |dir: &Path, args: &[&str]| {
            assert!(crate::command("git").arg("-C").arg(dir).args(args).output().unwrap().status.success());
        };
        let mut folders = Vec::new();
        for (name, origin) in [("web", Some("git@github.com:acme/web.git")), ("web-2", Some("https://github.com/acme/web.git")), ("api", Some("git@github.com:acme/api.git")), ("other", Some("git@example.org:x/y.git")), ("bare", None)] {
            let d = root.path().join(name);
            std::fs::create_dir(&d).unwrap();
            git(&d, &["init", "-q"]);
            if let Some(o) = origin {
                git(&d, &["remote", "add", "origin", o]);
            }
            folders.push(d);
        }
        folders.push(root.path().join("gone"));
        let sites = [ForgeSite { provider: GitProvider::Github, web_base: "https://github.com".into(), clone: Default::default() }];
        let gh = |p: &str| ItemRepo { forge: GitProvider::Github, path: p.into() };
        assert_eq!(repos_of_folders(&folders, &sites), vec![gh("acme/api"), gh("acme/web")]);
    }
}
