//! Line breaking: where the rows of a hard line end.
//!
//! GPUI's own wrapping (`shape_text` with a wrap width) allows a break
//! before every character it does not count as part of a word, which is
//! everything outside Latin and Cyrillic letters, digits and a dozen ASCII
//! signs. A line could then start with `!`, `?`, `;`, `)` or a closing
//! quote, an apostrophe could be parted from its word, and Greek, Hebrew
//! or Arabic words broke after any letter. The editor therefore asks GPUI
//! for unwrapped lines and breaks them here.
//!
//! The rules, in the spirit of the Unicode line breaking algorithm but only
//! as much of it as prose needs:
//!
//! - A row breaks after white space: the spaces stay at the end of the row
//!   and hang past the edge of the text box, the next word starts the next
//!   row. No-break spaces do not break.
//! - Never before closing punctuation (`! ? ; : , . ) ] }`, closing quotes,
//!   the ellipsis, `%`), not even after a space (French puts one before
//!   `!` and `?`), and never right after an opening bracket or quote.
//! - After a hyphen or a dash inside a word (`well-known`, `this—that`).
//! - Between ideographs, kana and Hangul, which are written without
//!   spaces; after a zero-width space.
//! - A word that is wider than the text box is cut where it reaches the
//!   edge, between two characters.
//! - The indentation at the start of a hard line is not a place to break.
//!
//! Wrapping is greedy and looks only at the row it is filling: a slice of
//! the text that starts at a row start breaks into the same rows as the
//! whole text does. Shaping long blocks in pieces depends on that (see the
//! parent module).
//!
//! Inline code is drawn smaller than it is shaped (see
//! [`INLINE_CODE_SCALE`]) and sits in a chip with a little padding, so the
//! width a row takes is not the sum of its shaped advances. The breaks are
//! computed with the widths that are drawn.

use gpui::{Pixels, px};
use writer_core::text;

/// Inline code is drawn at this fraction of the block's text size: the
/// size a code block has next to body text. A monospace face has a larger
/// x-height than a book face; at the same size it looks a size too big.
pub(crate) const INLINE_CODE_SCALE: f32 = 0.88;
/// Space between the edge of an inline code chip and its text, as a
/// fraction of the block's text size.
pub(crate) const INLINE_CODE_PAD: f32 = 0.16;

/// One glyph of an unwrapped line, as the wrapping needs it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WrapGlyph {
    /// Byte offset in the line's text.
    pub(crate) index: usize,
    /// Shaped x of the glyph's origin in the unwrapped line.
    pub(crate) x: Pixels,
    /// The glyph is inline code.
    pub(crate) code: bool,
}

/// White space a row may end at. No-break spaces are not.
fn is_break_space(c: char) -> bool {
    c == ' '
        || c == '\t'
        || (c.is_whitespace() && !matches!(c, '\u{00A0}' | '\u{202F}' | '\u{2007}' | '\u{2060}'))
}

/// Characters a row never starts with.
fn is_closing(c: char) -> bool {
    matches!(
        c,
        '!' | '?'
            | ';'
            | ':'
            | ','
            | '.'
            | ')'
            | ']'
            | '}'
            | '%'
            | '\u{2030}' // per mille
            | '\u{00B0}' // degree
            | '\u{00BB}' // »
            | '\u{201D}' // ”
            | '\u{2019}' // ’ (also the apostrophe)
            | '\u{203A}' // ›
            | '\u{2026}' // …
            | '\u{3001}' // 、
            | '\u{3002}' // 。
            | '\u{FF0C}' // ，
            | '\u{FF0E}' // ．
            | '\u{FF01}' // ！
            | '\u{FF1F}' // ？
            | '\u{FF1A}' // ：
            | '\u{FF1B}' // ；
            | '\u{FF09}' // ）
            | '\u{300D}' // 」
            | '\u{300F}' // 』
            | '\u{3011}' // 】
            | '\u{3009}' // 〉
            | '\u{300B}' // 》
            | '\u{30FC}' // ー
            | '\u{3005}' // 々
    )
}

