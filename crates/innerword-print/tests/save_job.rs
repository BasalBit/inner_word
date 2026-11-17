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

use objc::{class, msg_send, runtime::Object, sel, sel_impl};
use innerword_print::{
    Anchor, Color, FontSpec, GlyphStart, Item, JobInfo, Label, Page, PdfFile, PdfWriter,
    PrintError, PrintRequest, RectItem, RowRun, TextRow, check_selectors, run_save_job, run_sheet,
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

fn check(holds: bool, what: &str, failures: &mut Vec<String>) {
    if holds {
        println!("save_job: ok: {what}");
    } else {
        println!("save_job: FAILED: {what}");
        failures.push(what.to_string());
    }
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
            paper: A4,
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
                // The page is on the paper at its real size, where it was: the
                // first word of the source stands at x 72 with its baseline
                // at 100 (Courier New's descent at 12 points is 3.6).
                let source = PdfFile::open(pdf.bytes()).expect("the writer's PDF");
                let before = source.find(0, "The");
                let after = file.find(0, "The");
                let same = match (before, after) {
                    (Some(before), Some(after)) => {
                        (before.0 - after.0).abs() < 0.1
                            && (before.1 - after.1).abs() < 0.1
                            && (after.0 - 72.).abs() < 0.1
                    }
                    _ => false,
                };
                check(
                    same,
                    &format!(
                        "the first word is where it was, at real size (in the PDF {before:?}, \
                         in the saved job {after:?})"
                    ),
                    &mut failures,
                );
            }
        }
    }
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
            paper: A4,
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
                paper: A4,
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
