//! The phone's hub against a real core client on localhost: pairing, a
//! board, a command round trip, and which boards post notifications.

use maya_core::config::{Config, NetworkConfig, NetworkRole};
use maya_core::model::{AwaitKind, Awaiting, Card, Harness, State};
use maya_core::net::client::{pair_with, run_once, ClientNotify, Executor};
use maya_core::net::protocol::CommandKind;
use maya_core::net::NetworkStatus;
use maya_mobile_lib::alerts::{Alerts, Post};
use maya_mobile_lib::hub::{Hub, Settings, Sink};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Recording {
    posts: Mutex<Vec<Post>>,
    cleared: Mutex<Vec<i32>>,
    service: Mutex<Vec<String>>,
}

impl Alerts for Recording {
    fn post(&self, post: &Post) {
        self.posts.lock().unwrap().push(post.clone());
    }
    fn clear(&self, id: i32) {
        self.cleared.lock().unwrap().push(id);
    }
    fn service_line(&self, line: &str) {
        self.service.lock().unwrap().push(line.into());
    }
}

#[derive(Default)]
struct Page {
    boards: Mutex<Vec<Vec<Card>>>,
    statuses: Mutex<Vec<NetworkStatus>>,
    service: Mutex<Vec<String>>,
}

impl Sink for Page {
    fn sessions(&self, cards: &[Card]) {
        self.boards.lock().unwrap().push(cards.to_vec());
    }
    fn network(&self, status: &NetworkStatus) {
        self.statuses.lock().unwrap().push(status.clone());
    }
    fn service_start(&self, line: &str) {
        self.service.lock().unwrap().push(format!("start: {line}"));
    }
    fn service_stop(&self) {
        self.service.lock().unwrap().push("stop".into());
    }
}

/// A pretend assistant: one board it can swap, and a log of the commands it ran.
struct FakeAssistant {
    cards: Mutex<Vec<Card>>,
    due: AtomicBool,
    ran: Mutex<Vec<CommandKind>>,
}

impl Executor for FakeAssistant {
    fn execute(&self, kind: CommandKind) -> Result<Option<Value>, String> {
        self.ran.lock().unwrap().push(kind);
        Ok(None)
    }
    fn board(&self) -> (Vec<Card>, Vec<String>) {
        (self.cards.lock().unwrap().clone(), vec!["hexgrid".into(), "maya".into()])
    }
    fn board_requested(&self) -> bool {
        self.due.swap(false, Ordering::SeqCst)
    }
}

struct Quiet;
impl ClientNotify for Quiet {
    fn paired(&self, _: &str, _: &str) {}
    fn connected(&self, _: &str) {}
    fn disconnected(&self, _: &str) {}
    fn removed(&self) {}
}

fn card(id: &str, state: State, since: u64) -> Card {
    Card {
        session_id: id.into(),
        pid: 42,
        name: id.into(),
        cwd: "/home/u/dev/hexgrid".into(),
        state,
        state_since: since,
        snippet: "working on it".into(),
        awaiting: (state == State::Awaiting).then(|| Awaiting { kind: AwaitKind::Permission, detail: "Run the tests?".into(), questions: vec![] }),
        has_inbox: true,
        harness: Harness::ClaudeCode,
        pr: None,
        context: None,
        machine: None,
        machine_address: None,
        machine_platform: None,
        terminal: None,
        stale: false,
        model: None,
    }
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port()
}

