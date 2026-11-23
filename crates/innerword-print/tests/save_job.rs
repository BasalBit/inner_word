//! The macOS print operation, run from Rust as a save job: no panel, no printer.
//!
//! `innerword_print::run_save_job` hands a PDF to PDFKit's print operation with the
//! job set to "save" and both panels off, which writes what "Save as PDF" in the
//! print panel writes. That is every step of printing but the panel itself: the
//! print settings, the paper, PDFKit's document over the writer's own data, the
//! operation with scaling off, and the run.
//!
//! This is a program of its own and not a `#[test]`: AppKit runs a print
//! operation on the main thread of the process, and the default test harness
//! runs every test on another one. `cargo test -p innerword-print` runs it with the
//! rest; it exits with a failure when anything below does not hold.
//!
//! It writes one file into a folder of its own under the system's temporary
//! directory and removes the folder again.

#![allow(unexpected_cfgs)]

use std::{cell::Cell, path::PathBuf, process::ExitCode, rc::Rc, sync::Arc};

use innerword_print::{
    Anchor, Color, FontSpec, GlyphStart, Item, JobInfo, Label, Page, PdfFile, PdfWriter,
    PrintError, PrintRequest, RectItem, RowRun, Scaling, TextRow, check_selectors, run_save_job,
    run_sheet,
};
use objc::{
    class, msg_send,
    runtime::{Object, YES},
    sel, sel_impl,
};

const A4: (f32, f32) = (595.276, 841.89);
const INK: Color = Color {
    r: 0.1,
    g: 0.1,
    b: 0.1,
};

fn font(family: &str) -> Arc<FontSpec> {
    Arc::new(FontSpec {
        family: family.to_string(),
        weight: 400.,
        italic: false,
        fallbacks: Vec::new(),
    })
}

/// A row in Courier New with its glyphs 0.6 em apart.
fn row(text: &str, x: f32, baseline: f32) -> Item {
    let size = 12.;
    let glyphs: Vec<GlyphStart> = text
        .char_indices()
        .enumerate()
        .map(|(index, (byte, _))| GlyphStart {
            byte: byte as u32,
            x: index as f32 * 0.6 * size,
            scale: 1.,
        })
        .collect();
    Item::Row(TextRow {
        text: text.to_string(),
        x,
        baseline,
        size,
        runs: vec![RowRun {
            end: text.len(),
            font: font("Courier New"),
            color: INK,
        }],
        width: glyphs.len() as f32 * 0.6 * size,
        glyphs,
    })
}

fn page(number: usize, rows: &[&str]) -> Page {
    let mut items = vec![Item::Rect(RectItem {
        x: 72.,
        y: 60.,
        width: A4.0 - 144.,
        height: 1.,
        radius: 0.,
        fill: Some(INK),
        border: None,
    })];
    for (index, text) in rows.iter().enumerate() {
        items.push(row(text, 72., 100. + index as f32 * 20.));
    }
    items.push(Item::Label(Label {
        text: number.to_string(),
        font: font(".SystemUIFont"),
        size: 11.,
        color: INK,
        x: A4.0 / 2.,
        baseline: A4.1 - 30.,
        anchor: Anchor::Middle,
    }));
    Page {
        width: A4.0,
        height: A4.1,
        items,
    }
}

/// `NSSize`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
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

