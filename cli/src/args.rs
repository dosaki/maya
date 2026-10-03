//! The hand-rolled argument parser for the `maya` binary.

use maya_core::launch::LaunchOptions;

#[derive(Debug, PartialEq)]
pub enum Cmd {
    Pair { host: String, port: u16, code: String, name: Option<String> },
    Run,
    Status,
    Hooks(HooksOp),
    Config(ConfigOp),
    Start { dir: Option<String>, prompt: Option<String>, options: LaunchOptions },
    Version,
    Help,
}

#[derive(Debug, PartialEq)]
pub enum HooksOp {
    Install,
    Remove,
    Status,
}

#[derive(Debug, PartialEq)]
pub enum ConfigOp {
    /// `maya config projects-dir <path>`: the folder start and resume pick projects from.
    ProjectsDir(String),
}

pub const USAGE: &str = "usage: maya <pair|run|status|hooks|config|start|--version> …
       maya start [--dir <dir>] [--prompt <prompt>] [--agent <claude-code|codex|antigravity|grok>] [--model <model>] [--effort <effort>] [--mode <mode>] [--name <name>]";

/// The main Maya's default port when none is given after the host.
const DEFAULT_PORT: u16 = 4127;

pub fn parse(args: &[String]) -> Result<Cmd, String> {
    let Some(cmd) = args.first() else { return Ok(Cmd::Help) };
    match cmd.as_str() {
        "--help" => no_more(&args[1..], Cmd::Help),
        "--version" => no_more(&args[1..], Cmd::Version),
        "pair" => parse_pair(&args[1..]),
        "run" => no_more(&args[1..], Cmd::Run),
        "status" => no_more(&args[1..], Cmd::Status),
        "hooks" => parse_hooks(&args[1..]),
        "config" => parse_config(&args[1..]),
        "start" => parse_start(&args[1..]),
        other => Err(format!("{USAGE}\nunknown command: {other}")),
    }
}

/// `cmd` when nothing follows it; a usage error naming the first stray token otherwise.
fn no_more(rest: &[String], cmd: Cmd) -> Result<Cmd, String> {
    match rest.first() {
        None => Ok(cmd),
        Some(extra) => Err(format!("{USAGE}\nunexpected argument: {extra}")),
    }
}

/// Takes the flag's value from the next argument, advancing `i` past both;
/// a usage error naming the flag when there isn't one.
fn take_value(rest: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    let value = rest.get(*i + 1).cloned().ok_or_else(|| format!("{USAGE}\n{flag} needs a value"))?;
    *i += 2;
    Ok(value)
}

/// Splits `host[:port]` on the last `:`; `DEFAULT_PORT` when there is none.
fn split_host_port(s: &str) -> Result<(String, u16), String> {
    match s.rsplit_once(':') {
        Some((host, port)) => {
            let port: u16 = port.parse().ok().filter(|p| *p != 0).ok_or_else(|| format!("{USAGE}\nbad port: {port}"))?;
            Ok((host.to_string(), port))
        }
        None => Ok((s.to_string(), DEFAULT_PORT)),
    }
}

fn parse_pair(rest: &[String]) -> Result<Cmd, String> {
    let mut host: Option<String> = None;
    let mut code: Option<String> = None;
    let mut name: Option<String> = None;
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--code" => code = Some(take_value(rest, &mut i, "--code")?),
            "--name" => name = Some(take_value(rest, &mut i, "--name")?),
            s if s.starts_with("--") => return Err(format!("{USAGE}\nunknown flag: {s}")),
            s if host.is_none() => {
                host = Some(s.to_string());
                i += 1;
            }
            s => return Err(format!("{USAGE}\nunexpected argument: {s}")),
        }
    }
    let host = host.ok_or_else(|| format!("{USAGE}\npair needs a host"))?;
    let code = code.ok_or_else(|| format!("{USAGE}\npair needs --code"))?;
    let (host, port) = split_host_port(&host)?;
    Ok(Cmd::Pair { host, port, code, name })
}

fn parse_hooks(rest: &[String]) -> Result<Cmd, String> {
    let op = match rest.first().map(String::as_str) {
        Some("install") => HooksOp::Install,
        Some("remove") => HooksOp::Remove,
        Some("status") => HooksOp::Status,
        _ => return Err(format!("{USAGE}\nhooks needs one of: install|remove|status")),
    };
    no_more(&rest[1..], Cmd::Hooks(op))
}

fn parse_config(rest: &[String]) -> Result<Cmd, String> {
    match rest {
        [op, path] if op == "projects-dir" => Ok(Cmd::Config(ConfigOp::ProjectsDir(path.clone()))),
        [op, _, extra, ..] if op == "projects-dir" => Err(format!("{USAGE}\nunexpected argument: {extra}")),
        _ => Err(format!("{USAGE}\nconfig needs: projects-dir <path>")),
    }
}

