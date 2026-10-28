use crate::forge::github::{active_repos, pending_deployments, repo_needs_approval, runs};
use crate::forge::{is_refusal, Forge, Http};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A repo whose last push is older than this is not checked, in milliseconds.
pub const ACTIVE_WINDOW_MS: u64 = 7 * 86_400_000;
/// How long a repo's "needs approval" answer is kept, in milliseconds.
pub const ANSWER_TTL_MS: u64 = 3_600_000;
/// The most pages of the repo listing to read.
pub const REPO_PAGES: u32 = 5;
/// How many repos are checked at once.
const THREADS: usize = 8;

/// A deployment environment a run waits to enter.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Environment {
    pub id: u64,
    pub name: String,
}

/// The earlier run of the same workflow that a queued run waits behind.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Blocker {
    pub run_id: u64,
    pub sha: String,
    pub title: String,
    pub url: String,
}

/// What a run waits for: the user's approval, or another run.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RunWait {
    Approval { environments: Vec<Environment> },
    Blocked { by: Option<Blocker> },
}

/// A row on the Deployments tab.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentRun {
    /// `owner/name`.
    pub repo: String,
    pub run_id: u64,
    pub run_number: u64,
    pub workflow: String,
    pub branch: String,
    pub sha: String,
    /// First line of the commit message.
    pub title: String,
    pub actor: String,
    pub url: String,
    /// ISO 8601, as GitHub gives it; sorts lexically.
    pub since: String,
    pub wait: RunWait,
}

/// A workflow run, as GitHub lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub id: u64,
    pub number: u64,
    pub workflow_id: u64,
    pub name: String,
    pub branch: String,
    pub sha: String,
    pub title: String,
    pub actor: String,
    pub url: String,
    pub created_at: String,
    pub status: String,
}

fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or("").to_string()
}

/// The runs in a `workflow_runs` response; entries without an id are skipped.
pub fn parse_runs(v: &Value) -> Vec<Run> {
    let Some(list) = v["workflow_runs"].as_array() else { return Vec::new() };
    list.iter()
        .filter_map(|r| {
            let id = r["id"].as_u64()?;
            let actor = r["triggering_actor"]["login"]
                .as_str()
                .or_else(|| r["actor"]["login"].as_str())
                .unwrap_or("");
            let title = r["head_commit"]["message"].as_str().and_then(|m| m.lines().next()).unwrap_or("");
            Some(Run {
                id,
                number: r["run_number"].as_u64().unwrap_or(0),
                workflow_id: r["workflow_id"].as_u64().unwrap_or(0),
                name: text(r, "name"),
                branch: text(r, "head_branch"),
                sha: text(r, "head_sha"),
                title: title.to_string(),
                actor: actor.to_string(),
                url: text(r, "html_url"),
                created_at: text(r, "created_at"),
                status: text(r, "status"),
            })
        })
        .collect()
}

/// Whether any environment in the response has a required-reviewers rule.
pub fn needs_approval(v: &Value) -> bool {
    v["environments"].as_array().is_some_and(|envs| {
        envs.iter().any(|e| {
            e["protection_rules"]
                .as_array()
                .is_some_and(|rules| rules.iter().any(|r| r["type"] == "required_reviewers"))
        })
    })
}

/// The pending deployments the user can approve.
pub fn approvable(v: &Value) -> Vec<Environment> {
    let Some(list) = v.as_array() else { return Vec::new() };
    list.iter()
        .filter(|p| p["current_user_can_approve"] == true)
        .filter_map(|p| {
            Some(Environment { id: p["environment"]["id"].as_u64()?, name: text(&p["environment"], "name") })
        })
        .collect()
}

/// The newest earlier run of the same workflow that is still live.
pub fn find_blocker(pending: &Run, others: &[Run]) -> Option<Blocker> {
    others
        .iter()
        .filter(|o| {
            o.workflow_id == pending.workflow_id
                && o.number < pending.number
                && matches!(o.status.as_str(), "waiting" | "in_progress" | "queued")
        })
        .max_by_key(|o| o.number)
        .map(|o| Blocker { run_id: o.id, sha: o.sha.clone(), title: o.title.clone(), url: o.url.clone() })
}

