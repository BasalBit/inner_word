//! Which items of an old sequence are still there in a new one.

use std::collections::HashMap;
use std::hash::Hash;
use std::ops::Range;

/// The largest table the exact comparison fills (cells); beyond it only items that occur
/// once on each side are used as fixed points.
const EXACT_CELLS: usize = 1_000_000;

/// The pairs `(old index, new index)` of items that are the same in both sequences, in
/// order: a longest common subsequence where the sequences are small, one found around
/// the items that occur exactly once on each side where they are long. Everything not in
/// a pair was removed (old) or added (new).
pub(crate) fn common<K: Eq + Hash>(old: &[K], new: &[K]) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    range(old, new, 0..old.len(), 0..new.len(), &mut pairs);
    pairs
}

fn range<K: Eq + Hash>(
    old: &[K],
    new: &[K],
    mut o: Range<usize>,
    mut n: Range<usize>,
    pairs: &mut Vec<(usize, usize)>,
) {
    while o.start < o.end && n.start < n.end && old[o.start] == new[n.start] {
        pairs.push((o.start, n.start));
        o.start += 1;
        n.start += 1;
    }
    let mut tail = Vec::new();
    while o.start < o.end && n.start < n.end && old[o.end - 1] == new[n.end - 1] {
        o.end -= 1;
        n.end -= 1;
        tail.push((o.end, n.end));
    }
    if !o.is_empty() && !n.is_empty() {
        if o.len().saturating_mul(n.len()) <= EXACT_CELLS {
            exact(old, new, o, n, pairs);
        } else {
            let anchors = unique_anchors(old, new, o.clone(), n.clone());
            let mut at = (o.start, n.start);
            for (old_index, new_index) in anchors {
                range(old, new, at.0..old_index, at.1..new_index, pairs);
                pairs.push((old_index, new_index));
                at = (old_index + 1, new_index + 1);
            }
            // Without a single fixed point the stretch is taken as rewritten.
            if at != (o.start, n.start) {
                range(old, new, at.0..o.end, at.1..n.end, pairs);
            }
        }
    }
    pairs.extend(tail.into_iter().rev());
}

/// A longest common subsequence by the textbook table.
fn exact<K: Eq>(
    old: &[K],
    new: &[K],
    o: Range<usize>,
    n: Range<usize>,
    pairs: &mut Vec<(usize, usize)>,
) {
    let (rows, columns) = (o.len(), n.len());
    // lengths[i][j]: the longest common subsequence of old[o.start + i..] and
    // new[n.start + j..].
    let mut lengths = vec![0u32; (rows + 1) * (columns + 1)];
    let cell = |i: usize, j: usize| i * (columns + 1) + j;
    for i in (0..rows).rev() {
        for j in (0..columns).rev() {
            lengths[cell(i, j)] = if old[o.start + i] == new[n.start + j] {
                lengths[cell(i + 1, j + 1)] + 1
            } else {
                lengths[cell(i + 1, j)].max(lengths[cell(i, j + 1)])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < rows && j < columns {
        if old[o.start + i] == new[n.start + j] {
            pairs.push((o.start + i, n.start + j));
            i += 1;
            j += 1;
        } else if lengths[cell(i + 1, j)] >= lengths[cell(i, j + 1)] {
            i += 1;
        } else {
            j += 1;
        }
    }
}

/// The items that occur exactly once in each stretch, as the longest run of them that is
/// in the same order on both sides.
fn unique_anchors<K: Eq + Hash>(
    old: &[K],
    new: &[K],
    o: Range<usize>,
    n: Range<usize>,
) -> Vec<(usize, usize)> {
    // For each item: how often it occurs on each side, and where it was seen last.
    let mut seen: HashMap<&K, (usize, usize, usize, usize)> = HashMap::new();
    for index in o {
        let entry = seen.entry(&old[index]).or_insert((0, 0, 0, 0));
        entry.0 += 1;
        entry.1 = index;
    }
    for index in n.clone() {
        if let Some(entry) = seen.get_mut(&new[index]) {
            entry.2 += 1;
            entry.3 = index;
        }
    }
    // In the order of the new sequence, the old index of each such item.
    let mut candidates: Vec<(usize, usize)> = seen
        .values()
        .filter(|entry| entry.0 == 1 && entry.2 == 1)
        .map(|entry| (entry.3, entry.1))
        .collect();
    candidates.sort_unstable();
    // The longest increasing run of old indices (patience sorting).
    let mut tails: Vec<usize> = Vec::new();
    let mut previous: Vec<Option<usize>> = vec![None; candidates.len()];
    for (index, &(_, old_index)) in candidates.iter().enumerate() {
        let place = tails.partition_point(|&tail| candidates[tail].1 < old_index);
        if place > 0 {
            previous[index] = Some(tails[place - 1]);
        }
        if place == tails.len() {
            tails.push(index);
        } else {
            tails[place] = index;
        }
    }
    let mut run = Vec::with_capacity(tails.len());
    let mut next = tails.last().copied();
    while let Some(index) = next {
        run.push((candidates[index].1, candidates[index].0));
        next = previous[index];
    }
    run.reverse();
    run
}

#[cfg(test)]
mod tests {
    use super::*;

    fn letters(text: &str) -> Vec<char> {
        text.chars().collect()
    }

    fn check(old: &str, new: &str) -> usize {
        let (old, new) = (letters(old), letters(new));
        let pairs = common(&old, &new);
        for window in pairs.windows(2) {
            assert!(
                window[0].0 < window[1].0 && window[0].1 < window[1].1,
                "{pairs:?}"
            );
        }
        for &(o, n) in &pairs {
            assert_eq!(old[o], new[n]);
        }
        pairs.len()
    }

    #[test]
    fn what_is_common_is_kept() {
        assert_eq!(check("", ""), 0);
        assert_eq!(check("abc", ""), 0);
        assert_eq!(check("", "abc"), 0);
        assert_eq!(check("abc", "abc"), 3);
        assert_eq!(check("abcdef", "abXdef"), 5);
        assert_eq!(check("abcdef", "abdef"), 5);
        assert_eq!(check("abdef", "abcdef"), 5);
        assert_eq!(check("abcabba", "cbabac"), 4);
        assert_eq!(check("xyz", "abc"), 0);
        assert_eq!(check("aaaa", "aa"), 2);
    }

    #[test]
    fn long_sequences_are_compared_around_what_is_unique() {
        // 4,000 distinct items with a few changes: far beyond the exact table.
        let old: Vec<u32> = (0..4000).collect();
        let mut new = old.clone();
        new[10] = 100_000;
        new.remove(2000);
        new.insert(3000, 100_001);
        new.swap(500, 501);
        let started = std::time::Instant::now();
        let pairs = common(&old, &new);
        assert!(started.elapsed().as_secs() < 5);
        for window in pairs.windows(2) {
            assert!(window[0].0 < window[1].0 && window[0].1 < window[1].1);
        }
        for &(o, n) in &pairs {
            assert_eq!(old[o], new[n]);
        }
        // Everything but the changed, the removed and one of the swapped items.
        assert_eq!(pairs.len(), 4000 - 3);

        // Nothing unique at all: the middle counts as rewritten, the ends are kept.
        let old = vec![1u8; 3000];
        let mut new = vec![1u8; 3000];
        new[1500] = 2;
        assert_eq!(common(&old, &new).len(), 2999);
        let old: Vec<u32> = (0..2000).map(|i| i % 2).collect();
        let new: Vec<u32> = (0..2000).map(|i| (i + 1) % 2).collect();
        assert!(common(&old, &new).len() <= 2000);
    }
}
