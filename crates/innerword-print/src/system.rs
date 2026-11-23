//! The macOS print operation: a finished PDF to the print panel, or to a file the
//! way "Save as PDF" writes it.
//!
//! PDFKit makes the operation (`-[PDFDocument
//! printOperationForPrintInfo:scalingMode:autoRotate:]`), AppKit runs it:
//!
//! * [`run_sheet`]: the print panel as a sheet on a window. It returns at once;
//!   the app goes on running under the sheet.
//! * [`run_save_job`]: the same operation as a save job with both panels off. No
//!   panel, no printer: what scripted runs and tests use, and the proof that
//!   everything but the panel works.
//!
//! What was measured before this was written (Swift probes of the same calls):
//!
//! * The page is put on the paper at its real size only with scaling off
//!   (`kPDFPrintPageScaleNone`) **and** with `NSPrintInfo`'s centering left on.
//!   With centering off PDFKit moves the page by the printer's hardware margin;
//!   "scale down to fit" is 96.6 % on paper of the right size.
//! * The selector is `scalingMode:`, not `scaleMode:`. A wrong selector through
//!   `msg_send!` is an exception at run time, not a compile error, so
//!   [`check_selectors`] asks the runtime for every one this module sends.
//! * A print operation runs once: a new one for every job.
//!
//! One struct comes back through `msg_send!` here, the user's `paperSize` (an
//! `NSSize`, two doubles: sixteen bytes, which both platforms return in
//! registers, so `objc_msgSend` is the right call on either); everything else
//! that comes back is an object or a boolean.
//!
//! Two scalings ([`Scaling`]): a document's pages at their real size on the
//! paper they were made for, and a drawing of its own size made smaller to fit
//! the user's paper, turned to the drawing's orientation.

// `objc`'s macros ask for a cfg this crate does not have.
#![allow(unexpected_cfgs)]

use std::{
    ffi::c_void,
    fmt,
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    sync::OnceLock,
};

use core_foundation::{base::TCFType, string::CFString, url::CFURL};
use objc::{
    class,
    declare::ClassDecl,
    msg_send,
    rc::autoreleasepool,
    runtime::{BOOL, Class, NO, Object, Sel, YES},
    sel, sel_impl,
};

use crate::pdf::Pdf;

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    /// `NSPrintJobDispositionValue`: the job is saved to a file.
    static NSPrintSaveJob: *mut Object;
    /// `NSPrintInfoAttributeKey`: the URL a save job writes to.
    static NSPrintJobSavingURL: *mut Object;
}

#[link(name = "PDFKit", kind = "framework")]
unsafe extern "C" {}

/// `kPDFPrintPageScaleNone`: the page at its real size.
const SCALE_NONE: isize = 0;
/// `kPDFPrintPageScaleDownToFit`: a page larger than the paper made smaller
/// until it fits, never larger.
const SCALE_DOWN_TO_FIT: isize = 2;

/// `NSSize`.
#[repr(C)]
#[derive(Clone, Copy)]
struct Size {
    width: f64,
    height: f64,
}

// SAFETY: the Objective-C type encoding of `NSSize` on 64-bit macOS.
unsafe impl objc::Encode for Size {
    fn encode() -> objc::Encoding {
        unsafe { objc::Encoding::from_str("{CGSize=dd}") }
    }
}

/// What is to be printed.
pub struct PrintRequest<'a> {
    /// The finished PDF. PDFKit is handed the very data it holds.
    pub pdf: &'a Pdf,
    /// The size of the paper in points, orientation applied (a size that is
    /// wider than high is landscape): the panel opens on it. `None`: the
    /// user's paper, as `NSPrintInfo.sharedPrintInfo` has it (which depends on
    /// the Mac's default printer: a test always names one).
    ///
    /// With [`Scaling::DownToFit`] the paper, given or the user's, is turned
    /// landscape when the PDF's first page is wider than high, and portrait
    /// otherwise.
    pub paper: Option<(f32, f32)>,
    /// Whether the pages are printed at their real size or made smaller to fit
    /// the paper.
    pub scaling: Scaling,
    /// The name of the job: what the print queue shows and what "Save as PDF"
    /// suggests as the file's name.
    pub title: &'a str,
}

