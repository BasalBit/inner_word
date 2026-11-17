//! A simulation of how a CommonMark parser pairs emphasis delimiters.
//!
//! Whether `**` opens or closes emphasis depends on the characters around
//! it (the flanking rules), on other delimiter runs it touches (`***`), and
//! on the "rule of three". Instead of trusting simple placement rules, the
//! exporter runs the parser's algorithm (CommonMark spec, "phase 2: inline
//! structure", `process emphasis`) over the delimiters it is about to
//! write, and checks that every pair comes out as intended.
//!
//! Parsers disagree on one point: since CommonMark 0.31 symbols such as
//! `€` or an emoji count as punctuation next to a delimiter; before (and on
//! GitHub) they do not. The check runs once for each reading and both must
//! agree with the intent.

use super::escape::{CharClass, char_class};

/// One rendered token, as far as delimiter pairing is concerned.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Piece {
    /// An emphasis delimiter: `*`, `**` or `~~`.
    Delimiter { ch: char, len: usize },
    /// The `[` of a link.
    LinkOpen,
    /// The `](url)` of a link.
    LinkClose,
    /// Anything else, with its first and last character.
    Other {
        first: Option<char>,
        last: Option<char>,
    },
}

/// A maximal run of the same delimiter character.
#[derive(Clone, Copy, Debug)]
struct Run {
    ch: char,
    len: usize,
    before: Option<char>,
    after: Option<char>,
}

#[derive(Clone, Copy, Debug)]
enum Entry {
    Bracket,
    Run {
        id: usize,
        ch: char,
        remaining: usize,
        orig: usize,
        can_open: bool,
        can_close: bool,
    },
}

fn flanking(run: &Run, symbols_are_punctuation: bool) -> (bool, bool) {
    let class = |c: Option<char>| match char_class(c) {
        CharClass::Symbol if symbols_are_punctuation => CharClass::Punctuation,
        CharClass::Symbol => CharClass::Other,
        c => c,
    };
    let before = class(run.before);
    let after = class(run.after);
    let ws = |c: CharClass| c == CharClass::Whitespace;
    let punct = |c: CharClass| c == CharClass::Punctuation;
    let left = !ws(after) && (!punct(after) || ws(before) || punct(before));
    let right = !ws(before) && (!punct(before) || ws(after) || punct(after));
    (left, right)
}

/// The spec's `process emphasis` for the entries above `bottom`. Appends
/// `(opener run, closer run, delimiters used)` for every pair.
fn process(stack: &mut Vec<Entry>, bottom: usize, pairs: &mut Vec<(usize, usize, usize)>) {
    let mut ci = bottom;
    while ci < stack.len() {
        let Entry::Run {
            id: closer_id,
            ch,
            remaining: closer_left,
            orig: closer_orig,
            can_open: closer_opens,
            can_close: true,
        } = stack[ci]
        else {
            ci += 1;
            continue;
        };
        let mut found = None;
        for oi in (bottom..ci).rev() {
            let Entry::Run {
                ch: och,
                remaining,
                orig,
                can_open: true,
                can_close: opener_closes,
                ..
            } = stack[oi]
            else {
                continue;
            };
            if och != ch || remaining == 0 {
                continue;
            }
            if ch == '~' {
                // GitHub pairs tilde runs only when their lengths match.
                if orig != closer_orig {
                    continue;
                }
            } else if (opener_closes || closer_opens)
                && (orig + closer_orig) % 3 == 0
                && !(orig % 3 == 0 && closer_orig % 3 == 0)
            {
                // The rule of three.
                continue;
            }
            found = Some(oi);
            break;
        }
        let Some(oi) = found else {
            if closer_opens {
                ci += 1;
            } else {
                stack.remove(ci);
            }
            continue;
        };
        let Entry::Run {
            id: opener_id,
            remaining: opener_left,
            ..
        } = stack[oi]
        else {
            ci += 1;
            continue;
        };
        let used = if ch == '~' {
            closer_left.min(opener_left)
        } else if closer_left >= 2 && opener_left >= 2 {
            2
        } else {
            1
        };
        pairs.push((opener_id, closer_id, used));
        // Everything between the two is no longer a delimiter.
        stack.drain(oi + 1..ci);
        ci = oi + 1;
        if let Entry::Run { remaining, .. } = &mut stack[ci] {
            *remaining -= used;
        }
        if let Entry::Run { remaining, .. } = &mut stack[oi] {
            *remaining -= used;
        }
        if matches!(stack[ci], Entry::Run { remaining: 0, .. }) {
            stack.remove(ci);
        }
        if matches!(stack[oi], Entry::Run { remaining: 0, .. }) {
            stack.remove(oi);
            ci -= 1;
        }
    }
}

