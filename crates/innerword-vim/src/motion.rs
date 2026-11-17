//! Motions that need no layout: words, characters, sentences, paragraphs,
//! matching brackets. Each works on the cell model of [`crate::buf`].

use innerword_core::{Document, Position};

use crate::buf::{
    self, BlockClass, Class, ClassOpts, block_class, class_at, is_empty_line, is_newline,
    next_cell, prev_cell, text_of,
};

/// `w` / `W`: forward to the start of the `count`th next word.
///
/// With `op_pending` the last word moved over ends at the end of its line
/// (Vim's rule for `dw` on the last word of a line).
pub(crate) fn word_forward(
    doc: &Document,
    start: Position,
    count: usize,
    opts: ClassOpts,
    op_pending: bool,
) -> Position {
    let cls = |p: Position| class_at(doc, p, opts);
    let mut p = start;
    for i in 0..count {
        let last = i + 1 == count;
        let sclass = cls(p);
        let started_on_newline = is_newline(doc, p);
        // Always move at least one cell.
        let Some(n) = next_cell(doc, p) else {
            return p;
        };
        p = n;
        if op_pending && last && (started_on_newline || is_newline(doc, p)) {
            return p;
        }
        if sclass != Class::Blank {
            while cls(p) == sclass {
                let Some(n) = next_cell(doc, p) else {
                    return p;
                };
                p = n;
                if op_pending && last && is_newline(doc, p) {
                    return p;
                }
            }
        }
        while cls(p) == Class::Blank {
            if is_empty_line(doc, p) {
                break;
            }
            let Some(n) = next_cell(doc, p) else {
                return p;
            };
            p = n;
            // For an operator the last word ends with its line, also when
            // white space follows it (`dw` on `word  ` at the end of a line
            // does not join the next line).
            if op_pending && last && is_newline(doc, p) && !is_empty_line(doc, p) {
                return p;
            }
        }
    }
    p
}

/// `e` / `E`: forward to the end of the `count`th word. With `stop`, a
/// caret already at the end of a word counts that word (used by `cw`).
pub(crate) fn word_end(
    doc: &Document,
    start: Position,
    count: usize,
    opts: ClassOpts,
    stop: bool,
) -> Position {
    let cls = |p: Position| class_at(doc, p, opts);
    let mut stop = stop;
    let mut p = start;
    'counts: for _ in 0..count {
        let sclass = cls(p);
        let Some(n) = next_cell(doc, p) else {
            return p;
        };
        let origin = p;
        p = n;
        if cls(p) == sclass && sclass != Class::Blank {
            while cls(p) == sclass {
                let Some(n) = next_cell(doc, p) else {
                    return p;
                };
                p = n;
            }
        } else if !stop || sclass == Class::Blank {
            while cls(p) == Class::Blank {
                let Some(n) = next_cell(doc, p) else {
                    // Only blanks up to the end of the document.
                    return p;
                };
                p = n;
            }
            let c = cls(p);
            while cls(p) == c {
                let Some(n) = next_cell(doc, p) else {
                    return p;
                };
                p = n;
            }
        } else {
            // Already at the end of a word and asked to stop there.
            p = origin;
            stop = false;
            continue 'counts;
        }
        // Overshot by one.
        if let Some(b) = prev_cell(doc, p) {
            p = b;
        }
        stop = false;
    }
    p
}

/// `b` / `B`: backward to the start of the `count`th previous word.
pub(crate) fn word_backward(
    doc: &Document,
    start: Position,
    count: usize,
    opts: ClassOpts,
) -> Position {
    let cls = |p: Position| class_at(doc, p, opts);
    let mut p = start;
    'counts: for _ in 0..count {
        let Some(n) = prev_cell(doc, p) else {
            return p;
        };
        p = n;
        // Skip white space before the word; stop on an empty line.
        while cls(p) == Class::Blank {
            if is_empty_line(doc, p) {
                continue 'counts;
            }
            let Some(n) = prev_cell(doc, p) else {
                return p;
            };
            p = n;
        }
        // Move back to the start of this word.
        let c = cls(p);
        loop {
            let Some(n) = prev_cell(doc, p) else {
                return p;
            };
            if cls(n) != c {
                break;
            }
            p = n;
        }
    }
    p
}

