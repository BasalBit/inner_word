//! Text objects: words, sentences, paragraphs, quotes, brackets.

use std::ops::Range;

use writer_core::{Document, Position, TextRange};

use crate::buf::{self, BlockClass, Class, ClassOpts, Lines, block_class, class_at, text_of};
use crate::motion::sentences;

/// What an operator acts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// A run of text, end exclusive.
    Chars(TextRange),
    /// Whole lines.
    Lines(Lines),
}

/// A text object found by one of Vim's own algorithms: the range runs from
/// `start` to `end`, whose cell is part of it when `inclusive` is set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Object {
    /// `None` when an existing Visual selection is only extended.
    pub start: Option<Position>,
    pub end: Position,
    pub inclusive: bool,
}

impl Object {
    /// The text range for an operator at `cursor`.
    pub fn range(&self, doc: &Document, cursor: Position) -> TextRange {
        let start = self.start.unwrap_or(cursor);
        let end = if self.inclusive {
            buf::cell_end(doc, self.end)
        } else {
            self.end
        };
        TextRange::new(start, end.max(start))
    }
}

// Vim's cursor steps, on the cell model of `buf`: a newline cell stands for
// the NUL that ends a line.

/// How a step ended: within the line, on the end of the line, or on the
/// next (previous) line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Char,
    LineEnd,
    Line,
}

/// Vim's `inc()`.
fn inc(doc: &Document, p: &mut Position) -> Option<Step> {
    let from_newline = buf::is_newline(doc, *p);
    *p = buf::next_cell(doc, *p)?;
    Some(if from_newline {
        Step::Line
    } else if buf::is_newline(doc, *p) {
        Step::LineEnd
    } else {
        Step::Char
    })
}

/// Vim's `dec()`.
fn dec(doc: &Document, p: &mut Position) -> Option<Step> {
    let at_start = buf::at_line_start(doc, *p);
    *p = buf::prev_cell(doc, *p)?;
    Some(if at_start { Step::Line } else { Step::Char })
}

/// Vim's `incl()`: like `inc`, but steps over the end of a line that has
/// text.
fn incl(doc: &Document, p: &mut Position) -> Option<Step> {
    let mut r = inc(doc, p)?;
    if r != Step::Char && !buf::at_line_start(doc, *p) {
        r = inc(doc, p)?;
    }
    Some(r)
}

/// Vim's `decl()`.
fn decl(doc: &Document, p: &mut Position) -> Option<Step> {
    let mut r = dec(doc, p)?;
    if r == Step::Line && !buf::at_line_start(doc, *p) {
        r = dec(doc, p)?;
    }
    Some(r)
}

/// Vim's `oneleft()`: one cell left inside the line.
fn oneleft(doc: &Document, p: &mut Position) -> bool {
    if buf::at_line_start(doc, *p) {
        return false;
    }
    *p = buf::prev_cell(doc, *p).unwrap_or(*p);
    true
}

/// Vim's `fwd_word()` for one word with `eol` set. False is its `FAIL`.
fn fwd_word(doc: &Document, p: &mut Position, opts: ClassOpts) -> bool {
    let cls = |p: Position| class_at(doc, p, opts);
    let last_line =
        |p: Position| buf::next_cell(doc, Position::new(p.block, buf::line_end(doc, p))).is_none();
    let sclass = cls(*p);
    let on_last = last_line(*p);
    let Some(i) = inc(doc, p) else {
        return false;
    };
    if i != Step::Char && on_last {
        return false;
    }
    if i != Step::Char {
        return true;
    }
    if sclass != Class::Blank {
        while cls(*p) == sclass {
            match inc(doc, p) {
                None => return true,
                Some(Step::Char) => {}
                Some(_) => return true,
            }
        }
    }
    while cls(*p) == Class::Blank {
        if buf::is_empty_line(doc, *p) {
            break;
        }
        match inc(doc, p) {
            None => return true,
            Some(Step::Char) => {}
            Some(_) if on_last => return true,
            Some(_) => {}
        }
    }
    true
}

