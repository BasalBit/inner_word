//! Text helpers shared by the model, the editing engine and the UI.
//!
//! All offsets are UTF-8 byte offsets into a block's text. Every function
//! clamps out-of-range offsets to the text and rounds offsets that fall
//! inside a character down to its start, so none of them panic.

use std::borrow::Cow;
use std::ops::Range;

use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};

/// Clamps `offset` to `text.len()` and rounds it down to a char boundary.
pub fn floor_char_boundary(text: &str, offset: usize) -> usize {
    let mut o = offset.min(text.len());
    while o > 0 && !text.is_char_boundary(o) {
        o -= 1;
    }
    o
}

/// Clamps `offset` to `text.len()` and rounds it up to a char boundary.
pub fn ceil_char_boundary(text: &str, offset: usize) -> usize {
    let mut o = offset.min(text.len());
    while o < text.len() && !text.is_char_boundary(o) {
        o += 1;
    }
    o
}

/// Whether `offset` is a grapheme cluster boundary (0 and `len` always are).
pub fn is_grapheme_boundary(text: &str, offset: usize) -> bool {
    if offset > text.len() || !text.is_char_boundary(offset) {
        return false;
    }
    if offset == 0 || offset == text.len() {
        return true;
    }
    let mut cursor = GraphemeCursor::new(offset, text.len(), true);
    cursor.is_boundary(text, 0).unwrap_or(true)
}

/// The grapheme boundary after `offset` (or `text.len()`).
pub fn next_grapheme_boundary(text: &str, offset: usize) -> usize {
    let o = floor_char_boundary(text, offset);
    if o >= text.len() {
        return text.len();
    }
    let mut cursor = GraphemeCursor::new(o, text.len(), true);
    match cursor.next_boundary(text, 0) {
        Ok(Some(b)) => b,
        _ => text.len(),
    }
}

/// The grapheme boundary before `offset` (or 0).
pub fn prev_grapheme_boundary(text: &str, offset: usize) -> usize {
    let o = ceil_char_boundary(text, offset);
    if o == 0 {
        return 0;
    }
    let mut cursor = GraphemeCursor::new(o, text.len(), true);
    match cursor.prev_boundary(text, 0) {
        Ok(Some(b)) => b,
        _ => 0,
    }
}

/// Rounds `offset` down to the nearest grapheme boundary (clamped).
pub fn floor_grapheme_boundary(text: &str, offset: usize) -> usize {
    let o = floor_char_boundary(text, offset);
    if is_grapheme_boundary(text, o) {
        o
    } else {
        prev_grapheme_boundary(text, o)
    }
}

/// Rounds `offset` up to the nearest grapheme boundary (clamped).
pub fn ceil_grapheme_boundary(text: &str, offset: usize) -> usize {
    let o = floor_char_boundary(text, offset);
    if is_grapheme_boundary(text, o) {
        o
    } else {
        next_grapheme_boundary(text, o)
    }
}

/// Number of grapheme clusters in `text`.
pub fn grapheme_count(text: &str) -> usize {
    text.graphemes(true).count()
}

/// Converts a UTF-8 byte offset into a UTF-16 code unit offset.
///
/// An offset inside a character is rounded down to the character start.
pub fn utf8_to_utf16(text: &str, offset: usize) -> usize {
    let o = floor_char_boundary(text, offset);
    text[..o].chars().map(char::len_utf16).sum()
}

/// Converts a UTF-16 code unit offset into a UTF-8 byte offset.
///
/// Offsets past the end clamp to `text.len()`; an offset in the middle of a
/// surrogate pair rounds down to the start of that character.
pub fn utf16_to_utf8(text: &str, offset16: usize) -> usize {
    let mut units = 0usize;
    for (i, c) in text.char_indices() {
        let next = units + c.len_utf16();
        if next > offset16 {
            return i;
        }
        units = next;
    }
    text.len()
}

/// Converts a UTF-8 byte range into a UTF-16 range.
pub fn utf8_range_to_utf16(text: &str, range: Range<usize>) -> Range<usize> {
    utf8_to_utf16(text, range.start)..utf8_to_utf16(text, range.end)
}

/// Converts a UTF-16 range into a UTF-8 byte range.
pub fn utf16_range_to_utf8(text: &str, range: Range<usize>) -> Range<usize> {
    utf16_to_utf8(text, range.start)..utf16_to_utf8(text, range.end)
}

/// Whether a word segment (as produced by Unicode word segmentation)
/// counts as a word: it contains at least one alphanumeric character.
fn is_word_segment(seg: &str) -> bool {
    seg.chars().any(char::is_alphanumeric)
}

