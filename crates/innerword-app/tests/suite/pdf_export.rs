//! "Export as PDF…" in the assembled app: an exported PDF against the PDF
//! "Print…" makes of the same document, a document that is not open, a
//! folder as one PDF, the note and Cancel, one job of pages at a time,
//! where the command is reached from, and the steps of a scripted run.
//!
//! No test opens the save panel or the print panel: the save panel is the
//! test platform's (`simulate_new_path_selection`), print jobs go to a
//! recorder. The PDFs are read back with `innerword_print::PdfFile` (PDFKit).
//! Glyphs are 0.6 em wide in a test, so the documents are set in Courier
//! New and texts are compared without white space (`print.rs` says why).

use std::{path::PathBuf, time::Duration};

use gpui::{Modifiers, MouseButton, TestAppContext};
use innerword_app::document_io::StoredForm;
use innerword_app::{
    actions::{CancelPdfExport, CancelPrint, DismissNotifications, ExportPdf, Print},
    automation::{EXPORT_WAIT, Script, Step, describe_state},
    export::{
        ExportChoice, ExportDestination, ExportPicker, ExportScope, ExportTarget, FileFormat,
        pages_left_out_sentence,
    },
    menus, pages,
    pdf_export::{on_paper, shows_title},
    printing::{self, PrintJob},
    shortcuts::{Place, collect_shortcuts},
    workspace::{MenuKind, PRINT_NOTE_DELAY, PdfExporting, Printing, pdf_note},
};
use innerword_core::{Alignment, DocumentSettings, Orientation, PageSetup, PaperSize, Typography};
use innerword_export::{ExportSource, MarkdownOptions, to_markdown};
use innerword_print::PdfFile;
use innerword_ui::ContextMenu;

use super::{
    images::{picture, stored_png},
    *,
};

const LONG: &str = "The harbor was a plain grey fact and the boats came by road every spring. ";

/// `blocks` as a document in page view on `page`, set in Courier New.
fn in_courier(blocks: Vec<Block>, page: PageSetup) -> Document {
    Document::from_blocks(blocks).with_settings(DocumentSettings {
        page_view: true,
        page,
        typography: Typography {
            body_font: "Courier New".to_string(),
            ..Typography::default()
        },
        ..DocumentSettings::default()
    })
}

/// A heading and enough paragraphs for several pages.
fn manuscript(title: &str, paragraphs: usize) -> Vec<Block> {
    let mut blocks = vec![Block::with_text(BlockKind::heading(1), title)];
    for index in 0..paragraphs {
        blocks.push(Block::paragraph(&format!(
            "Paragraph {index}. {}",
            LONG.repeat(1 + index % 3).trim_end()
        )));
    }
    blocks
}

/// One-line blocks: a slice of zero length is one of them.
fn lines(count: usize) -> Vec<Block> {
    let mut blocks = vec![Block::with_text(BlockKind::heading(1), "A long list")];
    blocks.extend((1..count).map(|index| Block::paragraph(&format!("Line {index}."))));
    blocks
}

