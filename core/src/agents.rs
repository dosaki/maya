//! The agents installed on this machine, and the models each offers, from the
//! agents' own listings (`codex debug models`, `agy models`, `grok models`).
//! A listing can take seconds (`agy models` asks a server), so the list is
//! cached for ten minutes and refreshed in the background.

use crate::launch;
use crate::model::Harness;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{mpsc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
    /// The efforts this model takes when they differ by model (Codex);
    /// empty means the agent's own list.
    #[serde(default)]
    pub efforts: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub harness: Harness,
    /// Defaulted so a board from a Maya that leaves a list out still reads.
    #[serde(default)]
    pub models: Vec<ModelInfo>,
    #[serde(default)]
    pub efforts: Vec<String>,
    #[serde(default)]
    pub modes: Vec<String>,
}

impl AgentInfo {
    pub fn model_ids(&self) -> Vec<String> {
        self.models.iter().map(|m| m.id.clone()).collect()
    }
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// Claude Code, with Maya's fixed lists: its model aliases need no listing.
pub fn claude() -> AgentInfo {
    let label = |id: &str| id[..1].to_uppercase() + &id[1..];
    AgentInfo {
        harness: Harness::ClaudeCode,
        models: launch::MODELS.iter().map(|id| ModelInfo { id: id.to_string(), label: label(id), efforts: vec![] }).collect(),
        efforts: strings(launch::EFFORTS),
        modes: strings(launch::MODES),
    }
}

/// Models from `codex debug models`: those in Codex's own picker
/// (`visibility: "list"`), with the efforts each takes.
pub fn parse_codex(json: &str) -> Vec<ModelInfo> {
    let v: Value = serde_json::from_str(json).unwrap_or(Value::Null);
    let known = launch::efforts(Harness::Codex);
    v["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["visibility"].as_str() == Some("list"))
        .filter_map(|m| {
            let id = m["slug"].as_str().filter(|id| launch::plain_model_id(id))?;
            let efforts = m["supported_reasoning_levels"].as_array().into_iter().flatten().filter_map(|l| l["effort"].as_str()).filter(|e| known.contains(e)).map(String::from).collect();
            Some(ModelInfo { id: id.to_string(), label: m["display_name"].as_str().unwrap_or(id).to_string(), efforts })
        })
        .collect()
}

/// Models from `agy models`: `id<TAB>label` lines after "Fetching available models...".
pub fn parse_agy(text: &str) -> Vec<ModelInfo> {
    text.lines()
        .filter_map(|l| {
            let (id, label) = l.split_once('\t')?;
            let id = id.trim();
            launch::plain_model_id(id).then(|| ModelInfo { id: id.to_string(), label: label.trim().to_string(), efforts: vec![] })
        })
        .collect()
}

/// Models from `grok models`: the indented lines under "Available models:",
/// the default one marked `* … (default)`.
pub fn parse_grok(text: &str) -> Vec<ModelInfo> {
    text.lines()
        .skip_while(|l| l.trim() != "Available models:")
        .skip(1)
        .map(str::trim)
        .take_while(|l| !l.is_empty())
        .filter_map(|l| {
            let id = l.trim_start_matches('*').trim().trim_end_matches("(default)").trim();
            launch::plain_model_id(id).then(|| ModelInfo { id: id.to_string(), label: id.to_string(), efforts: vec![] })
        })
        .collect()
}

/// The arguments that make an agent print its models.
fn listing_args(agent: Harness) -> &'static [&'static str] {
    match agent {
        Harness::Codex => &["debug", "models"],
        Harness::Antigravity | Harness::Grok => &["models"],
        Harness::ClaudeCode => &[],
    }
}

/// An agent's choices from its listing; with none (not run, failed, timed
/// out) it offers no models, so only "Default", and Codex, whose efforts
/// come from its listing, offers no efforts either.
pub fn info_for(agent: Harness, listing: Option<&str>) -> AgentInfo {
    if agent == Harness::ClaudeCode {
        return claude();
    }
    let models = match (agent, listing) {
        (Harness::Codex, Some(t)) => parse_codex(t),
        (Harness::Antigravity, Some(t)) => parse_agy(t),
        (Harness::Grok, Some(t)) => parse_grok(t),
        _ => vec![],
    };
    // With "Default" chosen the model may be any of them: offer what all take.
    // With none listed nothing is known about what Codex's default takes, so
    // no effort is offered rather than every one.
    let efforts = if agent == Harness::Codex {
        launch::efforts(agent).iter().filter(|e| !models.is_empty() && models.iter().all(|m| m.efforts.iter().any(|x| x == *e))).map(|e| e.to_string()).collect()
    } else {
        strings(launch::efforts(agent))
    };
    AgentInfo { harness: agent, models, efforts, modes: strings(launch::modes(agent)) }
}

/// Each installed agent, Claude Code first, with the models `run` lists for it.
pub fn build(find: impl Fn(&str) -> Option<PathBuf>, run: impl Fn(&Path, &[&str]) -> Option<String>) -> Vec<AgentInfo> {
    let mut out = vec![];
    if find("claude").is_some() {
        out.push(claude());
    }
    for agent in [Harness::Codex, Harness::Antigravity, Harness::Grok] {
        let Some(bin) = find(launch::binary_name(agent)) else { continue };
        let listing = run(&bin, listing_args(agent));
        out.push(info_for(agent, listing.as_deref()));
    }
    out
}

pub const LISTING_TIMEOUT: Duration = Duration::from_secs(20);

/// What `binary args` printed, if it exited successfully within `timeout`.
/// Stdout is read on a thread of its own: Codex's catalogue is far bigger
/// than a pipe holds, and a child blocked on a full pipe never exits.
pub fn run_listing(binary: &Path, args: &[&str], timeout: Duration) -> Option<String> {
    let mut child = crate::command(binary)
        .args(args)
        .env_clear()
        .envs(launch::clean_env(std::env::vars()))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut stdout, &mut s);
        let _ = tx.send(s);
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    // A helper the agent left running may hold stdout open: do not wait on it for long.
    let out = rx.recv_timeout(Duration::from_secs(2)).ok()?;
    status.success().then_some(out)
}

