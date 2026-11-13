//! Escaping of literal text for Markdown, and character classes.
//!
//! Text is escaped only where a CommonMark or GitHub parser would otherwise
//! read it as markup, so ordinary prose stays clean: `2 * 3`, `snake_case`,
//! `AT&T`, `a < b` and `C:\Users` are written as they are, while `*word*`,
//! `<b>`, `&amp;`, a leading `#`, `-`, `>` or `1.` get a backslash.
//!
//! Characters that are markup only together with a partner are left alone
//! when the block holds no partner: a single `*` or `_` (a footnote
//! asterisk, `_config.yml`), a `<` with no `>` after it anywhere, and
//! brackets that nothing follows (`[1]`, `[sic]`). A bracket pair becomes a
//! link through what comes after it, so the escape goes on the closing
//! bracket: `[a\](b)`, `[a\]: b`.

use unicode_properties::{GeneralCategoryGroup, UnicodeGeneralCategory};

/// Context of a block that decides some escapes. A character that can
/// only be markup together with a partner is left alone when the block has
/// no partner for it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TextContext {
    /// Escape `*`: the block writes emphasis delimiters or has more than
    /// one asterisk.
    pub stars: bool,
    /// Escape `_`: the block has more than one underscore.
    pub underscores: bool,
    /// Escape `~`: the block uses strikethrough or has more than one tilde.
    pub tildes: bool,
    /// Escape `<`: a `>` occurs somewhere in the block, so a tag, a comment
    /// or an autolink could be completed.
    pub angles: bool,
    /// Escape a `[` that starts the block: a code span in the block holds
    /// `]:`, which would complete a link reference definition (block
    /// structure is read before code spans).
    pub code_label: bool,
}

impl TextContext {
    /// The context for plain text that stands alone (no inline formatting).
    pub fn for_text(text: &str) -> TextContext {
        TextContext {
            stars: text.matches('*').count() > 1,
            underscores: {
                let chars: Vec<char> = text.chars().collect();
                let word = |i: Option<usize>| {
                    i.and_then(|i| chars.get(i))
                        .is_some_and(|c| c.is_alphanumeric())
                };
                (0..chars.len())
                    .filter(|&i| chars[i] == '_' && !(word(i.checked_sub(1)) && word(Some(i + 1))))
                    .count()
                    > 1
            },
            tildes: text.matches('~').count() > 1,
            angles: text.contains('>'),
            code_label: false,
        }
    }
}

/// Where a chunk of literal text sits.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ChunkPosition {
    /// The chunk starts the block.
    pub block_start: bool,
    /// The chunk starts a line of the block.
    pub line_start: bool,
    /// The chunk ends a line of the block.
    pub line_end: bool,
    /// The chunk is inside link text.
    pub in_link: bool,
    /// The chunk ends an ATX heading.
    pub heading_end: bool,
    /// A `[` follows the chunk (a link or a footnote reference), so a final
    /// `!` would turn it into an image and a final `]` into a reference.
    pub before_bracket: bool,
    /// A footnote reference precedes the chunk, so a leading `(` or `[`
    /// would turn it into a link.
    pub after_bracket: bool,
}

fn is_space(c: Option<char>) -> bool {
    matches!(c, Some(' ' | '\t'))
}

/// Whether an entity or numeric character reference starts at `chars[i]`
/// (which is `&`).
fn entity_at(chars: &[char], i: usize) -> bool {
    let rest = &chars[i + 1..];
    let (body, max): (&[char], usize) = match rest {
        ['#', 'x' | 'X', tail @ ..] => (tail, 6),
        ['#', tail @ ..] => (tail, 7),
        tail => (tail, 32),
    };
    let hex = matches!(rest, ['#', 'x' | 'X', ..]);
    let numeric = matches!(rest, ['#', ..]);
    let n = body
        .iter()
        .take_while(|c| {
            if hex {
                c.is_ascii_hexdigit()
            } else if numeric {
                c.is_ascii_digit()
            } else {
                c.is_ascii_alphanumeric()
            }
        })
        .count();
    n >= 1 && n <= max && body.get(n) == Some(&';')
}

