//! The mounted crates working together in one window: what happens to the
//! tools, the comments panel and Vim mode when another document is opened,
//! who gets a key when several of them could take it, and what is stored.

use gpui::TestAppContext;
use writer_app::{palette::CommandPalette, settings::keys};
use writer_core::{EditorState, Position, Selection};
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

fn thread_bodies(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<String> {
    workspace.read_with(cx, |w, cx| {
        w.comments()
            .read(cx)
            .threads()
            .iter()
            .map(|entry| entry.thread.body.clone())
            .collect()
    })
}

/// `simple_document` with a comment on the first word of the paragraph.
fn commented_document(title: &str, body: &str, comment: &str) -> Document {
    let mut state = EditorState::new(simple_document(title, body));
    let first_word = body.find(' ').unwrap_or(body.len());
    state.set_selection(Selection::new(
        Position::new(1, 0),
        Position::new(1, first_word),
    ));
    state
        .add_comment(comment, "Ada", 1_700_000_000_000)
        .expect("a comment on the selection");
    state.into_document()
}

#[gpui::test]
fn another_document_takes_over_tools_comments_and_vim(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let alpha = add_document(
        &library,
        None,
        &commented_document("Alpha", "Salt is dear this year.", "How dear?"),
        cx,
    );
    let beta = add_document(
        &library,
        None,
        &simple_document("Beta", "No salt here, only sand, and salt again."),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(alpha, true, window, cx);
    });
    vcx.run_until_parked();

    // Alpha: its thread is beside the text.
    assert_eq!(thread_bodies(&workspace, vcx), vec!["How dear?"]);
    assert!(workspace.read_with(vcx, |w, _| w.has_right_panel()));

    // Vim on, an operator half typed, the find bar open on "salt".
    vcx.simulate_keystrokes("cmd-alt-v");
    assert_eq!(vim_mode(&workspace, vcx), Some(Mode::Normal));
    vcx.simulate_keystrokes("cmd-f");
    vcx.simulate_input("salt");
    assert_eq!(tool(&workspace, vcx), Some(ToolKind::Find));
    assert_eq!(
        workspace.read_with(vcx, |w, cx| w.tools().read(cx).find_match_count()),
        1
    );

    // Quick open works from the find bar; Beta opens with the keyboard in
    // its text.
    vcx.simulate_keystrokes("cmd-p");
    vcx.simulate_input("Beta");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(beta)
    );
    assert!(editor_focused(&workspace, vcx));
    // The find bar searched the new text.
    assert_eq!(tool(&workspace, vcx), Some(ToolKind::Find));
    assert_eq!(
        workspace.read_with(vcx, |w, cx| w.tools().read(cx).find_match_count()),
        2
    );
    assert_eq!(
        workspace.read_with(vcx, |w, cx| {
            w.editor()
                .read(cx)
                .highlights(writer_editor_tools::FIND_LAYER)
                .len()
        }),
        2
    );
    // No comments here: the panel is gone.
    assert!(thread_bodies(&workspace, vcx).is_empty());
    assert!(!workspace.read_with(vcx, |w, _| w.has_right_panel()));
    // Vim is in Normal mode on the new document: x deletes a character.
    assert_eq!(vim_mode(&workspace, vcx), Some(Mode::Normal));
    let before = editor_text(&workspace, vcx);
    vcx.simulate_input("x");
    assert_eq!(editor_text(&workspace, vcx).len(), before.len() - 1);
    vcx.simulate_input("u");
    assert_eq!(editor_text(&workspace, vcx), before);

    // A comment made here belongs to Beta.
    vcx.simulate_input("ggjviw");
    assert_eq!(vim_mode(&workspace, vcx), Some(Mode::Visual));
    vcx.simulate_keystrokes("cmd-alt-m");
    vcx.simulate_input("A note for Beta");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(thread_bodies(&workspace, vcx), vec!["A note for Beta"]);
    assert!(workspace.read_with(vcx, |w, _| w.has_right_panel()));

    // Back to Alpha: its own thread again, Beta's edits are stored.
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(alpha, true, window, cx);
    });
    vcx.run_until_parked();
    assert_eq!(thread_bodies(&workspace, vcx), vec!["How dear?"]);
    assert_eq!(vim_mode(&workspace, vcx), Some(Mode::Normal));
    assert_eq!(
        workspace.read_with(vcx, |w, cx| w.tools().read(cx).find_match_count()),
        1
    );
    let stored = library.read_with(vcx, |library, _| library.document(beta).unwrap().content);
    assert_eq!(Document::from_json(&stored).unwrap().comment_count(), 1);

    // No document at all (the open one goes to the trash, then the other):
    // nothing is left over from the last one.
    vcx.simulate_keystrokes("escape");
    workspace.update_in(vcx, |w, window, cx| w.close_document(window, cx));
    vcx.run_until_parked();
    assert!(thread_bodies(&workspace, vcx).is_empty());
    assert!(!workspace.read_with(vcx, |w, _| w.has_right_panel()));
    // App keys still work from the empty state.
    vcx.simulate_keystrokes("cmd-p");
    vcx.simulate_input("Alpha");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(alpha)
    );
    assert_eq!(thread_bodies(&workspace, vcx), vec!["How dear?"]);
}