/// A tab row for `run`.
pub fn row(repo: &str, run: &Run, wait: RunWait) -> DeploymentRun {
    DeploymentRun {
        repo: repo.to_string(),
        run_id: run.id,
        run_number: run.number,
        workflow: run.name.clone(),
        branch: run.branch.clone(),
        sha: run.sha.clone(),
        title: run.title.clone(),
        actor: run.actor.clone(),
        url: run.url.clone(),
        since: run.created_at.clone(),
        wait,
    }
}

fn blocker_id(r: &DeploymentRun) -> Option<u64> {
    match &r.wait {
        RunWait::Blocked { by: Some(b) } => Some(b.run_id),
        _ => None,
    }
}

/// Appends `rows[i]`, then the rows blocked by it, depth first.
fn emit(rows: &[DeploymentRun], i: usize, out: &mut Vec<DeploymentRun>) {
    out.push(rows[i].clone());
    for (j, r) in rows.iter().enumerate() {
        if blocker_id(r) == Some(rows[i].run_id) {
            emit(rows, j, out);
        }
    }
}

/// Oldest first, a repo's rows together, repos by their oldest row, and a
/// blocked row right after the run that blocks it.
pub fn order(mut runs: Vec<DeploymentRun>) -> Vec<DeploymentRun> {
    runs.sort_by(|a, b| a.since.cmp(&b.since));
    let mut repos: Vec<(String, Vec<DeploymentRun>)> = Vec::new();
    for r in runs {
        match repos.iter_mut().find(|(name, _)| *name == r.repo) {
            Some((_, rows)) => rows.push(r),
            None => repos.push((r.repo.clone(), vec![r])),
        }
    }
    let mut out = Vec::new();
    for (_, rows) in &repos {
        for (i, r) in rows.iter().enumerate() {
            let follows = blocker_id(r).is_some_and(|id| rows.iter().any(|o| o.run_id == id));
            if !follows {
                emit(rows, i, &mut out);
            }
        }
    }
    out
}

/// `runs` with `repo`'s rows swapped for `fresh`.
pub fn replace_repo(runs: &[DeploymentRun], repo: &str, fresh: Vec<DeploymentRun>) -> Vec<DeploymentRun> {
    let mut all: Vec<DeploymentRun> = runs.iter().filter(|r| r.repo != repo).cloned().collect();
    all.extend(fresh);
    order(all)
}

/// How many rows wait for the user's approval.
pub fn approval_count(runs: &[DeploymentRun]) -> usize {
    runs.iter().filter(|r| matches!(r.wait, RunWait::Approval { .. })).count()
}

/// Which repos have an environment with required reviewers, as last asked.
#[derive(Default, Clone)]
pub struct ApprovalRepos {
    answers: HashMap<String, (bool, u64)>,
}

impl ApprovalRepos {
    /// The answer for `repo`, if it is under `ANSWER_TTL_MS` old.
    pub fn fresh(&self, repo: &str, now_ms: u64) -> Option<bool> {
        let (needs, at) = *self.answers.get(repo)?;
        (now_ms.saturating_sub(at) < ANSWER_TTL_MS).then_some(needs)
    }

    pub fn set(&mut self, repo: &str, needs: bool, now_ms: u64) {
        self.answers.insert(repo.to_string(), (needs, now_ms));
    }
}

/// The rows for one repo: runs waiting for the user, and runs queued behind another.
pub fn check_repo(http: &dyn Http, forge: &Forge, repo: &str) -> Result<Vec<DeploymentRun>, String> {
    let mut rows = Vec::new();
    let waiting = runs(http, forge, repo, "waiting", 50)?;
    for run in &waiting {
        let environments = pending_deployments(http, forge, repo, run.id)?;
        if !environments.is_empty() {
            rows.push(row(repo, run, RunWait::Approval { environments }));
        }
    }
    let pending = runs(http, forge, repo, "pending", 20)?;
    if pending.is_empty() {
        return Ok(rows);
    }
    // The live runs a pending one may wait behind; the extra lists are asked for only when needed.
    let mut others = waiting;
    let (mut got_in_progress, mut got_queued) = (false, false);
    for run in &pending {
        let mut by = find_blocker(run, &others);
        if by.is_none() && !got_in_progress {
            got_in_progress = true;
            others.extend(runs(http, forge, repo, "in_progress", 50)?);
            by = find_blocker(run, &others);
        }
        if by.is_none() && !got_queued {
            got_queued = true;
            others.extend(runs(http, forge, repo, "queued", 50)?);
            by = find_blocker(run, &others);
        }
        rows.push(row(repo, run, RunWait::Blocked { by }));
    }
    Ok(rows)
}

