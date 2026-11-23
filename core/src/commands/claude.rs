//! Claude Code's commands, read from its files: skills, command files and the
//! commands of installed plugins.

use super::{
    dedupe, description_of, builtins, front_matter, project_dirs, usable_name, CommandInfo, CommandSource, Homes,
};
use crate::model::Harness;
use serde_json::Value;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Command folders go no deeper than this below their root.
const MAX_DEPTH: usize = 4;
/// The most read of a skill or command file, and of a JSON file.
const TEXT_MAX_BYTES: u64 = 64 * 1024;
const JSON_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// What Claude Code offers anywhere: user and plugin commands, no project's.
pub fn claude_machine(h: &Homes) -> Vec<CommandInfo> {
    collect(h, None)
}

/// What Claude Code offers in a session working in `cwd`.
pub fn claude_session(h: &Homes, cwd: &Path) -> Vec<CommandInfo> {
    collect(h, Some(cwd))
}

fn collect(h: &Homes, cwd: Option<&Path>) -> Vec<CommandInfo> {
    let projects = cwd.map(project_dirs).unwrap_or_default();
    let mut list = skills_in(&h.claude.join("skills"), CommandSource::User, None, true);
    list.extend(synced_skills(&h.claude.join("skills/synced")));
    list.extend(commands_in(&h.claude.join("commands"), CommandSource::User));
    for p in &projects {
        list.extend(skills_in(&p.join(".claude/skills"), CommandSource::Project, None, false));
    }
    for p in &projects {
        list.extend(commands_in(&p.join(".claude/commands"), CommandSource::Project));
    }
    list.extend(plugin_commands(h, cwd, &projects));
    list.extend(builtins(Harness::ClaudeCode));
    dedupe(list)
}

/// Up to `cap` bytes of a regular file, or None for anything else (a FIFO or
/// `/dev/zero` in a cloned repo would hang or fill memory) or on any error.
/// Text that is not UTF-8 is skipped; a cut that splits a last character is
/// dropped back to the character before it.
fn read_capped(path: &Path, cap: u64) -> Option<String> {
    if !fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    fs::File::open(path).ok()?.take(cap).read_to_end(&mut bytes).ok()?;
    match String::from_utf8(bytes) {
        Ok(text) => Some(text),
        Err(e) if e.utf8_error().error_len().is_none() => {
            let keep = e.utf8_error().valid_up_to();
            String::from_utf8(e.into_bytes()[..keep].to_vec()).ok()
        }
        Err(_) => None,
    }
}

/// A skill or command file: only its front matter and first line matter.
fn read(path: &Path) -> Option<String> {
    read_capped(path, TEXT_MAX_BYTES)
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&read_capped(path, JSON_MAX_BYTES)?).ok()
}

/// The entries of a folder, by name, so the list does not depend on the disk.
fn entries(dir: &Path) -> Vec<fs::DirEntry> {
    let mut all: Vec<_> = fs::read_dir(dir).into_iter().flatten().flatten().collect();
    all.sort_by_key(|e| e.file_name());
    all
}

/// One skill file: `fallback` names it when its front matter does not.
fn skill_file(path: &Path, fallback: &str, prefix: Option<&str>, source: CommandSource) -> Option<CommandInfo> {
    let text = read(path)?;
    let fm = front_matter(&text);
    if !fm.user_invocable {
        return None;
    }
    let name = fm.name.filter(|n| !n.is_empty()).unwrap_or_else(|| fallback.to_string());
    let name = match prefix {
        Some(p) => format!("{p}:{name}"),
        None => name,
    };
    usable_name(&name).then(|| CommandInfo::new(&name, &description_of(&text), source))
}

/// `<root>/<dir>/SKILL.md` for each folder of `root`.
pub(super) fn skills_in(root: &Path, source: CommandSource, prefix: Option<&str>, skip_synced: bool) -> Vec<CommandInfo> {
    entries(root)
        .into_iter()
        .filter(|e| !(skip_synced && e.file_name() == "synced"))
        .filter_map(|e| skill_file(&e.path().join("SKILL.md"), &e.file_name().to_string_lossy(), prefix, source))
        .collect()
}

