//! The keymap as a whole: every crate's bindings together, resolved the way
//! GPUI resolves a key press for the element that has the keyboard.
//!
//! The tables are the "Default keymap" of CLAUDE.md. A key that resolves to
//! another action here is shadowed in the running app.

use gpui::{Keystroke, TestAppContext};
use innerword_editor_tools::ToolKind;

use super::*;

/// Keys that work anywhere in the window.
pub(super) const GLOBAL: &[(&str, &str)] = &[
    ("cmd-shift-p", "workspace::ToggleCommandPalette"),
    ("cmd-p", "workspace::QuickOpen"),
    ("cmd-n", "workspace::NewDocument"),
    ("cmd-shift-n", "workspace::NewFolder"),
    ("cmd-0", "workspace::FocusLibrary"),
    ("cmd-1", "workspace::FocusEditor"),
    ("cmd-2", "comments::FocusComments"),
    ("cmd-alt-s", "workspace::ToggleSidebar"),
    ("cmd-alt-right", "workspace::WidenSidebar"),
    ("cmd-alt-left", "workspace::NarrowSidebar"),
    ("cmd-shift-f", "workspace::SearchLibrary"),
    ("cmd-shift-s", "workspace::Export"),
    ("cmd-s", "workspace::SaveNow"),
    ("cmd-,", "workspace::OpenSettings"),
    ("cmd-/", "workspace::ShowShortcuts"),
    ("cmd-alt-p", "workspace::TogglePageView"),
    ("cmd-alt-v", "workspace::ToggleVimMode"),
    ("cmd-alt-a", "workspace::ShowAgents"),
    ("cmd-j", "workspace::AskAssistant"),
    ("cmd-3", "workspace::FocusAssistant"),
    ("cmd-alt-c", "comments::ToggleComments"),
    ("cmd-alt-m", "comments::AddComment"),
    ("cmd-alt-]", "comments::NextComment"),
    ("cmd-alt-[", "comments::PreviousComment"),
    ("cmd-q", "app::Quit"),
    ("cmd-w", "app::CloseWindow"),
    // The zoom of the text. Zoom in is the plus key with `cmd`, with or
    // without shift: a real keyboard reports the one with shift as
    // `cmd-+`, a script spells it out. `cmd-0` stays "Go to library":
    // actual size is the same key with shift.
    ("cmd-=", "zoom::ZoomIn"),
    ("cmd-+", "zoom::ZoomIn"),
    ("cmd-shift-=", "zoom::ZoomIn"),
    ("cmd--", "zoom::ZoomOut"),
    ("cmd-shift-0", "zoom::ActualSize"),
    ("cmd-)", "zoom::ActualSize"),
];

