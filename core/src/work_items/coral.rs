//! Coral Boards over its REST API: the address and token, and the calls
//! every Coral request goes through. The token never appears in an error.

use super::WorkItem;
use crate::config::{Assignee, CoralItems, ItemRule, TokenSource};
use crate::forge::{answer_with, encode, scrub, status_message, Http, Reply};
use serde_json::Value;
use std::collections::HashSet;

/// The `source` of a Coral item.
pub const SOURCE: &str = "coral";
/// The most items one listing returns, across rules and pages.
pub const MAX_ITEMS: usize = 500;
/// The most pages of tags or users read.
const MAX_PAGES: usize = 50;
/// The most redirects one GET follows.
const MAX_HOPS: usize = 3;

/// A Coral address with its token. No `Debug`: it would show the token.
#[derive(Clone)]
pub struct Coral {
    /// `scheme://host[:port]` only.
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
        Some(token) => Ok(Coral { base: origin(&c.base_url), token }),
        None => Err(match &c.token {
            TokenSource::Stored => "No Coral token: paste one in Settings → Work Items.".to_string(),
            TokenSource::Env { .. } => format!("No Coral token: set {} or paste a token in Settings → Work Items.", c.env_name().unwrap_or_default()),
        }),
    }
}

/// The address's `scheme://host[:port]`: any path, query or slash is dropped.
pub fn origin(url: &str) -> String {
    let url = url.trim();
    match url.split_once("://") {
        Some((scheme, rest)) => format!("{scheme}://{}", rest.split(['/', '?', '#']).next().unwrap_or("")),
        None => url.trim_end_matches('/').to_string(),
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
    match reply.status {
        200..=299 => Ok(reply.body),
        401 | 403 => Err("Coral refused the token: paste a new one in Settings → Work Items.".into()),
        status => Err(status_message(status, &reply.body, token)),
    }
}

fn answer(c: &Coral, r: Result<Reply, String>) -> Result<String, String> {
    answer_with(c.host(), &c.token, r, |reply| check(reply, &c.token))
}

/// Where a redirect from `Location` leads, when it stays on Coral's own
/// address: an absolute path, or this origin spelled out. Nothing else
/// (another host, another scheme, a relative or scheme-less address) is
/// followed, so the token goes nowhere else.
fn same_origin_target(c: &Coral, location: &str) -> Option<String> {
    if location.starts_with('/') && !location.starts_with("//") {
        return Some(format!("{}{location}", c.base));
    }
    let rest = location.strip_prefix(c.base.as_str())?;
    (rest.is_empty() || rest.starts_with(['/', '?', '#'])).then(|| location.to_string())
}

/// A GET's JSON reply. A 301, 302, 307 or 308 to Coral's own address is
/// followed by hand (a few hops at most): the token never follows a
/// redirect on its own, and never to another address.
pub(crate) fn get_json(http: &dyn Http, c: &Coral, url: &str) -> Result<Value, String> {
    let mut url = url.to_string();
    for _ in 0..=MAX_HOPS {
        let r = http.get(&url, &c.token);
        if let Ok(Reply { status: 301 | 302 | 307 | 308, location: Some(location), .. }) = &r {
            url = same_origin_target(c, location).ok_or("Coral sent the request to another address, so Maya stopped.")?;
            continue;
        }
        let body = answer(c, r)?;
        return serde_json::from_str(&body).map_err(|e| scrub(&format!("Coral's reply was not JSON: {e}"), &c.token));
    }
    Err("Coral sent the request around in circles.".into())
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

fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_string()
}

/// An item of a Coral reply; None without an id, a code or a title.
pub fn parse_item(v: &Value, c: &Coral) -> Option<WorkItem> {
    let id = v["id"].as_str()?;
    let code = v["displayCode"].as_str()?;
    let labels = v["tags"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| {
                    let name = t["name"].as_str()?;
                    Some(match t["scope"].as_str().filter(|s| !s.is_empty()) {
                        Some(scope) => format!("{scope}: {name}"),
                        None => name.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let assignees = v["assignments"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|x| x["role"].as_str() == Some("assignee"))
                .filter_map(|x| {
                    let name = x["user"]["name"].as_str().filter(|n| !n.is_empty()).or_else(|| x["user"]["email"].as_str().filter(|e| !e.is_empty()))?;
                    Some(name.to_string())
                })
                .collect()
        })
        .unwrap_or_default();
    Some(WorkItem {
        key: format!("{SOURCE}:{id}"),
        source: SOURCE.into(),
        id: code.to_string(),
        number: None,
        title: v["title"].as_str()?.to_string(),
        url: c.web(code),
        description: text(v, "description"),
        status: v["status"]["name"].as_str().map(str::to_string),
        labels,
        assignees,
        author: text(&v["createdByUser"], "name"),
        updated_at: text(v, "updatedAt"),
        repo: None,
        tracker: true,
    })
}

/// Every `data` row of a paginated list (`?page=`), up to `MAX_PAGES` pages.
fn all_pages(http: &dyn Http, c: &Coral, path: &str) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    for page in 1..=MAX_PAGES {
        let v = get_json(http, c, &c.api(&format!("{path}?pageSize=100&page={page}")))?;
        let rows = v["data"].as_array().cloned().unwrap_or_default();
        let n = rows.len();
        out.extend(rows);
        let done = match v["total"].as_u64() {
            Some(total) => out.len() as u64 >= total,
            None => n < 100,
        };
        if n == 0 || done {
            break;
        }
    }
    Ok(out)
}

struct Tag {
    id: String,
    scope: String,
    name: String,
}

/// Why a rule gave nothing: a line to show and carry on, or a failure.
enum Skip {
    Warn(String),
    Fail(String),
}

impl From<String> for Skip {
    fn from(e: String) -> Self {
        Skip::Fail(e)
    }
}

/// The tags and users of Coral, each asked for the first time it is needed.
struct Lookups<'a> {
    http: &'a dyn Http,
    c: &'a Coral,
    tags: Option<Result<Vec<Tag>, String>>,
    users: Option<Result<Vec<(String, String)>, String>>,
}

impl Lookups<'_> {
    fn tags(&mut self) -> Result<&[Tag], String> {
        let (http, c) = (self.http, self.c);
        let tags = self.tags.get_or_insert_with(|| {
            all_pages(http, c, "/tags").map(|rows| rows.iter().filter_map(|t| Some(Tag { id: t["id"].as_str()?.to_string(), scope: text(t, "scope"), name: t["name"].as_str()?.to_string() })).collect())
        });
        tags.as_ref().map(Vec::as_slice).map_err(Clone::clone)
    }

    /// The id of the user with this email.
    fn user(&mut self, email: &str) -> Result<Option<String>, String> {
        let (http, c) = (self.http, self.c);
        let users = self.users.get_or_insert_with(|| all_pages(http, c, "/users").map(|rows| rows.iter().filter_map(|u| Some((u["id"].as_str()?.to_string(), text(u, "email")))).collect()));
        match users {
            Ok(users) => Ok(users.iter().find(|(_, e)| e.eq_ignore_ascii_case(email.trim())).map(|(id, _)| id.clone())),
            Err(e) => Err(e.clone()),
        }
    }

    /// For each label, the ids of the tags it names: "scope: name" matches
    /// that scope and name, a bare name every tag so named.
    fn tag_choices(&mut self, labels: &[String]) -> Result<Vec<Vec<String>>, Skip> {
        if labels.is_empty() {
            return Ok(vec![]);
        }
        let tags = self.tags()?;
        labels
            .iter()
            .map(|label| {
                let (scope, name) = match label.split_once(": ") {
                    Some((s, n)) => (Some(s.trim()), n.trim()),
                    None => (None, label.trim()),
                };
                let ids: Vec<String> = tags.iter().filter(|t| t.name.eq_ignore_ascii_case(name) && scope.is_none_or(|s| t.scope.eq_ignore_ascii_case(s))).map(|t| t.id.clone()).collect();
                if ids.is_empty() {
                    Err(Skip::Warn(format!("Coral has no tag {label}")))
                } else {
                    Ok(ids)
                }
            })
            .collect()
    }
}