/// Ranges of the words of `text` (Unicode word segmentation, keeping only
/// segments with an alphanumeric character), in order.
pub fn word_ranges(text: &str) -> impl Iterator<Item = Range<usize>> + '_ {
    text.split_word_bound_indices()
        .filter(|(_, s)| is_word_segment(s))
        .map(|(i, s)| i..i + s.len())
}

/// macOS option-left inside one text: the start of the word that begins
/// before `offset` (skipping any whitespace and punctuation first).
/// Returns `None` when no word starts before `offset`.
///
/// Only the hard lines from the one containing `offset` back to the one
/// holding the result are segmented (a `\n` always separates words), so in
/// a long code block the cost does not depend on the block's length.
pub fn word_start_before(text: &str, offset: usize) -> Option<usize> {
    let o = floor_char_boundary(text, offset);
    let mut line_e = line_end(text, o);
    loop {
        let ls = line_start(text, line_e);
        let best = word_ranges(&text[ls..line_e])
            .map(|r| ls + r.start)
            .take_while(|s| *s < o)
            .last();
        if best.is_some() || ls == 0 {
            return best;
        }
        line_e = ls - 1;
    }
}

/// macOS option-right inside one text: the end of the word that ends after
/// `offset` (skipping any whitespace and punctuation first).
/// Returns `None` when no word ends after `offset`.
///
/// Segments one hard line at a time, like [`word_start_before`].
pub fn word_end_after(text: &str, offset: usize) -> Option<usize> {
    let o = floor_char_boundary(text, offset);
    let mut ls = line_start(text, o);
    loop {
        let le = line_end(text, ls);
        let found = word_ranges(&text[ls..le]).map(|r| ls + r.end).find(|e| *e > o);
        if found.is_some() || le >= text.len() {
            return found;
        }
        ls = le + 1;
    }
}

/// The word-segmentation segment containing the character at `offset`
/// (the last segment when `offset` is at the end). This is what a double
/// click selects: a word, a run of spaces, or a punctuation mark.
pub fn segment_at(text: &str, offset: usize) -> Range<usize> {
    if text.is_empty() {
        return 0..0;
    }
    let o = floor_char_boundary(text, offset);
    let mut last = 0..0;
    for (i, s) in text.split_word_bound_indices() {
        let r = i..i + s.len();
        if r.start <= o && o < r.end {
            return r;
        }
        last = r;
    }
    last
}

/// Start of the hard line (after the previous `\n`) containing `offset`.
pub fn line_start(text: &str, offset: usize) -> usize {
    let o = floor_char_boundary(text, offset);
    text[..o].rfind('\n').map_or(0, |i| i + 1)
}

/// End of the hard line (the next `\n`, or the end) containing `offset`.
pub fn line_end(text: &str, offset: usize) -> usize {
    let o = floor_char_boundary(text, offset);
    text[o..].find('\n').map_or(text.len(), |i| o + i)
}

/// Normalises text coming from input or the clipboard: `"\r\n"`, `"\r"`,
/// U+0085, U+2028 and U+2029 become `"\n"`; other control characters except
/// tab are removed. Returns the input unchanged (borrowed) when it is clean.
pub fn normalize_input(text: &str) -> Cow<'_, str> {
    if text.chars().all(|c| !needs_normalizing(c)) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match normalize_char(c) {
            CharFix::Keep => out.push(c),
            CharFix::Drop => {}
            CharFix::Newline => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    continue;
                }
                out.push('\n');
            }
        }
    }
    Cow::Owned(out)
}

fn needs_normalizing(c: char) -> bool {
    !matches!(normalize_char(c), CharFix::Keep) || c == '\r'
}

pub(crate) enum CharFix {
    Keep,
    Drop,
    Newline,
}

/// How a single character is treated by [`normalize_input`].
pub(crate) fn normalize_char(c: char) -> CharFix {
    match c {
        '\n' | '\t' => CharFix::Keep,
        '\r' | '\u{85}' | '\u{2028}' | '\u{2029}' => CharFix::Newline,
        c if c.is_control() => CharFix::Drop,
        _ => CharFix::Keep,
    }
}

/// Whether `text` is already normalised (see [`normalize_input`]).
pub fn is_normalized(text: &str) -> bool {
    text.chars().all(|c| matches!(normalize_char(c), CharFix::Keep))
}

/// Character classes used for undo grouping and word logic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CharClass {
    Space,
    Word,
    Punct,
}

pub(crate) fn char_class(c: char) -> CharClass {
    if c.is_whitespace() {
        CharClass::Space
    } else if c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else {
        CharClass::Punct
    }
}

/// The char ending at `offset`, if any.
pub(crate) fn char_before(text: &str, offset: usize) -> Option<char> {
    let o = floor_char_boundary(text, offset);
    text[..o].chars().next_back()
}