const MAX_AGE: Duration = Duration::from_secs(600);

struct Cache {
    list: Option<Vec<AgentInfo>>,
    at: Option<Instant>,
    fetching: bool,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache { list: None, at: None, fetching: false });
static READY: Condvar = Condvar::new();

/// Starts a listing on a thread of its own when the last is missing or stale.
fn refresh_if_stale(c: &mut Cache) {
    if c.fetching || c.at.is_some_and(|at| at.elapsed() < MAX_AGE) {
        return;
    }
    c.fetching = true;
    std::thread::spawn(|| {
        // Runs however the listing ends, a panic included: left fetching, no
        // listing would start again, and `current` would wait forever. A
        // listing that never landed leaves Claude Code alone, and with no
        // `at` the next call lists again.
        struct Done;
        impl Drop for Done {
            fn drop(&mut self) {
                let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
                c.list.get_or_insert_with(|| vec![claude()]);
                c.fetching = false;
                READY.notify_all();
            }
        }
        let _done = Done;
        let list = build(launch::agent_binary, |bin, args| run_listing(bin, args, LISTING_TIMEOUT));
        let mut c = CACHE.lock().unwrap();
        c.list = Some(list);
        c.at = Some(Instant::now());
    });
}

/// The agents as last listed, starting a new listing when that one is over
/// ten minutes old. Never waits: before the first listing it is Claude Code alone.
pub fn snapshot() -> Vec<AgentInfo> {
    let mut c = CACHE.lock().unwrap();
    refresh_if_stale(&mut c);
    c.list.clone().unwrap_or_else(|| vec![claude()])
}

