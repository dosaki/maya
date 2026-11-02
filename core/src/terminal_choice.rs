//! Which terminals a machine can open a session in, and the one a session
//! picker starts on. The same list drives Settings and the per-session pick.

use crate::config::NewSessionTerminal;
use serde::{Deserialize, Serialize};

/// What one machine offers: its platform, the terminals in Settings order,
/// and the one to preselect.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TerminalChoices {
    pub platform: String,
    pub choices: Vec<NewSessionTerminal>,
    pub default: NewSessionTerminal,
}

/// The terminals a platform (`std::env::consts::OS`) can open. iTerm2 is
/// macOS only.
pub fn for_platform(os: &str) -> Vec<NewSessionTerminal> {
    if os == "macos" {
        vec![NewSessionTerminal::Terminal, NewSessionTerminal::Iterm, NewSessionTerminal::Native]
    } else {
        vec![NewSessionTerminal::Terminal, NewSessionTerminal::Native]
    }
}

/// The platform's terminals, preselecting the saved setting when the platform
/// has it, else Terminal.
pub fn describe(os: &str, saved: NewSessionTerminal) -> TerminalChoices {
    let choices = for_platform(os);
    let default = if choices.contains(&saved) { saved } else { NewSessionTerminal::Terminal };
    TerminalChoices { platform: os.to_string(), choices, default }
}

/// `describe` for this machine.
pub fn local(saved: NewSessionTerminal) -> TerminalChoices {
    describe(std::env::consts::OS, saved)
}

/// Refuses a pick the machine does not offer, before anything opens.
pub fn check_pick(pick: Option<NewSessionTerminal>, choices: &TerminalChoices, machine: &str) -> Result<(), String> {
    match pick {
        Some(p) if !choices.choices.contains(&p) => Err(format!("That terminal is not available on {machine}.")),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mac_offers_three_and_keeps_its_saved_default() {
        let t = describe("macos", NewSessionTerminal::Iterm);
        assert_eq!(t.choices, vec![NewSessionTerminal::Terminal, NewSessionTerminal::Iterm, NewSessionTerminal::Native]);
        assert_eq!(t.default, NewSessionTerminal::Iterm);
        assert_eq!(t.platform, "macos");
    }

    #[test]
    fn iterm_saved_off_a_mac_falls_back_to_terminal() {
        assert_eq!(describe("linux", NewSessionTerminal::Iterm).default, NewSessionTerminal::Terminal);
        assert_eq!(describe("windows", NewSessionTerminal::Native).default, NewSessionTerminal::Native);
    }

    #[test]
    fn a_pick_the_machine_lacks_is_refused() {
        let linux = describe("linux", NewSessionTerminal::Terminal);
        assert_eq!(check_pick(Some(NewSessionTerminal::Iterm), &linux, "laptop").unwrap_err(), "That terminal is not available on laptop.");
        assert!(check_pick(Some(NewSessionTerminal::Native), &linux, "laptop").is_ok());
        assert!(check_pick(None, &linux, "laptop").is_ok());
    }

    #[test]
    fn choices_travel_as_camel_case() {
        let text = serde_json::to_string(&describe("macos", NewSessionTerminal::Native)).unwrap();
        assert_eq!(text, r#"{"platform":"macos","choices":["terminal","iterm","native"],"default":"native"}"#);
    }
}
