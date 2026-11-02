//! Her markdown as the words to say: Chat shows the markdown, the voice
//! reads only its prose.

use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd};

/// What the voice says for `text`: links by their label, a bare URL as
/// "a link", each paragraph, heading and list item as a sentence. Tables,
/// code blocks, images and HTML are left out; she says in her prose what
/// they show. Plain text comes back as it was.
pub fn from_markdown(text: &str) -> String {
    blocks(text).join(" ")
}

/// The longest piece `pieces` makes, in bytes. Local voice models garble
/// text much longer than this.
pub const PIECE_LIMIT: usize = 400;

/// What the voice says for `text`, in pieces of at most `PIECE_LIMIT`
/// bytes that joined with a space are `from_markdown(text)`. Whole
/// paragraphs, headings and list items are packed together while they fit;
/// a longer one is cut at a sentence end, else at a `;` `:` or `,`, else
/// between words. A single word longer than the limit stays whole.
pub fn pieces(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for block in blocks(text) {
        let parts = if block.len() > PIECE_LIMIT { split_long(&block, PIECE_LIMIT) } else { vec![block] };
        for part in parts {
            if current.is_empty() {
                current = part;
            } else if current.len() + 1 + part.len() <= PIECE_LIMIT {
                current.push(' ');
                current.push_str(&part);
            } else {
                out.push(std::mem::replace(&mut current, part));
            }
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Cuts one long block into parts of at most `limit` bytes: each part is the
/// longest prefix that ends at a sentence end, else at a `;` `:` or `,`
/// followed by a space, else just before a space. A word longer than the
/// limit is a part of its own.
fn split_long(block: &str, limit: usize) -> Vec<String> {
    let mut parts = Vec::new();
    let mut rest = block.trim();
    while rest.len() > limit {
        let mut sentence = None;
        let mut clause = None;
        let mut space = None;
        for (i, c) in rest.char_indices() {
            let end = i + c.len_utf8();
            if end > limit + 1 {
                break;
            }
            if c == ' ' && i <= limit {
                space = Some(i);
            } else if end <= limit && rest[end..].starts_with(' ') {
                if matches!(c, '.' | '!' | '?' | '…') {
                    sentence = Some(end);
                } else if matches!(c, ';' | ':' | ',') {
                    clause = Some(end);
                }
            }
        }
        let cut = sentence.or(clause).or(space).unwrap_or_else(|| rest.find(' ').unwrap_or(rest.len()));
        parts.push(rest[..cut].to_string());
        rest = rest[cut..].trim_start();
    }
    if !rest.is_empty() {
        parts.push(rest.to_string());
    }
    parts
}

/// The prose of `text`, one entry per paragraph, heading or list item, each
/// closed with a full stop unless it already ends a sentence or a clause. A
/// full stop added only to separate sentences is not kept on the last one.
fn blocks(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    // Inside a table, code block, image or HTML block: nothing is said.
    let mut skip = 0usize;
    let mut autolink = false;
    // Whether the last block got a full stop that was not in the text.
    let mut added_stop = false;
    for event in Parser::new_ext(text, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS) {
        match event {
            Event::Start(Tag::Table(_) | Tag::CodeBlock(_) | Tag::Image { .. } | Tag::HtmlBlock) => skip += 1,
            Event::End(TagEnd::Table | TagEnd::CodeBlock | TagEnd::Image | TagEnd::HtmlBlock) => skip = skip.saturating_sub(1),
            _ if skip > 0 => {}
            Event::Start(Tag::Link { link_type: LinkType::Autolink | LinkType::Email, .. }) => {
                autolink = true;
                current.push_str("a link");
            }
            Event::End(TagEnd::Link) => autolink = false,
            Event::Text(t) | Event::Code(t) if !autolink => {
                let words: Vec<String> = t.split(' ').map(link_word).collect();
                current.push_str(&words.join(" "));
            }
            Event::SoftBreak | Event::HardBreak => current.push(' '),
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item) => {
                added_stop = close_block(&mut current, &mut out).unwrap_or(added_stop);
            }
            _ => {}
        }
    }
    if let Some(added) = close_block(&mut current, &mut out) {
        added_stop = added;
    }
    if added_stop {
        if let Some(last) = out.last_mut() {
            last.pop();
        }
    }
    out
}

/// Moves what was said so far into `out` as one block with its words
/// separated by single spaces and a full stop at the end unless it already
/// ends a sentence or a clause. None when there was nothing to move, else
/// whether it added the full stop.
fn close_block(current: &mut String, out: &mut Vec<String>) -> Option<bool> {
    let mut block = current.split_whitespace().collect::<Vec<_>>().join(" ");
    current.clear();
    if block.is_empty() {
        return None;
    }
    let add = !block.ends_with(['.', '!', '?', ':', ';', '…']);
    if add {
        block.push('.');
    }
    out.push(block);
    Some(add)
}

/// Plain text made safe to put into markdown: every character markdown
/// could read as markup is escaped, so Chat shows it and the voice says it
/// as written ("use the <Select> component", "__init__").
pub fn escape(plain: &str) -> String {
    let mut out = String::with_capacity(plain.len());
    for c in plain.chars() {
        if matches!(c, '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '&' | '|' | '~' | '#' | '!') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A word holding a URL, as "a link" with the punctuation around the URL
/// kept: "(https://x.com/a)." is "(a link).".
fn link_word(word: &str) -> String {
    let lower = word.to_ascii_lowercase();
    let Some(start) = ["http://", "https://"].iter().filter_map(|p| lower.find(p)).min() else {
        return word.to_string();
    };
    let tail = word[start..].trim_end_matches(|c: char| matches!(c, ')' | ']' | '>' | '.' | ',' | ';' | ':' | '!' | '?' | '"' | '\'' | '…'));
    format!("{}a link{}", &word[..start], &word[start + tail.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_lines_pass_through_unchanged() {
        for l in ["Dormant", "Working on: refactor", "Sure. Focusing api-fix.", "Telling api_fix_v2: open the PR.", "Resuming 2 sessions, 1 here and 1 on Gnowee.", "Which one?"] {
            assert_eq!(from_markdown(l), l);
        }
    }
    #[test]
    fn a_link_is_read_as_its_label_and_a_bare_url_as_a_link() {
        assert_eq!(from_markdown("See [the PR](https://github.com/o/r/pull/1)."), "See the PR.");
        assert_eq!(from_markdown("It is at https://example.com/x now."), "It is at a link now.");
        assert_eq!(from_markdown("<https://example.com>"), "a link");
    }
    #[test]
    fn a_url_in_brackets_or_quotes_is_read_as_a_link() {
        assert_eq!(from_markdown("The PR (https://github.com/o/r/pull/12) is open."), "The PR (a link) is open.");
        assert_eq!(from_markdown("Open \"https://x.com/a\", then wait."), "Open \"a link\", then wait.");
    }
    #[test]
    fn emphasis_headings_and_inline_code_are_plain_words() {
        assert_eq!(from_markdown("## Done\n**All green**, and `main` is merged."), "Done. All green, and main is merged.");
    }
    #[test]
    fn list_items_become_sentences() {
        assert_eq!(from_markdown("Two left:\n- api-fix\n- docs, waiting?"), "Two left: api-fix. docs, waiting?");
    }
    #[test]
    fn tables_code_blocks_and_images_are_not_read() {
        let t = "The failing tests are in a table in the chat.\n\n| test | file |\n|---|---|\n| a | x.rs |\n\n```sh\ncargo test\n```\n![shot](https://x/y.png)";
        assert_eq!(from_markdown(t), "The failing tests are in a table in the chat.");
    }
    #[test]
    fn only_a_table_says_nothing() {
        assert_eq!(from_markdown("| a |\n|---|\n| 1 |"), "");
    }
    #[test]
    fn escaped_plain_text_is_read_and_shown_as_written() {
        for x in ["use the <Select> component", "rename __init__ to *main*", "support <T> generics & [x] | y ~z~ `c`", "a\\b"] {
            assert_eq!(from_markdown(&escape(x)), x);
        }
    }
    #[test]
    fn paragraphs_join_into_one_line() {
        assert_eq!(from_markdown("Sure.\n\nFocusing api-fix."), "Sure. Focusing api-fix.");
    }
    #[test]
    fn short_replies_are_one_piece() {
        assert_eq!(pieces("Sure.\n\nFocusing api-fix."), ["Sure. Focusing api-fix."]);
        assert_eq!(pieces("Two left:\n- api-fix\n- docs, waiting?"), ["Two left: api-fix. docs, waiting?"]);
    }

    #[test]
    fn paragraphs_become_pieces_when_together_they_pass_the_limit() {
        let a = "a".repeat(10) + &" word".repeat(50); // ~260 chars
        let b = "b".repeat(10) + &" word".repeat(50);
        let p = pieces(&format!("{a}\n\n{b}"));
        assert_eq!(p.len(), 2);
        assert!(p[0].starts_with("aaaa") && p[0].ends_with('.'), "an added stop ends a piece that is not last: {:?}", p[0]);
        assert!(p[1].starts_with("bbbb") && !p[1].ends_with('.'), "the last piece keeps from_markdown's ending");
    }

    #[test]
    fn a_long_paragraph_splits_at_sentence_ends() {
        let s = "This sentence is about forty characters. ".repeat(15); // ~615 chars, one paragraph
        let p = pieces(&s);
        assert!(p.len() >= 2);
        assert!(p.iter().all(|x| x.len() <= PIECE_LIMIT), "{p:?}");
        assert!(p.iter().all(|x| x.ends_with('.')), "each piece ends at a sentence end: {p:?}");
    }

    #[test]
    fn a_long_sentence_splits_at_a_clause_before_a_word() {
        let s = format!("{}, {}", "word ".repeat(60).trim(), "more ".repeat(30).trim()); // one sentence, ~450 chars
        let p = pieces(&s);
        assert!(p[0].ends_with(','), "{p:?}");
    }

    #[test]
    fn a_sentence_with_no_stops_splits_between_words() {
        let s = "word ".repeat(200);
        let p = pieces(&s);
        assert!(p.iter().all(|x| x.len() <= PIECE_LIMIT && !x.starts_with(' ') && !x.ends_with(' ')), "{p:?}");
        assert_eq!(p.join(" "), from_markdown(&s));
        let long_word = "x".repeat(PIECE_LIMIT + 5);
        assert_eq!(pieces(&long_word), std::slice::from_ref(&long_word), "a single word is never cut");
    }

    #[test]
    fn a_reply_with_no_prose_has_no_pieces() {
        assert!(pieces("| a |\n|---|\n| 1 |").is_empty());
        assert!(pieces("").is_empty());
    }

    #[test]
    fn pieces_join_back_into_what_from_markdown_says() {
        let long = "This sentence is about forty characters. ".repeat(15);
        let double = format!("{long}\n\n{long}");
        for t in ["## Done\n**All green**, and `main` is merged.", "Two left:\n- api-fix\n- docs, waiting?", "See [the PR](https://github.com/o/r/pull/1).", "Sure.\n\nFocusing api-fix.", long.as_str(), double.as_str()] {
            assert_eq!(pieces(t).join(" "), from_markdown(t), "{t}");
        }
    }
}
