//! The context menus of the panel: a right-click on a thread, a reply, the
//! panel itself and the composer, the "more" button, and `ctrl-enter` in
//! the thread list.

use gpui::{
    Action, Bounds, Modifiers, MouseButton, Pixels, Point, TestAppContext, VisualTestContext,
    point, px,
};
use innerword_comments::{
    AddComment, CommentMenuTarget, CopyComment, DeleteComment, DeleteTarget, DraftKind,
    EditComment, GoToThread, ReplyToThread, ToggleComments, ToggleResolved, ToggleShowResolved,
};
use innerword_core::TextRange;
use innerword_editor::EditorConfig;
use innerword_ui::{ContextMenu, ContextMenuItem, KeyHint, MenuAnchor};

use super::{
    mouse::{bounds, card, card_point, click, click_card_button, move_to},
    support::{Fixture, Focus, open, open_window, sample, setup},
};

fn right_click(position: Point<Pixels>, cx: &mut VisualTestContext) {
    move_to(position, cx);
    cx.simulate_mouse_down(position, MouseButton::Right, Modifiers::none());
    cx.simulate_mouse_up(position, MouseButton::Right, Modifiers::none());
    cx.run_until_parked();
}

/// The titles of the open menu, `-` for a separator.
fn labels(cx: &mut VisualTestContext) -> Option<Vec<String>> {
    cx.update(|window, cx| ContextMenu::current(window, cx).map(|menu| menu.read(cx).labels()))
}

fn items(cx: &mut VisualTestContext) -> Vec<ContextMenuItem> {
    cx.update(|window, cx| {
        ContextMenu::current(window, cx)
            .map(|menu| menu.read(cx).items().to_vec())
            .unwrap_or_default()
    })
}

fn item(title: &str, cx: &mut VisualTestContext) -> ContextMenuItem {
    items(cx)
        .into_iter()
        .find(|item| item.title().as_ref() == title)
        .unwrap_or_else(|| panic!("the menu has no item {title}"))
}

/// Chooses an item of the open menu as a click on it does.
fn choose(title: &str, cx: &mut VisualTestContext) {
    let chosen = cx.update(|window, cx| ContextMenu::choose(title, window, cx));
    assert!(chosen, "the menu has no item {title} that can be chosen");
    cx.run_until_parked();
}

/// Clicks the row of an item of the open menu.
fn click_item(title: &str, cx: &mut VisualTestContext) {
    let row = cx.update(|window, cx| {
        let menu = ContextMenu::current(window, cx).expect("a menu is open");
        let menu = menu.read(cx);
        let ix = menu
            .items()
            .iter()
            .position(|item| item.title().as_ref() == title)
            .unwrap_or_else(|| panic!("the menu has no item {title}"));
        menu.item_bounds(ix).expect("the menu was drawn")
    });
    move_to(row.center(), cx);
    cx.simulate_click(row.center(), Modifiers::none());
    cx.run_until_parked();
}

fn menu_bounds(cx: &mut VisualTestContext) -> Bounds<Pixels> {
    cx.update(|window, cx| {
        ContextMenu::current(window, cx)
            .expect("a menu is open")
            .read(cx)
            .bounds()
            .expect("the menu was drawn")
    })
}

fn menu_target(f: &Fixture, cx: &mut VisualTestContext) -> Option<CommentMenuTarget> {
    f.panel.read_with(cx, |panel, _| panel.menu_target())
}

/// The key the keymap has for `action` where the thread list has the
/// keyboard, as the menu shows it.
fn key_in_list(f: &Fixture, action: &dyn Action, cx: &mut VisualTestContext) -> Option<String> {
    let list = f.panel.read_with(cx, |panel, _| panel.list_focus_handle());
    cx.update(|window, _| KeyHint::for_action_in(action, &list, window).map(|hint| hint.text()))
}

const THREAD_MENU: [&str; 7] = ["Reply", "Edit", "Resolve", "-", "Copy Text", "-", "Delete"];

