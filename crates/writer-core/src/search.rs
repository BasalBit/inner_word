//! Find in a document.

use std::ops::Range;

use crate::document::Document;
use crate::position::TextRange;
use unicode_segmentation::UnicodeSegmentation;

use crate::text;

/// Options for [`Document::find_all`]. Matches always start and end on
/// grapheme cluster boundaries (a search for `e` does not match the `e` of
/// `e` + combining accent).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SearchOptions {
    /// Match case exactly. When false, matching uses Unicode lower-casing
    /// (plus final sigma as sigma, `İ` as `i`); there is no Unicode
    /// normalisation, so
    /// precomposed and decomposed accents are different text.
    pub case_sensitive: bool,
    /// Only match whole words: both ends of a match must be word boundaries
    /// of Unicode word segmentation (UAX #29). So `don` does not match
    /// inside `don't`, `case` not inside `snake_case`, and each CJK
    /// ideograph counts as a word of its own.
    pub whole_word: bool,
}

/// Text folded for case-insensitive matching, with a map from every folded
/// byte back to the original byte offset of its character.
struct Folded {
    text: String,
    /// `map[i]` = original offset of the char that produced folded byte `i`;
    /// `map[text.len()]` = original length.
    map: Vec<usize>,
    /// Whether folded byte `i` starts the output of an original character.
    starts: Vec<bool>,
}

/// Simple case folding: Unicode lower-casing, plus final sigma folded to
/// sigma (so `ΣΟΦΟΣ` matches `σοφος`) and the Turkish dotted capital I to
/// a plain `i` (so `istanbul` matches `İstanbul`).
fn fold_char(c: char) -> impl Iterator<Item = char> {
    let dotted_i = c == '\u{130}';
    c.to_lowercase()
        .filter(move |&l| !(dotted_i && l == '\u{307}'))
        .map(|l| if l == '\u{3C2}' { '\u{3C3}' } else { l })
}

fn fold(src: &str) -> Folded {
    let mut text = String::with_capacity(src.len());
    let mut map = Vec::with_capacity(src.len() + 1);
    let mut starts = Vec::with_capacity(src.len() + 1);
    for (i, c) in src.char_indices() {
        let before = text.len();
        text.extend(fold_char(c));
        for k in before..text.len() {
            map.push(i);
            starts.push(k == before);
        }
    }
    map.push(src.len());
    starts.push(true);
    Folded { text, map, starts }
}

fn fold_query(q: &str) -> String {
    q.chars().flat_map(fold_char).collect()
}

/// Word boundaries of `text` (UAX #29), sorted, including 0 and the end.
fn word_bounds(text: &str) -> Vec<usize> {
    let mut b: Vec<usize> = text.split_word_bound_indices().map(|(i, _)| i).collect();
    b.push(text.len());
    b
}

/// Matches must start and end on grapheme boundaries (so they can be
/// selected and replaced) and, with `bounds` (whole words), on word
/// boundaries.
fn acceptable(text: &str, r: &Range<usize>, bounds: Option<&[usize]>) -> bool {
    text::is_grapheme_boundary(text, r.start)
        && text::is_grapheme_boundary(text, r.end)
        && bounds.is_none_or(|b| b.binary_search(&r.start).is_ok() && b.binary_search(&r.end).is_ok())
}

