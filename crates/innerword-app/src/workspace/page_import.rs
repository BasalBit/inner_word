//! Adding pages in the app: File > Import Page…, and `.html` files or
//! folders dropped on the sidebar, on an open page or on the text
//! (docs/pages-design.md, section 10, "Adding pages in the app").
//!
//! # A gesture, step by step
//!
//! One choice in the open panel, or one drop, is a **gesture**: its inputs
//! (each an `.html` or `.htm` file, or a folder) become one page each, one
//! after the other; what else was dropped is left out and counted. Gestures
//! that come while one is being worked on wait their turn.
//!
//! For each page:
//!
//! 1. **Fetching** ([`AddingPhase::Fetching`]): the input is read
//!    (`innerword_page::read_input`) and everything it names on the web is
//!    fetched once and stored beside it (`innerword_page::bundle`), on a thread
//!    of its own: a fetch can take up to three minutes. The fetcher is
//!    `curl` for public addresses, each address once per gesture
//!    (`pages::fetcher_for_a_gesture`), or what a test installed.
//! 2. **Checking**: the page is loaded offscreen with the network blocked,
//!    by the page engine (WebKit's check on the main thread, called from
//!    this task between two awaits; the fake's answer in a test). Never
//!    `static_check` in a real run.
//! 3. The verdict (`innerword_page::prepare`, in the background): the page is
//!    whole, or the sentence that says why not.
//! 4. **Storing**: its files go into `images.db` in the background, through
//!    the connection the pictures use (`LibraryImages::put_files`), and then
//!    the row on the main thread (`Library::create_page`, which refuses with
//!    `FilesMissing` when a file is not there after all: then the page is not
//!    added, "its files could not be stored"). The sidebar shows it; the
//!    first page of a gesture opens, unless another item was opened since the
//!    gesture began.
//!
//! # The note, Cancel, the summary
//!
//! As for printing: after [`NOTE_DELAY`] a note says what is going on
//! ("Adding 3 pages… (2 of 3: checking that “Orbitarium” is whole)") with
//! the button Cancel, which is the command "Cancel adding pages" (File, the
//! palette; a toast's button has no way in from the keyboard). The note
//! follows the work, is shown again as the newest when newer notes push it
//! out, and a click that takes it away (or "Dismiss notifications") cancels,
//! so the work never goes on unseen. Cancel stops between steps: what is
//! being fetched or checked is dropped, a page already stored stays, nothing
//! half is written (the files of a page that was not written are named by
//! nothing and go with the collector).
//!
//! When the gesture is done **one** toast sums it up ("2 pages added, 1 not
//! added", "Added “Orbitarium”"), with **Show Why** when a page was not
//! added, has warnings, or files were left out; Show Why is the command
//! "Show the report of the pages added last" ([`crate::page_report`]).
//! A single page that was added and is not open is offered with Open.
//!
//! # For tests and scripted runs
//!
//! [`Workspace::import_pages`] is the import of chosen inputs and answers
//! with the gesture's [`PagesReport`]; `page:import:<path>` waits for it.

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use gpui::{AsyncWindowContext, Context, Entity, Task, WeakEntity, Window, prelude::*};
use innerword_library::{DocumentId, FolderId, PageContent, StoreError};
use innerword_ui::{
    ModalPlacement, Toast, ToastLayer,
    toast::{MAX_VISIBLE, ToastId},
};

use super::Workspace;
use crate::{
    actions::{CancelAddingPages, ShowPageReport},
    live::Handoff,
    pages::{self, Answer, AnswerTx, PageEngine},
};

/// How long pages are being added before the note about it appears. A
/// page without pieces from the web is in long before.
pub const NOTE_DELAY: Duration = Duration::from_millis(400);

/// How long the note stays by itself: it is taken away when the work ends,
/// so this is only longer than any gesture.
const NOTE_STAYS: Duration = Duration::from_secs(60 * 60);

/// How long the summary of a gesture stays.
pub const SUMMARY_STAYS: Duration = Duration::from_secs(10);

/// Where a page that is being added is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddingPhase {
    /// Read, and what it names on the web fetched.
    Fetching,
    /// Loaded offscreen with the network blocked.
    Checking,
    /// Its files and its row written into the library.
    Storing,
}

impl AddingPhase {
    /// The word of the `state` line.
    pub fn word(self) -> &'static str {
        match self {
            AddingPhase::Fetching => "fetching",
            AddingPhase::Checking => "checking",
            AddingPhase::Storing => "storing",
        }
    }
}

