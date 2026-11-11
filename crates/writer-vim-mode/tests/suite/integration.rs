//! The clipboard registers, the palette commands and the views.

use gpui::{AppContext, ClipboardItem, TestAppContext, px, size};
use writer_core::{Block, BlockKind, Document, Fragment};
use writer_vim_mode::{
    ClearSearchHighlight, Mode, SEARCH_LAYER, ToggleVimMode, VimEvent, VimOptions,
};

use super::{at, doc, open, open_detached, setup};

#[gpui::test]
fn the_plus_register_is_the_system_clipboard(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(
        Document::from_blocks(vec![
            Block::with_text(BlockKind::heading(2), "Title"),
            Block::paragraph("body text"),
        ]),
        cx,
    );

    // A linewise yank: plain text with a final line break for other apps,
    // the block itself for pasting back.
    cx.simulate_keystrokes("\" + y y");
    let item = cx.read_from_clipboard().expect("clipboard written");
    assert_eq!(item.text().as_deref(), Some("Title\n"));
    let fragment = Fragment::from_json(item.metadata().unwrap()).unwrap();
    assert_eq!(fragment.blocks()[0].kind().heading_level(), Some(2));

    cx.simulate_keystrokes("j \" + p");
    assert_eq!(rig.text(cx), "Title\nbody text\nTitle");
    rig.editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.document().blocks()[2].kind().heading_level(),
            Some(2)
        );
    });

    // Text from another application: characterwise without a final break.
    cx.write_to_clipboard(ClipboardItem::new_string("pasted ".into()));
    cx.simulate_keystrokes("k 0 \" + shift-p");
    assert_eq!(rig.text(cx), "Title\npasted body text\nTitle");
    // Linewise with one.
    cx.write_to_clipboard(ClipboardItem::new_string("a line\n".into()));
    cx.simulate_keystrokes("\" * p");
    assert_eq!(rig.text(cx), "Title\npasted body text\na line\nTitle");

    // What the editor's own copy put there keeps its formatting.
    cx.simulate_keystrokes("g g v e cmd-b cmd-c escape shift-g $ \" + p");
    rig.editor.read_with(cx, |editor, _| {
        let last = editor.document().blocks().last().unwrap();
        assert_eq!(last.text(), "TitleTitle");
        assert!(last.attrs_at(5).unwrap().bold);
        assert!(!last.attrs_at(4).unwrap().bold);
    });

    // Dot-repeat of a put from the clipboard reads the clipboard again.
    cx.write_to_clipboard(ClipboardItem::new_string("!".into()));
    cx.simulate_keystrokes(".");
    rig.editor.read_with(cx, |editor, _| {
        assert!(
            editor
                .document()
                .blocks()
                .last()
                .unwrap()
                .text()
                .ends_with('!')
        );
    });

    // Without the option, plain yanks leave the clipboard alone.
    cx.write_to_clipboard(ClipboardItem::new_string("untouched".into()));
    cx.simulate_keystrokes("g g y y");
    assert_eq!(
        cx.read_from_clipboard().unwrap().text().as_deref(),
        Some("untouched")
    );
    rig.vim.update(cx, |vim, cx| {
        vim.set_options(
            VimOptions {
                unnamed_clipboard: true,
                ..VimOptions::default()
            },
            cx,
        )
    });
    cx.simulate_keystrokes("j y y");
    assert_eq!(
        cx.read_from_clipboard().unwrap().text().as_deref(),
        Some("pasted body text\n")
    );
}

#[gpui::test]
fn palette_commands_act_on_the_focused_editor(cx: &mut TestAppContext) {
    setup(cx);
    let commands = writer_vim_mode::commands();
    let titles: Vec<String> = commands.iter().map(|command| command.label()).collect();
    assert_eq!(
        titles,
        ["Vim: Toggle Vim mode", "Vim: Clear search highlight"]
    );
    assert_eq!(commands[0].action_name(), "vim_mode::ToggleVimMode");

    let (rig, cx) = open_detached(doc(&["alpha beta", "beta"]), cx);
    rig.take_events();
    // As the palette runs a command: dispatch its action in the window.
    cx.dispatch_action(ToggleVimMode);
    cx.run_until_parked();
    assert!(rig.vim.read_with(cx, |vim, _| vim.is_attached()));
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert!(rig.take_events().contains(&VimEvent::Attached));

    cx.simulate_keystrokes("/ b e t a enter");
    assert_eq!(rig.layer(SEARCH_LAYER, cx).len(), 2);
    cx.dispatch_action(ClearSearchHighlight);
    cx.run_until_parked();
    assert!(rig.layer(SEARCH_LAYER, cx).is_empty());

    cx.dispatch_action(ToggleVimMode);
    cx.run_until_parked();
    assert!(!rig.vim.read_with(cx, |vim, _| vim.is_attached()));
    assert!(rig.take_events().contains(&VimEvent::Detached));
    cx.simulate_input("x");
    assert_eq!(rig.text(cx), "alpha xbeta\nbeta");
}

#[gpui::test]
fn the_command_line_strip_takes_room_only_while_open(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["one", "two"]), cx);
    cx.simulate_resize(size(px(600.), px(400.)));
    cx.run_until_parked();
    let editor_height = |cx: &mut gpui::VisualTestContext| {
        rig.editor.read_with(cx, |editor, _| {
            f32::from(editor.viewport_bounds().unwrap().size.height)
        })
    };
    let closed = editor_height(cx);

    cx.simulate_keystrokes(":");
    cx.run_until_parked();
    let open_height = editor_height(cx);
    assert_eq!(closed - open_height, 28., "one line under the editor");
    // The keyboard never left the editor.
    assert!(
        rig.editor
            .update_in(cx, |editor, window, _| editor.is_focused(window))
    );

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(editor_height(cx), closed);

    // Detached, the status item draws nothing either: the editor gets its
    // line as well.
    cx.simulate_keystrokes("v :");
    rig.vim.update(cx, |vim, cx| vim.detach(cx));
    cx.run_until_parked();
    assert!(editor_height(cx) > closed);
    assert!(!rig.command_line.read_with(cx, |line, cx| line.is_open(cx)));
    assert_eq!(rig.head(cx), at(0, 0));
}

#[gpui::test]
fn dropping_vim_mode_gives_the_editor_back(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let editor = writer_editor::Editor::new(
            doc(&["hello"]),
            writer_editor::EditorConfig::full(),
            window,
            cx,
        );
        editor.focus(window);
        editor
    });
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();

    let vim = cx.new(|cx| writer_vim_mode::VimMode::new(editor.clone(), cx));
    vim.update(cx, |vim, cx| vim.attach(cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("x");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.plain_text(), "ello");
        assert!(!editor.text_input_enabled());
    });

    drop(vim);
    // The entity is released with the next effect cycle.
    cx.update(|_, _| {});
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        assert!(editor.text_input_enabled());
        assert_eq!(editor.config().caret_style, writer_editor::CaretStyle::Bar);
    });
    cx.simulate_input("x");
    editor.read_with(cx, |editor, _| assert_eq!(editor.plain_text(), "xello"));
}
