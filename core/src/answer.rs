use crate::launch::check_choice;
use crate::model::{AwaitKind, Card, State};

/// The slash command that compacts a running session's context.
pub const COMPACT: &str = "/compact";
/// The slash command that ends a Claude Code session.
pub const EXIT: &str = "/exit";

/// Shift+Tab, which cycles the permission mode in a running session.
pub const SHIFT_TAB: &str = "\x1b[Z";

/// The slash command that changes `setting` to `value` in a running session.
/// Only the model and effort have such a command; the value must be in
/// the agent's own list and plain enough to type.
pub fn slash_command(setting: &str, value: &str, models: &[String], efforts: &[String]) -> Result<String, String> {
    let allowed: Vec<&str> = match setting {
        "model" => models.iter().map(String::as_str).collect(),
        "effort" => efforts.iter().map(String::as_str).collect(),
        _ => return Err(format!("Unknown setting: {setting}")),
    };
    if !crate::launch::plain_model_id(value) {
        return Err(format!("Unknown {setting}: {value}"));
    }
    check_choice(setting, value, &allowed)?;
    Ok(format!("/{setting} {value}"))
}

/// The longest session name the board will type.
pub const MAX_NAME_CHARS: usize = 60;

/// The slash command that renames a running session, after checking the name
/// is one non-blank line of at most `MAX_NAME_CHARS` characters.
pub fn rename_command(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("The name is empty.".into());
    }
    if name.chars().any(|c| c.is_control()) {
        return Err("The name must be one line.".into());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("The name is too long (over {MAX_NAME_CHARS} characters)."));
    }
    Ok(format!("/rename {name}"))
}

/// The longest command line the board will type.
pub const MAX_COMMAND_CHARS: usize = 2000;

/// Checks a command typed in the composer for the terminal: one non-blank
/// line of at most `MAX_COMMAND_CHARS` characters that is a `/command`,
/// with a name right after the slash, or a `!` shell line, with a command
/// somewhere after the bang. It is typed into the terminal as is.
pub fn check_terminal_command(text: &str) -> Result<String, String> {
    let line = text.trim();
    if !line.starts_with(['/', '!']) {
        return Err("A command starts with / or !.".into());
    }
    if line.chars().any(|c| c.is_control()) {
        return Err("A command must be one line.".into());
    }
    if line.starts_with('/') && (line.len() == 1 || line[1..].starts_with(char::is_whitespace)) {
        return Err("The command has no name after the slash.".into());
    }
    if line.starts_with('!') && line[1..].trim().is_empty() {
        return Err("There is no command after the !.".into());
    }
    if line.chars().count() > MAX_COMMAND_CHARS {
        return Err(format!("The command is too long (over {MAX_COMMAND_CHARS} characters)."));
    }
    Ok(line.to_string())
}

/// Typing into a session that is waiting on a prompt would answer it (the
/// Enter after the text lands on the picker), so refuse until it moves on.
pub fn check_free(card: &Card) -> Result<(), String> {
    let prose_ask = card.awaiting.as_ref().map_or(false, |a| a.kind == AwaitKind::Text);
    if card.state == State::Awaiting && !prose_ask {
        return Err("This session is waiting for a decision; answer it first.".into());
    }
    Ok(())
}

/// Most characters a reply typed into a terminal may have; the inbox takes more.
pub const TYPED_MAX_CHARS: usize = 20_000;

/// `text` as keys that only ever type text: a tab becomes four spaces, and
/// escape sequences (colour codes, or Shift+Tab and the arrows a terminal
/// would act on) and the other control characters but newline go.
fn typeable(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' => out.push('\n'),
            '\t' => out.push_str("    "),
            // A CSI sequence runs to its final byte, '@' to '~'.
            '\x1b' if chars.peek() == Some(&'[') => {
                chars.next();
                while let Some(n) = chars.next() {
                    if ('@'..='~').contains(&n) {
                        break;
                    }
                }
            }
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// The lines to type for a reply to Claude Code, each followed by Enter.
/// Every line but the last ends in `\`, Claude Code's continuation, so its
/// Enter adds a line instead of sending; the last line's Enter sends. A
/// last line that itself ends in `\` gets a space, so its Enter still sends.
pub fn reply_lines(text: &str) -> Vec<String> {
    let text = typeable(text);
    let lines: Vec<&str> = text.trim_matches('\n').split('\n').collect();
    let last = lines.len() - 1;
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| match (i < last, l.ends_with('\\')) {
            (true, _) => format!("{l}\\"),
            (false, true) => format!("{l} "),
            (false, false) => l.to_string(),
        })
        .collect()
}

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

