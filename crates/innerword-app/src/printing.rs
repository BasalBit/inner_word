//! Printing, as the app sees it: where a finished print job goes.
//!
//! The pages are the editor's ([`innerword_editor::PrintPass`]: the document
//! laid out as page view lays it out, whatever the window shows), the PDF
//! and the macOS print operation are `innerword-print`'s. The command itself,
//! "Print…", is the workspace's (`workspace/print.rs`). This module holds
//! what lies between: the job ([`PrintJob`]) and who takes it
//! ([`PrintTarget`]).
//!
//! - **Which target a workspace hands its jobs to** ([`set_target`]):
//!   `main` installs the macOS print panel ([`use_system_panel`]), or for a
//!   scripted run a directory ([`PrintTarget::Directory`]); a test records
//!   the jobs ([`record`]). Without a target a workspace has no Print at
//!   all (no handler, the menu items greyed out), as it has no spell
//!   checker without a spell service: no test and no scripted run ever
//!   meets the print panel by accident.
//! - **How long a slice of preparing works** ([`set_slice`]). The pages of
//!   a book are made in slices with a turn of the main thread in between,
//!   so the window stays live; a test sets zero, which makes a slice one
//!   block and lets the test hold a job in "preparing".
//! - **What a scripted run logs about a job** ([`report_lines`]).
//!
//! Nothing here opens the print panel but [`show_panel`], and only the
//! workspace calls it, for the target [`PrintTarget::SystemPanel`].

use std::{
    cell::RefCell,
    future::Future,
    path::PathBuf,
    pin::Pin,
    rc::Rc,
    task::{Context as TaskContext, Poll, Waker},
    time::Duration,
};

use gpui::{App, Global, Window};
use innerword_print::{JobInfo, Pdf, PrintError, PrintRequest, Scaling};

/// How long one slice of preparing works unless [`set_slice`] says
/// otherwise: half a frame, so a key pressed meanwhile is not kept
/// waiting.
pub const DEFAULT_SLICE: Duration = Duration::from_millis(8);

/// What "Print…" hands over: the pages of a document as a PDF.
#[derive(Clone, Debug)]
pub struct PrintJob {
    /// The document's title: the name of the job in the print queue, and
    /// what "Save as PDF" suggests as the file's name.
    pub title: String,
    /// The size of the document's paper in points, orientation applied.
    pub paper: (f32, f32),
    /// The pages, with the report of how they were made
    /// ([`innerword_print::Report`]).
    pub pdf: Pdf,
}

impl PrintJob {
    /// The job as the print operation of `innerword-print` takes it.
    pub fn request(&self) -> PrintRequest<'_> {
        PrintRequest {
            pdf: &self.pdf,
            paper: Some(self.paper),
            scaling: Scaling::Actual,
            title: &self.title,
        }
    }

    /// The name of the file a scripted run writes the job to: the title as
    /// an export names its file, with `.pdf`.
    pub fn file_name(&self) -> String {
        crate::export::pdf_file_name(&self.title)
    }
}

/// Who takes a print job.
#[derive(Clone)]
pub enum PrintTarget {
    /// The macOS print panel as a sheet on the window. `main` installs it.
    SystemPanel,
    /// Tests: every job is kept, nothing is shown.
    Recorder(Rc<RefCell<Vec<PrintJob>>>),
    /// A scripted run: "<title>.pdf" into this directory
    /// (`INNER_WORD_EXPORT_DIR`), or with `None` only a toast.
    Directory(Option<PathBuf>),
}

/// The installed target.
struct Installed {
    target: PrintTarget,
}

impl Global for Installed {}

struct Slice(Duration);

impl Global for Slice {}

/// Installs `target` for every workspace: from now on "Print…" is there
/// while a document is open.
pub fn set_target(target: PrintTarget, cx: &mut App) {
    cx.set_global(Installed { target });
}