/// Vim's `end_word()` for one word with `stop` and `empty` set.
fn end_word(doc: &Document, p: &mut Position, opts: ClassOpts) -> bool {
    let cls = |p: Position| class_at(doc, p, opts);
    let sclass = cls(*p);
    if inc(doc, p).is_none() {
        return false;
    }
    if cls(*p) == sclass && sclass != Class::Blank {
        while cls(*p) == sclass {
            if inc(doc, p).is_none() {
                return false;
            }
        }
    } else if sclass == Class::Blank {
        while cls(*p) == Class::Blank {
            if buf::is_empty_line(doc, *p) {
                return true;
            }
            if inc(doc, p).is_none() {
                return false;
            }
        }
        let c = cls(*p);
        while cls(*p) == c {
            if inc(doc, p).is_none() {
                return false;
            }
        }
    }
    dec(doc, p);
    true
}

/// Vim's `bck_word()` for one word with `stop` set.
fn bck_word(doc: &Document, p: &mut Position, opts: ClassOpts) -> bool {
    let cls = |p: Position| class_at(doc, p, opts);
    let sclass = cls(*p);
    if dec(doc, p).is_none() {
        return false;
    }
    if sclass == cls(*p) || sclass == Class::Blank {
        while cls(*p) == Class::Blank {
            if buf::is_empty_line(doc, *p) {
                return true;
            }
            if dec(doc, p).is_none() {
                return true;
            }
        }
        let c = cls(*p);
        while cls(*p) == c {
            if dec(doc, p).is_none() {
                return true;
            }
        }
    }
    inc(doc, p);
    true
}

/// Vim's `bckend_word()` for one word with `eol` set.
fn bckend_word(doc: &Document, p: &mut Position, opts: ClassOpts) -> bool {
    let cls = |p: Position| class_at(doc, p, opts);
    let sclass = cls(*p);
    match dec(doc, p) {
        None => return false,
        Some(Step::Line) => return true,
        Some(_) => {}
    }
    if sclass != Class::Blank {
        while cls(*p) == sclass {
            match dec(doc, p) {
                None | Some(Step::Line) => return true,
                Some(_) => {}
            }
        }
    }
    while cls(*p) == Class::Blank {
        if buf::is_empty_line(doc, *p) {
            break;
        }
        match dec(doc, p) {
            None | Some(Step::Line) => return true,
            Some(_) => {}
        }
    }
    true
}

/// To the start of the word or white space under `p`, inside its line
/// (Vim's `back_in_line()`).
fn back_in_line(doc: &Document, p: &mut Position, opts: ClassOpts) {
    let sclass = class_at(doc, *p, opts);
    while !buf::at_line_start(doc, *p) {
        let q = buf::prev_cell(doc, *p).unwrap_or(*p);
        if class_at(doc, q, opts) != sclass {
            break;
        }
        *p = q;
    }
}

