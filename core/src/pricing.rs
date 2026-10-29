//! List prices for the models Claude Code and Codex run, in dollars per million tokens.
//!
//! Fetched 2026-10-29 from
//!   https://platform.claude.com/docs/en/about-claude/pricing (docs.anthropic.com redirects there)
//!   https://developers.openai.com/api/docs/pricing (https://openai.com/api/pricing answered 403)
//! Claude rows use the 5-minute cache-write rate. `long` carries the
//! threshold the page states: 100k for Claude Haiku 5.5, 272k for OpenAI
//! models with long-context rates. Other Claude rows have none, because the
//! 1M window is billed at the standard rate.

use crate::usage::TokenCounts;

/// The higher rates a model charges once a request's prompt passes `from` tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LongRates {
    pub from: u64,
    pub input: f64,
    pub cached_input: f64,
    pub cache_write: f64,
    pub output: f64,
}

/// Dollars per million tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Price {
    pub input: f64,
    pub cached_input: f64,
    pub cache_write: f64,
    pub output: f64,
    /// The rates past the row's own prompt-size threshold, when the model has them.
    pub long: Option<LongRates>,
}

static TABLE: &[(&str, Price)] = &[
    (
        "claude-fable-5-1",
        Price {
            input: 10.0,
            cached_input: 0.25,
            cache_write: 12.5,
            output: 50.0,
            long: None,
        },
    ),
    (
        "claude-mythos-5-1",
        Price {
            input: 10.0,
            cached_input: 0.25,
            cache_write: 12.5,
            output: 50.0,
            long: None,
        },
    ),
    (
        "claude-fable-5",
        Price {
            input: 10.0,
            cached_input: 1.0,
            cache_write: 12.5,
            output: 50.0,
            long: None,
        },
    ),
    (
        "claude-mythos-5",
        Price {
            input: 10.0,
            cached_input: 1.0,
            cache_write: 12.5,
            output: 50.0,
            long: None,
        },
    ),
    (
        "claude-opus-5-5",
        Price {
            input: 4.0,
            cached_input: 0.2,
            cache_write: 5.0,
            output: 20.0,
            long: None,
        },
    ),
    (
        "claude-opus-5",
        Price {
            input: 5.0,
            cached_input: 0.5,
            cache_write: 6.25,
            output: 25.0,
            long: None,
        },
    ),
    (
        "claude-opus-4-8",
        Price {
            input: 5.0,
            cached_input: 0.5,
            cache_write: 6.25,
            output: 25.0,
            long: None,
        },
    ),
    (
        "claude-opus-4-7",
        Price {
            input: 5.0,
            cached_input: 0.5,
            cache_write: 6.25,
            output: 25.0,
            long: None,
        },
    ),
    (
        "claude-opus-4-6",
        Price {
            input: 5.0,
            cached_input: 0.5,
            cache_write: 6.25,
            output: 25.0,
            long: None,
        },
    ),
    (
        "claude-opus-4-5",
        Price {
            input: 5.0,
            cached_input: 0.5,
            cache_write: 6.25,
            output: 25.0,
            long: None,
        },
    ),
    (
        "claude-sonnet-5-5",
        Price {
            input: 2.0,
            cached_input: 0.1,
            cache_write: 2.5,
            output: 10.0,
            long: None,
        },
    ),
    (
        "claude-sonnet-5",
        Price {
            input: 2.0,
            cached_input: 0.2,
            cache_write: 2.5,
            output: 10.0,
            long: None,
        },
    ),
    (
        "claude-sonnet-4-6",
        Price {
            input: 3.0,
            cached_input: 0.3,
            cache_write: 3.75,
            output: 15.0,
            long: None,
        },
    ),
    (
        "claude-sonnet-4-5",
        Price {
            input: 3.0,
            cached_input: 0.3,
            cache_write: 3.75,
            output: 15.0,
            long: None,
        },
    ),
    (
        "claude-haiku-5-5",
        Price {
            input: 0.1,
            cached_input: 0.01,
            cache_write: 0.125,
            output: 0.5,
            long: Some(LongRates {
                from: 100_000,
                input: 0.5,
                cached_input: 0.05,
                cache_write: 0.625,
                output: 2.5,
            }),
        },
    ),
    (
        "claude-haiku-4-5",
        Price {
            input: 1.0,
            cached_input: 0.1,
            cache_write: 1.25,
            output: 5.0,
            long: None,
        },
    ),
    // OpenAI rates have no cache-write charge, so cache_write repeats the input rate.
    // Models the page lists with no cached-input price repeat the input rate there.
    (
        "gpt-6-astra",
        Price {
            input: 10.0,
            cached_input: 1.0,
            cache_write: 10.0,
            output: 50.0,
            long: Some(LongRates {
                from: 272_000,
                input: 20.0,
                cached_input: 2.0,
                cache_write: 20.0,
                output: 75.0,
            }),
        },
    ),
    (
        "gpt-6.1-sol",
        Price {
            input: 2.0,
            cached_input: 0.1,
            cache_write: 2.0,
            output: 10.0,
            long: Some(LongRates {
                from: 272_000,
                input: 4.0,
                cached_input: 0.2,
                cache_write: 4.0,
                output: 15.0,
            }),
        },
    ),
    (
        "gpt-6.1-terra",
        Price {
            input: 1.0,
            cached_input: 0.05,
            cache_write: 1.0,
            output: 5.0,
            long: Some(LongRates {
                from: 272_000,
                input: 2.0,
                cached_input: 0.1,
                cache_write: 2.0,
                output: 7.5,
            }),
        },
    ),
    (
        "gpt-6-luna",
        Price {
            input: 0.1,
            cached_input: 0.01,
            cache_write: 0.1,
            output: 0.5,
            long: Some(LongRates {
                from: 272_000,
                input: 0.2,
                cached_input: 0.02,
                cache_write: 0.2,
                output: 0.75,
            }),
        },
    ),
    (
        "gpt-6-sol",
        Price {
            input: 2.0,
            cached_input: 0.2,
            cache_write: 2.0,
            output: 10.0,
            long: Some(LongRates {
                from: 272_000,
                input: 4.0,
                cached_input: 0.4,
                cache_write: 4.0,
                output: 15.0,
            }),
        },
    ),
    (
        "gpt-5.6-sol",
        Price {
            input: 4.0,
            cached_input: 0.4,
            cache_write: 4.0,
            output: 20.0,
            long: Some(LongRates {
                from: 272_000,
                input: 8.0,
                cached_input: 0.8,
                cache_write: 8.0,
                output: 30.0,
            }),
        },
    ),
    (
        "gpt-5.6-terra",
        Price {
            input: 2.0,
            cached_input: 0.2,
            cache_write: 2.0,
            output: 12.0,
            long: Some(LongRates {
                from: 272_000,
                input: 4.0,
                cached_input: 0.4,
                cache_write: 4.0,
                output: 18.0,
            }),
        },
    ),
    (
        "gpt-5.6-luna",
        Price {
            input: 0.2,
            cached_input: 0.02,
            cache_write: 0.2,
            output: 1.2,
            long: Some(LongRates {
                from: 272_000,
                input: 0.4,
                cached_input: 0.04,
                cache_write: 0.4,
                output: 1.8,
            }),
        },
    ),
    (
        "gpt-5.5-pro",
        Price {
            input: 30.0,
            cached_input: 30.0,
            cache_write: 30.0,
            output: 180.0,
            long: Some(LongRates {
                from: 272_000,
                input: 60.0,
                cached_input: 60.0,
                cache_write: 60.0,
                output: 270.0,
            }),
        },
    ),
    (
        "gpt-5.5",
        Price {
            input: 5.0,
            cached_input: 0.5,
            cache_write: 5.0,
            output: 30.0,
            long: Some(LongRates {
                from: 272_000,
                input: 10.0,
                cached_input: 1.0,
                cache_write: 10.0,
                output: 45.0,
            }),
        },
    ),
    (
        "gpt-5.4-pro",
        Price {
            input: 30.0,
            cached_input: 30.0,
            cache_write: 30.0,
            output: 180.0,
            long: Some(LongRates {
                from: 272_000,
                input: 60.0,
                cached_input: 60.0,
                cache_write: 60.0,
                output: 270.0,
            }),
        },
    ),
    (
        "gpt-5.4-mini",
        Price {
            input: 0.75,
            cached_input: 0.075,
            cache_write: 0.75,
            output: 4.5,
            long: None,
        },
    ),
    (
        "gpt-5.4-nano",
        Price {
            input: 0.2,
            cached_input: 0.02,
            cache_write: 0.2,
            output: 1.25,
            long: None,
        },
    ),
    (
        "gpt-5.4",
        Price {
            input: 2.5,
            cached_input: 0.25,
            cache_write: 2.5,
            output: 15.0,
            long: Some(LongRates {
                from: 272_000,
                input: 5.0,
                cached_input: 0.5,
                cache_write: 5.0,
                output: 22.5,
            }),
        },
    ),
    (
        "gpt-5.3-codex",
        Price {
            input: 1.75,
            cached_input: 0.175,
            cache_write: 1.75,
            output: 14.0,
            long: None,
        },
    ),
    (
        "gpt-5.2-pro",
        Price {
            input: 21.0,
            cached_input: 21.0,
            cache_write: 21.0,
            output: 168.0,
            long: None,
        },
    ),
    (
        "gpt-5.2",
        Price {
            input: 1.75,
            cached_input: 0.175,
            cache_write: 1.75,
            output: 14.0,
            long: None,
        },
    ),
    (
        "gpt-5.1",
        Price {
            input: 1.25,
            cached_input: 0.125,
            cache_write: 1.25,
            output: 10.0,
            long: None,
        },
    ),
    (
        "gpt-5-pro",
        Price {
            input: 15.0,
            cached_input: 15.0,
            cache_write: 15.0,
            output: 120.0,
            long: None,
        },
    ),
    (
        "gpt-5-mini",
        Price {
            input: 0.25,
            cached_input: 0.025,
            cache_write: 0.25,
            output: 2.0,
            long: None,
        },
    ),
    (
        "gpt-5-nano",
        Price {
            input: 0.05,
            cached_input: 0.005,
            cache_write: 0.05,
            output: 0.4,
            long: None,
        },
    ),
    (
        "gpt-5",
        Price {
            input: 1.25,
            cached_input: 0.125,
            cache_write: 1.25,
            output: 10.0,
            long: None,
        },
    ),
];