#[gpui::test]
fn a_right_click_on_a_thread_opens_its_menu(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let first = f.comment(0, 4, 9, "First", cx);
    let second = f.comment(1, 2, 8, "Second", cx);
    assert_eq!(f.active(cx), Some(second));
    assert_eq!(f.focus(cx), Focus::Editor);
    assert_eq!(labels(cx), None);

    // The thread that was clicked is the one the menu is about: it is the
    // active one and the list has the keyboard.
    let at = card_point(first, cx);
    right_click(at, cx);
    assert_eq!(labels(cx).unwrap(), THREAD_MENU);
    assert_eq!(menu_target(&f, cx), Some(CommentMenuTarget::Thread(first)));
    assert_eq!(f.active(cx), Some(first));
    assert_eq!(f.focus(cx), Focus::List);
    // At the pointer, inside the window.
    let menu = menu_bounds(cx);
    assert_eq!(menu.origin, at);

    // Every item is the action its key runs, and shows that key.
    let expect: [(&str, &dyn Action, &str); 5] = [
        ("Reply", &ReplyToThread, "↩"),
        ("Edit", &EditComment, "E"),
        ("Resolve", &ToggleResolved, "X"),
        ("Copy Text", &CopyComment, "⌘C"),
        ("Delete", &DeleteComment, "⌘⌫"),
    ];
    for (title, action, key) in expect {
        let item = item(title, cx);
        assert_eq!(item.action_name(), Some(action.name()), "{title}");
        assert!(!item.is_disabled(), "{title}");
        assert_eq!(key_in_list(&f, action, cx).as_deref(), Some(key), "{title}");
    }
    assert!(item("Delete", cx).is_danger());

    // A click on an item runs it on that thread.
    click_item("Reply", cx);
    assert_eq!(labels(cx), None);
    assert_eq!(menu_target(&f, cx), None);
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.draft(), Some(&DraftKind::Reply { thread: first }));
    });
    assert_eq!(f.focus(cx), Focus::Composer);
    cx.simulate_input("Yes");
    cx.simulate_keystrokes("enter");
    assert_eq!(f.focus(cx), Focus::List);

    // Edit.
    right_click(card_point(second, cx), cx);
    assert_eq!(f.active(cx), Some(second));
    click_item("Edit", cx);
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(
            panel.draft(),
            Some(&DraftKind::EditComment { thread: second })
        );
    });
    assert_eq!(f.composer_text(cx), "Second");
    cx.simulate_keystrokes("escape");

    // Resolve; on a resolved thread the item reopens.
    cx.simulate_keystrokes("s");
    right_click(card_point(first, cx), cx);
    click_item("Resolve", cx);
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.thread(first).unwrap().is_resolved());
    });
    right_click(card_point(first, cx), cx);
    assert_eq!(
        labels(cx).unwrap(),
        ["Reply", "Edit", "Reopen", "-", "Copy Text", "-", "Delete"]
    );
    click_item("Reopen", cx);
    f.panel.read_with(cx, |panel, _| {
        assert!(!panel.thread(first).unwrap().is_resolved());
    });

    // Copy Text.
    right_click(card_point(second, cx), cx);
    click_item("Copy Text", cx);
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("Second".to_string())
    );

    // Delete asks first, in the card, as the key does.
    right_click(card_point(first, cx), cx);
    click_item("Delete", cx);
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.pending_delete(), Some(DeleteTarget::Thread(first)));
    });
    assert_eq!(f.comment_count(cx), 2);
    click("confirm-delete-comment", cx);
    assert_eq!(f.all_ids(cx), vec![second]);
    assert_eq!(f.focus(cx), Focus::List);
}

