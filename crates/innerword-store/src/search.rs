//! Full-text search over document titles and plain text (FTS5).
//!
//! User input is never passed to FTS5 as query syntax. [`build_fts_query`] splits it into
//! words (runs of characters other than whitespace and ASCII punctuation), drops words with
//! no letter or digit, quotes each word as an FTS5 string, and joins them with implicit
//! AND. Text inside double quotes (straight or curly) becomes a phrase. The last word gets
//! a prefix match (`*`) unless the input ends in whitespace or a closing quote, so the
//! search works as type-ahead. Operators like `AND`, `NEAR`, `-` or `:` are matched as
//! plain words or ignored.

use std::collections::VecDeque;

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::Store;
use crate::documents::DocumentKind;
use crate::error::Result;
use crate::ids::{DocumentId, FolderId};

/// Marks the start of a match in [`SearchHit::snippet`] (a private-use character).
pub const MATCH_START: char = '\u{E000}';
/// Marks the end of a match in [`SearchHit::snippet`] (a private-use character).
pub const MATCH_END: char = '\u{E001}';

/// One search result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    /// The matching document.
    pub document_id: DocumentId,
    /// Its title.
    pub title: String,
    /// Its folder, `None` at the root.
    pub folder_id: Option<FolderId>,
    /// An excerpt of the plain text (up to 16 words, whitespace collapsed to single spaces)
    /// around the first matching word, with matching words wrapped in [`MATCH_START`] /
    /// [`MATCH_END`] and elisions shown as `…`. When no word of the text can be highlighted
    /// (for example only the title matched), this is the start of the text without
    /// markers. Use
    /// [`SearchHit::snippet_parts`] to split it.
    pub snippet: String,
    /// Relevance; higher is better (negated FTS5 bm25, title weighted 10x the body).
    /// Results are returned sorted by it.
    pub rank: f64,
    /// What the item is: a text, or a page (whose text is what it showed).
    #[serde(default)]
    pub kind: DocumentKind,
}

/// A piece of a snippet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnippetPart<'a> {
    /// The text.
    pub text: &'a str,
    /// Whether this piece is a match.
    pub is_match: bool,
}

impl SearchHit {
    /// Splits [`SearchHit::snippet`] into plain and matched pieces, dropping the markers.
    pub fn snippet_parts(&self) -> Vec<SnippetPart<'_>> {
        let mut parts = Vec::new();
        let mut is_match = false;
        let mut rest = self.snippet.as_str();
        loop {
            let marker = if is_match { MATCH_END } else { MATCH_START };
            match rest.find(marker) {
                Some(at) => {
                    if at > 0 {
                        parts.push(SnippetPart {
                            text: &rest[..at],
                            is_match,
                        });
                    }
                    rest = &rest[at + marker.len_utf8()..];
                    is_match = !is_match;
                }
                None => {
                    if !rest.is_empty() {
                        parts.push(SnippetPart {
                            text: rest,
                            is_match,
                        });
                    }
                    return parts;
                }
            }
        }
    }
}

fn is_quote(c: char) -> bool {
    matches!(c, '"' | '\u{201C}' | '\u{201D}' | '\u{201E}')
}

fn is_separator(c: char) -> bool {
    c.is_whitespace() || c.is_ascii_punctuation() || is_quote(c)
}

/// One parsed query term: a single word or a quoted phrase.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Term {
    words: Vec<String>,
    /// Whether the last word of this term is matched as a prefix.
    prefix: bool,
}

/// Splits user input into terms. See the module docs for the rules.
pub(crate) fn parse_terms(input: &str) -> Vec<Term> {
    let mut terms: Vec<Term> = Vec::new();
    let mut in_phrase = false;
    let mut phrase: Vec<String> = Vec::new();
    let mut word = String::new();
    // Whether the most recent character was part of a kept word that ended the input.
    let mut last_word_open;

    fn keep(word: &mut String) -> Option<String> {
        let kept = word
            .chars()
            .any(char::is_alphanumeric)
            .then(|| word.clone());
        word.clear();
        kept
    }

    for c in input.chars() {
        if is_quote(c) || is_separator(c) {
            let kept = keep(&mut word);
            if in_phrase {
                phrase.extend(kept);
            } else if let Some(w) = kept {
                terms.push(Term {
                    words: vec![w],
                    prefix: false,
                });
            }
            if is_quote(c) {
                if in_phrase && !phrase.is_empty() {
                    terms.push(Term {
                        words: std::mem::take(&mut phrase),
                        prefix: false,
                    });
                }
                in_phrase = !in_phrase;
            }
        } else {
            word.push(c);
        }
    }
    let kept = keep(&mut word);
    last_word_open = kept.is_some();
    if in_phrase {
        phrase.extend(kept);
        if !phrase.is_empty() {
            terms.push(Term {
                words: phrase,
                prefix: false,
            });
        } else {
            last_word_open = false;
        }
    } else if let Some(w) = kept {
        terms.push(Term {
            words: vec![w],
            prefix: false,
        });
    }
    if last_word_open && let Some(last) = terms.last_mut() {
        last.prefix = true;
    }
    terms
}

