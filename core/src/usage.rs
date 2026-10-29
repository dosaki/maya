//! What a session has used, what the account has left, and what it cost.
//! Costs are whole micro-units so the types stay `Eq`.

use crate::pricing;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TokenCounts {
    pub input: u64,
    pub cached_input: u64,
    pub cache_write: u64,
    pub output: u64,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CostUnit {
    Usd,
    Credits,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Cost {
    pub micros: u64,
    pub unit: CostUnit,
    pub estimated: bool,
    #[serde(default)]
    pub models: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionUsage {
    #[serde(flatten)]
    pub tokens: TokenCounts,
    pub cost: Option<Cost>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LimitWindow {
    pub label: String,
    pub percent: u8,
    pub resets_at: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountLimits {
    pub account: Option<String>,
    pub windows: Vec<LimitWindow>,
    pub read_at: u64,
}

/// "5h" for 300 minutes, "7d" for 10080, else "<n>h" (whole hours) or "<n>d"
/// (whole days, when at least 48 h and divisible by 1440).
pub fn window_label(minutes: u64) -> String {
    if minutes >= 2880 && minutes % 1440 == 0 {
        format!("{}d", minutes / 1440)
    } else {
        format!("{}h", minutes / 60)
    }
}

/// Per refresh, across all transcripts.
pub const TOTALS_BUDGET_BYTES: u64 = 8 * 1_048_576;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub tokens: TokenCounts,
    pub cost_micros: u64,
    /// Models with a price row, in first-seen order.
    pub priced: Vec<String>,
    /// Models without one.
    pub unpriced: Vec<String>,
}

#[derive(Default)]
struct FileState {
    consumed: u64,
    carry: Vec<u8>,
    totals: Totals,
}

#[derive(Default)]
pub struct TranscriptTotals {
    files: HashMap<PathBuf, FileState>,
}

impl TranscriptTotals {
    /// Reads what was appended to `path` since the last call, at most `budget`
    /// bytes, and returns the totals so far. A file shorter than what was
    /// consumed starts over from zero.
    pub fn get(&mut self, path: &Path, budget: u64) -> Totals {
        let state = self.files.entry(path.to_path_buf()).or_default();
        let Ok(mut file) = std::fs::File::open(path) else { return state.totals.clone() };
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        if len < state.consumed {
            *state = FileState::default();
        }
        if file.seek(SeekFrom::Start(state.consumed)).is_err() {
            return state.totals.clone();
        }
        let mut buf = Vec::new();
        if file.take(budget).read_to_end(&mut buf).is_err() {
            return state.totals.clone();
        }
        state.consumed += buf.len() as u64;
        let mut data = std::mem::take(&mut state.carry);
        data.extend_from_slice(&buf);
        let end = data.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
        state.carry = data.split_off(end);
        for raw in data.split(|&b| b == b'\n') {
            let Ok(v) = serde_json::from_slice::<serde_json::Value>(raw) else { continue };
            if let Some((model, counts)) = record_counts(&v) {
                add_record(&mut state.totals, &model, &counts);
            }
        }
        state.totals.clone()
    }

    pub fn retain(&mut self, live: &[PathBuf]) {
        self.files.retain(|p, _| live.contains(p));
    }
}

fn add_record(totals: &mut Totals, model: &str, counts: &TokenCounts) {
    let t = &mut totals.tokens;
    t.input += counts.input;
    t.cached_input += counts.cached_input;
    t.cache_write += counts.cache_write;
    t.output += counts.output;
    let (list, cost) = match pricing::cost_micros(model, counts, pricing::is_long(model, counts)) {
        Some(c) => (&mut totals.priced, c),
        None => (&mut totals.unpriced, 0),
    };
    totals.cost_micros += cost;
    if !list.iter().any(|m| m == model) {
        list.push(model.to_string());
    }
}

/// One main-chain assistant record's counts: `message.usage` of a record whose
/// `type` is `assistant`, `isSidechain` is not true, and `message.model` is
/// non-empty and not `<synthetic>`.
pub fn record_counts(v: &serde_json::Value) -> Option<(String, TokenCounts)> {
    if v["type"].as_str() != Some("assistant") || v["isSidechain"].as_bool() == Some(true) {
        return None;
    }
    let model = v["message"]["model"].as_str().filter(|m| !m.is_empty() && !m.starts_with('<'))?;
    let u = &v["message"]["usage"];
    let n = |k: &str| u[k].as_u64().unwrap_or(0);
    Some((
        model.to_string(),
        TokenCounts {
            input: n("input_tokens"),
            cached_input: n("cache_read_input_tokens"),
            cache_write: n("cache_creation_input_tokens"),
            output: n("output_tokens"),
        },
    ))
}

/// `Totals` as the card's usage: estimated, with `priced` as `models`;
/// `cost: None` when nothing was priced.
pub fn session_usage_of(t: &Totals) -> SessionUsage {
    SessionUsage {
        tokens: t.tokens,
        cost: (!t.priced.is_empty()).then(|| Cost {
            micros: t.cost_micros,
            unit: CostUnit::Usd,
            estimated: true,
            models: t.priced.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_labels_name_the_known_windows_and_fall_back_to_hours_or_days() {
        assert_eq!(window_label(300), "5h");
        assert_eq!(window_label(10080), "7d");
        assert_eq!(window_label(60), "1h");
        assert_eq!(window_label(2880), "2d");
        assert_eq!(window_label(90), "1h");
    }

    #[test]
    fn session_usage_round_trips_with_flattened_tokens() {
        let usage = SessionUsage {
            tokens: TokenCounts { input: 1, cached_input: 2, cache_write: 3, output: 4 },
            cost: Some(Cost { micros: 4_120_000, unit: CostUnit::Usd, estimated: true, models: vec!["claude-opus-5-5".into()] }),
        };
        let json = serde_json::to_value(&usage).unwrap();
        assert_eq!(json["input"], 1);
        assert_eq!(json["cachedInput"], 2);
        assert_eq!(json["cacheWrite"], 3);
        assert_eq!(json["output"], 4);
        assert_eq!(json["cost"]["unit"], "usd");
        let back: SessionUsage = serde_json::from_value(json).unwrap();
        assert_eq!(back, usage);
    }

    fn line(model: &str, input: u64, cached: u64, write: u64, output: u64) -> String {
        format!(
            r#"{{"type":"assistant","isSidechain":false,"message":{{"model":"{model}","usage":{{"input_tokens":{input},"cache_read_input_tokens":{cached},"cache_creation_input_tokens":{write},"output_tokens":{output}}},"content":[]}}}}"#
        )
    }

    #[test]
    fn record_counts_skips_synthetic_sidechain_and_user_records() {
        let real: serde_json::Value = serde_json::from_str(&line("claude-opus-5-5", 10, 1000, 90, 5)).unwrap();
        let synthetic: serde_json::Value = serde_json::from_str(&line("<synthetic>", 10, 1000, 90, 5)).unwrap();
        let mut side = real.clone();
        side["isSidechain"] = true.into();
        let user = serde_json::json!({"type":"user","message":{"content":"hi"}});
        assert_eq!(
            record_counts(&real),
            Some(("claude-opus-5-5".to_string(), TokenCounts { input: 10, cached_input: 1000, cache_write: 90, output: 5 }))
        );
        assert_eq!(record_counts(&synthetic), None);
        assert_eq!(record_counts(&side), None);
        assert_eq!(record_counts(&user), None);
    }

    #[test]
    fn totals_grow_with_appended_lines_and_restart_when_the_file_shrinks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let (a, b, c) = (line("claude-opus-5-5", 1, 2, 3, 4), line("claude-opus-5-5", 10, 20, 30, 40), line("claude-opus-5-5", 100, 200, 300, 400));
        std::fs::write(&path, format!("{a}\n{b}\n")).unwrap();
        let mut totals = TranscriptTotals::default();
        let t = totals.get(&path, u64::MAX);
        assert_eq!(t.tokens, TokenCounts { input: 11, cached_input: 22, cache_write: 33, output: 44 });
        std::fs::write(&path, format!("{a}\n{b}\n{c}\n")).unwrap();
        let t = totals.get(&path, u64::MAX);
        assert_eq!(t.tokens, TokenCounts { input: 111, cached_input: 222, cache_write: 333, output: 444 });
        std::fs::write(&path, format!("{a}\n")).unwrap();
        let t = totals.get(&path, u64::MAX);
        assert_eq!(t.tokens, TokenCounts { input: 1, cached_input: 2, cache_write: 3, output: 4 });
    }

    #[test]
    fn a_partial_last_line_is_carried_to_the_next_read() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let (a, b) = (line("claude-opus-5-5", 1, 0, 0, 0), line("claude-opus-5-5", 10, 0, 0, 0));
        let (head, tail) = b.split_at(b.len() / 2);
        std::fs::write(&path, format!("{a}\n{head}")).unwrap();
        let mut totals = TranscriptTotals::default();
        assert_eq!(totals.get(&path, u64::MAX).tokens.input, 1);
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        write!(f, "{tail}\n").unwrap();
        assert_eq!(totals.get(&path, u64::MAX).tokens.input, 11);
    }

    #[test]
    fn the_budget_caps_one_read_and_the_rest_comes_next_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let a = line("claude-opus-5-5", 1, 0, 0, 0);
        let body = format!("{a}\n{a}\n{a}\n");
        std::fs::write(&path, &body).unwrap();
        let mut totals = TranscriptTotals::default();
        assert_eq!(totals.get(&path, a.len() as u64 + 2).tokens.input, 1);
        assert_eq!(totals.get(&path, u64::MAX).tokens.input, 3);
    }

    #[test]
    fn cost_is_summed_per_record_with_long_rates_per_request() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let model = "claude-haiku-5-5";
        std::fs::write(&path, format!("{}\n{}\n", line(model, 50_000, 0, 0, 10), line(model, 150_000, 0, 0, 10))).unwrap();
        let small = TokenCounts { input: 50_000, output: 10, ..Default::default() };
        let big = TokenCounts { input: 150_000, output: 10, ..Default::default() };
        assert!(!pricing::is_long(model, &small));
        assert!(pricing::is_long(model, &big));
        let want = pricing::cost_micros(model, &small, false).unwrap() + pricing::cost_micros(model, &big, true).unwrap();
        let t = TranscriptTotals::default().get(&path, u64::MAX);
        assert_eq!(t.cost_micros, want);
        assert_eq!(t.priced, vec![model.to_string()]);
        assert!(t.unpriced.is_empty());
    }

    #[test]
    fn an_unpriced_model_is_listed_and_contributes_no_cost() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        std::fs::write(&path, format!("{}\n", line("made-up-model", 5, 0, 0, 5))).unwrap();
        let t = TranscriptTotals::default().get(&path, u64::MAX);
        assert_eq!(t.unpriced, vec!["made-up-model".to_string()]);
        assert_eq!(t.cost_micros, 0);
        assert!(session_usage_of(&t).cost.is_none());
    }
}
