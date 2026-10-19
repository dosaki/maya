# Git integration (GitHub and GitLab) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Settings → Git turns GitHub and GitLab on or off independently, each with a pasted or env-var token (GitLab with a base URL), and Maya talks to their APIs directly instead of running `gh`.

**Architecture:** A new `core/src/forge/` module holds the shared layer (sites, remote routing, token resolution, an `Http` trait over `ureq`, error scrubbing), the login-shell env reader, and one file per provider (`github.rs`, `gitlab.rs`). `pr.rs` (card badge), `reviews.rs` (Pull requests tab) and `actions::start_review` (review clone) call it in place of `gh`. `src-tauri/src/git.rs` resolves tokens from the key store, holds the Tauri commands and the one-time `gh` import; the page gets a Git settings group and forge-aware PR cards.

**Tech Stack:** Rust (Tauri 2, `ureq` 3, `serde_json`), vanilla TypeScript (vitest).

**Spec:** `docs/superpowers/specs/2026-10-18-git-integration-design.md`

## Global Constraints

- Branch `feat/git-integration`, made from `main` at 0.23.0. Never commit to or push `main`; ask before any push.
- Release: `sh scripts/set-version.sh 0.24.0`, committed on its own as `chore(release): 0.24.0`. Check `Cargo.lock` changed (cargo may be off `sh`'s PATH; run `cargo update --workspace --offline` by hand if not).
- Commit messages: Conventional Commits, plain `type: description`, ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- No new crates. `ureq` 3 is already in `core`.
- A token never enters `config.json`, the webview, a log line, or an error string: every error passes `forge::scrub`.
- Env var names are validated (`[A-Za-z_][A-Za-z0-9_]*`) before they reach a shell line.
- GitHub API base `https://api.github.com`; GitLab API `<base>/api/v4`, GraphQL `<base>/api/graphql`; GitLab default base `https://gitlab.com`.
- Every request sends `Authorization: Bearer <token>`, `User-Agent: Maya`, `Accept: application/json`, with a 15 s timeout.
- Default env names: `GH_TOKEN` (GitHub, also falling back to `GITHUB_TOKEN`), `GITLAB_TOKEN` (GitLab).
- Scopes named on 401/403: GitHub `repo` (fine-grained: Pull requests read), GitLab `read_api`.
- Unix-only tests are gated with `#[cfg(unix)]`; paths are compared by component, not as strings (Windows CI).
- Tests: `cargo test --workspace` and `pnpm test` from the repo root. `pnpm ear:build` must have run once (Tauri bundles the sidecar).

## Review Focus

1. An env var name like `GH_TOKEN; rm -rf ~` or `$(id)` must be refused by `set_config` and never reach the login shell (Task 1 `validate_git` test, Task 3 `read` test).
2. An HTTPS remote carrying credentials (`https://oauth2:glpat-secret@gitlab.acme.io/grp/r.git`) must still route to the right repo, and the credential must never show up in a path, log or error (Task 2 `route` test).
3. A self-hosted GitLab at a sub-path (`https://acme.io/gitlab`) with an SSH remote on a port (`ssh://git@acme.io:2222/grp/sub/r.git`) must route to `grp/sub/r` (Task 2 `route` test).
4. A branch named `feat/x#1+y` must reach the API percent-encoded, not cut at `#` or read as a path (Task 5 and Task 6 `current_pr` tests).
5. A GitLab URL pasted as `https://gitlab.acme.io/api/v4/` must save as `https://gitlab.acme.io` rather than produce `/api/v4/api/v4` calls (Task 1 `normalize_git` test).

---

### Task 1: Config — Git connections

**Files:**
- Modify: `core/src/config.rs` (new types after `VoiceProvider`, a field on `Config`, methods, tests in its `mod tests`)

**Interfaces:**
- Produces:
  - `pub enum GitProvider { Github, Gitlab }` (serde `"github"`/`"gitlab"`), with `label() -> &'static str`, `default_env() -> &'static str`, `sigil() -> char` (`#` / `!`).
  - `pub enum TokenSource { Stored, Env { name: String } }` (serde `{"kind":"stored"}` / `{"kind":"env","name":…}`).
  - `pub struct GitConnection { provider, enabled, base_url: Option<String>, token: TokenSource }` with `default_for(GitProvider)`, `web_base() -> String`, `env_name() -> Option<String>`.
  - `Config.git: Option<Vec<GitConnection>>`, `Config::git_connections() -> Vec<GitConnection>` (always `[github, gitlab]`), `Config::normalize_git(&mut self)`.
  - `pub fn valid_env_name(&str) -> bool`, `pub fn validate_git(&[GitConnection]) -> Result<(), String>`, `pub fn git_after_import(stored: bool) -> Vec<GitConnection>`.
  - `pub const GITHUB_URL: &str = "https://github.com"`, `pub const GITLAB_DEFAULT_URL: &str = "https://gitlab.com"`.

- [ ] **Step 1: Write the failing tests** (append inside `core/src/config.rs`'s `mod tests`)

```rust
    #[test]
    fn git_defaults_to_github_on_reading_gh_token_and_gitlab_off() {
        let c = Config::default();
        assert_eq!(c.git, None, "no git field until the first start imports gh's token");
        let conns = c.git_connections();
        assert_eq!(conns.len(), 2);
        assert_eq!((conns[0].provider, conns[0].enabled), (GitProvider::Github, true));
        assert_eq!(conns[0].token, TokenSource::Env { name: "GH_TOKEN".into() });
        assert_eq!((conns[1].provider, conns[1].enabled, conns[1].base_url.clone()), (GitProvider::Gitlab, false, None));
        assert_eq!(conns[1].token, TokenSource::Env { name: "GITLAB_TOKEN".into() });
    }

    #[test]
    fn an_older_config_reads_with_no_git_field_and_round_trips_once_set() {
        let older: Config = serde_json::from_str(r#"{"completedTimeoutMinutes":30}"#).unwrap();
        assert_eq!(older.git, None);
        let set = Config { git: Some(git_after_import(true)), ..Config::default() };
        let text = serde_json::to_string(&set).unwrap();
        assert!(text.contains(r#""git":[{"provider":"github","enabled":true,"token":{"kind":"stored"}}"#), "{text}");
        assert!(text.contains(r#"{"provider":"gitlab","enabled":false,"token":{"kind":"env","name":"GITLAB_TOKEN"}}"#), "{text}");
        let back: Config = serde_json::from_str(&text).unwrap();
        assert_eq!(back.git, set.git);
    }

    #[test]
    fn git_connections_fills_in_a_missing_provider_and_keeps_the_order() {
        let only_gitlab = GitConnection { provider: GitProvider::Gitlab, enabled: true, base_url: Some("https://gitlab.acme.io".into()), token: TokenSource::Stored };
        let c = Config { git: Some(vec![only_gitlab.clone()]), ..Config::default() };
        let conns = c.git_connections();
        assert_eq!(conns[0], GitConnection::default_for(GitProvider::Github));
        assert_eq!(conns[1], only_gitlab);
    }

    #[test]
    fn the_import_stores_github_as_a_pasted_token_or_falls_back_to_gh_token() {
        assert_eq!(git_after_import(true)[0].token, TokenSource::Stored);
        assert_eq!(git_after_import(false)[0].token, TokenSource::Env { name: "GH_TOKEN".into() });
        assert!(git_after_import(true)[0].enabled);
        assert!(!git_after_import(true)[1].enabled, "GitLab starts off");
    }

    #[test]
    fn web_base_is_github_or_the_gitlab_url_defaulting_to_gitlab_com() {
        assert_eq!(GitConnection::default_for(GitProvider::Github).web_base(), "https://github.com");
        let mut gl = GitConnection::default_for(GitProvider::Gitlab);
        assert_eq!(gl.web_base(), "https://gitlab.com");
        gl.base_url = Some("  ".into());
        assert_eq!(gl.web_base(), "https://gitlab.com", "blank is the default");
        gl.base_url = Some("https://acme.io/gitlab/".into());
        assert_eq!(gl.web_base(), "https://acme.io/gitlab");
    }

    #[test]
    fn env_name_is_the_default_when_blank() {
        let mut c = GitConnection::default_for(GitProvider::Github);
        c.token = TokenSource::Env { name: " ".into() };
        assert_eq!(c.env_name().as_deref(), Some("GH_TOKEN"));
        c.token = TokenSource::Env { name: " MY_TOKEN ".into() };
        assert_eq!(c.env_name().as_deref(), Some("MY_TOKEN"));
        c.token = TokenSource::Stored;
        assert_eq!(c.env_name(), None);
    }

    #[test]
    fn env_names_are_letters_digits_and_underscores_only() {
        for ok in ["GH_TOKEN", "_x", "a1"] {
            assert!(valid_env_name(ok), "{ok}");
        }
        for bad in ["", "1A", "GH TOKEN", "GH_TOKEN; rm -rf ~", "$(id)", "A-B", "É"] {
            assert!(!valid_env_name(bad), "{bad}");
        }
    }

    #[test]
    fn validate_git_refuses_a_bad_url_or_variable_name() {
        let gl = |url: &str| GitConnection { base_url: Some(url.into()), ..GitConnection::default_for(GitProvider::Gitlab) };
        assert!(validate_git(&[gl("https://gitlab.acme.io")]).is_ok());
        assert!(validate_git(&[gl("http://10.0.0.5:8080/gitlab")]).is_ok());
        assert!(validate_git(&[gl("")]).is_ok(), "blank means gitlab.com");
        assert_eq!(validate_git(&[gl("gitlab.acme.io")]).unwrap_err(), "GitLab URL must be an http(s) address.");
        assert_eq!(validate_git(&[gl("https://")]).unwrap_err(), "GitLab URL must be an http(s) address.");
        let bad_env = GitConnection { token: TokenSource::Env { name: "GH_TOKEN; rm -rf ~".into() }, ..GitConnection::default_for(GitProvider::Github) };
        assert_eq!(validate_git(&[bad_env]).unwrap_err(), "The GitHub token variable must be a name like GH_TOKEN.");
        let blank_env = GitConnection { token: TokenSource::Env { name: "".into() }, ..GitConnection::default_for(GitProvider::Github) };
        assert!(validate_git(&[blank_env]).is_ok(), "blank means the default");
    }

    #[test]
    fn normalize_git_trims_the_url_and_a_pasted_api_suffix_and_the_name() {
        let mut c = Config {
            git: Some(vec![
                GitConnection { token: TokenSource::Env { name: " GH_TOKEN ".into() }, ..GitConnection::default_for(GitProvider::Github) },
                GitConnection { base_url: Some(" https://gitlab.acme.io/api/v4/ ".into()), ..GitConnection::default_for(GitProvider::Gitlab) },
            ]),
            ..Config::default()
        };
        c.normalize_git();
        let g = c.git.unwrap();
        assert_eq!(g[0].token, TokenSource::Env { name: "GH_TOKEN".into() });
        assert_eq!(g[1].base_url.as_deref(), Some("https://gitlab.acme.io"));
        let mut blank = Config { git: Some(vec![GitConnection { base_url: Some("  ".into()), ..GitConnection::default_for(GitProvider::Gitlab) }]), ..Config::default() };
        blank.normalize_git();
        assert_eq!(blank.git.unwrap()[0].base_url, None);
        let mut none = Config::default();
        none.normalize_git();
        assert_eq!(none.git, None, "nothing to normalise stays unimported");
    }
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core config::tests`
Expected: compile errors (`GitProvider`, `git_after_import`… not found).

- [ ] **Step 3: Implement** (in `core/src/config.rs`)

After the `VoiceProvider` enum, add:

```rust
/// A code host Maya reads pull requests from.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum GitProvider {
    #[default]
    Github,
    Gitlab,
}

impl GitProvider {
    pub fn label(self) -> &'static str {
        match self {
            GitProvider::Github => "GitHub",
            GitProvider::Gitlab => "GitLab",
        }
    }

    /// The variable a new connection reads its token from.
    pub fn default_env(self) -> &'static str {
        match self {
            GitProvider::Github => "GH_TOKEN",
            GitProvider::Gitlab => "GITLAB_TOKEN",
        }
    }

    /// What goes between a repo and a number: `repo#12` on GitHub, `repo!12` on GitLab.
    pub fn sigil(self) -> char {
        match self {
            GitProvider::Github => '#',
            GitProvider::Gitlab => '!',
        }
    }
}

pub const GITHUB_URL: &str = "https://github.com";
pub const GITLAB_DEFAULT_URL: &str = "https://gitlab.com";

/// Where a connection's token comes from: the OS key store, or a variable
/// read from the login shell.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TokenSource {
    Stored,
    Env { name: String },
}

/// GitHub or GitLab, switched on or off, with where its token comes from.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitConnection {
    pub provider: GitProvider,
    pub enabled: bool,
    /// GitLab only; None is `GITLAB_DEFAULT_URL`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    pub token: TokenSource,
}

impl GitConnection {
    /// GitHub on, GitLab off, each reading its default variable.
    pub fn default_for(provider: GitProvider) -> Self {
        GitConnection { provider, enabled: provider == GitProvider::Github, base_url: None, token: TokenSource::Env { name: provider.default_env().into() } }
    }

    /// The web address repos live under, without a trailing slash.
    pub fn web_base(&self) -> String {
        match self.provider {
            GitProvider::Github => GITHUB_URL.into(),
            GitProvider::Gitlab => self.base_url.as_deref().map(str::trim).filter(|u| !u.is_empty()).unwrap_or(GITLAB_DEFAULT_URL).trim_end_matches('/').to_string(),
        }
    }

    /// The variable to read, the provider's default when blank; None for a stored token.
    pub fn env_name(&self) -> Option<String> {
        match &self.token {
            TokenSource::Stored => None,
            TokenSource::Env { name } => Some(Some(name.trim()).filter(|n| !n.is_empty()).unwrap_or(self.provider.default_env()).to_string()),
        }
    }
}

/// True for a shell variable name: letters, digits and `_`, not starting
/// with a digit. Only such a name goes into the login-shell probe.
pub fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Why the Git settings cannot be saved, if they cannot.
pub fn validate_git(conns: &[GitConnection]) -> Result<(), String> {
    for c in conns {
        if let Some(url) = c.base_url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
            let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"));
            if rest.map_or(true, |r| r.split('/').next().unwrap_or("").is_empty()) {
                return Err(format!("{} URL must be an http(s) address.", c.provider.label()));
            }
        }
        if let TokenSource::Env { name } = &c.token {
            let n = name.trim();
            if !n.is_empty() && !valid_env_name(n) {
                return Err(format!("The {} token variable must be a name like {}.", c.provider.label(), c.provider.default_env()));
            }
        }
    }
    Ok(())
}

/// The connections the first start saves: GitHub on with `gh`'s token when
/// it could be stored, else reading `GH_TOKEN`; GitLab off.
pub fn git_after_import(stored: bool) -> Vec<GitConnection> {
    let mut github = GitConnection::default_for(GitProvider::Github);
    if stored {
        github.token = TokenSource::Stored;
    }
    vec![github, GitConnection::default_for(GitProvider::Gitlab)]
}
```

In `struct Config`, after `whisper_model`, before `network`:

```rust
    /// GitHub and GitLab. None in a config from before 0.24 (and a fresh
    /// one): the first start fills it in after importing `gh`'s token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<Vec<GitConnection>>,
