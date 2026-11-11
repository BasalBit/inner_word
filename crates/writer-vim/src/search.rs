//! Search patterns and finding matches around the caret.

use writer_core::{Document, Position, SearchOptions, TextRange};

use crate::effect::SearchPattern;

/// Turns what was typed after `/` or `?` into a literal pattern.
///
/// Patterns are literal text, not regular expressions. A few pieces of Vim
/// syntax are understood:
///
/// - `\c` anywhere makes the search ignore case, `\C` makes it match case.
///   Without either the search ignores case unless the pattern contains an
///   upper-case letter (smartcase).
/// - `\<` at the start together with `\>` at the end match whole words only
///   (this is what `*` and `#` search for).
/// - `\\` is a backslash, `\/` a slash, `\?` a question mark, `\n` a soft
///   line break, `\t` a tab. A backslash before any other character is
///   dropped.
///
/// Returns `None` when nothing is left to search for.
pub(crate) fn parse_pattern(raw: &str) -> Option<SearchPattern> {
    let mut body = raw;
    let mut whole_word = false;
    if let Some(inner) = body.strip_prefix("\\<")
        && let Some(inner) = inner.strip_suffix("\\>")
    {
        body = inner;
        whole_word = true;
    }
    let mut text = String::with_capacity(body.len());
    let mut force_case: Option<bool> = None;
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            text.push(c);
            continue;
        }
        match chars.next() {
            Some('c') => force_case = Some(false),
            Some('C') => force_case = Some(true),
            Some('n') => text.push('\n'),
            Some('t') => text.push('\t'),
            Some(other) => text.push(other),
            None => text.push('\\'),
        }
    }
    if text.is_empty() {
        return None;
    }
    let case_sensitive = force_case.unwrap_or_else(|| text.chars().any(char::is_uppercase));
    Some(SearchPattern {
        text,
        options: SearchOptions {
            case_sensitive,
            whole_word,
        },
    })
}

/// A pattern that finds `text` literally (used by `*` and `#`).
pub(crate) fn literal_pattern(text: &str, whole_word: bool) -> SearchPattern {
    SearchPattern {
        text: text.to_string(),
        options: SearchOptions {
            case_sensitive: text.chars().any(char::is_uppercase),
            whole_word,
        },
    }
}

/// How a literal is written so that [`parse_pattern`] reads it back.
pub(crate) fn escape_literal(text: &str, whole_word: bool) -> String {
    let mut out = String::new();
    if whole_word {
        out.push_str("\\<");
    }
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '/' => out.push_str("\\/"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    if whole_word {
        out.push_str("\\>");
    }
    out
}

/// A found match and whether the search wrapped around the document end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Found {
    pub range: TextRange,
    pub wrapped: bool,
}

/// The next match that starts after `from` (forward) or before it
/// (backward), wrapping around the document.
pub(crate) fn find(
    doc: &Document,
    pat: &SearchPattern,
    from: Position,
    forward: bool,
) -> Option<Found> {
    let n = doc.block_count();
    let hit = |b: usize, r: std::ops::Range<usize>, wrapped: bool| Found {
        range: TextRange::in_block(b, r.start, r.end),
        wrapped,
    };
    if forward {
        let here = pat.matches_in_block(doc, from.block);
        if let Some(r) = here.iter().find(|r| r.start > from.offset) {
            return Some(hit(from.block, r.clone(), false));
        }
        for step in 1..n {
            let b = (from.block + step) % n;
            if let Some(r) = pat.matches_in_block(doc, b).into_iter().next() {
                return Some(hit(b, r, b < from.block));
            }
        }
        here.into_iter().next().map(|r| hit(from.block, r, true))
    } else {
        let here = pat.matches_in_block(doc, from.block);
        if let Some(r) = here.iter().rev().find(|r| r.start < from.offset) {
            return Some(hit(from.block, r.clone(), false));
        }
        for step in 1..n {
            let b = (from.block + n - step) % n;
            if let Some(r) = pat.matches_in_block(doc, b).into_iter().next_back() {
                return Some(hit(b, r, b > from.block));
            }
        }
        here.into_iter()
            .next_back()
            .map(|r| hit(from.block, r, true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use writer_core::Block;

    #[test]
    fn pattern_syntax() {
        let p = parse_pattern("foo").unwrap();
        assert!(!p.options.case_sensitive && !p.options.whole_word);
        let p = parse_pattern("Foo").unwrap();
        assert!(p.options.case_sensitive);
        let p = parse_pattern("Foo\\c").unwrap();
        assert!(!p.options.case_sensitive);
        assert_eq!(p.text, "Foo");
        let p = parse_pattern("\\Cfoo").unwrap();
        assert!(p.options.case_sensitive);
        let p = parse_pattern("\\<word\\>").unwrap();
        assert!(p.options.whole_word);
        assert_eq!(p.text, "word");
        let p = parse_pattern("a\\/b\\\\c\\n").unwrap();
        assert_eq!(p.text, "a/b\\c\n");
        assert!(parse_pattern("").is_none());
        assert!(parse_pattern("\\c").is_none());
        let raw = escape_literal("a/b\\c", true);
        let p = parse_pattern(&raw).unwrap();
        assert_eq!(p.text, "a/b\\c");
        assert!(p.options.whole_word);
    }

    #[test]
    fn find_wraps() {
        let d = Document::from_blocks(vec![
            Block::paragraph("one two one"),
            Block::paragraph("three"),
            Block::paragraph("one"),
        ]);
        let p = literal_pattern("one", false);
        let at = |b, o| Position::new(b, o);
        let f = find(&d, &p, at(0, 0), true).unwrap();
        assert_eq!((f.range.start, f.wrapped), (at(0, 8), false));
        let f = find(&d, &p, at(0, 8), true).unwrap();
        assert_eq!((f.range.start, f.wrapped), (at(2, 0), false));
        let f = find(&d, &p, at(2, 0), true).unwrap();
        assert_eq!((f.range.start, f.wrapped), (at(0, 0), true));
        let f = find(&d, &p, at(0, 0), false).unwrap();
        assert_eq!((f.range.start, f.wrapped), (at(2, 0), true));
        let f = find(&d, &p, at(2, 0), false).unwrap();
        assert_eq!((f.range.start, f.wrapped), (at(0, 8), false));
        assert!(find(&d, &literal_pattern("zzz", false), at(0, 0), true).is_none());
        // A single match under the caret is found again by wrapping.
        let f = find(&d, &literal_pattern("three", false), at(1, 0), true).unwrap();
        assert_eq!((f.range.start, f.wrapped), (at(1, 0), true));
    }
}
