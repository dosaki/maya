//! What's new after an update: which release notes a device shows the
//! first time it opens on a newer version, and what it saves.

use serde::Serialize;

/// The window to show: the notes after `from` up to `running`; with no
/// `from`, only `running`'s own.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    pub from: Option<String>,
    pub running: String,
}

fn parse(v: &str) -> Option<(u64, u64, u64)> {
    let mut parts = v.trim().split('.').map(|p| p.parse::<u64>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

/// Whether version `a` is newer than `b`, by number; `None` when either is
/// not three dot-separated numbers.
pub fn newer(a: &str, b: &str) -> Option<bool> {
    Some(parse(a)? > parse(b)?)
}

/// Marks a brand-new install (`existed` false: no settings file at start)
/// as having shown `running`, in memory, so that whatever saves the
/// settings first writes it: a start that ends before the page asks (the
/// first-run question) must not make the next start look like an update.
pub fn mark_new_install(config: &mut crate::config::Config, existed: bool, running: &str) {
    if !existed && config.last_shown_version.is_none() {
        config.last_shown_version = Some(running.into());
    }
}

/// What a device shows when it starts on `running`, given the version it
/// last showed (`saved`) and whether its settings file existed at start:
/// the window to show, and a version to save now without showing one (a
/// brand-new install's). A saved value that is no version still shows
/// that something was saved, so it counts as an existing install.
pub fn at_start(saved: Option<&str>, running: &str, existed: bool) -> (Option<Offer>, Option<String>) {
    if parse(running).is_none() {
        return (None, None);
    }
    let only_this = || Some(Offer { from: None, running: running.into() });
    match saved {
        None if !existed => (None, Some(running.into())),
        None => (only_this(), None),
        Some(s) => match newer(running, s) {
            Some(true) => (Some(Offer { from: Some(s.trim().into()), running: running.into() }), None),
            Some(false) => (None, None),
            None => (only_this(), None),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_brand_new_install_is_marked_shown_as_it_loads() {
        let mut c = crate::config::Config::default();
        mark_new_install(&mut c, false, "0.50.0");
        assert_eq!(c.last_shown_version.as_deref(), Some("0.50.0"));
        // So whatever saves first writes it, and the next start offers nothing.
        assert_eq!(at_start(c.last_shown_version.as_deref(), "0.50.0", true), (None, None));
        let mut existing = crate::config::Config::default();
        mark_new_install(&mut existing, true, "0.50.0");
        assert_eq!(existing.last_shown_version, None, "an existing install is left as it is");
    }

    #[test]
    fn a_brand_new_install_shows_nothing_and_saves_the_running_version() {
        assert_eq!(at_start(None, "0.50.0", false), (None, Some("0.50.0".into())));
    }

    #[test]
    fn an_existing_user_with_no_saved_version_sees_only_this_version() {
        assert_eq!(at_start(None, "0.50.0", true), (Some(Offer { from: None, running: "0.50.0".into() }), None));
    }

    #[test]
    fn an_older_saved_version_shows_from_it() {
        assert_eq!(at_start(Some("0.48.2"), "0.50.0", true), (Some(Offer { from: Some("0.48.2".into()), running: "0.50.0".into() }), None));
    }

    #[test]
    fn the_same_or_a_newer_saved_version_shows_and_saves_nothing() {
        assert_eq!(at_start(Some("0.50.0"), "0.50.0", true), (None, None));
        assert_eq!(at_start(Some("0.51.0"), "0.50.0", true), (None, None));
    }

    #[test]
    fn versions_compare_by_number() {
        assert_eq!(newer("0.10.0", "0.9.9"), Some(true));
        assert_eq!(newer("1.0.0", "0.99.0"), Some(true));
        assert_eq!(newer("0.9.0", "0.10.0"), Some(false));
        assert_eq!(newer("0.9", "0.8.0"), None);
    }

    #[test]
    fn a_saved_value_that_is_no_version_counts_as_none() {
        assert_eq!(at_start(Some(""), "0.50.0", true), (Some(Offer { from: None, running: "0.50.0".into() }), None));
        assert_eq!(at_start(Some("garbage"), "0.50.0", false), (Some(Offer { from: None, running: "0.50.0".into() }), None));
    }
}
