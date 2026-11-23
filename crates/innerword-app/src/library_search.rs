//! Library search (`cmd-shift-f`): full-text search over every document.
//!
//! Each hit shows the document title and a snippet with the matching words
//! highlighted. Enter opens the document and selects the first occurrence
//! of the query in its text.
//!
//! The text of a document usually starts with its title (the first
//! heading). The row shows the title already, so a snippet that starts
//! with it goes on after it; the words of the query are marked in the
//! title instead.

use gpui::{App, Context, Entity, SharedString, WeakEntity, Window};
use innerword_library::{DocumentId, Library, SearchHit};
use innerword_ui::{IconName, Picker, PickerDelegate, PickerEntry, PickerItem};

use crate::{document_io::UNTITLED, workspace::Workspace};

/// Library search as a modal view.
pub type LibrarySearch = Picker<LibrarySearchDelegate>;

/// The most hits one search lists.
pub const MAX_HITS: usize = 60;

/// Titles longer than this are cut in the list so the snippet stays visible.
pub const MAX_TITLE_CHARS: usize = 36;

fn short_title(title: &str) -> String {
    if title.chars().count() <= MAX_TITLE_CHARS {
        return title.to_string();
    }
    let cut: String = title.chars().take(MAX_TITLE_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

/// One listed hit, ready to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct HitRow {
    /// The document.
    pub document_id: DocumentId,
    /// Its title.
    pub title: String,
    /// The snippet without the match markers.
    pub snippet: String,
    /// Byte offsets into `snippet` of the characters to highlight.
    pub positions: Vec<usize>,
    /// A text or a page.
    pub kind: innerword_library::DocumentKind,
}

impl HitRow {
    fn from_hit(hit: &SearchHit) -> Self {
        let mut snippet = String::new();
        let mut positions = Vec::new();
        for part in hit.snippet_parts() {
            let start = snippet.len();
            snippet.push_str(part.text);
            if part.is_match {
                positions.extend(part.text.char_indices().map(|(offset, _)| start + offset));
            }
        }
        let title = hit.title.trim();
        // The title is on the row already: a snippet that begins with it
        // goes on after it (unless nothing would be left).
        if !title.is_empty()
            && let Some(rest) = snippet.strip_prefix(title)
            && !rest.trim().is_empty()
        {
            let cut = snippet.len() - rest.trim_start().len();
            snippet.drain(..cut);
            positions = positions
                .into_iter()
                .filter(|position| *position >= cut)
                .map(|position| position - cut)
                .collect();
        }
        Self {
            kind: hit.kind,
            document_id: hit.document_id,
            title: if title.is_empty() {
                UNTITLED.to_string()
            } else {
                hit.title.clone()
            },
            snippet,
            positions,
        }
    }
}

/// The byte offsets of the characters of `text` that belong to an
/// occurrence of one of the words of `query` (case-insensitive; quotes
/// around a phrase are ignored, its words are marked one by one).
pub fn word_positions(text: &str, query: &str) -> Vec<usize> {
    let words: Vec<Vec<char>> = query
        .split(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\u{201c}' | '\u{201d}'))
        .filter(|word| !word.is_empty())
        .map(|word| word.chars().flat_map(char::to_lowercase).collect())
        .collect();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut marked = vec![false; chars.len()];
    for word in &words {
        for start in 0..chars.len() {
            let mut end = start;
            let mut matched = 0;
            while matched < word.len() && end < chars.len() {
                let lower: Vec<char> = chars[end].1.to_lowercase().collect();
                if word[matched..].starts_with(&lower) {
                    matched += lower.len();
                    end += 1;
                } else {
                    break;
                }
            }
            if matched == word.len() {
                marked[start..end].iter_mut().for_each(|mark| *mark = true);
            }
        }
    }
    chars
        .iter()
        .zip(marked)
        .filter_map(|((offset, _), mark)| mark.then_some(*offset))
        .collect()
}

/// The picker delegate behind library search.
pub struct LibrarySearchDelegate {
    workspace: WeakEntity<Workspace>,
    library: Entity<Library>,
    query: String,
    hits: Vec<HitRow>,
    error: Option<String>,
}

impl LibrarySearchDelegate {
    /// A delegate searching `library`.
    pub fn new(workspace: WeakEntity<Workspace>, library: Entity<Library>) -> Self {
        Self {
            workspace,
            library,
            query: String::new(),
            hits: Vec::new(),
            error: None,
        }
    }

    /// The listed hits.
    pub fn hits(&self) -> &[HitRow] {
        &self.hits
    }
}

impl PickerDelegate for LibrarySearchDelegate {
    fn update_matches(&mut self, query: &str, _: &mut Window, cx: &mut Context<Picker<Self>>) {
        self.query = query.to_string();
        self.error = None;
        self.hits = match self.library.read(cx).search(query, MAX_HITS) {
            Ok(hits) => hits.iter().map(HitRow::from_hit).collect(),
            Err(error) => {
                self.error = Some(error.to_string());
                Vec::new()
            }
        };
    }

    fn entry_count(&self) -> usize {
        self.hits.len()
    }

    fn entry(&self, ix: usize, _: &App) -> PickerEntry {
        let hit = &self.hits[ix];
        let title = short_title(&hit.title);
        let marked = word_positions(&title, &self.query);
        PickerItem::new(title)
            .positions(marked)
            .detail(hit.snippet.clone())
            .detail_positions(hit.positions.clone())
            .icon(match hit.kind {
                innerword_library::DocumentKind::Page => IconName::Globe,
                innerword_library::DocumentKind::Board => IconName::Whiteboard,
                innerword_library::DocumentKind::Text => IconName::Document,
            })
            .into()
    }

    fn confirm(
        &mut self,
        ix: usize,
        _secondary: bool,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> bool {
        let Some(hit) = self.hits.get(ix) else {
            return false;
        };
        let id = hit.document_id;
        let query = self.query.clone();
        let workspace = self.workspace.clone();
        window.defer(cx, move |window, cx| {
            workspace
                .update(cx, |workspace, cx| {
                    workspace.open_search_hit(id, &query, window, cx);
                })
                .ok();
        });
        false
    }

    fn placeholder(&self) -> SharedString {
        "Search the text of every document".into()
    }

    fn empty_text(&self, query: &str) -> SharedString {
        if let Some(error) = &self.error {
            format!("The search failed: {error}").into()
        } else if query.trim().is_empty() {
            "Type to search every document. Put a phrase in quotes.".into()
        } else {
            format!("No documents contain “{}”", query.trim()).into()
        }
    }
}
