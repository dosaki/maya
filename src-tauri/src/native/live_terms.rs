//! The terminals this Maya hosts, served to viewers on other machines by
//! session id. Bytes cross as base64 strings.

use super::host::{Frame, Sink};
use super::HOST;
use crate::native_app::{term_for_pid, NOT_RUNNING};
use crate::store::now_ms;
use crate::AppState;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use maya_core::net::live::{EventSink, LiveTerms, TermEvent, TermOpened, TermView, REMOTE_LINES};
use tauri::{AppHandle, Manager};

/// The Native terminals of this Maya's sessions, as the network sees them.
pub struct NativeTerms {
    app: AppHandle,
}

impl NativeTerms {
    pub fn new(app: AppHandle) -> NativeTerms {
        NativeTerms { app }
    }
}

/// What a viewer on another machine is told of a host frame.
fn event_of(frame: Frame) -> TermEvent {
    match frame {
        Frame::Output(bytes) => TermEvent::Output { data: STANDARD.encode(bytes) },
        Frame::Size { rows, cols } => TermEvent::Size { rows, cols },
        Frame::Exit => TermEvent::Exit,
    }
}

impl LiveTerms for NativeTerms {
    fn attach(&self, session: &str, sink: EventSink) -> Result<TermOpened, String> {
        let state = self.app.state::<AppState>();
        // The store lock ends with this statement: attaching must not hold it.
        let (card, tree) = {
            let mut store = state.store.lock().unwrap();
            (store.card_for(session, now_ms()), store.process_tree())
        };
        let term = card.and_then(|c| term_for_pid(&tree, c.pid)).ok_or(NOT_RUNNING)?;
        let sink: Sink = Box::new(move |frame| sink(event_of(frame)));
        let attached = HOST.attach(term, REMOTE_LINES, sink)?;
        Ok(TermOpened { view: TermView { term: attached.term, viewer: attached.viewer }, snapshot: attached.snapshot, rows: attached.rows, cols: attached.cols })
    }

    fn input(&self, view: &TermView, data: &[u8]) -> Result<(), String> {
        HOST.enqueue(view.term, data)
    }

    fn detach(&self, view: &TermView) {
        HOST.detach(view.term, view.viewer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_frames_become_wire_events() {
        assert_eq!(event_of(Frame::Output(vec![0xC3, 0xA9])), TermEvent::Output { data: "w6k=".into() });
        assert_eq!(event_of(Frame::Exit), TermEvent::Exit);
        assert_eq!(event_of(Frame::Size { rows: 30, cols: 100 }), TermEvent::Size { rows: 30, cols: 100 });
    }
}