```

In `impl Default for Config`, add `git: None,` before `network`.

In the first `impl Config` block, add:

```rust
    /// One GitHub and one GitLab connection, in that order: the saved ones,
    /// with defaults for any missing.
    pub fn git_connections(&self) -> Vec<GitConnection> {
        [GitProvider::Github, GitProvider::Gitlab]
            .into_iter()
            .map(|p| self.git.as_ref().and_then(|g| g.iter().find(|c| c.provider == p)).cloned().unwrap_or_else(|| GitConnection::default_for(p)))
            .collect()
    }

    /// Trims the Git settings as typed: the URL's spaces, trailing slashes
    /// and a pasted `/api/v4` (blank becomes None), and the variable name.
    pub fn normalize_git(&mut self) {
        for c in self.git.iter_mut().flatten() {
            c.base_url = c.base_url.as_deref().map(|u| {
                let u = u.trim().trim_end_matches('/');
                u.strip_suffix("/api/v4").unwrap_or(u).trim_end_matches('/').to_string()
            }).filter(|u| !u.is_empty());
            if let TokenSource::Env { name } = &mut c.token {
                *name = name.trim().to_string();
            }
        }
    }
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p maya-core config::tests`
Expected: PASS. Then `cargo test --workspace` compiles (other crates build `Config` with `..Default::default()` or through serde; fix any struct literal that lists every field by adding `git: None`).

- [ ] **Step 5: Commit**

```bash
git add core/src/config.rs
git commit -m "feat: keep GitHub and GitLab connections in the config

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Forge core — sites, routing, tokens, HTTP

**Files:**
- Create: `core/src/forge/mod.rs`
- Modify: `core/src/lib.rs` (add `pub mod forge;`)

**Interfaces:**
- Consumes (Task 1): `GitProvider`, `GitConnection`, `TokenSource`.
- Produces:
  - `pub struct ForgeSite { pub provider: GitProvider, pub web_base: String }` with `of(&GitConnection)`, `api_base()`, `graphql_url()`, `host()`, `clone_url(repo)`; `impl AsRef<ForgeSite>` for `ForgeSite` and `Forge`.
  - `pub struct Forge { pub site: ForgeSite, pub token: String }`.
  - `pub struct ForgeError { pub forge: GitProvider, pub message: String }` (Serialize, camelCase).
  - `pub fn sites(&[GitConnection]) -> Vec<ForgeSite>` (enabled only).
  - `pub fn resolve(&GitConnection, stored: impl Fn(GitProvider) -> Option<String>, env: impl Fn(&str) -> Option<String>) -> Result<Forge, String>`.
  - `pub fn route<'a, T: AsRef<ForgeSite>>(remote: &str, sites: &'a [T]) -> Option<(&'a T, String)>`.
  - `pub trait Http: Sync { fn get(&self, url: &str, token: &str) -> Result<(u16, String), String>; fn post_json(&self, url: &str, token: &str, body: &str) -> Result<(u16, String), String>; }`, `pub struct Ureq` (`Ureq::new()`).
  - `pub fn scrub(text: &str, token: &str) -> String`, `pub fn encode(s: &str) -> String`.
  - `pub(crate) fn get_json(http: &dyn Http, forge: &Forge, url: &str) -> Result<serde_json::Value, String>`, `pub(crate) fn post_graphql(http: &dyn Http, forge: &Forge, query: &str) -> Result<String, String>`.
  - `#[cfg(test)] pub(crate) mod fake { pub struct FakeHttp }` for Tasks 5–7.

- [ ] **Step 1: Write the failing tests** (at the bottom of the new `core/src/forge/mod.rs`; create the file with only `#[cfg(test)] mod tests { … }` and `pub mod fake` first, add `pub mod forge;` to `lib.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GitConnection, GitProvider, TokenSource};

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
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core forge::tests`
Expected: compile errors (nothing defined yet).

- [ ] **Step 3: Implement** (top of `core/src/forge/mod.rs`, above the tests)

```rust
//! GitHub and GitLab over their APIs: which forge a remote belongs to,
//! where its token comes from, and the HTTP every call goes through. A
//! token never appears in an error: everything passes `scrub`.

pub mod env;
pub mod github;
pub mod gitlab;

use crate::config::{GitConnection, GitProvider, TokenSource};
use serde::Serialize;
use std::time::Duration;

/// A switched-on forge's address, without its token: enough to route a
/// remote and build a clone URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForgeSite {
    pub provider: GitProvider,
    /// `https://github.com`, or the GitLab URL; no trailing slash.
    pub web_base: String,
}

