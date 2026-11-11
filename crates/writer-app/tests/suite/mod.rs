//! Shared helpers. Every test runs against an in-memory or temp-dir library,
//! never the user's data directory.

mod assembly;
mod commands;
mod export;
mod keymap;
mod session;
mod settings;
mod story;
mod shell;
mod units;

use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use writer_app::{Workspace, document_io::StoredForm};
use writer_core::{Block, BlockKind, Document};
use writer_library::{DocumentId, FolderId, Library};

/// App-wide setup, once per test.
pub fn init(cx: &mut TestAppContext) {
    cx.update(writer_app::init);
}

/// An in-memory library entity that has already seen its first run (so no
/// welcome document appears unless a test wants it).
pub fn library(cx: &mut TestAppContext) -> Entity<Library> {
    let library = cx.new(|_| Library::in_memory().expect("in-memory library"));
    library.update(cx, |library, cx| {
        library
            .set_setting(writer_app::settings::keys::WELCOME_CREATED, &true, cx)
            .unwrap();
    });
    library
}

/// Opens a workspace window over `library`.
pub fn open_workspace<'a>(
    library: &Entity<Library>,
    cx: &'a mut TestAppContext,
) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    let library = library.clone();
    let (workspace, cx) = cx.add_window_view(|window, cx| {
        window.activate_window();
        Workspace::new(library, window, cx)
    });
    cx.run_until_parked();
    (workspace, cx)
}

/// A document with a heading and one paragraph.
pub fn simple_document(title: &str, body: &str) -> Document {
    Document::from_blocks(vec![
        Block::with_text(BlockKind::heading(1), title),
        Block::paragraph(body),
    ])
}

/// Stores `document` in `folder` and returns its id.
pub fn add_document(
    library: &Entity<Library>,
    folder: Option<FolderId>,
    document: &Document,
    cx: &mut TestAppContext,
) -> DocumentId {
    let form = StoredForm::of(document, None);
    library.update(cx, |library, cx| {
        library
            .create_document(folder, &form.content(), cx)
            .expect("create document")
            .id
    })
}

/// The stored text of document `id`.
pub fn stored_text(library: &Entity<Library>, id: DocumentId, cx: &mut TestAppContext) -> String {
    let content = library.read_with(cx, |library, _| library.document(id).unwrap().content);
    Document::from_json(&content).unwrap().plain_text()
}

/// The text in the editor.
pub fn editor_text(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> String {
    workspace.read_with(cx, |workspace, cx| {
        workspace.editor().read(cx).document().plain_text()
    })
}

/// Whether the editor has focus.
pub fn editor_focused(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> bool {
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.editor().read(cx).is_focused(window)
    })
}

/// Inserts `text` at the caret through the editor's mutation seam.
pub fn type_text(workspace: &Entity<Workspace>, text: &str, cx: &mut VisualTestContext) {
    let editor = workspace.read_with(cx, |workspace, _| workspace.editor().clone());
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.insert_text(text), cx);
    });
    cx.run_until_parked();
}
