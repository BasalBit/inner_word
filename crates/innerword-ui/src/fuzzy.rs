//! Fuzzy subsequence matching with scoring and match positions.
//!
//! Every query character must appear in the candidate in order (case
//! insensitive). Among all such alignments the best scoring one is chosen
//! with a small dynamic program, so "nd" in "New document" matches the "N"
//! and the "d" at word starts rather than the "n" and "d" inside "document".
//!
//! Scoring favours, in order: matches at word starts (after a space, `-`,
//! `_`, `/`, `.`, ...) and camelCase humps, consecutive runs, matches near the
//! start, exact case; gaps cost a little. Whitespace in the query is ignored
//! for matching, so "new doc" matches "New document".
//!
//! ```
//! use innerword_ui::fuzzy::{fuzzy_match, match_strings};
//!
//! let m = fuzzy_match("nd", "New document").unwrap();
//! assert_eq!(m.positions, vec![0, 4]);          // byte offsets into the candidate
//!
//! let names = ["Charter", "Georgia", "Menlo", "Helvetica Neue"];
//! let ranked = match_strings("ge", names.iter().copied(), 10);
//! assert_eq!(ranked[0].candidate_index, 1);     // "Georgia"
//! ```
//!
//! For display, pass `positions` to [`crate::label::HighlightedLabel`].

/// One successful match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuzzyMatch {
    /// Higher is better. Only comparable between matches of the same query.
    pub score: i32,
    /// Byte offsets (char starts) of the matched characters in the candidate,
    /// ascending.
    pub positions: Vec<usize>,
}

/// A ranked match from [`match_strings`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StringMatch {
    /// Index of the candidate in the input iterator.
    pub candidate_index: usize,
    pub score: i32,
    pub positions: Vec<usize>,
}

const SCORE_MATCH: i32 = 16;
const BONUS_START: i32 = 12;
const BONUS_BOUNDARY: i32 = 10;
const BONUS_CAMEL: i32 = 8;
const BONUS_CONSECUTIVE: i32 = 8;
const BONUS_EXACT_CASE: i32 = 1;
const GAP_START: i32 = 3;
const GAP_EXTEND: i32 = 1;
const LEADING_PENALTY_MAX: i32 = 3;
/// Above this many candidate characters the DP is skipped and a greedy
/// alignment is used, to keep pathological inputs cheap.
const MAX_DP_CELLS: usize = 64 * 1024;

fn is_separator(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            '-' | '_' | '/' | '\\' | '.' | ',' | ':' | ';' | '(' | ')' | '[' | ']' | '"' | '\''
        )
}

fn fold(ch: char) -> char {
    // Single-char lowercase mapping keeps char indices aligned.
    let mut lower = ch.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(l), None) => l,
        _ => ch,
    }
}

