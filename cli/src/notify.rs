//! What the client tells `maya run`: each change is logged and written to
//! the status file `maya status` reads.

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
    fn paired(&self, _id: &str, _token: &str) {}

    fn connected(&self, main_name: &str) {
        log::line("cli", format!("connected to {main_name}"));
        self.write(true, Some(main_name), None);
    }

    fn disconnected(&self, error: &str) {
        log::line("cli", format!("disconnected: {error}"));
        self.write(false, None, Some(error));
    }

    fn removed(&self) {
        log::line("cli", "removed by the main Maya");
        self.write(false, None, Some(REMOVED));
        self.removed.store(true, Ordering::SeqCst);
        self.stop.store(true, Ordering::SeqCst);
    }
}
