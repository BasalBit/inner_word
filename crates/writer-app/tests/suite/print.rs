//! "Print…" in the assembled app: the pages a job holds against the pages
//! page view shows, what of the window is not on paper, where the command
//! is reached from, when it is there, Cancel, and the keys.
//!
//! No test opens the print panel: every job goes to a recorder
//! (`printing::record`), installed before the workspace opens. The PDF of
//! a job is read back with `writer_print::PdfFile` (PDFKit).
//!
//! Glyphs are 0.6 em wide in a test (GPUI's test text system), so a PDF
//! reads back as its words only where the font is that wide: the documents
//! here are set in Courier New, and texts are compared without white
//! space. The print pass itself (rows, decorations, glyph places) is held
//! by the editor's own tests (`crates/writer-editor/src/tests/print.rs`);
//! here it is the assembly.

use std::{cell::RefCell, rc::Rc, time::Duration};

use gpui::{Modifiers, MouseButton, TestAppContext};
use writer_app::{
    actions::{CancelPrint, DismissNotifications, Print},
    automation::{PrintStep, Script, Step, describe_state},
    menus,
    printing::{self, PrintJob},
    shortcuts::{Place, collect_shortcuts},
    spelling,
    workspace::{MenuKind, PRINT_NOTE_DELAY, Printing, preparing_note, print_layout_share},
};
use writer_core::{
    DocumentSettings, Margins, Orientation, PageSetup, PaperSize, Position, Selection, Typography,
};
use writer_editor::{Autoscroll, Editor, Zoom};
use writer_keymap::{KeySequence, Target};
use writer_print::PdfFile;
use writer_spell::FakeSpellService;
use writer_ui::{ContextMenu, ThemeMode, Toast};

use super::*;

const LONG: &str = "The harbor was a plain grey fact and the boats came by road every spring. ";

/// `blocks` as a document in page view, set in Courier New.
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

/// A heading, a paragraph with a misspelled word, and enough paragraphs
/// for several pages.
fn manuscript(paragraphs: usize) -> Vec<Block> {
    let mut blocks = vec![
        Block::with_text(BlockKind::heading(1), "The harbor"),
        Block::paragraph("She bought bread and salt and teh ropes."),
    ];
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

type Jobs = Rc<RefCell<Vec<PrintJob>>>;

/// A workspace whose print jobs are recorded, with `document` open and
/// the keyboard in its text.
fn open_recording<'a>(
    document: &Document,
    cx: &'a mut TestAppContext,
) -> (Jobs, Entity<Workspace>, &'a mut VisualTestContext) {
    init(cx);
    let jobs = cx.update(printing::record);
    let library = library(cx);
    let id = add_document(&library, None, document, cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(id, true, window, cx);
    });
    vcx.run_until_parked();
    assert!(editor_focused(&workspace, vcx));
    (jobs, workspace, vcx)
}

