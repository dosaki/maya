//! The phone's side of a joined main, with the page and the Android service
//! behind traits: the config, the client, the notifier diff and the alert
//! state. The Tauri layer (`lib.rs`, `commands.rs`) is thin on purpose, so
//! this is what the integration test drives.

use crate::alerts::{self, Action, Alerts, Posted, Switches};
use maya_core::config::{self, Config, NetworkRole};
use maya_core::log;
use maya_core::model::Card;
use maya_core::net::identity::{self, Keypair};
use maya_core::net::client::{check_pair_args, pair_with, reconnect, ClientHandle, ClientNotify, Executor, RequestLane, NOT_PAIRED, REMOVED, REQUEST_TIMEOUT};
use maya_core::net::protocol::{CommandKind, Request, CAP_CHAT, CAP_FOLLOW, CAP_MIRROR, DEFAULT_PORT};
use maya_core::net::routing::MachineInfo;
use maya_core::remembered::{self, RememberedRow, ResumeOutcome};
use maya_core::net::{AssistantLink, MirrorAccess, NetworkStatus};
use maya_core::notify::Notifier;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

/// What the hub tells the outside: the page's events and the service.
pub trait Sink: Send + Sync {
    fn sessions(&self, cards: &[Card]);
    /// The "was running" list for the banner.
    fn remembered(&self, _rows: &[RememberedRow]) {}
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
/// `welcome` never says "localhost". A phone saved as a main (before 0.35)
/// loads as Off with no paired assistants, and that is saved: it now only
/// joins a main.
pub fn load_settings(maya_dir: &Path, model: &str) -> Settings {
    let path = settings_path(maya_dir);
    let mut config = config::load(&path);
    if config.network.role == NetworkRole::Main {
        config.network.role = NetworkRole::Off;
        config.network.assistants.clear();
        if let Err(e) = config::save(&path, &config) {
            log::line("network", format!("could not save the settings: {e}"));
        }
        log::line("network", "this phone is no longer a main: join a main computer");
    }
    if config.network.name.trim().is_empty() {
        config.network.name = alerts::default_name(model);
    }
    Settings { path, config }
}

/// The main's board while this phone has joined it.
#[derive(Default, Clone)]
pub struct MirrorView {
    pub cards: Vec<Card>,
    pub machines: Vec<MachineInfo>,
    pub remembered: Vec<RememberedRow>,
}

/// What this phone says it can do: follow a moved main, mirror the main's
/// board, and show its Chat.
fn phone_caps() -> Vec<String> {
    vec![CAP_FOLLOW.into(), CAP_MIRROR.into(), CAP_CHAT.into()]
}

/// The main's name while the link has not said it yet.
const SOME_MAIN: &str = "the main Maya";

pub struct Hub {
    /// `<app data dir>/maya`: the config, the log and saved attachments.
    pub maya_dir: PathBuf,
    /// Serializes `refresh`, so a stale snapshot never overtakes a newer one.
    /// Lock order: `refresh` first, alone (its callers hold no other lock);
    /// `settings`, `notifier`, `posted`, `client`, `client_error`, `link`
    /// and `mirror` are taken alone, never while holding another hub lock. `stop_client`
    /// joins the client thread, which may be in a callback that calls
    /// `refresh`, so it holds no hub lock while it joins. `lane` has its own
    /// lock, never held here.
    refresh: Mutex<()>,
    pub settings: Mutex<Settings>,
    /// The client that listens to the main.
    pub client: Mutex<Option<ClientHandle>>,
    /// Why the client could not start (the key file cannot be read…);
    /// `None` once it runs. The Off status carries it to the page.
    client_error: Mutex<Option<String>>,
    /// How that client stands with the main.
    pub link: Mutex<AssistantLink>,
    /// The joined main's last board; empty otherwise.
    pub mirror: Mutex<MirrorView>,
    /// Where the page's commands go while joined.
    pub lane: Arc<RequestLane>,
    /// This phone's key pair, read from `identity.key` once and kept.
    identity: Mutex<Option<Arc<Keypair>>>,
    /// The first mirror of the current link was seeded into the notifier.
    /// A reconnect keeps it, so an ask that began while away still notifies.
    mirror_seeded: AtomicBool,
    pub notifier: Mutex<Notifier>,
    pub posted: Mutex<Posted>,
    pub alerts: Arc<dyn Alerts>,
    pub sink: Arc<dyn Sink>,
    /// The phone's voice conversation, once the app has made it. Taken
    /// alone: `refresh` calls into it with only its own serial lock held,
    /// and it never calls back into a hub lock while it holds its own (its
    /// event gate, `events_may_speak`, runs with the voice's lock released).
    pub voice: OnceLock<Arc<crate::voice::Voice>>,
    /// On silent, vibrate or Do Not Disturb (the voice plugin's `quiet`).
    pub silenced: OnceLock<Box<dyn Fn() -> bool + Send + Sync>>,
    /// What `joined` answers in a test, so it needs no running client.
    #[cfg(test)]
    joined_override: Mutex<Option<bool>>,
}

/// Whether board events may be spoken at all: Speak is on and the phone is
/// not on silent, vibrate or Do Not Disturb. Replies are always spoken.
/// `silenced` is a call into Android, made only when Speak is on.
pub(crate) fn speaks_events(speak_on: bool, silenced: impl FnOnce() -> bool) -> bool {
    speak_on && !silenced()
}

impl Hub {
    pub fn new(maya_dir: PathBuf, settings: Settings, alerts: Arc<dyn Alerts>, sink: Arc<dyn Sink>) -> Arc<Hub> {
        Arc::new(Hub {
            maya_dir, settings: Mutex::new(settings), client: Mutex::new(None), client_error: Mutex::new(None), link: Mutex::new(AssistantLink::default()), refresh: Mutex::new(()), mirror: Mutex::new(MirrorView::default()), lane: Arc::new(RequestLane::default()), identity: Mutex::new(None), mirror_seeded: AtomicBool::new(false), notifier: Mutex::new(Notifier::default()), posted: Mutex::new(Posted::default()), alerts, sink, voice: OnceLock::new(), silenced: OnceLock::new(),
            #[cfg(test)]
            joined_override: Mutex::new(None),
        })
    }

