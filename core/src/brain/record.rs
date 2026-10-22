//! The saved record of her session, and whether to resume it or start fresh.

use std::path::Path;

use crate::model::Harness;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub agent: Harness,
    pub model: Option<String>,
    pub session_id: String,
    /// When her brain went dormant on this machine (epoch ms): it stopped
    /// because the machine became an assistant. None while she runs here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dormant_since: Option<u64>,
}

#[derive(Debug, PartialEq)]
pub enum Start {
    Resume(String),
    Fresh { reason: Option<String> },
}

pub fn load(path: &Path) -> Option<Record> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

pub fn save(path: &Path, r: &Record) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(r).map_err(|e| e.to_string())?;
    crate::config::write_private_atomic(path, &bytes)
}

/// Marks the record dormant from `now`, keeping an earlier mark. True when
/// it changed and must be saved.
pub fn mark_dormant(r: &mut Record, now: u64) -> bool {
    if r.dormant_since.is_some() {
        return false;
    }
    r.dormant_since = Some(now);
    true
}

/// Since when her brain has been dormant on an assistant: None when this
/// machine never ran one (no record); `now` for a record not marked yet.
pub fn dormant_since(saved: Option<&Record>, now: u64) -> Option<u64> {
    saved.map(|r| r.dormant_since.unwrap_or(now))
}

pub fn decide(saved: Option<&Record>, agent: Harness, model: Option<&str>) -> Start {
    let Some(r) = saved else {
        return Start::Fresh { reason: None };
    };
    if r.agent != agent {
        return Start::Fresh {
            reason: Some(format!("You switched my brain to {}, so I've started fresh.", crate::launch::label(agent))),
        };
    }
    // Claude Code applies `--model` on `--resume`; the others cannot.
    if r.model.as_deref() != model && agent != Harness::ClaudeCode {
        return Start::Fresh { reason: Some("You changed my brain's model, so I've started fresh.".into()) };
    }
    Start::Resume(r.session_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_agent_starts_fresh_and_says_why() {
        let r = Record { agent: Harness::ClaudeCode, model: None, session_id: "u".into(), dormant_since: None };
        assert!(matches!(decide(Some(&r), Harness::ClaudeCode, None), Start::Resume(id) if id == "u"));
        assert!(matches!(decide(Some(&r), Harness::Grok, None), Start::Fresh { reason: Some(_) }));
        assert!(matches!(decide(None, Harness::ClaudeCode, None), Start::Fresh { reason: None }));
    }
    #[test]
    fn the_reason_names_the_new_agent() {
        let r = Record { agent: Harness::ClaudeCode, model: None, session_id: "u".into(), dormant_since: None };
        assert_eq!(decide(Some(&r), Harness::Grok, None), Start::Fresh { reason: Some("You switched my brain to Grok Build, so I've started fresh.".into()) });
    }
    #[test]
    fn a_new_model_resumes_on_claude_and_starts_fresh_elsewhere() {
        let c = Record { agent: Harness::ClaudeCode, model: Some("opus".into()), session_id: "u".into(), dormant_since: None };
        assert_eq!(decide(Some(&c), Harness::ClaudeCode, Some("sonnet")), Start::Resume("u".into()));
        let x = Record { agent: Harness::Codex, model: None, session_id: "v".into(), dormant_since: None };
        assert!(matches!(decide(Some(&x), Harness::Codex, Some("gpt")), Start::Fresh { reason: Some(r) } if r.contains("model")));
        assert_eq!(decide(Some(&x), Harness::Codex, None), Start::Resume("v".into()));
    }
    #[test]
    fn a_record_round_trips_dormant_since_and_an_old_record_has_none() {
        let r = Record { agent: Harness::ClaudeCode, model: None, session_id: "u".into(), dormant_since: Some(42) };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains(r#""dormantSince":42"#), "{s}");
        assert_eq!(serde_json::from_str::<Record>(&s).unwrap(), r);
        let old: Record = serde_json::from_str(r#"{"agent":"claude-code","model":null,"sessionId":"u"}"#).unwrap();
        assert_eq!(old.dormant_since, None);
        assert!(!serde_json::to_string(&old).unwrap().contains("dormantSince"), "awake, no field");
    }

    #[test]
    fn going_dormant_marks_the_record_once_and_keeps_the_first_time() {
        let mut r = Record { agent: Harness::ClaudeCode, model: None, session_id: "u".into(), dormant_since: None };
        assert!(mark_dormant(&mut r, 10));
        assert_eq!(r.dormant_since, Some(10));
        assert!(!mark_dormant(&mut r, 20), "already dormant: nothing to save");
        assert_eq!(r.dormant_since, Some(10));
    }

    #[test]
    fn dormant_since_is_none_without_a_record_and_now_for_an_unmarked_one() {
        assert_eq!(dormant_since(None, 99), None, "never ran a brain");
        let r = Record { agent: Harness::ClaudeCode, model: None, session_id: "u".into(), dormant_since: None };
        assert_eq!(dormant_since(Some(&r), 99), Some(99));
        let d = Record { dormant_since: Some(5), ..r };
        assert_eq!(dormant_since(Some(&d), 99), Some(5));
    }

    #[test]
    fn a_record_round_trips_and_a_bad_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("brain").join("record.json");
        assert!(load(&p).is_none());
        let r = Record { agent: Harness::Codex, model: Some("m".into()), session_id: "s".into(), dormant_since: None };
        save(&p, &r).unwrap();
        assert_eq!(load(&p), Some(r.clone()));
        let r2 = Record { session_id: "t".into(), ..r };
        save(&p, &r2).unwrap();
        assert_eq!(load(&p), Some(r2));
        std::fs::write(&p, "{nope").unwrap();
        assert!(load(&p).is_none());
    }
}