/// All non-overlapping matches of `query` in `text`, left to right. The
/// query is normalised like typed text first (`\r\n` becomes `\n`).
pub(crate) fn find_in_text(text: &str, query: &str, opts: SearchOptions) -> Vec<Range<usize>> {
    let query = &*text::normalize_input(query);
    let mut out = Vec::new();
    if query.is_empty() || text.is_empty() {
        return out;
    }
    let bounds = opts.whole_word.then(|| word_bounds(text));
    let bounds = bounds.as_deref();
    if opts.case_sensitive {
        let mut from = 0;
        while let Some(i) = text[from..].find(query) {
            let r = from + i..from + i + query.len();
            if acceptable(text, &r, bounds) {
                from = r.end;
                out.push(r);
            } else {
                from = text::ceil_char_boundary(text, r.start + 1);
            }
            if from >= text.len() {
                break;
            }
        }
        return out;
    }
    let q = fold_query(query);
    if q.is_empty() {
        return out;
    }
    if text.is_ascii() && q.is_ascii() {
        // Fast path: ASCII folding keeps every byte offset.
        let lower = text.to_ascii_lowercase();
        let mut from = 0;
        while let Some(i) = lower[from..].find(&q) {
            let r = from + i..from + i + q.len();
            if bounds.is_none_or(|b| b.binary_search(&r.start).is_ok() && b.binary_search(&r.end).is_ok()) {
                from = r.end;
                out.push(r);
            } else {
                from = r.start + 1;
            }
            if from >= text.len() {
                break;
            }
        }
        return out;
    }
    let f = fold(text);
    let mut from = 0;
    while from < f.text.len() {
        let Some(i) = f.text[from..].find(&q) else {
            break;
        };
        let fs = from + i;
        let fe = fs + q.len();
        // Both ends must fall on whole original characters.
        let aligned = f.starts[fs] && f.starts[fe];
        let r = f.map[fs]..f.map[fe];
        if aligned && r.start < r.end && acceptable(text, &r, bounds) {
            out.push(r);
            from = fe;
        } else {
            from = text::ceil_char_boundary(&f.text, fs + 1);
        }
    }
    out
}

impl Document {
    /// Every match of `query` in the document, in order. Matches never span
    /// blocks; a `\n` in the query matches a soft line break. An empty query
    /// has no matches.
    pub fn find_all(&self, query: &str, opts: SearchOptions) -> Vec<TextRange> {
        let mut out = Vec::new();
        for (i, b) in self.blocks.iter().enumerate() {
            for r in find_in_text(&b.text, query, opts) {
                out.push(TextRange::in_block(i, r.start, r.end));
            }
        }
        out
    }

    /// The matches inside block `index` only (byte ranges). With the change
    /// report of [`EditorState::take_changes`](crate::EditorState::take_changes)
    /// the UI can keep a match list current without searching everything.
    pub fn find_in_block(&self, index: usize, query: &str, opts: SearchOptions) -> Vec<Range<usize>> {
        self.blocks
            .get(index)
            .map(|b| find_in_text(&b.text, query, opts))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Block;
    use crate::position::Position;

    #[test]
    fn case_and_whole_word() {
        let d = Document::from_blocks(vec![
            Block::paragraph("The cat scattered; CAT!"),
            Block::paragraph("cat"),
        ]);
        let any = SearchOptions::default();
        let m = d.find_all("cat", any);
        assert_eq!(m.len(), 4);
        let cs = SearchOptions {
            case_sensitive: true,
            ..any
        };
        assert_eq!(d.find_all("cat", cs).len(), 3);
        let ww = SearchOptions {
            whole_word: true,
            ..any
        };
        let m = d.find_all("cat", ww);
        assert_eq!(m.len(), 3);
        assert_eq!(m[0], TextRange::in_block(0, 4, 7));
        assert_eq!(m[2].start, Position::new(1, 0));
        assert!(d.find_all("", any).is_empty());
    }

    #[test]
    fn unicode_folding_maps_back() {
        // 'İ' lower-cases to two chars; offsets must map to the original.
        let text = "xİstanbul ÄÖ straße";
        let m = find_in_text(text, "istanbul", SearchOptions::default());
        assert_eq!(&text[m[0].clone()], "İstanbul");
        let m = find_in_text(text, "äö", SearchOptions::default());
        assert_eq!(&text[m[0].clone()], "ÄÖ");
        let m = find_in_text(text, "STRASSE", SearchOptions::default());
        assert!(m.is_empty()); // no full case folding of ß
        let m = find_in_text("aaaa", "aa", SearchOptions::default());
        assert_eq!(m, vec![0..2, 2..4]);
        let m = find_in_text("中文中文", "文", SearchOptions { case_sensitive: true, whole_word: false });
        assert_eq!(m, vec![3..6, 9..12]);
        // Never split a grapheme cluster.
        let m = find_in_text("a\u{301} a", "a", SearchOptions::default());
        assert_eq!(m, vec![4..5]);
        let fam = "\u{1F469}\u{200D}\u{1F467}";
        assert!(find_in_text(fam, "\u{1F469}", SearchOptions { case_sensitive: true, whole_word: false }).is_empty());
    }
}