/// `iw` / `aw` / `iW` / `aW`, following Vim's `current_word()`: a word, or
/// the white space between words, `count` times; `aw` takes the white
/// space after the word, or the white space before it when none follows.
/// The object runs on into the next line, as in Vim.
///
/// `anchor` is the other end of a Visual selection that is more than one
/// character: the selection is then extended at the cursor's end.
pub(crate) fn word(
    doc: &Document,
    cursor: Position,
    anchor: Option<Position>,
    around: bool,
    count: usize,
    opts: ClassOpts,
) -> Result<Object, Position> {
    let cls = |p: Position| class_at(doc, p, opts);
    let mut cur = cursor;
    let mut count = count.max(1);
    let mut inclusive = true;
    let mut include_white = false;
    let mut start = None;
    if anchor.is_none() {
        back_in_line(doc, &mut cur, opts);
        start = Some(cur);
        if (cls(cur) == Class::Blank) == around {
            if !end_word(doc, &mut cur, opts) {
                return Err(cur);
            }
        } else {
            fwd_word(doc, &mut cur, opts);
            if buf::at_line_start(doc, cur) {
                decl(doc, &mut cur);
            } else {
                oneleft(doc, &mut cur);
            }
            include_white = around;
        }
        count -= 1;
    }
    while count > 0 {
        inclusive = true;
        if anchor.is_some_and(|a| cur < a) {
            // The cursor is at the start of the selection: extend backward.
            if decl(doc, &mut cur).is_none() {
                return Err(cur);
            }
            if around != (cls(cur) != Class::Blank) {
                if !bck_word(doc, &mut cur, opts) {
                    return Err(cur);
                }
            } else {
                if !bckend_word(doc, &mut cur, opts) {
                    return Err(cur);
                }
                incl(doc, &mut cur);
            }
        } else {
            if incl(doc, &mut cur).is_none() {
                return Err(cur);
            }
            if around != (cls(cur) == Class::Blank) {
                if !fwd_word(doc, &mut cur, opts) && count > 1 {
                    return Err(cur);
                }
                // Not the line break after the word.
                if !oneleft(doc, &mut cur) {
                    inclusive = false;
                }
            } else if !end_word(doc, &mut cur, opts) {
                return Err(cur);
            }
        }
        count -= 1;
    }
    if include_white
        && (cls(cur) != Class::Blank || (buf::at_line_start(doc, cur) && !inclusive))
        && let Some(s) = start
    {
        // No white space after the word: take the white space before it,
        // unless that is the indent of the line.
        let mut p = s;
        if oneleft(doc, &mut p) {
            back_in_line(doc, &mut p, opts);
            if cls(p) == Class::Blank && !buf::at_line_start(doc, p) {
                start = Some(p);
            }
        }
    }
    Ok(Object {
        start,
        end: cur,
        inclusive,
    })
}

/// `is` / `as` in the block of `at`.
pub(crate) fn sentence(
    doc: &Document,
    at: Position,
    around: bool,
    count: usize,
) -> Option<TextRange> {
    let t = text_of(doc, at.block);
    let ss = sentences(t);
    if ss.is_empty() {
        return None;
    }
    let count = count.max(1);
    let o = at.offset.min(t.len());
    // The sentence the caret is in (leading white space counts for the first).
    let i = ss.iter().rposition(|s| s.start <= o).unwrap_or(0);
    let last = ss.len() - 1;
    let in_white = o >= ss[i].text_end && ss[i].end > ss[i].text_end && o < ss[i].end;
    let (start, end);
    if around {
        if in_white {
            // The white space and the sentence(s) after it.
            start = ss[i].text_end;
            end = ss[(i + count).min(last)].text_end;
        } else {
            let j = (i + count - 1).min(last);
            if ss[j].end > ss[j].text_end {
                start = ss[i].start;
                end = ss[j].end;
            } else {
                // No white space after: take the white space before.
                start = if i > 0 {
                    ss[i - 1].text_end
                } else {
                    ss[i].start
                };
                end = ss[j].text_end;
            }
        }
    } else {
        // Pieces alternate between sentences and the white space between.
        let mut pieces: Vec<Range<usize>> = Vec::new();
        for s in &ss {
            pieces.push(s.start..s.text_end);
            if s.end > s.text_end {
                pieces.push(s.text_end..s.end);
            }
        }
        let k = pieces
            .iter()
            .position(|r| r.start <= o && o < r.end)
            .or_else(|| pieces.iter().rposition(|r| r.start <= o))
            .unwrap_or(0);
        start = pieces[k].start;
        end = pieces[(k + count - 1).min(pieces.len() - 1)].end;
    }
    Some(TextRange::in_block(at.block, start, end))
}

/// Whether block `b` is a white line for the paragraph objects: an empty
/// block or one that holds only white space (Vim's `linewhite()`).
fn white(doc: &Document, b: usize) -> bool {
    let blk = &doc.blocks()[b];
    !blk.is_void() && blk.text().chars().all(char::is_whitespace)
}

