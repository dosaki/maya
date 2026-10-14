//! The secure channel between two Mayas (protocol 2). Pairing is SPAKE2
//! over the six-digit code, then a Noise `XXpsk3` handshake keyed by it;
//! every later connection is a Noise `IK` handshake against the main's
//! pinned key. After either, each WebSocket binary frame holds one or more
//! Noise transport messages, `[u16 length][message]`, so every byte is
//! encrypted and authenticated and a changed, replayed, reordered or
//! injected frame fails to open.

use super::identity::{Keypair, KEY_LEN};
use super::protocol::MAX_FRAME;
use snow::{Builder, HandshakeState, TransportState};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use std::io::{Read, Write};
use std::time::Instant;
use tungstenite::{Message, WebSocket};

const PAIR_PATTERN: &str = "Noise_XXpsk3_25519_ChaChaPoly_SHA256";
const CONNECT_PATTERN: &str = "Noise_IK_25519_ChaChaPoly_SHA256";
const PROLOGUE: &[u8] = b"maya protocol 2";
/// Noise's limit on one message, its 16-byte tag included.
const NOISE_MAX: usize = 65_535;
const TAG: usize = 16;
/// The most plaintext one Noise message carries.
const CHUNK: usize = NOISE_MAX - TAG;

/// The joining Maya's first frame starts with one of these.
pub const MODE_PAIR: u8 = 1;
pub const MODE_CONNECT: u8 = 2;
/// Every frame the main sends during the handshake starts with one of these.
pub const KIND_MESSAGE: u8 = 0x00;
/// A plain refusal, its reason as UTF-8 after it. Never authenticated, so
/// it only ever becomes an error message.
pub const KIND_REFUSAL: u8 = 0xFF;
pub const WRONG_CODE_REASON: &str = "wrong or expired pairing code";
pub const UNKNOWN_KEY_REASON: &str = "unknown key";
pub const FORGED: &str = "A frame failed authentication; connection closed.";
const SPAKE_LEN: usize = 33;
const JOINER: &[u8] = b"maya-join";
const MAIN: &[u8] = b"maya-main";

#[derive(Debug, Clone, PartialEq)]
pub enum ChannelError {
    /// A frame did not open: changed, replayed, reordered, injected or garbled.
    Forged,
    Io(String),
}

impl std::fmt::Display for ChannelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChannelError::Forged => f.write_str(FORGED),
            ChannelError::Io(e) => f.write_str(e),
        }
    }
}

/// What one read of a `Channel` gave.
#[derive(Debug, Clone, PartialEq)]
pub enum Got {
    Text(String),
    /// A ping or pong: the peer is alive, nothing to decode.
    Control,
    /// The read timed out (the connection loop's tick).
    Idle,
    Closed,
}

/// Why a handshake did not complete.
#[derive(Debug, Clone, PartialEq)]
pub enum Fail {
    /// The other side spoke plain text first: a protocol 1 Maya.
    OldPeer,
    /// The main refused, in plain text, with this reason.
    Refused(String),
    /// A handshake message did not decrypt: a wrong code, a changed key, tampering.
    Crypto,
    /// Timed out, closed, stopped or an I/O error.
    Io(String),
    /// A frame of no known shape.
    Malformed,
}

/// What the main learned from a handshake.
#[derive(Debug, Clone, PartialEq)]
pub struct Accepted {
    /// The other Maya's public key.
    pub key: [u8; KEY_LEN],
    /// It paired (with the code) rather than connected (with its key).
    pub paired: bool,
}

pub(crate) fn is_timeout(e: &tungstenite::Error) -> bool {
    matches!(e, tungstenite::Error::Io(io) if matches!(io.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut))
}

/// `plain` as one frame: Noise messages of at most `CHUNK` bytes of
/// plaintext each, each after its length. Empty plaintext is one message.
pub fn seal_frame(noise: &mut TransportState, plain: &[u8]) -> Result<Vec<u8>, String> {
    let chunks: Vec<&[u8]> = if plain.is_empty() { vec![&[][..]] } else { plain.chunks(CHUNK).collect() };
    let mut out = Vec::with_capacity(plain.len() + chunks.len() * (TAG + 2));
    let mut buf = vec![0u8; NOISE_MAX];
    for chunk in chunks {
        let n = noise.write_message(chunk, &mut buf).map_err(|e| format!("could not encrypt: {e}"))?;
        out.extend_from_slice(&(n as u16).to_be_bytes());
        out.extend_from_slice(&buf[..n]);
    }
    Ok(out)
}

