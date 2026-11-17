//! "Export as PDF…": a document, or a folder as one file, as a PDF.
//!
//! Reached from the export picker ("PDF file" for the document and for its
//! folder, [`crate::export::FileFormat::Pdf`]), from the command "Export as
//! PDF…" (the palette, File in the menu bar, the menu of the title; no key
//! by default) and, for a folder, from "Export Folder…" on its row. With a
//! page open the command is the page's own PDF, as WebKit prints it
//! (`page_export.rs`); with nothing open it has no handler, as Print has
//! none.
//!
//! # The same pages as "Print…"
//!
//! A PDF export is a job of pages (`workspace/print.rs`): the same print
//! pass ([`innerword_editor::PrintPass`], the document laid out as page view
//! lays it out, whatever the window shows), the same pictures read in the
//! background beforehand, the same slices of 8 ms with a turn of the main
//! thread in between, the same `innerword_print::PdfWriter`. So an exported
//! PDF has the pages "Print…" makes and "Save as PDF" in the print panel
//! writes: the same paper, the same rows, the same pictures. The open
//! document is taken as it is in the editor, unsaved text included; a
//! document that is not open is read from the library and laid out by the
//! same pass ([`innerword_editor::PrintPass::new`]).
//!
//! # A folder as one PDF
//!
//! Each document of the folder (subfolders included, in the order of the
//! other formats; pages left out, which the toast says) is laid out by a
//! pass of its own, **on its own paper**: its page setup and typography,
//! as a Word export gives each document a section of its own. A PDF holds
//! pages of different sizes, so nothing has to be chosen for the whole,
//! and every document looks as it looks when it is printed alone. Each
//! starts on a new page because each is a pass of its own. The pages are
//! numbered on through the file (a Word export does not start again
//! either). As in the other formats of a folder, a document's title is set
//! above it as a heading 1 unless the document already shows it: one of
//! its headings, or its first line, starts with the title ([`shows_title`],
//! the rule of `innerword_export::PartTitles::WhenMissing`). Page breaks at
//! the start of a document after the first and at the end of one before
//! the last are left out, as the Word export leaves them out: a new
//! document is a new page already.
//!
//! # Step by step
//!
//! 1. What is exported is taken when the export is chosen
//!    ([`Workspace::gather_export`], which every format uses): what is typed
//!    afterwards is not in it.
//! 2. Where it goes is asked next, in the save panel ("<title>.pdf"; a
//!    scripted run writes into `INNER_WORD_EXPORT_DIR` instead). No page is
//!    made before a file is chosen, so a cancelled panel costs nothing but
//!    the copies of the documents.
//! 3. The pages are made as a print job's are, with the note "Preparing the
//!    PDF of …" after 0.4 s whose Cancel is the command "Cancel PDF
//!    export" (also in File and the palette, and a click that takes the
//!    note away cancels too). One job of pages at a time: while a print or
//!    another PDF is under way, an export says so and does nothing.
//!    A picture the library has and that cannot be read (`images.db` of a
//!    newer version, busy past its timeout) ends the export before its
//!    first page, with nothing written ([`crate::export::pictures_not_read`],
//!    as for every format); a picture the library does not have is its box
//!    with its description, as on paper.
//! 4. The PDF's bytes are taken out of it on the main thread and written by
//!    a background thread ([`write`]); a toast says where the file is.

use std::{path::PathBuf, time::Instant};

use gpui::{App, AsyncWindowContext, Context, Stateful, WeakEntity, Window, prelude::*};
use innerword_core::{Block, BlockKind, Document};
use innerword_editor::PrintPass;
use innerword_ui::Toast;

use crate::{
    actions::{CancelPdfExport, ExportPdf},
    export::{
        ExportScope, ExportTarget, FileFormat, Gathered, pages_left_out_sentence, pdf_file_name,
        short_path, short_title, with_extension,
    },
    page_export::PageExportChoice,
    printing::PrintJob,
    settings::keys,
    workspace::{PagesPurpose, PdfExporting, Workspace},
};

/// Where the PDF of an export goes once its pages are made, and what the
/// toast says about it.
pub(crate) struct Destination {
    /// The file chosen in the save panel.
    path: PathBuf,
    /// The title of what is exported: the document's or the folder's.
    title: String,
    /// How many documents it holds.
    documents: usize,
    /// How many pages of the folder were left out.
    pages_left_out: usize,
    /// When the pages began to be made.
    started: Instant,
}

/// What an export of a folder holds besides its documents, until the
/// file is chosen.
struct Planned {
    title: String,
    file_name: String,
    documents: Vec<Document>,
    pages_left_out: usize,
}

impl Workspace {
    /// Whether "Export as PDF…" is there: a document is open, or a page in
    /// a run that shows pages (WebKit prints it).
    pub fn can_export_pdf(&self, cx: &App) -> bool {
        self.session.is_some() || (self.page.is_some() && crate::pages::engine(cx).is_some())
    }

