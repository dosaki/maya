use serde::Serialize;
use serde_json::Value;

/// A repo whose last push is older than this is not checked, in milliseconds.
pub const ACTIVE_WINDOW_MS: u64 = 7 * 86_400_000;
/// How long a repo's "needs approval" answer is kept, in milliseconds.
pub const ANSWER_TTL_MS: u64 = 3_600_000;
/// How many pages of runs to read per repo.
pub const REPO_PAGES: u32 = 5;

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
}
