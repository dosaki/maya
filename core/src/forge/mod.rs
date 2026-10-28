//! GitHub and GitLab over their APIs: which forge a remote belongs to,
//! where its token comes from, and the HTTP every call goes through. A
//! token never appears in an error: everything passes `scrub`.

pub mod env;
pub mod github;
pub mod gitlab;

use crate::config::{CloneProtocol, GitConnection, GitProvider, TokenSource};
use crate::reviews::ReviewPr;
use serde::Serialize;
use std::time::Duration;

/// A switched-on forge's address, without its token: enough to route a
/// remote and build a clone URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForgeSite {
    pub provider: GitProvider,
    /// `https://github.com`, or the GitLab URL; no trailing slash.
    pub web_base: String,
    /// How a review clones from this forge.
    pub clone: CloneProtocol,
}

impl ForgeSite {
    pub fn of(c: &GitConnection) -> ForgeSite {
        ForgeSite { provider: c.provider, web_base: c.web_base(), clone: c.clone }
    }

    pub fn api_base(&self) -> String {
        match self.provider {
            GitProvider::Github => "https://api.github.com".into(),
            GitProvider::Gitlab => format!("{}/api/v4", self.web_base),
        }
    }

    pub fn graphql_url(&self) -> String {
        match self.provider {
            GitProvider::Github => "https://api.github.com/graphql".into(),
            GitProvider::Gitlab => format!("{}/api/graphql", self.web_base),
        }
    }

    fn after_scheme(&self) -> &str {
        self.web_base.split_once("://").map_or(self.web_base.as_str(), |(_, r)| r)
    }

    /// The host, lower-case, without a port.
    pub fn host(&self) -> String {
        let authority = self.after_scheme().split('/').next().unwrap_or("");
        authority.split(':').next().unwrap_or("").to_ascii_lowercase()
    }

    /// The path the web base sits under (`/gitlab`), or "".
    fn prefix(&self) -> String {
        self.after_scheme().find('/').map_or(String::new(), |i| self.after_scheme()[i..].trim_end_matches('/').to_string())
    }

    pub fn clone_url(&self, repo: &str) -> String {
        format!("{}/{repo}.git", self.web_base)
    }

    /// `git@<host>:<repo>.git`: the SSH address when the API gave none.
    pub fn ssh_clone_url(&self, repo: &str) -> String {
        format!("git@{}:{repo}.git", self.host())
    }
}

/// What a review clones: the site's protocol, with the API's own address
/// for the repo when it gave one, else one built from the site.
pub fn clone_url_for(site: &ForgeSite, pr: &ReviewPr) -> String {
    match site.clone {
        CloneProtocol::Ssh => pr.ssh_url.clone().unwrap_or_else(|| site.ssh_clone_url(&pr.repo)),
        CloneProtocol::Https => pr.https_url.clone().unwrap_or_else(|| site.clone_url(&pr.repo)),
    }
}

impl AsRef<ForgeSite> for ForgeSite {
    fn as_ref(&self) -> &ForgeSite {
        self
    }
}

/// A switched-on forge with its token. Its `Debug` hides the token.
#[derive(Clone)]
pub struct Forge {
    pub site: ForgeSite,
    pub token: String,
}

impl std::fmt::Debug for Forge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Forge").field("site", &self.site).field("token", &"***").finish()
    }
}

impl AsRef<ForgeSite> for Forge {
    fn as_ref(&self) -> &ForgeSite {
        &self.site
    }
}

/// One forge's failure, as the Pull requests tab shows it.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ForgeError {
    pub forge: GitProvider,
    pub message: String,
}

/// The switched-on connections' sites.
pub fn sites(conns: &[GitConnection]) -> Vec<ForgeSite> {
    conns.iter().filter(|c| c.enabled).map(ForgeSite::of).collect()
}

/// The connection with its token: the stored one, or the variable's (an
/// empty `GH_TOKEN` falls back to `GITHUB_TOKEN`, as `gh` does).
pub fn resolve(c: &GitConnection, stored: impl Fn(GitProvider) -> Option<String>, env: impl Fn(&str) -> Option<String>) -> Result<Forge, String> {
    let label = c.provider.label();
    let token = match &c.token {
        TokenSource::Stored => stored(c.provider),
        TokenSource::Env { .. } => {
            let name = c.env_name().unwrap_or_default();
            env(&name).filter(|t| !t.trim().is_empty()).or_else(|| if name == "GH_TOKEN" { env("GITHUB_TOKEN") } else { None })
        }
    };
    match token.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) {
        Some(token) => Ok(Forge { site: ForgeSite::of(c), token }),
        None => Err(match &c.token {
            TokenSource::Stored => format!("No {label} token: paste one in Settings → Git."),
            TokenSource::Env { .. } => format!("No {label} token: set {} or paste a token in Settings → Git.", c.env_name().unwrap_or_default()),
        }),
    }
}

