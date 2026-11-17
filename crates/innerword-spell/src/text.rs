//! What of a block is checked, and how a long text is cut into pieces.

use std::borrow::Cow;
use std::ops::Range;

use innerword_core::Block;

/// The text of `block` as the spell service gets it, or `None` for a
/// block that is not checked at all (a code block, a divider, a page
/// break).
///
/// Inline code is masked: its bytes become spaces, so the offsets of
/// everything else stay what they are in the block, the code is never
/// marked and never reaches the checker (where an identifier in the
/// middle of a sentence would also confuse the detection of the
/// language).
pub(crate) fn checkable_text(block: &Block) -> Option<Cow<'_, str>> {
    if block.is_void() || block.kind().is_code() {
        return None;
    }
    let text = block.text();
    let mut masked: Option<String> = None;
    for (range, attrs) in block.spans() {
        if !attrs.code || range.is_empty() {
            continue;
        }
        let masked = masked.get_or_insert_with(|| text.to_string());
        // Spaces are one byte each: the text keeps its length and stays
        // valid UTF-8 (a span starts and ends on character boundaries).
        masked.replace_range(range.clone(), &" ".repeat(range.len()));
    }
    Some(match masked {
        Some(masked) => Cow::Owned(masked),
        None => Cow::Borrowed(text),
    })
}

/// Whether `text` has anything a spell checker could have an opinion on.
pub(crate) fn has_letters(text: &str) -> bool {
    text.chars().any(char::is_alphabetic)
}

/// Where the piece of `text` that starts at `start` ends: about `target`
/// bytes further on, at white space (so no word is cut), or at the end of
/// the text when not much more than that is left. Always on a character
/// boundary and always after `start` (unless `start` is the end).
pub(crate) fn piece_end(text: &str, start: usize, target: usize) -> usize {
    let len = text.len();
    if len.saturating_sub(start) <= target + target / 2 {
        return len;
    }
    let mut goal = start + target;
    while !text.is_char_boundary(goal) {
        goal += 1;
    }
    // The first white space from there on; a "word" that runs on for
    // another whole piece is cut where it stands.
    let limit = (goal + target).min(len);
    for (at, c) in text[goal..].char_indices() {
        if goal + at >= limit {
            break;
        }
        if c.is_whitespace() {
            return goal + at;
        }
    }
    let mut cut = limit;
    while !text.is_char_boundary(cut) {
        cut += 1;
    }
    cut
}

/// A stretch of `text` around `word` that is enough for the checker to
/// guess in context: the hard line the word is on, and of a very long
/// line at most `reach` bytes to either side (cut at white space).
pub(crate) fn window_around(text: &str, word: Range<usize>, reach: usize) -> Range<usize> {
    let line_start = text[..word.start].rfind('\n').map_or(0, |at| at + 1);
    let line_end = text[word.end..]
        .find('\n')
        .map_or(text.len(), |at| word.end + at);
    let mut start = line_start;
    if word.start - start > reach {
        let mut from = word.start - reach;
        while !text.is_char_boundary(from) {
            from += 1;
        }
        start = text[from..word.start]
            .char_indices()
            .find(|(_, c)| c.is_whitespace())
            .map_or(from, |(at, c)| from + at + c.len_utf8());
    }
    let mut end = line_end;
    if end - word.end > reach {
        let mut to = word.end + reach;
        while !text.is_char_boundary(to) {
            to -= 1;
        }
        end = text[word.end..to]
            .char_indices()
            .rev()
            .find(|(_, c)| c.is_whitespace())
            .map_or(to, |(at, _)| word.end + at);
    }
    start..end
}

/// The words of `text`: runs of letters, digits and combining marks,
/// with the apostrophes between two of them.
pub(crate) fn words(text: &str) -> Vec<Range<usize>> {
    let is_word = |c: char| c.is_alphanumeric() || ('\u{0300}'..='\u{036F}').contains(&c);
    let mut words = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        if !is_word(c) {
            continue;
        }
        let mut end = start + c.len_utf8();
        while let Some(&(at, c)) = chars.peek() {
            let apostrophe = (c == '\'' || c == '\u{2019}')
                && text[at + c.len_utf8()..]
                    .chars()
                    .next()
                    .is_some_and(is_word);
            if !is_word(c) && !apostrophe {
                break;
            }
            end = at + c.len_utf8();
            chars.next();
        }
        words.push(start..end);
    }
    words
}

/// The word of `text` that `offset` is in, or at an end of. `None` when
/// there is no word there. Only the hard line of `offset` is looked at.
pub(crate) fn word_at(text: &str, offset: usize) -> Option<Range<usize>> {
    if offset > text.len() || !text.is_char_boundary(offset) {
        return None;
    }
    let line_start = text[..offset].rfind('\n').map_or(0, |at| at + 1);
    let line_end = text[offset..]
        .find('\n')
        .map_or(text.len(), |at| offset + at);
    let at = offset - line_start;
    // At the seam of two words the one that starts there is meant.
    let words = words(&text[line_start..line_end]);
    words
        .iter()
        .find(|word| word.start <= at && at < word.end)
        .or_else(|| words.iter().find(|word| word.end == at))
        .map(|word| line_start + word.start..line_start + word.end)
}

/// The part of `old` and `new` that differs, when `new` replaces `old`:
/// `(prefix, suffix)`, the bytes they have in common at the start and at
/// the end (whole characters; the two never overlap). When `old` is not
/// empty, at least one of its characters is left between them, so a
/// replacement always has a character whose formatting it can take.
pub(crate) fn common_ends(old: &str, new: &str) -> (usize, usize) {
    let mut prefix = 0;
    for (a, b) in old.chars().zip(new.chars()) {
        if a != b {
            break;
        }
        prefix += a.len_utf8();
    }
    let mut suffix = 0;
    for (a, b) in old[prefix..].chars().rev().zip(new[prefix..].chars().rev()) {
        if a != b {
            break;
        }
        suffix += a.len_utf8();
    }
    if !old.is_empty() && prefix + suffix == old.len() {
        // Nothing of the old word would be replaced (the new one only
        // has more letters): give one character back, the last of the
        // prefix, or the first of the suffix when there is no prefix.
        if let Some(c) = old[..prefix].chars().next_back() {
            prefix -= c.len_utf8();
        } else if let Some(c) = old[old.len() - suffix..].chars().next() {
            suffix -= c.len_utf8();
        }
    }
    (prefix, suffix)
}