/// Whether a paragraph starts at block `b` although the block before it is
/// not white: dividers and page breaks are paragraphs of their own.
fn starts_paragraph(doc: &Document, b: usize) -> bool {
    b > 0
        && !white(doc, b)
        && !white(doc, b - 1)
        && (block_class(doc, b) == BlockClass::Void)
            != (block_class(doc, b - 1) == BlockClass::Void)
}

/// `ip` / `ap`, following Vim's `current_par()`: a paragraph is a run of
/// blocks with text, the white lines between paragraphs are runs of empty
/// (or blank) blocks, and a run of dividers or page breaks is a paragraph
/// too. `ap` takes the white lines after the paragraph, or those before it
/// when none follow. `None` when the document ends before `count` objects.
///
/// `anchor_block` is the block of the other end of a Visual selection that
/// spans several blocks: the selection is then extended, and the result is
/// the block the cursor moves to (as a one-block range).
pub(crate) fn paragraph(
    doc: &Document,
    at: Position,
    anchor_block: Option<usize>,
    around: bool,
    count: usize,
) -> Option<Range<usize>> {
    let n = doc.block_count();
    let last = n - 1;
    let count = count.max(1);
    if let Some(a) = anchor_block {
        // Extend the selection by whole objects in its direction (forward
        // when it covers one block).
        let forward = at.block >= a;
        let edge = if forward { last } else { 0 };
        let step = |b: usize| if forward { b + 1 } else { b - 1 };
        let mut b = at.block;
        for _ in 0..count {
            if b == edge {
                return None;
            }
            let mut prev_white = None;
            for _ in 0..2 {
                b = step(b);
                let is_white = white(doc, b);
                if prev_white == Some(is_white) {
                    b = if forward { b - 1 } else { b + 1 };
                    break;
                }
                while b != edge {
                    let next = step(b);
                    let boundary = if forward { next } else { b };
                    if is_white != white(doc, next)
                        || (!is_white && starts_paragraph(doc, boundary))
                    {
                        break;
                    }
                    b = next;
                }
                if !around || b == edge {
                    break;
                }
                prev_white = Some(is_white);
            }
        }
        return Some(b..b + 1);
    }
    let white_in_front = white(doc, at.block);
    let mut start = at.block;
    while start > 0 {
        if white_in_front {
            if !white(doc, start - 1) {
                break;
            }
        } else if white(doc, start - 1) || starts_paragraph(doc, start) {
            break;
        }
        start -= 1;
    }
    // `end` is one past the last block taken.
    let mut end = start;
    while end < n && white(doc, end) {
        end += 1;
    }
    let mut i = count;
    if !around && white_in_front {
        i -= 1;
    }
    while i > 0 {
        i -= 1;
        if end == n {
            return None;
        }
        let do_white = !around && white(doc, end);
        if around || !do_white {
            end += 1;
            while end < n && !white(doc, end) && !starts_paragraph(doc, end) {
                end += 1;
            }
        }
        if i == 0 && white_in_front && around {
            break;
        }
        if around || do_white {
            while end < n && white(doc, end) {
                end += 1;
            }
        }
    }
    if end == start {
        return None;
    }
    if !white_in_front && !white(doc, end - 1) && around {
        while start > 0 && white(doc, start - 1) {
            start -= 1;
        }
    }
    Some(start..end)
}

// ----- quotes ---------------------------------------------------------------------------------

/// A quoted string on a line: the offsets of the opening and closing quote
/// characters and their byte lengths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Quoted {
    open: usize,
    open_len: usize,
    close: usize,
    close_len: usize,
}

/// The next `q` at or after `col`; with `escape`, a backslash hides the
/// character after it (Vim's `find_next_quote()`).
fn find_next_quote(t: &str, mut col: usize, le: usize, q: char, escape: bool) -> Option<usize> {
    while col < le {
        let c = t[col..].chars().next()?;
        if escape && c == '\\' {
            col += 1;
            if col >= le {
                return None;
            }
            let c2 = t[col..].chars().next()?;
            col += c2.len_utf8();
            continue;
        }
        if c == q {
            return Some(col);
        }
        col += c.len_utf8();
    }
    None
}

