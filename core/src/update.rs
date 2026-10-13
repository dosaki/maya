//! Whether a newer Maya is out: the release feed's version against this
//! one. The desktop app installs through Tauri's updater, which reads the
//! same feed; this is for the installs it cannot update (the phone, a
//! `.deb`), which only say that a new version is out.

use serde::Serialize;
use std::time::Duration;

use crate::log;

/// The feed every release publishes: Tauri's static updater manifest.
pub const FEED_URL: &str = "https://github.com/dosaki/maya/releases/latest/download/latest.json";

const RELEASES: &str = "https://github.com/dosaki/maya/releases";
const TIMEOUT: Duration = Duration::from_secs(20);

/// A newer release, and the link a page that cannot install it opens.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct UpdateInfo {
    pub version: String,
    pub download: String,
}

fn parts(v: &str) -> Option<(u64, u64, u64)> {
    let mut it = v.trim().trim_start_matches('v').split('.').map(|p| p.parse::<u64>().ok());
    let parsed = (it.next()??, it.next()??, it.next()??);
    if it.next().is_some() {
        return None;
    }
    Some(parsed)
}

/// True when `latest` is a higher `major.minor.patch` than `current`;
/// anything either side that is not one is never newer.
pub fn is_newer(current: &str, latest: &str) -> bool {
    matches!((parts(current), parts(latest)), (Some(c), Some(l)) if l > c)
}

/// The `version` of a feed.
pub fn feed_version(json: &str) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("the update feed is not JSON: {e}"))?;
    v["version"].as_str().map(str::to_string).ok_or_else(|| "the update feed has no version".to_string())
}

/// The release's page.
pub fn release_page(version: &str) -> String {
    format!("{RELEASES}/tag/v{version}")
}

/// The release's APK, by the name `release.yml` gives it.
pub fn apk_url(version: &str) -> String {
    format!("{RELEASES}/download/v{version}/Maya_{version}.apk")
}

fn fetch_feed_version() -> Result<String, String> {
    let config = ureq::Agent::config_builder().timeout_global(Some(TIMEOUT)).http_status_as_error(true).build();
    let agent = ureq::Agent::new_with_config(config);
    let mut resp = agent.get(FEED_URL).call().map_err(|e| format!("could not check for updates: {e}"))?;
    let body = resp.body_mut().read_to_string().map_err(|e| format!("could not read the update feed: {e}"))?;
    feed_version(&body)
}

/// The release the feed names when it is newer than `current`, with
/// `download(version)` as its link. A failure is logged and returned.
pub fn notice(current: &str, download: fn(&str) -> String) -> Result<Option<UpdateInfo>, String> {
    match fetch_feed_version() {
        Ok(v) if is_newer(current, &v) => Ok(Some(UpdateInfo { download: download(&v), version: v })),
        Ok(_) => Ok(None),
        Err(e) => {
            log::line("update", e.clone());
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_higher_version_is_newer_and_an_equal_or_lower_one_is_not() {
        assert!(is_newer("0.17.0", "0.18.0"));
        assert!(is_newer("0.17.0", "1.0.0"));
        assert!(is_newer("0.17.0", "0.17.1"));
        assert!(!is_newer("0.17.0", "0.17.0"));
        assert!(!is_newer("0.18.0", "0.17.9"));
    }

    #[test]
    fn parts_compare_as_numbers_not_text() {
        assert!(is_newer("0.9.0", "0.10.0"));
        assert!(!is_newer("0.10.0", "0.9.0"));
    }

    #[test]
    fn a_leading_v_and_surrounding_space_are_tolerated() {
        assert!(is_newer("v0.17.0", " v0.18.0\n"));
    }

    #[test]
    fn anything_unparsable_is_never_newer() {
        for junk in ["", "latest", "0.18", "0.18.0.1", "0.18.x", "0.18.0-beta"] {
            assert!(!is_newer("0.17.0", junk), "{junk:?}");
            assert!(!is_newer(junk, "0.18.0"), "{junk:?}");
        }
    }

    #[test]
    fn the_feed_version_is_read_from_a_tauri_manifest() {
        let json = r#"{"version":"0.19.0","notes":"https://github.com/dosaki/maya/releases/tag/v0.19.0","platforms":{}}"#;
        assert_eq!(feed_version(json), Ok("0.19.0".to_string()));
    }

    #[test]
    fn a_feed_without_a_version_or_not_json_is_an_error() {
        assert!(feed_version(r#"{"platforms":{}}"#).unwrap_err().contains("no version"));
        assert!(feed_version("<html>").unwrap_err().contains("not JSON"));
    }

    #[test]
    fn the_links_point_at_the_release() {
        assert_eq!(release_page("0.19.0"), "https://github.com/dosaki/maya/releases/tag/v0.19.0");
        assert_eq!(apk_url("0.19.0"), "https://github.com/dosaki/maya/releases/download/v0.19.0/Maya_0.19.0.apk");
    }
}
