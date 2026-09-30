//! The CLI's `Executor`, used while pairing (and, from Task 7, while
//! `maya run` keeps a connection to the main open). A minimal stand-in for
//! now: it never runs a command, and its board mirrors the app's
//! `TauriExecutor::board` (`src-tauri/src/net_app.rs`).

use maya_core::model::Card;
use maya_core::net::client::Executor;
use maya_core::net::protocol::CommandKind;
use maya_core::store::Store;
use serde_json::Value;
use std::sync::{Arc, Mutex};

pub struct PairingExecutor {
    pub store: Arc<Mutex<Store>>,
}

impl Executor for PairingExecutor {
    fn execute(&self, _kind: CommandKind) -> Result<Option<Value>, String> {
        Err("not running".into())
    }

    fn board(&self) -> (Vec<Card>, Vec<String>) {
        let (cards, root) = {
            let mut store = self.store.lock().unwrap();
            (store.refresh(maya_core::now_ms()), store.config.projects_dir_path())
        };
        let dirs = root.filter(|r| r.is_dir()).map(|r| maya_core::launch::list_project_dirs(&r)).unwrap_or_default();
        (cards, dirs)
    }
}
