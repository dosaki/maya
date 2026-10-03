use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

pub const CLASSIFIER_TIMEOUT: Duration = Duration::from_secs(90);

use crate::model::Harness;

/// Model aliases `claude --model` accepts. Fixed lists keep the launch command
/// free of anything the user typed.
pub const MODELS: &[&str] = &["fable", "opus", "sonnet", "haiku"];
pub const EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
pub const MODES: &[&str] = &["manual", "acceptEdits", "plan", "auto", "dontAsk", "bypassPermissions"];

/// The command each agent runs as.
pub fn binary_name(agent: Harness) -> &'static str {
    match agent {
        Harness::ClaudeCode => "claude",
        Harness::Codex => "codex",
        Harness::Antigravity => "agy",
        Harness::Grok => "grok",
    }
}

/// The efforts an agent takes, from its `--help`. Each Codex model takes a
/// subset of these; Grok lists none.
pub fn efforts(agent: Harness) -> &'static [&'static str] {
    match agent {
        Harness::ClaudeCode => EFFORTS,
        Harness::Codex => &["low", "medium", "high", "xhigh", "max", "ultra"],
        Harness::Antigravity => &["low", "medium", "high", "max"],
        Harness::Grok => &[],
    }
}

/// The modes an agent takes: Claude's and Grok's permission modes, Codex's
/// sandbox policies, Antigravity's execution modes.
pub fn modes(agent: Harness) -> &'static [&'static str] {
    match agent {
        Harness::ClaudeCode => MODES,
        Harness::Codex => &["read-only", "workspace-write", "danger-full-access"],
        Harness::Antigravity => &["accept-edits", "plan"],
        Harness::Grok => &["default", "acceptEdits", "auto", "dontAsk", "bypassPermissions", "plan"],
    }
}

/// The agent's name as the UI shows it.
pub fn label(agent: Harness) -> &'static str {
    match agent {
        Harness::ClaudeCode => "Claude Code",
        Harness::Codex => "Codex",
        Harness::Antigravity => "Antigravity",
        Harness::Grok => "Grok Build",
    }
}

/// What Maya can type into a running session of this agent: from each
/// agent's docs and a probe of its TUI (the design's capability table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// `/compact` is a command.
    pub compact: bool,
    /// `/model <id>` switches the model; false where `/model` only opens a picker.
    pub model_switch: bool,
    /// `/effort <level>` switches the effort.
    pub effort_switch: bool,
    /// The keys that cycle the permission or execution mode; None where no key does.
    pub mode_cycle: Option<&'static str>,
    /// `/` lines are commands to the agent.
    pub slash_lines: bool,
    /// `!` lines run in the shell.
    pub shell_lines: bool,
}

pub fn capabilities(agent: Harness) -> Capabilities {
    let shift_tab = Some(crate::answer::SHIFT_TAB);
    match agent {
        Harness::ClaudeCode => Capabilities { compact: true, model_switch: true, effort_switch: true, mode_cycle: shift_tab, slash_lines: true, shell_lines: true },
        // `/model` and `/permissions` open pickers in Codex; a typed value does nothing.
        Harness::Codex => Capabilities { compact: true, model_switch: false, effort_switch: false, mode_cycle: None, slash_lines: true, shell_lines: false },
        Harness::Antigravity => Capabilities { compact: true, model_switch: false, effort_switch: false, mode_cycle: shift_tab, slash_lines: true, shell_lines: false },
        // Grok's `--help` lists no effort values, so none can be checked before typing.
        Harness::Grok => Capabilities { compact: true, model_switch: true, effort_switch: false, mode_cycle: shift_tab, slash_lines: true, shell_lines: false },
    }
}

/// A model id is put in a shell line only when it is plain: letters,
/// digits, `.`, `_` and `-`.
pub fn plain_model_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 100 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
}

