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
//! 5. The finished job is handed over. With the macOS print panel the job
//!    is then **in the panel** ([`Printing::Panel`]) until the sheet has
//!    ended; the sheet is started from the task, outside any update, as
//!    AppKit calls back into GPUI while it comes up.
//!
//! While a job is preparing or in the panel a second "Print…" does
//! nothing.
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

use gpui::{Context, Stateful, Task, Window, prelude::*};
use writer_editor::{PrintPass, PrintProgress};
use writer_print::PdfWriter;
use writer_ui::{Toast, toast::ToastId};

use super::Workspace;
use crate::{
    actions::{CancelPrint, Print},
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

/// What the workspace keeps about printing.
#[derive(Default)]
pub(crate) struct PrintState {
    /// Where the job is; `None` when there is none.
    stage: Option<Printing>,
    /// How far the pages are.
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
}

/// The words of the note for a document called `title`.
pub fn preparing_note(title: &str) -> String {
    format!("Preparing to print “{title}”…")
}

impl Workspace {
    // ----- what is going on -----------------------------------------------------------

    /// Where the print job is, if there is one.
    pub fn printing(&self) -> Option<Printing> {
        self.print.stage
    }

    /// Whether "Print" is there: a document is open and the app was given
    /// a target for print jobs.
    pub fn can_print(&self, cx: &gpui::App) -> bool {
        self.session.is_some() && printing::target(cx).is_some()
    }

    /// The note about the job that is preparing, while it is up.
    pub fn print_note(&self) -> Option<ToastId> {
        self.print.note
    }

    /// One word about printing for the `state` step of a scripted run:
    /// `idle`, `preparing 40%` (the share of the blocks that are on their
    /// pages, less what of those pages is not drawn yet) or `panel`.
    pub fn describe_printing(&self) -> String {
        match self.print.stage {
            None => "idle".to_string(),
            Some(Printing::Panel) => "panel".to_string(),
            Some(Printing::Preparing) => {
                let percent = self.print.progress.map_or(0, |progress| {
                    let laid = (progress.blocks_done * 100)
                        .checked_div(progress.blocks)
                        .unwrap_or(100);
                    // Of the pages handed out so far, those that are in
                    // the PDF.
                    let made = progress.pages_done;
                    let drawn = made.saturating_sub(self.print.waiting);
                    (laid * drawn).checked_div(made).unwrap_or(laid)
                });
                format!("preparing {percent}%")
            }
        }
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
        if self.print.stage == Some(Printing::Preparing) {
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
        if self.session.is_none() || self.print.stage.is_some() {
            return;
        }
        let Some(target) = printing::target(cx) else {
            return;
        };
        let mut pass = self.editor.read(cx).print_pass(window, cx);
        // The pictures are read in the background, before the first page
        // is made.
        let pictures = self.images.bytes_of_all(pass.images(), cx);
        let title = self.display_title();
        let slice = printing::slice(cx);
        self.print.stage = Some(Printing::Preparing);
        self.print.progress = Some(pass.progress());
        self.print.waiting = 0;
        self.print.note = None;

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
            let pictures = pictures.await;
            pass.set_images(Rc::new(move |image| pictures.get(&image).cloned()));
            let Some(job) = prepare(pass, title, slice, &this, cx).await else {
                // The workspace is gone.
                return;
            };
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
        }));
        cx.notify();
    }

    /// "Cancel printing": stops preparing the pages. Nothing is handed to
    /// the target. Does nothing unless a job is preparing (a job in the
    /// print panel is cancelled there).
    pub fn cancel_print(&mut self, cx: &mut Context<Self>) {
        if self.print.stage != Some(Printing::Preparing) {
            return;
        }
        // The task holds the pass and the PDF that was begun.
        self.print.task = None;
        self.print_ended(cx);
    }

    /// The exact job "Print…" would hand over, made at once: the pages of
    /// the open document as it is in the editor now. `None` without a
    /// document. For tests and for the scripted step `print:pdf:`; it
    /// holds the main thread for as long as the pages take.
    pub fn print_job(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<PrintJob> {
        self.session.as_ref()?;
        let mut pass = self.editor.read(cx).print_pass(window, cx);
        // The pictures, read at once.
        let pictures: std::collections::HashMap<_, _> = pass
            .images()
            .into_iter()
            .filter_map(|image| Some((image, self.images.bytes_now(image, cx)?)))
            .collect();
        pass.set_images(Rc::new(move |image| pictures.get(&image).cloned()));
        let title = self.display_title();
        let paper = pass.paper();
        let mut writer = PdfWriter::new(&printing::job_info(&title));
        // Page by page into the PDF, so that a book is not held as pages
        // and as a PDF at once.
        let mut pages = Vec::new();
        loop {
            let done = pass.advance(Duration::from_millis(50), &mut pages);
            for page in pages.drain(..) {
                writer.add_page(&page);
            }
            if done {
                break;
            }
        }
        Some(PrintJob {
            title,
            paper,
            pdf: writer.finish(),
        })
    }

    // ----- the stages -----------------------------------------------------------------

    /// The note "Preparing to print …" with its Cancel, if the job is
    /// still preparing.
    fn show_print_note(&mut self, title: &str, cx: &mut Context<Self>) {
        if self.print.stage != Some(Printing::Preparing) || self.print.note.is_some() {
            return;
        }
        let note = Toast::new(preparing_note(title))
            .duration(NOTE_STAYS)
            .action("Cancel", CancelPrint);
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
        self.print.stage = Some(Printing::Panel);
        self.print.progress = None;
        cx.notify();
    }

    /// The job is over: handed over, cancelled, or its panel has ended.
    /// Called from the job's own task too, which this must not drop from
    /// under itself: the task has nothing left to do and is let go by the
    /// next job.
    fn print_ended(&mut self, cx: &mut Context<Self>) {
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
        self.cancel_print(cx);
    }
}

/// Makes the pages of `pass` in slices and draws them into a PDF: the job.
/// `None` when the workspace went away meanwhile.
async fn prepare(
    mut pass: PrintPass,
    title: String,
    slice: Duration,
    this: &gpui::WeakEntity<Workspace>,
    cx: &mut gpui::AsyncWindowContext,
) -> Option<PrintJob> {
    let paper = pass.paper();
    let mut writer = PdfWriter::new(&printing::job_info(&title));
    let mut pages = Vec::new();
    // The pages that are laid out and wait to be drawn.
    let mut waiting: VecDeque<writer_print::Page> = VecDeque::new();
    let mut laid_out = false;
    let mut last_frame = Instant::now();
    // A slice is the whole turn: laying blocks out and drawing pages.
    // What share of it the layout gets follows from what the two have
    // cost so far.
    let (mut laying, mut drawing) = (Duration::ZERO, Duration::ZERO);
    loop {
        let started = Instant::now();
        // No more is laid out while pages wait: they are drawn first.
        if !laid_out && waiting.len() < PAGES_AHEAD {
            laid_out = pass.advance(layout_share(slice, laying, drawing), &mut pages);
            waiting.extend(pages.drain(..));
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
        if laid_out && waiting.is_empty() {
            break;
        }
        let progress = pass.progress();
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