/// `ge` / `gE`: backward to the end of the `count`th previous word.
pub(crate) fn word_end_backward(
    doc: &Document,
    start: Position,
    count: usize,
    opts: ClassOpts,
) -> Position {
    let cls = |p: Position| class_at(doc, p, opts);
    let mut p = start;
    for _ in 0..count {
        let sclass = cls(p);
        let Some(n) = prev_cell(doc, p) else {
            return p;
        };
        p = n;
        if sclass != Class::Blank {
            while cls(p) == sclass {
                let Some(n) = prev_cell(doc, p) else {
                    return p;
                };
                p = n;
            }
        }
        while cls(p) == Class::Blank {
            if is_empty_line(doc, p) {
                break;
            }
            let Some(n) = prev_cell(doc, p) else {
                return p;
            };
            p = n;
        }
    }
    p
}

/// `f` / `F` / `t` / `T` inside the hard line of `from`: the `count`th
/// occurrence of `ch`. `skip_adjacent` is set when `;` or `,` repeats a
/// `t` / `T`, so that a match right next to the caret does not hold the
/// caret in place.
pub(crate) fn find_char(
    doc: &Document,
    from: Position,
    ch: char,
    forward: bool,
    till: bool,
    count: usize,
    skip_adjacent: bool,
) -> Option<Position> {
    let t = text_of(doc, from.block);
    let ls = buf::line_start(doc, from);
    let le = buf::line_end(doc, from);
    let matches = |o: usize| t[o..].starts_with(ch);
    let mut found = 0;
    if forward {
        let mut o = if from.offset < le {
            buf::next_boundary(t, from.offset)
        } else {
            le
        };
        // (Only without a count, as in Vim.)
        if till && skip_adjacent && count == 1 && o < le {
            o = buf::next_boundary(t, o);
        }
        while o < le {
            if matches(o) {
                found += 1;
                if found == count {
                    let o = if till { buf::prev_boundary(t, o) } else { o };
                    return Some(Position::new(from.block, o));
                }
            }
            o = buf::next_boundary(t, o);
        }
    } else {
        let mut o = from.offset.min(le);
        if till && skip_adjacent && count == 1 && o > ls {
            o = buf::prev_boundary(t, o);
        }
        while o > ls {
            o = buf::prev_boundary(t, o);
            if matches(o) {
                found += 1;
                if found == count {
                    let o = if till { buf::next_boundary(t, o) } else { o };
                    return Some(Position::new(from.block, o));
                }
            }
        }
    }
    None
}

// ----- paragraphs ---------------------------------------------------------------------------

fn is_boundary(doc: &Document, b: usize) -> bool {
    block_class(doc, b) != BlockClass::Text
}

/// `}`: the next paragraph boundary (an empty block or a void block) after
/// the paragraph the caret is in or before. Returns the position and
/// whether the end of the document was hit instead (then the position is
/// the end of the last block and the motion is inclusive). `None` when the
/// document ends before the last of `count` paragraphs (Vim then does not
/// move at all).
pub(crate) fn paragraph_forward(
    doc: &Document,
    from: Position,
    count: usize,
) -> Option<(Position, bool)> {
    let n = doc.block_count();
    let mut i = from.block;
    for step in 0..count {
        while i < n && is_boundary(doc, i) {
            i += 1;
        }
        while i < n && !is_boundary(doc, i) {
            i += 1;
        }
        if i >= n {
            return (step + 1 == count).then(|| (doc.end(), true));
        }
    }
    Some((Position::new(i, 0), false))
}

/// `{`: the previous paragraph boundary, or the start of the document.
/// `None` when the document starts before the last of `count` paragraphs.
pub(crate) fn paragraph_backward(doc: &Document, from: Position, count: usize) -> Option<Position> {
    let mut i = from.block as isize;
    for step in 0..count {
        while i >= 0 && is_boundary(doc, i as usize) {
            i -= 1;
        }
        while i >= 0 && !is_boundary(doc, i as usize) {
            i -= 1;
        }
        if i < 0 {
            return (step + 1 == count).then_some(Position::ZERO);
        }
    }
    Some(Position::new(i as usize, 0))
}

// ----- sentences -----------------------------------------------------------------------------

fn is_sentence_end(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '\u{2026}')
}

fn is_cjk_sentence_end(c: char) -> bool {
    matches!(c, '\u{3002}' | '\u{FF01}' | '\u{FF1F}')
}