#[test]
fn pairs_merges_a_board_routes_a_reply_and_notifies_only_what_is_new() {
    let dir = tempfile::tempdir().unwrap();
    let maya_dir = dir.path().join("maya");
    let port = free_port();
    let mut config = Config::default();
    config.network.port = port;
    config.network.name = "Pixel".into();
    let settings = Settings { path: maya_dir.join("config.json"), config };
    let alerts = Arc::new(Recording::default());
    let page = Arc::new(Page::default());
    let hub = Hub::new(maya_dir.clone(), settings, alerts.clone(), page.clone());

    hub.start_server().unwrap();
    assert_eq!(page.service.lock().unwrap().as_slice(), ["start: No assistants paired yet"]);
    let status = hub.pairing_code().unwrap();
    let code = status.code.expect("a pairing code").code;

    // The assistant pairs with its first board already holding an ask: not news.
    let assistant = Arc::new(FakeAssistant { cards: Mutex::new(vec![card("s1", State::Awaiting, 1_000)]), due: AtomicBool::new(false), ran: Mutex::new(vec![]) });
    let (main_name, id, token) = pair_with(assistant.clone(), "127.0.0.1", port, "laptop", &code).unwrap();
    assert_eq!(main_name, "Pixel");
    let saved = maya_core::config::load(&maya_dir.join("config.json"));
    assert_eq!(saved.network.assistants.len(), 1, "the pairing was saved");
    assert_eq!(saved.network.assistants[0].id, id);

    let link = NetworkConfig { role: NetworkRole::Assistant, main_host: "127.0.0.1".into(), main_port: port, name: "laptop".into(), assistant_id: id, token, ..Default::default() };
    let stop = Arc::new(AtomicBool::new(false));
    let client = {
        let (assistant, stop) = (assistant.clone(), stop.clone());
        std::thread::spawn(move || run_once(&link, assistant, Arc::new(Quiet), &stop, None))
    };

    wait_until("the first board", || hub.cards().iter().any(|c| c.session_id == "s1"));
    let cards = hub.cards();
    assert_eq!(cards[0].machine.as_deref(), Some("laptop"), "merged cards carry the label");
    assert!(alerts.posts.lock().unwrap().is_empty(), "a first board after pairing is seeded, not announced");
    wait_until("the service line", || alerts.service.lock().unwrap().iter().any(|l| l == "Main for 1 assistant, connected"));

    // A reply from the phone reaches the assistant through the server.
    hub.send("s1", |s| CommandKind::Reply { session: s, text: "go ahead".into(), attachments: vec![] }).unwrap();
    wait_until("the reply", || !assistant.ran.lock().unwrap().is_empty());
    assert!(matches!(&assistant.ran.lock().unwrap()[0], CommandKind::Reply { session, text, .. } if session == "s1" && text == "go ahead"));
    assert_eq!(hub.send("nope", |s| CommandKind::Compact { session: s }).unwrap_err(), "Session is no longer running.");

    // A later board with a new ask posts exactly one notification; s1 moving on clears its (never posted) slot silently.
    *assistant.cards.lock().unwrap() = vec![card("s1", State::Working, 2_000), card("s2", State::Awaiting, 2_000)];
    assistant.due.store(true, Ordering::SeqCst);
    wait_until("the decision notification", || !alerts.posts.lock().unwrap().is_empty());
    std::thread::sleep(Duration::from_millis(300));
    let posts = alerts.posts.lock().unwrap().clone();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].title, "s2 on laptop needs a decision");
    assert_eq!(posts[0].body, "Run the tests?");
    assert!(alerts.cleared.lock().unwrap().is_empty());
    // A tap hands back only the notification's id: it leads to the card.
    assert_eq!(hub.session_for_notification(posts[0].id).as_deref(), Some("s2"));
    assert_eq!(hub.wait_for_notification(posts[0].id.wrapping_add(1), Duration::from_millis(300)), None);

    // Answered at the desk: the notification goes.
    *assistant.cards.lock().unwrap() = vec![card("s2", State::Working, 3_000)];
    assistant.due.store(true, Ordering::SeqCst);
    wait_until("the clear", || !alerts.cleared.lock().unwrap().is_empty());
    assert_eq!(alerts.cleared.lock().unwrap()[0], posts[0].id);

    hub.stop_server();
    stop.store(true, Ordering::SeqCst);
    let _ = client.join();
    assert!(page.service.lock().unwrap().iter().any(|l| l == "stop"));
    assert_eq!(hub.status().role, NetworkRole::Off);
}

#[test]
fn a_taken_port_is_reported_and_starts_no_service() {
    let dir = tempfile::tempdir().unwrap();
    let holder = std::net::TcpListener::bind(("0.0.0.0", 0)).unwrap();
    let port = holder.local_addr().unwrap().port();
    let mut config = Config::default();
    config.network.port = port;
    let settings = Settings { path: dir.path().join("config.json"), config };
    let page = Arc::new(Page::default());
    let hub = Hub::new(dir.path().to_path_buf(), settings, Arc::new(Recording::default()), page.clone());
    let err = hub.start_server().unwrap_err();
    assert!(err.contains(&format!("port {port}")), "{err}");
    assert_eq!(hub.status().main_error.as_deref(), Some(err.as_str()));
    assert!(page.service.lock().unwrap().is_empty(), "no foreground service for a server that is not running");
    assert_eq!(hub.pairing_code().unwrap_err(), err);
}