/// Characters a row never ends with.
fn is_opening(c: char) -> bool {
    matches!(
        c,
        '(' | '['
            | '{'
            | '\u{00AB}' // «
            | '\u{201C}' // “
            | '\u{2018}' // ‘
            | '\u{2039}' // ‹
            | '\u{00BF}' // ¿
            | '\u{00A1}' // ¡
            | '\u{FF08}' // （
            | '\u{300C}' // 「
            | '\u{300E}' // 『
            | '\u{3010}' // 【
            | '\u{3008}' // 〈
            | '\u{300A}' // 《
    )
}

/// Scripts written without spaces between words, where a row may end
/// after any character.
fn is_ideographic(c: char) -> bool {
    matches!(
        c as u32,
        0x2E80..=0x303F      // CJK radicals, symbols and punctuation
            | 0x3041..=0x30FF // hiragana, katakana
            | 0x3400..=0x4DBF // CJK extension A
            | 0x4E00..=0x9FFF // CJK unified ideographs
            | 0xAC00..=0xD7AF // Hangul syllables
            | 0xF900..=0xFAFF // CJK compatibility ideographs
            | 0xFF00..=0xFF60 // fullwidth forms
            | 0xFFE0..=0xFFE6
            | 0x20000..=0x3FFFF // CJK extensions B and later
    )
}

fn is_hyphen(c: char) -> bool {
    matches!(c, '-' | '\u{2010}' | '\u{2013}' | '\u{2014}')
}

/// Whether a row may end between `before` and `after` (the row's next
/// character). `before2` is the character ahead of `before`, when it is on
/// the same row.
pub(crate) fn can_break(before2: Option<char>, before: char, after: char) -> bool {
    if is_break_space(after) || is_closing(after) || is_opening(before) {
        return false;
    }
    if is_break_space(before) || before == '\u{200B}' {
        return true;
    }
    if is_hyphen(before) {
        // Inside a word, not after a minus sign or a leading dash.
        return before2.is_some_and(char::is_alphanumeric) && after.is_alphanumeric();
    }
    is_ideographic(before) || is_ideographic(after)
}

