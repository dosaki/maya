//! The Issues tab on the desktop: the poll that asks each service for the
//! rules' issues and keeps the started ones current, its state, and the
//! page's commands, Start's among them. Locks: `store`, `work_items`,
//! `forge_me` and the started file's one at a time, none across a network
//! call or the one-shot agent.

use crate::{git, native, AppState};
use maya_core::config::GitProvider;
use maya_core::forge::{self, Forge, ForgeError, Me};
use maya_core::store::now_ms;
use maya_core::work_items::started::{self, Started};
use maya_core::config::ForgeItems;
use maya_core::work_items::{self, github, gitlab, pick, start, Issue, ItemRepo, WorkItem};
use maya_core::{actions, launch, log};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
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
    /// No Git connection is switched on.
    pub git_off: bool,
    /// No switched-on Git connection has Work Items on.
    pub off: bool,
    pub fetched_at: Option<u64>,
}

pub(crate) fn poll_work_items(app: &AppHandle) {
    let state = app.state::<AppState>();
    let generation = state.git_generation.load(Ordering::SeqCst);
    let (conns, cfg) = {
        let store = state.store.lock().unwrap();
        (store.config.git_connections(), store.config.work_items.clone())
    };
    let git_off = !conns.iter().any(|c| c.enabled);
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

    // On a snapshot, with no lock held: a started item a rule returned takes
    // that copy; the others are asked for one by one.
    let mut latest: Vec<WorkItem> = returned.values().map(|i| (*i).clone()).collect();
    let mut closed = Vec::new();
    for s in started::load(&started_path(app)).into_iter().filter(|s| !returned.contains_key(s.key.as_str())) {
        let Some(repo) = &s.item.repo else { continue };
        let Some(forge) = forges.iter().find(|f| f.site.provider == repo.forge) else { continue };
        let Some(number) = s.item.number else { continue };
        let items = cfg.for_forge(repo.forge);
        let got = match repo.forge {
            GitProvider::Github => github::get(git::http(), forge, &repo.path, number, items),
            GitProvider::Gitlab => gitlab::get(git::http(), forge, &repo.path, number, items),
        };
        match got {
            Ok(Some(issue)) => latest.push(issue.item),
            Ok(None) => closed.push(s.key.clone()),
            Err(e) => log::line("app", format!("issues: {}: {e}", s.item.id)),
        }
    }
    let cards = crate::board_cards(app);
    let now = now_ms();
    // On the file as it is now, so an entry started meanwhile is kept.
    let mut refreshed = Vec::new();
    let kept = match update_started(app, |list| {
        *list = started::refresh(std::mem::take(list), &latest, &closed, &cards, now);
        refreshed = list.clone();
    }) {
        Ok(list) => list,
        Err(e) => {
            log::line("app", format!("issues: could not save the started issues: {e}"));
            refreshed
        }
    };

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
        w.git_off = git_off;
        w.off = off;
        w.fetched_at = Some(now_ms());
        w.clone()
    };
    let _ = app.emit("work-items", &snapshot);
}

fn started_path(app: &AppHandle) -> std::path::PathBuf {
    started::path(&app.state::<AppState>().store.lock().unwrap().claude_dir().join("maya"))
}

/// Changes the started file with `f`, one writer at a time: loads it fresh,
/// saves it only when `f` changed it, and returns the new list, or why it
/// could not be saved. Call it with no lock held, and never make a network
/// call inside `f`.
pub(crate) fn update_started(app: &AppHandle, f: impl FnOnce(&mut Vec<Started>)) -> Result<Vec<Started>, String> {
    let path = started_path(app);
    let state = app.state::<AppState>();
    let _writer = state.started_file.lock().unwrap();
    let before = started::load(&path);
    let mut after = before.clone();
    f(&mut after);
    if after != before {
        started::save(&path, &after)?;
    }
    Ok(after)
}

#[tauri::command(async)]
pub(crate) fn list_work_items(state: TauriState<AppState>) -> WorkItemsState {
    state.work_items.lock().unwrap().clone()
}

const GONE: &str = "That issue is no longer on the list.";

