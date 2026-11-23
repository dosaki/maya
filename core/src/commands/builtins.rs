//! The slash commands each agent ships with. No agent lists its own, so these
//! are written down here; a command an agent adds later shows up only once
//! the list is updated.

use super::{CommandInfo, CommandSource};
use crate::model::Harness;

type Table = &'static [(&'static str, &'static str)];

// From https://code.claude.com/docs/en/commands, with the aliases it gives
// (each described as the command it stands for); commands it marks removed
// are left out.
const CLAUDE_CODE: Table = &[
    ("add-dir", "Add a working folder"),
    ("advisor", "Set the advisor model"),
    ("agents", "Manage subagents"),
    ("allowed-tools", "Manage permissions"),
    ("android", "Get the mobile app"),
    ("app", "Continue in the desktop app"),
    ("artifacts", "List your artifacts"),
    ("autocompact", "Set when to compact"),
    ("autofix-pr", "Fix the pull request's CI in the cloud"),
    ("background", "Run this session in the background"),
    ("bashes", "List background tasks"),
    ("batch", "Make a large change in parallel"),
    ("bg", "Run this session in the background"),
    ("branch", "Branch the conversation"),
    ("btw", "Ask a side question"),
    ("bug", "Report a bug"),
    ("cd", "Move the session to another folder"),
    ("checkpoint", "Rewind the conversation"),
    ("checkup", "Check the installation"),
    ("chrome", "Set up Claude in Chrome"),
    ("claude-api", "Claude API reference"),
    ("clear", "Start a new conversation"),
    ("code-review", "Review the current diff"),
    ("color", "Set the prompt bar colour"),
    ("compact", "Clear history, keep a summary"),
    ("config", "Open settings"),
    ("context", "Show context use"),
    ("continue", "Resume a session"),
    ("copy", "Copy the last reply"),
    ("cost", "Show usage"),
    ("debug", "Troubleshoot with the debug log"),
    ("desktop", "Continue in the desktop app"),
    ("diff", "Show changes"),
    ("doctor", "Check the installation"),
    ("effort", "Set reasoning effort"),
    ("exit", "Quit"),
    ("export", "Export the conversation"),
    ("fast", "Toggle fast mode"),
    ("feedback", "Send feedback"),
    ("fewer-permission-prompts", "Allow common read-only tools"),
    ("focus", "Toggle the focus view"),
    ("fork", "Fork the conversation"),
    ("goal", "Set a goal"),
    ("help", "Show help"),
    ("hooks", "Manage hooks"),
    ("ide", "Manage IDE integrations"),
    ("import", "Import another agent's setup"),
    ("init", "Create CLAUDE.md"),
    ("insights", "Report on your sessions"),
    ("install-github-app", "Set up the GitHub app"),
    ("install-slack-app", "Install the Slack app"),
    ("ios", "Get the mobile app"),
    ("keybindings", "Edit keyboard shortcuts"),
    ("list-agents", "List the agents Claude can message"),
    ("login", "Sign in"),
    ("logout", "Sign out"),
    ("loop", "Run a prompt on an interval"),
    ("mcp", "Manage MCP servers"),
    ("memory", "Edit memory"),
    ("mobile", "Get the mobile app"),
    ("model", "Choose the model"),
    ("new", "Start a new conversation"),
    ("output-style", "Set the output style"),
    ("permissions", "Manage permissions"),
    ("plan", "Plan mode"),
    ("plugin", "Manage plugins"),
    ("powerup", "Learn features"),
    ("privacy-settings", "Privacy settings"),
    ("proactive", "Run a prompt on an interval"),
    ("quit", "Quit"),
    ("rc", "Allow Remote Control"),
    ("recap", "Summarise the session"),
    ("release-notes", "Show release notes"),
    ("reload-plugins", "Reload plugins"),
    ("reload-skills", "Reload skills"),
    ("remote-control", "Allow Remote Control"),
    ("remote-env", "Choose the cloud environment"),
    ("rename", "Rename the session"),
    ("reset", "Start a new conversation"),
    ("restart", "Restart and resume"),
    ("resume", "Resume a session"),
    ("review", "Review the current diff"),
    ("rewind", "Rewind the conversation"),
    ("routines", "Schedule a cloud agent"),
    ("run", "Run the app to check a change"),
    ("sandbox", "Toggle sandbox mode"),
    ("schedule", "Schedule a cloud agent"),
    ("scroll-speed", "Set the scroll speed"),
    ("security-review", "Review pending changes for security"),
    ("settings", "Open settings"),
    ("share", "Report a bug"),
    ("simplify", "Clean up changed code"),
    ("skill-doctor", "Show what each skill costs"),
    ("skills", "List skills"),
    ("stats", "Show usage stats"),
    ("status", "Show status"),
    ("statusline", "Set up the status line"),
    ("stop", "Stop"),
    ("subtask", "Run a task in a forked subagent"),
    ("tasks", "List background tasks"),
    ("teleport", "Pull a cloud session here"),
    ("terminal-setup", "Set up Shift+Enter for new lines"),
    ("theme", "Change the theme"),
    ("tui", "Choose the screen mode"),
    ("ultrareview", "Deep code review in the cloud"),
    ("undo", "Rewind the conversation"),
    ("update", "Restart and resume"),
    ("update-config", "Change settings"),
    ("upgrade", "Upgrade your plan"),
    ("usage", "Show usage and limits"),
    ("verify", "Check a change in the running app"),
    ("voice", "Voice input"),
    ("web-setup", "Connect GitHub for cloud sessions"),
    ("workflows", "Manage workflows"),
];

