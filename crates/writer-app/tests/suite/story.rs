//! The whole product in one sitting, from the keyboard, against a library
//! on disk: a folder and a document, rich text written with shortcuts,
//! Markdown rules, the format menu, the slash menu, the link popover and
//! the font picker, a comment, page view, Vim mode, both exports, and
//! everything found again after the library is reopened.

use std::path::Path;

use gpui::TestAppContext;
use writer_core::{Alignment, BlockKind, InlineAttrs};
use writer_editor_tools::ToolKind;
use writer_vim_mode::Mode;

use super::*;

fn tool(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Option<ToolKind> {
    workspace.read_with(cx, |w, cx| w.tools().read(cx).active_tool())
}

fn vim_mode(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Option<Mode> {
    workspace.read_with(cx, |w, cx| {
        let vim = w.vim().read(cx);
        vim.is_attached().then(|| vim.mode())
    })
}

fn document(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Document {
    workspace.read_with(cx, |w, cx| w.editor().read(cx).document().clone())
}

/// The attributes of the first character of `needle` in `document`.
#[track_caller]
fn attrs_of(document: &Document, needle: &str) -> InlineAttrs {
    for block in document.blocks() {
        if let Some(offset) = block.text().find(needle) {
            return block.attrs_at(offset).cloned().unwrap_or_default();
        }
    }
    panic!("{needle:?} is not in the document");
}

#[track_caller]
fn block_with<'a>(document: &'a Document, text: &str) -> &'a writer_core::Block {
    document
        .blocks()
        .iter()
        .find(|block| block.text() == text)
        .unwrap_or_else(|| panic!("no block reads {text:?} in {:?}", document.plain_text()))
}

/// Exports through the picker (`row` rows down from the first) into `path`.
fn export_row(
    workspace: &Entity<Workspace>,
    row: usize,
    path: &Path,
    cx: &mut VisualTestContext,
) {
    cx.simulate_keystrokes("cmd-shift-s");
    for _ in 0..row {
        cx.simulate_keystrokes("down");
    }
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(cx.did_prompt_for_new_path(), "the save panel opened");
    let path = path.to_path_buf();
    cx.simulate_new_path_selection(move |_| Some(path.clone()));
    cx.run_until_parked();
    assert!(editor_focused(workspace, cx));
}

/// The text macOS reads out of a Word document, when `textutil` is there.
fn docx_text(path: &Path) -> Option<String> {
    let output = std::process::Command::new("/usr/bin/textutil")
        .args(["-convert", "txt", "-stdout"])
        .arg(path)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[gpui::test]
fn a_writing_session_from_first_launch_to_reopening(cx: &mut TestAppContext) {
    init(cx);
    cx.update(|cx| {
        writer_editor_tools::set_font_families(["Charter", "Georgia", "Menlo"], cx);
    });
    let home = tempfile::tempdir().unwrap();
    let database = home.path().join("library.db");
    let out = tempfile::tempdir().unwrap();

    let document_id = {
        let library = cx.new(|_| Library::open(&database).expect("a library on disk"));
        let (workspace, vcx) = open_workspace(&library, cx);
        // First launch: the welcome document is open and the text has the
        // keyboard.
        assert_eq!(
            workspace.read_with(vcx, |w, _| w.document_title().map(str::to_string)),
            Some("Welcome to The Writer".to_string())
        );
        assert!(editor_focused(&workspace, vcx));

        // ----- a folder, named in the sidebar; a document in it ---------------
        vcx.simulate_keystrokes("cmd-shift-n");
        vcx.run_until_parked();
        vcx.simulate_input("Salt Road");
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
        let folder = library.read_with(vcx, |library, _| {
            let tree = library.tree();
            assert_eq!(tree.folders.len(), 1);
            assert_eq!(tree.folders[0].folder.name, "Salt Road");
            tree.folders[0].folder.id
        });
        vcx.simulate_keystrokes("cmd-n");
        vcx.run_until_parked();
        let id = workspace
            .read_with(vcx, |w, _| w.open_document_id())
            .expect("the new document is open");
        assert_eq!(
            library.read_with(vcx, |library, _| library
                .document_meta(id)
                .unwrap()
                .folder_id),
            Some(folder)
        );
        assert!(editor_focused(&workspace, vcx));

        // ----- writing: Markdown rules and shortcuts --------------------------
        vcx.simulate_input("# The Harbour");
        vcx.simulate_keystrokes("enter");
        vcx.simulate_input("The ferry leaves at ");
        vcx.simulate_keystrokes("cmd-b");
        vcx.simulate_input("dawn");
        vcx.simulate_keystrokes("cmd-b");
        vcx.simulate_input(" and the ");
        vcx.simulate_keystrokes("cmd-i");
        vcx.simulate_input("gulls");
        vcx.simulate_keystrokes("cmd-i");
        vcx.simulate_input(" follow it out.");
        vcx.simulate_keystrokes("enter");

        // The format menu (cmd-.): filter, enter.
        vcx.simulate_keystrokes("cmd-.");
        assert_eq!(tool(&workspace, vcx), Some(ToolKind::Format));
        assert!(!editor_focused(&workspace, vcx));
        vcx.simulate_input("heading 2");
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
        assert_eq!(tool(&workspace, vcx), None);
        assert!(editor_focused(&workspace, vcx));
        vcx.simulate_input("Things to pack");
        vcx.simulate_keystrokes("enter");

        // The slash menu: focus stays in the text, the query filters.
        vcx.simulate_input("/");
        assert_eq!(tool(&workspace, vcx), Some(ToolKind::Insert));
        assert!(editor_focused(&workspace, vcx));
        vcx.simulate_input("bul");
        vcx.simulate_keystrokes("enter");
        assert_eq!(tool(&workspace, vcx), None);
        vcx.simulate_input("rope");
        vcx.simulate_keystrokes("enter");
        vcx.simulate_input("salt");
        vcx.simulate_keystrokes("enter enter");
        // Numbered, to-do and quote by their Markdown prefixes.
        vcx.simulate_input("1. first");
        vcx.simulate_keystrokes("enter");
        vcx.simulate_input("second");
        vcx.simulate_keystrokes("enter enter");
        vcx.simulate_input("[] ticket");
        vcx.simulate_keystrokes("enter enter");
        vcx.simulate_input("> Never trust a calm sea.");
        vcx.simulate_keystrokes("enter enter");
        // A code block by its shortcut.
        vcx.simulate_keystrokes("cmd-shift-'");
        vcx.simulate_input("let knots = 12;");
        // Two empty lines at the end and one more enter leave the block.
        vcx.simulate_keystrokes("enter enter enter");

        // A link through the popover, a highlight, a font.
        vcx.simulate_input("A link to the port");
        vcx.simulate_keystrokes("shift-alt-left shift-alt-left cmd-k");
        assert_eq!(tool(&workspace, vcx), Some(ToolKind::Link));
        vcx.simulate_input("example.com/port");
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
        assert!(editor_focused(&workspace, vcx));
        vcx.simulate_keystrokes("cmd-down");
        vcx.simulate_input(" and ");
        vcx.simulate_keystrokes("cmd-shift-h");
        vcx.simulate_input("bright");
        vcx.simulate_keystrokes("cmd-shift-h");
        vcx.simulate_input(" words in Georgia");
        vcx.simulate_keystrokes("shift-alt-left cmd-t");
        assert_eq!(tool(&workspace, vcx), Some(ToolKind::Font));
        vcx.simulate_input("geo");
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
        assert!(editor_focused(&workspace, vcx));
        vcx.simulate_keystrokes("cmd-down enter");

        // Alignment and text size.
        vcx.simulate_keystrokes("cmd-shift-e");
        vcx.simulate_input("Centred");
        vcx.simulate_keystrokes("enter cmd-shift-l cmd-shift-. cmd-shift-.");
        vcx.simulate_input("Larger");

        let written = document(&workspace, vcx);
        assert_eq!(
            written.plain_text(),
            "The Harbour\n\
             The ferry leaves at dawn and the gulls follow it out.\n\
             Things to pack\n\
             rope\nsalt\nfirst\nsecond\nticket\n\
             Never trust a calm sea.\n\
             let knots = 12;\n\
             A link to the port and bright words in Georgia\n\
             Centred\nLarger"
        );
        let kind = |text: &str| block_with(&written, text).kind().clone();
        assert_eq!(kind("The Harbour"), BlockKind::heading(1));
        assert_eq!(kind("Things to pack"), BlockKind::heading(2));
        assert_eq!(kind("rope"), BlockKind::BulletItem);
        assert_eq!(kind("second"), BlockKind::NumberedItem);
        assert_eq!(kind("ticket"), BlockKind::todo());
        assert_eq!(kind("Never trust a calm sea."), BlockKind::Quote);
        assert!(kind("let knots = 12;").is_code());
        assert!(attrs_of(&written, "dawn").bold);
        assert!(attrs_of(&written, "gulls").italic);
        assert!(!attrs_of(&written, "follow").italic);
        assert_eq!(
            attrs_of(&written, "the port").link.as_deref(),
            Some("https://example.com/port")
        );
        assert!(attrs_of(&written, "bright").highlight.is_some());
        assert_eq!(attrs_of(&written, "Georgia").font.as_deref(), Some("Georgia"));
        assert_eq!(block_with(&written, "Centred").alignment(), Alignment::Center);
        let larger = block_with(&written, "Larger");
        assert_eq!(larger.alignment(), Alignment::Left);
        assert!(larger.text_size().expect("a size of its own") > 17.);
        written.check_invariants().unwrap();

        // ----- a comment ------------------------------------------------------
        vcx.simulate_keystrokes("cmd-up down shift-alt-right shift-alt-right cmd-alt-m");
        assert!(workspace.read_with(vcx, |w, cx| w.comments().read(cx).is_composing()));
        assert!(workspace.read_with(vcx, |w, _| w.has_right_panel()));
        vcx.simulate_input("Which ferry?");
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
        assert!(editor_focused(&workspace, vcx));
        let threads = workspace.read_with(vcx, |w, cx| {
            w.comments()
                .read(cx)
                .threads()
                .iter()
                .map(|entry| (entry.quote.clone(), entry.thread.body.clone()))
                .collect::<Vec<_>>()
        });
        assert_eq!(
            threads,
            vec![("The ferry".to_string(), "Which ferry?".to_string())]
        );
        // Hide and show: the panel leaves and comes back, the setting follows.
        vcx.simulate_keystrokes("cmd-alt-c");
        vcx.run_until_parked();
        assert!(!workspace.read_with(vcx, |w, _| w.has_right_panel()));
        assert!(!workspace.read_with(vcx, |w, _| w.settings().comments_visible));
        assert!(!workspace.read_with(vcx, |w, cx| w.editor().read(cx).config().comments_visible));
        vcx.simulate_keystrokes("cmd-alt-c");
        vcx.run_until_parked();
        assert!(workspace.read_with(vcx, |w, _| w.has_right_panel()));
        assert!(workspace.read_with(vcx, |w, _| w.settings().comments_visible));

        // ----- page view and Vim mode -----------------------------------------
        vcx.simulate_keystrokes("cmd-alt-p");
        assert!(workspace.read_with(vcx, |w, cx| w.editor().read(cx).page_view()));
        assert!(
            workspace
                .update(vcx, |w, cx| w.status_line(cx))
                .page
                .is_some_and(|page| page.starts_with("Page 1 of"))
        );

        vcx.simulate_keystrokes("cmd-alt-v");
        vcx.run_until_parked();
        // The commented words are still selected: Vim starts with them as
        // its Visual selection, and escape leaves it.
        assert_eq!(vim_mode(&workspace, vcx), Some(Mode::Visual));
        vcx.simulate_keystrokes("escape");
        assert_eq!(vim_mode(&workspace, vcx), Some(Mode::Normal));
        assert!(workspace.read_with(vcx, |w, _| w.settings().vim_mode));
        // Normal mode: letters are commands, not text.
        vcx.simulate_input("ggA");
        assert_eq!(vim_mode(&workspace, vcx), Some(Mode::Insert));
        vcx.simulate_input(" at night");
        vcx.simulate_keystrokes("escape");
        assert_eq!(vim_mode(&workspace, vcx), Some(Mode::Normal));
        assert_eq!(
            document(&workspace, vcx).blocks()[0].text(),
            "The Harbour at night"
        );
        // dd on the last line, then undo it.
        vcx.simulate_input("Gdd");
        assert!(!editor_text(&workspace, vcx).contains("Larger"));
        vcx.simulate_input("u");
        assert!(editor_text(&workspace, vcx).ends_with("Larger"));
        // :w saves now.
        assert_eq!(
            workspace.read_with(vcx, |w, _| w.save_state()),
            Some(writer_app::SaveState::Edited)
        );
        vcx.simulate_input(":w");
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
        assert_eq!(
            workspace.read_with(vcx, |w, _| w.save_state()),
            Some(writer_app::SaveState::Saved)
        );
        assert!(stored_text(&library, id, vcx).starts_with("The Harbour at night\n"));
        // A command of the palette by name, and an unknown one.
        vcx.simulate_input(":togglepageview");
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
        assert!(!workspace.read_with(vcx, |w, cx| w.editor().read(cx).page_view()));
        vcx.simulate_input(":nosuchthing");
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
        assert_eq!(
            workspace.read_with(vcx, |w, cx| {
                let status = w.vim().read(cx).status();
                status.message.as_ref().map(|message| message.text.clone())
            }),
            Some("E492: Not an editor command: nosuchthing".to_string())
        );
        vcx.simulate_keystrokes("cmd-alt-p cmd-alt-v");
        vcx.run_until_parked();
        assert_eq!(vim_mode(&workspace, vcx), None);
        assert!(!workspace.read_with(vcx, |w, _| w.settings().vim_mode));
        // Without Vim the same letters are text again.
        vcx.simulate_keystrokes("cmd-down");
        vcx.simulate_input(" gg");
        assert!(editor_text(&workspace, vcx).ends_with("Larger gg"));

        // ----- export ---------------------------------------------------------
        let markdown_path = out.path().join("harbour.md");
        let docx_path = out.path().join("harbour.docx");
        export_row(&workspace, 0, &markdown_path, vcx);
        export_row(&workspace, 1, &docx_path, vcx);
        let markdown = std::fs::read_to_string(&markdown_path).expect("the Markdown file");
        for expected in [
            "# The Harbour at night\n",
            "The ferry leaves at **dawn** and the *gulls* follow it out.\n",
            "## Things to pack\n",
            "- rope\n- salt\n",
            "1. first\n2. second\n",
            "- [ ] ticket\n",
            "> Never trust a calm sea.\n",
            "```\nlet knots = 12;\n```\n",
            "[the port](https://example.com/port)",
            "<mark>bright</mark>",
            "Larger gg\n",
        ] {
            assert!(
                markdown.contains(expected),
                "{expected:?} is in the Markdown:\n{markdown}"
            );
        }
        let docx = std::fs::read(&docx_path).expect("the Word file");
        assert_eq!(&docx[..4], b"PK\x03\x04");
        assert!(docx.len() > 2_000, "a real package, {} bytes", docx.len());
        if let Some(text) = docx_text(&docx_path) {
            for expected in [
                "The Harbour at night",
                "The ferry leaves at dawn and the gulls follow it out.",
                "rope",
                "Never trust a calm sea.",
                "let knots = 12;",
                "A link to the port and bright words in Georgia",
                "Larger gg",
            ] {
                assert!(text.contains(expected), "{expected:?} is in the DOCX:\n{text}");
            }
        }

        // ----- the session ends -----------------------------------------------
        assert!(workspace.update(vcx, |w, cx| w.flush(cx)));
        vcx.update(|window, _| window.remove_window());
        vcx.run_until_parked();
        id
    };

    // ----- another day: the library is opened again ---------------------------
    let library = cx.new(|_| Library::open(&database).expect("the library opens again"));
    let (tree, stored) = library.read_with(cx, |library, _| {
        (library.tree(), library.document(document_id).unwrap())
    });
    assert_eq!(tree.folders.len(), 1);
    assert_eq!(tree.folders[0].folder.name, "Salt Road");
    assert_eq!(tree.folders[0].documents.len(), 1);
    assert_eq!(tree.folders[0].documents[0].id, document_id);
    assert_eq!(tree.folders[0].documents[0].title, "The Harbour at night");
    // The welcome document is beside the folder.
    assert_eq!(tree.documents.len(), 1);
    let reread = Document::from_json(&stored.content).expect("the stored document");
    assert!(reread.plain_text().ends_with("Larger gg"));
    assert_eq!(*block_with(&reread, "rope").kind(), BlockKind::BulletItem);
    assert!(attrs_of(&reread, "dawn").bold);
    assert_eq!(reread.comment_count(), 1);
    let thread = reread.comments().next().unwrap();
    assert_eq!(thread.body, "Which ferry?");
    assert_eq!(reread.text_in(reread.comment_ranges(thread.id)[0]), "The ferry");
    assert!(reread.settings().page_view);
    // Full-text search finds it.
    let hits = library.read_with(cx, |library, _| library.search("gulls", 10).unwrap());
    assert_eq!(hits.len(), 1);

    // The app comes back where it was left: same document, page view, the
    // comment beside the text, Vim mode off as it was switched off.
    let (workspace, vcx) = open_workspace(&library, cx);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(document_id)
    );
    assert!(editor_text(&workspace, vcx).ends_with("Larger gg"));
    assert!(workspace.read_with(vcx, |w, cx| w.editor().read(cx).page_view()));
    assert!(workspace.read_with(vcx, |w, _| w.has_right_panel()));
    assert_eq!(
        workspace.read_with(vcx, |w, cx| w.comments().read(cx).threads().len()),
        1
    );
    assert_eq!(vim_mode(&workspace, vcx), None);
    assert!(editor_focused(&workspace, vcx));
}