/// Every combination of one choice per label.
fn combinations(choices: &[Vec<String>]) -> Vec<Vec<String>> {
    choices.iter().fold(vec![vec![]], |acc, ids| acc.iter().flat_map(|done| ids.iter().map(move |id| done.iter().cloned().chain([id.clone()]).collect())).collect())
}

/// The open items of one type for one rule, page after page, into `out`.
#[allow(clippy::too_many_arguments)]
fn list_type(http: &dyn Http, c: &Coral, item_type: &str, assignee: Option<&str>, codes: &[String], tag_ids: &[String], out: &mut Vec<WorkItem>, seen: &mut HashSet<String>) -> Result<(), Skip> {
    let mut filters = format!("itemType={item_type}&pageSize=100&statusCategory=todo,active");
    if let Some(id) = assignee {
        filters.push_str(&format!("&assignees={}", encode(id)));
    }
    if !codes.is_empty() {
        filters.push_str(&format!("&productCode={}", codes.iter().map(|c| encode(c)).collect::<Vec<_>>().join(",")));
    }
    for id in tag_ids {
        filters.push_str(&format!("&tagId={}", encode(id)));
    }
    if !tag_ids.is_empty() {
        filters.push_str("&tagCombinator=and");
    }
    let mut cursor = String::new();
    let mut cursors = HashSet::new();
    while out.len() < MAX_ITEMS {
        let url = c.api(&format!("/work-items?{filters}&cursor={}", encode(&cursor)));
        let v = match get_json(http, c, &url) {
            Ok(v) => v,
            // Coral answers 404 when none of the product codes exists.
            Err(e) if e.starts_with("404") && !codes.is_empty() => return Err(Skip::Warn(format!("Coral has no product {}", codes.join(", ")))),
            Err(e) => return Err(Skip::Fail(e)),
        };
        for item in v["data"].as_array().into_iter().flatten().filter_map(|i| parse_item(i, c)) {
            if out.len() < MAX_ITEMS && seen.insert(item.key.clone()) {
                out.push(item);
            }
        }
        match v["nextCursor"].as_str().filter(|n| !n.is_empty()) {
            Some(next) if cursors.insert(next.to_string()) && next != cursor => cursor = next.to_string(),
            _ => break,
        }
    }
    Ok(())
}

