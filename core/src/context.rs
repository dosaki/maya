use serde::{Deserialize, Serialize};

pub const WINDOW_200K: u64 = 200_000;
pub const WINDOW_1M: u64 = 1_000_000;

/// How full a session's context window is.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ContextUsage {
    pub used: u64,
    pub window: u64,
    /// 0 to 100.
    pub percent: u8,
}

/// The window a session most likely has. Transcripts rarely say, so: 1M when
/// the model id carries the marker, when usage already exceeds 200k, or when
/// the settings default (`default_1m`) is a 1M model; otherwise 200k.
pub fn window_for(model: &str, used: u64, default_1m: bool) -> u64 {
    if model.contains("[1m]") || used > WINDOW_200K || default_1m {
        WINDOW_1M
    } else {
        WINDOW_200K
    }
}

pub fn usage(model: &str, used: u64, default_1m: bool) -> ContextUsage {
    let window = window_for(model, used, default_1m);
    let percent = ((used as f64 / window as f64) * 100.0).round().min(100.0) as u8;
    ContextUsage { used, window, percent }
}

/// True when the `model` in a settings.json text names a 1M-context model.
pub fn default_is_1m(settings_json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(settings_json)
        .ok()
        .and_then(|v| v["model"].as_str().map(|m| m.contains("[1m]")))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_is_one_million_for_marked_models_big_usage_or_a_1m_default() {
        assert_eq!(window_for("claude-opus-5", 50_000, false), 200_000);
        assert_eq!(window_for("claude-opus-5[1m]", 50_000, false), 1_000_000);
        assert_eq!(window_for("claude-opus-5", 250_000, false), 1_000_000);
        assert_eq!(window_for("claude-opus-5", 50_000, true), 1_000_000);
    }

    #[test]
    fn usage_reports_percent_rounded_and_capped() {
        let u = usage("claude-opus-5", 124_000, false);
        assert_eq!(u, ContextUsage { used: 124_000, window: 200_000, percent: 62 });
        assert_eq!(usage("x", 199_999, false).percent, 100);
        assert_eq!(usage("x", 1_500_000, false).percent, 100);
        assert_eq!(usage("x", 0, false).percent, 0);
    }

    #[test]
    fn settings_default_window_is_read_from_the_model_field() {
        assert!(default_is_1m(r#"{"model": "claude-fable-5-1[1m]"}"#));
        assert!(!default_is_1m(r#"{"model": "opus"}"#));
        assert!(!default_is_1m("not json"));
        assert!(!default_is_1m("{}"));
    }
}
