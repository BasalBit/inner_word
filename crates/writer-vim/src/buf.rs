//! Reading the document the way Vim reads a buffer.
//!
//! The document is seen as a sequence of *cells*. Every grapheme of a
//! block's text is a cell; a soft line break (`\n` inside a block) is a
//! newline cell; and the end of every block (offset `len`) is a newline cell
//! too, as if each block ended with a line break. A void block (divider,
//! page break) is an empty line: one newline cell at offset 0.
//!
//! A *hard line* is the text between two newline cells: a whole block, or a
//! segment between soft line breaks. A *line unit* is what linewise commands
//! count: a whole block, except inside a code block, where every hard line
//! is a unit.

use std::ops::Range;

use writer_core::{Document, Position, text};

/// The text of block `b`.
pub(crate) fn text_of(doc: &Document, b: usize) -> &str {
    doc.blocks()[b].text()
}

/// Length of block `b` in bytes.
pub(crate) fn len_of(doc: &Document, b: usize) -> usize {
    doc.blocks()[b].len()
}

/// The grapheme boundary after `o` (ASCII fast path).
pub(crate) fn next_boundary(t: &str, o: usize) -> usize {
    let b = t.as_bytes();
    if o + 1 < b.len() && b[o] < 0x80 && b[o + 1] < 0x80 {
        return o + 1;
    }
    text::next_grapheme_boundary(t, o)
}

/// The grapheme boundary before `o` (ASCII fast path).
pub(crate) fn prev_boundary(t: &str, o: usize) -> usize {
    let b = t.as_bytes();
    if o >= 2 && o <= b.len() && b[o - 1] < 0x80 && b[o - 2] < 0x80 {
        return o - 1;
    }
    text::prev_grapheme_boundary(t, o)
}

/// Start of the hard line containing `p`.
pub(crate) fn line_start(doc: &Document, p: Position) -> usize {
    text::line_start(text_of(doc, p.block), p.offset)
}

/// End of the hard line containing `p` (the offset of its newline cell).
pub(crate) fn line_end(doc: &Document, p: Position) -> usize {
    text::line_end(text_of(doc, p.block), p.offset)
}

/// Whether `a` and `b` are on the same hard line.
pub(crate) fn same_line(doc: &Document, a: Position, b: Position) -> bool {
    a.block == b.block && line_start(doc, a) == line_start(doc, b)
}

/// Whether the cell at `p` is a newline cell.
pub(crate) fn is_newline(doc: &Document, p: Position) -> bool {
    let t = text_of(doc, p.block);
    p.offset >= t.len() || t.as_bytes()[p.offset] == b'\n'
}

/// Whether `p` is the single cell of an empty hard line.
pub(crate) fn is_empty_line(doc: &Document, p: Position) -> bool {
    is_newline(doc, p) && line_start(doc, p) == p.offset
}

/// Whether `p` is the first cell of its hard line.
pub(crate) fn at_line_start(doc: &Document, p: Position) -> bool {
    line_start(doc, p) == p.offset
}

/// The cell after `p`, crossing into the next block; `None` at the end of
/// the document.
pub(crate) fn next_cell(doc: &Document, p: Position) -> Option<Position> {
    let t = text_of(doc, p.block);
    if p.offset < t.len() {
        Some(Position::new(p.block, next_boundary(t, p.offset)))
    } else if p.block + 1 < doc.block_count() {
        Some(Position::new(p.block + 1, 0))
    } else {
        None
    }
}

/// The cell before `p`, crossing into the previous block; `None` at the
/// start of the document.
pub(crate) fn prev_cell(doc: &Document, p: Position) -> Option<Position> {
    if p.offset > 0 {
        Some(Position::new(
            p.block,
            prev_boundary(text_of(doc, p.block), p.offset),
        ))
    } else if p.block > 0 {
        Some(Position::new(p.block - 1, len_of(doc, p.block - 1)))
    } else {
        None
    }
}

/// The end (exclusive) of the cell at `p`: the next grapheme boundary, or
/// `p` itself on a newline cell.
pub(crate) fn cell_end(doc: &Document, p: Position) -> Position {
    if is_newline(doc, p) {
        p
    } else {
        Position::new(p.block, next_boundary(text_of(doc, p.block), p.offset))
    }
}

/// The grapheme at `p` (empty on a newline cell).
pub(crate) fn grapheme_at(doc: &Document, p: Position) -> &str {
    if is_newline(doc, p) {
        return "";
    }
    let t = text_of(doc, p.block);
    &t[p.offset..next_boundary(t, p.offset)]
}