#[gpui::test]
fn the_menu_closes_without_doing_anything(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let first = f.comment(0, 4, 9, "First", cx);
    let second = f.comment(1, 2, 8, "Second", cx);

    // Escape.
    right_click(card_point(first, cx), cx);
    assert!(labels(cx).is_some());
    cx.simulate_keystrokes("escape");
    assert_eq!(labels(cx), None);
    assert_eq!(menu_target(&f, cx), None);
    // The key went to the menu: the list still has the keyboard.
    assert_eq!(f.focus(cx), Focus::List);
    assert_eq!(f.comment_count(cx), 2);

    // A click outside closes it and does nothing else: the other card is
    // not activated by it.
    right_click(card_point(second, cx), cx);
    assert_eq!(f.active(cx), Some(second));
    let other = card_point(first, cx);
    assert!(!menu_bounds(cx).contains(&other));
    cx.simulate_click(other, Modifiers::none());
    assert_eq!(labels(cx), None);
    assert_eq!(f.active(cx), Some(second));
    // The next click is an ordinary one again.
    cx.simulate_click(other, Modifiers::none());
    assert_eq!(f.active(cx), Some(first));

    // A right-click on another card moves the menu there.
    right_click(card_point(second, cx), cx);
    assert_eq!(menu_target(&f, cx), Some(CommentMenuTarget::Thread(second)));
    right_click(other, cx);
    assert_eq!(menu_target(&f, cx), Some(CommentMenuTarget::Thread(first)));
    assert_eq!(f.active(cx), Some(first));
    assert_eq!(labels(cx).unwrap(), THREAD_MENU);
    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes("j ctrl-enter");
    assert_eq!(menu_target(&f, cx), Some(CommentMenuTarget::Thread(second)));

    // The list keys are the menu's while it is open: `x` does not resolve.
    cx.simulate_keystrokes("x");
    f.panel.read_with(cx, |panel, _| {
        assert!(!panel.thread(second).unwrap().is_resolved());
    });
    cx.simulate_keystrokes("escape");
    assert_eq!(labels(cx), None);
}

#[gpui::test]
fn the_more_button_and_ctrl_enter_open_the_same_menu(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let first = f.comment(0, 4, 9, "First", cx);
    let second = f.comment(1, 2, 8, "Second", cx);

    // The button of a card under the pointer: the menu of that card,
    // hanging under the button.
    click_card_button("comment-more-button", first, cx);
    assert_eq!(menu_target(&f, cx), Some(CommentMenuTarget::Thread(first)));
    assert_eq!(f.active(cx), Some(first));
    assert_eq!(f.focus(cx), Focus::List);
    assert_eq!(labels(cx).unwrap(), THREAD_MENU);
    let button = bounds(format!("comment-more-button-{}", first.0), cx);
    let menu = menu_bounds(cx);
    assert!(menu.top() >= button.bottom());
    assert!(menu.top() - button.bottom() < px(8.));
    let window = cx.update(|window, _| window.viewport_size());
    assert!(menu.right() <= window.width && menu.bottom() <= window.height);
    // Nothing is marked: the pointer opened it.
    let marked = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            ContextMenu::current(window, cx).and_then(|menu| menu.read(cx).selected_index())
        })
    };
    assert_eq!(marked(cx), None);
    cx.simulate_keystrokes("escape");

    // ctrl-enter in the list: the menu of the active thread at the same
    // place, with the first item marked, driven by the keyboard.
    cx.simulate_keystrokes("j");
    assert_eq!(f.active(cx), Some(second));
    cx.simulate_keystrokes("ctrl-enter");
    assert_eq!(menu_target(&f, cx), Some(CommentMenuTarget::Thread(second)));
    assert_eq!(labels(cx).unwrap(), THREAD_MENU);
    assert_eq!(marked(cx), Some(0));
    let anchor =
        cx.update(|window, cx| ContextMenu::current(window, cx).unwrap().read(cx).anchor());
    let button = bounds(format!("comment-more-button-{}", second.0), cx);
    assert_eq!(anchor, MenuAnchor::Below(button));
    cx.simulate_keystrokes("down down enter");
    assert_eq!(labels(cx), None);
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.thread(second).unwrap().is_resolved());
    });
    // The keyboard never left the list.
    assert_eq!(f.focus(cx), Focus::List);
    assert_eq!(f.active(cx), Some(first));

    // Pressed again, the key closes the menu.
    cx.simulate_keystrokes("ctrl-enter");
    assert!(labels(cx).is_some());
    cx.simulate_keystrokes("ctrl-enter");
    assert_eq!(labels(cx), None);
}

