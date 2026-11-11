//! Full-text search over document titles and plain text (FTS5).
//!
//! User input is never passed to FTS5 as query syntax. [`build_fts_query`] splits it into
//! words (runs of characters other than whitespace and ASCII punctuation), drops words with
//! no letter or digit, quotes each word as an FTS5 string, and joins them with implicit
//! AND. Text inside double quotes (straight or curly) becomes a phrase. The last word gets
//! a prefix match (`*`) unless the input ends in whitespace or a closing quote, so the
//! search works as type-ahead. Operators like `AND`, `NEAR`, `-` or `:` are matched as
//! plain words or ignored.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::Store;
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
    /// An excerpt of the plain text (about 16 words) with matches wrapped in
    /// [`MATCH_START`] / [`MATCH_END`] and elisions shown as `…`. When only the title
    /// matched, this is the start of the text without markers. Use
    /// [`SearchHit::snippet_parts`] to split it.
    pub snippet: String,
    /// Relevance; higher is better (negated FTS5 bm25, title weighted 10x the body).
    /// Results are returned sorted by it.
    pub rank: f64,
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

/// Turns arbitrary user input into a safe FTS5 query, or `None` when it has no searchable
/// words. See the module docs for the rules.
pub(crate) fn build_fts_query(input: &str) -> Option<String> {
    // (words, is_phrase)
    let mut terms: Vec<(Vec<String>, bool)> = Vec::new();
    let mut in_phrase = false;
    let mut phrase: Vec<String> = Vec::new();
    let mut word = String::new();

    fn flush(word: &mut String, out: &mut Vec<String>) {
        if word.chars().any(char::is_alphanumeric) {
            out.push(std::mem::take(word));
        } else {
            word.clear();
        }
    }

    for c in input.chars() {
        if is_quote(c) {
            if in_phrase {
                flush(&mut word, &mut phrase);
                if !phrase.is_empty() {
                    terms.push((std::mem::take(&mut phrase), true));
                }
            } else {
                let mut single = Vec::new();
                flush(&mut word, &mut single);
                terms.extend(single.into_iter().map(|w| (vec![w], false)));
            }
            in_phrase = !in_phrase;
        } else if is_separator(c) {
            if in_phrase {
                flush(&mut word, &mut phrase);
            } else {
                let mut single = Vec::new();
                flush(&mut word, &mut single);
                terms.extend(single.into_iter().map(|w| (vec![w], false)));
            }
        } else {
            word.push(c);
        }
    }
    let ends_in_word = !word.is_empty();
    if in_phrase {
        flush(&mut word, &mut phrase);
        if !phrase.is_empty() {
            terms.push((phrase, true));
        }
    } else {
        let mut single = Vec::new();
        flush(&mut word, &mut single);
        terms.extend(single.into_iter().map(|w| (vec![w], false)));
    }
    if terms.is_empty() {
        return None;
    }
    let last = terms.len() - 1;
    let parts: Vec<String> = terms
        .iter()
        .enumerate()
        .map(|(i, (words, _))| {
            // Words never contain '"', but escape anyway so the invariant is local.
            let body = words
                .iter()
                .map(|w| w.replace('"', "\"\""))
                .collect::<Vec<_>>()
                .join(" ");
            let prefix = if i == last && ends_in_word { "*" } else { "" };
            format!("\"{body}\"{prefix}")
        })
        .collect();
    Some(parts.join(" "))
}

impl Store {
    /// Searches live documents' titles and plain text. Any input is accepted (see the
    /// module rules); input without searchable words returns no results. At most `limit`
    /// hits, best first.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let Some(fts) = build_fts_query(query) else {
            return Ok(Vec::new());
        };
        let mut stmt = self.conn.prepare_cached(
            "SELECT d.id, d.title, d.folder_id,
                    snippet(documents_fts, 1, ?2, ?3, '…', 16),
                    bm25(documents_fts, 10.0, 1.0) AS score
             FROM documents_fts JOIN documents d ON d.id = documents_fts.rowid
             WHERE documents_fts MATCH ?1 AND d.trashed_at IS NULL
             ORDER BY score, d.updated_at DESC
             LIMIT ?4",
        )?;
        let hits = stmt
            .query_map(
                params![
                    fts,
                    MATCH_START.to_string(),
                    MATCH_END.to_string(),
                    i64::try_from(limit).unwrap_or(i64::MAX)
                ],
                |r| {
                    let score: f64 = r.get(4)?;
                    Ok(SearchHit {
                        document_id: r.get(0)?,
                        title: r.get(1)?,
                        folder_id: r.get(2)?,
                        snippet: r.get(3)?,
                        rank: -score,
                    })
                },
            )?
            .collect::<rusqlite::Result<_>>()?;
        Ok(hits)
    }

    /// Rebuilds the full-text index from the documents table. Only needed for repair; the
    /// index is otherwise kept in sync by triggers.
    pub fn rebuild_search_index(&mut self) -> Result<()> {
        self.conn
            .execute("INSERT INTO documents_fts(documents_fts) VALUES ('rebuild')", [])?;
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
        assert_eq!(build_fts_query("title:foo").as_deref(), Some("\"title\" \"foo\"*"));
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

    #[test]
    fn snippet_parts_split_markers() {
        let hit = SearchHit {
            document_id: DocumentId(1),
            title: String::new(),
            folder_id: None,
            snippet: format!("a {MATCH_START}b{MATCH_END} c"),
            rank: 0.0,
        };
        let parts = hit.snippet_parts();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[1], SnippetPart { text: "b", is_match: true });
    }
}