fn editor_of(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Entity<Editor> {
    workspace.read_with(cx, |w, _| w.editor().clone())
}

/// Lets the editor's background layout run until every block is measured:
/// until then its page count rests on estimates for what was never on
/// screen.
fn settle(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    let editor = editor_of(workspace, cx);
    for _ in 0..5000 {
        if !editor.read_with(cx, |editor, _| editor.layout_pending()) {
            return;
        }
        cx.executor().advance_clock(Duration::from_millis(2));
        cx.run_until_parked();
    }
    panic!("the background layout did not settle");
}

fn stage(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Option<Printing> {
    workspace.read_with(cx, |w, _| w.printing())
}

/// Lets the clock run until the job that is preparing is done.
fn let_it_prepare(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    for _ in 0..20_000 {
        cx.run_until_parked();
        if stage(workspace, cx) != Some(Printing::Preparing) {
            return;
        }
        cx.executor().advance_clock(Duration::from_millis(1));
    }
    panic!("the pages were never done");
}

/// "Print…" as its key or its menu item runs it, to the end.
fn print(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    cx.dispatch_action(Print);
    let_it_prepare(workspace, cx);
}

fn state_line(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> String {
    workspace.update_in(cx, |workspace, window, cx| {
        describe_state(workspace, window, cx)
    })
}

fn available(action: &dyn gpui::Action, cx: &mut VisualTestContext) -> bool {
    cx.run_until_parked();
    cx.update(|window, cx| window.is_action_available(action, cx))
}

fn squeezed(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// The text of every page of a job's PDF, without white space.
fn page_texts(job: &PrintJob) -> Vec<String> {
    let file = PdfFile::open(job.pdf.bytes()).expect("the job holds a PDF");
    (0..file.page_count())
        .map(|page| squeezed(&file.page_text(page)))
        .collect()
}

/// The text page view has on each page, without white space, read through
/// the editor's public seams independently of the print pass: every
/// visual row of every block, and the page a caret at its start is on.
fn text_page_view_has(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<String> {
    let editor = editor_of(workspace, cx);
    editor.update(cx, |editor, cx| {
        assert!(editor.page_view() && !editor.layout_pending());
        let before = editor.selection();
        let pages = editor.page_count().expect("page view");
        let mut text = vec![String::new(); pages];
        for block in 0..editor.document().block_count() {
            let length = editor.document().blocks()[block].len();
            let mut at = Position::new(block, 0);
            loop {
                let row = editor.visual_row(at).expect("laid out");
                editor.edit_with(Autoscroll::None, |state| state.set_caret(row.start), cx);
                let page = editor.caret_page().expect("page view") - 1;
                let whole = editor.document().blocks()[block].text();
                text[page].push_str(&squeezed(&whole[row.start.offset..row.end.offset]));
                let next = if row.end.offset < length && whole.as_bytes()[row.end.offset] == b'\n' {
                    row.end.offset + 1
                } else {
                    row.end.offset
                };
                if next >= length || next <= row.start.offset {
                    break;
                }
                at = Position::new(block, next);
            }
        }
        editor.edit_with(Autoscroll::None, |state| state.set_selection(before), cx);
        text
    })
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
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

fn menu_labels(cx: &mut VisualTestContext) -> Option<Vec<String>> {
    cx.update(|window, cx| ContextMenu::current(window, cx).map(|menu| menu.read(cx).labels()))
}

fn right_click_the_title(cx: &mut VisualTestContext) {
    let at = cx
        .debug_bounds(MenuKind::Title.selector())
        .expect("the title is drawn")
        .center();
    cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::default());
    cx.run_until_parked();
}

// ----- the pages --------------------------------------------------------------------------

/// The recorded PDF has the pages page view has, on the document's paper,
/// each with the text page view has on it and its number.
#[gpui::test]
fn the_pdf_has_the_pages_and_the_text_page_view_has(cx: &mut TestAppContext) {
    let document = in_courier(manuscript(300), PageSetup::default());
    let (jobs, workspace, cx) = open_recording(&document, cx);
    // The comparison waits for the editor: before its background layout
    // has measured every block, its page count is a guess.
    settle(&workspace, cx);
    print(&workspace, cx);
    assert_eq!(stage(&workspace, cx), None);
    let jobs = jobs.borrow();
    assert_eq!(jobs.len(), 1);
    let job = &jobs[0];

    let editor = editor_of(&workspace, cx);
    let pages = editor.read_with(cx, |editor, _| editor.page_count().unwrap());
    assert!(pages > 5, "{pages} pages");
    let paper = document.settings().page.page_size();
    assert_eq!(job.paper, paper);
    assert_eq!(job.pdf.report.pages, pages);
    let file = PdfFile::open(job.pdf.bytes()).expect("a PDF");
    assert_eq!(file.page_count(), pages);
    for page in 0..pages {
        let (width, height) = file.page_size(page);
        assert!(close(width, paper.0) && close(height, paper.1));
    }
    let (title, creator) = file.title_and_creator();
    assert_eq!(title, "The harbor");
    assert_eq!(creator, format!("The Writer {}", env!("CARGO_PKG_VERSION")));

    let wanted = text_page_view_has(&workspace, cx);
    let found = page_texts(job);
    assert_eq!(found.len(), wanted.len());
    for (number, (found, wanted)) in found.iter().zip(&wanted).enumerate() {
        assert_eq!(
            *found,
            format!("{wanted}{}", number + 1),
            "page {}: the text page view has on it, then its number",
            number + 1
        );
    }
    // Courier New at the layout's widths: the PDF found the faces the
    // layout measured with.
    assert_eq!(job.pdf.report.rows_off, 0, "{:?}", job.pdf.report);
    assert_eq!(job.pdf.report.images, 0);
}

/// A document that ends in a page break has the empty last sheet page
/// view counts, with its number.
#[gpui::test]
fn a_page_break_at_the_end_gives_an_empty_last_page(cx: &mut TestAppContext) {
    let document = in_courier(
        vec![
            Block::paragraph("The only words."),
            Block::new(BlockKind::PageBreak),
        ],
        PageSetup::default(),
    );
    let (jobs, workspace, cx) = open_recording(&document, cx);
    settle(&workspace, cx);
    print(&workspace, cx);
    let editor = editor_of(&workspace, cx);
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.page_count()),
        Some(2)
    );
    assert_eq!(page_texts(&jobs.borrow()[0]), ["Theonlywords.1", "2"]);
}

/// Landscape, a custom paper, a page break in the middle, a block that
/// runs over three pages, and an empty document.
#[gpui::test]
fn the_paper_and_the_breaks_are_the_documents(cx: &mut TestAppContext) {
    // Landscape A4, with a page break and a paragraph of three pages.
    let landscape = PageSetup {
        orientation: Orientation::Landscape,
        ..PageSetup::default()
    };
    let document = in_courier(
        vec![
            Block::paragraph("Before the break."),
            Block::new(BlockKind::PageBreak),
            Block::paragraph(LONG.repeat(130).trim_end()),
            Block::paragraph("After the long one."),
        ],
        landscape,
    );
    let (jobs, workspace, cx) = open_recording(&document, cx);
    settle(&workspace, cx);
    print(&workspace, cx);
    let job = jobs.borrow()[0].clone();
    let (width, height) = landscape.page_size();
    assert!(width > height);
    assert_eq!(job.paper, (width, height));
    let file = PdfFile::open(job.pdf.bytes()).unwrap();
    let pages = editor_of(&workspace, cx).read_with(cx, |editor, _| editor.page_count().unwrap());
    assert_eq!(file.page_count(), pages);
    assert!(pages >= 4, "the long paragraph is on three pages: {pages}");
    let size = file.page_size(0);
    assert!(close(size.0, width) && close(size.1, height), "{size:?}");
    let found = page_texts(&job);
    assert_eq!(found[0], "Beforethebreak.1");
    assert_eq!(found, {
        let mut wanted = text_page_view_has(&workspace, cx);
        for (number, page) in wanted.iter_mut().enumerate() {
            page.push_str(&(number + 1).to_string());
        }
        wanted
    });
    // The long paragraph begins on the second page and ends on a later
    // one, where the last paragraph follows it.
    assert!(found[1].starts_with("Theharborwas"));
    assert!(
        found[pages - 1].contains("Afterthelongone."),
        "{}",
        found[pages - 1]
    );

    // A paper of the writer's own: the document's settings change, and
    // the next job is on that paper.
    let custom = PageSetup {
        paper: PaperSize::Custom {
            width: 420.,
            height: 595.,
        },
        orientation: Orientation::Portrait,
        margins: Margins::default(),
    };
    workspace.update(cx, |w, cx| {
        w.change_document_settings(|settings| settings.page = custom, |_| String::new(), cx);
    });
    cx.run_until_parked();
    settle(&workspace, cx);
    print(&workspace, cx);
    let job = jobs.borrow()[1].clone();
    assert_eq!(job.paper, (420., 595.));
    let file = PdfFile::open(job.pdf.bytes()).unwrap();
    assert_eq!(
        Some(file.page_count()),
        editor_of(&workspace, cx).read_with(cx, |editor, _| editor.page_count())
    );
    for page in 0..file.page_count() {
        let size = file.page_size(page);
        assert!(close(size.0, 420.) && close(size.1, 595.), "{size:?}");
    }
}

#[gpui::test]
fn an_empty_document_is_one_page(cx: &mut TestAppContext) {
    let document = in_courier(vec![Block::paragraph("")], PageSetup::default());
    let (jobs, workspace, cx) = open_recording(&document, cx);
    print(&workspace, cx);
    let job = jobs.borrow()[0].clone();
    // Its number, and nothing else: the placeholder of an empty document
    // ("Start writing") is not on paper.
    assert_eq!(page_texts(&job), ["1"]);
    assert_eq!(job.title, "Untitled");
}

// ----- what is not printed ------------------------------------------------------------------

/// Whatever the window shows, the paper is the same: flow view, a zoom,
/// Night, a selection, a comment, an open find bar with matches and a
/// marked misspelling leave the PDF as it was, to the pixel.
#[gpui::test]
fn what_the_window_shows_is_not_on_paper(cx: &mut TestAppContext) {
    init(cx);
    // A spell checker that marks "teh".
    let words: Vec<&str> = LONG
        .split_whitespace()
        .chain([
            "The",
            "harbor",
            "She",
            "bought",
            "bread",
            "and",
            "salt",
            "ropes.",
            "Paragraph",
        ])
        .collect();
    let service = Rc::new(FakeSpellService::with_words(
        words
            .iter()
            .map(|word| word.trim_matches(|c: char| !c.is_alphabetic())),
    ));
    cx.update(|cx| spelling::set_service(service, cx));
    let document = in_courier(manuscript(12), PageSetup::default());
    let (_jobs, workspace, cx) = open_recording(&document, cx);
    settle(&workspace, cx);
    let editor = editor_of(&workspace, cx);

    let job_now = |cx: &mut VisualTestContext| {
        workspace
            .update_in(cx, |w, window, cx| w.print_job(window, cx))
            .expect("a document is open")
    };
    let pages_now = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| editor.read(cx).print_pass(window, cx).run_to_end())
    };
    let pixels = |job: &PrintJob| {
        let file = PdfFile::open(job.pdf.bytes()).unwrap();
        (0..file.page_count())
            .map(|page| file.page_pixels(page, 1.).expect("the page as pixels"))
            .collect::<Vec<_>>()
    };

    let plain = job_now(cx);
    let plain_pages = pages_now(cx);
    let plain_text = page_texts(&plain);
    assert!(plain_text[0].contains("tehropes."));

    // Flow view, twice the size, Night, a selection, a misspelling marked.
    workspace.update(cx, |w, cx| {
        w.toggle_page_view(cx);
        w.set_zoom(Zoom::Percent(200), cx);
        w.set_theme(ThemeMode::Dark, cx);
    });
    editor.update(cx, |editor, cx| {
        editor.edit_with(
            Autoscroll::None,
            |state| state.set_selection(Selection::new(Position::new(2, 0), Position::new(3, 9))),
            cx,
        );
    });
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    workspace.read_with(cx, |w, cx| {
        let editor = w.editor().read(cx);
        assert!(!editor.page_view());
        assert_eq!(editor.zoom_percent().round(), 200.);
        assert!(!editor.selection().is_collapsed());
        assert!(
            w.spell().expect("a checker").read(cx).misspelling_count(cx) > 0,
            "\"teh\" is marked"
        );
    });
    // None of it changes what the pass hands out, item for item (the view
    // of the document is a setting of its own, and no part of a page).
    assert_eq!(pages_now(cx), plain_pages);
    let job = job_now(cx);
    assert_eq!(page_texts(&job), plain_text);
    assert_eq!(job.pdf.report, plain.pdf.report);
    assert!(pixels(&job) == pixels(&plain), "the same picture");

    // A comment on the selection, and the find bar open with matches.
    cx.simulate_keystrokes("cmd-alt-m");
    cx.simulate_input("A note");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-1 cmd-f");
    cx.simulate_input("harbor");
    cx.run_until_parked();
    workspace.read_with(cx, |w, cx| {
        assert_eq!(w.comments().read(cx).threads().len(), 1);
        assert!(w.tools().read(cx).find_match_count() > 3);
    });
    let job = job_now(cx);
    assert_eq!(page_texts(&job), plain_text);
    assert_eq!(job.pdf.report, plain.pdf.report);
    assert!(
        pixels(&job) == pixels(&plain),
        "no tint of a comment, no mark of a match"
    );
    // The comparison would see a difference: one more word on the page
    // is another picture.
    cx.simulate_keystrokes("cmd-1");
    type_text(&workspace, "More", cx);
    let job = job_now(cx);
    assert_ne!(page_texts(&job), plain_text);
    assert!(pixels(&job) != pixels(&plain));
}

/// The job is the document as it is in the editor: its title, and text
/// that is not saved yet.
#[gpui::test]
fn the_job_has_the_title_and_the_unsaved_text(cx: &mut TestAppContext) {
    let document = in_courier(
        vec![
            Block::with_text(BlockKind::heading(1), "Chapter 2: The Harbour"),
            Block::paragraph("Saved words."),
        ],
        PageSetup::default(),
    );
    let (jobs, workspace, cx) = open_recording(&document, cx);
    let editor = editor_of(&workspace, cx);
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.set_caret(Position::new(1, 12));
                state.insert_text(" Typed just now.")
            },
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(
        workspace.read_with(cx, |w, _| w.save_state()),
        Some(writer_app::SaveState::Edited)
    );
    print(&workspace, cx);
    let job = jobs.borrow()[0].clone();
    assert_eq!(job.title, "Chapter 2: The Harbour");
    assert_eq!(job.file_name(), "Chapter 2 - The Harbour.pdf");
    assert_eq!(
        page_texts(&job),
        ["Chapter2:TheHarbourSavedwords.Typedjustnow.1"]
    );
    // Printing saved nothing and changed nothing.
    assert_eq!(
        workspace.read_with(cx, |w, _| w.save_state()),
        Some(writer_app::SaveState::Edited)
    );
    // What is typed after "Print…" was chosen is in the next job.
    type_text(&workspace, " And more.", cx);
    print(&workspace, cx);
    assert!(page_texts(&jobs.borrow()[1])[0].contains("Andmore."));
}

