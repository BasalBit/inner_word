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

/// The runs of equal character class on the hard line of `at`.
fn class_runs(doc: &Document, at: Position, opts: ClassOpts) -> Vec<(Range<usize>, Class)> {
    let t = text_of(doc, at.block);
    let ls = buf::line_start(doc, at);
    let le = buf::line_end(doc, at);
    let mut runs: Vec<(Range<usize>, Class)> = Vec::new();
    let mut o = ls;
    while o < le {
        let next = buf::next_boundary(t, o);
        let c = class_at(doc, Position::new(at.block, o), opts);
        match runs.last_mut() {
            Some((r, rc)) if *rc == c => r.end = next,
            _ => runs.push((o..next, c)),
        }
        o = next;
    }
    runs
}

/// `iw` / `aw` / `iW` / `aW` on the hard line of `at`.
pub(crate) fn word(
    doc: &Document,
    at: Position,
    around: bool,
    count: usize,
    opts: ClassOpts,
) -> Option<TextRange> {
    let runs = class_runs(doc, at, opts);
    if runs.is_empty() {
        // An empty line: nothing to select.
        return if around { None } else { Some(TextRange::empty(at)) };
    }
    let le = runs.last().map(|(r, _)| r.end).unwrap_or(at.offset);
    let o = if at.offset >= le {
        runs.last().map(|(r, _)| r.start).unwrap_or(at.offset)
    } else {
        at.offset
    };
    let idx = runs.iter().position(|(r, _)| r.start <= o && o < r.end)?;
    let last = runs.len() - 1;
    let ls = runs[0].0.start;
    let mut start = runs[idx].0.start;
    let end;
    if !around {
        end = runs[(idx + count.max(1) - 1).min(last)].0.end;
    } else {
        // Each count takes a word and the white space on one side of it.
        let mut i = idx;
        let mut trailing_white = false;
        for n in 0..count.max(1) {
            if n > 0 {
                if i >= last {
                    break;
                }
                i += 1;
            }
            if runs[i].1 == Class::Blank {
                // White space first, then the word after it.
                if i < last {
                    i += 1;
                }
                trailing_white = false;
            } else if i < last && runs[i + 1].1 == Class::Blank {
                i += 1;
                trailing_white = true;
            } else {
                trailing_white = false;
            }
        }
        end = runs[i].0.end;
        if !trailing_white
            && runs[idx].1 != Class::Blank
            && idx > 0
            && runs[idx - 1].1 == Class::Blank
            && runs[idx - 1].0.start > ls
        {
            // No white space after: take the white space before, unless it
            // is the indent of the line.
            start = runs[idx - 1].0.start;
        }
    }
    Some(TextRange::in_block(at.block, start, end))
}

