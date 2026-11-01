//! The live terminal over the network: what a hosting Maya streams to a
//! viewer on another machine, and the seam the host implements.

use base64::Engine;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Lines of scrollback a remote viewer keeps (the snapshot it is sent covers the same).
pub const REMOTE_LINES: usize = 1_000;
/// How often a link with a live terminal on it is served.
pub const LIVE_TICK: Duration = Duration::from_millis(10);

/// How long a link waits between turns: `LIVE_TICK` while a terminal streams on it, else `idle`.
pub fn tick_for(live: bool, idle: Duration) -> Duration {
    if live { LIVE_TICK } else { idle }
}

/// One thing that happened in a hosted terminal, as a viewer sees it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TermEvent {
    /// The terminal as it is now: the bytes that redraw its screen and scrollback, base64, and its size.
    Attached { snapshot: String, rows: u16, cols: u16 },
    /// New output from the terminal, base64.
    Output { data: String },
    /// The terminal was resized.
    Size { rows: u16, cols: u16 },
    /// The session ended in the terminal.
    Exit,
    /// The terminal cannot be shown. An `Error` frame ends its subscription everywhere: nothing follows it.
    Error { message: String },
    /// The link to the hosting Maya dropped; attach again.
    Lost,
}

/// A `TermEvent` numbered in the order the viewer must apply it.
/// `Attached` is `seq` 0 and what follows counts from 1; output that
/// raced ahead of the snapshot carries a higher `seq` and is held until it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TermFrame {
    pub seq: u64,
    #[serde(flatten)]
    pub event: TermEvent,
}

/// Where a host sends a viewer's events; false means the viewer is gone.
pub type EventSink = Box<dyn Fn(TermEvent) -> bool + Send>;
/// Where `open` sends numbered frames; false means the viewer is gone.
pub type FrameSink = Box<dyn Fn(TermFrame) -> bool + Send>;

/// One viewer of one hosted terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TermView {
    pub term: u64,
    pub viewer: u64,
}

/// A terminal a viewer attached to: its screen now, and its size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TermOpened {
    pub view: TermView,
    pub snapshot: Vec<u8>,
    pub rows: u16,
    pub cols: u16,
}

/// The terminals a Maya hosts, as the network sees them.
pub trait LiveTerms: Send + Sync {
    /// Show the terminal of `session` to `sink`, from now on.
    fn attach(&self, session: &str, sink: EventSink) -> Result<TermOpened, String>;
    /// Type `data` into the terminal.
    fn input(&self, view: &TermView, data: &[u8]) -> Result<(), String>;
    /// Stop showing the terminal to this viewer.
    fn detach(&self, view: &TermView);
}

/// Attach to `session` and stream it to `out`: the snapshot as `seq` 0, then
/// each event numbered from 1. On an attach error, returns it and sends nothing.
pub fn open(terms: &dyn LiveTerms, session: &str, out: FrameSink) -> Result<TermView, String> {
    // One lock numbers and sends, so frames leave in `seq` order even from two threads.
    let out = Arc::new(Mutex::new((1u64, out)));
    let sink: EventSink = {
        let out = out.clone();
        Box::new(move |event| {
            let mut o = out.lock().unwrap_or_else(|e| e.into_inner());
            let seq = o.0;
            o.0 += 1;
            (o.1)(TermFrame { seq, event })
        })
    };
    let opened = terms.attach(session, sink)?;
    let snapshot = base64::engine::general_purpose::STANDARD.encode(&opened.snapshot);
    let o = out.lock().unwrap_or_else(|e| e.into_inner());
    (o.1)(TermFrame { seq: 0, event: TermEvent::Attached { snapshot, rows: opened.rows, cols: opened.cols } });
    Ok(opened.view)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host that keeps the sink it is given, and may call it before `attach` returns.
    struct Fake {
        sink: Mutex<Option<EventSink>>,
        early: bool,
        fail: bool,
    }

    impl LiveTerms for Fake {
        fn attach(&self, _session: &str, sink: EventSink) -> Result<TermOpened, String> {
            if self.fail {
                return Err("This session is not running in Maya.".into());
            }
            if self.early {
                sink(TermEvent::Output { data: "eA==".into() });
            }
            *self.sink.lock().unwrap() = Some(sink);
            Ok(TermOpened { view: TermView { term: 1, viewer: 2 }, snapshot: b"hi".to_vec(), rows: 40, cols: 120 })
        }
        fn input(&self, _view: &TermView, _data: &[u8]) -> Result<(), String> {
            Ok(())
        }
        fn detach(&self, _view: &TermView) {}
    }

    fn collecting() -> (FrameSink, Arc<Mutex<Vec<TermFrame>>>) {
        let got = Arc::new(Mutex::new(vec![]));
        let g = got.clone();
        (Box::new(move |f| { g.lock().unwrap().push(f); true }), got)
    }

    #[test]
    fn open_sends_the_snapshot_as_seq_0_and_numbers_what_follows_from_1() {
        let fake = Fake { sink: Mutex::new(None), early: true, fail: false };
        let (out, got) = collecting();
        let view = open(&fake, "s1", out).unwrap();
        assert_eq!(view, TermView { term: 1, viewer: 2 });
        assert!(fake.sink.lock().unwrap().as_ref().unwrap()(TermEvent::Size { rows: 30, cols: 100 }));
        // The early output precedes the snapshot in push order; the viewer reorders by seq.
        assert_eq!(
            *got.lock().unwrap(),
            vec![
                TermFrame { seq: 1, event: TermEvent::Output { data: "eA==".into() } },
                TermFrame { seq: 0, event: TermEvent::Attached { snapshot: "aGk=".into(), rows: 40, cols: 120 } },
                TermFrame { seq: 2, event: TermEvent::Size { rows: 30, cols: 100 } },
            ]
        );
    }

    #[test]
    fn open_returns_the_error_and_sends_nothing_when_attach_fails() {
        let fake = Fake { sink: Mutex::new(None), early: false, fail: true };
        let (out, got) = collecting();
        assert_eq!(open(&fake, "s1", out), Err("This session is not running in Maya.".to_string()));
        assert!(got.lock().unwrap().is_empty());
    }

    #[test]
    fn a_live_link_ticks_fast() {
        assert_eq!(tick_for(true, Duration::from_millis(100)), Duration::from_millis(10));
        assert_eq!(tick_for(false, Duration::from_millis(100)), Duration::from_millis(100));
    }
}