// ----- where it is reached from, and when it is there ---------------------------------------

#[gpui::test]
fn print_is_in_the_menu_bar_the_title_menu_the_palette_and_vim(cx: &mut TestAppContext) {
    let document = in_courier(manuscript(3), PageSetup::default());
    let (jobs, workspace, cx) = open_recording(&document, cx);

    // File > Print… and Cancel Printing, after Export… and the PDF export.
    let file: Vec<(String, Box<dyn gpui::Action>)> = menus::app_menus()
        .into_iter()
        .filter(|menu| menu.name.as_ref() == "File")
        .flat_map(|menu| menus::menu_actions(&[menu]))
        .collect();
    let names: Vec<&str> = file.iter().map(|(name, _)| name.as_str()).collect();
    let export = names.iter().position(|name| *name == "Export…").unwrap();
    assert_eq!(
        names[export..export + 5],
        [
            "Export…",
            "Export as PDF…",
            "Cancel PDF Export",
            "Print…",
            "Cancel Printing"
        ]
    );
    // Neither has a key beside it.
    cx.update(|_, cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        assert!(menus::native_menu_binding(&keymap, &Print).is_none());
        assert!(menus::native_menu_binding(&keymap, &CancelPrint).is_none());
    });
    // "Print…" is enabled, "Cancel Printing" greyed out: no pages are
    // being prepared.
    assert!(available(&Print, cx));
    assert!(!available(&CancelPrint, cx));
    let item = file[export + 3].1.boxed_clone();
    cx.update(|window, cx| window.dispatch_action(item, cx));
    let_it_prepare(&workspace, cx);
    assert_eq!(jobs.borrow().len(), 1);

    // The menu of the title, with the pointer.
    right_click_the_title(cx);
    assert_eq!(
        menu_labels(cx).unwrap(),
        [
            "Rename…",
            "Export…",
            "Export as PDF…",
            "Print…",
            "Version History…",
            "Document Settings…",
            "-",
            "Reveal in Library"
        ]
    );
    let at = cx.update(|window, cx| {
        let menu = ContextMenu::current(window, cx).unwrap();
        let menu = menu.read(cx);
        let ix = menu.labels().iter().position(|l| l == "Print…").unwrap();
        menu.item_bounds(ix).unwrap().center()
    });
    cx.simulate_click(at, Modifiers::default());
    let_it_prepare(&workspace, cx);
    assert_eq!(jobs.borrow().len(), 2);
    assert!(editor_focused(&workspace, cx));

    // The palette: "Print", in File.
    cx.simulate_keystrokes("cmd-shift-p");
    cx.simulate_input("print");
    cx.simulate_keystrokes("enter");
    let_it_prepare(&workspace, cx);
    assert_eq!(jobs.borrow().len(), 3);
    assert!(editor_focused(&workspace, cx));
    cx.update(|_, cx| {
        let print = writer_app::commands::all(cx)
            .into_iter()
            .find(|command| command.action_name() == "workspace::Print")
            .expect("Print is a command");
        assert_eq!(print.label(), "File: Print");
        let cancel = writer_app::commands::all(cx)
            .into_iter()
            .find(|command| command.action_name() == "workspace::CancelPrint")
            .expect("Cancel printing is a command");
        assert_eq!(cancel.label(), "File: Cancel printing");
    });

    // Vim's command line.
    workspace.update(cx, |w, cx| w.toggle_vim_mode(cx));
    cx.run_until_parked();
    cx.simulate_input(":print");
    cx.simulate_keystrokes("enter");
    let_it_prepare(&workspace, cx);
    assert_eq!(jobs.borrow().len(), 4);

    // Every job is the same document.
    let texts: Vec<Vec<String>> = jobs.borrow().iter().map(page_texts).collect();
    assert!(texts.iter().all(|text| *text == texts[0]));
    // And each said nothing: a recorded job is no toast.
    assert!(toasts(&workspace, cx).is_empty());
}