/// `is` / `as` in the block of `at`.
pub(crate) fn sentence(doc: &Document, at: Position, around: bool, count: usize) -> Option<TextRange> {
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
                start = if i > 0 { ss[i - 1].text_end } else { ss[i].start };
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

/// `ip` / `ap`: runs of text blocks, of empty blocks, or of void blocks.
pub(crate) fn paragraph(doc: &Document, at: Position, around: bool, count: usize) -> Option<Lines> {
    let n = doc.block_count();
    let run_end = |from: usize| {
        let c = block_class(doc, from);
        let mut e = from + 1;
        while e < n && block_class(doc, e) == c {
            e += 1;
        }
        e
    };
    let class = block_class(doc, at.block);
    let mut start = at.block;
    while start > 0 && block_class(doc, start - 1) == class {
        start -= 1;
    }
    let mut end = run_end(at.block);
    let count = count.max(1);
    if !around {
        for _ in 1..count {
            if end >= n {
                break;
            }
            end = run_end(end);
        }
        return Some(Lines::Blocks(start..end));
    }
    let mut trailing_white = false;
    for i in 0..count {
        if i > 0 {
            if end >= n {
                break;
            }
            end = run_end(end);
        }
        let last_class = block_class(doc, end - 1);
        trailing_white = false;
        if end < n {
            match (last_class, block_class(doc, end)) {
                // A paragraph takes the empty blocks after it.
                (BlockClass::Text, BlockClass::Empty) => {
                    end = run_end(end);
                    trailing_white = true;
                }
                // Empty blocks take the paragraph after them.
                (BlockClass::Empty, BlockClass::Text) => end = run_end(end),
                _ => {}
            }
        }
    }
    if class == BlockClass::Text
        && !trailing_white
        && start > 0
        && block_class(doc, start - 1) == BlockClass::Empty
    {
        while start > 0 && block_class(doc, start - 1) == BlockClass::Empty {
            start -= 1;
        }
    }
    Some(Lines::Blocks(start..end))
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

fn straight_quotes(t: &str, ls: usize, le: usize, o: usize, q: char) -> Option<Quoted> {
    let mut marks: Vec<usize> = Vec::new();
    let mut escaped = false;
    for (i, c) in t[ls..le].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
        } else if c == q {
            marks.push(ls + i);
        }
    }
    let before = marks.iter().filter(|m| **m < o).count();
    let on_quote = marks.contains(&o);
    let open_idx = if on_quote {
        // An even number of quotes before it: it opens; odd: it closes.
        if before % 2 == 0 { before } else { before - 1 }
    } else if before % 2 == 1 {
        before - 1
    } else {
        // Not inside a string: the next one on the line.
        before
    };
    let open = *marks.get(open_idx)?;
    let close = *marks.get(open_idx + 1)?;
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

fn curly_quotes(t: &str, ls: usize, le: usize, o: usize, open: char, close: char) -> Option<Quoted> {
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
pub(crate) fn quote(doc: &Document, at: Position, q: char, around: bool) -> Option<TextRange> {
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
                if a.close - a.open <= b.close - b.open { a } else { b }
            }
            (false, false) => {
                if a.open <= b.open { a } else { b }
            }
        },
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => return None,
    };
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

/// The `count`th pair of `open` / `close` around offset `o` in `t`: the
/// offsets of both brackets.
fn enclosing(t: &str, o: usize, open: char, close: char, count: usize) -> Option<(usize, usize)> {
    let o = o.min(t.len());
    let under = t[o..].chars().next();
    // Scan back for the opening bracket; a caret on the closing bracket is
    // inside its pair.
    let mut scan_end = match under {
        Some(c) if c == open => o + c.len_utf8(),
        _ => o,
    };
    let mut open_at = None;
    for _ in 0..count.max(1) {
        let mut depth = 0usize;
        let mut found = None;
        for (i, c) in t[..scan_end].char_indices().rev() {
            if c == close {
                depth += 1;
            } else if c == open {
                if depth == 0 {
                    found = Some(i);
                    break;
                }
                depth -= 1;
            }
        }
        let at = found?;
        open_at = Some(at);
        scan_end = at;
    }
    let open_at = open_at?;
    let mut depth = 0usize;
    for (i, c) in t[open_at..].char_indices() {
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some((open_at, open_at + i));
            }
        }
    }
    None
}

/// `i(` / `a(` and the same for `[ ]`, `{ }` and `< >`, in the block of
/// `at`. With `around` the brackets are included.
pub(crate) fn bracket(
    doc: &Document,
    at: Position,
    open: char,
    close: char,
    around: bool,
    count: usize,
) -> Option<TextRange> {
    let t = text_of(doc, at.block);
    let (o, c) = enclosing(t, at.offset, open, close, count)?;
    Some(if around {
        TextRange::in_block(at.block, o, c + close.len_utf8())
    } else {
        TextRange::in_block(at.block, o + open.len_utf8(), c)
    })
}

