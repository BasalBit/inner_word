//! "Print…": the open document on paper.
//!
//! What is printed is the document as it is in the editor at that moment,
//! unsaved text included, laid out as page view lays it out: its paper,
//! orientation, margins, type, page breaks and page numbers, in Paper's
//! colours and at real size, whatever the window shows (flow view, any
//! zoom, Night, a selection). That is the editor's doing
//! ([`writer_editor::PrintPass`]); the pages become a PDF with real text in
//! it (`writer_print::PdfWriter`), and the PDF goes to whoever takes print
//! jobs in this run ([`crate::printing::PrintTarget`]): the macOS print
//! panel as a sheet on the window, a recorder in a test, a directory in a
//! scripted run.
//!
//! # A job, step by step ([`Workspace::print`])
//!
//! 1. The pass takes a copy of the document. From here on the job is
//!    **preparing** ([`Printing::Preparing`]); what is typed or changed by
//!    an agent meanwhile is not in the pages.
//! 2. The pictures of the document are read from the library, in the
//!    background ([`crate::images::LibraryImages::bytes_of_all`]), and
//!    handed to the pass ([`writer_editor::PrintPass::set_images`]): no
//!    picture is read on the main thread inside a slice. They stay in
//!    memory, as the files they are stored as, until the PDF is finished
//!    (it holds each picture once). A picture the library does not have
//!    is printed as its box with its description.
//! 3. A task of the main thread makes the pages in slices
//!    ([`crate::printing::slice`], 8 ms: laying blocks out and drawing
//!    pages into the PDF, both within the slice); between two slices it
//!    waits a moment on a timer, which is a turn of the main thread: keys
//!    are answered and frames are drawn, so a book does not freeze the
//!    window. A chapter is done in the first slice. Pages that are laid
//!    out and not drawn yet wait for the next slice (a slice draws at
//!    least one): a page of photographs is laid out in no time and takes
//!    long to draw, and fifty of them in one turn froze the window for two
//!    seconds.
//! 4. A job that is still preparing after [`NOTE_DELAY`] says so in a note
//!    at the bottom, "Preparing to print …", whose button is the command
//!    "Cancel printing". The command is in the palette and in File too (a
//!    toast's button has no way in from the keyboard), and "Dismiss
//!    notifications" cancels as well: otherwise the note would go, the work
//!    would go on unseen and the print panel would come up unannounced.
//!    For the same reason a click on the note (which takes any note away)
//!    cancels, and a note pushed out by four newer ones is shown again
//!    (`Workspace::print_note_went`).
//! 5. The finished job is handed over. With the macOS print panel the job
//!    is then **in the panel** ([`Printing::Panel`]) until the sheet has
//!    ended; the sheet is started from the task, outside any update, as
//!    AppKit calls back into GPUI while it comes up.
//!
//! While a job is preparing or in the panel a second "Print…" does
//! nothing.
//!
//! # A job of pages, for paper or for a file
//!
//! "Export as PDF…" (`crate::pdf_export`) makes its pages by the same
//! steps: the same pass (several passes for a folder, one per document,
//! numbered on), the same pictures read beforehand, the same slices, the
//! same writer, and a note "Preparing the PDF of …" whose Cancel is the
//! command "Cancel PDF export". Only the last step differs: the PDF is
//! written into the file chosen beforehand, off the main thread, instead
//! of being handed to the print target. So an exported PDF and the PDF of
//! "Print…" have the same pages. The workspace has one slot for such a
//! job ([`Purpose`]): while one is under way, a print does nothing more
//! (or says that a PDF is being made) and an export says what is under
//! way.
//!
//! # When the commands are there
//!
//! "Print" has a handler while a document is open **and** the app was
//! given a target; "Cancel printing" only while a job is preparing. The
//! menu bar greys an item out without a handler, and the title's menu has
//! "Print…" under the same condition.
//!
//! # For tests and scripted runs
//!
//! [`Workspace::print_job`] makes the same job at once and returns it
//! instead of handing it over.

