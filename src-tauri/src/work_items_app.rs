//! The Issues tab on the desktop: the poll that asks each service for the
//! rules' issues and keeps the started ones current, its state, and the
//! page's command. Locks: `store` and `work_items` one at a time, neither
//! across a network call.

use crate::{git, AppState};
use maya_core::config::GitProvider;
use maya_core::forge::ForgeError;
use maya_core::log;
use maya_core::store::now_ms;
use maya_core::work_items::started::{self, Started};
use maya_core::work_items::{self, github, gitlab, WorkItem};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager, State as TauriState};

/// The last issues fetched, the started ones, and why a service could not be asked.
#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkItemsState {
    /// The rules' items, then the started items no rule returned (their latest copy).
    pub items: Vec<WorkItem>,
    pub started: Vec<Started>,
    /// One line per service whose last check failed or that has no token.
    pub errors: Vec<ForgeError>,
    /// No switched-on Git connection has Work Items on.
    pub off: bool,
    pub fetched_at: Option<u64>,
}

pub(crate) fn poll_work_items(app: &AppHandle) {
    let state = app.state::<AppState>();
    let generation = state.git_generation.load(Ordering::SeqCst);
    let (conns, cfg, path) = {
        let store = state.store.lock().unwrap();
        (store.config.git_connections(), store.config.work_items.clone(), started::path(&store.claude_dir().join("maya")))
    };
    // Only the services with Work Items on are asked, or named when they have no token.
    let conns: Vec<_> = conns.into_iter().filter(|c| cfg.for_forge(c.provider).enabled).collect();
    let (forges, missing) = git::forges(&conns);
    let off = forges.is_empty() && missing.is_empty();
    let fetched = work_items::fetch(&forges, &cfg, git::http());
    for f in &fetched {
        if let Err(e) = &f.result {
            log::line("app", format!("issues: {}: {e}", f.forge.label()));
        }
    }
    let returned: HashMap<&str, &WorkItem> = fetched.iter().filter_map(|f| f.result.as_ref().ok()).flatten().map(|i| (i.key.as_str(), i)).collect();

    // A started item a rule returned takes that copy; the others are asked for one by one.
    let loaded = started::load(&path);
    let mut kept = loaded.clone();
    let mut closed = Vec::new();
    for s in kept.iter_mut() {
        if let Some(item) = returned.get(s.key.as_str()) {
            s.item = (*item).clone();
            continue;
        }
        let Some(repo) = &s.item.repo else { continue };
        let Some(forge) = forges.iter().find(|f| f.site.provider == repo.forge) else { continue };
        let Some(number) = s.item.number else { continue };
        let items = cfg.for_forge(repo.forge);
        let got = match repo.forge {
            GitProvider::Github => github::get(git::http(), forge, &repo.path, number, items),
            GitProvider::Gitlab => gitlab::get(git::http(), forge, &repo.path, number, items),
        };
        match got {
            Ok(Some((item, _))) => s.item = item,
            Ok(None) => closed.push(s.key.clone()),
            Err(e) => log::line("app", format!("issues: {}: {e}", s.item.id)),
        }
    }
    let kept = started::prune(kept, &closed, &crate::board_cards(app), now_ms());
    if kept != loaded {
        if let Err(e) = started::save(&path, &kept) {
            log::line("app", format!("issues: could not save the started issues: {e}"));
        }
    }

    let snapshot = {
        let mut w = state.work_items.lock().unwrap();
        if state.git_generation.load(Ordering::SeqCst) != generation {
            log::line("app", "issues: the Git or Work Items settings changed during the check; its answers are dropped");
            return;
        }
        // The started items join after the rules' ones, so they are not the rules' to keep.
        let previous: Vec<WorkItem> = w.items.iter().filter(|i| !w.started.iter().any(|s| s.key == i.key)).cloned().collect();
        let (mut items, errors) = work_items::combine(&previous, fetched, missing);
        let extra: Vec<WorkItem> = kept.iter().filter(|s| !items.iter().any(|i| i.key == s.key)).map(|s| s.item.clone()).collect();
        items.extend(extra);
        w.items = items;
        w.started = kept;
        w.errors = errors;
        w.off = off;
        w.fetched_at = Some(now_ms());
        w.clone()
    };
    let _ = app.emit("work-items", &snapshot);
}

#[tauri::command(async)]
pub(crate) fn list_work_items(state: TauriState<AppState>) -> WorkItemsState {
    state.work_items.lock().unwrap().clone()
}