/// The open work items and defects one rule picks, into `out`.
fn list_rule(l: &mut Lookups, rule: &ItemRule, me: &CoralMe, out: &mut Vec<WorkItem>, seen: &mut HashSet<String>) -> Result<(), Skip> {
    let assignee = match &rule.assignee {
        Assignee::Me => Some(me.id.clone()),
        Assignee::Anyone => None,
        Assignee::User { login } => Some(l.user(login)?.ok_or_else(|| Skip::Warn(format!("Coral has no user {login}")))?),
    };
    let codes: Vec<String> = rule.repos.iter().map(|r| r.trim().to_string()).filter(|r| !r.is_empty()).collect();
    let combos = combinations(&l.tag_choices(&rule.labels)?);
    for tag_ids in combos {
        for item_type in ["work_item", "defect"] {
            list_type(l.http, l.c, item_type, assignee.as_deref(), &codes, &tag_ids, out, seen)?;
        }
    }
    Ok(())
}

/// The open work items and defects the rules pick, and a line for each rule
/// that named a tag, product or user Coral does not have (or that failed
/// while others did). Err only when no rule could be asked: every one that
/// was tried failed, or warned and left the rest failing. The Err carries
/// the warnings too, so they still show.
pub fn list(http: &dyn Http, c: &Coral, cfg: &CoralItems, me: &CoralMe) -> Result<(Vec<WorkItem>, Vec<String>), (String, Vec<String>)> {
    let mut lookups = Lookups { http, c, tags: None, users: None };
    let (mut items, mut seen) = (Vec::new(), HashSet::new());
    let (mut warnings, mut failures) = (Vec::new(), Vec::new());
    let mut answered = false;
    for rule in &cfg.rules {
        match list_rule(&mut lookups, rule, me, &mut items, &mut seen) {
            Ok(()) => answered = true,
            Err(Skip::Warn(w)) => warnings.push(w),
            Err(Skip::Fail(e)) => failures.push(e),
        }
    }
    if !answered && !failures.is_empty() {
        return Err((failures.swap_remove(0), warnings));
    }
    failures.dedup();
    warnings.extend(failures);
    Ok((items, warnings))
}

/// One assignment of an item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assignment {
    pub user_id: String,
    pub role: String,
}

/// An item as read on its own: for Start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoralItem {
    pub item: WorkItem,
    pub uuid: String,
    pub item_type: String,
    pub assignments: Vec<Assignment>,
    pub status_id: Option<String>,
}

