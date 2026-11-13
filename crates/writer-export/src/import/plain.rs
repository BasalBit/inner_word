//! Plain-text import: paragraphs separated by blank lines.
//!
//! The inverse of how plain text is usually written, and of what
//! [`to_plain_text`](crate::to_plain_text) writes for paragraphs: one or
//! more blank lines separate paragraphs, and a single line break inside a
//! paragraph is a line break of the paragraph. Nothing is read as markup:
//! a line that starts with `#` or `-` stays text.
//!
//! ```
//! use writer_export::{PlainTextImportOptions, from_plain_text};
//!
//! let imported = from_plain_text(
//!     "First paragraph,\nsecond line.\n\n\nSecond paragraph.\n",
//!     &PlainTextImportOptions::default(),
//! );
//! let blocks = imported.document.blocks();
//! assert_eq!(blocks.len(), 2);
//! assert_eq!(blocks[0].text(), "First paragraph,\nsecond line.");
//! assert_eq!(blocks[1].text(), "Second paragraph.");
//! ```

use writer_core::{Block, Document, DocumentSettings, text::normalize_input};

use super::Imported;

/// What a single line break inside a paragraph of plain text becomes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlainTextLines {
    /// A line break inside the paragraph. The default: verse, addresses and
    /// posts keep their lines.
    #[default]
    LineBreaks,
    /// A new paragraph: every line is a paragraph, blank lines only
    /// separate them.
    Paragraphs,
    /// A space: the lines of a paragraph are joined, for text that was
    /// wrapped at a fixed width.
    Spaces,
}

/// Options for [`from_plain_text`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlainTextImportOptions {
    /// What a single line break inside a paragraph becomes.
    pub lines: PlainTextLines,
    /// The settings of the imported document (page setup, typography,
    /// language). Pass the settings of the document whose content is being
    /// replaced to keep them.
    pub settings: DocumentSettings,
}

/// Imports plain text: paragraphs separated by blank lines.
///
/// Line endings of every kind are understood, control characters are
/// removed, white space at the end of a line is dropped (at its start it is
/// kept: it may be an indent someone typed). The result has no title, no
/// front matter and no warnings.
pub fn from_plain_text(text: &str, options: &PlainTextImportOptions) -> Imported {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let text = normalize_input(text);
    let mut blocks: Vec<Block> = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let flush = |paragraph: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if paragraph.is_empty() {
            return;
        }
        match options.lines {
            PlainTextLines::LineBreaks => blocks.push(Block::paragraph(&paragraph.join("\n"))),
            PlainTextLines::Paragraphs => {
                blocks.extend(paragraph.iter().map(|line| Block::paragraph(line)));
            }
            PlainTextLines::Spaces => {
                let joined: Vec<&str> = paragraph.iter().map(|line| line.trim_start()).collect();
                let first_indent = paragraph[0].len() - paragraph[0].trim_start().len();
                let mut text = paragraph[0][..first_indent].to_string();
                text.push_str(&joined.join(" "));
                blocks.push(Block::paragraph(&text));
            }
        }
        paragraph.clear();
    };
    for line in text.split('\n') {
        let line = line.trim_end();
        if line.is_empty() {
            flush(&mut paragraph, &mut blocks);
        } else {
            paragraph.push(line);
        }
    }
    flush(&mut paragraph, &mut blocks);
    let has_content = !blocks.is_empty();
    Imported {
        document: Document::from_blocks(blocks).with_settings(options.settings.clone()),
        title: None,
        front_matter: None,
        warnings: Vec::new(),
        image_problems: Vec::new(),
        has_content,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use writer_core::BlockKind;

    fn texts(text: &str, lines: PlainTextLines) -> Vec<String> {
        let imported = from_plain_text(
            text,
            &PlainTextImportOptions {
                lines,
                ..Default::default()
            },
        );
        imported.document.check_invariants().unwrap();
        assert!(imported.warnings.is_empty());
        assert_eq!(imported.title, None);
        if imported.is_empty() {
            return Vec::new();
        }
        imported
            .document
            .blocks()
            .iter()
            .map(|b| {
                assert_eq!(b.kind(), &BlockKind::Paragraph);
                b.text().to_string()
            })
            .collect()
    }

    #[test]
    fn paragraphs_and_lines() {
        use PlainTextLines::*;
        let text = "One,\ntwo.  \n \t\n\n  Three\n";
        assert_eq!(texts(text, LineBreaks), ["One,\ntwo.", "  Three"]);
        assert_eq!(texts(text, Paragraphs), ["One,", "two.", "  Three"]);
        assert_eq!(texts(text, Spaces), ["One, two.", "  Three"]);
        assert_eq!(texts("  a\n   b\nc", Spaces), ["  a b c"]);
    }

    #[test]
    fn line_endings_and_odd_input() {
        use PlainTextLines::*;
        assert_eq!(texts("a\r\nb\r\n\r\nc\rd", LineBreaks), ["a\nb", "c\nd"]);
        assert_eq!(texts("\u{feff}a\u{2029}b", LineBreaks), ["a\nb"]);
        assert_eq!(texts("a\u{0}b\tc\u{7}", LineBreaks), ["ab\tc"]);
        assert_eq!(texts("# not a heading\n- not a list", Paragraphs).len(), 2);
        assert_eq!(texts("😀 שלום", LineBreaks), ["😀 שלום"]);
        for empty in ["", "\n", " \n\t\n", "\r\n\r\n"] {
            assert!(texts(empty, LineBreaks).is_empty(), "{empty:?}");
            let imported = from_plain_text(empty, &Default::default());
            assert_eq!(
                imported.document,
                Document::new().with_settings(Default::default())
            );
            assert!(imported.fragment().blocks().is_empty());
        }
    }

    #[test]
    fn settings_are_kept() {
        let mut settings = DocumentSettings {
            page_view: true,
            ..Default::default()
        };
        settings.typography.line_height = 2.0;
        let imported = from_plain_text(
            "x",
            &PlainTextImportOptions {
                settings: settings.clone(),
                ..Default::default()
            },
        );
        assert_eq!(imported.document.settings(), &settings);
        assert_eq!(imported.fragment().blocks().len(), 1);
    }
}