/// The offset of the last character cell of the hard line `ls..le` (`ls`
/// when the line is empty).
pub(crate) fn last_char(doc: &Document, block: usize, ls: usize, le: usize) -> usize {
    if le > ls {
        prev_boundary(text_of(doc, block), le)
    } else {
        ls
    }
}

/// Where a Normal-mode caret may rest for `p`: on a character, never on the
/// newline cell of a non-empty hard line.
pub(crate) fn clamp_normal(doc: &Document, p: Position) -> Position {
    let p = doc.clamp_position(p);
    if is_newline(doc, p) {
        let ls = line_start(doc, p);
        if p.offset > ls {
            return Position::new(p.block, prev_boundary(text_of(doc, p.block), p.offset));
        }
    }
    p
}

fn is_blank_char(c: char) -> bool {
    c != '\n' && c.is_whitespace()
}

/// The first non-blank offset of the hard line starting at `ls` (its end
/// when the line is blank).
pub(crate) fn first_non_blank(doc: &Document, block: usize, ls: usize) -> usize {
    let t = text_of(doc, block);
    let ls = ls.min(t.len());
    for (i, c) in t[ls..].char_indices() {
        if !is_blank_char(c) {
            return ls + i;
        }
    }
    t.len()
}

/// The offset of the last non-blank cell of the hard line `ls..le` (`ls`
/// when there is none).
pub(crate) fn last_non_blank(doc: &Document, block: usize, ls: usize, le: usize) -> usize {
    let t = text_of(doc, block);
    let trimmed = t[ls..le].trim_end_matches(is_blank_char);
    if trimmed.is_empty() {
        ls
    } else {
        prev_boundary(t, ls + trimmed.len())
    }
}

/// Number of graphemes between `from` and `to` in block text `t`.
pub(crate) fn graphemes_between(t: &str, from: usize, to: usize) -> usize {
    if from >= to {
        return 0;
    }
    text::grapheme_count(&t[from..to])
}

/// The offset `n` graphemes after `from` in `t`, not past `limit`.
pub(crate) fn advance_graphemes(t: &str, from: usize, n: usize, limit: usize) -> usize {
    let mut o = from;
    for _ in 0..n {
        if o >= limit {
            break;
        }
        o = next_boundary(t, o);
    }
    o.min(limit)
}

// ----- character classes ---------------------------------------------------------------

/// Vim's character classes for word motions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    /// White space and newline cells.
    Blank,
    /// Punctuation and symbols.
    Punct,
    /// Letters, digits, underscore.
    Word,
    /// Han ideographs.
    Han,
    /// Hiragana.
    Hiragana,
    /// Katakana.
    Katakana,
    /// Hangul.
    Hangul,
    /// Emoji and pictographs.
    Emoji,
}

fn is_apostrophe(c: char) -> bool {
    c == '\'' || c == '\u{2019}'
}

fn class_of_char(c: char) -> Class {
    if c.is_whitespace() {
        return Class::Blank;
    }
    match c as u32 {
        0x3040..=0x309F => Class::Hiragana,
        0x30A0..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F => Class::Katakana,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x3FFFF => Class::Han,
        0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF => Class::Hangul,
        0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x231A..=0x23FF | 0x2B50 | 0x2B55 => Class::Emoji,
        _ if c.is_alphanumeric() || c == '_' => Class::Word,
        // Combining marks that start a cell (rare) go with words.
        0x0300..=0x036F => Class::Word,
        _ => Class::Punct,
    }
}

/// Options that change how text is classified.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ClassOpts {
    /// `W`, `B`, `E`: every non-blank is one class.
    pub big: bool,
    /// An apostrophe between two letters belongs to the word.
    pub apostrophe: bool,
}

/// The class of the cell at `p`.
pub(crate) fn class_at(doc: &Document, p: Position, o: ClassOpts) -> Class {
    let t = text_of(doc, p.block);
    if p.offset >= t.len() {
        return Class::Blank;
    }
    let Some(c) = t[p.offset..].chars().next() else {
        return Class::Blank;
    };
    if c == '\n' {
        return Class::Blank;
    }
    let mut class = class_of_char(c);
    if class == Class::Punct && o.apostrophe && is_apostrophe(c) {
        let before = t[..p.offset].chars().next_back();
        let after = t[p.offset + c.len_utf8()..].chars().next();
        if before.is_some_and(char::is_alphanumeric) && after.is_some_and(char::is_alphanumeric) {
            class = Class::Word;
        }
    }
    if o.big && class != Class::Blank {
        return Class::Word;
    }
    class
}