/// The text a frame holds; anything that does not open is `Forged`.
pub fn open_frame(noise: &mut TransportState, frame: &[u8]) -> Result<String, ChannelError> {
    if frame.is_empty() {
        return Err(ChannelError::Forged);
    }
    let mut plain = Vec::new();
    let mut buf = vec![0u8; NOISE_MAX];
    let mut rest = frame;
    while !rest.is_empty() {
        let len = rest.get(..2).ok_or(ChannelError::Forged)?;
        let n = u16::from_be_bytes([len[0], len[1]]) as usize;
        let body = rest.get(2..2 + n).ok_or(ChannelError::Forged)?;
        let k = noise.read_message(body, &mut buf).map_err(|_| ChannelError::Forged)?;
        plain.extend_from_slice(&buf[..k]);
        if plain.len() > MAX_FRAME {
            return Err(ChannelError::Io(format!("frame too large ({} bytes)", plain.len())));
        }
        rest = &rest[2 + n..];
    }
    String::from_utf8(plain).map_err(|_| ChannelError::Forged)
}

/// A WebSocket after its handshake: JSON text in, JSON text out, sealed.
pub struct Channel<S> {
    pub ws: WebSocket<S>,
    noise: TransportState,
}

impl<S: Read + Write> Channel<S> {
    pub fn new(ws: WebSocket<S>, noise: TransportState) -> Channel<S> {
        Channel { ws, noise }
    }

    pub fn send(&mut self, text: &str) -> Result<(), String> {
        let frame = seal_frame(&mut self.noise, text.as_bytes())?;
        self.ws.send(Message::binary(frame)).map_err(|e| e.to_string())
    }

    /// Queues a frame without flushing; `flush` sends what is queued.
    pub fn write(&mut self, text: &str) -> Result<(), String> {
        let frame = seal_frame(&mut self.noise, text.as_bytes())?;
        self.ws.write(Message::binary(frame)).map_err(|e| e.to_string())
    }

    pub fn flush(&mut self) -> Result<(), String> {
        self.ws.flush().map_err(|e| e.to_string())
    }

    /// One read. A text frame after the handshake is never genuine.
    pub fn read(&mut self) -> Result<Got, ChannelError> {
        match self.ws.read() {
            Ok(Message::Binary(b)) => open_frame(&mut self.noise, &b).map(Got::Text),
            Ok(Message::Text(_)) => Err(ChannelError::Forged),
            Ok(Message::Close(_)) => Ok(Got::Closed),
            Ok(_) => Ok(Got::Control),
            Err(e) if is_timeout(&e) => Ok(Got::Idle),
            Err(e) => Err(ChannelError::Io(e.to_string())),
        }
    }
}

fn builder<'a>(pattern: &str, me: &'a Keypair) -> Result<Builder<'a>, Fail> {
    let params = pattern.parse().map_err(|_| Fail::Malformed)?;
    Builder::new(params).local_private_key(me.private()).and_then(|b| b.prologue(PROLOGUE)).map_err(|_| Fail::Malformed)
}

fn send_frame<S: Read + Write>(ws: &mut WebSocket<S>, bytes: Vec<u8>) -> Result<(), Fail> {
    ws.send(Message::binary(bytes)).map_err(|e| Fail::Io(e.to_string()))
}

/// The main's handshake message: `KIND_MESSAGE` then `body`.
fn send_message<S: Read + Write>(ws: &mut WebSocket<S>, body: &[u8]) -> Result<(), Fail> {
    let mut frame = vec![KIND_MESSAGE];
    frame.extend_from_slice(body);
    send_frame(ws, frame)
}

/// The main's plain refusal, during the handshake only.
pub fn refuse<S: Read + Write>(ws: &mut WebSocket<S>, reason: &str) {
    let mut frame = vec![KIND_REFUSAL];
    frame.extend_from_slice(reason.as_bytes());
    let _ = send_frame(ws, frame);
}