/// The last `q` before `col` that is not escaped, or `ls` when there is
/// none (Vim's `find_prev_quote()`).
fn find_prev_quote(t: &str, ls: usize, mut col: usize, q: char) -> usize {
    while col > ls {
        col = buf::prev_boundary(t, col).max(ls);
        while !t.is_char_boundary(col) {
            col -= 1;
        }
        let backslashes = t[ls..col].chars().rev().take_while(|c| *c == '\\').count();
        if backslashes % 2 == 1 {
            col -= backslashes;
        } else if t[col..].starts_with(q) {
            break;
        }
    }
    col
}

/// Vim's rule for the straight quote `q` around offset `o` of the line
/// `ls..le`. On a quote character the pairs are counted from the start of
/// the line; anywhere else the string starts at the nearest quote before
/// the caret, or at the first one after it when none comes before.
fn straight_quotes(t: &str, ls: usize, le: usize, o: usize, q: char) -> Option<Quoted> {
    let (open, close);
    if o < le && t[o..].starts_with(q) {
        let mut start = ls;
        loop {
            let s = find_next_quote(t, start, le, q, false)?;
            if s > o {
                return None;
            }
            let e = find_next_quote(t, s + q.len_utf8(), le, q, true)?;
            if s <= o && o <= e {
                open = s;
                close = e;
                break;
            }
            start = e + q.len_utf8();
        }
    } else {
        let mut s = find_prev_quote(t, ls, o, q);
        if !t[s..].starts_with(q) {
            s = find_next_quote(t, s, le, q, false)?;
        }
        open = s;
        close = find_next_quote(t, s + q.len_utf8(), le, q, true)?;
    }
    Some(Quoted {
        open,
        open_len: q.len_utf8(),
        close,
        close_len: q.len_utf8(),
    })
}

fn is_word_apostrophe(t: &str, i: usize, c: char) -> bool {
    let before = t[..i].chars().next_back();
    let after = t[i + c.len_utf8()..].chars().next();
    before.is_some_and(char::is_alphanumeric) && after.is_some_and(char::is_alphanumeric)
}

fn curly_quotes(
    t: &str,
    ls: usize,
    le: usize,
    o: usize,
    open: char,
    close: char,
) -> Option<Quoted> {
    // (offset, is_open) of every quote mark on the line; an apostrophe
    // inside a word is not a closing quote.
    let marks: Vec<(usize, bool)> = t[ls..le]
        .char_indices()
        .filter_map(|(i, c)| {
            let at = ls + i;
            if c == open {
                Some((at, true))
            } else if c == close && !(close == '\u{2019}' && is_word_apostrophe(t, at, c)) {
                Some((at, false))
            } else {
                None
            }
        })
        .collect();
    let matching_close = |open_idx: usize| {
        let mut depth = 0usize;
        for (at, is_open) in &marks[open_idx..] {
            if *is_open {
                depth += 1;
            } else {
                depth -= 1;
                if depth == 0 {
                    return Some(*at);
                }
            }
        }
        None
    };
    // The innermost pair around the caret.
    let mut depth = 0usize;
    for (idx, (at, is_open)) in marks.iter().enumerate().rev() {
        if *at > o {
            continue;
        }
        if *is_open {
            if depth == 0 {
                if let Some(c) = matching_close(idx)
                    && c >= o
                {
                    return Some(Quoted {
                        open: *at,
                        open_len: open.len_utf8(),
                        close: c,
                        close_len: close.len_utf8(),
                    });
                }
            } else {
                depth -= 1;
            }
        } else if *at < o {
            depth += 1;
        }
    }
    // Otherwise the first pair after the caret.
    let idx = marks.iter().position(|(at, is_open)| *is_open && *at > o)?;
    let c = matching_close(idx)?;
    Some(Quoted {
        open: marks[idx].0,
        open_len: open.len_utf8(),
        close: c,
        close_len: close.len_utf8(),
    })
}

