//! DOCX export (stub while the package writer is being built).

mod zip;

use crate::ExportSource;

/// Options for [`to_docx`].
#[derive(Clone, Debug, Default)]
pub struct DocxOptions {
    /// Creation time.
    pub created: i64,
    /// Modification time.
    pub modified: i64,
}

/// Exports `source` as a DOCX package.
pub fn to_docx(source: &ExportSource<'_>, options: &DocxOptions) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(options.modified);
    z.add("title.txt", source.title().as_bytes());
    z.finish()
}