fn is_closer(c: char) -> bool {
    matches!(
        c,
        ')' | ']' | '"' | '\'' | '\u{201D}' | '\u{2019}' | '\u{00BB}'
    )
}

/// One sentence of a block: `start..end` covers the sentence and the white
/// space after it; `text_end` is where its text ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sentence {
    pub start: usize,
    pub text_end: usize,
    pub end: usize,
}

/// The sentences of a block's text, in order. Leading white space belongs
/// to no sentence. A sentence ends at `.`, `!`, `?` or an ellipsis,
/// followed by any closing brackets and quotes, then white space or the end
/// of the text; at a CJK full stop; and at an empty line inside the block.
pub(crate) fn sentences(t: &str) -> Vec<Sentence> {
    let mut out = Vec::new();
    let chars: Vec<(usize, char)> = t.char_indices().collect();
    let n = chars.len();
    let mut i = 0;
    while i < n && chars[i].1.is_whitespace() {
        i += 1;
    }
    while i < n {
        let start = chars[i].0;
        let mut text_end = t.len();
        let mut next = n;
        let mut j = i;
        while j < n {
            let c = chars[j].1;
            let mut ends = false;
            let mut k = j + 1;
            if is_cjk_sentence_end(c) {
                while k < n && is_closer(chars[k].1) {
                    k += 1;
                }
                ends = true;
            } else if is_sentence_end(c) {
                while k < n && (is_sentence_end(chars[k].1) || is_closer(chars[k].1)) {
                    k += 1;
                }
                ends = k >= n || chars[k].1.is_whitespace();
            } else if c == '\n' && k < n && chars[k].1 == '\n' {
                // An empty line inside the block ends the sentence before it.
                text_end = chars[j].0;
                let mut m = j;
                while m < n && chars[m].1.is_whitespace() {
                    m += 1;
                }
                next = m;
                break;
            }
            if ends {
                text_end = if k < n { chars[k].0 } else { t.len() };
                let mut m = k;
                while m < n && chars[m].1.is_whitespace() {
                    m += 1;
                }
                next = m;
                break;
            }
            j += 1;
        }
        if next >= n && text_end == t.len() {
            // Unterminated last sentence: trailing white space is not text.
            text_end = start + t[start..].trim_end().len();
        }
        let end = if next < n { chars[next].0 } else { t.len() };
        out.push(Sentence {
            start,
            text_end: text_end.max(start),
            end,
        });
        i = next;
    }
    out
}

fn first_sentence_start(doc: &Document, b: usize) -> usize {
    sentences(text_of(doc, b)).first().map_or(0, |s| s.start)
}

/// Whether block `b` is an empty line for the sentence commands: no text,
/// or a divider or page break.
fn is_empty_block(doc: &Document, b: usize) -> bool {
    block_class(doc, b) != BlockClass::Text
}

/// `)`: the start of the `count`th next sentence. A block start is a
/// sentence start, and so is an empty block (or a divider), but a run of
/// empty blocks counts once: from an empty block the motion goes to the
/// text after the run, as in Vim. At the end of the document the last
/// count goes to its end; `None` when counts are left over then.
pub(crate) fn sentence_forward(doc: &Document, from: Position, count: usize) -> Option<Position> {
    let n = doc.block_count();
    let mut p = from;
    for step in 0..count {
        if is_empty_block(doc, p.block) {
            let mut b = p.block;
            while b < n && is_empty_block(doc, b) {
                b += 1;
            }
            if b >= n {
                return (step + 1 == count).then(|| doc.end());
            }
            p = Position::new(b, first_sentence_start(doc, b));
            continue;
        }
        // White space before the first sentence of a block belongs to that
        // sentence.
        let ss = sentences(text_of(doc, p.block));
        let skip = usize::from(ss.first().is_some_and(|f| p.offset < f.start));
        let next_in_block = ss
            .into_iter()
            .skip(skip)
            .map(|s| s.start)
            .find(|s| *s > p.offset);
        match next_in_block {
            Some(o) => p = Position::new(p.block, o),
            None => {
                if p.block + 1 >= n {
                    return (step + 1 == count).then(|| doc.end());
                }
                let b = p.block + 1;
                p = Position::new(b, first_sentence_start(doc, b));
            }
        }
    }
    Some(p)
}