/// Maya's pick of the repository for an item that has none, and the ones to
/// choose from instead.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoPick {
    /// None when Maya had no answer.
    pub picked: Option<ItemRepo>,
    /// Why she picked it; "" with no pick.
    pub why: String,
    pub options: Vec<ItemRepo>,
}

/// What Start must ask before it starts: whom it takes the issue from, and
/// the repository when the item has none.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartCheck {
    /// The assignees other than the user; empty asks nothing.
    pub takeover: Vec<String>,
    pub pick: Option<RepoPick>,
    pub branch: String,
}

/// A started item's folder, its Native terminal when it runs in one, and
/// what did not work besides (the issue not updated, the start not kept).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkStarted {
    pub dir: String,
    pub native_term: Option<u64>,
    pub notice: Option<String>,
}

/// Who each service's token belongs to, kept until the Git settings change.
#[derive(Default)]
pub struct MeCache {
    generation: u64,
    by_forge: Vec<(GitProvider, Me)>,
}

impl MeCache {
    fn get(&self, generation: u64, forge: GitProvider) -> Option<Me> {
        if self.generation != generation {
            return None;
        }
        self.by_forge.iter().find(|(f, _)| *f == forge).map(|(_, me)| me.clone())
    }

    /// Keeps `me` for `generation`; an answer from an older one is dropped.
    fn put(&mut self, generation: u64, forge: GitProvider, me: Me) {
        if generation < self.generation {
            return;
        }
        if generation > self.generation {
            self.by_forge.clear();
            self.generation = generation;
        }
        self.by_forge.retain(|(f, _)| *f != forge);
        self.by_forge.push((forge, me));
    }
}

/// The assignees that are not `me`, ignoring case.
fn others(assignees: &[String], me: &str) -> Vec<String> {
    assignees.iter().filter(|a| !a.eq_ignore_ascii_case(me)).cloned().collect()
}

/// The item has no repository of its own, or its repository only tracks issues.
fn needs_pick(item: &WorkItem) -> bool {
    item.repo.is_none() || item.tracker
}

/// The repository Start works in: the one chosen when the item needs a pick,
/// else the item's own.
fn repo_to_start(item: &WorkItem, chosen: Option<ItemRepo>) -> Result<ItemRepo, String> {
    if needs_pick(item) {
        return chosen.ok_or_else(|| "Choose a repository first.".to_string());
    }
    item.repo.clone().ok_or_else(|| "Choose a repository first.".to_string())
}

/// How long after a Start a second one of the same item waits for its session.
const STARTING_MS: u64 = 60 * 1000;

/// `key` was started under a minute ago and its session is not on the board yet.
fn still_starting(started: &[Started], key: &str, cards: &[maya_core::model::Card], now_ms: u64) -> bool {
    started.iter().any(|s| s.key == key && now_ms.saturating_sub(s.started_at) < STARTING_MS && !started::has_session(s, cards))
}

/// Puts `entry` in the started list in place of an earlier start of its item.
fn record(list: &mut Vec<Started>, entry: Started) {
    list.retain(|s| s.key != entry.key);
    list.push(entry);
}

/// The listed item with `key`, and the whole list.
fn listed(state: &AppState, key: &str) -> Result<(WorkItem, Vec<WorkItem>), String> {
    let w = state.work_items.lock().unwrap();
    let item = w.items.iter().find(|i| i.key == key).cloned().ok_or(GONE)?;
    Ok((item, w.items.clone()))
}

/// The switched-on service with its token. Reads the key store: call it with no lock held.
fn forge_for(state: &AppState, provider: GitProvider) -> Result<Forge, String> {
    let conn = state.store.lock().unwrap().config.git_connections().into_iter().find(|c| c.provider == provider && c.enabled);
    let conn = conn.ok_or_else(|| format!("Turn on {} in Settings → Git first.", provider.label()))?;
    let (mut forges, missing) = git::forges(&[conn]);
    forges.pop().ok_or_else(|| missing.into_iter().next().map(|m| m.message).unwrap_or_else(|| format!("{} has no token.", provider.label())))
}