    /// The handlers of "Export as PDF…" and "Cancel PDF export", on the
    /// workspace root: the first while [`Workspace::can_export_pdf`], the
    /// second while the pages of a PDF export are being prepared. The menu
    /// bar greys their items out otherwise.
    pub(crate) fn pdf_export_actions(
        &self,
        mut root: Stateful<gpui::Div>,
        cx: &mut Context<Self>,
    ) -> Stateful<gpui::Div> {
        if self.can_export_pdf(cx) {
            root = root.on_action(cx.listener(|this, _: &ExportPdf, window, cx| {
                this.export_as_pdf(window, cx);
            }));
        }
        if self.pdf_exporting() == Some(PdfExporting::Preparing) {
            root = root.on_action(cx.listener(|this, _: &CancelPdfExport, _, cx| {
                this.cancel_pdf_export(cx);
            }));
        }
        root
    }

    /// "Export as PDF…": the open page as WebKit prints it, else the open
    /// document. Does nothing with neither.
    pub fn export_as_pdf(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(page) = self.open_page_id() {
            if crate::pages::engine(cx).is_some() {
                self.export_page(page, PageExportChoice::Pdf, window, cx);
            }
            return;
        }
        if self.session.is_some() {
            self.export_pdf(ExportTarget::Open, ExportScope::Document, window, cx);
        }
    }

