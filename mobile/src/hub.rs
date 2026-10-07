//! The phone's main, with the page and the Android service behind traits:
//! the config, the server, the notifier diff and the alert state. The Tauri
//! layer (`lib.rs`, `commands.rs`) is thin on purpose, so this is what the
//! integration test drives.

use crate::alerts::{self, Action, Alerts, Posted, Switches};
use maya_core::config::{self, Config, NetworkRole, PairedAssistant};
use maya_core::log;
use maya_core::model::Card;
use maya_core::net::merge::{self, RemoteBoard};
use maya_core::net::protocol::CommandKind;
use maya_core::net::routing::ROUTE_TIMEOUT;
use maya_core::net::server::{send_command_with, start_with, Notify, ServerHandle};
use maya_core::net::{self, NetworkStatus};
use maya_core::notify::Notifier;
use maya_core::store::now_ms;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

/// What the hub tells the outside: the page's events and the service.
pub trait Sink: Send + Sync {
    fn sessions(&self, cards: &[Card]);
    fn network(&self, status: &NetworkStatus);
    fn service_start(&self, line: &str);
    fn service_stop(&self);
}

/// The config and where it is saved.
pub struct Settings {
    pub path: PathBuf,
    pub config: Config,
}

/// Where the config lives under the app's data directory.
pub fn settings_path(maya_dir: &Path) -> PathBuf {
    maya_dir.join("config.json")
}

/// Loads the config, filling a blank name with the device's so the main's
/// `welcome` never says "localhost".
pub fn load_settings(maya_dir: &Path, model: &str) -> Settings {
    let path = settings_path(maya_dir);
    let mut config = config::load(&path);
    if config.network.name.trim().is_empty() {
        config.network.name = alerts::default_name(model);
    }
    Settings { path, config }
}

pub struct Hub {
    /// `<app data dir>/maya`: the config, the log and saved attachments.
    pub maya_dir: PathBuf,
    /// Lock order: `server`, then the server's own mutex, then `settings`;
    /// `notifier` and `posted` are taken alone.
    pub settings: Mutex<Settings>,
    pub server: Mutex<Option<ServerHandle>>,
    /// Why the server is not running (the port is taken…); `None` once it runs.
    pub main_error: Mutex<Option<String>>,
    pub notifier: Mutex<Notifier>,
    pub posted: Mutex<Posted>,
    pub alerts: Arc<dyn Alerts>,
    pub sink: Arc<dyn Sink>,
}

impl Hub {
    pub fn new(maya_dir: PathBuf, settings: Settings, alerts: Arc<dyn Alerts>, sink: Arc<dyn Sink>) -> Arc<Hub> {
        Arc::new(Hub { maya_dir, settings: Mutex::new(settings), server: Mutex::new(None), main_error: Mutex::new(None), notifier: Mutex::new(Notifier::default()), posted: Mutex::new(Posted::default()), alerts, sink })
    }

    fn handle(&self) -> Option<ServerHandle> {
        self.server.lock().unwrap().clone()
    }

    /// The assistants' last snapshots; none while the server is down.
    pub fn boards(&self) -> Vec<RemoteBoard> {
        self.handle().map(|s| s.boards()).unwrap_or_default()
    }

    /// The board: every connected assistant's cards, no local ones.
    pub fn cards(&self) -> Vec<Card> {
        merge::merged(vec![], &self.boards(), now_ms())
    }

    /// Repaints the page and posts or clears notifications for what changed.
    pub fn refresh(&self) {
        let cards = self.cards();
        let (fresh, finished) = {
            let mut n = self.notifier.lock().unwrap();
            (n.take_new(&cards), n.take_finished(&cards))
        };
        let switches = {
            let s = self.settings.lock().unwrap();
            Switches { awaiting: s.config.notify_on_awaiting, completed: s.config.notify_on_completed }
        };
        let actions = alerts::plan(&cards, &fresh, &finished, &mut self.posted.lock().unwrap(), switches);
        for a in actions {
            match a {
                Action::Post(p) => self.alerts.post(&p),
                Action::Clear(id) => self.alerts.clear(id),
            }
        }
        self.sink.sessions(&cards);
    }

    /// The status as the Network screen shows it: the server's, or the
    /// paired list offline with why the server is down.
    pub fn status(&self) -> NetworkStatus {
        match self.handle() {
            Some(s) => s.status(),
            None => {
                let paired = self.settings.lock().unwrap().config.network.assistants.clone();
                NetworkStatus { role: NetworkRole::Off, code: None, assistants: net::paired_offline(&paired), main_error: self.main_error.lock().unwrap().clone(), ..Default::default() }
            }
        }
    }

    /// Changes the config with `f` and saves it; the config in memory
    /// changes only once the save succeeded.
    pub fn update_config(&self, f: impl FnOnce(&mut Config)) -> Result<Config, String> {
        let mut s = self.settings.lock().unwrap();
        let mut next = s.config.clone();
        f(&mut next);
        config::save(&s.path, &next).map_err(|e| format!("Could not save the settings: {e}"))?;
        s.config = next.clone();
        Ok(next)
    }

