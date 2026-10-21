//! The saved record of her session, and whether to resume it or start fresh.

use std::path::Path;

use crate::model::Harness;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub agent: Harness,
    pub model: Option<String>,
    pub session_id: String,
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
        let r = Record { agent: Harness::ClaudeCode, model: None, session_id: "u".into() };
        assert!(matches!(decide(Some(&r), Harness::ClaudeCode, None), Start::Resume(id) if id == "u"));
        assert!(matches!(decide(Some(&r), Harness::Grok, None), Start::Fresh { reason: Some(_) }));
        assert!(matches!(decide(None, Harness::ClaudeCode, None), Start::Fresh { reason: None }));
    }
    #[test]
    fn the_reason_names_the_new_agent() {
        let r = Record { agent: Harness::ClaudeCode, model: None, session_id: "u".into() };
        assert_eq!(decide(Some(&r), Harness::Grok, None), Start::Fresh { reason: Some("You switched my brain to Grok Build, so I've started fresh.".into()) });
    }
    #[test]
    fn a_new_model_resumes_on_claude_and_starts_fresh_elsewhere() {
        let c = Record { agent: Harness::ClaudeCode, model: Some("opus".into()), session_id: "u".into() };
        assert_eq!(decide(Some(&c), Harness::ClaudeCode, Some("sonnet")), Start::Resume("u".into()));
        let x = Record { agent: Harness::Codex, model: None, session_id: "v".into() };
        assert!(matches!(decide(Some(&x), Harness::Codex, Some("gpt")), Start::Fresh { reason: Some(r) } if r.contains("model")));
        assert_eq!(decide(Some(&x), Harness::Codex, None), Start::Resume("v".into()));
    }
    #[test]
    fn a_record_round_trips_and_a_bad_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("brain").join("record.json");
        assert!(load(&p).is_none());
        let r = Record { agent: Harness::Codex, model: Some("m".into()), session_id: "s".into() };
        save(&p, &r).unwrap();
        assert_eq!(load(&p), Some(r.clone()));
        let r2 = Record { session_id: "t".into(), ..r };
        save(&p, &r2).unwrap();
        assert_eq!(load(&p), Some(r2));
        std::fs::write(&p, "{nope").unwrap();
        assert!(load(&p).is_none());
    }
}