/// The glyphs at which the rows after the first one start: indexes into
/// `glyphs`, ascending. `line_width` is the shaped width of the whole
/// line (the x after its last glyph); `pad` is the padding inside an
/// inline code chip.
pub(crate) fn wrap_line(
    line: &str,
    glyphs: &[WrapGlyph],
    line_width: Pixels,
    wrap_width: Pixels,
    pad: Pixels,
) -> Vec<usize> {
    let mut breaks = Vec::new();
    // Nothing to do for the common short line (drawn widths are never
    // much larger than shaped ones: a chip's padding against its smaller
    // text).
    let chips = glyphs
        .windows(2)
        .filter(|pair| pair[0].code != pair[1].code)
        .count()
        + 2;
    if glyphs.is_empty() || line_width + pad * chips as f32 <= wrap_width {
        return breaks;
    }
    let char_at = |glyph: usize| line[glyphs[glyph].index..].chars().next().unwrap_or(' ');
    let advance =
        |glyph: usize| glyphs.get(glyph + 1).map_or(line_width, |next| next.x) - glyphs[glyph].x;

    let mut row_start = 0;
    // The first row of the line may begin with indentation; rows after a
    // break begin with a word.
    let mut seen_word = false;
    let mut candidate: Option<usize> = None;
    let mut width = px(0.);
    let mut at = 0;
    while at < glyphs.len() {
        let glyph = glyphs[at];
        let c = char_at(at);
        let space = is_break_space(c);
        if at > row_start && seen_word && !space {
            // (Glyphs of one character, a base and its marks, never part.)
            let before = line[..glyph.index].chars().next_back();
            let before_index = glyph.index - before.map_or(0, char::len_utf8);
            let before2 = (before_index > glyphs[row_start].index)
                .then(|| line[..before_index].chars().next_back())
                .flatten();
            if glyph.index > glyphs[at - 1].index
                && before.is_some_and(|before| can_break(before2, before, c))
            {
                candidate = Some(at);
            }
        }
        if !space {
            seen_word = true;
        }
        // The width the row has drawn through this glyph.
        let opens = glyph.code && (at == row_start || !glyphs[at - 1].code);
        let mut drawn = advance(at);
        if glyph.code {
            drawn *= INLINE_CODE_SCALE;
        }
        if opens {
            drawn += pad;
        }
        let closing = if glyph.code { pad } else { px(0.) };
        if !space && at > row_start && width + drawn + closing > wrap_width {
            // This glyph does not fit: the row ends at the last place it
            // may, or right here inside a word that is too long.
            let cut = match candidate.take() {
                Some(candidate) => candidate,
                None if text::floor_grapheme_boundary(line, glyph.index) == glyph.index
                    && glyph.index > glyphs[at - 1].index =>
                {
                    at
                }
                None => {
                    // Inside a character: it stays whole on this row.
                    width += drawn;
                    at += 1;
                    continue;
                }
            };
            breaks.push(cut);
            row_start = cut;
            seen_word = false;
            width = px(0.);
            at = cut;
            continue;
        }
        width += drawn;
        if glyph.code && glyphs.get(at + 1).is_none_or(|next| !next.code) {
            width += pad;
        }
        at += 1;
    }
    breaks
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character 10 wide; `code` is a byte range of inline code.
    fn glyphs(line: &str, code: Option<std::ops::Range<usize>>) -> Vec<WrapGlyph> {
        line.char_indices()
            .enumerate()
            .map(|(ix, (index, _))| WrapGlyph {
                index,
                x: px(10. * ix as f32),
                code: code.as_ref().is_some_and(|range| range.contains(&index)),
            })
            .collect()
    }

    fn rows_with(
        line: &str,
        width: f32,
        code: Option<std::ops::Range<usize>>,
        pad: f32,
    ) -> Vec<String> {
        let glyphs = glyphs(line, code);
        let line_width = px(10. * glyphs.len() as f32);
        let breaks = wrap_line(line, &glyphs, line_width, px(width), px(pad));
        let mut starts: Vec<usize> = vec![0];
        starts.extend(breaks.iter().map(|glyph| glyphs[*glyph].index));
        starts.push(line.len());
        starts
            .windows(2)
            .map(|pair| line[pair[0]..pair[1]].to_string())
            .collect()
    }

    fn rows(line: &str, width: f32) -> Vec<String> {
        rows_with(line, width, None, 0.)
    }

    #[test]
    fn rows_break_after_spaces_and_spaces_hang() {
        assert_eq!(rows("one two three", 70.), ["one two ", "three"]);
        // A word that fits keeps its place although the space after it
        // does not fit.
        assert_eq!(rows("one two three", 79.), ["one two ", "three"]);
        assert_eq!(rows("abcdefg hi", 70.), ["abcdefg ", "hi"]);
        // Several spaces hang together.
        assert_eq!(rows("abcdefg    hi", 70.), ["abcdefg    ", "hi"]);
        // Nothing to do when everything fits.
        assert_eq!(rows("one two", 70.), ["one two"]);
        assert_eq!(rows("", 70.), [""]);
    }

    #[test]
    fn punctuation_stays_with_its_word() {
        // GPUI would give "long" / "! Could".
        assert_eq!(
            rows("many stranger long! Could", 180.),
            ["many stranger ", "long! Could"]
        );
        // GPUI would give "not?" / the closing quote alone.
        assert_eq!(
            rows("say \u{201c}why not?\u{201d} and", 130.),
            ["say \u{201c}why ", "not?\u{201d} and"]
        );
        assert_eq!(rows("one (two) three", 90.), ["one (two) ", "three"]);
        assert_eq!(
            rows("aaaa don\u{2019}t go", 80.),
            ["aaaa ", "don\u{2019}t go"]
        );
        assert_eq!(rows("wait\u{2026} what", 50.), ["wait\u{2026} ", "what"]);
        // French spacing: the mark follows its word over the space.
        assert_eq!(rows("Bonjour ! Oui", 90.), ["Bonjour ! ", "Oui"]);
        assert_eq!(rows("aaaa bb ? cc", 70.), ["aaaa ", "bb ? cc"]);
        // A no-break space does not break.
        assert_eq!(
            rows("aaa bb\u{00a0}cc dd", 80.),
            ["aaa ", "bb\u{00a0}cc dd"]
        );
    }

    #[test]
    fn words_of_every_script_stay_whole() {
        let greek = "\u{03b1}\u{03b2}\u{03b3} \u{03b4}\u{03b5}\u{03b6}\u{03b7} \u{03b8}";
        assert_eq!(
            rows(greek, 60.),
            [
                "\u{03b1}\u{03b2}\u{03b3} ",
                "\u{03b4}\u{03b5}\u{03b6}\u{03b7} \u{03b8}"
            ]
        );
        let hebrew = "\u{05d0}\u{05d1}\u{05d2} \u{05d3}\u{05d4}\u{05d5}\u{05d6}";
        assert_eq!(
            rows(hebrew, 60.),
            [
                "\u{05d0}\u{05d1}\u{05d2} ",
                "\u{05d3}\u{05d4}\u{05d5}\u{05d6}"
            ]
        );
        // Ideographs break anywhere, but not before closing punctuation
        // or after an opening bracket.
        let cjk = "\u{65e5}\u{672c}\u{8a9e}\u{3002}\u{300c}\u{65e5}\u{672c}\u{300d}";
        assert_eq!(
            rows(cjk, 30.),
            [
                "\u{65e5}\u{672c}",
                "\u{8a9e}\u{3002}",
                "\u{300c}\u{65e5}",
                "\u{672c}\u{300d}"
            ]
        );
    }

    #[test]
    fn hyphens_and_long_words() {
        assert_eq!(
            rows("a well-known fact", 70.),
            ["a well-", "known ", "fact"]
        );
        // Not after a minus sign or a leading dash.
        assert_eq!(
            rows("aaaa -5 or --flag", 60.),
            ["aaaa ", "-5 or ", "--flag"]
        );
        assert_eq!(rows("this\u{2014}that", 60.), ["this\u{2014}", "that"]);
        // A word wider than the box is cut at the edge.
        assert_eq!(rows("abcdefghijkl", 50.), ["abcde", "fghij", "kl"]);
        assert_eq!(rows("ab cdefghijkl", 50.), ["ab ", "cdefg", "hijkl"]);
        // Indentation is not a place to break.
        assert_eq!(rows("    abcdefgh", 50.), ["    a", "bcdef", "gh"]);
        assert_eq!(rows("  ab cd", 40.), ["  ab ", "cd"]);
    }

    #[test]
    fn a_slice_from_a_row_start_wraps_like_the_whole() {
        let text = "The quick brown fox (jumps) over the lazy dog\u{2014}twice! And \
                    then a-b-c, well-known \u{201c}quotes\u{201d} follow.    Wide   gaps ? Yes";
        for width in [40., 70., 110., 150.] {
            let whole = rows(text, width);
            let mut start = 0;
            for row in &whole {
                let rest = rows(&text[start..], width);
                assert_eq!(&rest[0], row, "width {width}, from {start}");
                start += row.len();
            }
        }
    }

    #[test]
    fn inline_code_is_measured_as_it_is_drawn() {
        // "aa `bbbbbbbb` cc": the code is 8 glyphs, shaped 80 wide, drawn
        // at 0.88 with 2 of padding on each side: 74.4.
        let line = "aa bbbbbbbb cc";
        let code = Some(3..11);
        // Shaped, "aa bbbbbbbb" is 110 wide and would not fit in 108;
        // drawn it is 30 + 74.4.
        assert_eq!(rows_with(line, 108., None, 0.), ["aa ", "bbbbbbbb ", "cc"]);
        assert_eq!(
            rows_with(line, 108., code.clone(), 2.),
            ["aa bbbbbbbb ", "cc"]
        );
        assert_eq!(
            rows_with(line, 104., code.clone(), 2.),
            ["aa ", "bbbbbbbb ", "cc"]
        );
        // A one-letter chip is wider than its letter: 8.8 + 4.
        assert_eq!(rows_with("aaaa b", 62., None, 0.), ["aaaa b"]);
        assert_eq!(rows_with("aaaa b", 62., Some(5..6), 2.), ["aaaa ", "b"]);
        assert_eq!(rows_with("aaaa b", 63., Some(5..6), 2.), ["aaaa b"]);
        // Code that wraps inside has a chip on each row.
        assert_eq!(
            rows_with("bbbb bbbb", 50., Some(0..9), 2.),
            ["bbbb ", "bbbb"]
        );
    }
}