/// Who `forge`'s token belongs to: asked once per `generation`, which must
/// be read before `forge` was resolved, so a new token is never answered
/// for by the old one's user.
fn me_for(state: &AppState, forge: &Forge, generation: u64) -> Result<Me, String> {
    let provider = forge.site.provider;
    if let Some(me) = state.forge_me.lock().unwrap().get(generation, provider) {
        return Ok(me);
    }
    let me = forge::me(git::http(), forge).map_err(|e| format!("{}: {e}", provider.label()))?;
    let mut cache = state.forge_me.lock().unwrap();
    if state.git_generation.load(Ordering::SeqCst) == generation {
        cache.put(generation, provider, me.clone());
    }
    Ok(me)
}

/// Maya's brain picks the code's repository for `item` from the ones she
/// knows: the rules', the listed items', the clones' and those of the
/// sessions on the board or started from an issue.
fn pick_repo(app: &AppHandle, item: &WorkItem, items: &[WorkItem]) -> RepoPick {
    let state = app.state::<AppState>();
    let (cfg, clones_root, sites, brain, model, maya_dir) = {
        let store = state.store.lock().unwrap();
        let c = &store.config;
        (c.work_items.clone(), c.clones_dir_path(), forge::sites(&c.git_connections()), c.brain(), c.brain_model().map(String::from), store.claude_dir().join("maya"))
    };
    let mut folders: Vec<PathBuf> = crate::board_cards(app).into_iter().filter(|c| !c.brain && !c.cwd.is_empty()).map(|c| PathBuf::from(c.cwd)).collect();
    folders.extend(started::load(&started_path(app)).into_iter().filter(|s| !s.dir.is_empty()).map(|s| PathBuf::from(s.dir)));
    folders.sort();
    folders.dedup();
    // Git runs with no lock held.
    let mut known = pick::clone_repos(&clones_root, &sites);
    known.extend(pick::repos_of_folders(&folders, &sites));
    let options = pick::candidates(&cfg, items, &known);
    let answer = if options.is_empty() {
        None
    } else {
        // The one-shot runs in Maya's data folder, where no project's settings load and no board reads.
        launch::find_binary(brain)
            .ok()
            .and_then(|binary| launch::oneshot_text(brain, &binary, model.as_deref(), &pick::pick_prompt(item, &options), &maya_dir, launch::CLASSIFIER_TIMEOUT))
            .and_then(|reply| pick::parse_pick(&reply, &options))
    };
    let (picked, why) = match answer {
        Some((repo, why)) => (Some(repo), why),
        None => {
            log::line("app", format!("issues: Maya picked no repository for {}", item.id));
            (None, String::new())
        }
    };
    RepoPick { picked, why, options }
}

/// Why Start refuses the user's answer to the takeover question, if it does:
/// the issue is someone else's and the user did not agree to take it over,
/// or someone the check did not name (`checked` held its assignees) has
/// been assigned since.
fn takeover_refusal(checked: &[String], fresh: &[String], me: &str, take_over: bool) -> Option<&'static str> {
    let now = others(fresh, me);
    if now.is_empty() {
        return None;
    }
    if !take_over {
        return Some("This issue is assigned to someone else.");
    }
    let named = others(checked, me);
    let new = now.iter().any(|n| !named.iter().any(|c| c.eq_ignore_ascii_case(n)));
    new.then_some("Someone else was assigned since you checked; check again.")
}

/// `url` when it is a web address: nothing else goes to the system's opener.
fn web_link(url: &str) -> Result<&str, String> {
    if maya_core::attachments::is_web_url(url) {
        Ok(url)
    } else {
        Err("That issue's link is not a web address.".into())
    }
}

/// The listed item's issue, read afresh, with the service and the user it
/// was read as.
struct Fresh {
    forge: Forge,
    me: Me,
    cfg: ForgeItems,
    repo: String,
    number: u64,
    issue: Issue,
}

