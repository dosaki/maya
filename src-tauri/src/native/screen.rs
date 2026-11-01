//! A headless screen that follows everything a session prints, so a viewer
//! that opens late can be shown the same screen and its history.

use std::num::NonZeroU16;

/// Lines of history a screen keeps.
pub const SCROLLBACK: usize = 10_000;

/// A session's screen as a terminal would show it, with its history.
pub struct Screen {
    parser: vt100::Parser,
}

/// A size the parser takes: at least one row or column.
fn dim(n: u16) -> NonZeroU16 {
    NonZeroU16::new(n.max(1)).unwrap()
}

impl Screen {
    pub fn new(rows: u16, cols: u16) -> Screen {
        Screen { parser: vt100::Parser::new(dim(rows), dim(cols), SCROLLBACK) }
    }

    /// Reads bytes the program printed. A character split across two calls
    /// is joined up.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.set_size(dim(rows), dim(cols));
    }

    /// The size as (rows, cols).
    pub fn size(&self) -> (u16, u16) {
        let (rows, cols) = self.parser.screen().size();
        (rows.get(), cols.get())
    }

    /// Up to `n` of the screen's last lines that are not blank, as plain
    /// text, oldest first.
    pub fn last_lines(&self, n: usize) -> Vec<String> {
        let contents = self.parser.screen().contents();
        let lines: Vec<&str> = contents.lines().map(str::trim_end).filter(|l| !l.is_empty()).collect();
        lines[lines.len().saturating_sub(n)..].iter().map(|l| l.to_string()).collect()
    }

    /// Bytes that make a fresh terminal of the same size show this screen:
    /// its contents, cursor and input modes, with up to `scrollback` lines
    /// of history above it. What vt100 keeps no record of is not carried:
    /// the scroll region, origin mode, the saved cursor, and the main
    /// screen while the alternate screen is up.
    pub fn snapshot(&mut self, scrollback: usize) -> Vec<u8> {
        let (rows, cols) = self.size();
        let mut out = String::from("\x1bc");
        let screen = self.parser.screen_mut();
        if !screen.alternate_screen() {
            let n = scrollback.min(screen.scrollback_len());
            if n > 0 {
                let page = usize::from(rows);
                let mut s = n;
                while s > 0 {
                    screen.set_scrollback(s);
                    for line in screen.rows_formatted(0, cols).take(page.min(s)) {
                        out.push_str(&line);
                        out.push_str("\x1b[0m\r\n");
                    }
                    s = s.saturating_sub(page);
                }
                screen.set_scrollback(0);
                // Each of the n lines ended in a line break, so rows - 1
                // more push exactly those n lines into the viewer's history.
                out.push_str(&"\r\n".repeat(page - 1));
            }
        }
        if screen.alternate_screen() {
            out.push_str("\x1b[?1049h");
        }
        out.push_str(&screen.state_formatted());
        out.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plays `screen`'s snapshot into a fresh parser of the same size.
    fn replay(screen: &mut Screen, scrollback: usize) -> vt100::Parser {
        let (rows, cols) = screen.size();
        let mut fresh = vt100::Parser::new(NonZeroU16::new(rows).unwrap(), NonZeroU16::new(cols).unwrap(), SCROLLBACK);
        fresh.process(&screen.snapshot(scrollback));
        fresh
    }

    #[test]
    fn a_snapshot_redraws_the_same_screen_and_cursor() {
        let mut screen = Screen::new(24, 80);
        screen.feed(b"\x1b[31mred\x1b[0m\r\nplain");
        let fresh = replay(&mut screen, 0);
        assert_eq!(fresh.screen().contents(), screen.parser.screen().contents());
        assert_eq!(fresh.screen().cursor_position(), screen.parser.screen().cursor_position());
        assert_eq!(fresh.screen().cell(0, 0).unwrap().fgcolor(), vt100::Color::Idx(1));
    }

    #[test]
    fn a_snapshot_carries_the_scrollback_it_is_asked_for() {
        let mut screen = Screen::new(5, 80);
        let lines: Vec<String> = (0..20).map(|i| format!("l{i}")).collect();
        screen.feed(lines.join("\r\n").as_bytes());

        let mut fresh = replay(&mut screen, SCROLLBACK);
        assert_eq!(fresh.screen().scrollback_len(), 15);
        assert_eq!(fresh.screen().contents(), screen.parser.screen().contents());
        fresh.screen_mut().set_scrollback(15);
        assert_eq!(fresh.screen().rows(0, 80).next().unwrap(), "l0");
        fresh.screen_mut().set_scrollback(1);
        assert_eq!(fresh.screen().rows(0, 80).next().unwrap(), "l14");

        let mut fresh = replay(&mut screen, 3);
        assert_eq!(fresh.screen().scrollback_len(), 3);
        let history: Vec<String> = (1..=3).rev().map(|s| top_row(&mut fresh, s)).collect();
        assert_eq!(history, ["l12", "l13", "l14"], "the newest three");
    }

    /// The top row of `parser`'s screen scrolled back `s` lines.
    fn top_row(parser: &mut vt100::Parser, s: usize) -> String {
        parser.screen_mut().set_scrollback(s);
        let (_, cols) = parser.screen().size();
        parser.screen().rows(0, cols.get()).next().unwrap()
    }

    #[test]
    fn the_history_keeps_its_colours_and_its_wrapped_lines() {
        let mut screen = Screen::new(5, 20);
        let long = format!("{}{}", "a".repeat(20), "b".repeat(5));
        screen.feed(format!("\x1b[31mred\x1b[0m\r\nplain\r\n{long}\r\n").as_bytes());
        // Enough lines to push those into the history.
        screen.feed((0..10).map(|i| format!("l{i}")).collect::<Vec<_>>().join("\r\n").as_bytes());

        let mut fresh = replay(&mut screen, SCROLLBACK);
        let depth = fresh.screen().scrollback_len();
        fresh.screen_mut().set_scrollback(depth);
        let screen = fresh.screen();
        assert_eq!(screen.rows(0, 20).take(4).collect::<Vec<_>>(), ["red", "plain", "a".repeat(20).as_str(), "bbbbb"], "both halves of the wrapped line");
        assert_eq!(screen.cell(0, 0).unwrap().fgcolor(), vt100::Color::Idx(1), "the coloured line");
        assert_eq!(screen.cell(1, 0).unwrap().fgcolor(), vt100::Color::Default, "the line after it");
    }

    #[test]
    fn a_snapshot_of_the_alternate_screen_enters_it() {
        let mut screen = Screen::new(24, 80);
        screen.feed(b"\x1b[?1049hTUI");
        let fresh = replay(&mut screen, SCROLLBACK);
        assert!(fresh.screen().alternate_screen());
        assert!(fresh.screen().contents().contains("TUI"));
    }

    #[test]
    fn a_snapshot_keeps_the_input_modes() {
        let mut screen = Screen::new(24, 80);
        screen.feed(b"\x1b[?2004h\x1b[?1h");
        let fresh = replay(&mut screen, 0);
        assert!(fresh.screen().bracketed_paste());
        assert!(fresh.screen().application_cursor());
    }

    #[test]
    fn a_character_split_across_two_feeds_survives_the_snapshot() {
        let mut screen = Screen::new(24, 80);
        screen.feed(&[b'a', 0xC3]);
        screen.feed(&[0xA9]);
        let fresh = replay(&mut screen, 0);
        assert_eq!(fresh.screen().contents(), "aé");
    }

    #[test]
    fn the_last_lines_skip_blank_ones() {
        let mut screen = Screen::new(10, 80);
        screen.feed(b"one\r\n\r\ntwo  \r\n\x1b[31mthree\x1b[0m\r\nfour\r\n\r\n");
        assert_eq!(screen.last_lines(3), ["two", "three", "four"]);
        assert_eq!(screen.last_lines(9), ["one", "two", "three", "four"]);
        assert!(Screen::new(5, 20).last_lines(3).is_empty());
    }

    #[test]
    fn the_snapshot_leaves_the_live_view_at_the_bottom() {
        let mut screen = Screen::new(5, 80);
        screen.feed((0..20).map(|i| format!("l{i}")).collect::<Vec<_>>().join("\r\n").as_bytes());
        screen.snapshot(SCROLLBACK);
        assert_eq!(screen.parser.screen().scrollback(), 0);
    }
}
