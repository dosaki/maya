//! Spending a forge's request quota with care. GitHub gives each user one
//! hourly allowance, shared with `gh` and every other app on the account,
//! so a used-up quota is told apart from a refused token, nothing more is
//! asked of that quota until it resets, and an answer that has not changed
//! is asked for with its ETag: GitHub does not count a 304 against the limit.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

/// What a forge answered: the status, the body, and, for a used-up quota,
/// when it resets (unix ms).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub body: String,
    pub reset_at: Option<u64>,
}

impl From<(u16, String)> for Reply {
    fn from((status, body): (u16, String)) -> Self {
        Reply { status, body, reset_at: None }
    }
}

impl Reply {
    /// The answer for a quota Maya is waiting on, sent without asking the forge.
    pub fn waiting(until: u64) -> Self {
        Reply { status: 429, body: r#"{"message":"rate limit: waiting for it to reset"}"#.into(), reset_at: Some(until) }
    }
}

/// How long to wait when a rate-limited reply does not say.
pub const DEFAULT_WAIT_MS: u64 = 60_000;

/// Whether the body is the forge saying a rate limit is used up.
pub fn says_rate_limit(body: &str) -> bool {
    body.to_ascii_lowercase().contains("rate limit")
}

/// Whether a reply says the quota is used up: a 429, or a 403 that says so
/// or shows no requests left (GitHub answers an exhausted quota with 403).
pub fn is_rate_limited(status: u16, body: &str, remaining: Option<&str>) -> bool {
    status == 429 || (status == 403 && (remaining.map(str::trim) == Some("0") || says_rate_limit(body)))
}

/// When the quota may be asked again, from the reply's headers: GitHub's
/// `retry-after` (a secondary limit), else `x-ratelimit-reset` (GitLab:
/// `ratelimit-reset`), else a minute.
pub fn resume_at(header: &dyn Fn(&str) -> Option<String>, now_ms: u64) -> u64 {
    let secs = |name: &str| header(name).and_then(|v| v.trim().parse::<u64>().ok());
    if let Some(s) = secs("retry-after") {
        return now_ms + s.max(1) * 1000;
    }
    match secs("x-ratelimit-reset").or_else(|| secs("ratelimit-reset")) {
        Some(at) if at * 1000 > now_ms => at * 1000,
        _ => now_ms + DEFAULT_WAIT_MS,
    }
}

/// The quota a request draws on: its host and, as GitHub counts them apart,
/// GraphQL, search or the rest.
pub fn bucket(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let path = path.split('?').next().unwrap_or("");
    let kind = if path.ends_with("graphql") {
        "graphql"
    } else if path.contains("search/") {
        "search"
    } else {
        "core"
    };
    format!("{}|{kind}", host.to_ascii_lowercase())
}

/// The quotas that are used up, and when each resets.
#[derive(Default)]
pub struct Pauses(Mutex<HashMap<String, u64>>);

impl Pauses {
    /// When the quota `url` draws on resets, while it is used up.
    pub fn until(&self, url: &str, now_ms: u64) -> Option<u64> {
        let mut m = self.0.lock().unwrap();
        let key = bucket(url);
        match m.get(&key) {
            Some(&at) if at > now_ms => Some(at),
            Some(_) => {
                m.remove(&key);
                None
            }
            None => None,
        }
    }

    /// Notes a reply: a rate-limited one, or a success that spent the last
    /// request, pauses its quota until it resets. Returns when, if it did.
    pub fn note(&self, url: &str, status: u16, body: &str, header: &dyn Fn(&str) -> Option<String>, now_ms: u64) -> Option<u64> {
        let remaining = header("x-ratelimit-remaining").or_else(|| header("ratelimit-remaining"));
        let limited = is_rate_limited(status, body, remaining.as_deref());
        let spent = (200..300).contains(&status) && remaining.as_deref().map(str::trim) == Some("0");
        if !limited && !spent {
            return None;
        }
        let at = resume_at(header, now_ms);
        self.0.lock().unwrap().insert(bucket(url), at);
        Some(at)
    }

    /// Forgets every pause: the token may be another account's now.
    pub fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}

/// Most answers kept for their ETags; past it, the store starts over.
pub const MAX_ETAGS: usize = 1000;

/// The last 200 of each GET that came with an ETag, by token and URL, so
/// the next ask can be conditional and a 304 answered from here.
#[derive(Default)]
pub struct Etags(Mutex<HashMap<(u64, String), (String, String)>>);

fn token_key(token: &str) -> u64 {
    let mut h = DefaultHasher::new();
    token.hash(&mut h);
    h.finish()
}

impl Etags {
    /// The ETag to send with a GET of `url`, if its answer is kept.
    pub fn tag(&self, token: &str, url: &str) -> Option<String> {
        self.0.lock().unwrap().get(&(token_key(token), url.to_string())).map(|(tag, _)| tag.clone())
    }

    /// The kept body for a 304 to `url`.
    pub fn body(&self, token: &str, url: &str) -> Option<String> {
        self.0.lock().unwrap().get(&(token_key(token), url.to_string())).map(|(_, body)| body.clone())
    }

    pub fn store(&self, token: &str, url: &str, tag: &str, body: &str) {
        let mut m = self.0.lock().unwrap();
        if m.len() >= MAX_ETAGS {
            m.clear();
        }
        m.insert((token_key(token), url.to_string()), (tag.to_string(), body.to_string()));
    }