/// The index of a character that needs a backslash because of what the
/// whole line would otherwise start: a heading, quote, list item, thematic
/// break, setext underline or table delimiter row.
fn line_start_escape(chars: &[char], line_end: bool) -> Option<usize> {
    let first = *chars.first()?;
    let after = |n: usize| chars.get(n).copied();
    let ends_marker = |n: usize| after(n).is_none() || is_space(after(n));
    match first {
        '>' => return Some(0),
        '#' => {
            let n = chars.iter().take_while(|c| **c == '#').count();
            if n <= 6 && ends_marker(n) {
                return Some(0);
            }
        }
        '-' | '+' | '*' if ends_marker(1) => return Some(0),
        '0'..='9' => {
            let n = chars.iter().take_while(|c| c.is_ascii_digit()).count();
            if n <= 9 && matches!(after(n), Some('.' | ')')) && ends_marker(n + 1) {
                return Some(n);
            }
        }
        _ => {}
    }
    if line_end {
        // A whole line of dashes or equals signs: a thematic break or the
        // underline of a setext heading.
        if first == '=' && chars.iter().all(|c| *c == '=') {
            return Some(0);
        }
        if first == '-' && chars.iter().all(|c| matches!(c, '-' | ' ' | '\t')) {
            return Some(0);
        }
        // A table delimiter row such as `|---|:-:|`.
        if chars
            .iter()
            .all(|c| matches!(c, '|' | ':' | '-' | ' ' | '\t'))
            && chars.contains(&'-')
            && chars.iter().any(|c| matches!(c, '|' | ':'))
        {
            return Some(0);
        }
    }
    None
}

/// Whether `chars` starts with what GitHub reads as a task marker at the
/// start of a list item: `[ ]`, `[x]` or `[X]` before a space or the end.
fn task_marker_at(chars: &[char]) -> bool {
    matches!(chars, ['[', mark, ']', rest @ ..]
        if (mark.is_whitespace() || matches!(mark, 'x' | 'X'))
            && rest.first().is_none_or(|c| c.is_whitespace()))
}

