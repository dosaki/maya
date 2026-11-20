//! The slash commands an agent offers, for autocompletion in Maya's reply box.
//! This module holds the shared types and the small readers every source of
//! commands (built-ins, skill and command files) goes through.

mod builtins;
mod claude;
mod others;

pub use builtins::builtins;
pub use claude::{claude_machine, claude_session};
pub use others::{for_session, machine_wide, shared};

use crate::model::Harness;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The longest description kept, in characters, so a row stays one line.
const DESCRIPTION_MAX: usize = 100;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommandInfo {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub source: CommandSource,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CommandSource {
    Builtin,
    User,
    Plugin,
    Project,
    /// A source a newer Maya adds, so an older one still reads the list.
    #[serde(other)]
    Other,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AgentCommands {
    pub harness: Harness,
    pub commands: Vec<CommandInfo>,
}

impl CommandInfo {
    pub fn new(name: &str, description: &str, source: CommandSource) -> CommandInfo {
        CommandInfo {
            name: name.to_string(),
            description: cut_description(description),
            source,
        }
    }
}

/// The first non-empty line, at most 100 characters; a longer one is cut at a
/// word end and marked with an ellipsis.
pub fn cut_description(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    if line.chars().count() <= DESCRIPTION_MAX {
        return line.to_string();
    }
    let keep: Vec<char> = line.chars().take(DESCRIPTION_MAX - 1).collect();
    let mut end = keep.len();
    if !line.chars().nth(end).is_some_and(char::is_whitespace) {
        if let Some(space) = keep.iter().rposition(|c| c.is_whitespace()) {
            end = space;
        }
    }
    let cut: String = keep[..end].iter().collect();
    format!("{}…", cut.trim_end())
}

pub struct FrontMatter {
    pub name: Option<String>,
    pub description: Option<String>,
    pub user_invocable: bool,
}

/// Reads the `---` block at the top of a skill or command file. Only the
/// three keys Maya needs, with no YAML crate: `key: value` lines, quotes
/// stripped, and a `>` or `|` value taking the indented lines after it.
/// A file with no block, or one never closed, reads as no front matter.
pub fn front_matter(text: &str) -> FrontMatter {
    let mut fm = FrontMatter { name: None, description: None, user_invocable: true };
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return fm;
    }
    let block: Vec<&str> = lines.by_ref().take_while(|l| l.trim_end() != "---").collect();
    // take_while also stops at the end of the text, so check the close was seen.
    if text.lines().skip(1).all(|l| l.trim_end() != "---") {
        return fm;
    }
    let mut i = 0;
    while i < block.len() {
        let line = block[i];
        i += 1;
        let Some((key, value)) = line.split_once(':') else { continue };
        if line.starts_with(char::is_whitespace) {
            continue;
        }
        let mut value = value.trim().to_string();
        if matches!(value.as_str(), ">" | ">-" | "|" | "|-") {
            let mut parts = Vec::new();
            while i < block.len() && block[i].starts_with(char::is_whitespace) {
                parts.push(block[i].trim());
                i += 1;
            }
            value = parts.join(" ");
        } else {
            value = unquote(&value).to_string();
        }
        match key.trim() {
            "name" => fm.name = Some(value),
            "description" => fm.description = Some(value),
            "user-invocable" => fm.user_invocable = value != "false",
            _ => {}
        }
    }
    fm
}

fn unquote(value: &str) -> &str {
    for q in ['"', '\''] {
        if let Some(inner) = value.strip_prefix(q).and_then(|v| v.strip_suffix(q)) {
            return inner;
        }
    }
    value
}

/// The description a command file gives: the front matter's, else the first
/// non-empty line of the text.
pub fn description_of(text: &str) -> String {
    match front_matter(text).description {
        Some(d) if !d.is_empty() => cut_description(&d),
        _ => cut_description(body(text)),
    }
}

/// The text after the front matter, or all of it when there is none. A
/// header that is never closed is skipped, so it cannot become a description.
fn body(text: &str) -> &str {
    let mut lines = text.split_inclusive('\n');
    let Some(first) = lines.next() else { return text };
    if first.trim_end() != "---" {
        return text;
    }
    let mut offset = first.len();
    for line in lines {
        offset += line.len();
        if line.trim_end() == "---" {
            return &text[offset..];
        }
    }
    &text[first.len()..]
}

/// Where the agents keep their files; tests point these at temp dirs.
pub struct Homes {
    pub home: PathBuf,
    pub claude: PathBuf,
}

impl Homes {
    pub fn current() -> Homes {
        let home = dirs::home_dir().unwrap_or_default();
        let claude = home.join(".claude");
        Homes { home, claude }
    }
}

/// `cwd`, then its parents up to and including the first holding `.git`;
/// just `cwd` when none does.
pub fn project_dirs(cwd: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for dir in cwd.ancestors() {
        dirs.push(dir.to_path_buf());
        if dir.join(".git").exists() {
            return dirs;
        }
    }
    vec![cwd.to_path_buf()]
}

/// Whether a name read from a file or folder can be typed after "/": not
/// empty, no space or control character, and no part (between ":" or "/")
/// empty or starting with "." (a hidden file or folder in a cloned repo).
pub fn usable_name(name: &str) -> bool {
    !name.chars().any(|c| c.is_whitespace() || c.is_control())
        && name.split([':', '/']).all(|part| !part.is_empty() && !part.starts_with('.'))
}

/// Keeps the first of each name, in order, so a user's command wins over a
/// built-in of the same name.
pub fn dedupe(list: Vec<CommandInfo>) -> Vec<CommandInfo> {
    let mut seen = HashSet::new();
    list.into_iter().filter(|c| seen.insert(c.name.clone())).collect()
}

/// The built-ins of every agent that has any.
pub fn builtin_lists() -> Vec<AgentCommands> {
    [
        Harness::ClaudeCode,
        Harness::Codex,
        Harness::OpenCode,
        Harness::Kiro,
        Harness::Grok,
        Harness::Antigravity,
    ]
    .into_iter()
    .map(|harness| AgentCommands { harness, commands: builtins(harness) })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_description_keeps_its_first_line_and_at_most_100_chars() {
        assert_eq!(cut_description("\n  Make a commit.\nMore."), "Make a commit.");
        let long = "word ".repeat(40);
        let cut = cut_description(&long);
        assert!(cut.chars().count() <= 100 && cut.ends_with('…') && !cut.contains("wor…"), "{cut}");
    }

    #[test]
    fn front_matter_reads_name_description_and_user_invocable() {
        let fm = front_matter("---\nname: commit\ndescription: \"Create a git commit\"\nuser-invocable: false\n---\nbody");
        assert_eq!(
            (fm.name.as_deref(), fm.description.as_deref(), fm.user_invocable),
            (Some("commit"), Some("Create a git commit"), false)
        );
    }

    #[test]
    fn a_folded_description_is_joined() {
        assert_eq!(
            front_matter("---\ndescription: >\n  Use when\n  starting\n---\n").description.as_deref(),
            Some("Use when starting")
        );
    }

    #[test]
    fn no_or_unclosed_front_matter_reads_as_none() {
        assert_eq!(front_matter("# Title\nbody").description, None);
        assert_eq!(front_matter("---\ndescription: x\nno end").description, None);
        assert_eq!(description_of("\n\nRun the thing\nmore"), "Run the thing");
    }

    #[test]
    fn a_description_falls_back_to_the_body_after_front_matter() {
        assert_eq!(description_of("---\nname: x\n---\n\nDo it\nmore"), "Do it");
    }

    #[test]
    fn an_unclosed_header_is_not_the_description() {
        assert_eq!(description_of("---\nname: x\nDo it"), "name: x");
        assert_eq!(description_of("---  \nname: x\n---\nDo it"), "Do it");
        assert_eq!(front_matter("---  \nname: x\n---\n").name.as_deref(), Some("x"));
    }

    #[test]
    fn a_usable_name_can_be_typed() {
        for ok in ["compact", "team/review", "superpowers:brainstorming", "a.b"] {
            assert!(usable_name(ok), "{ok}");
        }
        for bad in ["", ".hidden", "two words", "tab\there", "bell\u{7}", "ns:.x", "a::b", "x/", "ns: y"] {
            assert!(!usable_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn dedupe_keeps_the_first_of_each_name() {
        let a_user = CommandInfo::new("a", "mine", CommandSource::User);
        let b = CommandInfo::new("b", "", CommandSource::Project);
        let a_builtin = CommandInfo::new("a", "theirs", CommandSource::Builtin);
        assert_eq!(dedupe(vec![a_user.clone(), b.clone(), a_builtin]), vec![a_user, b]);
    }

    #[test]
    fn every_agent_but_other_has_builtins_including_compact() {
        for h in [Harness::ClaudeCode, Harness::Codex, Harness::OpenCode, Harness::Kiro, Harness::Grok, Harness::Antigravity] {
            assert!(builtins(h).iter().any(|c| c.name == "compact" && c.source == CommandSource::Builtin), "{h:?}");
        }
        assert!(builtins(Harness::Other).is_empty());
        assert_eq!(builtin_lists().len(), 6);
    }

    #[test]
    fn an_unknown_source_reads_as_other() {
        let c: CommandInfo = serde_json::from_str(r#"{"name":"x","source":"future"}"#).unwrap();
        assert_eq!((c.source, c.description.as_str()), (CommandSource::Other, ""));
    }
}
