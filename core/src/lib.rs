pub mod actions;
pub mod agents;
pub mod answer;
pub mod antigravity;
pub mod attachments;
pub mod codex;
pub mod codex_hooks;
pub mod config;
pub mod context;
pub mod events;
pub mod foreign;
pub mod grok;
pub mod kiro;
pub mod hook_install;
pub mod inbox;
pub mod interpreter;
pub mod launch;
pub mod log;
pub mod model;
pub mod net;
pub mod notify;
pub mod opencode;
pub mod pending_names;
pub mod pr;
pub mod registry;
pub mod resume;
pub mod reviews;
pub mod state;
pub mod store;
pub mod terminal;
pub mod terminal_tmux;
pub mod transcript;
pub mod tty;
pub mod watcher;
#[cfg(windows)]
pub mod notify_win;
#[cfg(windows)]
pub mod win_console;
#[cfg(windows)]
pub mod win_process;

/// `~/.claude`, or `/.claude` when the home directory is unknown.
pub fn claude_dir() -> std::path::PathBuf {
    dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/")).join(".claude")
}

/// A `Command` for `program` that, on Windows, opens no console window:
/// Maya is a windowed app, and every console program it runs (gh, git,
/// curl, claude -p) would otherwise flash one up.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    #[allow(unused_mut)]
    let mut cmd = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// A `command` for GitHub's `gh`, found the way the agents are: Maya opened
/// from the Dock gets launchd's bare PATH, which leaves out Homebrew. Only a
/// hit is remembered, so a `gh` installed later is still found. The token
/// comes from this process's environment, else from the user's interactive
/// shell (`shell_gh_tokens`).
pub fn gh() -> std::process::Command {
    static FOUND: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    let mut cmd = match FOUND.get() {
        Some(p) => command(p),
        None => match launch::agent_binary("gh") {
            Some(p) => command(FOUND.get_or_init(|| p)),
            None => command("gh"),
        },
    };
    let own = GH_TOKEN_VARS.iter().filter_map(|n| Some((n.to_string(), std::env::var_os(n)?.to_string_lossy().into_owned())));
    if !has_gh_token(own) {
        cmd.envs(shell_gh_tokens().iter().cloned());
    }
    cmd
}

/// The environment variables `gh` reads its token from, in its order of
/// precedence.
pub const GH_TOKEN_VARS: [&str; 2] = ["GH_TOKEN", "GITHUB_TOKEN"];

/// True when `vars` carries a token `gh` would use: an empty value counts
/// as none.
pub fn has_gh_token(vars: impl IntoIterator<Item = (String, String)>) -> bool {
    vars.into_iter().any(|(k, v)| GH_TOKEN_VARS.contains(&k.as_str()) && !v.is_empty())
}

const GH_PROBE_BEGIN: &str = "MAYA-GH-BEGIN";
const GH_PROBE_END: &str = "MAYA-GH-END";

/// The shell line that prints each of `GH_TOKEN_VARS` as `NAME=value`
/// between two marker lines, so whatever the user's rc files print around
/// them is told apart from the tokens. Each item starts on a fresh line, so
/// rc output without a trailing newline does not glue onto the first
/// marker; the expansions are unset-safe, so an rc file's `set -u` does not
/// abort the line.
pub fn gh_token_probe() -> String {
    let vars: Vec<String> = GH_TOKEN_VARS.iter().map(|n| format!("\"{n}=${{{n}-}}\"")).collect();
    format!("printf '\\n%s\\n' {GH_PROBE_BEGIN} {} {GH_PROBE_END}", vars.join(" "))
}

/// The non-empty `NAME=value` pairs between the probe's markers in `out`;
/// nothing when the markers are missing.
pub fn parse_gh_token_probe(out: &str) -> Vec<(String, String)> {
    out.lines()
        .skip_while(|l| l.trim() != GH_PROBE_BEGIN)
        .skip(1)
        .take_while(|l| l.trim() != GH_PROBE_END)
        .filter_map(|l| l.split_once('='))
        .filter(|(k, v)| GH_TOKEN_VARS.contains(k) && !v.is_empty())
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// The token variables the user's interactive login shell exports, asked
/// once per launch: a token set in `~/.zshrc` reaches a terminal but not an
/// app opened from the Dock, which gets launchd's bare environment, nor a
/// plain login shell, which reads no rc file. Only Unix shells are asked;
/// elsewhere a windowed app already gets the user's environment.
fn shell_gh_tokens() -> &'static Vec<(String, String)> {
    static TOKENS: std::sync::OnceLock<Vec<(String, String)>> = std::sync::OnceLock::new();
    TOKENS.get_or_init(|| {
        #[cfg(unix)]
        {
            use std::process::Stdio;
            let out = command(launch::LOGIN_SHELL).args(["-lic", &gh_token_probe()]).stdin(Stdio::null()).stderr(Stdio::null()).output();
            match out {
                Ok(o) => parse_gh_token_probe(&String::from_utf8_lossy(&o.stdout)),
                Err(_) => Vec::new(),
            }
        }
        #[cfg(not(unix))]
        {
            Vec::new()
        }
    })
}

/// Epoch milliseconds now.
pub fn now_ms() -> u64 {
    store::now_ms()
}

#[cfg(test)]
mod gh_token_tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn a_token_is_present_only_when_a_gh_variable_is_non_empty() {
        assert!(has_gh_token(vars(&[("GITHUB_TOKEN", "ghp_x")])));
        assert!(has_gh_token(vars(&[("GH_TOKEN", "ghp_x")])));
        assert!(!has_gh_token(vars(&[("GITHUB_TOKEN", ""), ("PATH", "/usr/bin")])));
        assert!(!has_gh_token(vars(&[("GITHUB_PERSONAL_ACCESS_TOKEN", "ghp_x")])));
    }

    #[test]
    fn the_probe_output_yields_the_tokens_between_the_markers_ignoring_rc_noise() {
        let out = "Restored session: Thu  8 Oct 2026\nMAYA-GH-BEGIN\n\nGH_TOKEN=\n\nGITHUB_TOKEN=ghp_abc=def\n\nMAYA-GH-END\nSaving session...completed.\n";
        assert_eq!(parse_gh_token_probe(out), vars(&[("GITHUB_TOKEN", "ghp_abc=def")]));
    }

    #[test]
    fn a_probe_without_markers_yields_nothing() {
        assert_eq!(parse_gh_token_probe("GITHUB_TOKEN=ghp_x\n"), vec![]);
        assert_eq!(parse_gh_token_probe(""), vec![]);
    }

    #[test]
    fn the_probe_prints_every_gh_variable_between_the_markers() {
        let line = gh_token_probe();
        // A newline first, so rc output without one does not glue onto the marker.
        assert!(line.starts_with("printf '\\n%s\\n' MAYA-GH-BEGIN "), "{line}");
        for name in GH_TOKEN_VARS {
            // Unset-safe, so an rc file's `set -u` does not abort the line.
            assert!(line.contains(&format!("\"{name}=${{{name}-}}\"")), "{line}");
        }
        assert!(line.ends_with(" MAYA-GH-END"), "{line}");
    }
}
