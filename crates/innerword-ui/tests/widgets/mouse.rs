//! The widgets under the mouse: the text field's context menu, buttons that
//! need a press and a release, tooltips, switches and segments, and the
//! button of a toast.

use std::{cell::RefCell, rc::Rc, time::Duration};

use gpui::{
    ClipboardItem, Context, Edges, Entity, FocusHandle, KeyBinding, Modifiers, MouseButton, Pixels,
    Point, TestAppContext, VisualTestContext, Window, actions, div, point, prelude::*, px, size,
};
use innerword_ui::{
    Button, Checkbox, ContextMenu, ContextMenuItem, ContextMenuLayer, IconName, KeyHint,
    MenuAnchor, SegmentedControl, Switch, TextField, TextFieldEvent, Toast, ToastLayer, Tooltip,
};

actions!(mouse_test, [Undo, Clear]);

type Log = Rc<RefCell<Vec<String>>>;

fn menu(cx: &mut VisualTestContext) -> Option<Entity<ContextMenu>> {
    cx.update(|window, cx| ContextMenu::current(window, cx))
}

fn labels(cx: &mut VisualTestContext) -> Vec<String> {
    menu(cx)
        .expect("a menu is open")
        .read_with(cx, |menu, _| menu.labels())
}

/// The titles of the items that cannot be chosen.
fn disabled(cx: &mut VisualTestContext) -> Vec<String> {
    menu(cx).expect("a menu is open").read_with(cx, |menu, _| {
        menu.items()
            .iter()
            .filter(|item| item.is_disabled())
            .map(|item| item.title().to_string())
            .collect()
    })
}

fn choose(cx: &mut VisualTestContext, title: &str) {
    assert!(
        cx.update(|window, cx| ContextMenu::choose(title, window, cx)),
        "{title} cannot be chosen"
    );
    cx.run_until_parked();
}

fn right_click(cx: &mut VisualTestContext, position: Point<Pixels>) {
    cx.simulate_mouse_down(position, MouseButton::Right, Modifiers::none());
    cx.simulate_mouse_up(position, MouseButton::Right, Modifiers::none());
}

fn clipboard(cx: &mut VisualTestContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|item| item.text())
}

/// In the test text system every character of the 13px field text is 7.8px
/// wide; the bordered field has a 1px border and 8px of padding.
fn at_char(ix: f32) -> Point<Pixels> {
    point(px(9. + 7.8 * ix), px(12.))
}

// ---------------------------------------------------------------------------
// TextField
// ---------------------------------------------------------------------------

#[gpui::test]
fn text_field_menu_by_right_click(cx: &mut TestAppContext) {
    let (field, cx, events) = super::field_window(cx, "hello world");
    // a right-click on a word outside the selection selects the word
    right_click(cx, at_char(8.5));
    assert_eq!(super::selection(&field, cx), 6..11);
    assert_eq!(labels(cx), ["Cut", "Copy", "Paste", "-", "Select All"]);
    assert_eq!(disabled(cx), ["Paste"], "nothing to paste yet");
    choose(cx, "Copy");
    assert_eq!(clipboard(cx).as_deref(), Some("world"));
    assert!(menu(cx).is_none());
    assert_eq!(super::text(&field, cx), "hello world");

    // a right-click on the selection keeps it
    right_click(cx, at_char(7.5));
    assert_eq!(super::selection(&field, cx), 6..11);
    assert!(disabled(cx).is_empty());
    choose(cx, "Select All");
    assert_eq!(super::selection(&field, cx), 0..11);

    right_click(cx, at_char(2.));
    assert_eq!(super::selection(&field, cx), 0..11, "inside the selection");
    choose(cx, "Cut");
    assert_eq!(super::text(&field, cx), "");
    assert_eq!(clipboard(cx).as_deref(), Some("hello world"));

    // an empty field: only Paste applies
    right_click(cx, at_char(2.));
    assert_eq!(disabled(cx), ["Cut", "Copy", "Select All"]);
    choose(cx, "Paste");
    assert_eq!(super::text(&field, cx), "hello world");
    // the menu's edits are ordinary edits
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(super::text(&field, cx), "");

    // the field had the keyboard all along: nothing was blurred
    assert!(field.update_in(cx, |field, window, _| field.is_focused(window)));
    assert!(!events.borrow().contains(&TextFieldEvent::Blurred));
}

