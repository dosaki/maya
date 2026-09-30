use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::time::Duration;

/// Calls `on_change` whenever `sessions_dir` or `eye_dir` changes (debounced
/// 200 ms) and at least every `tick`. Blocks forever; run on its own thread.
pub fn run(sessions_dir: &Path, eye_dir: &Path, tick: Duration, mut on_change: impl FnMut()) {
    let (tx, rx) = channel::<()>();
    let mut watcher: Option<RecommendedWatcher> = notify::recommended_watcher(move |_res| {
        let _ = tx.send(());
    })
    .ok();
    if let Some(w) = watcher.as_mut() {
        let _ = std::fs::create_dir_all(eye_dir);
        let _ = w.watch(sessions_dir, RecursiveMode::NonRecursive);
        let _ = w.watch(eye_dir, RecursiveMode::NonRecursive);
    }

    on_change();
    loop {
        match rx.recv_timeout(tick) {
            Ok(()) => {
                // Debounce: swallow events for 200 ms, then refresh once.
                std::thread::sleep(Duration::from_millis(200));
                while rx.try_recv().is_ok() {}
                on_change();
            }
            Err(RecvTimeoutError::Timeout) => on_change(),
            Err(RecvTimeoutError::Disconnected) => {
                std::thread::sleep(tick);
                on_change();
            }
        }
    }
}