/// Checks the delimiters in `pieces` against the intent: mark `m` is meant
/// to open at token `ends[m].0` and close at token `ends[m].1`. Returns the
/// first mark a parser would not pair that way, or `None` when all is well.
#[cfg(test)]
pub(crate) fn first_mismatch(pieces: &[Piece], ends: &[Option<(usize, usize)>]) -> Option<usize> {
    mismatches(pieces, ends, false).first().copied()
}

/// Like `first_mismatch`, as a list: the first mark a parser would not
/// pair as intended, or with `all` every such mark (in either reading of
/// symbols). Empty when all is well.
pub(crate) fn mismatches(
    pieces: &[Piece],
    ends: &[Option<(usize, usize)>],
    all: bool,
) -> Vec<usize> {
    // Group the delimiter tokens into runs.
    let mut runs: Vec<Run> = Vec::new();
    let mut run_of: Vec<Option<usize>> = vec![None; pieces.len()];
    let edge_before = |i: usize| -> Option<char> {
        pieces[..i].iter().rev().find_map(|p| match p {
            Piece::Delimiter { ch, .. } => Some(*ch),
            Piece::LinkOpen => Some('['),
            Piece::LinkClose => Some(')'),
            Piece::Other { last, .. } => *last,
        })
    };
    let edge_after = |i: usize| -> Option<char> {
        pieces[i..].iter().find_map(|p| match p {
            Piece::Delimiter { ch, .. } => Some(*ch),
            Piece::LinkOpen => Some('['),
            Piece::LinkClose => Some(']'),
            Piece::Other { first, .. } => *first,
        })
    };
    let mut i = 0;
    while i < pieces.len() {
        let Piece::Delimiter { ch, .. } = pieces[i] else {
            i += 1;
            continue;
        };
        let start = i;
        let mut len = 0;
        while let Some(Piece::Delimiter { ch: c, len: l, .. }) = pieces.get(i)
            && *c == ch
        {
            run_of[i] = Some(runs.len());
            len += l;
            i += 1;
        }
        runs.push(Run {
            ch,
            len,
            before: edge_before(start),
            after: edge_after(i),
        });
    }
    if runs.is_empty() {
        return Vec::new();
    }
    let mut bad: Vec<usize> = Vec::new();

    let mut intended: Vec<(usize, (usize, usize, usize))> = Vec::new();
    for (mark, e) in ends.iter().enumerate() {
        let Some((open, close)) = *e else { continue };
        if let (Some(Piece::Delimiter { len, .. }), Some(o), Some(c)) =
            (pieces.get(open), run_of[open], run_of[close])
        {
            intended.push((mark, (o, c, *len)));
        }
    }

    for symbols_are_punctuation in [true, false] {
        let mut stack: Vec<Entry> = Vec::new();
        let mut pairs = Vec::new();
        for (i, piece) in pieces.iter().enumerate() {
            match piece {
                Piece::LinkOpen => stack.push(Entry::Bracket),
                Piece::LinkClose => {
                    if let Some(b) = stack.iter().rposition(|e| matches!(e, Entry::Bracket)) {
                        process(&mut stack, b + 1, &mut pairs);
                        stack.truncate(b);
                    }
                }
                Piece::Delimiter { .. } => {
                    let Some(id) = run_of[i] else { continue };
                    // Push a run once, at its first token.
                    if i > 0 && run_of[i - 1] == Some(id) {
                        continue;
                    }
                    let run = &runs[id];
                    let (can_open, can_close) = flanking(run, symbols_are_punctuation);
                    stack.push(Entry::Run {
                        id,
                        ch: run.ch,
                        remaining: run.len,
                        orig: run.len,
                        can_open,
                        can_close,
                    });
                }
                Piece::Other { .. } => {}
            }
        }
        process(&mut stack, 0, &mut pairs);

        let before = bad.len();
        if all {
            // Many marks: look the pairs up instead of searching them.
            let mut found: std::collections::HashMap<(usize, usize, usize), usize> =
                std::collections::HashMap::new();
            for p in &pairs {
                *found.entry(*p).or_default() += 1;
            }
            let mut matched = 0;
            for (mark, want) in &intended {
                match found.get_mut(want) {
                    Some(n) if *n > 0 => {
                        *n -= 1;
                        matched += 1;
                    }
                    _ => bad.push(*mark),
                }
            }
            if bad.len() == before && matched < pairs.len() {
                bad.extend(intended.first().map(|(mark, _)| *mark));
            }
        } else {
            for (mark, want) in &intended {
                match pairs.iter().position(|p| p == want) {
                    Some(found) => {
                        pairs.swap_remove(found);
                    }
                    None => return vec![*mark],
                }
            }
            if !pairs.is_empty() {
                return intended
                    .first()
                    .map(|(mark, _)| *mark)
                    .into_iter()
                    .collect();
            }
        }
    }
    bad.sort_unstable();
    bad.dedup();
    bad
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds pieces from a compact description: `B(`/`)B` bold, `I(`/`)I`
    /// italic, `S(`/`)S` strike, anything else is text.
    fn check(parts: &[&str]) -> Option<usize> {
        let mut pieces = Vec::new();
        let mut ends: Vec<Option<(usize, usize)>> = Vec::new();
        let mut open: Vec<(char, usize)> = Vec::new();
        for (i, p) in parts.iter().enumerate() {
            let delim = |k: char| match k {
                'B' => ('*', 2),
                'I' => ('*', 1),
                _ => ('~', 2),
            };
            let b = p.as_bytes();
            if p.len() == 2 && b[1] == b'(' {
                let (ch, len) = delim(b[0] as char);
                open.push((b[0] as char, ends.len()));
                ends.push(Some((i, 0)));
                pieces.push(Piece::Delimiter { ch, len });
            } else if p.len() == 2 && b[0] == b')' {
                let (ch, len) = delim(b[1] as char);
                let (_, mark) = open.pop().unwrap();
                ends[mark] = Some((ends[mark].unwrap().0, i));
                pieces.push(Piece::Delimiter { ch, len });
            } else if *p == "[" {
                pieces.push(Piece::LinkOpen);
            } else if *p == "]" {
                pieces.push(Piece::LinkClose);
            } else {
                pieces.push(Piece::Other {
                    first: p.chars().next(),
                    last: p.chars().next_back(),
                });
            }
        }
        first_mismatch(&pieces, &ends)
    }

    #[test]
    fn simple_pairs_are_fine() {
        assert_eq!(check(&["a ", "B(", "b", ")B", " c"]), None);
        assert_eq!(check(&["B(", "b", ")B"]), None);
        assert_eq!(check(&["I(", "B(", "b", ")B", ")I"]), None);
        assert_eq!(check(&["I(", "a ", "B(", "b", ")B", " c", ")I"]), None);
        assert_eq!(check(&["S(", "B(", "b", ")B", ")S", "."]), None);
        assert_eq!(check(&["(", "B(", "b", ")B", ")"]), None);
        assert_eq!(check(&["“", "B(", "b", ")B", "”"]), None);
    }

    #[test]
    fn intraword() {
        assert_eq!(check(&["un", "B(", "believ", ")B", "able"]), None);
        assert_eq!(check(&["a", "I(", "b", ")I", "c"]), None);
        // Italic inside bold inside a word, and the other way round.
        assert_eq!(check(&["B(", "a", "I(", "b", ")I", "c", ")B"]), None);
        assert_eq!(check(&["I(", "a", "B(", "b", ")B", "c", ")I"]), None);
    }

    #[test]
    fn punctuation_next_to_letters_fails() {
        // a**(b)**c: neither delimiter flanks the right way.
        assert_eq!(check(&["a", "B(", "(b)", ")B", "c"]), Some(0));
        assert_eq!(check(&["B(", "b!", ")B", "c"]), Some(0));
        assert_eq!(check(&["a", "B(", "`", ")B"]), Some(0));
        // Fine when the outside is a space or punctuation too.
        assert_eq!(check(&["a ", "B(", "(b)", ")B", ", c"]), None);
    }

    #[test]
    fn symbols_must_work_in_both_readings() {
        // Emoji after a letter: punctuation for new parsers, a letter for old.
        assert_eq!(check(&["a", "B(", "😀b", ")B"]), Some(0));
        assert_eq!(check(&["a ", "B(", "😀b", ")B"]), None);
        assert_eq!(check(&["B(", "b€", ")B", "x"]), Some(0));
    }

    #[test]
    fn merged_runs() {
        // **a***b*: bold then italic, sharing a run of three.
        assert_eq!(check(&["B(", "a", ")B", "I(", "b", ")I"]), None);
        // ***a*b**: italic nested at the start of bold.
        assert_eq!(check(&["B(", "I(", "a", ")I", "b", ")B"]), None);
        // ***aaa*bb*ccc*dd**: the second italic would pair with the opening run.
        assert!(check(&["B(", "I(", "aaa", ")I", "bb", "I(", "ccc", ")I", "dd", ")B"]).is_some());
    }

    #[test]
    fn links_are_separate_scopes() {
        assert_eq!(
            check(&["B(", "a ", "[", "I(", "b", ")I", "]", " c", ")B"]),
            None
        );
        assert_eq!(check(&["[", "B(", "b", ")B", "]"]), None);
    }
}