#[gpui::test]
fn text_field_menu_from_the_keyboard(cx: &mut TestAppContext) {
    let (field, cx, events) = super::field_window(cx, "hello world");
    cx.write_to_clipboard(ClipboardItem::new_string("pasted".into()));
    cx.simulate_keystrokes("ctrl-enter");
    let open = menu(cx).expect("ctrl-enter opens the menu");
    // it drops down below the caret, and the first item that applies is marked
    let (anchor, selected) = open.read_with(cx, |menu, _| {
        (
            menu.anchor(),
            menu.selected_item().map(|item| item.title().to_string()),
        )
    });
    let MenuAnchor::Below(caret) = anchor else {
        panic!("{anchor:?}");
    };
    assert_eq!(caret.left(), at_char(11.).x, "at the caret");
    assert_eq!(caret.size.width, px(0.));
    assert_eq!(disabled(cx), ["Cut", "Copy"], "nothing is selected");
    assert_eq!(selected.as_deref(), Some("Paste"));

    // keys go to the menu, not into the field
    cx.simulate_keystrokes("s");
    assert_eq!(super::text(&field, cx), "hello world");
    cx.simulate_keystrokes("enter");
    assert!(menu(cx).is_none());
    assert_eq!(super::selection(&field, cx), 0..11, "Select All");
    assert!(!events.borrow().contains(&TextFieldEvent::Submitted));

    // escape closes the menu only: the field is not cancelled
    cx.simulate_keystrokes("ctrl-enter");
    assert!(disabled(cx).is_empty());
    cx.simulate_keystrokes("escape");
    assert!(menu(cx).is_none());
    assert!(!events.borrow().contains(&TextFieldEvent::Cancelled));
    assert!(!events.borrow().contains(&TextFieldEvent::Blurred));
    // and the field still has the keyboard
    cx.simulate_keystrokes("ctrl-enter down down enter");
    assert_eq!(super::text(&field, cx), "pasted");
    cx.simulate_input("!");
    assert_eq!(super::text(&field, cx), "pasted!");

    // a disabled field can be copied from, not changed
    field.update(cx, |field, cx| {
        field.set_disabled(true, cx);
        field.select_all(cx);
    });
    cx.simulate_keystrokes("ctrl-enter");
    assert_eq!(disabled(cx), ["Cut", "Paste"]);
    cx.simulate_keystrokes("escape");
}

#[gpui::test]
fn text_field_menu_with_items_of_its_owner(cx: &mut TestAppContext) {
    super::init(cx);
    let log = Log::default();
    let (field, cx) = cx.add_window_view({
        let log = log.clone();
        move |window, cx| {
            TextField::new(window, cx)
                .with_text("Chapter 1")
                .with_menu_items(move |_, _| {
                    let log = log.clone();
                    vec![ContextMenuItem::new("Clear", move |_, _| {
                        log.borrow_mut().push("clear".into())
                    })]
                })
        }
    });
    cx.update(|window, cx| {
        window.activate_window();
        field.read(cx).focus(window);
    });
    cx.run_until_parked();
    right_click(cx, at_char(1.));
    assert_eq!(
        labels(cx),
        ["Cut", "Copy", "Paste", "-", "Select All", "-", "Clear"]
    );
    choose(cx, "Clear");
    assert_eq!(*log.borrow(), ["clear"]);
}

// ---------------------------------------------------------------------------
// Buttons, switches, segments, tooltips
// ---------------------------------------------------------------------------

struct Controls {
    focus: FocusHandle,
    button_focus: FocusHandle,
    log: Log,
    checked: bool,
    segment: usize,
    toasts: Entity<ToastLayer>,
}

impl Render for Controls {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let log = self.log.clone();
        let undo_log = self.log.clone();
        div()
            .size_full()
            .track_focus(&self.focus)
            .on_action(move |_: &Undo, _, _| undo_log.borrow_mut().push("undo".into()))
            .flex()
            .flex_col()
            .items_start()
            .gap(px(20.))
            .p(px(20.))
            .child(
                div().debug_selector(|| "button".into()).child(
                    Button::new("save", "Save")
                        .track_focus(&self.button_focus)
                        .on_click(move |_, _, _| log.borrow_mut().push("save".into())),
                ),
            )
            .child(
                div().debug_selector(|| "icon".into()).child(
                    Button::icon_only("close", IconName::Close)
                        .tooltip("Close")
                        .key_hint(KeyHint::parse("cmd-w")),
                ),
            )
            .child(
                div().debug_selector(|| "switch".into()).child(
                    Switch::new("switch", self.checked)
                        .label("Page view")
                        .on_change(cx.listener(|this, checked: &bool, _, cx| {
                            this.checked = *checked;
                            cx.notify();
                        })),
                ),
            )
            .child(
                div().debug_selector(|| "checkbox".into()).child(
                    Checkbox::new("checkbox", self.checked)
                        .label("Page view")
                        .on_change(cx.listener(|this, checked: &bool, _, cx| {
                            this.checked = *checked;
                            cx.notify();
                        })),
                ),
            )
            .child(div().debug_selector(|| "segments".into()).child(
                SegmentedControl::new("segments", ["One", "Two", "Three"], self.segment).on_select(
                    cx.listener(|this, ix: &usize, _, cx| {
                        this.segment = *ix;
                        cx.notify();
                    }),
                ),
            ))
            .child(
                div()
                    .id("plain")
                    .debug_selector(|| "plain".into())
                    .w(px(60.))
                    .h(px(20.))
                    .tooltip(Tooltip::for_action("Undo", &Undo)),
            )
            .child(self.toasts.clone())
            .child(ContextMenuLayer)
    }
}