/// The next binary frame by `deadline`; a text frame is a protocol 1 peer.
fn read_frame<S: Read + Write>(ws: &mut WebSocket<S>, deadline: Instant, stopped: &dyn Fn() -> bool) -> Result<Vec<u8>, Fail> {
    loop {
        if stopped() {
            return Err(Fail::Io("stopped".into()));
        }
        match ws.read() {
            Ok(Message::Binary(b)) => return Ok(b.to_vec()),
            Ok(Message::Text(_)) => return Err(Fail::OldPeer),
            Ok(Message::Close(_)) => return Err(Fail::Io("closed".into())),
            Ok(_) => {}
            Err(e) if is_timeout(&e) => {
                if Instant::now() > deadline {
                    return Err(Fail::Io("timed out".into()));
                }
            }
            Err(e) => return Err(Fail::Io(e.to_string())),
        }
    }
}

/// The main's next handshake frame: its message, or its refusal as `Refused`.
fn from_main<S: Read + Write>(ws: &mut WebSocket<S>, deadline: Instant, stopped: &dyn Fn() -> bool) -> Result<Vec<u8>, Fail> {
    let frame = read_frame(ws, deadline, stopped)?;
    match frame.split_first() {
        Some((&KIND_MESSAGE, body)) => Ok(body.to_vec()),
        Some((&KIND_REFUSAL, reason)) => Err(Fail::Refused(String::from_utf8_lossy(reason).chars().filter(|c| !c.is_control()).take(80).collect())),
        _ => Err(Fail::Malformed),
    }
}

/// `hs`'s next message, empty payload.
fn message(hs: &mut HandshakeState) -> Result<Vec<u8>, Fail> {
    let mut buf = vec![0u8; NOISE_MAX];
    let n = hs.write_message(&[], &mut buf).map_err(|_| Fail::Crypto)?;
    buf.truncate(n);
    Ok(buf)
}

fn absorb(hs: &mut HandshakeState, msg: &[u8]) -> Result<(), Fail> {
    let mut buf = vec![0u8; NOISE_MAX];
    hs.read_message(msg, &mut buf).map(|_| ()).map_err(|_| Fail::Crypto)
}

/// The transport and the other side's static key, once the handshake is done.
fn finish(hs: HandshakeState) -> Result<(TransportState, [u8; KEY_LEN]), Fail> {
    let key: [u8; KEY_LEN] = hs.get_remote_static().and_then(|k| k.try_into().ok()).ok_or(Fail::Malformed)?;
    let t = hs.into_transport_mode().map_err(|_| Fail::Crypto)?;
    Ok((t, key))
}

fn spake_key(spake: Spake2<Ed25519Group>, theirs: &[u8]) -> Result<[u8; 32], Fail> {
    let key = spake.finish(theirs).map_err(|_| Fail::Crypto)?;
    key.try_into().map_err(|_| Fail::Crypto)
}

fn password(code: &str) -> Password {
    Password::new(code.trim().as_bytes())
}

/// The joining Maya's pairing: SPAKE2 over `code`, then `XXpsk3` keyed by
/// it, then the main's word that the code matched. Returns the transport
/// and the main's public key.
pub fn pair<S: Read + Write>(ws: &mut WebSocket<S>, me: &Keypair, code: &str, deadline: Instant, stopped: &dyn Fn() -> bool) -> Result<(TransportState, [u8; KEY_LEN]), Fail> {
    let (spake, ours) = Spake2::<Ed25519Group>::start_a(&password(code), &Identity::new(JOINER), &Identity::new(MAIN));
    let mut first = vec![MODE_PAIR];
    first.extend_from_slice(&ours);
    send_frame(ws, first)?;
    let psk = spake_key(spake, &from_main(ws, deadline, stopped)?)?;
    let mut hs = builder(PAIR_PATTERN, me)?.psk(3, &psk).and_then(|b| b.build_initiator()).map_err(|_| Fail::Malformed)?;
    send_frame(ws, message(&mut hs)?)?;
    absorb(&mut hs, &from_main(ws, deadline, stopped)?)?;
    send_frame(ws, message(&mut hs)?)?;
    // The main checks the code with that last message and says so.
    from_main(ws, deadline, stopped)?;
    finish(hs)
}