/// The item `code`; None when it is gone (404) or done.
pub fn get(http: &dyn Http, c: &Coral, code: &str) -> Result<Option<CoralItem>, String> {
    let v = match get_json(http, c, &c.api(&format!("/work-items/{}", encode(code)))) {
        Ok(v) => v,
        Err(e) if e.starts_with("404") => return Ok(None),
        Err(e) => return Err(e),
    };
    if v["status"]["category"].as_str() == Some("done") {
        return Ok(None);
    }
    let item = parse_item(&v, c).ok_or("Coral's reply was not a work item.")?;
    let assignments = v["assignments"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| Some(Assignment { user_id: x["userId"].as_str()?.to_string(), role: text(x, "role") })).collect())
        .unwrap_or_default();
    Ok(Some(CoralItem {
        uuid: text(&v, "id"),
        item_type: text(&v, "itemType"),
        status_id: v["statusId"].as_str().or_else(|| v["status"]["id"].as_str()).map(str::to_string),
        assignments,
        item,
    }))
}

/// The product of an item code: the part before its last `-`.
pub fn product_of(code: &str) -> &str {
    code.rsplit_once('-').map_or(code, |(product, _)| product)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Assignee, ItemRule};
    use crate::forge::fake::FakeHttp;
    use serde_json::json;

    fn coral() -> Coral {
        Coral { base: "https://coral.test".into(), token: "cbp_secret".into() }
    }

    #[test]
    fn base_is_trimmed() {
        let c = CoralItems { base_url: "https://coral.test/ ".into(), ..Default::default() };
        let r = resolve(&c, || Some("cbp_x".into()), |_| None).unwrap();
        assert_eq!(r.api("/users/me"), "https://coral.test/api/v1/users/me");
        for pasted in ["https://coral.test/items/AB-1", "https://coral.test/", "https://coral.test/a/b?x=1#y"] {
            let c = CoralItems { base_url: pasted.into(), ..Default::default() };
            assert_eq!(resolve(&c, || Some("cbp_x".into()), |_| None).unwrap().api("/users/me"), "https://coral.test/api/v1/users/me", "{pasted}");
        }
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

    fn me_user() -> CoralMe {
        CoralMe { id: "u-me".into(), name: "Maria Silva".into(), email: "maria@example.test".into() }
    }

    fn rule(assignee: Assignee, labels: &[&str], repos: &[&str]) -> ItemRule {
        ItemRule { assignee, labels: labels.iter().map(|s| s.to_string()).collect(), repos: repos.iter().map(|s| s.to_string()).collect() }
    }

    fn cfg(rules: Vec<ItemRule>) -> CoralItems {
        CoralItems { enabled: true, base_url: "https://coral.test".into(), rules, ..Default::default() }
    }

    fn full_item() -> Value {
        json!({
            "id": "6f1c0000-0000-0000-0000-000000000021",
            "displayCode": "BED-0021",
            "itemType": "work_item",
            "title": "Pay the invoice",
            "description": "Details here",
            "statusId": "st-1",
            "status": {"id": "st-1", "name": "In progress", "category": "active"},
            "updatedAt": "2026-11-10T10:00:00Z",
            "createdByUser": {"id": "u-2", "name": "Rui Costa", "email": "rui@example.test"},
            "tags": [{"id": "t-1", "scope": "team", "name": "api"}, {"id": "t-2", "scope": "", "name": "urgent"}],
            "assignments": [
                {"userId": "u-me", "role": "assignee", "user": {"id": "u-me", "name": "Maria Silva", "email": "maria@example.test"}},
                {"userId": "u-3", "role": "assignee", "user": {"id": "u-3", "name": "", "email": "ana@example.test"}},
                {"userId": "u-4", "role": "reviewer", "user": {"id": "u-4", "name": "Rev Iewer", "email": "rev@example.test"}}
            ]
        })
    }

    fn numbered(n: usize) -> Value {
        json!({"id": format!("id-{n}"), "displayCode": format!("BED-{n:04}"), "title": format!("t{n}"), "updatedAt": "2026-11-01T00:00:00Z", "tags": [], "assignments": []})
    }

    fn page(items: Vec<Value>, next: Option<&str>) -> String {
        json!({"data": items, "descendants": [], "total": 1, "nextCursor": next}).to_string()
    }

    fn tags_page() -> String {
        json!({"data": [
            {"id": "t-1", "scope": "team", "name": "api"},
            {"id": "t-2", "scope": "", "name": "urgent"},
            {"id": "t-3", "scope": "area", "name": "urgent"}
        ], "page": 1, "pageSize": 100, "total": 3}).to_string()
    }

    fn urls(http: &FakeHttp) -> Vec<String> {
        http.seen.lock().unwrap().iter().map(|s| s.0.clone()).collect()
    }

    #[test]
    fn parse_item_maps_a_work_item() {
        let i = parse_item(&full_item(), &coral()).unwrap();
        assert_eq!(i.key, "coral:6f1c0000-0000-0000-0000-000000000021");
        assert_eq!(i.source, "coral");
        assert_eq!(i.id, "BED-0021");
        assert_eq!(i.number, None);
        assert_eq!(i.title, "Pay the invoice");
        assert_eq!(i.url, "https://coral.test/items/BED-0021");
        assert_eq!(i.description, "Details here");
        assert_eq!(i.status.as_deref(), Some("In progress"));
        assert_eq!(i.labels, vec!["team: api", "urgent"]);
        assert_eq!(i.assignees, vec!["Maria Silva", "ana@example.test"]);
        assert_eq!(i.author, "Rui Costa");
        assert_eq!(i.updated_at, "2026-11-10T10:00:00Z");
        assert_eq!(i.repo, None);
        assert!(i.tracker);
    }

    #[test]
    fn parse_item_tolerates_missing_fields() {
        let i = parse_item(&json!({"id": "x-1", "displayCode": "BED-0002", "title": "Bare", "description": null}), &coral()).unwrap();
        assert_eq!(i.description, "");
        assert_eq!(i.status, None);
        assert!(i.labels.is_empty() && i.assignees.is_empty());
        assert_eq!(i.author, "");
        assert!(parse_item(&json!({"displayCode": "BED-0002", "title": "No id"}), &coral()).is_none());
    }

    #[test]
    fn list_asks_for_work_items_and_defects_only_and_open_ones() {
        let http = FakeHttp::new(vec![("itemType=work_item", 200, &page(vec![numbered(1)], None)), ("itemType=defect", 200, &page(vec![numbered(2)], None))]);
        let (items, warnings) = list(&http, &coral(), &cfg(vec![rule(Assignee::Me, &[], &[])]), &me_user()).unwrap();
        assert_eq!(items.len(), 2);
        assert!(warnings.is_empty());
        let seen = urls(&http);
        assert_eq!(seen.len(), 2);
        assert!(seen[0].starts_with("https://coral.test/api/v1/work-items?"), "{}", seen[0]);
        for (u, t) in seen.iter().zip(["work_item", "defect"]) {
            assert!(u.contains(&format!("itemType={t}")), "{u}");
            assert!(u.contains("statusCategory=todo,active"), "{u}");
            assert!(u.contains("assignees=u-me"), "{u}");
            assert!(u.contains("pageSize=100"), "{u}");
            assert!(u.contains("cursor="), "{u}");
            assert!(!u.contains("phase") && !u.contains("subtask") && !u.contains("productCode") && !u.contains("tagId"), "{u}");
        }
    }

    #[test]
    fn a_rule_names_products_and_tags_by_id() {
        let http = FakeHttp::new(vec![("/tags", 200, &tags_page()), ("/work-items", 200, &page(vec![], None))]);
        list(&http, &coral(), &cfg(vec![rule(Assignee::Anyone, &["team: api"], &["BED", "OPS"])]), &me_user()).unwrap();
        let seen = urls(&http);
        let work = seen.iter().find(|u| u.contains("itemType=work_item")).unwrap();
        assert!(work.contains("productCode=BED,OPS"), "{work}");
        assert!(work.contains("tagId=t-1"), "{work}");
        assert!(work.contains("tagCombinator=and"), "{work}");
        assert!(!work.contains("assignees"), "{work}");
    }

    #[test]
    fn a_bare_tag_name_asks_once_per_matching_tag_and_merges() {
        let http = FakeHttp::new(vec![
            ("tagId=t-2", 200, &page(vec![numbered(1)], None)),
            ("tagId=t-3", 200, &page(vec![numbered(1), numbered(2)], None)),
            ("/tags", 200, &tags_page()),
        ]);
        let (items, _) = list(&http, &coral(), &cfg(vec![rule(Assignee::Anyone, &["urgent"], &[])]), &me_user()).unwrap();
        assert_eq!(items.len(), 2, "one item per key");
        assert_eq!(urls(&http).iter().filter(|u| u.contains("/tags")).count(), 1, "tags are asked once");
    }

    #[test]
    fn a_named_user_is_found_by_email_ignoring_case() {
        let users = json!({"data": [{"id": "u-9", "name": "Ana", "email": "Ana@Example.test"}], "page": 1, "pageSize": 100, "total": 1}).to_string();
        let http = FakeHttp::new(vec![("/api/v1/users?", 200, &users), ("/work-items", 200, &page(vec![], None))]);
        let rules = vec![rule(Assignee::User { login: "ana@example.test".into() }, &[], &[]), rule(Assignee::User { login: "who@example.test".into() }, &[], &[])];
        let (_, warnings) = list(&http, &coral(), &cfg(rules), &me_user()).unwrap();
        assert!(urls(&http).iter().any(|u| u.contains("assignees=u-9")));
        assert_eq!(warnings, vec!["Coral has no user who@example.test"]);
        assert_eq!(urls(&http).iter().filter(|u| u.contains("/users?")).count(), 1, "users are asked once");
    }

    #[test]
    fn an_unknown_tag_or_product_skips_only_that_rule() {
        let http = FakeHttp::new(vec![
            ("productCode=ZZZ", 404, r#"{"error":"Product not found"}"#),
            ("/tags", 200, &tags_page()),
            ("itemType=work_item", 200, &page(vec![numbered(1)], None)),
            ("itemType=defect", 200, &page(vec![], None)),
        ]);
        let rules = vec![rule(Assignee::Me, &["nope"], &[]), rule(Assignee::Me, &[], &[]), rule(Assignee::Me, &[], &["ZZZ"])];
        let (items, warnings) = list(&http, &coral(), &cfg(rules), &me_user()).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(warnings, vec!["Coral has no tag nope", "Coral has no product ZZZ"]);
    }

    #[test]
    fn every_rule_failing_is_an_error_and_a_refusal_says_so() {
        let http = FakeHttp::new(vec![("/work-items", 401, r#"{"error":"bad"}"#)]);
        let e = list(&http, &coral(), &cfg(vec![rule(Assignee::Me, &[], &[])]), &me_user()).unwrap_err();
        assert_eq!(e.0, "Coral refused the token: paste a new one in Settings → Work Items.");
        let http = FakeHttp::new(vec![]);
        assert!(list(&http, &coral(), &cfg(vec![rule(Assignee::Me, &[], &[])]), &me_user()).unwrap_err().0.starts_with("could not reach coral.test"));
    }

    #[test]
    fn a_warning_rule_does_not_hide_that_every_listing_failed() {
        let http = FakeHttp::new(vec![("/tags", 200, &tags_page()), ("/work-items", 500, "oops")]);
        let rules = vec![rule(Assignee::Me, &["nope"], &[]), rule(Assignee::Me, &[], &[])];
        let (e, warnings) = list(&http, &coral(), &cfg(rules), &me_user()).unwrap_err();
        assert_eq!(e, "500: oops");
        assert_eq!(warnings, vec!["Coral has no tag nope"]);
    }

    #[test]
    fn list_stops_at_the_cap_and_on_a_repeated_cursor() {
        // 6 pages of 100, one after the other: the cap cuts at 500.
        let pages: Vec<String> = (0..6).map(|p| page((p * 100..p * 100 + 100).map(numbered).collect(), (p < 5).then(|| ["", "c1", "c2", "c3", "c4", "c5"][p + 1]))).collect();
        let http = FakeHttp::new(vec![
            ("itemType=defect", 200, &page(vec![], None)),
            ("cursor=c1", 200, &pages[1]),
            ("cursor=c2", 200, &pages[2]),
            ("cursor=c3", 200, &pages[3]),
            ("cursor=c4", 200, &pages[4]),
            ("cursor=c5", 200, &pages[5]),
            ("itemType=work_item", 200, &pages[0]),
        ]);
        let (items, _) = list(&http, &coral(), &cfg(vec![rule(Assignee::Me, &[], &[])]), &me_user()).unwrap();
        assert_eq!(items.len(), MAX_ITEMS);
        assert!(!urls(&http).iter().any(|u| u.contains("cursor=c5")), "no page past the cap is asked");

        // A cursor that repeats itself ends the loop.
        let http = FakeHttp::new(vec![("itemType=defect", 200, &page(vec![], None)), ("cursor=same", 200, &page(vec![numbered(2)], Some("same"))), ("itemType=work_item", 200, &page(vec![numbered(1)], Some("same")))]);
        let (items, _) = list(&http, &coral(), &cfg(vec![rule(Assignee::Me, &[], &[])]), &me_user()).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(urls(&http).len(), 3);
    }

    #[test]
    fn get_reads_an_open_item() {
        let http = FakeHttp::new(vec![("/api/v1/work-items/BED-0021", 200, &full_item().to_string())]);
        let got = get(&http, &coral(), "BED-0021").unwrap().unwrap();
        assert_eq!(got.item.id, "BED-0021");
        assert_eq!(got.uuid, "6f1c0000-0000-0000-0000-000000000021");
        assert_eq!(got.item_type, "work_item");
        assert_eq!(got.status_id.as_deref(), Some("st-1"));
        assert_eq!(got.assignments, vec![Assignment { user_id: "u-me".into(), role: "assignee".into() }, Assignment { user_id: "u-3".into(), role: "assignee".into() }, Assignment { user_id: "u-4".into(), role: "reviewer".into() }]);
    }

    #[test]
    fn get_is_none_for_a_done_or_missing_item() {
        let mut done = full_item();
        done["status"]["category"] = json!("done");
        let http = FakeHttp::new(vec![("/BED-0021", 200, &done.to_string()), ("/BED-0099", 404, r#"{"error":"Work item not found"}"#), ("/BED-0500", 500, "oops")]);
        assert_eq!(get(&http, &coral(), "BED-0021").unwrap(), None);
        assert_eq!(get(&http, &coral(), "BED-0099").unwrap(), None);
        assert!(get(&http, &coral(), "BED-0500").unwrap_err().starts_with("500"));
    }

    #[test]
    fn get_follows_a_move_within_coral() {
        let http = FakeHttp::new(vec![("/work-items/OPS-0004", 200, &full_item().to_string()), ("/work-items/BED-0021", 301, "")]).with_location("/work-items/BED-0021", "/api/v1/work-items/OPS-0004");
        let got = get(&http, &coral(), "BED-0021").unwrap().unwrap();
        assert_eq!(got.item.id, "BED-0021", "the reply is parsed as sent");
        let seen = http.seen.lock().unwrap();
        assert_eq!(seen[1].0, "https://coral.test/api/v1/work-items/OPS-0004");
        assert_eq!(seen[1].1, "cbp_secret");
        drop(seen);
        let http = FakeHttp::new(vec![("/work-items/OPS-0004", 200, &full_item().to_string()), ("/work-items/BED-0021", 301, "")]).with_location("/work-items/BED-0021", "https://coral.test/api/v1/work-items/OPS-0004");
        assert!(get(&http, &coral(), "BED-0021").unwrap().is_some(), "an absolute address on the same origin");
    }

    #[test]
    fn get_does_not_follow_a_move_to_another_address() {
        for location in ["https://elsewhere.test/api/v1/work-items/OPS-0004", "http://coral.test/api/v1/work-items/OPS-0004", "//elsewhere.test/x", "OPS-0004"] {
            let http = FakeHttp::new(vec![("/work-items/BED-0021", 301, ""), ("elsewhere", 200, &full_item().to_string()), ("OPS-0004", 200, &full_item().to_string())]).with_location("/work-items/BED-0021", location);
            let e = get(&http, &coral(), "BED-0021").unwrap_err();
            assert_eq!(e, "Coral sent the request to another address, so Maya stopped.", "{location}");
            assert_eq!(http.seen.lock().unwrap().len(), 1, "{location}: only Coral was asked");
        }
    }

    #[test]
    fn a_redirect_loop_stops() {
        let http = FakeHttp::new(vec![("/work-items/BED-0021", 301, "")]).with_location("/work-items/BED-0021", "/api/v1/work-items/BED-0021");
        assert_eq!(get(&http, &coral(), "BED-0021").unwrap_err(), "Coral sent the request around in circles.");
    }

    #[test]
    fn product_of_is_the_code_before_its_number() {
        assert_eq!(product_of("BED-0021"), "BED");
        assert_eq!(product_of("MY-APP-0003"), "MY-APP");
        assert_eq!(product_of("BED"), "BED");
    }
}