fn controls_window(cx: &mut TestAppContext) -> (Entity<Controls>, &mut VisualTestContext, Log) {
    super::init(cx);
    cx.update(|cx| cx.bind_keys([KeyBinding::new("cmd-z", Undo, None)]));
    let log = Log::default();
    let (view, cx) = cx.add_window_view({
        let log = log.clone();
        move |window, cx| {
            let focus = cx.focus_handle();
            window.focus(&focus);
            Controls {
                focus,
                button_focus: cx.focus_handle().tab_stop(true),
                log,
                checked: false,
                segment: 0,
                toasts: cx.new(|_| ToastLayer::new()),
            }
        }
    });
    cx.simulate_resize(size(px(800.), px(600.)));
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    (view, cx, log)
}

fn centre(cx: &mut VisualTestContext, selector: &'static str) -> Point<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
        .center()
}

#[gpui::test]
fn a_button_needs_the_press_and_the_release(cx: &mut TestAppContext) {
    let (view, cx, log) = controls_window(cx);
    let button = centre(cx, "button");
    let elsewhere = point(px(600.), px(500.));
    let none = Modifiers::none();

    // letting go of a drag over the button does not press it
    cx.simulate_mouse_down(elsewhere, MouseButton::Left, none);
    cx.simulate_mouse_move(button, MouseButton::Left, none);
    cx.simulate_mouse_up(button, MouseButton::Left, none);
    assert!(log.borrow().is_empty());
    // neither does a press that is let go of somewhere else
    cx.simulate_mouse_down(button, MouseButton::Left, none);
    cx.simulate_mouse_up(elsewhere, MouseButton::Left, none);
    assert!(log.borrow().is_empty());
    // the right button does not press it
    right_click(cx, button);
    assert!(log.borrow().is_empty());

    cx.simulate_click(button, none);
    assert_eq!(*log.borrow(), ["save"]);

    // from the keyboard: once per key, on the way down
    let focus = view.read_with(cx, |view, _| view.button_focus.clone());
    cx.update(|window, _| window.focus(&focus));
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    assert_eq!(*log.borrow(), ["save", "save"]);
    cx.simulate_keystrokes("space");
    assert_eq!(*log.borrow(), ["save", "save", "save"]);
}

#[gpui::test]
fn switches_and_segments_follow_clicks(cx: &mut TestAppContext) {
    let (view, cx, _log) = controls_window(cx);
    let none = Modifiers::none();
    let switch = centre(cx, "switch");
    cx.simulate_click(switch, none);
    assert!(view.read_with(cx, |view, _| view.checked));
    let checkbox = centre(cx, "checkbox");
    cx.simulate_click(checkbox, none);
    assert!(!view.read_with(cx, |view, _| view.checked));
    // a release alone (the end of a drag) changes nothing
    cx.simulate_mouse_up(switch, MouseButton::Left, none);
    assert!(!view.read_with(cx, |view, _| view.checked));

    let segments = cx.debug_bounds("segments").unwrap();
    let third = point(segments.right() - px(12.), segments.center().y);
    cx.simulate_click(third, none);
    assert_eq!(view.read_with(cx, |view, _| view.segment), 2);
    cx.simulate_mouse_up(
        point(segments.left() + px(12.), segments.center().y),
        MouseButton::Left,
        none,
    );
    assert_eq!(view.read_with(cx, |view, _| view.segment), 2);
}

