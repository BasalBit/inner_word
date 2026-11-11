//! Between the editor's [`Document`] and the library's stored rows: what a
//! save writes, and how stored content becomes a document again.

use writer_core::{Document, Fragment, LoadError};
use writer_library::DocumentContent;

/// The title of a document whose text gives none.
pub const UNTITLED: &str = "Untitled";

/// Everything a save writes for one document.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredForm {
    /// The document as JSON.
    pub json: String,
    /// The title shown in the library.
    pub title: String,
    /// The text, for full-text search.
    pub plain_text: String,
    /// Word count.
    pub words: u64,
    /// Character count (grapheme clusters).
    pub characters: u64,
}

impl StoredForm {
    /// Derives the stored form of `document`. With `title` the given title is
    /// kept (the user named the document); without it the title comes from
    /// the text ([`Document::title`]), or is [`UNTITLED`].
    pub fn of(document: &Document, title: Option<&str>) -> Self {
        Self::with_stats(document, title, document.stats())
    }

    /// Like [`StoredForm::of`], with statistics the caller already has (the
    /// editor keeps them current), so a long document is not counted again
    /// on every save.
    pub fn with_stats(document: &Document, title: Option<&str>, stats: writer_core::Stats) -> Self {
        Self {
            json: document.to_json(),
            title: match title {
                Some(title) => title.to_string(),
                None => derived_title(document),
            },
            plain_text: document.plain_text(),
            words: stats.words as u64,
            characters: stats.characters as u64,
        }
    }

    /// The library's borrowed view of this form.
    pub fn content(&self) -> DocumentContent<'_> {
        DocumentContent {
            content: &self.json,
            title: &self.title,
            plain_text: &self.plain_text,
            word_count: self.words,
            char_count: self.characters,
        }
    }
}

/// The title the text of `document` gives: its first heading, else its first
/// non-empty line, else [`UNTITLED`].
pub fn derived_title(document: &Document) -> String {
    document
        .title()
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| UNTITLED.to_string())
}

/// A document read back from the library.
#[derive(Clone, Debug)]
pub struct LoadedDocument {
    /// The document.
    pub document: Document,
    /// Why the stored content could not be read as a document, when it could
    /// not. The document then holds the stored content as plain text and the
    /// caller should keep a snapshot of the original before saving over it.
    pub recovered_from: Option<LoadError>,
}

/// Reads stored content leniently. writer-core already repairs damaged
/// documents; content that is not a document at all (not JSON, another
/// format, a newer version) is shown as plain text instead of being
/// refused, so nothing the user wrote becomes unreachable. Empty content is
/// an empty document.
pub fn load_document(content: &str) -> LoadedDocument {
    if content.trim().is_empty() {
        return LoadedDocument {
            document: Document::new(),
            recovered_from: None,
        };
    }
    match Document::from_json(content) {
        Ok(document) => LoadedDocument {
            document,
            recovered_from: None,
        },
        Err(error) => LoadedDocument {
            document: Fragment::from_plain_text(content).to_document(),
            recovered_from: Some(error),
        },
    }
}