/// `<claude>/skills/synced/<id>/<name>/SKILL.md`, as `anthropic-skills:<name>`.
fn synced_skills(root: &Path) -> Vec<CommandInfo> {
    entries(root)
        .into_iter()
        .flat_map(|id| skills_in(&id.path(), CommandSource::User, Some("anthropic-skills"), false))
        .collect()
}

/// `<root>/**/*.md`, named by the path below `root` with folders joined by `:`.
fn commands_in(root: &Path, source: CommandSource) -> Vec<CommandInfo> {
    command_files(root, source, ":", MAX_DEPTH)
}

/// `<root>/**/*.md` down to `depth` levels (1 is the folder itself), named by
/// the path below `root` with folders joined by `sep`.
pub(super) fn command_files(root: &Path, source: CommandSource, sep: &str, depth: usize) -> Vec<CommandInfo> {
    let mut list = Vec::new();
    walk(root, &mut Vec::new(), (source, sep, depth), &mut list);
    list
}

fn walk(dir: &Path, below: &mut Vec<String>, opts: (CommandSource, &str, usize), list: &mut Vec<CommandInfo>) {
    let (source, sep, depth) = opts;
    for e in entries(dir) {
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(kind) = e.file_type() else { continue };
        if kind.is_dir() {
            // A symlinked folder reads as a symlink here, so it is not followed.
            if below.len() + 1 < depth {
                below.push(name);
                walk(&e.path(), below, opts, list);
                below.pop();
            }
        } else if let Some(stem) = name.strip_suffix(".md") {
            // A link to a file counts; a link to a folder does not.
            if kind.is_file() || (kind.is_symlink() && e.path().is_file()) {
                let full = below.iter().map(String::as_str).chain([stem]).collect::<Vec<_>>().join(sep);
                if !usable_name(&full) {
                    continue;
                }
                if let Some(text) = read(&e.path()) {
                    list.push(CommandInfo::new(&full, &description_of(&text), source));
                }
            }
        }
    }
}

/// `enabledPlugins` merged from the settings files, later winning.
fn enabled_plugins(h: &Homes, projects: &[PathBuf]) -> serde_json::Map<String, Value> {
    let mut files = vec![h.claude.join("settings.json")];
    // Outermost project first, so the nearest wins.
    for p in projects.iter().rev() {
        files.push(p.join(".claude/settings.json"));
        files.push(p.join(".claude/settings.local.json"));
    }
    let mut merged = serde_json::Map::new();
    for f in files {
        if let Some(Value::Object(map)) = read_json(&f).and_then(|v| v.get("enabledPlugins").cloned()) {
            merged.extend(map);
        }
    }
    merged
}

fn plugin_commands(h: &Homes, cwd: Option<&Path>, projects: &[PathBuf]) -> Vec<CommandInfo> {
    let Some(installed) = read_json(&h.claude.join("plugins/installed_plugins.json")) else { return Vec::new() };
    let Some(plugins) = installed.get("plugins").and_then(Value::as_object) else { return Vec::new() };
    let enabled = enabled_plugins(h, projects);
    let mut ids: Vec<&String> = plugins.keys().collect();
    ids.sort();
    let mut list = Vec::new();
    for id in ids {
        if enabled.get(id.as_str()) == Some(&Value::Bool(false)) {
            continue;
        }
        let Some(entries) = plugins[id].as_array() else { continue };
        let install = entries.iter().find_map(|e| {
            let in_scope = match e.get("scope").and_then(Value::as_str) {
                Some("user") => true,
                Some("project" | "local") => cwd
                    .zip(e.get("projectPath").and_then(Value::as_str))
                    .is_some_and(|(cwd, p)| cwd.starts_with(p)),
                _ => false,
            };
            in_scope.then(|| e.get("installPath").and_then(Value::as_str)).flatten()
        });
        if let Some(path) = install {
            list.extend(one_plugin(id, Path::new(path)));
        }
    }
    list
}

/// Paths a manifest key names, relative to the plugin: a string or a list.
fn manifest_paths(manifest: &Value, key: &str, root: &Path) -> Option<Vec<PathBuf>> {
    let paths = match manifest.get(key)? {
        Value::String(s) => vec![s.as_str()],
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        _ => return None,
    };
    Some(paths.into_iter().map(|p| root.join(p)).collect())
}

