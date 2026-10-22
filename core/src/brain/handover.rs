//! Handing the main role over: what the outgoing main tells the Maya whose
//! brain was dormant. Her brain writes the summary when it can; this is the
//! request she gets, and the plain digest of her Chat used otherwise.

use super::chat::ChatTurn;

/// How long the outgoing main waits for her summary before the digest.
pub const ASK_MS: u64 = 60_000;
/// The digest's length cap, in characters.
pub const DIGEST_CHARS: usize = 3_000;
/// The longest summary a Maya made main takes from the old main.
pub const RECEIVED_CHARS: usize = 4_000;
/// The digest when no turn falls in the period.
pub const NOTHING: &str = "Nothing happened in Chat.";

/// A digest of `turns` since `since` (all of them when None), one labelled
/// line per turn, oldest first. Over `max_chars`, the oldest lines go; a
/// single newest line that alone is too long is cut.
pub fn digest(turns: &[ChatTurn], since: Option<u64>, max_chars: usize) -> String {
    let lines: Vec<String> = turns.iter().filter(|t| since.map_or(true, |s| t.at >= s)).map(line).collect();
    let mut kept: Vec<&str> = Vec::new();
    let mut used = 0;
    for l in lines.iter().rev() {
        let cost = l.chars().count() + usize::from(!kept.is_empty());
        if used + cost > max_chars {
            break;
        }
        used += cost;
        kept.push(l);
    }
    if kept.is_empty() {
        return match lines.last() {
            Some(newest) => newest.chars().take(max_chars.saturating_sub(1)).chain(std::iter::once('…')).collect(),
            None => NOTHING.to_string(),
        };
    }
    kept.reverse();
    kept.join("\n")
}

/// A summary as the Maya made main takes it: trimmed, at most
/// `RECEIVED_CHARS` characters (cut with "…"); None when blank.
pub fn received(summary: &str) -> Option<String> {
    let s = summary.trim();
    if s.is_empty() {
        return None;
    }
    if s.chars().count() <= RECEIVED_CHARS {
        return Some(s.to_string());
    }
    Some(s.chars().take(RECEIVED_CHARS - 1).chain(std::iter::once('…')).collect())
}

/// One turn on one line: who said it, then the words, whitespace collapsed.
fn line(t: &ChatTurn) -> String {
    let who = if t.who == "user" { "User" } else { "Maya" };
    format!("{who}: {}", t.text.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// What the outgoing main's brain is asked: a summary for `assistant`,
/// whose brain has been dormant since `since` (None: it never ran one).
pub fn request(assistant: &str, since: Option<u64>) -> String {
    let when = since.map_or_else(|| "it first started".to_string(), utc);
    format!(
        "Write a compact handover summary for {assistant}, whose brain has been dormant since {when}: what the user asked, what you did, what is still running, any watches. Plain text, under 300 words, no ```maya blocks."
    )
}

/// Epoch milliseconds as `2026-10-21 14:03 UTC`.
pub fn utc(ms: u64) -> String {
    let secs = ms / 1000;
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Howard Hinnant's civil-from-days.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02} UTC", rest / 3_600, rest % 3_600 / 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::Via;

    fn turn(who: &str, text: &str, at: u64) -> ChatTurn {
        ChatTurn { who: who.into(), text: text.into(), at, via: Via::Chat }
    }

    #[test]
    fn the_digest_keeps_only_turns_since_the_dormant_time_and_labels_them() {
        let turns = [turn("user", "old", 1), turn("user", "make it a PR", 10), turn("maya", "Done,\n it's #52.", 11)];
        assert_eq!(digest(&turns, Some(10), 3_000), "User: make it a PR\nMaya: Done, it's #52.");
        assert_eq!(digest(&turns, None, 3_000), "User: old\nUser: make it a PR\nMaya: Done, it's #52.", "never dormant: all of it");
    }

    #[test]
    fn the_digest_keeps_the_newest_lines_within_the_cap() {
        let turns = [turn("user", "aaaa", 1), turn("maya", "bbbb", 2), turn("user", "cccc", 3)];
        // "Maya: bbbb" (10) + "\n" + "User: cccc" (10) = 21.
        assert_eq!(digest(&turns, None, 21), "Maya: bbbb\nUser: cccc");
        assert_eq!(digest(&turns, None, 20), "User: cccc");
        assert!(digest(&[turn("user", &"x".repeat(50), 1)], None, 10).chars().count() <= 10, "one long line is cut");
    }

    #[test]
    fn an_empty_period_says_nothing_happened() {
        assert_eq!(digest(&[], None, 3_000), NOTHING);
        assert_eq!(digest(&[turn("user", "old", 1)], Some(5), 3_000), NOTHING);
    }

    #[test]
    fn the_request_names_the_assistant_and_the_time_or_never() {
        let r = request("laptop", Some(1_792_591_380_000));
        assert!(r.starts_with("Write a compact handover summary for laptop, whose brain has been dormant since 2026-10-21 "), "{r}");
        assert!(r.contains("no ```maya blocks"));
        assert!(request("laptop", None).contains("dormant since it first started"));
    }

    #[test]
    fn a_received_summary_is_trimmed_capped_and_blank_is_none() {
        assert_eq!(received("  Did #52. \n"), Some("Did #52.".to_string()));
        assert_eq!(received(" \n "), None);
        let long = received(&"x".repeat(RECEIVED_CHARS + 50)).unwrap();
        assert_eq!(long.chars().count(), RECEIVED_CHARS);
        assert!(long.ends_with('…'));
        assert_eq!(received(&"é".repeat(RECEIVED_CHARS)).unwrap().chars().count(), RECEIVED_CHARS, "at the cap, kept whole");
    }

    #[test]
    fn utc_formats_epoch_milliseconds() {
        assert_eq!(utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(utc(951_782_400_000), "2000-02-29 00:00 UTC");
        assert_eq!(utc(1_792_591_380_000), "2026-10-21 14:03 UTC");
    }
}
