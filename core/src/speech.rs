//! Her markdown as the words to say: Chat shows the markdown, the voice
//! reads only its prose.

use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd};

/// What the voice says for `text`: links by their label, a bare URL as
/// "a link", each paragraph, heading and list item as a sentence. Tables,
/// code blocks, images and HTML are left out; she says in her prose what
/// they show. Plain text comes back as it was.
pub fn from_markdown(text: &str) -> String {
    let mut out = String::new();
    // Inside a table, code block, image or HTML block: nothing is said.
    let mut skip = 0usize;
    let mut autolink = false;
    for event in Parser::new_ext(text, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS) {
        match event {
            Event::Start(Tag::Table(_) | Tag::CodeBlock(_) | Tag::Image { .. } | Tag::HtmlBlock) => skip += 1,
            Event::End(TagEnd::Table | TagEnd::CodeBlock | TagEnd::Image | TagEnd::HtmlBlock) => skip = skip.saturating_sub(1),
            _ if skip > 0 => {}
            Event::Start(Tag::Link { link_type: LinkType::Autolink | LinkType::Email, .. }) => {
                autolink = true;
                out.push_str("a link");
            }
            Event::End(TagEnd::Link) => autolink = false,
            Event::Text(t) | Event::Code(t) if !autolink => {
                let words: Vec<&str> = t.split(' ').map(|w| if is_url(w) { "a link" } else { w }).collect();
                out.push_str(&words.join(" "));
            }
            Event::SoftBreak | Event::HardBreak => out.push(' '),
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item) => end_sentence(&mut out),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_url(word: &str) -> bool {
    let w = word.to_ascii_lowercase();
    w.starts_with("http://") || w.starts_with("https://")
}

/// Closes what was said so far with a full stop unless it already ends a
/// sentence or a clause, then a space for the next one.
fn end_sentence(out: &mut String) {
    let trimmed = out.trim_end().len();
    out.truncate(trimmed);
    if out.is_empty() {
        return;
    }
    if !out.ends_with(['.', '!', '?', ':', ';', '…']) {
        out.push('.');
    }
    out.push(' ');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_lines_pass_through_unchanged() {
        for l in ["Sure. Focusing api-fix.", "Telling api_fix_v2: open the PR.", "Resuming 2 sessions, 1 here and 1 on Gnowee.", "Which one?"] {
            assert_eq!(from_markdown(l), l);
        }
    }
    #[test]
    fn a_link_is_read_as_its_label_and_a_bare_url_as_a_link() {
        assert_eq!(from_markdown("See [the PR](https://github.com/o/r/pull/1)."), "See the PR.");
        assert_eq!(from_markdown("It is at https://example.com/x now."), "It is at a link now.");
        assert_eq!(from_markdown("<https://example.com>"), "a link.");
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
    fn paragraphs_join_into_one_line() {
        assert_eq!(from_markdown("Sure.\n\nFocusing api-fix."), "Sure. Focusing api-fix.");
    }
}
