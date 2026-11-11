//! Running a custom tool's command: the user's own script that lists work items.

use std::io::{Read, Write};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::config::{CustomTool, GitProvider};
use crate::forge::{route, ForgeSite};
use crate::work_items::{ItemRepo, WorkItem};

/// How long a command may run.
pub const TIMEOUT: Duration = Duration::from_secs(30);
/// The most stdout a command may print.
pub const MAX_OUTPUT: usize = 4 * 1024 * 1024;
/// The most items kept from one list.
pub const MAX_ITEMS: usize = 500;
/// The most characters kept of an item's description.
pub const MAX_DESCRIPTION: usize = 20_000;
/// How much stderr is kept for the error line.
const MAX_STDERR: usize = 64 * 1024;

/// The argv that runs `command` through the user's shell: `[$SHELL or LOGIN_SHELL, "-lic", command]`; Windows `["cmd.exe", "/C", command]`.
pub fn shell_argv(command: &str, shell: Option<&str>) -> Vec<String> {
    if cfg!(windows) {
        return vec!["cmd.exe".into(), "/C".into(), command.into()];
    }
    let shell = shell.filter(|s| std::path::Path::new(s).is_absolute()).unwrap_or(crate::launch::LOGIN_SHELL);
    vec![shell.into(), "-lic".into(), command.into()]
}

/// True for what an interactive shell prints with no terminal: the job
/// control warnings, and the `exit` it echoes as it leaves.
fn is_shell_noise(line: &str) -> bool {
    line.contains("no job control in this shell") || line.contains("cannot set terminal process group") || line.trim() == "exit"
}

/// Reads `r` to its end, keeping at most `cap` bytes. True when there was more.
fn read_capped(mut r: impl Read, cap: usize, flag: Option<&AtomicBool>) -> (Vec<u8>, bool) {
    let mut out = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        match r.read(&mut buf) {
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Ok(0) | Err(_) => return (out, false),
            Ok(n) => {
                let room = cap - out.len();
                if n > room {
                    out.extend_from_slice(&buf[..room]);
                    if let Some(f) = flag {
                        f.store(true, Ordering::SeqCst);
                    }
                    return (out, true);
                }
                out.extend_from_slice(&buf[..n]);
            }
        }
    }
}

/// Kills the command and everything it started.
fn kill_all(child: &mut std::process::Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Runs `argv` with `stdin` (None: closed) and returns its stdout, or the error line.
/// Anything the command leaves running is stopped when it exits.
pub fn run(argv: &[String], stdin: Option<&str>, timeout: Duration) -> Result<String, String> {
    let (program, args) = argv.split_first().ok_or("the command could not run: no command")?;
    let mut cmd = crate::command(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // cmd.exe wants the command as typed, not quoted the way Rust would.
        match args.split_last() {
            Some((last, rest)) => cmd.args(rest).raw_arg(last),
            None => &mut cmd,
        };
    }
    #[cfg(not(windows))]
    cmd.args(args);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A session of its own: no controlling terminal for an interactive shell to
        // stop on, and its group id is the pid, so one kill reaches everything.
        unsafe {
            cmd.pre_exec(|| if libc::setsid() == -1 { Err(std::io::Error::last_os_error()) } else { Ok(()) });
        }
    }
    crate::launch::scrub_appimage_env(&mut cmd);
    cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    let deadline = Instant::now() + timeout;
    let mut child = cmd.spawn().map_err(|e| format!("the command could not run: {e}"))?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let text = text.as_bytes().to_vec();
        // Its own thread: a command that never reads must not hold us up.
        std::thread::spawn(move || {
            let _ = pipe.write_all(&text);
        });
    }
    let too_much = Arc::new(AtomicBool::new(false));
    let (out_tx, out_rx) = mpsc::channel();
    let (err_tx, err_rx) = mpsc::channel();
    if let Some(stdout) = child.stdout.take() {
        let flag = too_much.clone();
        std::thread::spawn(move || {
            let _ = out_tx.send(read_capped(stdout, MAX_OUTPUT, Some(&flag)));
        });
    }
    if let Some(stderr) = child.stderr.take() {
        std::thread::spawn(move || {
            let _ = err_tx.send(read_capped(stderr, MAX_STDERR, None).0);
        });
    }
    let status = loop {
        if too_much.load(Ordering::SeqCst) {
            kill_all(&mut child);
            return Err("the command printed more than 4 MB".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                kill_all(&mut child);
                return Err(format!("the command did not finish within {} s", TIMEOUT.as_secs()));
            }
            Err(e) => {
                kill_all(&mut child);
                return Err(format!("the command could not run: {e}"));
            }
        }
    };
    // Stop what the command left running, so its pipes close.
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let timed_out = || format!("the command did not finish within {} s", TIMEOUT.as_secs());
    let (stdout, overflow) = out_rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).map_err(|_| timed_out())?;
    if overflow {
        return Err("the command printed more than 4 MB".into());
    }
    let stderr = err_rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).map_err(|_| timed_out())?;
    match status.code() {
        Some(0) => Ok(String::from_utf8_lossy(&stdout).into_owned()),
        Some(code) => {
            let lines: Vec<String> = String::from_utf8_lossy(&stderr).lines().filter(|l| !l.trim().is_empty() && !is_shell_noise(l)).take(3).map(|l| l.chars().take(200).collect()).collect();
            Err(if lines.is_empty() { format!("the command exited with status {code}") } else { format!("the command exited with status {code}: “{}”", lines.join(" / ")) })
        }
        None => Err("the command was stopped by a signal".into()),
    }
}