/// How the pages of a PDF are put on the paper.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scaling {
    /// At their real size, a point being 1/72 inch (`kPDFPrintPageScaleNone`):
    /// the pages of a document, made on the paper they are printed on.
    #[default]
    Actual,
    /// Made smaller when larger than the paper, never larger
    /// (`kPDFPrintPageScaleDownToFit`): a drawing of its own size, printed on
    /// whatever paper the user has, turned to the drawing's orientation.
    DownToFit,
}

impl Scaling {
    /// PDFKit's `PDFPrintScalingMode`.
    fn mode(self) -> isize {
        match self {
            Scaling::Actual => SCALE_NONE,
            Scaling::DownToFit => SCALE_DOWN_TO_FIT,
        }
    }
}

/// `paper` turned to the orientation of a page of `page` size: landscape for a
/// page wider than high, portrait otherwise. A square paper stays as it is.
pub(crate) fn turned_to(paper: (f32, f32), page: (f32, f32)) -> (f32, f32) {
    let (short, long) = if paper.0 <= paper.1 {
        (paper.0, paper.1)
    } else {
        (paper.1, paper.0)
    };
    if page.0 > page.1 {
        (long, short)
    } else {
        (short, long)
    }
}

/// Why nothing was printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrintError {
    /// PDFKit does not read the data as a PDF.
    NotAPdf,
    /// PDFKit made no print operation for the PDF.
    NoOperation,
    /// Something else, in words.
    Failed(String),
}

