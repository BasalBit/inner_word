//! Run-length encoded inline attributes.

use std::ops::Range;

use crate::attrs::InlineAttrs;

/// A run of `len` bytes of a block's text sharing the same attributes.
///
/// A block's runs are always normalised: no run is empty, two adjacent runs
/// never have equal attributes, and the lengths sum to the text length (an
/// empty text has no runs).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Run {
    /// Length in UTF-8 bytes.
    pub len: usize,
    /// The attributes of the run.
    pub attrs: InlineAttrs,
}

/// Total length covered by `runs`.
pub(crate) fn total(runs: &[Run]) -> usize {
    runs.iter().map(|r| r.len).sum()
}

/// Drops empty runs and merges adjacent equal runs.
pub(crate) fn normalize(runs: &mut Vec<Run>) {
    let mut out: Vec<Run> = Vec::with_capacity(runs.len());
    for r in runs.drain(..) {
        if r.len == 0 {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.attrs == r.attrs => last.len += r.len,
            _ => out.push(r),
        }
    }
    *runs = out;
}

/// Whether `runs` satisfy the normalisation invariant for a text of `len`.
pub(crate) fn is_normalized(runs: &[Run], len: usize) -> bool {
    if total(runs) != len {
        return false;
    }
    if runs.iter().any(|r| r.len == 0) {
        return false;
    }
    runs.windows(2).all(|w| w[0].attrs != w[1].attrs)
}

/// Ensures a run boundary at `offset` and returns the index of the first run
/// starting at or after `offset`.
fn split_at(runs: &mut Vec<Run>, offset: usize) -> usize {
    let mut pos = 0;
    for i in 0..runs.len() {
        if pos == offset {
            return i;
        }
        let end = pos + runs[i].len;
        if offset < end {
            let tail = Run {
                len: end - offset,
                attrs: runs[i].attrs.clone(),
            };
            runs[i].len = offset - pos;
            runs.insert(i + 1, tail);
            return i + 1;
        }
        pos = end;
    }
    runs.len()
}

/// Inserts `len` bytes with `attrs` at `offset`.
pub(crate) fn insert(runs: &mut Vec<Run>, offset: usize, len: usize, attrs: &InlineAttrs) {
    if len == 0 {
        return;
    }
    let i = split_at(runs, offset);
    runs.insert(
        i,
        Run {
            len,
            attrs: attrs.clone(),
        },
    );
    normalize(runs);
}

/// Inserts a sequence of runs at `offset`.
pub(crate) fn insert_runs(runs: &mut Vec<Run>, offset: usize, new: &[Run]) {
    if new.is_empty() {
        return;
    }
    let i = split_at(runs, offset);
    runs.splice(i..i, new.iter().cloned());
    normalize(runs);
}

/// Removes the bytes in `range`.
pub(crate) fn remove(runs: &mut Vec<Run>, range: Range<usize>) {
    if range.start >= range.end {
        return;
    }
    let start = split_at(runs, range.start);
    let end = split_at(runs, range.end);
    runs.drain(start..end);
    normalize(runs);
}

/// Copies the runs covering `range`.
pub(crate) fn slice(runs: &[Run], range: Range<usize>) -> Vec<Run> {
    let mut out = Vec::new();
    let mut pos = 0;
    for r in runs {
        let end = pos + r.len;
        let s = pos.max(range.start);
        let e = end.min(range.end);
        if s < e {
            out.push(Run {
                len: e - s,
                attrs: r.attrs.clone(),
            });
        }
        pos = end;
        if pos >= range.end {
            break;
        }
    }
    out
}

/// Applies `f` to the attributes of every byte in `range`.
pub(crate) fn apply(runs: &mut Vec<Run>, range: Range<usize>, mut f: impl FnMut(&mut InlineAttrs)) {
    if range.start >= range.end {
        return;
    }
    let start = split_at(runs, range.start);
    let end = split_at(runs, range.end);
    for r in &mut runs[start..end] {
        f(&mut r.attrs);
    }
    normalize(runs);
}

/// The attributes of the byte at `offset` (the run containing it).
pub(crate) fn attrs_at(runs: &[Run], offset: usize) -> Option<&InlineAttrs> {
    let mut pos = 0;
    for r in runs {
        let end = pos + r.len;
        if offset < end {
            return Some(&r.attrs);
        }
        pos = end;
    }
    None
}

/// Iterates `(byte range, attrs)` for every run.
pub(crate) fn spans(runs: &[Run]) -> impl Iterator<Item = (Range<usize>, &InlineAttrs)> {
    runs.iter().scan(0usize, |pos, r| {
        let start = *pos;
        *pos += r.len;
        Some((start..*pos, &r.attrs))
    })
}

/// Iterates the runs overlapping `range`, clipped to it.
pub(crate) fn spans_in(
    runs: &[Run],
    range: Range<usize>,
) -> impl Iterator<Item = (Range<usize>, &InlineAttrs)> {
    spans(runs).filter_map(move |(r, a)| {
        let s = r.start.max(range.start);
        let e = r.end.min(range.end);
        (s < e).then_some((s..e, a))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bold() -> InlineAttrs {
        InlineAttrs {
            bold: true,
            ..Default::default()
        }
    }

    fn plain(len: usize) -> Run {
        Run {
            len,
            attrs: InlineAttrs::default(),
        }
    }

    #[test]
    fn insert_and_merge() {
        let mut runs = vec![plain(5)];
        insert(&mut runs, 2, 3, &bold());
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].len, 2);
        assert_eq!(runs[1].len, 3);
        assert!(runs[1].attrs.bold);
        assert_eq!(runs[2].len, 3);
        insert(&mut runs, 2, 1, &InlineAttrs::default());
        assert_eq!(runs[0].len, 3);
        assert!(is_normalized(&runs, 9));
    }

    #[test]
    fn remove_merges_neighbours() {
        let mut runs = vec![plain(2), Run { len: 3, attrs: bold() }, plain(2)];
        remove(&mut runs, 2..5);
        assert_eq!(runs, vec![plain(4)]);
        remove(&mut runs, 0..4);
        assert!(runs.is_empty());
    }

    #[test]
    fn apply_and_slice() {
        let mut runs = vec![plain(10)];
        apply(&mut runs, 3..6, |a| a.bold = true);
        assert_eq!(slice(&runs, 2..7).len(), 3);
        assert_eq!(attrs_at(&runs, 3).map(|a| a.bold), Some(true));
        assert_eq!(attrs_at(&runs, 6).map(|a| a.bold), Some(false));
        assert_eq!(attrs_at(&runs, 10), None);
        apply(&mut runs, 0..10, |a| a.bold = false);
        assert_eq!(runs, vec![plain(10)]);
        let spans: Vec<_> = spans_in(&runs, 2..4).map(|(r, _)| r).collect();
        assert_eq!(spans, vec![2..4]);
    }
}