/// Reads `item`'s issue again; None for an item with no issue to read.
/// A closed issue is an error. Call it with no lock held.
fn read_fresh(state: &AppState, item: &WorkItem, generation: u64) -> Result<Option<Fresh>, String> {
    let (Some(repo), Some(number)) = (&item.repo, item.number) else { return Ok(None) };
    let forge = forge_for(state, repo.forge)?;
    let me = me_for(state, &forge, generation)?;
    let cfg = state.store.lock().unwrap().config.work_items.for_forge(repo.forge).clone();
    let got = match repo.forge {
        GitProvider::Github => github::get(git::http(), &forge, &repo.path, number, &cfg),
        GitProvider::Gitlab => gitlab::get(git::http(), &forge, &repo.path, number, &cfg),
    };
    let issue = got.map_err(|e| format!("{}: {e}", repo.forge.label()))?.ok_or("That issue was closed.")?;
    Ok(Some(Fresh { forge, me, cfg, repo: repo.path.clone(), number, issue }))
}

/// Puts `item` in the list in place of its older copy and tells the page.
fn show_fresh(app: &AppHandle, item: &WorkItem) {
    let snapshot = {
        let state = app.state::<AppState>();
        let mut w = state.work_items.lock().unwrap();
        let Some(listed) = w.items.iter_mut().find(|i| i.key == item.key) else { return };
        *listed = item.clone();
        w.clone()
    };
    let _ = app.emit("work-items", &snapshot);
}

/// What Start asks before it starts the listed item `key`, from the issue
/// as it is now (the list shows that copy from then on).
#[tauri::command(async)]
pub(crate) fn work_item_check(app: AppHandle, key: String) -> Result<StartCheck, String> {
    let state = app.state::<AppState>();
    let generation = state.git_generation.load(Ordering::SeqCst);
    let (listed_item, items) = listed(&state, &key)?;
    let (item, takeover) = match read_fresh(&state, &listed_item, generation)? {
        Some(f) => {
            show_fresh(&app, &f.issue.item);
            let takeover = others(&f.issue.item.assignees, &f.me.login);
            (f.issue.item, takeover)
        }
        None => (listed_item, Vec::new()),
    };
    let pick = needs_pick(&item).then(|| pick_repo(&app, &item, &items));
    Ok(StartCheck { takeover, pick, branch: start::branch_name(&item) })
}

/// Starts the listed item `key` in `repo` (the item's own when it needs no
/// pick): once the issue is still open, not someone else's unless
/// `take_over` covers them, and nothing known in advance refuses the
/// start, assigns it to the user and sets its start status, then opens its
/// session. A failed write-back is a notice (also a line above the areas),
/// not a failure. A Start under a minute old whose session has not shown
/// up yet refuses a second one.
#[tauri::command(async)]
pub(crate) fn work_item_start(app: AppHandle, key: String, repo: Option<ItemRepo>, take_over: bool) -> Result<WorkStarted, String> {
    let state = app.state::<AppState>();
    let generation = state.git_generation.load(Ordering::SeqCst);
    let (listed_item, _) = listed(&state, &key)?;
    let started_list = state.work_items.lock().unwrap().started.clone();
    if still_starting(&started_list, &key, &crate::board_cards(&app), now_ms()) {
        return Err(format!("Maya is still starting {}.", listed_item.id));
    }
    let repo = repo_to_start(&listed_item, repo)?;
    let mut notices = Vec::new();
    let item = match read_fresh(&state, &listed_item, generation)? {
        Some(f) => {
            let fresh = &f.issue.item;
            if let Some(why) = takeover_refusal(&listed_item.assignees, &fresh.assignees, &f.me.login, take_over) {
                // So the page's next check names whoever it is.
                show_fresh(&app, fresh);
                return Err(why.into());
            }
            actions::work_item_preflight(&crate::local(&state), fresh, &repo)?;
            let taken = match f.forge.site.provider {
                GitProvider::Github => github::take(git::http(), &f.forge, &f.repo, f.number, &f.me, &fresh.assignees, &f.issue.labels, &f.cfg),
                GitProvider::Gitlab => gitlab::take(git::http(), &f.forge, &f.repo, f.number, &f.me, &f.issue.assignee_ids, &f.issue.labels, &f.cfg),
            };
            if let Err(e) = taken {
                log::line("app", format!("issues: {} was not updated: {e}", fresh.id));
                notices.push(format!("Started, but {} was not updated: {e}", fresh.id));
            }
            f.issue.item
        }
        None => listed_item,
    };

    let (dir, native_term) = native::route::opening(|| actions::start_work_item(&crate::local(&state), &item, &repo));
    let dir = dir?;

    let entry = Started { key: item.key.clone(), item: item.clone(), dir: dir.clone(), started_at: now_ms() };
    let saved = update_started(&app, |list| record(list, entry.clone()));
    if let Err(e) = &saved {
        log::line("app", format!("issues: could not save the started issues: {e}"));
        notices.push(format!("Started, but Maya could not remember it: {e}"));
    }
    let (snapshot, notice) = {
        let mut w = state.work_items.lock().unwrap();
        match saved {
            Ok(list) => w.started = list,
            Err(_) => record(&mut w.started, entry),
        }
        let notice = (!notices.is_empty()).then(|| notices.join(" "));
        // Shown above the areas until the next check replaces the lines.
        if let Some(message) = &notice {
            let forge = item.repo.as_ref().map_or(repo.forge, |r| r.forge);
            w.errors.push(ForgeError { forge, message: message.clone() });
        }
        if let Some(listed) = w.items.iter_mut().find(|i| i.key == item.key) {
            *listed = item;
        }
        (w.clone(), notice)
    };
    let _ = app.emit("work-items", &snapshot);
    Ok(WorkStarted { dir, native_term, notice })
}