/// An assistant's connection: `IK` against the main's pinned key.
pub fn connect<S: Read + Write>(ws: &mut WebSocket<S>, me: &Keypair, main: &[u8; KEY_LEN], deadline: Instant, stopped: &dyn Fn() -> bool) -> Result<TransportState, Fail> {
    let mut hs = builder(CONNECT_PATTERN, me)?.remote_public_key(main).and_then(|b| b.build_initiator()).map_err(|_| Fail::Malformed)?;
    let mut first = vec![MODE_CONNECT];
    first.extend_from_slice(&message(&mut hs)?);
    send_frame(ws, first)?;
    absorb(&mut hs, &from_main(ws, deadline, stopped)?)?;
    finish(hs).map(|(t, _)| t)
}

/// The main's side of either handshake. `code` gives the open pairing code
/// (or why there is none) and is called only for a pairing, before SPAKE2.
pub fn accept<S: Read + Write>(ws: &mut WebSocket<S>, me: &Keypair, deadline: Instant, stopped: &dyn Fn() -> bool, code: &mut dyn FnMut() -> Result<String, String>) -> Result<(TransportState, Accepted), Fail> {
    let first = read_frame(ws, deadline, stopped)?;
    match first.split_first() {
        Some((&MODE_PAIR, theirs)) if theirs.len() == SPAKE_LEN => {
            let code = match code() {
                Ok(c) => c,
                Err(reason) => {
                    refuse(ws, &reason);
                    return Err(Fail::Refused(reason));
                }
            };
            let (spake, ours) = Spake2::<Ed25519Group>::start_b(&password(&code), &Identity::new(JOINER), &Identity::new(MAIN));
            send_message(ws, &ours)?;
            let psk = spake_key(spake, theirs)?;
            let mut hs = builder(PAIR_PATTERN, me)?.psk(3, &psk).and_then(|b| b.build_responder()).map_err(|_| Fail::Malformed)?;
            absorb(&mut hs, &read_frame(ws, deadline, stopped)?)?;
            send_message(ws, &message(&mut hs)?)?;
            // Only here does the joiner prove it knows the code (`psk3`).
            if let Err(f) = read_frame(ws, deadline, stopped).and_then(|m| absorb(&mut hs, &m)) {
                if f == Fail::Crypto {
                    refuse(ws, WRONG_CODE_REASON);
                }
                return Err(f);
            }
            send_message(ws, &[])?;
            let (t, key) = finish(hs)?;
            Ok((t, Accepted { key, paired: true }))
        }
        Some((&MODE_CONNECT, msg)) => {
            let mut hs = builder(CONNECT_PATTERN, me)?.build_responder().map_err(|_| Fail::Malformed)?;
            if absorb(&mut hs, msg).is_err() {
                // Sealed to another key: the assistant pinned someone else.
                refuse(ws, UNKNOWN_KEY_REASON);
                return Err(Fail::Crypto);
            }
            send_message(ws, &message(&mut hs)?)?;
            let (t, key) = finish(hs)?;
            Ok((t, Accepted { key, paired: false }))
        }
        _ => Err(Fail::Malformed),
    }
}

#[cfg(test)]
pub(crate) fn transport_pair() -> (TransportState, TransportState) {
    let (a, m) = (Keypair::generate(), Keypair::generate());
    let mut i = builder(CONNECT_PATTERN, &a).unwrap().remote_public_key(&m.public).unwrap().build_initiator().unwrap();
    let mut r = builder(CONNECT_PATTERN, &m).unwrap().build_responder().unwrap();
    absorb(&mut r, &message(&mut i).unwrap()).unwrap();
    absorb(&mut i, &message(&mut r).unwrap()).unwrap();
    (i.into_transport_mode().unwrap(), r.into_transport_mode().unwrap())
}

#[cfg(test)]
mod frame_tests {
    use super::*;

    #[test]
    fn a_frame_round_trips_small_empty_and_twenty_megabytes() {
        let (mut a, mut b) = transport_pair();
        for text in [String::new(), "hello".to_string(), "x".repeat(20 * 1024 * 1024)] {
            let frame = seal_frame(&mut a, text.as_bytes()).unwrap();
            assert_eq!(open_frame(&mut b, &frame).unwrap(), text);
        }
    }