/// Renders parsed terms as an FTS5 query: every term quoted, implicit AND.
fn fts_query(terms: &[Term]) -> Option<String> {
    if terms.is_empty() {
        return None;
    }
    let parts: Vec<String> = terms
        .iter()
        .map(|term| {
            // Words never contain '"', but escape anyway so the invariant is local.
            let body = term
                .words
                .iter()
                .map(|w| w.replace('"', "\"\""))
                .collect::<Vec<_>>()
                .join(" ");
            let star = if term.prefix { "*" } else { "" };
            format!("\"{body}\"{star}")
        })
        .collect();
    Some(parts.join(" "))
}

/// Turns arbitrary user input into a safe FTS5 query, or `None` when it has no searchable
/// words. See the module docs for the rules.
#[cfg(test)]
pub(crate) fn build_fts_query(input: &str) -> Option<String> {
    fts_query(&parse_terms(input))
}

// ---------------------------------------------------------------------------------------
// Snippets
//
// FTS5's snippet() re-tokenises the whole column and scores every match position, which
// takes minutes for a multi-megabyte document with many matches. Snippets are therefore
// built here: scan the plain text word by word, stop at the first matching word, and cut a
// window around it. Matching folds case and common Latin diacritics, mirroring the index's
// `unicode61 remove_diacritics 2` tokenizer closely enough for highlighting.
// ---------------------------------------------------------------------------------------

const SNIPPET_WORDS: usize = 16;
const SNIPPET_LEAD: usize = 5;

/// Base letters for U+00C0..=U+00FF; '_' keeps the character.
const LATIN1_FOLD: &str = "aaaaaa_ceeeeiiiidnooooo_ouuuuy__aaaaaa_ceeeeiiiidnooooo_ouuuuy_y";
/// Base letters for U+0100..=U+017F; '_' keeps the character.
const LATIN_EXT_A_FOLD: &str = concat!(
    "aaaaaa",
    "cccccccc",
    "dddd",
    "eeeeeeeeee",
    "gggggggg",
    "hhhh",
    "iiiiiiiiii",
    "__",
    "jj",
    "kk",
    "_",
    "llllllllll",
    "nnnnnn",
    "n",
    "__",
    "oooooo",
    "__",
    "rrrrrr",
    "ssssssss",
    "tttttt",
    "uuuuuuuuuuuu",
    "ww",
    "yy",
    "y",
    "zzzzzz",
    "s"
);

fn is_combining_mark(c: char) -> bool {
    matches!(c as u32, 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF)
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || is_combining_mark(c)
}

/// Lowercases and strips common diacritics.
fn fold(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    for c in word.chars() {
        if is_combining_mark(c) {
            continue;
        }
        for lower in c.to_lowercase() {
            let code = lower as u32;
            let table = match code {
                0xC0..=0xFF => LATIN1_FOLD.as_bytes().get((code - 0xC0) as usize),
                0x100..=0x17F => LATIN_EXT_A_FOLD.as_bytes().get((code - 0x100) as usize),
                _ => None,
            };
            match table {
                Some(&b) if b != b'_' => out.push(b as char),
                _ => out.push(lower),
            }
        }
    }
    out
}

struct Matcher {
    exact: Vec<String>,
    prefixes: Vec<String>,
}

impl Matcher {
    fn new(terms: &[Term]) -> Self {
        let mut exact = Vec::new();
        let mut prefixes = Vec::new();
        for term in terms {
            let last = term.words.len().saturating_sub(1);
            for (i, word) in term.words.iter().enumerate() {
                // A query word may itself contain several index tokens ("e-mail" is
                // already split; "naïve’s" is not): split the same way as the text.
                let pieces: Vec<String> = word
                    .split(|c: char| !is_word_char(c))
                    .filter(|p| !p.is_empty())
                    .map(fold)
                    .collect();
                let n = pieces.len();
                for (j, piece) in pieces.into_iter().enumerate() {
                    if term.prefix && i == last && j + 1 == n {
                        prefixes.push(piece);
                    } else {
                        exact.push(piece);
                    }
                }
            }
        }
        Self { exact, prefixes }
    }