#[gpui::test]
fn an_open_menu_goes_first_then_vim_then_the_editor(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    add_document(
        &library,
        None,
        &simple_document("Order", "One line of text."),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    vcx.simulate_keystrokes("cmd-alt-v");
    assert_eq!(vim_mode(&workspace, vcx), Some(Mode::Normal));

    // Normal mode: "/" is Vim's search, not the insert menu.
    vcx.simulate_input("/");
    assert_eq!(vim_mode(&workspace, vcx), Some(Mode::CommandLine));
    assert_eq!(tool(&workspace, vcx), None);
    vcx.simulate_keystrokes("escape");
    assert_eq!(vim_mode(&workspace, vcx), Some(Mode::Normal));

    // Insert mode: "/" at the start of a new line opens the insert menu.
    // Escape closes the menu first and leaves Insert mode on the next
    // press; the arrow keys belong to the menu while it is open.
    vcx.simulate_input("Go/");
    assert_eq!(vim_mode(&workspace, vcx), Some(Mode::Insert));
    assert_eq!(tool(&workspace, vcx), Some(ToolKind::Insert));
    let first = workspace.read_with(vcx, |w, cx| w.tools().read(cx).selected_menu_label(cx));
    vcx.simulate_keystrokes("down");
    let second = workspace.read_with(vcx, |w, cx| w.tools().read(cx).selected_menu_label(cx));
    assert_ne!(first, second);
    vcx.simulate_keystrokes("escape");
    assert_eq!(tool(&workspace, vcx), None);
    assert_eq!(vim_mode(&workspace, vcx), Some(Mode::Insert));
    vcx.simulate_keystrokes("escape");
    assert_eq!(vim_mode(&workspace, vcx), Some(Mode::Normal));
    assert!(editor_text(&workspace, vcx).ends_with("\n/"));
    vcx.simulate_input("dd");
    assert_eq!(editor_text(&workspace, vcx), "Order\nOne line of text.");

    // cmd shortcuts pass Vim. The format menu takes the keyboard: letters
    // filter it, they are not Vim commands, and escape closes only it.
    vcx.simulate_keystrokes("cmd-.");
    assert_eq!(tool(&workspace, vcx), Some(ToolKind::Format));
    vcx.simulate_input("dd");
    assert_eq!(editor_text(&workspace, vcx), "Order\nOne line of text.");
    vcx.simulate_keystrokes("escape");
    assert_eq!(tool(&workspace, vcx), None);
    assert!(editor_focused(&workspace, vcx));
    assert_eq!(vim_mode(&workspace, vcx), Some(Mode::Normal));

    // The find bar likewise; escape returns to the text in Normal mode.
    vcx.simulate_keystrokes("cmd-f");
    vcx.simulate_input("jjj");
    assert_eq!(
        workspace.read_with(vcx, |w, cx| w.tools().read(cx).find_query().to_string()),
        "jjj"
    );
    vcx.simulate_keystrokes("escape");
    assert!(editor_focused(&workspace, vcx));
    assert_eq!(vim_mode(&workspace, vcx), Some(Mode::Normal));

    // An editor shortcut in Normal mode reaches the editor: bold on the
    // Visual selection.
    vcx.simulate_input("ggjviw");
    vcx.simulate_keystrokes("cmd-b escape");
    let bold = workspace.read_with(vcx, |w, cx| {
        w.editor().read(cx).document().blocks()[1]
            .attrs_at(0)
            .is_some_and(|attrs| attrs.bold)
    });
    assert!(bold);

    // A modal over a tool that has the keyboard: the tool closes, and when
    // the modal closes the keyboard is back in the text, not nowhere.
    vcx.simulate_keystrokes("cmd-.");
    assert_eq!(tool(&workspace, vcx), Some(ToolKind::Format));
    vcx.simulate_keystrokes("cmd-shift-p");
    vcx.run_until_parked();
    assert!(
        workspace
            .read_with(vcx, |w, cx| w
                .modal_layer()
                .read(cx)
                .active::<CommandPalette>())
            .is_some()
    );
    assert_eq!(tool(&workspace, vcx), None);
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(editor_focused(&workspace, vcx));
    assert_eq!(vim_mode(&workspace, vcx), Some(Mode::Normal));

    // A palette command runs in the text although Vim is on.
    vcx.simulate_keystrokes("cmd-shift-p");
    vcx.simulate_input("heading 3");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(
        workspace.read_with(vcx, |w, cx| w.editor().read(cx).document().blocks()[1]
            .kind()
            .heading_level()),
        Some(3)
    );
}

#[gpui::test]
fn view_toggles_are_stored_and_come_back(cx: &mut TestAppContext) {
    init(cx);
    cx.update(|cx| writer_editor_tools::set_font_families(["Charter", "Georgia"], cx));
    let library = library(cx);
    let id = add_document(
        &library,
        None,
        &commented_document("Kept", "Words to keep.", "A resolved note"),
        cx,
    );
    {
        let (workspace, vcx) = open_workspace(&library, cx);
        assert_eq!(vim_mode(&workspace, vcx), None);

        // Vim through its key, comments through theirs, page view, "show
        // resolved" in the panel, a font in the picker.
        vcx.simulate_keystrokes("cmd-alt-v cmd-alt-p");
        assert_eq!(vim_mode(&workspace, vcx), Some(Mode::Normal));
        vcx.simulate_keystrokes("cmd-2 x s");
        assert!(workspace.read_with(vcx, |w, cx| w.comments().read(cx).show_resolved()));
        vcx.simulate_keystrokes("escape cmd-alt-c");
        vcx.run_until_parked();
        assert!(!workspace.read_with(vcx, |w, _| w.settings().comments_visible));
        vcx.simulate_keystrokes("cmd-t");
        vcx.simulate_input("geo");
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();

        workspace.update(vcx, |w, cx| w.flush(cx));
        vcx.update(|window, _| window.remove_window());
        vcx.run_until_parked();
    }
    let stored: (bool, bool, bool, Vec<String>) = library.read_with(cx, |library, _| {
        (
            library.setting_or(keys::VIM_MODE, false),
            library.setting_or(keys::COMMENTS_VISIBLE, true),
            library.setting_or(keys::COMMENTS_SHOW_RESOLVED, false),
            library.setting_or(keys::RECENT_FONTS, Vec::new()),
        )
    });
    assert_eq!(stored, (true, false, true, vec!["Georgia".to_string()]));
    cx.update(|cx| writer_editor_tools::set_recent_fonts(Vec::<String>::new(), cx));

    // The next window starts as the last one ended.
    let (workspace, vcx) = open_workspace(&library, cx);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(id)
    );
    assert_eq!(vim_mode(&workspace, vcx), Some(Mode::Normal));
    assert!(workspace.read_with(vcx, |w, cx| w.editor().read(cx).page_view()));
    assert!(!workspace.read_with(vcx, |w, cx| w.comments().read(cx).is_visible()));
    assert!(!workspace.read_with(vcx, |w, cx| w.editor().read(cx).config().comments_visible));
    assert!(!workspace.read_with(vcx, |w, _| w.has_right_panel()));
    assert!(workspace.read_with(vcx, |w, cx| w.comments().read(cx).show_resolved()));
    assert_eq!(
        vcx.update(|_, cx| writer_editor_tools::recent_fonts(cx)),
        vec![gpui::SharedString::from("Georgia")]
    );

    // The settings modal and the panel agree: switching comments on there
    // brings the panel back (the thread is resolved, and resolved threads
    // are shown).
    workspace.update(vcx, |w, cx| w.toggle_comments(cx));
    vcx.run_until_parked();
    assert!(workspace.read_with(vcx, |w, cx| w.comments().read(cx).is_visible()));
    assert!(workspace.read_with(vcx, |w, _| w.has_right_panel()));
    // And Vim off through the setting detaches.
    workspace.update(vcx, |w, cx| w.toggle_vim_mode(cx));
    vcx.run_until_parked();
    assert_eq!(vim_mode(&workspace, vcx), None);
}
