//! Export of a document, or of a folder as one file, to a path on disk.

use crate::error::Result;
use crate::library::{Library, unfinished};
use crate::types::{ExportRequest, Exported};

impl Library {
    /// Writes a document, or a folder as one file, as Markdown, DOCX or plain text. Only
    /// reads the library, so it works while the app is open. See [`ExportRequest`].
    pub fn export(&self, request: &ExportRequest) -> Result<Exported> {
        let _ = request;
        unfinished("export")
    }
}
