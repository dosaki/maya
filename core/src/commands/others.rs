//! The commands of every agent but Claude Code, read from their folders, and
//! the machine-wide lists Maya shares with its clients.

use super::claude::{claude_machine, claude_session, command_files, skills_in};
use super::{builtins, dedupe, project_dirs, AgentCommands, CommandInfo, CommandSource, Homes};
use crate::agents;
use crate::model::Harness;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long the shared lists are reused before they are read again.
const SHARED_MAX_AGE: Duration = Duration::from_secs(60);
/// The depth of OpenCode's command folders, as for Claude Code's.
const MAX_DEPTH: usize = 4;

type Shared = Option<(Instant, Vec<AgentCommands>)>;
static SHARED: Mutex<Shared> = Mutex::new(None);

/// What the agent offers anywhere: user commands and built-ins, no project's.
pub fn machine_wide(h: Harness, homes: &Homes) -> Vec<CommandInfo> {
    list(h, None, homes)
}

/// What the agent offers in a session working in `cwd`.
pub fn for_session(h: Harness, cwd: &Path, homes: &Homes) -> Vec<CommandInfo> {
    list(h, Some(cwd), homes)
}

fn list(h: Harness, cwd: Option<&Path>, homes: &Homes) -> Vec<CommandInfo> {
    use CommandSource::{Project, User};
    let home = &homes.home;
    let projects = cwd.map(project_dirs).unwrap_or_default();
    let mut list = Vec::new();
    match h {
        Harness::ClaudeCode => {
            return match cwd {
                Some(cwd) => claude_session(homes, cwd),
                None => claude_machine(homes),
            }
        }
        Harness::OpenCode => {
            for dir in ["commands", "command"] {
                list.extend(command_files(&home.join(".config/opencode").join(dir), User, "/", MAX_DEPTH));
            }
            if let Some(cwd) = cwd {
                for dir in [".opencode/commands", ".opencode/command"] {
                    list.extend(command_files(&cwd.join(dir), Project, "/", MAX_DEPTH));
                }
            }
        }
        Harness::Kiro => {
            list.extend(skills_in(&home.join(".kiro/skills"), User, None, false));
            list.extend(command_files(&home.join(".kiro/prompts"), User, "/", 1));
            if let Some(cwd) = cwd {
                list.extend(skills_in(&cwd.join(".kiro/skills"), Project, None, false));
                list.extend(command_files(&cwd.join(".kiro/prompts"), Project, "/", 1));
            }
        }
        Harness::Grok => {
            list.extend(skills_in(&home.join(".grok/skills"), User, None, false));
            list.extend(command_files(&home.join(".grok/commands"), User, "/", 1));
            for p in &projects {
                list.extend(skills_in(&p.join(".grok/skills"), Project, None, false));
                list.extend(command_files(&p.join(".grok/commands"), Project, "/", 1));
            }
        }
        Harness::Antigravity => {
            list.extend(skills_in(&home.join(".gemini/config/skills"), User, None, false));
            for p in &projects {
                list.extend(skills_in(&p.join(".agents/skills"), Project, None, false));
            }
        }
        // Codex types skills as `$name` and its custom prompts are deprecated.
        Harness::Codex | Harness::Other => {}
    }
    list.extend(builtins(h));
    dedupe(list)
}

