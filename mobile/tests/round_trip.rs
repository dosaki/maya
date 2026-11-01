//! The phone's hub against a real core main on localhost: joining, the
//! mirrored board, a command round trip, the main's Chat, and which boards
//! post notifications.

use maya_core::config::{Config, NetworkConfig, NetworkRole};
use maya_core::model::{AwaitKind, Awaiting, Card, Harness, State};
use maya_core::net::identity::Keypair;
use maya_core::net::protocol::{CommandKind, Down, Request, CAP_CHAT};
use maya_core::net::server::{start_with, ServerHandle};
use maya_core::net::{MirrorAccess, NetworkStatus};
use maya_mobile_lib::alerts::{Alerts, Post};
use maya_mobile_lib::hub::{Hub, Settings, Sink};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU32, Ordering};
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
    chats: Mutex<Vec<Value>>,
    brains: Mutex<Vec<Value>>,
    chat_resets: Mutex<Vec<Vec<Value>>>,
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
    fn chat(&self, turn: &Value) {
        self.chats.lock().unwrap().push(turn.clone());
    }
    fn brain(&self, status: &Value) {
        self.brains.lock().unwrap().push(status.clone());
    }
    fn chat_reset(&self, turns: &[Value]) {
        self.chat_resets.lock().unwrap().push(turns.to_vec());
    }
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
        branch: None, brain: false, native: false, shell: false, usage: None, limits: None, status_line: None, live: false,
    }
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// A free port below the ephemeral range. One picked by binding port 0
/// comes from the range outgoing connections use, so a parallel test's
/// client could take it before the hub binds it ("Address already in use").
fn free_port() -> u16 {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let base = 20_000 + std::process::id() % 1_000 * 20;
    loop {
        let port = (base + NEXT.fetch_add(1, Ordering::SeqCst) % 9_000) as u16;
        if std::net::TcpListener::bind(("0.0.0.0", port)).is_ok() {
            return port;
        }
    }
}

fn new_hub_recording(port: u16) -> (Arc<Hub>, Arc<Page>, Arc<Recording>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let maya_dir = dir.path().join("maya");
    let mut config = Config::default();
    config.network.port = port;
    config.network.name = "Pixel".into();
    let settings = Settings { path: maya_dir.join("config.json"), config };
    let page = Arc::new(Page::default());
    let alerts = Arc::new(Recording::default());
    let hub = Hub::new(maya_dir, settings, alerts.clone(), page.clone());
    (hub, page, alerts, dir)
}

fn new_hub(port: u16) -> (Arc<Hub>, Arc<Page>, tempfile::TempDir) {
    let (hub, page, _, dir) = new_hub_recording(port);
    (hub, page, dir)
}

/// Listens to a main that is not there: the client keeps retrying.
fn listen_to_nobody(hub: &Arc<Hub>) {
    hub.update_config(|c| {
        c.network.role = NetworkRole::Assistant;
        c.network.main_host = "127.0.0.1".into();
        c.network.main_port = free_port();
        c.network.main_key = "a2V5".into();
    })
    .unwrap();
    hub.start_client();
}

#[test]
fn stop_listening_turns_the_phone_off_and_forgets_its_link() {
    let (hub, _page, _dir) = new_hub(free_port());
    listen_to_nobody(&hub);
    let status = hub.stop_listening().unwrap();
    assert_eq!(status.role, NetworkRole::Off);
    let c = hub.settings.lock().unwrap().config.network.clone();
    assert_eq!(c.role, NetworkRole::Off);
    assert!(c.main_key.is_empty() && c.main_host.is_empty());
    assert!(hub.client.lock().unwrap().is_none());
}

#[test]
fn stop_listening_that_cannot_save_keeps_the_phone_listening() {
    let (hub, _page, dir) = new_hub(free_port());
    listen_to_nobody(&hub);
    // A directory where the config file should be: every save fails.
    hub.settings.lock().unwrap().path = dir.path().to_path_buf();
    assert!(hub.stop_listening().is_err());
    assert!(hub.client.lock().unwrap().is_some(), "still listening");
    assert_eq!(hub.status().role, NetworkRole::Assistant);
    assert_eq!(hub.settings.lock().unwrap().config.network.main_key, "a2V5");
    hub.client.lock().unwrap().take().unwrap().stop();
}