/// A remote's host (lower-case, no port or user) and path (no `.git`, no
/// outer slashes), and whether it is an http(s) remote.
fn split_remote(remote: &str) -> Option<(String, String, bool)> {
    let r = remote.trim().trim_end_matches('/');
    let r = r.strip_suffix(".git").unwrap_or(r);
    let (authority, path, http) = if let Some((scheme, rest)) = r.split_once("://") {
        let (authority, path) = rest.split_once('/')?;
        (authority, path, scheme.starts_with("http"))
    } else {
        // scp-like: [user@]host:path
        let (authority, path) = r.split_once(':')?;
        if authority.contains('/') {
            return None;
        }
        (authority, path, false)
    };
    let host = authority.rsplit('@').next()?.split(':').next()?.to_ascii_lowercase();
    let path = path.trim_matches('/').to_string();
    (!host.is_empty() && !path.is_empty()).then_some((host, path, http))
}

/// The site a remote belongs to and the repo's path there, or None.
pub fn route<'a, T: AsRef<ForgeSite>>(remote: &str, sites: &'a [T]) -> Option<(&'a T, String)> {
    let (host, path, http) = split_remote(remote)?;
    sites.iter().find_map(|s| {
        let site = s.as_ref();
        if site.host() != host {
            return None;
        }
        let prefix = site.prefix();
        let prefix = prefix.trim_start_matches('/');
        let path = if http && !prefix.is_empty() {
            // The prefix is the site's own spelling; the repo path keeps the remote's.
            let want = format!("{prefix}/");
            path.get(..want.len()).filter(|head| head.eq_ignore_ascii_case(&want)).map(|_| path[want.len()..].to_string())?
        } else {
            path.clone()
        };
        (!path.is_empty()).then_some((s, path))
    })
}

/// `text` with every copy of `token` replaced by `***`.
pub fn scrub(text: &str, token: &str) -> String {
    if token.is_empty() {
        text.to_string()
    } else {
        text.replace(token, "***")
    }
}

/// `s` percent-encoded as one URL path segment or query value.
pub fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

/// HTTP as the forges need it: a status and a body, or why there was none.
pub trait Http: Sync {
    fn get(&self, url: &str, token: &str) -> Result<(u16, String), String>;
    fn post_json(&self, url: &str, token: &str, body: &str) -> Result<(u16, String), String>;
}

const TIMEOUT: Duration = Duration::from_secs(15);

/// `Http` over `ureq`, every status handed back rather than raised. It
/// trusts the certificates the OS trusts, so an internal CA or a
/// TLS-inspecting proxy works as it does for `git` and `gh`.
pub struct Ureq {
    agent: ureq::Agent,
}

impl Ureq {
    pub fn new() -> Self {
        let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
        let config = ureq::Agent::config_builder().timeout_global(Some(TIMEOUT)).http_status_as_error(false).tls_config(tls).build();
        Ureq { agent: ureq::Agent::new_with_config(config) }
    }
}

impl Default for Ureq {
    fn default() -> Self {
        Self::new()
    }
}

fn read(r: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<(u16, String), String> {
    let mut resp = r.map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let body = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
    Ok((status, body))
}

impl Http for Ureq {
    fn get(&self, url: &str, token: &str) -> Result<(u16, String), String> {
        read(self.agent.get(url).header("Authorization", &format!("Bearer {token}")).header("User-Agent", "Maya").header("Accept", "application/json").call())
    }

    fn post_json(&self, url: &str, token: &str, body: &str) -> Result<(u16, String), String> {
        read(
            self.agent
                .post(url)
                .header("Authorization", &format!("Bearer {token}"))
                .header("User-Agent", "Maya")
                .header("Accept", "application/json")
                .header("Content-Type", "application/json")
                .send(body),
        )
    }
}

/// What a call is for, so a refused token is told what it needs.
#[derive(Clone, Copy)]
pub(crate) enum Need {
    Reviews,
    Deployments,
}

fn scopes(p: GitProvider, need: Need) -> &'static str {
    match (p, need) {
        (GitProvider::Github, Need::Reviews) => "repo scope (fine-grained: Pull requests read)",
        (GitProvider::Github, Need::Deployments) => "repo scope (fine-grained: Actions read, Deployments read and write, Environments read, Metadata read)",
        (GitProvider::Gitlab, _) => "read_api scope",
    }
}