/// Keys of the text, with the keyboard in the editor.
pub(super) const EDITOR: &[(&str, &str)] = &[
    ("cmd-b", "editor::ToggleBold"),
    ("cmd-i", "editor::ToggleItalic"),
    ("cmd-u", "editor::ToggleUnderline"),
    ("cmd-shift-x", "editor::ToggleStrikethrough"),
    ("cmd-e", "editor::ToggleInlineCode"),
    ("cmd-shift-h", "editor::ToggleHighlight"),
    ("cmd-k", "editor_tools::OpenLink"),
    ("cmd-shift-k", "editor_tools::RemoveLink"),
    ("cmd-\\", "editor::ClearFormatting"),
    ("cmd-alt-0", "editor::SetParagraph"),
    ("cmd-alt-1", "editor::SetHeading1"),
    ("cmd-alt-2", "editor::SetHeading2"),
    ("cmd-alt-3", "editor::SetHeading3"),
    ("cmd-alt-4", "editor::SetHeading4"),
    ("cmd-alt-5", "editor::SetHeading5"),
    ("cmd-alt-6", "editor::SetHeading6"),
    ("cmd-shift-7", "editor::ToggleNumberedList"),
    ("cmd-shift-8", "editor::ToggleBulletList"),
    ("cmd-shift-9", "editor::ToggleTodoList"),
    ("cmd-'", "editor::ToggleQuote"),
    ("cmd-shift-'", "editor::ToggleCodeBlock"),
    ("tab", "editor::Indent"),
    ("shift-tab", "editor::Outdent"),
    ("cmd-]", "editor::Indent"),
    ("cmd-[", "editor::Outdent"),
    ("alt-up", "editor::MoveBlockUp"),
    ("alt-down", "editor::MoveBlockDown"),
    ("enter", "editor::Newline"),
    ("shift-enter", "editor::InsertSoftBreak"),
    ("cmd-enter", "editor::InsertPageBreak"),
    ("cmd-shift-enter", "editor::ToggleTodoChecked"),
    ("cmd-shift-l", "editor::AlignLeft"),
    ("cmd-shift-e", "editor::AlignCenter"),
    ("cmd-shift-r", "editor::AlignRight"),
    ("cmd-shift-j", "editor::AlignJustify"),
    ("cmd-shift-.", "editor::IncreaseTextSize"),
    ("cmd-shift-,", "editor::DecreaseTextSize"),
    ("cmd-t", "editor_tools::OpenFontPicker"),
    ("cmd-.", "editor_tools::OpenFormatMenu"),
    ("cmd-f", "editor_tools::Find"),
    ("cmd-alt-f", "editor_tools::FindAndReplace"),
    ("cmd-g", "editor_tools::FindNext"),
    ("cmd-shift-g", "editor_tools::FindPrevious"),
    ("cmd-;", "spell::NextMisspelling"),
    ("cmd-shift-;", "spell::PreviousMisspelling"),
    ("cmd-z", "editor::Undo"),
    ("cmd-shift-z", "editor::Redo"),
    ("cmd-a", "editor::SelectAll"),
    ("cmd-c", "editor::Copy"),
    ("cmd-x", "editor::Cut"),
    ("cmd-v", "editor::Paste"),
    ("cmd-shift-v", "editor::PasteAsPlainText"),
    // The context menu of the text, from the keyboard.
    ("ctrl-enter", "editor::OpenContextMenu"),
    ("shift-f10", "editor::OpenContextMenu"),
    // The same keys as a real US keyboard reports them: GPUI names shift
    // plus a digit or punctuation by the shifted character.
    ("cmd-&", "editor::ToggleNumberedList"),
    ("cmd-*", "editor::ToggleBulletList"),
    ("cmd-(", "editor::ToggleTodoList"),
    ("cmd-\"", "editor::ToggleCodeBlock"),
    ("cmd->", "editor::IncreaseTextSize"),
    ("cmd-<", "editor::DecreaseTextSize"),
    ("cmd-:", "spell::PreviousMisspelling"),
    // macOS text navigation and the Emacs set.
    ("left", "editor::MoveLeft"),
    ("right", "editor::MoveRight"),
    ("up", "editor::MoveUp"),
    ("down", "editor::MoveDown"),
    ("alt-left", "editor::MoveWordLeft"),
    ("alt-right", "editor::MoveWordRight"),
    ("cmd-left", "editor::MoveToLineStart"),
    ("cmd-right", "editor::MoveToLineEnd"),
    ("cmd-up", "editor::MoveToDocumentStart"),
    ("cmd-down", "editor::MoveToDocumentEnd"),
    ("shift-left", "editor::SelectLeft"),
    ("shift-alt-right", "editor::SelectWordRight"),
    ("cmd-shift-down", "editor::SelectToDocumentEnd"),
    ("backspace", "editor::Backspace"),
    ("alt-backspace", "editor::DeleteWordBackward"),
    ("cmd-backspace", "editor::DeleteToLineStart"),
    // The Emacs keys work on the paragraph, as in every macOS text view.
    ("ctrl-a", "editor::MoveToParagraphStart"),
    ("ctrl-e", "editor::MoveToParagraphEnd"),
    ("ctrl-f", "editor::MoveRight"),
    ("ctrl-b", "editor::MoveLeft"),
    ("ctrl-n", "editor::MoveDown"),
    ("ctrl-p", "editor::MoveUp"),
    ("ctrl-k", "editor::DeleteToParagraphEnd"),
    ("ctrl-d", "editor::Delete"),
    ("ctrl-h", "editor::Backspace"),
];

/// The action GPUI would run for `keys` with the keyboard where it is now:
/// the best match of the keymap against the context stack of the focused
/// element.
fn winner(keys: &str, cx: &mut VisualTestContext) -> Option<String> {
    cx.run_until_parked();
    cx.update(|window, cx| {
        let keystrokes: Vec<Keystroke> = keys
            .split(' ')
            .map(|key| Keystroke::parse(key).expect("a valid keystroke"))
            .collect();
        let stack = window.context_stack();
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let (bindings, _) = keymap.bindings_for_input(&keystrokes, &stack);
        bindings
            .first()
            .map(|binding| binding.action().name().to_string())
    })
}

#[track_caller]
pub(super) fn assert_table(table: &[(&str, &str)], place: &str, cx: &mut VisualTestContext) {
    let wrong: Vec<String> = table
        .iter()
        .filter_map(|(keys, expected)| {
            let actual = winner(keys, cx);
            (actual.as_deref() != Some(*expected))
                .then(|| format!("{keys}: expected {expected}, runs {actual:?}"))
        })
        .collect();
    assert!(
        wrong.is_empty(),
        "with the keyboard in {place}:\n{}",
        wrong.join("\n")
    );
}