/// Like [`bracket`], but for a pair strictly larger than `current` (used to
/// grow a Visual selection that already covers a pair).
pub(crate) fn bracket_growing(
    doc: &Document,
    current: TextRange,
    open: char,
    close: char,
    around: bool,
) -> Option<TextRange> {
    let at = current.start;
    for count in 1..=64 {
        let r = bracket(doc, at, open, close, around, count)?;
        if r.start <= current.start && r.end >= current.end && r != current {
            return Some(r);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use writer_core::Block;

    fn doc(s: &str) -> Document {
        Document::from_blocks(vec![Block::paragraph(s)])
    }

    const OPTS: ClassOpts = ClassOpts {
        big: false,
        apostrophe: true,
    };

    fn r(d: &Document, x: Option<TextRange>) -> String {
        x.map(|x| d.text_in(x)).unwrap_or_else(|| "<none>".into())
    }

    #[test]
    fn words() {
        let d = doc("  foo bar.baz  qux");
        let at = |o| Position::new(0, o);
        assert_eq!(r(&d, word(&d, at(3), false, 1, OPTS)), "foo");
        assert_eq!(r(&d, word(&d, at(3), true, 1, OPTS)), "foo ");
        assert_eq!(r(&d, word(&d, at(5), false, 1, OPTS)), " ");
        assert_eq!(r(&d, word(&d, at(5), true, 1, OPTS)), " bar");
        assert_eq!(r(&d, word(&d, at(9), false, 1, OPTS)), ".");
        assert_eq!(r(&d, word(&d, at(15), true, 1, OPTS)), "  qux");
        assert_eq!(r(&d, word(&d, at(3), false, 3, OPTS)), "foo bar");
        assert_eq!(r(&d, word(&d, at(3), true, 2, OPTS)), "foo bar");
        let big = ClassOpts { big: true, ..OPTS };
        assert_eq!(r(&d, word(&d, at(7), false, 1, big)), "bar.baz");
        // The indent is not taken as leading white space.
        let d = doc("  foo");
        assert_eq!(r(&d, word(&d, at(3), true, 1, OPTS)), "foo");
        let d = doc("don\u{2019}t stop");
        assert_eq!(r(&d, word(&d, at(1), false, 1, OPTS)), "don\u{2019}t");
    }

    #[test]
    fn quotes_and_brackets() {
        let d = doc("say \"hi there\" and \u{201C}curly one\u{201D} end");
        let at = |o| Position::new(0, o);
        assert_eq!(r(&d, quote(&d, at(6), '"', false)), "hi there");
        assert_eq!(r(&d, quote(&d, at(6), '"', true)), "\"hi there\" ");
        assert_eq!(r(&d, quote(&d, at(0), '"', false)), "hi there");
        assert_eq!(r(&d, quote(&d, at(25), '"', false)), "curly one");
        assert_eq!(r(&d, quote(&d, at(25), '"', true)), "\u{201C}curly one\u{201D} ");
        let d = doc("f(a, (b + c), d) [x]");
        assert_eq!(r(&d, bracket(&d, at(7), '(', ')', false, 1)), "b + c");
        assert_eq!(r(&d, bracket(&d, at(7), '(', ')', true, 1)), "(b + c)");
        assert_eq!(r(&d, bracket(&d, at(7), '(', ')', false, 2)), "a, (b + c), d");
        assert_eq!(r(&d, bracket(&d, at(1), '(', ')', false, 1)), "a, (b + c), d");
        assert_eq!(r(&d, bracket(&d, at(15), '(', ')', false, 1)), "a, (b + c), d");
        assert_eq!(r(&d, bracket(&d, at(18), '[', ']', false, 1)), "x");
        assert_eq!(r(&d, bracket(&d, at(0), '(', ')', false, 1)), "<none>");
        let d = doc("it\u{2019}s \u{2018}a don\u{2019}t b\u{2019} x");
        let o = d.blocks()[0].text().find("don").unwrap();
        assert_eq!(r(&d, quote(&d, at(o), '\'', false)), "a don\u{2019}t b");
    }

    #[test]
    fn sentence_objects() {
        let d = doc("One. Two two.  Three");
        let at = |o| Position::new(0, o);
        assert_eq!(r(&d, sentence(&d, at(6), false, 1)), "Two two.");
        assert_eq!(r(&d, sentence(&d, at(6), true, 1)), "Two two.  ");
        assert_eq!(r(&d, sentence(&d, at(16), true, 1)), "  Three");
        assert_eq!(r(&d, sentence(&d, at(4), false, 1)), " ");
        assert_eq!(r(&d, sentence(&d, at(4), true, 1)), " Two two.");
        assert_eq!(r(&d, sentence(&d, at(0), true, 2)), "One. Two two.  ");
    }
}