// ----- line units ------------------------------------------------------------------------

/// What linewise commands count: a whole block, or one hard line of a code
/// block. `start..end` is the byte range of the unit's text in its block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Unit {
    pub block: usize,
    pub start: usize,
    pub end: usize,
}

fn is_code(doc: &Document, b: usize) -> bool {
    doc.blocks()[b].kind().is_code()
}

/// The unit containing `p`.
pub(crate) fn unit_at(doc: &Document, p: Position) -> Unit {
    let p = doc.clamp_position(p);
    if is_code(doc, p.block) {
        Unit {
            block: p.block,
            start: line_start(doc, p),
            end: line_end(doc, p),
        }
    } else {
        Unit {
            block: p.block,
            start: 0,
            end: len_of(doc, p.block),
        }
    }
}

/// The unit after `u`.
pub(crate) fn unit_next(doc: &Document, u: Unit) -> Option<Unit> {
    if u.end < len_of(doc, u.block) {
        return Some(unit_at(doc, Position::new(u.block, u.end + 1)));
    }
    if u.block + 1 < doc.block_count() {
        return Some(unit_at(doc, Position::new(u.block + 1, 0)));
    }
    None
}

/// The unit before `u`.
pub(crate) fn unit_prev(doc: &Document, u: Unit) -> Option<Unit> {
    if u.start > 0 {
        return Some(unit_at(doc, Position::new(u.block, u.start - 1)));
    }
    if u.block > 0 {
        let b = u.block - 1;
        return Some(unit_at(doc, Position::new(b, len_of(doc, b))));
    }
    None
}

/// The unit `n` units after (negative: before) `u`, clamped to the
/// document.
pub(crate) fn unit_offset(doc: &Document, u: Unit, n: isize) -> Unit {
    let mut cur = u;
    for _ in 0..n.unsigned_abs() {
        let next = if n > 0 {
            unit_next(doc, cur)
        } else {
            unit_prev(doc, cur)
        };
        match next {
            Some(x) => cur = x,
            None => break,
        }
    }
    cur
}

/// The first non-blank position of a unit (of its first hard line).
pub(crate) fn unit_first_non_blank(doc: &Document, u: Unit) -> Position {
    let le = line_end(doc, Position::new(u.block, u.start));
    let o = first_non_blank(doc, u.block, u.start).min(le);
    Position::new(u.block, o)
}

/// A linewise range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Lines {
    /// Whole blocks.
    Blocks(Range<usize>),
    /// Hard lines `start..end` (byte range from the start of the first line
    /// to the end of the last, without its line break) inside one code
    /// block; possibly all of them.
    Code {
        block: usize,
        start: usize,
        end: usize,
    },
}

impl Lines {
    /// The first position of the range.
    pub fn start(&self) -> Position {
        match self {
            Lines::Blocks(r) => Position::new(r.start, 0),
            Lines::Code { block, start, .. } => Position::new(*block, *start),
        }
    }

    /// Number of line units covered.
    pub fn count(&self, doc: &Document) -> usize {
        match self {
            Lines::Blocks(r) => r.len(),
            Lines::Code { block, start, end } => {
                text_of(doc, *block)[*start..*end].matches('\n').count() + 1
            }
        }
    }
}

/// The linewise range covering the units of `a` and `b` and everything
/// between them.
pub(crate) fn lines_between(doc: &Document, a: Position, b: Position) -> Lines {
    let (a, b) = if a <= b { (a, b) } else { (b, a) };
    let a = doc.clamp_position(a);
    let b = doc.clamp_position(b);
    if a.block == b.block && is_code(doc, a.block) {
        return Lines::Code {
            block: a.block,
            start: line_start(doc, a),
            end: line_end(doc, b),
        };
    }
    Lines::Blocks(a.block..b.block + 1)
}

/// The three kinds of block for paragraph motions and objects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BlockClass {
    /// A block with text.
    Text,
    /// A text block without text.
    Empty,
    /// A divider or page break.
    Void,
}

/// The paragraph class of block `b`.
pub(crate) fn block_class(doc: &Document, b: usize) -> BlockClass {
    let blk = &doc.blocks()[b];
    if blk.is_void() {
        BlockClass::Void
    } else if blk.is_empty() {
        BlockClass::Empty
    } else {
        BlockClass::Text
    }
}