/// Installs the macOS print panel. Called by `main` in a launch that is
/// not scripted, and by nothing else.
pub fn use_system_panel(cx: &mut App) {
    set_target(PrintTarget::SystemPanel, cx);
}

/// Installs a recorder and returns what it keeps: every job "Print…"
/// hands over, in order. A test calls this before it opens a workspace.
pub fn record(cx: &mut App) -> Rc<RefCell<Vec<PrintJob>>> {
    let jobs = Rc::new(RefCell::new(Vec::new()));
    set_target(PrintTarget::Recorder(jobs.clone()), cx);
    jobs
}

/// The installed target, if there is one.
pub fn target(cx: &App) -> Option<PrintTarget> {
    cx.try_global::<Installed>()
        .map(|installed| installed.target.clone())
}

/// How long one slice of preparing works ([`DEFAULT_SLICE`] unless set).
/// A test sets zero: a slice is then one block, and the job stays
/// "preparing" until the test clock is advanced.
pub fn set_slice(budget: Duration, cx: &mut App) {
    cx.set_global(Slice(budget));
}

/// The length of a slice of preparing.
pub fn slice(cx: &App) -> Duration {
    cx.try_global::<Slice>()
        .map_or(DEFAULT_SLICE, |slice| slice.0)
}

/// What a PDF made by this app says about itself.
pub fn job_info(title: &str) -> JobInfo {
    JobInfo {
        title: title.to_string(),
        creator: format!("Inner Word {}", env!("CARGO_PKG_VERSION")),
    }
}

/// What a scripted run logs about a job that took `made` to make: the
/// report of the PDF writer (pages, paper, rows, rows set in another face
/// than the layout's, images, fonts) and the time, and a second line when
/// a font the document asks for is not on this Mac.
pub fn report_lines(job: &PrintJob, made: Duration) -> Vec<String> {
    let mut lines = vec![format!(
        "{}, made in {:.2} s",
        job.pdf.report.line(job.paper),
        made.as_secs_f64()
    )];
    lines.extend(job.pdf.report.missing_line());
    lines
}

// ----- the panel ------------------------------------------------------------------------

/// The end of a print panel, for whoever waits for it: resolves to whether
/// the job was printed (`false` for Cancel, and when the panel went away
/// without saying).
///
/// The other half ([`PanelEnd`]) is called by AppKit when the sheet has
/// ended, on the main thread and from inside AppKit: it only notes the
/// answer and wakes the task that waits, which runs in a later turn of the
/// main thread. No thread waits meanwhile.
pub struct PanelEnded(Rc<RefCell<End>>);

/// The half of [`PanelEnded`] that says the panel has ended.
pub struct PanelEnd(Rc<RefCell<End>>);

#[derive(Default)]
struct End {
    answer: Option<bool>,
    /// The sender is gone: nobody will answer.
    gone: bool,
    waker: Option<Waker>,
}

/// A pair for one panel: what its end is told to, and what waits for it.
pub fn panel_end() -> (PanelEnd, PanelEnded) {
    let end = Rc::new(RefCell::new(End::default()));
    (PanelEnd(end.clone()), PanelEnded(end))
}

impl PanelEnd {
    /// The panel has ended; `printed` is false for Cancel.
    pub fn send(self, printed: bool) {
        self.0.borrow_mut().answer = Some(printed);
        // Dropping `self` wakes the task.
    }
}

