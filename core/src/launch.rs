use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const CLASSIFIER_TIMEOUT: Duration = Duration::from_secs(90);

/// Model aliases `claude --model` accepts. Fixed lists keep the launch command
/// free of anything the user typed.
pub const MODELS: &[&str] = &["fable", "opus", "sonnet", "haiku"];
pub const EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
pub const MODES: &[&str] = &["manual", "acceptEdits", "plan", "auto", "dontAsk", "bypassPermissions"];

/// Per-session choices for `claude`. None or "" means "use the defaults".
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct LaunchOptions {
    pub model: Option<String>,
    pub effort: Option<String>,
    pub mode: Option<String>,
}

fn chosen(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// `value` if it is one of `allowed`; a message naming `what` otherwise.
pub fn check_choice(what: &str, value: &str, allowed: &[&str]) -> Result<(), String> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(format!("Unknown {what}: {value}"))
    }
}

impl LaunchOptions {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(m) = chosen(&self.model) {
            check_choice("model", m, MODELS)?;
        }
        if let Some(e) = chosen(&self.effort) {
            check_choice("effort", e, EFFORTS)?;
        }
        if let Some(m) = chosen(&self.mode) {
            check_choice("mode", m, MODES)?;
        }
        Ok(())
    }

    /// The `claude` flags for the chosen options, each preceded by a space.
    /// Only valid values are rendered; call `validate` first.
    pub fn flags(&self) -> String {
        let mut out = String::new();
        if let Some(m) = chosen(&self.model) {
            out.push_str(&format!(" --model {m}"));
        }
        if let Some(e) = chosen(&self.effort) {
            out.push_str(&format!(" --effort {e}"));
        }
        if let Some(m) = chosen(&self.mode) {
            out.push_str(&format!(" --permission-mode {m}"));
        }
        out
    }
}

/// Names of the visible subfolders of `root`, sorted. Missing root gives none.
pub fn list_project_dirs(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else { return vec![] };
    let mut out: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
        .filter(|n| !n.starts_with('.'))
        .collect();
    out.sort();
    out
}

pub fn classifier_prompt(user_prompt: &str, dirs: &[String]) -> String {
    format!(
        "The user submitted this prompt without choosing a directory:\n==========\n{}\n==========\nThese are the folders in the projects directory: {}\nReply with exactly one folder name from that list that this prompt most likely belongs to, or NONE if none clearly fits. Reply with the name only, nothing else.",
        user_prompt.trim(),
        dirs.join(", ")
    )
}

/// The folder the classifier named, if its reply is exactly one known name
/// (ignoring case, surrounding quotes/backticks and a trailing period).
pub fn pick_dir(reply: &str, dirs: &[String]) -> Option<String> {
    let cleaned = reply.trim().trim_matches(|c| c == '"' || c == '\'' || c == '`').trim_end_matches('.').trim();
    if cleaned.is_empty() || cleaned.eq_ignore_ascii_case("none") {
        return None;
    }
    dirs.iter().find(|d| d.eq_ignore_ascii_case(cleaned)).cloned()
}

/// Environment for spawned `claude` processes: Maya's own, minus the variables
/// a nested Claude session exports (they break auth and mark the run as a child).
pub fn clean_env(vars: impl Iterator<Item = (String, String)>) -> Vec<(String, String)> {
    vars.filter(|(k, _)| k != "ANTHROPIC_API_KEY" && k != "CLAUDECODE" && !k.starts_with("CLAUDE_CODE_")).collect()
}

/// The login shell `claude_binary` asks last: zsh is macOS's default, and a
/// Linux box or container may not have it, so `sh` there.
#[cfg(target_os = "macos")]
pub const LOGIN_SHELL: &str = "zsh";
#[cfg(not(target_os = "macos"))]
pub const LOGIN_SHELL: &str = "sh";

/// The first executable file named `name` in one of `path`'s folders.
pub fn find_on_path(path: &str, name: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::split_paths(path)
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(name))
        .find(|p| std::fs::metadata(p).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false))
}