use std::{
    collections::VecDeque,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{Context, Entity, Stateful, Task, Window, prelude::*};
use writer_editor::{PrintPass, PrintProgress};
use writer_print::PdfWriter;
use writer_ui::{
    Toast, ToastLayer,
    toast::{MAX_VISIBLE, ToastId},
};

use super::Workspace;
use crate::{
    actions::{CancelPdfExport, CancelPrint, Print},
    export::short_path,
    printing::{self, PrintJob, PrintTarget},
};

/// How long a job prepares before the note about it appears. A chapter is
/// on its pages long before.
pub const NOTE_DELAY: Duration = Duration::from_millis(400);

/// The wait between two slices of preparing: a turn of the main thread,
/// in which keys are answered and a frame is drawn.
pub const SLICE_PAUSE: Duration = Duration::from_millis(1);

/// How long the note stays by itself: it is taken away when the job ends,
/// so this is only longer than any job.
const NOTE_STAYS: Duration = Duration::from_secs(60 * 60);

/// How many pages may wait to be drawn before more blocks are laid out: a
/// book is not held as pages and as a PDF at once.
const PAGES_AHEAD: usize = 4;

/// How often the window is asked for a frame while pages are prepared.
/// The lines the pass shapes lie in GPUI's line cache until the window has
/// drawn twice: with a frame now and then a book holds a moment's lines
/// there instead of all of them.
const FRAME_EVERY: Duration = Duration::from_millis(100);

/// What a scripted run says instead of opening the print panel.
pub const SCRIPTED_RUN_HAS_NO_PANEL: &str = "A scripted run does not open the print panel.";
/// What is said when the print panel could not be started.
pub const PANEL_COULD_NOT_OPEN: &str = "The print panel could not be opened.";

/// Where a print job is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Printing {
    /// Its pages are being made.
    Preparing,
    /// It is in the print panel.
    Panel,
}

/// Where a PDF export is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PdfExporting {
    /// Its pages are being made.
    Preparing,
    /// The PDF is made and is being written into its file.
    Writing,
}

/// What a job of pages makes its PDF for.
pub(crate) enum Purpose {
    /// "Print…": the PDF goes to whoever takes print jobs.
    Print(PrintTarget),
    /// "Export as PDF…": the PDF goes into a file chosen beforehand.
    Export(crate::pdf_export::Destination),
}

impl Purpose {
    fn kind(&self) -> Kind {
        match self {
            Purpose::Print(_) => Kind::Print,
            Purpose::Export(_) => Kind::Export,
        }
    }
}

/// Which of the two a job is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Print,
    Export,
}

/// Where a job of pages is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// Its pages are being made.
    Preparing,
    /// A print job in the print panel.
    Panel,
    /// An export's PDF being written into its file.
    Writing,
}

/// What the workspace keeps about the job of pages: a print or a PDF
/// export, one at a time.
#[derive(Default)]
pub(crate) struct PrintState {
    /// What the job is and where it is; `None` when there is none.
    stage: Option<(Kind, Stage)>,
    /// How far the pages are, over all of the job's passes.
    progress: Option<PrintProgress>,
    /// The pages that are laid out and not drawn into the PDF yet.
    waiting: usize,
    /// The task that makes the pages and hands the job over. Dropping it
    /// drops the pass and the half-written PDF.
    task: Option<Task<()>>,
    /// The timer of the note.
    note_task: Option<Task<()>>,
    /// The note "Preparing to print …", while it is up.
    note: Option<ToastId>,
    /// The title the note names: the document's when the job began.
    title: String,
    /// What came of the last PDF export, for the scripted step
    /// `export:wait`: what was written, or why nothing was.
    pub(crate) last_export: Option<Result<String, String>>,
}

/// The words of the note for a document called `title`.
pub fn preparing_note(title: &str) -> String {
    format!("Preparing to print “{title}”…")
}

/// The words of the note of a PDF export of `title` (a document or a
/// folder).
pub fn pdf_note(title: &str) -> String {
    format!("Preparing the PDF of “{title}”…")
}

impl Workspace {
    // ----- what is going on -----------------------------------------------------------

    /// Where the print job is, if there is one.
    pub fn printing(&self) -> Option<Printing> {
        match self.print.stage {
            Some((Kind::Print, Stage::Preparing)) => Some(Printing::Preparing),
            Some((Kind::Print, Stage::Panel)) => Some(Printing::Panel),
            _ => None,
        }
    }

    /// Where the PDF export is, if there is one.
    pub fn pdf_exporting(&self) -> Option<PdfExporting> {
        match self.print.stage {
            Some((Kind::Export, Stage::Preparing)) => Some(PdfExporting::Preparing),
            Some((Kind::Export, Stage::Writing)) => Some(PdfExporting::Writing),
            _ => None,
        }
    }