/// `NSRect`, its origin at the bottom left.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// The printer a job on `paper` goes to, the paper it has and the part of it
/// the printer prints on (`imageablePageBounds`), from print settings made the
/// way `innerword_print`'s `print_info` makes them: the user's, with that paper,
/// no margins and centring on.
fn printable_area(paper: (f32, f32)) -> (String, Size, Rect) {
    // SAFETY: on the main thread; AppKit's own selectors with the types its
    // headers declare; the copy is released here. `imageablePageBounds`
    // returns an NSRect, which `msg_send!` returns the way the platform
    // returns a struct of four doubles.
    unsafe {
        let shared: *mut Object = msg_send![class!(NSPrintInfo), sharedPrintInfo];
        let info: *mut Object = msg_send![shared, copy];
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
        let paper: Size = msg_send![info, paperSize];
        let area: Rect = msg_send![info, imageablePageBounds];
        let printer: *mut Object = msg_send![info, printer];
        let name: *mut Object = if printer.is_null() {
            std::ptr::null_mut()
        } else {
            msg_send![printer, name]
        };
        let name = if name.is_null() {
            String::new()
        } else {
            let utf8: *const std::ffi::c_char = msg_send![name, UTF8String];
            if utf8.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr(utf8)
                    .to_string_lossy()
                    .into_owned()
            }
        };
        let _: () = msg_send![info, release];
        (name, paper, area)
    }
}

fn check(holds: bool, what: &str, failures: &mut Vec<String>) {
    if holds {
        println!("save_job: ok: {what}");
    } else {
        println!("save_job: FAILED: {what}");
        failures.push(what.to_string());
    }
}

/// A drawing of its own size, 1,600 x 900 points, printed as a whiteboard is:
/// made smaller to fit A4 and the paper turned to its orientation. The paper is
/// named, never the user's (`None`), which would depend on the Mac's default
/// printer.
fn drawing_down_to_fit(folder: &std::path::Path, failures: &mut Vec<String>) {
    const DRAWING: (f32, f32) = (1600., 900.);
    let mut writer = PdfWriter::new(&JobInfo {
        title: "Plan".to_string(),
        creator: "Inner Word (a test)".to_string(),
    });
    let mut items = vec![Item::Rect(RectItem {
        x: 32.,
        y: 32.,
        width: DRAWING.0 - 64.,
        height: DRAWING.1 - 64.,
        radius: 0.,
        fill: None,
        border: Some((2., INK)),
    })];
    items.push(row("System overview", 100., 100.));
    writer.add_page(&Page {
        width: DRAWING.0,
        height: DRAWING.1,
        items,
    });
    let pdf = writer.finish();
    let to = folder.join("Plan.pdf");
    let ran = run_save_job(
        &PrintRequest {
            pdf: &pdf,
            paper: Some(A4),
            scaling: Scaling::DownToFit,
            title: "Plan",
        },
        &to,
    );
    check(
        ran.is_ok(),
        &format!("a drawing ran as a save job, down to fit ({ran:?})"),
        failures,
    );
    let Some(file) = std::fs::read(&to)
        .ok()
        .and_then(|bytes| PdfFile::open(&bytes))
    else {
        check(false, "the drawing's job wrote a PDF", failures);
        return;
    };
    check(file.page_count() == 1, "the drawing is one sheet", failures);
    let (width, height) = file.page_size(0);
    check(
        (width - A4.1).abs() < 0.6 && (height - A4.0).abs() < 0.6,
        &format!("the sheet is A4 turned landscape ({width} x {height})"),
        failures,
    );
    let source = PdfFile::open(pdf.bytes()).expect("the writer's PDF");
    let before = source.find(0, "overview");
    let after = file.find(0, "overview");
    println!("save_job: the drawing's word in the PDF {before:?}, on the sheet {after:?}");
    // Made smaller by at most what fits the sheet (A4's long side over the
    // drawing's: 0.526), and not by much more (the printer's margins).
    let fits = f64::from(A4.1 / DRAWING.0).min(f64::from(A4.0 / DRAWING.1));
    check(
        match (before, after) {
            (Some(before), Some(after)) => {
                let scale = f64::from(after.2 / before.2);
                scale <= fits + 0.005 && scale > fits * 0.85
            }
            _ => false,
        },
        &format!(
            "the drawing is made smaller to fit, its text still text (at most {fits:.3}: \
             in the PDF {before:?}, on the sheet {after:?})"
        ),
        failures,
    );
}