/// The char starting at `offset`, if any.
pub(crate) fn char_after(text: &str, offset: usize) -> Option<char> {
    let o = floor_char_boundary(text, offset);
    text[o..].chars().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAMILY: &str = "\u{1F469}\u{200D}\u{1F469}\u{200D}\u{1F467}"; // woman woman girl
    const E_ACUTE: &str = "e\u{301}";

    #[test]
    fn grapheme_navigation_multibyte() {
        let text = format!("a{FAMILY}{E_ACUTE}中");
        let a_end = 1;
        let fam_end = a_end + FAMILY.len();
        let e_end = fam_end + E_ACUTE.len();
        assert_eq!(next_grapheme_boundary(&text, 0), a_end);
        assert_eq!(next_grapheme_boundary(&text, a_end), fam_end);
        assert_eq!(next_grapheme_boundary(&text, fam_end), e_end);
        assert_eq!(next_grapheme_boundary(&text, e_end), text.len());
        assert_eq!(next_grapheme_boundary(&text, text.len()), text.len());
        assert_eq!(prev_grapheme_boundary(&text, text.len()), e_end);
        assert_eq!(prev_grapheme_boundary(&text, e_end), fam_end);
        assert_eq!(prev_grapheme_boundary(&text, fam_end), a_end);
        assert_eq!(prev_grapheme_boundary(&text, 0), 0);
        assert!(!is_grapheme_boundary(&text, a_end + 4));
        assert_eq!(floor_grapheme_boundary(&text, a_end + 5), a_end);
        assert_eq!(ceil_grapheme_boundary(&text, a_end + 5), fam_end);
        assert_eq!(grapheme_count(&text), 4);
        // Out of range never panics.
        assert_eq!(next_grapheme_boundary(&text, 999), text.len());
        assert_eq!(prev_grapheme_boundary(&text, 999), e_end);
    }

    #[test]
    fn utf16_conversions() {
        let text = format!("a{FAMILY}b");
        // family: 3 astral chars (2 units each) + 2 ZWJ (1 unit each) = 8 units
        assert_eq!(utf8_to_utf16(&text, 0), 0);
        assert_eq!(utf8_to_utf16(&text, 1), 1);
        assert_eq!(utf8_to_utf16(&text, 1 + FAMILY.len()), 9);
        assert_eq!(utf8_to_utf16(&text, text.len()), 10);
        assert_eq!(utf16_to_utf8(&text, 9), 1 + FAMILY.len());
        assert_eq!(utf16_to_utf8(&text, 2), 1); // middle of a surrogate pair
        assert_eq!(utf16_to_utf8(&text, 100), text.len());
        assert_eq!(utf8_to_utf16(&text, 2), 1); // inside a char rounds down
        let r = utf8_range_to_utf16(&text, 1..text.len());
        assert_eq!(utf16_range_to_utf8(&text, r), 1..text.len());
    }

    #[test]
    fn words_and_segments() {
        let text = "Hello, world! don't stop";
        assert_eq!(word_end_after(text, 0), Some(5));
        assert_eq!(word_end_after(text, 5), Some(12));
        assert_eq!(word_start_before(text, 12), Some(7));
        assert_eq!(word_start_before(text, 7), Some(0));
        assert_eq!(word_start_before(text, 0), None);
        assert_eq!(word_end_after(text, 14), Some(19)); // "don't" is one word
        assert_eq!(word_end_after(text, text.len()), None);
        assert_eq!(segment_at(text, 8), 7..12);
        assert_eq!(segment_at(text, 5), 5..6);
        assert_eq!(segment_at(text, text.len()), 20..24);
        assert_eq!(segment_at("", 0), 0..0);
        let cjk = "中文字";
        assert!(word_end_after(cjk, 0).is_some());
        let rtl = "שלום עולם";
        assert_eq!(word_end_after(rtl, 0), Some("שלום".len()));
    }

    #[test]
    fn lines() {
        let t = "ab\ncd\n";
        assert_eq!(line_start(t, 4), 3);
        assert_eq!(line_end(t, 4), 5);
        assert_eq!(line_start(t, 6), 6);
        assert_eq!(line_end(t, 6), 6);
        assert_eq!(line_end(t, 0), 2);
    }

    #[test]
    fn normalization() {
        assert!(matches!(normalize_input("plain\ttext\n"), Cow::Borrowed(_)));
        assert_eq!(normalize_input("a\r\nb\rc\u{2028}d"), "a\nb\nc\nd");
        assert_eq!(normalize_input("a\u{0}b\u{7}c\u{1b}"), "abc");
        assert_eq!(normalize_input("\r\n\r\n"), "\n\n");
        assert!(is_normalized("x\ty\nz"));
        assert!(!is_normalized("x\r"));
    }
}