/// Per-session choices for a new session. None or "" means "use the defaults".
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct LaunchOptions {
    pub agent: Harness,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub mode: Option<String>,
    /// The session's name; only Claude Code takes it as a flag.
    pub name: Option<String>,
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
    pub fn chosen_name(&self) -> Option<&str> {
        chosen(&self.name)
    }

    /// Checks everything that can be checked without the agent's model list.
    pub fn validate_shape(&self) -> Result<(), String> {
        if let Some(m) = chosen(&self.model) {
            if !plain_model_id(m) {
                return Err(format!("Unknown model: {m}"));
            }
        }
        if let Some(e) = chosen(&self.effort) {
            check_choice("effort", e, efforts(self.agent))?;
        }
        if let Some(m) = chosen(&self.mode) {
            check_choice("mode", m, modes(self.agent))?;
        }
        if let Some(n) = self.chosen_name() {
            crate::answer::rename_command(n)?;
        }
        Ok(())
    }

    /// `validate_shape`, and a chosen model must be in `models`, the agent's own list.
    pub fn validate(&self, models: &[String]) -> Result<(), String> {
        self.validate_shape()?;
        match chosen(&self.model) {
            Some(m) if !models.iter().any(|x| x == m) => Err(format!("Unknown model: {m}")),
            _ => Ok(()),
        }
    }

    /// The agent's flags for the chosen options, each preceded by a space.
    /// Only valid values are rendered; call `validate` first.
    pub fn flags(&self) -> String {
        let (model, effort, mode) = (chosen(&self.model), chosen(&self.effort), chosen(&self.mode));
        let (m_flag, e_flag, mode_flag) = match self.agent {
            Harness::ClaudeCode => ("--model", Some("--effort"), "--permission-mode"),
            Harness::Codex => ("-m", Some("-c model_reasoning_effort="), "-s"),
            Harness::Antigravity => ("--model", Some("--effort"), "--mode"),
            Harness::Grok => ("-m", None, "--permission-mode"),
        };
        let mut out = String::new();
        if let Some(m) = model {
            out.push_str(&format!(" {m_flag} {m}"));
        }
        match (e_flag, effort) {
            (Some(f), Some(e)) if f.ends_with('=') => out.push_str(&format!(" {f}{e}")),
            (Some(f), Some(e)) => out.push_str(&format!(" {f} {e}")),
            _ => {}
        }
        if let Some(m) = mode {
            out.push_str(&format!(" {mode_flag} {m}"));
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

/// The arguments for one non-interactive turn of `agent`: `user`, answered
/// under `system`, with no tools, nothing persisted, one turn, and the
/// agent's JSON envelope on stdout (see `final_text`). Codex and
/// Antigravity have no system-prompt flag: the system text leads the
/// prompt for them. Antigravity reads the prompt only attached to `-p`.
pub fn oneshot_args(agent: Harness, model: Option<&str>, system: Option<&str>, user: &str) -> Vec<String> {
    let s = |v: &str| v.to_string();
    let folded = match system {
        Some(sys) => format!("{sys}\n\n{user}"),
        None => user.to_string(),
    };
    match agent {
        Harness::ClaudeCode => {
            let mut a = vec![s("-p"), s("--output-format"), s("json"), s("--strict-mcp-config"), s("--disable-slash-commands"), s("--no-session-persistence"), s("--max-turns"), s("1"), s("--tools"), s(""), s("--setting-sources"), s("user")];
            if let Some(sys) = system {
                a.extend([s("--system-prompt"), s(sys)]);
            }
            if let Some(m) = model {
                a.extend([s("--model"), s(m)]);
            }
            a.push(s(user));
            a
        }
        Harness::Codex => {
            let mut a = vec![s("exec"), s("--json"), s("--ephemeral"), s("--skip-git-repo-check"), s("-s"), s("read-only"), s("--color"), s("never")];
            if let Some(m) = model {
                a.extend([s("-m"), s(m)]);
            }
            a.extend([s("--"), folded]);
            a
        }
        Harness::Antigravity => {
            let mut a = vec![s("--output-format"), s("json"), s("--sandbox"), s("--disable-slash-commands")];
            if let Some(m) = model {
                a.extend([s("--model"), s(m)]);
            }
            a.push(format!("-p={folded}"));
            a
        }
        Harness::Grok => {
            let mut a = vec![s("-p"), s(user), s("--output-format"), s("json"), s("--tools"), s(""), s("--max-turns"), s("1"), s("--permission-mode"), s("plan")];
            if let Some(sys) = system {
                a.extend([s("--system-prompt-override"), s(sys)]);
            }
            if let Some(m) = model {
                a.extend([s("-m"), s(m)]);
            }
            a
        }
    }
}

/// The agent's final answer inside its one-shot output: Claude's `result`,
/// the text of Codex's last `agent_message` item, Antigravity's `response`,
/// Grok's `text`. Err names the failure the envelope reports.
pub fn final_text(agent: Harness, stdout: &str) -> Result<String, String> {
    use serde_json::Value;
    let clip = |t: &str| t.chars().take(120).collect::<String>();
    let json = |who: &str| serde_json::from_str::<Value>(stdout.trim()).map_err(|e| format!("{who} output was not JSON: {e}"));
    match agent {
        Harness::ClaudeCode => {
            let v = json("claude")?;
            let result = v["result"].as_str().unwrap_or("");
            if v["is_error"].as_bool() == Some(true) {
                return Err(format!("claude failed: {}", clip(result)));
            }
            Ok(result.to_string())
        }
        Harness::Codex => {
            let mut last = None;
            for line in stdout.lines() {
                let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
                match v["type"].as_str() {
                    Some("item.completed") if v["item"]["type"].as_str() == Some("agent_message") => last = v["item"]["text"].as_str().map(str::to_string),
                    Some("error") => return Err(format!("codex failed: {}", clip(v["message"].as_str().unwrap_or("")))),
                    Some("turn.failed") => return Err(format!("codex failed: {}", clip(&v["error"].to_string()))),
                    _ => {}
                }
            }
            last.ok_or_else(|| format!("no agent message in: {}", clip(stdout)))
        }
        Harness::Antigravity => {
            let v = json("agy")?;
            if v["status"].as_str() != Some("SUCCESS") {
                return Err(format!("agy failed: {}", clip(&v.to_string())));
            }
            Ok(v["response"].as_str().unwrap_or("").to_string())
        }
        Harness::Grok => {
            let v = json("grok")?;
            v["text"].as_str().map(str::to_string).ok_or_else(|| format!("no text in: {}", clip(stdout)))
        }
    }
}

/// The agent's binary, or the message that names the command to install.
pub fn find_binary(agent: Harness) -> Result<PathBuf, String> {
    find_binary_named(binary_name(agent))
}

fn find_binary_named(name: &str) -> Result<PathBuf, String> {
    agent_binary(name).ok_or_else(|| format!("Could not find the {name} command."))
}

/// What the AppImage's runtime (`APPDIR`, `APPIMAGE`, `ARGV0`, `OWD`) and
/// linuxdeploy's GTK hook export into Maya's environment, removed outright
/// from the processes that outlive Maya. Every other variable that points
/// into the mount (`PATH`, `LD_LIBRARY_PATH`, `XDG_DATA_DIRS`, `PYTHONHOME`,
/// `GIO_MODULE_DIR` and more, set by the AppImage's AppRun) loses the
/// entries inside it. Inherited, they break a tmux server, terminal or
/// browser once Maya quits and the mount goes away.
#[cfg(any(test, target_os = "linux"))]
pub const APPIMAGE_VARS: &[&str] = &[
    "GDK_BACKEND",
    "GTK_THEME",
    "GTK_PATH",
    "GTK_EXE_PREFIX",
    "GTK_DATA_PREFIX",
    "GIO_EXTRA_MODULES",
    "GSETTINGS_SCHEMA_DIR",
    "GDK_PIXBUF_MODULE_FILE",
    "GI_TYPELIB_PATH",
    "APPDIR",
    "APPIMAGE",
    "ARGV0",
    "OWD",
];

/// A `:`-separated list (`XDG_DATA_DIRS`, `PATH`, …) without the entries
/// inside the AppImage's mount `appdir`, which the AppImage puts first.
pub fn without_appdir_prefix(xdg: &str, appdir: &str) -> String {
    let appdir = appdir.trim_end_matches('/');
    if appdir.is_empty() {
        return xdg.to_string();
    }
    xdg.split(':').filter(|e| *e != appdir && !e.starts_with(&format!("{appdir}/"))).collect::<Vec<_>>().join(":")
}

/// Clears the AppImage's environment from `cmd` when Maya runs as one
/// (`APPIMAGE` set in `env`, Maya's own environment).
#[cfg(any(test, target_os = "linux"))]
fn scrub_with(cmd: &mut std::process::Command, env: Vec<(String, String)>) {
    let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    if get("APPIMAGE").is_none() {
        return;
    }
    for var in APPIMAGE_VARS {
        cmd.env_remove(var);
    }
    let appdir = get("APPDIR").unwrap_or_default().trim_end_matches('/');
    if appdir.is_empty() {
        return;
    }
    for (k, v) in &env {
        if APPIMAGE_VARS.contains(&k.as_str()) || !v.contains(appdir) {
            continue;
        }
        let kept = without_appdir_prefix(v, appdir);
        if kept.split(':').all(str::is_empty) {
            cmd.env_remove(k);
        } else if kept != *v {
            cmd.env(k, kept);
        }
    }
}

/// Starts `cmd` without the AppImage's environment, for the processes that
/// outlive Maya (the tmux server, terminal windows, the browser). A no-op
/// outside an AppImage and on other platforms.
#[cfg(target_os = "linux")]
pub fn scrub_appimage_env(cmd: &mut std::process::Command) {
    scrub_with(cmd, std::env::vars_os().filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?))).collect());
}