    fn matches(&self, folded: &str) -> bool {
        self.exact.iter().any(|w| w == folded)
            || self.prefixes.iter().any(|p| folded.starts_with(p.as_str()))
    }
}

/// Byte ranges of the words in `text`, lazily.
fn words(text: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut chars = text.char_indices().peekable();
    std::iter::from_fn(move || {
        while let Some(&(_, c)) = chars.peek() {
            if is_word_char(c) {
                break;
            }
            chars.next();
        }
        let (start, _) = *chars.peek()?;
        let mut end = start;
        while let Some(&(i, c)) = chars.peek() {
            if !is_word_char(c) {
                break;
            }
            end = i + c.len_utf8();
            chars.next();
        }
        Some((start, end))
    })
}

/// Appends `s` with every whitespace run collapsed to one space.
fn push_collapsed(out: &mut String, s: &str) {
    let mut in_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
}

pub(crate) fn build_snippet(text: &str, terms: &[Term]) -> String {
    let matcher = Matcher::new(terms);
    let mut iter = words(text);
    let mut lead: VecDeque<(usize, usize)> = VecDeque::new();
    let mut dropped_before = false;
    let mut window: Vec<(usize, usize, bool)> = Vec::new();
    for (start, end) in iter.by_ref() {
        if matcher.matches(&fold(&text[start..end])) {
            window.extend(lead.drain(..).map(|(s, e)| (s, e, false)));
            window.push((start, end, true));
            break;
        }
        lead.push_back((start, end));
        if lead.len() > SNIPPET_LEAD {
            lead.pop_front();
            dropped_before = true;
        }
    }
    if window.is_empty() {
        // No highlightable word (e.g. a title-only match): the start of the text.
        return leading_text(text);
    }
    let mut more = false;
    for (start, end) in iter {
        if window.len() == SNIPPET_WORDS {
            more = true;
            break;
        }
        let is_match = matcher.matches(&fold(&text[start..end]));
        window.push((start, end, is_match));
    }
    let mut out = String::new();
    if dropped_before {
        out.push('…');
    }
    let mut cursor = window[0].0;
    for &(start, end, is_match) in &window {
        push_collapsed(&mut out, &text[cursor..start]);
        if is_match {
            out.push(MATCH_START);
            out.push_str(&text[start..end]);
            out.push(MATCH_END);
        } else {
            out.push_str(&text[start..end]);
        }
        cursor = end;
    }
    if more {
        out.push('…');
    }
    out
}

fn leading_text(text: &str) -> String {
    let mut out = String::new();
    let mut end = 0;
    let mut more = false;
    for (i, (_, word_end)) in words(text).enumerate() {
        if i == SNIPPET_WORDS {
            more = true;
            break;
        }
        end = word_end;
    }
    let lead = text[..end].trim_start();
    push_collapsed(&mut out, lead);
    if more {
        out.push('…');
    }
    out
}