/// Matches `query` against `candidate`. Returns `None` unless every
/// non-whitespace query character appears in order. An empty query matches
/// everything with score 0 and no positions.
pub fn fuzzy_match(query: &str, candidate: &str) -> Option<FuzzyMatch> {
    let query: Vec<char> = query.chars().filter(|ch| !ch.is_whitespace()).collect();
    if query.is_empty() {
        return Some(FuzzyMatch {
            score: 0,
            positions: Vec::new(),
        });
    }
    let chars: Vec<(usize, char)> = candidate.char_indices().collect();
    let folded: Vec<char> = chars.iter().map(|(_, ch)| fold(*ch)).collect();
    let query_folded: Vec<char> = query.iter().map(|ch| fold(*ch)).collect();

    // Quick reject: plain subsequence check.
    {
        let mut qi = 0;
        for ch in &folded {
            if qi < query_folded.len() && *ch == query_folded[qi] {
                qi += 1;
            }
        }
        if qi < query_folded.len() {
            return None;
        }
    }

    let n = chars.len();
    let m = query.len();
    let bonus: Vec<i32> = (0..n)
        .map(|j| {
            let ch = chars[j].1;
            if j == 0 {
                BONUS_START
            } else {
                let prev = chars[j - 1].1;
                if is_separator(prev) && !is_separator(ch) {
                    BONUS_BOUNDARY
                } else if prev.is_lowercase() && ch.is_uppercase() {
                    BONUS_CAMEL
                } else if !prev.is_ascii_digit() && ch.is_ascii_digit() {
                    BONUS_CAMEL / 2
                } else {
                    0
                }
            }
        })
        .collect();
    let cell = |i: usize, j: usize| -> i32 {
        SCORE_MATCH
            + bonus[j]
            + if chars[j].1 == query[i] {
                BONUS_EXACT_CASE
            } else {
                0
            }
    };

    if n * m > MAX_DP_CELLS {
        return Some(greedy(&chars, &folded, &query_folded, &cell));
    }

    const NONE: i32 = i32::MIN / 4;
    // score[i * n + j]: best score with query[i] matched at candidate j.
    let mut score = vec![NONE; n * m];
    // back[i * n + j]: candidate index of query[i - 1] in that best alignment.
    let mut back = vec![u32::MAX; n * m];

    for j in 0..n {
        if folded[j] == query_folded[0] {
            score[j] = cell(0, j) - (j as i32).min(LEADING_PENALTY_MAX);
        }
    }
    for i in 1..m {
        // best over k <= j - 2 of score[i-1][k] - gap(j - k - 1)
        let mut gap_best = NONE;
        let mut gap_arg = u32::MAX;
        for j in i..n {
            if j >= 2 {
                let k = j - 2;
                let from_k = score[(i - 1) * n + k];
                let extended = gap_best - GAP_EXTEND;
                if from_k > NONE && from_k - GAP_START >= extended {
                    gap_best = from_k - GAP_START;
                    gap_arg = k as u32;
                } else {
                    gap_best = extended;
                }
            }
            if folded[j] != query_folded[i] {
                continue;
            }
            let consecutive = score[(i - 1) * n + j - 1];
            let mut best = NONE;
            let mut arg = u32::MAX;
            if consecutive > NONE {
                best = consecutive + BONUS_CONSECUTIVE;
                arg = (j - 1) as u32;
            }
            if gap_arg != u32::MAX && gap_best > best {
                best = gap_best;
                arg = gap_arg;
            }
            if arg != u32::MAX {
                score[i * n + j] = best + cell(i, j);
                back[i * n + j] = arg;
            }
        }
    }

    let last = m - 1;
    let (end, best) = (0..n)
        .map(|j| (j, score[last * n + j]))
        .filter(|(_, s)| *s > NONE)
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))?;
    let mut positions = vec![0usize; m];
    let mut j = end;
    for i in (0..m).rev() {
        positions[i] = chars[j].0;
        if i > 0 {
            j = back[i * n + j] as usize;
        }
    }
    Some(FuzzyMatch {
        score: best,
        positions,
    })
}

fn greedy(
    chars: &[(usize, char)],
    folded: &[char],
    query_folded: &[char],
    cell: &dyn Fn(usize, usize) -> i32,
) -> FuzzyMatch {
    let mut positions = Vec::with_capacity(query_folded.len());
    let mut score = 0;
    let mut qi = 0;
    let mut last: Option<usize> = None;
    for (j, ch) in folded.iter().enumerate() {
        if qi < query_folded.len() && *ch == query_folded[qi] {
            score += cell(qi, j);
            if let Some(prev) = last {
                if prev + 1 == j {
                    score += BONUS_CONSECUTIVE;
                } else {
                    score -= GAP_START + GAP_EXTEND * (j - prev - 2) as i32;
                }
            }
            positions.push(chars[j].0);
            last = Some(j);
            qi += 1;
        }
    }
    FuzzyMatch { score, positions }
}