/// `(`: the start of the current sentence, or of the previous one when the
/// caret is already there. From an empty block it goes to the last sentence
/// of the text before the run of empty blocks. `None` when the document
/// starts before the last of `count` sentences.
pub(crate) fn sentence_backward(doc: &Document, from: Position, count: usize) -> Option<Position> {
    let mut p = from;
    for step in 0..count {
        if is_empty_block(doc, p.block) {
            let mut b = p.block;
            while b > 0 && is_empty_block(doc, b) {
                b -= 1;
            }
            if is_empty_block(doc, b) {
                // Only empty blocks up to the start of the document.
                return (step + 1 == count).then_some(Position::ZERO);
            }
            let o = sentences(text_of(doc, b)).last().map_or(0, |s| s.start);
            p = Position::new(b, o);
            continue;
        }
        let prev_in_block = sentences(text_of(doc, p.block))
            .into_iter()
            .map(|s| s.start)
            .take_while(|s| *s < p.offset)
            .last();
        match prev_in_block {
            Some(o) => p = Position::new(p.block, o),
            None => {
                if p.block == 0 {
                    let start = Position::new(0, first_sentence_start(doc, 0).min(p.offset));
                    if step + 1 == count {
                        return Some(start);
                    }
                    return None;
                }
                let b = p.block - 1;
                let o = sentences(text_of(doc, b)).last().map_or(0, |s| s.start);
                p = Position::new(b, o);
            }
        }
    }
    Some(p)
}

// ----- matching brackets --------------------------------------------------------------------

const PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];

/// The partner of the bracket `open`/`close` at `at`, scanning the whole
/// document with nesting.
pub(crate) fn match_bracket(
    doc: &Document,
    at: Position,
    open: char,
    close: char,
    forward: bool,
) -> Option<Position> {
    let mut depth = 0usize;
    if forward {
        let mut b = at.block;
        let mut from = at.offset;
        loop {
            let t = text_of(doc, b);
            for (i, c) in t[from..].char_indices() {
                if c == open {
                    depth += 1;
                } else if c == close {
                    depth -= 1;
                    if depth == 0 {
                        return Some(Position::new(b, from + i));
                    }
                }
            }
            b += 1;
            from = 0;
            if b >= doc.block_count() {
                return None;
            }
        }
    } else {
        let mut b = at.block;
        let t = text_of(doc, b);
        let c = t[at.offset..].chars().next()?;
        let mut upto = at.offset + c.len_utf8();
        loop {
            let t = text_of(doc, b);
            for (i, c) in t[..upto].char_indices().rev() {
                if c == close {
                    depth += 1;
                } else if c == open {
                    depth -= 1;
                    if depth == 0 {
                        return Some(Position::new(b, i));
                    }
                }
            }
            if b == 0 {
                return None;
            }
            b -= 1;
            upto = text_of(doc, b).len();
        }
    }
}

/// `%`: the first bracket at or after the caret on its hard line, and its
/// partner.
pub(crate) fn match_pair(doc: &Document, from: Position) -> Option<Position> {
    let t = text_of(doc, from.block);
    let le = buf::line_end(doc, from);
    let start = from.offset.min(le);
    for (i, c) in t[start..le].char_indices() {
        for (open, close) in PAIRS {
            if c == open {
                return match_bracket(doc, Position::new(from.block, start + i), open, close, true);
            }
            if c == close {
                return match_bracket(
                    doc,
                    Position::new(from.block, start + i),
                    open,
                    close,
                    false,
                );
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentence_splitting() {
        let t = "One. Two two!  Three (three)? \"Four.\" Five";
        let s = sentences(t);
        let texts: Vec<&str> = s.iter().map(|s| &t[s.start..s.text_end]).collect();
        assert_eq!(
            texts,
            ["One.", "Two two!", "Three (three)?", "\"Four.\"", "Five"]
        );
        assert_eq!(&t[s[1].start..s[1].end], "Two two!  ");
        assert_eq!(s[4].end, t.len());
        let t = "  lead. e.g. not split";
        let s = sentences(t);
        assert_eq!(s[0].start, 2);
        assert_eq!(s.len(), 3);
        let t = "中文。第二句。";
        assert_eq!(sentences(t).len(), 2);
        assert!(sentences("   ").is_empty());
        assert!(sentences("").is_empty());
        let t = "trailing   ";
        let s = sentences(t);
        assert_eq!((s[0].start, s[0].text_end, s[0].end), (0, 8, 11));
    }
}