#[gpui::test]
fn without_a_document_there_is_no_print(cx: &mut TestAppContext) {
    init(cx);
    let jobs = cx.update(printing::record);
    let library = library(cx);
    let (workspace, cx) = open_workspace(&library, cx);
    assert_eq!(workspace.read_with(cx, |w, _| w.open_document_id()), None);
    assert!(!available(&Print, cx));
    assert!(!available(&CancelPrint, cx));
    cx.dispatch_action(Print);
    cx.run_until_parked();
    assert_eq!(stage(&workspace, cx), None);
    assert!(jobs.borrow().is_empty());
    assert!(
        workspace
            .update_in(cx, |w, window, cx| w.print_job(window, cx))
            .is_none()
    );
    assert!(state_line(&workspace, cx).contains(" print=[idle] "));
}

/// A workspace has Print only when the app was given somewhere for print
/// jobs to go: no test that does not ask for it can meet a panel.
#[gpui::test]
fn without_a_target_there_is_no_print(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let id = add_document(
        &library,
        None,
        &in_courier(manuscript(3), PageSetup::default()),
        cx,
    );
    let (workspace, cx) = open_workspace(&library, cx);
    workspace.update_in(cx, |w, window, cx| {
        w.open_document(id, true, window, cx);
    });
    cx.run_until_parked();
    assert!(cx.update(|_, cx| printing::target(cx).is_none()));
    assert!(!workspace.read_with(cx, |w, cx| w.can_print(cx)));
    assert!(!available(&Print, cx));
    cx.dispatch_action(Print);
    cx.run_until_parked();
    assert_eq!(stage(&workspace, cx), None);
    // The title's menu does not offer what has no handler.
    right_click_the_title(cx);
    let labels = menu_labels(cx).unwrap();
    assert!(!labels.contains(&"Print…".to_string()), "{labels:?}");
    // The exact job can still be made (a test, a script step).
    cx.update(ContextMenu::dismiss);
    let job = workspace
        .update_in(cx, |w, window, cx| w.print_job(window, cx))
        .expect("a document is open");
    assert_eq!(job.pdf.report.pages, 1);
}

