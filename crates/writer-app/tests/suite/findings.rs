//! What the testers found, kept from coming back: a modal that lost the
//! keyboard, commands for the text chosen from elsewhere, a settings row
//! that stayed out of sight, the sidebar edge that clicked through, two
//! copies on one library, a polite request to end the process, a title that
//! lagged behind the text.

use gpui::{Focusable as _, Modifiers, MouseButton, TestAppContext, point, px, size};
use writer_app::{
    palette::CommandPalette,
    settings::keys,
    settings_view::{SettingRow, SettingsView},
    single_instance::{self, Acquired},
    window,
    workspace::{AUTOSAVE_DELAY, limit_label},
};
use writer_core::{BlockKind, Position};
use writer_editor::actions as editor;
use writer_export::{SocialCount, SocialNetwork};

use super::*;

fn has_modal(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> bool {
    workspace.read_with(cx, |w, cx| w.modal_layer().read(cx).has_modal())
}

fn comments_focused(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> bool {
    workspace.update_in(cx, |w, window, cx| {
        w.comments().read(cx).has_focus(window, cx)
    })
}

fn panel_focused(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> bool {
    workspace.update_in(cx, |w, window, cx| {
        w.panel().focus_handle(cx).contains_focused(window, cx)
    })
}

fn block_kind(
    workspace: &Entity<Workspace>,
    block: usize,
    cx: &mut VisualTestContext,
) -> BlockKind {
    workspace.read_with(cx, |w, cx| {
        w.editor().read(cx).document().blocks()[block]
            .kind()
            .clone()
    })
}

#[gpui::test]
fn a_modal_never_stays_on_screen_without_the_keyboard(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    add_document(
        &library,
        None,
        &assembly::commented_document("Noted", "Alpha beta gamma delta.", "A first note"),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);

    // The comments key behind the palette: the palette closes and the
    // keyboard is in the comment list. Escape then goes back to the text
    // (before, the palette stayed, and typing went into the hidden text).
    vcx.simulate_keystrokes("cmd-shift-p");
    assert!(has_modal(&workspace, vcx));
    vcx.simulate_keystrokes("cmd-2");
    assert!(!has_modal(&workspace, vcx));
    assert!(comments_focused(&workspace, vcx));
    vcx.simulate_keystrokes("escape");
    assert!(editor_focused(&workspace, vcx));
    let before = editor_text(&workspace, vcx);

    // "Add comment" behind the settings: the settings close, the composer
    // is open and has the keyboard.
    vcx.simulate_keystrokes("cmd-,");
    assert!(has_modal(&workspace, vcx));
    vcx.simulate_keystrokes("cmd-alt-m");
    vcx.run_until_parked();
    assert!(!has_modal(&workspace, vcx));
    assert!(workspace.read_with(vcx, |w, cx| w.comments().read(cx).is_composing()));
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(!workspace.read_with(vcx, |w, cx| w.comments().read(cx).is_composing()));
    assert_eq!(editor_text(&workspace, vcx), before);

    // Every modal, and both comment keys that move the caret.
    for open in ["cmd-p", "cmd-shift-f", "cmd-/", "cmd-shift-s", "cmd-,"] {
        for key in ["cmd-2", "cmd-alt-]", "cmd-alt-["] {
            vcx.simulate_keystrokes("cmd-1");
            vcx.simulate_keystrokes(open);
            assert!(has_modal(&workspace, vcx), "{open} opens a modal");
            vcx.simulate_keystrokes(key);
            vcx.run_until_parked();
            assert!(!has_modal(&workspace, vcx), "{key} behind {open}");
        }
    }

    // A view toggle closes the modal first instead of acting out of sight.
    vcx.simulate_keystrokes("cmd-1 cmd-shift-p cmd-alt-p");
    assert!(!has_modal(&workspace, vcx));
    assert!(workspace.read_with(vcx, |w, cx| w.editor().read(cx).page_view()));
    vcx.simulate_keystrokes("cmd-shift-p cmd-alt-c");
    vcx.run_until_parked();
    assert!(!has_modal(&workspace, vcx));
    assert!(!workspace.read_with(vcx, |w, _| w.settings().comments_visible));
    vcx.simulate_keystrokes("cmd-alt-c");

    // The rule itself: whatever moves the keyboard out of an open modal,
    // the modal closes and the keyboard stays where it went.
    vcx.simulate_keystrokes("cmd-shift-p");
    assert!(has_modal(&workspace, vcx));
    workspace.update_in(vcx, |w, window, cx| {
        let panel = w.panel().focus_handle(cx);
        window.focus(&panel);
        cx.notify();
    });
    vcx.run_until_parked();
    assert!(!has_modal(&workspace, vcx));
    assert!(panel_focused(&workspace, vcx));

    // And a modal that has the keyboard stays, also across frames.
    vcx.simulate_keystrokes("cmd-shift-p");
    workspace.update(vcx, |_, cx| cx.notify());
    vcx.run_until_parked();
    vcx.simulate_input("page");
    assert!(workspace.read_with(vcx, |w, cx| {
        w.modal_layer().read(cx).is_open::<CommandPalette>()
    }));
    vcx.simulate_keystrokes("escape");
    assert!(!has_modal(&workspace, vcx));
}

#[gpui::test]
fn commands_for_the_text_work_from_the_library_and_the_comment_list(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    add_document(
        &library,
        None,
        &assembly::commented_document("Noted", "Alpha beta gamma delta.", "A first note"),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    workspace.update(vcx, |w, cx| {
        w.editor().update(cx, |editor, cx| {
            editor.edit(|state| state.set_caret(Position::new(1, 0)), cx);
        });
    });

    // From the library, through the palette: the block becomes a heading
    // and the keyboard is in the text.
    vcx.simulate_keystrokes("cmd-0");
    assert!(panel_focused(&workspace, vcx));
    vcx.simulate_keystrokes("cmd-shift-p");
    vcx.simulate_input("Heading 2");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(block_kind(&workspace, 1, vcx), BlockKind::heading(2));
    assert!(editor_focused(&workspace, vcx));

    // From the comment list: a tool opens.
    vcx.simulate_keystrokes("cmd-2");
    assert!(comments_focused(&workspace, vcx));
    vcx.simulate_keystrokes("cmd-shift-p");
    vcx.simulate_input("Find and replace");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(
        workspace.read_with(vcx, |w, cx| w.tools().read(cx).active_tool()),
        Some(writer_editor_tools::ToolKind::Find)
    );
    vcx.simulate_keystrokes("escape");

    // The way a menu bar item arrives: the action is dispatched where the
    // keyboard is. It is offered there (the menu item is enabled) and it
    // reaches the text.
    vcx.simulate_keystrokes("cmd-0");
    assert!(vcx.update(|window, cx| window.is_action_available(&editor::SetParagraph, cx)));
    vcx.dispatch_action(editor::SetParagraph);
    vcx.run_until_parked();
    assert_eq!(block_kind(&workspace, 1, vcx), BlockKind::Paragraph);
    assert!(editor_focused(&workspace, vcx));

    // Also from inside a modal, which closes.
    vcx.simulate_keystrokes("cmd-,");
    vcx.dispatch_action(editor::SetHeading3);
    vcx.run_until_parked();
    assert!(!has_modal(&workspace, vcx));
    assert_eq!(block_kind(&workspace, 1, vcx), BlockKind::heading(3));

    // The Edit menu's own items are never sent to the text from elsewhere:
    // "Select all" with the keyboard in the library leaves the text alone.
    let selection_before = workspace.read_with(vcx, |w, cx| w.editor().read(cx).selection());
    vcx.simulate_keystrokes("cmd-0");
    vcx.dispatch_action(editor::SelectAll);
    vcx.run_until_parked();
    assert!(panel_focused(&workspace, vcx));
    assert_eq!(
        workspace.read_with(vcx, |w, cx| w.editor().read(cx).selection()),
        selection_before
    );

    // Without a document nothing for the text is on offer.
    workspace.update_in(vcx, |w, window, cx| w.close_document(window, cx));
    vcx.run_until_parked();
    assert!(!vcx.update(|window, cx| window.is_action_available(&editor::ToggleBold, cx)));
}

#[gpui::test]
fn the_focused_setting_is_always_in_view(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    add_document(&library, None, &simple_document("Doc", "Text."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);

    for (width, height) in [(1200., 800.), (1200., 640.), (700., 480.)] {
        vcx.simulate_resize(size(px(width), px(height)));
        vcx.run_until_parked();
        vcx.simulate_keystrokes("cmd-,");
        let view = workspace
            .read_with(vcx, |w, cx| {
                w.modal_layer().read(cx).active::<SettingsView>()
            })
            .expect("settings are open");
        let rows = view.read_with(vcx, |view, cx| view.rows(cx));
        let check = |ix: usize, cx: &mut VisualTestContext| {
            let list = cx.debug_bounds("settings-list").expect("the list is drawn");
            let row = cx
                .debug_bounds(Box::leak(format!("setting-{ix}").into_boxed_str()))
                .expect("the row is drawn");
            assert!(
                row.top() >= list.top() && row.bottom() <= list.bottom(),
                "row {ix} ({:?}) is cut by the list at {width}x{height}: {row:?} in {list:?}",
                rows[ix],
            );
        };
        // Down through every row (the tenth, "Character limit", is the
        // first of its section and used to stay below the fold), then up.
        check(0, vcx);
        for (ix, row) in rows.iter().enumerate().skip(1) {
            vcx.simulate_keystrokes("down");
            vcx.run_until_parked();
            assert_eq!(
                view.update_in(vcx, |view, window, cx| view.focused_row(window, cx)),
                Some(*row)
            );
            check(ix, vcx);
        }
        for ix in (0..rows.len() - 1).rev() {
            vcx.simulate_keystrokes("up");
            vcx.run_until_parked();
            check(ix, vcx);
        }
        vcx.simulate_keystrokes("escape");

        // Opened at "This document": that row is in view from the start.
        workspace.update_in(vcx, |w, window, cx| w.toggle_document_settings(window, cx));
        vcx.run_until_parked();
        let start = rows
            .iter()
            .position(|row| *row == SettingRow::DocumentPageView)
            .unwrap();
        check(start, vcx);
        // The section starts at the top of the list (unless the list is
        // too short to scroll that far).
        let list = vcx.debug_bounds("settings-list").unwrap();
        let section = vcx
            .debug_bounds(Box::leak(format!("setting-{start}").into_boxed_str()))
            .unwrap();
        let view = workspace
            .read_with(vcx, |w, cx| {
                w.modal_layer().read(cx).active::<SettingsView>()
            })
            .unwrap();
        let (offset, max) = view.read_with(vcx, |view, _| {
            (
                view.scroll_handle().offset().y,
                view.scroll_handle().max_offset().height,
            )
        });
        assert!(
            section.top() == list.top() || offset == -max,
            "{section:?} in {list:?}, scrolled {offset:?} of {max:?}"
        );
        vcx.simulate_keystrokes("escape");
    }
}

#[gpui::test]
fn the_sidebar_edge_does_not_click_through_to_the_text(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    add_document(
        &library,
        None,
        &simple_document("Edge", "One two three four five six seven."),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    let width = workspace.read_with(vcx, |w, _| w.settings().sidebar_width);
    vcx.simulate_keystrokes("cmd-a");
    let selected = workspace.read_with(vcx, |w, cx| w.editor().read(cx).selection());
    assert!(!selected.is_collapsed());

    // The right half of the handle lies over the editor area.
    let on_handle = point(px(width + 2.), px(300.));
    vcx.simulate_click(on_handle, Modifiers::default());
    assert_eq!(
        workspace.read_with(vcx, |w, cx| w.editor().read(cx).selection()),
        selected,
        "a press on the handle leaves the selection alone"
    );
    assert!(editor_focused(&workspace, vcx));

    // Dragging from there resizes, and still leaves the text alone.
    vcx.simulate_mouse_down(on_handle, MouseButton::Left, Modifiers::default());
    let target = point(px(width + 60.), px(300.));
    vcx.simulate_mouse_move(target, MouseButton::Left, Modifiers::default());
    vcx.simulate_mouse_up(target, MouseButton::Left, Modifiers::default());
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.settings().sidebar_width),
        width + 60.
    );
    assert_eq!(
        workspace.read_with(vcx, |w, cx| w.editor().read(cx).selection()),
        selected
    );

    // With the keyboard in the library a double-click on the handle resets
    // the width and the keyboard stays in the library.
    vcx.simulate_keystrokes("cmd-0");
    let on_handle = point(px(width + 62.), px(300.));
    vcx.simulate_event(gpui::MouseDownEvent {
        button: MouseButton::Left,
        position: on_handle,
        modifiers: Modifiers::default(),
        click_count: 2,
        first_mouse: false,
    });
    vcx.simulate_mouse_up(on_handle, MouseButton::Left, Modifiers::default());
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.settings().sidebar_width),
        writer_app::settings::DEFAULT_SIDEBAR_WIDTH
    );
    assert!(panel_focused(&workspace, vcx));
    assert_eq!(
        workspace.read_with(vcx, |w, cx| w.editor().read(cx).selection()),
        selected
    );
}

#[test]
fn one_running_copy_per_data_directory() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("The Writer");

    // The first launch gets the lock and writes who it is.
    let lock = match single_instance::acquire(&data).unwrap() {
        Acquired::Lock(lock) => lock,
        Acquired::Taken(other) => panic!("a fresh directory was taken: {other:?}"),
    };
    assert_eq!(lock.path(), data.join(single_instance::LOCK_FILE_NAME));
    assert_eq!(
        std::fs::read_to_string(lock.path()).unwrap().trim(),
        std::process::id().to_string()
    );

    // A second launch on the same directory is told who has it. (The lock
    // belongs to the open file, so a second open in this process is
    // refused exactly like one from another process.)
    let running = match single_instance::acquire(&data).unwrap() {
        Acquired::Taken(running) => running,
        Acquired::Lock(_) => panic!("the directory was locked twice"),
    };
    assert_eq!(running.pid, Some(std::process::id()));
    assert_eq!(running.data_dir, data);
    // Being refused changed nothing in the lock file.
    assert_eq!(
        std::fs::read_to_string(lock.path()).unwrap().trim(),
        std::process::id().to_string()
    );

    // What the second launch says.
    let (title, message, detail) = window::already_running_text(&running);
    assert_eq!(title, "The Writer is already open");
    assert!(message.contains("overwrite the other's text"), "{message}");
    assert!(detail.contains(&format!("process {}", std::process::id())));
    assert!(detail.contains(&data.display().to_string()));

    // Another directory is another library.
    let other = dir.path().join("Other");
    assert!(matches!(
        single_instance::acquire(&other).unwrap(),
        Acquired::Lock(_)
    ));

    // When the first copy ends its lock is gone, although the file stays:
    // a file left behind by a crash is not a running copy.
    let path = lock.path().to_path_buf();
    drop(lock);
    assert!(path.exists());
    assert!(matches!(
        single_instance::acquire(&data).unwrap(),
        Acquired::Lock(_)
    ));
}

#[gpui::test]
fn a_second_copy_gets_a_window_that_says_so(cx: &mut TestAppContext) {
    init(cx);
    let running = single_instance::RunningCopy {
        pid: Some(4321),
        data_dir: "/tmp/somewhere/The Writer".into(),
    };
    let handle = cx
        .update(|cx| window::open_already_running_window(&running, false, cx))
        .unwrap();
    cx.run_until_parked();
    let (title, detail, way_out) = handle
        .update(cx, |view, _, _| {
            (
                view.title().to_string(),
                view.detail().to_string(),
                view.way_out_label(),
            )
        })
        .unwrap();
    assert_eq!(title, "The Writer is already open");
    assert!(detail.contains("process 4321"));
    assert_eq!(
        way_out.map(|label| label.to_string()).as_deref(),
        Some("Show the Open Copy")
    );
    // The library error window has no such button: it can only quit.
    let plain = cx
        .update(|cx| window::open_error_window("T", "M", "D", cx))
        .unwrap();
    assert_eq!(
        plain.update(cx, |view, _, _| view.way_out_label()).unwrap(),
        None
    );
}

#[test]
fn a_termination_signal_is_heard() {
    // The app listens for SIGTERM, SIGHUP and SIGINT; the test uses a
    // signal nothing else sends, so the test process keeps its defaults.
    let fd = writer_app::signals::listen(&[libc::SIGUSR2]).unwrap();
    // SAFETY: a handler for the signal is installed; raising it runs the
    // handler on this thread.
    unsafe {
        libc::raise(libc::SIGUSR2);
    }
    // The byte is waiting: this returns at once.
    assert!(writer_app::signals::wait(fd));
}

#[test]
fn the_limit_indicator_reads_like_the_other_counts() {
    let count = |length| SocialCount {
        network: SocialNetwork::X,
        length,
        limit: 280,
    };
    assert_eq!(limit_label(count(127)), "127 / 280 on X");
    assert_eq!(limit_label(count(1722)), "1,722 / 280 on X");
    let linkedin = SocialCount {
        network: SocialNetwork::LinkedIn,
        length: 3100,
        limit: 3000,
    };
    assert_eq!(limit_label(linkedin), "3,100 / 3,000 on LinkedIn");
}

/// The title above the text is the title of the text as it is now: making a
/// heading, undoing it and redoing it change it at once, not when the next
/// save comes round. (Seen in the app: a paragraph was turned into a heading
/// and became the title; two undos brought the text back, and the title
/// band still showed the heading.)
#[gpui::test]
fn the_title_follows_the_text_after_every_change(cx: &mut TestAppContext) {
    init(cx);
    // A library on disk, as the app has: its autosave runs in the
    // background and reports back later.
    let dir = tempfile::tempdir().unwrap();
    let library = cx.new(|_| writer_library::Library::open(dir.path().join("library.db")).unwrap());
    library.update(cx, |library, cx| {
        library
            .set_setting(keys::WELCOME_CREATED, &true, cx)
            .unwrap();
    });
    let document = Document::from_blocks(vec![
        Block::paragraph("The Salt Road is out next month. It took four years."),
        Block::paragraph("What the harbour remembers"),
    ]);
    let id = add_document(&library, None, &document, cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(id, true, window, cx);
    });
    vcx.run_until_parked();
    let title = |cx: &mut VisualTestContext| {
        workspace
            .read_with(cx, |w, _| w.document_title().map(str::to_string))
            .unwrap()
    };
    let stored_title = |cx: &mut VisualTestContext| {
        library.read_with(cx, |library, _| library.document_meta(id).unwrap().title)
    };
    let first_sentence = "The Salt Road is out next month";
    let heading = "What the harbour remembers";
    assert_eq!(title(vcx), first_sentence);

    // The second paragraph becomes a heading: it is the title, before any
    // save has run.
    workspace.update(vcx, |w, cx| {
        w.editor().update(cx, |editor, cx| {
            editor.edit(|state| state.set_caret(Position::new(1, 4)), cx);
        });
    });
    vcx.simulate_keystrokes("cmd-alt-2");
    assert_eq!(block_kind(&workspace, 1, vcx), BlockKind::heading(2));
    assert_eq!(title(vcx), heading);
    assert_eq!(stored_title(vcx), first_sentence, "nothing was saved yet");

    // Undo: the text is back and so is the title, at once.
    vcx.simulate_keystrokes("cmd-z");
    assert_eq!(block_kind(&workspace, 1, vcx), BlockKind::Paragraph);
    assert_eq!(title(vcx), first_sentence);
    // Redo, and undo again while the background save of the heading is
    // still on its way: its answer must not bring the heading back.
    vcx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(title(vcx), heading);
    workspace.update(vcx, |w, cx| w.autosave(cx));
    assert!(workspace.read_with(vcx, |w, _| w.is_saving_in_background()));
    workspace.update(vcx, |w, cx| {
        w.editor().update(cx, |editor, cx| {
            editor.edit(
                |state| {
                    state.undo();
                },
                cx,
            );
        });
    });
    assert!(workspace.read_with(vcx, |w, _| w.is_saving_in_background()));
    assert_eq!(title(vcx), first_sentence);
    vcx.run_until_parked();
    assert_eq!(
        stored_title(vcx),
        heading,
        "the older text was saved meanwhile"
    );
    assert_eq!(title(vcx), first_sentence, "the band keeps to the text");
    // The next save stores what the band already shows.
    vcx.executor().advance_clock(AUTOSAVE_DELAY * 2);
    vcx.run_until_parked();
    assert_eq!(stored_title(vcx), first_sentence);
    assert_eq!(title(vcx), first_sentence);

    // Typing in the first sentence moves the title with it.
    workspace.update(vcx, |w, cx| {
        w.editor().update(cx, |editor, cx| {
            editor.edit(|state| state.set_caret(Position::new(0, 0)), cx);
        });
    });
    vcx.simulate_input("Soon: ");
    assert_eq!(title(vcx), "Soon: The Salt Road is out next month");

    // A title the user chose is left alone by all of this.
    vcx.executor().advance_clock(AUTOSAVE_DELAY * 2);
    vcx.run_until_parked();
    library.update(vcx, |library, cx| {
        library.rename_document(id, "Launch notes", cx).unwrap();
    });
    vcx.run_until_parked();
    assert_eq!(title(vcx), "Launch notes");
    vcx.simulate_keystrokes("cmd-alt-1");
    assert_eq!(title(vcx), "Launch notes");
    vcx.simulate_keystrokes("cmd-z");
    vcx.executor().advance_clock(AUTOSAVE_DELAY * 2);
    vcx.run_until_parked();
    assert_eq!(title(vcx), "Launch notes");
    assert_eq!(stored_title(vcx), "Launch notes");
}