/// Escapes one chunk of literal text. A chunk is the text between two
/// pieces of markup (or line ends) inside a block; it never contains a line
/// break. Chunk edges that touch markup are escaped conservatively.
pub(crate) fn escape_text(text: &str, pos: ChunkPosition, ctx: TextContext) -> String {
    let chars: Vec<char> = text.chars().collect();
    let line_escape = if pos.line_start {
        line_start_escape(&chars, pos.line_end)
    } else {
        None
    };
    // The last `#` run of a heading would be read as its closing sequence.
    let heading_escape = if pos.heading_end && chars.last() == Some(&'#') {
        let run = chars.iter().rev().take_while(|c| **c == '#').count();
        let start = chars.len() - run;
        (start > 0 && is_space(Some(chars[start - 1]))).then_some(start)
    } else {
        None
    };

    let mut out = String::with_capacity(text.len() + 8);
    for (i, &c) in chars.iter().enumerate() {
        if c == '[' && i == 0 && pos.after_bracket {
            // A footnote reference directly followed by an escaped bracket
            // is not reliably read as a reference; followed by an entity
            // it is.
            out.push_str("&#91;");
            continue;
        }
        let prev = i.checked_sub(1).map(|p| chars[p]);
        let next = chars.get(i + 1).copied();
        let escape = line_escape == Some(i)
            || heading_escape == Some(i)
            || match c {
                '\\' => next.is_none_or(|n| n.is_ascii_punctuation()),
                '`' => true,
                '*' => ctx.stars && !(is_space(prev) && is_space(next)),
                '_' => {
                    ctx.underscores
                        && !(prev.is_some_and(char::is_alphanumeric)
                            && next.is_some_and(char::is_alphanumeric))
                }
                '~' => ctx.tildes,
                // A bracket pair is only markup with something after it:
                // `[a](b)`, `[a][b]`, `[a]: b`. The closing bracket takes
                // the escape there, so `[1]` and `[sic]` stay as they are.
                '[' => {
                    pos.in_link
                        || matches!(next, Some('^' | '['))
                        || (i == 0 && pos.block_start && (ctx.code_label || task_marker_at(&chars)))
                }
                ']' => {
                    pos.in_link
                        || matches!(next, Some('(' | '[' | ':'))
                        || (next.is_none() && pos.before_bracket)
                }
                // A tag, comment or autolink needs a `>` to end it and no
                // space after the `<`; some HTML blocks start with just
                // `<name` at the start of a line.
                '<' => {
                    (ctx.angles && !is_space(next))
                        || (i == 0
                            && pos.line_start
                            && next.is_none_or(|n| {
                                n.is_ascii_alphabetic() || matches!(n, '/' | '!' | '?')
                            }))
                }
                '&' => entity_at(&chars, i),
                '!' => next.is_none() && pos.before_bracket,
                '(' => i == 0 && pos.after_bracket,
                _ => false,
            };
        if escape {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Escapes text that stands alone on one line (a title, a comment body
/// line): the chunk is the whole line.
pub(crate) fn escape_line(text: &str, heading: bool) -> String {
    escape_line_in(text, heading, true, TextContext::for_text(text))
}

/// Like [`escape_line`] for one line of a longer text whose context is
/// `ctx`; `block_start` says whether the line starts its paragraph.
pub(crate) fn escape_line_in(
    text: &str,
    heading: bool,
    block_start: bool,
    ctx: TextContext,
) -> String {
    escape_text(
        text,
        ChunkPosition {
            block_start,
            line_start: true,
            line_end: true,
            in_link: false,
            heading_end: heading,
            before_bracket: false,
            after_bracket: false,
        },
        ctx,
    )
}

/// A link destination for `[text](destination)`.
pub(crate) fn link_destination(url: &str) -> String {
    let chars: Vec<char> = url.trim().chars().collect();
    let mut out = String::with_capacity(url.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '\\' | '(' | ')' | '<' | '>' => {
                out.push('\\');
                out.push(c);
            }
            '&' if entity_at(&chars, i) => out.push_str("\\&"),
            c if c.is_whitespace() || c.is_control() => {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).bytes() {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
            c => out.push(c),
        }
    }
    out
}

/// The description of an image as the text between `![` and `](`: one
/// line, with everything a parser would read as markup escaped, so that
/// the alternative text it reads is the description.
pub(crate) fn image_alt(description: &str) -> String {
    let line = description
        .split(['\n', '\r'])
        .collect::<Vec<_>>()
        .join(" ");
    let escaped = escape_text(
        &line,
        ChunkPosition {
            in_link: true,
            ..ChunkPosition::default()
        },
        TextContext::for_text(&line),
    );
    // `![^1](…)`: to some parsers a bracket that opens with a caret is a
    // footnote reference before it is anything else.
    if escaped.starts_with('^') {
        format!("\\{escaped}")
    } else {
        escaped
    }
}

/// The destination of an image: a reference (`image:<id>`) or the path of
/// a picture file. A path with a space in it is written in angle brackets,
/// which is the one way CommonMark has to keep the space; a parser gives
/// back the path as it is on disk.
pub(crate) fn image_destination(destination: &str) -> String {
    if !destination.is_empty() && !destination.contains(char::is_whitespace) {
        return link_destination(destination);
    }
    let chars: Vec<char> = destination.chars().collect();
    let mut out = String::with_capacity(destination.len() + 4);
    out.push('<');
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '\\' | '<' | '>' => {
                out.push('\\');
                out.push(c);
            }
            '&' if entity_at(&chars, i) => out.push_str("\\&"),
            // No line can end inside the brackets.
            c if c.is_control() => {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).bytes() {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('>');
    out
}

/// Whether `url` can be written as an autolink `<url>`: an absolute URI
/// with a scheme and no characters the autolink syntax forbids.
pub(crate) fn is_autolinkable(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once(':') else {
        return false;
    };
    (2..=32).contains(&scheme.len())
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
        && !rest.is_empty()
        && url
            .chars()
            .all(|c| !c.is_whitespace() && !c.is_control() && c != '<' && c != '>')
}

/// How a character counts for the flanking rules of emphasis delimiters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CharClass {
    /// Unicode whitespace, or the start or end of the line.
    Whitespace,
    /// Punctuation in every CommonMark version.
    Punctuation,
    /// A symbol (currency, maths, emoji): punctuation since CommonMark
    /// 0.31, an ordinary character before that (as on GitHub).
    Symbol,
    /// Anything else.
    Other,
}

/// The class of a character next to a delimiter run; `None` is a line edge.
pub(crate) fn char_class(c: Option<char>) -> CharClass {
    let Some(c) = c else {
        return CharClass::Whitespace;
    };
    if matches!(c, '\t' | '\n' | '\u{c}' | '\r') {
        return CharClass::Whitespace;
    }
    if c.is_ascii_punctuation() {
        return CharClass::Punctuation;
    }
    match c.general_category_group() {
        GeneralCategoryGroup::Separator
            if c.general_category() == unicode_properties::GeneralCategory::SpaceSeparator =>
        {
            CharClass::Whitespace
        }
        GeneralCategoryGroup::Punctuation => CharClass::Punctuation,
        GeneralCategoryGroup::Symbol => CharClass::Symbol,
        _ => CharClass::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mid(text: &str) -> String {
        // A chunk in the middle of a line, between two words.
        escape_text(text, ChunkPosition::default(), TextContext::for_text(text))
    }

    #[test]
    fn prose_stays_clean() {
        for s in [
            "Plain prose, with punctuation: done! (Really?) \"Yes\" - no.",
            "2 * 3 = 6 and a < b > c",
            "snake_case_name and file_name.rs",
            "AT&T, R&D and a & b",
            "C:\\Users\\me",
            "100% #1 hashtag #rust ~5 minutes 1.5 + 2 - 3",
            "“Smart quotes” — and… ellipses",
            "😀 שלום 日本語",
            "an email@example.com and a lone ] bracket",
            "a | b",
        ] {
            assert_eq!(escape_line(s, false), s, "{s}");
        }
    }

    #[test]
    fn inline_markup_is_escaped() {
        assert_eq!(mid("a *b* c"), "a \\*b\\* c");
        assert_eq!(mid("**bold**"), "\\*\\*bold\\*\\*");
        assert_eq!(
            mid("_em_ and __strong__"),
            "\\_em\\_ and \\_\\_strong\\_\\_"
        );
        assert_eq!(mid("`code`"), "\\`code\\`");
        assert_eq!(mid("[text](url)"), "[text\\](url)");
        assert_eq!(mid("![img](url)"), "![img\\](url)");
        assert_eq!(mid("[a][b] [c][]"), "[a\\][b] [c\\][]");
        assert_eq!(mid("[[wiki]]"), "\\[[wiki]]");
        assert_eq!(mid("a [ b"), "a [ b");
        // Brackets that nothing follows are not markup.
        assert_eq!(
            mid("as shown [1], [sic] and [...]"),
            "as shown [1], [sic] and [...]"
        );
        // One asterisk or underscore has no partner.
        assert_eq!(mid("price* and _config"), "price* and _config");
        assert_eq!(mid("5*3"), "5*3");
        assert_eq!(mid("5*3*2"), "5\\*3\\*2");
        // No `>` to close a tag or an autolink.
        assert_eq!(mid("if a<b then"), "if a<b then");
        assert_eq!(mid("a<b and c>d"), "a\\<b and c>d");
        assert_eq!(mid("<+a@b.c> <3 > 2"), "\\<+a@b.c> \\<3 > 2");
        assert_eq!(mid("a < b > c"), "a < b > c");
        assert_eq!(
            mid("<b>bold</b> <!-- c --> <?php"),
            "\\<b>bold\\</b> \\<!-- c --> \\<?php"
        );
        assert_eq!(mid("<https://x.y>"), "\\<https://x.y>");
        assert_eq!(
            mid("&amp; &#123; &#x1F600; &copy"),
            "\\&amp; \\&#123; \\&#x1F600; &copy"
        );
        assert_eq!(mid("a\\*b \\ c\\"), "a\\\\*b \\ c\\\\");
        assert_eq!(mid("a\\*b* \\ c\\"), "a\\\\\\*b\\* \\ c\\\\");
        assert_eq!(mid("~~no~~"), "\\~\\~no\\~\\~");
        assert_eq!(mid("~one"), "~one");
        assert_eq!(mid("x <"), "x <");
        assert_eq!(mid("x < >"), "x < >");
        assert_eq!(mid("> x <"), "> x \\<");
    }

    #[test]
    fn line_starts_are_escaped() {
        let line = |s: &str| escape_line(s, false);
        assert_eq!(line("# heading"), "\\# heading");
        assert_eq!(line("###### six"), "\\###### six");
        assert_eq!(line("####### seven"), "####### seven");
        assert_eq!(line("#"), "\\#");
        assert_eq!(line("#hashtag"), "#hashtag");
        assert_eq!(line("> quote"), "\\> quote");
        assert_eq!(line("- item"), "\\- item");
        assert_eq!(line("+ item"), "\\+ item");
        assert_eq!(line("* item"), "\\* item");
        assert_eq!(line("-"), "\\-");
        assert_eq!(line("-5 degrees"), "-5 degrees");
        assert_eq!(line("1. one"), "1\\. one");
        assert_eq!(line("12) twelve"), "12\\) twelve");
        assert_eq!(line("1986. A year"), "1986\\. A year");
        assert_eq!(line("1.5 litres"), "1.5 litres");
        assert_eq!(line("---"), "\\---");
        assert_eq!(line("- - -"), "\\- - -");
        assert_eq!(line("--   ---"), "\\--   ---");
        assert_eq!(line("--"), "\\--");
        assert_eq!(line("-- x"), "-- x");
        assert_eq!(line("***"), "\\*\\*\\*");
        assert_eq!(line("___"), "\\_\\_\\_");
        assert_eq!(line("==="), "\\===");
        assert_eq!(line("|---|:-:|"), "\\|---|:-:|");
        assert_eq!(line("--|--"), "\\--|--");
        assert_eq!(line("```rust"), "\\`\\`\\`rust");
        assert_eq!(line("~~~"), "\\~\\~\\~");
        assert_eq!(line("[ ] task"), "\\[ ] task");
        assert_eq!(line("[^1]: note"), "\\[^1\\]: note");
        assert_eq!(line("[label]: /url"), "[label\\]: /url");
        assert_eq!(line("[x] done"), "\\[x] done");
        assert_eq!(line("[x]done"), "[x]done");
        assert_eq!(line("<div>"), "\\<div>");
        assert_eq!(line("<div"), "\\<div");
        assert_eq!(line("<!-- x"), "\\<!-- x");
        assert_eq!(line("* one star"), "\\* one star");
        assert_eq!(line("*"), "\\*");
        assert_eq!(line("*note"), "*note");
        assert_eq!(mid("[x] done"), "[x] done");
        // The same text in the middle of a line needs nothing.
        assert_eq!(mid("# heading"), "# heading");
        assert_eq!(mid("1. one"), "1. one");
        assert_eq!(mid("> quote"), "> quote");
    }

    #[test]
    fn heading_closing_sequence() {
        assert_eq!(escape_line("C#", true), "C#");
        assert_eq!(escape_line("About #", true), "About \\#");
        assert_eq!(escape_line("About ##", true), "About \\##");
        assert_eq!(escape_line("#", true), "\\#");
        assert_eq!(escape_line("About #", false), "About #");
    }

    #[test]
    fn link_text_brackets() {
        let pos = ChunkPosition {
            in_link: true,
            ..Default::default()
        };
        assert_eq!(
            escape_text("a ] b [ c", pos, TextContext::default()),
            "a \\] b \\[ c"
        );
    }

    #[test]
    fn image_descriptions_and_destinations() {
        assert_eq!(image_alt("The harbour at dusk"), "The harbour at dusk");
        assert_eq!(image_alt(""), "");
        assert_eq!(image_alt("a ] b [ c"), "a \\] b \\[ c");
        assert_eq!(image_alt("*a* `b`"), "\\*a\\* \\`b\\`");
        assert_eq!(image_alt("5 * 3 & snake_case"), "5 * 3 & snake_case");
        assert_eq!(image_alt("two\nlines"), "two lines");
        assert_eq!(image_alt("^1"), "\\^1");
        assert_eq!(image_alt("a^1"), "a^1");
        assert_eq!(image_alt("ends \\"), "ends \\\\");

        assert_eq!(
            image_destination("image:3f2a9c1b5d6e7f80a1b2c3d4e5f60718"),
            "image:3f2a9c1b5d6e7f80a1b2c3d4e5f60718"
        );
        assert_eq!(
            image_destination("p/image-3f2a9c1b.png"),
            "p/image-3f2a9c1b.png"
        );
        assert_eq!(image_destination("p(1)/a.png"), "p\\(1\\)/a.png");
        assert_eq!(image_destination("my p/a.png"), "<my p/a.png>");
        assert_eq!(image_destination("my (p)/a.png"), "<my (p)/a.png>");
        assert_eq!(
            image_destination("a <b>\\ c/a.png"),
            "<a \\<b\\>\\\\ c/a.png>"
        );
        assert_eq!(image_destination("R&amp;D x/a.png"), "<R\\&amp;D x/a.png>");
        assert_eq!(image_destination("a\nb c/a.png"), "<a%0Ab c/a.png>");
        assert_eq!(image_destination("a\u{a0}b/a.png"), "<a\u{a0}b/a.png>");
        assert_eq!(image_destination(""), "<>");
    }

    #[test]
    fn destinations() {
        assert_eq!(
            link_destination("https://example.com/a?b=c&d=e"),
            "https://example.com/a?b=c&d=e"
        );
        assert_eq!(
            link_destination("https://en.wikipedia.org/wiki/A_(b)"),
            "https://en.wikipedia.org/wiki/A_\\(b\\)"
        );
        assert_eq!(link_destination("a b\tc"), "a%20b%09c");
        assert_eq!(link_destination("<x>\\"), "\\<x\\>\\\\");
        assert_eq!(link_destination("?a=1&amp;b"), "?a=1\\&amp;b");
        assert_eq!(link_destination(""), "");
        assert!(is_autolinkable("https://example.com/x_y"));
        assert!(is_autolinkable("mailto:me@example.com"));
        assert!(!is_autolinkable("example.com"));
        assert!(!is_autolinkable("https://a b"));
        assert!(!is_autolinkable("https://a<b"));
        assert!(!is_autolinkable("x:"));
    }

    #[test]
    fn classes() {
        assert_eq!(char_class(None), CharClass::Whitespace);
        assert_eq!(char_class(Some(' ')), CharClass::Whitespace);
        assert_eq!(char_class(Some('\u{a0}')), CharClass::Whitespace);
        assert_eq!(char_class(Some('\n')), CharClass::Whitespace);
        assert_eq!(char_class(Some('*')), CharClass::Punctuation);
        assert_eq!(char_class(Some('$')), CharClass::Punctuation);
        assert_eq!(char_class(Some('”')), CharClass::Punctuation);
        assert_eq!(char_class(Some('—')), CharClass::Punctuation);
        assert_eq!(char_class(Some('€')), CharClass::Symbol);
        assert_eq!(char_class(Some('😀')), CharClass::Symbol);
        assert_eq!(char_class(Some('a')), CharClass::Other);
        assert_eq!(char_class(Some('é')), CharClass::Other);
        assert_eq!(char_class(Some('日')), CharClass::Other);
        assert_eq!(char_class(Some('\u{200d}')), CharClass::Other);
    }
}