    #[test]
    fn a_changed_byte_a_replay_and_a_reorder_are_forged() {
        let (mut a, mut b) = transport_pair();
        let mut flipped = seal_frame(&mut a, b"one").unwrap();
        *flipped.last_mut().unwrap() ^= 1;
        assert_eq!(open_frame(&mut b, &flipped), Err(ChannelError::Forged));

        let (mut a, mut b) = transport_pair();
        let first = seal_frame(&mut a, b"one").unwrap();
        assert_eq!(open_frame(&mut b, &first).unwrap(), "one");
        assert_eq!(open_frame(&mut b, &first), Err(ChannelError::Forged), "a replay");

        let (mut a, mut b) = transport_pair();
        let _first = seal_frame(&mut a, b"one").unwrap();
        let second = seal_frame(&mut a, b"two").unwrap();
        assert_eq!(open_frame(&mut b, &second), Err(ChannelError::Forged), "out of order");
    }

    #[test]
    fn garbled_frames_are_forged_not_panics() {
        let (mut a, mut b) = transport_pair();
        assert_eq!(open_frame(&mut b, &[]), Err(ChannelError::Forged), "empty");
        assert_eq!(open_frame(&mut b, &[0x00]), Err(ChannelError::Forged), "half a length");
        assert_eq!(open_frame(&mut b, &[0xFF, 0xFF, 1, 2, 3]), Err(ChannelError::Forged), "a length past the end");
        let bad_utf8 = seal_frame(&mut a, &[0xC3, 0x28]).unwrap();
        assert_eq!(open_frame(&mut b, &bad_utf8), Err(ChannelError::Forged), "not UTF-8");
    }

    #[test]
    fn a_large_frame_is_split_into_noise_sized_messages() {
        let (mut a, _) = transport_pair();
        let frame = seal_frame(&mut a, &vec![b'x'; CHUNK * 2 + 10]).unwrap();
        let first = u16::from_be_bytes([frame[0], frame[1]]) as usize;
        assert_eq!(first, NOISE_MAX, "a full chunk plus its tag");
        assert_eq!(frame.len(), 3 * 2 + (CHUNK * 2 + 10) + 3 * TAG);
    }
}