/// A core main with Chat that records the requests it is asked.
#[derive(Default)]
struct MainNotify {
    requests: Mutex<Vec<(Option<String>, Request)>>,
    /// The paired list as the main would have saved it.
    saved: Mutex<Vec<maya_core::config::PairedAssistant>>,
}

impl maya_core::net::server::Notify for MainNotify {
    fn board_changed(&self) {}
    fn board_seeded(&self, _: &[Card]) {}
    fn status_changed(&self, _: NetworkStatus) {}
    fn paired(&self, a: &maya_core::config::PairedAssistant) -> Result<(), String> {
        let mut saved = self.saved.lock().unwrap();
        match saved.iter_mut().find(|s| s.id == a.id) {
            Some(s) => *s = a.clone(),
            None => saved.push(a.clone()),
        }
        Ok(())
    }
    fn paired_list_changed(&self, list: &[maya_core::config::PairedAssistant]) -> Result<(), String> {
        *self.saved.lock().unwrap() = list.to_vec();
        Ok(())
    }
    fn request(&self, machine: Option<&str>, request: Request) -> Result<Option<Value>, String> {
        let answer = match request {
            Request::ChatHistory => Some(json!({"turns": [{"text": "old"}], "status": {"dot": "idle"}})),
            _ => None,
        };
        self.requests.lock().unwrap().push((machine.map(str::to_string), request));
        Ok(answer)
    }
    fn main_caps(&self) -> Vec<String> {
        vec![CAP_CHAT.into()]
    }
}

fn desk() -> (ServerHandle, u16, Arc<MainNotify>) {
    desk_with(Arc::new(Keypair::generate()))
}

fn desk_with(key: Arc<Keypair>) -> (ServerHandle, u16, Arc<MainNotify>) {
    let notify = Arc::new(MainNotify::default());
    let config = NetworkConfig { name: "desk".into(), ..Default::default() };
    let handle = start_with(notify.clone(), Arc::new(Mutex::new(config)), 0, key).unwrap();
    let port = handle.port();
    (handle, port, notify)
}