/// The row for a model id: `[1m]` suffix dropped, then the longest table
/// prefix that matches (`claude-opus-5-5-20260301` is the `claude-opus-5-5` row).
pub fn price_for(model: &str) -> Option<&'static Price> {
    let model = model.strip_suffix("[1m]").unwrap_or(model);
    TABLE
        .iter()
        .filter(|(prefix, _)| model.starts_with(prefix))
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|(_, price)| price)
}

/// Micro-dollars for one request's counts; `long` picks the over-200k rates
/// when the row has them. None when the model has no row.
pub fn cost_micros(model: &str, t: &TokenCounts, long: bool) -> Option<u64> {
    let p = price_for(model)?;
    let (input, cached, write, output) = match (long, p.long) {
        (true, Some(l)) => (l.input, l.cached_input, l.cache_write, l.output),
        _ => (p.input, p.cached_input, p.cache_write, p.output),
    };
    // Dollars per million tokens times tokens is micro-dollars.
    let micros = input * t.input as f64
        + cached * t.cached_input as f64
        + write * t.cache_write as f64
        + output * t.output as f64;
    Some(micros.round() as u64)
}

/// True when the model's row has long-context rates and the request's
/// input + cached + cache-write tokens exceed that row's threshold.
pub fn is_long(model: &str, t: &TokenCounts) -> bool {
    price_for(model)
        .and_then(|p| p.long)
        .is_some_and(|l| t.input + t.cached_input + t.cache_write > l.from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::TokenCounts;

    #[test]
    fn a_dated_model_id_matches_its_family_row_and_1m_suffix_is_ignored() {
        assert!(price_for("claude-opus-5-5-20260301").is_some());
        assert!(std::ptr::eq(
            price_for("claude-opus-5-5[1m]").unwrap(),
            price_for("claude-opus-5-5").unwrap()
        ));
        assert!(price_for("nonsense-9").is_none());
    }

    #[test]
    fn the_longest_prefix_wins() {
        assert!(!std::ptr::eq(
            price_for("claude-opus-5-5").unwrap(),
            price_for("claude-opus-5").unwrap()
        ));
        assert!(price_for("gpt-6.1-sol").is_some());
        assert!(price_for("gpt-5.4-mini").unwrap().input < price_for("gpt-5.4").unwrap().input);
    }

    #[test]
    fn cost_is_the_sum_of_the_four_rates_in_micros() {
        let p = price_for("claude-opus-5-5").unwrap();
        let t = TokenCounts {
            input: 1_000_000,
            cached_input: 2_000_000,
            cache_write: 500_000,
            output: 100_000,
        };
        let want = ((p.input + 2.0 * p.cached_input + 0.5 * p.cache_write + 0.1 * p.output) * 1e6)
            .round() as u64;
        assert_eq!(cost_micros("claude-opus-5-5", &t, false), Some(want));
        assert_eq!(cost_micros("nonsense-9", &t, false), None);
    }

    #[test]
    fn long_context_uses_the_long_rates_only_when_the_row_has_them() {
        let t = TokenCounts {
            input: 1_000_000,
            cached_input: 1_000_000,
            cache_write: 0,
            output: 1_000_000,
        };
        assert!(price_for("gpt-6.1-sol").unwrap().long.is_some());
        assert!(
            cost_micros("gpt-6.1-sol", &t, true).unwrap()
                > cost_micros("gpt-6.1-sol", &t, false).unwrap()
        );
        assert!(price_for("claude-opus-5-5").unwrap().long.is_none());
        assert_eq!(
            cost_micros("claude-opus-5-5", &t, true),
            cost_micros("claude-opus-5-5", &t, false)
        );
    }

    #[test]
    fn is_long_uses_the_rows_own_threshold() {
        let t = |input, cached_input, cache_write| TokenCounts {
            input,
            cached_input,
            cache_write,
            output: 0,
        };
        assert!(is_long("claude-haiku-5-5", &t(90_000, 60_000, 0)));
        assert!(!is_long("claude-opus-5-5", &t(90_000, 60_000, 0)));
        assert!(!is_long("claude-opus-5-5", &t(1_000_000, 0, 0)));
        assert!(!is_long("nonsense-9", &t(1_000_000, 0, 0)));
        assert!(!is_long("gpt-6.1-sol", &t(272_000, 0, 0)));
        assert!(is_long("gpt-6.1-sol", &t(100_000, 100_000, 72_001)));
    }
}