#[test]
fn a_refresh_with_no_new_frame_drops_an_expired_assistant_and_clears_its_notification() {
    use maya_core::net::merge::EXPIRE_MS;
    use std::sync::atomic::AtomicU64;

    let dir = tempfile::tempdir().unwrap();
    let maya_dir = dir.path().join("maya");
    let port = free_port();
    let mut config = Config::default();
    config.network.port = port;
    config.network.name = "Pixel".into();
    let settings = Settings { path: maya_dir.join("config.json"), config };
    let alerts = Arc::new(Recording::default());
    let page = Arc::new(Page::default());
    // The hub's clock runs `ahead` of the real one, which the server stamps boards with.
    let ahead = Arc::new(AtomicU64::new(0));
    let clock = {
        let ahead = ahead.clone();
        move || maya_core::store::now_ms() + ahead.load(Ordering::SeqCst)
    };
    let hub = Hub::with_clock(maya_dir, settings, alerts.clone(), page.clone(), clock);

    hub.start_server().unwrap();
    let code = hub.pairing_code().unwrap().code.expect("a pairing code").code;
    let assistant = Arc::new(FakeAssistant { cards: Mutex::new(vec![card("s1", State::Working, 1_000)]), due: AtomicBool::new(false), ran: Mutex::new(vec![]) });
    let (_, id, token) = pair_with(assistant.clone(), "127.0.0.1", port, "laptop", &code).unwrap();
    let link = NetworkConfig { role: NetworkRole::Assistant, main_host: "127.0.0.1".into(), main_port: port, name: "laptop".into(), assistant_id: id, token, ..Default::default() };
    let stop = Arc::new(AtomicBool::new(false));
    let client = {
        let (assistant, stop) = (assistant.clone(), stop.clone());
        std::thread::spawn(move || run_once(&link, assistant, Arc::new(Quiet), &stop, None))
    };
    wait_until("the first board", || hub.cards().iter().any(|c| c.session_id == "s1"));
    *assistant.cards.lock().unwrap() = vec![card("s1", State::Awaiting, 2_000)];
    assistant.due.store(true, Ordering::SeqCst);
    wait_until("the decision notification", || !alerts.posts.lock().unwrap().is_empty());
    let posted = alerts.posts.lock().unwrap()[0].id;

    // The assistant goes away: its card stays, greyed, and so does the notification.
    stop.store(true, Ordering::SeqCst);
    let _ = client.join();
    wait_until("the disconnect", || hub.status().assistants.iter().all(|a| !a.connected));
    hub.refresh();
    assert!(hub.cards().iter().any(|c| c.session_id == "s1" && c.stale), "a disconnected board shows greyed until it expires");
    assert!(alerts.cleared.lock().unwrap().is_empty());

    // Five minutes on, a refresh with no new frame drops the board and clears the notification.
    ahead.store(EXPIRE_MS + 1_000, Ordering::SeqCst);
    hub.refresh();
    assert!(hub.cards().is_empty());
    assert!(page.boards.lock().unwrap().last().unwrap().is_empty(), "the page is told the board emptied");
    assert_eq!(alerts.cleared.lock().unwrap().as_slice(), [posted]);
    hub.stop_server();
}

fn new_hub(port: u16) -> (Arc<Hub>, Arc<Page>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let maya_dir = dir.path().join("maya");
    let mut config = Config::default();
    config.network.port = port;
    config.network.name = "Pixel".into();
    config.network.role = NetworkRole::Main;
    let settings = Settings { path: maya_dir.join("config.json"), config };
    let page = Arc::new(Page::default());
    let hub = Hub::new(maya_dir, settings, Arc::new(Recording::default()), page.clone());
    (hub, page, dir)
}

/// A desktop-like assistant: its client becomes a main on `promote`.
struct Lead {
    config: Arc<Mutex<NetworkConfig>>,
    server: Mutex<Option<maya_core::net::server::ServerHandle>>,
}

impl ClientNotify for Lead {
    fn paired(&self, _: &str, _: &str) {}
    fn connected(&self, _: &str) {}
    fn disconnected(&self, _: &str) {}
    fn removed(&self) {}
    fn caps(&self) -> Vec<String> {
        vec!["follow".into(), "lead".into()]
    }
    fn promoted(&self, list: Vec<maya_core::config::PairedAssistant>) -> Result<u16, String> {
        let mut c = Config::default();
        c.network = self.config.lock().unwrap().clone();
        maya_core::net::promote::as_main(&mut c, list);
        let h = maya_core::net::server::start_with(Arc::new(NoNotify), Arc::new(Mutex::new(c.network.clone())), 0)?;
        *self.config.lock().unwrap() = c.network;
        let port = h.port();
        *self.server.lock().unwrap() = Some(h);
        Ok(port)
    }
}