#[gpui::test]
fn a_reply_has_its_own_menu(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let id = f.comment(0, 4, 9, "Tighten this", cx);
    cx.simulate_keystrokes("cmd-2 r");
    cx.simulate_input("One");
    cx.simulate_keystrokes("enter r");
    cx.simulate_input("Two");
    cx.simulate_keystrokes("enter escape");
    let replies: Vec<_> = f.panel.read_with(cx, |panel, _| {
        let thread = &panel.thread(id).unwrap().thread;
        thread.replies.iter().map(|reply| reply.id).collect()
    });
    let reply_bounds = |ix: usize, cx: &mut VisualTestContext| {
        bounds(format!("comment-reply-{}-{}", id.0, replies[ix].0), cx)
    };

    // A right-click on a reply selects it and offers what applies to it.
    let at = reply_bounds(1, cx).center();
    right_click(at, cx);
    assert_eq!(
        labels(cx).unwrap(),
        ["Edit Reply", "Copy Text", "-", "Delete Reply"]
    );
    assert_eq!(
        menu_target(&f, cx),
        Some(CommentMenuTarget::Reply(id, replies[1]))
    );
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.selected_reply(), Some(replies[1]));
    });
    assert_eq!(f.focus(cx), Focus::List);
    assert_eq!(
        item("Edit Reply", cx).action_name(),
        Some(EditComment.name())
    );
    assert_eq!(
        item("Delete Reply", cx).action_name(),
        Some(DeleteComment.name())
    );
    assert!(item("Delete Reply", cx).is_danger());

    click_item("Copy Text", cx);
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("Two".to_string())
    );

    right_click(at, cx);
    click_item("Edit Reply", cx);
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(
            panel.draft(),
            Some(&DraftKind::EditReply {
                thread: id,
                reply: replies[1]
            })
        );
    });
    assert_eq!(f.composer_text(cx), "Two");
    cx.simulate_keystrokes("escape");

    // The button of a reply under the pointer opens the same menu; so
    // does ctrl-enter with the reply selected.
    let first_reply = reply_bounds(0, cx).center();
    move_to(first_reply, cx);
    click(
        format!("comment-reply-more-button-{}-{}", id.0, replies[0].0),
        cx,
    );
    assert_eq!(
        menu_target(&f, cx),
        Some(CommentMenuTarget::Reply(id, replies[0]))
    );
    click_item("Delete Reply", cx);
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(
            panel.pending_delete(),
            Some(DeleteTarget::Reply(id, replies[0]))
        );
    });
    cx.simulate_keystrokes("enter");
    f.panel.read_with(cx, |panel, _| {
        let thread = &panel.thread(id).unwrap().thread;
        assert_eq!(thread.replies.len(), 1);
        assert_eq!(thread.replies[0].id, replies[1]);
    });

    cx.simulate_keystrokes("l");
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.selected_reply(), Some(replies[1]));
    });
    cx.simulate_keystrokes("ctrl-enter");
    assert_eq!(
        menu_target(&f, cx),
        Some(CommentMenuTarget::Reply(id, replies[1]))
    );
    cx.simulate_keystrokes("escape");
}

#[gpui::test]
fn a_detached_thread_can_be_deleted_from_its_menu(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let id = f.comment(0, 4, 9, "About the fox", cx);
    // Delete the commented text.
    f.editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.select_range(TextRange::in_block(0, 0, 44));
                state.delete_selection();
            },
            cx,
        );
    });
    cx.run_until_parked();
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.thread(id).unwrap().is_detached());
    });

    right_click(card_point(id, cx), cx);
    assert_eq!(labels(cx).unwrap(), THREAD_MENU);
    click_item("Delete", cx);
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.pending_delete(), Some(DeleteTarget::Thread(id)));
    });
    click("confirm-delete-comment", cx);
    assert_eq!(f.comment_count(cx), 0);
    assert_eq!(f.focus(cx), Focus::Editor);
}