    /// What the workspace is doing with pages, in words, while it is doing
    /// something: what a second job is told.
    pub(crate) fn pages_under_way(&self) -> Option<String> {
        let title = &self.print.title;
        match self.print.stage? {
            (Kind::Print, Stage::Panel) => Some(format!("“{title}” is in the print panel.")),
            (Kind::Print, _) => Some(format!("“{title}” is still being prepared to print.")),
            (Kind::Export, _) => Some(format!("The PDF of “{title}” is still being made.")),
        }
    }

    /// Whether "Print" is there: a document is open and the app was given
    /// a target for print jobs.
    pub fn can_print(&self, cx: &gpui::App) -> bool {
        let page_shown = self.page.is_some() && crate::pages::engine(cx).is_some();
        (self.session.is_some() || page_shown) && printing::target(cx).is_some()
    }

    /// The note about the job of pages that is preparing (a print or a
    /// PDF export), while it is up.
    pub fn print_note(&self) -> Option<ToastId> {
        self.print.note
    }

    /// One word about printing for the `state` step of a scripted run:
    /// `idle`, `preparing 40%` (the share of the blocks that are on their
    /// pages, less what of those pages is not drawn yet) or `panel`.
    pub fn describe_printing(&self) -> String {
        match self.printing() {
            None => "idle".to_string(),
            Some(Printing::Panel) => "panel".to_string(),
            Some(Printing::Preparing) => format!("preparing {}%", self.pages_percent()),
        }
    }

    /// The same for a PDF export: `idle`, `preparing 40%` or `writing`.
    pub fn describe_pdf_export(&self) -> String {
        match self.pdf_exporting() {
            None => "idle".to_string(),
            Some(PdfExporting::Writing) => "writing".to_string(),
            Some(PdfExporting::Preparing) => format!("preparing {}%", self.pages_percent()),
        }
    }

    /// How far the pages of the job are, in percent.
    fn pages_percent(&self) -> usize {
        self.print.progress.map_or(0, |progress| {
            let laid = (progress.blocks_done * 100)
                .checked_div(progress.blocks)
                .unwrap_or(100);
            // Of the pages handed out so far, those that are in the PDF.
            let made = progress.pages_done;
            let drawn = made.saturating_sub(self.print.waiting);
            (laid * drawn).checked_div(made).unwrap_or(laid)
        })
    }

    // ----- the commands ---------------------------------------------------------------

    /// The handlers of "Print" and "Cancel printing", on the workspace
    /// root: the first while a document is open and print jobs have
    /// somewhere to go, the second while pages are being prepared. The
    /// menu bar greys their items out otherwise.
    pub(crate) fn print_actions(
        &self,
        mut root: Stateful<gpui::Div>,
        cx: &mut Context<Self>,
    ) -> Stateful<gpui::Div> {
        if self.can_print(cx) {
            root = root.on_action(cx.listener(|this, _: &Print, window, cx| {
                this.print(window, cx);
            }));
        }
        if self.printing() == Some(Printing::Preparing) {
            root = root.on_action(cx.listener(|this, _: &CancelPrint, _, cx| {
                this.cancel_print(cx);
            }));
        }
        root
    }

    /// "Print…": prepares the pages of the open document and hands them
    /// to the target (see the [module docs](self)). Does nothing without a
    /// document, without a target, and while a job is preparing or in the
    /// print panel.
    pub fn print(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // One job of pages at a time; a PDF that is being made says so.
        if let Some((kind, _)) = self.print.stage {
            if kind == Kind::Export
                && let Some(sentence) = self.pages_under_way()
            {
                self.show_toast(Toast::new(sentence), cx);
            }
            return;
        }
        if self.page.is_some() {
            self.print_page(window, cx);
            return;
        }
        if self.session.is_none() {
            return;
        }
        let Some(target) = printing::target(cx) else {
            return;
        };
        let pass = self.editor.read(cx).print_pass(window, cx);
        let title = self.display_title();
        self.make_pages(vec![pass], title, Purpose::Print(target), window, cx);
    }