    /// The board: the joined main's mirror, greyed while the link is down.
    /// Empty until this phone joins a main; it never has cards of its own.
    pub fn cards(&self) -> Vec<Card> {
        let connected = self.link.lock().unwrap().connected;
        let mut cards = self.mirror.lock().unwrap().cards.clone();
        for c in &mut cards {
            c.stale |= !connected;
        }
        cards
    }

    /// The "was running" list the banner shows: the joined main's.
    pub fn remembered(&self) -> Vec<RememberedRow> {
        self.mirror.lock().unwrap().remembered.clone()
    }

    /// Resumes `rows`, each on its own machine, one at a time,
    /// `RESUME_GAP_MS` apart; a waiting machine's rows are skipped.
    pub fn resume_remembered(&self, rows: &[RememberedRow]) -> Vec<ResumeOutcome> {
        let plan = remembered::plan(rows);
        let mut out = Vec::new();
        for e in &plan.here {
            out.push(ResumeOutcome::of(None, e, &Err("This phone runs no sessions.".into())));
        }
        for (machine, entries) in &plan.remote {
            for (i, e) in entries.iter().enumerate() {
                if i > 0 {
                    std::thread::sleep(Duration::from_millis(remembered::RESUME_GAP_MS));
                }
                let kind = CommandKind::Resume { dir: e.dir.clone(), session: e.session_id.clone(), agent: e.agent };
                out.push(ResumeOutcome::of(Some(machine), e, &self.send_to(machine, kind).map(|_| ())));
            }
        }
        for w in &plan.waiting {
            let machine = w.machine.clone().unwrap_or_default();
            out.push(ResumeOutcome::of(Some(&machine), &w.entry, &Err(remembered::waiting_for(&machine))));
        }
        self.refresh();
        out
    }

    /// Forgets `rows` on their machines. The banner updates first; the
    /// `Forget` commands follow, every group through the main, a waiting
    /// machine's too: the main drops what it holds for that machine, or its
    /// next mirror would list it again. An older assistant that never
    /// answers one is logged and ignored.
    pub fn forget_remembered(&self, rows: &[RememberedRow]) {
        let groups: Vec<_> = remembered::keys_by_machine(rows).into_iter().filter_map(|(m, _, k)| m.map(|m| (m, k))).collect();
        // The main's next mirror says so too; until then do not list them again.
        self.mirror.lock().unwrap().remembered.retain(|r| !rows.iter().any(|f| f.machine == r.machine && f.entry.key() == r.entry.key()));
        self.refresh();
        for (machine, keys) in groups {
            if let Err(e) = self.send_to(&machine, CommandKind::Forget { sessions: keys }) {
                log::line("remembered", format!("{machine} did not forget its sessions: {e}"));
            }
        }
    }

    /// This phone listens to a main.
    pub fn joined(&self) -> bool {
        #[cfg(test)]
        if let Some(joined) = *self.joined_override.lock().unwrap() {
            return joined;
        }
        self.client.lock().unwrap().is_some()
    }

    /// Sets the link and what `joined` answers, with no client running.
    #[cfg(test)]
    pub(crate) fn set_link_for_test(&self, link: AssistantLink, joined: bool) {
        *self.link.lock().unwrap() = link;
        *self.joined_override.lock().unwrap() = Some(joined);
    }