/// While a job is preparing or in the print panel, a second "Print…" does
/// nothing.
#[gpui::test]
fn a_second_print_does_nothing_while_one_is_under_way(cx: &mut TestAppContext) {
    let document = in_courier(lines(200), PageSetup::default());
    let (jobs, workspace, cx) = open_recording(&document, cx);
    // A slice is one block, and the next comes a millisecond of the test
    // clock later.
    cx.update(|_, cx| printing::set_slice(Duration::ZERO, cx));
    let panel = cx.update(|_, cx| printing::hold_the_panel(cx));

    cx.dispatch_action(Print);
    cx.run_until_parked();
    assert_eq!(stage(&workspace, cx), Some(Printing::Preparing));
    // Still "Print…" has its handler (the item is not greyed out): it
    // just does nothing.
    assert!(available(&Print, cx));
    cx.dispatch_action(Print);
    cx.executor().advance_clock(Duration::from_millis(20));
    cx.dispatch_action(Print);
    let_it_prepare(&workspace, cx);
    assert_eq!(jobs.borrow().len(), 1, "one job, made once");

    // In the panel.
    assert_eq!(stage(&workspace, cx), Some(Printing::Panel));
    assert_eq!(panel.open_count(), 1);
    assert!(state_line(&workspace, cx).contains(" print=[panel] "));
    // Nothing is being prepared: "Cancel Printing" is greyed out (the
    // panel has its own Cancel).
    assert!(!available(&CancelPrint, cx));
    cx.dispatch_action(Print);
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();
    assert_eq!(jobs.borrow().len(), 1);
    assert_eq!(panel.open_count(), 1);
    // "Dismiss notifications" does not end a panel.
    cx.dispatch_action(DismissNotifications);
    cx.run_until_parked();
    assert_eq!(stage(&workspace, cx), Some(Printing::Panel));
    // What is typed while the panel is up is not in its pages.
    type_text(&workspace, "Typed under the panel. ", cx);
    assert!(!page_texts(&jobs.borrow()[0])[0].contains("Typedunderthepanel."));

    // The panel ends (Print or Cancel in it): the next "Print…" works.
    assert!(panel.end(false));
    cx.run_until_parked();
    assert_eq!(stage(&workspace, cx), None);
    assert!(state_line(&workspace, cx).contains(" print=[idle] "));
    cx.dispatch_action(Print);
    let_it_prepare(&workspace, cx);
    assert_eq!(jobs.borrow().len(), 2);
    assert_eq!(panel.open_count(), 1);
    assert!(page_texts(&jobs.borrow()[1])[0].contains("Typedunderthepanel."));
    assert!(panel.end(true));
    cx.run_until_parked();
    assert_eq!(stage(&workspace, cx), None);
}

// ----- cancel -------------------------------------------------------------------------------

