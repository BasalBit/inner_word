//! Caret movement boundaries for single-line text: graphemes and words.
//!
//! All offsets are UTF-8 byte offsets into the text and always land on
//! grapheme boundaries. Words follow Unicode word segmentation (UAX #29), so
//! "don't" and "naïve" are single words and punctuation is skipped the way
//! macOS text fields skip it with `alt-left` / `alt-right`.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

/// Start of the grapheme before `offset` (0 at the start).
pub fn prev_grapheme(text: &str, offset: usize) -> usize {
    text.grapheme_indices(true)
        .rev()
        .find_map(|(ix, _)| (ix < offset).then_some(ix))
        .unwrap_or(0)
}

/// End of the grapheme after `offset` (`text.len()` at the end).
pub fn next_grapheme(text: &str, offset: usize) -> usize {
    text.grapheme_indices(true)
        .find_map(|(ix, _)| (ix > offset).then_some(ix))
        .unwrap_or(text.len())
}

/// Snaps `offset` to the closest grapheme boundary at or before it.
pub fn clamp_to_grapheme(text: &str, offset: usize) -> usize {
    let offset = offset.min(text.len());
    if offset == text.len() {
        return offset;
    }
    text.grapheme_indices(true)
        .take_while(|(ix, _)| *ix <= offset)
        .last()
        .map(|(ix, _)| ix)
        .unwrap_or(0)
}

fn is_word(segment: &str) -> bool {
    segment.chars().any(|ch| ch.is_alphanumeric())
}

/// `alt-left`: the start of the word that contains `offset` or, if `offset`
/// is at a word start or between words, the start of the previous word.
pub fn prev_word_start(text: &str, offset: usize) -> usize {
    text.split_word_bound_indices()
        .rev()
        .find_map(|(start, segment)| (start < offset && is_word(segment)).then_some(start))
        .unwrap_or(0)
}

/// `alt-right`: the end of the word that contains `offset` or, if `offset`
/// is at a word end or between words, the end of the next word.
pub fn next_word_end(text: &str, offset: usize) -> usize {
    text.split_word_bound_indices()
        .find_map(|(start, segment)| {
            let end = start + segment.len();
            (end > offset && is_word(segment)).then_some(end)
        })
        .unwrap_or(text.len())
}

/// The word (or run of punctuation / whitespace) under `offset`, for
/// double-click selection. A click just after a word selects that word.
pub fn word_range_at(text: &str, offset: usize) -> Range<usize> {
    if text.is_empty() {
        return 0..0;
    }
    let segments: Vec<(usize, &str)> = text.split_word_bound_indices().collect();
    let position = segments
        .iter()
        .position(|(start, segment)| offset >= *start && offset < start + segment.len())
        .unwrap_or(segments.len() - 1);
    let (start, segment) = segments[position];
    if !is_word(segment) && offset == start && position > 0 {
        let (prev_start, prev) = segments[position - 1];
        if is_word(prev) {
            return prev_start..prev_start + prev.len();
        }
    }
    start..start + segment.len()
}

/// Replaces line breaks with spaces: a single-line field never contains
/// `\n` (pasted multi-line text becomes one line).
pub fn single_line(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains(['\n', '\r']) {
        let mut out = String::with_capacity(text.len());
        let mut chars = text.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    out.push(' ');
                }
                '\n' => out.push(' '),
                _ => out.push(ch),
            }
        }
        std::borrow::Cow::Owned(out)
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

/// UTF-16 offset to UTF-8 offset (clamped to the text).
pub fn utf16_to_utf8(text: &str, utf16: usize) -> usize {
    let mut utf8 = 0;
    let mut count = 0;
    for ch in text.chars() {
        if count >= utf16 {
            break;
        }
        count += ch.len_utf16();
        utf8 += ch.len_utf8();
    }
    utf8
}

/// UTF-8 offset to UTF-16 offset.
pub fn utf8_to_utf16(text: &str, utf8: usize) -> usize {
    let mut utf16 = 0;
    let mut count = 0;
    for ch in text.chars() {
        if count >= utf8 {
            break;
        }
        count += ch.len_utf8();
        utf16 += ch.len_utf16();
    }
    utf16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphemes() {
        let text = "ae\u{301}🇩🇪x";
        assert_eq!(next_grapheme(text, 0), 1);
        assert_eq!(next_grapheme(text, 1), 4); // e + combining acute
        assert_eq!(next_grapheme(text, 4), 12); // flag is two scalars
        assert_eq!(prev_grapheme(text, 12), 4);
        assert_eq!(prev_grapheme(text, 0), 0);
        assert_eq!(clamp_to_grapheme(text, 2), 1);
        assert_eq!(clamp_to_grapheme(text, 99), text.len());
    }

    #[test]
    fn words() {
        let text = "Hello, naïve world  don't";
        assert_eq!(next_word_end(text, 0), 5);
        assert_eq!(next_word_end(text, 5), 13); // skips ", "
        assert_eq!(prev_word_start(text, 13), 7);
        assert_eq!(prev_word_start(text, 10), 7); // inside a word
        assert_eq!(prev_word_start(text, 7), 0);
        assert_eq!(next_word_end(text, 19), text.len()); // don't is one word
        assert_eq!(prev_word_start(text, text.len()), 21);
        assert_eq!(prev_word_start("  ", 2), 0);
        assert_eq!(next_word_end("  ", 0), 2);
    }

    #[test]
    fn double_click_word() {
        let text = "one two, three";
        assert_eq!(word_range_at(text, 1), 0..3);
        assert_eq!(word_range_at(text, 3), 0..3); // just after "one"
        assert_eq!(word_range_at(text, 4), 4..7);
        assert_eq!(word_range_at(text, text.len()), 9..14);
        assert_eq!(word_range_at("", 0), 0..0);
    }

    #[test]
    fn utf16_round_trip() {
        let text = "a😀é";
        assert_eq!(utf8_to_utf16(text, 5), 3);
        assert_eq!(utf16_to_utf8(text, 3), 5);
        assert_eq!(utf16_to_utf8(text, 99), text.len());
    }

    #[test]
    fn newlines_become_spaces() {
        assert_eq!(single_line("a\r\nb\nc"), "a b c");
        assert!(matches!(single_line("abc"), std::borrow::Cow::Borrowed(_)));
    }
}