/// `i"` / `a"` and the same for `'` and `` ` `` on the hard line of `at`.
/// The double and single quote objects also match the typographic pairs
/// (`“ ”` and `‘ ’`) that smart punctuation produces.
pub(crate) fn quote(
    doc: &Document,
    at: Position,
    q: char,
    around: bool,
    count: usize,
) -> Option<TextRange> {
    let t = text_of(doc, at.block);
    let ls = buf::line_start(doc, at);
    let le = buf::line_end(doc, at);
    let o = at.offset.min(le);
    let straight = straight_quotes(t, ls, le, o, q);
    let curly = match q {
        '"' => curly_quotes(t, ls, le, o, '\u{201C}', '\u{201D}'),
        '\'' => curly_quotes(t, ls, le, o, '\u{2018}', '\u{2019}'),
        _ => None,
    };
    let contains = |x: &Quoted| x.open <= o && o <= x.close;
    let found = match (straight, curly) {
        (Some(a), Some(b)) => match (contains(&a), contains(&b)) {
            (true, false) => a,
            (false, true) => b,
            (true, true) => {
                if a.close - a.open <= b.close - b.open {
                    a
                } else {
                    b
                }
            }
            (false, false) => {
                if a.open <= b.open {
                    a
                } else {
                    b
                }
            }
        },
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => return None,
    };
    if !around && count >= 2 {
        // `2i"`: the quotes too, without the white space `a"` adds.
        return Some(TextRange::in_block(
            at.block,
            found.open,
            found.close + found.close_len,
        ));
    }
    if !around {
        return Some(TextRange::in_block(
            at.block,
            found.open + found.open_len,
            found.close,
        ));
    }
    let mut start = found.open;
    let mut end = found.close + found.close_len;
    let after: usize = t[end..le]
        .chars()
        .take_while(|c| c.is_whitespace())
        .map(char::len_utf8)
        .sum();
    if after > 0 {
        end += after;
    } else {
        let before: usize = t[ls..start]
            .chars()
            .rev()
            .take_while(|c| c.is_whitespace())
            .map(char::len_utf8)
            .sum();
        start -= before;
    }
    Some(TextRange::in_block(at.block, start, end))
}

// ----- brackets -------------------------------------------------------------------------------

fn char_at(doc: &Document, p: Position) -> Option<char> {
    text_of(doc, p.block).get(p.offset..)?.chars().next()
}

/// The unmatched `open` before `from` (not looking at `from` itself),
/// anywhere in the document.
fn unmatched_open(doc: &Document, from: Position, open: char, close: char) -> Option<Position> {
    let mut depth = 0usize;
    let mut b = from.block;
    let mut upto = from.offset.min(buf::len_of(doc, b));
    loop {
        for (i, c) in text_of(doc, b)[..upto].char_indices().rev() {
            if c == close {
                depth += 1;
            } else if c == open {
                if depth == 0 {
                    return Some(Position::new(b, i));
                }
                depth -= 1;
            }
        }
        if b == 0 {
            return None;
        }
        b -= 1;
        upto = buf::len_of(doc, b);
    }
}

/// The unmatched `close` after `from` (not looking at `from` itself).
fn unmatched_close(doc: &Document, from: Position, open: char, close: char) -> Option<Position> {
    let mut depth = 0usize;
    let mut b = from.block;
    let mut start = match char_at(doc, from) {
        Some(c) => from.offset + c.len_utf8(),
        None => buf::len_of(doc, b),
    };
    loop {
        for (i, c) in text_of(doc, b)[start..].char_indices() {
            if c == open {
                depth += 1;
            } else if c == close {
                if depth == 0 {
                    return Some(Position::new(b, start + i));
                }
                depth -= 1;
            }
        }
        b += 1;
        if b >= doc.block_count() {
            return None;
        }
        start = 0;
    }
}