/// Opens a listed item in the browser; the URL comes from the fetched list,
/// and only a web address is opened.
#[tauri::command(async)]
pub(crate) fn open_work_item(state: TauriState<AppState>, key: String) -> Result<(), String> {
    let url = state.work_items.lock().unwrap().items.iter().find(|i| i.key == key).map(|i| i.url.clone()).ok_or(GONE)?;
    crate::open_in_browser(web_link(&url)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn others_are_the_assignees_that_are_not_me() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(others(&s(&["Tiago", "maria"]), "tiago"), s(&["maria"]));
        assert!(others(&s(&["tiago"]), "TIAGO").is_empty());
        assert!(others(&[], "tiago").is_empty());
    }

    #[test]
    fn a_takeover_covers_only_the_names_the_check_showed() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let (me, gone) = ("tiago", "This issue is assigned to someone else.");
        let new = "Someone else was assigned since you checked; check again.";
        assert_eq!(takeover_refusal(&s(&[]), &s(&["Tiago"]), me, false), None, "mine alone asks nothing");
        assert_eq!(takeover_refusal(&s(&["maria"]), &s(&["maria"]), me, false), Some(gone));
        assert_eq!(takeover_refusal(&s(&["maria"]), &s(&["Maria", "tiago"]), me, true), None, "the name the check showed");
        assert_eq!(takeover_refusal(&s(&["maria"]), &s(&["maria", "john"]), me, true), Some(new));
        assert_eq!(takeover_refusal(&s(&[]), &s(&["john"]), me, true), Some(new));
        assert_eq!(takeover_refusal(&s(&["maria", "john"]), &s(&["john"]), me, true), None, "one fewer is still covered");
    }

    #[test]
    fn only_a_web_link_is_opened() {
        assert_eq!(web_link("https://github.com/acme/api/issues/7"), Ok("https://github.com/acme/api/issues/7"));
        assert_eq!(web_link("http://gitlab.local/g/r/-/issues/1"), Ok("http://gitlab.local/g/r/-/issues/1"));
        for bad in ["file:///etc/passwd", "/Applications/Calculator.app", "javascript:alert(1)", ""] {
            assert_eq!(web_link(bad), Err("That issue's link is not a web address.".to_string()), "{bad}");
        }
    }

    fn item(repo: Option<&str>, tracker: bool) -> WorkItem {
        WorkItem {
            key: "github:acme/api#7".into(),
            source: "github".into(),
            id: "acme/api#7".into(),
            number: Some(7),
            title: "t".into(),
            url: "https://github.com/acme/api/issues/7".into(),
            description: String::new(),
            status: None,
            labels: vec![],
            assignees: vec![],
            author: "a".into(),
            updated_at: String::new(),
            repo: repo.map(|p| ItemRepo { forge: GitProvider::Github, path: p.into() }),
            tracker,
        }
    }

    #[test]
    fn start_works_in_the_items_repo_unless_it_needs_a_pick() {
        let chosen = ItemRepo { forge: GitProvider::Gitlab, path: "grp/r".into() };
        assert_eq!(repo_to_start(&item(Some("acme/api"), false), None).unwrap().path, "acme/api");
        assert_eq!(repo_to_start(&item(Some("acme/api"), false), Some(chosen.clone())).unwrap().path, "acme/api", "a plain repo is not changed");
        assert_eq!(repo_to_start(&item(Some("acme/issues"), true), Some(chosen.clone())).unwrap(), chosen);
        assert_eq!(repo_to_start(&item(Some("acme/issues"), true), None).unwrap_err(), "Choose a repository first.");
        assert_eq!(repo_to_start(&item(None, false), None).unwrap_err(), "Choose a repository first.");
    }

    #[test]
    fn the_user_is_kept_per_service_until_the_git_settings_change() {
        let me = |l: &str| Me { login: l.into(), id: 1 };
        let mut c = MeCache::default();
        c.put(1, GitProvider::Github, me("tiago"));
        c.put(1, GitProvider::Gitlab, me("tco"));
        assert_eq!(c.get(1, GitProvider::Github), Some(me("tiago")));
        assert_eq!(c.get(1, GitProvider::Gitlab), Some(me("tco")));
        assert_eq!(c.get(2, GitProvider::Github), None, "a newer generation asks again");
        c.put(2, GitProvider::Github, me("new"));
        assert_eq!(c.get(2, GitProvider::Gitlab), None, "the older answers are gone");
        c.put(1, GitProvider::Gitlab, me("stale"));
        assert_eq!(c.get(2, GitProvider::Gitlab), None, "an answer from before the change is dropped");
        assert_eq!(c.get(2, GitProvider::Github), Some(me("new")));
    }

    #[test]
    fn a_start_under_a_minute_old_without_its_session_is_still_starting() {
        use maya_core::model::{Card, Harness, State};
        let entry = |key: &str, at: u64| Started { key: key.into(), item: WorkItem { key: key.into(), ..item(Some("acme/api"), false) }, dir: "/c/api-issue-7".into(), started_at: at };
        let now = 1_000_000;
        let young = [entry("github:acme/api#7", now - 59_000)];
        assert!(still_starting(&young, "github:acme/api#7", &[], now));
        assert!(!still_starting(&young, "github:acme/api#8", &[], now), "another item");
        assert!(!still_starting(&[entry("github:acme/api#7", now - 61_000)], "github:acme/api#7", &[], now), "over a minute old");
        let session = Card {
            session_id: "s".into(), pid: 1, name: "issue acme/api#7".into(), cwd: "/c/api-issue-7".into(), state: State::Working, state_since: 0, snippet: String::new(),
            awaiting: None, has_inbox: false, harness: Harness::ClaudeCode, pr: None, context: None, machine: None, machine_address: None, machine_platform: None,
            terminal: None, stale: false, model: None, branch: None, brain: false, native: false, shell: false, usage: None, limits: None, status_line: None, live: false,
        };
        assert!(!still_starting(&young, "github:acme/api#7", &[session], now), "its session is on the board");
    }

    #[test]
    fn a_second_start_replaces_the_items_first() {
        let s = |dir: &str| Started { key: "github:acme/api#7".into(), item: item(Some("acme/api"), false), dir: dir.into(), started_at: 1 };
        let other = Started { key: "github:acme/api#8".into(), ..s("/b") };
        let mut list = vec![s("/old"), other.clone()];
        record(&mut list, s("/new"));
        assert_eq!(list, vec![other, s("/new")]);
    }
}