#[gpui::test]
fn tooltips_name_the_control_and_its_key(cx: &mut TestAppContext) {
    let (_view, cx, _log) = controls_window(cx);
    let icon = centre(cx, "icon");
    assert!(cx.debug_bounds("tooltip").is_none());
    cx.simulate_mouse_move(icon, None, Modifiers::none());
    assert!(cx.debug_bounds("tooltip").is_none(), "not at once");
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    let tooltip = cx.debug_bounds("tooltip").expect("the tooltip is shown");
    // "Close" and "⌘W" in the 12px test font, with the gap and the padding
    assert!(tooltip.size.width > px(60.), "{tooltip:?}");

    // the pieces, for elements that are not kit buttons
    let tip = Tooltip::new("Resolve").key_hint(KeyHint::parse("cmd-enter"));
    assert_eq!(tip.title().as_ref(), "Resolve");
    assert_eq!(tip.hint().map(|hint| hint.text()), Some("⌘↩".to_string()));
    cx.update(|window, cx| {
        let view = Tooltip::for_action("Undo", &Undo)(window, cx);
        let tip = view.downcast::<Tooltip>().unwrap();
        assert_eq!(
            tip.read(cx).hint().map(|hint| hint.text()),
            Some("⌘Z".to_string()),
            "the key is looked up in the keymap"
        );
        let view = Tooltip::text("Page view")(window, cx);
        assert!(
            view.downcast::<Tooltip>()
                .unwrap()
                .read(cx)
                .hint()
                .is_none()
        );
    });
}

// ---------------------------------------------------------------------------
// Toasts
// ---------------------------------------------------------------------------

#[gpui::test]
fn a_toast_with_a_button(cx: &mut TestAppContext) {
    let (view, cx, log) = controls_window(cx);
    let toasts = view.read_with(cx, |view, _| view.toasts.clone());
    let none = Modifiers::none();

    let id = toasts.update(cx, |layer, cx| {
        layer.push(
            Toast::new("Moved “Draft” to the trash.").action("Undo", Undo),
            cx,
        )
    });
    cx.run_until_parked();
    assert_eq!(
        toasts.read_with(cx, |layer, _| layer
            .toasts()
            .map(|toast| toast.action_label().map(|label| label.to_string()))
            .collect::<Vec<_>>()),
        [Some("Undo".to_string())]
    );
    assert_eq!(toasts.read_with(cx, |layer, _| layer.ids()), [id]);
    // the button runs the action where the keyboard is and closes the toast
    let button = centre(cx, "toast-action-0");
    cx.simulate_click(button, none);
    assert_eq!(*log.borrow(), ["undo"]);
    assert_eq!(toasts.read_with(cx, |layer, _| layer.toasts().count()), 0);

    // a closure button; a click beside the button only closes the toast
    let closure_log = log.clone();
    toasts.update(cx, |layer, cx| {
        layer.push(
            Toast::new("Exported Essay.md").on_action("Show", move |_, _| {
                closure_log.borrow_mut().push("show".into())
            }),
            cx,
        );
    });
    cx.run_until_parked();
    let toast = cx.debug_bounds("toast-1").unwrap();
    cx.simulate_click(point(toast.left() + px(6.), toast.center().y), none);
    assert_eq!(toasts.read_with(cx, |layer, _| layer.toasts().count()), 0);
    assert_eq!(*log.borrow(), ["undo"]);

    let closure_log = log.clone();
    let id = toasts.update(cx, |layer, cx| {
        layer.push(
            Toast::new("Exported Essay.md").on_action("Show", move |_, _| {
                closure_log.borrow_mut().push("show".into())
            }),
            cx,
        )
    });
    cx.run_until_parked();
    let button = centre(cx, "toast-action-2");
    cx.simulate_click(button, none);
    assert_eq!(*log.borrow(), ["undo", "show"]);
    assert!(!toasts.update_in(cx, |layer, window, cx| layer.run_action(id, window, cx)));

    // the stack clears a status bar and can be centred on a column
    let id = toasts.update(cx, |layer, cx| layer.push(Toast::new("Saved"), cx));
    cx.run_until_parked();
    let _ = id;
    let toast = cx.debug_bounds("toast-3").unwrap();
    assert_eq!(
        toast.bottom(),
        px(600.) - innerword_ui::toast::DEFAULT_BOTTOM_INSET
    );
    assert_eq!(toast.center().x, px(400.));
    toasts.update(cx, |layer, cx| {
        layer.set_insets(
            Edges {
                left: px(240.),
                bottom: px(60.),
                ..Edges::default()
            },
            cx,
        )
    });
    cx.run_until_parked();
    let toast = cx.debug_bounds("toast-3").unwrap();
    assert_eq!(toast.bottom(), px(540.));
    assert_eq!(toast.center().x, px(520.), "centred right of the sidebar");
}