#[gpui::test]
fn a_right_click_beside_the_cards_opens_the_menu_of_the_panel(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let first = f.comment(0, 4, 9, "First", cx);
    f.select(1, 2, 8, cx);
    assert_eq!(f.focus(cx), Focus::Editor);

    // The header is not a card.
    let header = bounds("comments-add", cx);
    let at = point(header.left() - px(120.), header.center().y);
    right_click(at, cx);
    assert_eq!(
        labels(cx).unwrap(),
        [
            "Add Comment on Selection",
            "-",
            "Show Resolved",
            "Hide Comments"
        ]
    );
    assert_eq!(menu_target(&f, cx), Some(CommentMenuTarget::Panel));
    // The keyboard stays where it was, and so does the selection.
    assert_eq!(f.focus(cx), Focus::Editor);
    assert_eq!(f.selection(cx).range(), TextRange::in_block(1, 2, 8));
    let expect: [(&str, &dyn Action); 3] = [
        ("Add Comment on Selection", &AddComment),
        ("Show Resolved", &ToggleShowResolved),
        ("Hide Comments", &ToggleComments),
    ];
    for (title, action) in expect {
        assert_eq!(
            item(title, cx).action_name(),
            Some(action.name()),
            "{title}"
        );
    }
    // Nothing is resolved, so there is nothing to show.
    assert!(item("Show Resolved", cx).is_disabled());
    assert!(!item("Add Comment on Selection", cx).is_disabled());
    f.panel
        .read_with(cx, |panel, cx| assert!(panel.can_add_comment(cx)));
    assert_eq!(
        key_in_list(&f, &AddComment, cx).as_deref(),
        Some("⌥⌘M"),
        "the key of Add Comment"
    );

    click_item("Add Comment on Selection", cx);
    f.panel.read_with(cx, |panel, _| {
        assert!(matches!(panel.draft(), Some(DraftKind::New { range })
            if *range == TextRange::in_block(1, 2, 8)));
    });
    assert_eq!(f.focus(cx), Focus::Composer);
    cx.simulate_input("Second");
    cx.simulate_keystrokes("enter");
    let second = f.all_ids(cx)[1];

    // The room under the cards is the panel too. With a resolved thread
    // the menu shows and hides them.
    click_card_button("comment-resolve-button", second, cx);
    let panel = bounds("comments-panel", cx);
    let below = point(panel.center().x, card(first, cx).bottom() + px(120.));
    right_click(below, cx);
    assert_eq!(menu_target(&f, cx), Some(CommentMenuTarget::Panel));
    assert!(!item("Show Resolved", cx).is_disabled());
    click_item("Show Resolved", cx);
    f.panel
        .read_with(cx, |panel, _| assert!(panel.show_resolved()));
    right_click(point(at.x, at.y), cx);
    assert_eq!(labels(cx).unwrap()[2], "Hide Resolved");
    click_item("Hide Resolved", cx);
    f.panel
        .read_with(cx, |panel, _| assert!(!panel.show_resolved()));

    // Without text to anchor to (the caret in white space), a comment
    // cannot be added: the item says so.
    f.editor.update(cx, |editor, cx| {
        editor.edit(|state| state.insert_text("   "), cx);
    });
    cx.run_until_parked();
    f.panel
        .read_with(cx, |panel, cx| assert!(!panel.can_add_comment(cx)));
    right_click(at, cx);
    assert!(item("Add Comment on Selection", cx).is_disabled());
    cx.simulate_keystrokes("escape");

    right_click(at, cx);
    click_item("Hide Comments", cx);
    f.panel
        .read_with(cx, |panel, _| assert!(!panel.is_visible()));
    assert_eq!(f.visibility_events(cx).last(), Some(&false));
}

#[gpui::test]
fn ctrl_enter_on_an_empty_list_opens_the_menu_of_the_panel(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    f.comment(0, 4, 9, "Only", cx);
    cx.simulate_keystrokes("cmd-2 x cmd-2");
    assert_eq!(f.focus(cx), Focus::List);
    assert_eq!(f.active(cx), None);
    cx.simulate_keystrokes("ctrl-enter");
    assert_eq!(menu_target(&f, cx), Some(CommentMenuTarget::Panel));
    assert_eq!(labels(cx).unwrap()[2], "Show Resolved");
    cx.simulate_keystrokes("down enter");
    f.panel
        .read_with(cx, |panel, _| assert!(panel.show_resolved()));
}