/// Whether `e` says the token was refused.
pub fn is_refusal(e: &str) -> bool {
    e.starts_with("the token was refused")
}

/// The body of a 2xx reply, or what went wrong in words.
/// The body of a 2xx reply, else why not. An error body is scrubbed of
/// `token` before it is cut, so no part of the token survives the cut.
fn check(p: GitProvider, status: u16, body: String, token: &str, need: Need) -> Result<String, String> {
    match status {
        200..=299 => Ok(body),
        401 | 403 => Err(format!("the token was refused ({status}). It needs the {}.", scopes(p, need))),
        _ => {
            let body = scrub(&body, token);
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            let message = v["message"].as_str().or_else(|| v["error"].as_str()).map(str::to_string).unwrap_or_else(|| body.chars().take(200).collect());
            Err(format!("{status}: {message}"))
        }
    }
}

fn answer(forge: &Forge, r: Result<(u16, String), String>, need: Need) -> Result<String, String> {
    let out = match r {
        Ok((status, body)) => check(forge.site.provider, status, body, &forge.token, need),
        Err(e) => Err(format!("could not reach {}: {e}", forge.site.host())),
    };
    out.map_err(|e| scrub(&e, &forge.token))
}

/// A GET's JSON reply.
pub(crate) fn get_json(http: &dyn Http, forge: &Forge, url: &str) -> Result<serde_json::Value, String> {
    get_json_as(http, forge, url, Need::Reviews)
}

/// A GET's JSON reply, where a refusal names the permissions `need` calls for.
pub(crate) fn get_json_as(http: &dyn Http, forge: &Forge, url: &str, need: Need) -> Result<serde_json::Value, String> {
    let body = answer(forge, http.get(url, &forge.token), need)?;
    serde_json::from_str(&body).map_err(|e| format!("{}'s reply was not JSON: {e}", forge.site.provider.label()))
}

/// A GraphQL query's reply body, for the provider's parser.
pub(crate) fn post_graphql(http: &dyn Http, forge: &Forge, query: &str) -> Result<String, String> {
    let body = serde_json::json!({ "query": query }).to_string();
    answer(forge, http.post_json(&forge.site.graphql_url(), &forge.token, &body), Need::Reviews)
}

/// A POST's JSON body sent, and the reply body, where a refusal names the permissions `need` calls for.
pub(crate) fn post_json_as(http: &dyn Http, forge: &Forge, url: &str, body: &serde_json::Value, need: Need) -> Result<String, String> {
    answer(forge, http.post_json(url, &forge.token, &body.to_string()), need)
}

/// The PR or MR of `branch` in `repo`.
pub fn current_pr(http: &dyn Http, forge: &Forge, repo: &str, branch: &str) -> Result<Option<crate::model::PullRequest>, String> {
    match forge.site.provider {
        GitProvider::Github => github::current_pr(http, forge, repo, branch),
        GitProvider::Gitlab => gitlab::current_pr(http, forge, repo, branch),
    }
}

/// The open PRs or MRs waiting on the user.
pub fn review_prs(http: &dyn Http, forge: &Forge) -> Result<Vec<crate::reviews::ReviewPr>, String> {
    match forge.site.provider {
        GitProvider::Github => github::review_prs(http, forge),
        GitProvider::Gitlab => gitlab::review_prs(http, forge),
    }
}

/// Who the token belongs to, for Settings' Test button.
pub fn whoami(http: &dyn Http, forge: &Forge) -> Result<String, String> {
    match forge.site.provider {
        GitProvider::Github => github::whoami(http, forge),
        GitProvider::Gitlab => gitlab::whoami(http, forge),
    }
}

/// The `origin` remote's URL of the checkout at `dir`.
pub fn origin_of(dir: &std::path::Path) -> Option<String> {
    let out = crate::command("git").args(["-C", dir.to_str()?, "config", "--get", "remote.origin.url"]).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())
}

#[cfg(test)]
pub(crate) mod fake {
    use std::sync::Mutex;

    /// Answers each request with the first reply whose needle the URL
    /// contains, else fails with "no route"; records (url, token, body).
    pub struct FakeHttp {
        pub replies: Vec<(&'static str, u16, String)>,
        pub seen: Mutex<Vec<(String, String, Option<String>)>>,
    }