/// What is being added now: the `index`-th page (from 1) of `total`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddingProgress {
    pub index: usize,
    pub total: usize,
    /// The page's name before its title is known.
    pub name: String,
    pub phase: AddingPhase,
}

/// What came of one page of a gesture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageOutcome {
    /// Stored, as `id`.
    Added {
        id: DocumentId,
        /// The files it serves.
        files: usize,
        /// Its distinct bytes, served and original together.
        bytes: u64,
        /// The pieces fetched from the web.
        externals: usize,
        /// What the bundler and the check had to say (an address that is not
        /// kept, data a page kept on claude.ai).
        warnings: Vec<String>,
    },
    /// Not stored; the whole sentence says why.
    NotAdded { sentence: String },
    /// Not begun: adding pages was cancelled first.
    Cancelled,
}

/// One page of a gesture: the input, its name (the title once it was
/// added) and what came of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddedPage {
    pub path: PathBuf,
    pub name: String,
    pub outcome: PageOutcome,
}

/// What one gesture came to: a line for each page, the files that were no
/// pages, and whether it was cancelled.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PagesReport {
    pub pages: Vec<AddedPage>,
    pub left_out: Vec<PathBuf>,
    pub cancelled: bool,
}

/// "1 page" / "3 pages".
fn pages_counted(count: usize) -> String {
    if count == 1 {
        "1 page".to_string()
    } else {
        format!("{count} pages")
    }
}

impl PagesReport {
    /// The ids of the pages that were added, in order.
    pub fn added_ids(&self) -> Vec<DocumentId> {
        self.pages
            .iter()
            .filter_map(|page| match page.outcome {
                PageOutcome::Added { id, .. } => Some(id),
                _ => None,
            })
            .collect()
    }

    /// How many pages were added.
    pub fn added(&self) -> usize {
        self.added_ids().len()
    }

    /// How many were not (refused, or cancelled before they were begun).
    pub fn not_added(&self) -> usize {
        self.pages.len() - self.added()
    }

    /// How many warnings the pages that were added have.
    pub fn warnings(&self) -> usize {
        self.pages
            .iter()
            .map(|page| match &page.outcome {
                PageOutcome::Added { warnings, .. } => warnings.len(),
                _ => 0,
            })
            .sum()
    }

    /// Whether the summary offers "Show Why": a page was refused, has
    /// warnings, or files were left out (a page that was not begun because
    /// the writer cancelled has nothing to explain).
    pub fn needs_why(&self) -> bool {
        let refused = self
            .pages
            .iter()
            .any(|page| matches!(page.outcome, PageOutcome::NotAdded { .. }));
        refused || self.warnings() > 0 || !self.left_out.is_empty()
    }

    /// "3 files were not pages and were left out".
    pub fn left_out_sentence(count: usize) -> String {
        if count == 1 {
            "1 file was not a page and was left out".to_string()
        } else {
            format!("{count} files were not pages and were left out")
        }
    }

    /// What the summary toast says.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        let (added, not_added) = (self.added(), self.not_added());
        if self.cancelled {
            parts.push(match (added, not_added) {
                (0, _) => "Stopped adding pages: none was added".to_string(),
                (added, 0) => format!("Stopped adding pages: {} added", pages_counted(added)),
                (added, not) => {
                    format!(
                        "Stopped adding pages: {} added, {not} not added",
                        pages_counted(added)
                    )
                }
            });
        } else if let [page] = self.pages.as_slice() {
            parts.push(match &page.outcome {
                PageOutcome::Added { warnings, .. } => match warnings.len() {
                    0 => format!("Added \u{201c}{}\u{201d}", page.name),
                    1 => format!("Added \u{201c}{}\u{201d}, with 1 warning", page.name),
                    n => format!("Added \u{201c}{}\u{201d}, with {n} warnings", page.name),
                },
                _ => format!("\u{201c}{}\u{201d} was not added", page.name),
            });
        } else if !self.pages.is_empty() {
            parts.push(match (added, not_added) {
                (0, not) => format!("None of the {not} pages was added"),
                (added, 0) => format!("{} added", pages_counted(added)),
                (added, not) => format!("{} added, {not} not added", pages_counted(added)),
            });
        }
        if !self.left_out.is_empty() {
            parts.push(Self::left_out_sentence(self.left_out.len()));
        }
        parts.join(". ")
    }

    /// The whole report as plain text: what "Copy" copies.
    pub fn text(&self) -> String {
        let mut text = format!("{}\n", self.summary());
        for page in &self.pages {
            text.push('\n');
            match &page.outcome {
                PageOutcome::Added {
                    files,
                    bytes,
                    externals,
                    warnings,
                    ..
                } => {
                    text.push_str(&format!(
                        "\u{201c}{}\u{201d}: added ({files} files, {}, {externals} pieces from \
                         the web)\n",
                        page.name,
                        innerword_page::megabytes(*bytes)
                    ));
                    for warning in warnings {
                        text.push_str(&format!("  {warning}\n"));
                    }
                }
                PageOutcome::NotAdded { sentence } => {
                    text.push_str(&format!(
                        "\u{201c}{}\u{201d}: not added\n  {sentence}\n",
                        page.name
                    ));
                }
                PageOutcome::Cancelled => {
                    text.push_str(&format!(
                        "\u{201c}{}\u{201d}: not added, adding pages was cancelled first\n",
                        page.name
                    ));
                }
            }
        }
        if !self.left_out.is_empty() {
            text.push_str(&format!(
                "\n{}:\n",
                Self::left_out_sentence(self.left_out.len())
            ));
            for path in &self.left_out {
                text.push_str(&format!("  {}\n", path.display()));
            }
        }
        text
    }
}

