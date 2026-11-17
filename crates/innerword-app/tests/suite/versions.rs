//! Version history: the list, the preview, restoring and taking it back.

use gpui::{AppContext as _, TestAppContext};
use innerword_app::{settings::keys, version_history::VersionHistory, workspace::AUTOSAVE_DELAY};
use innerword_library::{Clock, Library};

use super::*;

const MINUTE: i64 = 60_000;

fn history(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
) -> Option<Entity<VersionHistory>> {
    workspace.read_with(cx, |workspace, cx| {
        workspace.modal_layer().read(cx).active::<VersionHistory>()
    })
}

fn open_history(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
) -> Entity<VersionHistory> {
    cx.simulate_keystrokes("cmd-shift-p");
    cx.simulate_input("version history");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    history(workspace, cx).expect("the version history is open")
}

#[gpui::test]
fn version_history_lists_previews_and_restores(cx: &mut TestAppContext) {
    init(cx);
    // A clock the test moves, so versions are taken when it says.
    let clock = Clock::manual(1_800_000_000_000);
    let library = cx.new(|_| Library::in_memory_with_clock(clock.clone()).unwrap());
    library.update(cx, |library, cx| {
        library
            .set_setting(keys::WELCOME_CREATED, &true, cx)
            .unwrap();
    });
    let id = add_document(&library, None, &simple_document("Chapter", "Draft."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    let save = |text: &str, cx: &mut VisualTestContext| {
        type_text(&workspace, text, cx);
        cx.executor().advance_clock(AUTOSAVE_DELAY * 2);
        cx.run_until_parked();
    };
    let versions = |cx: &mut VisualTestContext| {
        library.read_with(cx, |library, _| library.snapshots(id).unwrap().len())
    };

    // Three versions, twenty minutes apart, then a little more text that
    // is too fresh for a version of its own.
    save("One. ", vcx);
    let one = editor_text(&workspace, vcx);
    clock.advance_ms(20 * MINUTE);
    save("Two. ", vcx);
    let two = editor_text(&workspace, vcx);
    clock.advance_ms(20 * MINUTE);
    save("Three. ", vcx);
    let three = editor_text(&workspace, vcx);
    assert_eq!(versions(vcx), 3);
    clock.advance_ms(MINUTE);
    type_text(&workspace, "Four. ", vcx);
    let four = editor_text(&workspace, vcx);
    assert_eq!(four, "One. Two. Three. Four. Chapter\nDraft.");

    // From the palette. The newest version is selected and previewed.
    let view = open_history(&workspace, vcx);
    assert!(!editor_focused(&workspace, vcx));
    // Opening saved what was on screen, so nothing typed can be lost.
    assert_eq!(stored_text(&library, id, vcx), four);
    let rows = view.read_with(vcx, |view, _| view.row_labels());
    assert_eq!(rows.len(), 3);
    assert!(rows[0].0.starts_with("Today "), "{rows:?}");
    assert_eq!(rows[0].1, "1 min ago \u{b7} 5 words");
    assert_eq!(rows[1].1, "21 min ago \u{b7} 4 words");
    assert_eq!(rows[2].1, "41 min ago \u{b7} 3 words");
    let preview = |cx: &mut VisualTestContext| view.read_with(cx, |view, _| view.preview_text());
    assert_eq!(preview(vcx), three);

    // The keyboard walks the list; the preview follows.
    vcx.simulate_keystrokes("down");
    assert_eq!(preview(vcx), two);
    vcx.simulate_keystrokes("end");
    assert_eq!(preview(vcx), one);
    vcx.simulate_keystrokes("down");
    assert_eq!(preview(vcx), one, "the list ends there");
    vcx.simulate_keystrokes("home ctrl-n");
    assert_eq!(preview(vcx), two);
    // Scrolling the preview changes neither the choice nor the text.
    vcx.simulate_keystrokes("space shift-space pagedown pageup");
    assert_eq!(preview(vcx), two);
    assert_eq!(editor_text(&workspace, vcx), four);

    // Enter restores: in the editor, in the library, with the keyboard
    // back in the text.
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert!(history(&workspace, vcx).is_none());
    assert!(editor_focused(&workspace, vcx));
    assert_eq!(editor_text(&workspace, vcx), two);
    assert_eq!(stored_text(&library, id, vcx), two);
    let meta = library.read_with(vcx, |library, _| library.document_meta(id).unwrap());
    assert_eq!(meta.title, "One. Two. Chapter");
    assert_eq!(meta.word_count, 4);
    let toasts = workspace.read_with(vcx, |w, cx| {
        w.toast_layer()
            .read(cx)
            .toasts()
            .map(|toast| toast.message.to_string())
            .collect::<Vec<_>>()
    });
    assert!(
        toasts
            .iter()
            .any(|message| message.starts_with("Restored Today ")),
        "{toasts:?}"
    );

    // The text from before the restore became a version, so the step can
    // be taken back the same way.
    assert_eq!(versions(vcx), 4);
    let view = open_history(&workspace, vcx);
    let preview = |cx: &mut VisualTestContext| view.read_with(cx, |view, _| view.preview_text());
    assert_eq!(preview(vcx), four);
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(editor_text(&workspace, vcx), four);
    assert_eq!(stored_text(&library, id, vcx), four);
    // Typing goes on from there and is saved as usual.
    type_text(&workspace, "Five. ", vcx);
    vcx.executor().advance_clock(AUTOSAVE_DELAY * 2);
    vcx.run_until_parked();
    assert!(stored_text(&library, id, vcx).contains("Five. "));
    assert!(stored_text(&library, id, vcx).contains("Four. "));

    // Escape closes without changing anything.
    let before = editor_text(&workspace, vcx);
    open_history(&workspace, vcx);
    vcx.simulate_keystrokes("down escape");
    assert!(history(&workspace, vcx).is_none());
    assert!(editor_focused(&workspace, vcx));
    assert_eq!(editor_text(&workspace, vcx), before);
}

#[gpui::test]
fn a_version_an_agent_left_says_why_it_was_kept(cx: &mut TestAppContext) {
    init(cx);
    // The library as the `innerword` tool leaves it: a version kept before an agent's
    // edit, with its label, and one kept while writing, without.
    let clock = Clock::manual(1_800_000_000_000);
    let mut store = innerword_library::store::Store::open_in_memory_with_clock(clock.clone()).unwrap();
    let form = StoredForm::of(&simple_document("Chapter", "Draft."), None);
    let id = store.create_document(None, &form.content()).unwrap().id;
    store.create_snapshot(id).unwrap();
    clock.advance_ms(20 * MINUTE);
    let labelled = store
        .create_labeled_snapshot(id, "Before an edit by Claude")
        .unwrap();
    clock.advance_ms(MINUTE);
    let library = cx.new(|_| Library::from_store(store).unwrap());
    library.update(cx, |library, cx| {
        library
            .set_setting(keys::WELCOME_CREATED, &true, cx)
            .unwrap();
    });
    let (workspace, vcx) = open_workspace(&library, cx);
    workspace.update_in(vcx, |workspace, window, cx| {
        workspace.open_document(id, true, window, cx);
    });
    let view = open_history(&workspace, vcx);
    let rows = view.read_with(vcx, |view, _| view.row_labels());
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(
        view.read_with(vcx, |view, _| view.versions()[0].id),
        labelled.id
    );
    assert!(
        rows[0].0.starts_with("Today ") && rows[0].0.ends_with(" \u{b7} 2 words"),
        "{rows:?}"
    );
    assert_eq!(rows[0].1, "Before an edit by Claude");
    assert!(rows[1].0.starts_with("Today ") && !rows[1].0.contains("words"));
    assert_eq!(rows[1].1, "21 min ago \u{b7} 2 words");
}

#[gpui::test]
fn version_history_without_versions_or_document(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    // No document: nothing to list, and the app says why.
    workspace.update_in(vcx, |workspace, window, cx| {
        workspace.toggle_version_history(window, cx)
    });
    assert!(history(&workspace, vcx).is_none());
    let toasts = workspace.read_with(vcx, |w, cx| w.toast_layer().read(cx).toasts().count());
    assert_eq!(toasts, 1);

    // A document that was never edited has no versions: an empty state,
    // enter does nothing, escape closes.
    let id = add_document(
        &library,
        None,
        &simple_document("Fresh", "Nothing yet."),
        vcx,
    );
    workspace.update_in(vcx, |workspace, window, cx| {
        workspace.open_document(id, true, window, cx);
    });
    let view = open_history(&workspace, vcx);
    assert!(view.read_with(vcx, |view, _| view.versions().is_empty()));
    assert_eq!(view.read_with(vcx, |view, _| view.preview_text()), "");
    vcx.simulate_keystrokes("down enter");
    vcx.run_until_parked();
    assert!(history(&workspace, vcx).is_some());
    assert_eq!(editor_text(&workspace, vcx), "Fresh\nNothing yet.");
    vcx.simulate_keystrokes("escape");
    assert!(history(&workspace, vcx).is_none());
    assert!(editor_focused(&workspace, vcx));
}