#[cfg(not(target_os = "linux"))]
pub fn scrub_appimage_env(_cmd: &mut std::process::Command) {}

/// The login shell `agent_binary` asks last: zsh is macOS's default, and a
/// Linux box or container may not have it, so `sh` there.
#[cfg(target_os = "macos")]
pub const LOGIN_SHELL: &str = "zsh";
#[cfg(not(target_os = "macos"))]
pub const LOGIN_SHELL: &str = "sh";

/// The first file named `name` plus one of `PATHEXT`'s extensions (or
/// `name` itself when it has one) in one of `path`'s folders.
#[cfg(windows)]
pub fn find_on_path(path: &str, name: &str) -> Option<PathBuf> {
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    let candidates: Vec<String> = if Path::new(name).extension().is_some() {
        vec![name.to_string()]
    } else {
        exts.split(';').filter(|e| !e.is_empty()).map(|e| format!("{name}{}", e.to_ascii_lowercase())).collect()
    };
    std::env::split_paths(path)
        .filter(|dir| !dir.as_os_str().is_empty())
        .flat_map(|dir| candidates.iter().map(move |c| dir.join(c)))
        .find(|p| std::fs::metadata(p).map(|m| m.is_file()).unwrap_or(false))
}

/// The first executable file named `name` in one of `path`'s folders.
#[cfg(unix)]
pub fn find_on_path(path: &str, name: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::split_paths(path)
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(name))
        .find(|p| std::fs::metadata(p).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false))
}