fn squeezed(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

fn exporting(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Option<PdfExporting> {
    workspace.read_with(cx, |w, _| w.pdf_exporting())
}

/// Lets the clock run until the PDF export under way has written its file.
fn let_it_export(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    for _ in 0..20_000 {
        cx.run_until_parked();
        if exporting(workspace, cx).is_none() {
            return;
        }
        cx.executor().advance_clock(Duration::from_millis(1));
    }
    panic!("the PDF was never written");
}

/// Answers the save panel with `path` and lets the export run to its end.
fn choose_and_export(path: PathBuf, workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    cx.run_until_parked();
    assert!(
        cx.did_prompt_for_new_path(),
        "the save panel is asked first"
    );
    // Nothing is made before a file is chosen.
    assert_eq!(exporting(workspace, cx), None);
    cx.simulate_new_path_selection(move |_| Some(path));
    let_it_export(workspace, cx);
}

fn toasts(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<String> {
    workspace.read_with(cx, |workspace, cx| {
        workspace
            .toast_layer()
            .read(cx)
            .toasts()
            .map(|toast| toast.message.to_string())
            .collect()
    })
}

fn available(action: &dyn gpui::Action, cx: &mut VisualTestContext) -> bool {
    cx.run_until_parked();
    cx.update(|window, cx| window.is_action_available(action, cx))
}

fn state_line(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> String {
    workspace.update_in(cx, |workspace, window, cx| {
        describe_state(workspace, window, cx)
    })
}

fn print_job(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> PrintJob {
    workspace
        .update_in(cx, |w, window, cx| w.print_job(window, cx))
        .expect("a document is open")
}

/// The text of every page of a PDF, without white space.
fn page_texts(file: &PdfFile) -> Vec<String> {
    (0..file.page_count())
        .map(|page| squeezed(&file.page_text(page)))
        .collect()
}

/// Holds two PDFs to the same paper: page count, the size, the text and
/// the pixels of every page, and the title they carry.
fn assert_same_paper(exported: &[u8], printed: &PrintJob) {
    let exported = PdfFile::open(exported).expect("the export is a PDF");
    let printed = PdfFile::open(printed.pdf.bytes()).expect("the print is a PDF");
    assert_eq!(exported.page_count(), printed.page_count(), "page count");
    for page in 0..printed.page_count() {
        assert_eq!(
            exported.page_size(page),
            printed.page_size(page),
            "page {page}"
        );
        assert_eq!(
            exported.page_text(page),
            printed.page_text(page),
            "the text of page {page}"
        );
        assert!(
            exported
                .page_pixels(page, 1.)
                .expect("the export's page as pixels")
                == printed
                    .page_pixels(page, 1.)
                    .expect("the print's page as pixels"),
            "the pixels of page {page}"
        );
    }
    assert_eq!(exported.title_and_creator(), printed.title_and_creator());
}

/// A workspace whose print jobs are recorded, with a document open: a
/// heading, a picture of the library, paragraphs over several pages.
fn open_illustrated(
    cx: &mut TestAppContext,
) -> (Entity<Workspace>, DocumentId, &mut VisualTestContext) {
    init(cx);
    cx.update(printing::record);
    let library = library(cx);
    let image = stored_png(&library, 800, 500, [60, 160, 90], cx);
    let mut blocks = manuscript("The harbor", 40);
    blocks.insert(
        1,
        picture(&image, 75, Alignment::Center, "The harbour at dusk"),
    );
    blocks.push(picture(&image, 25, Alignment::Right, ""));
    let id = add_document(
        &library,
        None,
        &in_courier(blocks, PageSetup::default()),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    workspace.update_in(vcx, |w, window, cx| {
        assert!(w.open_document(id, true, window, cx));
    });
    vcx.run_until_parked();
    (workspace, id, vcx)
}

// ----- the same paper as Print ------------------------------------------------------------

/// The picker's "PDF file", chosen with the keys, writes the PDF "Print…"
/// makes of the open document: the same pages, the same text, the same
/// picture, unsaved text included.
#[gpui::test]
fn an_exported_pdf_is_the_pdf_print_makes(cx: &mut TestAppContext) {
    let (workspace, _, cx) = open_illustrated(cx);
    // Typed and not saved yet.
    type_text(&workspace, "Unsaved words. ", cx);
    let out = tempfile::tempdir().unwrap();

    cx.simulate_keystrokes("cmd-shift-s");
    cx.simulate_input("pdf");
    let picker = workspace
        .read_with(cx, |w, cx| {
            w.modal_layer().read(cx).active::<ExportPicker>()
        })
        .expect("the export picker is open");
    let choices = picker.read_with(cx, |picker, _| picker.delegate.choices());
    assert_eq!(
        choices,
        [
            ExportChoice::new(
                ExportScope::Document,
                FileFormat::Pdf,
                ExportDestination::File
            ),
            ExportChoice::new(
                ExportScope::Folder,
                FileFormat::Pdf,
                ExportDestination::File
            ),
        ]
    );
    cx.simulate_keystrokes("enter");
    choose_and_export(out.path().join("chosen"), &workspace, cx);

    // The extension is added to a name typed without it.
    let written = out.path().join("chosen.pdf");
    let bytes = std::fs::read(&written).expect("the PDF is written");
    let printed = print_job(&workspace, cx);
    assert!(printed.pdf.report.pages >= 3, "{:?}", printed.pdf.report);
    assert_eq!(printed.pdf.report.images, 1);
    assert_eq!(printed.pdf.report.rows_off, 0);
    assert_same_paper(&bytes, &printed);
    let file = PdfFile::open(&bytes).unwrap();
    assert!(
        page_texts(&file)[0].starts_with("Unsavedwords.Theharbor"),
        "{:?}",
        page_texts(&file)[0]
    );

    let said = toasts(&workspace, cx);
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(
        said[0].starts_with("Exported “Unsaved words. The harbor” to ")
            && said[0].ends_with("chosen.pdf"),
        "{said:?}"
    );
    assert!(editor_focused(&workspace, cx));
    // The save panel starts there next time.
    let library = workspace.read_with(cx, |w, _| w.library().clone());
    let stored: String = library.read_with(cx, |library, _| {
        library.setting_or(
            innerword_app::settings::keys::EXPORT_DIRECTORY,
            String::new(),
        )
    });
    assert_eq!(PathBuf::from(stored), out.path());
    // Nothing went to the print target.
    assert_eq!(workspace.read_with(cx, |w, _| w.printing()), None);
}

/// A document of the library that is not open ("Export…" on its row) is
/// read from the library and laid out as "Print…" lays it out once it is
/// open; the open document stays open.
#[gpui::test]
fn a_document_that_is_not_open_is_laid_out_as_print_lays_it_out(cx: &mut TestAppContext) {
    init(cx);
    cx.update(printing::record);
    let library = library(cx);
    let open = add_document(&library, None, &simple_document("Open", "Here."), cx);
    let letter = PageSetup {
        paper: PaperSize::UsLetter,
        orientation: Orientation::Landscape,
        ..PageSetup::default()
    };
    let other = add_document(
        &library,
        None,
        &in_courier(manuscript("Elsewhere", 30), letter),
        cx,
    );
    let (workspace, cx) = open_workspace(&library, cx);
    workspace.update_in(cx, |w, window, cx| {
        assert!(w.open_document(open, true, window, cx));
    });
    cx.run_until_parked();

    // The row's "Export…": the picker for that document.
    workspace.update_in(cx, |w, window, cx| {
        w.show_export(ExportTarget::Document(other), window, cx)
    });
    cx.simulate_input("pdf");
    cx.simulate_keystrokes("enter");
    let out = tempfile::tempdir().unwrap();
    choose_and_export(out.path().join("Elsewhere.pdf"), &workspace, cx);
    assert_eq!(
        workspace.read_with(cx, |w, _| w.open_document_id()),
        Some(open)
    );
    let bytes = std::fs::read(out.path().join("Elsewhere.pdf")).expect("written");

    workspace.update_in(cx, |w, window, cx| {
        assert!(w.open_document(other, true, window, cx));
    });
    cx.run_until_parked();
    let printed = print_job(&workspace, cx);
    assert!(printed.pdf.report.pages >= 2);
    assert_eq!(printed.paper, (792., 612.));
    assert_same_paper(&bytes, &printed);
}

// ----- a folder ---------------------------------------------------------------------------

/// A folder as one PDF: each document on its own paper and on a new page,
/// its title above it where it does not show it, the page numbers going
/// on, a page of the folder left out and said so.
#[gpui::test]
fn a_folder_is_one_pdf_with_each_document_on_its_own_paper(cx: &mut TestAppContext) {
    init(cx);
    cx.update(printing::record);
    let _fake = cx.update(pages::use_fake);
    let library = library(cx);
    let folder = library.update(cx, |library, cx| {
        library.create_folder(None, "Voyage", cx).unwrap().id
    });
    // Shows its title: nothing is added, and its page is its print alone.
    let first = add_document(
        &library,
        Some(folder),
        &in_courier(
            vec![
                Block::with_text(BlockKind::heading(1), "Chapter 1"),
                Block::paragraph("The ferry came in late."),
                Block::new(BlockKind::PageBreak),
            ],
            PageSetup::default(),
        ),
        cx,
    );
    // Its title (a name the writer gave it) is not in its text, and it is
    // on Letter, landscape.
    let named = StoredForm::of(
        &in_courier(
            vec![Block::paragraph("By morning the fog had gone.")],
            PageSetup {
                paper: PaperSize::UsLetter,
                orientation: Orientation::Landscape,
                ..PageSetup::default()
            },
        ),
        Some("Chapter 2"),
    );
    library.update(cx, |library, cx| {
        library
            .create_document(Some(folder), &named.content(), cx)
            .expect("create document")
    });
    // A page break at its start (left out: a new document is a new page)
    // and one at its end (kept: it is the last).
    add_document(
        &library,
        Some(folder),
        &in_courier(
            vec![
                Block::new(BlockKind::PageBreak),
                Block::with_text(BlockKind::heading(1), "Chapter 3"),
                Block::paragraph("Home again."),
                Block::new(BlockKind::PageBreak),
            ],
            PageSetup::default(),
        ),
        cx,
    );
    let bundle = pages::offline_bundle(
        "harbour",
        "index.html",
        vec![(
            "index.html".to_string(),
            b"<!doctype html><title>Harbour</title><p>A page.</p>".to_vec(),
        )],
    );
    let stored_library = library.clone();
    cx.update(|cx| {
        pages::store_offline_bundle(&stored_library, bundle, Some(folder), None, cx)
            .expect("the page is stored")
    });
    let (workspace, cx) = open_workspace(&library, cx);
    workspace.update_in(cx, |w, window, cx| {
        assert!(w.open_document(first, true, window, cx));
    });
    cx.run_until_parked();
    let alone = print_job(&workspace, cx);

    // "Export Folder…" on its row: the picker for the folder only.
    workspace.update_in(cx, |w, window, cx| {
        w.show_export(ExportTarget::Folder(folder), window, cx)
    });
    let picker = workspace
        .read_with(cx, |w, cx| {
            w.modal_layer().read(cx).active::<ExportPicker>()
        })
        .unwrap();
    let choices = picker.read_with(cx, |picker, _| picker.delegate.choices());
    assert_eq!(
        choices.last(),
        Some(&ExportChoice::new(
            ExportScope::Folder,
            FileFormat::Pdf,
            ExportDestination::File
        ))
    );
    cx.simulate_input("pdf");
    cx.simulate_keystrokes("enter");
    let out = tempfile::tempdir().unwrap();
    choose_and_export(out.path().join("Voyage.pdf"), &workspace, cx);
    let bytes = std::fs::read(out.path().join("Voyage.pdf")).expect("written");
    let file = PdfFile::open(&bytes).unwrap();

    // Chapter 1 on one sheet (the break at the end of a document before the
    // last is left out), Chapter 2 on Letter under the title it was given,
    // Chapter 3 (its break at the start left out) and the empty sheet its
    // last break makes; numbered 1 to 4.
    assert_eq!(
        page_texts(&file),
        [
            "Chapter1Theferrycameinlate.1",
            "Chapter2Bymorningthefoghadgone.2",
            "Chapter3Homeagain.3",
            "4"
        ]
    );
    let a4 = PageSetup::default().page_size();
    let sizes: Vec<(f32, f32)> = (0..4).map(|page| file.page_size(page)).collect();
    let close = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01;
    assert!(
        close(sizes[0], a4) && close(sizes[2], a4) && close(sizes[3], a4),
        "{sizes:?}"
    );
    assert!(close(sizes[1], (792., 612.)), "{sizes:?}");
    // The first document is the page it is when printed alone (its own
    // break at the end aside, which makes a second sheet there).
    let printed = PdfFile::open(alone.pdf.bytes()).unwrap();
    assert_eq!(printed.page_count(), 2);
    assert_eq!(file.page_text(0), printed.page_text(0));
    // Every word where it stands there. (Not the pixels: the subset of
    // Courier New in this PDF holds the glyphs of the other documents too,
    // and PDFKit smooths the edges of a few glyphs of the row otherwise.)
    for word in ["Chapter", "ferry", "late.", "1"] {
        assert_eq!(file.find(0, word), printed.find(0, word), "{word}");
        assert!(file.find(0, word).is_some(), "{word}");
    }
    assert_eq!(file.title_and_creator().0, "Voyage");

    let said = toasts(&workspace, cx);
    assert!(
        said.iter()
            .any(|toast| toast.starts_with("Exported 3 documents to ")
                && toast.ends_with(&pages_left_out_sentence(1))),
        "{said:?}"
    );
}

/// The documents of a folder as they go on paper: the title above a
/// document that does not show it, by the rule the other formats of a
/// folder follow (here held against the Markdown export), and the page
/// breaks a new document makes needless left out.
#[test]
fn a_folders_documents_get_their_titles_as_the_other_formats_give_them() {
    let cases: Vec<(&str, Document)> = vec![
        ("Chapter 1", simple_document("Chapter 1", "Text.")),
        (
            "Chapter 2",
            Document::from_blocks(vec![Block::paragraph("Chapter 2 begins on a quay.")]),
        ),
        (
            "Notes",
            Document::from_blocks(vec![
                Block::paragraph("Some words first."),
                Block::with_text(BlockKind::heading(2), "notes and more"),
            ]),
        ),
        (
            "Harbour",
            Document::from_blocks(vec![Block::paragraph("The boats.")]),
        ),
        (
            "  Spaced   title ",
            Document::from_blocks(vec![Block::with_text(
                BlockKind::heading(3),
                "Spaced title",
            )]),
        ),
        (
            "",
            Document::from_blocks(vec![Block::paragraph("Nameless.")]),
        ),
        (
            "Second line",
            Document::from_blocks(vec![
                Block::paragraph("First words."),
                Block::paragraph("Second line here."),
            ]),
        ),
    ];
    for (title, document) in &cases {
        let clean = title.split_whitespace().collect::<Vec<_>>().join(" ");
        let clean = if clean.is_empty() {
            "Untitled".to_string()
        } else {
            clean
        };
        let heading = format!("# {clean}");
        let count = |markdown: &str| markdown.lines().filter(|line| *line == heading).count();
        let alone = to_markdown(
            &ExportSource::single(*title, document),
            &MarkdownOptions::default(),
        );
        let in_folder = to_markdown(
            &ExportSource::collection("Folder", [(*title, document)]),
            &MarkdownOptions::default(),
        );
        let markdown_adds = count(&in_folder) - count(&alone) == 1;
        assert_eq!(
            !shows_title(document, title),
            markdown_adds,
            "{title:?}: {in_folder}"
        );
        // As it goes on paper (alone in its folder: first and last).
        let paper = on_paper(vec![(title.to_string(), document.clone())]);
        assert_eq!(
            paper[0].block_count(),
            document.block_count() + usize::from(markdown_adds),
            "{title:?}"
        );
        if markdown_adds {
            let first = &paper[0].blocks()[0];
            assert_eq!(first.kind(), &BlockKind::heading(1));
            assert_eq!(first.text(), clean);
            assert_eq!(&paper[0].blocks()[1..], document.blocks());
        } else {
            assert_eq!(&paper[0], document);
        }
    }

    // The breaks: kept at the start of the first and the end of the last.
    let breaks = |title: &str| {
        Document::from_blocks(vec![
            Block::new(BlockKind::PageBreak),
            Block::with_text(BlockKind::heading(1), title),
            Block::new(BlockKind::PageBreak),
        ])
    };
    let paper = on_paper(vec![
        ("One".to_string(), breaks("One")),
        ("Two".to_string(), breaks("Two")),
        ("Three".to_string(), breaks("Three")),
    ]);
    let kinds = |document: &Document| {
        document
            .blocks()
            .iter()
            .map(|block| matches!(block.kind(), BlockKind::PageBreak))
            .collect::<Vec<_>>()
    };
    assert_eq!(kinds(&paper[0]), [true, false]);
    assert_eq!(kinds(&paper[1]), [false]);
    assert_eq!(kinds(&paper[2]), [false, true]);
    // The settings of each document are its own.
    let landscape = Document::from_blocks(vec![
        Block::new(BlockKind::PageBreak),
        Block::paragraph("Wide."),
    ])
    .with_settings(DocumentSettings {
        page: PageSetup {
            orientation: Orientation::Landscape,
            ..PageSetup::default()
        },
        ..DocumentSettings::default()
    });
    let paper = on_paper(vec![
        ("A".to_string(), simple_document("A", "a")),
        ("Wide".to_string(), landscape.clone()),
    ]);
    assert_eq!(paper[1].settings(), landscape.settings());
}

// ----- slices, the note, Cancel, one job at a time ----------------------------------------

/// A book takes a while: its pages are made in slices, after 0.4 s a note
/// says so, and "Cancel PDF export" stops it (nothing is written). While a
/// PDF is being made a print says so and does nothing, and the other way
/// round.
#[gpui::test]
fn a_long_export_says_so_can_be_cancelled_and_is_the_only_job(cx: &mut TestAppContext) {
    init(cx);
    let jobs = cx.update(printing::record);
    let library = library(cx);
    let id = add_document(
        &library,
        None,
        &in_courier(lines(1000), PageSetup::default()),
        cx,
    );
    let (workspace, cx) = open_workspace(&library, cx);
    workspace.update_in(cx, |w, window, cx| {
        assert!(w.open_document(id, true, window, cx));
    });
    cx.run_until_parked();
    // A slice is one block and the slices are a millisecond of the test
    // clock apart.
    cx.update(|_, cx| printing::set_slice(Duration::ZERO, cx));
    let out = tempfile::tempdir().unwrap();
    let path = out.path().join("A long list.pdf");
    let note = pdf_note("A long list");
    assert_eq!(note, "Preparing the PDF of “A long list”…");
    let note_button = |cx: &mut VisualTestContext| {
        workspace.read_with(cx, |w, cx| {
            w.toast_layer()
                .read(cx)
                .toasts()
                .find(|toast| toast.message.as_ref() == note)
                .map(|toast| toast.action_label().map(|label| label.to_string()))
        })
    };

    // The command: the save panel first, then the pages in slices.
    assert!(!available(&CancelPdfExport, cx));
    cx.dispatch_action(ExportPdf);
    cx.run_until_parked();
    let chosen = path.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    assert_eq!(exporting(&workspace, cx), Some(PdfExporting::Preparing));
    assert!(available(&CancelPdfExport, cx));
    assert!(!available(&CancelPrint, cx));
    let line = state_line(&workspace, cx);
    assert!(line.contains(" print=[idle] export=[preparing "), "{line}");
    assert_eq!(note_button(cx), None, "no note before the delay");
    cx.executor().advance_clock(PRINT_NOTE_DELAY);
    cx.run_until_parked();
    assert_eq!(note_button(cx), Some(Some("Cancel".to_string())));
    let line = state_line(&workspace, cx);
    assert!(
        !line.contains("export=[preparing 0%]"),
        "it goes on: {line}"
    );

    // A print while the PDF is made: it says so, and nothing is printed.
    cx.dispatch_action(Print);
    cx.run_until_parked();
    assert!(
        toasts(&workspace, cx)
            .contains(&"The PDF of “A long list” is still being made.".to_string())
    );
    assert_eq!(workspace.read_with(cx, |w, _| w.printing()), None);
    // A second export says so too, without asking where.
    cx.dispatch_action(ExportPdf);
    cx.run_until_parked();
    assert!(!cx.did_prompt_for_new_path());

    // Cancel: nothing is written, the note goes.
    cx.dispatch_action(CancelPdfExport);
    cx.run_until_parked();
    assert_eq!(exporting(&workspace, cx), None);
    assert_eq!(note_button(cx), None);
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    assert!(!path.exists(), "a cancelled export writes nothing");
    assert!(state_line(&workspace, cx).contains(" export=[idle] "));
    assert_eq!(
        workspace.update(cx, |w, _| w.take_pdf_export_report()),
        Some(Err("cancelled".to_string()))
    );

    // "Dismiss notifications" cancels too.
    cx.dispatch_action(ExportPdf);
    cx.run_until_parked();
    let chosen = path.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.executor().advance_clock(PRINT_NOTE_DELAY);
    cx.run_until_parked();
    assert_eq!(note_button(cx), Some(Some("Cancel".to_string())));
    cx.dispatch_action(DismissNotifications);
    cx.run_until_parked();
    assert_eq!(exporting(&workspace, cx), None);
    assert!(!path.exists());

    // Let one run to its end: the file, and a toast.
    cx.dispatch_action(ExportPdf);
    choose_and_export(path.clone(), &workspace, cx);
    let file = PdfFile::open(&std::fs::read(&path).expect("written")).unwrap();
    assert!(file.page_count() > 10);
    let report = workspace
        .update(cx, |w, _| w.take_pdf_export_report())
        .expect("a report")
        .expect("written");
    assert!(report.starts_with("“A long list” written to "), "{report}");
    assert!(report.contains(" 0 rows off, "), "{report}");
    assert!(
        toasts(&workspace, cx)
            .iter()
            .any(|toast| toast.starts_with("Exported “A long list” to "))
    );

    // A print under way: an export says so, and asks nothing.
    cx.dispatch_action(Print);
    cx.run_until_parked();
    assert_eq!(
        workspace.read_with(cx, |w, _| w.printing()),
        Some(Printing::Preparing)
    );
    cx.dispatch_action(ExportPdf);
    cx.run_until_parked();
    assert!(!cx.did_prompt_for_new_path());
    assert!(
        toasts(&workspace, cx)
            .contains(&"“A long list” is still being prepared to print.".to_string())
    );
    assert!(!available(&CancelPdfExport, cx));
    cx.dispatch_action(CancelPrint);
    cx.run_until_parked();
    assert!(jobs.borrow().is_empty());
}

/// A cancelled save panel makes no page.
#[gpui::test]
fn a_cancelled_save_panel_makes_nothing(cx: &mut TestAppContext) {
    let (workspace, _, cx) = open_illustrated(cx);
    cx.dispatch_action(ExportPdf);
    cx.run_until_parked();
    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
    assert_eq!(exporting(&workspace, cx), None);
    assert!(toasts(&workspace, cx).is_empty());
    assert_eq!(
        workspace.update(cx, |w, _| w.take_pdf_export_report()),
        None
    );
}

// ----- where it is reached from -----------------------------------------------------------

/// "Export as PDF…" is in File, the menu of the title and the palette,
/// without a key (a row "Anywhere" of the shortcuts, with "Cancel PDF
/// export"); without a document it has no handler.
#[gpui::test]
fn export_as_pdf_is_in_the_menu_bar_the_title_menu_and_the_palette(cx: &mut TestAppContext) {
    let (workspace, _, cx) = open_illustrated(cx);
    let out = tempfile::tempdir().unwrap();

    // File > Export as PDF…, with no key beside it.
    let file: Vec<(String, Box<dyn gpui::Action>)> = menus::app_menus()
        .into_iter()
        .filter(|menu| menu.name.as_ref() == "File")
        .flat_map(|menu| menus::menu_actions(&[menu]))
        .collect();
    let item = file
        .iter()
        .find(|(name, _)| name == "Export as PDF…")
        .map(|(_, action)| action.boxed_clone())
        .expect("File has Export as PDF…");
    cx.update(|_, cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        assert!(menus::native_menu_binding(&keymap, &ExportPdf).is_none());
        assert!(menus::native_menu_binding(&keymap, &CancelPdfExport).is_none());
        assert_eq!(keymap.bindings_for_action(&ExportPdf).count(), 0);
    });
    assert!(available(&ExportPdf, cx));
    assert!(!available(&CancelPdfExport, cx));
    cx.update(|window, cx| window.dispatch_action(item, cx));
    choose_and_export(out.path().join("menu.pdf"), &workspace, cx);
    assert!(out.path().join("menu.pdf").exists());

    // The menu of the title, with the pointer.
    let at = cx
        .debug_bounds(MenuKind::Title.selector())
        .expect("the title is drawn")
        .center();
    cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::default());
    cx.run_until_parked();
    let at = cx.update(|window, cx| {
        let menu = ContextMenu::current(window, cx).expect("the title's menu");
        let menu = menu.read(cx);
        let labels = menu.labels();
        let export = labels.iter().position(|l| l == "Export…").unwrap();
        assert_eq!(labels[export + 1], "Export as PDF…");
        menu.item_bounds(export + 1).unwrap().center()
    });
    cx.simulate_click(at, Modifiers::default());
    choose_and_export(out.path().join("title.pdf"), &workspace, cx);
    assert!(out.path().join("title.pdf").exists());

    // The palette.
    cx.simulate_keystrokes("cmd-shift-p");
    cx.simulate_input("export as pdf");
    cx.simulate_keystrokes("enter");
    choose_and_export(out.path().join("palette.pdf"), &workspace, cx);
    assert!(out.path().join("palette.pdf").exists());
    assert!(editor_focused(&workspace, cx));
    cx.update(|_, cx| {
        let label = |name: &str| {
            innerword_app::commands::all(cx)
                .into_iter()
                .find(|command| command.action_name() == name)
                .map(|command| command.label())
                .unwrap_or_else(|| panic!("{name} is a command"))
        };
        assert_eq!(label("workspace::ExportPdf"), "File: Export as PDF…");
        assert_eq!(
            label("workspace::CancelPdfExport"),
            "File: Cancel PDF export"
        );
    });

    // Two rows of the shortcuts, "Anywhere", without keys.
    let rows = cx.update(|_, cx| {
        collect_shortcuts(cx)
            .into_iter()
            .filter(|row| {
                row.title.as_ref() == "Export as PDF…" || row.title.as_ref() == "Cancel PDF export"
            })
            .map(|row| (row.title.to_string(), row.place, row.all_keys.len()))
            .collect::<Vec<_>>()
    });
    assert_eq!(
        rows,
        [
            ("Export as PDF…".to_string(), Place::Anywhere, 0),
            ("Cancel PDF export".to_string(), Place::Anywhere, 0),
        ]
    );
    // Each of the three made the same paper.
    let read = |name: &str| std::fs::read(out.path().join(name)).unwrap();
    assert_eq!(
        page_texts(&PdfFile::open(&read("menu.pdf")).unwrap()),
        page_texts(&PdfFile::open(&read("palette.pdf")).unwrap())
    );
    assert_eq!(
        page_texts(&PdfFile::open(&read("title.pdf")).unwrap()),
        page_texts(&PdfFile::open(&read("palette.pdf")).unwrap())
    );
}

#[gpui::test]
fn without_a_document_there_is_no_export_as_pdf(cx: &mut TestAppContext) {
    init(cx);
    cx.update(printing::record);
    let library = library(cx);
    let (workspace, cx) = open_workspace(&library, cx);
    assert_eq!(workspace.read_with(cx, |w, _| w.open_document_id()), None);
    assert!(!available(&ExportPdf, cx));
    assert!(!workspace.read_with(cx, |w, cx| w.can_export_pdf(cx)));
    cx.dispatch_action(ExportPdf);
    cx.run_until_parked();
    assert!(!cx.did_prompt_for_new_path());
    assert!(state_line(&workspace, cx).contains(" export=[idle] "));
}

/// With a page open, "Export as PDF…" is the page's own PDF, as WebKit
/// prints it (the fake engine writes its fixed page).
#[gpui::test]
fn on_a_page_export_as_pdf_is_the_pages_own_pdf(cx: &mut TestAppContext) {
    init(cx);
    let fake = cx.update(pages::use_fake);
    let library = library(cx);
    let bundle = pages::offline_bundle(
        "harbour",
        "index.html",
        vec![(
            "index.html".to_string(),
            b"<!doctype html><title>Harbour Guide</title><p>The boats.</p>".to_vec(),
        )],
    );
    let stored_library = library.clone();
    let page = cx.update(|cx| {
        pages::store_offline_bundle(&stored_library, bundle, None, None, cx)
            .expect("the page is stored")
            .id
    });
    let (workspace, cx) = open_workspace(&library, cx);
    workspace.update_in(cx, |w, window, cx| {
        w.open_document(page, true, window, cx);
    });
    cx.run_until_parked();
    fake.finish_loading();
    assert!(available(&ExportPdf, cx));
    cx.dispatch_action(ExportPdf);
    cx.run_until_parked();
    let out = tempfile::tempdir().unwrap();
    let path = out.path().join("Harbour Guide.pdf");
    let chosen = path.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    assert_eq!(std::fs::read(&path).unwrap(), pages::fake_pdf());
    assert_eq!(exporting(&workspace, cx), None, "not a job of pages");
}

// ----- scripted runs ----------------------------------------------------------------------

#[gpui::test]
fn the_export_step_of_a_script_parses_and_the_state_says_idle(cx: &mut TestAppContext) {
    let script = Script::parse(
        "keys:cmd-shift-s; type:pdf; keys:enter; export:wait; export:wait:2500; \
         action:workspace::ExportPdf; action:workspace::CancelPdfExport",
    )
    .unwrap();
    assert_eq!(script.steps[3], Step::ExportWait(EXPORT_WAIT));
    assert_eq!(
        script.steps[4],
        Step::ExportWait(Duration::from_millis(2500))
    );
    assert_eq!(
        script.steps[5],
        Step::Action("workspace::ExportPdf".to_string())
    );
    assert!(Script::parse("export").is_err());
    assert!(Script::parse("export:go").is_err());
    assert!(Script::parse("export:wait:soon").is_err());

    let (workspace, _, cx) = open_illustrated(cx);
    let line = state_line(&workspace, cx);
    assert!(line.contains(" print=[idle] export=[idle] "), "{line}");
}