    /// Exports `target` as a PDF (see the [module docs](self)): the
    /// document it names, or the folder it names or its document is in
    /// (`scope`). Asks where first; the outcome is told in a toast.
    pub fn export_pdf(
        &mut self,
        target: ExportTarget,
        scope: ExportScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A page has a PDF of its own.
        let page = match (target, scope) {
            (ExportTarget::Open, ExportScope::Document) => self.open_page_id(),
            (ExportTarget::Document(id), ExportScope::Document) if self.is_page(id, cx) => Some(id),
            _ => None,
        };
        if let Some(page) = page {
            self.export_page(page, PageExportChoice::Pdf, window, cx);
            return;
        }
        if let Some(sentence) = self.pages_under_way() {
            self.print.last_export = Some(Err(sentence.clone()));
            self.show_toast(Toast::new(sentence), cx);
            return;
        }
        let planned = match self.gather_export(target, scope, cx) {
            Ok(gathered) => plan(gathered),
            Err(error) => {
                self.print.last_export = Some(Err(error.clone()));
                self.toast_error(format!("Could not export: {error}"), cx);
                return;
            }
        };
        // A scripted run cannot drive the native save panel: it names the
        // directory instead and the suggested file name is used as it is.
        let scripted = crate::automation::export_directory();
        let receiver = scripted.is_none().then(|| {
            let directory = self.export_directory(cx);
            cx.prompt_for_new_path(&directory, Some(&planned.file_name))
        });
        cx.spawn_in(window, async move |this, cx| {
            let chosen = match (scripted, receiver) {
                (Some(directory), _) => directory.join(&planned.file_name),
                (None, Some(receiver)) => match receiver.await {
                    Ok(Ok(Some(path))) => path,
                    // Cancelled: no page was made.
                    Ok(Ok(None)) | Err(_) => return,
                    Ok(Err(error)) => {
                        this.update(cx, |this, cx| {
                            this.toast_error(format!("Could not open the save panel: {error}"), cx);
                        })
                        .ok();
                        return;
                    }
                },
                (None, None) => return,
            };
            let path = with_extension(chosen, FileFormat::Pdf);
            this.update_in(cx, |this, window, cx| {
                this.make_the_pdf(planned, path, window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Starts making the pages of `planned` for the file at `path`.
    fn make_the_pdf(
        &mut self,
        planned: Planned,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A print may have begun while the save panel was up.
        if let Some(sentence) = self.pages_under_way() {
            self.print.last_export = Some(Err(sentence.clone()));
            self.show_toast(Toast::new(sentence), cx);
            return;
        }
        let destination = Destination {
            path,
            title: planned.title.clone(),
            documents: planned.documents.len(),
            pages_left_out: planned.pages_left_out,
            started: Instant::now(),
        };
        let passes = planned
            .documents
            .into_iter()
            .map(|document| PrintPass::new(document, window, cx))
            .collect();
        self.make_pages(
            passes,
            planned.title,
            PagesPurpose::Export(destination),
            window,
            cx,
        );
    }

    /// What came of the last PDF export since this was last asked (the
    /// file and the report of its pages, or why nothing was written), for
    /// the scripted step `export:wait`. `None` while an export is under
    /// way, and when none ended since.
    pub fn take_pdf_export_report(&mut self) -> Option<Result<String, String>> {
        if self.pdf_exporting().is_some() {
            return None;
        }
        self.print.last_export.take()
    }
}

/// The documents of `gathered` as they go on paper, with the title and
/// the name of the file.
fn plan(gathered: Gathered) -> Planned {
    let file_name = pdf_file_name(&gathered.title);
    let documents = if gathered.collection {
        on_paper(gathered.parts)
    } else {
        gathered
            .parts
            .into_iter()
            .map(|(_, document)| document)
            .collect()
    };
    Planned {
        title: gathered.title,
        file_name,
        documents,
        pages_left_out: gathered.pages_left_out,
    }
}

/// The documents of a folder as they go into its PDF, one pass each (the
/// [module docs](self), "A folder as one PDF"): each with its title above
/// it as a heading 1 unless it shows the title already, and without the
/// page breaks a new document makes needless (at the start of every
/// document but the first, at the end of every one but the last).
/// Everything else of a document, its settings included, is as it is.
pub fn on_paper(parts: Vec<(String, Document)>) -> Vec<Document> {
    let count = parts.len();
    parts
        .into_iter()
        .enumerate()
        .map(|(index, (title, document))| {
            part_on_paper(&title, document, index == 0, index + 1 == count)
        })
        .collect()
}

fn part_on_paper(title: &str, document: Document, first: bool, last: bool) -> Document {
    let is_break = |block: &&Block| matches!(block.kind(), BlockKind::PageBreak);
    let blocks = document.blocks();
    let skip = if first {
        0
    } else {
        blocks.iter().take_while(is_break).count()
    };
    let keep = if last {
        blocks.len()
    } else {
        blocks.len() - blocks.iter().rev().take_while(is_break).count()
    }
    .max(skip);
    let heading = !shows_title(&document, title);
    if !heading && skip == 0 && keep == blocks.len() {
        return document;
    }
    let mut out = Vec::with_capacity(keep - skip + 1);
    if heading {
        out.push(Block::with_text(BlockKind::heading(1), &clean_title(title)));
    }
    out.extend(blocks[skip..keep].iter().cloned());
    Document::from_blocks(out).with_settings(document.settings().clone())
}

/// A title as the other formats of a folder write it above a document:
/// its white space made single, "Untitled" for none.
fn clean_title(title: &str) -> String {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        innerword_export::UNTITLED.to_string()
    } else {
        title
    }
}

/// Whether `document` shows `title` already, so that a folder's PDF does
/// not set it above the document: one of its headings, or its first line
/// of text, starts with the title (case and white space aside). The rule of
/// `innerword_export::PartTitles::WhenMissing`, by which the Markdown, Word
/// and plain-text exports of a folder write a title; `innerword-export` keeps
/// its own copy to itself, and a test holds the two to the same answers.
pub fn shows_title(document: &Document, title: &str) -> bool {
    let fold = |text: &str| {
        text.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let title = fold(&clean_title(title));
    if title.is_empty() {
        return true;
    }
    let first_line = |block: &Block| {
        fold(
            block
                .text()
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or(""),
        )
    };
    let mut first_text = true;
    for block in document.blocks() {
        if block.text().trim().is_empty() {
            continue;
        }
        let heading = block.kind().heading_level().is_some();
        if (heading || first_text) && first_line(block).starts_with(&title) {
            return true;
        }
        first_text = false;
    }
    false
}

/// Writes the PDF of a finished export into its file: the bytes are taken
/// out of the PDF here, on the main thread (it is not `Send`), and written
/// by a background thread; then the toast, and the job is over.
pub(crate) async fn write(
    job: PrintJob,
    destination: Destination,
    this: &WeakEntity<Workspace>,
    cx: &mut AsyncWindowContext,
) {
    let made = destination.started.elapsed();
    let report = {
        let mut lines = vec![format!(
            "{}, made in {:.2} s",
            job.pdf.report.line(job.paper),
            made.as_secs_f64()
        )];
        lines.extend(job.pdf.report.missing_line());
        lines.join("; ")
    };
    let bytes = (!job.pdf.is_empty()).then(|| job.pdf.bytes().to_vec());
    drop(job);
    if this
        .update(cx, |this, cx| this.pdf_is_being_written(cx))
        .is_err()
    {
        return;
    }
    let path = destination.path.clone();
    let written = match bytes {
        Some(bytes) => {
            cx.background_spawn(async move { std::fs::write(&path, &bytes) })
                .await
        }
        None => Err(std::io::Error::other("the PDF could not be made")),
    };
    this.update(cx, |this, cx| {
        match written {
            Ok(()) => {
                if let Some(parent) = destination.path.parent() {
                    let parent = parent.to_string_lossy().to_string();
                    this.library
                        .update(cx, |library, cx| {
                            library.set_setting(keys::EXPORT_DIRECTORY, &parent, cx)
                        })
                        .ok();
                }
                let what = if destination.documents == 1 {
                    format!("“{}”", short_title(&destination.title))
                } else {
                    format!("{} documents", destination.documents)
                };
                let mut said = format!("Exported {what} to {}", short_path(&destination.path));
                if destination.pages_left_out > 0 {
                    said.push_str(". ");
                    said.push_str(&pages_left_out_sentence(destination.pages_left_out));
                }
                this.print.last_export = Some(Ok(format!(
                    "“{}” written to {}: {report}",
                    destination.title,
                    destination.path.display()
                )));
                this.show_toast(Toast::new(said).success(), cx);
            }
            Err(error) => {
                let said = format!("Could not write {}: {error}", short_path(&destination.path));
                this.print.last_export = Some(Err(said.clone()));
                this.toast_error(said, cx);
            }
        }
        this.print_ended(cx);
    })
    .ok();
}