    /// Starts a job of pages: the passes, one after the other and numbered
    /// on, made into one PDF in slices (the [module docs](self), 2 to 4),
    /// which then goes where `purpose` says. `title` names the job in its
    /// note and in the PDF. Does nothing without a pass or while a job is
    /// under way.
    pub(crate) fn make_pages(
        &mut self,
        mut passes: Vec<PrintPass>,
        title: String,
        purpose: Purpose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if passes.is_empty() || self.print.stage.is_some() {
            return;
        }
        // The pictures of every pass, each once, read in the background
        // before the first page is made.
        let mut seen = std::collections::HashSet::new();
        let images: Vec<_> = passes
            .iter()
            .flat_map(PrintPass::images)
            .filter(|image| seen.insert(*image))
            .collect();
        let pictures = self.images.bytes_of_all(images, cx);
        let slice = printing::slice(cx);
        let kind = purpose.kind();
        self.print.stage = Some((kind, Stage::Preparing));
        self.print.progress = Some(progress_of(&passes, NOTHING_DONE));
        self.print.waiting = 0;
        self.print.note = None;
        self.print.title = title.clone();

        // The note, when the job is still preparing after a moment.
        self.print.note_task = Some(cx.spawn({
            let title = title.clone();
            async move |this, cx| {
                cx.background_executor().timer(NOTE_DELAY).await;
                this.update(cx, |this, cx| this.show_print_note(&title, cx))
                    .ok();
            }
        }));

        self.print.task = Some(cx.spawn_in(window, async move |this, cx| {
            // The job is "preparing" while they come, and Cancel drops
            // this task with them.
            let pictures = Rc::new(pictures.await);
            for pass in &mut passes {
                let pictures = pictures.clone();
                pass.set_images(Rc::new(move |image| pictures.get(&image).cloned()));
            }
            let Some(job) = prepare(passes, title, slice, &this, cx).await else {
                // The workspace is gone.
                return;
            };
            match purpose {
                Purpose::Print(target) => hand_over(job, target, &this, cx).await,
                Purpose::Export(destination) => {
                    crate::pdf_export::write(job, destination, &this, cx).await
                }
            }
        }));
        cx.notify();
    }

    /// "Print…" on a page: WebKit prints the view as it is now into a PDF
    /// (in its own process; the job is "preparing" with the same note and
    /// Cancel until the PDF is there), which goes to the target as a
    /// text's job does. `print_to_pdf` is called from the job's task,
    /// between two awaits and outside any update: AppKit may call the
    /// window's delegate meanwhile.
    fn print_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = printing::target(cx) else {
            return;
        };
        let Some(view) = self.page_view_handle() else {
            return;
        };
        let title = self.display_title();
        self.print.stage = Some((Kind::Print, Stage::Preparing));
        self.print.progress = None;
        self.print.waiting = 0;
        self.print.note = None;
        self.print.title = title.clone();
        self.print.note_task = Some(cx.spawn({
            let title = title.clone();
            async move |this, cx| {
                cx.background_executor().timer(NOTE_DELAY).await;
                this.update(cx, |this, cx| this.show_print_note(&title, cx))
                    .ok();
            }
        }));
        self.print.task = Some(cx.spawn_in(window, async move |this, cx| {
            // A turn of the main thread first: this is called from an
            // action handler, inside an update.
            cx.background_executor().timer(Duration::ZERO).await;
            let printed = super::page::print_view(&view).await;
            match printed {
                Ok(printed) => {
                    let job = PrintJob {
                        title,
                        paper: (printed.paper.0 as f32, printed.paper.1 as f32),
                        pdf: writer_print::Pdf::from_bytes(printed.bytes, printed.pages),
                    };
                    hand_over(job, target, &this, cx).await;
                }
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.toast_error(format!("The page could not be printed: {error}"), cx);
                        this.print_ended(cx);
                    })
                    .ok();
                }
            }
        }));
        cx.notify();
    }

    // ----- the stages -----------------------------------------------------------------
}