/// Git for Windows' `bash.exe`, which Claude Code itself needs on Windows
/// and which runs Maya's shell lines there: `CLAUDE_CODE_GIT_BASH_PATH`,
/// the usual install folders, then beside the `git` on PATH.
#[cfg(windows)]
pub fn git_bash() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CLAUDE_CODE_GIT_BASH_PATH").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    let program_files = [std::env::var_os("ProgramFiles"), std::env::var_os("ProgramFiles(x86)"), std::env::var_os("LOCALAPPDATA").map(|l| Path::new(&l).join("Programs").into_os_string())];
    if let Some(p) = program_files.into_iter().flatten().map(|d| Path::new(&d).join("Git").join("bin").join("bash.exe")).find(|p| p.is_file()) {
        return Some(p);
    }
    // git.exe lives in <Git>\cmd; bash.exe in <Git>in.
    let git = std::env::var("PATH").ok().and_then(|path| find_on_path(&path, "git.exe"))?;
    Some(git.parent()?.parent()?.join("bin").join("bash.exe")).filter(|p| p.is_file())
}

/// The native `<name>.exe`: from this process's PATH, then where the
/// installer puts it. npm's `.cmd` wrappers are passed over: Windows cannot
/// hand a batch file the multi-line prompts Maya sends.
#[cfg(windows)]
pub fn agent_binary(name: &str) -> Option<PathBuf> {
    let exe = format!("{name}.exe");
    if let Some(p) = std::env::var("PATH").ok().and_then(|path| find_on_path(&path, &exe)) {
        return Some(p);
    }
    let home = dirs::home_dir()?;
    Some(home.join(".local").join("bin").join(&exe)).filter(|p| p.is_file())
}