impl Drop for PanelEnd {
    fn drop(&mut self) {
        let waker = {
            let mut end = self.0.borrow_mut();
            end.gone = true;
            end.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl Future for PanelEnded {
    type Output = bool;

    fn poll(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<bool> {
        let mut end = self.0.borrow_mut();
        match end.answer {
            Some(printed) => Poll::Ready(printed),
            None if end.gone => Poll::Ready(false),
            None => {
                end.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

/// The `NSWindow` behind a GPUI window, as a number that can be carried
/// out of an update. `None` when the window is not on screen as an AppKit
/// window.
#[cfg(target_os = "macos")]
// objc 0.2's msg_send! expands to a `cfg(feature = "cargo-clippy")` check.
#[allow(unexpected_cfgs)]
pub fn native_window(window: &Window) -> Option<NativeWindow> {
    use objc::{msg_send, runtime::Object, sel, sel_impl};

    let view = crate::screenshot::native_view(window).ok()?;
    let ns_view = view.as_ptr() as *mut Object;
    if ns_view.is_null() {
        return None;
    }
    // SAFETY: `ns_view` is the live NSView backing this GPUI window, and
    // `window` is a plain AppKit getter called on the main thread.
    let ns_window: *mut Object = unsafe { msg_send![ns_view, window] };
    (!ns_window.is_null()).then_some(NativeWindow(ns_window as usize))
}

#[cfg(not(target_os = "macos"))]
pub fn native_window(_window: &Window) -> Option<NativeWindow> {
    None
}

/// See [`native_window`]. Good for as long as the window is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeWindow(usize);

/// Shows the macOS print panel for `job` as a sheet on `window` and
/// returns at once; `end` is told when the sheet has ended. An error means
/// there is no sheet, and `end` was dropped (whoever waits is answered
/// "not printed").
///
/// Call it on the main thread, **outside any GPUI update** (from a
/// foreground task, between two awaits): AppKit calls the window's
/// delegate while the sheet comes up, and that re-enters GPUI. The window
/// must still be open.
///
/// **No test and no scripted run calls this**: nothing but a person can
/// answer a print panel.
pub fn show_panel(job: &PrintJob, window: NativeWindow, end: PanelEnd) -> Result<(), PrintError> {
    // A message this macOS does not understand would end the app: ask
    // the runtime first, and say that there is no panel instead.
    if let Err(missing) = innerword_print::check_selectors() {
        return Err(PrintError::Failed(format!(
            "This macOS does not have what the print panel is asked for: {}",
            missing.join(", ")
        )));
    }
    // SAFETY: `window` came from `native_window` for a window that is
    // still open (the caller's promise, as is the main thread and being
    // outside an update). The closure only notes the answer and wakes a
    // task.
    unsafe {
        innerword_print::run_sheet(
            &job.request(),
            window.0 as *mut std::ffi::c_void,
            Box::new(move |printed| end.send(printed)),
        )
    }
}

// ----- a panel for tests ----------------------------------------------------------------

/// A stand-in for the print panel in tests: with one installed
/// ([`hold_the_panel`]), a job that reaches the recorder stays "in the
/// panel" until the test ends it, as a job stays in the real panel until
/// the user has printed or cancelled.
#[derive(Clone, Default)]
pub struct PanelStandIn {
    open: Rc<RefCell<Vec<PanelEnd>>>,
}

impl Global for PanelStandIn {}

impl PanelStandIn {
    /// How many panels are up.
    pub fn open_count(&self) -> usize {
        self.open.borrow().len()
    }

    /// Ends the panel that came up first, as "Print" (`true`) or "Cancel"
    /// in it does. False when no panel is up.
    pub fn end(&self, printed: bool) -> bool {
        let first = {
            let mut open = self.open.borrow_mut();
            (!open.is_empty()).then(|| open.remove(0))
        };
        match first {
            Some(end) => {
                end.send(printed);
                true
            }
            None => false,
        }
    }

    /// A panel comes up: what waits for its end.
    pub(crate) fn open(&self) -> PanelEnded {
        let (end, ended) = panel_end();
        self.open.borrow_mut().push(end);
        ended
    }
}

/// Tests: from now on a recorded job stays in a panel that only the
/// returned stand-in ends. Call it beside [`record`].
pub fn hold_the_panel(cx: &mut App) -> PanelStandIn {
    let panel = PanelStandIn::default();
    cx.set_global(panel.clone());
    panel
}

/// The stand-in of [`hold_the_panel`], if a test installed one.
pub(crate) fn held_panel(cx: &App) -> Option<PanelStandIn> {
    cx.try_global::<PanelStandIn>().cloned()
}
