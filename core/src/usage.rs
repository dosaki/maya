//! What a session has used, what the account has left, and what it cost.
//! Costs are whole micro-units so the types stay `Eq`.

use serde::{Deserialize, Serialize};

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
}
