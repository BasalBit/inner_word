//! Quick open (`cmd-p`): jump to a document by name.
//!
//! Lists every document, most recently edited first, with its folder path
//! as the detail. The query matches titles fuzzily. When nothing matches,
//! the only row offers to create a document with the typed name.

use gpui::{App, Context, Entity, SharedString, WeakEntity, Window};
use writer_library::{DocumentEntry, DocumentId, Library};
use writer_ui::{IconName, Picker, PickerDelegate, PickerEntry, PickerItem, fuzzy::match_strings};

use crate::{document_io::UNTITLED, workspace::Workspace};

/// Quick open as a modal view.
pub type QuickOpen = Picker<QuickOpenDelegate>;

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
                let folder = self.documents[*index].folder_label();
                let item = PickerItem::new(self.titles[*index].clone())
                    .positions(positions.clone())
                    .icon(IconName::Document);
                if folder.is_empty() {
                    item.into()
                } else {
                    item.detail(folder).into()
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