    impl FakeHttp {
        pub fn new(replies: Vec<(&'static str, u16, &str)>) -> Self {
            FakeHttp { replies: replies.into_iter().map(|(n, s, b)| (n, s, b.to_string())).collect(), seen: Mutex::new(Vec::new()) }
        }

        fn answer(&self, url: &str, token: &str, body: Option<&str>) -> Result<(u16, String), String> {
            self.seen.lock().unwrap().push((url.to_string(), token.to_string(), body.map(str::to_string)));
            self.replies.iter().find(|(n, _, _)| url.contains(n)).map(|(_, s, b)| (*s, b.clone())).ok_or_else(|| "no route".to_string())
        }
    }

    impl super::Http for FakeHttp {
        fn get(&self, url: &str, token: &str) -> Result<(u16, String), String> {
            self.answer(url, token, None)
        }
        fn post_json(&self, url: &str, token: &str, body: &str) -> Result<(u16, String), String> {
            self.answer(url, token, Some(body))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GitConnection, GitProvider, TokenSource};

    #[test]
    fn the_forges_trust_the_systems_certificates() {
        // A self-hosted GitLab on an internal CA, or a TLS-inspecting proxy,
        // is trusted where the OS trusts it, as `gh` did.
        let roots = Ureq::new().agent.config().tls_config().root_certs().clone();
        assert!(matches!(roots, ureq::tls::RootCerts::PlatformVerifier), "{roots:?}");
    }

    fn github() -> ForgeSite {
        ForgeSite::of(&GitConnection::default_for(GitProvider::Github))
    }

    fn gitlab(url: &str) -> ForgeSite {
        ForgeSite::of(&GitConnection { base_url: Some(url.into()), enabled: true, ..GitConnection::default_for(GitProvider::Gitlab) })
    }

    #[test]
    fn a_site_knows_its_api_graphql_host_and_clone_url() {
        let gh = github();
        assert_eq!((gh.api_base(), gh.graphql_url(), gh.host()), ("https://api.github.com".into(), "https://api.github.com/graphql".into(), "github.com".into()));
        assert_eq!(gh.clone_url("Org/bedrock"), "https://github.com/Org/bedrock.git");
        let gl = gitlab("https://Acme.io:8443/gitlab");
        assert_eq!((gl.api_base(), gl.graphql_url(), gl.host()), ("https://Acme.io:8443/gitlab/api/v4".into(), "https://Acme.io:8443/gitlab/api/graphql".into(), "acme.io".into()));
        assert_eq!(gl.clone_url("grp/sub/r"), "https://Acme.io:8443/gitlab/grp/sub/r.git");
    }

    fn with_clone(mut site: ForgeSite, clone: CloneProtocol) -> ForgeSite {
        site.clone = clone;
        site
    }

    fn review_pr(repo: &str, ssh: Option<&str>, https: Option<&str>) -> ReviewPr {
        ReviewPr { number: 1, forge: GitProvider::Github, repo: repo.into(), title: String::new(), author: String::new(), url: String::new(), is_draft: false, updated_at: String::new(), reasons: vec![], my_review: None, pr_review: None, pr_reviewers: vec![], ssh_url: ssh.map(Into::into), https_url: https.map(Into::into) }
    }

    #[test]
    fn a_site_builds_an_ssh_address_from_its_host() {
        assert_eq!(github().ssh_clone_url("Org/r"), "git@github.com:Org/r.git");
        assert_eq!(gitlab("https://acme.io/gitlab").ssh_clone_url("grp/r"), "git@acme.io:grp/r.git");
        assert_eq!(gitlab("https://Acme.io:8443/gitlab").ssh_clone_url("grp/sub/r"), "git@acme.io:grp/sub/r.git", "no port, no sub-path");
    }

