//! The one place where Markdown is read: everything in this crate that takes Markdown
//! goes through [`import`], which is `writer_export::from_markdown` with the options an
//! agent's text wants.

use writer_core::{Block, Document, DocumentSettings};
use writer_export::{MarkdownImportOptions, from_markdown, summarize_warnings};

/// Markdown, read.
pub(crate) struct Imported {
    /// The blocks as a document with the given settings. One empty paragraph when the
    /// text held nothing.
    pub(crate) document: Document,
    /// Whether the text held anything.
    pub(crate) has_content: bool,
    /// What could not be kept as written, one sentence per kind of thing.
    pub(crate) warnings: Vec<String>,
}

impl Imported {
    /// The blocks the text gave; none when it held nothing.
    pub(crate) fn blocks(&self) -> &[Block] {
        if self.has_content {
            self.document.blocks()
        } else {
            &[]
        }
    }
}

/// Reads `markdown`. A level-1 heading that opens the text stays in the text (the title
/// of a document follows its first heading); front matter is taken out and reported.
pub(crate) fn import(markdown: &str, settings: &DocumentSettings) -> Imported {
    let options = MarkdownImportOptions {
        settings: settings.clone(),
        ..MarkdownImportOptions::default()
    };
    let imported = from_markdown(markdown, &options);
    Imported {
        has_content: !imported.is_empty(),
        warnings: summarize_warnings(&imported.warnings),
        document: imported.document,
    }
}
