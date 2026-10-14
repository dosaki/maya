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
fn relay(m: Message, dir: Way, count: &mut usize, target: (Way, usize, Harm), held: &mut Option<Message>) -> Vec<Message> {
    let (way, at, harm) = target;
    let mut out = vec![];
    if dir == way && *count == at {
        match harm {
            Harm::Flip => {
                let mut b = m.into_data().to_vec();
                *b.last_mut().unwrap() ^= 1;
                out.push(Message::binary(b));
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
/// harming frame `at` (counted per direction from 0) going `way`.
fn proxy(main_port: u16, way: Way, at: usize, harm: Harm) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
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
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            match assistant.read() {
                Ok(m) if m.is_close() => return,
                Ok(m) => {
                    for m in relay(m, Way::Up, &mut ups, target, &mut held) {
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
                    for m in relay(m, Way::Down, &mut downs, target, &mut held) {
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
    port
}

/// Pairs a Maya, then runs a real client for it through a harming proxy.
/// Returns (the client's result, the commands it ran).
fn through(way: Way, at: usize, harm: Harm) -> (Result<(), String>, Vec<CommandKind>) {
    let (handle, port) = test_server();
    let me = Keypair::generate();
    let (ws, id, _) = pair_as(&handle, port, &me, "laptop", &[]);
    drop(ws);
    wait_until(|| !handle.status().assistants.iter().any(|a| a.connected));
    let via = proxy(port, way, at, harm);
    let mut config = assistant_config(via, &handle);
    config.name = "laptop".into();
    let exec = Arc::new(FakeExec::default());
    let stop = Arc::new(AtomicBool::new(false));
    let (e, s, k) = (exec.clone(), stop.clone(), me.clone());
    let run = std::thread::spawn(move || client::run_once(&config, &k, e, Arc::new(FakeClientNotify::default()), &s, None));
    // Once connected, the main sends one command: the frame after `welcome`.
    let shared = handle.shared.clone();
    let id2 = id.clone();
    std::thread::spawn(move || {
        let end = Instant::now() + Duration::from_secs(3);
        while Instant::now() < end && !shared.lock().unwrap().conns.contains_key(&id2) {
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = send_command_to(&shared, &id2, CommandKind::Rename { session: "r1".into(), name: "n".into() }, Duration::from_secs(2));
    });
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || tx.send(run.join().unwrap()));
    let out = rx.recv_timeout(Duration::from_secs(20)).unwrap_or(Ok(()));
    stop.store(true, Ordering::SeqCst);
    handle.stop();
    let ran = exec.kinds.lock().unwrap().clone();
    (out, ran)
}

// Frame numbers: going Up, 0 is the IK message and 1 is `hello`, 2 the
// first board. Going Down, 0 is the IK answer, 1 is `welcome`, 2 the command.

#[test]
fn a_changed_byte_either_way_closes_the_connection() {
    for way in [Way::Up, Way::Down] {
        let (out, ran) = through(way, 2, Harm::Flip);
        assert!(out.is_err(), "{way:?}: {out:?}");
        if way == Way::Down {
            assert!(ran.is_empty(), "a changed command never runs");
        }
    }
}

#[test]
fn a_replayed_or_reordered_frame_closes_the_connection() {
    for harm in [Harm::Replay, Harm::Reorder] {
        let (out, ran) = through(Way::Down, 1, harm);
        assert!(out.is_err(), "{harm:?}: {out:?}");
        assert!(ran.is_empty(), "{harm:?}: nothing runs");
    }
}

#[test]
fn an_injected_command_never_runs() {
    for harm in [Harm::InjectText, Harm::InjectBinary] {
        let (out, ran) = through(Way::Down, 2, harm);
        assert!(out.is_err(), "{harm:?}: {out:?}");
        assert!(ran.is_empty(), "{harm:?}: neither the injected command nor the real one after it runs");
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
    let via = proxy(port, Way::Down, usize::MAX, Harm::Flip);
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