/// The `name` binary: from this process's PATH, then the usual install
/// locations, then the login shell's PATH.
#[cfg(unix)]
pub fn agent_binary(name: &str) -> Option<PathBuf> {
    if let Some(p) = std::env::var("PATH").ok().and_then(|path| find_on_path(&path, name)) {
        return Some(p);
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let candidates = [home.join(".local/bin").join(name), PathBuf::from("/opt/homebrew/bin").join(name), PathBuf::from("/usr/local/bin").join(name)];
    if let Some(p) = candidates.iter().find(|p| p.is_file()) {
        return Some(p.clone());
    }
    let out = crate::command(LOGIN_SHELL).args(["-lc", &format!("command -v {name}")]).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() && !s.is_empty() {
        Some(PathBuf::from(s))
    } else {
        None
    }
}

/// Runs the headless folder picker on `agent`; None on NONE, no match,
/// timeout or any error.
pub fn classify(agent: Harness, binary: &Path, model: Option<&str>, root: &Path, user_prompt: &str, dirs: &[String], timeout: Duration) -> Option<String> {
    let mut child = crate::command(binary)
        .args(oneshot_args(agent, model, None, &classifier_prompt(user_prompt, dirs)))
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
    let text = final_text(agent, &String::from_utf8_lossy(&out.stdout)).ok()?;
    pick_dir(&text, dirs)
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
/// variable, deletes the file, and runs the agent with the prompt as one
/// argument that is never read as an option: after `--`, or for
/// Antigravity, whose Go flag parser has no `--`, as `--prompt-interactive="$p"`.
/// `session_id` is Grok's `--session-id`, so Maya knows the new session's id.
pub fn session_command(target: &Path, prompt_file: &Path, opts: &LaunchOptions, session_id: Option<&str>) -> String {
    let file = shell_single_quote(&prompt_file.to_string_lossy());
    let mut args = opts.flags();
    if opts.agent == Harness::ClaudeCode {
        if let Some(n) = opts.chosen_name() {
            args.push_str(&format!(" -n {}", shell_single_quote(n)));
        }
    }
    if let Some(id) = session_id {
        args.push_str(&format!(" --session-id {}", shell_single_quote(id)));
    }
    let prompt = if opts.agent == Harness::Antigravity { " --prompt-interactive=\"$p\"" } else { " -- \"$p\"" };
    format!(
        "cd {} && p=\"$(cat {file})\" && rm -f {file} && {}{args}{prompt}",
        shell_single_quote(&target.to_string_lossy()),
        binary_name(opts.agent)
    )
}

pub fn applescript_launch(target: &Path, prompt_file: &Path, opts: &LaunchOptions) -> String {
    applescript_run(&session_command(target, prompt_file, opts, None))
}

/// A random version-4 UUID, for Grok's `--session-id`.
pub fn new_session_uuid() -> String {
    use rand::Rng;
    let mut b = [0u8; 16];
    rand::rng().fill_bytes(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
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
    use std::time::Duration;
    #[cfg(unix)]
    use std::time::Instant;

    fn dirs() -> Vec<String> {
        vec!["a".into(), "b".into(), "sonarqube".into()]
    }

    /// A `claude.cmd` running `body`.
    #[cfg(windows)]
    fn fake_binary(dir: &Path, body: &str) -> PathBuf {
        let p = dir.join("claude.cmd");
        std::fs::write(&p, format!("@echo off\r\n{body}\r\n")).unwrap();
        p
    }

    #[cfg(unix)]
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
    fn the_appimage_mount_is_dropped_from_xdg_data_dirs() {
        let appdir = "/tmp/.mount_MayaAb12";
        assert_eq!(without_appdir_prefix("/tmp/.mount_MayaAb12/usr/share:/usr/local/share:/usr/share", appdir), "/usr/local/share:/usr/share");
        assert_eq!(without_appdir_prefix("/usr/local/share:/usr/share", appdir), "/usr/local/share:/usr/share");
        assert_eq!(without_appdir_prefix("/tmp/.mount_MayaAb12/usr/share", appdir), "");
        assert_eq!(without_appdir_prefix("/tmp/.mount_MayaAb12x/share:/usr/share", appdir), "/tmp/.mount_MayaAb12x/share:/usr/share", "only entries inside the mount");
        assert_eq!(without_appdir_prefix("/a:/b", ""), "/a:/b");
    }

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn a_child_of_the_appimage_loses_the_bundles_environment() {
        use std::collections::HashMap;
        use std::ffi::OsStr;
        // What a Tauri AppImage's AppRun and GTK hook really set (seen in
        // /proc/<pid>/environ on Ubuntu 24.04), mount shortened.
        let m = "/tmp/.mount_maya-aDfnLHi";
        let mut cmd = std::process::Command::new("true");
        scrub_with(
            &mut cmd,
            env(&[
                ("APPIMAGE", "/home/u/.local/bin/maya-app"),
                ("APPDIR", m),
                ("ARGV0", "/home/u/.local/bin/maya-app"),
                ("OWD", "/home/u"),
                ("GTK_THEME", "Adwaita:light"),
                ("GTK_PATH", &format!("{m}//usr/lib/gtk-3.0")),
                ("XDG_DATA_DIRS", &format!("{m}/usr/share/:{m}/usr/share:/usr/share:/usr/local/share")),
                ("PATH", &format!("{m}/usr/bin/:{m}/bin/:/home/u/.local/bin:/usr/bin:/bin")),
                ("LD_LIBRARY_PATH", &format!("{m}/usr/lib/:{m}/lib/:")),
                ("PYTHONHOME", &format!("{m}/usr/")),
                ("GIO_MODULE_DIR", &format!("{m}//usr/lib/gio/modules")),
                ("GTK_IM_MODULE_FILE", &format!("{m}//usr/lib/gtk-3.0/3.0.0/immodules.cache")),
                ("HOME", "/home/u"),
                ("LANG", "en_GB.UTF-8"),
            ]),
        );
        let envs: HashMap<&OsStr, Option<&OsStr>> = cmd.get_envs().collect();
        for var in APPIMAGE_VARS {
            assert_eq!(envs.get(OsStr::new(var)), Some(&None), "{var} is removed");
        }
        let set = |k: &str| envs.get(OsStr::new(k)).copied();
        assert_eq!(set("XDG_DATA_DIRS"), Some(Some(OsStr::new("/usr/share:/usr/local/share"))));
        assert_eq!(set("PATH"), Some(Some(OsStr::new("/home/u/.local/bin:/usr/bin:/bin"))));
        for gone in ["LD_LIBRARY_PATH", "PYTHONHOME", "GIO_MODULE_DIR", "GTK_IM_MODULE_FILE"] {
            assert_eq!(set(gone), Some(None), "{gone}: nothing left outside the mount, so unset");
        }
        assert_eq!(set("HOME"), None, "untouched");
        assert_eq!(set("LANG"), None, "untouched");

        let mut cmd = std::process::Command::new("true");
        scrub_with(&mut cmd, env(&[("GTK_PATH", "/somewhere"), ("PATH", "/usr/bin")]));
        assert_eq!(cmd.get_envs().count(), 0, "outside an AppImage nothing changes");
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
        let models: Vec<String> = MODELS.iter().map(|s| s.to_string()).collect();
        assert!(LaunchOptions::default().validate(&models).is_ok());
        let ok = LaunchOptions { model: Some("opus".into()), effort: Some("xhigh".into()), mode: Some("acceptEdits".into()), ..Default::default() };
        assert!(ok.validate(&models).is_ok());
        let bad_model = LaunchOptions { model: Some("gpt; rm -rf /".into()), ..Default::default() };
        assert!(bad_model.validate(&models).unwrap_err().contains("model"));
        let bad_effort = LaunchOptions { effort: Some("turbo".into()), ..Default::default() };
        assert!(bad_effort.validate(&models).unwrap_err().contains("effort"));
        let bad_mode = LaunchOptions { mode: Some("yolo".into()), ..Default::default() };
        assert!(bad_mode.validate(&models).unwrap_err().contains("mode"));
        // Empty strings mean "default": no flag.
        let empty = LaunchOptions { model: Some("".into()), effort: Some("".into()), mode: Some("".into()), ..Default::default() };
        assert!(empty.validate(&models).is_ok());
        assert_eq!(empty.flags(), "");
    }

    #[test]
    fn launch_options_render_as_claude_flags() {
        assert_eq!(LaunchOptions::default().flags(), "");
        let all = LaunchOptions { model: Some("sonnet".into()), effort: Some("low".into()), mode: Some("plan".into()), ..Default::default() };
        assert_eq!(all.flags(), " --model sonnet --effort low --permission-mode plan");
        let one = LaunchOptions { effort: Some("max".into()), ..Default::default() };
        assert_eq!(one.flags(), " --effort max");
    }

    fn opts(agent: Harness) -> LaunchOptions {
        LaunchOptions { agent, ..Default::default() }
    }

    #[test]
    fn the_agent_defaults_to_claude_code_when_a_caller_leaves_it_out() {
        let o: LaunchOptions = serde_json::from_str(r#"{"model":"opus"}"#).unwrap();
        assert_eq!(o.agent, Harness::ClaudeCode);
        assert_eq!(o.name, None);
    }

    #[test]
    fn each_agent_renders_its_own_flags() {
        let all = |agent| LaunchOptions { agent, model: Some("m-1".into()), effort: Some("high".into()), mode: Some(modes(agent)[0].into()), ..Default::default() };
        assert_eq!(all(Harness::ClaudeCode).flags(), " --model m-1 --effort high --permission-mode manual");
        assert_eq!(all(Harness::Codex).flags(), " -m m-1 -c model_reasoning_effort=high -s read-only");
        assert_eq!(all(Harness::Antigravity).flags(), " --model m-1 --effort high --mode accept-edits");
        // Grok lists no effort values, so none is passed.
        assert_eq!(all(Harness::Grok).flags(), " -m m-1 --permission-mode default");
    }

    #[test]
    fn effort_and_mode_must_be_the_agents_own() {
        let o = |agent, effort: &str, mode: &str| LaunchOptions { agent, effort: Some(effort.into()), mode: Some(mode.into()), ..Default::default() };
        assert!(o(Harness::Codex, "ultra", "workspace-write").validate_shape().is_ok());
        assert!(o(Harness::Codex, "high", "plan").validate_shape().unwrap_err().contains("mode"));
        assert!(o(Harness::Antigravity, "xhigh", "plan").validate_shape().unwrap_err().contains("effort"));
        assert!(o(Harness::Grok, "high", "plan").validate_shape().unwrap_err().contains("effort"));
    }

    #[test]
    fn a_model_must_be_plain_and_in_the_agents_list() {
        let m = |model: &str| LaunchOptions { agent: Harness::Codex, model: Some(model.into()), ..Default::default() };
        let listed = vec!["gpt-6.1-sol".to_string()];
        assert!(m("gpt-6.1-sol").validate(&listed).is_ok());
        assert!(m("gpt-9").validate(&listed).unwrap_err().contains("model"));
        assert!(m("gpt; rm -rf /").validate_shape().unwrap_err().contains("model"));
        assert!(plain_model_id("claude-opus-4-6-thinking") && plain_model_id("grok-4.7"));
        assert!(!plain_model_id("") && !plain_model_id("a b") && !plain_model_id("$(x)"));
    }

    #[test]
    fn a_name_follows_the_rename_rules() {
        let n = |name: &str| LaunchOptions { name: Some(name.into()), ..Default::default() };
        assert!(n("Fix the CI").validate_shape().is_ok());
        assert!(n("   ").validate_shape().is_ok(), "blank means no name");
        assert_eq!(n("   ").chosen_name(), None);
        assert!(n("two\nlines").validate_shape().unwrap_err().contains("one line"));
        assert!(n(&"x".repeat(61)).validate_shape().unwrap_err().contains("too long"));
    }

    #[test]
    fn each_agent_runs_its_binary_with_the_prompt() {
        let line = |o: &LaunchOptions, id| session_command(Path::new("/r/a"), Path::new("/p/1.txt"), o, id);
        assert!(line(&opts(Harness::ClaudeCode), None).ends_with("&& claude -- \"$p\""));
        assert!(line(&opts(Harness::Codex), None).ends_with("&& codex -- \"$p\""));
        assert!(line(&opts(Harness::Antigravity), None).ends_with("&& agy --prompt-interactive=\"$p\""));
        assert!(line(&opts(Harness::Grok), Some("0b9c-id")).ends_with("&& grok --session-id '0b9c-id' -- \"$p\""));
    }

    #[test]
    fn each_agent_takes_a_dash_prompt_as_the_prompt() {
        // The prompt is never in the line itself: it is read from the file into
        // $p, and given after `--`, or as `--flag="$p"` for Go's flag parser.
        for agent in [Harness::ClaudeCode, Harness::Codex, Harness::Antigravity, Harness::Grok] {
            let s = session_command(Path::new("/r"), Path::new("/p/1.txt"), &opts(agent), None);
            assert!(s.ends_with(" -- \"$p\"") || s.ends_with("--prompt-interactive=\"$p\""), "{s}");
        }
    }

    #[test]
    fn a_claude_name_is_single_quoted() {
        let o = LaunchOptions { name: Some("it's \"$(rm -rf ~)\"".into()), ..Default::default() };
        let s = session_command(Path::new("/r"), Path::new("/p/1.txt"), &o, None);
        assert!(s.ends_with(r#"&& claude -n 'it'\''s "$(rm -rf ~)"' -- "$p""#), "{s}");
        // The other agents take no name flag: Maya renames them later.
        let codex = LaunchOptions { agent: Harness::Codex, name: Some("x".into()), ..Default::default() };
        assert!(!session_command(Path::new("/r"), Path::new("/p/1.txt"), &codex, None).contains(" -n "));
    }

    #[test]
    fn session_uuids_are_version_4() {
        let id = new_session_uuid();
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.iter().map(|p| p.len()).collect::<Vec<_>>(), vec![8, 4, 4, 4, 12]);
        assert!(parts[2].starts_with('4'));
        assert!("89ab".contains(&parts[3][..1]));
        assert_ne!(id, new_session_uuid());
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

    // A fake claude on Windows is a batch file, which cannot take the
    // classifier's multi-line prompt; the real one is claude.exe.
    #[cfg(unix)]
    #[test]
    fn classify_uses_the_reply_and_ignores_none() {
        let t = tempfile::tempdir().unwrap();
        let bin = fake_binary(t.path(), r#"echo '{"type":"result","result":"b"}'"#);
        assert_eq!(classify(Harness::ClaudeCode, &bin, None, t.path(), "p", &dirs(), Duration::from_secs(5)).as_deref(), Some("b"));
        let bin = fake_binary(t.path(), r#"echo '{"type":"result","result":"NONE"}'"#);
        assert_eq!(classify(Harness::ClaudeCode, &bin, None, t.path(), "p", &dirs(), Duration::from_secs(5)), None);
        // Another agent's envelope is read the same way.
        let bin = fake_binary(t.path(), r#"echo '{"text":"sonarqube","stopReason":"end_turn"}'"#);
        assert_eq!(classify(Harness::Grok, &bin, Some("grok-4.7"), t.path(), "p", &dirs(), Duration::from_secs(5)).as_deref(), Some("sonarqube"));
    }

    #[cfg(unix)]
    #[test]
    fn classify_times_out_and_falls_back() {
        let t = tempfile::tempdir().unwrap();
        let bin = fake_binary(t.path(), "sleep 5; echo b");
        let start = Instant::now();
        assert_eq!(classify(Harness::ClaudeCode, &bin, None, t.path(), "p", &dirs(), Duration::from_secs(1)), None);
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
        std::fs::OpenOptions::new().write(true).open(&old).unwrap().set_modified(stale).unwrap();
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

    fn oneshot_fixture(name: &str) -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/oneshot").join(name)).unwrap()
    }

    #[test]
    fn labels_name_every_agent() {
        assert_eq!(label(Harness::ClaudeCode), "Claude Code");
        assert_eq!(label(Harness::Codex), "Codex");
        assert_eq!(label(Harness::Antigravity), "Antigravity");
        assert_eq!(label(Harness::Grok), "Grok Build");
    }

    #[test]
    fn capabilities_follow_the_table() {
        let c = capabilities(Harness::ClaudeCode);
        assert!(c.compact && c.model_switch && c.effort_switch && c.slash_lines && c.shell_lines);
        assert_eq!(c.mode_cycle, Some(crate::answer::SHIFT_TAB));
        let x = capabilities(Harness::Codex);
        assert!(x.compact && x.slash_lines);
        assert!(!x.model_switch && !x.effort_switch, "Codex's /model opens a picker");
        let g = capabilities(Harness::Grok);
        assert!(g.compact && g.model_switch && g.slash_lines);
        assert_eq!(g.mode_cycle, Some(crate::answer::SHIFT_TAB));
        assert_eq!(capabilities(Harness::Antigravity).mode_cycle, Some(crate::answer::SHIFT_TAB));
    }

    #[test]
    fn oneshot_args_per_agent() {
        let pair = |a: &[String], f: &str, v: &str| a.windows(2).any(|w| w[0] == f && w[1] == v);
        let a = oneshot_args(Harness::ClaudeCode, Some("haiku"), Some("SYS"), "USER");
        assert_eq!(a[0], "-p");
        assert!(pair(&a, "--model", "haiku") && pair(&a, "--system-prompt", "SYS") && pair(&a, "--output-format", "json") && pair(&a, "--tools", "") && pair(&a, "--max-turns", "1"));
        assert_eq!(a.last().unwrap(), "USER");
        assert!(!oneshot_args(Harness::ClaudeCode, None, None, "U").iter().any(|x| x == "--model" || x == "--system-prompt"));

        let c = oneshot_args(Harness::Codex, Some("gpt-5.5"), Some("SYS"), "USER");
        assert_eq!(&c[..2], ["exec", "--json"]);
        assert!(c.contains(&"--ephemeral".to_string()) && c.contains(&"--skip-git-repo-check".to_string()) && pair(&c, "-s", "read-only") && pair(&c, "-m", "gpt-5.5"));
        assert_eq!(c[c.len() - 2], "--");
        assert_eq!(c.last().unwrap(), "SYS\n\nUSER", "no system flag: the system text leads the prompt");

        let g = oneshot_args(Harness::Antigravity, None, Some("SYS"), "USER");
        assert!(pair(&g, "--output-format", "json") && g.contains(&"--sandbox".to_string()) && g.contains(&"--disable-slash-commands".to_string()));
        assert_eq!(g.last().unwrap(), "-p=SYS\n\nUSER", "agy takes the prompt attached to -p");
        assert!(!g.iter().any(|x| x == "--model"));

        let k = oneshot_args(Harness::Grok, Some("grok-4.7"), Some("SYS"), "USER");
        assert_eq!(&k[..2], ["-p", "USER"]);
        assert!(pair(&k, "--output-format", "json") && pair(&k, "--tools", "") && pair(&k, "--max-turns", "1") && pair(&k, "--permission-mode", "plan") && pair(&k, "-m", "grok-4.7") && pair(&k, "--system-prompt-override", "SYS"));
    }

    #[test]
    fn final_text_reads_each_agents_envelope() {
        assert!(final_text(Harness::ClaudeCode, &oneshot_fixture("claude.json")).unwrap().contains("\"say\":\"Telling hexgrid: go ahead.\""));
        assert_eq!(final_text(Harness::Codex, &oneshot_fixture("codex.jsonl")).unwrap(), r#"{"say":"hi","action":null,"confirm":false}"#);
        assert_eq!(final_text(Harness::Antigravity, &oneshot_fixture("agy.json")).unwrap().trim(), r#"{"say":"hi","action":null,"confirm":false}"#);
        assert_eq!(final_text(Harness::Grok, &oneshot_fixture("grok.json")).unwrap(), r#"{"say":"hi","action":null,"confirm":false}"#);
    }

    #[test]
    fn final_text_reports_failures_and_garbage() {
        assert!(final_text(Harness::ClaudeCode, r#"{"type":"result","is_error":true,"result":"Not logged in"}"#).unwrap_err().contains("Not logged in"));
        assert!(final_text(Harness::ClaudeCode, "not json").unwrap_err().contains("not JSON"));
        assert!(final_text(Harness::Codex, "{\"type\":\"turn.started\"}\n").unwrap_err().contains("no agent message"));
        assert!(final_text(Harness::Codex, "{\"type\":\"error\",\"message\":\"quota\"}\n").unwrap_err().contains("quota"));
        assert!(final_text(Harness::Antigravity, r#"{"status":"ERROR","response":""}"#).unwrap_err().contains("agy failed"));
        assert!(final_text(Harness::Grok, r#"{"stopReason":"error"}"#).unwrap_err().contains("no text"));
    }

    #[test]
    fn find_binary_names_the_missing_command() {
        // A binary no machine has: the message names the command the user must install.
        let err = find_binary_named("maya-no-such-agent-xyz").unwrap_err();
        assert_eq!(err, "Could not find the maya-no-such-agent-xyz command.");
    }
}
