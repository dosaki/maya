// A man in the middle between a real assistant and a real main, at the
// WebSocket level: it relays every frame and does one thing wrong to the
// frame numbered `at` in one direction. Each kind of tampering must close
// the connection and run nothing.

use super::*;
use crate::net::identity::Keypair;
use std::net::TcpListener;
use std::sync::mpsc;

#[derive(Clone, Copy, Debug)]
enum Harm {
    /// Flip the last byte of the frame.
    Flip,
    /// Send the frame twice.
    Replay,
    /// Hold the frame and send the next one first.
    Reorder,
    /// Insert a plain-text command before the frame.
    InjectText,
    /// Insert 64 random bytes as a binary frame before it.
    InjectBinary,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Way {
    /// Main to assistant.
    Down,
    /// Assistant to main.
    Up,
}

/// What to forward for frame `m`, the `count`th going `dir`: harmed when it
/// is the chosen one, and a frame held back for `Reorder` after the next.
fn relay(m: Message, dir: Way, count: &mut usize, target: (Way, usize, Harm), held: &mut Option<Message>, fired: &AtomicBool) -> Vec<Message> {
    let (way, at, harm) = target;
    let mut out = vec![];
    if dir == way && *count == at {
        fired.store(true, Ordering::SeqCst);
        match harm {
            Harm::Flip => {
                // The frame keeps its type, so only its bytes differ.
                out.push(match m {
                    Message::Text(t) => {
                        let mut s = t.to_string();
                        let last = s.pop().unwrap();
                        s.push(if last == 'x' { 'y' } else { 'x' });
                        Message::text(s)
                    }
                    m => {
                        let mut b = m.into_data().to_vec();
                        *b.last_mut().unwrap() ^= 1;
                        Message::binary(b)
                    }
                });
            }
            Harm::Replay => out.extend([m.clone(), m]),
            Harm::Reorder => *held = Some(m),
            Harm::InjectText => out.extend([Message::text(encode(&Down::Command { id: 99, kind: CommandKind::Compact { session: "r1".into() } })), m]),
            Harm::InjectBinary => out.extend([Message::binary(crate::net::protocol::random(64)), m]),
        }
    } else {
        out.push(m);
        if dir == way {
            if let Some(h) = held.take() {
                out.push(h);
            }
        }
    }
    *count += 1;
    out
}

/// Listens on a free port and relays one connection to `main_port`,
/// harming frame `at` (counted per direction from 0) going `way`. The flag
/// is set when the harm is done.
fn proxy(main_port: u16, way: Way, at: usize, harm: Harm) -> (u16, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let fired = Arc::new(AtomicBool::new(false));
    let did = fired.clone();
    std::thread::spawn(move || {
        let (inbound, _) = listener.accept().unwrap();
        inbound.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        let mut assistant = tungstenite::accept(inbound).unwrap();
        let (mut main, _) = tungstenite::connect(format!("ws://127.0.0.1:{main_port}/")).unwrap();
        if let MaybeTlsStream::Plain(s) = main.get_ref() {
            s.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        }
        let target = (way, at, harm);
        let (mut ups, mut downs, mut held) = (0usize, 0usize, None);
        let end = Instant::now() + Duration::from_secs(30);
        while Instant::now() < end {
            match assistant.read() {
                Ok(m) if m.is_close() => return,
                Ok(m) => {
                    for m in relay(m, Way::Up, &mut ups, target, &mut held, &did) {
                        if main.send(m).is_err() {
                            return;
                        }
                    }
                }
                Err(e) if crate::net::secure::is_timeout(&e) => {}
                Err(_) => return,
            }
            match main.read() {
                Ok(m) if m.is_close() => return,
                Ok(m) => {
                    for m in relay(m, Way::Down, &mut downs, target, &mut held, &did) {
                        if assistant.send(m).is_err() {
                            return;
                        }
                    }
                }
                Err(e) if crate::net::secure::is_timeout(&e) => {}
                Err(_) => return,
            }
        }
    });
    (port, fired)
}

/// Pairs a Maya, then runs a real client for it through a harming proxy,
/// while the main sends it two commands, one after the other.
/// What a run through the proxy showed.
struct Outcome {
    /// The client's result.
    out: Result<(), String>,
    /// The commands it ran.
    ran: Vec<CommandKind>,
    /// Whether the proxy did its harm.
    fired: bool,
    /// Whether the main kept any card from the client's board.
    cards: bool,
}

fn through(way: Way, at: usize, harm: Harm) -> Outcome {
    let (handle, port) = test_server();
    let me = Keypair::generate();
    let (ws, id, _) = pair_as(&handle, port, &me, "laptop", &[]);
    drop(ws);
    wait_until(|| !handle.status().assistants.iter().any(|a| a.connected));
    let (via, fired) = proxy(port, way, at, harm);
    let mut config = assistant_config(via, &handle);
    config.name = "laptop".into();
    let exec = Arc::new(FakeExec::default());
    let stop = Arc::new(AtomicBool::new(false));
    let (e, s, k) = (exec.clone(), stop.clone(), me.clone());
    let run = std::thread::spawn(move || client::run_once(&config, &k, e, Arc::new(FakeClientNotify::default()), &s, None));
    let began = Instant::now();
    let shared = handle.shared.clone();
    let id2 = id.clone();
    std::thread::spawn(move || {
        let end = Instant::now() + Duration::from_secs(3);
        while Instant::now() < end && !shared.lock().unwrap().conns.contains_key(&id2) {
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = send_command_to(&shared, &id2, CommandKind::Rename { session: "r1".into(), name: "n".into() }, Duration::from_secs(2));
        let _ = send_command_to(&shared, &id2, CommandKind::Compact { session: "r1".into() }, Duration::from_secs(2));
    });
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || tx.send((run.join().unwrap(), Instant::now())));
    let (out, returned) = rx.recv_timeout(Duration::from_secs(20)).expect("the client never returned");
    assert!(returned - began < Duration::from_secs(10), "the client returned only when the proxy gave up");
    stop.store(true, Ordering::SeqCst);
    handle.stop();
    let ran = exec.kinds.lock().unwrap().clone();
    let cards = handle.boards().iter().any(|b| !b.cards.is_empty());
    Outcome { out, ran, fired: fired.load(Ordering::SeqCst), cards }
}

// Frame numbers: going Up, 0 is the IK message and 1 is `hello`, 2 the
// first board. Going Down, 0 is the IK answer, 1 is `welcome`, 2 the first
// command and 3 the second.

#[test]
fn a_changed_byte_either_way_closes_the_connection() {
    for way in [Way::Up, Way::Down] {
        let o = through(way, 2, Harm::Flip);
        assert!(o.fired, "{way:?}: the frame was changed");
        assert!(o.out.is_err(), "{way:?}: {:?}", o.out);
        match way {
            Way::Down => assert!(o.ran.is_empty(), "a changed command never runs"),
            Way::Up => assert!(!o.cards, "the main never took a changed board"),
        }
    }
}

#[test]
fn a_replayed_or_reordered_frame_closes_the_connection() {
    // Replay `welcome`; reorder the two commands, so the second arrives first.
    for (harm, at) in [(Harm::Replay, 1), (Harm::Reorder, 2)] {
        let o = through(Way::Down, at, harm);
        assert!(o.fired, "{harm:?}: the harm was done");
        assert!(o.out.is_err(), "{harm:?}: {:?}", o.out);
        assert!(o.ran.is_empty(), "{harm:?}: nothing runs");
    }
}

#[test]
fn an_injected_command_never_runs() {
    for harm in [Harm::InjectText, Harm::InjectBinary] {
        let o = through(Way::Down, 2, harm);
        assert!(o.fired, "{harm:?}: the frame was injected");
        assert!(o.out.is_err(), "{harm:?}: {:?}", o.out);
        assert!(o.ran.is_empty(), "{harm:?}: neither the injected command nor the real one after it runs");
    }
}

#[test]
fn an_untouched_relay_works() {
    // The control: the same path with no harm connects and runs the command.
    let (handle, port) = test_server();
    let me = Keypair::generate();
    let (ws, id, _) = pair_as(&handle, port, &me, "laptop", &[]);
    drop(ws);
    wait_until(|| !handle.status().assistants.iter().any(|a| a.connected));
    let (via, _) = proxy(port, Way::Down, usize::MAX, Harm::Flip);
    let config = assistant_config(via, &handle);
    let exec = Arc::new(FakeExec::default());
    let stop = Arc::new(AtomicBool::new(false));
    let (e, s, k) = (exec.clone(), stop.clone(), me.clone());
    let run = std::thread::spawn(move || client::run_once(&config, &k, e, Arc::new(FakeClientNotify::default()), &s, None));
    wait_until(|| handle.status().assistants.iter().any(|a| a.id == id && a.connected));
    send_command_to(&handle.shared, &id, CommandKind::Compact { session: "r1".into() }, Duration::from_secs(5)).unwrap();
    assert_eq!(*exec.kinds.lock().unwrap(), [CommandKind::Compact { session: "r1".into() }]);
    stop.store(true, Ordering::SeqCst);
    let _ = run.join();
    handle.stop();
}