impl fmt::Display for PrintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PrintError::NotAPdf => f.write_str("The pages could not be read as a PDF."),
            PrintError::NoOperation => f.write_str("macOS made no print job for the pages."),
            PrintError::Failed(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for PrintError {}

/// `text` as a string object. A `CFString` is an `NSString`: [`object`] hands
/// it to Objective-C for as long as the `CFString` lives.
pub(crate) fn string(text: &str) -> CFString {
    CFString::new(text)
}

/// A CoreFoundation object as the Objective-C object it also is (`CFString` is
/// `NSString`, `CFURL` is `NSURL`). Valid while `value` lives.
pub(crate) fn object<T: TCFType>(value: &T) -> *mut Object {
    value.as_CFTypeRef() as *mut Object
}

fn on_main_thread() -> bool {
    // SAFETY: a class method of Foundation that takes nothing.
    let main: BOOL = unsafe { msg_send![class!(NSThread), isMainThread] };
    main != NO
}

/// The user's print settings with the request's paper and no margins: a copy
/// the caller owns. Null when AppKit has none.
///
/// Centering is on: PDFKit puts the page exactly on the paper only then.
unsafe fn print_info(request: &PrintRequest<'_>) -> *mut Object {
    let Some(class) = Class::get("NSPrintInfo") else {
        return std::ptr::null_mut();
    };
    // SAFETY (the whole body): AppKit's own selectors with the argument types
    // its headers declare; `copy` returns an object the caller owns.
    unsafe {
        let shared: *mut Object = msg_send![class, sharedPrintInfo];
        if shared.is_null() {
            return shared;
        }
        let info: *mut Object = msg_send![shared, copy];
        if info.is_null() {
            return info;
        }
        let paper = match request.paper {
            Some(paper) => paper,
            None => {
                let size: Size = msg_send![info, paperSize];
                (size.width as f32, size.height as f32)
            }
        };
        let paper = match request.scaling {
            Scaling::Actual => paper,
            Scaling::DownToFit => match request.pdf.first_page_size() {
                Some(page) => turned_to(paper, page),
                None => paper,
            },
        };
        // A size that is wider than high makes the orientation landscape by
        // itself.
        let size = Size {
            width: f64::from(paper.0),
            height: f64::from(paper.1),
        };
        let _: () = msg_send![info, setPaperSize: size];
        let _: () = msg_send![info, setTopMargin: 0f64];
        let _: () = msg_send![info, setBottomMargin: 0f64];
        let _: () = msg_send![info, setLeftMargin: 0f64];
        let _: () = msg_send![info, setRightMargin: 0f64];
        let _: () = msg_send![info, setHorizontallyCentered: YES];
        let _: () = msg_send![info, setVerticallyCentered: YES];
        info
    }
}

/// The PDFKit document of a request and its print operation for `info`. The
/// document is owned by the caller; the operation is autoreleased.
unsafe fn operation(
    request: &PrintRequest<'_>,
    info: *mut Object,
) -> Result<(*mut Object, *mut Object), PrintError> {
    if request.pdf.is_empty() {
        return Err(PrintError::NotAPdf);
    }
    let Some(class) = Class::get("PDFDocument") else {
        return Err(PrintError::Failed("PDFKit is not there.".to_string()));
    };
    // SAFETY (the whole body): PDFKit's and AppKit's own selectors with the
    // argument types their headers declare. `initWithData:` takes an NSData,
    // which the PDF's CFData is, and keeps it.
    unsafe {
        let allocated: *mut Object = msg_send![class, alloc];
        let document: *mut Object =
            msg_send![allocated, initWithData: request.pdf.ns_data().cast::<Object>()];
        if document.is_null() {
            return Err(PrintError::NotAPdf);
        }
        let operation: *mut Object = msg_send![document,
            printOperationForPrintInfo: info
            scalingMode: request.scaling.mode()
            autoRotate: YES];
        if operation.is_null() {
            let _: () = msg_send![document, release];
            return Err(PrintError::NoOperation);
        }
        let title = string(request.title);
        let _: () = msg_send![operation, setJobTitle: object(&title)];
        Ok((document, operation))
    }
}

/// What the delegate is handed back when a sheet has ended.
struct SheetEnd {
    done: Box<dyn FnOnce(bool)>,
    /// The document, the print settings and the operation, each held until the
    /// sheet has ended.
    held: [*mut Object; 3],
}

/// `-printOperationDidRun:success:contextInfo:` of the delegate.
extern "C" fn did_run(
    _this: &Object,
    _selector: Sel,
    _operation: *mut Object,
    success: BOOL,
    context: *mut c_void,
) {
    if context.is_null() {
        return;
    }
    // SAFETY: the box `run_sheet` made for this run of the operation; AppKit
    // hands it back once.
    let end = unsafe { Box::from_raw(context.cast::<SheetEnd>()) };
    let SheetEnd { done, held } = *end;
    // This runs inside AppKit: a panic must not travel into it.
    let _ = catch_unwind(AssertUnwindSafe(move || done(success != NO)));
    for object in held {
        // SAFETY: a reference `run_sheet` took.
        unsafe {
            let _: () = msg_send![object, release];
        }
    }
}

/// The one delegate of every print sheet: an object of a class made here, kept
/// for the life of the process (a print operation does not retain its delegate).
fn delegate() -> Option<*mut Object> {
    static DELEGATE: OnceLock<usize> = OnceLock::new();
    let address = *DELEGATE.get_or_init(|| {
        const NAME: &str = "InnerWordPrintSheetDelegate";
        let class = match ClassDecl::new(NAME, class!(NSObject)) {
            Some(mut declared) => {
                // SAFETY: the function has the signature the selector's type
                // encoding says: an object, a BOOL and a pointer, no result.
                unsafe {
                    declared.add_method(
                        sel!(printOperationDidRun:success:contextInfo:),
                        did_run as extern "C" fn(&Object, Sel, *mut Object, BOOL, *mut c_void),
                    );
                }
                Some(declared.register())
            }
            // Declared before (by an earlier call that lost a race).
            None => Class::get(NAME),
        };
        let Some(class) = class else {
            return 0;
        };
        // SAFETY: `new` on a subclass of NSObject returns an object we own.
        let object: *mut Object = unsafe { msg_send![class, new] };
        object as usize
    });
    (address != 0).then_some(address as *mut Object)
}

/// Shows the macOS print panel as a sheet on `ns_window` (an `NSWindow *`).
/// Returns at once.
///
/// `done(printed)` runs on the main thread, inside AppKit, when the sheet has
/// ended (`false` for Cancel too): it may only send on a channel. When this
/// returns an error there is no sheet and `done` is dropped without having run.
///
/// # Safety
///
/// `ns_window` must be a valid `NSWindow *`. Call it on the main thread and
/// **outside a GPUI update** (from a foreground task): AppKit calls the window's
/// delegate while the sheet comes up, and that re-enters GPUI.
pub unsafe fn run_sheet(
    request: &PrintRequest<'_>,
    ns_window: *mut c_void,
    done: Box<dyn FnOnce(bool)>,
) -> Result<(), PrintError> {
    if !on_main_thread() {
        return Err(PrintError::Failed(
            "The print panel was asked for off the main thread.".to_string(),
        ));
    }
    if ns_window.is_null() {
        return Err(PrintError::Failed("There is no window.".to_string()));
    }
    let Some(delegate) = delegate() else {
        return Err(PrintError::Failed(
            "The print panel has nobody to answer to.".to_string(),
        ));
    };
    autoreleasepool(move || {
        // SAFETY: on the main thread, with a valid window (the caller's
        // promise); every object is held until the sheet has ended.
        unsafe {
            let info = print_info(request);
            if info.is_null() {
                return Err(PrintError::Failed(
                    "macOS has no print settings.".to_string(),
                ));
            }
            let (document, operation) = match operation(request, info) {
                Ok(made) => made,
                Err(error) => {
                    let _: () = msg_send![info, release];
                    return Err(error);
                }
            };
            let _: *mut Object = msg_send![operation, retain];
            let context = Box::into_raw(Box::new(SheetEnd {
                done,
                held: [document, info, operation],
            }));
            let _: () = msg_send![operation,
                runOperationModalForWindow: ns_window.cast::<Object>()
                delegate: delegate
                didRunSelector: sel!(printOperationDidRun:success:contextInfo:)
                contextInfo: context.cast::<c_void>()];
            Ok(())
        }
    })
}

/// The same print operation as a save job, both panels off: writes to `to` what
/// "Save as PDF" in the print panel would write. No panel, no printer.
///
/// Call it on the main thread of a program that has an `NSApplication`. It
/// returns when the file is written.
pub fn run_save_job(request: &PrintRequest<'_>, to: &Path) -> Result<(), PrintError> {
    if !on_main_thread() {
        return Err(PrintError::Failed(
            "The print job was asked for off the main thread.".to_string(),
        ));
    }
    let Some(url) = CFURL::from_path(to, false) else {
        return Err(PrintError::Failed(
            "The path of the file cannot be made a URL.".to_string(),
        ));
    };
    autoreleasepool(|| {
        // SAFETY: on the main thread; AppKit's own selectors with the argument
        // types its headers declare; what is made here is released here.
        unsafe {
            let info = print_info(request);
            if info.is_null() {
                return Err(PrintError::Failed(
                    "macOS has no print settings.".to_string(),
                ));
            }
            let _: () = msg_send![info, setJobDisposition: NSPrintSaveJob];
            let settings: *mut Object = msg_send![info, dictionary];
            let _: () = msg_send![settings, setObject: object(&url) forKey: NSPrintJobSavingURL];
            let (document, operation) = match operation(request, info) {
                Ok(made) => made,
                Err(error) => {
                    let _: () = msg_send![info, release];
                    return Err(error);
                }
            };
            let _: () = msg_send![operation, setShowsPrintPanel: NO];
            let _: () = msg_send![operation, setShowsProgressPanel: NO];
            let ran: BOOL = msg_send![operation, runOperation];
            let _: () = msg_send![document, release];
            let _: () = msg_send![info, release];
            if ran == NO {
                Err(PrintError::Failed(
                    "The print job did not run to its end.".to_string(),
                ))
            } else {
                Ok(())
            }
        }
    })
}

/// Every class and selector [`run_sheet`] and [`run_save_job`] send, with whether
/// it goes to the class itself.
const USED: [(&str, bool, &str); 21] = [
    ("NSThread", true, "isMainThread"),
    ("NSMutableDictionary", false, "setObject:forKey:"),
    ("NSPrintInfo", true, "sharedPrintInfo"),
    ("NSPrintInfo", false, "copy"),
    ("NSPrintInfo", false, "paperSize"),
    ("NSPrintInfo", false, "setPaperSize:"),
    ("NSPrintInfo", false, "setTopMargin:"),
    ("NSPrintInfo", false, "setBottomMargin:"),
    ("NSPrintInfo", false, "setLeftMargin:"),
    ("NSPrintInfo", false, "setRightMargin:"),
    ("NSPrintInfo", false, "setHorizontallyCentered:"),
    ("NSPrintInfo", false, "setVerticallyCentered:"),
    ("NSPrintInfo", false, "setJobDisposition:"),
    ("NSPrintInfo", false, "dictionary"),
    ("PDFDocument", false, "initWithData:"),
    (
        "PDFDocument",
        false,
        "printOperationForPrintInfo:scalingMode:autoRotate:",
    ),
    ("NSPrintOperation", false, "setJobTitle:"),
    ("NSPrintOperation", false, "setShowsPrintPanel:"),
    ("NSPrintOperation", false, "setShowsProgressPanel:"),
    ("NSPrintOperation", false, "runOperation"),
    (
        "NSPrintOperation",
        false,
        "runOperationModalForWindow:delegate:didRunSelector:contextInfo:",
    ),
];

/// Whether every class and selector [`run_sheet`] and [`run_save_job`] use
/// exists in the Objective-C runtime of this Mac. The error lists the names that
/// do not (`-[PDFDocument printOperationForPrintInfo:scaleMode:autoRotate:]`,
/// `class PDFDocument`).
pub fn check_selectors() -> Result<(), Vec<String>> {
    let mut failed = Vec::new();
    autoreleasepool(|| {
        for (class_name, of_class, selector) in USED {
            let Some(class) = Class::get(class_name) else {
                let name = format!("class {class_name}");
                if !failed.contains(&name) {
                    failed.push(name);
                }
                continue;
            };
            let selector_name = selector;
            let selector = Sel::register(selector_name);
            // SAFETY: both questions are NSObject's and take a selector.
            let answers: BOOL = unsafe {
                if of_class {
                    msg_send![class, respondsToSelector: selector]
                } else {
                    msg_send![class, instancesRespondToSelector: selector]
                }
            };
            if answers == NO {
                failed.push(format!(
                    "{}[{class_name} {selector_name}]",
                    if of_class { '+' } else { '-' }
                ));
            }
        }
    });
    if delegate().is_none() {
        failed.push("the delegate of the print sheet".to_string());
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(failed)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::Path,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
    };

    use super::*;
    use crate::{JobInfo, Page, PdfWriter};

    fn a_pdf() -> Pdf {
        let mut writer = PdfWriter::new(&JobInfo {
            title: "A page".to_string(),
            creator: "A test".to_string(),
        });
        writer.add_page(&Page {
            width: 595.276,
            height: 841.89,
            items: Vec::new(),
        });
        writer.finish()
    }

    /// Three objects for a sheet's end to release.
    fn held() -> [*mut Object; 3] {
        // SAFETY: `new` on NSObject returns an object the caller owns.
        [(); 3].map(|()| unsafe { msg_send![class!(NSObject), new] })
    }

    #[test]
    fn the_delegate_hands_the_result_back() {
        let delegate = delegate().expect("the delegate of the print sheet");
        // The one object, however often it is asked for.
        assert_eq!(delegate, super::delegate().unwrap());
        // It answers the selector the print operation is given, with the
        // types AppKit calls it with: nothing back; the operation, a BOOL
        // and a pointer in.
        let selector = sel!(printOperationDidRun:success:contextInfo:);
        // SAFETY: a live object of a registered class.
        let class = unsafe { &*delegate }.class();
        let method = class
            .instance_method(selector)
            .expect("the delegate has the method");
        assert_eq!(method.return_type(), <() as objc::Encode>::encode());
        assert_eq!(method.arguments_count(), 5);
        assert_eq!(
            method.argument_type(2),
            Some(<*mut Object as objc::Encode>::encode())
        );
        assert_eq!(
            method.argument_type(3),
            Some(<BOOL as objc::Encode>::encode())
        );
        assert_eq!(
            method.argument_type(4),
            Some(<*mut c_void as objc::Encode>::encode())
        );

        // Called the way AppKit calls it when a sheet has ended: `done` gets
        // the result, once, and what was held is let go.
        for success in [YES, NO] {
            let (sender, receiver) = mpsc::channel();
            let context = Box::into_raw(Box::new(SheetEnd {
                done: Box::new(move |printed| sender.send(printed).expect("the receiver")),
                held: held(),
            }));
            // SAFETY: the delegate's own method with the types it declares;
            // the context is a `SheetEnd` it takes over.
            unsafe {
                let _: () = msg_send![delegate,
                    printOperationDidRun: std::ptr::null_mut::<Object>()
                    success: success
                    contextInfo: context.cast::<c_void>()];
            }
            assert_eq!(receiver.try_recv(), Ok(success != NO));
            assert!(receiver.try_recv().is_err(), "once");
        }

        // A panic in `done` does not travel into AppKit, and nothing happens
        // without a context.
        let context = Box::into_raw(Box::new(SheetEnd {
            done: Box::new(|_| panic!("a closure that does more than send")),
            held: held(),
        }));
        // SAFETY: as above.
        unsafe {
            let _: () = msg_send![delegate,
                printOperationDidRun: std::ptr::null_mut::<Object>()
                success: YES
                contextInfo: context.cast::<c_void>()];
            let _: () = msg_send![delegate,
                printOperationDidRun: std::ptr::null_mut::<Object>()
                success: YES
                contextInfo: std::ptr::null_mut::<c_void>()];
        }
    }

    #[test]
    fn nothing_is_started_off_the_main_thread() {
        // A test runs on a thread of the harness: AppKit must not be asked
        // for a print operation from here, and is not.
        assert!(!on_main_thread());
        let pdf = a_pdf();
        let request = PrintRequest {
            pdf: &pdf,
            paper: Some((595.276, 841.89)),
            scaling: Scaling::Actual,
            title: "A page",
        };
        let saved = run_save_job(&request, Path::new("/nonexistent/never-written.pdf"));
        assert!(matches!(saved, Err(PrintError::Failed(_))), "{saved:?}");
        assert!(!Path::new("/nonexistent/never-written.pdf").exists());

        let ran = Arc::new(AtomicBool::new(false));
        let done = {
            let ran = ran.clone();
            Box::new(move |_| ran.store(true, Ordering::SeqCst))
        };
        // SAFETY: the thread is asked about before the window is touched;
        // no window is given at all.
        let shown = unsafe { run_sheet(&request, std::ptr::null_mut(), done) };
        assert!(matches!(shown, Err(PrintError::Failed(_))), "{shown:?}");
        assert!(
            !ran.load(Ordering::SeqCst),
            "`done` is dropped without having run"
        );
        assert_eq!(Arc::strong_count(&ran), 1, "and it is dropped");
    }

    #[test]
    fn the_paper_is_turned_to_the_drawing() {
        const A4: (f32, f32) = (595.276, 841.89);
        const A4_LANDSCAPE: (f32, f32) = (841.89, 595.276);
        assert_eq!(turned_to(A4, (1600., 900.)), A4_LANDSCAPE);
        assert_eq!(turned_to(A4_LANDSCAPE, (1600., 900.)), A4_LANDSCAPE);
        assert_eq!(turned_to(A4, (300., 900.)), A4);
        assert_eq!(turned_to(A4_LANDSCAPE, (300., 900.)), A4);
        // A square drawing is portrait.
        assert_eq!(turned_to(A4_LANDSCAPE, (500., 500.)), A4);
        assert_eq!(Scaling::default(), Scaling::Actual);
        assert_eq!(Scaling::Actual.mode(), 0);
        assert_eq!(Scaling::DownToFit.mode(), 2);
    }

    #[test]
    fn an_error_says_what_went_wrong() {
        assert_eq!(
            PrintError::NotAPdf.to_string(),
            "The pages could not be read as a PDF."
        );
        assert_eq!(
            PrintError::NoOperation.to_string(),
            "macOS made no print job for the pages."
        );
        assert_eq!(PrintError::Failed("Why.".to_string()).to_string(), "Why.");
        let error: Box<dyn std::error::Error> = Box::new(PrintError::NotAPdf);
        assert!(error.source().is_none());
    }
}