/// A book takes a while: after 0.4 s a note says so, and "Cancel
/// printing" stops it from the palette, from the note's button, from a
/// key the user gave it, and by dismissing the notes.
#[gpui::test]
fn a_long_job_says_so_and_can_be_cancelled(cx: &mut TestAppContext) {
    let document = in_courier(lines(1000), PageSetup::default());
    let (jobs, workspace, cx) = open_recording(&document, cx);
    // A slice is one block and the slices are a millisecond of the test
    // clock apart: a second in all.
    cx.update(|_, cx| printing::set_slice(Duration::ZERO, cx));
    let note = preparing_note("A long list");
    assert_eq!(note, "Preparing to print “A long list”…");
    let note_button = |cx: &mut VisualTestContext| {
        workspace.read_with(cx, |w, cx| {
            let layer = w.toast_layer().read(cx);
            layer
                .toasts()
                .find(|toast| toast.message.as_ref() == note)
                .map(|toast| toast.action_label().map(|label| label.to_string()))
        })
    };
    // Starts a job and lets it come to its note.
    let begin = |cx: &mut VisualTestContext| {
        assert_eq!(stage(&workspace, cx), None);
        assert!(!available(&CancelPrint, cx));
        cx.dispatch_action(Print);
        cx.run_until_parked();
        assert_eq!(stage(&workspace, cx), Some(Printing::Preparing));
        assert!(state_line(&workspace, cx).contains(" print=[preparing 0%] "));
        // File > Cancel Printing is enabled now.
        assert!(available(&CancelPrint, cx));
        // Not yet a note: a chapter is done before anybody could read one.
        cx.executor()
            .advance_clock(PRINT_NOTE_DELAY - Duration::from_millis(50));
        cx.run_until_parked();
        assert_eq!(note_button(cx), None);
        cx.executor().advance_clock(Duration::from_millis(60));
        cx.run_until_parked();
        assert_eq!(note_button(cx), Some(Some("Cancel".to_string())));
        assert!(workspace.read_with(cx, |w, _| w.print_note()).is_some());
        let line = state_line(&workspace, cx);
        assert!(line.contains(" print=[preparing 4"), "{line}");
        assert!(jobs.borrow().is_empty());
    };
    // After a cancel: nothing is recorded however far the clock runs.
    let cancelled = |how: &str, cx: &mut VisualTestContext| {
        cx.run_until_parked();
        assert_eq!(stage(&workspace, cx), None, "{how}");
        assert_eq!(note_button(cx), None, "{how}: the note is gone");
        assert!(state_line(&workspace, cx).contains(" print=[idle] "));
        assert!(!available(&CancelPrint, cx), "{how}: no handler again");
        cx.executor().advance_clock(Duration::from_secs(5));
        cx.run_until_parked();
        assert!(jobs.borrow().is_empty(), "{how}: nothing was printed");
        assert_eq!(stage(&workspace, cx), None);
    };

    // The palette.
    begin(cx);
    cx.simulate_keystrokes("cmd-shift-p");
    cx.simulate_input("cancel printing");
    cx.simulate_keystrokes("enter");
    cancelled("the palette", cx);
    assert!(editor_focused(&workspace, cx));

    // The button of the note.
    begin(cx);
    let layer = workspace.read_with(cx, |w, _| w.toast_layer().clone());
    let id = workspace
        .read_with(cx, |w, _| w.print_note())
        .expect("the note is up");
    assert!(cx.update(|window, cx| layer.update(cx, |layer, cx| layer.run_action(id, window, cx))));
    cancelled("the button of the note", cx);

    // A key the user gave the command; the button shows it.
    let cancel = Target::new("workspace::CancelPrint", None);
    workspace
        .update_in(cx, |w, window, cx| {
            w.change_keys(window, cx, |keymap| {
                keymap.add_key(&cancel, KeySequence::parse("ctrl-alt-.").unwrap())
            })
        })
        .expect("the app has a keymap");
    cx.run_until_parked();
    begin(cx);
    assert_eq!(
        cx.update(|_, cx| writer_ui::KeyHint::for_action(&CancelPrint, cx).map(|hint| hint.text())),
        Some("⌃⌥.".to_string())
    );
    cx.simulate_keystrokes("ctrl-alt-.");
    cancelled("the key", cx);
    // The key does nothing while there is nothing to cancel.
    cx.simulate_keystrokes("ctrl-alt-.");
    cx.run_until_parked();
    assert_eq!(stage(&workspace, cx), None);

    // "Dismiss notifications": the note must not go while the work goes
    // on unseen.
    begin(cx);
    cx.dispatch_action(DismissNotifications);
    cancelled("Dismiss notifications", cx);

    // Before the note is up, too.
    cx.dispatch_action(Print);
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(100));
    cx.dispatch_action(CancelPrint);
    cancelled("a cancel before the note", cx);

    // Without a cancel the same job is recorded once the clock has run,
    // and the note goes with it.
    begin(cx);
    let_it_prepare(&workspace, cx);
    assert_eq!(stage(&workspace, cx), None);
    assert_eq!(note_button(cx), None);
    assert_eq!(jobs.borrow().len(), 1);
    let job = jobs.borrow()[0].clone();
    assert_eq!(job.pdf.report.rows, 1000);
    assert_eq!(Some(job.pdf.report.pages), {
        settle(&workspace, cx);
        editor_of(&workspace, cx).read_with(cx, |editor, _| editor.page_count())
    });
    assert!(!available(&CancelPrint, cx));
}

/// The note of a job goes only with the job, whichever way notes go: a
/// click on it (which takes any note away) stops the job as "Dismiss
/// notifications" does, and four newer notes do not push it out of sight.
/// (Found in review: both took the note away while the pages went on being
/// made unseen, and the print panel came up unannounced.)
#[gpui::test]
fn the_note_of_a_long_job_goes_only_with_the_job(cx: &mut TestAppContext) {
    let document = in_courier(lines(1000), PageSetup::default());
    let (jobs, workspace, cx) = open_recording(&document, cx);
    cx.update(|_, cx| printing::set_slice(Duration::ZERO, cx));
    let note = preparing_note("A long list");
    let layer = workspace.read_with(cx, |w, _| w.toast_layer().clone());
    let begin = |cx: &mut VisualTestContext| {
        cx.dispatch_action(Print);
        cx.run_until_parked();
        cx.executor()
            .advance_clock(PRINT_NOTE_DELAY + Duration::from_millis(10));
        cx.run_until_parked();
        assert_eq!(toasts(&workspace, cx), [note.clone()]);
        assert_eq!(stage(&workspace, cx), Some(Printing::Preparing));
    };

    // Four newer notes (four saves with cmd-s say "Saved" four times): the
    // layer shows four and drops the oldest, and the note of the job comes
    // back as the newest.
    begin(cx);
    for n in 1..=4 {
        layer.update(cx, |layer, cx| {
            layer.push(Toast::new(format!("Note {n}")), cx);
        });
        cx.run_until_parked();
    }
    let shown = toasts(&workspace, cx);
    assert_eq!(shown.len(), writer_ui::toast::MAX_VISIBLE);
    assert_eq!(shown.last(), Some(&note), "{shown:?}");
    assert_eq!(stage(&workspace, cx), Some(Printing::Preparing));
    let id = workspace
        .read_with(cx, |w, _| w.print_note())
        .expect("the note is up");
    assert!(layer.read_with(cx, |layer, _| layer.ids().contains(&id)));
    // The job ends as it would have: handed over, and its note goes.
    let_it_prepare(&workspace, cx);
    assert_eq!(jobs.borrow().len(), 1);
    assert!(!toasts(&workspace, cx).contains(&note));
    assert_eq!(workspace.read_with(cx, |w, _| w.print_note()), None);
    layer.update(cx, |layer, cx| layer.clear(cx));
    cx.run_until_parked();

    // A click on the note itself, not on its button: the note goes, and
    // the job with it.
    begin(cx);
    let id = workspace
        .read_with(cx, |w, _| w.print_note())
        .expect("the note is up");
    // The layer draws a toast as `toast-<n>`; the id says its n.
    let n: String = format!("{id:?}")
        .chars()
        .filter(char::is_ascii_digit)
        .collect();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds(Box::leak(format!("toast-{n}").into_boxed_str()))
        .expect("the note is drawn");
    cx.simulate_click(
        gpui::point(bounds.left() + gpui::px(6.), bounds.center().y),
        Modifiers::none(),
    );
    cx.run_until_parked();
    assert!(
        !toasts(&workspace, cx).contains(&note),
        "the click took it away"
    );
    assert_eq!(stage(&workspace, cx), None, "the job stopped with it");
    assert!(state_line(&workspace, cx).contains(" print=[idle] "));
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();
    assert_eq!(jobs.borrow().len(), 1, "nothing more was handed over");
}