/// One choice in the open panel, or one drop.
struct Gesture {
    inputs: Vec<PathBuf>,
    left_out: Vec<PathBuf>,
    folder: Option<FolderId>,
    /// Who waits for its report.
    done: Vec<AnswerTx<PagesReport>>,
}

/// The gesture that is being worked on.
struct Current {
    /// Its inputs, and what came of those that are done.
    inputs: Vec<PathBuf>,
    report: PagesReport,
    folder: Option<FolderId>,
    done: Vec<AnswerTx<PagesReport>>,
    /// The item that was open when it began: its first page opens only when
    /// nothing else was opened since.
    open_at_start: Option<DocumentId>,
}

/// What the workspace keeps about adding pages.
#[derive(Default)]
pub(crate) struct AddingState {
    /// Gestures that wait their turn.
    queue: VecDeque<Gesture>,
    current: Option<Current>,
    progress: Option<AddingProgress>,
    /// Whether pages are being added (the task may outlive this: it is let
    /// go by the next gesture, never dropped from under itself).
    running: bool,
    /// The task that adds them. Dropping it stops the work between steps.
    task: Option<Task<()>>,
    /// The timer of the note.
    note_task: Option<Task<()>>,
    /// The note "Adding … pages", while it is up, and what it says.
    note: Option<ToastId>,
    note_message: String,
    /// The report of the last gesture that ended: what Show Why shows.
    report: Option<PagesReport>,
}

/// The words the app gives an input that cannot be used: the bundler's,
/// without the options of the command line (the app has no `--entry`).
fn in_the_apps_words(name: &str, message: &str) -> String {
    let message = message
        .replace(
            ": say which one opens with --entry",
            ": name the one it opens with index.html",
        )
        .replace(" and say which page opens with --entry.", ".");
    let message = message.trim_end_matches('.');
    format!("\u{201c}{name}\u{201d} was not added: {message}. Nothing was saved.")
}

/// The sentence for a page that could not be made.
/// The name a refused page goes by in the summary: the title the bundler or
/// the check found, as the sentence of the refusal has it, else the input's.
fn refused_name(name: &str, error: &innerword_page::PageError) -> String {
    match error {
        innerword_page::PageError::Incomplete { name, .. } => name.clone(),
        _ => name.to_string(),
    }
}

fn refusal(name: &str, error: &innerword_page::PageError) -> String {
    match error {
        // Names the page, gives its reasons and ends "Nothing was saved."
        innerword_page::PageError::Incomplete { .. } => error.sentence(),
        innerword_page::PageError::Input { message } => in_the_apps_words(name, message),
        other => in_the_apps_words(name, &other.sentence()),
    }
}

impl Workspace {
    // ----- what is going on -----------------------------------------------------------

    /// Whether pages are being added.
    pub fn is_adding_pages(&self) -> bool {
        self.adding.running
    }

    /// The provider of pictures over the library, which also stores the
    /// files of pages that are being added.
    pub fn library_images(&self) -> &std::rc::Rc<crate::images::LibraryImages> {
        &self.images
    }