/// Refuses unless the card is still waiting on the ask the buttons were
/// rendered for (`ask_id` is that ask's awaiting timestamp), the option
/// exists, the question is single-select and the picker has had time to appear.
pub fn check(card: &Card, ask_id: u64, question_index: usize, option_index: usize, now_ms: u64) -> Result<(), String> {
    let aw = match (&card.state, &card.awaiting) {
        (State::Awaiting, Some(aw)) if aw.kind == AwaitKind::Question => aw,
        _ => return Err("This session is not waiting for a question.".into()),
    };
    if card.state_since != ask_id {
        return Err("The question has changed; look again.".into());
    }
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
            harness: crate::model::Harness::ClaudeCode,
            pr: None,
            context: None,
            machine: None,
            machine_address: None, machine_platform: None,
            terminal: None,
            stale: false, model: None
        }
    }

    #[test]
    fn slash_commands_only_for_listed_values() {
        let models = vec!["opus".to_string(), "grok-4.7".to_string()];
        let efforts = vec!["xhigh".to_string()];
        assert_eq!(slash_command("model", "opus", &models, &efforts), Ok("/model opus".to_string()));
        assert_eq!(slash_command("model", "grok-4.7", &models, &efforts), Ok("/model grok-4.7".to_string()));
        assert_eq!(slash_command("effort", "xhigh", &models, &efforts), Ok("/effort xhigh".to_string()));
        assert!(slash_command("model", "gpt", &models, &efforts).unwrap_err().contains("model"));
        assert!(slash_command("effort", "turbo", &models, &efforts).unwrap_err().contains("effort"));
        assert!(slash_command("model", "opus; rm -rf", &models, &efforts).unwrap_err().contains("model"));
        assert!(slash_command("mode", "plan", &models, &efforts).unwrap_err().contains("setting"));
    }

    #[test]
    fn rename_command_takes_a_single_clean_line() {
        assert_eq!(rename_command("  maya board  "), Ok("/rename maya board".to_string()));
        assert!(rename_command("   ").unwrap_err().contains("empty"));
        assert!(rename_command("a\nb").unwrap_err().contains("one line"));
        assert!(rename_command("tab\there").unwrap_err().contains("one line"));
        assert!(rename_command(&"x".repeat(61)).unwrap_err().contains("60"));
        assert!(rename_command(&"x".repeat(60)).is_ok());
    }

    #[test]
    fn slash_commands_from_the_composer_are_one_named_line() {
        assert_eq!(check_terminal_command("  /compact  "), Ok("/compact".to_string()));
        assert_eq!(check_terminal_command("/review  the diff"), Ok("/review  the diff".to_string()));
        assert!(check_terminal_command("hello").unwrap_err().contains("/ or !"));
        assert!(check_terminal_command("/").unwrap_err().contains("no name"));
        assert!(check_terminal_command("/ compact").unwrap_err().contains("no name"));
        assert!(check_terminal_command("/compact\nmore").unwrap_err().contains("one line"));
        assert!(check_terminal_command("/x\x1b[B").unwrap_err().contains("one line"));
        let long = format!("/{}", "a".repeat(MAX_COMMAND_CHARS));
        assert!(check_terminal_command(&long).unwrap_err().contains("too long"));
    }

    #[test]
    fn shell_lines_from_the_composer_need_a_command_after_the_bang() {
        assert_eq!(check_terminal_command("!ls -la"), Ok("!ls -la".to_string()));
        assert_eq!(check_terminal_command("  ! git status  "), Ok("! git status".to_string()));
        assert!(check_terminal_command("!").unwrap_err().contains("no command"));
        assert!(check_terminal_command("!   ").unwrap_err().contains("no command"));
        assert!(check_terminal_command("!ls\npwd").unwrap_err().contains("one line"));
        let long = format!("!{}", "a".repeat(MAX_COMMAND_CHARS));
        assert!(check_terminal_command(&long).unwrap_err().contains("too long"));
    }

    #[test]
    fn compact_is_the_compact_slash_command() {
        assert_eq!(COMPACT, "/compact");
    }

    #[test]
    fn shift_tab_is_the_reverse_tab_sequence() {
        assert_eq!(SHIFT_TAB, "\x1b[Z");
    }

    #[test]
    fn typing_is_refused_while_the_session_awaits_a_decision() {
        let mut c = card(vec![q(2, false)], 1000);
        assert!(check_free(&c).unwrap_err().contains("waiting"));
        c.state = State::Working;
        c.awaiting = None;
        assert_eq!(check_free(&c), Ok(()));
        c.state = State::Idle;
        assert_eq!(check_free(&c), Ok(()));
        // A question asked in prose leaves the terminal at its prompt: typing is safe.
        c.state = State::Awaiting;
        c.awaiting = Some(Awaiting { kind: AwaitKind::Text, detail: "Go ahead?".into(), questions: vec![] });
        assert_eq!(check_free(&c), Ok(()));
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
    fn check_accepts_an_open_question_after_one_second() {
        assert_eq!(check(&card(vec![q(3, false)], 1000), 1000, 0, 2, 2000), Ok(()));
    }

    #[test]
    fn check_refuses_every_bad_case() {
        let c = card(vec![q(3, false), q(2, true)], 1000);
        assert!(check(&c, 1000, 0, 1, 1500).unwrap_err().contains("second"));
        assert!(check(&c, 1000, 0, 3, 2000).unwrap_err().contains("no such option"));
        assert!(check(&c, 1000, 2, 0, 2000).unwrap_err().contains("no such option"));
        assert!(check(&c, 1000, 1, 0, 2000).unwrap_err().contains("Multi-select"));
        // The board rendered an earlier ask; the session has since moved to a new one.
        assert!(check(&c, 900, 0, 0, 2000).unwrap_err().contains("changed"));
        let mut working = c.clone();
        working.state = State::Working;
        working.awaiting = None;
        assert!(check(&working, 1000, 0, 0, 2000).unwrap_err().contains("not waiting"));
        let mut perm = c.clone();
        perm.awaiting = Some(Awaiting { kind: AwaitKind::Permission, detail: "Bash".into(), questions: vec![] });
        assert!(check(&perm, 1000, 0, 0, 2000).unwrap_err().contains("not waiting"));
    }

    #[test]
    fn reply_lines_continue_every_line_but_the_last() {
        assert_eq!(reply_lines("go on"), vec!["go on"]);
        assert_eq!(reply_lines("a\nb\nc"), vec!["a\\", "b\\", "c"]);
    }

    #[test]
    fn reply_lines_keep_blank_lines_and_drop_carriage_returns_and_outer_newlines() {
        assert_eq!(reply_lines("a\n\nb"), vec!["a\\", "\\", "b"]);
        assert_eq!(reply_lines("a\r\nb\r\n"), vec!["a\\", "b"]);
        assert_eq!(reply_lines("\n\nhi\n"), vec!["hi"]);
    }

    #[test]
    fn reply_lines_still_send_when_the_last_line_ends_in_a_backslash() {
        assert_eq!(reply_lines("see C:\\"), vec!["see C:\\ "]);
        // A middle line ending in `\` gets the continuation after it; the live check confirms what Claude Code makes of `\\`.
        assert_eq!(reply_lines("a\\\nb"), vec!["a\\\\", "b"]);
    }

    #[test]
    fn reply_lines_type_tabs_as_spaces_and_drop_control_keys() {
        assert_eq!(reply_lines("fn a() {\n\treturn 1;\n}"), vec!["fn a() {\\", "    return 1;\\", "}"]);
        // Coloured output pasted from a terminal: the colour codes go, the text stays.
        assert_eq!(reply_lines("\x1b[31merror\x1b[0m: boom"), vec!["error: boom"]);
        // Shift+Tab and Down as escape sequences, a bare Esc, a bell and DEL.
        assert_eq!(reply_lines("a\x1b[Zb\x1b[Bc\x1bd\x07e\x7f"), vec!["abcde"]);
    }
}