// ----- keys ---------------------------------------------------------------------------------

/// Print and Cancel printing are rows "Anywhere" without a key, take one
/// in the list, and are entries of the shortcuts file.
#[gpui::test]
fn print_has_no_key_and_takes_one(cx: &mut TestAppContext) {
    let document = in_courier(manuscript(3), PageSetup::default());
    let (jobs, workspace, cx) = open_recording(&document, cx);
    let rows = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            collect_shortcuts(cx)
                .into_iter()
                .filter(|row| {
                    row.title.as_ref() == "Print" || row.title.as_ref() == "Cancel printing"
                })
                .map(|row| {
                    (
                        row.title.to_string(),
                        row.place,
                        row.all_keys
                            .iter()
                            .map(|key| key.keys.to_string())
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(
        rows(cx),
        [
            ("Print".to_string(), Place::Anywhere, vec![]),
            ("Cancel printing".to_string(), Place::Anywhere, vec![]),
        ]
    );
    // No key of the default keymap prints: cmd-p opens a document.
    cx.update(|_, cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        assert_eq!(keymap.bindings_for_action(&Print).count(), 0);
        assert_eq!(keymap.bindings_for_action(&CancelPrint).count(), 0);
    });

    let print_target = Target::new("workspace::Print", None);
    let added = workspace
        .update_in(cx, |w, window, cx| {
            w.change_keys(window, cx, |keymap| {
                let key = KeySequence::parse("ctrl-alt-p").unwrap();
                assert!(keymap.check(&print_target, &key, None).is_allowed());
                keymap.add_key(&print_target, key)
            })
        })
        .expect("the app has a keymap");
    assert!(added);
    cx.run_until_parked();
    assert_eq!(rows(cx)[0].2, ["ctrl-alt-p"]);
    // From the text, and from the library: a key of the window.
    cx.simulate_keystrokes("ctrl-alt-p");
    let_it_prepare(&workspace, cx);
    assert_eq!(jobs.borrow().len(), 1);
    cx.simulate_keystrokes("cmd-0 ctrl-alt-p");
    let_it_prepare(&workspace, cx);
    assert_eq!(jobs.borrow().len(), 2);
    // The menu bar shows the key beside "Print…".
    cx.update(|_, cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let shown = menus::native_menu_binding(&keymap, &Print).expect("a key in the menu");
        assert_eq!(shown.keystrokes()[0].unparse(), "ctrl-alt-p");
    });

    // The shortcuts file has both, with the key that was given.
    let out = tempfile::tempdir().expect("a temporary directory");
    let path = out.path().join("The Writer Shortcuts.json");
    cx.dispatch_action(writer_app::actions::ExportShortcuts);
    cx.run_until_parked();
    assert!(cx.did_prompt_for_new_path(), "the save panel is asked");
    let target = path.clone();
    cx.simulate_new_path_selection(move |_| Some(target));
    cx.run_until_parked();
    let exported: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let entry = |command: &str| {
        exported["shortcuts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["command"] == command)
            .unwrap_or_else(|| panic!("{command} is an entry of the file"))
            .clone()
    };
    let print = entry("workspace::Print");
    assert_eq!(print["title"], "Print");
    assert_eq!(print["where"], "Anywhere");
    assert_eq!(print["default"], serde_json::json!([]));
    assert_eq!(print["keys"], serde_json::json!(["ctrl-alt-p"]));
    let cancel = entry("workspace::CancelPrint");
    assert_eq!(cancel["title"], "Cancel printing");
    assert_eq!(cancel["where"], "Anywhere");
    assert_eq!(cancel["keys"], serde_json::json!([]));
}

// ----- scripted runs ------------------------------------------------------------------------

#[gpui::test]
fn the_print_steps_of_a_script_parse_and_the_state_says_idle(cx: &mut TestAppContext) {
    let script = Script::parse(
        "print:pdf:/tmp/a file.pdf; print:job:out/b.pdf; action:workspace::Print; \
         action:workspace::CancelPrint",
    )
    .unwrap();
    assert_eq!(
        script.steps,
        [
            Step::Print(PrintStep::Pdf("/tmp/a file.pdf".into())),
            Step::Print(PrintStep::Job("out/b.pdf".into())),
            Step::Action("workspace::Print".to_string()),
            Step::Action("workspace::CancelPrint".to_string()),
        ]
    );
    assert!(Script::parse("print").is_err());
    assert!(Script::parse("print:pdf").is_err());
    assert!(Script::parse("print:pdf:").is_err());
    assert!(Script::parse("print:paper:/tmp/a.pdf").is_err());

    let document = in_courier(manuscript(3), PageSetup::default());
    let (_jobs, workspace, cx) = open_recording(&document, cx);
    let line = state_line(&workspace, cx);
    assert!(line.contains(" print=[idle] "), "{line}");

    // What `print:pdf:` logs about the job it made.
    let job = workspace
        .update_in(cx, |w, window, cx| w.print_job(window, cx))
        .unwrap();
    let lines = printing::report_lines(&job, Duration::from_millis(40));
    assert_eq!(
        lines,
        [format!(
            "1 page, 595.28 x 841.89 pt, {} rows, 0 rows off, 0 images, fonts: .SFNS-Regular, \
             CourierNewPS-BoldMT, CourierNewPSMT, made in 0.04 s",
            job.pdf.report.rows
        )]
    );
    assert!(job.pdf.report.rows > 5);
    // A font the document asks for and this Mac does not have is a second
    // line.
    let mut without = job.clone();
    without.pdf.report.missing_families = vec!["Sabon".to_string()];
    assert_eq!(
        printing::report_lines(&without, Duration::from_millis(40))[1],
        "missing fonts: Sabon"
    );
}

/// A scripted run never opens the panel: the command writes the PDF into
/// the directory the run names, or says that it has none.
#[gpui::test]
fn in_a_scripted_run_print_writes_a_file_or_says_why_not(cx: &mut TestAppContext) {
    init(cx);
    let out = tempfile::tempdir().expect("a temporary directory");
    let directory = out.path().join("prints");
    cx.update(|cx| {
        printing::set_target(
            printing::PrintTarget::Directory(Some(directory.clone())),
            cx,
        )
    });
    let library = library(cx);
    let id = add_document(
        &library,
        None,
        &in_courier(
            vec![
                Block::with_text(BlockKind::heading(1), "Notes: one/two"),
                Block::paragraph("A few words."),
            ],
            PageSetup::default(),
        ),
        cx,
    );
    let (workspace, cx) = open_workspace(&library, cx);
    workspace.update_in(cx, |w, window, cx| {
        w.open_document(id, true, window, cx);
    });
    cx.run_until_parked();
    print(&workspace, cx);
    let written = directory.join("Notes - one-two.pdf");
    let file = PdfFile::open(&std::fs::read(&written).expect("the PDF was written"))
        .expect("the file is a PDF");
    assert_eq!(file.page_count(), 1);
    assert_eq!(squeezed(&file.page_text(0)), "Notes:one/twoAfewwords.1");
    let said = toasts(&workspace, cx);
    assert_eq!(said.len(), 1);
    assert!(
        said[0].starts_with("Wrote “Notes - one-two.pdf” to ") && said[0].ends_with("/prints."),
        "{said:?}"
    );

    // Without a directory: only the sentence.
    cx.dispatch_action(DismissNotifications);
    cx.update(|_, cx| printing::set_target(printing::PrintTarget::Directory(None), cx));
    print(&workspace, cx);
    assert_eq!(
        toasts(&workspace, cx),
        ["A scripted run does not open the print panel."]
    );
    assert_eq!(stage(&workspace, cx), None);
}

/// A slice is laying out and drawing together: the layout gets the share
/// of it that leaves room for drawing what it lays out.
#[test]
fn a_slice_leaves_room_for_drawing_the_pages() {
    let ms = Duration::from_millis;
    // Nothing is known yet: half.
    assert_eq!(print_layout_share(ms(8), ms(0), ms(0)), ms(4));
    // Drawing has cost as much as laying out: half; three times as much:
    // a quarter.
    assert_eq!(print_layout_share(ms(8), ms(50), ms(50)), ms(4));
    assert_eq!(print_layout_share(ms(8), ms(50), ms(150)), ms(2));
    // Never all of it and never nothing.
    assert_eq!(print_layout_share(ms(10), ms(1000), ms(1)), ms(9));
    assert_eq!(print_layout_share(ms(10), ms(1), ms(1000)), ms(2));
    // A test's slice of zero stays zero: one block a slice.
    assert_eq!(print_layout_share(ms(0), ms(5), ms(5)), ms(0));
}

/// The welcome document says where Print is.
#[test]
fn the_welcome_document_says_where_print_is() {
    let text = writer_app::welcome::welcome_document().plain_text();
    assert!(
        text.contains(
            "Print… in the File menu, or Print in the command palette, puts it on paper as \
             page view shows it; Export as PDF… in the File menu makes a PDF of the same pages."
        ),
        "{text}"
    );
}

/// Everything the print panel is asked for is in the Objective-C runtime
/// of the app as it is built (PDFKit is linked in): what `show_panel`
/// asks before it sends a message. The panel itself is never opened here.
#[test]
fn the_app_has_what_the_print_panel_is_asked_for() {
    assert_eq!(writer_print::check_selectors(), Ok(()));
}

/// The end of a panel reaches whoever waits for it: the answer that was
/// sent, and "not printed" when the panel went away without one.
#[gpui::test]
async fn the_end_of_a_panel_is_heard(cx: &mut TestAppContext) {
    let (end, ended) = printing::panel_end();
    let waiting = cx.foreground_executor().spawn(ended);
    cx.run_until_parked();
    end.send(true);
    assert!(waiting.await);

    let (end, ended) = printing::panel_end();
    end.send(false);
    assert!(!ended.await);

    let (end, ended) = printing::panel_end();
    let waiting = cx.foreground_executor().spawn(ended);
    cx.run_until_parked();
    drop(end);
    assert!(!waiting.await);
}