/// Hands a finished job to `target` and waits for its panel, if it goes to
/// one; the job is over afterwards.
async fn hand_over(
    job: PrintJob,
    target: PrintTarget,
    this: &gpui::WeakEntity<Workspace>,
    cx: &mut gpui::AsyncWindowContext,
) {
    {
        {
            // Handing over. `ended`: the panel the job is in, if it is in
            // one.
            let ended = match target {
                PrintTarget::Recorder(jobs) => {
                    jobs.borrow_mut().push(job.clone());
                    // A test's stand-in for the panel, if it has one.
                    let panel = cx.update(|_, cx| printing::held_panel(cx)).ok().flatten();
                    panel.map(|panel| panel.open())
                }
                PrintTarget::Directory(directory) => {
                    let said = match directory {
                        Some(directory) => write_job(&job, &directory),
                        None => Toast::new(SCRIPTED_RUN_HAS_NO_PANEL),
                    };
                    this.update(cx, |this, cx| this.show_toast(said, cx)).ok();
                    None
                }
                PrintTarget::SystemPanel => {
                    // The job is in the panel before the sheet is asked
                    // for: AppKit calls back into the app while it comes
                    // up.
                    let native = this
                        .update_in(cx, |this, window, cx| {
                            this.print_is_in_the_panel(cx);
                            printing::native_window(window)
                        })
                        .ok()
                        .flatten();
                    // Outside any update, from here to the next await.
                    let (end, ended) = printing::panel_end();
                    let shown = match native {
                        Some(native) => printing::show_panel(&job, native, end),
                        None => Err(writer_print::PrintError::Failed(
                            "The window is not on screen.".to_string(),
                        )),
                    };
                    match shown {
                        Ok(()) => Some(ended),
                        Err(error) => {
                            eprintln!("The Writer: the print panel did not open: {error}");
                            this.update(cx, |this, cx| {
                                this.toast_error(PANEL_COULD_NOT_OPEN, cx);
                            })
                            .ok();
                            None
                        }
                    }
                }
            };
            if let Some(ended) = ended {
                this.update(cx, |this, cx| this.print_is_in_the_panel(cx))
                    .ok();
                // The PDF is kept until the panel has ended.
                ended.await;
            }
            drop(job);
            this.update(cx, |this, cx| this.print_ended(cx)).ok();
        }
    }
}

impl Workspace {
    /// "Cancel printing": stops preparing the pages. Nothing is handed to
    /// the target. Does nothing unless a print is preparing (a job in the
    /// print panel is cancelled there).
    pub fn cancel_print(&mut self, cx: &mut Context<Self>) {
        if self.printing() == Some(Printing::Preparing) {
            self.cancel_pages(cx);
        }
    }

    /// "Cancel PDF export": stops preparing the pages of a PDF export.
    /// Nothing is written. Does nothing unless a PDF export is preparing.
    pub fn cancel_pdf_export(&mut self, cx: &mut Context<Self>) {
        if self.pdf_exporting() == Some(PdfExporting::Preparing) {
            self.cancel_pages(cx);
        }
    }