const CODEX: Table = &[
    ("model", "Choose model and effort"),
    ("permissions", ""),
    ("skills", "Pick a skill"),
    ("review", "Review changes"),
    ("rename", ""),
    ("new", "New session"),
    ("resume", ""),
    ("fork", ""),
    ("init", "Create AGENTS.md"),
    ("compact", "Summarise the conversation"),
    ("diff", ""),
    ("mention", "Mention a file"),
    ("status", ""),
    ("usage", ""),
    ("mcp", ""),
    ("copy", ""),
    ("export", ""),
    ("logout", ""),
    ("quit", ""),
    ("clear", ""),
    ("agents", ""),
    ("stop", ""),
];

const OPEN_CODE: Table = &[
    ("agents", "Switch agent"),
    ("clear", "Clear session"),
    ("compact", "Compact session"),
    ("copy", ""),
    ("diff", ""),
    ("editor", "Open editor"),
    ("exit", ""),
    ("export", ""),
    ("fork", ""),
    ("help", ""),
    ("models", "Switch model"),
    ("new", "New session"),
    ("redo", ""),
    ("rename", ""),
    ("sessions", "Resume a session"),
    ("skills", ""),
    ("stats", "Usage statistics"),
    ("themes", ""),
    ("undo", ""),
    ("variants", "Switch model variant"),
];

const KIRO: Table = &[
    ("help", ""),
    ("quit", ""),
    ("clear", ""),
    ("compact", ""),
    ("chat", ""),
    ("context", ""),
    ("model", ""),
    ("effort", ""),
    ("agent", ""),
    ("plan", ""),
    ("tools", ""),
    ("permissions", ""),
    ("prompts", ""),
    ("mcp", ""),
    ("hooks", ""),
    ("usage", ""),
    ("editor", ""),
    ("copy", ""),
    ("rewind", ""),
    ("todos", ""),
];

const GROK: Table = &[
    ("new", "New conversation"),
    ("resume", ""),
    ("compact", ""),
    ("context", ""),
    ("fork", ""),
    ("rewind", ""),
    ("copy", ""),
    ("export", ""),
    ("quit", ""),
    ("rename", ""),
    ("model", ""),
    ("effort", ""),
    ("plan", ""),
    ("memory", ""),
    ("hooks", ""),
    ("plugins", ""),
    ("skills", ""),
    ("loop", ""),
    ("goal", ""),
    ("usage", ""),
    ("settings", ""),
    ("doctor", ""),
];

const ANTIGRAVITY: Table = &[
    ("clear", "New conversation"),
    ("resume", ""),
    ("fork", ""),
    ("rewind", ""),
    ("rename", ""),
    ("exit", ""),
    ("model", ""),
    ("effort", ""),
    ("config", ""),
    ("permissions", ""),
    ("plan", ""),
    ("goal", ""),
    ("agents", ""),
    ("mcp", ""),
    ("plugin", ""),
    ("skills", ""),
    ("hooks", ""),
    ("tasks", ""),
    ("diff", ""),
    ("context", ""),
    ("copy", ""),
    ("usage", ""),
    ("compact", ""),
    ("help", ""),
];

/// The built-in commands of one agent; empty for `Other`, which has none we know.
pub fn builtins(h: Harness) -> Vec<CommandInfo> {
    let table: Table = match h {
        Harness::ClaudeCode => CLAUDE_CODE,
        Harness::Codex => CODEX,
        Harness::OpenCode => OPEN_CODE,
        Harness::Kiro => KIRO,
        Harness::Grok => GROK,
        Harness::Antigravity => ANTIGRAVITY,
        Harness::Other => &[],
    };
    table
        .iter()
        .map(|(name, description)| CommandInfo::new(name, description, CommandSource::Builtin))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(h: Harness) -> Vec<String> {
        builtins(h).into_iter().map(|c| c.name).collect()
    }

    #[test]
    fn claude_code_lists_its_commands_and_their_aliases() {
        let names = names(Harness::ClaudeCode);
        for want in [
            "review", "new", "reset", "quit", "settings", "login", "logout", "ide", "btw", "cd", "branch", "sandbox",
            "terminal-setup", "upgrade", "continue", "undo", "checkpoint", "share", "bashes",
        ] {
            assert!(names.iter().any(|n| n == want), "{want} is missing");
        }
        // Removed from Claude Code.
        for gone in ["pr-comments", "vim", "ultraplan"] {
            assert!(!names.iter().any(|n| n == gone), "{gone} was removed");
        }
    }

    #[test]
    fn no_table_names_a_command_twice() {
        for h in [Harness::ClaudeCode, Harness::Codex, Harness::OpenCode, Harness::Kiro, Harness::Grok, Harness::Antigravity] {
            let names = names(h);
            let unique: std::collections::HashSet<_> = names.iter().collect();
            assert_eq!(unique.len(), names.len(), "{h:?}");
        }
    }
}