#[cfg(test)]
mod handshake_tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};
    use std::time::Duration;

    type Ws = WebSocket<TcpStream>;

    /// A WebSocket pair over loopback: (joiner side, main side).
    fn ws_pair() -> (Ws, Ws) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let joiner = std::thread::spawn(move || {
            let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
            tungstenite::client::client(format!("ws://127.0.0.1:{port}/"), stream).unwrap().0
        });
        let (stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
        let main = tungstenite::accept(stream).unwrap();
        (joiner.join().unwrap(), main)
    }

    fn soon() -> Instant {
        Instant::now() + Duration::from_secs(5)
    }

    type MainOut = (Ws, Result<(TransportState, Accepted), Fail>);

    /// The main's side on a thread, with `code` as the open code or the refusal.
    fn main_side(mut ws: Ws, me: Keypair, code: Result<String, String>) -> std::thread::JoinHandle<MainOut> {
        std::thread::spawn(move || {
            let mut code = Some(code);
            let out = accept(&mut ws, &me, soon(), &|| false, &mut || code.take().expect("asked once"));
            (ws, out)
        })
    }

    fn read_text<S: Read + Write>(c: &mut Channel<S>) -> String {
        let end = soon();
        loop {
            match c.read().unwrap() {
                Got::Text(t) => return t,
                _ if Instant::now() > end => panic!("nothing within 5 s"),
                _ => {}
            }
        }
    }

    #[test]
    fn the_same_code_pairs_and_both_learn_each_others_key() {
        let (mut j, m) = ws_pair();
        let (joiner, main) = (Keypair::generate(), Keypair::generate());
        let t = main_side(m, main.clone(), Ok("123456".into()));
        let (jt, main_key) = pair(&mut j, &joiner, " 123456 ", soon(), &|| false).unwrap();
        let (m, out) = t.join().unwrap();
        let (mt, accepted) = out.unwrap();
        assert_eq!(main_key, main.public);
        assert_eq!(accepted, Accepted { key: joiner.public, paired: true });
        let (mut jc, mut mc) = (Channel::new(j, jt), Channel::new(m, mt));
        jc.send("hello").unwrap();
        assert_eq!(read_text(&mut mc), "hello");
        mc.send("welcome").unwrap();
        assert_eq!(read_text(&mut jc), "welcome");
    }

    #[test]
    fn a_wrong_code_fails_on_the_main_and_the_joiner_hears_why() {
        let (mut j, m) = ws_pair();
        let t = main_side(m, Keypair::generate(), Ok("654321".into()));
        assert_eq!(pair(&mut j, &Keypair::generate(), "123456", soon(), &|| false).unwrap_err(), Fail::Refused(WRONG_CODE_REASON.into()));
        assert_eq!(t.join().unwrap().1.unwrap_err(), Fail::Crypto);
    }

    #[test]
    fn no_open_code_is_refused_before_any_spake() {
        let (mut j, m) = ws_pair();
        let t = main_side(m, Keypair::generate(), Err(WRONG_CODE_REASON.into()));
        assert_eq!(pair(&mut j, &Keypair::generate(), "123456", soon(), &|| false).unwrap_err(), Fail::Refused(WRONG_CODE_REASON.into()));
        assert_eq!(t.join().unwrap().1.unwrap_err(), Fail::Refused(WRONG_CODE_REASON.into()));
    }

    #[test]
    fn connecting_with_the_pinned_key_works_and_the_main_learns_who() {
        let (mut j, m) = ws_pair();
        let (assistant, main) = (Keypair::generate(), Keypair::generate());
        let t = main_side(m, main.clone(), Err("not pairing".into()));
        let jt = connect(&mut j, &assistant, &main.public, soon(), &|| false).unwrap();
        let (m, out) = t.join().unwrap();
        let (mt, accepted) = out.unwrap();
        assert_eq!(accepted, Accepted { key: assistant.public, paired: false });
        let (mut jc, mut mc) = (Channel::new(j, jt), Channel::new(m, mt));
        jc.send("hello").unwrap();
        assert_eq!(read_text(&mut mc), "hello");
    }

    #[test]
    fn a_main_without_the_pinned_key_refuses_by_name() {
        let (mut j, m) = ws_pair();
        let t = main_side(m, Keypair::generate(), Err("not pairing".into()));
        let pinned = Keypair::generate().public;
        assert_eq!(connect(&mut j, &Keypair::generate(), &pinned, soon(), &|| false).unwrap_err(), Fail::Refused(UNKNOWN_KEY_REASON.into()));
        assert_eq!(t.join().unwrap().1.unwrap_err(), Fail::Crypto);
    }

    #[test]
    fn a_text_frame_first_is_an_old_peer_both_ways() {
        let (mut j, mut m) = ws_pair();
        j.send(Message::text(r#"{"type":"hello","protocol":1}"#)).unwrap();
        assert_eq!(accept(&mut m, &Keypair::generate(), soon(), &|| false, &mut || panic!("no code for an old peer")).unwrap_err(), Fail::OldPeer);
        let (mut j, mut m) = ws_pair();
        let t = std::thread::spawn(move || {
            let end = soon();
            while Instant::now() < end && !matches!(m.read(), Ok(Message::Binary(_))) {}
            m.send(Message::text(r#"{"type":"challenge","nonce":"n"}"#)).unwrap();
            m
        });
        assert_eq!(connect(&mut j, &Keypair::generate(), &Keypair::generate().public, soon(), &|| false).unwrap_err(), Fail::OldPeer);
        drop(t.join());
    }

    #[test]
    fn silence_is_an_io_failure_by_the_deadline() {
        let (mut j, _m) = ws_pair();
        let out = connect(&mut j, &Keypair::generate(), &Keypair::generate().public, Instant::now() + Duration::from_millis(300), &|| false);
        assert!(matches!(out, Err(Fail::Io(_))), "{out:?}");
    }

    #[test]
    fn a_text_frame_after_the_handshake_is_forged() {
        let (mut j, m) = ws_pair();
        let main = Keypair::generate();
        let t = main_side(m, main.clone(), Err("not pairing".into()));
        let jt = connect(&mut j, &Keypair::generate(), &main.public, soon(), &|| false).unwrap();
        let (mut m, out) = t.join().unwrap();
        let _mt = out.unwrap().0;
        m.send(Message::text(r#"{"type":"command","id":1,"kind":"compact","session":"s"}"#)).unwrap();
        let mut jc = Channel::new(j, jt);
        let end = soon();
        let got = loop {
            match jc.read() {
                Ok(Got::Idle) if Instant::now() < end => {}
                other => break other,
            }
        };
        assert_eq!(got, Err(ChannelError::Forged));
    }
}
