//! Library search (`cmd-shift-f`): full-text search over every document.
//!
//! Each hit shows the document title and a snippet with the matching words
//! highlighted. Enter opens the document and selects the first occurrence
//! of the query in its text.

use gpui::{App, Context, Entity, SharedString, WeakEntity, Window};
use writer_library::{DocumentId, Library, SearchHit};
use writer_ui::{IconName, Picker, PickerDelegate, PickerEntry, PickerItem};

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
        Self {
            document_id: hit.document_id,
            title: if hit.title.trim().is_empty() {
                UNTITLED.to_string()
            } else {
                hit.title.clone()
            },
            snippet,
            positions,
        }
    }
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
        PickerItem::new(short_title(&hit.title))
            .detail(hit.snippet.clone())
            .detail_positions(hit.positions.clone())
            .icon(IconName::Document)
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
            "Type to search. The last word matches as a prefix; quotes make a phrase.".into()
        } else {
            format!("No document contains “{}”", query.trim()).into()
        }
    }
}
