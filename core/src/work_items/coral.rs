//! Coral Boards over its REST API: the address and token, and the calls
//! every Coral request goes through. The token never appears in an error.

use crate::config::{CoralItems, TokenSource};
use crate::forge::{scrub, Http, Reply};
use serde_json::Value;

/// A Coral address with its token. No `Debug`: it would show the token.
#[derive(Clone)]
pub struct Coral {
    /// Without a trailing `/`.
    pub base: String,
    pub token: String,
}

/// The Coral connection with its token: the stored one, or the variable's.
pub fn resolve(c: &CoralItems, stored: impl Fn() -> Option<String>, env: impl Fn(&str) -> Option<String>) -> Result<Coral, String> {
    let token = match &c.token {
        TokenSource::Stored => stored(),
        TokenSource::Env { .. } => env(&c.env_name().unwrap_or_default()),
    };
    match token.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) {
        Some(token) => Ok(Coral { base: c.base_url.trim().trim_end_matches('/').to_string(), token }),
        None => Err(match &c.token {
            TokenSource::Stored => "No Coral token: paste one in Settings → Work Items.".to_string(),
            TokenSource::Env { .. } => format!("No Coral token: set {} or paste a token in Settings → Work Items.", c.env_name().unwrap_or_default()),
        }),
    }
}

impl Coral {
    /// `{base}/api/v1{path}`.
    pub fn api(&self, path: &str) -> String {
        format!("{}/api/v1{path}", self.base)
    }

    /// The item's page.
    pub fn web(&self, code: &str) -> String {
        format!("{}/items/{code}", self.base)
    }

    fn host(&self) -> &str {
        let rest = self.base.split_once("://").map_or(self.base.as_str(), |(_, r)| r);
        rest.split('/').next().unwrap_or("")
    }
}

/// The body of a 2xx reply, else why not.
fn check(reply: Reply, token: &str) -> Result<String, String> {
    let Reply { status, body, .. } = reply;
    match status {
        200..=299 => Ok(body),
        401 | 403 => Err("Coral refused the token: paste a new one in Settings → Work Items.".into()),
        _ => {
            let body = scrub(&body, token);
            let v: Value = serde_json::from_str(&body).unwrap_or_default();
            let message = v["message"].as_str().or_else(|| v["error"].as_str()).map(str::to_string).unwrap_or_else(|| body.chars().take(200).collect());
            Err(format!("{status}: {message}"))
        }
    }
}

fn answer(c: &Coral, r: Result<Reply, String>) -> Result<String, String> {
    let out = match r {
        Ok(reply) => check(reply, &c.token),
        Err(e) => Err(format!("could not reach {}: {e}", c.host())),
    };
    out.map_err(|e| scrub(&e, &c.token))
}

/// A GET's JSON reply.
pub(crate) fn get_json(http: &dyn Http, c: &Coral, url: &str) -> Result<Value, String> {
    let body = answer(c, http.get(url, &c.token))?;
    serde_json::from_str(&body).map_err(|e| scrub(&format!("Coral's reply was not JSON: {e}"), &c.token))
}

/// A PATCH's or PUT's (or POST's) JSON body sent; the reply is not read.
#[allow(dead_code)] // Start (a later task) calls it
pub(crate) fn send_json(http: &dyn Http, c: &Coral, method: &str, url: &str, body: &Value) -> Result<(), String> {
    let body = body.to_string();
    let r = if method == "POST" { http.post_json(url, &c.token, &body) } else { http.send_json(method, url, &c.token, &body) };
    answer(c, r).map(|_| ())
}

/// Who the token belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoralMe {
    pub id: String,
    pub name: String,
    pub email: String,
}

/// The user the token belongs to.
pub fn me(http: &dyn Http, c: &Coral) -> Result<CoralMe, String> {
    let v = get_json(http, c, &c.api("/users/me"))?;
    let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
    match v["id"].as_str() {
        Some(id) => Ok(CoralMe { id: id.to_string(), name: s("name"), email: s("email") }),
        None => Err("Coral did not say who the token belongs to.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::fake::FakeHttp;

    fn coral() -> Coral {
        Coral { base: "https://coral.test".into(), token: "cbp_secret".into() }
    }

    #[test]
    fn base_is_trimmed() {
        let c = CoralItems { base_url: "https://coral.test/ ".into(), ..Default::default() };
        let r = resolve(&c, || Some("cbp_x".into()), |_| None).unwrap();
        assert_eq!(r.api("/users/me"), "https://coral.test/api/v1/users/me");
        assert_eq!(r.web("AB-1"), "https://coral.test/items/AB-1");
    }

    #[test]
    fn resolve_trims_and_refuses_blank() {
        let c = CoralItems::default();
        assert_eq!(resolve(&c, || Some(" cbp_x\n".into()), |_| None).unwrap().token, "cbp_x");
        assert_eq!(resolve(&c, || None, |_| None).err().unwrap(), "No Coral token: paste one in Settings → Work Items.");
        assert_eq!(resolve(&c, || Some("  ".into()), |_| None).err().unwrap(), "No Coral token: paste one in Settings → Work Items.");
        let e = CoralItems { token: TokenSource::Env { name: "".into() }, ..Default::default() };
        assert_eq!(resolve(&e, || None, |_| None).err().unwrap(), "No Coral token: set CORAL_TOKEN or paste a token in Settings → Work Items.");
        assert_eq!(resolve(&e, || None, |n| (n == "CORAL_TOKEN").then(|| "cbp_e\n".to_string())).unwrap().token, "cbp_e");
    }

    #[test]
    fn a_refused_token_says_so_without_the_token() {
        let http = FakeHttp::new(vec![("/users/me", 401, r#"{"error":"bad token cbp_secret"}"#)]);
        let e = get_json(&http, &coral(), "https://coral.test/api/v1/users/me").unwrap_err();
        assert_eq!(e, "Coral refused the token: paste a new one in Settings → Work Items.");
        assert!(!e.contains("cbp_"));
    }

    #[test]
    fn other_failures_are_scrubbed_and_shaped() {
        let http = FakeHttp::new(vec![("/missing", 404, r#"{"message":"no item cbp_secret"}"#), ("/plain", 500, "oops cbp_secret")]);
        let e = get_json(&http, &coral(), "https://coral.test/missing").unwrap_err();
        assert_eq!(e, "404: no item ***");
        assert_eq!(get_json(&http, &coral(), "https://coral.test/plain").unwrap_err(), "500: oops ***");
        let e = get_json(&http, &coral(), "https://coral.test/elsewhere").unwrap_err();
        assert_eq!(e, "could not reach coral.test: no route");
        assert!(send_json(&http, &coral(), "PATCH", "https://coral.test/missing", &serde_json::json!({})).unwrap_err().starts_with("404"));
    }

    #[test]
    fn me_reads_id_name_and_email() {
        let http = FakeHttp::new(vec![("/api/v1/users/me", 200, r#"{"id":"u-1","name":"Maria Silva","email":"maria@example.test"}"#)]);
        assert_eq!(me(&http, &coral()).unwrap(), CoralMe { id: "u-1".into(), name: "Maria Silva".into(), email: "maria@example.test".into() });
        let seen = http.seen.lock().unwrap();
        assert_eq!(seen[0].1, "cbp_secret");
    }
}
