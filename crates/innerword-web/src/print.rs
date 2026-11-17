//! A web view printed into a PDF: WebKit's print operation as a save job into a
//! temporary file (both panels off), on the paper of the user's default print info,
//! the file read and removed afterwards.
//!
//! The operation is run with `runOperationModalForWindow:delegate:didRunSelector:…`,
//! the call that draws a `WKWebView` (`runOperation` draws nothing). Nothing comes up
//! on screen, but AppKit may call the window's delegate while it runs: a caller in the
//! app calls this outside any GPUI update. AppKit ends the operation on a thread of its
//! own; the end is carried back to the main thread before anything else is done.

use std::{
    ffi::c_void,
    path::PathBuf,
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use dispatch::Queue;
use objc::{
    class,
    declare::ClassDecl,
    msg_send,
    runtime::{BOOL, Class, NO, Object, Sel},
    sel, sel_impl,
};

use crate::{
    PrintedPdf,
    ns::{self, Id, NSPrintJobSavingURL, NSPrintSaveJob, NSRect, nsstring, release, retain},
    pixels,
};

type Done = Box<dyn FnOnce(Result<PrintedPdf, String>)>;

/// What the delegate gets back when the operation has run.
struct Run {
    done: Done,
    path: PathBuf,
    /// The print info and the operation, held until it has run.
    held: [Id; 2],
}

/// The folder of this user's temporary files of Inner Word (mode 0700).
pub(crate) fn private_temp_dir() -> std::io::Result<PathBuf> {
    // SAFETY: getuid cannot fail.
    let uid = unsafe { getuid() };
    let dir = std::env::temp_dir().join(format!("inner-word-{uid}"));
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(dir)
}

unsafe extern "C" {
    fn getuid() -> u32;
}

fn temp_pdf() -> std::io::Result<PathBuf> {
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let n = COUNT.fetch_add(1, Ordering::Relaxed);
    Ok(private_temp_dir()?.join(format!("print-{}-{n}.pdf", std::process::id())))
}

fn delegate() -> Id {
    static DELEGATE: OnceLock<usize> = OnceLock::new();
    *DELEGATE.get_or_init(|| {
        const NAME: &str = "InnerWordWebPrintDelegate";
        let class = match ClassDecl::new(NAME, class!(NSObject)) {
            Some(mut decl) => {
                // SAFETY: the signature of `printOperationDidRun:success:contextInfo:`.
                unsafe {
                    decl.add_method(
                        sel!(printOperationDidRun:success:contextInfo:),
                        did_run as extern "C" fn(&Object, Sel, Id, BOOL, *mut c_void),
                    );
                }
                decl.register()
            }
            None => Class::get(NAME).expect("the print delegate's class"),
        };
        // SAFETY: `new` on a subclass of NSObject; kept for the life of the process
        // (an operation does not retain its delegate).
        let object: Id = unsafe { msg_send![class, new] };
        object as usize
    }) as Id
}

/// Prints `web_view` (which must be in `window`) into a PDF and hands it to `done`, on
/// the main thread, when the operation has run.
pub(crate) unsafe fn print_web_view(web_view: Id, window: Id, done: Done) {
    if window.is_null() {
        done(Err(
            "the page is not in a window, so it cannot be printed".to_string()
        ));
        return;
    }
    let path = match temp_pdf() {
        Ok(path) => path,
        Err(error) => {
            done(Err(format!("no temporary file for the print: {error}")));
            return;
        }
    };
    let _ = std::fs::remove_file(&path);
    // SAFETY: AppKit's and WebKit's own selectors with the types their headers
    // declare; what is made here is held until the operation has run.
    unsafe {
        let shared: Id = msg_send![class!(NSPrintInfo), sharedPrintInfo];
        let info: Id = msg_send![shared, copy];
        if info.is_null() {
            done(Err("macOS has no print settings".to_string()));
            return;
        }
        let _: () = msg_send![info, setJobDisposition: NSPrintSaveJob];
        let settings: Id = msg_send![info, dictionary];
        let url: Id = msg_send![class!(NSURL), fileURLWithPath: nsstring(&path.to_string_lossy())];
        let _: () = msg_send![settings, setObject: url forKey: NSPrintJobSavingURL];
        let operation: Id = msg_send![web_view, printOperationWithPrintInfo: info];
        if operation.is_null() {
            release(info);
            done(Err("WebKit made no print job for the page".to_string()));
            return;
        }
        let _: () = msg_send![operation, setShowsPrintPanel: NO];
        let _: () = msg_send![operation, setShowsProgressPanel: NO];
        // The printing view WebKit hands over has no size of its own until it is
        // given one; without it the pages come out empty.
        let view: Id = msg_send![operation, view];
        if !view.is_null() {
            let frame: NSRect = msg_send![view, frame];
            if frame.size.width <= 0. || frame.size.height <= 0. {
                let bounds: NSRect = msg_send![web_view, bounds];
                let _: () = msg_send![view, setFrame: bounds];
            }
        }
        let operation = retain(operation);
        let context = Box::into_raw(Box::new(Run {
            done,
            path,
            held: [info, operation],
        }));
        let _: () = msg_send![operation,
            runOperationModalForWindow: window
            delegate: delegate()
            didRunSelector: sel!(printOperationDidRun:success:contextInfo:)
            contextInfo: context.cast::<c_void>()];
    }
}

extern "C" fn did_run(_this: &Object, _: Sel, _operation: Id, success: BOOL, context: *mut c_void) {
    if context.is_null() {
        return;
    }
    // AppKit ends a modal print operation on a thread of its own
    // (`-[NSConcretePrintOperation _continueModalOperationToTheEnd:]`): the end is
    // carried to the main thread, where WebKit and the owner are.
    let context = context as usize;
    let success = success != NO;
    if ns::on_main_thread() {
        ended(context, success);
    } else {
        Queue::main().exec_async(move || ended(context, success));
    }
}

/// The end of a run, on the main thread.
fn ended(context: usize, success: bool) {
    // SAFETY: the box `print_web_view` made for this run; AppKit hands it back once.
    let run = unsafe { Box::from_raw(context as *mut Run) };
    let Run { done, path, held } = *run;
    for object in held {
        // SAFETY: the references `print_web_view` took.
        unsafe { release(object) };
    }
    if !success {
        let _ = std::fs::remove_file(&path);
        ns::guarded("the end of a print", move || {
            done(Err("the print job did not run to its end".to_string()))
        });
        return;
    }
    // The file is written by the time the operation reports; should WebKit still be
    // finishing it, it is waited for a moment.
    collect(path, done, Instant::now());
}

fn collect(path: PathBuf, done: Done, since: Instant) {
    let written = std::fs::metadata(&path).is_ok_and(|meta| meta.len() > 0);
    if !written && since.elapsed() < Duration::from_secs(10) {
        ns::after(Duration::from_millis(20), move || {
            collect(path, done, since)
        });
        return;
    }
    let result = std::fs::read(&path)
        .map_err(|error| format!("the printed PDF could not be read: {error}"))
        .and_then(|bytes| match pixels::pdf_pages(&bytes) {
            Some((pages, paper)) => Ok(PrintedPdf {
                bytes,
                pages,
                paper,
            }),
            None => Err("WebKit's print is not a PDF with pages".to_string()),
        });
    let _ = std::fs::remove_file(&path);
    ns::guarded("the end of a print", move || done(result));
}