    #[test]
    fn the_clone_url_follows_the_protocol_and_prefers_the_apis_address() {
        let ssh = with_clone(github(), CloneProtocol::Ssh);
        let https = with_clone(github(), CloneProtocol::Https);
        let api = review_pr("Org/r", Some("git@github.com:Org/api.git"), Some("https://github.com/Org/api.git"));
        let bare = review_pr("Org/r", None, None);
        assert_eq!(clone_url_for(&ssh, &api), "git@github.com:Org/api.git");
        assert_eq!(clone_url_for(&ssh, &bare), "git@github.com:Org/r.git");
        assert_eq!(clone_url_for(&https, &api), "https://github.com/Org/api.git");
        assert_eq!(clone_url_for(&https, &bare), "https://github.com/Org/r.git");
        let gl = with_clone(gitlab("https://acme.io/gitlab"), CloneProtocol::Ssh);
        assert_eq!(clone_url_for(&gl, &review_pr("grp/r", None, None)), "git@acme.io:grp/r.git");
        let gl = with_clone(gitlab("https://acme.io/gitlab"), CloneProtocol::Https);
        assert_eq!(clone_url_for(&gl, &review_pr("grp/r", None, None)), "https://acme.io/gitlab/grp/r.git");
        assert_eq!(ForgeSite::of(&GitConnection { clone: CloneProtocol::Https, ..GitConnection::default_for(GitProvider::Github) }).clone, CloneProtocol::Https);
    }

    #[test]
    fn sites_are_the_enabled_connections() {
        let conns = vec![GitConnection::default_for(GitProvider::Github), GitConnection::default_for(GitProvider::Gitlab)];
        assert_eq!(sites(&conns).iter().map(|s| s.provider).collect::<Vec<_>>(), vec![GitProvider::Github]);
    }

    #[test]
    fn github_remotes_route_in_every_form() {
        let s = [github()];
        for remote in ["git@github.com:Org/bedrock.git", "https://github.com/Org/bedrock", "https://github.com/Org/bedrock.git/", "ssh://git@github.com/Org/bedrock.git", "https://x-access-token:ghp_secret@github.com/Org/bedrock.git"] {
            let (site, path) = route(remote, &s).unwrap_or_else(|| panic!("{remote}"));
            assert_eq!((site.provider, path.as_str()), (GitProvider::Github, "Org/bedrock"), "{remote}");
        }
        assert!(route("git@gitlab.com:Org/bedrock.git", &s).is_none(), "GitLab is off");
        assert!(route("/Users/x/dev/bare.git", &s).is_none());
        assert!(route("", &s).is_none());
    }

    #[test]
    fn gitlab_remotes_keep_nested_groups_and_drop_a_sub_path_and_credentials() {
        let s = [github(), gitlab("https://acme.io/gitlab")];
        let cases = [
            ("git@acme.io:grp/sub/r.git", "grp/sub/r"),
            ("ssh://git@acme.io:2222/grp/sub/r.git", "grp/sub/r"),
            ("https://acme.io/gitlab/grp/sub/r.git", "grp/sub/r"),
            ("https://oauth2:glpat-secret@ACME.io/gitlab/grp/sub/r", "grp/sub/r"),
        ];
        for (remote, want) in cases {
            let (site, path) = route(remote, &s).unwrap_or_else(|| panic!("{remote}"));
            assert_eq!((site.provider, path.as_str()), (GitProvider::Gitlab, want), "{remote}");
            assert!(!path.contains("secret"));
        }
        assert!(route("git@other.io:grp/r.git", &s).is_none());
        let upper = [gitlab("https://acme.io/GitLab")];
        let (_, path) = route("https://acme.io/gitlab/grp/r.git", &upper).unwrap();
        assert_eq!(path, "grp/r", "the prefix matches in any case; the repo path keeps its own");
        let (_, path) = route("https://acme.io/GITLAB/Grp/R.git", &upper).unwrap();
        assert_eq!(path, "Grp/R");
    }

    #[test]
    fn resolve_reads_the_stored_token_or_the_variable() {
        let stored = |_: GitProvider| Some(" tok-stored \n".to_string());
        let none = |_: GitProvider| None;
        let env = |n: &str| (n == "GITHUB_TOKEN").then(|| "tok-env".to_string());
        let gh_stored = GitConnection { token: TokenSource::Stored, ..GitConnection::default_for(GitProvider::Github) };
        assert_eq!(resolve(&gh_stored, stored, env).unwrap().token, "tok-stored");
        assert_eq!(resolve(&gh_stored, none, env).unwrap_err(), "No GitHub token: paste one in Settings → Git.");
        let gh_env = GitConnection::default_for(GitProvider::Github);
        assert_eq!(resolve(&gh_env, none, env).unwrap().token, "tok-env", "an empty GH_TOKEN falls back to GITHUB_TOKEN");
        let gl = GitConnection { enabled: true, ..GitConnection::default_for(GitProvider::Gitlab) };
        assert_eq!(resolve(&gl, none, env).unwrap_err(), "No GitLab token: set GITLAB_TOKEN or paste a token in Settings → Git.");
        let f = resolve(&gl, none, |n: &str| (n == "GITLAB_TOKEN").then(|| "glpat".into())).unwrap();
        assert_eq!((f.site.provider, f.site.web_base.as_str(), f.token.as_str()), (GitProvider::Gitlab, "https://gitlab.com", "glpat"));
    }

