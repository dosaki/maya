//! Settings → Git on the desktop: tokens from the key store or a
//! variable, the forges the polls ask, the page's commands, and the
//! one-time import of `gh`'s token.

use crate::{keystore, AppState};
use maya_core::config::{self, GitConnection, GitProvider};
use maya_core::forge::{self, Forge, ForgeError};
use maya_core::log;
use serde::Serialize;
use tauri::{AppHandle, Manager, State as TauriState};

pub fn token_item(p: GitProvider) -> &'static keystore::Item {
    match p {
        GitProvider::Github => &keystore::GITHUB_TOKEN,
        GitProvider::Gitlab => &keystore::GITLAB_TOKEN,
    }
}

pub fn http() -> &'static forge::Ureq {
    static HTTP: std::sync::OnceLock<forge::Ureq> = std::sync::OnceLock::new();
    HTTP.get_or_init(forge::Ureq::new)
}

/// The switched-on forges that have a token, and a line for each that has none.
/// Reads the key store: call it without the store's lock held.
pub fn forges(conns: &[GitConnection]) -> (Vec<Forge>, Vec<ForgeError>) {
    let mut out = (Vec::new(), Vec::new());
    for c in conns.iter().filter(|c| c.enabled) {
        match forge::resolve(c, |p| keystore::load(token_item(p)).ok().flatten(), forge::env::read) {
            Ok(f) => out.0.push(f),
            Err(message) => out.1.push(ForgeError { forge: c.provider, message }),
        }
    }
    out
}

/// After a Git setting or token changed: look everything up again, now.
pub fn settings_changed(app: &AppHandle) {
    forge::env::forget();
    let state = app.state::<AppState>();
    // Before the cache is cleared: a poll that began earlier then drops its answers.
    state.git_generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    state.store.lock().unwrap().clear_prs();
    let handle = app.clone();
    std::thread::spawn(move || {
        crate::poll_reviews(&handle);
        crate::poll_deployments(&handle);
        crate::poll_pull_requests(&handle);
    });
}

/// The first start after 0.24 (or ever): GitHub on with `gh`'s token when
/// `gh` is logged in, else reading `GH_TOKEN`; GitLab off. Never again after.
/// Never over a config.json that did not parse: that would save the
/// defaults `load` fell back to. True when it saved the Git settings.
pub fn import_if_needed(app: &AppHandle) -> bool {
    let path = {
        let state = app.state::<AppState>();
        let store = state.store.lock().unwrap();
        if store.config.git.is_some() {
            return false;
        }
        store.config_path()
    };
    if !config::import_allowed(&path) {
        log::line("app", "git: config.json did not parse; the Git import waits so it cannot overwrite it");
        return false;
    }
    let stored = match forge::env::import_gh_token() {
        Some(token) => match keystore::store(&keystore::GITHUB_TOKEN, &token) {
            Ok(()) => true,
            Err(e) => {
                log::line("app", format!("git: could not keep gh's token: {}", forge::scrub(&e, token.trim())));
                false
            }
        },
        None => false,
    };
    let mut saved = false;
    if let Err(e) = crate::update_config(app, |c| {
        if c.git.is_none() {
            c.git = Some(config::git_after_import(stored));
            saved = true;
        }
    }) {
        log::line("app", format!("git: could not save the Git settings: {e}"));
        return false;
    }
    if saved {
        log::line("app", if stored { "git: imported gh's GitHub token" } else { "git: no gh token to import; GitHub reads GH_TOKEN" });
    }
    saved
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeStatus {
    has_token: bool,
}

#[derive(Serialize)]
pub struct GitStatus {
    github: ForgeStatus,
    gitlab: ForgeStatus,
}

/// Whether a pasted token is stored for each provider; never the token.
#[tauri::command(async)]
pub fn get_git_status() -> GitStatus {
    let has = |p| ForgeStatus { has_token: keystore::load(token_item(p)).ok().flatten().is_some() };
    GitStatus { github: has(GitProvider::Github), gitlab: has(GitProvider::Gitlab) }
}

#[tauri::command(async)]
pub fn store_git_token(app: AppHandle, provider: GitProvider, token: String) -> Result<(), String> {
    keystore::store(token_item(provider), &token).map_err(|e| format!("Could not store the token: {}", forge::scrub(&e, token.trim())))?;
    settings_changed(&app);
    Ok(())
}

#[tauri::command(async)]
pub fn clear_git_token(app: AppHandle, provider: GitProvider) -> Result<(), String> {
    keystore::remove(token_item(provider)).map_err(|e| format!("Could not remove the token: {e}"))?;
    settings_changed(&app);
    Ok(())
}

/// The token's user, from one call to the forge, for the Test button.
#[tauri::command(async)]
pub fn test_git_connection(state: TauriState<AppState>, provider: GitProvider) -> Result<String, String> {
    let conn = state.store.lock().unwrap().config.git_connections().into_iter().find(|c| c.provider == provider).ok_or("No such connection.")?;
    forge::env::forget();
    let f = forge::resolve(&conn, |p| keystore::load(token_item(p)).ok().flatten(), forge::env::read)?;
    forge::whoami(http(), &f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use maya_core::config::{GitConnection, GitProvider, TokenSource};

    #[test]
    fn each_provider_has_its_key_store_item() {
        assert_eq!(token_item(GitProvider::Github).service, "maya-github");
        assert_eq!(token_item(GitProvider::Gitlab).service, "maya-gitlab");
    }

    #[test]
    fn forges_with_tokens_are_asked_and_the_rest_say_why_not() {
        let conns = vec![
            GitConnection { token: TokenSource::Env { name: "MAYA_GIT_TEST_GITHUB".into() }, ..GitConnection::default_for(GitProvider::Github) },
            GitConnection { enabled: true, token: TokenSource::Env { name: "MAYA_GIT_TEST_GITLAB_UNSET".into() }, ..GitConnection::default_for(GitProvider::Gitlab) },
        ];
        std::env::set_var("MAYA_GIT_TEST_GITHUB", "ghp_test");
        let (forges, errors) = forges(&conns);
        std::env::remove_var("MAYA_GIT_TEST_GITHUB");
        assert_eq!(forges.iter().map(|f| f.site.provider).collect::<Vec<_>>(), vec![GitProvider::Github]);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].forge, GitProvider::Gitlab);
        assert!(errors[0].message.starts_with("No GitLab token: set MAYA_GIT_TEST_GITLAB_UNSET"), "{}", errors[0].message);
    }

    #[test]
    fn a_switched_off_forge_is_neither_asked_nor_an_error() {
        let conns = vec![GitConnection { enabled: false, ..GitConnection::default_for(GitProvider::Github) }, GitConnection::default_for(GitProvider::Gitlab)];
        let (forges, errors) = forges(&conns);
        assert!(forges.is_empty() && errors.is_empty());
    }
}
