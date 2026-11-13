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
//! No struct comes back through `msg_send!` here (sizes go in, results are
//! objects and booleans), so nothing depends on how a platform returns one.

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
    /// The size of the document's paper in points, orientation applied (a size
    /// that is wider than high is landscape). The panel opens on it.
    pub paper: (f32, f32),
    /// The name of the job: what the print queue shows and what "Save as PDF"
    /// suggests as the file's name.
    pub title: &'a str,
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

/// The user's print settings with the document's paper and no margins: a copy
/// the caller owns. Null when AppKit has none.
///
/// Centering is on: PDFKit puts the page exactly on the paper only then.
unsafe fn print_info(paper: (f32, f32)) -> *mut Object {
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
            scalingMode: SCALE_NONE
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
        const NAME: &str = "TheWriterPrintSheetDelegate";
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
            let info = print_info(request.paper);
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
            let info = print_info(request.paper);
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
const USED: [(&str, bool, &str); 20] = [
    ("NSThread", true, "isMainThread"),
    ("NSMutableDictionary", false, "setObject:forKey:"),
    ("NSPrintInfo", true, "sharedPrintInfo"),
    ("NSPrintInfo", false, "copy"),
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