#[gpui::test]
fn the_app_adds_its_own_items(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let first = f.comment(0, 4, 9, "First", cx);
    cx.simulate_keystrokes("cmd-2 r");
    cx.simulate_input("One");
    cx.simulate_keystrokes("enter escape");
    let reply = f.panel.read_with(cx, |panel, _| {
        panel.thread(first).unwrap().thread.replies[0].id
    });

    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let chosen = std::rc::Rc::new(std::cell::Cell::new(None));
    let handle = f.panel.clone();
    f.panel.update(cx, |panel, _| {
        let seen = seen.clone();
        let chosen = chosen.clone();
        panel.set_menu_extension(move |target, _, cx| {
            seen.borrow_mut().push(target);
            // The panel can be read while the extension runs, and what
            // the menu is on is already the active thread.
            assert_eq!(
                handle.read(cx).active_thread().is_some(),
                target.thread().is_some() || handle.read(cx).active_thread().is_some()
            );
            if let Some(thread) = target.thread() {
                assert_eq!(handle.read(cx).active_thread(), Some(thread));
            }
            match target {
                CommentMenuTarget::Thread(thread) => {
                    let chosen = chosen.clone();
                    vec![ContextMenuItem::new("Export Thread", move |_, _| {
                        chosen.set(Some(thread))
                    })]
                }
                CommentMenuTarget::Panel => {
                    vec![ContextMenuItem::action(
                        "Next Comment",
                        innerword_comments::NextComment,
                    )]
                }
                CommentMenuTarget::Reply(..) => Vec::new(),
            }
        });
    });

    // On a thread: between the panel's items and Delete.
    right_click(card_point(first, cx), cx);
    assert_eq!(
        labels(cx).unwrap(),
        [
            "Reply",
            "Edit",
            "Resolve",
            "-",
            "Copy Text",
            "-",
            "Export Thread",
            "-",
            "Delete"
        ]
    );
    choose("Export Thread", cx);
    assert_eq!(chosen.get(), Some(first));

    // On a reply the app adds nothing: no empty group.
    let at = bounds(format!("comment-reply-{}-{}", first.0, reply.0), cx).center();
    right_click(at, cx);
    assert_eq!(
        labels(cx).unwrap(),
        ["Edit Reply", "Copy Text", "-", "Delete Reply"]
    );
    cx.simulate_keystrokes("escape");

    // On the panel: after its own items. An action item is dispatched at
    // the thread list and reaches the panel like a key.
    let header = bounds("comments-add", cx);
    right_click(point(header.left() - px(120.), header.center().y), cx);
    assert_eq!(
        labels(cx).unwrap(),
        [
            "Add Comment on Selection",
            "-",
            "Show Resolved",
            "Hide Comments",
            "-",
            "Next Comment"
        ]
    );
    f.caret(2, 0, cx);
    right_click(point(header.left() - px(120.), header.center().y), cx);
    choose("Next Comment", cx);
    assert_eq!(f.selection(cx).head.block, 0);
    assert_eq!(f.active(cx), Some(first));

    assert_eq!(
        *seen.borrow(),
        vec![
            CommentMenuTarget::Thread(first),
            CommentMenuTarget::Reply(first, reply),
            CommentMenuTarget::Panel,
            CommentMenuTarget::Panel,
        ]
    );
}

#[gpui::test]
fn a_read_only_document_keeps_the_menu_and_dims_what_would_change_it(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let id = f.comment(0, 4, 9, "First", cx);
    f.editor
        .update(cx, |editor, cx| editor.set_read_only(true, cx));
    cx.run_until_parked();

    right_click(card_point(id, cx), cx);
    assert_eq!(labels(cx).unwrap(), THREAD_MENU);
    for title in ["Reply", "Edit", "Resolve", "Delete"] {
        assert!(item(title, cx).is_disabled(), "{title}");
    }
    assert!(!item("Copy Text", cx).is_disabled());
    click_item("Copy Text", cx);
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("First".to_string())
    );
    // The card offers only the menu.
    move_to(card_point(id, cx), cx);
    click(format!("comment-more-button-{}", id.0), cx);
    assert_eq!(menu_target(&f, cx), Some(CommentMenuTarget::Thread(id)));
    cx.simulate_keystrokes("escape");
    f.panel.read_with(cx, |panel, _| {
        assert!(!panel.is_composing());
        assert!(!panel.thread(id).unwrap().is_resolved());
    });
}