fn one_plugin(id: &str, root: &Path) -> Vec<CommandInfo> {
    if !root.is_dir() {
        return Vec::new();
    }
    let manifest = read_json(&root.join(".claude-plugin/plugin.json")).unwrap_or(Value::Null);
    let ns = manifest
        .get("name")
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| id.split('@').next().unwrap_or(id))
        .to_string();
    let source = CommandSource::Plugin;
    let mut list = Vec::new();

    let command_paths = manifest_paths(&manifest, "commands", root).unwrap_or_else(|| vec![root.join("commands")]);
    for path in command_paths {
        let files = if path.is_dir() { entries(&path).into_iter().map(|e| e.path()).collect() } else { vec![path] };
        for file in files {
            let Some(stem) = file.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".md")) else { continue };
            let name = format!("{ns}:{stem}");
            if !usable_name(&name) {
                continue;
            }
            if let Some(text) = read(&file) {
                list.push(CommandInfo::new(&name, &description_of(&text), source));
            }
        }
    }

    list.extend(skills_in(&root.join("skills"), source, Some(&ns), false));
    list.extend(skill_file(&root.join("SKILL.md"), &ns, Some(&ns), source));
    for path in manifest_paths(&manifest, "skills", root).unwrap_or_default() {
        if path.join("SKILL.md").is_file() {
            let dir = path.file_name().and_then(|n| n.to_str()).unwrap_or(&ns);
            list.extend(skill_file(&path.join("SKILL.md"), dir, Some(&ns), source));
        } else {
            list.extend(skills_in(&path, source, Some(&ns), false));
        }
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandSource::*, Homes};
    use std::fs;
    use std::path::{Path, PathBuf};

    struct Env {
        _t: tempfile::TempDir,
        root: PathBuf,
        homes: Homes,
    }

    fn env() -> Env {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().canonicalize().unwrap();
        let homes = Homes { home: root.join("home"), claude: root.join("home/.claude") };
        fs::create_dir_all(&homes.claude).unwrap();
        Env { _t: t, root, homes }
    }

    fn write(path: impl AsRef<Path>, text: &str) {
        let path = path.as_ref();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn find<'a>(list: &'a [CommandInfo], name: &str) -> Option<&'a CommandInfo> {
        list.iter().find(|c| c.name == name)
    }

    fn has(list: &[CommandInfo], name: &str, description: &str, source: CommandSource) -> bool {
        find(list, name).is_some_and(|c| c.description == description && c.source == source)
    }

    /// An installed plugin `id` of the given scope, with its files under `plugins/<ns>`.
    fn install(e: &Env, id: &str, scope: &str, project: Option<&Path>) -> PathBuf {
        let ns = id.split('@').next().unwrap();
        let path = e.homes.claude.join("plugins").join(ns);
        write(path.join(".claude-plugin/plugin.json"), &format!(r#"{{"name":"{ns}"}}"#));
        let mut entry = serde_json::json!({"scope": scope, "installPath": path});
        if let Some(p) = project {
            entry["projectPath"] = serde_json::json!(p);
        }
        write(
            e.homes.claude.join("plugins/installed_plugins.json"),
            &serde_json::json!({"version": 2, "plugins": {id: [entry]}}).to_string(),
        );
        path
    }

    #[test]
    fn user_skills_and_commands_are_listed_with_their_names() {
        let e = env();
        write(e.homes.claude.join("skills/review/SKILL.md"), "---\ndescription: Review it\n---\nbody");
        write(e.homes.claude.join("commands/frontend/component.md"), "Make a component\nmore");
        let list = claude_machine(&e.homes);
        assert!(has(&list, "review", "Review it", User), "{list:?}");
        assert!(has(&list, "frontend:component", "Make a component", User), "{list:?}");
    }

    #[test]
    fn a_skill_takes_its_front_matter_name_and_synced_ones_are_namespaced() {
        let e = env();
        write(e.homes.claude.join("skills/dir/SKILL.md"), "---\nname: named\ndescription: D\n---\n");
        write(e.homes.claude.join("skills/synced/abc/docs/SKILL.md"), "---\ndescription: Docs\n---\n");
        let list = claude_machine(&e.homes);
        assert!(has(&list, "named", "D", User), "{list:?}");
        assert!(has(&list, "anthropic-skills:docs", "Docs", User), "{list:?}");
        assert!(find(&list, "synced").is_none() && find(&list, "abc").is_none());
    }

    #[test]
    fn a_skill_that_is_not_user_invocable_is_left_out() {
        let e = env();
        write(e.homes.claude.join("skills/hidden/SKILL.md"), "---\nuser-invocable: false\n---\nx");
        assert!(find(&claude_machine(&e.homes), "hidden").is_none());
    }

    #[test]
    fn command_folders_stop_at_four_levels() {
        let e = env();
        write(e.homes.claude.join("commands/a/b/c/d.md"), "four");
        write(e.homes.claude.join("commands/a/b/c/d/e.md"), "five");
        let list = claude_machine(&e.homes);
        assert!(find(&list, "a:b:c:d").is_some());
        assert!(find(&list, "a:b:c:d:e").is_none());
    }

    #[test]
    fn an_enabled_user_plugin_lists_namespaced_commands_and_skills() {
        let e = env();
        let p = install(&e, "commit-commands@tkc", "user", None);
        write(p.join("commands/commit.md"), "Commit it");
        write(p.join("skills/clean/SKILL.md"), "---\ndescription: Clean up\n---\n");
        let list = claude_machine(&e.homes);
        assert!(has(&list, "commit-commands:commit", "Commit it", Plugin), "{list:?}");
        assert!(has(&list, "commit-commands:clean", "Clean up", Plugin), "{list:?}");
    }

    #[test]
    fn a_plugin_manifest_can_point_at_other_command_paths() {
        let e = env();
        let p = install(&e, "tools@tkc", "user", None);
        write(p.join(".claude-plugin/plugin.json"), r#"{"name":"tools","commands":["./extra/go.md"]}"#);
        write(p.join("commands/ignored.md"), "Not listed");
        write(p.join("extra/go.md"), "Go");
        let list = claude_machine(&e.homes);
        assert!(has(&list, "tools:go", "Go", Plugin), "{list:?}");
        assert!(find(&list, "tools:ignored").is_none());
    }

    #[test]
    fn a_plugin_disabled_in_settings_is_left_out() {
        let e = env();
        let p = install(&e, "off@tkc", "user", None);
        write(p.join("commands/x.md"), "X");
        write(e.homes.claude.join("settings.json"), r#"{"enabledPlugins":{"off@tkc":false}}"#);
        assert!(find(&claude_machine(&e.homes), "off:x").is_none());
    }

    #[test]
    fn a_project_scoped_plugin_counts_only_inside_its_project() {
        let e = env();
        let proj = e.root.join("proj");
        fs::create_dir_all(proj.join("sub")).unwrap();
        let p = install(&e, "local@tkc", "project", Some(&proj));
        write(p.join("commands/x.md"), "X");
        assert!(find(&claude_machine(&e.homes), "local:x").is_none());
        assert!(find(&claude_session(&e.homes, &proj.join("sub")), "local:x").is_some());
        assert!(find(&claude_session(&e.homes, &e.root), "local:x").is_none());
        // A project's settings can turn it off.
        write(proj.join(".claude/settings.local.json"), r#"{"enabledPlugins":{"local@tkc":false}}"#);
        assert!(find(&claude_session(&e.homes, &proj), "local:x").is_none());
    }

    #[test]
    fn project_commands_come_only_with_a_session_and_from_parents_up_to_the_repo() {
        let e = env();
        let repo = e.root.join("repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::create_dir_all(repo.join("app")).unwrap();
        write(repo.join(".claude/commands/ship.md"), "Ship it");
        write(repo.join("app/.claude/skills/local/SKILL.md"), "---\ndescription: Local\n---\n");
        write(e.root.join(".claude/commands/above.md"), "Above the repo");
        let list = claude_session(&e.homes, &repo.join("app"));
        assert!(has(&list, "ship", "Ship it", Project), "{list:?}");
        assert!(has(&list, "local", "Local", Project), "{list:?}");
        assert!(find(&list, "above").is_none());
        assert!(find(&claude_machine(&e.homes), "ship").is_none());
    }

    #[test]
    fn a_user_skill_wins_over_a_builtin_of_the_same_name() {
        let e = env();
        write(e.homes.claude.join("skills/compact/SKILL.md"), "---\ndescription: Mine\n---\n");
        let list = claude_machine(&e.homes);
        assert_eq!(list.iter().filter(|c| c.name == "compact").count(), 1);
        assert!(has(&list, "compact", "Mine", User));
    }

    #[test]
    fn a_file_is_read_only_up_to_the_cap_and_only_when_regular() {
        let e = env();
        write(e.root.join("big.md"), &"x".repeat(1000));
        assert_eq!(read_capped(&e.root.join("big.md"), 100).unwrap().len(), 100);
        assert!(read_capped(&e.root, 100).is_none());
        assert!(read_capped(&e.root.join("missing"), 100).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_or_device_in_place_of_a_skill_is_skipped_without_hanging() {
        let e = env();
        let repo = e.root.join("repo");
        let fifo = repo.join(".claude/skills/evil/SKILL.md");
        fs::create_dir_all(fifo.parent().unwrap()).unwrap();
        assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
        let zero = repo.join(".claude/skills/zero/SKILL.md");
        fs::create_dir_all(zero.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink("/dev/zero", &zero).unwrap();
        let cmd = repo.join(".claude/commands/pipe.md");
        fs::create_dir_all(cmd.parent().unwrap()).unwrap();
        assert!(std::process::Command::new("mkfifo").arg(&cmd).status().unwrap().success());
        let start = std::time::Instant::now();
        let list = claude_session(&e.homes, &repo);
        assert!(start.elapsed().as_secs() < 5);
        assert!(find(&list, "evil").is_none() && find(&list, "zero").is_none() && find(&list, "pipe").is_none());
        assert!(find(&list, "compact").is_some());
    }

    #[test]
    fn names_that_cannot_be_typed_are_left_out() {
        let e = env();
        let repo = e.root.join("repo");
        write(repo.join(".claude/commands/.hidden.md"), "Hidden");
        write(repo.join(".claude/commands/two words.md"), "Spaced");
        // Windows allows no control characters in a file name, so these
        // files can exist only elsewhere; the front matter "a\tb" below
        // covers a control character on every platform.
        if cfg!(unix) {
            write(repo.join(".claude/commands/bell\u{7}.md"), "Control");
            write(repo.join(".claude/skills/tab\tdir/SKILL.md"), "---\ndescription: T\n---\n");
        }
        write(repo.join(".claude/commands/.git/inside.md"), "In a dot folder");
        write(repo.join(".claude/commands/ok.md"), "Fine");
        write(repo.join(".claude/skills/named/SKILL.md"), "---\nname: \"has space\"\n---\n");
        write(repo.join(".claude/skills/dotted/SKILL.md"), "---\nname: .dot\n---\n");
        write(repo.join(".claude/skills/blank/SKILL.md"), "---\nname: \"\"\n---\n");
        let p = install(&e, "tools@tkc", "user", None);
        write(p.join("commands/ bad.md"), "Leading space");
        write(p.join("skills/fine/SKILL.md"), "---\nname: \"a\tb\"\n---\n");
        let list = claude_session(&e.homes, &repo);
        let mine: Vec<&str> = list.iter().filter(|c| c.source != Builtin).map(|c| c.name.as_str()).collect();
        // A front matter name that cannot be typed falls back to nothing, not to the folder.
        assert_eq!(mine, ["blank", "ok"], "{list:?}");
    }

    #[test]
    fn broken_files_are_skipped() {
        let e = env();
        write(e.homes.claude.join("plugins/installed_plugins.json"), "{not json");
        write(e.homes.claude.join("settings.json"), "");
        write(e.homes.claude.join("commands/ok.md"), "Fine");
        fs::write(e.homes.claude.join("commands/binary.md"), [0xff, 0xfe, 0x00]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(e.homes.claude.join("commands"), e.homes.claude.join("commands/loop")).unwrap();
        let list = claude_session(&e.homes, &e.root);
        assert!(has(&list, "ok", "Fine", User) && find(&list, "compact").is_some(), "{list:?}");
        assert!(find(&list, "binary").is_none());

        write(
            e.homes.claude.join("plugins/installed_plugins.json"),
            r#"{"version":2,"plugins":{"gone@x":[{"scope":"user","installPath":"/nonexistent/x"}],"bad@x":"nope"}}"#,
        );
        let list = claude_session(&e.homes, &e.root);
        assert!(has(&list, "ok", "Fine", User), "{list:?}");
    }
}