impl ForgeSite {
    pub fn of(c: &GitConnection) -> ForgeSite {
        ForgeSite { provider: c.provider, web_base: c.web_base() }
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
}

impl AsRef<ForgeSite> for ForgeSite {
    fn as_ref(&self) -> &ForgeSite {
        self
    }
}

/// A switched-on forge with its token.
#[derive(Clone)]
pub struct Forge {
    pub site: ForgeSite,
    pub token: String,
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
        let path = if http && !prefix.is_empty() { path.strip_prefix(&format!("{prefix}/"))?.to_string() } else { path.clone() };
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

/// `Http` over `ureq`, every status handed back rather than raised.
pub struct Ureq {
    agent: ureq::Agent,
}

impl Ureq {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder().timeout_global(Some(TIMEOUT)).http_status_as_error(false).build();
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

fn scopes(p: GitProvider) -> &'static str {
    match p {
        GitProvider::Github => "repo scope (fine-grained: Pull requests read)",
        GitProvider::Gitlab => "read_api scope",
    }
}

/// The body of a 2xx reply, or what went wrong in words.
fn check(p: GitProvider, status: u16, body: String) -> Result<String, String> {
    match status {
        200..=299 => Ok(body),
        401 | 403 => Err(format!("the token was refused ({status}). It needs the {}.", scopes(p))),
        _ => {
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            let message = v["message"].as_str().or_else(|| v["error"].as_str()).map(str::to_string).unwrap_or_else(|| body.chars().take(200).collect());
            Err(format!("{status}: {message}"))
        }
    }
}

fn answer(forge: &Forge, r: Result<(u16, String), String>) -> Result<String, String> {
    let out = match r {
        Ok((status, body)) => check(forge.site.provider, status, body),
        Err(e) => Err(format!("could not reach {}: {e}", forge.site.host())),
    };
    out.map_err(|e| scrub(&e, &forge.token))
}

/// A GET's JSON reply.
pub(crate) fn get_json(http: &dyn Http, forge: &Forge, url: &str) -> Result<serde_json::Value, String> {
    let body = answer(forge, http.get(url, &forge.token))?;
    serde_json::from_str(&body).map_err(|e| format!("{}'s reply was not JSON: {e}", forge.site.provider.label()))
}

/// A GraphQL query's reply body, for the provider's parser.
pub(crate) fn post_graphql(http: &dyn Http, forge: &Forge, query: &str) -> Result<String, String> {
    let body = serde_json::json!({ "query": query }).to_string();
    answer(forge, http.post_json(&forge.site.graphql_url(), &forge.token, &body))
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
```

`env`, `github` and `gitlab` come in Tasks 3, 5 and 6: create each as an empty file now (`//! filled in by a later task` is not needed; an empty file compiles) so this task builds.

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p maya-core forge::tests`
Expected: PASS (9 tests).

- [ ] **Step 5: Commit**

```bash
git add core/src/lib.rs core/src/forge/
git commit -m "feat: route remotes to GitHub or GitLab and resolve their tokens

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Environment variables and the `gh` import

**Files:**
- Modify: `core/src/forge/env.rs` (empty from Task 2)

**Interfaces:**
- Consumes: `config::valid_env_name`, `crate::command`, `launch::LOGIN_SHELL`, `launch::agent_binary`.
- Produces:
  - `pub fn probe_line(name: &str) -> String`, `pub fn parse_probe(out: &str, name: &str) -> Option<String>`.
  - `pub fn read(name: &str) -> Option<String>`: the process's value when set and non-empty, else (Unix) the login shell's, cached per name; None for an invalid name.
  - `pub fn import_gh_token() -> Option<String>`: `gh auth token`'s output, with a 5 s limit.

- [ ] **Step 1: Write the failing tests** (in `core/src/forge/env.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_probe_prints_one_variable_between_markers_unset_safe() {
        assert_eq!(probe_line("GH_TOKEN"), "printf '\\n%s\\n' MAYA-ENV-BEGIN \"GH_TOKEN=${GH_TOKEN-}\" MAYA-ENV-END");
    }

    #[test]
    fn the_probe_output_yields_the_value_between_the_markers_ignoring_rc_noise() {
        let out = "Restored session\nMAYA-ENV-BEGIN\nGH_TOKEN=ghp_abc=def\nMAYA-ENV-END\nSaving session...\n";
        assert_eq!(parse_probe(out, "GH_TOKEN").as_deref(), Some("ghp_abc=def"));
        assert_eq!(parse_probe("MAYA-ENV-BEGIN\nGH_TOKEN=\nMAYA-ENV-END\n", "GH_TOKEN"), None, "empty is none");
        assert_eq!(parse_probe("GH_TOKEN=ghp_x\n", "GH_TOKEN"), None, "no markers, nothing");
        assert_eq!(parse_probe("MAYA-ENV-BEGIN\nOTHER=x\nMAYA-ENV-END\n", "GH_TOKEN"), None);
    }

    #[test]
    fn an_invalid_name_is_never_read() {
        assert_eq!(read("GH_TOKEN; touch /tmp/maya-pwned"), None);
        assert_eq!(read("$(id)"), None);
        assert_eq!(read(""), None);
    }

    #[test]
    fn the_processes_own_value_wins() {
        // A name nothing else uses, so the test owns it.
        let name = "MAYA_ENV_TEST_OWN_VALUE";
        std::env::set_var(name, "from-process");
        assert_eq!(read(name).as_deref(), Some("from-process"));
        std::env::remove_var(name);
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core forge::env`
Expected: compile errors.

- [ ] **Step 3: Implement** (top of `core/src/forge/env.rs`)

```rust
//! Token variables. An app opened from the Dock gets launchd's bare
//! environment, not the user's `~/.zshrc`, so a variable missing from this
//! process is asked of the interactive login shell, once per name. Only a
//! valid name (`config::valid_env_name`) reaches the shell line.

use crate::config::valid_env_name;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const BEGIN: &str = "MAYA-ENV-BEGIN";
const END: &str = "MAYA-ENV-END";

/// The shell line that prints `NAME=value` between two marker lines, so
/// whatever the rc files print around it is told apart. Each item starts a
/// fresh line; the expansion is unset-safe for an rc file's `set -u`.
pub fn probe_line(name: &str) -> String {
    format!("printf '\\n%s\\n' {BEGIN} \"{name}=${{{name}-}}\" {END}")
}

/// The non-empty value of `name` between the probe's markers in `out`.
pub fn parse_probe(out: &str, name: &str) -> Option<String> {
    out.lines()
        .skip_while(|l| l.trim() != BEGIN)
        .skip(1)
        .take_while(|l| l.trim() != END)
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(unix)]
fn from_login_shell(name: &str) -> Option<String> {
    static CACHE: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);
    if let Some(v) = CACHE.lock().unwrap().get_or_insert_with(HashMap::new).get(name) {
        return v.clone();
    }
    let out = crate::command(crate::launch::LOGIN_SHELL).args(["-lic", &probe_line(name)]).stdin(Stdio::null()).stderr(Stdio::null()).output();
    let value = out.ok().and_then(|o| parse_probe(&String::from_utf8_lossy(&o.stdout), name));
    CACHE.lock().unwrap().get_or_insert_with(HashMap::new).insert(name.to_string(), value.clone());
    value
}

#[cfg(not(unix))]
fn from_login_shell(_name: &str) -> Option<String> {
    // A windowed app on Windows already gets the user's environment.
    None
}

/// The variable's value: this process's when set and non-empty, else the
/// login shell's. None for a name that is not a plain variable name.
pub fn read(name: &str) -> Option<String> {
    if !valid_env_name(name) {
        return None;
    }
    std::env::var(name).ok().filter(|v| !v.is_empty()).or_else(|| from_login_shell(name))
}

/// What `gh auth token` prints, when `gh` is installed and logged in.
/// Run once, at the first start after the upgrade; never logged.
pub fn import_gh_token() -> Option<String> {
    let gh = crate::launch::agent_binary("gh")?;
    let mut child = crate::command(gh).args(["auth", "token"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if started.elapsed() > Duration::from_secs(5) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    Some(out.trim().to_string()).filter(|t| !t.is_empty())
}
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p maya-core forge::env`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add core/src/forge/env.rs
git commit -m "feat: read token variables from the login shell and import gh's token

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: A pull request knows its forge

**Files:**
- Modify: `core/src/model.rs` (`PullRequest.forge`)
- Modify: `core/src/reviews.rs` (`ReviewPr.forge`, `ReviewPr::reference`, `parse_node`, `short_title`, `pr_spoken_line`, test helpers)
- Modify: `core/src/interpreter.rs` (`pr_summary`, `find_pr`, `validate`, test helper)
- Modify: `core/src/actions.rs` (test `ReviewPr` literal)
- Modify: `src-tauri/src/lib.rs` (`review_pr_for`, `open_review_pr`, `review_pr`)
- Modify: `src-tauri/src/listener.rs` (`review`/`open` actions pass `forge`)
- Modify: `src-tauri/src/net_app.rs` (`revalidate`, test helper)

**Interfaces:**
- Consumes (Task 1): `GitProvider` (`sigil()`, `label()`).
- Produces:
  - `PullRequest { number, url, state, forge: GitProvider }` (`#[serde(default)]`: an older assistant's card reads as GitHub).
  - `ReviewPr.forge: GitProvider` (first field after `number`), `ReviewPr::reference(&self) -> String` (`Org/bedrock#451`, `grp/r!12`).
  - Tauri commands `open_review_pr(forge: Option<GitProvider>, repo, number)` and `review_pr(forge: Option<GitProvider>, repo, number)`; None means GitHub.
  - Voice actions `review`/`open` carry `"forge"` after `validate`.

- [ ] **Step 1: Write the failing tests**

In `core/src/reviews.rs`'s tests, give the `pr()` helper `forge: GitProvider::Github` (add `use crate::config::GitProvider;` in the test module) and add:

```rust
    #[test]
    fn a_merge_request_is_named_with_a_bang_and_spoken_plainly() {
        let mr = ReviewPr { forge: GitProvider::Gitlab, repo: "grp/sub/bedrock".into(), number: 12, pr_review: Some(ReviewMark::Approved), pr_reviewers: vec!["jane".into()], ..pr() };
        assert_eq!(mr.reference(), "grp/sub/bedrock!12");
        assert_eq!(pr().reference(), "Org/bedrock#451");
        assert_eq!(pr_banner(&mr).0, "bedrock !12");
        assert_eq!(pr_spoken_line(&mr).as_deref(), Some("bedrock merge request 12 was approved"));
    }

    #[test]
    fn graphql_prs_are_github_prs() {
        let prs = parse_graphql(&response(&[node(1, "o/r", "jane", "", "", "")], &[], &[])).unwrap();
        assert_eq!(prs[0].forge, GitProvider::Github);
    }
```

In `core/src/model.rs`'s tests, add:

```rust
    #[test]
    fn a_pull_request_from_an_older_maya_is_a_github_one() {
        let pr: PullRequest = serde_json::from_str(r#"{"number":7,"url":"https://github.com/o/r/pull/7","state":"open"}"#).unwrap();
        assert_eq!(pr.forge, crate::config::GitProvider::Github);
        let json = serde_json::to_value(PullRequest { forge: crate::config::GitProvider::Gitlab, ..pr }).unwrap();
        assert_eq!(json["forge"], "gitlab");
    }
```

In `core/src/interpreter.rs`'s tests, give the `pr(...)` helper `forge: crate::config::GitProvider::Github` and add:

```rust
    #[test]
    fn merge_requests_are_listed_and_found_by_their_bang() {
        let mr = crate::reviews::ReviewPr { forge: crate::config::GitProvider::Gitlab, ..pr("grp/collector", 14, "Fix ERD", "bob", false) };
        let gh = pr("dosaki/collector", 14, "Add ERD overlay", "alex", false);
        assert!(pr_summary(&[mr.clone()]).starts_with("grp/collector!14 (GitLab) | Fix ERD | by bob"), "{}", pr_summary(&[mr.clone()]));
        let prs = [gh, mr];
        let a = validate(&json!({"kind":"review","pr":"grp/collector!14"}), &[], &[], &[], &prs).unwrap();
        assert_eq!((a["repo"].as_str(), a["number"].as_u64(), a["forge"].as_str()), (Some("grp/collector"), Some(14), Some("gitlab")));
        let b = validate(&json!({"kind":"open","pr":"dosaki/collector#14"}), &[], &[], &[], &prs).unwrap();
        assert_eq!(b["forge"], "github");
        let clash = validate(&json!({"kind":"review","pr":"14"}), &[], &[], &[], &prs).unwrap_err();
        assert_eq!(clash, "Which pull request: dosaki/collector#14 or grp/collector!14?");
    }
```

In `src-tauri/src/net_app.rs`'s tests, give the `pr(repo, number)` helper `forge: maya_core::config::GitProvider::Github` and add (next to `a_review_whose_pull_request_has_gone_is_refused_not_moved_to_another`, building `VoiceContext` the same way that test does):

```rust
    #[test]
    fn a_review_whose_forge_changed_is_refused() {
        // Same repo path and number, but the action was for GitLab's MR.
        let action = serde_json::json!({"kind": "review", "pr": "dosaki/collector!14", "forge": "gitlab", "repo": "dosaki/collector", "number": 14, "title": "dosaki/collector 14"});
        let ctx = VoiceContext { prs: vec![pr("dosaki/collector", 14)], ..Default::default() };
        assert_eq!(revalidate(&action, &ctx).unwrap_err(), "The pull request has changed; ask me again.", "the number found GitHub's PR, which is not the MR the user heard");
    }
```

(If `VoiceContext` has no `Default`, construct it exactly as the neighbouring test does, with the same `prs`.)

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core reviews:: interpreter:: model:: && cargo test -p maya net_app`
Expected: compile errors (`forge`, `reference` not found).

- [ ] **Step 3: Implement**

`core/src/model.rs`, in `PullRequest` after `state`:

```rust
    /// GitHub or GitLab; a card from an older Maya has none and is GitHub.
    #[serde(default)]
    pub forge: crate::config::GitProvider,
```

Fix every `PullRequest { … }` literal in the crate (`pr.rs`, `model.rs` tests, `store.rs`, `net/` tests: `cargo build --tests` lists them) by adding `forge: Default::default()`. In `pr::parse` (removed in Task 7) add `forge: GitProvider::Github`.

`core/src/reviews.rs`:

```rust
use crate::config::GitProvider;

pub struct ReviewPr {
    pub number: u64,
    /// GitHub or GitLab.
    pub forge: GitProvider,
    /// The repo's full path: `owner/name`, or `group/sub/name` on GitLab.
    pub repo: String,
    // … the rest unchanged
}

impl ReviewPr {
    /// `Org/bedrock#451` or `grp/r!12`: how the lists and the voice name it.
    pub fn reference(&self) -> String {
        format!("{}{}{}", self.repo, self.forge.sigil(), self.number)
    }

    /// The repo's last path segment.
    pub fn repo_name(&self) -> &str {
        self.repo.rsplit('/').next().unwrap_or(&self.repo)
    }
}
```

In `parse_node`, add `forge: GitProvider::Github,`. Replace `short_title` and the line in `pr_spoken_line`:

```rust
fn short_title(pr: &ReviewPr) -> String {
    format!("{} {}{}", pr.repo_name(), pr.forge.sigil(), pr.number)
}

/// What Maya says when an assigned PR is approved or needs changes.
pub fn pr_spoken_line(pr: &ReviewPr) -> Option<String> {
    let name = match pr.forge {
        GitProvider::Github => crate::notify::spoken_name(&format!("{} {}", pr.repo_name(), pr.number)),
        GitProvider::Gitlab => crate::notify::spoken_name(&format!("{} merge request {}", pr.repo_name(), pr.number)),
    };
    // … the match is unchanged
}
```

`core/src/interpreter.rs`, in `pr_summary` replace the first line of the closure:

```rust
            let label = if p.forge == crate::config::GitProvider::Gitlab { " (GitLab)" } else { "" };
            let mut line = format!("{}{label} | {} | by {}", p.reference(), p.title.trim(), p.author);
```

Update its doc comment to `` `repo#number` (`repo!number (GitLab)` for a merge request) | title | … ``. In `find_pr`:

```rust
    if let Some(p) = prs.iter().find(|p| p.reference().to_lowercase() == w) {
        return Ok(p);
    }
    // … unchanged …
    let words: String = w.chars().map(|c| if c.is_ascii_digit() || c == '#' || c == '!' { ' ' } else { c }).collect();
```

and both ambiguity messages become `format!("Which pull request: {} or {}?", a.reference(), b.reference())`. In `validate`'s `"review" | "open"` arm add `a["forge"] = serde_json::to_value(p.forge).unwrap_or_default();`.

`src-tauri/src/lib.rs`:

```rust
fn review_pr_for(state: &TauriState<AppState>, forge: Option<GitProvider>, repo: &str, number: u64) -> Result<reviews::ReviewPr, String> {
    let forge = forge.unwrap_or_default();
    state.reviews.lock().unwrap().prs.iter().find(|p| p.forge == forge && p.repo == repo && p.number == number).cloned().ok_or("That pull request is no longer on the list.".into())
}

#[tauri::command(async)]
fn open_review_pr(state: TauriState<AppState>, forge: Option<GitProvider>, repo: String, number: u64) -> Result<(), String> {
    let pr = review_pr_for(&state, forge, &repo, number)?;
    open_in_browser(&pr.url)
}

#[tauri::command(async)]
fn review_pr(state: TauriState<AppState>, forge: Option<GitProvider>, repo: String, number: u64) -> Result<String, String> {
    let pr = review_pr_for(&state, forge, &repo, number)?;
    actions::start_review(&local(&state), &pr)
}
```

(`use maya_core::config::GitProvider;` at the top.)

`src-tauri/src/listener.rs`, in the `"review"` and `"open"` arms:

```rust
            let forge = serde_json::from_value(action["forge"].clone()).ok();
            review_pr(state.clone(), forge, repo, number)?;   // open_review_pr(state.clone(), forge, repo, number)? in "open"
```

`src-tauri/src/net_app.rs`, in `revalidate`'s `Some("review")` arm build the reference with the action's forge:

```rust
        Some("review") => {
            if let (Some(repo), Some(number)) = (action["repo"].as_str(), action["number"].as_u64()) {
                let forge: GitProvider = serde_json::from_value(action["forge"].clone()).unwrap_or_default();
                asked["pr"] = Value::String(format!("{repo}{}{number}", forge.sigil()));
            }
        }
```

and the final check also compares forges, an action without one reading as GitHub:

```rust
    let forge_of = |v: &Value| v["forge"].as_str().unwrap_or("github").to_string();
    if action["kind"] == "review" && (fresh["repo"] != action["repo"] || fresh["number"] != action["number"] || forge_of(&fresh) != forge_of(action)) {
```

Add `forge: Default::default()` to the `ReviewPr` literal in `core/src/actions.rs`'s `start_review_runs_maya_agent_and_queues_the_review_name`.

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src src-tauri/src
git commit -m "feat: name a pull request by its forge as well as its repo and number

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: GitHub over its API

**Files:**
- Modify: `core/src/forge/github.rs` (empty from Task 2)
- Modify: `core/src/forge/mod.rs` (dispatch functions)
- Modify: `core/src/reviews.rs` (`parse_graphql`'s error texts no longer say `gh`)

**Interfaces:**
- Consumes: `Forge`, `Http`, `get_json`, `post_graphql`, `encode` (Task 2); `reviews::{QUERY, parse_graphql}`; `PullRequest` (Task 4).
- Produces:
  - `github::parse_pulls(&serde_json::Value) -> Option<PullRequest>`, `github::current_pr(http, forge, repo, branch) -> Result<Option<PullRequest>, String>`, `github::review_prs(http, forge) -> Result<Vec<ReviewPr>, String>`, `github::whoami(http, forge) -> Result<String, String>`.
  - In `forge/mod.rs`: `pub fn current_pr(http: &dyn Http, forge: &Forge, repo: &str, branch: &str) -> Result<Option<PullRequest>, String>`, `pub fn review_prs(http: &dyn Http, forge: &Forge) -> Result<Vec<ReviewPr>, String>`, `pub fn whoami(http: &dyn Http, forge: &Forge) -> Result<String, String>`; each matches on `forge.site.provider`. Until Task 6 the GitLab arms return `Err("GitLab is not supported yet".into())`.

- [ ] **Step 1: Write the failing tests** (in `core/src/forge/github.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GitConnection, GitProvider};
    use crate::forge::{fake::FakeHttp, ForgeSite};

    fn forge() -> Forge {
        Forge { site: ForgeSite::of(&GitConnection::default_for(GitProvider::Github)), token: "ghp_x".into() }
    }

    const PULLS: &str = r#"[
      {"number":9,"html_url":"https://github.com/Org/r/pull/9","state":"closed","draft":false,"merged_at":"2026-10-01T00:00:00Z"},
      {"number":12,"html_url":"https://github.com/Org/r/pull/12","state":"open","draft":true,"merged_at":null}
    ]"#;

    #[test]
    fn the_open_pr_wins_over_a_newer_closed_one_and_draft_is_a_state() {
        let v: serde_json::Value = serde_json::from_str(PULLS).unwrap();
        let pr = parse_pulls(&v).unwrap();
        assert_eq!((pr.number, pr.state.as_str(), pr.url.as_str(), pr.forge), (12, "draft", "https://github.com/Org/r/pull/12", GitProvider::Github));
        let merged: serde_json::Value = serde_json::from_str(r#"[{"number":9,"html_url":"u","state":"closed","draft":false,"merged_at":"x"}]"#).unwrap();
        assert_eq!(parse_pulls(&merged).unwrap().state, "merged");
        let closed: serde_json::Value = serde_json::from_str(r#"[{"number":9,"html_url":"u","state":"closed","draft":false,"merged_at":null}]"#).unwrap();
        assert_eq!(parse_pulls(&closed).unwrap().state, "closed");
        assert_eq!(parse_pulls(&serde_json::json!([])), None);
    }

    #[test]
    fn current_pr_asks_for_the_branch_on_the_owner_encoded() {
        let http = FakeHttp::new(vec![("/pulls", 200, PULLS)]);
        let pr = current_pr(&http, &forge(), "Org/r", "feat/x#1+y").unwrap().unwrap();
        assert_eq!(pr.number, 12);
        let url = &http.seen.lock().unwrap()[0].0;
        assert_eq!(url, "https://api.github.com/repos/Org/r/pulls?head=Org%3Afeat%2Fx%231%2By&state=all&sort=updated&direction=desc&per_page=10");
    }

    #[test]
    fn review_prs_posts_the_query_and_whoami_reads_the_login() {
        let body = r#"{"data":{"viewer":{"login":"Me"},"requested":{"nodes":[]},"reviewed":{"nodes":[]},"assigned":{"nodes":[]}}}"#;
        let http = FakeHttp::new(vec![("/graphql", 200, body), ("/user", 200, r#"{"login":"tiago"}"#)]);
        assert_eq!(review_prs(&http, &forge()).unwrap(), vec![]);
        assert!(http.seen.lock().unwrap()[0].2.as_deref().unwrap().contains("review-requested:@me"));
        assert_eq!(whoami(&http, &forge()).unwrap(), "tiago");
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core forge::github`
Expected: compile errors.

- [ ] **Step 3: Implement** (top of `core/src/forge/github.rs`)

```rust
//! GitHub: the PR of a branch (REST) and the PRs waiting on the user (the
//! GraphQL search `reviews::QUERY`, parsed by `reviews::parse_graphql`).

use super::{encode, get_json, post_graphql, Forge, Http};
use crate::config::GitProvider;
use crate::model::PullRequest;
use crate::reviews::ReviewPr;

/// The PR to show from a `GET /pulls` list: the first open one, else the
/// first (the list is newest-updated first).
pub fn parse_pulls(v: &serde_json::Value) -> Option<PullRequest> {
    let list = v.as_array()?;
    let p = list.iter().find(|p| p["state"] == "open").or_else(|| list.first())?;
    let state = if p["state"] == "open" {
        if p["draft"].as_bool() == Some(true) { "draft" } else { "open" }
    } else if p["merged_at"].is_string() {
        "merged"
    } else {
        "closed"
    };
    Some(PullRequest { number: p["number"].as_u64()?, url: p["html_url"].as_str()?.to_string(), state: state.into(), forge: GitProvider::Github })
}

/// The PR whose head is `branch` on the repo's owner. PRs from forks are not found.
pub fn current_pr(http: &dyn Http, forge: &Forge, repo: &str, branch: &str) -> Result<Option<PullRequest>, String> {
    let owner = repo.split('/').next().unwrap_or(repo);
    let url = format!("{}/repos/{repo}/pulls?head={}&state=all&sort=updated&direction=desc&per_page=10", forge.site.api_base(), encode(&format!("{owner}:{branch}")));
    Ok(parse_pulls(&get_json(http, forge, &url)?))
}

/// Open PRs asking for the user's review, reviewed by them, or assigned to them.
pub fn review_prs(http: &dyn Http, forge: &Forge) -> Result<Vec<ReviewPr>, String> {
    crate::reviews::parse_graphql(&post_graphql(http, forge, crate::reviews::QUERY)?)
}

/// The token's user.
pub fn whoami(http: &dyn Http, forge: &Forge) -> Result<String, String> {
    let v = get_json(http, forge, &format!("{}/user", forge.site.api_base()))?;
    v["login"].as_str().map(str::to_string).ok_or_else(|| "GitHub did not say who the token belongs to.".into())
}
```

In `core/src/forge/mod.rs`, after `post_graphql`:

```rust
/// The PR or MR of `branch` in `repo`.
pub fn current_pr(http: &dyn Http, forge: &Forge, repo: &str, branch: &str) -> Result<Option<crate::model::PullRequest>, String> {
    match forge.site.provider {
        GitProvider::Github => github::current_pr(http, forge, repo, branch),
        GitProvider::Gitlab => Err("GitLab is not supported yet".into()),
    }
}

/// The open PRs or MRs waiting on the user.
pub fn review_prs(http: &dyn Http, forge: &Forge) -> Result<Vec<crate::reviews::ReviewPr>, String> {
    match forge.site.provider {
        GitProvider::Github => github::review_prs(http, forge),
        GitProvider::Gitlab => Err("GitLab is not supported yet".into()),
    }
}

/// Who the token belongs to, for Settings' Test button.
pub fn whoami(http: &dyn Http, forge: &Forge) -> Result<String, String> {
    match forge.site.provider {
        GitProvider::Github => github::whoami(http, forge),
        GitProvider::Gitlab => Err("GitLab is not supported yet".into()),
    }
}
```

In `core/src/reviews.rs`'s `parse_graphql`: `"gh output was not JSON: {e}"` → `"GitHub's reply was not JSON: {e}"`, `"gh output had no viewer"` → `"GitHub's reply had no viewer"`, and its doc comment says "GitHub's reply to `QUERY`".

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p maya-core forge:: reviews::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/forge core/src/reviews.rs
git commit -m "feat: ask GitHub's API for a branch's PR and the PRs waiting on you

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: GitLab over its API

**Files:**
- Modify: `core/src/forge/gitlab.rs` (empty from Task 2)
- Modify: `core/src/forge/mod.rs` (the GitLab arms of the three dispatch functions)

**Interfaces:**
- Consumes: as Task 5; `reviews::{merge, Reason, ReviewMark, ReviewPr}`.
- Produces: `gitlab::QUERY`, `gitlab::parse_graphql(&str) -> Result<Vec<ReviewPr>, String>`, `gitlab::parse_mrs(&serde_json::Value) -> Option<PullRequest>`, `gitlab::current_pr`, `gitlab::review_prs`, `gitlab::whoami` (same signatures as GitHub's).

The field names below were checked against gitlab.com's live GraphQL schema on 2026-10-18: `currentUser { username reviewRequestedMergeRequests assignedMergeRequests }` (both take `state`, `first`, `sort`; `UPDATED_DESC` is a `MergeRequestSort`), `MergeRequest { iid (String) title webUrl draft updatedAt project { fullPath } author { username } reviewers }`, `MergeRequestReviewer { username bot mergeRequestInteraction { reviewState } }`, `MergeRequestReviewState`: `APPROVED REQUESTED_CHANGES REVIEWED REVIEW_STARTED UNAPPROVED UNREVIEWED`.

- [ ] **Step 1: Write the failing tests** (in `core/src/forge/gitlab.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GitConnection, GitProvider};
    use crate::forge::{fake::FakeHttp, ForgeSite};
    use crate::reviews::{Reason, ReviewMark};

    fn forge() -> Forge {
        let c = GitConnection { enabled: true, base_url: Some("https://gitlab.acme.io".into()), ..GitConnection::default_for(GitProvider::Gitlab) };
        Forge { site: ForgeSite::of(&c), token: "glpat".into() }
    }

    fn reviewer(name: &str, state: &str, bot: bool) -> String {
        format!(r#"{{"username":"{name}","bot":{bot},"mergeRequestInteraction":{{"reviewState":"{state}"}}}}"#)
    }

    fn mr(iid: u64, author: &str, updated: &str, reviewers: &[String]) -> String {
        format!(
            r#"{{"iid":"{iid}","title":"t{iid}","webUrl":"https://gitlab.acme.io/grp/sub/r/-/merge_requests/{iid}","draft":false,"updatedAt":"{updated}","project":{{"fullPath":"grp/sub/r"}},"author":{{"username":"{author}"}},"reviewers":{{"nodes":[{}]}}}}"#,
            reviewers.join(",")
        )
    }

    fn response(requested: &[String], assigned: &[String]) -> String {
        format!(r#"{{"data":{{"currentUser":{{"username":"Me","requested":{{"nodes":[{}]}},"assigned":{{"nodes":[{}]}}}}}}}}"#, requested.join(","), assigned.join(","))
    }

    #[test]
    fn requested_and_assigned_mrs_become_one_list_newest_first() {
        let a = mr(3, "jane", "2026-10-01T10:00:00Z", &[reviewer("me", "UNREVIEWED", false)]);
        let b = mr(7, "bob", "2026-10-02T10:00:00Z", &[]);
        let prs = parse_graphql(&response(&[a.clone(), b.clone()], &[b])).unwrap();
        assert_eq!(prs.iter().map(|p| p.number).collect::<Vec<_>>(), vec![7, 3]);
        let p = &prs[1];
        assert_eq!((p.forge, p.repo.as_str(), p.title.as_str(), p.author.as_str(), p.is_draft), (GitProvider::Gitlab, "grp/sub/r", "t3", "jane", false));
        assert_eq!(p.url, "https://gitlab.acme.io/grp/sub/r/-/merge_requests/3");
        assert_eq!((p.reasons.clone(), p.my_review), (vec![Reason::Review], None));
        assert_eq!(prs[0].reasons, vec![Reason::Review, Reason::Assigned]);
    }

    #[test]
    fn the_users_review_comes_from_their_reviewer_state() {
        let mine = |state: &str| parse_graphql(&response(&[mr(1, "jane", "", &[reviewer("ME", state, false)])], &[])).unwrap()[0].my_review;
        assert_eq!(mine("APPROVED"), Some(ReviewMark::Approved), "usernames match case-insensitively");
        assert_eq!(mine("REQUESTED_CHANGES"), Some(ReviewMark::ChangesRequested));
        assert_eq!(mine("REVIEWED"), Some(ReviewMark::Commented));
        for none in ["UNREVIEWED", "REVIEW_STARTED", "UNAPPROVED"] {
            assert_eq!(mine(none), None, "{none}");
        }
    }

    #[test]
    fn the_mrs_review_state_comes_from_other_people_changes_first() {
        let state = |author: &str, rs: &[String]| {
            let p = parse_graphql(&response(&[], &[mr(1, author, "", rs)])).unwrap().remove(0);
            (p.pr_review, p.pr_reviewers)
        };
        let split = [reviewer("jane", "APPROVED", false), reviewer("bob", "REQUESTED_CHANGES", false)];
        assert_eq!(state("me", &split), (Some(ReviewMark::ChangesRequested), vec!["bob".into()]));
        assert_eq!(state("me", &[reviewer("jane", "APPROVED", false)]), (Some(ReviewMark::Approved), vec!["jane".into()]));
        assert_eq!(state("me", &[reviewer("jane", "REVIEWED", false)]), (Some(ReviewMark::Commented), vec!["jane".into()]));
        assert_eq!(state("me", &[reviewer("renovate", "APPROVED", true)]), (None, vec![]), "bots do not count");
        assert_eq!(state("jane", &[reviewer("me", "APPROVED", false)]), (None, vec![]), "the user's own review is not the MR's");
        assert_eq!(state("jane", &[reviewer("jane", "APPROVED", false)]), (None, vec![]), "nor the author's");
    }

    #[test]
    fn no_current_user_means_the_token_was_not_accepted() {
        assert_eq!(parse_graphql(r#"{"data":{"currentUser":null}}"#).unwrap_err(), "the token was refused. It needs the read_api scope.");
        assert!(parse_graphql(r#"{"errors":[{"message":"Field 'x' doesn't exist"}]}"#).unwrap_err().contains("Field 'x'"));
        assert!(parse_graphql("<html>").is_err());
    }

    const MRS: &str = r#"[
      {"iid":4,"web_url":"https://gitlab.acme.io/grp/sub/r/-/merge_requests/4","state":"merged","draft":false},
      {"iid":5,"web_url":"https://gitlab.acme.io/grp/sub/r/-/merge_requests/5","state":"opened","draft":true}
    ]"#;

    #[test]
    fn the_open_mr_wins_and_states_map_to_the_cards() {
        let v: serde_json::Value = serde_json::from_str(MRS).unwrap();
        let pr = parse_mrs(&v).unwrap();
        assert_eq!((pr.number, pr.state.as_str(), pr.forge), (5, "draft", GitProvider::Gitlab));
        for (state, want) in [("merged", "merged"), ("closed", "closed"), ("locked", "closed"), ("opened", "open")] {
            let v = serde_json::json!([{ "iid": 1, "web_url": "u", "state": state, "draft": false }]);
            assert_eq!(parse_mrs(&v).unwrap().state, want, "{state}");
        }
        assert_eq!(parse_mrs(&serde_json::json!([])), None);
    }

    #[test]
    fn current_pr_asks_the_project_by_its_encoded_path_and_branch() {
        let http = FakeHttp::new(vec![("/merge_requests", 200, MRS)]);
        assert_eq!(current_pr(&http, &forge(), "grp/sub/r", "feat/x#1+y").unwrap().unwrap().number, 5);
        assert_eq!(
            http.seen.lock().unwrap()[0].0,
            "https://gitlab.acme.io/api/v4/projects/grp%2Fsub%2Fr/merge_requests?source_branch=feat%2Fx%231%2By&state=all&order_by=updated_at&per_page=10"
        );
    }

    #[test]
    fn review_prs_posts_to_the_graphql_url_and_whoami_reads_the_username() {
        let http = FakeHttp::new(vec![("/api/graphql", 200, &response(&[], &[])), ("/api/v4/user", 200, r#"{"username":"tiago"}"#)]);
        assert_eq!(review_prs(&http, &forge()).unwrap(), vec![]);
        assert_eq!(http.seen.lock().unwrap()[0].0, "https://gitlab.acme.io/api/graphql");
        assert_eq!(whoami(&http, &forge()).unwrap(), "tiago");
    }
}
```

(`FakeHttp::new` takes `&str` bodies; for `&response(…)` bind it to a `let` first if the borrow checker objects.)

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core forge::gitlab`
Expected: compile errors.

- [ ] **Step 3: Implement** (top of `core/src/forge/gitlab.rs`)

```rust
//! GitLab: the MR of a branch (REST) and the MRs waiting on the user (one
//! GraphQL query). A reviewer stays on an MR after reviewing, so the
//! review-requested list also holds the ones the user already reviewed.

use super::{encode, get_json, post_graphql, Forge, Http};
use crate::config::GitProvider;
use crate::model::PullRequest;
use crate::reviews::{merge, Reason, ReviewMark, ReviewPr};
use serde_json::Value;

pub const QUERY: &str = r#"query {
  currentUser {
    username
    requested: reviewRequestedMergeRequests(state: opened, first: 50, sort: UPDATED_DESC) { nodes { ...mr } }
    assigned: assignedMergeRequests(state: opened, first: 50, sort: UPDATED_DESC) { nodes { ...mr } }
  }
}
fragment mr on MergeRequest {
  iid title webUrl draft updatedAt
  project { fullPath }
  author { username }
  reviewers { nodes { username bot mergeRequestInteraction { reviewState } } }
}"#;

fn mark(state: &str) -> Option<ReviewMark> {
    match state {
        "APPROVED" => Some(ReviewMark::Approved),
        "REQUESTED_CHANGES" => Some(ReviewMark::ChangesRequested),
        "REVIEWED" => Some(ReviewMark::Commented),
        _ => None,
    }
}

/// (username, bot, mark) for each reviewer.
fn reviewers(m: &Value) -> Vec<(String, bool, Option<ReviewMark>)> {
    m["reviewers"]["nodes"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| Some((r["username"].as_str()?.to_string(), r["bot"].as_bool().unwrap_or(false), mark(r["mergeRequestInteraction"]["reviewState"].as_str().unwrap_or("")))))
                .collect()
        })
        .unwrap_or_default()
}

fn parse_node(m: &Value, reason: Reason, me: &str) -> Option<ReviewPr> {
    let author = m["author"]["username"].as_str().unwrap_or("").to_string();
    let rs = reviewers(m);
    let my_review = if author.eq_ignore_ascii_case(me) { None } else { rs.iter().find(|(u, _, _)| u.eq_ignore_ascii_case(me)).and_then(|(_, _, k)| *k) };
    let others: Vec<&(String, bool, Option<ReviewMark>)> = rs.iter().filter(|(u, bot, _)| !bot && !u.eq_ignore_ascii_case(me) && !u.eq_ignore_ascii_case(&author)).collect();
    let (pr_review, pr_reviewers) = [ReviewMark::ChangesRequested, ReviewMark::Approved, ReviewMark::Commented]
        .into_iter()
        .map(|want| (want, others.iter().filter(|(_, _, k)| *k == Some(want)).map(|(u, _, _)| u.clone()).collect::<Vec<_>>()))
        .find(|(_, who)| !who.is_empty())
        .map_or((None, vec![]), |(k, who)| (Some(k), who));
    Some(ReviewPr {
        number: m["iid"].as_str()?.parse().ok()?,
        forge: GitProvider::Gitlab,
        repo: m["project"]["fullPath"].as_str()?.to_string(),
        title: m["title"].as_str().unwrap_or("").to_string(),
        author,
        url: m["webUrl"].as_str()?.to_string(),
        is_draft: m["draft"].as_bool().unwrap_or(false),
        updated_at: m["updatedAt"].as_str().unwrap_or("").to_string(),
        reasons: vec![reason],
        my_review,
        pr_review,
        pr_reviewers,
    })
}

/// GitLab's reply to `QUERY` as one list of review MRs.
pub fn parse_graphql(json: &str) -> Result<Vec<ReviewPr>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("GitLab's reply was not JSON: {e}"))?;
    let user = &v["data"]["currentUser"];
    let Some(me) = user["username"].as_str() else {
        if let Some(m) = v["errors"].as_array().and_then(|e| e.first()).and_then(|e| e["message"].as_str()) {
            return Err(format!("GitLab said: {m}"));
        }
        // An anonymous query answers with a null user rather than a 401.
        return Err("the token was refused. It needs the read_api scope.".into());
    };
    let list = |key: &str, reason: Reason| -> Vec<ReviewPr> { user[key]["nodes"].as_array().map(|a| a.iter().filter_map(|m| parse_node(m, reason, me)).collect()).unwrap_or_default() };
    Ok(merge(list("requested", Reason::Review), list("assigned", Reason::Assigned)))
}

/// The MR to show from a `GET /merge_requests` list: the first open one, else the first.
pub fn parse_mrs(v: &Value) -> Option<PullRequest> {
    let list = v.as_array()?;
    let m = list.iter().find(|m| m["state"] == "opened").or_else(|| list.first())?;
    let state = match m["state"].as_str()? {
        "opened" if m["draft"].as_bool() == Some(true) => "draft",
        "opened" => "open",
        "merged" => "merged",
        _ => "closed",
    };
    Some(PullRequest { number: m["iid"].as_u64()?, url: m["web_url"].as_str()?.to_string(), state: state.into(), forge: GitProvider::Gitlab })
}

pub fn current_pr(http: &dyn Http, forge: &Forge, repo: &str, branch: &str) -> Result<Option<PullRequest>, String> {
    let url = format!("{}/projects/{}/merge_requests?source_branch={}&state=all&order_by=updated_at&per_page=10", forge.site.api_base(), encode(repo), encode(branch));
    Ok(parse_mrs(&get_json(http, forge, &url)?))
}

pub fn review_prs(http: &dyn Http, forge: &Forge) -> Result<Vec<ReviewPr>, String> {
    parse_graphql(&post_graphql(http, forge, QUERY)?)
}

pub fn whoami(http: &dyn Http, forge: &Forge) -> Result<String, String> {
    let v = get_json(http, forge, &format!("{}/user", forge.site.api_base()))?;
    v["username"].as_str().map(str::to_string).ok_or_else(|| "GitLab did not say who the token belongs to.".into())
}
```

In `core/src/forge/mod.rs`, replace the three `Err("GitLab is not supported yet".into())` arms with `gitlab::current_pr(http, forge, repo, branch)`, `gitlab::review_prs(http, forge)` and `gitlab::whoami(http, forge)`.

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p maya-core forge::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/forge
git commit -m "feat: ask GitLab's API for a branch's MR and the MRs waiting on you

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Core wiring — badge, review list, review clone; `gh` removed

**Files:**
- Modify: `core/src/forge/mod.rs` (`origin_of`)
- Modify: `core/src/pr.rs` (new `lookup`, `branch_of`, `PrCache::clear`; old `parse`/`lookup` removed)
- Modify: `core/src/reviews.rs` (`fetch`, `Fetched`, `combine`; `find_checkout`, `resolve_target`, `shell_command` through sites; `remote_matches` and old `origin_of` removed)
- Modify: `core/src/actions.rs` (`start_review` uses sites)
- Modify: `core/src/store.rs` (`clear_prs`)
- Modify: `core/src/lib.rs` (remove `gh`, `GH_TOKEN_VARS`, `has_gh_token`, the probe functions, `shell_gh_tokens`, `mod gh_token_tests`; the `command` doc comment's "(gh, git, …)" becomes "(git, curl, …)")

**Interfaces:**
- Consumes: Tasks 2–6.
- Produces:
  - `forge::origin_of(dir: &Path) -> Option<String>`.
  - `pr::branch_of(dir: &Path) -> Option<String>`; `pr::lookup(dir: &Path, forges: &[Forge], http: &dyn Http) -> Result<Option<PullRequest>, String>`; `PrCache::clear(&mut self)`; `Store::clear_prs(&mut self)`.
  - `reviews::Fetched { pub forge: GitProvider, pub result: Result<Vec<ReviewPr>, String> }`; `reviews::fetch(forges: &[Forge], http: &dyn Http) -> Vec<Fetched>`; `reviews::combine(previous: &[ReviewPr], fetched: Vec<Fetched>, errors: Vec<ForgeError>) -> (Vec<ReviewPr>, Vec<ForgeError>)`.
  - `reviews::find_checkout(projects_root: &Path, site: &ForgeSite, repo: &str) -> Option<PathBuf>`; `reviews::resolve_target(projects_root, clones_root, site: &ForgeSite, repo, number, live_cwds) -> ReviewTarget`; `reviews::shell_command(agent, target, clone_url: &str, repo, number, prompt_file, grok_id) -> String`.

- [ ] **Step 1: Write the failing tests**

In `core/src/pr.rs`'s tests, replace `parses_gh_pr_view_output_with_lowercase_state_and_draft` and `lookup_in_a_directory_without_a_repo_is_none` with:

```rust
    use crate::config::{GitConnection, GitProvider};
    use crate::forge::{fake::FakeHttp, Forge, ForgeSite};

    fn github() -> Forge {
        Forge { site: ForgeSite::of(&GitConnection::default_for(GitProvider::Github)), token: "t".into() }
    }

    #[cfg(unix)]
    fn repo_on_branch(origin: Option<&str>, branch: &str) -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| assert!(std::process::Command::new("git").args(["-C", t.path().to_str().unwrap()]).args(args).status().unwrap().success(), "{args:?}");
        git(&["init", "-q", "-b", branch]);
        if let Some(o) = origin {
            git(&["remote", "add", "origin", o]);
        }
        t
    }

    #[cfg(unix)]
    #[test]
    fn lookup_asks_the_routed_forge_for_the_current_branch() {
        let t = repo_on_branch(Some("git@github.com:Org/r.git"), "feat/x");
        let http = FakeHttp::new(vec![("/pulls", 200, r#"[{"number":3,"html_url":"https://github.com/Org/r/pull/3","state":"open","draft":false}]"#)]);
        assert_eq!(lookup(t.path(), &[github()], &http).unwrap().map(|p| p.number), Some(3));
        assert!(http.seen.lock().unwrap()[0].0.contains("head=Org%3Afeat%2Fx"));
    }

    #[cfg(unix)]
    #[test]
    fn lookup_is_none_without_a_repo_a_known_forge_or_an_origin() {
        let http = FakeHttp::new(vec![]);
        let plain = tempfile::tempdir().unwrap();
        assert_eq!(lookup(plain.path(), &[github()], &http), Ok(None));
        let elsewhere = repo_on_branch(Some("git@gitlab.com:grp/r.git"), "main");
        assert_eq!(lookup(elsewhere.path(), &[github()], &http), Ok(None), "GitLab is off");
        let no_origin = repo_on_branch(None, "main");
        assert_eq!(lookup(no_origin.path(), &[github()], &http), Ok(None));
        assert!(http.seen.lock().unwrap().is_empty(), "no call was made");
    }

    #[test]
    fn clear_forgets_every_result() {
        let mut c = PrCache::default();
        c.set("/a", None, 1000);
        c.clear();
        assert_eq!(c.due(&["/a".to_string()], 1000), vec!["/a".to_string()]);
    }
```

In `core/src/reviews.rs`'s tests, delete `remote_matching_accepts_ssh_and_https_forms_case_insensitively`, change the two `find_checkout`/`resolve_target` tests to pass `&github_site()` (and add a GitLab case), change the `shell_command` test to pass the clone URL, and add:

```rust
    use crate::forge::{ForgeError, ForgeSite};

    fn github_site() -> ForgeSite {
        ForgeSite::of(&crate::config::GitConnection::default_for(GitProvider::Github))
    }

    #[test]
    fn find_checkout_matches_a_gitlab_checkout_through_its_site() {
        let t = tempfile::tempdir().unwrap();
        let gl = ForgeSite::of(&crate::config::GitConnection { base_url: Some("https://gitlab.acme.io".into()), enabled: true, ..crate::config::GitConnection::default_for(GitProvider::Gitlab) });
        let r = repo_with_origin(t.path(), "r", "git@gitlab.acme.io:grp/sub/r.git");
        assert_eq!(find_checkout(t.path(), &gl, "grp/sub/r"), Some(r));
        assert_eq!(find_checkout(t.path(), &github_site(), "grp/sub/r"), None, "the same path on GitHub is another repo");
    }

    #[test]
    fn combine_keeps_a_failing_forges_last_prs_and_says_why() {
        let gh_old = ReviewPr { number: 1, url: "gh1".into(), updated_at: "2026-10-01".into(), ..pr() };
        let gl_old = ReviewPr { number: 2, url: "gl2".into(), forge: GitProvider::Gitlab, updated_at: "2026-10-02".into(), ..pr() };
        let gh_new = ReviewPr { number: 3, url: "gh3".into(), updated_at: "2026-10-03".into(), ..pr() };
        let fetched = vec![Fetched { forge: GitProvider::Github, result: Ok(vec![gh_new]) }, Fetched { forge: GitProvider::Gitlab, result: Err("401".into()) }];
        let (prs, errors) = combine(&[gh_old, gl_old], fetched, vec![]);
        assert_eq!(prs.iter().map(|p| p.number).collect::<Vec<_>>(), vec![3, 2], "GitHub's list is replaced, GitLab's kept");
        assert_eq!(errors, vec![ForgeError { forge: GitProvider::Gitlab, message: "GitLab check failed: 401".into() }]);
    }

    #[test]
    fn combine_drops_a_forge_that_was_not_asked_and_passes_its_error_on() {
        let gl_old = ReviewPr { forge: GitProvider::Gitlab, ..pr() };
        let missing = vec![ForgeError { forge: GitProvider::Gitlab, message: "No GitLab token: paste one in Settings → Git.".into() }];
        let (prs, errors) = combine(&[gl_old], vec![], missing.clone());
        assert!(prs.is_empty());
        assert_eq!(errors, missing);
    }

    #[test]
    fn fetch_asks_every_forge() {
        let gh = crate::forge::Forge { site: github_site(), token: "t".into() };
        let body = r#"{"data":{"viewer":{"login":"Me"},"requested":{"nodes":[]},"reviewed":{"nodes":[]},"assigned":{"nodes":[]}}}"#;
        let http = crate::forge::fake::FakeHttp::new(vec![("/graphql", 200, body)]);
        let out = fetch(&[gh], &http);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].forge, out[0].result.clone()), (GitProvider::Github, Ok(vec![])));
    }
```

The updated `shell_command` assertion:

```rust
        let s = shell_command(crate::model::Harness::Codex, &clone, "https://github.com/Org/bedrock.git", "Org/bedrock", 451, file, None);
        assert!(s.starts_with("mkdir -p '/Users/x/dev/reviews' && git clone 'https://github.com/Org/bedrock.git' '/Users/x/dev/reviews/bedrock-451' && "), "{s}");
```

In `core/src/actions.rs`'s `start_review_runs_maya_agent_and_queues_the_review_name`, the assertion becomes `command.contains("git clone 'https://github.com/Org/bedrock.git'")`, and add a test:

```rust
    #[test]
    fn a_review_on_a_forge_that_is_off_says_so() {
        let (_dir, store) = store_with_projects(&["elsewhere"]);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        let pr = crate::reviews::ReviewPr { number: 3, forge: crate::config::GitProvider::Gitlab, repo: "grp/r".into(), title: "t".into(), author: "a".into(), url: "u".into(), is_draft: false, updated_at: String::new(), reasons: vec![], my_review: None, pr_review: None, pr_reviewers: vec![] };
        assert_eq!(start_review(&l, &pr).unwrap_err(), "Turn on GitLab in Settings → Git first.");
    }
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core pr:: reviews:: actions::`
Expected: compile errors (`combine`, `Fetched`, new signatures).

- [ ] **Step 3: Implement**

`core/src/forge/mod.rs`:

```rust
/// The `origin` remote's URL of the checkout at `dir`.
pub fn origin_of(dir: &std::path::Path) -> Option<String> {
    let out = crate::command("git").args(["-C", dir.to_str()?, "config", "--get", "remote.origin.url"]).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())
}
```

`core/src/pr.rs`: delete `parse` and the old `lookup`; update the module's comments (`TTL_MS`: "before the forge is asked again"); add:

```rust
use crate::forge::{Forge, Http};

/// The checked-out branch; None when detached or not a repo.
pub fn branch_of(dir: &Path) -> Option<String> {
    // `symbolic-ref` also names a branch with no commits yet, and fails when detached.
    let out = crate::command("git").args(["-C", dir.to_str()?, "symbolic-ref", "--short", "-q", "HEAD"]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    let b = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !b.is_empty()).then_some(b)
}

/// The PR or MR of `dir`'s branch, from the forge its `origin` belongs to.
/// Ok(None) when there is no repo, origin, branch or switched-on forge.
pub fn lookup(dir: &Path, forges: &[Forge], http: &dyn Http) -> Result<Option<PullRequest>, String> {
    let Some(origin) = crate::forge::origin_of(dir) else { return Ok(None) };
    let Some((forge, repo)) = crate::forge::route(&origin, forges) else { return Ok(None) };
    let Some(branch) = branch_of(dir) else { return Ok(None) };
    crate::forge::current_pr(http, forge, &repo, &branch)
}
```

and in `impl PrCache`:

```rust
    /// Forgets everything, so every directory is looked up again.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
```

`core/src/store.rs`, next to `set_pr`:

```rust
    /// Forgets every PR lookup: the Git settings changed.
    pub fn clear_prs(&mut self) {
        self.prs.clear();
    }
```

`core/src/reviews.rs`: delete `fetch` (the `gh` one), `remote_matches` and `origin_of`; remove the now-unused `std::process::Stdio` import if nothing else uses it. Add:

```rust
use crate::forge::{Forge, ForgeError, ForgeSite, Http};

/// One forge's answer to "what is waiting on me".
pub struct Fetched {
    pub forge: GitProvider,
    pub result: Result<Vec<ReviewPr>, String>,
}

/// Asks every forge at once.
pub fn fetch(forges: &[Forge], http: &dyn Http) -> Vec<Fetched> {
    std::thread::scope(|s| {
        let handles: Vec<_> = forges.iter().map(|f| (f.site.provider, s.spawn(move || crate::forge::review_prs(http, f)))).collect();
        handles.into_iter().map(|(forge, h)| Fetched { forge, result: h.join().unwrap_or_else(|_| Err("the check stopped unexpectedly".into())) }).collect()
    })
}

/// The new list and the error lines: a forge that answered replaces its
/// PRs, one that failed keeps its last ones and adds a line, and a forge
/// that was not asked (off, or no token) drops out. `errors` holds the
/// lines for forges that were not asked.
pub fn combine(previous: &[ReviewPr], fetched: Vec<Fetched>, mut errors: Vec<ForgeError>) -> (Vec<ReviewPr>, Vec<ForgeError>) {
    let mut out = Vec::new();
    for f in fetched {
        match f.result {
            Ok(prs) => out = merge(out, prs),
            Err(e) => {
                out = merge(out, previous.iter().filter(|p| p.forge == f.forge).cloned().collect());
                errors.push(ForgeError { forge: f.forge, message: format!("{} check failed: {e}", f.forge.label()) });
            }
        }
    }
    (out, errors)
}
```

Replace `find_checkout` and `resolve_target`:

```rust
fn is_checkout_of(dir: &Path, site: &ForgeSite, repo: &str) -> bool {
    crate::forge::origin_of(dir).and_then(|o| crate::forge::route(&o, std::slice::from_ref(site)).map(|(_, p)| p)).is_some_and(|p| p.eq_ignore_ascii_case(repo))
}

/// A checkout of `repo` on `site` under `projects_root`: the folder named
/// after the repo when it is one, else the first match by folder name.
pub fn find_checkout(projects_root: &Path, site: &ForgeSite, repo: &str) -> Option<PathBuf> {
    let Ok(entries) = std::fs::read_dir(projects_root) else { return None };
    let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir() && p.join(".git").exists()).collect();
    dirs.sort();
    let named = projects_root.join(repo.rsplit('/').next().unwrap_or(repo));
    if dirs.contains(&named) && is_checkout_of(&named, site, repo) {
        return Some(named);
    }
    dirs.into_iter().find(|d| is_checkout_of(d, site, repo))
}

/// The free project checkout if there is one; else `<clones>/<repo>-<number>`,
/// cloned unless it already exists.
pub fn resolve_target(projects_root: &Path, clones_root: &Path, site: &ForgeSite, repo: &str, number: u64, live_cwds: &[String]) -> ReviewTarget {
    if let Some(dir) = find_checkout(projects_root, site, repo) {
        if !is_busy(&dir, live_cwds) {
            return ReviewTarget { dir, clone: false };
        }
    }
    let repo_name = repo.rsplit('/').next().unwrap_or(repo);
    let dir = clones_root.join(format!("{repo_name}-{number}"));
    let clone = !dir.is_dir();
    ReviewTarget { dir, clone }
}
```

`shell_command` gains `clone_url: &str` after `target`, and its clone line becomes:

```rust
        format!("mkdir -p {parent} && git clone {} {dir} && {start}", shell_single_quote(clone_url))
```

(its doc comment: "a `git clone` first when needed, with the user's own git credentials").

`core/src/actions.rs`, in `start_review`, read the sites with the rest of the config and use them:

```rust
    let (agent, projects, clones, live, maya_dir, template, agents, sites) = {
        let store = l.store.lock().unwrap();
        (store.config.brain(), store.config.projects_dir_path(), store.config.clones_dir_path(), store.live_cwds(), store.claude_dir().join("maya"), store.config.review_prompt.clone(), store.agents_source(), crate::forge::sites(&store.config.git_connections()))
    };
    let site = sites.into_iter().find(|s| s.provider == pr.forge).ok_or_else(|| format!("Turn on {} in Settings → Git first.", pr.forge.label()))?;
    let projects = projects.ok_or("Set a projects directory in Settings first.")?;
```

then `resolve_target(&projects, &clones, &site, &pr.repo, pr.number, &live)` and `shell_command(agent, &target, &site.clone_url(&pr.repo), &pr.repo, pr.number, &file, grok_id.as_deref())`. (The site check comes before the projects check so the test above needs no projects directory; `store_with_projects` sets one anyway.)

`core/src/lib.rs`: delete `gh()`, `GH_TOKEN_VARS`, `has_gh_token`, `GH_PROBE_BEGIN`, `GH_PROBE_END`, `gh_token_probe`, `parse_gh_token_probe`, `shell_gh_tokens` and `mod gh_token_tests`.

`core/src/pending_names.rs:459`'s comment says "before `gh repo clone` creates its folder": make it "before `git clone` creates its folder".

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p maya-core`
Expected: PASS. `grep -rn 'gh()\|gh repo clone\|GH_TOKEN_VARS' core/src` prints nothing. (`src-tauri` does not compile until Task 8; that is expected here.)

- [ ] **Step 5: Commit**

```bash
git add core/src
git commit -m "feat: look up PRs, list reviews and clone through the forges, not gh

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: App wiring — key store, commands, polls, first-start import

**Files:**
- Modify: `src-tauri/src/keystore.rs` (`GITHUB_TOKEN`, `GITLAB_TOKEN` items; `remove` on all three platforms)
- Create: `src-tauri/src/git.rs`
- Modify: `src-tauri/src/lib.rs` (`mod git;`, `ReviewState`, `poll_reviews`, `poll_pull_requests`, `set_config`, startup, `generate_handler!`)

**Interfaces:**
- Consumes: Tasks 1–7.
- Produces:
  - `keystore::GITHUB_TOKEN`, `keystore::GITLAB_TOKEN`, `keystore::remove(&Item) -> Result<(), String>`.
  - `git::token_item(GitProvider) -> &'static keystore::Item`, `git::http() -> &'static forge::Ureq`, `git::forges(&[GitConnection]) -> (Vec<Forge>, Vec<ForgeError>)`, `git::settings_changed(&AppHandle)`, `git::import_if_needed(&AppHandle)`.
  - Tauri commands (camelCase JSON): `get_git_status() -> { github: { hasToken }, gitlab: { hasToken } }`, `store_git_token(provider, token)`, `clear_git_token(provider)`, `test_git_connection(provider) -> String` (the login).
  - `ReviewState { prs, errors: Vec<ForgeError>, off: bool, fetched_at }` (JSON `prs`, `errors`, `off`, `fetchedAt`).

- [ ] **Step 1: Write the failing tests**

In `src-tauri/src/keystore.rs`'s tests:

```rust
    #[test]
    fn the_git_tokens_have_their_own_items() {
        assert_eq!((GITHUB_TOKEN.service, GITHUB_TOKEN.account, GITHUB_TOKEN.linux_account), ("maya-github", "token", "github-token"));
        assert_eq!((GITLAB_TOKEN.service, GITLAB_TOKEN.account, GITLAB_TOKEN.linux_account), ("maya-gitlab", "token", "gitlab-token"));
        assert_eq!(secret_tool_clear_args(&GITLAB_TOKEN), ["clear", "service", "maya", "account", "gitlab-token"].map(String::from).to_vec());
    }
```

and extend the Windows round-trip test to end with `remove(&item).unwrap(); assert_eq!(load(&item), Ok(None)); remove(&item).unwrap();` in place of the raw `CredDeleteW` call ("removing what is not there is fine").

In `src-tauri/src/git.rs`:

```rust
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
```

(Unix's `forge::env::read` would also ask the login shell for `MAYA_GIT_TEST_GITLAB_UNSET`; it is unset there too.)

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya git:: keystore::`
Expected: compile errors.

- [ ] **Step 3: Implement**

`src-tauri/src/keystore.rs`, after `NETWORK_KEY`:

```rust
pub const GITHUB_TOKEN: Item = Item { service: "maya-github", account: "token", label: "Maya GitHub token", linux_account: "github-token" };
pub const GITLAB_TOKEN: Item = Item { service: "maya-gitlab", account: "token", label: "Maya GitLab token", linux_account: "gitlab-token" };
```

Add `remove` for each platform, beside `load`:

```rust
/// Deletes the secret; removing one that is not there is fine.
#[cfg(target_os = "macos")]
pub fn remove(item: &Item) -> Result<(), String> {
    let out = maya_core::command("security")
        .args(["delete-generic-password", "-s", item.service, "-a", item.account])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("could not run security: {e}"))?;
    match out.status.code() {
        Some(0) | Some(ERR_SEC_ITEM_NOT_FOUND) => Ok(()),
        _ => Err(format!("Keychain: {}", String::from_utf8_lossy(&out.stderr).trim())),
    }
}

/// Deletes the secret; removing one that is not there is fine.
#[cfg(windows)]
pub fn remove(item: &Item) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_NOT_FOUND};
    use windows_sys::Win32::Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC};
    // SAFETY: the target string outlives the call.
    unsafe {
        if CredDeleteW(wide(item.service).as_ptr(), CRED_TYPE_GENERIC, 0) != 0 || GetLastError() == ERROR_NOT_FOUND {
            Ok(())
        } else {
            Err(format!("Credential Manager: {}", std::io::Error::last_os_error()))
        }
    }
}

/// `secret-tool` arguments that delete the item.
#[cfg(any(test, target_os = "linux"))]
fn secret_tool_clear_args(item: &Item) -> Vec<String> {
    ["clear", "service", "maya", "account", item.linux_account].map(String::from).to_vec()
}

/// Deletes the secret; `secret-tool clear` succeeds when nothing matches.
#[cfg(target_os = "linux")]
pub fn remove(item: &Item) -> Result<(), String> {
    let out = maya_core::command("secret-tool")
        .args(secret_tool_clear_args(item))
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("could not run secret-tool (install libsecret-tools): {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("GNOME Keyring: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}
```

Create `src-tauri/src/git.rs`:

```rust
//! Settings → Git on the desktop: tokens from the key store or a
//! variable, the forges the polls ask, the page's commands, and the
//! one-time import of `gh`'s token.

use crate::{keystore, AppState};
use maya_core::log;
use maya_core::config::{self, GitConnection, GitProvider};
use maya_core::forge::{self, Forge, ForgeError};
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
    app.state::<AppState>().store.lock().unwrap().clear_prs();
    let handle = app.clone();
    std::thread::spawn(move || {
        crate::poll_reviews(&handle);
        crate::poll_pull_requests(&handle);
    });
}

/// The first start after 0.24 (or ever): GitHub on with `gh`'s token when
/// `gh` is logged in, else reading `GH_TOKEN`; GitLab off. Never again after.
pub fn import_if_needed(app: &AppHandle) {
    if app.state::<AppState>().store.lock().unwrap().config.git.is_some() {
        return;
    }
    let stored = match forge::env::import_gh_token() {
        Some(token) => match keystore::store(&keystore::GITHUB_TOKEN, &token) {
            Ok(()) => true,
            Err(e) => {
                log::line("app", format!("git: could not keep gh's token: {e}"));
                false
            }
        },
        None => false,
    };
    log::line("app", if stored { "git: imported gh's GitHub token" } else { "git: no gh token to import; GitHub reads GH_TOKEN" });
    if let Err(e) = crate::update_config(app, |c| c.git = Some(config::git_after_import(stored))) {
        log::line("app", format!("git: could not save the Git settings: {e}"));
    }
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
    let f = forge::resolve(&conn, |p| keystore::load(token_item(p)).ok().flatten(), forge::env::read)?;
    forge::whoami(http(), &f)
}
```

`src-tauri/src/lib.rs`:

- `mod git;` beside the other modules.
- `ReviewState`:

```rust
#[derive(Default, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewState {
    pub prs: Vec<reviews::ReviewPr>,
    /// One line per forge whose last check failed or that has no token.
    pub errors: Vec<maya_core::forge::ForgeError>,
    /// Neither GitHub nor GitLab is switched on.
    pub off: bool,
    pub fetched_at: Option<u64>,
}
```

- `poll_reviews` becomes `pub(crate)` and reads:

```rust
pub(crate) fn poll_reviews(app: &AppHandle) {
    let conns = app.state::<AppState>().store.lock().unwrap().config.git_connections();
    let (forges, missing) = git::forges(&conns);
    let off = forges.is_empty() && missing.is_empty();
    let fetched = reviews::fetch(&forges, git::http());
    for f in &fetched {
        if let Err(e) = &f.result {
            log::line("app", format!("pull requests: {}: {e}", f.forge.label()));
        }
    }
    let state = app.state::<AppState>();
    let decided;
    let snapshot = {
        let mut r = state.reviews.lock().unwrap();
        let (prs, errors) = reviews::combine(&r.prs, fetched, missing);
        decided = state.pr_notifier.lock().unwrap().take_changes(&prs);
        r.prs = prs;
        r.errors = errors;
        r.off = off;
        r.fetched_at = Some(now_ms());
        r.clone()
    };
    announce_pr_reviews(app, &decided);
    let _ = app.emit("reviews", &snapshot);
}
```

- `poll_pull_requests` becomes `pub(crate)` and resolves the forges once per round, only when something is due:

```rust
pub(crate) fn poll_pull_requests(app: &AppHandle) {
    let (due, conns) = {
        let state = app.state::<AppState>();
        let mut store = state.store.lock().unwrap();
        (store.pr_dirs_due(now_ms()), store.config.git_connections())
    };
    if due.is_empty() {
        return;
    }
    let (forges, _) = git::forges(&conns);
    for dir in due {
        let pr = pr::lookup(std::path::Path::new(&dir), &forges, git::http()).unwrap_or_else(|e| {
            log::line("app", format!("pull request for {dir}: {e}"));
            None
        });
        let state = app.state::<AppState>();
        state.store.lock().unwrap().set_pr(&dir, pr, now_ms());
    }
    refresh_and_emit(app);
}
```

- `set_config`: after the `projects_dir` check add `config::validate_git(config.git.as_deref().unwrap_or(&[]))?;`; after `let mut config = config.for_this_platform();` add `config.normalize_git();`; inside the store block, next to the network fields, keep an imported `git` that the page left out:

```rust
        // A page holding a config from before the first-start import must not undo it.
        if config.git.is_none() {
            config.git = before.git.clone();
        }
```

and after `apply_config_change(&app, &before, &config);`:

```rust
    if before.git_connections() != config.git_connections() {
        git::settings_changed(&app);
    }
```

- Startup: replace the two poll threads with one that imports first, then starts both loops:

```rust
            let git_handle = app.handle().clone();
            std::thread::spawn(move || {
                git::import_if_needed(&git_handle);
                let pr_handle = git_handle.clone();
                std::thread::spawn(move || loop {
                    poll_pull_requests(&pr_handle);
                    std::thread::sleep(Duration::from_secs(10));
                });
                loop {
                    poll_reviews(&git_handle);
                    std::thread::sleep(Duration::from_secs(120));
                }
            });
```

- `generate_handler!`: add `git::get_git_status, git::store_git_token, git::clear_git_token, git::test_git_connection,`.
- `announce_pr_reviews` needs no change: `pr_banner` already names an MR `repo !12`.

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test --workspace`
Expected: PASS. `cargo clippy --workspace --all-targets` shows no new warnings.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src
git commit -m "feat: resolve Git tokens from the key store, poll both forges, import gh's token once

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: Pull requests tab and card badge

**Files:**
- Create: `src/assets/icons/github.svg`, `src/assets/icons/gitlab.svg`
- Modify: `src/icons.ts` (register both)
- Modify: `src/types.ts` (`GitProvider`, `PullRequest.forge`, `ReviewPr.forge`, `ForgeError`, `ReviewState`, `prButton`)
- Modify: `src/reviews.ts` (forge mark, `!n`, filter keys, per-forge errors, the off state, `ReviewAction.forge`)
- Modify: `src/main.ts` (pass `forge`; the off state's button)
- Modify: `src/styles.css` (`.pr__forge`, `.reviews__off`)
- Test: `src/reviews.test.ts`, `src/card.test.ts`

**Interfaces:**
- Consumes (Task 8): `ReviewState { prs, errors: { forge, message }[], off, fetchedAt }`; commands `open_review_pr` / `review_pr` take `{ forge, repo, number }`.
- Produces:
  - `export type GitProvider = "github" | "gitlab"`; `ReviewPr.forge: GitProvider`; `PullRequest.forge?: GitProvider` (missing reads as GitHub).
  - `ReviewAction` = `{ kind: "open-pr" | "review"; forge; repo; number } | { kind: "terminal"; forge; repo; number; pid } | { kind: "git-settings" }`.
  - `export const OPEN_SETTINGS_GROUP = "maya:settings-group"`, a `CustomEvent<string>` on `document` that Task 10's Settings listens for.
  - Exported helper `prRef(pr: { forge?: GitProvider; number: number }): string` → `#451` / `!12`.

- [ ] **Step 1: Write the failing tests**

In `src/reviews.test.ts`, add `forge: "github"` to the `pr()` helper and add:

```ts
describe("forges", () => {
  const mr = (over: Partial<ReviewPr> = {}) => pr({ forge: "gitlab", repo: "grp/sub/bedrock", number: 12, url: "https://gitlab.acme.io/grp/sub/bedrock/-/merge_requests/12", ...over });

  it("marks each card with its forge and names a merge request with a bang", () => {
    const root = renderReviews({ prs: [mr(), pr()], errors: [], off: false, fetchedAt: NOW }, NOW);
    const cards = [...root.querySelectorAll<HTMLElement>(".pr")];
    expect(cards.map((c) => c.querySelector(".pr__repo")?.textContent)).toEqual(["bedrock !12", "bedrock #451"]);
    expect(cards.map((c) => c.dataset.forge)).toEqual(["gitlab", "github"]);
    expect(cards[0].querySelector(".pr__forge")?.getAttribute("aria-label")).toBe("GitLab");
  });

  it("says each forge's error on its own line and still lists the PRs", () => {
    const root = renderReviews({ prs: [pr()], errors: [{ forge: "gitlab", message: "GitLab check failed: the token was refused (401)." }, { forge: "github", message: "No GitHub token: paste one in Settings → Git." }], off: false, fetchedAt: NOW }, NOW);
    expect([...root.querySelectorAll(".reviews__error")].map((e) => e.textContent)).toEqual(["GitLab check failed: the token was refused (401).", "No GitHub token: paste one in Settings → Git."]);
    expect(root.querySelectorAll(".pr")).toHaveLength(1);
  });

  it("says the integration is off, with a way to Settings → Git", () => {
    const root = renderReviews({ prs: [], errors: [], off: true, fetchedAt: NOW }, NOW);
    expect(root.querySelector(".reviews__off")?.textContent).toContain("Git integration is off. Turn on GitHub or GitLab in Settings → Git.");
    expect(reviewActionFor(root.querySelector("button[data-action=git-settings]")!)).toEqual({ kind: "git-settings" });
  });

  it("keeps the same path on both forges as two repository options", () => {
    const opts = repoOptions([pr({ repo: "acme/api" }), mr({ repo: "acme/api" })]);
    expect(opts.map((o) => o.value)).toEqual(["github:acme/api", "gitlab:acme/api"]);
    expect(applyFilters([pr({ repo: "acme/api" }), mr({ repo: "acme/api" })], { state: "all", repo: "gitlab:acme/api" }).map((p) => p.forge)).toEqual(["gitlab"]);
  });

  it("a click carries the forge", () => {
    const root = renderReviews({ prs: [mr()], errors: [], off: false, fetchedAt: NOW }, NOW);
    expect(reviewActionFor(root.querySelector("button[data-action=review]")!)).toEqual({ kind: "review", forge: "gitlab", repo: "grp/sub/bedrock", number: 12 });
  });
});
```

Update every existing `renderReviews({ prs, error, fetchedAt })` in the file to `{ prs, errors: [], off: false, fetchedAt }`; the existing error test becomes `errors: [{ forge: "github", message: "GitHub check failed: no token" }]` and expects that text. Expectations of `repoOptions(…).value` and `filters.repo` gain the `github:` prefix; `reviewActionFor` expectations gain `forge: "github"`. "Checking GitHub…" expectations become "Checking…".

In `src/card.test.ts`:

```ts
it("names a GitLab merge request MR !n and a GitHub one PR #n", () => {
  const gl = renderCard(card({ pr: { number: 12, url: "https://gitlab.com/g/r/-/merge_requests/12", state: "open", forge: "gitlab" } }));
  expect(gl.querySelector("button[data-action=pr]")?.textContent).toBe("MR !12");
  const gh = renderCard(card({ pr: { number: 7, url: "https://github.com/o/r/pull/7", state: "open" } }));
  expect(gh.querySelector("button[data-action=pr]")?.textContent).toBe("PR #7");
});
```

(Use the file's existing card builder and render function names; check its imports.)

- [ ] **Step 2: Run the tests to see them fail**

Run: `pnpm vitest run src/reviews.test.ts src/card.test.ts`
Expected: FAIL (type errors / wrong text).

- [ ] **Step 3: Implement**

`src/assets/icons/github.svg` (Octicons `mark-github`, MIT):

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path fill="currentColor" d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"/></svg>
```

`src/assets/icons/gitlab.svg` (a plain tanuki outline in `currentColor`):

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path fill="currentColor" d="M8 15 1.1 9.9a.6.6 0 0 1-.2-.7L2.6 2.3a.3.3 0 0 1 .6 0L4.7 7h6.6l1.5-4.7a.3.3 0 0 1 .6 0l1.7 6.9a.6.6 0 0 1-.2.7z"/></svg>
```

`src/icons.ts`: import both (`import github from "./assets/icons/github.svg?raw";`, same for gitlab) and add them to `FILES`.

`src/types.ts`:

```ts
export type GitProvider = "github" | "gitlab";

export interface PullRequest {
  number: number;
  url: string;
  /** "open", "draft", "merged" or "closed". */
  state: string;
  /** Absent from an older Maya, which only knew GitHub. */
  forge?: GitProvider;
}

/** `#451` on GitHub, `!12` on GitLab. */
export function prRef(pr: { forge?: GitProvider; number: number }): string {
  return `${pr.forge === "gitlab" ? "!" : "#"}${pr.number}`;
}
```

`ReviewPr` gains `forge: GitProvider;` after `number`, and its `repo` comment becomes "the repo's full path (owner/name, or group/sub/name)". Replace `ReviewState`:

```ts
export interface ForgeError {
  forge: GitProvider;
  /** The whole line, e.g. "GitLab check failed: …". */
  message: string;
}

export interface ReviewState {
  prs: ReviewPr[];
  errors: ForgeError[];
  /** Neither GitHub nor GitLab is switched on. */
  off: boolean;
  fetchedAt: number | null;
}
```

`prButton`: `b.textContent = \`${pr.forge === "gitlab" ? "MR" : "PR"} ${prRef(pr)}\`;`.

`src/reviews.ts`:

```ts
export const OPEN_SETTINGS_GROUP = "maya:settings-group";

export type ReviewAction =
  | { kind: "open-pr" | "review"; forge: GitProvider; repo: string; number: number }
  | { kind: "terminal"; forge: GitProvider; repo: string; number: number; pid: number }
  | { kind: "git-settings" };

const repoKey = (p: ReviewPr) => `${p.forge}:${p.repo}`;
```

- `ReviewFilters.repo`'s comment: "`<forge>:<repo>`, or null for every repository".
- `applyFilters`: compare `repoKey(p) === f.repo`.
- `repoOptions`: build from `repoKey`, label by the short name unless two share it (then the full path, plus " (GitLab)" for a GitLab one when a GitHub repo has the same full path):

```ts
export function repoOptions(prs: ReviewPr[]): { value: string; label: string }[] {
  const keys = [...new Map(prs.map((p) => [repoKey(p), p])).values()];
  const short = (p: ReviewPr) => p.repo.split("/").pop() ?? p.repo;
  const label = (p: ReviewPr) => {
    if (keys.filter((o) => short(o) === short(p)).length === 1) return short(p);
    const clash = keys.some((o) => o.repo === p.repo && o.forge !== p.forge);
    return clash && p.forge === "gitlab" ? `${p.repo} (GitLab)` : p.repo;
  };
  return keys.map((p) => ({ value: repoKey(p), label: label(p) })).sort((a, b) => a.label.localeCompare(b.label) || a.value.localeCompare(b.value));
}
```

- `renderReviewCard`: `root.dataset.forge = pr.forge;`; before the repo span:

```ts
  const forgeMark = iconElement(pr.forge, 12);
  forgeMark.classList.add("pr__forge");
  forgeMark.setAttribute("role", "img");
  forgeMark.setAttribute("aria-label", pr.forge === "gitlab" ? "GitLab" : "GitHub");
  name.append(forgeMark);
  name.append(el("span", "pr__repo", `${pr.repo.split("/").pop() ?? pr.repo} ${prRef(pr)}`));
```

(import `iconElement` from `./icons` and `prRef`, `GitProvider` from `./types`).

- `renderReviews`:

```ts
  const root = el("section", "reviews");
  if (state.off) {
    const off = el("div", "reviews__off", "Git integration is off. Turn on GitHub or GitLab in Settings → Git. ");
    const go = el("button", "card__btn", "Open Settings");
    go.type = "button";
    go.dataset.action = "git-settings";
    off.append(go);
    root.append(off);
    return root;
  }
  for (const e of state.errors) root.append(el("div", "reviews__error", e.message));
  if (state.prs.length === 0) {
    root.append(el("div", "reviews__empty", state.fetchedAt === null ? "Checking…" : "Nothing waiting on you."));
    return root;
  }
```

- `makeReviewsView`'s initial state: `{ prs: [], errors: [], off: false, fetchedAt: null }`.
- `reviewActionFor`:

```ts
export function reviewActionFor(target: Element): ReviewAction | null {
  const btn = target.closest<HTMLElement>("button[data-action]");
  if (btn?.dataset.action === "git-settings") return { kind: "git-settings" };
  const card = target.closest<HTMLElement>(".pr");
  if (!btn || !card?.dataset.repo || !card.dataset.number) return null;
  const kind = btn.dataset.action;
  const forge: GitProvider = card.dataset.forge === "gitlab" ? "gitlab" : "github";
  const repo = card.dataset.repo;
  const number = Number(card.dataset.number);
  if (kind === "terminal") return { kind, forge, repo, number, pid: Number(btn.dataset.pid) };
  if (kind !== "open-pr" && kind !== "review") return null;
  return { kind, forge, repo, number };
}
```

`src/main.ts`, in `reviewAction`:

```ts
    if (action.kind === "git-settings") {
      document.dispatchEvent(new CustomEvent(OPEN_SETTINGS_GROUP, { detail: "git" }));
      tabs?.show("settings");
    } else if (action.kind === "terminal") {
      await focus(action.pid);
    } else if (action.kind === "open-pr") {
      await invoke("open_review_pr", { forge: action.forge, repo: action.repo, number: action.number });
    } else {
      const dir = await invoke<string>("review_pr", { forge: action.forge, repo: action.repo, number: action.number });
      showToast(`Reviewing ${prRef(action)} in ${dir.split("/").filter(Boolean).pop() ?? dir}`);
    }
```

(Check `ReviewState`'s initial value in `main.ts` too: it becomes `{ prs: [], errors: [], off: false, fetchedAt: null }`.)

`src/styles.css`, beside `.pr__repo`:

```css
.pr__forge { color: var(--muted); margin-right: 4px; vertical-align: -1px; }
.reviews__off { color: var(--muted); padding: 8px 0 12px; font-size: 13px; display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `pnpm test && pnpm exec tsc --noEmit`
Expected: PASS, no type errors (the phone's `src/mobile` may build cards with `pr`: `forge` is optional there).

- [ ] **Step 5: Commit**

```bash
git add src
git commit -m "feat: show GitLab merge requests beside GitHub PRs, with an error line per forge

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 10: Settings → Git

**Files:**
- Modify: `src/settings.ts` (group, model, handlers, render, controller)
- Test: `src/settings.test.ts`, `src/settings-flow.test.ts`

**Interfaces:**
- Consumes (Task 8): commands `get_git_status`, `store_git_token({ provider, token })`, `clear_git_token({ provider })`, `test_git_connection({ provider })`; `get_config`/`set_config` carrying `git`. (Task 9): `OPEN_SETTINGS_GROUP`.
- Produces:
  - `SETTINGS_GROUPS` gains `["git", "Git"]` before `["reviews", "Pull requests"]`.
  - `export interface GitConnectionJson { provider: GitProvider; enabled: boolean; baseUrl?: string | null; token: { kind: "stored" } | { kind: "env"; name: string } }`.
  - `export function gitConnections(raw: GitConnectionJson[] | null | undefined): GitConnectionJson[]` (always `[github, gitlab]`, defaults as in Rust).
  - Model: `git?: GitConnectionJson[]`, `gitStatus?: Record<GitProvider, { hasToken: boolean; test?: string | null }>`.
  - Handlers (optional, so older tests' handler objects still type-check): `onGitEnabled?(p, on)`, `onGitTokenKind?(p, kind: "stored" | "env")`, `onGitEnvName?(p, name)`, `onGitBaseUrl?(url)`, `onGitToken?(p, token)`, `onGitClearToken?(p)`, `onGitTest?(p)`.

- [ ] **Step 1: Write the failing tests**

In `src/settings.test.ts`:

```ts
import { gitConnections } from "./settings";

describe("Settings → Git", () => {
  const gitModel = (over: Partial<SettingsModel> = {}): SettingsModel => ({
    ...voiceBase,
    settingsGroup: "git",
    git: gitConnections(null),
    gitStatus: { github: { hasToken: false }, gitlab: { hasToken: false } },
    ...over,
  });
  const block = (root: HTMLElement, p: string) => root.querySelector<HTMLElement>(`.settings__git[data-provider=${p}]`)!;

  it("defaults to GitHub on reading GH_TOKEN and GitLab off reading GITLAB_TOKEN", () => {
    expect(gitConnections(undefined)).toEqual([
      { provider: "github", enabled: true, baseUrl: null, token: { kind: "env", name: "GH_TOKEN" } },
      { provider: "gitlab", enabled: false, baseUrl: null, token: { kind: "env", name: "GITLAB_TOKEN" } },
    ]);
    expect(gitConnections([{ provider: "gitlab", enabled: true, token: { kind: "stored" } }])[1]).toEqual({ provider: "gitlab", enabled: true, baseUrl: null, token: { kind: "stored" } });
  });

  it("shows a block per provider, its fields disabled while it is off", () => {
    const root = renderSettings(gitModel(), handlers());
    expect(block(root, "github").querySelector<HTMLInputElement>("input[name=gitEnabled]")!.checked).toBe(true);
    expect(block(root, "gitlab").querySelector<HTMLInputElement>("input[name=gitEnabled]")!.checked).toBe(false);
    expect(block(root, "gitlab").querySelector<HTMLInputElement>("input[name=gitBaseUrl]")!.disabled).toBe(true);
    expect(block(root, "gitlab").querySelector<HTMLInputElement>("input[name=gitBaseUrl]")!.placeholder).toBe("https://gitlab.com");
    expect(block(root, "github").querySelector("input[name=gitBaseUrl]")).toBeNull();
  });

  it("shows the variable field for env and the token field for stored, never filled", () => {
    const stored = gitConnections([{ provider: "github", enabled: true, token: { kind: "stored" } }]);
    const root = renderSettings(gitModel({ git: stored, gitStatus: { github: { hasToken: true }, gitlab: { hasToken: false } } }), handlers());
    const gh = block(root, "github");
    const tokenField = gh.querySelector<HTMLInputElement>("input[name=gitToken]")!;
    expect(tokenField.type).toBe("password");
    expect(tokenField.value).toBe("");
    expect(gh.querySelector("input[name=gitEnvName]")).toBeNull();
    expect(gh.querySelector("[data-action=git-clear]")).not.toBeNull();
    const gl = block(root, "gitlab");
    expect(gl.querySelector<HTMLInputElement>("input[name=gitEnvName]")!.value).toBe("GITLAB_TOKEN");
    expect(gl.querySelector("input[name=gitToken]")).toBeNull();
  });

  it("calls a handler for each change", () => {
    const h = { ...handlers(), onGitEnabled: vi.fn(), onGitTokenKind: vi.fn(), onGitEnvName: vi.fn(), onGitBaseUrl: vi.fn(), onGitToken: vi.fn(), onGitClearToken: vi.fn(), onGitTest: vi.fn() };
    const stored = gitConnections([{ provider: "github", enabled: true, token: { kind: "stored" } }, { provider: "gitlab", enabled: true, token: { kind: "env", name: "GITLAB_TOKEN" } }]);
    const root = renderSettings(gitModel({ git: stored, gitStatus: { github: { hasToken: true }, gitlab: { hasToken: false } } }), h);
    const gl = block(root, "gitlab");
    const on = gl.querySelector<HTMLInputElement>("input[name=gitEnabled]")!;
    on.checked = false;
    on.dispatchEvent(new Event("change"));
    expect(h.onGitEnabled).toHaveBeenCalledWith("gitlab", false);
    const url = gl.querySelector<HTMLInputElement>("input[name=gitBaseUrl]")!;
    url.value = "  https://gitlab.acme.io/ ";
    url.dispatchEvent(new Event("change"));
    expect(h.onGitBaseUrl).toHaveBeenCalledWith("https://gitlab.acme.io/");
    const env = gl.querySelector<HTMLInputElement>("input[name=gitEnvName]")!;
    env.value = " MY_TOKEN ";
    env.dispatchEvent(new Event("change"));
    expect(h.onGitEnvName).toHaveBeenCalledWith("gitlab", "MY_TOKEN");
    gl.querySelector<HTMLInputElement>("input[name=gitTokenKind-gitlab][value=stored]")!.dispatchEvent(new Event("change"));
    expect(h.onGitTokenKind).toHaveBeenCalledWith("gitlab", "stored");
    const gh = block(root, "github");
    const tok = gh.querySelector<HTMLInputElement>("input[name=gitToken]")!;
    tok.value = " ghp_new ";
    tok.dispatchEvent(new Event("change"));
    expect(h.onGitToken).toHaveBeenCalledWith("github", "ghp_new");
    expect(tok.value).toBe(""); // emptied at once
    gh.querySelector<HTMLButtonElement>("[data-action=git-clear]")!.click();
    expect(h.onGitClearToken).toHaveBeenCalledWith("github");
    gh.querySelector<HTMLButtonElement>("[data-action=git-test]")!.click();
    expect(h.onGitTest).toHaveBeenCalledWith("github");
  });

  it("shows the last test's outcome", () => {
    const root = renderSettings(gitModel({ gitStatus: { github: { hasToken: false, test: "Connected as @tiago" }, gitlab: { hasToken: false } } }), handlers());
    expect(block(root, "github").querySelector(".settings__git-test")?.textContent).toBe("Connected as @tiago");
  });
});
```

In `src/settings-flow.test.ts`, every existing `invoke.mockImplementation` keeps rejecting unknown commands, so `initSettings` must tolerate `get_git_status` failing. Add:

```ts
  it("saves a GitLab URL into the git list and opens on Git when the Pull requests tab asks", async () => {
    let saved: any = null;
    const config = { completedTimeoutMinutes: 30, projectsDir: null, clonesDir: null, notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin", elevenlabsVoiceId: null, listen: false, microphone: null, agent: "claude-code", agentModel: "", reviewPrompt: "", git: null };
    invoke.mockImplementation((cmd: string, args?: any) => {
      if (cmd === "codex_hook_status") return Promise.resolve(false);
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "set_config") { saved = args.config; return Promise.resolve(args.config); }
      if (cmd === "get_git_status") return Promise.resolve({ github: { hasToken: false }, gitlab: { hasToken: false } });
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([]);
      if (cmd === "list_agents") return Promise.resolve({ agents: [], names: true });
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await initSettings();
    await flush();
    document.dispatchEvent(new CustomEvent("maya:settings-group", { detail: "git" }));
    await flush();
    const visible = [...document.querySelectorAll<HTMLElement>(".settings__section")].filter((s) => !s.hidden).map((s) => s.dataset.group);
    expect(visible).toEqual(["git"]);
    const enable = document.querySelector<HTMLInputElement>(".settings__git[data-provider=gitlab] input[name=gitEnabled]")!;
    enable.checked = true;
    enable.dispatchEvent(new Event("change"));
    await flush();
    expect(saved.git).toEqual([
      { provider: "github", enabled: true, baseUrl: null, token: { kind: "env", name: "GH_TOKEN" } },
      { provider: "gitlab", enabled: true, baseUrl: null, token: { kind: "env", name: "GITLAB_TOKEN" } },
    ]);
    localStorage.removeItem("maya.settingsGroup");
  });
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `pnpm vitest run src/settings.test.ts src/settings-flow.test.ts`
Expected: FAIL (`gitConnections` missing; no `.settings__git`).

- [ ] **Step 3: Implement** (all in `src/settings.ts`)

Types and defaults, near `ConfigJson`:

```ts
export interface GitConnectionJson {
  provider: GitProvider;
  enabled: boolean;
  /** GitLab only; null is https://gitlab.com. */
  baseUrl?: string | null;
  token: { kind: "stored" } | { kind: "env"; name: string };
}

const GIT_DEFAULT_ENV: Record<GitProvider, string> = { github: "GH_TOKEN", gitlab: "GITLAB_TOKEN" };
const GIT_LABEL: Record<GitProvider, string> = { github: "GitHub", gitlab: "GitLab" };

/** One GitHub and one GitLab connection, in that order, as the Rust side defaults them. */
export function gitConnections(raw: GitConnectionJson[] | null | undefined): GitConnectionJson[] {
  return (["github", "gitlab"] as const).map((provider) => {
    const found = raw?.find((c) => c.provider === provider);
    return found ? { ...found, baseUrl: found.baseUrl ?? null } : { provider, enabled: provider === "github", baseUrl: null, token: { kind: "env", name: GIT_DEFAULT_ENV[provider] } };
  });
}
```

`ConfigJson` gains `git?: GitConnectionJson[] | null;`. `SettingsModel` gains:

```ts
  /** GitHub then GitLab, from `gitConnections`. */
  git?: GitConnectionJson[];
  /** Whether a pasted token is stored, and the last Test's outcome. */
  gitStatus?: Record<GitProvider, { hasToken: boolean; test?: string | null }>;
```

`SettingsHandlers` gains the seven optional `onGit…` handlers listed under Interfaces. `SETTINGS_GROUPS`: insert `["git", "Git"],` before `["reviews", "Pull requests"]`.

Render, in `renderSettings` after the `reviews` controls (destructure `git` from `sections`):

```ts
  for (const conn of model.git ?? gitConnections(null)) {
    const p = conn.provider;
    const status = model.gitStatus?.[p] ?? { hasToken: false };
    const box = document.createElement("div");
    box.className = "settings__git";
    box.dataset.provider = p;

    const onLabel = document.createElement("label");
    onLabel.className = "settings__check";
    const on = document.createElement("input");
    on.type = "checkbox";
    on.name = "gitEnabled";
    on.checked = conn.enabled;
    on.addEventListener("change", () => h.onGitEnabled?.(p, on.checked));
    onLabel.append(on, document.createTextNode(` ${GIT_LABEL[p]}`));
    box.append(onLabel);

    if (p === "gitlab") {
      const urlLabel = document.createElement("label");
      urlLabel.textContent = "URL";
      const url = document.createElement("input");
      url.type = "text";
      url.name = "gitBaseUrl";
      url.placeholder = "https://gitlab.com";
      url.value = conn.baseUrl ?? "";
      url.disabled = !conn.enabled;
      url.addEventListener("change", () => h.onGitBaseUrl?.(url.value.trim()));
      urlLabel.append(url);
      box.append(urlLabel);
    }

    const kinds = document.createElement("div");
    kinds.className = "settings__git-kinds";
    for (const [kind, text] of [["stored", "Paste a token"], ["env", "From env var"]] as const) {
      const l = document.createElement("label");
      l.className = "settings__check";
      const r = document.createElement("input");
      r.type = "radio";
      r.name = `gitTokenKind-${p}`;
      r.value = kind;
      r.checked = conn.token.kind === kind;
      r.disabled = !conn.enabled;
      r.addEventListener("change", () => h.onGitTokenKind?.(p, kind));
      l.append(r, document.createTextNode(` ${text}`));
      kinds.append(l);
    }
    box.append(kinds);

    if (conn.token.kind === "stored") {
      const tokLabel = document.createElement("label");
      tokLabel.textContent = `${GIT_LABEL[p]} token`;
      const tok = document.createElement("input");
      tok.type = "password";
      tok.name = "gitToken";
      tok.autocomplete = "off";
      tok.disabled = !conn.enabled;
      tok.placeholder = status.hasToken ? `saved in ${secretStore()}; paste to replace` : "paste your token";
      tok.addEventListener("change", () => {
        const t = tok.value.trim();
        tok.value = "";
        if (t) h.onGitToken?.(p, t);
      });
      tokLabel.append(tok);
      box.append(tokLabel);
      if (status.hasToken) {
        const clear = document.createElement("button");
        clear.type = "button";
        clear.className = "settings__link";
        clear.dataset.action = "git-clear";
        clear.textContent = "Clear";
        clear.addEventListener("click", () => h.onGitClearToken?.(p));
        box.append(clear);
      }
    } else {
      const envLabel = document.createElement("label");
      envLabel.textContent = "Variable";
      const env = document.createElement("input");
      env.type = "text";
      env.name = "gitEnvName";
      env.placeholder = GIT_DEFAULT_ENV[p];
      env.value = conn.token.name;
      env.disabled = !conn.enabled;
      env.addEventListener("change", () => h.onGitEnvName?.(p, env.value.trim()));
      envLabel.append(env);
      box.append(envLabel);
    }

    const test = document.createElement("button");
    test.type = "button";
    test.className = "settings__link";
    test.dataset.action = "git-test";
    test.textContent = "Test";
    test.disabled = !conn.enabled;
    test.addEventListener("click", () => h.onGitTest?.(p));
    box.append(test);
    if (status.test) {
      const out = document.createElement("div");
      out.className = "settings__hint settings__git-test";
      out.textContent = status.test;
      box.append(out);
    }
    git.append(box);
  }
  const gitHint = document.createElement("div");
  gitHint.className = "settings__hint";
  gitHint.textContent = "Maya reads pull requests and merge requests with these tokens. GitHub needs the repo scope; GitLab needs read_api. Reviews clone with your own git credentials.";
  git.append(gitHint);
```

Controller, in `initSettings`:

```ts
  const loadGitStatus = async () => {
    try {
      const s = await invoke<Record<GitProvider, { hasToken: boolean }>>("get_git_status");
      model.gitStatus = { github: { ...model.gitStatus?.github, ...s.github }, gitlab: { ...model.gitStatus?.gitlab, ...s.gitlab } };
    } catch {
      model.gitStatus ??= { github: { hasToken: false }, gitlab: { hasToken: false } };
    }
  };

  /** Applies `change` to one connection of the freshest config and saves the list. */
  const saveGit = (p: GitProvider, change: (c: GitConnectionJson) => void) =>
    run(async () => {
      const fresh = await invoke<ConfigJson>("get_config");
      const git = gitConnections(fresh.git);
      change(git.find((c) => c.provider === p)!);
      const c = await invoke<ConfigJson>("set_config", { config: { ...fresh, git } });
      applyConfig(c);
    });

  const setTest = (p: GitProvider, text: string) => {
    model.gitStatus = { github: { hasToken: false }, gitlab: { hasToken: false }, ...model.gitStatus, [p]: { ...model.gitStatus?.[p], hasToken: model.gitStatus?.[p]?.hasToken ?? false, test: text } };
  };
```

`applyConfig` gains `model.git = gitConnections(c.git);`. `handlers` gains:

```ts
    onGitEnabled: (p, on) => void saveGit(p, (c) => { c.enabled = on; }),
    onGitBaseUrl: (url) => void saveGit("gitlab", (c) => { c.baseUrl = url || null; }),
    onGitTokenKind: (p, kind) => void saveGit(p, (c) => { c.token = kind === "stored" ? { kind: "stored" } : { kind: "env", name: GIT_DEFAULT_ENV[p] }; }),
    onGitEnvName: (p, name) => void saveGit(p, (c) => { c.token = { kind: "env", name }; }),
    onGitToken: (p, token) => void run(async () => { await invoke("store_git_token", { provider: p, token }); await loadGitStatus(); }),
    onGitClearToken: (p) => void run(async () => { await invoke("clear_git_token", { provider: p }); await loadGitStatus(); }),
    onGitTest: (p) => {
      setTest(p, "Testing…");
      paint();
      invoke<string>("test_git_connection", { provider: p })
        .then((login) => setTest(p, `Connected as @${login}`))
        .catch((e) => setTest(p, String(e)))
        .finally(() => { if (!panel.hidden) paint(); });
    },
```

In the initial load (where `loadVoices()` runs), add `await loadGitStatus();`. Listen for the Pull requests tab's request:

```ts
  document.addEventListener(OPEN_SETTINGS_GROUP, (e) => {
    const group = (e as CustomEvent<string>).detail;
    if (!SETTINGS_GROUPS.some(([key]) => key === group)) return;
    model.settingsGroup = group as SettingsGroup;
    try {
      localStorage.setItem(SETTINGS_GROUP_KEY, group);
    } catch {
      /* private mode: nothing to remember */
    }
    paint();
  });
```

(import `OPEN_SETTINGS_GROUP` from `./reviews` and `GitProvider` from `./types`). Add to `src/styles.css`:

```css
.settings__git { display: flex; flex-direction: column; gap: 6px; padding: 8px 0 12px; border-bottom: 1px solid var(--border); }
.settings__git-kinds { display: flex; gap: 12px; flex-wrap: wrap; }
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `pnpm test && pnpm exec tsc --noEmit`
Expected: PASS.

- [ ] **Step 5: Run the app and look at it**

Run `pnpm tauri dev` (see the `maya-run-and-verify` memory: vite plus the debug binary; screenshot, then click). Check in Settings → Git:
- both blocks render, GitLab's fields are greyed while it is off;
- switching GitLab on, typing `https://gitlab.com/api/v4/` and leaving the field saves and redraws as `https://gitlab.com`;
- a variable name `A B` shows "The GitLab token variable must be a name like GITLAB_TOKEN." at the top;
- Test with no token says "No GitHub token: …".

Then on the Pull requests tab: with both off, the off line and its button, which opens Settings → Git. Report what was seen; don't claim a check that could not be made.

- [ ] **Step 6: Commit**

```bash
git add src
git commit -m "feat: add Settings → Git with GitHub and GitLab tokens and a Test button

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 11: Docs and release

**Files:**
- Modify: `docs/DEVELOPING.md` (requirements line; a "Git integration" note)
- Modify: `README.md` (only where it says the Pull requests tab needs `gh`; check with `grep -n "gh" README.md`)
- Modify: `docs/superpowers/specs/2026-10-18-git-integration-design.md` (`Status: implemented`)
- Version files via `scripts/set-version.sh`

- [ ] **Step 1: Update the docs**

`docs/DEVELOPING.md`, line 12: `` - `jq` on PATH for the Claude Code hook `` (drop `` `gh` for the Pull Requests tab ``). After the "Build and run" section add:

```markdown
## Git integration

The Pull requests tab and each card's PR badge talk to GitHub's and
GitLab's APIs directly (`core/src/forge/`); `gh` is not needed. Settings
→ Git switches each on or off and says where its token comes from: a
pasted token, kept in the OS key store (`maya-github`, `maya-gitlab`), or
a variable read from the login shell (`forge::env`). GitLab's URL defaults
to `https://gitlab.com`. The first start after 0.24 imports `gh auth
token` once, when `gh` is logged in. Review clones run `git clone` with
the user's own git credentials.
```

In `README.md`, replace any line that tells users to install or log in to `gh` for pull requests with: "Pull requests: turn on GitHub or GitLab in Settings → Git and give it a token (or the name of a variable that holds one)."

- [ ] **Step 2: Full check**

Run: `cargo test --workspace && pnpm test && pnpm exec tsc --noEmit && grep -rn 'crate::gh()\|gh repo clone\|gh pr view' core src-tauri cli src`
Expected: tests PASS; the grep prints nothing.

- [ ] **Step 3: Commit the docs**

```bash
git add docs README.md
git commit -m "docs: describe the Git integration and drop gh from the requirements

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 4: Bump the version**

```bash
git fetch origin && git show origin/main:package.json | grep '"version"'
```

Expected: `"version": "0.23.0"`. If `main` has moved, bump the minor from its version instead (0.23.x → 0.24.0; 0.24.y → 0.25.0).

```bash
sh scripts/set-version.sh 0.24.0
git diff --stat
```

Expected: `package.json`, `src-tauri/tauri.conf.json`, `mobile/tauri.conf.json`, `Cargo.toml` and `Cargo.lock` changed. If `Cargo.lock` did not, run `cargo update --workspace --offline` and check again.

```bash
git add package.json src-tauri/tauri.conf.json mobile/tauri.conf.json Cargo.toml Cargo.lock
git commit -m "chore(release): 0.24.0

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 5: Record what is still owed**

Add a memory `maya-git-integration-hand-check.md` (and its line in `MEMORY.md`): feat/git-integration (0.24.0) owes a real github.com token, a gitlab.com token, a self-hosted GitLab if one is available, and env var mode, on the Mac, laptop and Windows; the first-start `gh` import on a machine with `gh` logged in.

Do not push. Ask the user before opening the pull request.
