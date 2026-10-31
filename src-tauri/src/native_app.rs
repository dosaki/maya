//! The commands the terminal viewer uses to attach to a Native session,
//! type into it, resize it and detach. Bytes cross as base64 strings.

use crate::native::host::{Frame, Sink, Unseen};
use crate::native::{screen, HOST};
use crate::AppState;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use maya_core::native::root_of;
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::State as TauriState;
use tauri::{AppHandle, Emitter};

/// What the viewer receives as the terminal runs.
#[derive(Serialize, Clone)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum NativeFrame {
    Output { data: String },
    Exit,
}

/// What `native_attach` hands back: the screen so far and where to send input.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeAttached {
    pub term: u64,
    pub viewer: u64,
    pub snapshot: String,
    pub rows: u16,
    pub cols: u16,
}

fn frame_of(frame: Frame) -> NativeFrame {
    match frame {
        Frame::Output(bytes) => NativeFrame::Output { data: STANDARD.encode(bytes) },
        Frame::Exit => NativeFrame::Exit,
    }
}

fn decode_input(data: &str) -> Result<Vec<u8>, String> {
    STANDARD.decode(data).map_err(|e| format!("Input is not base64: {e}"))
}

/// Attaches the viewer to the terminal `pid` runs in; frames go to `on_frame`.
/// Off the main thread: the snapshot can hold 10,000 lines.
#[tauri::command(async)]
pub fn native_attach(state: TauriState<AppState>, pid: i32, on_frame: Channel<NativeFrame>) -> Result<NativeAttached, String> {
    // The store lock ends with this statement: attaching must not hold it.
    let tree = state.store.lock().unwrap().process_tree();
    let term = root_of(pid, &HOST.root_pids(), &tree)
        .and_then(|root| HOST.term_for_root(root))
        .ok_or("This session is not running in Maya.")?;
    attach(term, on_frame)
}

/// Attaches the viewer to terminal `term` itself, for a session started a
/// moment ago that has no card yet.
#[tauri::command(async)]
pub fn native_attach_term(term: u64, on_frame: Channel<NativeFrame>) -> Result<NativeAttached, String> {
    attach(term, on_frame)
}

fn attach(term: u64, on_frame: Channel<NativeFrame>) -> Result<NativeAttached, String> {
    let sink: Sink = Box::new(move |frame| on_frame.send(frame_of(frame)).is_ok());
    let attached = HOST.attach(term, screen::SCROLLBACK, sink)?;
    Ok(NativeAttached {
        term: attached.term,
        viewer: attached.viewer,
        snapshot: STANDARD.encode(attached.snapshot),
        rows: attached.rows,
        cols: attached.cols,
    })
}

/// What the page is told when a Native terminal ends before any card
/// showed it: the folder it ran in and its last lines.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NativeEnded {
    pub term: u64,
    pub dir: String,
    pub lines: Vec<String>,
}

impl From<Unseen> for NativeEnded {
    fn from(u: Unseen) -> NativeEnded {
        NativeEnded { term: u.term, dir: u.folder.display().to_string(), lines: u.lines }
    }
}

/// Tells the page about each Native terminal that ends unseen.
pub fn tell_unseen_ends(app: AppHandle) {
    HOST.on_unseen_end(Box::new(move |u| {
        let ended = NativeEnded::from(u);
        maya_core::log::line("native", format!("a terminal in {} ended before its session showed", ended.dir));
        let _ = app.emit("native-ended", ended);
    }));
}

/// Queues `data` (base64) to be typed into the terminal, and returns at
/// once. It stays on the main thread, which runs commands in the order they
/// come, so keystrokes keep their order.
#[tauri::command]
pub fn native_input(term: u64, data: String) -> Result<(), String> {
    HOST.enqueue(term, &decode_input(&data)?)
}

#[tauri::command(async)]
pub fn native_resize(term: u64, rows: u16, cols: u16) -> Result<(), String> {
    HOST.resize(term, rows, cols)
}

#[tauri::command(async)]
pub fn native_detach(term: u64, viewer: u64) {
    HOST.detach(term, viewer);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_go_to_the_viewer_as_base64() {
        let out = serde_json::to_string(&frame_of(Frame::Output(vec![0xC3, 0xA9]))).unwrap();
        assert_eq!(out, r#"{"kind":"output","data":"w6k="}"#);
        assert_eq!(serde_json::to_string(&frame_of(Frame::Exit)).unwrap(), r#"{"kind":"exit"}"#);
    }

    #[test]
    fn an_unseen_end_reaches_the_page_with_its_folder_and_lines() {
        let ended = NativeEnded::from(Unseen { term: 3, folder: "/work/eye".into(), lines: vec!["zsh: command not found: claude".into()] });
        let json = serde_json::to_value(&ended).unwrap();
        assert_eq!(json, serde_json::json!({ "term": 3, "dir": "/work/eye", "lines": ["zsh: command not found: claude"] }));
    }

    #[test]
    fn input_is_decoded_before_it_is_written() {
        assert_eq!(decode_input("aGk=").unwrap(), b"hi");
        assert!(decode_input("!!").is_err());
    }
}