    /// The page that is being added now, and where it is.
    pub fn adding_progress(&self) -> Option<&AddingProgress> {
        self.adding.progress.as_ref()
    }

    /// The note about the pages that are being added, while it is up.
    pub fn adding_note(&self) -> Option<ToastId> {
        self.adding.note
    }

    /// The report of the last gesture of adding pages that ended.
    pub fn last_pages_report(&self) -> Option<&PagesReport> {
        self.adding.report.as_ref()
    }

    /// One part of the `state` line of a scripted run: `idle`, or
    /// `2/3 "Orbitarium" fetching`.
    pub fn describe_adding(&self) -> String {
        match &self.adding.progress {
            Some(progress) if self.adding.running => format!(
                "{}/{} {:?} {}",
                progress.index,
                progress.total,
                progress.name,
                progress.phase.word()
            ),
            _ if self.adding.running => "starting".to_string(),
            _ => "idle".to_string(),
        }
    }

    // ----- the commands ---------------------------------------------------------------

    /// The handlers of the commands of adding pages, on the workspace root:
    /// "Import page…" with a page engine, "Cancel adding pages" while pages
    /// are being added, and "Show the report of the pages added last". The
    /// menu bar greys out an item without a handler.
    pub(crate) fn adding_actions(
        &self,
        mut root: gpui::Stateful<gpui::Div>,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        if pages::engine(cx).is_some() {
            root = root.on_action(cx.listener(
                |this, _: &crate::actions::ImportPage, window, cx| {
                    let folder = this.panel.read(cx).selected_folder();
                    this.choose_pages_to_import(folder, window, cx);
                },
            ));
        }
        if self.adding.running {
            root = root.on_action(cx.listener(|this, _: &CancelAddingPages, _, cx| {
                this.cancel_adding_pages(cx);
            }));
        }
        root.on_action(cx.listener(|this, _: &ShowPageReport, window, cx| {
            this.show_pages_report(window, cx);
        }))
    }