/// The agents, waiting for the first listing when there is none yet.
pub fn current() -> Vec<AgentInfo> {
    let mut c = CACHE.lock().unwrap();
    refresh_if_stale(&mut c);
    while c.list.is_none() {
        c = READY.wait(c).unwrap();
    }
    c.list.clone().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn fixture(rel: &str) -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(rel)).unwrap()
    }

    #[test]
    fn codex_models_are_the_listed_ones_with_their_efforts() {
        let m = parse_codex(&fixture("codex/models.json"));
        assert_eq!(m.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["gpt-6.1-sol", "gpt-6-luna", "gpt-5.5"], "gpt-reserve is hidden");
        assert_eq!(m[0].label, "GPT-6.1-Sol");
        assert_eq!(m[2].efforts, vec!["low", "medium", "high", "xhigh"]);
        assert!(parse_codex("not json").is_empty());
    }

    #[test]
    fn antigravity_models_are_the_tab_separated_lines() {
        let m = parse_agy(&fixture("antigravity/models.txt"));
        assert_eq!(m[0], ModelInfo { id: "gemini-3.8-flash-high".into(), label: "Gemini 3.8 Flash (High)".into(), efforts: vec![] });
        assert!(m.iter().all(|m| !m.id.starts_with("Fetching")));
        assert_eq!(m.len(), 14);
    }

    #[test]
    fn grok_models_are_the_lines_under_available_models() {
        assert_eq!(parse_grok(&fixture("grok/models.txt")).iter().map(|m| m.id.clone()).collect::<Vec<_>>(), vec!["grok-4.7"]);
        let two = "Default model: a\n\nAvailable models:\n  * grok-4.7 (default)\n    grok-5-mini\n";
        assert_eq!(parse_grok(two).iter().map(|m| m.id.clone()).collect::<Vec<_>>(), vec!["grok-4.7", "grok-5-mini"]);
        assert!(parse_grok("You are not authenticated.").is_empty());
    }

    #[test]
    fn codex_default_model_offers_the_efforts_every_model_takes() {
        let info = info_for(Harness::Codex, Some(&fixture("codex/models.json")));
        assert_eq!(info.efforts, vec!["low", "medium", "high", "xhigh"]);
        assert_eq!(info.modes, vec!["read-only", "workspace-write", "danger-full-access"]);
    }

    #[test]
    fn codex_without_a_listing_offers_no_efforts() {
        // Nothing listed means nothing known about what "Default" takes.
        assert!(info_for(Harness::Codex, None).efforts.is_empty());
        assert!(info_for(Harness::Codex, Some("not json")).efforts.is_empty());
    }

    #[test]
    fn info_without_a_listing_offers_default_only() {
        for agent in [Harness::Codex, Harness::Antigravity, Harness::Grok] {
            let info = info_for(agent, None);
            assert!(info.models.is_empty(), "{agent:?}");
            assert_eq!(info.modes, launch::modes(agent).iter().map(|s| s.to_string()).collect::<Vec<_>>());
        }
    }

    #[test]
    fn build_lists_only_the_agents_found() {
        let none = build(|_| None, |_, _| None);
        assert!(none.is_empty(), "no agent installed, none listed: {none:?}");
        let only_grok = build(|name| (name == "grok").then(|| PathBuf::from("/bin/grok")), |_, _| Some("Available models:\n  grok-4.7 (default)\n".into()));
        assert_eq!(only_grok.iter().map(|a| a.harness).collect::<Vec<_>>(), vec![Harness::Grok]);
        let both = build(|name| matches!(name, "claude" | "codex").then(|| PathBuf::from(format!("/bin/{name}"))), |_, _| None);
        assert_eq!(both.iter().map(|a| a.harness).collect::<Vec<_>>(), vec![Harness::ClaudeCode, Harness::Codex], "Claude Code stays first");
    }

    #[test]
    fn build_lists_claude_then_each_agent_it_finds() {
        let found = |name: &str| matches!(name, "claude" | "codex" | "grok").then(|| PathBuf::from(format!("/bin/{name}")));
        let run = |bin: &Path, args: &[&str]| -> Option<String> {
            match (bin.to_str().unwrap(), args) {
                ("/bin/codex", ["debug", "models"]) => Some(fixture("codex/models.json")),
                ("/bin/grok", ["models"]) => None,
                other => panic!("unexpected {other:?}"),
            }
        };
        let list = build(found, run);
        assert_eq!(list.iter().map(|a| a.harness).collect::<Vec<_>>(), vec![Harness::ClaudeCode, Harness::Codex, Harness::Grok]);
        assert_eq!(list[0], claude());
        assert_eq!(list[1].models.len(), 3);
        assert!(list[2].models.is_empty(), "a failed listing still lists the agent");
    }

    #[test]
    fn claude_offers_maya_s_fixed_lists() {
        let c = claude();
        assert_eq!(c.model_ids(), vec!["fable", "opus", "sonnet", "haiku"]);
        assert_eq!(c.models[1].label, "Opus");
        assert_eq!(c.efforts.len(), 5);
        assert_eq!(c.modes.len(), 6);
    }

    #[cfg(unix)]
    fn script(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join("lister");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[cfg(unix)]
    #[test]
    fn run_listing_reads_output_bigger_than_a_pipe() {
        let t = tempfile::tempdir().unwrap();
        let bin = script(t.path(), "head -c 300000 /dev/zero | tr '\\0' x");
        let out = run_listing(&bin, &[], Duration::from_secs(10)).unwrap();
        assert_eq!(out.len(), 300_000);
    }

    #[cfg(unix)]
    #[test]
    fn run_listing_gives_up_after_the_timeout_or_a_failure() {
        let t = tempfile::tempdir().unwrap();
        let slow = script(t.path(), "sleep 5; echo late");
        let start = std::time::Instant::now();
        assert_eq!(run_listing(&slow, &[], Duration::from_secs(1)), None);
        assert!(start.elapsed() < Duration::from_secs(4));
        let failing = script(t.path(), "echo partial; exit 3");
        assert_eq!(run_listing(&failing, &[], Duration::from_secs(5)), None);
    }
}