fn parse_start(rest: &[String]) -> Result<Cmd, String> {
    let mut dir: Option<String> = None;
    let mut prompt: Option<String> = None;
    let mut options = LaunchOptions::default();
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--dir" => dir = Some(take_value(rest, &mut i, "--dir")?),
            "--prompt" => prompt = Some(take_value(rest, &mut i, "--prompt")?),
            "--model" => options.model = Some(take_value(rest, &mut i, "--model")?),
            "--effort" => options.effort = Some(take_value(rest, &mut i, "--effort")?),
            "--mode" => options.mode = Some(take_value(rest, &mut i, "--mode")?),
            "--agent" => {
                let v = take_value(rest, &mut i, "--agent")?;
                // The decode fallback for agents this build does not know is not a choice.
                let agent = serde_json::from_value::<maya_core::model::Harness>(serde_json::Value::String(v.clone())).ok().filter(|a| *a != maya_core::model::Harness::Other);
                options.agent = agent.ok_or_else(|| format!("{USAGE}\nunknown agent: {v} (claude-code, codex, antigravity, grok or kiro)"))?;
            }
            "--name" => options.name = Some(take_value(rest, &mut i, "--name")?),
            s => return Err(format!("{USAGE}\nunknown flag: {s}")),
        }
    }
    Ok(Cmd::Start { dir, prompt, options })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn a(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn pair_takes_host_port_code_and_name() {
        assert_eq!(parse(&a("pair 10.0.0.5 --code 483921")).unwrap(), Cmd::Pair { host: "10.0.0.5".into(), port: 4127, code: "483921".into(), name: None });
        assert_eq!(parse(&a("pair desk.local:5000 --code 111222 --name box")).unwrap(), Cmd::Pair { host: "desk.local".into(), port: 5000, code: "111222".into(), name: Some("box".into()) });
        assert!(parse(&a("pair 10.0.0.5")).unwrap_err().contains("--code"));
        assert!(parse(&a("pair --code 1")).unwrap_err().contains("host"));
        assert!(parse(&a("pair desk.local:0 --code 111222")).unwrap_err().contains("bad port: 0"));
    }

    #[test]
    fn the_other_subcommands_parse() {
        assert_eq!(parse(&a("run")).unwrap(), Cmd::Run);
        assert_eq!(parse(&a("status")).unwrap(), Cmd::Status);
        assert_eq!(parse(&a("hooks install")).unwrap(), Cmd::Hooks(HooksOp::Install));
        assert_eq!(parse(&a("hooks status")).unwrap(), Cmd::Hooks(HooksOp::Status));
        assert_eq!(parse(&a("--version")).unwrap(), Cmd::Version);
        assert_eq!(parse(&a("")).unwrap(), Cmd::Help);
        assert!(parse(&a("dance")).unwrap_err().starts_with("usage:"));
        assert!(parse(&a("hooks")).unwrap_err().contains("install|remove|status"));
    }

    #[test]
    fn commands_without_arguments_refuse_a_trailing_token() {
        assert_eq!(parse(&a("--help")).unwrap(), Cmd::Help);
        for (line, stray) in [("run --foo", "--foo"), ("status now", "now"), ("--version x", "x"), ("--help run", "run"), ("run a b", "a")] {
            let err = parse(&a(line)).unwrap_err();
            assert!(err.starts_with("usage:"), "{line}: {err}");
            assert!(err.ends_with(&format!("\nunexpected argument: {stray}")), "{line}: {err}");
        }
    }

    #[test]
    fn hooks_takes_exactly_one_operation() {
        assert_eq!(parse(&a("hooks remove")).unwrap(), Cmd::Hooks(HooksOp::Remove));
        assert!(parse(&a("hooks install typo")).unwrap_err().ends_with("\nunexpected argument: typo"));
        assert!(parse(&a("hooks dance")).unwrap_err().contains("install|remove|status"));
    }

    #[test]
    fn config_projects_dir_takes_one_path() {
        assert_eq!(parse(&["config", "projects-dir", "~/dev projects"].map(String::from)).unwrap(), Cmd::Config(ConfigOp::ProjectsDir("~/dev projects".into())));
        assert!(parse(&a("config projects-dir")).unwrap_err().contains("projects-dir <path>"));
        assert!(parse(&a("config")).unwrap_err().contains("projects-dir <path>"));
        assert!(parse(&a("config colour red")).unwrap_err().contains("projects-dir <path>"));
        assert!(parse(&a("config projects-dir a b")).unwrap_err().contains("unexpected argument: b"));
    }

    #[test]
    fn start_takes_dir_prompt_and_launch_options() {
        let c = parse(&["start", "--dir", "proj", "--prompt", "fix the tests", "--model", "opus"].map(String::from)).unwrap();
        match c {
            Cmd::Start { dir, prompt, options } => {
                assert_eq!(dir.as_deref(), Some("proj"));
                assert_eq!(prompt.as_deref(), Some("fix the tests"));
                assert_eq!(options.model.as_deref(), Some("opus"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn start_takes_an_agent_and_a_name() {
        let Cmd::Start { options, .. } = parse(&["start".into(), "--agent".into(), "codex".into(), "--name".into(), "Fix CI".into(), "--prompt".into(), "go".into()]).unwrap() else { panic!() };
        assert_eq!(options.agent, maya_core::model::Harness::Codex);
        assert_eq!(options.name.as_deref(), Some("Fix CI"));
        assert!(parse(&a("start --agent gpt")).unwrap_err().contains("unknown agent: gpt"));
    }
}