fn workspace_in_the_text(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    init(cx);
    let library = library(cx);
    add_document(
        &library,
        None,
        &simple_document("Keys", "A paragraph with a few words in it."),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    assert!(editor_focused(&workspace, vcx));
    (workspace, vcx)
}

#[gpui::test]
fn every_documented_key_runs_its_action_in_the_text(cx: &mut TestAppContext) {
    let (_workspace, vcx) = workspace_in_the_text(cx);
    assert_table(GLOBAL, "the text", vcx);
    assert_table(EDITOR, "the text", vcx);
}

#[gpui::test]
fn global_keys_work_wherever_the_keyboard_is(cx: &mut TestAppContext) {
    let (workspace, vcx) = workspace_in_the_text(cx);

    // The library sidebar.
    vcx.simulate_keystrokes("cmd-0");
    assert!(workspace.update_in(vcx, |w, window, cx| {
        gpui::Focusable::focus_handle(w.panel().read(cx), cx).contains_focused(window, cx)
    }));
    assert_table(GLOBAL, "the library", vcx);
    // The sidebar's own keys are not taken by the text's.
    assert_table(
        &[
            ("cmd-f", "library_panel::FocusFilter"),
            ("cmd-backspace", "library_panel::MoveToTrash"),
            ("n", "library_panel::NewDocument"),
            ("enter", "library_panel::OpenSelected"),
        ],
        "the library",
        vcx,
    );

    // The comment thread list.
    vcx.simulate_keystrokes("cmd-1 cmd-up shift-alt-right cmd-alt-m");
    vcx.simulate_input("A note");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    vcx.simulate_keystrokes("cmd-2");
    assert!(workspace.update_in(vcx, |w, window, cx| {
        w.comments().read(cx).is_list_focused(window)
    }));
    assert_table(GLOBAL, "the comment list", vcx);
    assert_table(
        &[
            ("j", "comments::SelectNextThread"),
            ("x", "comments::ToggleResolved"),
            ("escape", "comments::FocusEditor"),
        ],
        "the comment list",
        vcx,
    );

    // The find bar: everything global works from its field, except that
    // cmd-alt-c is "case sensitive" there, as the bar's own hint says.
    vcx.simulate_keystrokes("escape cmd-f");
    assert!(workspace.read_with(vcx, |w, cx| w.tools().read(cx).is_open(ToolKind::Find)));
    assert!(!editor_focused(&workspace, vcx));
    let in_find_bar: Vec<(&str, &str)> = GLOBAL
        .iter()
        .map(|&(keys, action)| match keys {
            "cmd-alt-c" => (keys, "editor_tools::ToggleCaseSensitive"),
            _ => (keys, action),
        })
        .collect();
    assert_table(&in_find_bar, "the find bar", vcx);
    assert_table(
        &[
            ("cmd-alt-w", "editor_tools::ToggleWholeWord"),
            ("cmd-g", "editor_tools::FindNext"),
            ("cmd-shift-g", "editor_tools::FindPrevious"),
            ("shift-enter", "editor_tools::FindPrevious"),
            ("cmd-enter", "editor_tools::ReplaceAll"),
            ("cmd-alt-f", "editor_tools::FindAndReplace"),
            ("cmd-.", "editor_tools::OpenFormatMenu"),
        ],
        "the find bar",
        vcx,
    );
    // And it really toggles.
    vcx.simulate_keystrokes("cmd-alt-c");
    assert!(workspace.read_with(vcx, |w, cx| {
        w.tools().read(cx).find_options().case_sensitive
    }));
    assert!(workspace.read_with(vcx, |w, cx| w.comments().read(cx).is_visible()));

    // A modal (the palette): the app keys still resolve.
    vcx.simulate_keystrokes("escape cmd-shift-p");
    assert!(workspace.read_with(vcx, |w, cx| w.modal_layer().read(cx).has_modal()));
    assert_table(GLOBAL, "the command palette", vcx);
}

/// No binding without a context is lost to another crate's binding: with
/// the keyboard in the text, every such key runs the action that was bound
/// to it last.
#[gpui::test]
fn no_window_wide_key_is_shadowed_in_the_text(cx: &mut TestAppContext) {
    let (_workspace, vcx) = workspace_in_the_text(cx);
    let window_wide: Vec<(String, String)> = vcx.update(|_, cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let mut last: Vec<(String, String)> = Vec::new();
        for binding in keymap.bindings() {
            if binding.predicate().is_some() {
                continue;
            }
            let keys = binding
                .keystrokes()
                .iter()
                .map(|key| key.inner().unparse())
                .collect::<Vec<_>>()
                .join(" ");
            let action = binding.action().name().to_string();
            match last.iter_mut().find(|(existing, _)| *existing == keys) {
                Some(entry) => entry.1 = action,
                None => last.push((keys, action)),
            }
        }
        last
    });
    assert!(window_wide.len() >= GLOBAL.len());
    let table: Vec<(&str, &str)> = window_wide
        .iter()
        .map(|(keys, action)| (keys.as_str(), action.as_str()))
        .collect();
    assert_table(&table, "the text", vcx);
}