/// The main again on `port` with its key and the assistants it had saved,
/// once the port is free.
fn desk_again(key: Arc<Keypair>, port: u16, assistants: Vec<maya_core::config::PairedAssistant>) -> (ServerHandle, Arc<MainNotify>) {
    let notify = Arc::new(MainNotify::default());
    let config = Arc::new(Mutex::new(NetworkConfig { name: "desk".into(), assistants, ..Default::default() }));
    let started = Instant::now();
    loop {
        match start_with(notify.clone(), config.clone(), port, key.clone()) {
            Ok(m) => return (m, notify),
            Err(e) => {
                assert!(started.elapsed() < Duration::from_secs(10), "the port did not come back: {e}");
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

#[test]
fn the_phone_joins_a_main_mirrors_it_quietly_acts_through_it_and_leaves() {
    let (main, port, notify) = desk();
    let (hub, _page, alerts, _dir) = new_hub_recording(free_port());
    let code = main.open_pairing(maya_core::store::now_ms()).unwrap().0;
    let status = hub.join("127.0.0.1", port, "Pixel", &code).unwrap();
    assert_eq!(status.role, NetworkRole::Assistant);
    wait_until("the offer", || hub.status().assistant.mirror == Some(MirrorAccess::Allowed));
    let phone_id = main.status().assistants[0].id.clone();
    assert!(main.status().assistants[0].mirror);
    wait_until("the service line", || alerts.service.lock().unwrap().last().map(String::as_str) == Some("Joined to desk"));

    // What already waits is on the board but not announced.
    main.mirror(vec![card("s1", State::Awaiting, 1_000)]);
    wait_until("the mirrored card", || hub.cards().iter().any(|c| c.session_id == "s1" && c.machine.as_deref() == Some("desk")));
    std::thread::sleep(Duration::from_millis(200));
    assert!(alerts.posts.lock().unwrap().is_empty(), "the first mirror after a connect is seeded, not announced");
    // A new ask is.
    main.mirror(vec![card("s1", State::Awaiting, 1_000), card("s2", State::Awaiting, 2_000)]);
    wait_until("one notification", || alerts.posts.lock().unwrap().len() == 1);
    assert_eq!(alerts.posts.lock().unwrap()[0].title, "s2 on desk needs a decision");
    assert_eq!(hub.mirror.lock().unwrap().machines[0].name, "desk");

    // A command goes to the main.
    hub.send("s1", |session| CommandKind::Compact { session }).unwrap();
    assert!(notify.requests.lock().unwrap().iter().any(|r| matches!(r, (None, Request::Command(CommandKind::Compact { session })) if session == "s1")));

    // The switch off empties the board; on brings it back.
    main.set_mirror(&phone_id, false).unwrap();
    wait_until("the refusal", || hub.cards().is_empty() && hub.status().assistant.mirror == Some(MirrorAccess::Refused));
    main.set_mirror(&phone_id, true).unwrap();
    main.mirror(vec![card("s1", State::Idle, 3_000)]);
    wait_until("the board again", || hub.cards().iter().any(|c| c.session_id == "s1"));

    // The main goes away: the cards stay, greyed.
    main.stop();
    wait_until("stale cards", || !hub.cards().is_empty() && hub.cards().iter().all(|c| c.stale));
    wait_until("the service line", || alerts.service.lock().unwrap().last().map(String::as_str) == Some("Reconnecting to desk…"));

    let status = hub.stop_listening().unwrap();
    assert_eq!(status.role, NetworkRole::Off);
    assert!(hub.cards().is_empty());
}

#[test]
fn a_reconnect_announces_the_asks_that_began_while_away() {
    let key = Arc::new(Keypair::generate());
    let (main, port, notify) = desk_with(key.clone());
    let (hub, _page, alerts, _dir) = new_hub_recording(free_port());
    let code = main.open_pairing(maya_core::store::now_ms()).unwrap().0;
    hub.join("127.0.0.1", port, "Pixel", &code).unwrap();
    wait_until("the offer", || hub.status().assistant.mirror == Some(MirrorAccess::Allowed));
    main.mirror(vec![card("s1", State::Awaiting, 1_000)]);
    wait_until("the mirrored card", || hub.cards().iter().any(|c| c.session_id == "s1"));
    std::thread::sleep(Duration::from_millis(200));
    assert!(alerts.posts.lock().unwrap().is_empty());

    // The main goes away and comes back on the same port with the phone's pairing.
    let saved = notify.saved.lock().unwrap().clone();
    main.stop();
    wait_until("the link drops", || !hub.status().assistant.connected);
    let (main, _) = desk_again(key, port, saved);
    wait_until("the phone is back", || hub.status().assistant.connected && hub.status().assistant.mirror == Some(MirrorAccess::Allowed));
    // s1 still waits (same since); s2 began while the phone was away.
    main.mirror(vec![card("s1", State::Awaiting, 1_000), card("s2", State::Awaiting, 2_000)]);
    wait_until("the new ask", || alerts.posts.lock().unwrap().len() == 1);
    std::thread::sleep(Duration::from_millis(300));
    let posts = alerts.posts.lock().unwrap();
    assert_eq!(posts.len(), 1, "only the ask that began while away is announced");
    assert_eq!(posts[0].title, "s2 on desk needs a decision");
    drop(posts);
    hub.stop_listening().unwrap();
    main.stop();
}

#[test]
fn a_joined_phone_gets_the_chat_live_and_again_after_a_reconnect() {
    let key = Arc::new(Keypair::generate());
    let (main, port, notify) = desk_with(key.clone());
    let (hub, page, _dir) = new_hub(free_port());
    let code = main.open_pairing(maya_core::store::now_ms()).unwrap().0;
    hub.join("127.0.0.1", port, "Pixel", &code).unwrap();
    let old = vec![json!({"text": "old"})];
    // Fetched off the client thread, which carries the answer: on it, this would never come.
    wait_until("the history", || page.chat_resets.lock().unwrap().last() == Some(&old));
    wait_until("the main's status", || page.brains.lock().unwrap().last() == Some(&json!({"dot": "idle"})));

    // A turn comes live.
    main.to_mirrors(Down::Chat { turn: json!({"text": "new"}) });
    wait_until("the live turn", || page.chats.lock().unwrap().iter().any(|t| t["text"] == "new") && hub.chat.lock().unwrap().turns.len() == 2);

    // Typing goes to the main.
    hub.chat_request(Request::ChatSend { text: "hi".into() }).unwrap();
    assert!(notify.requests.lock().unwrap().iter().any(|r| matches!(r, (None, Request::ChatSend { text }) if text == "hi")));

    // The switch off empties the Chat; on fetches it again.
    let phone_id = main.status().assistants[0].id.clone();
    main.set_mirror(&phone_id, false).unwrap();
    wait_until("the Chat emptied", || hub.chat.lock().unwrap().turns.is_empty() && page.chat_resets.lock().unwrap().last() == Some(&vec![]));
    main.set_mirror(&phone_id, true).unwrap();
    main.mirror(vec![]);
    wait_until("the history again", || page.chat_resets.lock().unwrap().last() == Some(&old));

    // The main goes away: the page says so, and the turns stay.
    let saved = notify.saved.lock().unwrap().clone();
    main.to_mirrors(Down::Chat { turn: json!({"text": "new"}) });
    wait_until("the live turn again", || hub.chat.lock().unwrap().turns.len() == 2);
    let resets = page.chat_resets.lock().unwrap().len();
    main.stop();
    wait_until("the note", || page.brains.lock().unwrap().last().is_some_and(|b| b["dormant"] == "Reconnecting to desk…"));
    assert_eq!(hub.chat.lock().unwrap().turns.len(), 2);
    assert_eq!(page.chat_resets.lock().unwrap().len(), resets, "the page keeps its turns too");

    // Back on the same port: the history replaces the turns.
    let (main, _) = desk_again(key, port, saved);
    wait_until("the history after the reconnect", || page.chat_resets.lock().unwrap().len() == resets + 1);
    assert_eq!(hub.chat.lock().unwrap().turns, old, "replaced, not appended to");
    assert_eq!(page.chat_resets.lock().unwrap().last(), Some(&old));

    // Leaving empties the page's Chat too, so the next main never shows it.
    hub.stop_listening().unwrap();
    assert!(hub.chat.lock().unwrap().turns.is_empty());
    assert_eq!(page.chat_resets.lock().unwrap().last(), Some(&vec![]));
    main.stop();
}

#[test]
fn a_join_with_a_wrong_code_reconnects_a_phone_that_was_joined() {
    let (a, a_port, _) = desk();
    let (b, b_port, _) = desk();
    let (hub, _page, _dir) = new_hub(free_port());
    let code = a.open_pairing(maya_core::store::now_ms()).unwrap().0;
    hub.join("127.0.0.1", a_port, "Pixel", &code).unwrap();
    wait_until("connected to the first main", || hub.status().assistant.connected);
    // No pairing window is open on the second main: any code is wrong.
    let err = hub.join("127.0.0.1", b_port, "Pixel", "123456").unwrap_err();
    assert_eq!(err, "Wrong or expired pairing code.");
    assert!(hub.joined());
    wait_until("reconnected to the first main", || hub.status().assistant.connected);
    assert_eq!(hub.status().role, NetworkRole::Assistant);
    hub.stop_listening().unwrap();
    a.stop();
    b.stop();
}

#[test]
fn a_request_to_an_older_main_fails_at_once() {
    let (hub, _page, _dir) = new_hub(free_port());
    *hub.link.lock().unwrap() = maya_core::net::AssistantLink { mirror: Some(MirrorAccess::Unsupported), main_name: Some("Studio".into()), ..Default::default() };
    let started = std::time::Instant::now();
    let err = hub.request(None, Request::ListDirs).unwrap_err();
    assert_eq!(err, "Studio runs an older Maya that cannot share its sessions. Update it.");
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn the_phone_keeps_one_key_until_new_key_drops_every_pairing() {
    let (hub, _page, dir) = new_hub(free_port());
    let first = hub.identity().unwrap();
    assert_eq!(hub.identity().unwrap().public, first.public, "kept");
    assert!(dir.path().join("maya").join(maya_core::net::identity::FILE).exists());
    hub.update_config(|c| c.network.main_key = "a2V5".into()).unwrap();
    hub.new_key().unwrap();
    assert_ne!(hub.identity().unwrap().public, first.public);
    let config = hub.settings.lock().unwrap().config.clone();
    assert!(config.network.main_key.is_empty() && config.network.assistants.is_empty());
    assert_eq!(config.network.role, NetworkRole::Off);
}

#[test]
fn every_status_the_page_gets_carries_pair_again() {
    let (hub, page, _dir) = new_hub(free_port());
    hub.update_config(|c| c.network.pair_again = true).unwrap();
    assert!(hub.status().pair_again);
    listen_to_nobody(&hub);
    assert!(hub.status().pair_again);
    assert!(page.statuses.lock().unwrap().last().unwrap().pair_again, "the client start pushes it");
    hub.stop_client();
    hub.update_config(|c| c.network.pair_again = true).unwrap();
    assert!(!hub.new_key().unwrap().pair_again, "New key clears it");
}

#[test]
fn a_new_key_that_cannot_save_the_config_changes_and_stops_nothing() {
    let (hub, _page, dir) = new_hub(free_port());
    listen_to_nobody(&hub);
    let key = hub.identity().unwrap().public;
    let on_disk = std::fs::read(dir.path().join("maya").join(maya_core::net::identity::FILE)).unwrap();
    // A directory where the config file should be: every save fails.
    hub.settings.lock().unwrap().path = dir.path().to_path_buf();
    assert!(hub.new_key().is_err());
    assert_eq!(hub.status().role, NetworkRole::Assistant, "still listening");
    assert!(hub.client.lock().unwrap().is_some(), "the client still runs");
    assert_eq!(hub.identity().unwrap().public, key);
    assert_eq!(std::fs::read(dir.path().join("maya").join(maya_core::net::identity::FILE)).unwrap(), on_disk);
    hub.stop_client();
}

#[test]
fn a_new_key_that_cannot_write_the_key_file_restores_the_config() {
    let (hub, _page, dir) = new_hub(free_port());
    listen_to_nobody(&hub);
    let key = hub.identity().unwrap().public;
    // Replacing the key renames a file over this path; a directory there makes that fail.
    let path = dir.path().join("maya").join(maya_core::net::identity::FILE);
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir_all(path.join("x")).unwrap();
    assert!(hub.new_key().is_err());
    let c = hub.settings.lock().unwrap().config.network.clone();
    assert_eq!((c.role, c.main_key.as_str()), (NetworkRole::Assistant, "a2V5"));
    assert!(hub.client.lock().unwrap().is_some(), "the client still runs");
    assert_eq!(hub.identity().unwrap().public, key, "the cached key is unchanged");
    hub.stop_client();
}

#[test]
fn a_join_with_an_unreadable_key_file_fails_before_it_changes_anything() {
    let (main, port, _) = desk();
    let (hub, _page, dir) = new_hub(free_port());
    hub.update_config(|c| {
        c.network.role = NetworkRole::Assistant;
        c.network.main_key = "a2V5".into();
    })
    .unwrap();
    let path = dir.path().join("maya").join(maya_core::net::identity::FILE);
    std::fs::create_dir_all(&path).unwrap();
    let code = main.open_pairing(maya_core::store::now_ms()).unwrap().0;
    assert!(hub.join("127.0.0.1", port, "Pixel", &code).is_err());
    let c = hub.settings.lock().unwrap().config.network.clone();
    assert_eq!((c.role, c.main_key.as_str()), (NetworkRole::Assistant, "a2V5"));
    assert!(main.status().assistants.is_empty(), "no pairing was attempted");
    main.stop();
}