    /// "Import Page…": asks for files and folders (the open panel, or what a
    /// test named) and adds each as a page in `folder`.
    pub fn choose_pages_to_import(
        &mut self,
        folder: Option<FolderId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if pages::engine(cx).is_none() {
            self.toast_error(pages::cannot_add_sentence(cx), cx);
            return;
        }
        let chosen = pages::choose_inputs(cx);
        cx.spawn_in(window, async move |this, cx| {
            let Some(paths) = chosen.await else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                this.import_pages(paths, folder, window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Files dropped somewhere that takes pages (the sidebar, an open page,
    /// the text): the web pages among them are added in `folder`, the rest
    /// is left out and counted. Without a page engine the toast says why.
    pub fn import_dropped(
        &mut self,
        paths: Vec<PathBuf>,
        folder: Option<FolderId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Answer<PagesReport>> {
        if paths.is_empty() {
            return None;
        }
        self.import_pages(paths, folder, window, cx)
    }

    /// Adds the web pages among `paths` (`.html` and `.htm` files, folders)
    /// in `folder`, one after the other, as Import Page… does once they are
    /// chosen; the rest is left out and counted. Answers with the gesture's
    /// report when it is done (also when it was cancelled). `None` without a
    /// page engine (the toast says why) or when nothing was given.
    pub fn import_pages(
        &mut self,
        paths: Vec<PathBuf>,
        folder: Option<FolderId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Answer<PagesReport>> {
        if paths.is_empty() {
            return None;
        }
        let (inputs, left_out): (Vec<PathBuf>, Vec<PathBuf>) = paths
            .into_iter()
            .partition(|path| pages::is_page_input(path));
        if pages::engine(cx).is_none() {
            if !inputs.is_empty() {
                self.toast_error(pages::cannot_add_sentence(cx), cx);
            }
            return None;
        }
        let (tx, rx) = pages::answer();
        if inputs.is_empty() {
            // Nothing to add: the summary says what was left out.
            let report = PagesReport {
                left_out,
                ..PagesReport::default()
            };
            self.show_pages_summary(&report, cx);
            self.adding.report = Some(report.clone());
            tx.send(report);
            return Some(rx);
        }
        self.adding.queue.push_back(Gesture {
            inputs,
            left_out,
            folder,
            done: vec![tx],
        });
        if !self.adding.running {
            self.start_adding(window, cx);
        }
        cx.notify();
        Some(rx)
    }

    /// "Cancel adding pages": stops adding the pages of every gesture that
    /// is being worked on or waits. A page already stored stays. Does
    /// nothing while no pages are being added.
    pub fn cancel_adding_pages(&mut self, cx: &mut Context<Self>) {
        if !self.adding.running {
            return;
        }
        // The task holds what is being fetched or checked.
        self.adding.task = None;
        if let Some(current) = self.adding.current.take() {
            self.end_gesture(current, true, cx);
        }
        for gesture in std::mem::take(&mut self.adding.queue) {
            let report = PagesReport {
                pages: gesture
                    .inputs
                    .iter()
                    .map(|path| AddedPage {
                        path: path.clone(),
                        name: pages::input_name(path),
                        outcome: PageOutcome::Cancelled,
                    })
                    .collect(),
                left_out: gesture.left_out,
                cancelled: true,
            };
            for tx in gesture.done {
                tx.send(report.clone());
            }
        }
        self.adding_ended(cx);
    }

    /// "Show the report of the pages added last": the dialog with every page
    /// of the last gesture and its sentence whole.
    pub fn show_pages_report(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(report) = self.adding.report.clone() else {
            self.show_toast(
                Toast::new("No pages were added since Inner Word was opened"),
                cx,
            );
            return;
        };
        self.modal_layer.update(cx, |layer, cx| {
            let view = cx.new(|cx| crate::page_report::PageReport::new(report, cx));
            layer.show_with_placement(view, ModalPlacement::Center, window, cx);
        });
    }

    // ----- the work -------------------------------------------------------------------

    fn start_adding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.adding.running = true;
        self.adding.progress = None;
        self.adding.note = None;
        self.adding.note_message.clear();
        self.adding.note_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(NOTE_DELAY).await;
            this.update(cx, |this, cx| this.show_adding_note(cx)).ok();
        }));
        self.adding.task = Some(cx.spawn_in(window, async move |this, cx| {
            add_all(this, cx).await;
        }));
    }

    /// Takes the next gesture up; `None` when none waits.
    fn begin_gesture(&mut self) -> Option<GestureJob> {
        let gesture = self.adding.queue.pop_front()?;
        self.adding.current = Some(Current {
            inputs: gesture.inputs.clone(),
            report: PagesReport {
                pages: Vec::new(),
                left_out: gesture.left_out,
                cancelled: false,
            },
            folder: gesture.folder,
            done: gesture.done,
            open_at_start: self.open_item_id(),
        });
        Some(GestureJob {
            inputs: gesture.inputs,
        })
    }

    /// Where the page that is being added is now.
    fn set_adding_progress(&mut self, progress: AddingProgress, cx: &mut Context<Self>) {
        self.adding.progress = Some(progress);
        self.refresh_adding_note(cx);
        cx.notify();
    }

    /// What the next page of the gesture needs from the window: the engine,
    /// the fetcher, where WebKit keeps its rules.
    fn adding_needs(
        &self,
        cx: &gpui::App,
    ) -> Option<(PageEngine, Arc<dyn innerword_page::Fetcher>, PathBuf)> {
        let engine = pages::engine(cx)?;
        let rules_dir = pages::rules_dir_of(self.library.read(cx).path())
            .unwrap_or_else(|| std::env::temp_dir().join("inner-word-WebKit"));
        Some((engine, pages::fetcher_for_a_gesture(cx), rules_dir))
    }

    /// Writes the row of a page whose files are stored: in the folder of the
    /// gesture (at the top when that folder went meanwhile).
    fn store_page_row(
        &mut self,
        prepared: &innerword_page::Prepared,
        manifest: &innerword_page::Manifest,
        cx: &mut Context<Self>,
    ) -> PageOutcome {
        let folder = self
            .adding
            .current
            .as_ref()
            .and_then(|current| current.folder);
        let title = prepared.title();
        let json = manifest.to_json();
        let stats = innerword_core::Fragment::from_plain_text(&prepared.text)
            .to_document()
            .stats();
        let content = PageContent {
            manifest: &json,
            title: &title,
            plain_text: &prepared.text,
            word_count: stats.words as u64,
            char_count: stats.characters as u64,
            source_url: None,
        };
        let created = self.library.update(cx, |library, cx| {
            match library.create_page(folder, &content, cx) {
                Err(StoreError::NotFound { .. }) if folder.is_some() => {
                    library.create_page(None, &content, cx)
                }
                created => created,
            }
        });
        match created {
            Ok(meta) => PageOutcome::Added {
                id: meta.id,
                files: manifest.files.len(),
                bytes: prepared.distinct_bytes(),
                externals: prepared.externals.len(),
                warnings: prepared.warnings.clone(),
            },
            Err(StoreError::FilesMissing { ids }) => PageOutcome::NotAdded {
                sentence: format!(
                    "\u{201c}{title}\u{201d} was not added: its files could not be stored in the \
                     library ({} missing). Nothing was saved.",
                    ids.len()
                ),
            },
            Err(error) => PageOutcome::NotAdded {
                sentence: format!(
                    "\u{201c}{title}\u{201d} was not added: it could not be stored in the \
                     library ({error}). Nothing was saved."
                ),
            },
        }
    }

    /// One page of the gesture is done: noted, and the first page that was
    /// added opens.
    fn page_done(
        &mut self,
        path: &Path,
        name: String,
        outcome: PageOutcome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(current) = self.adding.current.as_mut() else {
            return;
        };
        let first = current.report.added() == 0;
        let open_at_start = current.open_at_start;
        current.report.pages.push(AddedPage {
            path: path.to_path_buf(),
            name,
            outcome: outcome.clone(),
        });
        if let PageOutcome::Added { id, .. } = outcome
            && first
            && self.open_item_id() == open_at_start
        {
            let focus = !self.modal_layer.read(cx).has_modal();
            self.open_document(id, focus, window, cx);
        }
        cx.notify();
    }

    /// The gesture is done (or cancelled): its summary, its report, who
    /// waits for it.
    fn end_gesture(&mut self, mut current: Current, cancelled: bool, cx: &mut Context<Self>) {
        if cancelled {
            current.report.cancelled = true;
            let done = current.report.pages.len();
            for path in current.inputs.iter().skip(done) {
                current.report.pages.push(AddedPage {
                    path: path.clone(),
                    name: pages::input_name(path),
                    outcome: PageOutcome::Cancelled,
                });
            }
        }
        self.take_adding_note(cx);
        self.show_pages_summary(&current.report, cx);
        self.adding.report = Some(current.report.clone());
        for tx in current.done {
            tx.send(current.report.clone());
        }
        self.adding.progress = None;
        cx.notify();
    }

    /// The task found nothing more to do. Called from the task itself, which
    /// this must not drop from under itself: it has nothing left to do and
    /// is let go by the next gesture.
    fn adding_ended(&mut self, cx: &mut Context<Self>) {
        self.take_adding_note(cx);
        self.adding.running = false;
        self.adding.progress = None;
        self.adding.current = None;
        cx.notify();
    }

    // ----- the note -------------------------------------------------------------------

    /// What the note says now.
    fn adding_note_message(&self) -> String {
        let waiting: usize = self
            .adding
            .queue
            .iter()
            .map(|gesture| gesture.inputs.len())
            .sum();
        match &self.adding.progress {
            None => "Adding pages\u{2026}".to_string(),
            Some(progress) if progress.total == 1 && waiting == 0 => {
                let doing = match progress.phase {
                    AddingPhase::Fetching => "fetching what it loads from the web",
                    AddingPhase::Checking => "checking that it is whole",
                    AddingPhase::Storing => "storing it",
                };
                format!("Adding \u{201c}{}\u{201d}\u{2026} ({doing})", progress.name)
            }
            Some(progress) => {
                let name = &progress.name;
                let doing = match progress.phase {
                    AddingPhase::Fetching => {
                        format!("fetching what \u{201c}{name}\u{201d} loads from the web")
                    }
                    AddingPhase::Checking => {
                        format!("checking that \u{201c}{name}\u{201d} is whole")
                    }
                    AddingPhase::Storing => format!("storing \u{201c}{name}\u{201d}"),
                };
                let total = progress.total + waiting;
                format!(
                    "Adding {total} pages\u{2026} ({} of {total}: {doing})",
                    progress.index
                )
            }
        }
    }

    /// The note with its Cancel, once it is due, while pages are being added.
    fn show_adding_note(&mut self, cx: &mut Context<Self>) {
        if !self.adding.running || self.adding.note.is_some() {
            return;
        }
        let message = self.adding_note_message();
        let note = Toast::new(message.clone())
            .duration(NOTE_STAYS)
            .action("Cancel", CancelAddingPages);
        let id = self
            .toast_layer
            .update(cx, |layer, cx| layer.push(note, cx));
        self.adding.note = Some(id);
        self.adding.note_message = message;
    }

    /// The note follows the work: it is shown again with what is being done
    /// now (in place of the one that is up).
    fn refresh_adding_note(&mut self, cx: &mut Context<Self>) {
        let Some(up) = self.adding.note else {
            return;
        };
        let message = self.adding_note_message();
        if message == self.adding.note_message {
            return;
        }
        let note = Toast::new(message.clone())
            .duration(NOTE_STAYS)
            .action("Cancel", CancelAddingPages);
        let id = self.toast_layer.update(cx, |layer, cx| {
            layer.dismiss(up, cx);
            layer.push(note, cx)
        });
        self.adding.note = Some(id);
        self.adding.note_message = message;
    }

    /// Takes the note away and stops its timer.
    fn take_adding_note(&mut self, cx: &mut Context<Self>) {
        self.adding.note_task = None;
        if let Some(note) = self.adding.note.take() {
            self.toast_layer
                .update(cx, |layer, cx| layer.dismiss(note, cx));
        }
        self.adding.note_message.clear();
    }

    /// The one toast that sums a gesture up.
    fn show_pages_summary(&mut self, report: &PagesReport, cx: &mut Context<Self>) {
        let message = report.summary();
        if message.is_empty() {
            return;
        }
        let mut toast = Toast::new(message).duration(SUMMARY_STAYS);
        toast = match (report.added(), report.not_added()) {
            (_, 0) if !report.needs_why() => toast.success(),
            (0, _) => toast.error(),
            _ => toast,
        };
        if report.needs_why() {
            toast = toast.action("Show Why", ShowPageReport);
        } else if let [id] = report.added_ids().as_slice()
            && self.open_item_id() != Some(*id)
        {
            let id = *id;
            let workspace = cx.weak_entity();
            toast = toast.on_action("Open", move |window, cx| {
                workspace
                    .update(cx, |this, cx| {
                        this.open_document(id, true, window, cx);
                    })
                    .ok();
            });
        }
        self.show_toast(toast, cx);
    }

    /// "Dismiss notifications" took the note away: the work it stood for
    /// stops with it (as for a print).
    pub(crate) fn adding_notifications_dismissed(&mut self, cx: &mut Context<Self>) {
        // The layer is empty already.
        self.adding.note = None;
        self.cancel_adding_pages(cx);
    }

    /// The notes changed. The note may have gone by a way the workspace is
    /// not told of, and it must not go while pages are being added: pushed
    /// out by newer notes it is shown again as the newest; taken away by the
    /// writer (a click on it, which takes any note away) the work stops. The
    /// two are told apart by what is left, as for the note of a print.
    pub(crate) fn adding_note_went(&mut self, layer: Entity<ToastLayer>, cx: &mut Context<Self>) {
        let Some(note) = self.adding.note else {
            return;
        };
        let ids = layer.read(cx).ids();
        if ids.contains(&note) {
            return;
        }
        self.adding.note = None;
        self.adding.note_message.clear();
        if !self.adding.running {
            return;
        }
        if ids.len() >= MAX_VISIBLE {
            self.show_adding_note(cx);
        } else {
            self.cancel_adding_pages(cx);
        }
    }

    /// The other places that take files: an open page (the web view takes
    /// no drops, so they reach GPUI's view and the page element) and the
    /// text (through the provider of pictures, which hands web pages on).
    pub(crate) fn connect_page_drops(&mut self, window: &Window) {
        let Some(handle) = window.window_handle().downcast::<Workspace>() else {
            return;
        };
        self.images.set_page_import(move |paths, cx| {
            handle
                .update(cx, |this, window, cx| {
                    let folder = this.panel.read(cx).selected_folder();
                    this.import_dropped(paths, folder, window, cx);
                })
                .ok();
        });
    }

    /// Files dropped on an open page.
    pub fn files_dropped_on_page(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Answer<PagesReport>> {
        let folder = self.panel.read(cx).selected_folder();
        self.import_dropped(paths, folder, window, cx)
    }
}

/// What the task takes of a gesture.
struct GestureJob {
    inputs: Vec<PathBuf>,
}

/// The task: every gesture, one page after the other, until none waits.
async fn add_all(this: WeakEntity<Workspace>, cx: &mut AsyncWindowContext) {
    loop {
        let Ok(Some(job)) = this.update(cx, |this, _| this.begin_gesture()) else {
            break;
        };
        let total = job.inputs.len();
        for (ix, path) in job.inputs.iter().enumerate() {
            let name = pages::input_name(path);
            let Some((name, outcome)) = add_one(&this, path, name, ix + 1, total, cx).await else {
                // The workspace is gone.
                return;
            };
            if this
                .update_in(cx, |this, window, cx| {
                    this.page_done(path, name, outcome, window, cx)
                })
                .is_err()
            {
                return;
            }
        }
        let ended = this.update(cx, |this, cx| {
            if let Some(current) = this.adding.current.take() {
                this.end_gesture(current, false, cx);
            }
        });
        if ended.is_err() {
            return;
        }
    }
    this.update(cx, |this, cx| this.adding_ended(cx)).ok();
}

/// Adds the page at `path`: the name it goes by in the report (its title
/// once it was made) and what came of it. `None` when the workspace went
/// away.
async fn add_one(
    this: &WeakEntity<Workspace>,
    path: &Path,
    name: String,
    index: usize,
    total: usize,
    cx: &mut AsyncWindowContext,
) -> Option<(String, PageOutcome)> {
    let progress = |phase| AddingProgress {
        index,
        total,
        name: name.clone(),
        phase,
    };
    let needs = this
        .update(cx, |this, cx| {
            this.set_adding_progress(progress(AddingPhase::Fetching), cx);
            this.adding_needs(cx)
                .ok_or_else(|| pages::cannot_add_sentence(cx))
        })
        .ok()?;
    let (engine, fetcher, rules_dir) = match needs {
        Ok(needs) => needs,
        Err(why) => {
            return Some((
                name.clone(),
                PageOutcome::NotAdded {
                    sentence: in_the_apps_words(&name, &why),
                },
            ));
        }
    };

    // 1. Read and fetch, on a thread of its own: a fetch can take minutes.
    let handoff: Handoff<Result<innerword_page::Bundle, innerword_page::PageError>> = Handoff::new();
    {
        let handoff = handoff.clone();
        let path = path.to_path_buf();
        let spawned = std::thread::Builder::new()
            .name("inner-word-page".to_string())
            .spawn(move || {
                let bundled = innerword_page::read_input(&path, None)
                    .and_then(|input| innerword_page::bundle(input, fetcher.as_ref()));
                handoff.push(bundled).ok();
            });
        if let Err(error) = spawned {
            return Some((
                name.clone(),
                PageOutcome::NotAdded {
                    sentence: in_the_apps_words(&name, &format!("it could not be read ({error})")),
                },
            ));
        }
    }
    let bundle = match handoff.next().await {
        Some(Ok(bundle)) => bundle,
        Some(Err(error)) => {
            return Some((
                refused_name(&name, &error),
                PageOutcome::NotAdded {
                    sentence: refusal(&name, &error),
                },
            ));
        }
        None => return None,
    };

    // 2. The check, outside any update.
    this.update(cx, |this, cx| {
        this.set_adding_progress(progress(AddingPhase::Checking), cx)
    })
    .ok()?;
    let report = pages::check_bundle(&engine, &bundle, rules_dir).await;
    let Some(report) = report else {
        return Some((
            name.clone(),
            PageOutcome::NotAdded {
                sentence: in_the_apps_words(&name, "the page could not be checked"),
            },
        ));
    };

    // 3. The verdict, in the background.
    let prepared = cx
        .background_spawn(async move { innerword_page::prepare(bundle, &report) })
        .await;
    let mut prepared = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            return Some((
                refused_name(&name, &error),
                PageOutcome::NotAdded {
                    sentence: refusal(&name, &error),
                },
            ));
        }
    };
    let title = prepared.title();

    // 4. Its files in the background, then its row.
    let manifest = prepared.manifest(None, pages::now_ms());
    let blobs: Vec<(String, Vec<u8>)> = prepared
        .blobs
        .iter_mut()
        .map(|blob| (blob.file.clone(), std::mem::take(&mut blob.data)))
        .collect();
    let stored = this
        .update(cx, |this, cx| {
            this.set_adding_progress(progress(AddingPhase::Storing), cx);
            this.images.put_files(blobs, cx)
        })
        .ok()?
        .await;
    if let Err(reason) = stored {
        return Some((
            title.clone(),
            PageOutcome::NotAdded {
                sentence: format!(
                    "\u{201c}{title}\u{201d} was not added: its files could not be stored in the \
                     library ({reason}). Nothing was saved."
                ),
            },
        ));
    }
    let outcome = this
        .update(cx, |this, cx| this.store_page_row(&prepared, &manifest, cx))
        .ok()?;
    Some((title, outcome))
}
