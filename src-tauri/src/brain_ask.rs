//! An assistant answering the main's question from its own brain notes.

use crate::brain_app::brain_dir;
use crate::AppState;
use maya_core::brain::ask::{answer_with, cut, gather, memory_dir, notes_cap, to_data, ANSWER_TIMEOUT, QUESTION_CHARS};
use maya_core::config::NetworkRole;
use maya_core::{launch, log};
use serde_json::Value;
use tauri::{AppHandle, Manager};

/// Answers `question` from this machine's brain notes with a one-shot of its
/// agent. Only an assistant answers. The store is locked just to read the
/// settings; the agent runs with no lock held. Nothing is stored and nothing
/// goes to Chat.
pub(crate) fn answer(app: &AppHandle, question: &str) -> Result<Option<Value>, String> {
    let state = app.state::<AppState>();
    let (role, claude_dir, brain, model) = {
        let store = state.store.lock().unwrap();
        let c = &store.config;
        let settings = (c.network.role, store.claude_dir().to_path_buf(), c.brain(), c.brain_model().map(String::from));
        settings
    };
    if role == NetworkRole::Main {
        return Err("Only an assistant answers a brain question.".into());
    }
    let dir = brain_dir(&claude_dir);
    let notes = gather(&dir, &memory_dir(&claude_dir, &dir), notes_cap());
    let binary = launch::find_binary(brain);
    if binary.is_err() && !notes.trim().is_empty() {
        log::line("brain", format!("asked \"{}\": failed, its agent is not installed", cut(question, 80)));
        return Err("Its brain's agent is not installed.".into());
    }
    let question = cut(question, QUESTION_CHARS);
    let result = answer_with(&notes, &question, |p| {
        let binary = binary.as_ref().ok()?;
        launch::oneshot_text(brain, binary, model.as_deref(), p, &claude_dir.join("maya"), ANSWER_TIMEOUT)
    });
    let outcome = match &result {
        Ok(Some(_)) => "answered",
        Ok(None) => "did not know",
        Err(_) => "failed",
    };
    log::line("brain", format!("asked \"{}\": {outcome}", cut(&question, 80)));
    result.map(|a| Some(to_data(a)))
}