/// Matches `query` against every candidate and returns the matches sorted
/// best first (ties: shorter candidate, then input order), at most `limit`.
/// An empty query returns every candidate in input order.
pub fn match_strings<'a>(
    query: &str,
    candidates: impl IntoIterator<Item = &'a str>,
    limit: usize,
) -> Vec<StringMatch> {
    let mut matches: Vec<(StringMatch, usize)> = candidates
        .into_iter()
        .enumerate()
        .filter_map(|(candidate_index, candidate)| {
            fuzzy_match(query, candidate).map(|m| {
                (
                    StringMatch {
                        candidate_index,
                        score: m.score,
                        positions: m.positions,
                    },
                    candidate.len(),
                )
            })
        })
        .collect();
    if !query.trim().is_empty() {
        matches.sort_by(|(a, a_len), (b, b_len)| {
            b.score
                .cmp(&a.score)
                .then(a_len.cmp(b_len))
                .then(a.candidate_index.cmp(&b.candidate_index))
        });
    }
    matches.truncate(limit);
    matches.into_iter().map(|(m, _)| m).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn positions(query: &str, candidate: &str) -> Option<Vec<usize>> {
        fuzzy_match(query, candidate).map(|m| m.positions)
    }

    #[test]
    fn subsequence_required() {
        assert!(fuzzy_match("abc", "a b c").is_some());
        assert!(fuzzy_match("acb", "abc").is_none());
        assert!(fuzzy_match("x", "").is_none());
        assert_eq!(
            fuzzy_match("", "anything").unwrap().positions,
            Vec::<usize>::new()
        );
    }

    #[test]
    fn prefers_word_starts() {
        assert_eq!(positions("nd", "New document"), Some(vec![0, 4]));
        assert_eq!(positions("tp", "Toggle page view"), Some(vec![0, 7]));
        assert_eq!(positions("ff", "Find in files"), Some(vec![0, 8]));
        // camelCase humps
        assert_eq!(positions("sf", "showFolders"), Some(vec![0, 4]));
    }

    #[test]
    fn prefers_consecutive_runs() {
        assert_eq!(positions("doc", "a document"), Some(vec![2, 3, 4]));
        let run = fuzzy_match("char", "Charter").unwrap().score;
        let spread = fuzzy_match("char", "Cheap hard r").unwrap().score;
        assert!(run > spread);
    }

    #[test]
    fn case_insensitive_and_unicode() {
        assert_eq!(positions("CAFE", "café"), None); // é is not e
        assert_eq!(positions("CAFÉ", "café"), Some(vec![0, 1, 2, 3]));
        assert_eq!(positions("nai", "Naïve"), None);
        assert_eq!(positions("naï", "Naïve"), Some(vec![0, 1, 2]));
        // positions are byte offsets
        assert_eq!(positions("ve", "naïve"), Some(vec![4, 5]));
    }

    #[test]
    fn whitespace_in_query_is_ignored() {
        assert_eq!(
            positions("new doc", "New document"),
            Some(vec![0, 1, 2, 4, 5, 6])
        );
    }

    #[test]
    fn ranking() {
        let candidates = [
            "Toggle Vim mode",
            "Export document",
            "Open settings",
            "Settings: theme",
            "Set heading 1",
        ];
        let ranked = match_strings("set", candidates.iter().copied(), 10);
        let order: Vec<usize> = ranked.iter().map(|m| m.candidate_index).collect();
        // "Set heading 1" (prefix, shorter) and "Settings: theme" (prefix) beat
        // "Open settings" (word start later on).
        assert_eq!(&order[..2], &[4, 3]);
        assert_eq!(order[2], 2);
        assert!(!order.contains(&0));

        let all = match_strings("", candidates.iter().copied(), 3);
        assert_eq!(
            all.iter().map(|m| m.candidate_index).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn long_candidates_fall_back_to_greedy() {
        let long = "x".repeat(10_000) + "abc";
        let m = fuzzy_match("xxxxxxxabc", &long).unwrap();
        assert_eq!(m.positions.len(), 10);
        assert_eq!(*m.positions.last().unwrap(), long.len() - 1);
    }

    #[test]
    fn thousands_of_candidates_are_fast() {
        let candidates: Vec<String> = (0..5000)
            .map(|i| format!("Document number {i} in folder Drafts/Chapter {}", i % 37))
            .collect();
        let started = std::time::Instant::now();
        let ranked = match_strings("dcf", candidates.iter().map(String::as_str), usize::MAX);
        assert_eq!(ranked.len(), 5000);
        // Generous bound for debug builds on a busy machine.
        assert!(
            started.elapsed().as_millis() < 2000,
            "{:?}",
            started.elapsed()
        );
    }
}