    pub fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name: &str| pairs.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.to_string())
    }

    #[test]
    fn a_used_up_quota_is_told_apart_from_a_refused_token() {
        assert!(is_rate_limited(403, r#"{"message":"API rate limit exceeded for user ID 1957450."}"#, None));
        assert!(is_rate_limited(403, r#"{"message":"API rate limit already exceeded for user ID 1."}"#, None));
        assert!(is_rate_limited(403, r#"{"message":"You have exceeded a secondary rate limit."}"#, None));
        assert!(is_rate_limited(403, "{}", Some("0")));
        assert!(is_rate_limited(429, "", None));
        assert!(!is_rate_limited(403, r#"{"message":"Resource not accessible by personal access token"}"#, Some("4321")));
        assert!(!is_rate_limited(401, r#"{"message":"Bad credentials"}"#, None));
        assert!(!is_rate_limited(200, "rate limit", Some("0")));
    }

    #[test]
    fn the_wait_comes_from_retry_after_then_the_reset_then_a_minute() {
        let now = 1_000_000_000;
        assert_eq!(resume_at(&headers(&[("Retry-After", "30"), ("X-RateLimit-Reset", "2000000")]), now), now + 30_000);
        assert_eq!(resume_at(&headers(&[("X-RateLimit-Reset", "1003600")]), now), 1_003_600_000);
        assert_eq!(resume_at(&headers(&[("RateLimit-Reset", "1000060")]), now), 1_000_060_000, "GitLab's header");
        assert_eq!(resume_at(&headers(&[("X-RateLimit-Reset", "999999")]), now), now + DEFAULT_WAIT_MS, "a reset already past");
        assert_eq!(resume_at(&headers(&[]), now), now + DEFAULT_WAIT_MS);
    }

    #[test]
    fn rest_graphql_and_search_are_separate_quotas_per_host() {
        assert_eq!(bucket("https://api.github.com/repos/O/r/pulls?head=O%3Ab"), "api.github.com|core");
        assert_eq!(bucket("https://api.github.com/graphql"), "api.github.com|graphql");
        assert_eq!(bucket("https://api.github.com/search/issues?q=x"), "api.github.com|search");
        assert_eq!(bucket("https://GitLab.example.com/api/v4/projects"), "gitlab.example.com|core");
    }

    #[test]
    fn a_rate_limited_reply_pauses_only_its_quota_until_the_reset() {
        let p = Pauses::default();
        let now = 1_000_000_000;
        let limited = headers(&[("X-RateLimit-Remaining", "0"), ("X-RateLimit-Reset", "1003600")]);
        assert_eq!(p.note("https://api.github.com/repos/O/r/pulls", 403, "{}", &limited, now), Some(1_003_600_000));
        assert_eq!(p.until("https://api.github.com/user/repos", now + 1), Some(1_003_600_000));
        assert_eq!(p.until("https://api.github.com/graphql", now + 1), None, "GraphQL has its own quota");
        assert_eq!(p.until("https://api.github.com/user/repos", 1_003_600_000), None, "over at the reset");
        assert_eq!(p.until("https://api.github.com/user/repos", now + 1), None, "and forgotten");
    }

    #[test]
    fn a_success_that_spent_the_last_request_pauses_too_and_others_do_not() {
        let p = Pauses::default();
        let now = 1_000_000_000;
        assert_eq!(p.note("https://api.github.com/x", 200, "[]", &headers(&[("X-RateLimit-Remaining", "12")]), now), None);
        assert_eq!(p.note("https://api.github.com/x", 403, "{}", &headers(&[("X-RateLimit-Remaining", "12")]), now), None, "a refusal");
        assert_eq!(p.until("https://api.github.com/x", now), None);
        assert_eq!(p.note("https://api.github.com/x", 200, "[]", &headers(&[("X-RateLimit-Remaining", "0"), ("X-RateLimit-Reset", "1000500")]), now), Some(1_000_500_000));
        p.clear();
        assert_eq!(p.until("https://api.github.com/x", now), None, "cleared");
    }

    #[test]
    fn an_etag_is_kept_per_token_and_url() {
        let e = Etags::default();
        e.store("ghp_a", "https://api.github.com/x", "W/\"1\"", "[1]");
        assert_eq!(e.tag("ghp_a", "https://api.github.com/x").as_deref(), Some("W/\"1\""));
        assert_eq!(e.body("ghp_a", "https://api.github.com/x").as_deref(), Some("[1]"));
        assert_eq!(e.tag("ghp_b", "https://api.github.com/x"), None, "another token asks afresh");
        assert_eq!(e.tag("ghp_a", "https://api.github.com/y"), None);
        e.store("ghp_a", "https://api.github.com/x", "W/\"2\"", "[2]");
        assert_eq!(e.body("ghp_a", "https://api.github.com/x").as_deref(), Some("[2]"));
        e.clear();
        assert_eq!(e.tag("ghp_a", "https://api.github.com/x"), None);
    }

    #[test]
    fn the_etag_store_starts_over_when_full() {
        let e = Etags::default();
        for i in 0..MAX_ETAGS {
            e.store("t", &format!("u{i}"), "x", "b");
        }
        e.store("t", "last", "x", "b");
        assert_eq!(e.tag("t", "u0"), None);
        assert_eq!(e.tag("t", "last").as_deref(), Some("x"));
    }
}