    /// Stops the job whose pages are being prepared, whichever it is.
    fn cancel_pages(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.print.stage, Some((_, Stage::Preparing))) {
            return;
        }
        if self.print.stage.map(|(kind, _)| kind) == Some(Kind::Export) {
            self.print.last_export = Some(Err("cancelled".to_string()));
        }
        // The task holds the passes and the PDF that was begun.
        self.print.task = None;
        self.print_ended(cx);
    }

    /// The exact job "Print…" would hand over, made at once: the pages of
    /// the open document as it is in the editor now. `None` without a
    /// document. For tests and for the scripted step `print:pdf:`; it
    /// holds the main thread for as long as the pages take.
    pub fn print_job(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<PrintJob> {
        self.session.as_ref()?;
        let pass = self.editor.read(cx).print_pass(window, cx);
        Some(self.pages_now(vec![pass], self.display_title(), cx))
    }

    /// The job of `passes` made at once, its pictures read at once: what a
    /// job of pages makes in slices. It holds the main thread for as long
    /// as the pages take.
    pub(crate) fn pages_now(
        &self,
        mut passes: Vec<PrintPass>,
        title: String,
        cx: &gpui::App,
    ) -> PrintJob {
        let pictures: Rc<std::collections::HashMap<_, _>> = Rc::new(
            passes
                .iter()
                .flat_map(PrintPass::images)
                .filter_map(|image| Some((image, self.images.bytes_now(image, cx)?)))
                .collect(),
        );
        for pass in &mut passes {
            let pictures = pictures.clone();
            pass.set_images(Rc::new(move |image| pictures.get(&image).cloned()));
        }
        let paper = passes.first().map_or((0., 0.), PrintPass::paper);
        let mut writer = PdfWriter::new(&printing::job_info(&title));
        // Page by page into the PDF, so that a book is not held as pages
        // and as a PDF at once.
        let mut pages = Vec::new();
        let mut numbered = 0;
        for mut pass in passes {
            pass.number_pages_from(numbered + 1);
            loop {
                let done = pass.advance(Duration::from_millis(50), &mut pages);
                for page in pages.drain(..) {
                    writer.add_page(&page);
                }
                if done {
                    break;
                }
            }
            numbered += pass.progress().pages_done;
        }
        PrintJob {
            title,
            paper,
            pdf: writer.finish(),
        }
    }

    // ----- the stages -----------------------------------------------------------------

    /// The note "Preparing to print …" (or "Preparing the PDF of …") with
    /// its Cancel, if the job is still preparing.
    fn show_print_note(&mut self, title: &str, cx: &mut Context<Self>) {
        let Some((kind, Stage::Preparing)) = self.print.stage else {
            return;
        };
        if self.print.note.is_some() {
            return;
        }
        let note = match kind {
            Kind::Print => Toast::new(preparing_note(title)).action("Cancel", CancelPrint),
            Kind::Export => Toast::new(pdf_note(title)).action("Cancel", CancelPdfExport),
        }
        .duration(NOTE_STAYS);
        let id = self
            .toast_layer
            .update(cx, |layer, cx| layer.push(note, cx));
        self.print.note = Some(id);
    }

    /// Takes the note away and stops its timer.
    fn take_print_note(&mut self, cx: &mut Context<Self>) {
        self.print.note_task = None;
        if let Some(note) = self.print.note.take() {
            self.toast_layer
                .update(cx, |layer, cx| layer.dismiss(note, cx));
        }
    }

    /// The pages are done and the job is in the print panel.
    fn print_is_in_the_panel(&mut self, cx: &mut Context<Self>) {
        self.take_print_note(cx);
        self.print.stage = Some((Kind::Print, Stage::Panel));
        self.print.progress = None;
        cx.notify();
    }

    /// The pages of a PDF export are done and the PDF is being written.
    pub(crate) fn pdf_is_being_written(&mut self, cx: &mut Context<Self>) {
        self.take_print_note(cx);
        self.print.stage = Some((Kind::Export, Stage::Writing));
        self.print.progress = None;
        cx.notify();
    }

    /// The job is over: handed over, written, cancelled, or its panel has
    /// ended. Called from the job's own task too, which this must not drop
    /// from under itself: the task has nothing left to do and is let go by
    /// the next job.
    pub(crate) fn print_ended(&mut self, cx: &mut Context<Self>) {
        self.take_print_note(cx);
        self.print.stage = None;
        self.print.progress = None;
        self.print.waiting = 0;
        cx.notify();
    }

    /// "Dismiss notifications" took the note away: the job it stood for
    /// stops with it.
    pub(crate) fn notifications_dismissed(&mut self, cx: &mut Context<Self>) {
        // The layer is empty already.
        self.print.note = None;
        self.cancel_pages(cx);
    }

    /// The notes changed. The note of the job may have gone by a way the
    /// workspace is not told of, and it must not go while the pages are
    /// still being made: the work would go on unseen and the print panel
    /// come up unannounced (the module docs, 4).
    ///
    /// - Pushed out by newer notes (the layer shows
    ///   [`writer_ui::toast::MAX_VISIBLE`] and drops the oldest): nobody
    ///   asked for it to go, so it is shown again, as the newest.
    /// - Taken away by the writer, with a click on it (every note goes so)
    ///   or its own button: the job stops, as with "Dismiss notifications".
    ///
    /// The two are told apart by what is left: a note that was pushed out
    /// leaves the layer full, one that was clicked away leaves room.
    pub(crate) fn print_note_went(&mut self, layer: Entity<ToastLayer>, cx: &mut Context<Self>) {
        let Some(note) = self.print.note else {
            return;
        };
        let ids = layer.read(cx).ids();
        if ids.contains(&note) {
            return;
        }
        self.print.note = None;
        if !matches!(self.print.stage, Some((_, Stage::Preparing))) {
            return;
        }
        if ids.len() >= MAX_VISIBLE {
            let title = self.print.title.clone();
            self.show_print_note(&title, cx);
        } else {
            self.cancel_pages(cx);
        }
    }
}

/// The progress of a job before it began.
const NOTHING_DONE: PrintProgress = PrintProgress {
    blocks_done: 0,
    blocks: 0,
    pages_done: 0,
};

/// How far `passes` are, after the passes before them have come as far as
/// `before` (which counts all of their blocks as done and their pages).
fn progress_of(passes: &[PrintPass], before: PrintProgress) -> PrintProgress {
    passes.iter().fold(before, |sum, pass| {
        let progress = pass.progress();
        PrintProgress {
            blocks_done: sum.blocks_done + progress.blocks_done,
            blocks: sum.blocks + progress.blocks,
            pages_done: sum.pages_done + progress.pages_done,
        }
    })
}