    /// Why voice is not offered, or `None` when it is: the phone must have
    /// joined a main that interprets (`CAP_VOICE`) and lets it control
    /// sessions, and be connected to it now. Withdrawn on a disconnect, it
    /// resets the conversation; `HubClientNotify` refreshes on every link
    /// change, so it comes back with the link.
    pub fn voice_offer(&self) -> Option<String> {
        if !self.joined() {
            return Some(crate::voice::NO_MAIN.into());
        }
        let link = self.link.lock().unwrap();
        let main = link.main_name.clone().unwrap_or_else(|| SOME_MAIN.into());
        if !link.connected {
            return Some(format!("Reconnecting to {main}…"));
        }
        if link.mirror != Some(MirrorAccess::Allowed) {
            return Some(format!("{main} has not allowed this phone to control its sessions."));
        }
        if !link.voice {
            return Some(format!("Update {main} to talk to Maya."));
        }
        None
    }

    /// `speaks_events` now, for a board event that waited behind a
    /// conversation: the voice asks this as it is about to speak it, with
    /// none of its locks held. Takes `settings` alone and briefly; the
    /// `silenced` call into Android runs after it is released.
    pub fn events_may_speak(&self) -> bool {
        let speak_on = self.settings.lock().unwrap().config.speak_notifications;
        speaks_events(speak_on, || self.silenced.get().is_some_and(|f| f()))
    }

    /// Sends a request to the joined main and waits for its answer.
    pub fn request(&self, machine: Option<&str>, request: Request) -> Result<Option<Value>, String> {
        {
            let link = self.link.lock().unwrap();
            if link.mirror == Some(MirrorAccess::Unsupported) {
                let main = link.main_name.clone().unwrap_or_else(|| "The main Maya".into());
                return Err(format!("{main} runs an older Maya that cannot share its sessions. Update it."));
            }
        }
        self.lane.request(machine.map(str::to_string), request, REQUEST_TIMEOUT)
    }

    /// The session whose notification has this id, if its card is on the board.
    pub fn session_for_notification(&self, id: i32) -> Option<String> {
        self.cards().into_iter().map(|c| c.session_id).find(|s| alerts::notification_id(s) == id)
    }