/// What one repo's work found.
struct Outcome {
    /// An answer to cache, when `repo_needs_approval` was asked.
    asked: Option<bool>,
    rows: Vec<DeploymentRun>,
    /// Whether the check made a call after the repo list.
    attempted: bool,
    error: Option<String>,
}

fn check_one(http: &dyn Http, forge: &Forge, cache: &ApprovalRepos, repo: &str, now_ms: u64) -> Outcome {
    let mut out = Outcome { asked: None, rows: Vec::new(), attempted: false, error: None };
    let needs = match cache.fresh(repo, now_ms) {
        Some(needs) => needs,
        None => {
            out.attempted = true;
            match repo_needs_approval(http, forge, repo) {
                Ok(needs) => {
                    out.asked = Some(needs);
                    needs
                }
                Err(e) => {
                    out.error = Some(e);
                    return out;
                }
            }
        }
    };
    if needs {
        out.attempted = true;
        match check_repo(http, forge, repo) {
            Ok(rows) => out.rows = rows,
            Err(e) => out.error = Some(e),
        }
    }
    out
}

/// Every row across the user's active repos, ordered. Fails when the repo
/// list fails, or when every repo the check tried refused the token.
pub fn fetch(http: &dyn Http, forge: &Forge, cache: &mut ApprovalRepos, now_ms: u64) -> Result<Vec<DeploymentRun>, String> {
    let repos = active_repos(http, forge, now_ms)?;
    let next = AtomicUsize::new(0);
    let shared: &ApprovalRepos = cache;
    let joined: Result<Vec<Vec<(usize, Outcome)>>, String> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..THREADS.min(repos.len()))
            .map(|_| {
                s.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(repo) = repos.get(i) else { break };
                        done.push((i, check_one(http, forge, shared, repo, now_ms)));
                    }
                    done
                })
            })
            .collect();
        workers.into_iter().map(|w| w.join().map_err(|_| "the check stopped unexpectedly".to_string())).collect()
    });
    let mut outcomes: Vec<(usize, Outcome)> = joined?.into_iter().flatten().collect();
    outcomes.sort_by_key(|(i, _)| *i);
    let (mut rows, mut attempted, mut refused, mut first_refusal) = (Vec::new(), 0, 0, None);
    for (i, out) in outcomes {
        let repo = &repos[i];
        if let Some(needs) = out.asked {
            cache.set(repo, needs, now_ms);
        }
        attempted += usize::from(out.attempted);
        if let Some(e) = out.error {
            crate::log::line("app", format!("deployments: {repo}: {e}"));
            if is_refusal(&e) {
                refused += 1;
                first_refusal.get_or_insert(e);
            }
        }
        rows.extend(out.rows);
    }
    match first_refusal {
        Some(e) if attempted > 0 && refused == attempted => Err(e),
        _ => Ok(order(rows)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(id: u64, number: u64, workflow_id: u64, status: &str, created_at: &str) -> Run {
        Run {
            id,
            number,
            workflow_id,
            name: "Deploy".into(),
            branch: "main".into(),
            sha: format!("sha{id}"),
            title: format!("run {id}"),
            actor: "tiago".into(),
            url: format!("https://github.com/O/r/actions/runs/{id}"),
            created_at: created_at.into(),
            status: status.into(),
        }
    }

    fn approval(repo: &str, id: u64, since: &str) -> DeploymentRun {
        row(repo, &run(id, id, 3, "waiting", since), RunWait::Approval { environments: vec![] })
    }

    fn blocked(repo: &str, id: u64, since: &str, by_run_id: Option<u64>) -> DeploymentRun {
        let by = by_run_id.map(|b| Blocker { run_id: b, sha: "s".into(), title: "t".into(), url: "u".into() });
        row(repo, &run(id, id, 3, "pending", since), RunWait::Blocked { by })
    }

    #[test]
    fn parse_runs_reads_the_fields_a_row_needs() {
        let v = json!({"workflow_runs": [{"id": 7, "run_number": 41, "workflow_id": 3, "name": "Deploy", "head_branch": "main",
            "head_sha": "a1b2c3d4e5", "head_commit": {"message": "fix login redirect\n\nbody"}, "triggering_actor": {"login": "tiago"},
            "actor": {"login": "bot"}, "html_url": "https://github.com/O/r/actions/runs/7", "created_at": "2026-10-28T08:00:00Z", "status": "waiting"}]});
        let r = &parse_runs(&v)[0];
        assert_eq!((r.id, r.number, r.workflow_id, r.title.as_str(), r.actor.as_str(), r.status.as_str()), (7, 41, 3, "fix login redirect", "tiago", "waiting"));
    }

    #[test]
    fn a_run_without_head_commit_or_triggering_actor_still_parses() {
        let v = json!({"workflow_runs": [{"id": 7, "run_number": 1, "workflow_id": 3, "name": "Deploy", "head_branch": "main", "head_sha": "s",
            "head_commit": null, "triggering_actor": null, "actor": {"login": "bot"}, "html_url": "u", "created_at": "c", "status": "pending"}]});
        let r = &parse_runs(&v)[0];
        assert_eq!((r.title.as_str(), r.actor.as_str()), ("", "bot"));
    }

    #[test]
    fn needs_approval_only_for_required_reviewers() {
        assert!(needs_approval(&json!({"environments": [{"protection_rules": [{"type": "branch_policy"}]}, {"protection_rules": [{"type": "required_reviewers"}]}]})));
        assert!(!needs_approval(&json!({"environments": [{"protection_rules": [{"type": "branch_policy"}]}]})));
        assert!(!needs_approval(&json!({"total_count": 0, "environments": []})));
    }

    #[test]
    fn approvable_keeps_only_what_the_user_can_approve() {
        let v = json!([{"environment": {"id": 1, "name": "production"}, "current_user_can_approve": true},
                       {"environment": {"id": 2, "name": "eu"}, "current_user_can_approve": false}]);
        assert_eq!(approvable(&v), vec![Environment { id: 1, name: "production".into() }]);
    }

    #[test]
    fn the_blocker_is_the_newest_earlier_live_run_of_the_same_workflow() {
        let pending = run(9, 12, 3, "pending", "t");
        let others = vec![run(5, 10, 3, "waiting", "t"), run(6, 11, 3, "in_progress", "t"), run(8, 11, 4, "waiting", "t"),
                          run(4, 9, 3, "completed", "t"), run(10, 13, 3, "queued", "t")];
        assert_eq!(find_blocker(&pending, &others).map(|b| b.run_id), Some(6));
        assert_eq!(find_blocker(&pending, &[run(8, 11, 4, "waiting", "t")]), None);
    }

    #[test]
    fn order_groups_by_repo_oldest_first_with_blocked_rows_under_their_blocker() {
        // Repo B's oldest row is older than A's, so B comes first; in A the blocked row follows its blocker.
        let a1 = approval("O/a", 1, "2026-10-28T09:00:00Z");
        let a2 = approval("O/a", 2, "2026-10-28T08:30:00Z");
        let a3 = blocked("O/a", 3, "2026-10-28T08:00:00Z", Some(1));
        let b1 = approval("O/b", 4, "2026-10-28T07:00:00Z");
        let ids: Vec<u64> = order(vec![a1, a2, a3, b1]).iter().map(|r| r.run_id).collect();
        assert_eq!(ids, vec![4, 2, 1, 3]);
    }

    #[test]
    fn replace_repo_swaps_one_repos_rows_and_the_count_counts_approvals() {
        let runs = order(vec![approval("O/a", 1, "t1"), blocked("O/a", 2, "t2", Some(1)), approval("O/b", 3, "t0")]);
        assert_eq!(approval_count(&runs), 2);
        let after = replace_repo(&runs, "O/a", vec![]);
        assert_eq!(after.iter().map(|r| r.run_id).collect::<Vec<_>>(), vec![3]);
    }

    use crate::config::{GitConnection, GitProvider};
    use crate::forge::{fake::FakeHttp, is_refusal, Forge, ForgeSite};

    fn forge() -> Forge {
        Forge { site: ForgeSite::of(&GitConnection::default_for(GitProvider::Github)), token: "ghp_x".into() }
    }

    fn now() -> u64 {
        crate::codex::ms_of("2026-10-28T12:00:00Z").unwrap()
    }

    fn runs_json(runs: &[(u64, u64, u64, &str)]) -> String {
        let list: Vec<Value> = runs
            .iter()
            .map(|(id, number, wf, status)| {
                json!({"id": id, "run_number": number, "workflow_id": wf, "name": "Deploy", "head_branch": "main", "head_sha": format!("sha{id}"),
                    "head_commit": {"message": format!("run {id}")}, "triggering_actor": {"login": "tiago"},
                    "html_url": format!("https://github.com/O/r/actions/runs/{id}"), "created_at": "2026-10-28T08:00:00Z", "status": status})
            })
            .collect();
        json!({"workflow_runs": list}).to_string()
    }

    const REPOS_A: &str = r#"[{"full_name":"O/a","pushed_at":"2026-10-28T00:00:00Z"}]"#;
    const REPOS_AB: &str = r#"[{"full_name":"O/a","pushed_at":"2026-10-28T00:00:00Z"},{"full_name":"O/b","pushed_at":"2026-10-28T00:00:00Z"}]"#;
    const REVIEWERS: &str = r#"{"environments":[{"protection_rules":[{"type":"required_reviewers"}]}]}"#;
    const CAN: &str = r#"[{"environment":{"id":1,"name":"production"},"current_user_can_approve":true}]"#;
    const CANNOT: &str = r#"[{"environment":{"id":1,"name":"production"},"current_user_can_approve":false}]"#;

    fn asked(http: &FakeHttp, needle: &str) -> usize {
        http.seen.lock().unwrap().iter().filter(|(u, _, _)| u.contains(needle)).count()
    }

    #[test]
    fn a_waiting_run_the_user_can_approve_is_a_row_and_one_waiting_on_others_is_not() {
        let waiting = runs_json(&[(7, 10, 3, "waiting"), (8, 11, 4, "waiting")]);
        let http = FakeHttp::new(vec![
            ("/user/repos", 200, REPOS_A),
            ("/repos/O/a/environments", 200, REVIEWERS),
            ("/runs/7/pending_deployments", 200, CAN),
            ("/runs/8/pending_deployments", 200, CANNOT),
            ("status=waiting", 200, &waiting),
            ("status=pending", 200, r#"{"workflow_runs":[]}"#),
        ]);
        let rows = fetch(&http, &forge(), &mut ApprovalRepos::default(), now()).unwrap();
        assert_eq!(rows.iter().map(|r| r.run_id).collect::<Vec<_>>(), vec![7]);
        assert_eq!(rows[0].wait, RunWait::Approval { environments: vec![Environment { id: 1, name: "production".into() }] });
        assert_eq!(rows[0].repo, "O/a");
    }

    #[test]
    fn a_pending_run_finds_its_blocker_and_only_asks_for_in_progress_when_needed() {
        let waiting = runs_json(&[(7, 10, 3, "waiting")]);
        let pending = runs_json(&[(9, 12, 3, "pending")]);
        let http = FakeHttp::new(vec![
            ("/runs/7/pending_deployments", 200, "[]"),
            ("status=waiting", 200, &waiting),
            ("status=pending", 200, &pending),
        ]);
        let rows = check_repo(&http, &forge(), "O/a").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].run_id, 9);
        assert!(matches!(&rows[0].wait, RunWait::Blocked { by: Some(b) } if b.run_id == 7));
        assert_eq!(asked(&http, "status=in_progress") + asked(&http, "status=queued"), 0);
    }

    #[test]
    fn a_pending_run_with_no_live_run_in_its_workflow_waits_on_another_deployment() {
        let pending = runs_json(&[(9, 12, 3, "pending")]);
        let empty = r#"{"workflow_runs":[]}"#;
        let http = FakeHttp::new(vec![
            ("status=waiting", 200, empty),
            ("status=in_progress", 200, empty),
            ("status=queued", 200, empty),
            ("status=pending", 200, &pending),
        ]);
        let rows = check_repo(&http, &forge(), "O/a").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].run_id, &rows[0].wait), (9, &RunWait::Blocked { by: None }));
        assert_eq!((asked(&http, "status=in_progress"), asked(&http, "status=queued")), (1, 1));
    }

    #[test]
    fn repos_without_required_reviewers_are_not_checked_and_the_answer_lasts_an_hour() {
        let http = FakeHttp::new(vec![
            ("/user/repos", 200, REPOS_A),
            ("/repos/O/a/environments", 200, r#"{"environments":[]}"#),
        ]);
        let mut cache = ApprovalRepos::default();
        let t = now();
        assert_eq!(fetch(&http, &forge(), &mut cache, t).unwrap(), vec![]);
        assert_eq!((asked(&http, "/environments"), asked(&http, "/actions/runs")), (1, 0));
        fetch(&http, &forge(), &mut cache, t + 59 * 60_000).unwrap();
        assert_eq!(asked(&http, "/environments"), 1);
        fetch(&http, &forge(), &mut cache, t + 61 * 60_000).unwrap();
        assert_eq!(asked(&http, "/environments"), 2);
        assert_eq!(asked(&http, "/actions/runs"), 0);
    }

    #[test]
    fn a_failing_repo_is_skipped_and_the_others_show() {
        let waiting = runs_json(&[(7, 10, 3, "waiting")]);
        let http = FakeHttp::new(vec![
            ("/user/repos", 200, REPOS_AB),
            ("/repos/O/a/environments", 500, "boom"),
            ("/repos/O/b/environments", 200, REVIEWERS),
            ("/repos/O/b/actions/runs/7/pending_deployments", 200, CAN),
            ("/repos/O/b/actions/runs?status=waiting", 200, &waiting),
            ("/repos/O/b/actions/runs?status=pending", 200, r#"{"workflow_runs":[]}"#),
        ]);
        let rows = fetch(&http, &forge(), &mut ApprovalRepos::default(), now()).unwrap();
        assert_eq!(rows.iter().map(|r| (r.repo.as_str(), r.run_id)).collect::<Vec<_>>(), vec![("O/b", 7)]);
    }

    #[test]
    fn every_repo_refusing_is_an_error_naming_the_permissions() {
        let http = FakeHttp::new(vec![("/user/repos", 200, REPOS_AB), ("/environments", 403, "{}")]);
        let mut cache = ApprovalRepos::default();
        let e = fetch(&http, &forge(), &mut cache, now()).unwrap_err();
        assert!(is_refusal(&e) && e.contains("Actions read"), "{e}");
        assert_eq!(cache.fresh("O/a", now()), None);
    }

    #[test]
    fn the_cache_answers_only_while_fresh() {
        let mut cache = ApprovalRepos::default();
        assert_eq!(cache.fresh("O/a", 0), None);
        cache.set("O/a", true, 1_000);
        assert_eq!(cache.fresh("O/a", 1_000 + ANSWER_TTL_MS - 1), Some(true));
        assert_eq!(cache.fresh("O/a", 1_000 + ANSWER_TTL_MS), None);
    }

    struct PanicOnB(FakeHttp);

    impl Http for PanicOnB {
        fn get(&self, url: &str, token: &str) -> Result<(u16, String), String> {
            assert!(!url.contains("/repos/O/b/"), "boom");
            self.0.get(url, token)
        }
        fn post_json(&self, url: &str, token: &str, body: &str) -> Result<(u16, String), String> {
            self.0.post_json(url, token, body)
        }
    }

    #[test]
    fn a_stopped_worker_fails_the_check_and_leaves_the_cache_alone() {
        let http = PanicOnB(FakeHttp::new(vec![("/user/repos", 200, REPOS_AB), ("/repos/O/a/environments", 200, r#"{"environments":[]}"#)]));
        let mut cache = ApprovalRepos::default();
        assert_eq!(fetch(&http, &forge(), &mut cache, now()), Err("the check stopped unexpectedly".to_string()));
        assert_eq!(cache.fresh("O/a", now()), None);
    }

    #[test]
    fn one_repo_refusing_while_another_works_is_not_an_error() {
        let waiting = runs_json(&[(7, 10, 3, "waiting")]);
        let http = FakeHttp::new(vec![
            ("/user/repos", 200, REPOS_AB),
            ("/repos/O/a/environments", 403, "{}"),
            ("/repos/O/b/environments", 200, REVIEWERS),
            ("/repos/O/b/actions/runs/7/pending_deployments", 200, CAN),
            ("/repos/O/b/actions/runs?status=waiting", 200, &waiting),
            ("/repos/O/b/actions/runs?status=pending", 200, r#"{"workflow_runs":[]}"#),
        ]);
        let rows = fetch(&http, &forge(), &mut ApprovalRepos::default(), now()).unwrap();
        assert_eq!(rows.iter().map(|r| r.run_id).collect::<Vec<_>>(), vec![7]);
    }
}
