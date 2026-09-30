pub mod actions;
pub mod answer;
pub mod antigravity;
pub mod attachments;
pub mod codex;
pub mod config;
pub mod context;
pub mod events;
pub mod foreign;
pub mod grok;
pub mod hook_install;
pub mod inbox;
pub mod interpreter;
pub mod launch;
pub mod log;
pub mod model;
pub mod net;
pub mod notify;
pub mod pr;
pub mod registry;
pub mod resume;
pub mod reviews;
pub mod state;
pub mod store;
pub mod terminal;
pub mod transcript;
pub mod tty;
pub mod watcher;
#[cfg(windows)]
pub mod win_console;

/// `~/.claude`, or `/.claude` when the home directory is unknown.
pub fn claude_dir() -> std::path::PathBuf {
    dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/")).join(".claude")
}

/// Epoch milliseconds now.
pub fn now_ms() -> u64 {
    store::now_ms()
}