    /// `session_for_notification`, waiting up to `timeout` for the card.
    pub fn wait_for_notification(&self, id: i32, timeout: Duration) -> Option<String> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Some(s) = self.session_for_notification(id) {
                return Some(s);
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    /// Repaints the page and posts or clears notifications for what changed.
    /// Runs on every new mirror and every change of the link.
    pub fn refresh(&self) {
        let _serial = self.refresh.lock().unwrap();
        let cards = self.cards();
        let (fresh, finished) = {
            let mut n = self.notifier.lock().unwrap();
            (n.take_new(&cards), n.take_finished(&cards))
        };
        let (switches, speak_on) = {
            let s = self.settings.lock().unwrap();
            (Switches { awaiting: s.config.notify_on_awaiting, completed: s.config.notify_on_completed }, s.config.speak_notifications)
        };
        let actions = alerts::plan(&cards, &fresh, &finished, &mut self.posted.lock().unwrap(), switches);
        // First, so an event arriving as voice comes or goes sees the new
        // offer. No hub lock but the serial one is held here: the voice's
        // callbacks (the page event, the plugin queue) take none.
        if let Some(v) = self.voice.get() {
            v.set_offer(self.voice_offer());
        }
        // Asked only when there is something to post: it is a call into Android.
        let mut events_spoken = None;
        for a in actions {
            match a {
                Action::Post(mut p) => {
                    let may_speak = *events_spoken.get_or_insert_with(|| speaks_events(speak_on, || self.silenced.get().is_some_and(|f| f())));
                    if let (true, Some(v), Some(line)) = (may_speak, self.voice.get(), p.spoken.as_deref()) {
                        // Spoken (now or once the conversation is idle): post without a sound.
                        p.quiet = v.announce(line);
                    }
                    self.alerts.post(&p)
                }
                Action::Clear(id) => self.alerts.clear(id),
            }
        }
        self.sink.sessions(&cards);
        self.sink.remembered(&self.remembered());
    }

    /// The status as the Network screen shows it: the link while this phone
    /// listens to a main, Off otherwise, with why the client could not start.
    pub fn status(&self) -> NetworkStatus {
        if self.client.lock().unwrap().is_some() {
            let assistant = self.link.lock().unwrap().clone();
            return self.with_pair_again(NetworkStatus { role: NetworkRole::Assistant, assistant, ..Default::default() });
        }
        let main_error = self.client_error.lock().unwrap().clone();
        self.with_pair_again(NetworkStatus { role: NetworkRole::Off, main_error, ..Default::default() })
    }

    /// The core never fills `pair_again`; it is the config's, so every
    /// status that reaches the page goes through here.
    fn with_pair_again(&self, mut status: NetworkStatus) -> NetworkStatus {
        status.pair_again = self.settings.lock().unwrap().config.network.pair_again;
        status
    }

    /// This phone's key pair, from `identity.key` beside its config.
    pub fn identity(&self) -> Result<Arc<Keypair>, String> {
        let mut held = self.identity.lock().unwrap();
        if let Some(k) = held.as_ref() {
            return Ok(k.clone());
        }
        let k = Arc::new(identity::load_or_create(&self.maya_dir.join(identity::FILE))?);
        *held = Some(k.clone());
        Ok(k)
    }

    /// Drops every pairing and makes a fresh key: the link stops and the
    /// role goes Off. Nothing stops or changes unless both the config and
    /// the key file are written.
    pub fn new_key(&self) -> Result<NetworkStatus, String> {
        let before = self.settings.lock().unwrap().config.clone();
        self.update_config(|c| {
            c.network.role = NetworkRole::Off;
            c.network.assistants.clear();
            c.network.main_key.clear();
            c.network.main_host.clear();
            c.network.main_port = 0;
            c.network.pair_again = false;
        })?;
        let mut held = self.identity.lock().unwrap();
        match identity::replace(&self.maya_dir.join(identity::FILE)) {
            Ok(k) => *held = Some(Arc::new(k)),
            Err(e) => {
                drop(held);
                let _ = self.update_config(|c| *c = before);
                return Err(e);
            }
        }
        drop(held);
        self.stop_client();
        let status = self.status();
        self.sink.network(&status);
        Ok(status)
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

    /// Listens to the main in the config and mirrors its board; the keep-awake
    /// service runs meanwhile.
    pub fn start_client(self: &Arc<Self>) {
        // The old client and the last link's state go before the new client
        // starts, so its first mirror is never wiped behind it. A new link
        // seeds the notifier afresh and shows none of the last one's cards.
        let old = self.client.lock().unwrap().take();
        if let Some(old) = old {
            old.stop();
        }
        *self.link.lock().unwrap() = AssistantLink { retrying: true, ..Default::default() };
        self.mirror_seeded.store(false, Ordering::SeqCst);
        *self.mirror.lock().unwrap() = MirrorView::default();
        let key = match self.identity() {
            Ok(k) => k,
            Err(e) => {
                log::line("network", format!("not listening to the main: {e}"));
                *self.client_error.lock().unwrap() = Some(e);
                self.sink.network(&self.status());
                return;
            }
        };
        let weak = Arc::downgrade(self);
        let handle = ClientHandle::spawn(move |stop| {
            let notify: Arc<dyn ClientNotify> = Arc::new(HubClientNotify { hub: weak.clone(), stop: stop.clone() });
            let config_of = move || weak.upgrade().map(|h| h.settings.lock().unwrap().config.network.clone()).unwrap_or_default();
            reconnect(&config_of, key, Arc::new(NoSessions), notify, &stop);
        });
        *self.client.lock().unwrap() = Some(handle);
        *self.client_error.lock().unwrap() = None;
        // A mirror that landed before the handle was in shows now.
        self.refresh();
        self.sink.service_start(&alerts::service_line(SOME_MAIN, false));
        self.sink.network(&self.status());
    }

    /// Stops listening: the role goes Off and the link is forgotten.
    pub fn stop_listening(&self) -> Result<NetworkStatus, String> {
        // Saved first: if the save fails, the phone keeps listening as before.
        self.update_config(|c| {
            c.network.role = NetworkRole::Off;
            c.network.main_host.clear();
            c.network.main_port = 0;
            c.network.main_key.clear();
        })?;
        self.stop_client();
        let status = self.status();
        self.sink.network(&status);
        Ok(status)
    }

    /// Pairs with a running main and listens to it. A phone that was joined
    /// to another main listens to that one again if the pairing fails.
    pub fn join(self: &Arc<Self>, host: &str, port: u16, name: &str, code: &str) -> Result<NetworkStatus, String> {
        let (host, code) = check_pair_args(host, code)?;
        let port = if port == 0 { DEFAULT_PORT } else { port };
        let name = match name.trim() {
            "" => self.settings.lock().unwrap().config.network.name.clone(),
            n => n.to_string(),
        };
        let identity = self.identity()?;
        let was_joined = self.joined();
        self.stop_client();
        let paired = pair_with(Arc::new(NoSessions), identity, &host, port, &name, &code, phone_caps()).and_then(|(main, main_key)| {
            self.update_config(|c| {
                c.network.role = NetworkRole::Assistant;
                c.network.main_host = host.clone();
                c.network.main_port = port;
                c.network.name = name.clone();
                c.network.main_key = main_key;
                c.network.pair_again = false;
            })
            .map(|_| main)
        });
        match paired {
            Ok(main) => {
                log::line("network", format!("joined {main}"));
                self.start_client();
                Ok(self.status())
            }
            Err(e) => {
                // The config is unchanged, so this reconnects to the main it had.
                if was_joined {
                    self.start_client();
                }
                Err(e)
            }
        }
    }

    /// Stops listening to the main without changing the config; the board
    /// and the link are forgotten. Holds no hub lock while it joins the
    /// client thread, which may be inside a callback that refreshes.
    pub fn stop_client(&self) {
        let client = self.client.lock().unwrap().take();
        if let Some(c) = client {
            c.stop_and_join();
            self.sink.service_stop();
        }
        *self.link.lock().unwrap() = AssistantLink::default();
        *self.mirror.lock().unwrap() = MirrorView::default();
        self.refresh();
    }

    /// Sends a session command to the main, which routes it to the machine
    /// that lists the session. There are no local sessions.
    pub fn send(&self, session_id: &str, kind: impl FnOnce(String) -> CommandKind) -> Result<Option<Value>, String> {
        self.request(None, Request::Command(kind(session_id.to_string())))
    }

    /// Sends a command to a machine by its label (start, resume, listings),
    /// through the main.
    pub fn send_to(&self, machine: &str, kind: CommandKind) -> Result<Option<Value>, String> {
        self.request(Some(machine), Request::Command(kind))
    }
}

/// The phone while it listens: no sessions, so every command is refused.
pub(crate) struct NoSessions;

impl Executor for NoSessions {
    fn execute(&self, _: CommandKind) -> Result<Option<Value>, String> {
        Err("This phone runs no sessions.".into())
    }
    fn board(&self) -> (Vec<Card>, Vec<String>) {
        (vec![], vec![])
    }
}

/// The client's line to the hub.
struct HubClientNotify {
    hub: Weak<Hub>,
    stop: Arc<AtomicBool>,
}

impl HubClientNotify {
    /// Repaints the board, so its cards grey or come back with the link.
    fn refresh(&self) {
        if self.stop.load(Ordering::SeqCst) {
            return;
        }
        if let Some(h) = self.hub.upgrade() {
            h.refresh();
        }
    }

    fn set(&self, f: impl FnOnce(&mut AssistantLink)) {
        if self.stop.load(Ordering::SeqCst) {
            return;
        }
        let Some(h) = self.hub.upgrade() else { return };
        let line = {
            let mut l = h.link.lock().unwrap();
            f(&mut l);
            alerts::service_line(l.main_name.as_deref().unwrap_or(SOME_MAIN), l.connected)
        };
        h.alerts.service_line(&line);
        h.sink.network(&h.status());
    }
}

impl ClientNotify for HubClientNotify {
    fn paired(&self, _: &str) {}
    fn connected(&self, main_name: &str) {
        let name = main_name.to_string();
        self.set(|l| *l = AssistantLink { connected: true, main_name: Some(name), error: None, retrying: false, mirror: None, voice: false, chat: false, wake_word: None });
        self.refresh();
    }
    fn disconnected(&self, error: &str) {
        let retrying = error != NOT_PAIRED && error != REMOVED;
        self.set(|l| {
            l.connected = false;
            l.error = Some(error.to_string());
            l.retrying = retrying;
        });
        self.refresh();
    }
    fn removed(&self) {
        self.set(|l| {
            l.connected = false;
            l.error = Some(REMOVED.into());
            l.retrying = false;
        });
        self.refresh();
    }
    fn caps(&self) -> Vec<String> {
        phone_caps()
    }
    fn mirror_offer(&self, offered: Option<bool>) {
        let Some(h) = self.hub.upgrade() else { return };
        let access = match offered {
            Some(true) => MirrorAccess::Allowed,
            Some(false) => MirrorAccess::Refused,
            None => MirrorAccess::Unsupported,
        };
        if access != MirrorAccess::Allowed {
            *h.mirror.lock().unwrap() = MirrorView::default();
        }
        self.set(|l| l.mirror = Some(access));
        h.refresh();
    }
    fn main_caps(&self, caps: &[String]) {
        let voice = caps.iter().any(|c| c == maya_core::net::protocol::CAP_VOICE);
        self.set(|l| l.voice = voice);
        // So the voice's offer follows the link.
        self.refresh();
    }
    fn main_wake_word(&self, word: Option<&str>) {
        let Some(h) = self.hub.upgrade() else { return };
        if h.link.lock().unwrap().wake_word.as_deref() == word {
            return;
        }
        self.set(|l| l.wake_word = word.map(String::from));
        if let Some(v) = h.voice.get() {
            v.set_wake_word(&maya_core::wake::wake_word(word));
        }
    }
    fn mirrored_remembered(&self, rows: Vec<RememberedRow>) {
        if self.stop.load(Ordering::SeqCst) {
            return;
        }
        if let Some(h) = self.hub.upgrade() {
            h.mirror.lock().unwrap().remembered = rows;
        }
    }
    fn mirrored(&self, allowed: bool, cards: Vec<Card>, machines: Vec<MachineInfo>) {
        if self.stop.load(Ordering::SeqCst) {
            return;
        }
        let Some(h) = self.hub.upgrade() else { return };
        if !allowed {
            *h.mirror.lock().unwrap() = MirrorView::default();
            self.set(|l| l.mirror = Some(MirrorAccess::Refused));
            h.refresh();
            return;
        }
        // What already waits when the link comes up is not news.
        if !h.mirror_seeded.swap(true, Ordering::SeqCst) {
            h.notifier.lock().unwrap().seed(&cards);
        }
        {
            let mut view = h.mirror.lock().unwrap();
            view.cards = cards;
            view.machines = machines;
        }
        self.set(|l| l.mirror = Some(MirrorAccess::Allowed));
        h.refresh();
    }
    fn lane(&self) -> Option<Arc<RequestLane>> {
        self.hub.upgrade().map(|h| h.lane.clone())
    }
    fn moved(&self, host: &str, port: u16, key: &str) -> Result<(), String> {
        let h = self.hub.upgrade().ok_or("the app is gone")?;
        h.update_config(|c| {
            c.network.main_host = host.into();
            c.network.main_port = port;
            c.network.main_key = key.into();
        })
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maya_core::config::PairedAssistant;
    use maya_core::net::client::Executor;
    use maya_core::store::now_ms;

    struct NoAlerts;
    impl crate::alerts::Alerts for NoAlerts {
        fn post(&self, _: &crate::alerts::Post) {}
        fn clear(&self, _: i32) {}
        fn service_line(&self, _: &str) {}
    }
    struct NoSink;
    impl Sink for NoSink {
        fn sessions(&self, _: &[Card]) {}
        fn network(&self, _: &NetworkStatus) {}
        fn service_start(&self, _: &str) {}
        fn service_stop(&self) {}
    }

    /// A hub over an empty config in a fresh directory, with no-op alerts and sink.
    pub(crate) fn test_hub() -> Arc<Hub> {
        let dir = std::env::temp_dir().join(format!("maya-hub-test-{}-{}", std::process::id(), now_ms()));
        let settings = Settings { path: settings_path(&dir), config: Config::default() };
        Hub::new(dir, settings, Arc::new(NoAlerts), Arc::new(NoSink))
    }

    #[test]
    fn voice_is_offered_only_when_joined_to_a_main_that_interprets() {
        let hub = test_hub();
        assert_eq!(hub.voice_offer().as_deref(), Some("Voice needs a desktop main."));
        hub.set_link_for_test(AssistantLink { connected: true, main_name: Some("Yhi".into()), mirror: Some(MirrorAccess::Allowed), voice: false, ..Default::default() }, true);
        assert_eq!(hub.voice_offer().as_deref(), Some("Update Yhi to talk to Maya."));
        hub.set_link_for_test(AssistantLink { connected: true, main_name: Some("Yhi".into()), mirror: Some(MirrorAccess::Allowed), voice: true, ..Default::default() }, true);
        assert_eq!(hub.voice_offer(), None);
        hub.set_link_for_test(AssistantLink { connected: false, main_name: Some("Yhi".into()), mirror: Some(MirrorAccess::Allowed), voice: true, ..Default::default() }, true);
        assert_eq!(hub.voice_offer().as_deref(), Some("Reconnecting to Yhi…"), "a dropped link cannot carry a command");
    }

    #[test]
    fn a_disconnect_withdraws_voice_and_a_reconnect_brings_it_back() {
        let (hub, _, _) = hub_with_a_new_ask(false);
        let v = hub.voice.get().unwrap().clone();
        let link = |connected| AssistantLink { connected, main_name: Some("Yhi".into()), mirror: Some(MirrorAccess::Allowed), voice: true, ..Default::default() };
        hub.refresh();
        assert_eq!(v.view().offer, None);
        v.tap();
        assert_eq!(v.view().state, "awaiting-command");
        hub.set_link_for_test(link(false), true);
        hub.refresh();
        assert_eq!(v.view().offer.as_deref(), Some("Reconnecting to Yhi…"));
        assert_eq!(v.view().state, "off", "the open window is dropped");
        hub.set_link_for_test(link(true), true);
        hub.refresh();
        assert_eq!(v.view().offer, None);
        assert_eq!(v.view().state, "idle");
    }

    #[test]
    fn events_are_spoken_only_with_speak_on_and_the_phone_not_silenced() {
        assert!(speaks_events(true, || false));
        assert!(!speaks_events(false, || panic!("not asked with Speak off")), "Speak off");
        assert!(!speaks_events(true, || true), "silent, vibrate or Do Not Disturb");
    }

    #[derive(Default)]
    struct Posts(Mutex<Vec<crate::alerts::Post>>);
    impl crate::alerts::Alerts for Posts {
        fn post(&self, p: &crate::alerts::Post) {
            self.0.lock().unwrap().push(p.clone());
        }
        fn clear(&self, _: i32) {}
        fn service_line(&self, _: &str) {}
    }

    #[derive(Default)]
    struct Said(Mutex<Vec<String>>);
    impl crate::voice::VoiceIo for Said {
        fn listen(&self, _: bool) {}
        fn stop_listening(&self) {}
        fn speak(&self, line: &str) {
            self.0.lock().unwrap().push(line.to_string());
        }
        fn stop_speaking(&self) {}
    }

    /// A hub joined to a main that interprets, with Speak as given, the
    /// voice on screen but not yet told it is offered, and a card that has
    /// just started waiting on the mirrored board.
    fn hub_with_a_new_ask(speak: bool) -> (Arc<Hub>, Arc<Posts>, Arc<Said>) {
        let dir = std::env::temp_dir().join(format!("maya-hub-voice-test-{}-{}-{speak}", std::process::id(), now_ms()));
        let config = Config { speak_notifications: speak, ..Config::default() };
        let posts = Arc::new(Posts::default());
        let hub = Hub::new(dir.clone(), Settings { path: settings_path(&dir), config }, posts.clone(), Arc::new(NoSink));
        hub.set_link_for_test(AssistantLink { connected: true, main_name: Some("Yhi".into()), mirror: Some(MirrorAccess::Allowed), voice: true, ..Default::default() }, true);
        let said = Arc::new(Said::default());
        let ask: crate::voice::Ask = Arc::new(|_| Err("not asked".into()));
        let v = crate::voice::Voice::new(said.clone(), ask, Arc::new(|_| {}), now_ms);
        v.screen(true);
        let _ = hub.voice.set(v);
        hub.notifier.lock().unwrap().take_new(&[]);
        let card: Card = serde_json::from_value(serde_json::json!({
            "sessionId": "s1", "pid": 0, "name": "hexgrid", "cwd": "/home/u/dev/hexgrid", "state": "awaiting", "stateSince": 5,
            "snippet": "", "hasInbox": true, "harness": "claudeCode", "stale": false
        }))
        .unwrap();
        hub.mirror.lock().unwrap().cards = vec![card];
        (hub, posts, said)
    }

    #[test]
    fn an_ask_arriving_as_voice_comes_up_is_spoken() {
        let (hub, posts, said) = hub_with_a_new_ask(true);
        hub.refresh();
        assert_eq!(said.0.lock().unwrap().as_slice(), ["hexgrid needs a decision"]);
        assert!(posts.0.lock().unwrap()[0].quiet, "posted without a sound, since it is spoken");
    }

    #[test]
    fn with_speak_off_the_phone_is_not_asked_whether_it_is_silenced() {
        let (hub, posts, said) = hub_with_a_new_ask(false);
        let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let a = asked.clone();
        let _ = hub.silenced.set(Box::new(move || {
            a.fetch_add(1, Ordering::SeqCst);
            false
        }));
        hub.refresh();
        assert_eq!(asked.load(Ordering::SeqCst), 0, "a call into Android for nothing");
        assert!(said.0.lock().unwrap().is_empty());
        assert!(!posts.0.lock().unwrap()[0].quiet);
    }

    #[test]
    fn a_queued_event_asks_the_settings_and_the_phone_at_playback() {
        let (hub, _, _) = hub_with_a_new_ask(false);
        let quiet = Arc::new(AtomicBool::new(false));
        let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (q, a) = (quiet.clone(), asked.clone());
        let _ = hub.silenced.set(Box::new(move || {
            a.fetch_add(1, Ordering::SeqCst);
            q.load(Ordering::SeqCst)
        }));
        assert!(!hub.events_may_speak(), "Speak off");
        assert_eq!(asked.load(Ordering::SeqCst), 0, "not asked with Speak off");
        hub.update_config(|c| c.speak_notifications = true).unwrap();
        assert!(hub.events_may_speak());
        quiet.store(true, Ordering::SeqCst);
        assert!(!hub.events_may_speak(), "silenced since");
    }

    #[test]
    fn a_listening_phone_has_no_sessions_and_runs_no_commands() {
        let e = NoSessions;
        assert_eq!(e.board(), (vec![], vec![]));
        assert_eq!(e.execute(CommandKind::Compact { session: "s".into() }).unwrap_err(), "This phone runs no sessions.");
    }

    fn remembered_row(machine: Option<&str>, waiting: bool, id: &str) -> maya_core::remembered::RememberedRow {
        maya_core::remembered::RememberedRow { machine: machine.map(String::from), waiting, entry: maya_core::remembered::Remembered { agent: maya_core::model::Harness::ClaudeCode, session_id: id.into(), dir: "/x".into(), name: id.into(), first_seen_ms: 1, last_seen_ms: 2, gone_at_ms: None, offered: true, offered_at_ms: None, error: None } }
    }

    #[test]
    fn a_joined_phone_shows_the_main_s_rows_and_skips_the_ones_waiting() {
        let hub = test_hub();
        hub.set_link_for_test(AssistantLink { connected: true, main_name: Some("desk".into()), mirror: Some(MirrorAccess::Allowed), voice: true, ..Default::default() }, true);
        let rows = vec![remembered_row(Some("Gnowee"), true, "g1")];
        let notify = HubClientNotify { hub: Arc::downgrade(&hub), stop: Arc::new(AtomicBool::new(false)) };
        notify.mirrored_remembered(rows.clone());
        notify.mirrored(true, vec![], vec![]);
        assert_eq!(hub.remembered(), rows, "the board that follows keeps the rows");
        let out = hub.resume_remembered(&rows);
        assert_eq!(out[0].error.as_deref(), Some("waiting for Gnowee"));
    }

    #[test]
    fn a_joined_phone_asks_the_main_to_forget_a_waiting_machine_s_rows_too() {
        let hub = test_hub();
        hub.set_link_for_test(AssistantLink { connected: true, main_name: Some("desk".into()), mirror: Some(MirrorAccess::Allowed), voice: true, ..Default::default() }, true);
        let machine = format!("Away-{}", now_ms());
        let rows = vec![remembered_row(Some(&machine), true, "w1")];
        let notify = HubClientNotify { hub: Arc::downgrade(&hub), stop: Arc::new(AtomicBool::new(false)) };
        notify.mirrored_remembered(rows.clone());
        notify.mirrored(true, vec![], vec![]);
        hub.forget_remembered(&rows);
        assert!(hub.remembered().is_empty(), "off the banner at once");
        // No main is listening in this test: the attempt is what shows the
        // request went to the main, which forgets the rows it holds for the
        // away machine so its next mirror does not bring them back.
        let tried = maya_core::log::lines().iter().any(|l| l.source == "remembered" && l.text.starts_with(&format!("{machine} did not forget its sessions")));
        assert!(tried, "the Forget was sent through the main, not skipped");
    }

    #[test]
    fn a_joined_phone_shows_the_main_s_wake_word() {
        let hub = test_hub();
        hub.set_link_for_test(AssistantLink { connected: true, main_name: Some("desk".into()), mirror: Some(MirrorAccess::Allowed), voice: true, ..Default::default() }, true);
        let notify = HubClientNotify { hub: Arc::downgrade(&hub), stop: Arc::new(AtomicBool::new(false)) };
        let word = || hub.link.lock().unwrap().wake_word.clone();
        assert_eq!(word(), None, "nothing until the main says");
        notify.main_wake_word(Some("Jarvis"));
        assert_eq!(word().as_deref(), Some("Jarvis"), "the page's status carries the link");
        notify.main_wake_word(None);
        assert_eq!(word(), None, "an older main says nothing");
    }

    #[test]
    fn a_phone_saved_as_main_loads_off_with_no_assistants() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.network.role = NetworkRole::Main;
        config.network.assistants = vec![PairedAssistant { id: "a1".into(), name: "laptop".into(), hostname: "h".into(), platform: "macos".into(), key: "k".into(), address: "10.0.0.5".into(), last_seen: None, mirror: false }];
        config::save(&settings_path(dir.path()), &config).unwrap();
        let settings = load_settings(dir.path(), "Pixel 8");
        assert_eq!(settings.config.network.role, NetworkRole::Off);
        assert!(settings.config.network.assistants.is_empty());
        let saved = config::load(&settings_path(dir.path()));
        assert_eq!(saved.network.role, NetworkRole::Off, "the change is saved");
        assert!(saved.network.assistants.is_empty());
        assert!(maya_core::log::lines().iter().any(|l| l.source == "network" && l.text == "this phone is no longer a main: join a main computer"));
    }

    #[test]
    fn the_phone_asks_to_follow_mirror_and_chat_but_not_lead() {
        assert_eq!(phone_caps(), vec![CAP_FOLLOW.to_string(), CAP_MIRROR.into(), maya_core::net::protocol::CAP_CHAT.into()]);
    }

    #[test]
    fn a_key_file_that_cannot_be_read_shows_until_the_client_starts() {
        let hub = test_hub();
        let key_file = hub.maya_dir.join(identity::FILE);
        // A directory where the key file should be: it cannot be read.
        std::fs::create_dir_all(&key_file).unwrap();
        hub.start_client();
        assert!(!hub.joined());
        let status = hub.status();
        assert_eq!(status.role, NetworkRole::Off);
        assert!(status.main_error.is_some_and(|e| !e.is_empty()), "the page is told why");
        std::fs::remove_dir_all(&key_file).unwrap();
        hub.start_client();
        assert!(hub.joined());
        hub.stop_client();
        assert_eq!(hub.status().main_error, None, "cleared once the client started");
    }

    #[test]
    fn the_phone_has_no_sessions_of_its_own_to_resume() {
        let hub = test_hub();
        assert!(hub.remembered().is_empty(), "not joined to a main");
        let out = hub.resume_remembered(&[remembered_row(None, false, "l1")]);
        assert_eq!(out[0].error.as_deref(), Some("This phone runs no sessions."));
    }
}
