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
/// hit is remembered, so a `gh` installed later is still found.
pub fn gh() -> std::process::Command {
    static FOUND: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    if let Some(p) = FOUND.get() {
        return command(p);
    }
    match launch::agent_binary("gh") {
        Some(p) => command(FOUND.get_or_init(|| p)),
        None => command("gh"),
    }
}

/// Epoch milliseconds now.
pub fn now_ms() -> u64 {
    store::now_ms()
}