struct NoNotify;
impl maya_core::net::server::Notify for NoNotify {
    fn board_changed(&self) {}
    fn board_seeded(&self, _: &[Card]) {}
    fn status_changed(&self, _: NetworkStatus) {}
    fn paired(&self, _: &maya_core::config::PairedAssistant) -> Result<(), String> {
        Ok(())
    }
    fn paired_list_changed(&self, _: &[maya_core::config::PairedAssistant]) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn the_phone_hands_the_main_role_over_listens_and_is_made_the_main_again() {
    let (hub, page, _dir) = new_hub(free_port());
    hub.start_server().unwrap();
    let port = hub.settings.lock().unwrap().config.listen_port();
    let code = hub.pairing_code().unwrap().code.unwrap().code;
    let exec: Arc<dyn Executor> = Arc::new(FakeAssistant { cards: Mutex::new(vec![]), due: AtomicBool::new(false), ran: Mutex::new(vec![]) });
    let (_, id, token) = pair_with(exec.clone(), "127.0.0.1", port, "laptop", &code).unwrap();
    let config = Arc::new(Mutex::new(NetworkConfig { role: NetworkRole::Assistant, main_host: "127.0.0.1".into(), main_port: port, name: "laptop".into(), assistant_id: id.clone(), token, port: free_port(), ..Default::default() }));
    let lead = Arc::new(Lead { config: config.clone(), server: Mutex::new(None) });
    let stop = Arc::new(AtomicBool::new(false));
    let (n, s, cfg) = (lead.clone(), stop.clone(), config.clone());
    let laptop = std::thread::spawn(move || maya_core::net::client::reconnect(&move || cfg.lock().unwrap().clone(), exec, n, &s));
    wait_until("the laptop can lead", || hub.status().assistants.iter().any(|a| a.id == id && a.can_lead));

    // The phone makes the laptop the main and listens to it.
    let status = hub.promote(&id).unwrap();
    assert_eq!(status.role, NetworkRole::Assistant);
    laptop.join().unwrap();
    let new_main = lead.server.lock().unwrap().clone().unwrap();
    wait_until("the phone listens to the laptop", || hub.status().assistant.connected);
    let phone = new_main.status().assistants.into_iter().find(|a| a.connected).unwrap();
    assert!(phone.can_lead, "the phone can be made the main again");
    assert!(hub.cards().is_empty());
    assert!(page.statuses.lock().unwrap().iter().any(|s| s.role == NetworkRole::Assistant));

    // The laptop makes the phone the main again.
    let back = new_main.promote(&phone.id).unwrap();
    wait_until("the phone runs its server", || hub.status().role == NetworkRole::Main);
    assert_eq!(back.port, hub.settings.lock().unwrap().config.listen_port());
    assert!(hub.client.lock().unwrap().is_none());
    new_main.stop();
    hub.stop_server();
}

#[test]
fn stop_listening_turns_the_phone_off_and_forgets_its_link() {
    let (hub, _page, _dir) = new_hub(free_port());
    hub.update_config(|c| {
        c.network.role = NetworkRole::Assistant;
        c.network.main_host = "127.0.0.1".into();
        c.network.main_port = free_port();
        c.network.assistant_id = "id".into();
        c.network.token = "tok".into();
    })
    .unwrap();
    hub.start_client();
    let status = hub.stop_listening().unwrap();
    assert_eq!(status.role, NetworkRole::Off);
    let c = hub.settings.lock().unwrap().config.network.clone();
    assert_eq!(c.role, NetworkRole::Off);
    assert!(c.assistant_id.is_empty() && c.token.is_empty() && c.main_host.is_empty());
    assert!(hub.client.lock().unwrap().is_none());
}

#[test]
fn stop_listening_that_cannot_save_keeps_the_phone_listening() {
    let (hub, _page, dir) = new_hub(free_port());
    hub.update_config(|c| {
        c.network.role = NetworkRole::Assistant;
        c.network.main_host = "127.0.0.1".into();
        c.network.main_port = free_port();
        c.network.assistant_id = "id".into();
        c.network.token = "tok".into();
    })
    .unwrap();
    hub.start_client();
    // A directory where the config file should be: every save fails.
    hub.settings.lock().unwrap().path = dir.path().to_path_buf();
    assert!(hub.stop_listening().is_err());
    assert!(hub.client.lock().unwrap().is_some(), "still listening");
    assert_eq!(hub.status().role, NetworkRole::Assistant);
    assert_eq!(hub.settings.lock().unwrap().config.network.token, "tok");
    hub.client.lock().unwrap().take().unwrap().stop();
}
