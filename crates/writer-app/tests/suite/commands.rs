//! The picker modals: command palette, quick open, library search and the
//! shortcuts overlay.

use gpui::TestAppContext;
use writer_app::{
    library_search::LibrarySearch,
    palette::CommandPalette,
    quick_open::{QuickOpen, QuickOpenChoice},
    shortcuts::ShortcutsOverlay,
};
use writer_core::{Movement, Position, Selection};
use writer_library::LibraryItem;

use super::*;

/// The open modal of type `V`.
macro_rules! modal {
    ($workspace:expr, $cx:expr, $ty:ty) => {
        $workspace.read_with($cx, |workspace, cx| {
            workspace.modal_layer().read(cx).active::<$ty>()
        })
    };
}

fn has_modal(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> bool {
    workspace.read_with(cx, |workspace, cx| {
        workspace.modal_layer().read(cx).has_modal()
    })
}

#[gpui::test]
fn palette_runs_a_command_where_focus_was(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    add_document(
        &library,
        None,
        &simple_document("Title", "Make me bold."),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    let editor = workspace.read_with(vcx, |w, _| w.editor().clone());
    // Select "bold" in the paragraph.
    editor.update(vcx, |editor, cx| {
        editor.edit(
            |state| state.set_selection(Selection::new(Position::new(1, 8), Position::new(1, 12))),
            cx,
        );
    });
    vcx.run_until_parked();

    vcx.simulate_keystrokes("cmd-shift-p");
    let palette = modal!(workspace, vcx, CommandPalette).expect("the palette is open");
    assert!(!editor_focused(&workspace, vcx));
    // Browsing: every category is there, with the editor's and the
    // library's commands beside the app's own.
    let titles = palette.read_with(vcx, |palette, _| palette.delegate.listed_titles());
    for expected in [
        "New document",
        "Bold",
        "Settings",
        "Expand folder",
        "Heading 1",
    ] {
        assert!(
            titles.iter().any(|title| title == expected),
            "{expected} is listed in {titles:?}"
        );
    }

    // Under a category heading the row does not repeat the category.
    let details = |palette: &Entity<CommandPalette>, cx: &mut VisualTestContext| {
        palette.read_with(cx, |palette, cx| {
            use writer_ui::{PickerDelegate, PickerEntry};
            (0..palette.delegate.entry_count())
                .filter_map(|ix| match palette.delegate.entry(ix, cx) {
                    PickerEntry::Item(item) => Some((
                        item.label.to_string(),
                        item.detail.map(|detail| detail.to_string()),
                        item.key_hint.map(|hint| hint.text()),
                    )),
                    PickerEntry::Header(_) => None,
                })
                .collect::<Vec<_>>()
        })
    };
    assert!(
        details(&palette, vcx)
            .iter()
            .all(|(_, detail, _)| detail.is_none())
    );

    vcx.simulate_input("bold");
    let titles = palette.read_with(vcx, |palette, _| palette.delegate.listed_titles());
    assert_eq!(titles.first().map(String::as_str), Some("Bold"));
    // In search results the category and the real key are beside the title.
    assert_eq!(
        details(&palette, vcx)[0],
        (
            "Bold".to_string(),
            Some("Format".to_string()),
            Some("⌘B".to_string())
        )
    );
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();

    assert!(!has_modal(&workspace, vcx));
    assert!(editor_focused(&workspace, vcx));
    let bold = editor.read_with(vcx, |editor, _| {
        let block = &editor.document().blocks()[1];
        block
            .text_spans()
            .filter(|(_, _, attrs)| attrs.bold)
            .map(|(_, text, _)| text.to_string())
            .collect::<String>()
    });
    assert_eq!(bold, "bold");

    // Recently used comes first the next time.
    vcx.simulate_keystrokes("cmd-shift-p");
    let palette = modal!(workspace, vcx, CommandPalette).unwrap();
    let titles = palette.read_with(vcx, |palette, _| palette.delegate.listed_titles());
    assert_eq!(titles.first().map(String::as_str), Some("Bold"));
    // The same key closes it, and focus is back in the editor.
    vcx.simulate_keystrokes("cmd-shift-p");
    assert!(!has_modal(&workspace, vcx));
    assert!(editor_focused(&workspace, vcx));

    // Keywords find commands whose title says something else; loose
    // subsequences over keywords do not.
    vcx.simulate_keystrokes("cmd-shift-p");
    let palette = modal!(workspace, vcx, CommandPalette).unwrap();
    vcx.simulate_input("dark");
    let titles = palette.read_with(vcx, |palette, _| palette.delegate.listed_titles());
    assert_eq!(titles, ["Use Night theme", "Follow system appearance"]);
    vcx.simulate_keystrokes("escape");

    // An app-level command through the palette: hide the sidebar.
    vcx.simulate_keystrokes("cmd-shift-p");
    vcx.simulate_input("hide library");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert!(!workspace.read_with(vcx, |w, _| w.settings().sidebar_visible));
    assert!(editor_focused(&workspace, vcx));
}

#[gpui::test]
fn palette_shows_the_real_key_bindings(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let (_workspace, vcx) = open_workspace(&library, cx);
    vcx.update(|_, cx| {
        let all = writer_app::commands::all(cx);
        let hint = |title: &str| {
            all.iter()
                .find(|command| command.title.as_ref() == title)
                .unwrap_or_else(|| panic!("no command titled {title}"))
                .key_hint(cx)
                .map(|hint| hint.text())
        };
        assert_eq!(hint("Command palette").as_deref(), Some("⇧⌘P"));
        assert_eq!(hint("Show or hide library").as_deref(), Some("⌥⌘S"));
        assert_eq!(hint("Bold").as_deref(), Some("⌘B"));
        assert_eq!(hint("Settings").as_deref(), Some("⌘,"));
        assert_eq!(hint("Quit The Writer").as_deref(), Some("⌘Q"));
        assert_eq!(hint("Close window").as_deref(), Some("⌘W"));
        assert_eq!(hint("Use Night theme"), None);
        // Registering again changes nothing.
        let before = all.len();
        writer_app::commands::register(writer_app::actions::commands(), cx);
        assert_eq!(writer_app::commands::all(cx).len(), before);
    });
}

#[gpui::test]
fn quick_open_finds_opens_and_creates(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let folder = library.update(cx, |library, cx| {
        library.create_folder(None, "Novel", cx).unwrap().id
    });
    let harbour = add_document(
        &library,
        Some(folder),
        &simple_document("The Harbour", "Boats."),
        cx,
    );
    let arrival = add_document(&library, None, &simple_document("Arrival", "Late."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    workspace.update_in(vcx, |workspace, window, cx| {
        workspace.open_document(arrival, true, window, cx);
    });

    vcx.simulate_keystrokes("cmd-p");
    let picker = modal!(workspace, vcx, QuickOpen).expect("quick open is open");
    let all = picker.read_with(vcx, |picker, _| picker.delegate.choices());
    assert_eq!(all.len(), 2);

    vcx.simulate_input("harb");
    let choices = picker.read_with(vcx, |picker, _| picker.delegate.choices());
    assert_eq!(choices, vec![QuickOpenChoice::Open(harbour)]);
    // The folder path is the detail.
    let detail = picker.read_with(vcx, |picker, cx| {
        use writer_ui::{PickerDelegate, PickerEntry};
        match picker.delegate.entry(0, cx) {
            PickerEntry::Item(item) => item.detail.map(|detail| detail.to_string()),
            PickerEntry::Header(_) => None,
        }
    });
    assert_eq!(detail.as_deref(), Some("Novel"));
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert!(!has_modal(&workspace, vcx));
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(harbour)
    );
    assert!(editor_focused(&workspace, vcx));

    // Nothing matches: offer to create it, in the folder of the open document.
    vcx.simulate_keystrokes("cmd-p");
    let picker = modal!(workspace, vcx, QuickOpen).unwrap();
    vcx.simulate_input("Chapter Nine");
    let choices = picker.read_with(vcx, |picker, _| picker.delegate.choices());
    assert_eq!(
        choices,
        vec![QuickOpenChoice::Create("Chapter Nine".to_string())]
    );
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let created = workspace
        .read_with(vcx, |w, _| w.open_document_id())
        .unwrap();
    assert_ne!(created, harbour);
    let meta = library.read_with(vcx, |library, _| library.document_meta(created).unwrap());
    assert_eq!(meta.title, "Chapter Nine");
    assert_eq!(meta.folder_id, Some(folder));
    assert!(editor_focused(&workspace, vcx));
    // The caret waits below the heading.
    vcx.simulate_input("It began.");
    assert_eq!(editor_text(&workspace, vcx), "Chapter Nine\nIt began.");

    // Escape closes and returns focus.
    vcx.simulate_keystrokes("cmd-p");
    assert!(has_modal(&workspace, vcx));
    vcx.simulate_keystrokes("escape");
    assert!(!has_modal(&workspace, vcx));
    assert!(editor_focused(&workspace, vcx));
}

#[gpui::test]
fn library_search_opens_the_hit_and_selects_the_match(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    add_document(
        &library,
        None,
        &simple_document("Bread", "She bought bread."),
        cx,
    );
    let salt = add_document(
        &library,
        None,
        &Document::from_blocks(vec![
            Block::with_text(BlockKind::heading(1), "Salt"),
            Block::paragraph("The pans stretched to the horizon."),
            Block::paragraph("Pink and white, the salt works slept."),
        ]),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);

    vcx.simulate_keystrokes("cmd-shift-f");
    let picker = modal!(workspace, vcx, LibrarySearch).expect("the search is open");
    vcx.simulate_input("horizon");
    let hits = picker.read_with(vcx, |picker, _| picker.delegate.hits().to_vec());
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].document_id, salt);
    assert_eq!(hits[0].title, "Salt");
    // The snippet is clean text and the match is marked.
    assert!(hits[0].snippet.contains("horizon"));
    assert!(!hits[0].snippet.contains('\u{E000}'));
    let marked: String = hits[0]
        .positions
        .iter()
        .filter_map(|offset| hits[0].snippet[*offset..].chars().next())
        .collect();
    assert_eq!(marked, "horizon");

    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert!(!has_modal(&workspace, vcx));
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(salt)
    );
    assert!(editor_focused(&workspace, vcx));
    let selected = workspace.read_with(vcx, |w, cx| {
        let editor = w.editor().read(cx);
        (editor.state().selected_text(), editor.selection().start())
    });
    assert_eq!(selected.0, "horizon");
    assert_eq!(selected.1.block, 1);

    // No hit: a clear empty state, nothing to open.
    vcx.simulate_keystrokes("cmd-shift-f");
    let picker = modal!(workspace, vcx, LibrarySearch).unwrap();
    vcx.simulate_input("zeppelin");
    assert!(picker.read_with(vcx, |picker, _| picker.delegate.hits().is_empty()));
    vcx.simulate_keystrokes("enter escape");
    assert!(!has_modal(&workspace, vcx));
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(salt)
    );
}