fn main() -> ExitCode {
    let mut failures = Vec::new();

    // AppKit's print operation belongs to an application.
    // SAFETY: on the main thread of the process; the selector takes nothing and
    // returns the application object.
    let app: *mut Object = unsafe { msg_send![class!(NSApplication), sharedApplication] };
    check(
        !app.is_null(),
        "there is an application object",
        &mut failures,
    );

    match check_selectors() {
        Ok(()) => check(
            true,
            "every selector of the print operation is there",
            &mut failures,
        ),
        Err(missing) => check(
            false,
            &format!("selectors that are not there: {missing:?}"),
            &mut failures,
        ),
    }

    let first = ["The first page of the job.", "A second row on it."];
    let second = ["The second page of the job."];
    let mut writer = PdfWriter::new(&JobInfo {
        title: "Save job".to_string(),
        creator: "Inner Word (a test)".to_string(),
    });
    writer.add_page(&page(1, &first));
    writer.add_page(&page(2, &second));
    let pdf = writer.finish();
    check(
        pdf.report.pages == 2,
        "the writer made two pages",
        &mut failures,
    );

    let folder: PathBuf = std::env::temp_dir().join(format!(
        "inner-word-save-job-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos())
    ));
    if let Err(error) = std::fs::create_dir_all(&folder) {
        println!("save_job: FAILED: no folder to save into: {error}");
        return ExitCode::FAILURE;
    }
    let to = folder.join("Save job.pdf");

    let ran = run_save_job(
        &PrintRequest {
            pdf: &pdf,
            paper: Some(A4),
            scaling: Scaling::Actual,
            title: "Save job",
        },
        &to,
    );
    match &ran {
        Ok(()) => check(true, "the print operation ran as a save job", &mut failures),
        Err(error) => check(
            false,
            &format!("the print operation did not run: {error} ({error:?})"),
            &mut failures,
        ),
    }

    if ran.is_ok() {
        match std::fs::read(&to).ok().and_then(|bytes| {
            let size = bytes.len();
            PdfFile::open(&bytes).map(|file| (file, size))
        }) {
            None => check(
                false,
                "the job wrote a PDF to the path it was given",
                &mut failures,
            ),
            Some((file, size)) => {
                println!(
                    "save_job: the job wrote {size} bytes ({} went in)",
                    pdf.len()
                );
                check(
                    file.page_count() == 2,
                    "the saved PDF has two pages",
                    &mut failures,
                );
                for index in 0..file.page_count().min(2) {
                    let (width, height) = file.page_size(index);
                    check(
                        (width - A4.0).abs() < 0.6 && (height - A4.1).abs() < 0.6,
                        &format!(
                            "page {} is the paper's size ({width} x {height})",
                            index + 1
                        ),
                        &mut failures,
                    );
                }
                let text_of = |index: usize| {
                    file.page_text(index)
                        .lines()
                        .map(str::trim)
                        .filter(|line| !line.is_empty())
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                check(
                    text_of(0) == format!("{}\n{}\n1", first[0], first[1]),
                    &format!("the text of page 1 ({:?})", text_of(0)),
                    &mut failures,
                );
                check(
                    text_of(1) == format!("{}\n2", second[0]),
                    &format!("the text of page 2 ({:?})", text_of(1)),
                    &mut failures,
                );
                // The page is on the paper at its real size: the first word
                // of the source (at x 72, its baseline at 100) is exactly as
                // wide and as high in the saved job.
                let source = PdfFile::open(pdf.bytes()).expect("the writer's PDF");
                let before = source.find(0, "The");
                let after = file.find(0, "The");
                check(
                    match (before, after) {
                        (Some(before), Some(after)) => {
                            (before.0 - 72.).abs() < 0.1
                                && (before.2 - after.2).abs() < 0.01
                                && (before.3 - after.3).abs() < 0.01
                        }
                        _ => false,
                    },
                    &format!(
                        "the first word is at real size (in the PDF {before:?}, in the saved \
                         job {after:?})"
                    ),
                    &mut failures,
                );
                // Where it is on the paper is the printer's: PDFKit centres
                // the page in the area the printer can print on, so the page
                // is exactly on the paper only where that area is centred on
                // it (every printer of the author's Mac). macOS's generic
                // printer, which a Mac without printers has (GitHub's
                // runners), prints A4 in (18, 41, 559, 783) of a 595 x 842
                // sheet, 41 points from the bottom and 18 from the top, and
                // the page lands 11.6 points up and 0.14 to the left. So the
                // move is computed from the print settings the job used and
                // must be that, no more.
                let (printer, paper, area) = printable_area(A4);
                let sheet = file.page_size(0);
                // From the top left corner, as `find` measures.
                let expected = (
                    area.x + area.width / 2. - f64::from(A4.0) / 2.,
                    f64::from(sheet.1) - (area.y + area.height / 2.) - f64::from(A4.1) / 2.,
                );
                println!(
                    "save_job: printer {printer:?}, paper {:.2} x {:.2}, printable area x {:.2} \
                     to {:.2} and y {:.2} to {:.2} from the bottom: the page is to move \
                     {:.3} right and {:.3} down",
                    paper.width,
                    paper.height,
                    area.x,
                    area.x + area.width,
                    area.y,
                    area.y + area.height,
                    expected.0,
                    expected.1,
                );
                let moved = before
                    .zip(after)
                    .map(|(before, after)| (after.0 - before.0, after.1 - before.1));
                if let Some(moved) = moved {
                    println!(
                        "save_job: the first word moved {:.3} right and {:.3} down",
                        moved.0, moved.1
                    );
                }
                check(
                    moved.is_some_and(|moved| {
                        (f64::from(moved.0) - expected.0).abs() < 0.05
                            && (f64::from(moved.1) - expected.1).abs() < 0.05
                    }),
                    &format!(
                        "the page moved only as the printer's printable area centres it \
                         (expected {expected:?}, moved {moved:?})"
                    ),
                    &mut failures,
                );
            }
        }
    }
    drawing_down_to_fit(&folder, &mut failures);

    // What is no PDF is refused before AppKit is asked for anything, and so is
    // a sheet without a window: `done` is dropped without having run.
    let nothing = PdfWriter::new(&JobInfo {
        title: String::new(),
        creator: String::new(),
    })
    .finish();
    let refused = run_save_job(
        &PrintRequest {
            pdf: &nothing,
            paper: Some(A4),
            scaling: Scaling::Actual,
            title: "Nothing",
        },
        &folder.join("Nothing.pdf"),
    );
    check(
        refused == Err(PrintError::NotAPdf) && !folder.join("Nothing.pdf").exists(),
        &format!("a PDF without pages is not printed ({refused:?})"),
        &mut failures,
    );
    let ran = Rc::new(Cell::new(false));
    let done = {
        let ran = ran.clone();
        Box::new(move |_| ran.set(true))
    };
    // SAFETY: on the main thread; no window is given, and none is touched.
    let shown = unsafe {
        run_sheet(
            &PrintRequest {
                pdf: &pdf,
                paper: Some(A4),
                scaling: Scaling::Actual,
                title: "Save job",
            },
            std::ptr::null_mut(),
            done,
        )
    };
    check(
        matches!(shown, Err(PrintError::Failed(_))) && !ran.get() && Rc::strong_count(&ran) == 1,
        &format!("no sheet without a window ({shown:?})"),
        &mut failures,
    );

    std::fs::remove_dir_all(&folder).ok();

    if failures.is_empty() {
        println!(
            "save_job: the print operation ran from Rust, without a panel and without a printer"
        );
        ExitCode::SUCCESS
    } else {
        println!("save_job: {} of its checks failed:", failures.len());
        for failure in &failures {
            println!("save_job:   {failure}");
        }
        ExitCode::FAILURE
    }
}
