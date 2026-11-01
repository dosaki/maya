//! The commands the terminal viewer uses to attach to a Native session,
//! type into it, resize it and detach, here or (as the main) on a connected
//! Maya. Bytes cross as base64 strings.

use crate::native::host::{Frame, Sink, TermId, Unseen};
use crate::native::{screen, HOST};
use crate::{remote_machine_of, AppState};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use maya_core::foreign::Proc;
use maya_core::native::{parents, root_of};
use maya_core::net::live::TermFrame;
use maya_core::net::server::{self, Server};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::ipc::Channel;
use tauri::State as TauriState;
use tauri::{AppHandle, Emitter};

/// What the viewer receives as the terminal runs.
#[derive(Serialize, Clone)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum NativeFrame {
    Output { data: String },
    Size { rows: u16, cols: u16 },
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
        Frame::Size { rows, cols } => NativeFrame::Size { rows, cols },
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
    let term = term_for_pid(&tree, pid).ok_or(NOT_RUNNING)?;
    attach(term, on_frame)
}

/// What attaching to a session says when Maya hosts no terminal for it.
pub(crate) const NOT_RUNNING: &str = "This session is not running in Maya.";

/// The hosted terminal that `pid` runs in, walking up `tree` to its program.
pub(crate) fn term_for_pid(tree: &[Proc], pid: i32) -> Option<TermId> {
    root_of(pid, &HOST.root_pids(), &parents(tree)).and_then(|root| HOST.term_for_root(root))
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

/// What opening a remote terminal says for a session no connected Maya lists.
const NOT_REMOTE: &str = "This session is not on a connected Maya.";
/// What a remote terminal command says when this Maya runs no server.
const NOT_MAIN: &str = "This Maya is not the main.";

/// The running server, or `NOT_MAIN`.
fn main_server(state: &AppState) -> Result<Arc<Mutex<Server>>, String> {
    let server = state.network.lock().unwrap().server.as_ref().map(|s| s.shared.clone());
    server.ok_or_else(|| NOT_MAIN.into())
}

/// Where a remote terminal is relayed from: the server, then the machine
/// that lists the session (asked only once there is a server).
fn relay_target<S>(server: Result<S, String>, machine: impl FnOnce() -> Option<String>) -> Result<(S, String), String> {
    let server = server?;
    Ok((server, machine().ok_or(NOT_REMOTE)?))
}

/// Opens the live terminal of `session_id` on the connected Maya that lists
/// it, through this main's server; its frames go to `on_frame`. Returns the
/// sub for `live_input_remote` and `live_detach_remote`.
#[tauri::command(async)]
pub fn live_attach_remote(state: TauriState<AppState>, session_id: String, on_frame: Channel<TermFrame>) -> Result<u64, String> {
    let (server, machine) = relay_target(main_server(&state), || remote_machine_of(&state, &session_id))?;
    server::term_attach(&server, &machine, &session_id, Box::new(move |frame| on_frame.send(frame).is_ok()))
}

/// Types `data` (base64) into the remote terminal `sub`. On the main thread,
/// as `native_input`, so keystrokes keep their order.
#[tauri::command]
pub fn live_input_remote(state: TauriState<AppState>, sub: u64, data: String) -> Result<(), String> {
    server::term_input(&main_server(&state)?, sub, data)
}

#[tauri::command(async)]
pub fn live_detach_remote(state: TauriState<AppState>, sub: u64) {
    if let Ok(server) = main_server(&state) {
        server::term_detach(&server, sub);
    }
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
    fn a_size_frame_reaches_the_viewer() {
        assert_eq!(serde_json::to_string(&frame_of(Frame::Size { rows: 30, cols: 100 })).unwrap(), r#"{"kind":"size","rows":30,"cols":100}"#);
    }

    #[test]
    fn an_unseen_end_reaches_the_page_with_its_folder_and_lines() {
        let ended = NativeEnded::from(Unseen { term: 3, folder: "/work/eye".into(), lines: vec!["zsh: command not found: claude".into()] });
        let json = serde_json::to_value(&ended).unwrap();
        assert_eq!(json, serde_json::json!({ "term": 3, "dir": "/work/eye", "lines": ["zsh: command not found: claude"] }));
    }

    #[test]
    fn a_remote_terminal_needs_the_main_and_a_connected_maya() {
        let no_server: Result<(), String> = Err(NOT_MAIN.into());
        assert_eq!(relay_target(no_server, || panic!("not asked without a server")), Err(NOT_MAIN.to_string()));
        assert_eq!(relay_target(Ok(()), || None), Err(NOT_REMOTE.to_string()));
        assert_eq!(relay_target(Ok(()), || Some("laptop".into())), Ok(((), "laptop".to_string())));
    }

    #[test]
    fn input_is_decoded_before_it_is_written() {
        assert_eq!(decode_input("aGk=").unwrap(), b"hi");
        assert!(decode_input("!!").is_err());
    }
}