#[gpui::test]
fn shortcuts_overlay_is_generated_from_the_keymap(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    add_document(&library, None, &simple_document("Keys", "Text here."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);

    vcx.simulate_keystrokes("cmd-/");
    let overlay = modal!(workspace, vcx, ShortcutsOverlay).expect("the overlay is open");
    let listed = overlay.read_with(vcx, |overlay, _| {
        overlay
            .delegate
            .listed()
            .iter()
            .map(|shortcut| {
                (
                    shortcut.category.to_string(),
                    shortcut.title.to_string(),
                    shortcut.keys.clone(),
                )
            })
            .collect::<Vec<_>>()
    });
    let find = |title: &str| {
        listed
            .iter()
            .find(|(_, listed_title, _)| listed_title == title)
            .unwrap_or_else(|| panic!("{title} is not listed"))
            .clone()
    };
    assert_eq!(find("Command palette").2, "⇧⌘P");
    assert_eq!(find("Keyboard shortcuts").2, "⌘/");
    assert_eq!(find("Bold"), ("Format".into(), "Bold".into(), "⌘B".into()));
    // Keys without a command descriptor come from the keymap itself.
    assert_eq!(
        find("Move word left"),
        ("Editing keys".into(), "Move word left".into(), "⌥←".into())
    );
    // Commands without a binding are not shortcuts.
    assert!(
        !listed
            .iter()
            .any(|(_, title, _)| title == "Use Night theme")
    );
    // Nothing is listed twice.
    let mut titles: Vec<_> = listed.iter().map(|(c, t, _)| (c, t)).collect();
    let total = titles.len();
    titles.sort();
    titles.dedup();
    assert_eq!(titles.len(), total);

    // Searchable, and enter runs the shortcut's command.
    vcx.simulate_input("document end");
    let first = overlay.read_with(vcx, |overlay, _| {
        overlay
            .delegate
            .listed()
            .first()
            .map(|s| s.title.to_string())
    });
    assert_eq!(first.as_deref(), Some("Move to document end"));
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert!(!has_modal(&workspace, vcx));
    assert!(editor_focused(&workspace, vcx));
    let (head, end) = workspace.read_with(vcx, |w, cx| {
        let editor = w.editor().read(cx);
        (editor.selection().head, editor.document().end())
    });
    assert_eq!(head, end);
    let _ = (Movement::DocEnd, LibraryItem::Document);
}
