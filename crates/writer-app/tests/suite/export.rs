//! Export: the picker, files written through the save panel, clipboard.

use gpui::TestAppContext;
use writer_app::export::{
    ExportChoice, ExportDestination, ExportPicker, ExportScope, LIBRARY_ROOT_NAME,
};
use writer_export::ExportFormat;

use super::*;

fn choice(
    scope: ExportScope,
    format: ExportFormat,
    destination: ExportDestination,
) -> ExportChoice {
    ExportChoice::new(scope, format, destination)
}

fn toast_count(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> usize {
    workspace.read_with(cx, |w, cx| w.toast_layer().read(cx).toasts().count())
}

#[gpui::test]
fn export_writes_files_and_copies(cx: &mut TestAppContext) {
    init(cx);
    let out = tempfile::tempdir().unwrap();
    let library = library(cx);
    let folder = library.update(cx, |library, cx| {
        library.create_folder(None, "Novel", cx).unwrap().id
    });
    let one = add_document(
        &library,
        Some(folder),
        &simple_document("Chapter 1: Arrival", "The ferry came in late."),
        cx,
    );
    add_document(
        &library,
        Some(folder),
        &simple_document("Chapter 2", "By morning the fog had gone."),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    workspace.update_in(vcx, |workspace, window, cx| {
        workspace.open_document(one, true, window, cx);
    });
    // An unsaved edit must be part of the export.
    type_text(&workspace, "Fresh. ", vcx);

    // The picker: five ways for the document, three for the folder.
    vcx.simulate_keystrokes("cmd-shift-s");
    let picker = workspace
        .read_with(vcx, |w, cx| {
            w.modal_layer().read(cx).active::<ExportPicker>()
        })
        .expect("the export picker is open");
    let choices = picker.read_with(vcx, |picker, _| picker.delegate.choices());
    assert_eq!(choices, ExportChoice::ALL.to_vec());
    assert_eq!(
        workspace.read_with(vcx, |w, cx| w.export_folder_name(cx)),
        "Novel"
    );

    // Enter on the first row: Markdown file, through the save panel.
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert!(vcx.did_prompt_for_new_path());
    let target = out.path().join("picked name");
    vcx.simulate_new_path_selection(|_| Some(target.clone()));
    vcx.run_until_parked();
    let written = out.path().join("picked name.md");
    let markdown = std::fs::read_to_string(&written).expect("the Markdown file exists");
    assert!(
        markdown.starts_with("# Fresh. Chapter 1: Arrival"),
        "{markdown}"
    );
    assert!(markdown.contains("The ferry came in late."));
    assert_eq!(toast_count(&workspace, vcx), 1);
    assert!(editor_focused(&workspace, vcx));

    // Word document.
    workspace.update_in(vcx, |workspace, window, cx| {
        workspace.export(
            choice(
                ExportScope::Document,
                ExportFormat::Docx,
                ExportDestination::File,
            ),
            window,
            cx,
        );
    });
    vcx.run_until_parked();
    let suggested = std::rc::Rc::new(std::cell::RefCell::new(None));
    vcx.simulate_new_path_selection({
        let suggested = suggested.clone();
        let out = out.path().to_path_buf();
        move |directory| {
            *suggested.borrow_mut() = Some(directory.to_path_buf());
            Some(out.join("book.docx"))
        }
    });
    vcx.run_until_parked();
    let bytes = std::fs::read(out.path().join("book.docx")).unwrap();
    assert_eq!(&bytes[..2], b"PK");
    // The save panel starts where the last export went.
    assert_eq!(suggested.borrow().as_deref(), Some(out.path()));

    // The whole folder as one plain text file.
    workspace.update_in(vcx, |workspace, window, cx| {
        workspace.export(
            choice(
                ExportScope::Folder,
                ExportFormat::PlainText,
                ExportDestination::File,
            ),
            window,
            cx,
        );
    });
    vcx.run_until_parked();
    vcx.simulate_new_path_selection(|_| Some(out.path().join("novel.txt")));
    vcx.run_until_parked();
    let text = std::fs::read_to_string(out.path().join("novel.txt")).unwrap();
    assert!(text.contains("The ferry came in late."));
    assert!(text.contains("By morning the fog had gone."));
    assert!(
        text.find("ferry").unwrap() < text.find("fog").unwrap(),
        "library order is kept"
    );

    // Clipboard.
    workspace.update_in(vcx, |workspace, window, cx| {
        workspace.export(
            choice(
                ExportScope::Document,
                ExportFormat::PlainText,
                ExportDestination::Clipboard,
            ),
            window,
            cx,
        );
    });
    let clipboard = vcx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap();
    assert!(clipboard.contains("The ferry came in late."));
    assert!(!clipboard.contains('#'));

    // Four exports, four notes; the palette command clears them.
    assert_eq!(toast_count(&workspace, vcx), 4);
    vcx.dispatch_action(writer_app::actions::DismissNotifications);
    vcx.run_until_parked();

    // Cancelling the save panel writes nothing and says nothing.
    let toasts = toast_count(&workspace, vcx);
    assert_eq!(toasts, 0);
    workspace.update_in(vcx, |workspace, window, cx| {
        workspace.export(
            choice(
                ExportScope::Document,
                ExportFormat::Markdown,
                ExportDestination::File,
            ),
            window,
            cx,
        );
    });
    vcx.run_until_parked();
    vcx.simulate_new_path_selection(|_| None);
    vcx.run_until_parked();
    assert_eq!(toast_count(&workspace, vcx), toasts);

    // A path that cannot be written is an error toast, not silence.
    workspace.update_in(vcx, |workspace, window, cx| {
        workspace.export(
            choice(
                ExportScope::Document,
                ExportFormat::Markdown,
                ExportDestination::File,
            ),
            window,
            cx,
        );
    });
    vcx.run_until_parked();
    vcx.simulate_new_path_selection(|_| Some(out.path().join("no such dir").join("x.md")));
    vcx.run_until_parked();
    assert_eq!(toast_count(&workspace, vcx), toasts + 1);
    assert!(!out.path().join("no such dir").exists());
}

#[gpui::test]
fn export_of_a_top_level_document_names_the_library(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    add_document(&library, None, &simple_document("Loose", "One."), cx);
    add_document(&library, None, &simple_document("Leaf", "Two."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    assert_eq!(
        workspace.read_with(vcx, |w, cx| w.export_folder_name(cx)),
        LIBRARY_ROOT_NAME
    );
    let prepared = workspace
        .update(vcx, |workspace, cx| {
            workspace.prepare_export(ExportScope::Folder, ExportFormat::Markdown, cx)
        })
        .unwrap();
    assert_eq!(prepared.documents, 2);
    assert_eq!(prepared.file_name, "Library.md");
    let text = String::from_utf8(prepared.bytes).unwrap();
    assert!(text.contains("One.") && text.contains("Two."));

    let single = workspace
        .update(vcx, |workspace, cx| {
            workspace.prepare_export(ExportScope::Document, ExportFormat::Docx, cx)
        })
        .unwrap();
    assert_eq!(single.documents, 1);
    assert!(single.file_name.ends_with(".docx"));

    // The keyboard path to "Copy as Markdown": fourth row of the picker.
    vcx.simulate_keystrokes("cmd-shift-s down down down enter");
    vcx.run_until_parked();
    let clipboard = vcx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap();
    assert!(clipboard.starts_with("# Leaf"), "{clipboard}");
    assert_eq!(toast_count(&workspace, vcx), 1);
    assert!(editor_focused(&workspace, vcx));
    // Typing filters the picker: "word" leaves the two Word rows.
    vcx.simulate_keystrokes("cmd-shift-s");
    vcx.simulate_input("word");
    let picker = workspace
        .read_with(vcx, |w, cx| {
            w.modal_layer().read(cx).active::<ExportPicker>()
        })
        .unwrap();
    let filtered = picker.read_with(vcx, |picker, _| picker.delegate.choices());
    assert_eq!(filtered.len(), 2);
    assert!(
        filtered
            .iter()
            .all(|choice| choice.format == ExportFormat::Docx)
    );
    vcx.simulate_keystrokes("escape");

    // Without a document there is nothing to export, and no picker.
    workspace.update_in(vcx, |workspace, window, cx| {
        workspace.close_document(window, cx);
    });
    vcx.simulate_keystrokes("cmd-shift-s");
    assert!(!workspace.read_with(vcx, |w, cx| w.modal_layer().read(cx).has_modal()));
}
