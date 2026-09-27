use crate::model::{AwaitKind, Card, State};
use std::process::Command;

/// The picker must be on screen before any key is sent; PreToolUse fires just before it renders.
pub const OPEN_DELAY_MS: u64 = 1000;
/// Pause between the last answer and the Enter on the "Submit answers" screen.
pub const SUBMIT_DELAY_MS: u64 = 400;

/// Down-arrow escape sequences that move the picker cursor to `option_index`
/// (0-based). `do script` appends the Enter that selects it.
pub fn keys_for_option(option_index: usize) -> String {
    "\x1b[B".repeat(option_index)
}

/// A multi-question ask ends on a review screen that needs one more Enter.
pub fn needs_submit(question_index: usize, question_count: usize) -> bool {
    question_count > 1 && question_index + 1 == question_count
}

fn applescript_string(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Types `text` (plus Enter) into the Terminal tab on `tty` without activating Terminal.
pub fn applescript_type(tty: &str, text: &str) -> String {
    format!(
        r#"tell application "Terminal"
  repeat with w in windows
    repeat with t in tabs of w
      if tty of t is "{tty}" then
        do script "{}" in t
        return "ok"
      end if
    end repeat
  end repeat
end tell
return "not found""#,
        applescript_string(text)
    )
}

/// Refuses unless the card is still waiting on that exact question, the
/// option exists, the question is single-select and the picker has had time
/// to appear.
pub fn check(card: &Card, question_index: usize, option_index: usize, now_ms: u64) -> Result<(), String> {
    let aw = match (&card.state, &card.awaiting) {
        (State::Awaiting, Some(aw)) if aw.kind == AwaitKind::Question => aw,
        _ => return Err("This session is not waiting for a question.".into()),
    };
    let Some(q) = aw.questions.get(question_index) else {
        return Err("That question has no such option.".into());
    };
    if option_index >= q.options.len() {
        return Err("That question has no such option.".into());
    }
    if q.multi_select {
        return Err("Multi-select questions must be answered in the terminal.".into());
    }
    if now_ms.saturating_sub(card.state_since) < OPEN_DELAY_MS {
        return Err("Give the terminal a second to show the question.".into());
    }
    Ok(())
}

pub fn type_into_tty(tty: &str, text: &str) -> Result<(), String> {
    let out = Command::new("osascript")
        .arg("-e")
        .arg(applescript_type(tty, text))
        .output()
        .map_err(|e| format!("could not run osascript: {e}"))?;
    if !out.status.success() {
        return Err(format!("osascript failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    if String::from_utf8_lossy(&out.stdout).trim() == "ok" {
        Ok(())
    } else {
        Err(format!("No Terminal tab found for {tty}."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AwaitKind, Awaiting, Card, Choice, Question, State};

    fn q(n: usize, multi: bool) -> Question {
        Question {
            question: "Q?".into(),
            header: "H".into(),
            multi_select: multi,
            options: (0..n).map(|i| Choice { label: format!("o{i}"), description: "".into() }).collect(),
        }
    }
    fn card(questions: Vec<Question>, since: u64) -> Card {
        Card {
            session_id: "s".into(),
            pid: 1,
            name: "n".into(),
            cwd: "/x".into(),
            state: State::Awaiting,
            state_since: since,
            snippet: "".into(),
            awaiting: Some(Awaiting { kind: AwaitKind::Question, detail: "Q?".into(), questions }),
            has_inbox: true,
        }
    }

    #[test]
    fn keys_are_down_arrows_only() {
        assert_eq!(keys_for_option(0), "");
        assert_eq!(keys_for_option(2), "\x1b[B\x1b[B");
    }

    #[test]
    fn submit_only_after_last_of_several() {
        assert!(!needs_submit(0, 1));
        assert!(!needs_submit(0, 2));
        assert!(needs_submit(1, 2));
    }

    #[test]
    fn applescript_types_without_activating_and_escapes_text() {
        let s = applescript_type("/dev/ttys021", "a\"b\\c");
        assert!(s.contains("if tty of t is \"/dev/ttys021\""));
        assert!(s.contains("do script \"a\\\"b\\\\c\" in t"), "{s}");
        assert!(!s.contains("activate"));
        assert!(s.contains("return \"not found\""));
    }

    #[test]
    fn check_accepts_an_open_question_after_one_second() {
        assert_eq!(check(&card(vec![q(3, false)], 1000), 0, 2, 2000), Ok(()));
    }

    #[test]
    fn check_refuses_every_bad_case() {
        let c = card(vec![q(3, false), q(2, true)], 1000);
        assert!(check(&c, 0, 1, 1500).unwrap_err().contains("second"));
        assert!(check(&c, 0, 3, 2000).unwrap_err().contains("no such option"));
        assert!(check(&c, 2, 0, 2000).unwrap_err().contains("no such option"));
        assert!(check(&c, 1, 0, 2000).unwrap_err().contains("Multi-select"));
        let mut working = c.clone();
        working.state = State::Working;
        working.awaiting = None;
        assert!(check(&working, 0, 0, 2000).unwrap_err().contains("not waiting"));
        let mut perm = c.clone();
        perm.awaiting = Some(Awaiting { kind: AwaitKind::Permission, detail: "Bash".into(), questions: vec![] });
        assert!(check(&perm, 0, 0, 2000).unwrap_err().contains("not waiting"));
    }
}
