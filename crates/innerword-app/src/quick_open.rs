//! Quick open (`cmd-p`): jump to a document by name.
//!
//! Lists every document, most recently edited first, with its folder path
//! as the detail. The query matches titles fuzzily. When nothing matches,
//! the only row offers to create a document with the typed name.
//!
//! A title derived from the text can be eighty characters long. In a row
//! that also names a folder it is cut short ([`clip_title`]) so the folder
//! stays in view: two documents with the same opening words are told apart
//! by where they are.

use gpui::{App, Context, Entity, SharedString, WeakEntity, Window};
use innerword_library::{DocumentEntry, DocumentId, Library};
use innerword_ui::{
    IconName, Picker, PickerDelegate, PickerEntry, PickerItem, fuzzy::match_strings,
};

use crate::{document_io::UNTITLED, workspace::Workspace};

/// Quick open as a modal view.
pub type QuickOpen = Picker<QuickOpenDelegate>;

/// The longest title, in characters, shown in a row that also names the
/// document's folder.
pub const MAX_TITLE_BESIDE_FOLDER: usize = 48;

/// `title` cut to at most `max` characters (grapheme clusters), ending in an
/// ellipsis when something was cut, with the match `positions` (byte
/// offsets) that are still in view.
pub fn clip_title(title: &str, positions: &[usize], max: usize) -> (String, Vec<usize>) {
    use unicode_segmentation::UnicodeSegmentation;
    let max = max.max(2);
    let Some((cut, _)) = title.grapheme_indices(true).nth(max) else {
        return (title.to_string(), positions.to_vec());
    };
    // Room for the ellipsis, and no space before it.
    let keep = title
        .grapheme_indices(true)
        .nth(max - 1)
        .map_or(cut, |(at, _)| at);
    let kept = title[..keep].trim_end();
    let positions = positions
        .iter()
        .copied()
        .filter(|position| *position < kept.len())
        .collect();
    (format!("{kept}\u{2026}"), positions)
}

#[derive(Clone, Debug, PartialEq)]
enum Row {
    Header(&'static str),
    Document { index: usize, positions: Vec<usize> },
    Create(String),
}

/// What confirming a row does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuickOpenChoice {
    /// Open this document.
    Open(DocumentId),
    /// Create a document with this name.
    Create(String),
}

/// The picker delegate behind quick open.
pub struct QuickOpenDelegate {
    workspace: WeakEntity<Workspace>,
    documents: Vec<DocumentEntry>,
    titles: Vec<String>,
    rows: Vec<Row>,
}

impl QuickOpenDelegate {
    /// Reads the document list from `library` (most recently edited first).
    pub fn new(workspace: WeakEntity<Workspace>, library: &Entity<Library>, cx: &App) -> Self {
        let documents = library.read(cx).all_documents().unwrap_or_default();
        let titles = documents
            .iter()
            .map(|entry| {
                if entry.meta.title.trim().is_empty() {
                    UNTITLED.to_string()
                } else {
                    entry.meta.title.clone()
                }
            })
            .collect();
        Self {
            workspace,
            documents,
            titles,
            rows: Vec::new(),
        }
    }

    /// What each listed row would do, in order (headers excluded).
    pub fn choices(&self) -> Vec<QuickOpenChoice> {
        self.rows
            .iter()
            .filter_map(|row| self.choice(row))
            .collect()
    }

    fn choice(&self, row: &Row) -> Option<QuickOpenChoice> {
        match row {
            Row::Header(_) => None,
            Row::Document { index, .. } => {
                Some(QuickOpenChoice::Open(self.documents[*index].meta.id))
            }
            Row::Create(name) => Some(QuickOpenChoice::Create(name.clone())),
        }
    }
}

impl PickerDelegate for QuickOpenDelegate {
    fn update_matches(&mut self, query: &str, _: &mut Window, _: &mut Context<Picker<Self>>) {
        let name = query.trim();
        self.rows = if name.is_empty() {
            let mut rows = Vec::with_capacity(self.documents.len() + 1);
            if !self.documents.is_empty() {
                rows.push(Row::Header("Recently edited"));
            }
            rows.extend((0..self.documents.len()).map(|index| Row::Document {
                index,
                positions: Vec::new(),
            }));
            rows
        } else {
            let matches = match_strings(
                query,
                self.titles.iter().map(String::as_str),
                self.titles.len(),
            );
            if matches.is_empty() {
                vec![Row::Create(name.to_string())]
            } else {
                matches
                    .into_iter()
                    .map(|found| Row::Document {
                        index: found.candidate_index,
                        positions: found.positions,
                    })
                    .collect()
            }
        };
    }

    fn entry_count(&self) -> usize {
        self.rows.len()
    }

    fn entry(&self, ix: usize, _: &App) -> PickerEntry {
        match &self.rows[ix] {
            Row::Header(title) => PickerEntry::Header((*title).into()),
            Row::Document { index, positions } => {
                let entry = &self.documents[*index];
                let folder = entry.folder_label();
                let title = &self.titles[*index];
                // A page: the globe, and "Page" beside it; a whiteboard: its
                // board, and "Whiteboard".
                let (icon, kind) = match entry.kind {
                    innerword_library::DocumentKind::Page => (IconName::Globe, Some("Page")),
                    innerword_library::DocumentKind::Board => {
                        (IconName::Whiteboard, Some("Whiteboard"))
                    }
                    innerword_library::DocumentKind::Text => (IconName::Document, None),
                };
                let detail = match (kind, folder.is_empty()) {
                    (Some(kind), true) => kind.to_string(),
                    (Some(kind), false) => format!("{kind} \u{b7} {folder}"),
                    (None, _) => folder,
                };
                if detail.is_empty() {
                    PickerItem::new(title.clone())
                        .positions(positions.clone())
                        .icon(icon)
                        .into()
                } else {
                    let (title, positions) = clip_title(title, positions, MAX_TITLE_BESIDE_FOLDER);
                    PickerItem::new(title)
                        .positions(positions)
                        .icon(icon)
                        .detail(detail)
                        .into()
                }
            }
            Row::Create(name) => PickerItem::new(format!("Create “{name}”"))
                .detail("New document")
                .icon(IconName::Plus)
                .into(),
        }
    }

    fn is_selectable(&self, ix: usize, _: &App) -> bool {
        !matches!(self.rows.get(ix), Some(Row::Header(_)) | None)
    }

    fn confirm(
        &mut self,
        ix: usize,
        _secondary: bool,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> bool {
        let Some(choice) = self.rows.get(ix).and_then(|row| self.choice(row)) else {
            return false;
        };
        let workspace = self.workspace.clone();
        window.defer(cx, move |window, cx| {
            workspace
                .update(cx, |workspace, cx| match choice {
                    QuickOpenChoice::Open(id) => {
                        workspace.open_document(id, true, window, cx);
                    }
                    QuickOpenChoice::Create(name) => {
                        workspace.new_document_named(&name, window, cx);
                    }
                })
                .ok();
        });
        false
    }

    fn placeholder(&self) -> SharedString {
        "Open a document".into()
    }

    fn empty_text(&self, _query: &str) -> SharedString {
        "No documents yet. Type a name to create one.".into()
    }
}