#[gpui::test]
fn the_composer_has_the_menu_of_a_text_field(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let id = f.comment(0, 4, 9, "First", cx);
    cx.simulate_keystrokes("cmd-2 r");
    cx.simulate_input("some words");
    assert_eq!(f.focus(cx), Focus::Composer);

    // A right-click in the composer is about its text, not the card.
    let composer = bounds("comment-composer", cx);
    right_click(point(composer.left() + px(30.), composer.center().y), cx);
    assert_eq!(
        labels(cx).unwrap(),
        ["Cut", "Copy", "Paste", "-", "Select All"]
    );
    assert_eq!(menu_target(&f, cx), None);
    assert_eq!(f.focus(cx), Focus::Composer);
    // The click selected the word under the pointer.
    assert!(!item("Copy", cx).is_disabled());
    click_item("Copy", cx);
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("some".to_string())
    );
    assert_eq!(f.composer_text(cx), "some words");

    // From the keyboard, at the caret.
    cx.simulate_keystrokes("ctrl-enter");
    assert_eq!(labels(cx).unwrap()[0], "Cut");
    choose("Select All", cx);
    cx.simulate_keystrokes("ctrl-enter");
    choose("Cut", cx);
    assert_eq!(f.composer_text(cx), "");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("some words".to_string())
    );
    cx.simulate_keystrokes("ctrl-enter");
    choose("Paste", cx);
    assert_eq!(f.composer_text(cx), "some words");
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.draft(), Some(&DraftKind::Reply { thread: id }));
    });
}

#[gpui::test]
fn cmd_c_in_the_list_copies_the_comment(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    f.comment(0, 4, 9, "Tighten this", cx);
    cx.simulate_keystrokes("cmd-2 r");
    cx.simulate_input("A reply");
    cx.simulate_keystrokes("enter cmd-c");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("Tighten this".to_string())
    );
    cx.simulate_keystrokes("l cmd-c");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("A reply".to_string())
    );
}

#[gpui::test]
fn go_to_a_thread_by_its_id(cx: &mut TestAppContext) {
    setup(cx);
    let mut config = EditorConfig::full();
    config.comments_visible = false;
    let (f, cx) = open_window(sample(), config, cx);
    cx.simulate_keystrokes("cmd-alt-c");
    let first = f.comment(0, 4, 9, "First", cx);
    let second = f.comment(1, 2, 8, "Second", cx);
    // Resolved and not listed, comments hidden, the keyboard in the text.
    cx.simulate_keystrokes("cmd-2");
    cx.simulate_keystrokes("k x escape cmd-alt-c");
    f.panel.read_with(cx, |panel, _| {
        assert!(!panel.is_visible());
        assert!(panel.thread(first).unwrap().is_resolved());
        assert!(!panel.show_resolved());
    });
    assert_eq!(f.focus(cx), Focus::Editor);

    // The method the app calls for "Go to Comment" in the text's menu.
    let went = f.panel.update_in(cx, |panel, window, cx| {
        panel.go_to_thread(first, window, cx)
    });
    cx.run_until_parked();
    assert!(went);
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.is_visible());
        assert!(panel.show_resolved());
    });
    assert_eq!(f.active(cx), Some(first));
    assert_eq!(f.focus(cx), Focus::List);
    // The caret did not move.
    assert_eq!(f.selection(cx).head.block, 1);

    // The action does the same from anywhere in the window.
    cx.simulate_keystrokes("escape");
    assert_eq!(f.focus(cx), Focus::Editor);
    cx.dispatch_action(GoToThread::new(second));
    cx.run_until_parked();
    assert_eq!(f.active(cx), Some(second));
    assert_eq!(f.focus(cx), Focus::List);

    // A thread that does not exist: nothing happens.
    let went = f.panel.update_in(cx, |panel, window, cx| {
        panel.go_to_thread(innerword_core::CommentId(999), window, cx)
    });
    assert!(!went);
    assert_eq!(f.active(cx), Some(second));
}