impl Store {
    /// Searches live documents' titles and plain text. Any input is accepted (see the
    /// module rules); input without searchable words returns no results. At most `limit`
    /// hits, best first.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let terms = parse_terms(query);
        let Some(fts) = fts_query(&terms) else {
            return Ok(Vec::new());
        };
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut hits: Vec<SearchHit> = {
            let mut stmt = self.conn.prepare_cached(
                "SELECT d.id, d.title, d.folder_id, bm25(documents_fts, 10.0, 1.0) AS score,
                        d.kind
                 FROM documents_fts JOIN documents d ON d.id = documents_fts.rowid
                 WHERE documents_fts MATCH ?1 AND d.trashed_at IS NULL
                 ORDER BY score, d.updated_at DESC
                 LIMIT ?2",
            )?;
            stmt.query_map(
                params![fts, i64::try_from(limit).unwrap_or(i64::MAX)],
                |r| {
                    let score: f64 = r.get(3)?;
                    Ok(SearchHit {
                        document_id: r.get(0)?,
                        title: r.get(1)?,
                        folder_id: r.get(2)?,
                        snippet: String::new(),
                        rank: -score,
                        kind: r.get(4)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<_>>()?
        };
        let mut text_stmt = self
            .conn
            .prepare_cached("SELECT plain_text FROM documents WHERE id = ?1")?;
        for hit in &mut hits {
            let text: String = text_stmt.query_row([hit.document_id], |r| r.get(0))?;
            hit.snippet = build_snippet(&text, &terms);
        }
        Ok(hits)
    }

    /// Rebuilds the full-text index from the documents table. Only needed for repair; the
    /// index is otherwise kept in sync by triggers.
    pub fn rebuild_search_index(&mut self) -> Result<()> {
        self.conn.execute(
            "INSERT INTO documents_fts(documents_fts) VALUES ('rebuild')",
            [],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_words_get_prefix_on_last() {
        assert_eq!(
            build_fts_query("hello wor").as_deref(),
            Some("\"hello\" \"wor\"*")
        );
        assert_eq!(
            build_fts_query("hello world ").as_deref(),
            Some("\"hello\" \"world\"")
        );
    }

    #[test]
    fn operators_and_punctuation_are_neutralised() {
        assert_eq!(
            build_fts_query("a AND -b OR (c* NEAR d)").as_deref(),
            Some("\"a\" \"AND\" \"b\" \"OR\" \"c\" \"NEAR\" \"d\"")
        );
        assert_eq!(
            build_fts_query("title:foo").as_deref(),
            Some("\"title\" \"foo\"*")
        );
        assert_eq!(build_fts_query("\"\"\"").as_deref(), None);
        assert_eq!(build_fts_query("   ").as_deref(), None);
        assert_eq!(build_fts_query("— … ¿").as_deref(), None);
    }

    #[test]
    fn quotes_make_phrases() {
        assert_eq!(
            build_fts_query("\"dark and stormy\" night").as_deref(),
            Some("\"dark and stormy\" \"night\"*")
        );
        assert_eq!(
            build_fts_query("x \"dark sto").as_deref(),
            Some("\"x\" \"dark sto\"*")
        );
        assert_eq!(
            build_fts_query("“curly quoted”").as_deref(),
            Some("\"curly quoted\"")
        );
    }

    #[test]
    fn unicode_words_survive() {
        assert_eq!(
            build_fts_query("café naïve 東京").as_deref(),
            Some("\"café\" \"naïve\" \"東京\"*")
        );
    }

    fn snip(text: &str, query: &str) -> String {
        build_snippet(text, &parse_terms(query))
            .replace(MATCH_START, "[")
            .replace(MATCH_END, "]")
    }

    #[test]
    fn snippets_mark_matches_and_elide() {
        assert_eq!(
            snip("the quick brown fox", "quick"),
            "the [quick] brown fox"
        );
        assert_eq!(snip("The Quick", "qui"), "The [Quick]");
        let long: String = (0..100).map(|i| format!("w{i} ")).collect();
        assert_eq!(
            snip(&long, "w50"),
            "…w45 w46 w47 w48 w49 [w50] w51 w52 w53 w54 w55 w56 w57 w58 w59 w60…"
        );
        assert_eq!(
            snip(&long, "nomatch"),
            "w0 w1 w2 w3 w4 w5 w6 w7 w8 w9 w10 w11 w12 w13 w14 w15…"
        );
        assert_eq!(snip("a\n\n  b crème", "creme"), "a b [crème]");
        assert_eq!(snip("ÉCOLE d’été", "ecole ete"), "[ÉCOLE] d’[été]");
        assert_eq!(snip("", "x"), "");
    }

    #[test]
    fn fold_strips_latin_diacritics() {
        assert_eq!(fold("ÀÉÎÕÜçñŁŚŽÿ"), "aeioucnlszy");
        assert_eq!(fold("Straße"), "straße");
        assert_eq!(fold("Привет"), "привет");
    }

    #[test]
    fn junk_last_word_does_not_move_prefix() {
        assert_eq!(build_fts_query("hello —").as_deref(), Some("\"hello\""));
        assert_eq!(build_fts_query("\"a b\" ").as_deref(), Some("\"a b\""));
    }

    #[test]
    fn snippet_parts_split_markers() {
        let hit = SearchHit {
            document_id: DocumentId(1),
            title: String::new(),
            folder_id: None,
            snippet: format!("a {MATCH_START}b{MATCH_END} c"),
            rank: 0.0,
            kind: DocumentKind::Text,
        };
        let parts = hit.snippet_parts();
        assert_eq!(parts.len(), 3);
        assert_eq!(
            parts[1],
            SnippetPart {
                text: "b",
                is_match: true
            }
        );
    }
}