/// This machine's machine-wide lists for Claude Code and every agent
/// `agents::cached()` lists; rebuilt at most every 60 s. It never waits for
/// the agents' listing: until one lands, it is Claude Code alone.
pub fn shared() -> Vec<AgentCommands> {
    let mut guard = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, lists)) = guard.as_ref() {
        if at.elapsed() < SHARED_MAX_AGE {
            return lists.clone();
        }
    }
    let homes = Homes::current();
    let mut harnesses = vec![Harness::ClaudeCode];
    for a in agents::cached().unwrap_or_default() {
        if a.harness != Harness::Other && !harnesses.contains(&a.harness) {
            harnesses.push(a.harness);
        }
    }
    let lists: Vec<AgentCommands> = harnesses
        .into_iter()
        .map(|harness| AgentCommands { harness, commands: machine_wide(harness, &homes) })
        .collect();
    *guard = Some((Instant::now(), lists.clone()));
    lists
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: impl AsRef<Path>, text: &str) {
        let path = path.as_ref();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn homes(root: &Path) -> Homes {
        Homes { home: root.join("home"), claude: root.join("home/.claude") }
    }

    fn names(list: &[CommandInfo]) -> Vec<&str> {
        list.iter().filter(|c| c.source != CommandSource::Builtin).map(|c| c.name.as_str()).collect()
    }

    #[test]
    fn opencode_commands_join_folders_with_a_slash() {
        let t = tempfile::tempdir().unwrap();
        let h = homes(t.path());
        write(h.home.join(".config/opencode/commands/team/review.md"), "Review");
        write(h.home.join(".config/opencode/command/plain.md"), "Plain");
        let list = machine_wide(Harness::OpenCode, &h);
        assert_eq!(names(&list), ["team/review", "plain"], "{list:?}");
        assert!(list.iter().any(|c| c.name == "compact" && c.source == CommandSource::Builtin));
    }

    #[test]
    fn kiro_prompts_and_skills_are_listed() {
        let t = tempfile::tempdir().unwrap();
        let h = homes(t.path());
        write(h.home.join(".kiro/skills/deploy/SKILL.md"), "---\ndescription: Deploy\n---\n");
        write(h.home.join(".kiro/prompts/standup.md"), "Write a standup");
        write(h.home.join(".kiro/prompts/nested/skipped.md"), "Too deep");
        let list = machine_wide(Harness::Kiro, &h);
        assert_eq!(names(&list), ["deploy", "standup"], "{list:?}");
    }

    #[test]
    fn codex_has_builtins_only() {
        let t = tempfile::tempdir().unwrap();
        let h = homes(t.path());
        write(h.home.join(".codex/prompts/x.md"), "X");
        let list = machine_wide(Harness::Codex, &h);
        assert!(!list.is_empty() && list.iter().all(|c| c.source == CommandSource::Builtin), "{list:?}");
    }

    #[test]
    fn for_session_adds_project_commands_machine_wide_does_not() {
        let t = tempfile::tempdir().unwrap();
        let h = homes(t.path());
        let cwd = t.path().join("proj");
        write(cwd.join(".opencode/commands/o.md"), "O");
        write(cwd.join(".kiro/prompts/k.md"), "K");
        write(cwd.join(".grok/commands/g.md"), "G");
        write(cwd.join(".agents/skills/a/SKILL.md"), "---\ndescription: A\n---\n");
        for (harness, name) in [
            (Harness::OpenCode, "o"),
            (Harness::Kiro, "k"),
            (Harness::Grok, "g"),
            (Harness::Antigravity, "a"),
        ] {
            let with = for_session(harness, &cwd, &h);
            assert!(with.iter().any(|c| c.name == name && c.source == CommandSource::Project), "{harness:?}");
            assert!(!machine_wide(harness, &h).iter().any(|c| c.name == name), "{harness:?}");
        }
    }

    #[test]
    fn grok_hides_a_skill_that_is_not_user_invocable() {
        let t = tempfile::tempdir().unwrap();
        let h = homes(t.path());
        write(h.home.join(".grok/skills/hidden/SKILL.md"), "---\nuser-invocable: false\n---\n");
        write(h.home.join(".gemini/config/skills/g/SKILL.md"), "---\ndescription: G\n---\n");
        assert!(names(&machine_wide(Harness::Grok, &h)).is_empty());
        assert_eq!(names(&machine_wide(Harness::Antigravity, &h)), ["g"]);
    }

    #[test]
    fn other_has_nothing() {
        let t = tempfile::tempdir().unwrap();
        assert!(for_session(Harness::Other, Path::new("/"), &homes(t.path())).is_empty());
    }

    #[test]
    fn shared_lists_claude_code_and_is_reused_while_fresh() {
        let first = shared();
        assert_eq!(first[0].harness, Harness::ClaudeCode);
        assert!(first.iter().all(|l| l.harness != Harness::Other));
        assert_eq!(shared(), first);
    }
}