/// Makes the pages of `passes` in slices, one pass after the other with
/// its pages numbered on from the pass before, and draws them into a PDF:
/// the job. `None` when the workspace went away meanwhile.
async fn prepare(
    passes: Vec<PrintPass>,
    title: String,
    slice: Duration,
    this: &gpui::WeakEntity<Workspace>,
    cx: &mut gpui::AsyncWindowContext,
) -> Option<PrintJob> {
    let paper = passes.first().map_or((0., 0.), PrintPass::paper);
    let mut passes: VecDeque<PrintPass> = passes.into();
    // What the passes that are done came to: all of their blocks, and
    // their pages.
    let mut before = NOTHING_DONE;
    let mut writer = PdfWriter::new(&printing::job_info(&title));
    let mut pages = Vec::new();
    // The pages that are laid out and wait to be drawn.
    let mut waiting: VecDeque<writer_print::Page> = VecDeque::new();
    let mut last_frame = Instant::now();
    // A slice is the whole turn: laying blocks out and drawing pages.
    // What share of it the layout gets follows from what the two have
    // cost so far.
    let (mut laying, mut drawing) = (Duration::ZERO, Duration::ZERO);
    loop {
        let started = Instant::now();
        // No more is laid out while pages wait: they are drawn first.
        if waiting.len() < PAGES_AHEAD
            && let Some(pass) = passes.front_mut()
        {
            let done = pass.advance(layout_share(slice, laying, drawing), &mut pages);
            waiting.extend(pages.drain(..));
            if done && let Some(finished) = passes.pop_front() {
                let progress = finished.progress();
                before.blocks_done += progress.blocks;
                before.blocks += progress.blocks;
                before.pages_done += progress.pages_done;
                // The next document goes on counting.
                if let Some(next) = passes.front_mut() {
                    next.number_pages_from(before.pages_done + 1);
                }
            }
        }
        let laid = started.elapsed();
        // At least one page a slice, and more while the slice lasts: a
        // page of photographs takes far longer to draw than to lay out.
        while let Some(page) = waiting.pop_front() {
            writer.add_page(&page);
            if started.elapsed() >= slice {
                break;
            }
        }
        laying += laid;
        drawing += started.elapsed() - laid;
        if passes.is_empty() && waiting.is_empty() {
            break;
        }
        let progress = progress_of(passes.make_contiguous(), before);
        let left = waiting.len();
        let frame = last_frame.elapsed() >= FRAME_EVERY;
        if frame {
            last_frame = Instant::now();
        }
        this.update(cx, |this, cx| {
            this.print.progress = Some(progress);
            this.print.waiting = left;
            if frame {
                cx.notify();
            }
        })
        .ok()?;
        // A turn of the main thread: keys, frames, an agent's change. In
        // a test this is the test's clock, so a job stays "preparing" for
        // as long as the test likes.
        cx.background_executor().timer(SLICE_PAUSE).await;
    }
    Some(PrintJob {
        title,
        paper,
        pdf: writer.finish(),
    })
}

/// How much of a slice the layout gets, when laying out has taken
/// `laying` so far and drawing the pages `drawing`: the share that leaves
/// room for drawing what it lays out. Half, before anything was drawn.
pub fn layout_share(slice: Duration, laying: Duration, drawing: Duration) -> Duration {
    let (laying, drawing) = (laying.as_secs_f64(), drawing.as_secs_f64());
    let share = if laying > 0. && drawing > 0. {
        (laying / (laying + drawing)).clamp(0.2, 0.9)
    } else {
        0.5
    };
    slice.mul_f64(share)
}

/// Writes `job` into `directory` under the name its title gives it, and
/// says what came of it.
fn write_job(job: &PrintJob, directory: &std::path::Path) -> Toast {
    let name = job.file_name();
    let path = directory.join(&name);
    let written =
        std::fs::create_dir_all(directory).and_then(|()| std::fs::write(&path, job.pdf.bytes()));
    match written {
        Ok(()) => Toast::new(format!("Wrote “{name}” to {}.", short_path(directory))).success(),
        Err(error) => Toast::new(format!("Could not write {}: {error}", short_path(&path)))
            .error()
            .duration(Duration::from_secs(8)),
    }
}
