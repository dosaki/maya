//! What the client tells `maya run`: each change is written to the status
//! file `maya status` reads. The core client logs them already.

use crate::status_file::{self, RunStatus};
use maya_core::log;
use maya_core::net::client::{ClientNotify, REMOVED};
use maya_core::now_ms;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct CliNotify {
    pub maya_dir: PathBuf,
    /// `run`'s stop flag, set when the main removes this assistant.
    pub stop: Arc<AtomicBool>,
    /// The main removed this assistant: `run` exits 1, not 0, although `stop` is set.
    pub removed: AtomicBool,
}

impl CliNotify {
    pub fn new(maya_dir: PathBuf, stop: Arc<AtomicBool>) -> Self {
        Self { maya_dir, stop, removed: AtomicBool::new(false) }
    }

    pub fn write(&self, connected: bool, main_name: Option<&str>, error: Option<&str>) {
        let s = RunStatus { pid: std::process::id(), connected, main_name: main_name.map(str::to_string), error: error.map(str::to_string), updated_ms: now_ms() };
        if let Err(e) = status_file::write(&self.maya_dir, &s) {
            log::line("cli", format!("could not write the status file: {e}"));
        }
    }
}

impl ClientNotify for CliNotify {
    fn paired(&self, _main_key: &str) {}

    fn connected(&self, main_name: &str) {
        self.write(true, Some(main_name), None);
    }

    fn disconnected(&self, error: &str) {
        self.write(false, None, Some(error));
    }

    fn removed(&self) {
        self.write(false, None, Some(REMOVED));
        self.removed.store(true, Ordering::SeqCst);
        self.stop.store(true, Ordering::SeqCst);
    }

    fn caps(&self) -> Vec<String> {
        vec![maya_core::net::protocol::CAP_FOLLOW.into(), maya_core::net::protocol::CAP_WORKTREE.into()]
    }

    /// Rewrites the main's host and port in `config.json`; `run` reads it
    /// again for the next connection, which `reconnect` makes at once.
    fn moved(&self, host: &str, port: u16, key: &str) -> Result<(), String> {
        let path = self.maya_dir.join("config.json");
        let mut c = maya_core::config::load(&path);
        c.network.main_host = host.into();
        c.network.main_port = port;
        c.network.main_key = key.into();
        maya_core::config::save(&path, &c).map_err(|e| format!("could not save config.json: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maya_core::config::{self, Config, NetworkRole};

    #[test]
    fn a_move_saves_the_new_host_and_saves_the_new_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut c = Config::default();
        c.network.role = NetworkRole::Assistant;
        c.network.main_host = "10.0.0.2".into();
        c.network.main_key = "old".into();
        config::save(&path, &c).unwrap();
        let n = CliNotify::new(dir.path().to_path_buf(), Arc::new(AtomicBool::new(false)));
        assert_eq!(n.caps(), ["follow", "worktree"]);
        n.moved("10.0.0.9", 5001, "new").unwrap();
        let after = config::load(&path);
        assert_eq!((after.network.main_host.as_str(), after.network.main_port), ("10.0.0.9", 5001));
        assert_eq!(after.network.main_key, "new");
        assert_eq!(after.network.role, NetworkRole::Assistant);
    }
}