    fn save_assistants(&self, f: impl FnOnce(&mut Vec<PairedAssistant>)) -> Result<(), String> {
        self.update_config(|c| f(&mut c.network.assistants)).map(|_| ()).map_err(|e| {
            log::line("network", format!("could not save the paired assistants: {e}"));
            format!("Could not save the paired assistants: {e}")
        })
    }

    /// Starts the server on the configured port, replacing a running one,
    /// and the foreground service with it. When the port cannot be bound
    /// the error is kept for the Network screen and no service starts.
    pub fn start_server(self: &Arc<Self>) -> Result<(), String> {
        self.stop_server();
        let (port, view) = {
            let s = self.settings.lock().unwrap();
            (s.config.listen_port(), s.config.network.clone())
        };
        let notify: Arc<dyn Notify> = Arc::new(HubNotify(Arc::downgrade(self)));
        let mut started = start_with(notify.clone(), Arc::new(Mutex::new(view.clone())), port);
        for _ in 0..5 {
            if started.is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(150));
            started = start_with(notify.clone(), Arc::new(Mutex::new(view.clone())), port);
        }
        match started {
            Ok(handle) => {
                let status = handle.status();
                *self.server.lock().unwrap() = Some(handle);
                *self.main_error.lock().unwrap() = None;
                log::line("network", format!("server listening on port {port}"));
                self.sink.service_start(&alerts::service_line(status.assistants.len(), 0));
                self.sink.network(&status);
                Ok(())
            }
            Err(e) => {
                *self.main_error.lock().unwrap() = Some(e.clone());
                log::line("network", format!("server not started: {e}"));
                self.sink.network(&self.status());
                Err(e)
            }
        }
    }

    /// Stops the server and the service; the board empties.
    pub fn stop_server(&self) {
        let running = self.server.lock().unwrap().take();
        if let Some(s) = running {
            s.stop();
            self.sink.service_stop();
        }
        self.refresh();
        self.sink.network(&self.status());
    }

    /// Opens pairing, or regenerates the code when it is already open.
    pub fn pairing_code(&self) -> Result<NetworkStatus, String> {
        let Some(server) = self.handle() else {
            return Err(self.main_error.lock().unwrap().clone().unwrap_or_else(|| "Start the server first.".into()));
        };
        server.open_pairing(now_ms());
        Ok(server.status())
    }

    /// Forgets a paired assistant; a connected one is told and closed.
    pub fn remove_assistant(&self, id: &str) -> Result<NetworkStatus, String> {
        match self.handle() {
            Some(s) => s.remove_assistant(id)?,
            None => {
                self.save_assistants(|list| list.retain(|a| a.id != id))?;
                log::line("network", format!("{id}: removed"));
            }
        }
        Ok(self.status())
    }

    /// Sends a session command to the machine that lists the session. There
    /// are no local sessions: a session nobody lists is no longer running.
    pub fn send(&self, session_id: &str, kind: impl FnOnce(String) -> CommandKind) -> Result<Option<Value>, String> {
        let machine = merge::route(&[], &self.boards(), session_id, now_ms()).ok_or("Session is no longer running.")?;
        self.send_to(&machine, kind(session_id.to_string()))
    }

    /// Sends a command to a machine by its label (start, resume, listings).
    pub fn send_to(&self, machine: &str, kind: CommandKind) -> Result<Option<Value>, String> {
        let shared = self.handle().map(|s| s.shared.clone()).ok_or("The server is stopped.")?;
        send_command_with(&shared, machine, kind, ROUTE_TIMEOUT)
    }
}

/// The server's line to the hub. `paired` and `paired_list_changed` run
/// with the server's mutex held and take only `settings`; the others run
/// on the server's threads with no lock held.
struct HubNotify(Weak<Hub>);

impl HubNotify {
    fn with(&self, f: impl FnOnce(&Hub)) {
        if let Some(hub) = self.0.upgrade() {
            f(&hub);
        }
    }
}

impl Notify for HubNotify {
    fn board_changed(&self) {
        self.with(|h| h.refresh());
    }

    fn board_seeded(&self, cards: &[Card]) {
        self.with(|h| h.notifier.lock().unwrap().seed(cards));
    }

    fn status_changed(&self, status: NetworkStatus) {
        self.with(|h| {
            let connected = status.assistants.iter().filter(|a| a.connected).count();
            h.alerts.service_line(&alerts::service_line(status.assistants.len(), connected));
            h.sink.network(&status);
        });
    }

    fn paired(&self, assistant: &PairedAssistant) -> Result<(), String> {
        let Some(h) = self.0.upgrade() else { return Err("the app is gone".into()) };
        h.save_assistants(|list| match list.iter_mut().find(|a| a.id == assistant.id) {
            Some(a) => *a = assistant.clone(),
            None => list.push(assistant.clone()),
        })
    }

    fn paired_list_changed(&self, assistants: &[PairedAssistant]) -> Result<(), String> {
        let Some(h) = self.0.upgrade() else { return Err("the app is gone".into()) };
        h.save_assistants(|list| *list = assistants.to_vec())
    }
}