const NO_ITEMS: &str = "the command did not print {\"items\": [...]}";

/// The `items` array of the first JSON object in `stdout` that has one: the whole output, else from each line starting with `{`.
fn find_items(stdout: &str) -> Option<Vec<Value>> {
    let items_of = |v: Value| match v {
        Value::Object(mut o) => match o.remove("items") {
            Some(Value::Array(a)) => Some(a),
            _ => None,
        },
        _ => None,
    };
    if let Ok(v) = serde_json::from_str::<Value>(stdout.trim()) {
        if let Some(a) = items_of(v) {
            return Some(a);
        }
    }
    let mut start = 0;
    for line in stdout.split_inclusive('\n') {
        if line.starts_with('{') {
            if let Some(Ok(v)) = serde_json::Deserializer::from_str(&stdout[start..]).into_iter::<Value>().next() {
                if let Some(a) = items_of(v) {
                    return Some(a);
                }
            }
        }
        start += line.len();
    }
    None
}

/// A string field, trimmed; None when missing, not a string or blank.
fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

/// The string entries of a list field; anything else gives none.
fn strings(v: &Value, key: &str) -> Vec<String> {
    v.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

/// The items of a list command's stdout; rc-file noise before the JSON is skipped.
pub fn parse_list(stdout: &str, tool: &CustomTool, sites: &[ForgeSite]) -> Result<Vec<WorkItem>, String> {
    let raw = find_items(stdout).ok_or(NO_ITEMS)?;
    let mut items = Vec::new();
    for v in &raw {
        let (Some(id), Some(title)) = (text(v, "id"), text(v, "title")) else { continue };
        let Some(url) = text(v, "url").filter(|u| crate::attachments::is_web_url(u)) else { continue };
        items.push(WorkItem {
            key: format!("{}:{id}", tool.id),
            source: tool.id.clone(),
            id,
            number: None,
            title,
            url,
            description: v.get("description").and_then(Value::as_str).unwrap_or("").chars().take(MAX_DESCRIPTION).collect(),
            status: v.get("status").and_then(Value::as_str).map(String::from),
            labels: strings(v, "labels"),
            assignees: strings(v, "assignees"),
            author: v.get("author").and_then(Value::as_str).unwrap_or("").into(),
            updated_at: v.get("updatedAt").and_then(Value::as_str).unwrap_or("").into(),
            repo: v.get("repo").and_then(Value::as_str).and_then(|r| repo_of(r, sites)),
            tracker: false,
        });
        if items.len() == MAX_ITEMS {
            break;
        }
    }
    Ok(items)
}

/// A repo path without a trailing `.git` and surrounding slashes or spaces; None when empty.
/// The repository path without `.git` and outer slashes; None when a part of
/// it could step outside the clones folder (`.`, `..`, an empty part, a
/// backslash or a control character): a tracker's data is not trusted.
fn clean_path(path: &str) -> Option<String> {
    let p = path.trim().trim_matches('/');
    let p = p.strip_suffix(".git").unwrap_or(p).trim_matches('/');
    let safe = |part: &str| !part.is_empty() && part != "." && part != ".." && !part.chars().any(|c| c == '\\' || c.is_control());
    (!p.is_empty() && p.split('/').all(safe)).then(|| p.to_string())
}

/// Where `repo` (as the script spelled it) lives, among the switched-on sites; None when Maya cannot place it.
pub fn repo_of(repo: &str, sites: &[ForgeSite]) -> Option<ItemRepo> {
    let repo = repo.trim();
    let has = |p: GitProvider| sites.iter().any(|s| s.provider == p);
    for (prefix, forge) in [("github:", GitProvider::Github), ("gitlab:", GitProvider::Gitlab)] {
        if let Some(rest) = repo.strip_prefix(prefix) {
            return if has(forge) { Some(ItemRepo { forge, path: clean_path(rest)? }) } else { None };
        }
    }
    let scp = repo.split_once('@').is_some_and(|(_, rest)| rest.split('/').next().unwrap_or("").contains(':'));
    if repo.contains("://") || scp {
        let (site, path) = route(repo, sites)?;
        return Some(ItemRepo { forge: site.provider, path: clean_path(&path)? });
    }
    if repo.contains('@') || repo.contains(':') {
        return None;
    }
    let path = clean_path(repo).filter(|p| p.contains('/'))?;
    let forge = [GitProvider::Github, GitProvider::Gitlab].into_iter().find(|p| has(*p))?;
    Some(ItemRepo { forge, path })
}

/// The item as the on-start command reads it: the list format, `repo` as `github:<path>` / `gitlab:<path>`.
pub fn item_json(item: &WorkItem) -> String {
    let mut v = json!({
        "id": item.id,
        "title": item.title,
        "url": item.url,
        "description": item.description,
        "labels": item.labels,
        "assignees": item.assignees,
        "author": item.author,
        "updatedAt": item.updated_at,
    });
    if let Some(status) = &item.status {
        v["status"] = json!(status);
    }
    if let Some(r) = &item.repo {
        let forge = match r.forge {
            GitProvider::Github => "github",
            GitProvider::Gitlab => "gitlab",
        };
        v["repo"] = json!(format!("{forge}:{}", r.path));
    }
    v.to_string()
}

/// Runs the tool's list command through `argv_of(command)`; a blank command is Ok(empty).
pub fn list_with(tool: &CustomTool, sites: &[ForgeSite], argv_of: &dyn Fn(&str) -> Vec<String>) -> Result<Vec<WorkItem>, String> {
    let command = tool.list_command.trim();
    if command.is_empty() {
        return Ok(Vec::new());
    }
    parse_list(&run(&argv_of(command), None, TIMEOUT)?, tool, sites)
}

/// `list_with` through the user's shell (`shell_argv` with `$SHELL`).
pub fn list(tool: &CustomTool, sites: &[ForgeSite]) -> Result<Vec<WorkItem>, String> {
    list_with(tool, sites, &|c| shell_argv(c, std::env::var("SHELL").ok().as_deref()))
}

/// Runs the tool's on-start command with `item_json(item)` on stdin; a blank one does nothing.
pub fn on_start(tool: &CustomTool, item: &WorkItem) -> Result<(), String> {
    let command = tool.start_command.trim();
    if command.is_empty() {
        return Ok(());
    }
    run(&shell_argv(command, std::env::var("SHELL").ok().as_deref()), Some(&item_json(item)), TIMEOUT).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn sh(script: &str) -> Vec<String> {
        vec!["sh".into(), "-c".into(), script.into()]
    }

    #[test]
    fn the_shell_runs_the_command_as_typed() {
        if cfg!(windows) {
            return;
        }
        assert_eq!(shell_argv("linear-items --mine", Some("/bin/zsh")), ["/bin/zsh", "-lic", "linear-items --mine"]);
        assert_eq!(shell_argv("x", Some("zsh")), [crate::launch::LOGIN_SHELL, "-lic", "x"]);
        assert_eq!(shell_argv("x", None), [crate::launch::LOGIN_SHELL, "-lic", "x"]);
    }

    #[cfg(unix)]
    #[test]
    fn stdout_is_the_answer() {
        assert_eq!(run(&sh("echo '{\"items\":[]}'"), None, TIMEOUT), Ok("{\"items\":[]}\n".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn a_failure_says_its_status_and_first_stderr_lines() {
        assert_eq!(run(&sh("echo one >&2; echo two >&2; echo three >&2; echo four >&2; exit 3"), None, TIMEOUT), Err("the command exited with status 3: “one / two / three”".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn the_shells_own_noise_is_left_out_of_a_failure() {
        let script = r"printf 'bash: cannot set terminal process group (1): Inappropriate ioctl for device\nbash: no job control in this shell\nboom\nexit\n' >&2; exit 2";
        assert_eq!(run(&sh(script), None, TIMEOUT), Err("the command exited with status 2: “boom”".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn a_timeout_kills_the_command_and_its_children() {
        let start = Instant::now();
        assert_eq!(run(&sh("sleep 30 & sleep 30"), None, Duration::from_millis(300)), Err("the command did not finish within 30 s".to_string()));
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn a_background_job_does_not_hold_up_or_empty_the_answer() {
        let start = Instant::now();
        assert_eq!(run(&sh("echo '{\"items\":[]}'; sleep 30 &"), None, TIMEOUT), Ok("{\"items\":[]}\n".to_string()));
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn a_timeout_really_stops_the_children() {
        let dir = std::env::temp_dir().join(format!("maya-custom-kill-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = format!("cd '{}' && sleep 30 & echo $! > '{}/pid'; sleep 30", dir.display(), dir.display());
        assert!(run(&sh(&script), None, Duration::from_millis(500)).is_err());
        let pid: i32 = std::fs::read_to_string(dir.join("pid")).unwrap().trim().parse().unwrap();
        let mut gone = false;
        for _ in 0..20 {
            if unsafe { libc::kill(pid, 0) } != 0 {
                gone = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(gone, "the background job is still alive");
    }

    #[cfg(unix)]
    #[test]
    fn too_much_output_is_an_error() {
        assert_eq!(run(&sh("head -c 5000000 /dev/zero"), None, TIMEOUT), Err("the command printed more than 4 MB".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn item_data_reaches_the_command_only_on_stdin() {
        let dir = std::env::temp_dir().join(format!("maya-custom-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = format!("cd '{}' && cat > got.json", dir.display());
        let json = r#"{"id":"$(touch pwned)"}"#;
        assert!(run(&sh(&script), Some(json), TIMEOUT).is_ok());
        assert_eq!(std::fs::read_to_string(dir.join("got.json")).unwrap(), json);
        assert!(!dir.join("pwned").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    use crate::config::CloneProtocol;

    fn tool() -> CustomTool {
        CustomTool { id: "custom-1".into(), name: "Linear".into(), enabled: true, list_command: String::new(), start_command: String::new(), me: String::new() }
    }

    fn site(provider: GitProvider, web_base: &str) -> ForgeSite {
        ForgeSite { provider, web_base: web_base.into(), clone: CloneProtocol::Ssh }
    }

    fn both() -> Vec<ForgeSite> {
        vec![site(GitProvider::Github, "https://github.com"), site(GitProvider::Gitlab, "https://gitlab.acme.io")]
    }

    const EXAMPLE: &str = r#"{"id":"ENG-1204","title":"Export reports as CSV","url":"https://linear.app/acme/issue/ENG-1204","description":"d","status":"Todo","labels":["frontend"],"assignees":["tiago"],"author":"maria","updatedAt":"2026-11-06T10:00:00Z","repo":"acme/web"}"#;

    #[test]
    fn an_item_needs_an_id_a_title_and_a_web_url() {
        let out = format!(
            r#"{{"items":[{EXAMPLE},{{"title":"no id","url":"https://x.io/1"}},{{"id":"B","title":"  ","url":"https://x.io/2"}},{{"id":"C","title":"file","url":"file:///etc/passwd"}}]}}"#
        );
        let items = parse_list(&out, &tool(), &both()).unwrap();
        assert_eq!(items.len(), 1);
        let i = &items[0];
        assert_eq!(i.key, "custom-1:ENG-1204");
        assert_eq!(i.source, "custom-1");
        assert_eq!(i.id, "ENG-1204");
        assert_eq!(i.status.as_deref(), Some("Todo"));
        assert_eq!(i.labels, ["frontend"]);
        assert_eq!(i.assignees, ["tiago"]);
        assert_eq!(i.author, "maria");
        assert_eq!(i.updated_at, "2026-11-06T10:00:00Z");
        assert_eq!(i.number, None);
        assert!(!i.tracker);
    }

    #[test]
    fn noise_before_the_json_is_skipped() {
        let out = format!("Restored session\n{{\"items\":[{EXAMPLE}]}}\nSaving session...\n");
        assert_eq!(parse_list(&out, &tool(), &both()).unwrap().len(), 1);
    }

    #[test]
    fn bad_output_is_an_error() {
        for out in ["not json", "{}", "{\"items\":3}"] {
            assert_eq!(parse_list(out, &tool(), &both()), Err("the command did not print {\"items\": [...]}".to_string()));
        }
    }

    #[test]
    fn unknown_fields_and_odd_types_are_ignored() {
        let out = r#"{"items":[{"id":"A","title":"t","url":"https://x.io/1","labels":["a",3,null],"priority":1,"assignees":"tiago"}]}"#;
        let items = parse_list(out, &tool(), &both()).unwrap();
        assert_eq!(items[0].labels, ["a"]);
        assert!(items[0].assignees.is_empty());
        assert_eq!(items[0].updated_at, "");
    }

    #[test]
    fn a_repo_that_could_leave_the_clones_folder_is_not_placed() {
        let s = both();
        for bad in ["acme/..", "../acme", "github:acme/..", "github:acme/./web", "acme//web", "gitlab:grp\\..\\x", "acme\\..\\..\\x", "acme/we\u{0}b", "git@github.com:acme/...git", "https://github.com/acme/../x.git"] {
            assert_eq!(repo_of(bad, &s), None, "{bad}");
        }
        assert_eq!(repo_of("github:acme/web.app", &s), Some(ItemRepo { forge: GitProvider::Github, path: "acme/web.app".into() }), "a dot inside a name is fine");
    }

    #[test]
    fn the_repo_is_placed_by_its_spelling() {
        let gh = |p: &str| Some(ItemRepo { forge: GitProvider::Github, path: p.into() });
        let gl = |p: &str| Some(ItemRepo { forge: GitProvider::Gitlab, path: p.into() });
        let s = both();
        assert_eq!(repo_of("acme/web", &s), gh("acme/web"));
        assert_eq!(repo_of("github:acme/web", &s), gh("acme/web"));
        assert_eq!(repo_of("gitlab:grp/sub/r", &s), gl("grp/sub/r"));
        assert_eq!(repo_of("git@gitlab.acme.io:grp/r.git", &s), gl("grp/r"));
        assert_eq!(repo_of("https://github.com/acme/web.git", &s), gh("acme/web"));
        assert_eq!(repo_of("https://bitbucket.org/x/y", &s), None);
        assert_eq!(repo_of("gitlab:g/r", &s[..1]), None);
        assert_eq!(repo_of("acme/web", &s[1..]), gl("acme/web"));
        assert_eq!(repo_of("", &s), None);
        assert_eq!(repo_of("web", &s), None);
    }

    #[test]
    fn the_start_command_reads_the_item_as_listed() {
        let out = format!(r#"{{"items":[{EXAMPLE},{{"id":"B","title":"t","url":"https://x.io/2"}}]}}"#);
        let items = parse_list(&out, &tool(), &both()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&item_json(&items[0])).unwrap();
        assert_eq!(v["id"], "ENG-1204");
        assert_eq!(v["title"], "Export reports as CSV");
        assert_eq!(v["url"], "https://linear.app/acme/issue/ENG-1204");
        assert_eq!(v["labels"], serde_json::json!(["frontend"]));
        assert_eq!(v["assignees"], serde_json::json!(["tiago"]));
        assert_eq!(v["repo"], "github:acme/web");
        let v: serde_json::Value = serde_json::from_str(&item_json(&items[1])).unwrap();
        assert!(v.get("repo").is_none());
        assert!(v.get("status").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_blank_command_lists_nothing_and_starts_nothing() {
        let t = tool();
        assert_eq!(list(&t, &both()), Ok(vec![]));
        let item = parse_list(&format!(r#"{{"items":[{EXAMPLE}]}}"#), &t, &both()).unwrap().remove(0);
        assert_eq!(on_start(&t, &item), Ok(()));
    }
}