    #[test]
    fn scrub_hides_the_token_and_encode_escapes_everything_but_unreserved() {
        assert_eq!(scrub("bad credentials for ghp_abc123 here", "ghp_abc123"), "bad credentials for *** here");
        assert_eq!(scrub("nothing", ""), "nothing", "an empty token scrubs nothing");
        assert_eq!(encode("grp/sub/r"), "grp%2Fsub%2Fr");
        assert_eq!(encode("feat/x#1+y z"), "feat%2Fx%231%2By%20z");
        assert_eq!(encode("a-b_c.d~e"), "a-b_c.d~e");
    }

    #[test]
    fn a_refused_token_names_the_scope_and_other_errors_the_message() {
        let f = Forge { site: github(), token: "ghp_x".into() };
        let http = fake::FakeHttp::new(vec![("/user", 401, r#"{"message":"Bad credentials"}"#), ("/teapot", 418, r#"{"message":"short and stout ghp_x"}"#)]);
        assert_eq!(get_json(&http, &f, "https://api.github.com/user").unwrap_err(), "the token was refused (401). It needs the repo scope (fine-grained: Pull requests read).");
        assert_eq!(get_json(&http, &f, "https://api.github.com/teapot").unwrap_err(), "418: short and stout ***");
        assert_eq!(get_json(&http, &f, "https://api.github.com/nowhere").unwrap_err(), "could not reach github.com: no route");
        let gl = Forge { site: gitlab("https://gitlab.com"), token: "glpat".into() };
        let http = fake::FakeHttp::new(vec![("/user", 403, "{}")]);
        assert_eq!(get_json(&http, &gl, "https://gitlab.com/api/v4/user").unwrap_err(), "the token was refused (403). It needs the read_api scope.");
    }

    #[test]
    fn a_refusal_names_the_permissions_of_its_use() {
        let f = Forge { site: github(), token: "ghp_x".into() };
        let http = fake::FakeHttp::new(vec![("/x", 403, "{}")]);
        let reviews = get_json(&http, &f, "https://api.github.com/x").unwrap_err();
        let deploys = get_json_as(&http, &f, "https://api.github.com/x", Need::Deployments).unwrap_err();
        assert_eq!(reviews, "the token was refused (403). It needs the repo scope (fine-grained: Pull requests read).");
        assert_eq!(deploys, "the token was refused (403). It needs the repo scope (fine-grained: Actions read, Deployments read and write, Environments read, Metadata read).");
        assert!(is_refusal(&deploys) && !is_refusal("404: Not Found"));
        let http = fake::FakeHttp::new(vec![("/y", 200, "ok")]);
        assert_eq!(post_json_as(&http, &f, "https://api.github.com/y", &serde_json::json!({"a": 1}), Need::Deployments).unwrap(), "ok");
        assert_eq!(http.seen.lock().unwrap()[0].2.as_deref(), Some(r#"{"a":1}"#));
    }

    #[test]
    fn a_long_error_body_is_scrubbed_before_it_is_cut() {
        let f = Forge { site: github(), token: "ghp_secrettoken".into() };
        let body = format!("{}ghp_secrettoken and more", "x".repeat(195));
        let http = fake::FakeHttp::new(vec![("/user", 500, &body)]);
        let e = get_json(&http, &f, "https://api.github.com/user").unwrap_err();
        assert!(!e.contains("ghp_s"), "a token cut at the limit must not survive: {e}");
    }

    #[test]
    fn requests_carry_the_token() {
        let f = Forge { site: github(), token: "ghp_x".into() };
        let http = fake::FakeHttp::new(vec![("/graphql", 200, r#"{"data":{}}"#)]);
        assert_eq!(post_graphql(&http, &f, "query { viewer { login } }").unwrap(), r#"{"data":{}}"#);
        let seen = http.seen.lock().unwrap();
        assert_eq!(seen[0].0, "https://api.github.com/graphql");
        assert_eq!(seen[0].1, "ghp_x");
        assert_eq!(seen[0].2.as_deref(), Some(r#"{"query":"query { viewer { login } }"}"#));
    }
}