/// The next `open` after `from` that starts a block of its own: a `close`
/// on the way hides the `open` after it, as in Vim's forward search.
fn next_open(doc: &Document, from: Position, open: char, close: char) -> Option<Position> {
    let mut depth = 0usize;
    let mut b = from.block;
    let mut start = match char_at(doc, from) {
        Some(c) => from.offset + c.len_utf8(),
        None => buf::len_of(doc, b),
    };
    loop {
        for (i, c) in text_of(doc, b)[start..].char_indices() {
            if c == close {
                depth += 1;
            } else if c == open {
                if depth == 0 {
                    return Some(Position::new(b, start + i));
                }
                depth -= 1;
            }
        }
        b += 1;
        if b >= doc.block_count() {
            return None;
        }
        start = 0;
    }
}

/// Whether `p` is on the white space that starts its line.
fn in_indent(doc: &Document, p: Position) -> bool {
    let ls = buf::line_start(doc, p);
    !buf::is_newline(doc, p) && p.offset < buf::first_non_blank(doc, p.block, ls)
}

/// `i(` / `a(` and the same for `[ ]`, `{ }` and `< >`, following Vim's
/// `current_block()`: the `count`th pair around the cursor, anywhere in the
/// document; when the cursor is in no pair, the next one after it. With
/// `around` the brackets are included.
///
/// `anchor` is the other end of a Visual selection of more than one
/// character; a selection that already covers the pair grows to the next
/// pair out.
pub(crate) fn bracket(
    doc: &Document,
    cursor: Position,
    anchor: Option<Position>,
    open: char,
    close: char,
    around: bool,
    count: usize,
) -> Option<Object> {
    let mut old_start = cursor;
    let mut old_end = cursor;
    let mut cur = cursor;
    match anchor {
        None => {
            if char_at(doc, cur) == Some(open) {
                // On the opening bracket: inside its pair.
                cur = buf::next_cell(doc, cur)?;
            }
        }
        Some(a) if a < cursor => {
            old_start = a;
            cur = a;
        }
        Some(a) => old_end = a,
    }
    let around_cursor = unmatched_open(doc, cur, open, close).is_some();
    let mut start_pos = None;
    for _ in 0..count.max(1) {
        let found = if around_cursor {
            unmatched_open(doc, cur, open, close)
        } else {
            next_open(doc, cur, open, close)
        };
        start_pos = found;
        match found {
            Some(p) => cur = p,
            None => break,
        }
    }
    let mut start_pos = start_pos?;
    cur = unmatched_close(doc, start_pos, open, close)?;
    let mut sol = false;
    loop {
        if around {
            break;
        }
        incl(doc, &mut start_pos);
        sol = buf::at_line_start(doc, cur);
        decl(doc, &mut cur);
        while in_indent(doc, cur) {
            sol = true;
            if decl(doc, &mut cur) != Some(Step::Char) {
                break;
            }
        }
        if anchor.is_some() && start_pos >= old_start && old_end >= cur && start_pos != cur {
            // Not bigger than what was selected: take the next pair out.
            let mut p = old_start;
            decl(doc, &mut p);
            let s = unmatched_open(doc, p, open, close)?;
            start_pos = s;
            cur = unmatched_close(doc, s, open, close)?;
        } else {
            break;
        }
    }
    if !around && sol {
        // The closing bracket starts a line: up to and with the line break.
        let mut end = cur;
        incl(doc, &mut end);
        return Some(Object {
            start: Some(start_pos),
            end,
            inclusive: false,
        });
    }
    if start_pos <= cur {
        Some(Object {
            start: Some(start_pos),
            end: cur,
            inclusive: true,
        })
    } else {
        // Nothing between the brackets.
        Some(Object {
            start: Some(start_pos),
            end: start_pos,
            inclusive: false,
        })
    }
}
