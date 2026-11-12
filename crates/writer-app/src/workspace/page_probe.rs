//! How much text a page holds: a measurement for scripted runs.
//!
//! The `pagefit:` step of a scripted run lays a text out in page view in a
//! font, at a size, with a line spacing, on a paper, and says what came of
//! it: how many characters the lines hold and how many lines a page holds.
//! It is how the page is compared with another program's at the same
//! settings (Word, or CoreText itself), with the fonts of this Mac, which
//! no headless test has.
//!
//! Nothing here is used by the app outside a scripted run.

use gpui::{Context, Window};
use writer_core::{
    Block, Document, DocumentSettings, Margins, Orientation, PageSetup, PaperSize, Position,
    Typography,
};

use super::Workspace;
use crate::document_io::StoredForm;

/// What a `pagefit:` step asks for.
#[derive(Clone, Debug, PartialEq)]
pub struct PageFit {
    /// The font family of the text.
    pub font: String,
    /// The text size in points.
    pub size: f32,
    /// The line spacing, as the document settings have it: a multiple of
    /// the text size.
    pub line_height: f32,
    /// The paper.
    pub paper: PaperSize,
    /// The file the text is read from: one paragraph (line breaks in the
    /// file count as spaces).
    pub text: std::path::PathBuf,
}

impl PageFit {
    /// Parses `<font>|<size>|<line spacing>|<a4 or letter>|<text file>`.
    pub fn parse(spec: &str) -> anyhow::Result<Self> {
        let parts: Vec<&str> = spec.split('|').map(str::trim).collect();
        let [font, size, line_height, paper, text] = parts.as_slice() else {
            anyhow::bail!("expected <font>|<size>|<line spacing>|<a4 or letter>|<text file>");
        };
        let number = |text: &str, what: &str| -> anyhow::Result<f32> {
            text.parse::<f32>()
                .ok()
                .filter(|value| value.is_finite() && *value > 0.)
                .ok_or_else(|| anyhow::anyhow!("expected {what}, got {text:?}"))
        };
        Ok(Self {
            font: font.to_string(),
            size: number(size, "a text size")?,
            line_height: number(line_height, "a line spacing")?,
            paper: match paper.to_ascii_lowercase().as_str() {
                "a4" => PaperSize::A4,
                "letter" | "usletter" | "us-letter" => PaperSize::UsLetter,
                other => anyhow::bail!("expected a4 or letter, got {other:?}"),
            },
            text: std::path::PathBuf::from(text),
        })
    }
}

/// What the lines of a paragraph hold: `(rows, mean, least, most)` of the
/// characters in its full rows (every row but the last; the space a row
/// ends in counts). `starts` are the byte offsets at which the rows of
/// `text` begin.
pub fn characters_per_row(text: &str, starts: &[usize]) -> (usize, f32, usize, usize) {
    let mut counts: Vec<usize> = starts
        .windows(2)
        .map(|pair| text[pair[0]..pair[1]].chars().count())
        .collect();
    if counts.is_empty() {
        counts.push(text.chars().count());
    }
    let total: usize = counts.iter().sum();
    (
        starts.len(),
        total as f32 / counts.len() as f32,
        counts.iter().copied().min().unwrap_or(0),
        counts.iter().copied().max().unwrap_or(0),
    )
}

impl Workspace {
    /// Creates a document with the text of `fit` in page view at its
    /// settings (2.54 cm margins, nothing after a paragraph) and opens it:
    /// the paragraph by itself, then enough of it again to fill pages.
    /// Returns the paragraph. [`Workspace::report_page_fit`] says what the
    /// page holds, once a frame has drawn it.
    pub fn open_page_fit(
        &mut self,
        fit: &PageFit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<String> {
        let text = std::fs::read_to_string(&fit.text)
            .map_err(|error| anyhow::anyhow!("reading {}: {error}", fit.text.display()))?;
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            anyhow::bail!("{} has no text", fit.text.display());
        }
        let settings = DocumentSettings {
            page_view: true,
            page: PageSetup {
                paper: fit.paper,
                orientation: Orientation::Portrait,
                margins: Margins::uniform(72.),
            },
            typography: Typography {
                body_font: fit.font.clone(),
                base_size: fit.size,
                line_height: fit.line_height,
                paragraph_spacing: 0.,
            },
            ..DocumentSettings::default()
        }
        .sanitized();
        let document = Document::from_blocks(vec![
            Block::paragraph(&text),
            Block::paragraph(&[text.as_str(); 4].join(" ")),
        ])
        .with_settings(settings);
        let form = StoredForm::of(&document, None);
        let meta = self
            .library
            .update(cx, |library, cx| {
                library.create_document(None, &form.content(), cx)
            })
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        if !self.open_document(meta.id, true, window, cx) {
            anyhow::bail!("the document could not be opened");
        }
        Ok(text)
    }

    /// What the page of the open document holds, one line per fact: for
    /// the document [`Workspace::open_page_fit`] opened, whose first
    /// paragraph is `text`.
    pub fn report_page_fit(
        &mut self,
        text: &str,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<Vec<String>> {
        let editor = self.editor.clone();
        editor.update(cx, |editor, cx| editor.finish_layout(cx));
        if editor.read(cx).document().block_count() < 2 || !editor.read(cx).page_view() {
            anyhow::bail!("the open document is not one a pagefit step made");
        }

        // The rows of both paragraphs, and the page each is on.
        let mut starts: Vec<Vec<usize>> = Vec::new();
        let mut on_first_page = 0;
        for block in 0..2 {
            let length = editor.read(cx).document().blocks()[block].len();
            let mut rows = Vec::new();
            let mut at = Position::new(block, 0);
            loop {
                let Some(row) = editor.read(cx).visual_row(at) else {
                    anyhow::bail!("the text was not laid out yet");
                };
                rows.push(row.start.offset);
                let page = editor.update(cx, |editor, cx| {
                    editor.edit_with(
                        writer_editor::Autoscroll::None,
                        |state| state.set_caret(row.start),
                        cx,
                    );
                    editor.caret_page()
                });
                if page == Some(1) {
                    on_first_page += 1;
                }
                if row.end.offset >= length || row.end.offset <= row.start.offset {
                    break;
                }
                at = row.end;
            }
            starts.push(rows);
        }
        let (rows, mean, least, most) = characters_per_row(text, &starts[0]);
        let editor = editor.read(cx);
        let settings = editor.document().settings();
        let (width, height) = settings.page.content_size();
        let line = editor
            .position_bounds(Position::new(0, 0))
            .map(|bounds| f32::from(bounds.size.height) / (editor.zoom_percent() / 100.));
        let paper = match settings.page.paper {
            PaperSize::A4 => "A4",
            PaperSize::UsLetter => "Letter",
            PaperSize::Custom { .. } => "custom paper",
        };
        Ok(vec![
            format!(
                "{} {} pt, line spacing {} ({} pt a line), {paper}, text area {width:.1} x \
                 {height:.1} pt",
                settings.typography.body_font,
                settings.typography.base_size,
                settings.typography.line_height,
                line.map_or("?".to_string(), |line| format!("{line:.2}")),
            ),
            format!(
                "the paragraph ({} characters) is {rows} rows; a full row holds {mean:.1} \
                 characters ({least} to {most})",
                text.chars().count()
            ),
            format!(
                "{on_first_page} rows on the first page; {} pages in all",
                editor.page_count().unwrap_or(0)
            ),
            format!(
                "row starts: {}",
                starts[0]
                    .iter()
                    .map(|start| start.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        ])
    }
}
