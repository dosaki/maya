//! Token variables. An app opened from the Dock gets launchd's bare
//! environment, not the user's `~/.zshrc`, so a variable missing from this
//! process is asked of the interactive login shell, once per name. Only a
//! valid name (`config::valid_env_name`) reaches the shell line.

use crate::config::valid_env_name;
#[cfg(unix)]
use std::collections::HashMap;
use std::process::Stdio;
#[cfg(unix)]
use std::sync::Mutex;
use std::time::{Duration, Instant};

const BEGIN: &str = "MAYA-ENV-BEGIN";
const END: &str = "MAYA-ENV-END";

/// The shell line that prints `NAME=value` between two marker lines, so
/// whatever the rc files print around it is told apart. Each item starts a
/// fresh line; the expansion is unset-safe for an rc file's `set -u`.
pub fn probe_line(name: &str) -> String {
    format!("printf '\\n%s\\n' {BEGIN} \"{name}=${{{name}-}}\" {END}")
}

/// The non-empty value of `name` between the probe's markers in `out`.
pub fn parse_probe(out: &str, name: &str) -> Option<String> {
    out.lines()
        .skip_while(|l| l.trim() != BEGIN)
        .skip(1)
        .take_while(|l| l.trim() != END)
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
        .filter(|v| !v.is_empty())
}

/// What the login shell said for each name asked, found or not.
#[cfg(unix)]
static CACHE: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);

/// Forgets what the login shell said, so a variable set since is found.
pub fn forget() {
    #[cfg(unix)]
    {
        *CACHE.lock().unwrap() = None;
    }
}

#[cfg(unix)]
fn from_login_shell(name: &str) -> Option<String> {
    if let Some(v) = CACHE.lock().unwrap().get_or_insert_with(HashMap::new).get(name) {
        return v.clone();
    }
    let out = crate::command(crate::launch::LOGIN_SHELL).args(["-lic", &probe_line(name)]).stdin(Stdio::null()).stderr(Stdio::null()).output();
    let value = out.ok().and_then(|o| parse_probe(&String::from_utf8_lossy(&o.stdout), name));
    CACHE.lock().unwrap().get_or_insert_with(HashMap::new).insert(name.to_string(), value.clone());
    value
}

#[cfg(not(unix))]
fn from_login_shell(_name: &str) -> Option<String> {
    // A windowed app on Windows already gets the user's environment.
    None
}

/// The variable's value: this process's when set and non-empty, else the
/// login shell's. None for a name that is not a plain variable name.
pub fn read(name: &str) -> Option<String> {
    if !valid_env_name(name) {
        return None;
    }
    std::env::var(name).ok().filter(|v| !v.is_empty()).or_else(|| from_login_shell(name))
}

/// What `gh auth token` prints, when `gh` is installed and logged in.
/// Run once, at the first start after the upgrade; never logged.
pub fn import_gh_token() -> Option<String> {
    let gh = crate::launch::agent_binary("gh")?;
    let mut child = crate::command(gh).args(["auth", "token"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if started.elapsed() > Duration::from_secs(5) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    Some(out.trim().to_string()).filter(|t| !t.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_probe_prints_one_variable_between_markers_unset_safe() {
        assert_eq!(probe_line("GH_TOKEN"), "printf '\\n%s\\n' MAYA-ENV-BEGIN \"GH_TOKEN=${GH_TOKEN-}\" MAYA-ENV-END");
    }

    #[test]
    fn the_probe_output_yields_the_value_between_the_markers_ignoring_rc_noise() {
        let out = "Restored session\nMAYA-ENV-BEGIN\nGH_TOKEN=ghp_abc=def\nMAYA-ENV-END\nSaving session...\n";
        assert_eq!(parse_probe(out, "GH_TOKEN").as_deref(), Some("ghp_abc=def"));
        assert_eq!(parse_probe("MAYA-ENV-BEGIN\nGH_TOKEN=\nMAYA-ENV-END\n", "GH_TOKEN"), None, "empty is none");
        assert_eq!(parse_probe("GH_TOKEN=ghp_x\n", "GH_TOKEN"), None, "no markers, nothing");
        assert_eq!(parse_probe("MAYA-ENV-BEGIN\nOTHER=x\nMAYA-ENV-END\n", "GH_TOKEN"), None);
    }

    #[test]
    fn an_invalid_name_is_never_read() {
        assert_eq!(read("GH_TOKEN; touch /tmp/maya-pwned"), None);
        assert_eq!(read("$(id)"), None);
        assert_eq!(read(""), None);
    }

    #[cfg(unix)]
    #[test]
    fn forget_drops_what_the_login_shell_said() {
        let name = "MAYA_ENV_TEST_FORGET";
        CACHE.lock().unwrap().get_or_insert_with(HashMap::new).insert(name.into(), None);
        forget();
        assert!(!CACHE.lock().unwrap().as_ref().is_some_and(|c| c.contains_key(name)));
    }

    #[test]
    fn the_processes_own_value_wins() {
        // A name nothing else uses, so the test owns it.
        let name = "MAYA_ENV_TEST_OWN_VALUE";
        std::env::set_var(name, "from-process");
        assert_eq!(read(name).as_deref(), Some("from-process"));
        std::env::remove_var(name);
    }
}