/// The `claude` binary: from this process's PATH, then the usual install
/// locations, then the login shell's PATH.
pub fn claude_binary() -> Option<PathBuf> {
    if let Some(p) = std::env::var("PATH").ok().and_then(|path| find_on_path(&path, "claude")) {
        return Some(p);
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let candidates = [home.join(".local/bin/claude"), PathBuf::from("/opt/homebrew/bin/claude"), PathBuf::from("/usr/local/bin/claude")];
    if let Some(p) = candidates.iter().find(|p| p.is_file()) {
        return Some(p.clone());
    }
    let out = Command::new(LOGIN_SHELL).args(["-lc", "command -v claude"]).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() && !s.is_empty() {
        Some(PathBuf::from(s))
    } else {
        None
    }
}

/// Runs the headless picker; None on NONE, no match, timeout or any error.
pub fn classify(binary: &Path, root: &Path, user_prompt: &str, dirs: &[String], timeout: Duration) -> Option<String> {
    let mut child = Command::new(binary)
        .args(["-p", "--strict-mcp-config", "--disable-slash-commands", "--model", "haiku", "--output-format", "text", "--no-session-persistence", "--max-turns", "1"])
        .arg(classifier_prompt(user_prompt, dirs))
        .current_dir(root)
        .env_clear()
        .envs(clean_env(std::env::vars()))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    pick_dir(&String::from_utf8_lossy(&out.stdout), dirs)
}

/// Prompt files older than this are removed whenever a new one is written.
/// The launched command deletes its own file; this catches the ones left
/// behind when a launch failed part-way.
pub const PROMPT_FILE_MAX_AGE: Duration = Duration::from_secs(24 * 3600);

fn prune_prompt_files(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let now = std::time::SystemTime::now();
    for e in entries.flatten() {
        let stale = e
            .metadata()
            .and_then(|m| m.modified())
            .map(|m| now.duration_since(m).map(|age| age > PROMPT_FILE_MAX_AGE).unwrap_or(false))
            .unwrap_or(false);
        if stale {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Saves the first prompt to `<eye_dir>/prompts/<epoch-millis>.txt`.
pub fn write_prompt_file(eye_dir: &Path, prompt: &str) -> Result<PathBuf, String> {
    let dir = eye_dir.join("prompts");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    prune_prompt_files(&dir);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let path = dir.join(format!("{stamp}.txt"));
    std::fs::write(&path, prompt).map_err(|e| format!("cannot write the prompt file: {e}"))?;
    Ok(path)
}

/// Single-quotes `s` for a POSIX shell.
pub fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn applescript_string(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Opens a new Terminal window running `cmd`. Terminal is brought forward
/// afterwards via AppKit so only the new window comes up.
pub fn applescript_run(cmd: &str) -> String {
    format!(
        "tell application \"Terminal\"\n  do script \"{}\"\nend tell",
        applescript_string(cmd)
    )
}

/// The shell line that starts a session: it reads the prompt file into a
/// variable, deletes the file, and passes the prompt after `--` so a prompt
/// starting with `-` is not an option.
pub fn session_command(target: &Path, prompt_file: &Path, opts: &LaunchOptions) -> String {
    let file = shell_single_quote(&prompt_file.to_string_lossy());
    format!(
        "cd {} && p=\"$(cat {file})\" && rm -f {file} && claude{} -- \"$p\"",
        shell_single_quote(&target.to_string_lossy()),
        opts.flags()
    )
}

pub fn applescript_launch(target: &Path, prompt_file: &Path, opts: &LaunchOptions) -> String {
    applescript_run(&session_command(target, prompt_file, opts))
}

/// Where the session starts and why: the user's choice, the classifier's pick,
/// or the projects directory itself.
pub fn resolve_target(root: &Path, dirs: &[String], dir: Option<&str>, picked: Option<&str>) -> Result<(PathBuf, &'static str), String> {
    match (dir, picked) {
        (Some(d), _) => {
            if dirs.iter().any(|x| x == d) {
                Ok((root.join(d), "chosen"))
            } else {
                Err("That folder is not in the projects directory.".into())
            }
        }
        (None, Some(p)) => Ok((root.join(p), "classifier")),
        (None, None) => Ok((root.to_path_buf(), "fallback")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn dirs() -> Vec<String> {
        vec!["a".into(), "b".into(), "sonarqube".into()]
    }

    fn fake_binary(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join("claude");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[test]
    fn find_on_path_takes_the_first_executable_file_in_the_path() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        std::fs::write(a.path().join("claude"), "not executable").unwrap();
        let expected = fake_binary(b.path(), "exit 0");
        let path = std::env::join_paths([a.path(), b.path()]).unwrap().into_string().unwrap();
        assert_eq!(find_on_path(&path, "claude"), Some(expected));
        assert_eq!(find_on_path(&a.path().to_string_lossy(), "claude"), None, "a file that is not executable");
        assert_eq!(find_on_path("", "claude"), None);
        std::fs::create_dir(a.path().join("dir")).unwrap();
        assert_eq!(find_on_path(&a.path().to_string_lossy(), "dir"), None, "a folder");
    }

    #[test]
    fn the_login_shell_fallback_is_zsh_on_macos_and_sh_elsewhere() {
        assert_eq!(LOGIN_SHELL, if cfg!(target_os = "macos") { "zsh" } else { "sh" });
    }

    #[test]
    fn lists_visible_subfolders_sorted() {
        let t = tempfile::tempdir().unwrap();
        for d in ["b", "a", ".hidden"] {
            std::fs::create_dir(t.path().join(d)).unwrap();
        }
        std::fs::write(t.path().join("file.txt"), "x").unwrap();
        assert_eq!(list_project_dirs(t.path()), vec!["a".to_string(), "b".to_string()]);
        assert!(list_project_dirs(Path::new("/nonexistent")).is_empty());
    }

    #[test]
    fn prompt_template_wraps_the_prompt_and_lists_folders() {
        let p = classifier_prompt("fix the CI", &dirs());
        assert!(p.contains("==========\nfix the CI\n=========="));
        assert!(p.contains("a, b, sonarqube"));
        assert!(p.contains("NONE"));
    }

    #[test]
    fn pick_dir_matches_loosely_but_only_real_names() {
        let d = dirs();
        assert_eq!(pick_dir("sonarqube", &d).as_deref(), Some("sonarqube"));
        assert_eq!(pick_dir("  Sonarqube.\n", &d).as_deref(), Some("sonarqube"));
        assert_eq!(pick_dir("`sonarqube`", &d).as_deref(), Some("sonarqube"));
        assert_eq!(pick_dir("\"b\"", &d).as_deref(), Some("b"));
        assert_eq!(pick_dir("NONE", &d), None);
        assert_eq!(pick_dir("", &d), None);
        assert_eq!(pick_dir("I think sonarqube fits best", &d), None);
    }

    #[test]
    fn clean_env_drops_nested_session_variables() {
        let vars = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("ANTHROPIC_API_KEY".to_string(), "x".to_string()),
            ("CLAUDECODE".to_string(), "1".to_string()),
            ("CLAUDE_CODE_SESSION_ID".to_string(), "s".to_string()),
            ("HOME".to_string(), "/h".to_string()),
        ];
        let kept: Vec<String> = clean_env(vars.into_iter()).into_iter().map(|(k, _)| k).collect();
        assert_eq!(kept, vec!["PATH", "HOME"]);
    }

    #[test]
    fn launch_options_accept_only_known_values() {
        assert!(LaunchOptions::default().validate().is_ok());
        let ok = LaunchOptions { model: Some("opus".into()), effort: Some("xhigh".into()), mode: Some("acceptEdits".into()) };
        assert!(ok.validate().is_ok());
        let bad_model = LaunchOptions { model: Some("gpt; rm -rf /".into()), ..Default::default() };
        assert!(bad_model.validate().unwrap_err().contains("model"));
        let bad_effort = LaunchOptions { effort: Some("turbo".into()), ..Default::default() };
        assert!(bad_effort.validate().unwrap_err().contains("effort"));
        let bad_mode = LaunchOptions { mode: Some("yolo".into()), ..Default::default() };
        assert!(bad_mode.validate().unwrap_err().contains("mode"));
        // Empty strings mean "default": no flag.
        let empty = LaunchOptions { model: Some("".into()), effort: Some("".into()), mode: Some("".into()) };
        assert!(empty.validate().is_ok());
        assert_eq!(empty.flags(), "");
    }

    #[test]
    fn launch_options_render_as_claude_flags() {
        assert_eq!(LaunchOptions::default().flags(), "");
        let all = LaunchOptions { model: Some("sonnet".into()), effort: Some("low".into()), mode: Some("plan".into()) };
        assert_eq!(all.flags(), " --model sonnet --effort low --permission-mode plan");
        let one = LaunchOptions { effort: Some("max".into()), ..Default::default() };
        assert_eq!(one.flags(), " --effort max");
    }

    #[test]
    fn any_shell_command_can_be_opened_in_a_terminal_window() {
        let s = applescript_run("cd '/r' && echo \"hi\"");
        // No `activate`: the new window is raised via AppKit afterwards, on its own.
        assert_eq!(s, "tell application \"Terminal\"\n  do script \"cd '/r' && echo \\\"hi\\\"\"\nend tell");
    }

    #[test]
    fn launch_command_carries_the_flags_before_the_prompt() {
        let opts = LaunchOptions { model: Some("haiku".into()), mode: Some("bypassPermissions".into()), ..Default::default() };
        let s = applescript_launch(Path::new("/r/a"), Path::new("/p/1.txt"), &opts);
        assert!(s.contains("claude --model haiku --permission-mode bypassPermissions -- \\\"$p\\\""), "{s}");
    }

    #[test]
    fn shell_quoting_and_applescript_escaping() {
        assert_eq!(shell_single_quote("it's"), "'it'\\''s'");
        let s = applescript_launch(Path::new("/Users/x/dev/it's-here"), Path::new("/Users/x/.claude/maya/prompts/1.txt"), &LaunchOptions::default());
        assert!(s.contains("tell application \"Terminal\""));
        // The prompt file is consumed and deleted, and `--` protects prompts that start with `-`.
        assert!(s.contains("do script \"cd '/Users/x/dev/it'\\\\''s-here' && p=\\\"$(cat '/Users/x/.claude/maya/prompts/1.txt')\\\" && rm -f '/Users/x/.claude/maya/prompts/1.txt' && claude -- \\\"$p\\\"\""), "{s}");
        assert!(!s.contains("activate"), "activation happens via AppKit, not AppleScript");
    }

    #[test]
    fn classify_uses_the_reply_and_ignores_none() {
        let t = tempfile::tempdir().unwrap();
        let bin = fake_binary(t.path(), "echo b");
        assert_eq!(classify(&bin, t.path(), "p", &dirs(), Duration::from_secs(5)).as_deref(), Some("b"));
        let bin = fake_binary(t.path(), "echo NONE");
        assert_eq!(classify(&bin, t.path(), "p", &dirs(), Duration::from_secs(5)), None);
    }

    #[test]
    fn classify_times_out_and_falls_back() {
        let t = tempfile::tempdir().unwrap();
        let bin = fake_binary(t.path(), "sleep 5; echo b");
        let start = Instant::now();
        assert_eq!(classify(&bin, t.path(), "p", &dirs(), Duration::from_secs(1)), None);
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn writes_the_prompt_file_under_prompts_and_prunes_old_ones() {
        let t = tempfile::tempdir().unwrap();
        let prompts = t.path().join("prompts");
        std::fs::create_dir_all(&prompts).unwrap();
        let old = prompts.join("1.txt");
        std::fs::write(&old, "old").unwrap();
        let stale = std::time::SystemTime::now() - Duration::from_secs(2 * 24 * 3600);
        std::fs::File::open(&old).unwrap().set_modified(stale).unwrap();
        let fresh = prompts.join("2.txt");
        std::fs::write(&fresh, "fresh").unwrap();

        let p = write_prompt_file(t.path(), "hello\nworld").unwrap();
        assert!(p.starts_with(&prompts));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "hello\nworld");
        assert!(!old.exists(), "files older than a day are pruned");
        assert!(fresh.exists());
    }

    #[test]
    fn resolve_target_honours_choice_classifier_and_fallback() {
        let root = Path::new("/r");
        let d = dirs();
        assert_eq!(resolve_target(root, &d, Some("b"), None).unwrap(), (PathBuf::from("/r/b"), "chosen"));
        assert!(resolve_target(root, &d, Some("../.."), None).unwrap_err().contains("not in the projects directory"));
        assert_eq!(resolve_target(root, &d, None, Some("sonarqube")).unwrap(), (PathBuf::from("/r/sonarqube"), "classifier"));
        assert_eq!(resolve_target(root, &d, None, None).unwrap(), (PathBuf::from("/r"), "fallback"));
    }
}
